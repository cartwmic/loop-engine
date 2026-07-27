use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};

pub const GIT_PROGRAM: &str = "/usr/bin/git";
pub(crate) const PRIVATE_STAGED_INDEX_ENVIRONMENT: &str = "LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE";

const SCRUBBED_GIT_ENVIRONMENT: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_QUARANTINE_PATH",
    "GIT_NAMESPACE",
    "GIT_SHALLOW_FILE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_ATTR_NOSYSTEM",
    "GIT_OPTIONAL_LOCKS",
    "GIT_GRAFT_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_NO_REPLACE_OBJECTS",
];

/// Explicit Git state allowed back into a scrubbed plumbing invocation.
#[derive(Debug, Default)]
pub(crate) struct GitEnvironment<'a> {
    pub index_file: Option<&'a Path>,
    pub object_directory: Option<&'a Path>,
    pub alternate_object_directories: Option<&'a Path>,
}

/// Authoritative paths for one active Git repository/worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    worktree_root: PathBuf,
    git_directory: PathBuf,
    git_common_directory: PathBuf,
    effective_index: PathBuf,
}

impl Repository {
    /// Resolve repository paths using Git rather than filesystem assumptions.
    pub fn resolve(path: &Path) -> Result<Self> {
        let index = std::env::var_os("GIT_INDEX_FILE").map(PathBuf::from);
        Self::resolve_with_index(path, index.as_deref())
    }

    /// Resolve repository paths with a typed effective-index override.
    pub(crate) fn resolve_with_index(path: &Path, index: Option<&Path>) -> Result<Self> {
        let invocation_directory =
            std::env::current_dir().context("failed to resolve Git invocation directory")?;
        let starting_path = path
            .canonicalize()
            .with_context(|| format!("failed to resolve repository path `{}`", path.display()))?;
        if !starting_path.is_dir() {
            bail!(
                "repository start path is not a directory: {}",
                starting_path.display()
            );
        }

        let worktree_root = absolute_path_output(
            &starting_path,
            [OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
        )
        .context("failed to resolve Git worktree root")?;
        let git_directory = absolute_path_output(
            &starting_path,
            [OsStr::new("rev-parse"), OsStr::new("--absolute-git-dir")],
        )
        .context("failed to resolve absolute Git directory")?;
        let common_output = git_output(
            &starting_path,
            None,
            [OsStr::new("rev-parse"), OsStr::new("--git-common-dir")],
            &GitEnvironment::default(),
            None,
        )?;
        ensure_success(&common_output, "git rev-parse --git-common-dir")?;
        let common_path = output_path(&common_output.stdout, "Git common directory")?;
        // Git documents relative --git-common-dir output relative to command cwd.
        let git_common_directory = if common_path.is_absolute() {
            common_path
        } else {
            starting_path.join(common_path)
        }
        .canonicalize()
        .context("failed to resolve absolute Git common directory")?;

        let git_directory = git_directory
            .canonicalize()
            .context("failed to canonicalize Git directory")?;
        let effective_index = match index {
            Some(index) if index.as_os_str().is_empty() => {
                bail!("effective Git index path is empty")
            }
            Some(index) if index.is_absolute() => index.to_owned(),
            // Git resolves GIT_INDEX_FILE relative to process invocation cwd.
            Some(index) => invocation_directory.join(index),
            None => git_directory.join("index"),
        };

        Ok(Self {
            worktree_root: worktree_root
                .canonicalize()
                .context("failed to canonicalize Git worktree root")?,
            git_directory,
            git_common_directory,
            effective_index,
        })
    }

    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub fn git_directory(&self) -> &Path {
        &self.git_directory
    }

    pub fn git_common_directory(&self) -> &Path {
        &self.git_common_directory
    }

    pub fn effective_index(&self) -> &Path {
        &self.effective_index
    }

    /// Resolve `HEAD` to a commit, returning `None` only for an unborn branch.
    pub fn head(&self) -> Result<Option<String>> {
        let output = self.output(
            [
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--end-of-options"),
                OsStr::new("HEAD^{object}"),
            ],
            None,
        )?;
        if output.status.success() {
            let oid = utf8_line(&output.stdout, "HEAD object ID")?;
            self.require_object_type(&oid, "commit", "HEAD")?;
            return Ok(Some(oid));
        }

        let symbolic = self.output(
            [
                OsStr::new("symbolic-ref"),
                OsStr::new("--quiet"),
                OsStr::new("--no-recurse"),
                OsStr::new("HEAD"),
            ],
            None,
        )?;
        if !symbolic.status.success() {
            ensure_success(&output, "git rev-parse --verify HEAD")?;
            unreachable!()
        }
        let target = utf8_line(&symbolic.stdout, "HEAD symbolic ref")?;
        if !target.starts_with("refs/heads/") {
            bail!("HEAD symbolic ref `{target}` is not a local branch");
        }
        let format = self.output([OsStr::new("check-ref-format"), OsStr::new(&target)], None)?;
        ensure_success(&format, "git check-ref-format for HEAD branch")?;

        let reference = self.output(
            [
                OsStr::new("show-ref"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(&target),
            ],
            None,
        )?;
        if reference.status.success() {
            ensure_success(&output, "git rev-parse --verify HEAD")?;
            unreachable!()
        }
        if reference.status.code() != Some(1) {
            ensure_success(&reference, "git show-ref --verify HEAD branch")?;
            unreachable!()
        }

        let loose_reference = self.git_common_directory.join(&target);
        match std::fs::symlink_metadata(&loose_reference) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Ok(_) => {
                bail!("HEAD branch ref `{target}` exists but does not resolve to a valid commit")
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to inspect HEAD branch ref `{}`",
                    loose_reference.display()
                )
            }),
        }
    }

    /// Ensure Git's hash-algorithm-specific empty tree exists and return its ID.
    pub fn empty_tree(&self) -> Result<String> {
        let output = self.output([OsStr::new("mktree")], Some(&[]))?;
        ensure_success(&output, "git mktree")?;
        utf8_line(&output.stdout, "empty tree object ID")
    }

    /// Write a tree using only an explicitly supplied external index.
    pub(crate) fn write_tree_from_index(&self, index: &Path) -> Result<String> {
        let environment = GitEnvironment {
            index_file: Some(index),
            ..GitEnvironment::default()
        };
        let output =
            self.output_with_environment([OsStr::new("write-tree")], &environment, None)?;
        ensure_success(&output, "git write-tree against index snapshot")?;
        utf8_line(&output.stdout, "index tree object ID")
    }

    /// Resolve revision option-safely and require the supplied object itself be a commit.
    pub fn resolve_commit(&self, revision: &OsStr) -> Result<String> {
        let mut object_expression = revision.to_os_string();
        object_expression.push("^{object}");
        let output = self.output(
            [
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--end-of-options"),
                object_expression.as_os_str(),
            ],
            None,
        )?;
        ensure_success(&output, "git rev-parse commit")?;
        let oid = utf8_line(&output.stdout, "commit object ID")?;
        self.require_object_type(&oid, "commit", &revision.to_string_lossy())?;
        Ok(oid)
    }

    pub fn resolve_tree(&self, commit_oid: &str) -> Result<String> {
        let expression = format!("{commit_oid}^{{tree}}");
        let output = self.output(
            [
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--end-of-options"),
                OsStr::new(&expression),
            ],
            None,
        )?;
        ensure_success(&output, "git rev-parse commit tree")?;
        utf8_line(&output.stdout, "tree object ID")
    }

    /// Return sorted, distinct UTF-8 report paths changed between exact objects.
    pub fn changed_paths(&self, base: &str, candidate: &str) -> Result<Vec<PathBuf>> {
        let output = self.output(
            [
                OsStr::new("diff"),
                OsStr::new("--name-only"),
                OsStr::new("--no-renames"),
                OsStr::new("-z"),
                OsStr::new(base),
                OsStr::new(candidate),
                OsStr::new("--"),
            ],
            None,
        )?;
        ensure_success(&output, "git diff --name-only")?;
        parse_nul_paths(&output.stdout, "changed")
    }

    pub(crate) fn output<'a>(
        &self,
        args: impl IntoIterator<Item = &'a OsStr>,
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        self.output_with_environment(args, &GitEnvironment::default(), stdin)
    }

    pub(crate) fn output_with_environment<'a>(
        &self,
        args: impl IntoIterator<Item = &'a OsStr>,
        environment: &GitEnvironment<'_>,
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        git_output(
            &self.worktree_root,
            Some(&self.git_directory),
            args,
            environment,
            stdin,
        )
    }

    fn require_object_type(&self, oid: &str, expected: &str, supplied: &str) -> Result<()> {
        let output = self.output(
            [OsStr::new("cat-file"), OsStr::new("-t"), OsStr::new(oid)],
            None,
        )?;
        ensure_success(&output, "git cat-file -t")?;
        let actual = utf8_line(&output.stdout, "Git object type")?;
        if actual != expected {
            bail!("supplied revision `{supplied}` is a {actual} object, not a {expected}");
        }
        Ok(())
    }
}

pub(crate) fn ensure_success(output: &Output, operation: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{operation} failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim_end()
    )
}

pub(crate) fn utf8_line(bytes: &[u8], label: &str) -> Result<String> {
    let value = std::str::from_utf8(bytes)
        .with_context(|| format!("{label} is not valid UTF-8"))?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        bail!("{label} is empty");
    }
    Ok(value.to_owned())
}

fn output_path(bytes: &[u8], label: &str) -> Result<PathBuf> {
    let mut value = bytes;
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
    {
        value = &value[..value.len() - 1];
    }
    if value.is_empty() {
        bail!("{label} is empty");
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(value.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(
            std::str::from_utf8(value).with_context(|| format!("{label} is not valid UTF-8"))?,
        ))
    }
}

fn parse_nul_paths(bytes: &[u8], label: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for raw in bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let path = std::str::from_utf8(raw)
            .with_context(|| format!("{label} Git path is not valid UTF-8"))?;
        paths.push(PathBuf::from(path));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn absolute_path_output<'a>(
    cwd: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> Result<PathBuf> {
    let output = git_output(cwd, None, args, &GitEnvironment::default(), None)?;
    ensure_success(&output, "git path resolution")?;
    let path = output_path(&output.stdout, "Git path")?;
    if !path.is_absolute() {
        bail!("Git returned non-absolute path `{}`", path.display());
    }
    Ok(path)
}

fn git_output<'a>(
    cwd: &Path,
    git_directory: Option<&Path>,
    args: impl IntoIterator<Item = &'a OsStr>,
    environment: &GitEnvironment<'_>,
    stdin: Option<&[u8]>,
) -> Result<Output> {
    let mut command = Command::new(GIT_PROGRAM);
    for name in SCRUBBED_GIT_ENVIRONMENT {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os().filter(|(name, _)| {
        let name = name.to_string_lossy();
        name.starts_with("GIT_CONFIG_KEY_") || name.starts_with("GIT_CONFIG_VALUE_")
    }) {
        command.env_remove(name);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .arg("--no-replace-objects");
    if let Some(git_directory) = git_directory {
        command.arg("--git-dir").arg(git_directory);
    }
    if let Some(index) = environment.index_file {
        command.env("GIT_INDEX_FILE", index);
    }
    if let Some(objects) = environment.object_directory {
        command.env("GIT_OBJECT_DIRECTORY", objects);
    }
    if let Some(alternates) = environment.alternate_object_directories {
        command.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates);
    }
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().context("failed to spawn /usr/bin/git")?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .context("Git stdin was unavailable")?
            .write_all(bytes)
            .context("failed writing Git stdin")?;
    }
    child
        .wait_with_output()
        .context("failed waiting for /usr/bin/git")
}
