//! Exact staged deterministic validation used by the thin pre-commit adapter.

use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, bail};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::candidate::Candidate;
use crate::config::{Phase, SemanticRequirement};
use crate::git::PRIVATE_STAGED_INDEX_ENVIRONMENT;
use crate::process::Cancellation;
use crate::quality::{CommandRecord, DeterministicResult};

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
