//! Composition root and command-line driver for Loop Engine.
//!
//! The CLI deliberately stays thin: it parses caller input, constructs the
//! concrete integrations, invokes exactly one core operation, and renders the
//! core operation outcome.  Workflow and provider policy remain in the core
//! and integration crates respectively.

mod dagu;
mod fan_out;
mod invocation_progress;
mod preview_bindings;

pub use dagu::{names_for_capture_root, resolve_dagu, write_locator, DaguError, DaguLocator};
pub use fan_out::FanOutArgs;

use loop_core::{
    self as core, AppendContextRequest, CarryAct, CarryRequest, CompleteWorkSlotInvocationRequest,
    EventRequest, HistoryRequest, InnerWorker, InvocationId, InvokeRequest, OperationOutcome,
    Persistence, ProcessError, ProviderResolutionError, RunId, ShowRequest, StartRequest,
    StartedWaiter, TerminateRunRequest, Timestamp, WaiterSpawnArgs, WaiterWrittenStatus,
    WorkSlotProcess,
};
use loop_integrations::{
    ConfiguredProviderResolver, ProviderConfiguration, SqlitePersistence, SubprocessProviderGateway,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
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

/// Exit mode for the hidden `stdin-exec` helper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdinExecExitMode {
    Sidecar,
    Propagate,
}

/// Parsed values for hidden `loop-engine stdin-exec`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StdinExecArgs {
    pub stdin_file: PathBuf,
    pub exit_mode: StdinExecExitMode,
    pub sidecar_file: Option<PathBuf>,
    pub command: String,
    pub args: Vec<String>,
}

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
    /// Human-only compact rendering for the existing `show` operation.
    pub compact: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            output: OutputFormat::Human,
            database: None,
            provider_config: None,
            provider_timeout: None,
            compact: false,
        }
    }
}

/// The eight primary CLI operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrimaryCommand {
    Start(StartArgs),
    List,
    Show(RunId),
    Append(AppendArgs),
    Event {
        run_id: RunId,
        event: String,
    },
    History(RunId),
    Terminate(RunId),
    Invoke {
        run_id: RunId,
        slot_id: String,
        assignment_selection: Option<Vec<String>>,
        invocation_input: Option<Value>,
    },
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
            Self::Invoke { .. } => "invoke",
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
    WaitInvocation {
        options: CliOptions,
        run_id: RunId,
        invocation_id: InvocationId,
    },
    StdinExec {
        args: StdinExecArgs,
    },
    FanOut {
        options: CliOptions,
        args: FanOutArgs,
    },
    FanOutJoin {
        capture_dir: PathBuf,
    },
    FanOutWorker {
        capture_dir: PathBuf,
        worker_index: usize,
    },
    PreviewBindings {
        options: CliOptions,
        operand: Option<String>,
    },
    InvocationProgress {
        options: CliOptions,
        run_id: RunId,
        invocation_id: Option<InvocationId>,
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
    let mut fan_out_tokens = Vec::new();
    let mut stdin_file = None;
    let mut exit_mode = None;
    let mut sidecar_file = None;
    let mut capture_dir = None;
    let mut worker_index = None;
    let mut compact = false;
    let mut assignment_selection: Option<Vec<String>> = None;
    let mut assignment_flags_seen = false;
    let mut assignments_option_seen = false;

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
                "--compact" => {
                    compact = true;
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
                "--worker" => {
                    let value = next_option_value(args, &mut index, token)?;
                    fan_out_tokens.push("--worker".to_owned());
                    fan_out_tokens.push(value);
                    continue;
                }
                value if value.starts_with("--worker=") => {
                    fan_out_tokens.push("--worker".to_owned());
                    fan_out_tokens.push(
                        value
                            .strip_prefix("--worker=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--instructions" => {
                    let value = next_option_value(args, &mut index, token)?;
                    fan_out_tokens.push("--instructions".to_owned());
                    fan_out_tokens.push(value);
                    continue;
                }
                value if value.starts_with("--instructions=") => {
                    fan_out_tokens.push("--instructions".to_owned());
                    fan_out_tokens.push(
                        value
                            .strip_prefix("--instructions=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--max-active" => {
                    let value = next_option_value(args, &mut index, token)?;
                    fan_out_tokens.push("--max-active".to_owned());
                    fan_out_tokens.push(value);
                    continue;
                }
                value if value.starts_with("--max-active=") => {
                    fan_out_tokens.push("--max-active".to_owned());
                    fan_out_tokens.push(
                        value
                            .strip_prefix("--max-active=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--stdin-file" => {
                    stdin_file = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--stdin-file=") => {
                    stdin_file = Some(
                        value
                            .strip_prefix("--stdin-file=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--exit-mode" => {
                    exit_mode = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--exit-mode=") => {
                    exit_mode = Some(
                        value
                            .strip_prefix("--exit-mode=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--sidecar-file" => {
                    sidecar_file = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--sidecar-file=") => {
                    sidecar_file = Some(
                        value
                            .strip_prefix("--sidecar-file=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--capture-dir" => {
                    capture_dir = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--capture-dir=") => {
                    capture_dir = Some(
                        value
                            .strip_prefix("--capture-dir=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--worker-index" => {
                    worker_index = Some(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--worker-index=") => {
                    worker_index = Some(
                        value
                            .strip_prefix("--worker-index=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--assignment" => {
                    if assignments_option_seen {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "`--assignment` may not be combined with `--assignments`",
                        ));
                    }
                    assignment_flags_seen = true;
                    assignment_selection
                        .get_or_insert_with(Vec::new)
                        .push(next_option_value(args, &mut index, token)?);
                    continue;
                }
                value if value.starts_with("--assignment=") => {
                    if assignments_option_seen {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "`--assignment` may not be combined with `--assignments`",
                        ));
                    }
                    assignment_flags_seen = true;
                    assignment_selection.get_or_insert_with(Vec::new).push(
                        value
                            .strip_prefix("--assignment=")
                            .expect("checked prefix")
                            .to_owned(),
                    );
                    index += 1;
                    continue;
                }
                "--assignments" => {
                    if assignment_flags_seen || assignments_option_seen {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "`--assignments` may be supplied at most once and may not be combined with `--assignment`",
                        ));
                    }
                    assignments_option_seen = true;
                    let raw = next_option_value(args, &mut index, token)?;
                    assignment_selection = Some(parse_assignment_selection_value(&raw)?);
                    continue;
                }
                value if value.starts_with("--assignments=") => {
                    if assignment_flags_seen || assignments_option_seen {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "`--assignments` may be supplied at most once and may not be combined with `--assignment`",
                        ));
                    }
                    assignments_option_seen = true;
                    assignment_selection = Some(parse_assignment_selection_value(
                        value
                            .strip_prefix("--assignments=")
                            .expect("checked prefix"),
                    )?);
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
            "a primary operation is required (start, list, show, append, event, history, terminate, or invoke)",
        )
    })?;

    if compact && command_name != "show" {
        return Err(CliError::new(
            "invalid-invocation",
            "`--compact` is only valid for the existing show operation",
        ));
    }
    if compact && options.output == OutputFormat::Json {
        return Err(CliError::new(
            "invalid-invocation",
            "show --compact is human-only and cannot be combined with --json, --format=json, or --output=json",
        ));
    }
    options.compact = compact;

    if command_name != "invoke" && assignment_selection.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "assignment selection is only valid for invoke",
        ));
    }

    if command_name == "wait-invocation" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        reject_capture_dir_option(&capture_dir)?;
        reject_worker_index_option(&worker_index)?;
        return parse_wait_invocation(options, positionals);
    }

    if command_name == "stdin-exec" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_capture_dir_option(&capture_dir)?;
        reject_worker_index_option(&worker_index)?;
        return parse_stdin_exec(stdin_file, exit_mode, sidecar_file, positionals);
    }

    if command_name == "fan-out-join" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        reject_worker_index_option(&worker_index)?;
        return parse_fan_out_join(capture_dir, positionals);
    }

    if command_name == "fan-out-worker" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        return parse_fan_out_worker(capture_dir, worker_index, positionals);
    }

    if command_name == "fan-out" {
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        reject_capture_dir_option(&capture_dir)?;
        reject_worker_index_option(&worker_index)?;
        fan_out_tokens.extend(positionals);
        return parse_fan_out_request(options, fan_out_tokens);
    }

    if command_name == "preview-bindings" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        reject_capture_dir_option(&capture_dir)?;
        reject_worker_index_option(&worker_index)?;
        reject_unrelated_options(
            "preview-bindings",
            provider,
            input,
            label,
            start_id,
            kind,
            data,
            record_id,
            event,
        )?;
        if run_id.is_some() {
            return Err(CliError::new(
                "invalid-invocation",
                "preview-bindings does not accept a run ID",
            ));
        }
        return parse_preview_bindings_request(options, positionals);
    }

    if command_name == "invocation-progress" {
        if let Some(option) = fan_out_tokens.first() {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown option `{option}`"),
            ));
        }
        reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
        reject_capture_dir_option(&capture_dir)?;
        reject_worker_index_option(&worker_index)?;
        reject_unrelated_options(
            "invocation-progress",
            provider,
            input,
            label,
            start_id,
            kind,
            data,
            record_id,
            event,
        )?;
        if run_id.is_some() {
            return Err(CliError::new(
                "invalid-invocation",
                "invocation-progress takes RUN_ID as a positional argument",
            ));
        }
        return parse_invocation_progress_request(options, positionals);
    }

    if let Some(option) = fan_out_tokens.first() {
        return Err(CliError::new(
            "invalid-invocation",
            format!("unknown option `{option}`"),
        ));
    }
    reject_stdin_exec_options(&stdin_file, &exit_mode, &sidecar_file)?;
    reject_capture_dir_option(&capture_dir)?;
    reject_worker_index_option(&worker_index)?;

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
        assignment_selection,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryInput {
    #[serde(default)]
    source_record_id: String,
    invocation_id: String,
    assignment_id: String,
    attesting_driver: Value,
    #[serde(default)]
    overridden_inputs: Vec<String>,
}

fn parse_carry_request(data: Value, act: CarryAct) -> Result<CarryRequest, CliError> {
    let input = serde_json::from_value::<CarryInput>(data).map_err(|error| {
        CliError::new(
            "invalid-carry-request",
            format!(
                "{act:?} data must name invocation_id, assignment_id, and attesting_driver; source_record_id is optional for plan-task results: {error}"
            ),
        )
    })?;
    Ok(CarryRequest::new(
        input.source_record_id,
        input.invocation_id,
        input.assignment_id,
        act,
        input.attesting_driver,
    )
    .with_overridden_inputs(input.overridden_inputs))
}

fn parse_assignment_selection_value(raw: &str) -> Result<Vec<String>, CliError> {
    if raw.trim_start().starts_with('[') {
        return serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
            CliError::new(
                "invalid-invocation",
                format!("`--assignments` must be comma-separated or a JSON string array: {error}"),
            )
        });
    }
    Ok(raw.split(',').map(str::to_owned).collect())
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
    assignment_selection: Option<Vec<String>>,
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
                None,
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
        "invoke" => {
            let run = run_id.or_else(|| take_positional(&mut positionals));
            let run = required(run, "run ID")?;
            let slot = take_positional(&mut positionals);
            let slot = required(slot, "slot ID")?;
            ensure_no_positionals(&positionals, name)?;
            if input.is_some() && assignment_selection.is_some() {
                return Err(CliError::new(
                    "invalid-invocation",
                    "`--input` may not be combined with `--assignment` or `--assignments`",
                ));
            }
            let invocation_input = input
                .map(|raw| parse_json_source(&raw, "invocation input"))
                .transpose()?;
            reject_unrelated_options(
                name,
                provider,
                None,
                label,
                start_id,
                kind,
                data,
                record_id,
                event,
            )?;
            Ok(PrimaryCommand::Invoke {
                run_id: run.into(),
                slot_id: slot,
                assignment_selection,
                invocation_input,
            })
        }
        _ => Err(CliError::new(
            "invalid-operation",
            format!(
                "unknown operation `{name}`; expected start, list, show, append, event, history, terminate, or invoke"
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

fn reject_capture_dir_option(capture_dir: &Option<String>) -> Result<(), CliError> {
    if capture_dir.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "unknown option `--capture-dir`",
        ));
    }
    Ok(())
}

fn reject_worker_index_option(worker_index: &Option<String>) -> Result<(), CliError> {
    if worker_index.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "unknown option `--worker-index`",
        ));
    }
    Ok(())
}

fn reject_stdin_exec_options(
    stdin_file: &Option<String>,
    exit_mode: &Option<String>,
    sidecar_file: &Option<String>,
) -> Result<(), CliError> {
    if stdin_file.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "unknown option `--stdin-file`",
        ));
    }
    if exit_mode.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "unknown option `--exit-mode`",
        ));
    }
    if sidecar_file.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "unknown option `--sidecar-file`",
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
        ParsedRequest::WaitInvocation {
            options,
            run_id,
            invocation_id,
        } => execute_wait_invocation(options, run_id, invocation_id),
        ParsedRequest::StdinExec { args } => execute_stdin_exec(args),
        ParsedRequest::FanOut { options, args } => execute_fan_out(options, args),
        ParsedRequest::FanOutJoin { capture_dir } => execute_fan_out_join(capture_dir),
        ParsedRequest::FanOutWorker {
            capture_dir,
            worker_index,
        } => execute_fan_out_worker(capture_dir, worker_index),
        ParsedRequest::PreviewBindings { options, operand } => {
            execute_preview_bindings(options, operand)
        }
        ParsedRequest::InvocationProgress {
            options,
            run_id,
            invocation_id,
        } => invocation_progress::execute_invocation_progress(options, run_id, invocation_id),
    }
}

#[derive(Clone, Debug, Deserialize)]
struct WaiterEnvelope {
    command: String,
    args: Vec<String>,
    worker_packet: Value,
}

fn parse_fan_out_join(
    capture_dir: Option<String>,
    positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let capture_dir = required(capture_dir, "--capture-dir path")?;
    ensure_no_positionals(&positionals, "fan-out-join")?;
    Ok(ParsedRequest::FanOutJoin {
        capture_dir: PathBuf::from(capture_dir),
    })
}

fn parse_fan_out_worker(
    capture_dir: Option<String>,
    worker_index: Option<String>,
    positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let capture_dir = required(capture_dir, "--capture-dir path")?;
    let worker_index = required(worker_index, "--worker-index")?;
    let worker_index = worker_index.parse::<usize>().map_err(|_| {
        CliError::new(
            "invalid-invocation",
            format!("--worker-index must be a non-negative integer, got `{worker_index}`"),
        )
    })?;
    ensure_no_positionals(&positionals, "fan-out-worker")?;
    Ok(ParsedRequest::FanOutWorker {
        capture_dir: PathBuf::from(capture_dir),
        worker_index,
    })
}

fn execute_fan_out_join(capture_dir: PathBuf) -> Execution {
    match fan_out::run_fan_out_join(&capture_dir) {
        Ok(()) => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(fan_out::CollectorError::Invalid(message)) => Execution {
            exit_code: EXIT_INVALID_INVOCATION,
            stdout: String::new(),
            stderr: format!("fan-out-join error: {message}\n"),
        },
        Err(fan_out::CollectorError::Failed(message)) => Execution {
            exit_code: EXIT_ERROR,
            stdout: String::new(),
            stderr: format!("fan-out-join error: {message}\n"),
        },
    }
}

fn execute_fan_out_worker(capture_dir: PathBuf, worker_index: usize) -> Execution {
    match fan_out::run_fan_out_worker(&capture_dir, worker_index) {
        Ok(()) => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(fan_out::CollectorError::Invalid(message)) => Execution {
            exit_code: EXIT_INVALID_INVOCATION,
            stdout: String::new(),
            stderr: format!("fan-out-worker error: {message}\n"),
        },
        Err(fan_out::CollectorError::Failed(message)) => Execution {
            exit_code: EXIT_ERROR,
            stdout: String::new(),
            stderr: format!("fan-out-worker error: {message}\n"),
        },
    }
}

fn parse_fan_out_request(
    options: CliOptions,
    tokens: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let args = fan_out::parse_fan_out_args(&tokens)
        .map_err(|error| CliError::new("invalid-invocation", error.to_string()))?;
    Ok(ParsedRequest::FanOut { options, args })
}

fn execute_fan_out(options: CliOptions, args: FanOutArgs) -> Execution {
    let output = options.output;
    let stdin_bytes = if fan_out::drain_stdin(io::stdin().is_terminal()) {
        let mut stdin_bytes = Vec::new();
        if let Err(error) = io::stdin().read_to_end(&mut stdin_bytes) {
            return fan_out_failed(format!("could not read stdin: {error}"));
        }
        stdin_bytes
    } else {
        Vec::new()
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return fan_out_failed(format!("could not determine current directory: {error}"))
        }
    };
    match fan_out::run_collector(args, &stdin_bytes, &cwd) {
        Ok(summary) => match serde_json::to_string(&summary) {
            Ok(stdout) => Execution {
                exit_code: EXIT_COMPLETED,
                stdout: format!("{stdout}\n"),
                stderr: String::new(),
            },
            Err(error) => fan_out_failed(format!("could not serialize fan-out summary: {error}")),
        },
        Err(fan_out::CollectorError::Invalid(message)) => render_invalid_invocation_with_format(
            CliError::new("invalid-invocation", message),
            output,
        ),
        Err(fan_out::CollectorError::Failed(message)) => fan_out_failed(message),
    }
}

fn fan_out_failed(message: String) -> Execution {
    Execution {
        exit_code: EXIT_ERROR,
        stdout: String::new(),
        stderr: format!("fan-out error: {message}\n"),
    }
}

fn parse_preview_bindings_request(
    options: CliOptions,
    mut positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let operand = take_positional(&mut positionals);
    ensure_no_positionals(&positionals, "preview-bindings")?;
    Ok(ParsedRequest::PreviewBindings { options, operand })
}

fn parse_invocation_progress_request(
    options: CliOptions,
    mut positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let run_id = required(take_positional(&mut positionals), "run ID")?;
    let invocation_id = take_positional(&mut positionals);
    ensure_no_positionals(&positionals, "invocation-progress")?;
    Ok(ParsedRequest::InvocationProgress {
        options,
        run_id: run_id.into(),
        invocation_id: invocation_id.map(InvocationId::from),
    })
}

fn execute_preview_bindings(options: CliOptions, operand: Option<String>) -> Execution {
    let warn_default_timeout = options.provider_timeout.is_none();
    let source = match preview_bindings::load_from_stdin_or_operand(operand.as_deref()) {
        Ok(source) => source,
        Err(error) => {
            return render_invalid_invocation_with_format(
                CliError::new("invalid-invocation", error.message),
                options.output,
            );
        }
    };
    match preview_bindings::preview(&source, warn_default_timeout) {
        Ok(report) => {
            let exit_code = if report.errors.is_empty() {
                EXIT_COMPLETED
            } else {
                EXIT_INVALID_INVOCATION
            };
            match serde_json::to_string(&report) {
                Ok(stdout) => Execution {
                    exit_code,
                    stdout: format!("{stdout}\n"),
                    stderr: String::new(),
                },
                Err(error) => Execution {
                    exit_code: EXIT_ERROR,
                    stdout: String::new(),
                    stderr: format!("preview-bindings error: {error}\n"),
                },
            }
        }
        Err(error) => render_invalid_invocation_with_format(
            CliError::new("invalid-invocation", error.message),
            options.output,
        ),
    }
}

fn parse_stdin_exec(
    stdin_file: Option<String>,
    exit_mode: Option<String>,
    sidecar_file: Option<String>,
    mut positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let stdin_file = required(stdin_file, "--stdin-file path")?;
    let exit_mode_raw = required(exit_mode, "--exit-mode")?;
    let exit_mode = match exit_mode_raw.as_str() {
        "sidecar" => StdinExecExitMode::Sidecar,
        "propagate" => StdinExecExitMode::Propagate,
        other => {
            return Err(CliError::new(
                "invalid-invocation",
                format!("unknown --exit-mode `{other}`; expected sidecar or propagate"),
            ))
        }
    };
    match exit_mode {
        StdinExecExitMode::Sidecar => {
            if sidecar_file.is_none() {
                return Err(CliError::new(
                    "invalid-invocation",
                    "sidecar mode requires --sidecar-file",
                ));
            }
        }
        StdinExecExitMode::Propagate => {
            if sidecar_file.is_some() {
                return Err(CliError::new(
                    "invalid-invocation",
                    "--sidecar-file is rejected in propagate mode",
                ));
            }
        }
    }
    let command = required(take_positional(&mut positionals), "COMMAND")?;
    Ok(ParsedRequest::StdinExec {
        args: StdinExecArgs {
            stdin_file: PathBuf::from(stdin_file),
            exit_mode,
            sidecar_file: sidecar_file.map(PathBuf::from),
            command,
            args: positionals,
        },
    })
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

fn execute_stdin_exec(args: StdinExecArgs) -> Execution {
    let stdin = match fs::File::open(&args.stdin_file) {
        Ok(file) => file,
        Err(error) => {
            return stdin_exec_failed(
                format!(
                    "could not open --stdin-file {}: {error}",
                    args.stdin_file.display()
                ),
                EXIT_ERROR,
            )
        }
    };
    let session_dir = match prepare_child_session_dir(&args.stdin_file) {
        Ok(path) => path,
        Err(message) => return stdin_exec_failed(message, EXIT_ERROR),
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
                EXIT_ERROR,
            )
        }
    };
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            return stdin_exec_failed(
                format!("could not wait for `{}`: {error}", args.command),
                EXIT_ERROR,
            )
        }
    };
    let exit_code = inner_waitpid_as_i32(status);
    match args.exit_mode {
        StdinExecExitMode::Sidecar => {
            let Some(sidecar_file) = args.sidecar_file.as_ref() else {
                return stdin_exec_failed(
                    "sidecar mode requires --sidecar-file".to_owned(),
                    EXIT_INVALID_INVOCATION,
                );
            };
            if let Some(parent) = sidecar_file.parent() {
                if !parent.as_os_str().is_empty() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        return stdin_exec_failed(
                            format!(
                                "could not create sidecar directory {}: {error}",
                                parent.display()
                            ),
                            EXIT_ERROR,
                        );
                    }
                }
            }
            let body = match serde_json::to_vec(&json!({ "exit_code": exit_code })) {
                Ok(body) => body,
                Err(error) => {
                    return stdin_exec_failed(
                        format!("could not serialize sidecar JSON: {error}"),
                        EXIT_ERROR,
                    )
                }
            };
            if let Err(error) = fs::write(sidecar_file, body) {
                return stdin_exec_failed(
                    format!(
                        "could not write sidecar {}: {error}",
                        sidecar_file.display()
                    ),
                    EXIT_ERROR,
                );
            }
            Execution {
                exit_code: EXIT_COMPLETED,
                stdout: String::new(),
                stderr: String::new(),
            }
        }
        StdinExecExitMode::Propagate => Execution {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
        },
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

fn stdin_exec_failed(message: String, exit_code: i32) -> Execution {
    Execution {
        exit_code,
        stdout: String::new(),
        stderr: format!("stdin-exec error: {message}\n"),
    }
}

fn parse_wait_invocation(
    options: CliOptions,
    mut positionals: Vec<String>,
) -> Result<ParsedRequest, CliError> {
    let run_id = required(take_positional(&mut positionals), "run ID")?;
    let invocation_id = required(take_positional(&mut positionals), "invocation ID")?;
    ensure_no_positionals(&positionals, "wait-invocation")?;
    Ok(ParsedRequest::WaitInvocation {
        options,
        run_id: run_id.into(),
        invocation_id: invocation_id.into(),
    })
}

fn execute_wait_invocation(
    options: CliOptions,
    run_id: RunId,
    invocation_id: InvocationId,
) -> Execution {
    let output = options.output;
    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return render_operation_error("wait-invocation", output, error),
    };
    let persistence = match open_persistence(&paths.database) {
        Ok(persistence) => persistence,
        Err(error) => return render_operation_error("wait-invocation", output, error),
    };
    let envelope = match read_waiter_envelope() {
        Ok(envelope) => envelope,
        Err(error) => return render_invalid_invocation_with_format(error, output),
    };
    match wait_for_worker_and_complete(&persistence, run_id, invocation_id, envelope) {
        Ok(()) => Execution {
            exit_code: EXIT_COMPLETED,
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(error) => {
            if error.code == "invalid-invocation" || error.code == "invalid-json-input" {
                render_invalid_invocation_with_format(error, output)
            } else {
                render_operation_error("wait-invocation", output, error)
            }
        }
    }
}

fn read_waiter_envelope() -> Result<WaiterEnvelope, CliError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).map_err(|error| {
        CliError::new(
            "input-read-failed",
            format!("could not read waiter envelope from stdin: {error}"),
        )
    })?;
    serde_json::from_str(&input).map_err(|error| {
        CliError::new(
            "invalid-json-input",
            format!("waiter envelope is not valid JSON: {error}"),
        )
    })
}

fn wait_for_worker_and_complete(
    persistence: &SqlitePersistence,
    run_id: RunId,
    invocation_id: InvocationId,
    envelope: WaiterEnvelope,
) -> Result<(), CliError> {
    let mut child = Command::new(&envelope.command)
        .args(&envelope.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            CliError::new(
                "worker-spawn-failed",
                format!("could not spawn waiter worker: {error}"),
            )
        })?;

    let packet = serde_json::to_vec(&envelope.worker_packet).map_err(|error| {
        CliError::new(
            "worker-packet-serialization-failed",
            format!("could not serialize worker packet: {error}"),
        )
    })?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(error) = stdin.write_all(&packet) {
            if error.kind() != io::ErrorKind::BrokenPipe {
                return Err(CliError::new(
                    "worker-stdin-write-failed",
                    format!("could not write worker packet to stdin: {error}"),
                ));
            }
        }
    }

    let status = child.wait().map_err(|error| {
        CliError::new(
            "worker-wait-failed",
            format!("could not wait for waiter worker: {error}"),
        )
    })?;
    let exit_code = status.code().unwrap_or(1);
    let written = if status.success() {
        WaiterWrittenStatus::Succeeded
    } else {
        WaiterWrittenStatus::Failed
    };
    let inner_workers = inner_workers_after_reap(persistence, &run_id, &invocation_id);

    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            written,
            exit_code,
            now_timestamp(),
            inner_workers,
        ))
        .map_err(|error| CliError::new(error.code(), error.to_string()))?;
    Ok(())
}

fn inner_workers_after_reap(
    persistence: &SqlitePersistence,
    run_id: &RunId,
    invocation_id: &InvocationId,
) -> Vec<InnerWorker> {
    let Ok(invocations) = persistence.load_work_slot_invocations(run_id) else {
        return Vec::new();
    };
    let Some(invocation) = invocations
        .into_iter()
        .find(|invocation| invocation.invocation_id == *invocation_id)
    else {
        return Vec::new();
    };
    inner_workers_from_capture_dir(&invocation.capture_dir)
}

fn inner_workers_from_capture_dir(capture_dir: &str) -> Vec<InnerWorker> {
    if capture_dir.is_empty() {
        return Vec::new();
    }
    let bytes = match fs::read(Path::new(capture_dir).join("summary.json")) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    parse_summary_inner_workers(&bytes).unwrap_or_default()
}

#[derive(Deserialize)]
struct SummaryFile {
    workers: Vec<SummaryWorker>,
}

#[derive(Deserialize)]
struct SummaryWorker {
    #[serde(default)]
    assignment_id: Option<String>,
    command: String,
    args: Vec<String>,
    exit_code: i32,
    #[serde(default)]
    selected_attempt: Option<Option<u32>>,
    #[serde(default)]
    selected_output_sha256: Option<String>,
    #[serde(default)]
    selected_output_path: Option<String>,
    #[serde(default)]
    declared_output_contract: Option<Value>,
    #[serde(default)]
    routed_inputs: Option<Value>,
    #[serde(default)]
    task_definition: Option<Value>,
    #[serde(default)]
    task_packet: Option<Value>,
    #[serde(default)]
    dependencies: Option<Vec<String>>,
    #[serde(default)]
    repository_effect: Option<Value>,
}

fn parse_summary_inner_workers(bytes: &[u8]) -> Option<Vec<InnerWorker>> {
    serde_json::from_slice::<SummaryFile>(bytes)
        .ok()
        .and_then(|summary| normalize_summary_inner_workers(summary.workers))
}

fn normalize_summary_inner_workers(workers: Vec<SummaryWorker>) -> Option<Vec<InnerWorker>> {
    let any_assignment_ids = workers.iter().any(|worker| {
        worker
            .assignment_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    });
    let mut assignment_ids = BTreeSet::new();
    let mut normalized = Vec::with_capacity(workers.len());
    for worker in workers {
        // A summary produced by the current fan-out and plan-graph paths has
        // an identity for every worker. Preserve the legacy all-omitted shape,
        // but never persist a partial or colliding provider-supplied set.
        let assignment_id = worker.assignment_id.unwrap_or_default();
        if any_assignment_ids
            && (assignment_id.is_empty() || !assignment_ids.insert(assignment_id.clone()))
        {
            return None;
        }

        let selected_attempt = worker.selected_attempt.flatten();
        let has_digest = worker.selected_output_sha256.is_some();
        let has_path = worker.selected_output_path.is_some();
        // A selected attempt must carry both durable linkage facts. A
        // coverage gap carries neither; a half-linked record is not a result.
        if has_digest != has_path || selected_attempt.is_some() != has_digest {
            return None;
        }
        if let (Some(attempt), Some(path)) =
            (selected_attempt, worker.selected_output_path.as_deref())
        {
            // Fan-out retries store selected bytes below attempts/<n>/stdout.
            // Plan-graph tasks have one provider-owned attempt and retain
            // their direct task stdout path; task_definition distinguishes
            // that shape from the compatibility worker-level stdout copy.
            let mut components = Path::new(path).components().rev();
            let is_originating_attempt =
                matches!(
                    components.next(),
                    Some(std::path::Component::Normal(name)) if name == "stdout"
                ) && components.next().and_then(|component| match component {
                    std::path::Component::Normal(name) => name.to_str()?.parse::<u32>().ok(),
                    _ => None,
                }) == Some(attempt)
                    && matches!(
                        components.next(),
                        Some(std::path::Component::Normal(name)) if name == "attempts"
                    );
            let single_components = Path::new(path).components().rev().collect::<Vec<_>>();
            let is_single_attempt_output = attempt == 1
                && matches!(
                    single_components.first(),
                    Some(std::path::Component::Normal(name)) if *name == "stdout"
                )
                && matches!(
                    single_components.get(1),
                    Some(std::path::Component::Normal(name))
                        if name.to_str().is_some_and(|value| value.parse::<usize>().is_ok())
                );
            let is_plan_task_output = worker.task_definition.is_some();
            if !is_originating_attempt && !is_single_attempt_output && !is_plan_task_output {
                return None;
            }
        }

        normalized.push(InnerWorker {
            assignment_id,
            command: worker.command,
            args: worker.args,
            exit_code: worker.exit_code,
            selected_attempt,
            selected_output_sha256: worker.selected_output_sha256,
            selected_output_path: worker.selected_output_path,
            declared_output_contract: worker.declared_output_contract,
            routed_inputs: worker.routed_inputs,
            task_definition: worker.task_definition,
            task_packet: worker.task_packet,
            dependencies: worker.dependencies,
            repository_effect: worker.repository_effect,
        });
    }
    Some(normalized)
}

struct CliWorkSlotProcess {
    binary: PathBuf,
}

struct CliWaiterHandle {
    child: process::Child,
}

fn same_executable_file(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        if left == right {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(left) = std::fs::metadata(left) else {
            return false;
        };
        let Ok(right) = std::fs::metadata(right) else {
            return false;
        };
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
mod unix_signal {
    extern "C" {
        pub fn kill(pid: i32, sig: i32) -> i32;
    }
}

impl WorkSlotProcess for CliWorkSlotProcess {
    type Handle = CliWaiterHandle;

    fn waiter_alive(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            unsafe { unix_signal::kill(pid as i32, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }

    fn enumerate_assignments(
        &self,
        binding: &loop_core::WorkSlotBinding,
    ) -> std::result::Result<Option<Vec<String>>, ProcessError> {
        // Only this executable's bound `fan-out` command is known to consume
        // and enforce the engine's assignment-selection packet. Argv that
        // merely resembles fan-out behind another executable is opaque.
        if !same_executable_file(&self.binary, Path::new(&binding.command)) {
            return Ok(None);
        }
        Ok(fan_out::enumerate_bound_assignments(binding))
    }

    fn spawn_wait_invocation(
        &self,
        args: WaiterSpawnArgs,
    ) -> std::result::Result<StartedWaiter<CliWaiterHandle>, ProcessError> {
        let database = args.database.to_str().ok_or_else(|| {
            ProcessError::new("invalid-database-path", "database path is not valid UTF-8")
        })?;
        let child = Command::new(&self.binary)
            .args([
                "--database",
                database,
                "wait-invocation",
                args.run_id.as_str(),
                args.invocation_id.as_str(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                ProcessError::new(
                    "waiter-spawn-failed",
                    format!("could not spawn wait-invocation: {error}"),
                )
            })?;
        Ok(StartedWaiter::new(child.id(), CliWaiterHandle { child }))
    }

    fn send_envelope_and_detach(
        &self,
        mut waiter: StartedWaiter<CliWaiterHandle>,
        envelope_json: &[u8],
    ) -> std::result::Result<(), ProcessError> {
        if let Some(mut stdin) = waiter.handle.child.stdin.take() {
            if let Err(error) = stdin.write_all(envelope_json) {
                if error.kind() != io::ErrorKind::BrokenPipe {
                    return Err(ProcessError::new(
                        "waiter-stdin-write-failed",
                        format!("could not write waiter envelope to stdin: {error}"),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn execute_operation(options: CliOptions, command: PrimaryCommand) -> Execution {
    let output = options.output;
    let compact = options.compact;
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
                catalog_root(&paths.database),
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
            let binary = match std::env::current_exe() {
                Ok(binary) => binary,
                Err(error) => {
                    return render_operation_error(
                        operation,
                        output,
                        CliError::new(
                            "current-executable-unavailable",
                            format!("could not resolve current executable: {error}"),
                        ),
                    );
                }
            };
            let now = now_timestamp();
            let outcome = core::execute_show(
                ShowRequest::new(run_id),
                &persistence,
                &CliWorkSlotProcess { binary },
                now,
            )
            .map(CliShowProjection::from);
            if compact {
                render_show_compact(&persistence, &outcome, timeout, now)
            } else {
                render_operation(operation, output, &outcome)
            }
        }
        PrimaryCommand::Append(args) => {
            let request = AppendContextRequest::new(
                args.run_id.clone(),
                args.record_id.unwrap_or_else(new_context_id),
                args.kind.clone(),
                args.data.clone(),
                now_timestamp(),
            );
            let request = if matches!(args.kind.as_str(), "unchanged-carry" | "override-carry") {
                let act = if args.kind == "unchanged-carry" {
                    CarryAct::Unchanged
                } else {
                    CarryAct::Override
                };
                let carry = match parse_carry_request(args.data, act) {
                    Ok(carry) => carry,
                    Err(error) => return render_invalid_invocation_with_format(error, output),
                };
                request.with_carry(carry)
            } else {
                request
            };
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
        PrimaryCommand::Invoke {
            run_id,
            slot_id,
            assignment_selection,
            invocation_input,
        } => {
            let binary = match std::env::current_exe() {
                Ok(binary) => binary,
                Err(error) => {
                    return render_operation_error(
                        operation,
                        output,
                        CliError::new(
                            "waiter-spawn-failed",
                            format!("could not resolve current executable: {error}"),
                        ),
                    );
                }
            };
            let process = CliWorkSlotProcess { binary };
            let allowed_time_ms = timeout.as_millis().min(u64::MAX as u128) as u64;
            let request =
                InvokeRequest::new(run_id, slot_id, new_invocation_id(), paths.database.clone())
                    .with_assignment_selection(assignment_selection)
                    .with_invocation_input(invocation_input);
            let outcome = core::execute_invoke(
                request,
                &persistence,
                &process,
                now_timestamp(),
                allowed_time_ms,
            )
            .map(CliInvokeResult::from);
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

fn catalog_root(database: &Path) -> PathBuf {
    database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
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

fn new_invocation_id() -> InvocationId {
    unique_token("invocation").into()
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
    #[serde(skip_serializing_if = "Option::is_none")]
    guidance: Option<String>,
}

impl From<core::AppendContextResult> for CliAppendContextResult {
    fn from(result: core::AppendContextResult) -> Self {
        let guidance = result
            .context
            .data
            .get("loop_engine_carry")
            .and_then(|value| value.get("act"))
            .and_then(Value::as_str)
            .map(|act| {
                format!(
                    "Consult the change report of record; this append used {act}. Unchanged-carry requires every covered input to be unchanged. Override-carry must name every changed input it overrides."
                )
            });
        Self {
            run: result.run.into(),
            context: result.context,
            history: result.history,
            guidance,
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
struct CliInvokeResult {
    invocation_id: core::InvocationId,
    slot_id: core::WorkSlotId,
    started_at: core::Timestamp,
    allowed_time_ms: u64,
    capture_dir: String,
}

impl From<core::InvokeResult> for CliInvokeResult {
    fn from(result: core::InvokeResult) -> Self {
        Self {
            invocation_id: result.invocation_id,
            slot_id: result.slot_id,
            started_at: result.started_at,
            allowed_time_ms: result.allowed_time_ms,
            capture_dir: result.capture_dir,
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
    provider: Option<String>,
    artifact_root: Option<String>,
}

impl From<core::RunSummary> for CliRunSummary {
    fn from(summary: core::RunSummary) -> Self {
        Self {
            id: summary.id,
            label: summary.label,
            workflow_id: summary.workflow_id,
            lifecycle: summary.lifecycle,
            current_state: summary.current_state,
            provider: summary.provider,
            artifact_root: summary.artifact_root,
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
    work_slots: Vec<core::WorkSlot>,
    change_report: core::operations::RunChangeReport,
    work_slot_invocations: Vec<core::operations::WorkSlotInvocationView>,
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
            work_slots: projection.work_slots,
            change_report: projection.change_report,
            work_slot_invocations: projection.work_slot_invocations,
        }
    }
}

/// Inner progress is deliberately subordinate to the normal show projection.
/// A collection failure is rendered as unavailable rather than changing the
/// durable show outcome or guessing a task state.
enum CompactInnerProgress {
    Unavailable { code: String, message: String },
    Snapshot(invocation_progress::InvocationProgressSnapshot),
}

fn select_compact_invocation(
    projection: &CliShowProjection,
) -> Option<&core::operations::WorkSlotInvocationView> {
    let running = projection
        .work_slot_invocations
        .iter()
        .filter(|invocation| invocation.status == core::ProjectedInvocationStatus::Running)
        .collect::<Vec<_>>();
    if running.len() == 1 {
        return running.into_iter().next();
    }
    projection
        .work_slot_invocations
        .iter()
        .max_by_key(|invocation| invocation.started_at)
}

fn render_show_compact(
    persistence: &SqlitePersistence,
    outcome: &OperationOutcome<CliShowProjection>,
    timeout: Duration,
    now: Timestamp,
) -> Execution {
    match outcome {
        OperationOutcome::Completed(projection) => {
            let invocation = select_compact_invocation(projection);
            let progress = match invocation {
                Some(invocation) => match invocation_progress::collect_snapshot_for_invocation(
                    persistence,
                    &projection.run_id,
                    &invocation.invocation_id,
                    timeout,
                    now,
                ) {
                    Ok(snapshot) => CompactInnerProgress::Snapshot(snapshot),
                    Err(error) => CompactInnerProgress::Unavailable {
                        code: error.code,
                        message: error.message,
                    },
                },
                None => CompactInnerProgress::Unavailable {
                    code: "no-invocations".to_owned(),
                    message: "no work-slot invocation is recorded on this run".to_owned(),
                },
            };
            Execution {
                exit_code: EXIT_COMPLETED,
                stdout: render_compact_show(projection, invocation, progress),
                stderr: String::new(),
            }
        }
        OperationOutcome::Rejected(_) => Execution {
            exit_code: EXIT_REJECTED,
            stdout: render_operation_human("show", outcome),
            stderr: String::new(),
        },
        OperationOutcome::Error(_) => Execution {
            exit_code: EXIT_ERROR,
            stdout: render_operation_human("show", outcome),
            stderr: String::new(),
        },
    }
}

fn render_compact_show(
    projection: &CliShowProjection,
    invocation: Option<&core::operations::WorkSlotInvocationView>,
    progress: CompactInnerProgress,
) -> String {
    let run = match projection.label.as_deref() {
        Some(label) if !label.is_empty() => format!(
            "run: {} (label: {})",
            compact_text(projection.run_id.as_str()),
            compact_text(label)
        ),
        _ => format!("run: {}", compact_text(projection.run_id.as_str())),
    };
    let events = if projection.requestable_events.is_empty() {
        "none".to_owned()
    } else {
        projection
            .requestable_events
            .iter()
            .map(|event| {
                format!(
                    "{} -> {} ({})",
                    compact_text(event.event.as_str()),
                    compact_text(event.target.as_str()),
                    compact_transition_kind(event.kind)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let checked = compact_latest_evaluation(projection);
    // `show` also carries observation-time elapsed/remaining counters. Compact
    // omits them so an unchanged running invocation has stable human output.
    let invocation_line = invocation
        .map(|invocation| {
            let exit_code = invocation
                .exit_code
                .map_or_else(|| "pending".to_owned(), |code| code.to_string());
            format!(
                "invocation: {} slot={} status={} exit_code={}",
                compact_text(invocation.invocation_id.as_str()),
                compact_text(invocation.slot_id.as_str()),
                compact_invocation_status(invocation.status),
                exit_code,
            )
        })
        .unwrap_or_else(|| "invocation: none".to_owned());

    [
        "completed show --compact".to_owned(),
        run,
        format!("lifecycle: {}", compact_lifecycle(projection.lifecycle)),
        format!(
            "state: {} ({})",
            compact_text(projection.current_state.as_str()),
            compact_text(&projection.current_state_title)
        ),
        format!("requestable events: {events}"),
        format!("latest checked result: {checked}"),
        invocation_line,
        compact_progress_line(progress),
    ]
    .join("\n")
        + "\n"
}

fn compact_latest_evaluation(projection: &CliShowProjection) -> String {
    let Some(evaluation) = projection
        .latest_evaluations
        .iter()
        .max_by_key(|evaluation| evaluation.sequence)
    else {
        return "none".to_owned();
    };
    let result = if evaluation.is_allow() {
        "allow"
    } else {
        "deny"
    };
    let mut summary = format!(
        "{} event={} target={} kind={} sequence={}",
        result,
        compact_text(evaluation.transition.event.as_str()),
        compact_text(evaluation.transition.target.as_str()),
        compact_transition_kind(evaluation.transition.kind),
        evaluation.sequence,
    );
    if let Some(feedback) = evaluation.feedback() {
        summary.push_str(&format!(
            " code={} message={}",
            compact_text(&feedback.code),
            compact_text(&feedback.message)
        ));
    }
    summary
}

fn compact_progress_line(progress: CompactInnerProgress) -> String {
    match progress {
        CompactInnerProgress::Unavailable { code, message } => format!(
            "inner progress: unavailable [{}] {}",
            compact_text(&code),
            compact_text(&message)
        ),
        CompactInnerProgress::Snapshot(snapshot) => {
            let Some(graph) = snapshot.graph else {
                return "inner progress: unavailable [no-dagu-graph] no Dagu graph locator is available; task state is unknown".to_owned();
            };
            let mut not_started = 0;
            let mut running = 0;
            let mut reaped = 0;
            for step in graph.steps {
                match step.state {
                    invocation_progress::GraphStepState::NotStarted => not_started += 1,
                    invocation_progress::GraphStepState::Running => running += 1,
                    invocation_progress::GraphStepState::Reaped => reaped += 1,
                }
            }
            format!(
                "inner progress (Dagu helper liveness): steps={} not_started={} running={} reaped={}",
                not_started + running + reaped,
                not_started,
                running,
                reaped
            )
        }
    }
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_lifecycle(lifecycle: core::Lifecycle) -> &'static str {
    match lifecycle {
        core::Lifecycle::Active => "active",
        core::Lifecycle::Final => "final",
        core::Lifecycle::Terminated => "terminated",
    }
}

fn compact_invocation_status(status: core::ProjectedInvocationStatus) -> &'static str {
    match status {
        core::ProjectedInvocationStatus::Running => "running",
        core::ProjectedInvocationStatus::Succeeded => "succeeded",
        core::ProjectedInvocationStatus::Failed => "failed",
        core::ProjectedInvocationStatus::Overrun => "overrun",
    }
}

fn compact_transition_kind(kind: core::TransitionKind) -> &'static str {
    match kind {
        core::TransitionKind::Checked => "checked",
        core::TransitionKind::CheckFree => "check-free",
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
        Some("show") => {
            "Usage: loop-engine [options] show [--compact] <run-id>\n\n".to_owned()
                + "Without --compact, human output is the detailed projection. --compact is\n"
                + "human-only and summarizes the same show data plus opportunistic inner progress.\n"
        }
        Some("history") => "Usage: loop-engine [options] history <run-id>\n".to_owned(),
        Some("terminate") => "Usage: loop-engine [options] terminate <run-id>\n".to_owned(),
        Some("invoke") => {
            "Usage: loop-engine [options] invoke <run-id> <slot-id> [--assignment ID ... | --assignments ID,...]\n"
                .to_owned()
        }
        Some("list") => "Usage: loop-engine [options] list\n".to_owned(),
        Some("fan-out") => {
            "Usage: loop-engine [options] fan-out [--worker JSON]... [--instructions FILE] [--max-active N]\n\n"
                .to_owned()
                + "Start one process per --worker concurrently as a local Dagu type:graph under\n"
                + "an isolated home in the capture directory. Per-step progress is dagu status /\n"
                + "dagu history against capture_dir/dagu-locator.json. A mechanical join writes\n"
                + "summary.json. This is not a run-state operation and does not open the run\n"
                + "database. Callers do not supply Dagu YAML.\n\n"
                + "Options:\n"
                + "  --worker JSON            Strict nested worker object with command, args, and\n"
                + "                           optional preamble, legacy output_schema, or full_output_schema; repeatable\n"
                + "  --instructions FILE      Shared instructions file for ad hoc mode.\n"
                + "                           Bound mode reads the invoke packet from stdin instead.\n"
                + "  --max-active N           At most N worker steps run at once. Omitted means\n"
                + "                           uncapped concurrent worker start.\n"
        }
        Some("preview-bindings") => {
            "Usage: loop-engine [options] preview-bindings [JSON|@FILE]\n\n"
                .to_owned()
                + "Inspect work_slot_bindings without starting a run or opening the database.\n"
                + "Omitted operand reads stdin; @FILE reads that path; otherwise the operand is\n"
                + "inline JSON. Accepted JSON is a work_slot_bindings map or an object containing\n"
                + "that key. Reports a dagu PATH check (minimum 2.14.0) as ok with path and\n"
                + "version, or as a warning; warnings alone still exit 0. This is not a run-state\n"
                + "operation.\n"
        }
        Some("invocation-progress") => {
            "Usage: loop-engine [options] invocation-progress RUN_ID [INVOCATION_ID]\n\n"
                .to_owned()
                + "Open the catalog, select one invocation, and print a JSON progress snapshot of\n"
                + "that invocation's capture_dir graph liveness and already-associated traces.\n"
                + "Does not write overlay or invocation status. Show remains the overlay authority.\n\n"
                + "When INVOCATION_ID is omitted, the unique overlay-running invocation is selected\n"
                + "if one exists; otherwise the latest invocation by started_at.\n\n"
                + "Graph state is Dagu step-helper liveness (not_started|running|reaped), not overlay\n"
                + "success and not inner waitpid. Graph is omitted when capture_dir/dagu-locator.json\n"
                + "is absent. --timeout-ms bounds helper spawns only, never invocation allowed_time_ms.\n\n"
                + "Options: --database <path> --json --timeout-ms <milliseconds>\n"
        }
        _ => {
            "Usage: loop-engine [options] <operation> [arguments]\n\n"
                .to_owned()
                + "Operations:\n"
                + "  start\n"
                + "  list\n"
                + "  show\n"
                + "  append\n"
                + "  event\n"
                + "  history\n"
                + "  terminate\n"
                + "  invoke\n\n"
                + "Other commands:\n"
                + "  invocation-progress RUN_ID [INVOCATION_ID]\n"
                + "                             Snapshot capture_dir graph liveness and traces\n"
                + "                             Opens the catalog; graph state is Dagu helper liveness\n"
                + "                             --timeout-ms bounds helper spawns only\n"
                + "  fan-out                    Run worker CLIs concurrently via a local Dagu graph\n"
                + "                             --worker JSON          Nested worker contract; repeatable\n"
                + "                             --instructions FILE    Shared instructions (ad hoc mode)\n"
                + "                             --max-active N         Cap concurrent workers; omitted is uncapped\n"
                + "  preview-bindings [JSON|@FILE]  Inspect work_slot_bindings without starting a run\n"
                + "                             Omitted operand reads stdin; @FILE reads that path\n\n"
                + "Global options:\n"
                + "  --json, -j                 Render machine-readable JSON\n"
                + "  --database <path>          SQLite database path\n"
                + "  --config <path>            Provider TOML configuration path\n"
                + "  --timeout-ms <milliseconds> Provider operation timeout\n"
                + "  --help, -h                 Show help\n"
                + "  --version, -V              Show version\n"
        }
    }
}

#[cfg(test)]
#[test]
fn help_lists_fan_out_and_hides_wait_invocation() {
    let help = execute(["--help"]);
    assert_eq!(help.exit_code, EXIT_COMPLETED);
    assert!(
        help.stdout.contains("fan-out"),
        "help must list fan-out: {}",
        help.stdout
    );
    assert!(
        help.stdout.contains("invocation-progress"),
        "help must list invocation-progress: {}",
        help.stdout
    );
    assert!(
        !help.stdout.contains("wait-invocation"),
        "help must not mention hidden wait-invocation: {}",
        help.stdout
    );
    assert!(
        !help.stdout.contains("stdin-exec"),
        "help must not mention hidden stdin-exec: {}",
        help.stdout
    );
    assert!(
        !help.stdout.contains("fan-out-join"),
        "help must not mention hidden fan-out-join: {}",
        help.stdout
    );
    let fan_out_help = execute(["fan-out", "--help"]);
    assert_eq!(fan_out_help.exit_code, EXIT_COMPLETED);
    assert!(fan_out_help.stdout.contains("fan-out"));
    assert!(fan_out_help.stdout.contains("--max-active"));
    assert!(fan_out_help.stdout.contains("uncapped"));
    assert!(!fan_out_help.stdout.contains("wait-invocation"));
    assert!(!fan_out_help.stdout.contains("stdin-exec"));
    assert!(!fan_out_help.stdout.contains("fan-out-join"));
    let show_help = execute(["show", "--help"]);
    assert_eq!(show_help.exit_code, EXIT_COMPLETED);
    assert!(show_help.stdout.contains("show [--compact]"));
    assert!(show_help.stdout.contains("human-only"));

    let progress_help = execute(["invocation-progress", "--help"]);
    assert_eq!(progress_help.exit_code, EXIT_COMPLETED);
    assert!(progress_help.stdout.contains("invocation-progress"));
    assert!(progress_help.stdout.contains("RUN_ID [INVOCATION_ID]"));
    assert!(progress_help.stdout.contains("catalog"));
    assert!(progress_help.stdout.contains("--timeout-ms"));
    assert!(progress_help.stdout.contains("Dagu"));
    assert!(!progress_help.stdout.contains("wait-invocation"));
    assert!(!progress_help.stdout.contains("stdin-exec"));
    assert!(!progress_help.stdout.contains("fan-out-join"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{EvaluationFeedback, OutcomeIssue};
    use serde_json::json;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

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

    fn allocated_run_dir(database: &Path, run_id: &str) -> PathBuf {
        catalog_root(database)
            .join("runs")
            .join(run_id)
            .canonicalize()
            .expect("allocated run directory")
    }

    fn with_allocated_artifact_root(mut input: Value, database: &Path, run_id: &str) -> Value {
        input.as_object_mut().expect("object initial_input").insert(
            "artifact_root".to_owned(),
            json!(allocated_run_dir(database, run_id)
                .to_string_lossy()
                .into_owned()),
        );
        input
    }

    #[test]
    fn parser_exposes_exactly_the_eight_primary_operations() {
        let inputs = [
            vec!["start", "provider", "{}"],
            vec!["list"],
            vec!["show", "run-1"],
            vec!["append", "run-1", "note", "{}"],
            vec!["event", "run-1", "finish"],
            vec!["history", "run-1"],
            vec!["terminate", "run-1"],
            vec!["invoke", "run-1", "slot-1"],
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
                "terminate",
                "invoke"
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
            "invoke",
            "fan-out",
            "preview-bindings",
            "invocation-progress",
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
    fn invoke_run_slot_parses() {
        let parsed = parse_args(["invoke", "run-1", "slot-1"]).expect("invoke should parse");
        let ParsedRequest::Operation {
            command: PrimaryCommand::Invoke {
                run_id, slot_id, ..
            },
            options,
        } = parsed
        else {
            panic!("expected invoke operation");
        };
        assert_eq!(run_id.as_str(), "run-1");
        assert_eq!(slot_id, "slot-1");
        assert_eq!(options.output, OutputFormat::Human);
        assert!(options.database.is_none());
        assert!(options.provider_timeout.is_none());
    }

    #[test]
    fn invoke_assignment_selection_parses_repeated_and_comma_forms() {
        let parsed = parse_args([
            "invoke",
            "run-1",
            "slot-1",
            "--assignment",
            "worker-1",
            "--assignment=worker-2",
        ])
        .expect("repeated assignment selection should parse");
        let ParsedRequest::Operation {
            command:
                PrimaryCommand::Invoke {
                    assignment_selection,
                    ..
                },
            ..
        } = parsed
        else {
            panic!("expected invoke operation");
        };
        assert_eq!(
            assignment_selection,
            Some(vec!["worker-1".to_owned(), "worker-2".to_owned()])
        );

        let parsed = parse_args([
            "invoke",
            "run-1",
            "slot-1",
            "--assignments=worker-1,worker-2",
        ])
        .expect("comma assignment selection should parse");
        let ParsedRequest::Operation {
            command:
                PrimaryCommand::Invoke {
                    assignment_selection,
                    ..
                },
            ..
        } = parsed
        else {
            panic!("expected invoke operation");
        };
        assert_eq!(
            assignment_selection,
            Some(vec!["worker-1".to_owned(), "worker-2".to_owned()])
        );

        let parsed = parse_args(["invoke", "run-1", "slot-1", "--assignments=[]"])
            .expect("an explicitly empty selection should reach core validation");
        let ParsedRequest::Operation {
            command:
                PrimaryCommand::Invoke {
                    assignment_selection,
                    ..
                },
            ..
        } = parsed
        else {
            panic!("expected invoke operation");
        };
        assert_eq!(assignment_selection, Some(Vec::new()));
    }

    #[test]
    fn invoke_missing_args_is_invalid_invocation() {
        let missing_both = parse_args(["invoke"]).expect_err("invoke requires run and slot");
        assert_eq!(missing_both.code, "invalid-invocation");
        let missing_slot = parse_args(["invoke", "run-1"]).expect_err("invoke requires a slot ID");
        assert_eq!(missing_slot.code, "invalid-invocation");
        let extra = parse_args(["invoke", "run-1", "slot-1", "extra"])
            .expect_err("invoke rejects extra positionals");
        assert_eq!(extra.code, "invalid-invocation");
    }

    #[test]
    fn invoke_accepts_global_timeout_database_json_before_operation() {
        let parsed = parse_args([
            "--json",
            "--database",
            "/tmp/loop.db",
            "--timeout-ms",
            "12345",
            "invoke",
            "run-1",
            "slot-1",
        ])
        .expect("globals before invoke should parse");
        let ParsedRequest::Operation {
            options,
            command: PrimaryCommand::Invoke {
                run_id, slot_id, ..
            },
        } = parsed
        else {
            panic!("expected invoke operation");
        };
        assert_eq!(options.output, OutputFormat::Json);
        assert_eq!(options.database, Some(PathBuf::from("/tmp/loop.db")));
        assert_eq!(options.provider_timeout, Some(Duration::from_millis(12345)));
        assert_eq!(run_id.as_str(), "run-1");
        assert_eq!(slot_id, "slot-1");
    }

    #[test]
    fn help_lists_invoke_as_public_operation_and_hides_wait_invocation() {
        let help = execute(["--help"]);
        assert_eq!(help.exit_code, EXIT_COMPLETED);
        assert!(
            help.stdout.contains("invoke"),
            "help must list invoke: {}",
            help.stdout
        );
        assert!(
            help.stdout.contains("fan-out"),
            "help must list fan-out: {}",
            help.stdout
        );
        assert!(
            help.stdout.contains("preview-bindings"),
            "help must list preview-bindings: {}",
            help.stdout
        );
        assert!(
            help.stdout.contains("invocation-progress"),
            "help must list invocation-progress: {}",
            help.stdout
        );
        let (operations, other) = help
            .stdout
            .split_once("Other commands:")
            .expect("help must have Other commands");
        assert!(
            operations.contains("Operations:"),
            "help must list the eight operations: {operations}"
        );
        assert!(
            !operations.contains("preview-bindings"),
            "preview-bindings must not be a ninth primary operation: {operations}"
        );
        assert!(
            !operations.contains("invocation-progress"),
            "invocation-progress must not be a ninth primary operation: {operations}"
        );
        assert!(
            other.contains("preview-bindings"),
            "preview-bindings must be under Other commands: {other}"
        );
        assert!(
            other.contains("invocation-progress"),
            "invocation-progress must be under Other commands: {other}"
        );
        assert!(
            other.contains("fan-out"),
            "fan-out must be under Other commands: {other}"
        );
        assert!(
            help.stdout.contains("--worker"),
            "help must document --worker: {}",
            help.stdout
        );
        assert!(
            help.stdout.contains("--instructions"),
            "help must document --instructions: {}",
            help.stdout
        );
        assert!(
            help.stdout.contains("--max-active"),
            "help must document --max-active: {}",
            help.stdout
        );
        assert!(
            !help.stdout.contains("wait-invocation"),
            "help must not mention hidden wait-invocation: {}",
            help.stdout
        );
        assert!(
            !help.stdout.contains("stdin-exec"),
            "help must not mention hidden stdin-exec: {}",
            help.stdout
        );
        let invoke_help = execute(["invoke", "--help"]);
        assert_eq!(invoke_help.exit_code, EXIT_COMPLETED);
        assert!(invoke_help.stdout.contains("invoke"));
        assert!(!invoke_help.stdout.contains("wait-invocation"));
        assert!(!invoke_help.stdout.contains("stdin-exec"));
        assert!(!invoke_help.stdout.contains("fan-out-join"));
        let fan_out_help = execute(["fan-out", "--help"]);
        assert_eq!(fan_out_help.exit_code, EXIT_COMPLETED);
        assert!(fan_out_help.stdout.contains("fan-out"));
        assert!(fan_out_help.stdout.contains("--worker JSON"));
        assert!(fan_out_help.stdout.contains("--instructions FILE"));
        assert!(fan_out_help.stdout.contains("--max-active N"));
        assert!(fan_out_help
            .stdout
            .contains("uncapped concurrent worker start"));
        assert!(!fan_out_help.stdout.contains("wait-invocation"));
        assert!(!fan_out_help.stdout.contains("stdin-exec"));
        assert!(!fan_out_help.stdout.contains("fan-out-join"));
        let preview_help = execute(["preview-bindings", "--help"]);
        assert_eq!(preview_help.exit_code, EXIT_COMPLETED);
        assert!(preview_help.stdout.contains("preview-bindings"));
        assert!(preview_help.stdout.contains("[JSON|@FILE]"));
        assert!(!preview_help.stdout.contains("wait-invocation"));
        assert!(!preview_help.stdout.contains("stdin-exec"));
        assert!(!preview_help.stdout.contains("fan-out-join"));
        let show_help = execute(["show", "--help"]);
        assert_eq!(show_help.exit_code, EXIT_COMPLETED);
        assert!(show_help.stdout.contains("show [--compact]"));
        assert!(show_help.stdout.contains("human-only"));
        let progress_help = execute(["invocation-progress", "--help"]);
        assert_eq!(progress_help.exit_code, EXIT_COMPLETED);
        assert!(progress_help.stdout.contains("invocation-progress"));
        assert!(progress_help.stdout.contains("RUN_ID [INVOCATION_ID]"));
        assert!(progress_help.stdout.contains("catalog"));
        assert!(progress_help.stdout.contains("--timeout-ms"));
        assert!(progress_help.stdout.contains("helper"));
        assert!(progress_help.stdout.contains("Dagu"));
        assert!(!progress_help.stdout.contains("wait-invocation"));
        assert!(!progress_help.stdout.contains("stdin-exec"));
        assert!(!progress_help.stdout.contains("fan-out-join"));
    }

    #[test]
    fn parser_fan_out_is_not_a_primary_command() {
        let parsed = parse_args(["fan-out"]).expect("fan-out should parse");
        let ParsedRequest::FanOut { args, .. } = parsed else {
            panic!("expected ParsedRequest::FanOut, got {parsed:?}");
        };
        assert!(args.workers.is_empty());
        assert!(args.instructions_path.is_none());
        assert!(args.max_active.is_none());

        let worker = r#"{"command":"echo","args":[]}"#;
        let parsed = parse_args(["fan-out", "--worker", worker]).expect("fan-out with worker");
        let ParsedRequest::FanOut { args, options } = parsed else {
            panic!("expected ParsedRequest::FanOut");
        };
        assert_eq!(args.workers.len(), 1);
        assert_eq!(args.workers[0].command, "echo");
        assert!(args.max_active.is_none());
        assert!(options.database.is_none());

        let parsed = parse_args(["fan-out", "--max-active", "2", "--worker", worker])
            .expect("fan-out with --max-active");
        let ParsedRequest::FanOut { args, .. } = parsed else {
            panic!("expected ParsedRequest::FanOut");
        };
        assert_eq!(args.max_active, Some(2));
        assert_eq!(args.workers.len(), 1);

        let unknown = parse_args(["fan-out", "--max-concurrency", "2"])
            .expect_err("unknown --max-concurrency");
        assert_eq!(unknown.code, "invalid-invocation");
        assert!(unknown.message.contains("unknown option"), "{unknown}");
    }

    #[test]
    fn parser_preview_bindings_is_not_a_primary_command() {
        let parsed = parse_args(["preview-bindings"]).expect("preview-bindings should parse");
        let ParsedRequest::PreviewBindings { operand, options } = parsed else {
            panic!("expected ParsedRequest::PreviewBindings, got {parsed:?}");
        };
        assert!(operand.is_none());
        assert!(options.database.is_none());
        assert!(options.provider_timeout.is_none());

        let parsed = parse_args(["preview-bindings", "{}", "--timeout-ms", "60000"])
            .expect("preview-bindings with timeout");
        let ParsedRequest::PreviewBindings { operand, options } = parsed else {
            panic!("expected ParsedRequest::PreviewBindings");
        };
        assert_eq!(operand.as_deref(), Some("{}"));
        assert_eq!(options.provider_timeout, Some(Duration::from_millis(60000)));
    }

    #[test]
    fn parser_invocation_progress_is_not_a_primary_command() {
        let parsed = parse_args(["invocation-progress", "run-1"]).expect("progress should parse");
        let ParsedRequest::InvocationProgress {
            run_id,
            invocation_id,
            options,
        } = parsed
        else {
            panic!("expected ParsedRequest::InvocationProgress, got {parsed:?}");
        };
        assert_eq!(run_id.as_str(), "run-1");
        assert!(invocation_id.is_none());
        assert_eq!(options.output, OutputFormat::Human);

        let parsed = parse_args([
            "--json",
            "--database",
            "/tmp/loop.db",
            "--timeout-ms",
            "5000",
            "invocation-progress",
            "run-1",
            "inv-1",
        ])
        .expect("progress with globals and invocation id");
        let ParsedRequest::InvocationProgress {
            run_id,
            invocation_id,
            options,
        } = parsed
        else {
            panic!("expected ParsedRequest::InvocationProgress");
        };
        assert_eq!(run_id.as_str(), "run-1");
        assert_eq!(
            invocation_id.as_ref().map(InvocationId::as_str),
            Some("inv-1")
        );
        assert_eq!(options.output, OutputFormat::Json);
        assert_eq!(options.database, Some(PathBuf::from("/tmp/loop.db")));
        assert_eq!(options.provider_timeout, Some(Duration::from_millis(5000)));

        let missing = parse_args(["invocation-progress"]).expect_err("run id required");
        assert_eq!(missing.code, "invalid-invocation");
        let extra = parse_args(["invocation-progress", "run-1", "inv-1", "extra"])
            .expect_err("leftover tokens");
        assert_eq!(extra.code, "invalid-invocation");
        let capture = parse_args(["invocation-progress", "run-1", "--capture-dir", "/tmp"])
            .expect_err("reject capture-dir");
        assert_eq!(capture.code, "invalid-invocation");
        let worker = parse_args([
            "invocation-progress",
            "run-1",
            "--worker",
            r#"{"command":"echo","args":[]}"#,
        ])
        .expect_err("reject worker");
        assert_eq!(worker.code, "invalid-invocation");
        let stdin = parse_args(["invocation-progress", "run-1", "--stdin-file", "x"])
            .expect_err("reject stdin-exec flags");
        assert_eq!(stdin.code, "invalid-invocation");
    }

    #[test]
    fn parser_show_compact_is_human_only_and_not_a_new_operation() {
        let parsed =
            parse_args(["show", "--compact", "run-1"]).expect("show --compact should parse");
        let ParsedRequest::Operation {
            options,
            command: PrimaryCommand::Show(run_id),
        } = parsed
        else {
            panic!("expected show operation");
        };
        assert!(options.compact);
        assert_eq!(run_id.as_str(), "run-1");

        for selector in [
            "--json",
            "--machine-readable",
            "-j",
            "--format=json",
            "--output=json",
        ] {
            let json_error = parse_args([selector, "show", "--compact", "run-1"])
                .expect_err("compact show must reject JSON output");
            assert_eq!(json_error.code, "invalid-invocation");
            assert!(json_error.message.contains("human-only"));
            assert!(json_error.message.contains("--json"));
        }

        let other_error =
            parse_args(["list", "--compact"]).expect_err("compact must remain local to show");
        assert_eq!(other_error.code, "invalid-invocation");
        assert!(other_error.message.contains("only valid"));

        let execution = execute(["--json", "show", "--compact", "run-1"]);
        assert_eq!(execution.exit_code, EXIT_INVALID_INVOCATION);
        let output: Value = serde_json::from_str(&execution.stdout).expect("JSON error");
        assert_eq!(output["status"], "invalid-invocation");
        assert!(output["message"].as_str().unwrap().contains("human-only"));
    }

    #[test]
    fn append_parser_preserves_separate_record_id_value() {
        let parsed = parse_args([
            "append",
            "--record-id",
            "caller-record",
            "run-1",
            "note",
            "{}",
        ])
        .expect("append with separate record id should parse");
        let ParsedRequest::Operation {
            command: PrimaryCommand::Append(args),
            ..
        } = parsed
        else {
            panic!("expected append operation");
        };
        assert_eq!(args.record_id.as_deref(), Some("caller-record"));
    }

    #[test]
    fn append_parser_preserves_equals_record_id_value() {
        let parsed = parse_args(["append", "--record-id=caller-record", "run-1", "note", "{}"])
            .expect("append with equals record id should parse");
        let ParsedRequest::Operation {
            command: PrimaryCommand::Append(args),
            ..
        } = parsed
        else {
            panic!("expected append operation");
        };
        assert_eq!(args.record_id.as_deref(), Some("caller-record"));
    }

    #[test]
    fn append_parser_still_rejects_foreign_options() {
        let error = parse_args([
            "append",
            "--provider",
            "software-change",
            "run-1",
            "note",
            "{}",
        ])
        .expect_err("append must reject start-only provider option");
        assert_eq!(error.code, "invalid-invocation");
        assert!(error.message.contains("another operation"));
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
    fn compact_renderer_keeps_fixed_order_and_separates_helper_liveness() {
        let transition = core::Transition::checked("draft", "approve", "done");
        let evaluation = core::DurableEvaluation::allow(
            transition.clone(),
            4_u64.into(),
            core::Timestamp::from_unix_millis(40),
        );
        let invocation = core::operations::WorkSlotInvocationView {
            invocation_id: "inv-1".into(),
            slot_id: "slot-1".into(),
            binding: core::WorkSlotBinding::new("worker", vec!["--flag".to_owned()]),
            instruction_digest: "digest".to_owned(),
            subject: "subject".to_owned(),
            status: core::ProjectedInvocationStatus::Running,
            started_at: core::Timestamp::from_unix_millis(10),
            allowed_time_ms: 1_000,
            exit_code: None,
            completed_at: None,
            overlay_meaning: "overlay remains authoritative".to_owned(),
            elapsed_ms: 30,
            remaining_allowed_ms: 970,
            capture_dir: "/tmp/capture".to_owned(),
            inner_workers: Vec::new(),
            assignment_selection: None,
            invocation_input: None,
            change_report: core::operations::show::InvocationChangeReport {
                identity: "inv-1".into(),
                standing: false,
                subject_revision: "subject".to_owned(),
                dimensions: Value::Null,
                assignments: Vec::new(),
                plan_task_results: Vec::new(),
            },
        };
        let projection = CliShowProjection {
            run_id: "run-1".into(),
            label: Some("compact test".to_owned()),
            workflow_id: "workflow".into(),
            lifecycle: core::Lifecycle::Active,
            current_state: "draft".into(),
            current_state_title: "Draft".to_owned(),
            current_state_instructions: "Do work".to_owned(),
            initial_input: json!({}),
            context: Vec::new(),
            requestable_events: vec![core::operations::RequestableEvent::from_transition(
                &transition,
            )],
            latest_evaluations: vec![evaluation],
            work_slots: vec![core::WorkSlot::new("slot-1", "draft", "approve")],
            change_report: core::operations::RunChangeReport {
                assignments: Vec::new(),
                plan_task_results: Vec::new(),
            },
            work_slot_invocations: vec![invocation],
        };
        let progress =
            CompactInnerProgress::Snapshot(invocation_progress::InvocationProgressSnapshot {
                run_id: "run-1".into(),
                invocation_id: "inv-1".into(),
                slot_id: "slot-1".into(),
                capture_dir: "/tmp/capture".to_owned(),
                graph: Some(invocation_progress::GraphProgress {
                    locator: DaguLocator {
                        dagu_home: "/tmp/dagu".to_owned(),
                        dag_name: "dag".to_owned(),
                        run_name: "run".to_owned(),
                    },
                    steps: vec![
                        invocation_progress::GraphStep {
                            name: "w0".to_owned(),
                            state: invocation_progress::GraphStepState::Running,
                        },
                        invocation_progress::GraphStep {
                            name: "join".to_owned(),
                            state: invocation_progress::GraphStepState::Reaped,
                        },
                        invocation_progress::GraphStep {
                            name: "w1".to_owned(),
                            state: invocation_progress::GraphStepState::NotStarted,
                        },
                    ],
                }),
                traces: Vec::new(),
            });
        let output = render_compact_show(
            &projection,
            Some(&projection.work_slot_invocations[0]),
            progress,
        );
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "completed show --compact");
        assert!(lines[1].starts_with("run: run-1"));
        assert_eq!(lines[2], "lifecycle: active");
        assert_eq!(lines[3], "state: draft (Draft)");
        assert_eq!(lines[4], "requestable events: approve -> done (checked)");
        assert!(lines[5].contains("latest checked result: allow event=approve"));
        assert_eq!(
            lines[6],
            "invocation: inv-1 slot=slot-1 status=running exit_code=pending"
        );
        assert_eq!(
            lines[7],
            "inner progress (Dagu helper liveness): steps=3 not_started=1 running=1 reaped=1"
        );
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
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let composed_input =
            with_allocated_artifact_root(initial_input.clone(), Path::new(&database), &run_id);
        assert_eq!(start_json["result"]["run"]["initial_input"], composed_input);

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
        assert_eq!(list_json["result"][0]["provider"], "fixture");
        let listed_artifact_root = list_json["result"][0]["artifact_root"]
            .as_str()
            .unwrap_or_default();
        assert!(!listed_artifact_root.is_empty());
        assert_eq!(
            listed_artifact_root,
            allocated_run_dir(Path::new(&database), &run_id).to_string_lossy()
        );

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
        assert_eq!(show_json["result"]["initial_input"], composed_input);
        let show_result = show_json["result"]
            .as_object()
            .expect("show result must be an object");
        assert!(!show_result.contains_key("provider"));
        assert!(!show_result.contains_key("artifact_root"));

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
        let human_run_id = human_start_json["run"]["id"].as_str().unwrap().to_owned();
        let human_composed = with_allocated_artifact_root(
            initial_input.clone(),
            Path::new(&database),
            &human_run_id,
        );
        assert_eq!(human_start_json["run"]["initial_input"], human_composed);

        let append = execute([
            "--json".to_owned(),
            "append".to_owned(),
            "--record-id".to_owned(),
            "caller-record-separate".to_owned(),
            run_id.clone(),
            "note".to_owned(),
            context_data_json.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(append.exit_code, EXIT_COMPLETED);
        let append_json: Value = serde_json::from_str(&append.stdout).unwrap();
        assert_eq!(append_json["status"], "completed");
        assert_internal_metadata_absent(&append_json);
        assert_eq!(append_json["result"]["run"]["id"], run_id);
        assert_eq!(
            append_json["result"]["context"]["id"],
            "caller-record-separate"
        );
        assert_eq!(append_json["result"]["context"]["data"], context_data);

        let append_equals = execute([
            "--json".to_owned(),
            "append".to_owned(),
            "--record-id=caller-record-equals".to_owned(),
            run_id.clone(),
            "note".to_owned(),
            serde_json::to_string(&context_data).unwrap(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(append_equals.exit_code, EXIT_COMPLETED);
        let append_equals_json: Value = serde_json::from_str(&append_equals.stdout).unwrap();
        assert_eq!(append_equals_json["status"], "completed");
        assert_eq!(
            append_equals_json["result"]["context"]["id"],
            "caller-record-equals"
        );

        let caller_show = execute([
            "--json".to_owned(),
            "show".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(caller_show.exit_code, EXIT_COMPLETED);
        let caller_show_json: Value = serde_json::from_str(&caller_show.stdout).unwrap();
        assert_eq!(
            caller_show_json["result"]["context"][0]["id"],
            "caller-record-separate"
        );
        assert_eq!(
            caller_show_json["result"]["context"][1]["id"],
            "caller-record-equals"
        );

        let caller_history = execute([
            "--json".to_owned(),
            "history".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(caller_history.exit_code, EXIT_COMPLETED);
        let caller_history_json: Value = serde_json::from_str(&caller_history.stdout).unwrap();
        let history_ids = caller_history_json["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["action"]["context_record_id"].as_str())
            .collect::<Vec<_>>();
        assert!(history_ids.contains(&"caller-record-separate"));
        assert!(history_ids.contains(&"caller-record-equals"));

        let human_observation = execute([
            "show".to_owned(),
            human_run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(human_observation.exit_code, EXIT_COMPLETED);

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
        assert_eq!(show_human_json["initial_input"], human_composed);
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

        let termination_observation = execute([
            "--json".to_owned(),
            "show".to_owned(),
            run_id.clone(),
            "--database".to_owned(),
            database.clone(),
        ]);
        assert_eq!(termination_observation.exit_code, EXIT_COMPLETED);

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

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const CATALOG_ENV_VARS: &[&str] = &[
        "HOME",
        "XDG_DATA_HOME",
        "LOOP_ENGINE_HOME",
        "LOOP_HOME",
        "LOOP_ENGINE_DATABASE",
        "LOOP_ENGINE_DATABASE_PATH",
        "LOOP_DATABASE",
        "LOOP_DATABASE_PATH",
        "LOOP_DB_PATH",
        "LOOP_ENGINE_DB",
        "LOOP_DB",
    ];

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(unique_token(label));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct ProcessIsolation {
        _lock: MutexGuard<'static, ()>,
        previous_cwd: PathBuf,
        previous_vars: Vec<(String, Option<OsString>)>,
    }

    impl ProcessIsolation {
        fn acquire() -> Self {
            let lock = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous_cwd = std::env::current_dir().expect("current directory");
            let previous_vars = CATALOG_ENV_VARS
                .iter()
                .map(|name| ((*name).to_owned(), std::env::var_os(name)))
                .collect();
            Self {
                _lock: lock,
                previous_cwd,
                previous_vars,
            }
        }

        fn set_home_and_xdg(&self, home: &Path, xdg_data_home: &Path) {
            std::env::set_var("HOME", home);
            std::env::set_var("XDG_DATA_HOME", xdg_data_home);
            for name in CATALOG_ENV_VARS {
                if *name == "HOME" || *name == "XDG_DATA_HOME" {
                    continue;
                }
                std::env::remove_var(name);
            }
        }

        fn chdir(&self, path: &Path) {
            std::env::set_current_dir(path).expect("chdir");
        }
    }

    impl Drop for ProcessIsolation {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous_cwd);
            for (name, value) in &self.previous_vars {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    #[cfg(unix)]
    fn write_fixture_provider(root: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        fs::create_dir_all(root).unwrap();
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
        (config_path, root.join("loop.db"))
    }

    #[cfg(unix)]
    #[test]
    fn default_catalog_is_user_level_and_cwd_independent() {
        let root = TempRoot::new("cli-default-catalog");
        let isolation = ProcessIsolation::acquire();
        let home = root.path().join("home");
        let xdg = root.path().join("xdg");
        let cwd_a = root.path().join("cwd-a");
        let cwd_b = root.path().join("cwd-b");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        fs::create_dir_all(&cwd_a).unwrap();
        fs::create_dir_all(&cwd_b).unwrap();
        isolation.set_home_and_xdg(&home, &xdg);

        let provider_root = root.path().join("provider");
        let (config_path, _) = write_fixture_provider(&provider_root);

        isolation.chdir(&cwd_a);
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            json!({"goal": "catalog"}).to_string(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED, "{}", start.stdout);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        isolation.chdir(&cwd_b);
        let list = execute(["--json".to_owned(), "list".to_owned()]);
        assert_eq!(list.exit_code, EXIT_COMPLETED, "{}", list.stdout);
        let list_json: Value = serde_json::from_str(&list.stdout).unwrap();
        assert_eq!(list_json["result"][0]["id"], run_id);

        assert!(!cwd_a.join(".loop-engine").join("loop.db").exists());
        assert!(!cwd_b.join(".loop-engine").join("loop.db").exists());
        assert!(xdg.join("loop-engine").join("loop.db").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn isolated_database_creates_sibling_runs_dir_and_does_not_write_machine_default() {
        let root = TempRoot::new("cli-isolated-db");
        let isolation = ProcessIsolation::acquire();
        let home = root.path().join("home");
        let xdg = root.path().join("xdg");
        let catalog = root.path().join("catalog");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        isolation.set_home_and_xdg(&home, &xdg);

        let (config_path, database) = write_fixture_provider(&catalog);
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            json!({"goal": "isolated"}).to_string(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED, "{}", start.stdout);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        assert!(catalog.join("runs").join(&run_id).is_dir());
        assert!(!xdg.join("loop-engine").join("loop.db").exists());
        assert!(!home
            .join(".local")
            .join("share")
            .join("loop-engine")
            .join("loop.db")
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn start_keeps_caller_nonempty_artifact_root_and_still_creates_allocated_dir() {
        let root = TempRoot::new("cli-keep-caller");
        let (config_path, database) = write_fixture_provider(root.path());
        let caller_input = json!({
            "goal": "keep-caller",
            "artifact_root": "relative/caller-path"
        });
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            caller_input.to_string(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED, "{}", start.stdout);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(start_json["result"]["run"]["initial_input"], caller_input);
        assert!(allocated_run_dir(&database, &run_id).is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn start_non_object_input_unchanged_but_list_has_provider_and_artifact_root() {
        let root = TempRoot::new("cli-non-object");
        let (config_path, database) = write_fixture_provider(root.path());
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            "\"plain-string\"".to_owned(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED, "{}", start.stdout);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(start_json["result"]["run"]["initial_input"], "plain-string");

        let list = execute([
            "--json".to_owned(),
            "list".to_owned(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
        ]);
        assert_eq!(list.exit_code, EXIT_COMPLETED, "{}", list.stdout);
        let list_json: Value = serde_json::from_str(&list.stdout).unwrap();
        assert_eq!(list_json["result"][0]["provider"], "fixture");
        let artifact_root = list_json["result"][0]["artifact_root"]
            .as_str()
            .unwrap_or_default();
        assert!(!artifact_root.is_empty());
        assert_eq!(
            artifact_root,
            allocated_run_dir(&database, &run_id).to_string_lossy()
        );
    }

    #[test]
    fn summary_worker_linkage_requires_distinct_ids_and_complete_selected_facts() {
        let valid = serde_json::json!({
            "workers": [
                {
                    "assignment_id": "axis-a",
                    "command": "worker",
                    "args": ["--shared"],
                    "exit_code": 0,
                    "selected_attempt": 2,
                    "selected_output_sha256": "sha256:selected",
                    "selected_output_path": "/capture/axis-a/attempts/2/stdout"
                },
                {
                    "assignment_id": "axis-b",
                    "command": "worker",
                    "args": ["--shared"],
                    "exit_code": 0,
                    "selected_attempt": null,
                    "selected_output_sha256": null,
                    "selected_output_path": null
                }
            ]
        });
        let workers = parse_summary_inner_workers(&serde_json::to_vec(&valid).unwrap())
            .expect("valid selected and coverage-gap workers");
        assert_eq!(workers[0].assignment_id, "axis-a");
        assert_eq!(workers[0].selected_attempt, Some(2));
        assert_eq!(
            workers[0].selected_output_sha256.as_deref(),
            Some("sha256:selected")
        );
        assert_eq!(workers[1].assignment_id, "axis-b");
        assert_eq!(workers[1].selected_attempt, None);
        assert!(workers[1].selected_output_sha256.is_none());

        let mut duplicate = valid.clone();
        duplicate["workers"][1]["assignment_id"] = json!("axis-a");
        assert!(parse_summary_inner_workers(&serde_json::to_vec(&duplicate).unwrap()).is_none());

        let mut half_linked = valid.clone();
        half_linked["workers"][0]["selected_output_path"] = Value::Null;
        assert!(parse_summary_inner_workers(&serde_json::to_vec(&half_linked).unwrap()).is_none());

        let mut worker_level_path = valid;
        worker_level_path["workers"][0]["selected_output_path"] = json!("/capture/axis-a/stdout");
        assert!(
            parse_summary_inner_workers(&serde_json::to_vec(&worker_level_path).unwrap()).is_none(),
            "selected attempt must not be linked to the worker-level stdout copy"
        );
    }

    #[cfg(unix)]
    #[test]
    fn show_json_has_no_top_level_provider_or_artifact_root() {
        let root = TempRoot::new("cli-show-keys");
        let (config_path, database) = write_fixture_provider(root.path());
        let start = execute([
            "--json".to_owned(),
            "start".to_owned(),
            "fixture".to_owned(),
            json!({"goal": "show-keys"}).to_string(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(start.exit_code, EXIT_COMPLETED, "{}", start.stdout);
        let start_json: Value = serde_json::from_str(&start.stdout).unwrap();
        let run_id = start_json["result"]["run"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let show = execute([
            "--json".to_owned(),
            "show".to_owned(),
            run_id,
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
        ]);
        assert_eq!(show.exit_code, EXIT_COMPLETED, "{}", show.stdout);
        let show_json: Value = serde_json::from_str(&show.stdout).unwrap();
        let result = show_json["result"]
            .as_object()
            .expect("show result must be an object");
        assert!(!result.contains_key("provider"));
        assert!(!result.contains_key("artifact_root"));
    }
}
