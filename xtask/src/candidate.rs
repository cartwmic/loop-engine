use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::config::{ManifestDocument, SemanticRequirement, load_manifest};
use crate::git::{GitEnvironment, Repository, ensure_success, utf8_line};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Materialized Git candidate. Call [`Candidate::prepare`] before execution.
#[derive(Debug)]
pub struct Candidate {
    repository: Repository,
    base_revision: String,
    candidate_revision: String,
    candidate_tree: String,
    changed_paths: Vec<PathBuf>,
    source_root: PathBuf,
    scratch_root: PathBuf,
    cache_root: PathBuf,
    target_root: PathBuf,
    storage_root: PathBuf,
    origin: CandidateOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateOrigin {
    Staged,
    Revision,
}

/// Candidate descriptor proving manifest policy and runner-input parity.
#[derive(Debug)]
pub struct PreparedCandidate {
    candidate: Candidate,
    manifest: ManifestDocument,
}

/// Failed consuming cleanup. Owns candidate so caller can repair cause and retry.
#[derive(Debug)]
pub struct CandidateCleanupError {
    candidate: Box<PreparedCandidate>,
    error: anyhow::Error,
}

impl CandidateCleanupError {
    pub fn storage_root(&self) -> &Path {
        &self.candidate.candidate.storage_root
    }

    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }

    pub fn retry(mut self) -> std::result::Result<(), Self> {
        match self.candidate.candidate.cleanup_inner() {
            Ok(()) => Ok(()),
            Err(error) => Err(Self {
                candidate: self.candidate,
                error,
            }),
        }
    }
}

impl std::fmt::Display for CandidateCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for CandidateCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

impl Candidate {
    /// Materialize exact effective-index state. Missing `HEAD` uses empty tree.
    pub fn staged(repository_path: &Path) -> Result<Self> {
        Self::staged_with_after_snapshot(repository_path, || {})
    }

    fn staged_with_after_snapshot(
        repository_path: &Path,
        after_snapshot: impl FnOnce(),
    ) -> Result<Self> {
        let repository = Repository::resolve(repository_path)?;
        let head_before = repository.head()?;
        let base_revision = match head_before.as_ref() {
            Some(head) => head.clone(),
            None => repository.empty_tree()?,
        };
        let candidate_tree = snapshot_effective_index_tree(&repository)?;
        after_snapshot();
        let head_after = repository.head()?;
        if head_after != head_before {
            bail!("HEAD changed while constructing staged candidate; retry candidate construction");
        }
        let changed_paths = repository.changed_paths(&base_revision, &candidate_tree)?;
        Self::materialize(
            repository,
            base_revision,
            candidate_tree.clone(),
            candidate_tree,
            changed_paths,
            CandidateOrigin::Staged,
        )
    }

    /// Materialize exact candidate commit. `None` base represents new branch.
    pub fn revision(
        repository_path: &Path,
        base_revision: Option<&OsStr>,
        candidate_revision: &OsStr,
    ) -> Result<Self> {
        let repository = Repository::resolve(repository_path)?;
        let base_revision = match base_revision {
            Some(base) => repository.resolve_commit(base)?,
            None => repository.empty_tree()?,
        };
        let candidate_revision = repository.resolve_commit(candidate_revision)?;
        let candidate_tree = repository.resolve_tree(&candidate_revision)?;
        let changed_paths = repository.changed_paths(&base_revision, &candidate_revision)?;
        Self::materialize(
            repository,
            base_revision,
            candidate_revision,
            candidate_tree,
            changed_paths,
            CandidateOrigin::Revision,
        )
    }

    /// Load candidate policy and prove its declared runner inputs match worktree.
    pub fn prepare(self, semantic_requirement: SemanticRequirement) -> Result<PreparedCandidate> {
        let manifest_path = self.source_root.join("quality/manifest.toml");
        let manifest = load_manifest(&manifest_path, semantic_requirement)?;
        if self.origin == CandidateOrigin::Revision {
            self.require_revision_at_checkout_head()?;
        }
        verify_runner_inputs(
            &self.repository,
            &self.source_root,
            manifest.manifest().runner().inputs(),
        )?;
        if self.origin == CandidateOrigin::Revision {
            self.require_revision_at_checkout_head()?;
        }
        self.verify_unchanged_internal(true)?;
        Ok(PreparedCandidate {
            candidate: self,
            manifest,
        })
    }

    fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    fn require_revision_at_checkout_head(&self) -> Result<()> {
        let head = self.repository.head()?;
        if head.as_deref() != Some(self.candidate_revision.as_str()) {
            bail!(
                "revision candidate {} is not current checkout HEAD ({})",
                self.candidate_revision,
                head.as_deref().unwrap_or("unborn")
            );
        }
        Ok(())
    }

    fn materialize(
        repository: Repository,
        base_revision: String,
        candidate_revision: String,
        candidate_tree: String,
        changed_paths: Vec<PathBuf>,
        origin: CandidateOrigin,
    ) -> Result<Self> {
        // Load every blob and validate complete path/symlink/mode plan before
        // creating candidate filesystem state.
        let plan = load_materialization_plan(&repository, &candidate_tree)?;
        let mut storage_guard = TempRootGuard::create("candidate")?;
        let storage_root = storage_guard.path().to_owned();
        let result = (|| {
            let source_root = storage_root.join("source");
            let scratch_root = storage_root.join("scratch");
            let cache_root = storage_root.join("cache");
            let target_root = storage_root.join("target");
            for directory in [&source_root, &scratch_root, &cache_root, &target_root] {
                fs::create_dir(directory).with_context(|| {
                    format!(
                        "failed creating candidate directory `{}`",
                        directory.display()
                    )
                })?;
            }
            materialize_plan(&source_root, &plan)?;

            let candidate = Self {
                repository,
                base_revision,
                candidate_revision,
                candidate_tree,
                changed_paths,
                source_root,
                scratch_root,
                cache_root,
                target_root,
                storage_root: storage_root.clone(),
                origin,
            };
            candidate.verify_unchanged_internal(false)?;
            make_read_only(&candidate.source_root)?;
            candidate.verify_unchanged_internal(true)?;
            Ok(candidate)
        })();
        if result.is_ok() {
            storage_guard.disarm();
        }
        result
    }

    fn verify_unchanged_internal(&self, require_sealed: bool) -> Result<()> {
        validate_symlinks(&self.source_root)?;
        if require_sealed {
            verify_sealed_permissions(&self.source_root)?;
        }
        let verification_root = create_child_temp(self.storage_root(), "verify")?;
        let result = recompute_tree(&self.repository, &self.source_root, &verification_root);
        let cleanup_result = remove_writable_tree(&verification_root);
        let recomputed = result?;
        cleanup_result.context("failed cleaning candidate verification state")?;
        if recomputed != self.candidate_tree {
            bail!(
                "materialized source differs from bound candidate tree: expected {}, recomputed {}",
                self.candidate_tree,
                recomputed
            );
        }
        Ok(())
    }

    fn cleanup_inner(&mut self) -> Result<()> {
        remove_writable_tree(&self.storage_root).with_context(|| {
            format!(
                "failed cleaning candidate state `{}`",
                self.storage_root.display()
            )
        })
    }
}

impl PreparedCandidate {
    pub fn manifest(&self) -> &ManifestDocument {
        &self.manifest
    }

    pub fn repository(&self) -> &Repository {
        &self.candidate.repository
    }

    pub fn base_revision(&self) -> &str {
        &self.candidate.base_revision
    }

    pub fn candidate_revision(&self) -> &str {
        &self.candidate.candidate_revision
    }

    pub fn candidate_tree(&self) -> &str {
        &self.candidate.candidate_tree
    }

    pub fn changed_paths(&self) -> &[PathBuf] {
        &self.candidate.changed_paths
    }

    pub fn source_root(&self) -> &Path {
        &self.candidate.source_root
    }

    pub fn scratch_root(&self) -> &Path {
        &self.candidate.scratch_root
    }

    pub fn cache_root(&self) -> &Path {
        &self.candidate.cache_root
    }

    pub fn target_root(&self) -> &Path {
        &self.candidate.target_root
    }

    pub fn storage_root(&self) -> &Path {
        &self.candidate.storage_root
    }

    /// Prove exact source namespace, bytes, Git modes, types, and sealed permissions.
    pub fn verify_unchanged(&self) -> Result<()> {
        self.candidate.verify_unchanged_internal(true)
    }

    /// Remove state, consuming descriptor so no post-cleanup path can be observed.
    ///
    /// All child process handles using candidate paths must first be cancelled and
    /// awaited. Top-level interruption orchestration in T008/T010/T012 owns that
    /// ordering before it calls this method. Failure returns ownership for explicit
    /// retry; dropping that error retains best-effort RAII cleanup.
    pub fn cleanup(mut self) -> std::result::Result<(), CandidateCleanupError> {
        match self.candidate.cleanup_inner() {
            Ok(()) => Ok(()),
            Err(error) => Err(CandidateCleanupError {
                candidate: Box::new(self),
                error,
            }),
        }
    }
}

impl Drop for Candidate {
    fn drop(&mut self) {
        let _ = self.cleanup_inner();
    }
}

#[derive(Debug)]
struct MaterializationEntry {
    path: PathBuf,
    kind: MaterializationKind,
}

#[derive(Debug)]
enum MaterializationKind {
    Regular { executable: bool, bytes: Vec<u8> },
    Symlink { target: OsString },
}

fn load_materialization_plan(
    repository: &Repository,
    tree: &str,
) -> Result<Vec<MaterializationEntry>> {
    let output = repository.output(
        [
            OsStr::new("ls-tree"),
            OsStr::new("-r"),
            OsStr::new("-z"),
            OsStr::new("--full-tree"),
            OsStr::new(tree),
        ],
        None,
    )?;
    ensure_success(&output, "git ls-tree for candidate")?;
    let mut plan = Vec::new();
    let mut paths = BTreeSet::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("malformed git ls-tree candidate record")?;
        let header = std::str::from_utf8(&record[..tab])
            .context("candidate tree metadata is not valid UTF-8")?;
        let mut fields = header.split_whitespace();
        let mode = fields.next().context("candidate tree entry has no mode")?;
        let object_type = fields.next().context("candidate tree entry has no type")?;
        let oid = fields
            .next()
            .context("candidate tree entry has no object ID")?;
        if fields.next().is_some() {
            bail!("candidate tree entry has malformed metadata");
        }
        let path_text = std::str::from_utf8(&record[tab + 1..])
            .context("candidate tree path is not valid UTF-8")?;
        let path = PathBuf::from(path_text);
        validate_relative_path(&path)
            .with_context(|| format!("invalid candidate Git path `{path_text}`"))?;
        if !paths.insert(path.clone()) {
            bail!("duplicate candidate Git path `{path_text}`");
        }
        if object_type != "blob" || !matches!(mode, "100644" | "100755" | "120000") {
            bail!(
                "unsupported Git mode/type `{mode} {object_type}` for candidate path `{path_text}`"
            );
        }
        let blob = repository.output(
            [OsStr::new("cat-file"), OsStr::new("blob"), OsStr::new(oid)],
            None,
        )?;
        ensure_success(&blob, "git cat-file candidate blob")?;
        let kind = if mode == "120000" {
            if blob.stdout.contains(&0) {
                bail!("symlink target at `{path_text}` contains NUL");
            }
            let target = OsString::from_vec(blob.stdout);
            validate_symlink_target(&path, Path::new(&target))?;
            MaterializationKind::Symlink { target }
        } else {
            MaterializationKind::Regular {
                executable: mode == "100755",
                bytes: blob.stdout,
            }
        };
        plan.push(MaterializationEntry { path, kind });
    }
    plan.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(plan)
}

fn materialize_plan(root: &Path, plan: &[MaterializationEntry]) -> Result<()> {
    for entry in plan {
        let destination = root.join(&entry.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        match &entry.kind {
            MaterializationKind::Regular { executable, bytes } => {
                let mode = if *executable { 0o755 } else { 0o644 };
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&destination)
                    .with_context(|| format!("failed creating `{}`", destination.display()))?;
                file.write_all(bytes)?;
                fs::set_permissions(&destination, fs::Permissions::from_mode(mode))?;
            }
            MaterializationKind::Symlink { target } => {
                symlink(target, &destination).with_context(|| {
                    format!(
                        "failed creating candidate symlink `{}`",
                        destination.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn snapshot_effective_index_tree(repository: &Repository) -> Result<String> {
    let mut root = TempRootGuard::create("index-snapshot")?;
    let result = snapshot_effective_index_tree_in(repository, root.path(), || {});
    let cleanup = root.cleanup();
    let tree = result?;
    cleanup.context("failed cleaning effective-index snapshot")?;
    Ok(tree)
}

fn snapshot_effective_index_tree_in(
    repository: &Repository,
    root: &Path,
    after_copy: impl FnOnce(),
) -> Result<String> {
    let source = repository.effective_index();
    let snapshot = root.join("index");
    match fs::symlink_metadata(source) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!(
                    "effective Git index is not a regular file: {}",
                    source.display()
                );
            }
            let mut file = fs::File::open(source).with_context(|| {
                format!("failed opening effective Git index `{}`", source.display())
            })?;
            let opened_before = file.metadata()?;
            if !same_identity(&metadata, &opened_before) {
                bail!("effective Git index changed identity while opening snapshot");
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let opened_after = file.metadata()?;
            if !same_version(&opened_before, &opened_after) {
                bail!("effective Git index changed while snapshot was read");
            }
            atomic_write(&snapshot, &bytes)?;
            after_copy();
            let path_after = fs::symlink_metadata(source)
                .context("effective Git index disappeared during snapshot")?;
            if !same_version(&opened_after, &path_after) {
                bail!("effective Git index identity or metadata changed during snapshot");
            }
            let bytes_after =
                fs::read(source).context("failed re-reading effective Git index after snapshot")?;
            if bytes_after != bytes {
                bail!("effective Git index content changed during snapshot");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let environment = GitEnvironment {
                index_file: Some(&snapshot),
                ..GitEnvironment::default()
            };
            let empty = repository.output_with_environment(
                [OsStr::new("read-tree"), OsStr::new("--empty")],
                &environment,
                None,
            )?;
            ensure_success(&empty, "git read-tree --empty for absent index")?;
            after_copy();
            if fs::symlink_metadata(source).is_ok() {
                bail!("effective Git index appeared during empty-index snapshot");
            }
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed inspecting effective Git index `{}`",
                    source.display()
                )
            });
        }
    }
    repository.write_tree_from_index(&snapshot)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let partial = path.with_extension("partial");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&partial, path)?;
    Ok(())
}

fn same_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn same_version(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    Regular { executable: bool, bytes: Vec<u8> },
    Symlink { bytes: Vec<u8> },
}

fn verify_runner_inputs(
    repository: &Repository,
    source_root: &Path,
    inputs: &[PathBuf],
) -> Result<()> {
    for input in inputs {
        validate_relative_path(input).with_context(|| {
            format!(
                "runner input `{}` is not a normal relative path",
                input.display()
            )
        })?;
        reject_symlink_ancestors(source_root, input, "candidate")?;
        reject_symlink_ancestors(repository.worktree_root(), input, "worktree")?;
        let candidate_path = source_root.join(input);
        if fs::symlink_metadata(&candidate_path).is_err() {
            bail!(
                "runner input `{}` is absent from candidate tree",
                input.display()
            );
        }
        let worktree_path = repository.worktree_root().join(input);
        if fs::symlink_metadata(&worktree_path).is_err() {
            bail!(
                "runner input `{}` differs from candidate content: worktree path is absent",
                input.display()
            );
        }
        let candidate_entries = snapshot_path(&candidate_path, input)?;
        let worktree_entries = snapshot_path(&worktree_path, input)?;
        if candidate_entries != worktree_entries {
            bail!(
                "runner input `{}` differs from candidate content, mode, namespace, or type",
                input.display()
            );
        }
    }
    Ok(())
}

fn reject_symlink_ancestors(root: &Path, relative: &Path, side: &str) -> Result<()> {
    let mut current = root.to_owned();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(name) = component else {
            bail!("runner input contains non-normal component");
        };
        if index + 1 == component_count {
            break;
        }
        current.push(name);
        let metadata = fs::symlink_metadata(&current).with_context(|| {
            format!(
                "failed reading {side} runner-input ancestor `{}`",
                current.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            bail!(
                "runner input `{}` has symlink ancestor `{}` in {side}",
                relative.display(),
                current.display()
            );
        }
        if !metadata.file_type().is_dir() {
            bail!(
                "runner input `{}` has non-directory ancestor `{}` in {side}",
                relative.display(),
                current.display()
            );
        }
    }
    Ok(())
}

fn snapshot_path(path: &Path, relative: &Path) -> Result<BTreeMap<PathBuf, SnapshotEntry>> {
    let mut entries = BTreeMap::new();
    collect_snapshot(path, relative, &mut entries)?;
    Ok(entries)
}

fn collect_snapshot(
    path: &Path,
    relative: &Path,
    entries: &mut BTreeMap<PathBuf, SnapshotEntry>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed reading input `{}`", path.display()))?;
    if metadata.file_type().is_dir() {
        entries.insert(relative.to_owned(), SnapshotEntry::Directory);
        let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child.file_name();
            if name.to_str().is_none() {
                bail!(
                    "runner input path is not valid UTF-8 beneath `{}`",
                    relative.display()
                );
            }
            collect_snapshot(&child.path(), &relative.join(name), entries)?;
        }
        return Ok(());
    }
    let entry = entry_snapshot(path, &metadata)?;
    entries.insert(relative.to_owned(), entry);
    Ok(())
}

fn entry_snapshot(path: &Path, metadata: &fs::Metadata) -> Result<SnapshotEntry> {
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path)?;
        return Ok(SnapshotEntry::Symlink {
            bytes: target.as_os_str().as_bytes().to_vec(),
        });
    }
    if !metadata.file_type().is_file() {
        bail!("unsupported candidate file type at `{}`", path.display());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(SnapshotEntry::Regular {
        executable: metadata.mode() & 0o111 != 0,
        bytes,
    })
}

fn recompute_tree(
    repository: &Repository,
    source_root: &Path,
    state_root: &Path,
) -> Result<String> {
    fs::create_dir_all(state_root.join("objects"))?;
    let index = state_root.join("index");
    let objects = state_root.join("objects");
    let alternates = repository.git_common_directory().join("objects");
    let environment = GitEnvironment {
        index_file: Some(&index),
        object_directory: Some(&objects),
        alternate_object_directories: Some(&alternates),
    };
    let empty = repository.output_with_environment(
        [OsStr::new("read-tree"), OsStr::new("--empty")],
        &environment,
        None,
    )?;
    ensure_success(&empty, "git read-tree --empty for verification")?;

    let snapshot = snapshot_directory_contents(source_root)?;
    let mut index_info = Vec::new();
    for (path, entry) in snapshot {
        let (mode, bytes) = match entry {
            SnapshotEntry::Directory => continue,
            SnapshotEntry::Regular { executable, bytes } => {
                (if executable { 0o100755 } else { 0o100644 }, bytes)
            }
            SnapshotEntry::Symlink { bytes } => (0o120000, bytes),
        };
        let hash = repository.output_with_environment(
            [
                OsStr::new("hash-object"),
                OsStr::new("-w"),
                OsStr::new("--stdin"),
            ],
            &environment,
            Some(&bytes),
        )?;
        ensure_success(&hash, "git hash-object for candidate verification")?;
        let oid = utf8_line(&hash.stdout, "verified blob object ID")?;
        index_info.extend_from_slice(format!("{mode:06o} {oid}\t").as_bytes());
        index_info.extend_from_slice(path.as_os_str().as_bytes());
        index_info.push(0);
    }
    let update = repository.output_with_environment(
        [
            OsStr::new("update-index"),
            OsStr::new("-z"),
            OsStr::new("--index-info"),
        ],
        &environment,
        Some(&index_info),
    )?;
    ensure_success(&update, "git update-index for candidate verification")?;
    repository.write_tree_from_index(&index)
}

fn snapshot_directory_contents(root: &Path) -> Result<BTreeMap<PathBuf, SnapshotEntry>> {
    let mut entries = BTreeMap::new();
    let mut children = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child.file_name();
        if name.to_str().is_none() {
            bail!("candidate source path is not valid UTF-8");
        }
        collect_snapshot(&child.path(), Path::new(&name), &mut entries)?;
    }
    Ok(entries)
}

fn validate_symlinks(root: &Path) -> Result<()> {
    fn visit(root: &Path, directory: &Path) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_dir() {
                visit(root, &path)?;
            } else if metadata.file_type().is_symlink() {
                let relative = path.strip_prefix(root)?;
                let target = fs::read_link(&path)?;
                validate_symlink_target(relative, &target)?;
            } else if !metadata.file_type().is_file() {
                bail!("unsupported candidate file type at `{}`", path.display());
            }
        }
        Ok(())
    }
    visit(root, root)
}

fn validate_symlink_target(link_path: &Path, target: &Path) -> Result<()> {
    if target.is_absolute() {
        bail!(
            "absolute symlink target at `{}` is not allowed",
            link_path.display()
        );
    }
    let parent = link_path.parent().unwrap_or_else(|| Path::new(""));
    let mut depth = parent.components().count();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir => {
                bail!(
                    "symlink target at `{}` escapes candidate source",
                    link_path.display()
                )
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "absolute symlink target at `{}` is not allowed",
                    link_path.display()
                )
            }
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("path must be non-empty and relative");
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("path contains non-normal component");
        }
    }
    if path.as_os_str().to_str().is_none() {
        bail!("path is not valid UTF-8");
    }
    Ok(())
}

fn make_read_only(root: &Path) -> Result<()> {
    fn visit(path: &Path, directories: &mut Vec<PathBuf>) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.file_type().is_dir() {
            directories.push(path.to_owned());
            for entry in fs::read_dir(path)? {
                visit(&entry?.path(), directories)?;
            }
        } else if metadata.file_type().is_file() {
            let mode = if metadata.mode() & 0o111 == 0 {
                0o444
            } else {
                0o555
            };
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        } else {
            bail!("unsupported candidate file type at `{}`", path.display());
        }
        Ok(())
    }
    let mut directories = Vec::new();
    visit(root, &mut directories)?;
    for directory in directories.into_iter().rev() {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o555))?;
    }
    Ok(())
}

fn verify_sealed_permissions(root: &Path) -> Result<()> {
    fn visit(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.permissions().mode() & 0o222 != 0 {
            bail!("candidate source path is writable: {}", path.display());
        }
        if metadata.file_type().is_dir() {
            for entry in fs::read_dir(path)? {
                visit(&entry?.path())?;
            }
        } else if !metadata.file_type().is_file() {
            bail!("unsupported candidate file type at `{}`", path.display());
        }
        Ok(())
    }
    visit(root)
}

fn remove_writable_tree(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    // Cooperative local callers stop and await candidate children before cleanup.
    // Component metadata never follows symlinks, and directory chmod uses a
    // no-follow descriptor so static redirects cannot affect external targets.
    fn make_writable(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if metadata.file_type().is_dir() {
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
                .open(path)?;
            directory.set_permissions(fs::Permissions::from_mode(0o700))?;
            for entry in fs::read_dir(path)? {
                make_writable(&entry?.path())?;
            }
        }
        Ok(())
    }
    make_writable(root)?;
    fs::remove_dir_all(root)?;
    Ok(())
}

#[derive(Debug)]
struct TempRootGuard {
    path: Option<PathBuf>,
}

impl TempRootGuard {
    fn create(label: &str) -> Result<Self> {
        for _ in 0..100 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "loop-engine-{label}-{}-{nonce}-{counter}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path: Some(path) }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).context("failed creating candidate temporary root");
                }
            }
        }
        bail!("failed creating unique candidate temporary root")
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("temporary root guard is armed")
    }

    fn cleanup(&mut self) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        remove_writable_tree(path)?;
        self.path = None;
        Ok(())
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TempRootGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn create_child_temp(parent: &Path, label: &str) -> Result<PathBuf> {
    for _ in 0..100 {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{label}-{counter}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("failed creating candidate child state"),
        }
    }
    bail!("failed creating unique candidate child state")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("/usr/bin/git")
            .args(args)
            .current_dir(repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    #[test]
    fn staged_candidate_rejects_head_reset_after_index_tree_derivation() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.email", "candidate@test"]);
        git(&repo, &["config", "user.name", "Candidate Test"]);
        fs::write(repo.join("file"), b"one").unwrap();
        git(&repo, &["add", "file"]);
        git(&repo, &["commit", "-m", "one"]);
        fs::write(repo.join("file"), b"two").unwrap();
        git(&repo, &["add", "file"]);
        git(&repo, &["commit", "-m", "two"]);

        let error = Candidate::staged_with_after_snapshot(&repo, || {
            git(&repo, &["reset", "--hard", "HEAD^"]);
        })
        .unwrap_err();
        assert!(error.to_string().contains("HEAD changed"), "{error:#}");
    }

    #[test]
    fn index_snapshot_rejects_change_after_copy() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let status = std::process::Command::new("/usr/bin/git")
            .args(["init", "-b", "main"])
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(status.success());
        fs::write(repo.join("file"), b"one").unwrap();
        assert!(
            std::process::Command::new("/usr/bin/git")
                .args(["add", "file"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let repository = Repository::resolve(&repo).unwrap();
        let snapshot_root = root.path().join("snapshot");
        fs::create_dir(&snapshot_root).unwrap();
        let changed = AtomicBool::new(false);
        let error = snapshot_effective_index_tree_in(&repository, &snapshot_root, || {
            fs::write(repo.join("other"), b"two").unwrap();
            assert!(
                std::process::Command::new("/usr/bin/git")
                    .args(["add", "other"])
                    .current_dir(&repo)
                    .status()
                    .unwrap()
                    .success()
            );
            changed.store(true, Ordering::SeqCst);
        })
        .unwrap_err();
        assert!(changed.load(Ordering::SeqCst));
        assert!(
            error.to_string().contains("changed during snapshot"),
            "{error:#}"
        );
    }
}
