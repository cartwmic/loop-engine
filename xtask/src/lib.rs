use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;
use std::thread;

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Parser, error::ErrorKind};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub mod candidate;
pub mod config;
pub mod git;
pub mod hooks;
pub mod process;
pub mod publication;
pub mod publication_input;
pub mod quality;
pub mod report;
pub mod semantic_judge;

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Loop Engine build tooling",
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Manage repository Git hooks.
    Hooks(HooksArgs),
    /// Validate an exact Git candidate.
    Validate(ValidateArgs),
    /// Manage validation evidence.
    Validation(ValidationArgs),
}

#[derive(Debug, clap::Args)]
struct HooksArgs {
    #[command(subcommand)]
    command: HooksCommand,
}

#[derive(Debug, clap::Subcommand)]
enum HooksCommand {
    /// Validate tracked adapters and set local core.hooksPath.
    Install,
}

#[derive(Debug, clap::Args)]
#[command(group(ArgGroup::new("mode").required(true).multiple(false).args(["staged", "semantic", "publication"])))]
struct ValidateArgs {
    /// Validate the effective index as the pre-commit candidate.
    #[arg(long)]
    staged: bool,
    /// Run advisory publication-phase deterministic and semantic validation.
    #[arg(long, requires_all = ["base", "candidate"])]
    semantic: bool,
    /// Validate one aggregate publication from a supported input source.
    #[arg(long, requires = "updates_stdin")]
    publication: bool,
    /// Read exact Git pre-push update lines from standard input.
    #[arg(long, requires = "publication")]
    updates_stdin: bool,
    /// Advisory base revision.
    #[arg(long, requires = "semantic")]
    base: Option<OsString>,
    /// Advisory candidate revision; must resolve to HEAD.
    #[arg(long, requires = "semantic")]
    candidate: Option<OsString>,
}

#[derive(Debug, clap::Args)]
struct ValidationArgs {
    #[command(subcommand)]
    command: ValidationCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ValidationCommand {
    /// Approve one exact verified semantic-block evaluation.
    Approve(ApproveArgs),
}

#[derive(Debug, clap::Args)]
struct ApproveArgs {
    /// SHA-256 digest of stored evaluation report.
    #[arg(long)]
    report: String,
    /// Non-empty owner reason retained in immutable evidence.
    #[arg(long)]
    reason: String,
}

#[derive(Debug)]
pub struct AdvisoryOutcome {
    pub report_digest: String,
    pub evaluation: report::EvaluationRecord,
}

/// Run the xtask command dispatcher.
pub fn run<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
{
    match try_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn try_run<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(Cli { command: None }) => Ok(()),
        Ok(Cli {
            command:
                Some(Command::Hooks(HooksArgs {
                    command: HooksCommand::Install,
                })),
        }) => hooks::run_install(&std::env::current_dir()?),
        Ok(Cli {
            command:
                Some(Command::Validate(ValidateArgs {
                    staged: true,
                    semantic: false,
                    publication: false,
                    ..
                })),
        }) => hooks::run_staged(&std::env::current_dir()?),
        Ok(Cli {
            command:
                Some(Command::Validate(ValidateArgs {
                    staged: false,
                    semantic: true,
                    publication: false,
                    base: Some(base),
                    candidate: Some(candidate),
                    ..
                })),
        }) => {
            let outcome = run_advisory_cli(&std::env::current_dir()?, &base, &candidate)?;
            println!("{}", outcome.report_digest);
            match outcome.evaluation.derived_disposition {
                report::DerivedDisposition::Pass => Ok(()),
                report::DerivedDisposition::DeterministicBlock => {
                    bail!("advisory deterministic validation failed")
                }
                report::DerivedDisposition::SemanticBlock => {
                    bail!("advisory semantic validation blocked")
                }
            }
        }
        Ok(Cli {
            command:
                Some(Command::Validate(ValidateArgs {
                    staged: false,
                    semantic: false,
                    publication: true,
                    updates_stdin: true,
                    ..
                })),
        }) => {
            let mut input = Vec::new();
            std::io::stdin()
                .read_to_end(&mut input)
                .context("failed reading publication updates from stdin")?;
            let outcome = run_publication_cli(&std::env::current_dir()?, &input)?;
            println!("{}", outcome.attempt_digest);
            match outcome.attempt.gate_decision {
                report::GateDecision::Pass | report::GateDecision::Approved => Ok(()),
                report::GateDecision::Block => bail!("publication validation blocked"),
            }
        }
        Ok(Cli {
            command:
                Some(Command::Validation(ValidationArgs {
                    command: ValidationCommand::Approve(arguments),
                })),
        }) => {
            let store = report::Store::open(&std::env::current_dir()?)?;
            let (digest, _) = store.approve(&arguments.report, &arguments.reason)?;
            println!("{digest}");
            Ok(())
        }
        Ok(Cli {
            command: Some(Command::Validate(_)),
        }) => unreachable!("clap validates exact validation mode arguments"),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Run advisory publication validation and write only its evaluation report.
pub fn run_advisory(
    repository_path: &Path,
    base_revision: &OsStr,
    candidate_revision: &OsStr,
) -> Result<AdvisoryOutcome> {
    run_advisory_with_cancellation(
        repository_path,
        base_revision,
        candidate_revision,
        &process::Cancellation::new(),
    )
}

/// Run advisory validation under caller-owned cancellation.
pub fn run_advisory_with_cancellation(
    repository_path: &Path,
    base_revision: &OsStr,
    candidate_revision: &OsStr,
    cancellation: &process::Cancellation,
) -> Result<AdvisoryOutcome> {
    let candidate =
        candidate::Candidate::revision(repository_path, Some(base_revision), candidate_revision)
            .context("failed to materialize advisory candidate")?
            .prepare(config::SemanticRequirement::Required)
            .context("failed to prepare advisory candidate")?;

    let operation = (|| {
        let deterministic =
            quality::run_with_cancellation(&candidate, config::Phase::Publication, cancellation);
        if cancellation.is_cancelled() {
            bail!("advisory validation interrupted before semantic evaluation");
        }
        let semantic = if deterministic.passed() {
            Some(
                semantic_judge::run_with_cancellation(&candidate, &deterministic, cancellation)
                    .context("failed to run advisory semantic validation")?,
            )
        } else {
            None
        };
        if cancellation.is_cancelled() {
            bail!("advisory validation interrupted before evaluation storage");
        }
        let binding = config::compute_binding(candidate.manifest(), candidate.source_root())
            .context("failed to compute advisory policy binding")?;
        let evaluation = report::EvaluationRecord::new(deterministic, semantic, &binding)?;
        if !cancellation.finish() {
            bail!("advisory validation interrupted before evaluation storage");
        }
        let store = report::Store::from_repository(candidate.repository());
        let report_digest = store.write_evaluation(&evaluation)?;
        Ok(AdvisoryOutcome {
            report_digest,
            evaluation,
        })
    })();

    let cleanup = candidate
        .cleanup()
        .map_err(anyhow::Error::new)
        .context("failed to clean advisory candidate state");
    combine_advisory_operation_and_cleanup(operation, cleanup)
}

fn run_publication_cli(
    repository_path: &Path,
    input: &[u8],
) -> Result<publication::PublicationOutcome> {
    let cancellation = process::Cancellation::new();
    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to install signal listener")?;
    let signal_handle = signals.handle();
    let listener_cancellation = cancellation.clone();
    let listener = thread::Builder::new()
        .name("xtask-publication-signals".to_owned())
        .spawn(move || {
            let mut first_signal = None;
            for signal in signals.forever() {
                if listener_cancellation.cancel() != process::CancellationRequest::AlreadyFinished {
                    first_signal.get_or_insert(signal);
                }
            }
            first_signal
        })
        .context("failed to start signal listener")?;

    let validation = publication::run_publication(repository_path, input, &cancellation);
    signal_handle.close();
    let interrupted = listener
        .join()
        .map_err(|_| anyhow::anyhow!("signal listener panicked"))?;
    if let Some(signal) = interrupted {
        bail!("publication validation interrupted by signal {signal}");
    }
    validation
}

fn run_advisory_cli(
    repository_path: &Path,
    base_revision: &OsStr,
    candidate_revision: &OsStr,
) -> Result<AdvisoryOutcome> {
    let cancellation = process::Cancellation::new();
    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to install signal listener")?;
    let signal_handle = signals.handle();
    let listener_cancellation = cancellation.clone();
    let listener = thread::Builder::new()
        .name("xtask-advisory-signals".to_owned())
        .spawn(move || {
            let mut first_signal = None;
            for signal in signals.forever() {
                if listener_cancellation.cancel() != process::CancellationRequest::AlreadyFinished {
                    first_signal.get_or_insert(signal);
                }
            }
            first_signal
        })
        .context("failed to start signal listener")?;

    let validation = run_advisory_with_cancellation(
        repository_path,
        base_revision,
        candidate_revision,
        &cancellation,
    );
    signal_handle.close();
    let interrupted = listener
        .join()
        .map_err(|_| anyhow::anyhow!("signal listener panicked"))?;
    if let Some(signal) = interrupted {
        bail!("advisory validation interrupted by signal {signal}");
    }
    validation
}

fn combine_advisory_operation_and_cleanup<T>(
    operation: Result<T>,
    cleanup: Result<()>,
) -> Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "advisory candidate cleanup also failed: {cleanup:#}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_help_flag_succeeds() {
        let status = run(["xtask", "--help"]);
        assert_eq!(status, ExitCode::SUCCESS);
    }

    #[test]
    fn no_arguments_succeeds() {
        let status = run(["xtask"]);
        assert_eq!(status, ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_fails() {
        let result = Cli::try_parse_from(["xtask", "unknown"]);
        assert!(result.is_err());
    }
}
