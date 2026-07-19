use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

pub mod architecture;
pub mod dependencies;
pub mod docs_check;
pub mod hooks;
pub mod publication;
pub mod quality;
pub mod semantic_judge;

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Loop Engine build tooling",
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show available commands.
    Help,
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
        /// Exclusive start of an unpublished publication range (`from..HEAD`).
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
    /// Run currently-implemented quality checks from `quality/manifest.toml`.
    Quality {
        /// Tree root to check (defaults to repository root).
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Repository root used to locate the quality manifest.
        #[arg(long, value_name = "PATH")]
        repo_root: Option<PathBuf>,
        /// Override path to the incremental quality manifest.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
        /// Exact revision to check in a temporary detached worktree.
        #[arg(long, value_name = "REV")]
        revision: Option<String>,
    },
    /// Exact-commit publication / pre-push gate for an unpublished range.
    Publication {
        /// Optional repository root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Exclusive start of the unpublished range (`from..to`).
        #[arg(long, value_name = "REV")]
        from: Option<String>,
        /// Inclusive end of the unpublished range (defaults to HEAD).
        #[arg(long, value_name = "REV")]
        to: Option<String>,
        /// Override judge executable path.
        #[arg(long, value_name = "PATH")]
        executable: Option<PathBuf>,
        /// Override request timeout in seconds.
        #[arg(long, value_name = "SECONDS")]
        timeout_seconds: Option<u64>,
    },
    /// Install, verify, or run versioned local git hooks.
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
}

#[derive(Debug, Subcommand)]
enum HooksCommand {
    /// Point `core.hooksPath` at the versioned `.githooks/` directory.
    Install {
        /// Optional repository root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
    },
    /// Verify hooksPath installation and pre-commit hook version.
    Verify {
        /// Optional repository root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
    },
    /// Run the local pre-commit adapter against the exact staged tree.
    PreCommit {
        /// Optional repository root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Override judge executable path.
        #[arg(long, value_name = "PATH")]
        executable: Option<PathBuf>,
        /// Override request timeout in seconds.
        #[arg(long, value_name = "SECONDS")]
        timeout_seconds: Option<u64>,
    },
    /// Run the exact-commit pre-push adapter (reads remote updates from stdin).
    PrePush {
        /// Optional repository root override.
        #[arg(long, value_name = "PATH")]
        root: Option<PathBuf>,
        /// Exact destination remote name supplied by Git's pre-push hook.
        #[arg(long, value_name = "NAME")]
        remote_name: Option<String>,
        /// Exact destination remote URL supplied by Git's pre-push hook.
        #[arg(long, value_name = "URL")]
        remote_url: Option<String>,
        /// Override judge executable path.
        #[arg(long, value_name = "PATH")]
        executable: Option<PathBuf>,
        /// Override request timeout in seconds.
        #[arg(long, value_name = "SECONDS")]
        timeout_seconds: Option<u64>,
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
    let cli = Cli::parse_from(args);

    match cli.command {
        Some(Commands::Help) => {
            print_help();
            Ok(())
        }
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
        Some(Commands::Quality {
            root,
            repo_root,
            manifest,
            revision,
        }) => quality::run_cli(
            root.as_deref(),
            repo_root.as_deref(),
            manifest.as_deref(),
            revision.as_deref(),
        ),
        Some(Commands::Publication {
            root,
            from,
            to,
            executable,
            timeout_seconds,
        }) => publication::run_cli(
            root.as_deref(),
            from.as_deref(),
            to.as_deref(),
            executable.as_deref(),
            timeout_seconds,
        ),
        Some(Commands::Hooks { command }) => match command {
            HooksCommand::Install { root } => hooks::run_install(root.as_deref()),
            HooksCommand::Verify { root } => hooks::run_verify(root.as_deref()),
            HooksCommand::PreCommit {
                root,
                executable,
                timeout_seconds,
            } => hooks::run_pre_commit(root.as_deref(), executable.as_deref(), timeout_seconds),
            HooksCommand::PrePush {
                root,
                remote_name,
                remote_url,
                executable,
                timeout_seconds,
            } => hooks::run_pre_push(
                root.as_deref(),
                remote_name.as_deref(),
                remote_url.as_deref(),
                executable.as_deref(),
                timeout_seconds,
            ),
        },
        None => {
            eprintln!("error: missing command");
            print_help();
            bail!("missing command");
        }
    }
}

fn print_help() {
    println!("Loop Engine xtask");
    println!();
    println!("Usage: xtask <command>");
    println!();
    println!("Commands:");
    println!("  help           Show available commands");
    println!("  architecture   Verify crate-level product dependency architecture");
    println!("  docs-check     Verify deterministic documentation formatting and link policy");
    println!("  dependencies   Verify dependency license/source/lockfile policy (T030)");
    println!("  judge          Run semantic judge (`judge --staged` after T024)");
    println!("  quality        Run currently-implemented quality manifest checks (T028)");
    println!("  publication    Exact-commit publication / pre-push gate (T028)");
    println!("  hooks          Install/verify/run versioned local git hooks (T027/T028)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_succeeds() {
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
}
