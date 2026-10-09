//! `software-change` subprocess provider.
//!
//! One process handles one request.  Protocol errors use exit 2, evaluation
//! errors use exit 1, and a successfully written result uses exit 0.

mod artifacts;
mod checkpoint;
mod config;
mod criterion;
mod evidence;
// Retain task-routing ledger projections for external-driver policy consumers.
#[allow(dead_code)]
mod finding_ledger;
mod gates;
mod narratives;
mod option_evaluation;
mod overlay;
mod protocol;
mod recovery_contract;
mod review_batch;
mod review_candidates;
mod schema;
mod source_identity;
mod source_scopes;
mod validation;
mod workflow;

use protocol::{DescribeRequest, EvaluateRequest};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

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
        Some(command) if command == "bookends-preview" => {
            let rest = match args
                .map(|arg| arg.into_string())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!("bookends-preview arguments must be valid UTF-8");
                    return 2;
                }
            };
            return bookends_preview_from_args(&rest);
        }
        Some(command) if command == "author-counts" => {
            let rest: Vec<String> = args.map(|v| v.to_string_lossy().into_owned()).collect();
            let mut stdin = String::new();
            let result = io::stdin()
                .read_to_string(&mut stdin)
                .map_err(|e| e.to_string())
                .and_then(|_| serde_json::from_str::<Value>(&stdin).map_err(|e| e.to_string()))
                .and_then(|show| software_change_provider::author_counts::run(&rest, &show));
            return match result {
                Ok(value) => {
                    println!("{value}");
                    0
                }
                Err(error) => {
                    eprintln!("author-counts: {error}");
                    2
                }
            };
        }
        Some(command) if command == "setup" => {
            let rest = match args
                .map(|arg| arg.into_string())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!("setup arguments must be valid UTF-8");
                    return 2;
                }
            };
            return software_change_provider::setup::run_from_args(&rest);
        }
        Some(command) if command == "source-identity" => {
            let rest = match args
                .map(|arg| arg.into_string())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => return 2,
            };
            return match source_identity::run(&rest) {
                Ok(value) => write_json(&value),
                Err(error) => {
                    eprintln!("source-identity: {error}");
                    2
                }
            };
        }
        Some(command) if command == "prepare-validation" => {
            if args.next().is_some() {
                eprintln!("prepare-validation takes one JSON packet on stdin");
                return 2;
            }
            return match serde_json::from_reader(io::stdin())
                .map_err(|e| e.to_string())
                .and_then(|input| validation::prepare(&input))
            {
                Ok(value) => {
                    println!("{value}");
                    0
                }
                Err(error) => {
                    eprintln!("prepare-validation: {error}");
                    2
                }
            };
        }
        Some(command) if command == "commission" => {
            let rest = match args
                .map(|arg| arg.into_string())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rest) => rest,
                Err(_) => {
                    eprintln!("commission arguments must be UTF-8");
                    return 2;
                }
            };
            return software_change_provider::commission::run_from_stdin(&rest);
        }
        Some(command) if command == "advice-request" => {
            if args.next().is_some() {
                eprintln!("advice-request accepts one JSON packet on stdin and no arguments");
                return 2;
            }
            return match software_change_provider::advice::prepare_from_stdin() {
                Ok(request) => write_json(&request),
                Err(error) => {
                    eprintln!("advice-request: {error}");
                    2
                }
            };
        }
        Some(command) if command == "review-candidates" => {
            if args.next().is_some() {
                eprintln!("review-candidates accepts no additional arguments");
                return 2;
            }
            return review_candidates::run_from_stdin();
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

fn bookends_preview_from_args(args: &[String]) -> i32 {
    if args == ["--help"] || args == ["-h"] {
        println!("usage: software-change bookends-preview --working-directory ABS");
        return 0;
    }
    let mut working_directory = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        let value = if token == "--working-directory" {
            index += 1;
            args.get(index).cloned()
        } else if let Some(value) = token.strip_prefix("--working-directory=") {
            Some(value.to_owned())
        } else {
            eprintln!("unknown bookends-preview argument `{token}`; usage: software-change bookends-preview --working-directory ABS");
            return 2;
        };
        let Some(value) = value else {
            eprintln!("bookends-preview requires --working-directory ABS");
            return 2;
        };
        if working_directory.replace(PathBuf::from(value)).is_some() {
            eprintln!("bookends-preview --working-directory may be supplied once");
            return 2;
        }
        index += 1;
    }
    let Some(working_directory) = working_directory else {
        eprintln!("bookends-preview requires --working-directory ABS");
        return 2;
    };
    if !working_directory.is_absolute() {
        eprintln!("bookends-preview --working-directory must be absolute");
        return 2;
    }
    match bookends_check::preview_repo(&working_directory) {
        Ok(preview) => write_json(&preview),
        Err(error) => {
            eprintln!(
                "bookends-preview could not inspect {}: {error}",
                working_directory.display()
            );
            1
        }
    }
}

fn provider_help() -> i32 {
    println!("software-change\n\nCommands: author-counts, data-dump, bookends-preview, setup, advice-request, checkpoint, review-candidates, prepare-validation, commission, source-identity\nStdin operations: describe, evaluate\nsetup (--rigor minimal|standard|high | --profile FILE) --output FILE (--enable-advice | --decline-advice) [--bookends]");
    0
}

fn data_dump_usage(message: &str) -> i32 {
    eprintln!("{message}; usage: software-change data-dump DIR");
    2
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
        if token == "--json" {
            index += 1;
            continue;
        }
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

fn describe(request: Value) -> i32 {
    let request = match serde_json::from_value::<DescribeRequest>(request) {
        Ok(request) if request.operation == "describe" => request,
        Ok(_) => return protocol_error("describe request has the wrong operation".into()),
        Err(error) => return protocol_error(format!("invalid describe request: {error}")),
    };
    // Bare describe remains topology discovery, not execution admission.
    if let Some(input) = request
        .initial_input
        .as_ref()
        .filter(|v| v.get("config_version").is_some() || v.get("contract_version").is_some())
    {
        if let Err(message) = recovery_contract::RecoveryContract::from_input(input) {
            return protocol_error(message);
        }
    }
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
