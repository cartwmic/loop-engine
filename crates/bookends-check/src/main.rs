use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use bookends_check::{
    check_publication, check_repo, parse_ref_updates, validate_candidate, CheckStatus,
    PublicationOptions, PublicationReport,
};
use serde::Serialize;

fn main() -> ExitCode {
    match run(env::args().collect()) {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(io::stderr(), "{err}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, io::Error> {
    let invoked_at = std::time::SystemTime::now();
    let invocation = match parse_args(&args) {
        Ok(invocation) => invocation,
        Err(ParseOutcome::Help) => {
            print_help();
            return Ok(ExitCode::from(0));
        }
        Err(ParseOutcome::Usage(msg)) => {
            let _ = writeln!(io::stderr(), "{msg}");
            print_help_stderr();
            return Ok(ExitCode::from(2));
        }
    };

    if let Invocation::Candidate(path) = invocation {
        return run_candidate(&path);
    }
    let Invocation::Check(opts) = invocation else {
        unreachable!("candidate handled above");
    };
    let repo = match opts.repo {
        Some(path) => path,
        None => env::current_dir()?,
    };
    let bypass = opts.bypass.as_ref().map(|(c, r)| (c.as_str(), r.as_str()));

    if opts.updates_stdin {
        let mut input = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut input) {
            let mut stdout = io::stdout();
            writeln!(stdout, "RED")?;
            writeln!(stdout, "cannot read pre-push updates: {error}")?;
            return Ok(ExitCode::from(1));
        }
        let updates = match parse_ref_updates(&input) {
            Ok(updates) => updates,
            Err(error) => {
                let mut stdout = io::stdout();
                writeln!(stdout, "RED")?;
                writeln!(stdout, "invalid pre-push updates: {error}")?;
                return Ok(ExitCode::from(1));
            }
        };
        if !updates.is_empty() {
            let options = PublicationOptions {
                remote: opts.remote.unwrap_or_else(|| "origin".to_owned()),
                max_commits: opts.max_commits,
            };
            let report = match check_publication(&repo, &updates, &options, bypass) {
                Ok(report) => report,
                Err(error) => {
                    let mut stdout = io::stdout();
                    writeln!(stdout, "RED")?;
                    writeln!(stdout, "{error}")?;
                    return Ok(ExitCode::from(1));
                }
            };
            return run_publication(
                &repo,
                &report,
                &options,
                opts.receipt_root.as_deref(),
                invoked_at,
            );
        }
    }

    let mut stdout = io::stdout();
    let report = match check_repo(&repo, bypass) {
        Ok(report) => report,
        Err(err) => {
            // A repository-root I/O error cannot produce a graph report, but
            // the CLI still fails closed with the required status line.
            writeln!(stdout, "RED")?;
            writeln!(stdout, "{err}")?;
            return Ok(ExitCode::from(1));
        }
    };
    match &report.status {
        CheckStatus::Green => {
            writeln!(stdout, "GREEN")?;
            Ok(ExitCode::from(0))
        }
        CheckStatus::Red => {
            writeln!(stdout, "RED")?;
            for finding in &report.findings {
                writeln!(stdout, "{finding}")?;
            }
            Ok(ExitCode::from(1))
        }
        CheckStatus::Bypass { class, reason } => {
            if let Err(error) = retain_bypass(
                &repo,
                opts.receipt_root.as_deref(),
                class,
                reason,
                invoked_at,
            ) {
                writeln!(stdout, "RED")?;
                writeln!(
                    stdout,
                    "bypass receipt unavailable; permission refused: {error}"
                )?;
                return Ok(ExitCode::from(1));
            }
            writeln!(stdout, "BYPASS")?;
            writeln!(stdout, "{class}")?;
            writeln!(stdout, "{reason}")?;
            Ok(ExitCode::from(0))
        }
    }
}

#[derive(Debug)]
struct Opts {
    repo: Option<PathBuf>,
    bypass: Option<(String, String)>,
    receipt_root: Option<PathBuf>,
    updates_stdin: bool,
    remote: Option<String>,
    max_commits: usize,
}

#[derive(Debug)]
enum Invocation {
    Check(Opts),
    Candidate(PathBuf),
}

#[derive(Debug)]
enum ParseOutcome {
    Help,
    Usage(String),
}

fn parse_args(args: &[String]) -> Result<Invocation, ParseOutcome> {
    if matches!(
        args.get(1).map(String::as_str),
        Some("candidate" | "validate-candidate")
    ) {
        let path = args
            .get(2)
            .ok_or_else(|| ParseOutcome::Usage("candidate requires a markdown path".into()))?;
        if args.len() != 3 || (path.starts_with('-') && path != "-") {
            return Err(ParseOutcome::Usage(
                "candidate requires exactly one markdown path".into(),
            ));
        }
        return Ok(Invocation::Candidate(PathBuf::from(path)));
    }
    let mut repo = None;
    let mut bypass = None;
    let mut receipt_root = None;
    let mut updates_stdin = false;
    let mut remote = None;
    let mut max_commits = bookends_check::DEFAULT_MAX_COMMITS;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Err(ParseOutcome::Help),
            "--repo" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| ParseOutcome::Usage("missing value for --repo".into()))?;
                if value.starts_with('-') {
                    return Err(ParseOutcome::Usage("missing value for --repo".into()));
                }
                repo = Some(PathBuf::from(value));
            }
            "--receipt-root" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| {
                    ParseOutcome::Usage("missing value for --receipt-root".into())
                })?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(ParseOutcome::Usage(
                        "--receipt-root must be absolute".into(),
                    ));
                }
                receipt_root = Some(path);
            }
            "--updates-stdin" => {
                if updates_stdin {
                    return Err(ParseOutcome::Usage(
                        "--updates-stdin may be supplied only once".into(),
                    ));
                }
                updates_stdin = true;
            }
            "--remote" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| ParseOutcome::Usage("missing value for --remote".into()))?;
                if value.is_empty() || value.starts_with('-') {
                    return Err(ParseOutcome::Usage("invalid value for --remote".into()));
                }
                remote = Some(value.to_owned());
            }
            "--max-commits" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| ParseOutcome::Usage("missing value for --max-commits".into()))?;
                max_commits = value.parse().map_err(|_| {
                    ParseOutcome::Usage("--max-commits requires a positive integer".into())
                })?;
                if max_commits == 0 {
                    return Err(ParseOutcome::Usage(
                        "--max-commits requires a positive integer".into(),
                    ));
                }
            }
            "--bypass" => {
                if bypass.is_some() {
                    return Err(ParseOutcome::Usage(
                        "--bypass may be supplied only once".into(),
                    ));
                }
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| ParseOutcome::Usage("missing value for --bypass".into()))?;
                bypass = Some(parse_bypass(value)?);
            }
            other => {
                return Err(ParseOutcome::Usage(format!("unknown argument: {other}")));
            }
        }
        i += 1;
    }
    Ok(Invocation::Check(Opts {
        repo,
        bypass,
        receipt_root,
        updates_stdin,
        remote,
        max_commits,
    }))
}

fn run_candidate(path: &PathBuf) -> Result<ExitCode, io::Error> {
    let text = if path == std::path::Path::new("-") {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        text
    } else {
        match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                let mut stdout = io::stdout();
                writeln!(stdout, "RED")?;
                writeln!(stdout, "cannot read candidate {}: {error}", path.display())?;
                return Ok(ExitCode::from(1));
            }
        }
    };
    match validate_candidate(&text) {
        Ok(_) => {
            println!("GREEN");
            Ok(ExitCode::from(0))
        }
        Err(findings) => {
            let mut stdout = io::stdout();
            writeln!(stdout, "RED")?;
            for finding in findings {
                writeln!(stdout, "{finding}")?;
            }
            Ok(ExitCode::from(1))
        }
    }
}

fn parse_bypass(value: &str) -> Result<(String, String), ParseOutcome> {
    let Some((class, reason)) = value.split_once(':') else {
        return Err(ParseOutcome::Usage(
            "--bypass requires <class>:<reason>".into(),
        ));
    };
    if class.is_empty() || reason.is_empty() {
        return Err(ParseOutcome::Usage(
            "--bypass requires non-empty class and reason".into(),
        ));
    }
    Ok((class.to_string(), reason.to_string()))
}

#[derive(Serialize)]
struct PublicationReceipt {
    schema: u32,
    invoked_at_utc_unix_seconds: String,
    repository: String,
    remote: String,
    max_commits: usize,
    outcome: String,
    complete: bool,
    updates: Vec<bookends_check::RefUpdate>,
    ranges: Vec<bookends_check::PublicationRange>,
    enumerated_commits: Vec<String>,
    diagnostics: Vec<String>,
}

fn run_publication(
    repo: &std::path::Path,
    report: &PublicationReport,
    options: &PublicationOptions,
    receipt_root: Option<&std::path::Path>,
    invoked_at: std::time::SystemTime,
) -> Result<ExitCode, io::Error> {
    if let Err(error) = retain_publication_receipt(repo, report, options, receipt_root, invoked_at)
    {
        let mut stdout = io::stdout();
        writeln!(stdout, "RED")?;
        writeln!(
            stdout,
            "publication receipt unavailable; permission refused: {error}"
        )?;
        return Ok(ExitCode::from(1));
    }

    let mut stdout = io::stdout();
    match &report.status {
        CheckStatus::Green => {
            writeln!(stdout, "GREEN")?;
            Ok(ExitCode::from(0))
        }
        CheckStatus::Red => {
            writeln!(stdout, "RED")?;
            for finding in &report.findings {
                writeln!(stdout, "{finding}")?;
            }
            Ok(ExitCode::from(1))
        }
        CheckStatus::Bypass { class, reason } => {
            if let Err(error) = retain_bypass(repo, receipt_root, class, reason, invoked_at) {
                writeln!(stdout, "RED")?;
                writeln!(
                    stdout,
                    "bypass receipt unavailable; permission refused: {error}"
                )?;
                return Ok(ExitCode::from(1));
            }
            writeln!(stdout, "BYPASS")?;
            writeln!(stdout, "{class}")?;
            writeln!(stdout, "{reason}")?;
            Ok(ExitCode::from(0))
        }
    }
}

fn retain_publication_receipt(
    repo: &std::path::Path,
    report: &PublicationReport,
    options: &PublicationOptions,
    root: Option<&std::path::Path>,
    invoked_at: std::time::SystemTime,
) -> io::Result<()> {
    use std::process::Command;
    use std::time::UNIX_EPOCH;

    let repository = fs::canonicalize(repo)?;
    let root = match root {
        Some(root) => root.to_path_buf(),
        None => env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .ok_or_else(|| io::Error::other("no local state directory"))?
            .join("bookends-check/gates"),
    };
    fs::create_dir_all(&root)?;
    let root = fs::canonicalize(root)?;
    let git_root = Command::new("git")
        .current_dir(&repository)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if git_root.status.success()
        && root.starts_with(fs::canonicalize(
            String::from_utf8_lossy(&git_root.stdout).trim(),
        )?)
    {
        return Err(io::Error::other(
            "receipt root must be outside the repository",
        ));
    }

    let now = invoked_at
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let outcome = match &report.status {
        CheckStatus::Green => "GREEN",
        CheckStatus::Red => "RED",
        CheckStatus::Bypass { .. } => "BYPASS",
    };
    let receipt = PublicationReceipt {
        schema: 1,
        invoked_at_utc_unix_seconds: format!("{}.{:09}", now.as_secs(), now.subsec_nanos()),
        repository: repository.display().to_string(),
        remote: options.remote.clone(),
        max_commits: options.max_commits,
        outcome: outcome.to_owned(),
        complete: report.complete,
        updates: report.updates.clone(),
        ranges: report.ranges.clone(),
        enumerated_commits: report.checked_commits.clone(),
        diagnostics: report.diagnostics.clone(),
    };
    let bytes = serde_yaml::to_string(&receipt).map_err(io::Error::other)?;
    let path = root.join(format!(
        "{}-{}-gate.yaml",
        now.as_nanos(),
        std::process::id()
    ));
    let pending = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    file.write_all(bytes.as_bytes())?;
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)?;
    file.sync_all()?;
    fs::hard_link(&pending, &path)?;
    fs::remove_file(pending)?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

// A receipt is write-once, synced before permission, and outside the checkout.
fn retain_bypass(
    repo: &std::path::Path,
    root: Option<&std::path::Path>,
    class: &str,
    reason: &str,
    invoked_at: std::time::SystemTime,
) -> io::Result<()> {
    use std::process::Command;
    use std::time::UNIX_EPOCH;
    let repository = fs::canonicalize(repo)?;
    let root = match root {
        Some(root) => root.to_path_buf(),
        None => env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .ok_or_else(|| io::Error::other("no local state directory"))?
            .join("bookends-check/bypasses"),
    };
    fs::create_dir_all(&root)?;
    let root = fs::canonicalize(root)?;
    let git_root = Command::new("git")
        .current_dir(&repository)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if git_root.status.success()
        && root.starts_with(fs::canonicalize(
            String::from_utf8_lossy(&git_root.stdout).trim(),
        )?)
    {
        return Err(io::Error::other(
            "receipt root must be outside the repository",
        ));
    }
    let revision = Command::new("git")
        .current_dir(&repository)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?;
    let revision = if revision.status.success() {
        String::from_utf8_lossy(&revision.stdout).trim().to_owned()
    } else {
        format!(
            "unavailable: {}",
            String::from_utf8_lossy(&revision.stderr).trim()
        )
    };
    let now = invoked_at
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let receipt = std::collections::BTreeMap::from([
        (
            "invoked_at_utc_unix_seconds",
            format!("{}.{:09}", now.as_secs(), now.subsec_nanos()),
        ),
        ("repository", repository.display().to_string()),
        ("revision", revision),
        ("class", class.to_owned()),
        ("reason", reason.to_owned()),
        ("outcome", "BYPASS".to_owned()),
    ]);
    let bytes = serde_yaml::to_string(&receipt).map_err(io::Error::other)?;
    let path = root.join(format!("{}-{}.yaml", now.as_nanos(), std::process::id()));
    let pending = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    file.write_all(bytes.as_bytes())?;
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)?;
    file.sync_all()?;
    // Publish a complete receipt without ever replacing an existing record.
    fs::hard_link(&pending, &path)?;
    fs::remove_file(pending)?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

fn print_help() {
    println!(
        "Usage: bookends-check [--repo <path>] [--updates-stdin] [--remote <name>] [--max-commits <n>] [--bypass <class>:<reason>]\n\
         Usage: bookends-check candidate <markdown-path>\n\n\
         Evaluate the enabled bookends graph, or parse-only validate a PRD\n\
         candidate. With --updates-stdin, read Git pre-push updates and check\n\
         every introduced historical commit. First stdout line is GREEN, RED,\n\
         or BYPASS.\n\
         --receipt-root <absolute-path> overrides local receipt storage.\n\
         Publication checks retain a synced range/completeness receipt; bypass\n\
         permission also requires a synced write-once receipt."
    );
}

fn print_help_stderr() {
    let _ = writeln!(
        io::stderr(),
        "Usage: bookends-check [--repo <path>] [--updates-stdin] [--remote <name>] [--max-commits <n>] [--bypass <class>:<reason>]\n\
         bookends-check candidate <markdown-path>"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_help() {
        let args = vec!["bookends-check".into(), "--help".into()];
        assert!(matches!(parse_args(&args), Err(ParseOutcome::Help)));
    }

    #[test]
    fn parse_candidate_command() {
        let args = vec![
            "bookends-check".into(),
            "candidate".into(),
            "draft.md".into(),
        ];
        assert!(matches!(
            parse_args(&args),
            Ok(Invocation::Candidate(path)) if path == std::path::Path::new("draft.md")
        ));
    }

    #[test]
    fn parse_unknown_is_usage() {
        let args = vec!["bookends-check".into(), "--nope".into()];
        assert!(matches!(parse_args(&args), Err(ParseOutcome::Usage(_))));
    }

    #[test]
    fn parse_bypass_split() {
        let args = vec![
            "bookends-check".into(),
            "--bypass".into(),
            "test:reason".into(),
        ];
        let Invocation::Check(opts) = parse_args(&args).unwrap() else {
            panic!("expected check invocation");
        };
        assert_eq!(
            opts.bypass.as_ref().map(|(c, r)| (c.as_str(), r.as_str())),
            Some(("test", "reason"))
        );
    }

    #[test]
    fn repo_flag_cannot_consume_another_flag() {
        let args = vec!["bookends-check".into(), "--repo".into(), "--help".into()];
        assert!(matches!(parse_args(&args), Err(ParseOutcome::Usage(_))));
    }

    #[test]
    fn bypass_is_single_explicit_override() {
        let args = vec![
            "bookends-check".into(),
            "--bypass".into(),
            "ci:first".into(),
            "--bypass".into(),
            "ci:second".into(),
        ];
        assert!(matches!(parse_args(&args), Err(ParseOutcome::Usage(_))));
    }
}
