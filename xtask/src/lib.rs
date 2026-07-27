use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, error::ErrorKind};

pub mod candidate;
pub mod config;
pub mod git;
pub mod hooks;
pub mod process;
pub mod quality;

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
    /// Validate an exact Git candidate.
    Validate(ValidateArgs),
}

#[derive(Debug, clap::Args)]
struct ValidateArgs {
    /// Validate the effective index as the pre-commit candidate.
    #[arg(long, required = true)]
    staged: bool,
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
            command: Some(Command::Validate(ValidateArgs { staged: true })),
        }) => hooks::run_staged(&std::env::current_dir()?),
        Ok(Cli {
            command: Some(Command::Validate(_)),
        }) => unreachable!("clap requires --staged"),
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
