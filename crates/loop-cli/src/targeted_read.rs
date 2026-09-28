//! Bounded provider-free history, assignment, attempt, and original-stream reads.
use super::{resolve_paths, CliError, CliOptions, Execution, OutputFormat, EXIT_COMPLETED};
use loop_core::{PersistenceError, RunId};
use loop_integrations::{SqlitePersistence, TargetedReadRequest};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const HELP: &str = "Usage: loop-engine read [--database DB] [--json] RUN_ID --kind history|delta|assignment|error|attempt|stdout|stderr [--assignment ID] [--invocation ID] [--cursor N] [--attempt N] [--offset N] [--limit N]\nHistory cursors are semantic sequence numbers. Assignment/attempt cursors are numeric page offsets. Stream output is exact hex-encoded original bytes.\n";
const READ_BUDGET: Duration = Duration::from_millis(1100);

pub(super) fn dispatch(args: &[String]) -> Option<Execution> {
    if args.first().map(String::as_str) == Some("read") {
        return Some(execute(args));
    }
    let mut prefix = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "read" => {
                let mut routed = vec!["read".to_owned()];
                routed.extend(prefix);
                routed.extend_from_slice(&args[index + 1..]);
                return Some(execute(&routed));
            }
            "--json" | "-j" | "--machine-readable" => {
                prefix.push("--json".to_owned());
                index += 1;
            }
            "--database" | "--timeout-ms" | "--config" | "--provider-timeout-ms" => {
                let value = args.get(index + 1)?;
                if args[index] == "--database" {
                    prefix.extend(["--database".to_owned(), value.clone()]);
                }
                index += 2;
            }
            token if token.starts_with("--database=") => {
                prefix.extend([
                    "--database".to_owned(),
                    token["--database=".len()..].to_owned(),
                ]);
                index += 1;
            }
            token if token.starts_with("--format=") || token.starts_with("--output=") => {
                prefix.push("--json".to_owned());
                index += 1;
            }
            _ => return None,
        }
    }
    None
}

fn execute(args: &[String]) -> Execution {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Execution {
            exit_code: EXIT_COMPLETED,
            stdout: HELP.to_owned(),
            stderr: String::new(),
        };
    }
    match parse(&args[1..]) {
        Ok((options, run_id, request)) => {
            let paths = match resolve_paths(&options) {
                Ok(paths) => paths,
                Err(error) => return super::render_operation_error("read", options.output, error),
            };
            let deadline = Instant::now() + READ_BUDGET;
            let persistence =
                match SqlitePersistence::open_for_observation(&paths.database, deadline, false) {
                    Ok(persistence) => persistence,
                    Err(error) => {
                        return super::render_operation_error(
                            "read",
                            options.output,
                            persistence_error(error),
                        )
                    }
                };
            match persistence.read_targeted(&run_id, &request, deadline) {
                Ok(result) => Execution {
                    exit_code: EXIT_COMPLETED,
                    stdout: format!(
                        "{}\n",
                        json!({"operation":"read","status":"completed","result":result})
                    ),
                    stderr: String::new(),
                },
                Err(error) => {
                    super::render_operation_error("read", options.output, persistence_error(error))
                }
            }
        }
        Err(error) => super::render_invalid_invocation_with_format(error, OutputFormat::Json),
    }
}

fn parse(args: &[String]) -> Result<(CliOptions, RunId, TargetedReadRequest), CliError> {
    let mut database = None;
    let mut run_id = None;
    let mut request = TargetedReadRequest {
        limit: 100,
        ..TargetedReadRequest::default()
    };
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if token == "--json" || token == "-j" {
            index += 1;
            continue;
        }
        if !token.starts_with('-') && run_id.is_none() {
            run_id = Some(RunId::from(token.clone()));
            index += 1;
            continue;
        }
        let key = token.as_str();
        index += 1;
        let value = args.get(index).ok_or_else(|| {
            CliError::new("invalid-invocation", format!("missing value for {key}"))
        })?;
        match key {
            "--database" => database = Some(PathBuf::from(value)),
            "--kind" => request.kind = value.clone(),
            "--assignment" => request.assignment_id = Some(value.clone()),
            "--invocation" => request.invocation_id = Some(value.clone()),
            "--cursor" => request.cursor = Some(value.clone()),
            "--attempt" => request.attempt = Some(parse_u32(value, key)?),
            "--offset" => request.offset = parse_u64(value, key)?,
            "--limit" => request.limit = parse_u32(value, key)?,
            _ => {
                return Err(CliError::new(
                    "invalid-invocation",
                    format!("unknown read option `{key}`"),
                ))
            }
        }
        index += 1;
    }
    let run_id =
        run_id.ok_or_else(|| CliError::new("invalid-invocation", "read requires RUN_ID"))?;
    if !matches!(
        request.kind.as_str(),
        "history" | "delta" | "assignment" | "error" | "attempt" | "stdout" | "stderr"
    ) {
        return Err(CliError::new(
            "invalid-invocation",
            "--kind must be history, delta, assignment, error, attempt, stdout, or stderr",
        ));
    }
    if matches!(
        request.kind.as_str(),
        "assignment" | "error" | "attempt" | "stdout" | "stderr"
    ) && request.assignment_id.as_deref().is_none_or(str::is_empty)
    {
        return Err(CliError::new(
            "invalid-invocation",
            "this read kind requires --assignment ID",
        ));
    }
    if matches!(request.kind.as_str(), "stdout" | "stderr")
        && request.invocation_id.as_deref().is_none_or(str::is_empty)
    {
        return Err(CliError::new(
            "invalid-invocation",
            "stdout/stderr reads require --invocation ID",
        ));
    }
    if request.kind != "stdout" && request.kind != "stderr" && request.attempt.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "--attempt is valid only for stdout/stderr reads",
        ));
    }
    if request.kind != "stdout" && request.kind != "stderr" && request.offset != 0 {
        return Err(CliError::new(
            "invalid-invocation",
            "--offset is valid only for stdout/stderr reads",
        ));
    }
    let options = CliOptions {
        output: OutputFormat::Json,
        database,
        ..CliOptions::default()
    };
    Ok((options, run_id, request))
}

fn persistence_error(error: PersistenceError) -> CliError {
    match error {
        PersistenceError::Failure(failure) => CliError::new(failure.code, failure.message),
        other => CliError::new(other.code(), other.to_string()),
    }
}

fn parse_u32(value: &str, option: &str) -> Result<u32, CliError> {
    value.parse().map_err(|_| {
        CliError::new(
            "invalid-invocation",
            format!("{option} must be a non-negative integer"),
        )
    })
}

fn parse_u64(value: &str, option: &str) -> Result<u64, CliError> {
    value.parse().map_err(|_| {
        CliError::new(
            "invalid-invocation",
            format!("{option} must be a non-negative integer"),
        )
    })
}
