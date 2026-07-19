//! Versioned git hook installer and local adapters (T027/T028).
//!
//! Local pre-commit runs deterministic checks plus a semantic-judge attempt
//! against the **exact staged index tree**. Pre-push delegates to the exact-
//! commit publication gate. Hooks never rewrite user files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::architecture;
use crate::docs_check;
use crate::publication::{self, PublicationOptions, PushUpdate};
use crate::quality;
use crate::semantic_judge::{self, Disposition, JudgeOptions, JudgeOutcome, Mode};

/// Current version embedded in versioned hook adapters.
pub const HOOK_VERSION: u32 = 1;

/// Repository-relative directory containing versioned hooks.
pub const HOOKS_DIR: &str = ".githooks";

/// Repository-relative path to the versioned pre-commit adapter.
pub const PRE_COMMIT_HOOK_PATH: &str = ".githooks/pre-commit";

/// Repository-relative path to the versioned pre-push adapter.
pub const PRE_PUSH_HOOK_PATH: &str = ".githooks/pre-push";

const VERSION_MARKER_PREFIX: &str = "loop-engine-hook-version:";

/// Options for the local pre-commit adapter.
#[derive(Debug, Clone)]
pub struct PreCommitOptions {
    pub repo_root: PathBuf,
    pub judge_executable: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
    /// Optional Git root used to load immutable foundation seed blobs in tests.
    pub foundation_git_root: Option<PathBuf>,
}

impl PreCommitOptions {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            judge_executable: None,
            timeout_seconds: None,
            foundation_git_root: None,
        }
    }
}

/// Outcome of a local pre-commit attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCommitOutcome {
    pub warnings: Vec<String>,
    pub judge: JudgeOutcome,
}

/// Repository root used by hook commands when no override is supplied.
pub fn default_repository_root() -> PathBuf {
    semantic_judge::default_repository_root()
}

/// Install local hooks by pointing `core.hooksPath` at the versioned `.githooks/` directory.
///
/// This never copies or rewrites hook bodies; the versioned files in the working
/// tree / index remain the source of truth.
pub fn install(repo_root: Option<&Path>) -> Result<()> {
    let root = resolve_root(repo_root)?;
    ensure_versioned_pre_commit(&root)?;
    ensure_versioned_pre_push_if_present(&root)?;
    git_run(&root, &["config", "core.hooksPath", HOOKS_DIR])?;
    Ok(())
}

/// Verify that hooks are installed and the versioned pre-commit marker matches [`HOOK_VERSION`].
pub fn verify(repo_root: Option<&Path>) -> Result<()> {
    let root = resolve_root(repo_root)?;
    ensure_versioned_pre_commit(&root)?;

    let hooks_path =
        git_output_trimmed(&root, &["config", "--get", "core.hooksPath"]).map_err(|_| {
            anyhow::anyhow!("core.hooksPath is not set; run `cargo run -p xtask -- hooks install`")
        })?;

    if !hooks_path_matches(&root, &hooks_path) {
        bail!(
            "core.hooksPath is `{hooks_path}`, expected `{HOOKS_DIR}` (or an absolute path to that directory)"
        );
    }

    let version = read_pre_commit_version(&root)?;
    if version != HOOK_VERSION {
        bail!(
            "hook-version mismatch: `{PRE_COMMIT_HOOK_PATH}` declares version {version}, expected {HOOK_VERSION}"
        );
    }

    if root.join(PRE_PUSH_HOOK_PATH).is_file() {
        let push_version = read_pre_push_version(&root)?;
        if push_version != HOOK_VERSION {
            bail!(
                "hook-version mismatch: `{PRE_PUSH_HOOK_PATH}` declares version {push_version}, expected {HOOK_VERSION}"
            );
        }
    }

    Ok(())
}

/// Run the local pre-commit gate against the exact staged index tree.
///
/// Current manifest-driven deterministic checks execute against a temporary
/// materialization of `git write-tree` / index contents. Before the quality manifest
/// exists, documentation and architecture form the compatibility baseline. Semantic
/// judgment uses the staged-index request builder. Neither path reads unstaged
/// working-tree files or rewrites user content.
pub fn pre_commit(options: &PreCommitOptions) -> Result<PreCommitOutcome> {
    let root = resolve_root(Some(&options.repo_root))?;
    git_run(&root, &["diff", "--cached", "--check", "HEAD"])
        .context("git diff --cached --check failed for exact staged candidate")?;
    let staged_revision = git_output_trimmed(&root, &["write-tree"])?;
    let staged = materialize_staged_tree(&root)?;
    let quality_manifest_path = staged.path.join(quality::MANIFEST_PATH);

    let quality_evidence = if quality_manifest_path.is_file() {
        let manifest = quality::load_manifest(&staged.path, Some(&quality_manifest_path))?;
        let report = quality::run_manifest(
            &staged.path,
            &manifest,
            &quality_manifest_path,
            &staged_revision,
        )
        .with_context(|| {
            format!(
                "manifest-driven quality checks failed for exact staged tree {}",
                staged_revision
            )
        })?;
        report.deterministic_evidence()
    } else {
        docs_check::run(Some(&staged.path)).with_context(|| {
            format!(
                "deterministic docs-check failed for exact staged tree at {}",
                staged.path.display()
            )
        })?;

        let manifest = staged.path.join("Cargo.toml");
        architecture::run(Some(&manifest)).with_context(|| {
            format!(
                "deterministic architecture check failed for exact staged tree at {}",
                staged.path.display()
            )
        })?;
        Vec::new()
    };

    // Drop the staged materialization before semantic judgment so temporary files
    // cannot be mistaken for working-tree inputs.
    drop(staged);

    let mut judge_options = JudgeOptions::new(&root, Mode::Local);
    judge_options.executable = options.judge_executable.clone();
    judge_options.timeout_seconds = options.timeout_seconds;
    judge_options.foundation_git_root = options.foundation_git_root.clone();
    judge_options.extra_deterministic_evidence = quality_evidence;

    let judge = semantic_judge::judge_staged(&judge_options)?;
    let mut warnings = Vec::new();
    if let Some(warning) = &judge.warning {
        eprintln!("warning: {warning}");
        warnings.push(warning.clone());
    }

    match judge.disposition {
        Disposition::Allow | Disposition::WarnAllow => Ok(PreCommitOutcome { warnings, judge }),
        Disposition::Block => bail!(
            "semantic judge blocked local commit (verdict={:?}): {}",
            judge.response.verdict,
            judge.response.message
        ),
    }
}

/// CLI entrypoint for hook subcommands.
pub fn run_install(repo_root: Option<&Path>) -> Result<()> {
    install(repo_root)?;
    println!("installed hooks: core.hooksPath={HOOKS_DIR}");
    Ok(())
}

/// CLI entrypoint for hook verification.
pub fn run_verify(repo_root: Option<&Path>) -> Result<()> {
    verify(repo_root)?;
    println!("hooks verify ok (version {HOOK_VERSION})");
    Ok(())
}

/// CLI entrypoint for the local pre-commit adapter.
pub fn run_pre_commit(
    repo_root: Option<&Path>,
    judge_executable: Option<&Path>,
    timeout_seconds: Option<u64>,
) -> Result<()> {
    let root = resolve_root(repo_root)?;
    let mut options = PreCommitOptions::new(root);
    options.judge_executable = judge_executable.map(Path::to_path_buf);
    options.timeout_seconds = timeout_seconds;
    let _outcome = pre_commit(&options)?;
    Ok(())
}

/// Run the exact-commit pre-push / publication adapter.
///
/// Reads git pre-push remote-update lines from stdin and delegates to the
/// canonical publication gate. Never rewrites the user working tree.
pub fn run_pre_push(
    repo_root: Option<&Path>,
    remote_name: Option<&str>,
    remote_url: Option<&str>,
    judge_executable: Option<&Path>,
    timeout_seconds: Option<u64>,
) -> Result<()> {
    let root = resolve_root(repo_root)?;
    let updates = publication::read_push_updates_from_stdin()?;
    pre_push(
        &root,
        &updates,
        remote_name,
        remote_url,
        judge_executable,
        timeout_seconds,
    )?;
    Ok(())
}

/// Canonical pre-push entry used by tests and the thin hook adapter.
pub fn pre_push(
    repo_root: &Path,
    updates: &[PushUpdate],
    remote_name: Option<&str>,
    remote_url: Option<&str>,
    judge_executable: Option<&Path>,
    timeout_seconds: Option<u64>,
) -> Result<Vec<publication::PublicationOutcome>> {
    let mut options = PublicationOptions::new(repo_root);
    options.judge_executable = judge_executable.map(Path::to_path_buf);
    options.remote_name = remote_name.map(str::to_owned);
    options.remote_url = remote_url.map(str::to_owned);
    options.timeout_seconds = timeout_seconds;
    options.foundation_git_root = None;
    publication::publish_updates(&options, updates)
}

/// Parse the version marker from a pre-commit hook body.
pub fn parse_hook_version(contents: &str) -> Result<u32> {
    for line in contents.lines() {
        let trimmed = line.trim().trim_start_matches('#').trim();
        if let Some(rest) = trimmed.strip_prefix(VERSION_MARKER_PREFIX) {
            let value = rest.trim();
            return value.parse::<u32>().with_context(|| {
                format!("invalid hook version marker value `{value}` in pre-commit hook")
            });
        }
    }
    bail!("missing `{VERSION_MARKER_PREFIX}` marker in pre-commit hook");
}

fn resolve_root(repo_root: Option<&Path>) -> Result<PathBuf> {
    let path = match repo_root {
        Some(path) => path.to_path_buf(),
        None => find_repository_root().unwrap_or_else(default_repository_root),
    };
    if !path.is_dir() {
        bail!("repository root does not exist: {}", path.display());
    }
    Ok(path)
}

fn find_repository_root() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        if current.join(".git").exists() && current.join("Cargo.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn ensure_versioned_pre_commit(repo_root: &Path) -> Result<()> {
    let path = repo_root.join(PRE_COMMIT_HOOK_PATH);
    if !path.is_file() {
        bail!("missing versioned hook at `{}`", path.display());
    }
    Ok(())
}

fn ensure_versioned_pre_push_if_present(repo_root: &Path) -> Result<()> {
    let path = repo_root.join(PRE_PUSH_HOOK_PATH);
    if path.is_file() {
        let version = read_pre_push_version(repo_root)?;
        if version != HOOK_VERSION {
            bail!(
                "hook-version mismatch: `{PRE_PUSH_HOOK_PATH}` declares version {version}, expected {HOOK_VERSION}"
            );
        }
    }
    Ok(())
}

fn read_pre_commit_version(repo_root: &Path) -> Result<u32> {
    let path = repo_root.join(PRE_COMMIT_HOOK_PATH);
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    parse_hook_version(&contents)
}

fn read_pre_push_version(repo_root: &Path) -> Result<u32> {
    let path = repo_root.join(PRE_PUSH_HOOK_PATH);
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    parse_hook_version(&contents)
}

fn hooks_path_matches(repo_root: &Path, configured: &str) -> bool {
    if configured == HOOKS_DIR || configured == format!("./{HOOKS_DIR}") {
        return true;
    }
    let configured_path = PathBuf::from(configured);
    let expected = repo_root.join(HOOKS_DIR);
    if configured_path == expected {
        return true;
    }
    match (configured_path.canonicalize(), expected.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Temporary materialization of the exact staged index tree.
struct StagedTree {
    path: PathBuf,
}

impl Drop for StagedTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn materialize_staged_tree(repo_root: &Path) -> Result<StagedTree> {
    let path = create_temp_dir("exact-staged")?;
    // checkout-index writes only under --prefix and does not modify the working tree.
    let prefix = format!("{}/", path.display());
    git_run(repo_root, &["checkout-index", "--all", "--prefix", &prefix]).with_context(|| {
        format!(
            "failed to materialize exact staged tree into {}",
            path.display()
        )
    })?;
    // Candidate tests may need immutable historical blobs (notably the consumed
    // foundation rubric seed). A gitfile exposes the source repository's object
    // database while all candidate file reads still resolve inside this exact
    // index materialization.
    let git_dir = git_output_trimmed(repo_root, &["rev-parse", "--absolute-git-dir"])?;
    fs::write(path.join(".git"), format!("gitdir: {git_dir}\n"))
        .context("failed linking exact staged tree to source Git object database")?;
    Ok(StagedTree { path })
}

fn create_temp_dir(label: &str) -> Result<PathBuf> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "loop-engine-{label}-{}-{nanos}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create temporary directory {}", path.display()))?;
    Ok(path)
}

fn git_run(repo_root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    Ok(())
}

fn git_output_trimmed(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("git stdout is not UTF-8")?;
    Ok(stdout.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hook_version_reads_marker() {
        let body = "#!/usr/bin/env bash\n# loop-engine-hook-version: 1\nset -e\n";
        assert_eq!(parse_hook_version(body).unwrap(), 1);
    }

    #[test]
    fn parse_hook_version_rejects_missing_marker() {
        let error = parse_hook_version("#!/usr/bin/env bash\n").unwrap_err();
        assert!(error.to_string().contains("missing"));
    }
}
