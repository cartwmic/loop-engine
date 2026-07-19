//! Versioned git hook installer and local adapters (T027/T028, repaired by R002/R003).
//!
//! Local pre-commit runs bounded deterministic checks against the **exact staged
//! index tree**. Semantic judgment is an explicit command, not default commit
//! latency. Pre-push delegates to the aggregate publication gate. Hooks never
//! rewrite user files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::architecture;
use crate::docs_check;
use crate::publication::{self, PublicationOptions, PushUpdate};
use crate::semantic_judge;

/// Current version embedded in versioned hook adapters.
pub const HOOK_VERSION: u32 = 2;

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
}

impl PreCommitOptions {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }
}

/// Outcome of the bounded local pre-commit checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCommitOutcome {
    pub checks: Vec<&'static str>,
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

/// Run the bounded local pre-commit gate against the exact staged index tree.
///
/// The default path deliberately excludes the quality manifest, tests, Clippy,
/// dependency/advisory scans, and semantic judgment. Those belong to aggregate
/// publication. This path never reads unstaged working-tree content or rewrites it.
pub fn pre_commit(options: &PreCommitOptions) -> Result<PreCommitOutcome> {
    let root = resolve_root(Some(&options.repo_root))?;
    eprintln!("pre-commit: staged diff check");
    git_run(&root, &["diff", "--cached", "--check", "HEAD"])
        .context("git diff --cached --check failed for exact staged candidate")?;

    let staged_revision = git_output_trimmed(&root, &["write-tree"])?;
    let staged = materialize_staged_tree(&root)?;

    eprintln!("pre-commit: documentation check");
    docs_check::run(Some(&staged.path)).with_context(|| {
        format!(
            "deterministic docs-check failed for exact staged tree {staged_revision} at {}",
            staged.path.display()
        )
    })?;

    eprintln!("pre-commit: architecture check");
    let manifest = staged.path.join("Cargo.toml");
    architecture::run(Some(&manifest)).with_context(|| {
        format!(
            "deterministic architecture check failed for exact staged tree {staged_revision} at {}",
            staged.path.display()
        )
    })?;

    eprintln!("pre-commit: formatting check");
    run_cargo_fmt(&staged.path)
        .with_context(|| format!("cargo fmt failed for exact staged tree {staged_revision}"))?;

    Ok(PreCommitOutcome {
        checks: vec!["diff-check", "docs-check", "architecture", "fmt"],
    })
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
pub fn run_pre_commit(repo_root: Option<&Path>) -> Result<()> {
    let root = resolve_root(repo_root)?;
    let options = PreCommitOptions::new(root);
    let _outcome = pre_commit(&options)?;
    Ok(())
}

/// Run the aggregate base-to-head pre-push / publication adapter.
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
    options.publication_migration_rubric =
        std::env::var_os("LOOP_ENGINE_OWNER_MIGRATION_RUBRIC").map(PathBuf::from);
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

fn run_cargo_fmt(check_root: &Path) -> Result<()> {
    let output = Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(check_root)
        .output()
        .context("failed to execute cargo fmt --all --check")?;
    if !output.status.success() {
        bail!(
            "cargo fmt --all --check failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
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
