//! Exact staged deterministic validation used by the thin pre-commit adapter.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, bail};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::candidate::Candidate;
use crate::config::{Phase, SemanticRequirement};
use crate::git::{PRIVATE_STAGED_INDEX_ENVIRONMENT, Repository, ensure_success};
use crate::process::Cancellation;
use crate::quality::{CommandRecord, CommandStatus, DeterministicResult};

const HOOKS_PATH: &str = ".githooks";
const PRE_COMMIT_PATH: &str = ".githooks/pre-commit";
const PRE_PUSH_PATH: &str = ".githooks/pre-push";
const PRE_COMMIT_ADAPTER: &[u8] = br#"#!/usr/bin/env bash
# loop-engine-hook-version: 2
set -euo pipefail
INDEX_INPUT=
if [[ ${GIT_INDEX_FILE+x} == x ]]; then
  [[ -n "$GIT_INDEX_FILE" ]] || { echo "pre-commit: GIT_INDEX_FILE is empty" >&2; exit 1; }
  case "$GIT_INDEX_FILE" in /*) INDEX_INPUT="$GIT_INDEX_FILE" ;; *) INDEX_INPUT="$(pwd -P)/$GIT_INDEX_FILE" ;; esac
  [[ -f "$INDEX_INPUT" && ! -L "$INDEX_INPUT" ]] || { echo "pre-commit: Git index is not a regular file: $INDEX_INPUT" >&2; exit 1; }
fi
ROOT="$(/usr/bin/git rev-parse --show-toplevel)"
cd "$ROOT"
while IFS= read -r name; do unset "$name"; done < <(/usr/bin/git rev-parse --local-env-vars)
unset LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE
if [[ -n "$INDEX_INPUT" ]]; then export LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE="$INDEX_INPUT"; fi
exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --staged
"#;
const PRE_PUSH_ADAPTER: &[u8] = br#"#!/usr/bin/env bash
# loop-engine-hook-version: 2
set -euo pipefail
ROOT="$(/usr/bin/git rev-parse --show-toplevel)"
cd "$ROOT"
unset $(/usr/bin/git rev-parse --local-env-vars)
exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --publication --updates-stdin
"#;
const PRE_COMMIT_COMMAND: &str = "exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --staged";
const PRE_PUSH_COMMAND: &str =
    "exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --publication --updates-stdin";

/// Validate materialized `HEAD`, then install repository-local hook dispatch.
pub fn install(repository_path: &Path) -> Result<()> {
    install_with_cancellation(repository_path, &Cancellation::new())
}

/// Install hook dispatch under caller-owned cancellation.
///
/// Candidate cleanup happens only after cancelled prerequisite groups are fully
/// awaited. The final cancellation transition precedes repository config writes,
/// so either interruption wins without mutation or installation finishes fully.
pub fn install_with_cancellation(
    repository_path: &Path,
    cancellation: &Cancellation,
) -> Result<()> {
    let repository = Repository::resolve(repository_path)?;
    let head = repository
        .head()?
        .context("cannot install hooks in repository with unborn HEAD")?;
    let candidate =
        Candidate::revision(repository_path, Some(OsStr::new(&head)), OsStr::new(&head))
            .context("failed to materialize HEAD for hook installation")?
            .prepare(SemanticRequirement::Optional)
            .context("failed to validate materialized HEAD for hook installation")?;

    let operation = (|| {
        validate_adapter(
            candidate.source_root(),
            PRE_COMMIT_PATH,
            PRE_COMMIT_ADAPTER,
            PRE_COMMIT_COMMAND,
        )?;
        validate_adapter(
            candidate.source_root(),
            PRE_PUSH_PATH,
            PRE_PUSH_ADAPTER,
            PRE_PUSH_COMMAND,
        )?;

        let prerequisites =
            crate::quality::run_prerequisites_with_cancellation(&candidate, cancellation);
        if prerequisites.iter().any(|record| {
            !matches!(
                record.status,
                CommandStatus::Passed | CommandStatus::SkippedSuccess
            )
        }) {
            print_failures_from_records(&prerequisites);
            bail!("hook installation prerequisite validation failed");
        }
        candidate
            .verify_unchanged()
            .context("materialized HEAD changed during hook installation validation")?;

        let configured_hooks_path = local_hooks_path(candidate.repository())?;
        if !cancellation.finish() {
            bail!("hook installation interrupted before repository configuration");
        }

        match configured_hooks_path {
            None => set_local_hooks_path(candidate.repository()),
            Some(values) if values == [HOOKS_PATH.as_bytes()] => Ok(()),
            Some(values) => {
                let rendered = values
                    .iter()
                    .map(|value| format!("`{}`", String::from_utf8_lossy(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "conflicting local core.hooksPath value(s): {rendered}; expected `{HOOKS_PATH}`"
                )
            }
        }
    })();

    let cleanup = candidate
        .cleanup()
        .map_err(anyhow::Error::new)
        .context("failed to clean hook installation candidate state");
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(operation), Err(cleanup)) => Err(operation.context(format!(
            "hook installation candidate cleanup also failed: {cleanup:#}"
        ))),
    }
}

/// Stable CLI installation lifecycle, including SIGINT/SIGTERM coordination.
pub fn run_install(repository_path: &Path) -> Result<()> {
    let cancellation = Cancellation::new();
    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to install signal listener")?;
    let signal_handle = signals.handle();
    let listener_cancellation = cancellation.clone();
    let listener = thread::Builder::new()
        .name("xtask-install-signals".to_owned())
        .spawn(move || {
            let mut first_signal = None;
            for signal in signals.forever() {
                if listener_cancellation.cancel()
                    != crate::process::CancellationRequest::AlreadyFinished
                {
                    first_signal.get_or_insert(signal);
                }
            }
            first_signal
        })
        .context("failed to start signal listener")?;

    let installation = install_with_cancellation(repository_path, &cancellation);
    signal_handle.close();
    let interrupted = listener
        .join()
        .map_err(|_| anyhow::anyhow!("signal listener panicked"))?;
    if let Some(signal) = interrupted {
        bail!("hook installation interrupted by signal {signal}");
    }
    installation
}

fn validate_adapter(
    candidate_root: &Path,
    relative_path: &str,
    expected: &[u8],
    final_command: &str,
) -> Result<()> {
    let expected_text = std::str::from_utf8(expected)
        .with_context(|| format!("embedded {relative_path} adapter is not UTF-8"))?;
    if !expected_text
        .lines()
        .any(|line| line == "# loop-engine-hook-version: 2")
        || expected_text.lines().last() != Some(final_command)
    {
        bail!("embedded {relative_path} does not satisfy final v2 adapter contract");
    }

    let path = candidate_root.join(relative_path);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("required hook `{relative_path}` is missing from HEAD"))?;
    if !metadata.file_type().is_file() {
        bail!("required hook `{relative_path}` is not a regular file in HEAD");
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        bail!("required hook `{relative_path}` is not executable in HEAD");
    }
    let actual = fs::read(&path)
        .with_context(|| format!("failed reading required hook `{relative_path}` from HEAD"))?;
    if actual != expected {
        bail!("required hook `{relative_path}` does not match final v2 adapter contract");
    }
    Ok(())
}

fn local_hooks_path(repository: &Repository) -> Result<Option<Vec<Vec<u8>>>> {
    let output = repository.output(
        [
            OsStr::new("config"),
            OsStr::new("--local"),
            OsStr::new("--null"),
            OsStr::new("--get-all"),
            OsStr::new("core.hooksPath"),
        ],
        None,
    )?;
    if output.status.success() {
        if !output.stdout.ends_with(&[0]) {
            bail!("git config returned unterminated local core.hooksPath value");
        }
        let values = output.stdout[..output.stdout.len() - 1]
            .split(|byte| *byte == 0)
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        return Ok(Some(values));
    }
    if output.status.code() == Some(1) && output.stdout.is_empty() {
        return Ok(None);
    }
    ensure_success(&output, "git config --local --get-all core.hooksPath")?;
    unreachable!()
}

fn set_local_hooks_path(repository: &Repository) -> Result<()> {
    let output = repository.output(
        [
            OsStr::new("config"),
            OsStr::new("--local"),
            OsStr::new("--replace-all"),
            OsStr::new("core.hooksPath"),
            OsStr::new(HOOKS_PATH),
        ],
        None,
    )?;
    ensure_success(&output, "git config --local core.hooksPath=.githooks")
}

fn print_failures_from_records(records: &[CommandRecord]) {
    for record in records {
        if let Some(failure) = &record.failure {
            eprintln!("{}: {}", record.id, failure.message);
            if let Some(hint) = &record.install_hint {
                eprintln!("{} install hint: {}", record.id, hint);
            }
            print_process_stderr(record);
        }
    }
}

/// Build and validate only the effective Git index, then consume all temporary state.
///
/// Caller-owned cancellation reaches any active process group. Scheduler awaits
/// group cleanup before returning, so candidate cleanup never races child access.
pub fn validate_staged(
    repository_path: &Path,
    cancellation: &Cancellation,
) -> Result<DeterministicResult> {
    validate_staged_with_index(repository_path, None, cancellation)
}

fn validate_staged_with_index(
    repository_path: &Path,
    index: Option<&Path>,
    cancellation: &Cancellation,
) -> Result<DeterministicResult> {
    let candidate = match index {
        Some(index) => Candidate::staged_with_index(repository_path, index),
        None => Candidate::staged(repository_path),
    }
    .context("failed to materialize exact staged candidate")?
    .prepare(SemanticRequirement::Optional)
    .context("failed to prepare exact staged candidate")?;
    let result = crate::quality::run_with_cancellation(&candidate, Phase::PreCommit, cancellation);
    candidate
        .cleanup()
        .map_err(anyhow::Error::new)
        .context("failed to clean exact staged candidate state")?;
    Ok(result)
}

/// Stable CLI staged-validation lifecycle, including SIGINT/SIGTERM coordination.
pub fn run_staged(repository_path: &Path) -> Result<()> {
    let index = staged_index_from_environment()?;
    let cancellation = Cancellation::new();
    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to install signal listener")?;
    let signal_handle = signals.handle();
    let listener_cancellation = cancellation.clone();
    let listener = thread::Builder::new()
        .name("xtask-staged-signals".to_owned())
        .spawn(move || {
            let mut first_signal = None;
            for signal in signals.forever() {
                first_signal.get_or_insert(signal);
                listener_cancellation.cancel();
            }
            first_signal
        })
        .context("failed to start signal listener")?;

    let validation = validate_staged_with_index(repository_path, index.as_deref(), &cancellation);
    signal_handle.close();
    let interrupted = listener
        .join()
        .map_err(|_| anyhow::anyhow!("signal listener panicked"))?;

    let result = validation?;
    if let Some(signal) = interrupted {
        print_failures(&result);
        bail!("staged validation interrupted by signal {signal}");
    }
    if !result.passed() {
        print_failures(&result);
        bail!("staged deterministic validation failed");
    }
    Ok(())
}

fn staged_index_from_environment() -> Result<Option<PathBuf>> {
    let private = std::env::var_os(PRIVATE_STAGED_INDEX_ENVIRONMENT);
    let git = std::env::var_os("GIT_INDEX_FILE");
    let selected = match (private, git) {
        (Some(_), Some(_)) => bail!(
            "both {PRIVATE_STAGED_INDEX_ENVIRONMENT} and GIT_INDEX_FILE select a staged index"
        ),
        (Some(index), None) | (None, Some(index)) => Some(PathBuf::from(index)),
        (None, None) => None,
    };
    let Some(index) = selected else {
        return Ok(None);
    };
    if index.as_os_str().is_empty() {
        bail!("effective Git index path is empty");
    }
    if index.is_absolute() {
        Ok(Some(index))
    } else {
        Ok(Some(
            std::env::current_dir()
                .context("failed to resolve staged-index invocation directory")?
                .join(index),
        ))
    }
}

fn print_failures(result: &DeterministicResult) {
    for record in result.prerequisites.iter().chain(&result.checks) {
        if let Some(failure) = &record.failure {
            eprintln!("{}: {}", record.id, failure.message);
            print_process_stderr(record);
        }
    }
    if let Some(failure) = &result.final_failure {
        eprintln!("final-verification: {}", failure.message);
    }
}

fn print_process_stderr(record: &CommandRecord) {
    let Some(process) = &record.process else {
        return;
    };
    if process.stderr.exact_bytes().is_empty() {
        return;
    }
    eprintln!(
        "{} stderr: {}",
        record.id,
        String::from_utf8_lossy(process.stderr.exact_bytes()).trim_end()
    );
}
