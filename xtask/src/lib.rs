use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand, error::ErrorKind};

pub mod acceptance_report;
pub mod architecture;
pub mod candidate;
pub mod config;
pub mod dependencies;
pub mod docs_check;
pub mod git;
pub mod operation_coverage;
pub mod process;
pub mod quality;
pub mod semantic_judge;

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Loop Engine build tooling",
    subcommand_required = true,
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Generate and validate final reference/invariant/facet acceptance evidence.
    AcceptanceReport {
        /// Repository root to inspect (defaults to the current repository).
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Exact candidate revision represented by the report.
        #[arg(long, value_name = "REVISION")]
        revision: String,
        /// Optional JSON report output path.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Verify crate-level product dependency architecture (I22).
    Architecture {
        /// Workspace manifest to inspect (defaults to the current repository).
        #[arg(long, value_name = "PATH")]
        manifest_path: Option<PathBuf>,
    },
    /// Verify deterministic documentation formatting and link policy.
    DocsCheck {
        /// Repository root to inspect (defaults to the current repository).
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
    },
    /// Verify dependency license/source/lockfile policy from `deny.toml`.
    Dependencies {
        /// Repository root to inspect (defaults to the current repository).
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
    },
    /// Verify equality of currently exposed operation catalogs.
    OperationCoverage {
        /// Closure stage: baseline, candidate, exposed, or final.
        #[arg(long, default_value = "exposed")]
        mode: String,
        /// Candidate-mode comma-separated operation IDs whose facets remain open.
        #[arg(long, default_value = "")]
        allow_open: String,
    },
    /// Run the semantic judge against an exact staged tree or revision range.
    Judge {
        /// Judge the exact staged index tree against HEAD (local mode).
        #[arg(long)]
        staged: bool,
        /// Parent revision for an exact revision-pair judgment.
        #[arg(long, value_name = "REV")]
        parent: Option<String>,
        /// Candidate revision for an exact revision-pair judgment.
        #[arg(long, value_name = "REV")]
        candidate: Option<String>,
        /// Base of one aggregate unpublished publication range (`from..HEAD`).
        #[arg(long, value_name = "REV")]
        unpublished_from: Option<String>,
        /// Disposition mode for revision-pair judgment (`local` or `publication`).
        #[arg(long, default_value = "local", value_name = "MODE")]
        mode: String,
        /// Override judge executable path (otherwise env/default contract path).
        #[arg(long, value_name = "PATH")]
        executable: Option<PathBuf>,
        /// Override request timeout in seconds.
        #[arg(long, value_name = "SECONDS")]
        timeout_seconds: Option<u64>,
        /// Explicitly claim a second bootstrap publication exception (always rejected).
        #[arg(long)]
        claim_bootstrap_exception: bool,
        /// Optional repository root override (defaults to workspace root).
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
    },
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
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    match cli.command {
        Some(Commands::AcceptanceReport {
            root,
            revision,
            output,
        }) => acceptance_report::run(root.as_deref(), &revision, output.as_deref()),
        Some(Commands::Architecture { manifest_path }) => {
            architecture::run(manifest_path.as_deref())?;
            Ok(())
        }
        Some(Commands::DocsCheck { root }) => {
            docs_check::run(root.as_deref())?;
            Ok(())
        }
        Some(Commands::Dependencies { root }) => {
            dependencies::run(root.as_deref())?;
            Ok(())
        }
        Some(Commands::OperationCoverage { mode, allow_open }) => {
            operation_coverage::run(operation_coverage::CoverageMode::parse(&mode)?, &allow_open)
        }
        Some(Commands::Judge {
            staged,
            parent,
            candidate,
            unpublished_from,
            mode,
            executable,
            timeout_seconds,
            claim_bootstrap_exception,
            root,
        }) => {
            let mode = mode.parse()?;
            semantic_judge::run(semantic_judge::RunArgs {
                repo_root: root.as_deref(),
                staged,
                parent: parent.as_deref(),
                candidate: candidate.as_deref(),
                unpublished_from: unpublished_from.as_deref(),
                mode,
                executable: executable.as_deref(),
                timeout_seconds,
                claim_bootstrap_exception,
            })
        }
        None => unreachable!("clap requires a subcommand"),
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
    fn standard_help_subcommand_succeeds() {
        let status = run(["xtask", "help"]);
        assert_eq!(status, ExitCode::SUCCESS);
    }

    #[test]
    fn missing_command_fails() {
        let status = run(["xtask"]);
        assert_eq!(status, ExitCode::FAILURE);
    }

    #[test]
    fn unknown_command_fails() {
        let result = Cli::try_parse_from(["xtask", "unknown"]);
        assert!(result.is_err());
    }

    #[test]
    fn removed_hook_certification_commands_are_not_callable() {
        let result = Cli::try_parse_from(["xtask", "hooks", "verify"]);
        assert!(result.is_err());
    }
}
