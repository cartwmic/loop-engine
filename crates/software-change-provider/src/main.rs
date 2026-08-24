//! `software-change` subprocess provider.
//!
//! One process handles one request.  Protocol errors use exit 2, evaluation
//! errors use exit 1, and a successfully written result uses exit 0.

mod artifacts;
mod checkpoint;
mod config;
mod dagu;
mod evidence;
mod finding_ledger;
mod gates;
mod overlay;
mod protocol;
mod run_plan_graph;
mod schema;
mod workflow;

use protocol::{DescribeRequest, EvaluateRequest};
use run_plan_graph::{parse_run_plan_graph_args, MAX_CONCURRENCY};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

const STDIN_EXEC_USAGE: &str =
    "usage: software-change stdin-exec --stdin-file ABS --exit-mode sidecar|propagate [--sidecar-file ABS] -- COMMAND [ARG]...";
const EXIT_STDIN_EXEC_ERROR: i32 = 20;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let mut args = std::env::args_os();
    let _program = args.next();
    match args.next() {
        Some(command) if command == "--help" || command == "-h" => {
            if args.next().is_some() {
                return data_dump_usage("help accepts no additional arguments");
            }
            return provider_help();
        }
        Some(command) if command == "--version" || command == "-V" => {
            if args.next().is_some() {
                return data_dump_usage("version accepts no additional arguments");
            }
            println!("software-change {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        Some(command) if command == "data-dump" => {
            let Some(destination) = args.next() else {
                return data_dump_usage("missing destination directory");
            };
            if args.next().is_some() {
                return data_dump_usage("data-dump accepts exactly one destination directory");
            }
            return match software_change_provider::embedded_data::dump(Path::new(&destination)) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("data-dump failed: {error}");
                    1
                }
            };
        }
        Some(command) if command == "checkpoint" => {
            let rest = match args
                .map(|token| token.into_string())
                .collect::<Result<Vec<String>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!("checkpoint arguments must be valid UTF-8");
                    return 2;
                }
            };
            return match parse_checkpoint_args(&rest) {
                Ok(parsed) => match checkpoint::create(
                    parsed.phase,
                    &parsed.artifact_root,
                    &parsed.working_directory,
                ) {
                    Ok(result) => write_json(&result),
                    Err(error) => {
                        eprintln!("checkpoint failed: {error}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!(
                        "{error}; usage: software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS"
                    );
                    2
                }
            };
        }
        Some(command) if command == "run-plan-graph" => {
            let rest = match args
                .map(|token| token.into_string())
                .collect::<Result<Vec<String>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!(
                        "run-plan-graph arguments must be valid UTF-8; usage: software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--max-active N]"
                    );
                    return 2;
                }
            };
            return match parse_run_plan_graph_args(&rest) {
                Ok(parsed) => {
                    if let Err(error) = dagu::resolve_dagu() {
                        eprintln!("{error}");
                        return 1;
                    }
                    run_plan_graph::execute(&parsed)
                }
                Err(error) => {
                    eprintln!(
                        "{error}; usage: software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--max-active N]"
                    );
                    2
                }
            };
        }
        Some(command) if command == "stdin-exec" => {
            let rest = match args
                .map(|token| token.into_string())
                .collect::<Result<Vec<String>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!("stdin-exec arguments must be valid UTF-8; {STDIN_EXEC_USAGE}");
                    return 2;
                }
            };
            return match parse_stdin_exec_args(&rest) {
                Ok(parsed) => execute_stdin_exec(parsed),
                Err(error) => {
                    eprintln!("{error}; {STDIN_EXEC_USAGE}");
                    2
                }
            };
        }
        Some(command) => {
            return data_dump_usage(&format!(
                "unsupported command `{}`",
                command.to_string_lossy()
            ));
        }
        None => {}
    }

    run_protocol()
}

fn run_protocol() -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        return protocol_error(format!("could not read request: {error}"));
    }

    let request = match serde_json::from_str::<Value>(&input) {
        Ok(request) => request,
        Err(error) => return protocol_error(format!("malformed JSON request: {error}")),
    };

    let operation = match request
        .as_object()
        .and_then(|object| object.get("operation"))
        .and_then(Value::as_str)
    {
        Some(operation) => operation.to_owned(),
        None => {
            return protocol_error("request must be a JSON object with a string `operation`".into())
        }
    };

    match operation.as_str() {
        "describe" => describe(request),
        "evaluate" => evaluate(request),
        other => protocol_error(format!("unsupported provider operation `{other}`")),
    }
}

fn provider_help() -> i32 {
    println!(
        "software-change\n\nUsage:\n  software-change < stdin\n  software-change data-dump DIR\n  software-change checkpoint --phase implementation|validation --artifact-root ABS --working-directory ABS\n  software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--max-active N]\n  software-change --help | -h\n  software-change --version | -V\n\nStdin operations:\n  describe   return workflow topology\n  evaluate   validate one checked transition\n\nData:\n  data-dump  materialize embedded provider data under DIR\n\nPlan graph:\n  run-plan-graph  requires --working-directory ABS (one existing driver-selected directory for every task and summarizer; no Git/worktree management) and executes plan.json as a Dagu type:graph (--max-active N; omitted means {MAX_CONCURRENCY} ordinary tasks) with a mandatory summarizer"
    );
    0
}

fn data_dump_usage(message: &str) -> i32 {
    eprintln!("{message}; usage: software-change data-dump DIR");
    2
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StdinExecExitMode {
    Sidecar,
    Propagate,
}

struct StdinExecArgs {
    stdin_file: PathBuf,
    exit_mode: StdinExecExitMode,
    sidecar_file: Option<PathBuf>,
    command: String,
    args: Vec<String>,
}

struct CheckpointArgs {
    phase: checkpoint::CheckpointPhase,
    artifact_root: PathBuf,
    working_directory: PathBuf,
}

fn parse_checkpoint_args(args: &[String]) -> Result<CheckpointArgs, String> {
    let mut phase = None;
    let mut artifact_root = None;
    let mut working_directory = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        let (name, inline) = if let Some(value) = token.strip_prefix("--phase=") {
            ("--phase", Some(value.to_owned()))
        } else if token == "--phase" {
            ("--phase", None)
        } else if let Some(value) = token.strip_prefix("--artifact-root=") {
            ("--artifact-root", Some(value.to_owned()))
        } else if token == "--artifact-root" {
            ("--artifact-root", None)
        } else if let Some(value) = token.strip_prefix("--working-directory=") {
            ("--working-directory", Some(value.to_owned()))
        } else if token == "--working-directory" {
            ("--working-directory", None)
        } else {
            return Err(format!("unknown or unexpected argument `{token}`"));
        };
        let value = match inline {
            Some(value) => value,
            None => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("option `{name}` requires a value"))?;
                if value.starts_with('-') && value != "-" {
                    return Err(format!("option `{name}` requires a value"));
                }
                index += 1;
                value.clone()
            }
        };
        match name {
            "--phase" => {
                if phase.is_some() {
                    return Err("`--phase` may be supplied at most once".to_owned());
                }
                phase = Some(checkpoint::CheckpointPhase::parse(&value)?);
            }
            "--artifact-root" => {
                if artifact_root.is_some() {
                    return Err("`--artifact-root` may be supplied at most once".to_owned());
                }
                artifact_root = Some(PathBuf::from(value));
            }
            "--working-directory" => {
                if working_directory.is_some() {
                    return Err("`--working-directory` may be supplied at most once".to_owned());
                }
                working_directory = Some(PathBuf::from(value));
            }
            _ => unreachable!(),
        }
        index += 1;
    }
    let phase = phase.ok_or_else(|| "missing required option `--phase`".to_owned())?;
    let artifact_root =
        artifact_root.ok_or_else(|| "missing required option `--artifact-root`".to_owned())?;
    let working_directory = working_directory
        .ok_or_else(|| "missing required option `--working-directory`".to_owned())?;
    for (label, path) in [
        ("--artifact-root", &artifact_root),
        ("--working-directory", &working_directory),
    ] {
        if !path.is_absolute() {
            return Err(format!("{label} must be an absolute directory"));
        }
        let metadata = fs::metadata(path)
            .map_err(|error| format!("{label} must be an existing directory: {error}"))?;
        if !metadata.is_dir() {
            return Err(format!("{label} must be an existing directory"));
        }
    }
    Ok(CheckpointArgs {
        phase,
        artifact_root,
        working_directory,
    })
}

fn parse_stdin_exec_args(args: &[String]) -> Result<StdinExecArgs, String> {
    let mut stdin_file = None;
    let mut exit_mode = None;
    let mut sidecar_file = None;
    let mut command = Vec::new();
    let mut after_options = false;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if !after_options && token == "--" {
            after_options = true;
            index += 1;
            continue;
        }
        if !after_options {
            if let Some(value) = take_option_value(args, &mut index, token, "--stdin-file")? {
                stdin_file = Some(value);
                continue;
            }
            if let Some(value) = take_option_value(args, &mut index, token, "--exit-mode")? {
                exit_mode = Some(value);
                continue;
            }
            if let Some(value) = take_option_value(args, &mut index, token, "--sidecar-file")? {
                sidecar_file = Some(value);
                continue;
            }
            if token.starts_with('-') {
                return Err(format!("unknown option `{token}`"));
            }
        }
        command.push(token.clone());
        index += 1;
    }

    let stdin_file = stdin_file.ok_or_else(|| "missing --stdin-file path".to_owned())?;
    let exit_mode_raw = exit_mode.ok_or_else(|| "missing --exit-mode".to_owned())?;
    let exit_mode = match exit_mode_raw.as_str() {
        "sidecar" => StdinExecExitMode::Sidecar,
        "propagate" => StdinExecExitMode::Propagate,
        other => {
            return Err(format!(
                "unknown --exit-mode `{other}`; expected sidecar or propagate"
            ))
        }
    };
    match exit_mode {
        StdinExecExitMode::Sidecar => {
            if sidecar_file.is_none() {
                return Err("sidecar mode requires --sidecar-file".to_owned());
            }
        }
        StdinExecExitMode::Propagate => {
            if sidecar_file.is_some() {
                return Err("--sidecar-file is rejected in propagate mode".to_owned());
            }
        }
    }
    let mut command = command.into_iter();
    let program = command.next().ok_or_else(|| "missing COMMAND".to_owned())?;
    Ok(StdinExecArgs {
        stdin_file: PathBuf::from(stdin_file),
        exit_mode,
        sidecar_file: sidecar_file.map(PathBuf::from),
        command: program,
        args: command.collect(),
    })
}

fn take_option_value(
    args: &[String],
    index: &mut usize,
    token: &str,
    name: &str,
) -> Result<Option<String>, String> {
    if token == name {
        let value = args
            .get(*index + 1)
            .ok_or_else(|| format!("missing value for {name}"))?;
        *index += 2;
        return Ok(Some(value.clone()));
    }
    let prefix = format!("{name}=");
    if let Some(value) = token.strip_prefix(&prefix) {
        *index += 1;
        return Ok(Some(value.to_owned()));
    }
    Ok(None)
}

const PI_CODING_AGENT_SESSION_DIR: &str = "PI_CODING_AGENT_SESSION_DIR";

/// When `PI_CODING_AGENT_SESSION_DIR` is already present in the inherited
/// environment, leave it unchanged. Otherwise, if `--stdin-file` has a non-empty
/// parent, create `<parent>/sessions` and return that absolute path for the child.
fn prepare_child_session_dir(stdin_file: &Path) -> Result<Option<PathBuf>, String> {
    if std::env::var_os(PI_CODING_AGENT_SESSION_DIR).is_some() {
        return Ok(None);
    }
    let Some(parent) = stdin_file.parent() else {
        return Ok(None);
    };
    if parent.as_os_str().is_empty() {
        return Ok(None);
    }
    let sessions = parent.join("sessions");
    fs::create_dir_all(&sessions).map_err(|error| {
        format!(
            "could not create sessions directory {}: {error}",
            sessions.display()
        )
    })?;
    if sessions.is_absolute() {
        return Ok(Some(sessions));
    }
    let cwd = std::env::current_dir().map_err(|error| {
        format!("could not resolve current directory for sessions path: {error}")
    })?;
    Ok(Some(cwd.join(sessions)))
}

fn execute_stdin_exec(args: StdinExecArgs) -> i32 {
    let stdin = match fs::File::open(&args.stdin_file) {
        Ok(file) => file,
        Err(error) => {
            return stdin_exec_failed(
                format!(
                    "could not open --stdin-file {}: {error}",
                    args.stdin_file.display()
                ),
                EXIT_STDIN_EXEC_ERROR,
            )
        }
    };
    let session_dir = match prepare_child_session_dir(&args.stdin_file) {
        Ok(path) => path,
        Err(message) => return stdin_exec_failed(message, EXIT_STDIN_EXEC_ERROR),
    };
    let mut command = Command::new(&args.command);
    command
        .args(&args.args)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(session_dir) = session_dir.as_ref() {
        command.env(PI_CODING_AGENT_SESSION_DIR, session_dir);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return stdin_exec_failed(
                format!("could not spawn `{}`: {error}", args.command),
                EXIT_STDIN_EXEC_ERROR,
            )
        }
    };
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            return stdin_exec_failed(
                format!("could not wait for `{}`: {error}", args.command),
                EXIT_STDIN_EXEC_ERROR,
            )
        }
    };
    let exit_code = inner_waitpid_as_i32(status);
    match args.exit_mode {
        StdinExecExitMode::Sidecar => {
            let Some(sidecar_file) = args.sidecar_file.as_ref() else {
                return stdin_exec_failed("sidecar mode requires --sidecar-file".to_owned(), 2);
            };
            if let Some(parent) = sidecar_file.parent() {
                if !parent.as_os_str().is_empty() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        return stdin_exec_failed(
                            format!(
                                "could not create sidecar directory {}: {error}",
                                parent.display()
                            ),
                            EXIT_STDIN_EXEC_ERROR,
                        );
                    }
                }
            }
            let body = match serde_json::to_vec(&json!({ "exit_code": exit_code })) {
                Ok(body) => body,
                Err(error) => {
                    return stdin_exec_failed(
                        format!("could not serialize sidecar JSON: {error}"),
                        EXIT_STDIN_EXEC_ERROR,
                    )
                }
            };
            if let Err(error) = fs::write(sidecar_file, body) {
                return stdin_exec_failed(
                    format!(
                        "could not write sidecar {}: {error}",
                        sidecar_file.display()
                    ),
                    EXIT_STDIN_EXEC_ERROR,
                );
            }
            0
        }
        StdinExecExitMode::Propagate => exit_code,
    }
}

fn inner_waitpid_as_i32(status: process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return signal;
        }
    }
    1
}

fn stdin_exec_failed(message: String, exit_code: i32) -> i32 {
    eprintln!("stdin-exec error: {message}");
    exit_code
}

fn describe(request: Value) -> i32 {
    let request = match serde_json::from_value::<DescribeRequest>(request) {
        Ok(request) if request.operation == "describe" => request,
        Ok(_) => return protocol_error("describe request has the wrong operation".into()),
        Err(error) => return protocol_error(format!("invalid describe request: {error}")),
    };
    match workflow::describe_workflow(request.initial_input.as_ref()) {
        Ok(workflow) => write_json(&workflow),
        Err(message) => protocol_error(message),
    }
}

fn evaluate(request: Value) -> i32 {
    let request = match serde_json::from_value::<EvaluateRequest>(request) {
        Ok(request) if request.operation == "evaluate" => request,
        Ok(_) => return protocol_error("evaluate request has the wrong operation".into()),
        Err(error) => return protocol_error(format!("invalid evaluate request: {error}")),
    };

    match gates::evaluate(&request) {
        gates::EvaluationOutcome::Response(response) => write_json(&response),
        gates::EvaluationOutcome::EvaluationError(message) => evaluation_error(&message),
    }
}

fn write_json<T: Serialize>(value: &T) -> i32 {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match serde_json::to_writer(&mut stdout, value) {
        Ok(()) => match stdout.flush() {
            Ok(()) => 0,
            Err(error) => protocol_error(format!("could not flush response: {error}")),
        },
        Err(error) => protocol_error(format!("could not write response: {error}")),
    }
}

fn protocol_error(message: String) -> i32 {
    eprintln!("{message}");
    2
}

fn evaluation_error(message: &str) -> i32 {
    eprintln!("{message}");
    1
}
