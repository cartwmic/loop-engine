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
    let started = std::time::Instant::now();
    let invoked_at = std::time::SystemTime::now();
    let mut action = ActionLog::new(invoked_at);
    let code = match run(env::args().collect(), invoked_at, &mut action) {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(io::stderr(), "{err}");
            action.outcome = "error";
            action.error = Some(err.to_string());
            ExitCode::from(1)
        }
    };
    action.write(started.elapsed());
    code
}

fn run(
    args: Vec<String>,
    invoked_at: std::time::SystemTime,
    action: &mut ActionLog,
) -> Result<ExitCode, io::Error> {
    let invocation = match parse_args(&args) {
        Ok(invocation) => invocation,
        Err(ParseOutcome::Help) => {
            print_help();
            return Ok(ExitCode::from(0));
        }
        Err(ParseOutcome::Version) => {
            println!("bookends-check {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::from(0));
        }
        Err(ParseOutcome::Usage(msg)) => {
            let _ = writeln!(io::stderr(), "{msg}");
            print_help_stderr();
            action.skip = true;
            return Ok(ExitCode::from(2));
        }
    };

    if let Invocation::Candidate(path) = invocation {
        action.action = "candidate";
        return run_candidate(&path, action);
    }
    let Invocation::Check(opts) = invocation else {
        unreachable!("candidate handled above");
    };
    let repo = match opts.repo {
        Some(path) => path,
        None => env::current_dir()?,
    };
    let bypass = opts.bypass.as_ref().map(|(c, r)| (c.as_str(), r.as_str()));
    action.action = "check";
    action.repo = Some(repo.display().to_string());
    action.receipt_root = opts.receipt_root.clone();

    if opts.updates_stdin {
        let mut input = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut input) {
            let mut stdout = io::stdout();
            writeln!(stdout, "RED")?;
            writeln!(stdout, "cannot read pre-push updates: {error}")?;
            action.red(error.to_string());
            return Ok(ExitCode::from(1));
        }
        let updates = match parse_ref_updates(&input) {
            Ok(updates) => updates,
            Err(error) => {
                let mut stdout = io::stdout();
                writeln!(stdout, "RED")?;
                writeln!(stdout, "invalid pre-push updates: {error}")?;
                action.red(error.to_string());
                return Ok(ExitCode::from(1));
            }
        };
        if !updates.is_empty() {
            action.action = "publication";
            action.refs = updates.iter().map(|u| u.remote_ref.clone()).collect();
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
                    action.red(error.to_string());
                    return Ok(ExitCode::from(1));
                }
            };
            return run_publication(
                &repo,
                &report,
                &options,
                opts.receipt_root.as_deref(),
                invoked_at,
                action,
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
            action.red(err.to_string());
            return Ok(ExitCode::from(1));
        }
    };
    action.findings = report.findings.len();
    match &report.status {
        CheckStatus::Green => {
            writeln!(stdout, "GREEN")?;
            action.outcome = "green";
            Ok(ExitCode::from(0))
        }
        CheckStatus::Red => {
            writeln!(stdout, "RED")?;
            for finding in &report.findings {
                writeln!(stdout, "{finding}")?;
            }
            action.outcome = "red";
            Ok(ExitCode::from(1))
        }
        CheckStatus::Bypass { class, reason } => {
            action.bypass_class = Some(class.clone());
            match retain_bypass(
                &repo,
                opts.receipt_root.as_deref(),
                class,
                reason,
                invoked_at,
                &action.id,
            ) {
                Ok(path) => action.receipt = Some(path),
                Err(error) => {
                    writeln!(stdout, "RED")?;
                    writeln!(
                        stdout,
                        "bypass receipt unavailable; permission refused: {error}"
                    )?;
                    action.refused(error.to_string());
                    return Ok(ExitCode::from(1));
                }
            }
            action.outcome = "bypass";
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
    Version,
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
            "-V" | "--version" => return Err(ParseOutcome::Version),
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

fn run_candidate(path: &PathBuf, action: &mut ActionLog) -> Result<ExitCode, io::Error> {
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
                action.red(error.to_string());
                return Ok(ExitCode::from(1));
            }
        }
    };
    match validate_candidate(&text) {
        Ok(_) => {
            println!("GREEN");
            action.outcome = "green";
            Ok(ExitCode::from(0))
        }
        Err(findings) => {
            action.outcome = "red";
            action.findings = findings.len();
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
    run_id: String,
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
    action: &mut ActionLog,
) -> Result<ExitCode, io::Error> {
    action.remote = Some(options.remote.clone());
    action.findings = report.findings.len();
    action.commits_checked = report.checked_commits.len();
    action.complete = Some(report.complete);
    match retain_publication_receipt(repo, report, options, receipt_root, invoked_at, &action.id) {
        Ok(path) => action.receipt = Some(path),
        Err(error) => {
            let mut stdout = io::stdout();
            writeln!(stdout, "RED")?;
            writeln!(
                stdout,
                "publication receipt unavailable; permission refused: {error}"
            )?;
            action.refused(error.to_string());
            return Ok(ExitCode::from(1));
        }
    }

    let mut stdout = io::stdout();
    match &report.status {
        CheckStatus::Green => {
            action.outcome = "green";
            writeln!(stdout, "GREEN")?;
            for diagnostic in &report.diagnostics {
                writeln!(stdout, "note: {diagnostic}")?;
            }
            Ok(ExitCode::from(0))
        }
        CheckStatus::Red => {
            action.outcome = "red";
            writeln!(stdout, "RED")?;
            for finding in &report.findings {
                writeln!(stdout, "{finding}")?;
            }
            Ok(ExitCode::from(1))
        }
        CheckStatus::Bypass { class, reason } => {
            action.bypass_class = Some(class.clone());
            // The bypass receipt is a second record; the log names the
            // publication receipt, which carries the same id.
            if let Err(error) =
                retain_bypass(repo, receipt_root, class, reason, invoked_at, &action.id)
            {
                writeln!(stdout, "RED")?;
                writeln!(
                    stdout,
                    "bypass receipt unavailable; permission refused: {error}"
                )?;
                action.refused(error.to_string());
                return Ok(ExitCode::from(1));
            }
            action.outcome = "bypass";
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
    run_id: &str,
) -> io::Result<PathBuf> {
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
        run_id: run_id.to_owned(),
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
    Ok(path)
}

// A receipt is write-once, synced before permission, and outside the checkout.
fn retain_bypass(
    repo: &std::path::Path,
    root: Option<&std::path::Path>,
    class: &str,
    reason: &str,
    invoked_at: std::time::SystemTime,
    run_id: &str,
) -> io::Result<PathBuf> {
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
        ("run_id", run_id.to_owned()),
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
    Ok(path)
}

/// One structured log line per run of a check or candidate command, kept
/// beside the receipts. It never changes the verdict: a failed write is
/// reported on stderr only.
struct ActionLog {
    id: String,
    invoked_at: std::time::SystemTime,
    action: &'static str,
    outcome: &'static str,
    skip: bool,
    repo: Option<String>,
    remote: Option<String>,
    refs: Vec<String>,
    findings: usize,
    commits_checked: usize,
    complete: Option<bool>,
    bypass_class: Option<String>,
    receipt: Option<PathBuf>,
    receipt_root: Option<PathBuf>,
    error: Option<String>,
}

impl ActionLog {
    fn new(invoked_at: std::time::SystemTime) -> Self {
        let nanos = invoked_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self {
            // Same shape as the receipt file name stem, so the id is
            // unique per process and run.
            id: format!("{nanos}-{}", std::process::id()),
            invoked_at,
            action: "check",
            outcome: "error",
            skip: false,
            repo: None,
            remote: None,
            refs: Vec::new(),
            findings: 0,
            commits_checked: 0,
            complete: None,
            bypass_class: None,
            receipt: None,
            receipt_root: None,
            error: None,
        }
    }

    fn red(&mut self, error: String) {
        self.outcome = "red";
        self.error = Some(error);
    }

    fn refused(&mut self, error: String) {
        self.outcome = "refused";
        self.error = Some(error);
    }

    fn log_path(&self) -> io::Result<PathBuf> {
        if let Some(root) = &self.receipt_root {
            return Ok(root.parent().unwrap_or(root).join("log.jsonl"));
        }
        Ok(env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .ok_or_else(|| io::Error::other("no local state directory"))?
            .join("bookends-check/log.jsonl"))
    }

    fn write(&self, duration: std::time::Duration) {
        if self.skip {
            return;
        }
        if let Err(error) = self.append(duration) {
            let _ = writeln!(
                io::stderr(),
                "bookends-check: action log unavailable: {error}"
            );
        }
    }

    fn append(&self, duration: std::time::Duration) -> io::Result<()> {
        let path = self.log_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = serde_json::json!({
            "ts": utc_timestamp(self.invoked_at),
            "action": self.action,
            "outcome": self.outcome,
            "duration_ms": u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            "id": self.id,
            "repo": self.repo,
            "remote": self.remote,
            "refs": self.refs,
            "findings": self.findings,
            "commits_checked": self.commits_checked,
            "complete": self.complete,
            "bypass_class": self.bypass_class,
            "receipt": self.receipt.as_ref().map(|p| p.display().to_string()),
            "error": self.error,
        });
        let mut line = serde_json::to_vec(&record).map_err(io::Error::other)?;
        line.push(b'\n');
        // One write call on an append handle keeps concurrent runs' lines whole.
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(&line)
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` in UTC.
fn utc_timestamp(time: std::time::SystemTime) -> String {
    let since = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60,
        since.subsec_millis()
    )
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
         permission also requires a synced write-once receipt.\n\
         Every run of a check or candidate command appends one JSON line to\n\
         log.jsonl in the receipt root's parent directory (default\n\
         $XDG_STATE_HOME/bookends-check/log.jsonl). Its run_id matches the\n\
         receipt's. A log write failure is reported on stderr and never\n\
         changes the verdict."
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
    fn utc_timestamp_is_calendar_correct() {
        let at = |secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        assert_eq!(utc_timestamp(at(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_timestamp(at(951_782_400)), "2000-02-29T00:00:00.000Z");
        assert_eq!(utc_timestamp(at(1_795_779_078)), "2026-11-27T11:31:18.000Z");
    }

    #[test]
    fn parse_version() {
        for flag in ["--version", "-V"] {
            let args = vec!["bookends-check".into(), flag.into()];
            assert!(matches!(parse_args(&args), Err(ParseOutcome::Version)));
        }
    }

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
