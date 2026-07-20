//! Incremental currently-implemented quality manifest (T028).
//!
//! The manifest at `quality/manifest.toml` lists only checks that exist today.
//! Later tasks extend the set; T195 freezes the final canonical gate.
//! Deterministic quality checks remain separate from semantic judgment.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::architecture;
use crate::dependencies;
use crate::docs_check;
use crate::operation_coverage::{self, CoverageMode};
use crate::semantic_judge;

/// Repository-relative path to the incremental quality manifest.
pub const MANIFEST_PATH: &str = "quality/manifest.toml";
const QUALITY_COMMAND_UID_ENV: &str = "LOOP_ENGINE_QUALITY_COMMAND_UID";

/// Supported runners for currently-implemented checks.
pub const RUNNER_DOCS_CHECK: &str = "docs-check";
pub const RUNNER_ARCHITECTURE: &str = "architecture";
pub const RUNNER_CARGO_CHECK: &str = "cargo-check";
pub const RUNNER_CARGO_TEST: &str = "cargo-test";
pub const RUNNER_CARGO_DOC: &str = "cargo-doc";
pub const RUNNER_CARGO_FMT: &str = "cargo-fmt";
pub const RUNNER_CARGO_CLIPPY: &str = "cargo-clippy";
pub const RUNNER_DEPENDENCIES: &str = "dependencies";
pub const RUNNER_OPERATION_COVERAGE: &str = "operation-coverage";
pub const RUNNER_CARGO_DENY: &str = "cargo-deny";

/// Exact command output captured for semantic-judge deterministic evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEvidence {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub candidate_revision: String,
}

impl CommandEvidence {
    /// Serialize to the semantic-judge deterministic-evidence object shape.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "command": self.command,
            "exit_code": self.exit_code,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "candidate_revision": self.candidate_revision,
        })
    }
}

/// One currently-implemented quality check from the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityCheck {
    pub id: String,
    pub runner: String,
}

/// Parsed incremental quality manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityManifest {
    pub schema_version: u32,
    pub checks: Vec<QualityCheck>,
}

/// Per-check execution result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub id: String,
    pub runner: String,
    pub ok: bool,
    pub message: String,
    pub evidence: CommandEvidence,
}

/// Aggregate report for a manifest run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityReport {
    pub manifest_path: PathBuf,
    pub candidate_revision: String,
    pub checks: Vec<CheckOutcome>,
}

impl QualityReport {
    /// True when every check succeeded.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.ok)
    }

    /// Deterministic evidence for every executed quality command.
    pub fn deterministic_evidence(&self) -> Vec<Value> {
        self.checks
            .iter()
            .map(|check| check.evidence.to_json())
            .collect()
    }
}

/// Load and parse `quality/manifest.toml` from `repo_root` (or an override path).
pub fn load_manifest(repo_root: &Path, manifest_path: Option<&Path>) -> Result<QualityManifest> {
    let path = match manifest_path {
        Some(path) => path.to_path_buf(),
        None => repo_root.join(MANIFEST_PATH),
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read quality manifest at {}", path.display()))?;
    parse_manifest(&text).with_context(|| format!("invalid quality manifest at {}", path.display()))
}

/// Load the quality manifest committed at `revision`, when present.
pub fn load_manifest_at_revision(
    repo_root: &Path,
    revision: &str,
) -> Result<Option<QualityManifest>> {
    match git_show_revision(repo_root, revision, MANIFEST_PATH)? {
        None => Ok(None),
        Some(text) => parse_manifest(&text)
            .with_context(|| {
                format!("invalid quality manifest at {MANIFEST_PATH} in revision {revision}")
            })
            .map(Some),
    }
}

/// Reject manifest regressions: parent checks cannot be removed, renamed, or weakened.
pub fn enforce_manifest_monotonic_evolution(
    parent_manifest: Option<&QualityManifest>,
    candidate_manifest: &QualityManifest,
) -> Result<()> {
    let Some(parent) = parent_manifest else {
        return Ok(());
    };

    let candidate_by_id: HashMap<&str, &QualityCheck> = candidate_manifest
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();

    for parent_check in &parent.checks {
        let Some(candidate_check) = candidate_by_id.get(parent_check.id.as_str()) else {
            bail!(
                "quality manifest regression: candidate removed check `{}` present in parent manifest",
                parent_check.id
            );
        };
        if candidate_check.runner != parent_check.runner {
            bail!(
                "quality manifest regression: candidate changed runner for check `{}` from `{}` to `{}` (weakening or rename forbidden)",
                parent_check.id,
                parent_check.runner,
                candidate_check.runner
            );
        }
    }

    Ok(())
}

/// Parse the constrained incremental quality manifest TOML subset.
pub fn parse_manifest(text: &str) -> Result<QualityManifest> {
    let mut schema_version: Option<u32> = None;
    let mut checks = Vec::new();
    let mut current: Option<(Option<String>, Option<String>)> = None;

    let flush = |current: &mut Option<(Option<String>, Option<String>)>,
                 checks: &mut Vec<QualityCheck>|
     -> Result<()> {
        if let Some((id, runner)) = current.take() {
            let id = id.context("quality check is missing `id`")?;
            let runner = runner.context("quality check is missing `runner`")?;
            if id.is_empty() || runner.is_empty() {
                bail!("quality check id/runner must be non-empty");
            }
            checks.push(QualityCheck { id, runner });
        }
        Ok(())
    };

    for raw_line in text.lines() {
        let owned = strip_comment(raw_line);
        let line = owned.trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[checks]]" {
            flush(&mut current, &mut checks)?;
            current = Some((None, None));
            continue;
        }
        if let Some(rest) = line.strip_prefix("schema_version") {
            let value = parse_assignment_value(rest, "schema_version")?;
            let parsed = value
                .parse::<u32>()
                .with_context(|| format!("invalid schema_version `{value}`"))?;
            schema_version = Some(parsed);
            continue;
        }
        if let Some((id, runner)) = current.as_mut() {
            if let Some(rest) = line.strip_prefix("id") {
                *id = Some(parse_quoted_assignment(rest, "id")?);
                continue;
            }
            if let Some(rest) = line.strip_prefix("runner") {
                *runner = Some(parse_quoted_assignment(rest, "runner")?);
                continue;
            }
            bail!("unsupported key inside [[checks]] section: {line}");
        }
        bail!("unsupported top-level quality manifest line: {line}");
    }
    flush(&mut current, &mut checks)?;

    let schema_version =
        schema_version.context("quality manifest is missing required `schema_version`")?;
    if schema_version != 1 {
        bail!("unsupported quality manifest schema_version {schema_version}; expected 1");
    }
    if checks.is_empty() {
        bail!("quality manifest must declare at least one [[checks]] entry");
    }

    let mut seen = std::collections::BTreeSet::new();
    for check in &checks {
        if !seen.insert(check.id.clone()) {
            bail!("duplicate quality check id `{}`", check.id);
        }
        validate_runner(&check.runner)?;
    }

    Ok(QualityManifest {
        schema_version,
        checks,
    })
}

/// Run every currently-implemented check against `check_root`.
///
/// Fail-closed: unknown runners and any non-successful check return [`Err`].
pub fn run_manifest(
    check_root: &Path,
    manifest: &QualityManifest,
    manifest_path: &Path,
    candidate_revision: &str,
) -> Result<QualityReport> {
    if !check_root.is_dir() {
        bail!(
            "quality check root does not exist: {}",
            check_root.display()
        );
    }

    let mut outcomes = Vec::with_capacity(manifest.checks.len());
    for check in &manifest.checks {
        enforce_detached_source_clean(check_root, &format!("before `{}`", check.id))?;
        let check_result = run_check(check_root, check, candidate_revision);
        let source_result =
            enforce_detached_source_clean(check_root, &format!("after `{}`", check.id));
        if let Err(source_error) = source_result {
            let check_detail = check_result
                .err()
                .map(|error| format!("; check also failed: {error:#}"))
                .unwrap_or_default();
            bail!("{source_error:#}{check_detail}");
        }
        match check_result {
            Ok((message, evidence)) => outcomes.push(CheckOutcome {
                id: check.id.clone(),
                runner: check.runner.clone(),
                ok: true,
                message,
                evidence,
            }),
            Err(error) => {
                let evidence = error
                    .downcast_ref::<CheckFailure>()
                    .map(|failure| failure.evidence.clone())
                    .unwrap_or_else(|| CommandEvidence {
                        command: format!(
                            "quality check `{}` (runner `{}`)",
                            check.id, check.runner
                        ),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("{error:#}"),
                        candidate_revision: candidate_revision.to_owned(),
                    });
                outcomes.push(CheckOutcome {
                    id: check.id.clone(),
                    runner: check.runner.clone(),
                    ok: false,
                    message: format!("{error:#}"),
                    evidence,
                });
                bail!(
                    "quality check `{}` (runner `{}`) failed for {}: {error:#}",
                    check.id,
                    check.runner,
                    check_root.display()
                );
            }
        }
    }

    Ok(QualityReport {
        manifest_path: manifest_path.to_path_buf(),
        candidate_revision: candidate_revision.to_owned(),
        checks: outcomes,
    })
}

fn enforce_detached_source_clean(check_root: &Path, phase: &str) -> Result<()> {
    // Revision quality uses a linked-worktree `.git` file. Ordinary `--root .`
    // quality may intentionally run with owner changes and is not publication evidence.
    if !check_root.join(".git").is_file() {
        return Ok(());
    }
    let output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignored=matching",
        ])
        .current_dir(check_root)
        .output()
        .with_context(|| format!("failed checking detached source purity {phase}"))?;
    if !output.status.success() {
        bail!(
            "failed checking detached source purity {phase}: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let status =
        String::from_utf8(output.stdout).context("detached source purity status was not UTF-8")?;
    let violations: Vec<&str> = status
        .lines()
        .filter(|line| {
            let path = line.strip_prefix("!! ").unwrap_or_default();
            let cargo_target =
                path == "target/" || path.starts_with("target/") || path.contains("/target/");
            !(line.starts_with("!! ") && cargo_target)
        })
        .collect();
    if !violations.is_empty() {
        bail!(
            "detached candidate source changed {phase}; quality evidence is not revision-bound:\n{}",
            violations.join("\n")
        );
    }
    Ok(())
}

/// Load the manifest from `repo_root` (or override) and run it against `check_root`.
///
/// When `revision` is set, checks run against a temporary detached worktree of that
/// revision (same detached candidate-head model as publication). `--root` / `check_root` cannot
/// be combined with `revision`.
pub fn run(
    check_root: Option<&Path>,
    repo_root: Option<&Path>,
    manifest_path: Option<&Path>,
    revision: Option<&str>,
) -> Result<QualityReport> {
    let repo = resolve_repo_root(repo_root)?;

    if let Some(revision) = revision {
        if check_root.is_some() {
            bail!("--root cannot be combined with --revision");
        }
        let worktree = DetachedWorktree::create(&repo, revision)?;
        let path = match manifest_path {
            Some(path) => path.to_path_buf(),
            None => worktree.path.join(MANIFEST_PATH),
        };
        let manifest_root = if manifest_path.is_some() {
            repo.as_path()
        } else {
            worktree.path.as_path()
        };
        let manifest = load_manifest(manifest_root, Some(&path))?;
        return run_manifest(&worktree.path, &manifest, &path, revision);
    }

    let path = match manifest_path {
        Some(path) => path.to_path_buf(),
        None => repo.join(MANIFEST_PATH),
    };
    let manifest = load_manifest(&repo, Some(&path))?;
    let candidate_revision = match git_output_trimmed(&repo, &["rev-parse", "HEAD"]) {
        Ok(revision) => revision,
        Err(_) if check_root.is_some() => "uncommitted-working-tree".to_owned(),
        Err(error) => return Err(error),
    };
    let root = match check_root {
        Some(path) => path.to_path_buf(),
        None => repo.clone(),
    };
    run_manifest(&root, &manifest, &path, &candidate_revision)
}

/// CLI entrypoint for `xtask quality`.
pub fn run_cli(
    check_root: Option<&Path>,
    repo_root: Option<&Path>,
    manifest_path: Option<&Path>,
    revision: Option<&str>,
) -> Result<()> {
    let report = run(check_root, repo_root, manifest_path, revision)?;
    println!(
        "quality ok ({} checks from {})",
        report.checks.len(),
        report.manifest_path.display()
    );
    Ok(())
}

#[derive(Debug)]
struct CheckFailure {
    evidence: CommandEvidence,
    message: String,
}

impl CheckFailure {
    fn new(evidence: CommandEvidence, message: impl Into<String>) -> Self {
        Self {
            evidence,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (exit={})\nstdout:\n{}\nstderr:\n{}",
            self.message, self.evidence.exit_code, self.evidence.stdout, self.evidence.stderr
        )
    }
}

impl std::error::Error for CheckFailure {}

fn run_check(
    check_root: &Path,
    check: &QualityCheck,
    candidate_revision: &str,
) -> Result<(String, CommandEvidence)> {
    match check.runner.as_str() {
        RUNNER_DOCS_CHECK => {
            if quality_command_uid().is_some() {
                let root = check_root.to_string_lossy().to_string();
                let evidence = run_xtask_with_evidence(
                    check_root,
                    &["docs-check", "--root", &root],
                    candidate_revision,
                )?;
                return Ok(("docs-check passed".to_owned(), evidence));
            }
            let command = format!("docs-check under {}", check_root.display());
            docs_check::run(Some(check_root))
                .with_context(|| format!("docs-check failed under {}", check_root.display()))?;
            Ok((
                "docs-check passed".to_owned(),
                success_evidence(command, candidate_revision),
            ))
        }
        RUNNER_ARCHITECTURE => {
            let manifest = check_root.join("Cargo.toml");
            if quality_command_uid().is_some() {
                let manifest = manifest.to_string_lossy().to_string();
                let args = architecture_xtask_args(&manifest);
                let evidence = run_xtask_with_evidence(check_root, &args, candidate_revision)?;
                return Ok(("architecture passed".to_owned(), evidence));
            }
            let command = format!("architecture {}", manifest.display());
            architecture::run(Some(&manifest))
                .with_context(|| format!("architecture check failed for {}", manifest.display()))?;
            Ok((
                "architecture passed".to_owned(),
                success_evidence(command, candidate_revision),
            ))
        }
        RUNNER_CARGO_CHECK => {
            let args = ["check", "--workspace", "--locked"];
            let evidence = run_cargo_with_evidence(check_root, &args, candidate_revision).map_err(
                |evidence| {
                    anyhow::Error::new(CheckFailure::new(
                        evidence,
                        "cargo check --workspace --locked failed",
                    ))
                },
            )?;
            Ok((
                "cargo check --workspace --locked passed".to_owned(),
                evidence,
            ))
        }
        RUNNER_CARGO_TEST => {
            let args = ["test", "--workspace", "--locked"];
            let evidence = run_cargo_with_evidence(check_root, &args, candidate_revision).map_err(
                |evidence| {
                    anyhow::Error::new(CheckFailure::new(
                        evidence,
                        "cargo test --workspace --locked failed",
                    ))
                },
            )?;
            Ok((
                "cargo test --workspace --locked passed".to_owned(),
                evidence,
            ))
        }
        RUNNER_CARGO_DOC => {
            let args = ["doc", "--workspace", "--no-deps", "--locked"];
            let evidence = run_cargo_with_evidence(check_root, &args, candidate_revision).map_err(
                |evidence| {
                    anyhow::Error::new(CheckFailure::new(
                        evidence,
                        "cargo doc --workspace --no-deps --locked failed",
                    ))
                },
            )?;
            Ok((
                "cargo doc --workspace --no-deps --locked passed".to_owned(),
                evidence,
            ))
        }
        RUNNER_CARGO_FMT => {
            let args = ["fmt", "--all", "--check"];
            let evidence = run_cargo_with_evidence(check_root, &args, candidate_revision).map_err(
                |evidence| {
                    anyhow::Error::new(CheckFailure::new(
                        evidence,
                        "cargo fmt --all --check failed",
                    ))
                },
            )?;
            Ok(("cargo fmt --all --check passed".to_owned(), evidence))
        }
        RUNNER_CARGO_CLIPPY => {
            let args = [
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ];
            let evidence = run_cargo_with_evidence(check_root, &args, candidate_revision).map_err(
                |evidence| {
                    anyhow::Error::new(CheckFailure::new(
                        evidence,
                        "cargo clippy --workspace --all-targets -- -D warnings failed",
                    ))
                },
            )?;
            Ok(("cargo clippy passed".to_owned(), evidence))
        }
        RUNNER_DEPENDENCIES => {
            if quality_command_uid().is_some() {
                let root = check_root.to_string_lossy().to_string();
                let evidence = run_xtask_with_evidence(
                    check_root,
                    &["dependencies", "--root", &root],
                    candidate_revision,
                )?;
                return Ok(("dependencies policy passed".to_owned(), evidence));
            }
            let command = format!(
                "cargo run --locked -p xtask -- dependencies --root {}",
                check_root.display()
            );
            dependencies::run(Some(check_root)).with_context(|| {
                format!(
                    "dependency policy gate failed under {}",
                    check_root.display()
                )
            })?;
            Ok((
                "dependencies policy passed".to_owned(),
                success_evidence(command, candidate_revision),
            ))
        }
        RUNNER_OPERATION_COVERAGE => {
            let command = "cargo run --locked -p xtask -- operation-coverage --mode baseline";
            operation_coverage::run_at(check_root, CoverageMode::Baseline, "")?;
            Ok((
                "baseline operation coverage passed".to_owned(),
                success_evidence(command.to_owned(), candidate_revision),
            ))
        }
        RUNNER_CARGO_DENY => {
            match dependencies::run_cargo_deny_with_evidence(check_root, candidate_revision) {
                Ok(evidence) => Ok(("cargo deny check passed".to_owned(), evidence)),
                Err(error) => {
                    if let Some(failure) = error.downcast_ref::<dependencies::DenyFailure>() {
                        Err(anyhow::Error::new(CheckFailure::new(
                            failure.evidence.clone(),
                            "cargo deny failed",
                        )))
                    } else {
                        Err(error)
                    }
                }
            }
        }
        other => bail!("unavailable quality runner `{other}` (fail-closed)"),
    }
}

fn success_evidence(command: String, candidate_revision: &str) -> CommandEvidence {
    CommandEvidence {
        command,
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
        candidate_revision: candidate_revision.to_owned(),
    }
}

fn run_cargo_with_evidence(
    check_root: &Path,
    args: &[&str],
    candidate_revision: &str,
) -> Result<CommandEvidence, CommandEvidence> {
    let command = format!("cargo {}", args.join(" "));
    let mut process = Command::new("cargo");
    process.args(args).current_dir(check_root);
    apply_quality_command_uid(&mut process).map_err(|error| CommandEvidence {
        command: command.clone(),
        exit_code: 127,
        stdout: String::new(),
        stderr: error,
        candidate_revision: candidate_revision.to_owned(),
    })?;
    let output = process.output().map_err(|error| CommandEvidence {
        command: command.clone(),
        exit_code: 127,
        stdout: String::new(),
        stderr: format!("failed to spawn cargo: {error}"),
        candidate_revision: candidate_revision.to_owned(),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);
    let evidence = CommandEvidence {
        command,
        exit_code,
        stdout,
        stderr,
        candidate_revision: candidate_revision.to_owned(),
    };

    if output.status.success() {
        Ok(evidence)
    } else {
        Err(evidence)
    }
}

fn architecture_xtask_args(manifest: &str) -> [&str; 3] {
    ["architecture", "--manifest-path", manifest]
}

fn run_xtask_with_evidence(
    check_root: &Path,
    args: &[&str],
    candidate_revision: &str,
) -> Result<CommandEvidence> {
    let executable = std::env::current_exe().context("resolve trusted xtask executable")?;
    let command = format!("{} {}", executable.display(), args.join(" "));
    let mut process = Command::new(&executable);
    process.args(args).current_dir(check_root);
    apply_quality_command_uid(&mut process).map_err(anyhow::Error::msg)?;
    let output = process
        .output()
        .with_context(|| format!("failed spawning unprivileged `{command}`"))?;
    let evidence = CommandEvidence {
        command,
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        candidate_revision: candidate_revision.to_owned(),
    };
    if !output.status.success() {
        return Err(anyhow::Error::new(CheckFailure::new(
            evidence,
            "unprivileged quality command failed",
        )));
    }
    Ok(evidence)
}

fn quality_command_uid() -> Option<std::ffi::OsString> {
    std::env::var_os(QUALITY_COMMAND_UID_ENV)
}

fn apply_quality_command_uid(process: &mut Command) -> std::result::Result<(), String> {
    let Some(uid) = quality_command_uid() else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let uid = uid
            .to_string_lossy()
            .parse::<u32>()
            .map_err(|error| format!("invalid {QUALITY_COMMAND_UID_ENV}: {error}"))?;
        process.uid(uid).gid(uid);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = process;
        Err(format!(
            "{QUALITY_COMMAND_UID_ENV} is unsupported on this platform"
        ))
    }
}

fn validate_runner(runner: &str) -> Result<()> {
    match runner {
        RUNNER_DOCS_CHECK
        | RUNNER_ARCHITECTURE
        | RUNNER_CARGO_CHECK
        | RUNNER_CARGO_TEST
        | RUNNER_CARGO_DOC
        | RUNNER_CARGO_FMT
        | RUNNER_CARGO_CLIPPY
        | RUNNER_DEPENDENCIES
        | RUNNER_OPERATION_COVERAGE
        | RUNNER_CARGO_DENY => Ok(()),
        other => bail!(
            "unknown quality runner `{other}`; currently implemented runners: {RUNNER_DOCS_CHECK}, {RUNNER_ARCHITECTURE}, {RUNNER_CARGO_CHECK}, {RUNNER_CARGO_TEST}, {RUNNER_CARGO_DOC}, {RUNNER_CARGO_FMT}, {RUNNER_CARGO_CLIPPY}, {RUNNER_DEPENDENCIES}, {RUNNER_OPERATION_COVERAGE}, {RUNNER_CARGO_DENY}"
        ),
    }
}

fn resolve_repo_root(repo_root: Option<&Path>) -> Result<PathBuf> {
    let path = match repo_root {
        Some(path) => path.to_path_buf(),
        None => find_repository_root().unwrap_or_else(semantic_judge::default_repository_root),
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

struct DetachedWorktree {
    repo_root: PathBuf,
    path: PathBuf,
}

impl DetachedWorktree {
    fn create(repo_root: &Path, commit: &str) -> Result<Self> {
        let path = create_temp_dir("quality-revision")?;
        let _ = fs::remove_dir_all(&path);
        git_run(
            repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().context("worktree path is not UTF-8")?,
                commit,
            ],
        )
        .with_context(|| {
            format!(
                "failed to create detached worktree for {commit} at {}",
                path.display()
            )
        })?;
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            path,
        })
    }
}

impl Drop for DetachedWorktree {
    fn drop(&mut self) {
        let path = self.path.to_string_lossy().to_string();
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force", &path])
            .current_dir(&self.repo_root)
            .output();
        let _ = fs::remove_dir_all(&self.path);
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.repo_root)
            .output();
    }
}

fn create_temp_dir(label: &str) -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "loop-engine-{label}-{}-{nanos}",
        std::process::id()
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
    Ok(String::from_utf8(output.stdout)
        .context("git stdout is not UTF-8")?
        .trim_end()
        .to_owned())
}

fn git_show_revision(repo_root: &Path, revision: &str, path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["show", &format!("{revision}:{path}")])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git show {revision}:{path}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(output.stdout).context("git stdout is not UTF-8")?,
    ))
}

fn strip_comment(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut in_string = false;
    for ch in line.chars() {
        if ch == '"' {
            result.push(ch);
            in_string = !in_string;
            continue;
        }
        if ch == '#' && !in_string {
            break;
        }
        result.push(ch);
    }
    result
}

fn parse_assignment_value(rest: &str, key: &str) -> Result<String> {
    let rest = rest.trim();
    let Some(value) = rest.strip_prefix('=') else {
        bail!("expected `{key} = ...`");
    };
    Ok(value.trim().trim_matches('"').to_owned())
}

fn parse_quoted_assignment(rest: &str, key: &str) -> Result<String> {
    let rest = rest.trim();
    let Some(value) = rest.strip_prefix('=') else {
        bail!("expected `{key} = \"...\"`");
    };
    let value = value.trim();
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        bail!("expected quoted string for `{key}`, got `{value}`");
    };
    Ok(inner.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_reads_checks() {
        let manifest = parse_manifest(
            r#"
# comment
schema_version = 1

[[checks]]
id = "docs-check"
runner = "docs-check"

[[checks]]
id = "architecture"
runner = "architecture"
"#,
        )
        .unwrap();
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.checks.len(), 2);
        assert_eq!(manifest.checks[0].id, "docs-check");
        assert_eq!(manifest.checks[1].runner, "architecture");
    }

    #[test]
    fn privileged_architecture_reexec_uses_registered_cli_flag() {
        assert_eq!(
            architecture_xtask_args("/candidate/Cargo.toml"),
            ["architecture", "--manifest-path", "/candidate/Cargo.toml"]
        );
    }

    #[test]
    fn parse_manifest_rejects_unknown_runner() {
        let error = parse_manifest(
            r#"
schema_version = 1

[[checks]]
id = "mystery"
runner = "not-a-runner"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown quality runner"));
    }

    #[test]
    fn monotonic_evolution_rejects_removed_check() {
        let parent = parse_manifest(
            r#"
schema_version = 1
[[checks]]
id = "docs-check"
runner = "docs-check"
[[checks]]
id = "architecture"
runner = "architecture"
"#,
        )
        .unwrap();
        let candidate = parse_manifest(
            r#"
schema_version = 1
[[checks]]
id = "docs-check"
runner = "docs-check"
"#,
        )
        .unwrap();
        let error = enforce_manifest_monotonic_evolution(Some(&parent), &candidate).unwrap_err();
        assert!(error.to_string().contains("removed check"));
    }

    #[test]
    fn monotonic_evolution_allows_additions() {
        let parent = parse_manifest(
            r#"
schema_version = 1
[[checks]]
id = "docs-check"
runner = "docs-check"
"#,
        )
        .unwrap();
        let candidate = parse_manifest(
            r#"
schema_version = 1
[[checks]]
id = "docs-check"
runner = "docs-check"
[[checks]]
id = "architecture"
runner = "architecture"
"#,
        )
        .unwrap();
        enforce_manifest_monotonic_evolution(Some(&parent), &candidate).unwrap();
    }
}
