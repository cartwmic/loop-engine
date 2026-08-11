//! Composition root and command-line driver for Loop Engine.
//!
//! The CLI deliberately stays thin: it parses caller input, constructs the
//! concrete integrations, invokes exactly one core operation, and renders the
//! core operation outcome.  Workflow and provider policy remain in the core
//! and integration crates respectively.

use loop_core::{
    self as core, AppendContextRequest, EventRequest, HistoryRequest, OperationOutcome,
    ProviderResolutionError, RunId, ShowRequest, StartRequest, TerminateRunRequest, Timestamp,
};
use loop_integrations::{
    ConfiguredProviderResolver, ProviderConfiguration, SqlitePersistence, SubprocessProviderGateway,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Exit status for a successfully completed semantic operation.
pub const EXIT_COMPLETED: i32 = 0;
/// Exit status for an understood request rejected by workflow/lifecycle
/// semantics.
pub const EXIT_REJECTED: i32 = 10;
/// Exit status for an operation that could not be evaluated or committed.
pub const EXIT_ERROR: i32 = 20;
/// Exit status for malformed CLI syntax or input.
pub const EXIT_INVALID_INVOCATION: i32 = 2;

const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);
const VERSION: &str = env!("CARGO_PKG_VERSION");
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Output format selected by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Human,
    Json,
}

/// Options that affect composition and rendering but not workflow semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliOptions {
    pub output: OutputFormat,
    pub database: Option<PathBuf>,
    pub provider_config: Option<PathBuf>,
    pub provider_timeout: Option<Duration>,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            output: OutputFormat::Human,
            database: None,
            provider_config: None,
            provider_timeout: None,
        }
    }
}

/// The seven and only seven primary CLI operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrimaryCommand {
    Start(StartArgs),
    List,
    Show(RunId),
    Append(AppendArgs),
    Event { run_id: RunId, event: String },
    History(RunId),
    Terminate(RunId),
}

impl PrimaryCommand {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Start(_) => "start",
            Self::List => "list",
            Self::Show(_) => "show",
            Self::Append(_) => "append",
            Self::Event { .. } => "event",
            Self::History(_) => "history",
            Self::Terminate(_) => "terminate",
        }
    }
}

/// Parsed values for `loop-engine start`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartArgs {
    pub provider: String,
    pub initial_input: Value,
    pub label: Option<String>,
    pub run_id: Option<RunId>,
}

/// Parsed values for `loop-engine append`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendArgs {
    pub run_id: RunId,
    pub kind: String,
    pub data: Value,
    pub record_id: Option<String>,
}

/// A parsed CLI request, including the two non-operation informational
/// requests.  `Help` and `Version` do not dispatch a semantic operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedRequest {
    Help {
        command: Option<String>,
    },
    Version,
    Operation {
        options: CliOptions,
        command: PrimaryCommand,
    },
}

/// A parser/composition error with a stable actionable code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliError {
    pub code: String,
    pub message: String,
}

impl CliError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

/// Captured process result.  The binary entry point writes these strings to
/// stdout/stderr and exits with `exit_code`; keeping it as a value makes the
/// CLI grammar and rendering straightforward to test without a subprocess.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Execution {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Parse command-line arguments.  The iterator form intentionally accepts
/// both `Vec<String>` and test-friendly arrays of `&str`.
pub fn parse_args<I, S>(args: I) -> Result<ParsedRequest, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    parse_args_slice(&args)
}

fn parse_args_slice(args: &[String]) -> Result<ParsedRequest, CliError> {
    let mut options = CliOptions::default();
    let mut command_name: Option<String> = None;
    let mut positionals = Vec::new();
    let mut help = false;
    let mut help_command = None;
    let mut version = false;
    let mut after_options = false;

    // These are command-local options collected before command validation so
    // options can be placed before or after the primary operation.
    let mut provider = None;
    let mut input = None;
    let mut label = None;
    let mut run_id = None;
    let mut start_id = None;
    let mut kind = None;
    let mut data = None;
    let mut record_id = None;
    let mut event = None;

    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if !after_options && token == "--" {
            after_options = true;
            index += 1;
            continue;
        }

        if !after_options {
            match token.as_str() {
                "--json" | "--machine-readable" | "-j" => {
                    options.output = OutputFormat::Json;
                    index += 1;
                    continue;
                }
                "--human" => {
                    options.output = OutputFormat::Human;
                    index += 1;
                    continue;
                }
                "--format" | "--output" => {
                    let value = next_option_value(args, &mut index, token)?;
                    options.output = parse_output_format(&value)?;
                    continue;
                }
                value if value.starts_with("--format=") => {
                    options.output = parse_output_format(
                        value.strip_prefix("--format=").expect("checked prefix"),
                    )?;
                    index += 1;
                    continue;
                }
                value if value.starts_with("--output=") => {
                    options.output = parse_output_format(
                        value.strip_prefix("--output=").expect("checked prefix"),
                    )?;
                    index += 1;
                    continue;
                }
                "--database" | "--database-path" | "--db" | "-d" => {
                    options.database =
                        Some(PathBuf::from(next_option_value(args, &mut index, token)?));
                    continue;
                }
                value if value.starts_with("--database=") => {
                    options.database = Some(PathBuf::from(
                        value.strip_prefix("--database=").expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                value if value.starts_with("--db=") => {
                    options.database = Some(PathBuf::from(
                        value.strip_prefix("--db=").expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                value if value.starts_with("--database-path=") => {
                    options.database = Some(PathBuf::from(
                        value
                            .strip_prefix("--database-path=")
                            .expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                "--config" | "--config-path" | "--provider-config" | "-c" => {
                    options.provider_config =
                        Some(PathBuf::from(next_option_value(args, &mut index, token)?));
                    continue;
                }
                value if value.starts_with("--config=") => {
                    options.provider_config = Some(PathBuf::from(
                        value.strip_prefix("--config=").expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                value if value.starts_with("--provider-config=") => {
                    options.provider_config = Some(PathBuf::from(
                        value
                            .strip_prefix("--provider-config=")
                            .expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                value if value.starts_with("--config-path=") => {
                    options.provider_config = Some(PathBuf::from(
                        value
                            .strip_prefix("--config-path=")
                            .expect("checked prefix"),
                    ));
                    index += 1;
                    continue;
                }
                "--timeout-ms" | "--provider-timeout-ms" | "--provider-timeout" => {
                    let raw = next_option_value(args, &mut index, token)?;
                    options.provider_timeout = Some(parse_timeout(&raw)?);
                    continue;
                }
                value if value.starts_with("--timeout-ms=") => {
                    options.provider_timeout = Some(parse_timeout(
                        value.strip_prefix("--timeout-ms=").expect("checked prefix"),
                    )?);
                    index += 1;
                    continue;
                }
                value if value.starts_with("--provider-timeout-ms=") => {
                    options.provider_timeout = Some(parse_timeout(
                        value
                            .strip_prefix("--provider-timeout-ms=")
                            .expect("checked prefix"),
                    )?);
                    index += 1;
                    continue;
                }
                value if value.starts_with("--provider-timeout=") => {
                    options.provider_timeout = Some(parse_timeout(
                        value
                            .strip_prefix("--provider-timeout=")
                            .expect("checked prefix"),
                    )?);
                    index += 1;
                    continue;
                }
                "--help" | "-h" => {
                    help = true;
                    help_command = command_name.clone();
                    index += 1;
                    continue;
                }
                "--version" | "-V" => {
                    version = true;
                    index += 1;
                    continue;
                }
                "--provider" => {
                    provider = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--provider=") => {
                    provider = Some(
                        value
                            .strip_prefix("--provider=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--input" | "--initial-input" => {
                    input = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--input=") => {
                    input = Some(
                        value
                            .strip_prefix("--input=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                value if value.starts_with("--initial-input=") => {
                    input = Some(
                        value
                            .strip_prefix("--initial-input=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--label" => {
                    label = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--label=") => {
                    label = Some(
                        value
                            .strip_prefix("--label=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--id" => {
                    start_id = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--id=") => {
                    start_id = Some(
                        value
                            .strip_prefix("--id=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--run" | "--run-id" => {
                    run_id = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--run=") => {
                    run_id = Some(
                        value
                            .strip_prefix("--run=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                value if value.starts_with("--run-id=") => {
                    run_id = Some(
                        value
                            .strip_prefix("--run-id=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--kind" | "--context-kind" => {
                    kind = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--kind=") => {
                    kind = Some(
                        value
                            .strip_prefix("--kind=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                value if value.starts_with("--context-kind=") => {
                    kind = Some(
                        value
                            .strip_prefix("--context-kind=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--data" | "--context-data" => {
                    data = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--data=") => {
                    data = Some(
                        value
                            .strip_prefix("--data=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                value if value.starts_with("--context-data=") => {
                    data = Some(
                        value
                            .strip_prefix("--context-data=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--record-id" => {
                    record_id = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--record-id=") => {
                    record_id = Some(
                        value
                            .strip_prefix("--record-id=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--event" => {
                    event = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--event=") => {
                    event = Some(
                        value
                            .strip_prefix("--event=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                value if value.starts_with('-') => {
                    return Err(CliError::new(
                        "invalid-invocation",
                        format!("unknown option `{value}`"),
                    ));
                }
                _ => {}
            }
        }

        if command_name.is_none() {
            command_name = Some(token.clone());
        } else {
            positionals.push(token.clone());
        }
        index += 1;
    }

    if version {
        return Ok(ParsedRequest::Version);
    }
    if help {
        return Ok(ParsedRequest::Help {
            command: help_command.or(command_name),
        });
    }

    let command_name = command_name.ok_or_else(|| {
        CliError::new(
            "invalid-invocation",
            "a primary operation is required (start, list, show, append, event, history, or terminate)",
        )
    })?;

    let command = parse_primary_command(
        &command_name,
        positionals,
        provider,
        input,
        label,
        run_id,
        start_id,
        kind,
        data,
        record_id,
        event,
    )?;

    Ok(ParsedRequest::Operation { options, command })
}

fn next_option_value(args: &[String], index: &mut usize, option: &str) -> Result<String, CliError> {
    *index += 1;
    let value = args.get(*index).ok_or_else(|| {
        CliError::new(
            "invalid-invocation",
            format!("option `{option}` requires a value"),
        )
    })?;
    if value.starts_with('-') && value != "-" {
        return Err(CliError::new(
            "invalid-invocation",
            format!("option `{option}` requires a value"),
        ));
    }
    *index += 1;
    Ok(value.clone())
}

fn parse_output_format(value: &str) -> Result<OutputFormat, CliError> {
    match value {
        "human" | "text" => Ok(OutputFormat::Human),
        "json" | "machine" => Ok(OutputFormat::Json),
        _ => Err(CliError::new(
            "invalid-output-format",
            format!("unknown output format `{value}`; expected human or json"),
        )),
    }
}

fn parse_timeout(value: &str) -> Result<Duration, CliError> {
    let milliseconds = value.parse::<u64>().map_err(|_| {
        CliError::new(
            "invalid-timeout",
            format!("provider timeout `{value}` is not a non-negative integer in milliseconds"),
        )
    })?;
    Ok(Duration::from_millis(milliseconds))
}

#[allow(clippy::too_many_arguments)]
fn parse_primary_command(
    name: &str,
    mut positionals: Vec<String>,
    provider: Option<String>,
    input: Option<String>,
    label: Option<String>,
    run_id: Option<String>,
    start_id: Option<String>,
    kind: Option<String>,
    data: Option<String>,
    record_id: Option<String>,
    event: Option<String>,
) -> Result<PrimaryCommand, CliError> {
    match name {
        "start" => {
            let provider = provider.or_else(|| take_positional(&mut positionals));
            let provider = required(provider, "provider")?;
            let initial_input = input.or_else(|| take_positional(&mut positionals));
            let initial_input = required(initial_input, "initial input JSON")?;
            let initial_input = parse_json_source(&initial_input, "initial input")?;
            let positional_label = take_positional(&mut positionals);
            let label = match (label, positional_label) {
                (Some(_), Some(_)) => {
                    return Err(CliError::new(
                        "invalid-invocation",
                        "start accepts at most one label",
                    ))
                }
                (Some(label), None) | (None, Some(label)) => Some(label),
                (None, None) => None,
            };
            ensure_no_positionals(&positionals, name)?;
            if kind.is_some() || data.is_some() || record_id.is_some() || event.is_some() {
                return Err(CliError::new(
                    "invalid-invocation",
                    "start accepts only provider, initial input, label, and --id/--run-id",
                ));
            }
            Ok(PrimaryCommand::Start(StartArgs {
                provider,
                initial_input,
                label,
                run_id: start_id.or(run_id).map(RunId::from),
            }))
        }
        "list" => {
            ensure_no_positionals(&positionals, name)?;
            if provider.is_some()
                || input.is_some()
                || label.is_some()
                || run_id.is_some()
                || start_id.is_some()
                || kind.is_some()
                || data.is_some()
                || record_id.is_some()
                || event.is_some()
            {
                return Err(CliError::new(
                    "invalid-invocation",
                    "list does not accept operation arguments",
                ));
            }
            Ok(PrimaryCommand::List)
        }
        "show" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            ensure_no_positionals(&positionals, name)?;
            reject_unrelated_options(
                name,
                provider,
                input,
                label,
                start_id,
                kind,
                data,
                record_id,
                event,
            )?;
            Ok(PrimaryCommand::Show(run.into()))
        }
        "append" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            let kind = kind.or_else(|| take_positional(&mut positionals));
            let kind = required(kind, "context kind")?;
            let data = data.or_else(|| take_positional(&mut positionals));
            let data = required(data, "context data JSON")?;
            let data = parse_json_source(&data, "context data")?;
            ensure_no_positionals(&positionals, name)?;
            reject_unrelated_options(
                name,
                provider,
                input,
                label,
                start_id,
                None,
                None,
                record_id.clone(),
                event,
            )?;
            Ok(PrimaryCommand::Append(AppendArgs {
                run_id: run.into(),
                kind,
                data,
                record_id,
            }))
        }
        "event" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            let event = event.or_else(|| take_positional(&mut positionals));
            let event = required(event, "event ID")?;
            ensure_no_positionals(&positionals, name)?;
            reject_unrelated_options(
                name,
                provider,
                input,
                label,
                start_id,
                kind,
                data,
                record_id,
                None,
            )?;
            Ok(PrimaryCommand::Event {
                run_id: run.into(),
                event,
            })
        }
        "history" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            ensure_no_positionals(&positionals, name)?;
            reject_unrelated_options(
                name,
                provider,
                input,
                label,
                start_id,
                kind,
                data,
                record_id,
                event,
            )?;
            Ok(PrimaryCommand::History(run.into()))
        }
        "terminate" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            ensure_no_positionals(&positionals, name)?;
            reject_unrelated_options(
                name,
                provider,
                input,
                label,
                start_id,
                kind,
                data,
                record_id,
                event,
            )?;
            Ok(PrimaryCommand::Terminate(run.into()))
        }
        _ => Err(CliError::new(
            "invalid-operation",
            format!(
                "unknown operation `{name}`; expected start, list, show, append, event, history, or terminate"
            ),
        )),
    }
}

fn take_positional(positionals: &mut Vec<String>) -> Option<String> {
    if positionals.is_empty() {
        None
    } else {
        Some(positionals.remove(0))
    }
}

fn required(value: Option<String>, description: &str) -> Result<String, CliError> {
    value.ok_or_else(|| {
        CliError::new(
            "invalid-invocation",
            format!("missing required {description}"),
        )
    })
}

fn ensure_no_positionals(positionals: &[String], operation: &str) -> Result<(), CliError> {
    if let Some(extra) = positionals.first() {
        return Err(CliError::new(
            "invalid-invocation",
            format!("unexpected argument `{extra}` for {operation}"),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reject_unrelated_options(
    operation: &str,
    provider: Option<String>,
    input: Option<String>,
    label: Option<String>,
    start_id: Option<String>,
    kind: Option<String>,
    data: Option<String>,
    record_id: Option<String>,
    event: Option<String>,
) -> Result<(), CliError> {
    if provider.is_some()
        || input.is_some()
        || label.is_some()
        || start_id.is_some()
        || kind.is_some()
        || data.is_some()
        || record_id.is_some()
        || event.is_some()
    {
        return Err(CliError::new(
            "invalid-invocation",
            format!("{operation} received an argument intended for another operation"),
        ));
    }
    Ok(())
}

/// Execute one parsed or raw CLI invocation and capture its rendered output.
pub fn execute<I, S>(args: I) -> Execution
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    let json_requested = args.iter().enumerate().any(|(index, arg)| {
        arg == "--json"
            || arg == "--machine-readable"
            || arg == "-j"
            || arg == "--format=json"
            || arg == "--format=machine"
            || arg == "--output=json"
            || arg == "--output=machine"
            || (arg == "--format"
                && args
                    .get(index + 1)
                    .is_some_and(|value| value == "json" || value == "machine"))
            || (arg == "--output"
                && args
                    .get(index + 1)
                    .is_some_and(|value| value == "json" || value == "machine"))
    });

    let parsed = match parse_args_slice(&args) {
        Ok(parsed) => parsed,
        Err(error) => return render_invalid_invocation(error, json_requested),
    };

    match parsed {
        ParsedRequest::Help { command } => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: usage(command.as_deref()),
            stderr: String::new(),
        },
        ParsedRequest::Version => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: format!("{VERSION}\n"),
            stderr: String::new(),
        },
        ParsedRequest::Operation { options, command } => execute_operation(options, command),
    }
}

fn execute_operation(options: CliOptions, command: PrimaryCommand) -> Execution {
    let output = options.output;
    let operation = command.name();

    // Parse JSON before opening storage.  Malformed caller input is an
    // invocation error, not a semantic outcome produced by core.
    let command = match parse_command_json(command) {
        Ok(command) => command,
        Err(error) => return render_invalid_invocation_with_format(error, output),
    };

    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return render_operation_error(operation, output, error),
    };
    let persistence = match open_persistence(&paths.database) {
        Ok(persistence) => persistence,
        Err(error) => return render_operation_error(operation, output, error),
    };
    let timeout = match resolve_provider_timeout(options.provider_timeout) {
        Ok(timeout) => timeout,
        Err(error) => return render_operation_error(operation, output, error),
    };
    let gateway = SubprocessProviderGateway::with_timeout(timeout);

    match command {
        PrimaryCommand::Start(args) => {
            let resolver = match load_provider_resolver(paths.provider_config.as_deref()) {
                Ok(resolver) => resolver,
                Err(error) => return render_operation_error(operation, output, error),
            };
            let request = StartRequest::new(
                args.run_id.unwrap_or_else(new_run_id),
                args.provider,
                args.initial_input,
                args.label,
                now_timestamp(),
            );
            let outcome = core::execute_start(request, &resolver, &gateway, &persistence)
                .map(CliCreateRunResult::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::List => {
            let outcome = core::execute_list(&persistence).map(|runs| {
                runs.into_iter()
                    .map(CliRunSummary::from)
                    .collect::<Vec<_>>()
            });
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::Show(run_id) => {
            let outcome = core::execute_show(ShowRequest::new(run_id), &persistence)
                .map(CliShowProjection::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::Append(args) => {
            let request = AppendContextRequest::new(
                args.run_id.clone(),
                args.record_id.unwrap_or_else(new_context_id),
                args.kind,
                args.data,
                now_timestamp(),
            );
            let outcome =
                core::execute_append(request, &persistence).map(CliAppendContextResult::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::Event { run_id, event } => {
            let outcome =
                core::execute_event(EventRequest::new(run_id, event), &gateway, &persistence)
                    .map(CliCommitTransitionResult::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::History(run_id) => {
            let outcome =
                core::execute_history(HistoryRequest::new(run_id), &persistence).map(|history| {
                    history
                        .into_iter()
                        .map(CliHistoryEntry::from)
                        .collect::<Vec<_>>()
                });
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::Terminate(run_id) => {
            let outcome = core::execute_terminate(TerminateRunRequest::new(run_id), &persistence)
                .map(CliTerminateResult::from);
            render_operation(operation, output, &outcome)
        }
    }
}

fn parse_command_json(command: PrimaryCommand) -> Result<PrimaryCommand, CliError> {
    // JSON input is parsed while constructing the command DTO.  Keeping this
    // boundary as a separate step makes the dispatch path explicit and leaves
    // room for future input sources without changing core requests.
    Ok(command)
}

fn parse_json_source(raw: &str, description: &str) -> Result<Value, CliError> {
    let source = if raw == "-" {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input).map_err(|error| {
            CliError::new(
                "input-read-failed",
                format!("could not read {description} from stdin: {error}"),
            )
        })?;
        input
    } else if let Some(path) = raw.strip_prefix('@') {
        if path.is_empty() {
            return Err(CliError::new(
                "invalid-json-input",
                format!("{description} file path after `@` is empty"),
            ));
        }
        fs::read_to_string(expand_tilde(Path::new(path))).map_err(|error| {
            CliError::new(
                "input-read-failed",
                format!("could not read {description} file `{path}`: {error}"),
            )
        })?
    } else {
        raw.to_owned()
    };

    serde_json::from_str(&source).map_err(|error| {
        CliError::new(
            "invalid-json-input",
            format!("{description} is not valid JSON: {error}"),
        )
    })
}

#[derive(Clone, Debug)]
struct ResolvedPaths {
    database: PathBuf,
    provider_config: Option<PathBuf>,
}

fn resolve_paths(options: &CliOptions) -> Result<ResolvedPaths, CliError> {
    let database = options
        .database
        .clone()
        .or_else(|| {
            first_env_path(&[
                "LOOP_ENGINE_DATABASE",
                "LOOP_ENGINE_DATABASE_PATH",
                "LOOP_DATABASE",
                "LOOP_DATABASE_PATH",
                "LOOP_DB_PATH",
                "LOOP_ENGINE_DB",
                "LOOP_DB",
            ])
        })
        .or_else(|| application_home().map(|home| home.join("loop.db")))
        .or_else(|| {
            let project = project_directory().join(".loop-engine").join("loop.db");
            Some(project)
        })
        .or_else(|| data_directory().map(|directory| directory.join("loop.db")))
        .ok_or_else(|| {
            CliError::new(
                "database-path-unavailable",
                "could not discover an application database path",
            )
        })?;

    let provider_config = if let Some(path) = options.provider_config.clone() {
        Some(expand_tilde(path.as_path()))
    } else if let Some(path) = first_env_path(&[
        "LOOP_ENGINE_PROVIDER_CONFIG",
        "LOOP_ENGINE_PROVIDER_CONFIG_PATH",
        "LOOP_PROVIDER_CONFIG",
        "LOOP_PROVIDER_CONFIG_PATH",
        "LOOP_ENGINE_CONFIG",
        "LOOP_ENGINE_CONFIG_PATH",
        "LOOP_CONFIG",
        "LOOP_CONFIG_PATH",
    ]) {
        Some(path)
    } else {
        discover_default_provider_config()
    };

    Ok(ResolvedPaths {
        database: expand_tilde(database.as_path()),
        provider_config,
    })
}

fn project_directory() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn application_home() -> Option<PathBuf> {
    first_env_path(&["LOOP_ENGINE_HOME", "LOOP_HOME"])
}

fn data_directory() -> Option<PathBuf> {
    if let Some(path) = first_env_path(&["XDG_DATA_HOME"]) {
        return Some(path.join("loop-engine"));
    }
    home_directory().map(|home| home.join(".local").join("share").join("loop-engine"))
}

fn discover_default_provider_config() -> Option<PathBuf> {
    let project = project_directory()
        .join(".loop-engine")
        .join("providers.toml");
    if project.is_file() {
        return Some(project);
    }
    if let Some(home) = application_home() {
        let path = home.join("providers.toml");
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(config_home) = first_env_path(&["XDG_CONFIG_HOME"]) {
        let path = config_home.join("loop-engine").join("providers.toml");
        if path.is_file() {
            return Some(path);
        }
    }
    home_directory().and_then(|home| {
        let path = home
            .join(".config")
            .join("loop-engine")
            .join("providers.toml");
        path.is_file().then_some(path)
    })
}

fn first_env_path(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn home_directory() -> Option<PathBuf> {
    first_env_path(&["HOME", "USERPROFILE"])
}

fn expand_tilde(path: &Path) -> PathBuf {
    match path.to_str() {
        Some("~") => home_directory().unwrap_or_else(|| path.to_path_buf()),
        Some(value) if value.starts_with("~/") => home_directory()
            .map(|home| home.join(&value[2..]))
            .unwrap_or_else(|| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

fn open_persistence(path: &Path) -> Result<SqlitePersistence, CliError> {
    if path.as_os_str() != ":memory:" {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                CliError::new(
                    "database-unavailable",
                    format!(
                        "could not create database directory `{}`: {error}",
                        parent.display()
                    ),
                )
            })?;
        }
    }
    SqlitePersistence::open(path).map_err(|error| CliError::new(error.code(), error.to_string()))
}

fn load_provider_resolver(path: Option<&Path>) -> Result<ConfiguredProviderResolver, CliError> {
    let configuration = match path {
        Some(path) => ProviderConfiguration::from_file(path)
            .map_err(|error| provider_configuration_error(path, error))?,
        None => ProviderConfiguration::default(),
    };
    Ok(ConfiguredProviderResolver::new(configuration))
}

fn provider_configuration_error(path: &Path, error: ProviderResolutionError) -> CliError {
    CliError::new(
        error.code(),
        format!("{} (configuration `{}`)", error, path.display()),
    )
}

fn resolve_provider_timeout(explicit: Option<Duration>) -> Result<Duration, CliError> {
    if let Some(timeout) = explicit {
        return Ok(timeout);
    }
    if let Some(value) = first_env_value(&[
        "LOOP_ENGINE_PROVIDER_TIMEOUT_MS",
        "LOOP_PROVIDER_TIMEOUT_MS",
    ]) {
        return parse_timeout(&value);
    }
    Ok(DEFAULT_PROVIDER_TIMEOUT)
}

fn first_env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

fn now_timestamp() -> Timestamp {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Timestamp::from_unix_millis(milliseconds)
}

fn unique_token(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{nanos}-{sequence}-{}", process::id(),)
}

fn new_run_id() -> RunId {
    unique_token("run").into()
}

fn new_context_id() -> String {
    unique_token("context")
}

/// Caller-visible run data.  Core's `Run` also carries persistence control
/// metadata; those fields are deliberately not represented by this CLI DTO.
#[derive(Clone, Debug, Serialize)]
struct CliRun {
    id: core::RunId,
    label: Option<String>,
    workflow: core::Workflow,
    provider_association: core::ProviderAssociation,
    initial_input: Value,
    current_state: core::StateId,
    lifecycle: core::Lifecycle,
    created_at: core::Timestamp,
}

impl From<core::Run> for CliRun {
    fn from(run: core::Run) -> Self {
        Self {
            id: run.id,
            label: run.label,
            workflow: run.workflow,
            provider_association: run.provider_association,
            initial_input: run.initial_input,
            current_state: run.current_state,
            lifecycle: run.lifecycle,
            created_at: run.created_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliCreateRunResult {
    run: CliRun,
    history: core::HistoryEntry,
}

impl From<core::CreateRunResult> for CliCreateRunResult {
    fn from(result: core::CreateRunResult) -> Self {
        Self {
            run: result.run.into(),
            history: result.history,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliAppendContextResult {
    run: CliRun,
    context: core::ContextRecord,
    history: core::HistoryEntry,
}

impl From<core::AppendContextResult> for CliAppendContextResult {
    fn from(result: core::AppendContextResult) -> Self {
        Self {
            run: result.run.into(),
            context: result.context,
            history: result.history,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliCommitTransitionResult {
    run: CliRun,
    history: core::HistoryEntry,
}

impl From<core::CommitTransitionResult> for CliCommitTransitionResult {
    fn from(result: core::CommitTransitionResult) -> Self {
        Self {
            run: result.run.into(),
            history: result.history,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliTerminateResult {
    run: CliRun,
    history: core::HistoryEntry,
}

impl From<core::TerminateResult> for CliTerminateResult {
    fn from(result: core::TerminateResult) -> Self {
        Self {
            run: result.run.into(),
            history: result.history,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliRunSummary {
    id: core::RunId,
    label: Option<String>,
    workflow_id: core::WorkflowId,
    lifecycle: core::Lifecycle,
    current_state: core::StateId,
}

impl From<core::RunSummary> for CliRunSummary {
    fn from(summary: core::RunSummary) -> Self {
        Self {
            id: summary.id,
            label: summary.label,
            workflow_id: summary.workflow_id,
            lifecycle: summary.lifecycle,
            current_state: summary.current_state,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliShowProjection {
    run_id: core::RunId,
    label: Option<String>,
    workflow_id: core::WorkflowId,
    lifecycle: core::Lifecycle,
    current_state: core::StateId,
    current_state_title: String,
    current_state_instructions: String,
    initial_input: Value,
    context: Vec<core::ContextRecord>,
    requestable_events: Vec<core::RequestableEvent>,
    latest_evaluations: Vec<core::DurableEvaluation>,
}

impl From<core::ShowProjection> for CliShowProjection {
    fn from(projection: core::ShowProjection) -> Self {
        Self {
            run_id: projection.run_id,
            label: projection.label,
            workflow_id: projection.workflow_id,
            lifecycle: projection.lifecycle,
            current_state: projection.current_state,
            current_state_title: projection.current_state_title,
            current_state_instructions: projection.current_state_instructions,
            initial_input: projection.initial_input,
            context: projection.context,
            requestable_events: projection.requestable_events,
            latest_evaluations: projection.latest_evaluations,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CliHistoryEntry {
    sequence: core::SemanticSequence,
    occurred_at: core::Timestamp,
    action: core::HistoryAction,
}

impl From<core::HistoryEntry> for CliHistoryEntry {
    fn from(entry: core::HistoryEntry) -> Self {
        Self {
            sequence: entry.sequence,
            occurred_at: entry.occurred_at,
            action: entry.action,
        }
    }
}

fn render_operation<T>(
    operation: &str,
    output: OutputFormat,
    outcome: &OperationOutcome<T>,
) -> Execution
where
    T: Serialize,
{
    let exit_code = match outcome {
        OperationOutcome::Completed(_) => EXIT_COMPLETED,
        OperationOutcome::Rejected(_) => EXIT_REJECTED,
        OperationOutcome::Error(_) => EXIT_ERROR,
    };

    match output {
        OutputFormat::Json => match render_operation_json(operation, outcome) {
            Ok(stdout) => Execution {
                exit_code,
                stdout: format!("{stdout}\n"),
                stderr: String::new(),
            },
            Err(error) => render_operation_error(operation, output, error),
        },
        OutputFormat::Human => Execution {
            exit_code,
            stdout: render_operation_human(operation, outcome),
            stderr: String::new(),
        },
    }
}

/// Serialize an already constructed caller-facing projection.
///
/// Dispatch constructs explicit DTOs above, so persistence-only run metadata
/// is omitted at that known projection boundary.  This function deliberately
/// performs no recursive filtering: every JSON value nested in an opaque
/// caller payload must pass through unchanged.
fn serialize_projection_value<T: Serialize>(value: &T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(|error| {
        CliError::new(
            "output-serialization-failed",
            format!("could not serialize completed result: {error}"),
        )
    })
}

/// Render a core operation outcome as one compact, valid JSON document.
pub fn render_operation_json<T: Serialize>(
    operation: &str,
    outcome: &OperationOutcome<T>,
) -> Result<String, CliError> {
    let mut object = serde_json::Map::new();
    object.insert("operation".to_owned(), Value::String(operation.to_owned()));
    match outcome {
        OperationOutcome::Completed(value) => {
            object.insert("status".to_owned(), Value::String("completed".to_owned()));
            object.insert("result".to_owned(), serialize_projection_value(value)?);
        }
        OperationOutcome::Rejected(issue) | OperationOutcome::Error(issue) => {
            let status = if matches!(outcome, OperationOutcome::Rejected(_)) {
                "rejected"
            } else {
                "error"
            };
            object.insert("status".to_owned(), Value::String(status.to_owned()));
            object.insert("code".to_owned(), Value::String(issue.code.clone()));
            object.insert("message".to_owned(), Value::String(issue.message.clone()));
            if let Some(details) = &issue.details {
                object.insert("details".to_owned(), details.clone());
            }
        }
    }
    serde_json::to_string(&Value::Object(object)).map_err(|error| {
        CliError::new(
            "output-serialization-failed",
            format!("could not serialize operation output: {error}"),
        )
    })
}

fn render_operation_human<T: Serialize>(operation: &str, outcome: &OperationOutcome<T>) -> String {
    match outcome {
        OperationOutcome::Completed(value) => {
            match serialize_projection_value(value).and_then(|value| {
                serde_json::to_string_pretty(&value).map_err(|error| {
                    CliError::new(
                        "output-serialization-failed",
                        format!("could not serialize operation output: {error}"),
                    )
                })
            }) {
                Ok(value) => format!("completed {operation}\n{value}\n"),
                Err(error) => format!("error {operation}: output serialization failed: {error}\n"),
            }
        }
        OperationOutcome::Rejected(issue) => format!(
            "rejected {operation}\n[{}] {}{}\n",
            issue.code,
            issue.message,
            format_details(issue.details.as_ref()),
        ),
        OperationOutcome::Error(issue) => format!(
            "error {operation}\n[{}] {}{}\n",
            issue.code,
            issue.message,
            format_details(issue.details.as_ref()),
        ),
    }
}

fn format_details(details: Option<&Value>) -> String {
    details
        .map(|details| format!("\ndetails: {}", details))
        .unwrap_or_default()
}

fn render_operation_error(operation: &str, output: OutputFormat, error: CliError) -> Execution {
    let issue = core::OutcomeIssue::new(error.code, error.message);
    let outcome = OperationOutcome::<Value>::error_with_issue(issue);
    render_operation(operation, output, &outcome)
}

fn render_invalid_invocation(error: CliError, json_requested: bool) -> Execution {
    render_invalid_invocation_with_format(
        error,
        if json_requested {
            OutputFormat::Json
        } else {
            OutputFormat::Human
        },
    )
}

fn render_invalid_invocation_with_format(error: CliError, output: OutputFormat) -> Execution {
    match output {
        OutputFormat::Json => {
            let value = json!({
                "status": "invalid-invocation",
                "code": error.code,
                "message": error.message,
            });
            Execution {
                exit_code: EXIT_INVALID_INVOCATION,
                stdout: format!("{}\n", value),
                stderr: String::new(),
            }
        }
        OutputFormat::Human => Execution {
            exit_code: EXIT_INVALID_INVOCATION,
            stdout: String::new(),
            stderr: format!("invalid invocation: {}\n\n{}", error, usage(None)),
        },
    }
}

fn usage(command: Option<&str>) -> String {
    match command {
        Some("start") => {
            "Usage: loop-engine [options] start <provider> <initial-json> [label]\n\n".to_owned()
                + "Options: --id <run-id> --label <label> --database <path> --config <path> --json\n"
        }
        Some("append") => {
            "Usage: loop-engine [options] append <run-id> <kind> <data-json>\n\n".to_owned()
                + "Options: --record-id <id> --database <path> --json\n"
        }
        Some("event") => "Usage: loop-engine [options] event <run-id> <event>\n".to_owned(),
        Some("show") => "Usage: loop-engine [options] show <run-id>\n".to_owned(),
        Some("history") => "Usage: loop-engine [options] history <run-id>\n".to_owned(),
        Some("terminate") => "Usage: loop-engine [options] terminate <run-id>\n".to_owned(),
        Some("list") => "Usage: loop-engine [options] list\n".to_owned(),
        _ => {
            "Usage: loop-engine [options] <operation> [arguments]\n\nOperations:\n  start\n  list\n  show\n  append\n  event\n  history\n  terminate\n\nGlobal options:\n  --json, -j                 Render machine-readable JSON\n  --database <path>          SQLite database path\n  --config <path>            Provider TOML configuration path\n  --timeout-ms <milliseconds> Provider operation timeout\n  --help, -h                 Show help\n  --version, -V              Show version\n"
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{EvaluationFeedback, OutcomeIssue};
    use serde_json::json;

    /// Internal run metadata is omitted by the explicit DTO at its known
    /// projection boundary.  Do not recurse into opaque caller JSON here:
    /// those payloads may legitimately contain the same field names.
    fn assert_internal_metadata_absent(value: &Value) {
        let Some(result) = value.get("result") else {
            return;
        };

        match result {
            Value::Object(object) => {
                let projection = object.get("run").unwrap_or(result);
                let projection = projection
                    .as_object()
                    .expect("caller projection must be a JSON object");
                assert!(!projection.contains_key("control_revision"));
                assert!(!projection.contains_key("last_sequence"));
            }
            Value::Array(values) => {
                for projection in values {
                    let projection = projection
                        .as_object()
                        .expect("caller projection array entries must be JSON objects");
                    assert!(!projection.contains_key("control_revision"));
                    assert!(!projection.contains_key("last_sequence"));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn parse_human_projection(stdout: &str) -> Value {
        let (_, body) = stdout
            .split_once('\n')
            .expect("human output must have a status line");
        serde_json::from_str(body.trim()).expect("human projection body must be valid JSON")
    }

    #[test]
    fn parser_exposes_exactly_the_seven_primary_operations() {
        let inputs = [
            vec!["start", "provider", "{}"],
            vec!["list"],
            vec!["show", "run-1"],
            vec!["append", "run-1", "note", "{}"],
            vec!["event", "run-1", "finish"],
            vec!["history", "run-1"],
            vec!["terminate", "run-1"],
        ];

        let names = inputs
            .iter()
            .map(|input| match parse_args(input.iter().copied()).unwrap() {
                ParsedRequest::Operation { command, .. } => command.name(),
                other => panic!("unexpected parse result: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "start",
                "list",
                "show",
                "append",
                "event",
                "history",
                "terminate"
            ]
        );
    }

    #[test]
    fn help_uses_the_canonical_executable_name() {
        let help = execute(["--help"]);
        assert_eq!(help.exit_code, EXIT_COMPLETED);
        assert!(help.stdout.starts_with("Usage: loop-engine [options]"));
        assert!(!help.stdout.contains("Usage: loop ["));

        for command in [
            "start",
            "list",
            "show",
            "append",
            "event",
            "history",
            "terminate",
        ] {
            let command_help = execute([command, "--help"]);
            assert_eq!(command_help.exit_code, EXIT_COMPLETED);
            assert!(
                command_help
                    .stdout
                    .starts_with("Usage: loop-engine [options]"),
                "help for {command} used the wrong executable name: {}",
                command_help.stdout
            );
        }
    }

    #[test]
    fn parser_accepts_global_options_before_and_after_operation() {
        let parsed = parse_args([
            "--json",
            "start",
            "provider",
            "{\"goal\":true}",
            "--database",
            "/tmp/loop.db",
            "--config=/tmp/providers.toml",
            "--timeout-ms=17",
        ])
        .unwrap();
        let ParsedRequest::Operation { options, command } = parsed else {
            panic!("expected operation")
        };
        assert_eq!(options.output, OutputFormat::Json);
        assert_eq!(options.database, Some(PathBuf::from("/tmp/loop.db")));
        assert_eq!(options.provider_timeout, Some(Duration::from_millis(17)));
        assert!(matches!(command, PrimaryCommand::Start(_)));
    }

    #[test]
    fn malformed_invocation_has_distinct_exit_code_and_json_is_valid() {
        let result = execute(["--json", "event", "only-run"]);
        assert_eq!(result.exit_code, EXIT_INVALID_INVOCATION);
        let output: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["status"], "invalid-invocation");
        assert_eq!(output["code"], "invalid-invocation");
        assert!(output.get("outcome").is_none());
    }

    #[test]
    fn json_rendering_preserves_completed_rejected_and_error() {
        let completed = OperationOutcome::completed(json!({"ok": true}));
        let rejected = OperationOutcome::<Value>::rejected_with_issue(OutcomeIssue::new(
            "event-unavailable",
            "No event",
        ));
        let error = OperationOutcome::<Value>::error_with_issue(OutcomeIssue::from_feedback(
            EvaluationFeedback::new("provider-timeout", "Provider timed out")
                .with_details(json!({"timeout_ms": 100})),
        ));

        let completed_json: Value =
            serde_json::from_str(&render_operation_json("list", &completed).unwrap()).unwrap();
        assert_eq!(completed_json["status"], "completed");
        assert!(completed_json.get("outcome").is_none());
        let rejected_json: Value =
            serde_json::from_str(&render_operation_json("event", &rejected).unwrap()).unwrap();
        assert_eq!(rejected_json["status"], "rejected");
        assert_eq!(rejected_json["code"], "event-unavailable");
        assert!(rejected_json.get("outcome").is_none());
        let error_json: Value =
            serde_json::from_str(&render_operation_json("event", &error).unwrap()).unwrap();
        assert_eq!(error_json["status"], "error");
        assert_eq!(error_json["code"], "provider-timeout");
        assert!(error_json.get("outcome").is_none());
        assert_eq!(error_json["details"]["timeout_ms"], 100);
    }

    #[test]
    fn caller_renderers_omit_internal_run_metadata_from_explicit_dto() {
        let run = core::Run::new(
            "run-1",
            Some("example".to_owned()),
            core::Workflow::new(
                "workflow",
                "start",
                vec![core::State::new("start", "Start", "Begin", false)],
                vec![],
            ),
            core::ProviderAssociation::new(json!({"provider": "fixture"})),
            json!({"input": true}),
            "start",
            core::Lifecycle::Active,
            3_u64.into(),
            7_u64.into(),
            core::Timestamp::from_unix_millis(1),
        );
        // The composition root converts core::Run at the known projection
        // boundary. Rendering the DTO, rather than scanning its JSON, keeps
        // opaque caller data intact while omitting persistence metadata.
        let outcome = OperationOutcome::completed(CliRun::from(run));

        let json_output: Value =
            serde_json::from_str(&render_operation_json("show", &outcome).unwrap()).unwrap();
        assert_internal_metadata_absent(&json_output);
        assert_eq!(json_output["result"]["id"], "run-1");

        let human_output = render_operation_human("show", &outcome);
        assert!(!human_output.contains("control_revision"));
        assert!(!human_output.contains("last_sequence"));
    }

    #[test]
    fn semantic_exit_codes_are_distinct() {
        assert_ne!(EXIT_COMPLETED, EXIT_REJECTED);
        assert_ne!(EXIT_REJECTED, EXIT_ERROR);
        assert_ne!(EXIT_ERROR, EXIT_INVALID_INVOCATION);
    }

    #[test]
    fn json_input_is_parsed_at_dispatch_boundary() {
        let command = parse_args(["start", "provider", "{\"x\":1}"]).unwrap();
        let ParsedRequest::Operation { command, .. } = command else {
            panic!("expected operation")
        };
        let command = parse_command_json(command).unwrap();
        let PrimaryCommand::Start(args) = command else {
            panic!("expected start")
        };
        assert_eq!(args.initial_input, json!({"x": 1}));
    }

    #[cfg(unix)]
    #[test]
    fn dispatches_all_seven_operations_against_real_composed_integrations() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(unique_token("loop-cli-test"));
        fs::create_dir_all(&root).unwrap();
        let provider_path = root.join("provider.sh");
        fs::write(
            &provider_path,
            r#"#!/bin/sh
read request
printf '%s' '{"id":"cli-fixture","initial_state":"start","states":[{"id":"start","title":"Start","instructions":"Begin","final":false},{"id":"middle","title":"Middle","instructions":"Continue","final":false}],"transitions":[{"source":"start","event":"finish","target":"middle","kind":"check-free"}]}'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&provider_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&provider_path, permissions).unwrap();

        let config_path = root.join("providers.toml");
        let command = provider_path.to_string_lossy().replace('"', "\\\"");
        fs::write(
            &config_path,
            format!("[providers.fixture]\ncommand = \"{command}\"\n"),
        )
        .unwrap();
        let database = root.join("loop.db");
        let database = database.to_string_lossy().to_string();
        let config = config_path.to_string_lossy().to_string();

        let initial_input = json!({
            "goal": "test",
            "control_revision": "caller-input-revision",
            "last_sequence": "caller-input-sequence",
            "opaque_run_like": {
                "id": "caller-input-id",
                "workflow": {"owned": true},
                "provider_association": {"owned": true},
                "initial_input": {"owned": true},
                "current_state": "caller-input-state",
                "lifecycle": "caller-input-lifecycle",
                "created_at": "caller-input-created",
                "control_revision": "caller-input-nested-revision",
                "last_sequence": "caller-input-nested-sequence"
            }
        });
        let initial_input_json = serde_json::to_string(&initial_input).unwrap();
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            initial_input_json,
            "--database".to_owned(),
            database.clone(),
            "--config".to_owned(),
            config.clone(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        assert_eq!(start_json["status"], "completed");
        assert_internal_metadata_absent(&start_json);
        assert_eq!(start_json["result"]["run"]["lifecycle"], "active");
        assert_eq!(start_json["result"]["run"]["initial_input"], initial_input);
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let list = execute([
            "--json".to_owned(),
            "list".to_owned(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(list.exit_code, EXIT_COMPLETED);
        let list_json: Value = serde_json::from_str(&list.stdout).unwrap();
        assert_eq!(list_json["status"], "completed");
        assert_internal_metadata_absent(&list_json);
        assert_eq!(list_json["result"][0]["id"], run_id);

        let show = execute([
            "--json".to_owned(),
            "show".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(show.exit_code, EXIT_COMPLETED);
        let show_json: Value = serde_json::from_str(&show.stdout).unwrap();
        assert_eq!(show_json["status"], "completed");
        assert_internal_metadata_absent(&show_json);
        assert_eq!(show_json["result"]["run_id"], run_id);
        assert_eq!(show_json["result"]["initial_input"], initial_input);

        let context_data = json!({
            "text": "hello",
            "control_revision": "caller-context-revision",
            "last_sequence": "caller-context-sequence",
            "opaque_run_like": {
                "id": "caller-context-id",
                "workflow": {"owned": true},
                "provider_association": {"owned": true},
                "initial_input": {"owned": true},
                "current_state": "caller-context-state",
                "lifecycle": "caller-context-lifecycle",
                "created_at": "caller-context-created",
                "control_revision": "caller-context-nested-revision",
                "last_sequence": "caller-context-nested-sequence"
            }
        });
        let context_data_json = serde_json::to_string(&context_data).unwrap();

        let human_start = execute([
            "start".to_owned(),
            "fixture".to_owned(),
            serde_json::to_string(&initial_input).unwrap(),
            "--database".to_owned(),
            database.clone(),
            "--config".to_owned(),
            config.clone(),
        ]);
        assert_eq!(human_start.exit_code, EXIT_COMPLETED);
        let human_start_json = parse_human_projection(&human_start.stdout);
        let human_start_run = human_start_json["run"]
            .as_object()
            .expect("human start must render a run object");
        assert!(!human_start_run.contains_key("control_revision"));
        assert!(!human_start_run.contains_key("last_sequence"));
        assert_eq!(human_start_json["run"]["initial_input"], initial_input);
        let human_run_id = human_start_json["run"]["id"].as_str().unwrap().to_owned();

        let append = execute([
            "--json".to_owned(),
            "append".to_owned(),
            run_id.clone(),
            "note".to_owned(),
            context_data_json,
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(append.exit_code, EXIT_COMPLETED);
        let append_json: Value = serde_json::from_str(&append.stdout).unwrap();
        assert_eq!(append_json["status"], "completed");
        assert_internal_metadata_absent(&append_json);
        assert_eq!(append_json["result"]["run"]["id"], run_id);
        assert_eq!(append_json["result"]["context"]["data"], context_data);

        let human_append = execute([
            "append".to_owned(),
            human_run_id.clone(),
            "note".to_owned(),
            serde_json::to_string(&context_data).unwrap(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(human_append.exit_code, EXIT_COMPLETED);

        let show_human = execute([
            "show".to_owned(),
            human_run_id,
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(show_human.exit_code, EXIT_COMPLETED);
        let show_human_json = parse_human_projection(&show_human.stdout);
        assert!(!show_human_json
            .as_object()
            .expect("human show must render an object")
            .contains_key("control_revision"));
        assert!(!show_human_json
            .as_object()
            .expect("human show must render an object")
            .contains_key("last_sequence"));
        assert_eq!(show_human_json["initial_input"], initial_input);
        assert_eq!(show_human_json["context"][0]["data"], context_data);

        let event = execute([
            "--json".to_owned(),
            "event".to_owned(),
            run_id.clone(),
            "finish".to_owned(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(event.exit_code, EXIT_COMPLETED);
        let event_json: Value = serde_json::from_str(&event.stdout).unwrap();
        assert_eq!(event_json["status"], "completed");
        assert_internal_metadata_absent(&event_json);
        assert_eq!(event_json["result"]["run"]["id"], run_id);

        let history = execute([
            "--json".to_owned(),
            "history".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(history.exit_code, EXIT_COMPLETED);
        let history_json: Value = serde_json::from_str(&history.stdout).unwrap();
        assert_eq!(history_json["status"], "completed");
        assert_internal_metadata_absent(&history_json);
        assert_eq!(history_json["result"][0]["action"]["kind"], "run_created");

        let terminate = execute([
            "--json".to_owned(),
            "terminate".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(terminate.exit_code, EXIT_COMPLETED);
        let terminate_json: Value = serde_json::from_str(&terminate.stdout).unwrap();
        assert_eq!(terminate_json["status"], "completed");
        assert_internal_metadata_absent(&terminate_json);
        assert_eq!(terminate_json["result"]["run"]["id"], run_id);

        let rejected = execute([
            "--json".to_owned(),
            "event".to_owned(),
            run_id.clone(),
            "finish".to_owned(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(rejected.exit_code, EXIT_REJECTED);
        let rejected_json: Value = serde_json::from_str(&rejected.stdout).unwrap();
        assert_eq!(rejected_json["status"], "rejected");
        assert_internal_metadata_absent(&rejected_json);

        let error = execute([
            "--json".to_owned(),
            "show".to_owned(),
            "missing-run".to_owned(),
            "--database".to_owned(),
            database,
        ]);
        assert_eq!(error.exit_code, EXIT_ERROR);
        let error_json: Value = serde_json::from_str(&error.stdout).unwrap();
        assert_eq!(error_json["status"], "error");
        assert_eq!(error_json["code"], "run-not-found");
        assert_internal_metadata_absent(&error_json);

        let _ = fs::remove_dir_all(root);
    }
}
