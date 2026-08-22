use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use bookends_check::{check_repo, validate_candidate, CheckStatus};

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
    Ok(Invocation::Check(Opts { repo, bypass }))
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

fn print_help() {
    println!(
        "Usage: bookends-check [--repo <path>] [--bypass <class>:<reason>]\n\
         Usage: bookends-check candidate <markdown-path>\n\n\
         Evaluate the enabled bookends graph, or parse-only validate a PRD\
         candidate. First stdout line is GREEN, RED, or BYPASS."
    );
}

fn print_help_stderr() {
    let _ = writeln!(
        io::stderr(),
        "Usage: bookends-check [--repo <path>] [--bypass <class>:<reason>]\n\
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
