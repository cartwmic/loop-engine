//! Composition root and command-line driver for Loop Engine.
//!
//! The CLI deliberately stays thin: it parses caller input, constructs the
//! concrete integrations, invokes exactly one core operation, and renders the
//! core operation outcome.  Workflow and provider policy remain in the core
//! and integration crates respectively.

pub mod monitor;
mod targeted_read;
mod terminal_explorer;
mod visibility;

use loop_core::{
    self as core, AppendContextRequest, EventRequest, HistoryRequest, OperationOutcome,
    Persistence, ProviderResolutionError, RunId, StartRequest, TerminateRunRequest, Timestamp,
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
    /// Alias for the non-arming status view.
    pub compact: bool,
    pub show_view: String,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            output: OutputFormat::Human,
            database: None,
            provider_config: None,
            provider_timeout: None,
            compact: false,
            show_view: "action".to_owned(),
        }
    }
}

/// The primary CLI operations.
#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "preserve the public parsed operation payload shape"
)]
pub enum PrimaryCommand {
    Start(StartArgs),
    List,
    Show(RunId),
    Append(AppendArgs),
    Event {
        run_id: RunId,
        event: String,
        override_attestation: Option<core::StateVisitAttestation>,
        advice_exception: Option<core::AdviceExceptionAttestation>,
        driver_act: Option<core::DriverActRequest>,
    },
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
#[expect(
    clippy::large_enum_variant,
    reason = "boxing the operation payload would change the public parsed-request variant shape"
)]
pub enum ParsedRequest {
    Help {
        command: Option<String>,
    },
    Version,
    Operation {
        options: CliOptions,
        command: PrimaryCommand,
    },
    TerminalExplore {
        options: CliOptions,
        run_id: RunId,
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
    let mut compact = false;
    let mut show_view_seen = false;
    let mut event_override = None;
    let mut event_advice_exception = None;
    let mut event_driver_act = None;
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
                "--view" => {
                    show_view_seen = true;
                    options.show_view = next_option_value(args, &mut index, token)?;
                    if !matches!(options.show_view.as_str(), "action" | "status" | "full") {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "show --view must be action, status, or full",
                        ));
                    }
                    continue;
                }
                value if value.starts_with("--view=") => {
                    show_view_seen = true;
                    options.show_view = value.trim_start_matches("--view=").to_owned();
                    if !matches!(options.show_view.as_str(), "action" | "status" | "full") {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "show --view must be action, status, or full",
                        ));
                    }
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
                value
                    if value == "--advice-exception"
                        || value.starts_with("--advice-exception=") =>
                {
                    if event_advice_exception.is_some() {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "--advice-exception may be supplied once",
                        ));
                    }
                    let raw = if let Some(raw) = value.strip_prefix("--advice-exception=") {
                        index += 1;
                        raw.to_owned()
                    } else {
                        next_option_value(args, &mut index, token)?
                    };
                    event_advice_exception = Some(
                        serde_json::from_value::<core::AdviceExceptionAttestation>(
                            parse_json_source(&raw, "event advice exception")?,
                        )
                        .map_err(|error| {
                            CliError::new("invalid-advice-exception", error.to_string())
                        })?,
                    );
                    continue;
                }
                value if value == "--driver-act" || value.starts_with("--driver-act=") => {
                    if event_driver_act.is_some() {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "--driver-act may be supplied once",
                        ));
                    }
                    let raw = if let Some(raw) = value.strip_prefix("--driver-act=") {
                        index += 1;
                        raw.to_owned()
                    } else {
                        next_option_value(args, &mut index, token)?
                    };
                    event_driver_act = Some(
                        serde_json::from_value::<core::DriverActRequest>(parse_json_source(
                            &raw,
                            "event driver act",
                        )?)
                        .map_err(|error| CliError::new("invalid-driver-act", error.to_string()))?,
                    );
                    continue;
                }
                value if value == "--override" || value.starts_with("--override=") => {
                    if event_override.is_some() {
                        return Err(CliError::new(
                            "invalid-invocation",
                            "--override may be supplied once",
                        ));
                    }
                    let raw = if let Some(raw) = value.strip_prefix("--override=") {
                        index += 1;
                        raw.to_owned()
                    } else {
                        next_option_value(args, &mut index, token)?
                    };
                    event_override = Some(
                        serde_json::from_value::<core::StateVisitAttestation>(parse_json_source(
                            &raw,
                            "event override",
                        )?)
                        .map_err(|error| CliError::new("invalid-override", error.to_string()))?,
                    );
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

    if let Some(name) = command_name.as_deref() {
        if !matches!(
            name,
            "start" | "list" | "show" | "append" | "event" | "history" | "terminate" | "explore"
        ) {
            return Err(CliError::new(
                "invalid-operation",
                format!("unknown operation `{name}`"),
            ));
        }
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
            "a primary operation is required (start, list, show, append, event, history, terminate)",
        )
    })?;

    if compact && command_name != "show" {
        return Err(CliError::new(
            "invalid-invocation",
            "`--compact` is only valid for the existing show operation",
        ));
    }
    if show_view_seen && command_name != "show" {
        return Err(CliError::new(
            "invalid-invocation",
            "--view is only valid for show",
        ));
    }
    if compact && options.show_view != "action" && options.show_view != "status" {
        return Err(CliError::new(
            "invalid-invocation",
            "--compact conflicts with --view full",
        ));
    }
    if compact {
        options.show_view = "status".to_owned();
    }
    options.compact = compact;

    if command_name != "event" && event_override.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "--override requires event",
        ));
    }
    if command_name != "event" && event_advice_exception.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "--advice-exception requires event",
        ));
    }
    if command_name != "event" && event_driver_act.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "--driver-act requires event",
        ));
    }
    if event_driver_act.is_some() && (event_override.is_some() || event_advice_exception.is_some())
    {
        return Err(CliError::new(
            "invalid-invocation",
            "--driver-act cannot be combined with --override or --advice-exception",
        ));
    }
    if event_override.is_some() && event_advice_exception.is_some() {
        return Err(CliError::new(
            "invalid-invocation",
            "--override and --advice-exception cannot be combined",
        ));
    }
    if command_name == "explore" {
        reject_unrelated_options(
            "explore", provider, input, label, start_id, kind, data, record_id, event,
        )?;
        if options.output != OutputFormat::Human {
            return Err(CliError::new(
                "invalid-invocation",
                "explore is an interactive terminal command; omit --json",
            ));
        }
        if options.provider_config.is_some() || options.provider_timeout.is_some() {
            return Err(CliError::new(
                "invalid-invocation",
                "explore is provider-free; --config and --timeout-ms do not apply",
            ));
        }
        let run_id = run_id.or_else(|| take_positional(&mut positionals));
        let run_id = required(run_id, "run ID")?.into();
        ensure_no_positionals(&positionals, "explore")?;
        return Ok(ParsedRequest::TerminalExplore { options, run_id });
    }

    let mut command = parse_primary_command(
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

    if let PrimaryCommand::Event {
        override_attestation,
        advice_exception,
        driver_act,
        ..
    } = &mut command
    {
        *override_attestation = event_override;
        *advice_exception = event_advice_exception;
        *driver_act = event_driver_act;
    }
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
                override_attestation: None,
                advice_exception: None,
                driver_act: None,
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
                "unknown operation `{name}`; expected start, list, show, append, event, history, terminate"
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

pub fn execute<I, S>(args: I) -> Execution
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let operation_started_at = std::time::Instant::now();
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    if let Some(execution) = targeted_read::dispatch(&args) {
        return execution;
    }
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
        ParsedRequest::Operation { options, command } => {
            execute_operation(options, command, operation_started_at)
        }
        ParsedRequest::TerminalExplore { options, run_id } => {
            execute_terminal_explorer(options, run_id)
        }
    }
}

fn execute_operation(
    options: CliOptions,
    command: PrimaryCommand,
    started_at: std::time::Instant,
) -> Execution {
    let output = options.output;
    let show_view = options.show_view.clone();
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
    if let PrimaryCommand::Show(run_id) = &command {
        if show_view != "full" {
            return render_bounded_show(run_id, &paths.database, &show_view, output, started_at);
        }
    }
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
            let now = now_timestamp();
            let outcome = core::operations::show::execute_passive(
                core::ShowRequest::new(run_id),
                &persistence,
                now,
                show_view != "status",
            )
            .map(CliShowProjection::from);
            if show_view == "status" && output == OutputFormat::Human {
                render_operation(operation, output, &outcome)
            } else {
                let outcome = outcome
                    .map(|projection| show_packet(projection, &show_view, now, &paths.database));
                render_operation(operation, output, &outcome)
            }
        }
        PrimaryCommand::Append(args) => {
            let request = AppendContextRequest::new(
                args.run_id,
                args.record_id.unwrap_or_else(new_context_id),
                args.kind,
                args.data,
                now_timestamp(),
            );
            let outcome =
                core::execute_append(request, &persistence).map(CliAppendContextResult::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::Event {
            run_id,
            event,
            override_attestation,
            advice_exception,
            driver_act,
        } => {
            let mut request = EventRequest::new(run_id, event);
            request.override_attestation = override_attestation;
            request.advice_exception = advice_exception;
            request.driver_act = driver_act;
            let outcome = core::execute_event(request, &gateway, &persistence)
                .map(CliCommitTransitionResult::from);
            render_operation(operation, output, &outcome)
        }
        PrimaryCommand::History(run_id) => {
            let summary = match persistence.load_authoritative_run(&run_id) {
                Ok(run) => run.override_summary,
                Err(error) => {
                    return render_operation_error(
                        operation,
                        output,
                        CliError::new(error.code(), error.to_string()),
                    )
                }
            };
            let outcome =
                core::execute_history(HistoryRequest::new(run_id), &persistence).map(|history| {
                    history
                        .into_iter()
                        .map(CliHistoryEntry::from)
                        .collect::<Vec<_>>()
                });
            let mut rendered = render_operation(operation, output, &outcome);
            if outcome.is_completed() {
                // Preserve the historical result array; run-wide metadata lives
                // on the history envelope rather than duplicating every row.
                if output == OutputFormat::Json {
                    let mut envelope: Value =
                        serde_json::from_str(&rendered.stdout).expect("rendered envelope");
                    let fields = serde_json::to_value(&summary).expect("override summary");
                    envelope
                        .as_object_mut()
                        .expect("envelope object")
                        .extend(fields.as_object().expect("summary object").clone());
                    rendered.stdout = format!("{envelope}\n");
                } else {
                    rendered.stdout.push_str(&format!(
                        "override summary: {}\n",
                        serde_json::to_string(&summary).expect("override summary")
                    ));
                }
            }
            rendered
        }
        PrimaryCommand::Terminate(run_id) => {
            let outcome = core::execute_terminate(TerminateRunRequest::new(run_id), &persistence)
                .map(CliTerminateResult::from);
            render_operation(operation, output, &outcome)
        }
    }
}

fn execute_terminal_explorer(options: CliOptions, run_id: RunId) -> Execution {
    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return render_operation_error("explore", OutputFormat::Human, error),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let persistence =
        match SqlitePersistence::open_for_observation(&paths.database, deadline, false) {
            Ok(persistence) => persistence,
            Err(error) => {
                return render_operation_error(
                    "explore",
                    OutputFormat::Human,
                    CliError::new(error.code(), error.to_string()),
                )
            }
        };
    let projection = match core::operations::show::execute_passive(
        core::ShowRequest::new(run_id.clone()),
        &persistence,
        now_timestamp(),
        false,
    ) {
        OperationOutcome::Completed(projection) => projection,
        OperationOutcome::Rejected(issue) | OperationOutcome::Error(issue) => {
            return render_operation_error(
                "explore",
                OutputFormat::Human,
                CliError::new(issue.code, issue.message),
            )
        }
    };
    let history = match core::execute_history(HistoryRequest::new(run_id), &persistence) {
        OperationOutcome::Completed(history) => history,
        OperationOutcome::Rejected(issue) | OperationOutcome::Error(issue) => {
            return render_operation_error(
                "explore",
                OutputFormat::Human,
                CliError::new(issue.code, issue.message),
            )
        }
    };
    terminal_explorer::run(&projection, &history)
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

/// Caller-visible run data.  Core's `Run` also carries persistence control
/// metadata; those fields are deliberately not represented by this CLI DTO.
#[derive(Clone, Debug, Serialize)]
struct CliRun {
    #[serde(flatten)]
    override_summary: core::OverrideSummary,
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
            override_summary: run.override_summary,
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
    #[serde(flatten)]
    override_summary: core::OverrideSummary,
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
            override_summary: summary.override_summary,
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
    #[serde(flatten)]
    override_summary: core::OverrideSummary,
    run_id: core::RunId,
    label: Option<String>,
    workflow_id: core::WorkflowId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_graph: Option<core::Workflow>,
    lifecycle: core::Lifecycle,
    current_state: core::StateId,
    current_state_title: String,
    current_state_instructions: String,
    initial_input: Value,
    state_visit: u64,
    binding_amendments: Vec<core::BindingAmendment>,
    effective_bindings: std::collections::BTreeMap<String, core::WorkSlotBinding>,
    context: Vec<core::ContextRecord>,
    requestable_events: Vec<core::RequestableEvent>,
    latest_evaluations: Vec<core::DurableEvaluation>,
    evaluation_history: Vec<core::DurableEvaluation>,
    action_guidance: Option<Value>,
    work_slots: Vec<core::WorkSlot>,
    change_report: core::operations::RunChangeReport,
    work_slot_invocations: Vec<core::operations::WorkSlotInvocationView>,
}

impl From<core::ShowProjection> for CliShowProjection {
    fn from(projection: core::ShowProjection) -> Self {
        Self {
            override_summary: projection.override_summary,
            run_id: projection.run_id,
            label: projection.label,
            workflow_id: projection.workflow_id,
            workflow_graph: projection.workflow_graph,
            lifecycle: projection.lifecycle,
            current_state: projection.current_state,
            current_state_title: projection.current_state_title,
            current_state_instructions: projection.current_state_instructions,
            initial_input: projection.initial_input,
            state_visit: projection.state_visit,
            binding_amendments: projection.binding_amendments,
            effective_bindings: projection.effective_bindings,
            context: projection.context,
            requestable_events: projection.requestable_events,
            latest_evaluations: projection.latest_evaluations,
            evaluation_history: projection.evaluation_history,
            action_guidance: projection.action_guidance,
            work_slots: projection.work_slots,
            change_report: projection.change_report,
            work_slot_invocations: projection.work_slot_invocations,
        }
    }
}

fn render_bounded_show(
    run_id: &RunId,
    database: &Path,
    view: &str,
    output: OutputFormat,
    started_at: std::time::Instant,
) -> Execution {
    const READ_BUDGET: Duration = Duration::from_millis(1100);
    let deadline = started_at + READ_BUDGET;
    let arm = view == "action";
    let snapshot = match SqlitePersistence::open_for_observation(database, deadline, arm)
        .and_then(|persistence| persistence.read_bounded_observation(run_id, deadline, arm))
    {
        Ok(snapshot) => snapshot,
        Err(error) if !arm => {
            let packet = unavailable_observation(run_id, view, database, error.to_string());
            return render_bounded_unavailable(packet, view, output);
        }
        Err(error) => {
            return render_operation_error(
                "show",
                output,
                CliError::new(error.code(), error.to_string()),
            )
        }
    };
    let packet = bounded_show_packet(snapshot, view, database, deadline);
    if output == OutputFormat::Json {
        return Execution {
            exit_code: EXIT_COMPLETED,
            stdout: format!(
                "{}\n",
                json!({"operation":"show","status":"completed","result":packet})
            ),
            stderr: String::new(),
        };
    }
    Execution {
        exit_code: EXIT_COMPLETED,
        stdout: render_bounded_show_human(&packet, view),
        stderr: String::new(),
    }
}

fn render_bounded_unavailable(packet: Value, view: &str, output: OutputFormat) -> Execution {
    if output == OutputFormat::Json {
        return Execution {
            exit_code: EXIT_COMPLETED,
            stdout: format!(
                "{}\n",
                json!({"operation":"show","status":"completed","result":packet})
            ),
            stderr: String::new(),
        };
    }
    Execution {
        exit_code: EXIT_COMPLETED,
        stdout: format!(
            "completed show --view {view}\nrun: {}\nstatus: unavailable; {}\n",
            packet["run_id"].as_str().unwrap_or("unknown"),
            packet["uncertainty"]["reason"]
                .as_str()
                .unwrap_or("sample unavailable")
        ),
        stderr: String::new(),
    }
}

fn unavailable_observation(run_id: &RunId, view: &str, database: &Path, reason: String) -> Value {
    json!({
        "schema_version":1,"view":view,"run_id":run_id,"mutation_armed":false,
        "sampled_at_ms":now_timestamp().as_unix_millis(),
        "completeness":{"run":"unavailable","invocations":"unavailable","assignments":"unavailable"},
        "uncertainty":{"state":"unavailable","reason":reason},
        "locators":bounded_locators(run_id,database)
    })
}

fn bounded_locators(run_id: &RunId, database: &Path) -> Value {
    json!({
        "full":["loop-engine","--database",database,"--json","show",run_id,"--view","full"],
        "history":["loop-engine","--database",database,"read",run_id,"--kind","history"],
        "assignment_read":["loop-engine","--database",database,"read",run_id,"--kind","assignment","--assignment","ASSIGNMENT_ID"],
        "output_read":["loop-engine","--database",database,"read",run_id,"--kind","stdout|stderr","--invocation","INVOCATION_ID","--assignment","ASSIGNMENT_ID","--offset","0","--limit","65536"]
    })
}

fn bounded_show_packet(
    snapshot: Value,
    view: &str,
    database: &Path,
    _deadline: std::time::Instant,
) -> Value {
    let run_id = snapshot["run"]["run_id"]
        .as_str()
        .unwrap_or("unknown")
        .to_owned();
    let run_id = RunId::from(run_id);
    let sampled_at = snapshot["sampled_at_ms"].clone();
    let armed = snapshot["mutation_armed"].as_bool().unwrap_or(false);
    let current_state = snapshot["run"]["current_state"].clone();
    let events = snapshot["run"]["requestable_events"].clone();
    let state = snapshot["run"]["state"].clone();
    let slots = snapshot["run"]["work_slots"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let assignments = snapshot["assignments"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let invocations = snapshot["invocations"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let current_slot = slots.iter().find(|slot| slot["state"] == current_state);
    let current_slot_id = current_slot.and_then(|slot| slot["id"].as_str());
    let instructions = state["instructions"]
        .as_str()
        .unwrap_or("Current instructions are unavailable.")
        .to_owned();
    let next_action = if matches!(
        snapshot["run"]["lifecycle"].as_str(),
        Some("final" | "terminated")
    ) {
        json!({"kind":"inspect","reason":"the run is terminal and read-only"})
    } else {
        json!({"kind":"perform-current-work","slot_id":current_slot_id,"reason":"follow current instructions externally, then append required evidence and request a listed event"})
    };
    let execution_lane = if invocations.is_empty() {
        json!({"state":"unknown","reason":"no invocation was sampled"})
    } else {
        json!({"state":"available","invocations":invocations,"meaning":"outer execution and process liveness only"})
    };
    let worker_lane = if assignments.is_empty() {
        json!({"state":"unknown","reason":"no current-subject assignment facts were sampled"})
    } else {
        json!({"state":"available","assignments":assignments,"meaning":"captured worker process facts; no semantic interpretation"})
    };
    let conformance_lane = if assignments.is_empty() {
        json!({"state":"unknown","reason":"no output-conformance facts were sampled"})
    } else {
        json!({"state":"available","assignments":assignments.iter().map(|row| json!({
            "slot_id":row["slot_id"],"assignment_id":row["assignment_id"],
            "state":row["conformance"]["state"],"attempt_count":row["conformance"]["attempt_count"],
            "repeated_failure_count":row["conformance"]["repeated_failure_count"],"error":row["conformance"]["error"],
            "error_truncated":row["conformance"]["error_truncated"]
        })).collect::<Vec<_>>(),"meaning":"mechanical output checks only; not acceptance"})
    };
    let workflow_lane = json!({
        "state":snapshot["run"]["lifecycle"],"current_state":current_state,
        "title":state["title"],"requestable_events":events,"mutation_armed":armed
    });
    let mut packet = json!({
        "schema_version":1,"view":view,"sampled_at_ms":sampled_at,
        "mutation_armed":armed,"run_id":run_id,"label":snapshot["run"]["label"],
        "workflow_id":snapshot["run"]["workflow_id"],"lifecycle":snapshot["run"]["lifecycle"],
        "current_state":current_state,"current_state_title":state["title"],
        "state_visit":snapshot["run"]["state_visit"],"current_state_instructions":instructions,
        "action_guidance":state["action_guidance"],"requestable_events":events,"work_slots":slots,
        "override_summary":snapshot["run"]["override_summary"],
        "invocations":snapshot["invocations"],"assignments":snapshot["assignments"],
        "delta_sequence":snapshot["delta_sequence"],"completeness":snapshot["completeness"],
        "workflow":workflow_lane,"execution":execution_lane,"worker":worker_lane,
        "conformance":conformance_lane,
        "evidence":{"state":"partial","locations":[bounded_locators(&run_id,database),assignments.iter().filter_map(|row|row["conformance"]["attempts_locator"].as_str()).collect::<Vec<_>>()],"meaning":"bounded locations preserve access to full history and original captures"},
        "freshness":{"state":"observed","sampled_at_ms":sampled_at,"source":"bounded local observation","meaning":"sampled facts may change after this read"},
        "owner_update_guidance":{"channel":"active Pi conversation","required_fields":["observed_change","needed_action_or_decision"],"machine_notification_is_not_owner_update":true,"authority":"assistant-owned guidance; it does not approve or control work"},
        "acceptance":{"state":"unknown","reason":"execution and conformance are not provider acceptance"},
        "uncertainty":{"state":if snapshot["completeness"].as_object().is_some_and(|o| o.values().any(|v| v == "unavailable" || v == "partial")) {"partial"} else {"sampled"},"sources":[snapshot["completeness"]]},
        "next_action":next_action,"locators":bounded_locators(&run_id,database)
    });
    if view == "status" {
        packet
            .as_object_mut()
            .unwrap()
            .remove("current_state_instructions");
        packet.as_object_mut().unwrap().remove("action_guidance");
        packet.as_object_mut().unwrap().remove("work_slots");
    }
    packet
}

fn render_bounded_show_human(packet: &Value, view: &str) -> String {
    let mut lines = vec![format!("completed show --view {view}")];
    lines.push(format!(
        "run: {}",
        packet["run_id"].as_str().unwrap_or("unknown")
    ));
    lines.push(format!(
        "state: {} ({})",
        packet["current_state"].as_str().unwrap_or("unknown"),
        packet["current_state_title"].as_str().unwrap_or("unknown")
    ));
    lines.push(format!(
        "lifecycle: {}; mutation_armed: {}; override summary: {}",
        packet["lifecycle"].as_str().unwrap_or("unknown"),
        packet["mutation_armed"].as_bool().unwrap_or(false),
        packet["override_summary"]
    ));
    if view == "action" {
        lines.push(format!(
            "instructions: {}",
            packet["current_state_instructions"]
                .as_str()
                .unwrap_or("unavailable")
        ));
        lines.push(format!(
            "next action: {}",
            packet["next_action"]["reason"]
                .as_str()
                .unwrap_or("inspect uncertainty")
        ));
    }
    let events = packet["requestable_events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let event_text = events
        .iter()
        .map(|event| {
            format!(
                "{} -> {}",
                event["event"].as_str().unwrap_or("?"),
                event["target"].as_str().unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!(
        "requestable events: {}",
        if event_text.is_empty() {
            "none".to_owned()
        } else {
            event_text
        }
    ));
    let invocations = packet["invocations"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if invocations.is_empty() {
        lines.push("invocations: unavailable or none sampled".to_owned());
    }
    for row in invocations {
        lines.push(format!(
            "invocation: {} slot={} execution={}",
            row["invocation_id"].as_str().unwrap_or("?"),
            row["slot_id"].as_str().unwrap_or("?"),
            row["execution"]["state"].as_str().unwrap_or("unknown")
        ));
    }
    let assignments = packet["assignments"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if assignments.is_empty() {
        lines.push("assignments: unknown (no current-subject facts sampled)".to_owned());
    }
    for row in assignments {
        let identity = row["assignment_id"].as_str().unwrap_or("unknown");
        let title = row["title"].as_str().unwrap_or(identity);
        let role = row["role"].as_str().unwrap_or("unknown");
        let attempts = row["conformance"]["attempt_count"]
            .as_u64()
            .map_or_else(|| "unknown".to_owned(), |count| count.to_string());
        let failures = row["conformance"]["repeated_failure_count"]
            .as_u64()
            .map_or_else(|| "unknown".to_owned(), |count| count.to_string());
        lines.push(format!("assignment: {title} [{role}] id={identity} state={} execution={} conformance={} attempts={} failures={} acceptance={}",
            row["state"].as_str().unwrap_or("unknown"),
            row["execution"]["state"].as_str().unwrap_or("unknown"),
            row["conformance"]["state"].as_str().unwrap_or("unknown"),
            attempts,
            failures,
            row["acceptance"]["state"].as_str().unwrap_or("unknown")));
        if let Some(error) = row["conformance"]["error"].as_str() {
            lines.push(format!("  error: {error}"));
        }
    }
    lines.push(format!("completeness: {}", packet["completeness"]));
    lines.join("\n") + "\n"
}

/// Bound execution instructions and provider obligations remain opaque; this
/// projection selects durable current facts, not provider-specific policy.
fn show_packet(
    projection: CliShowProjection,
    view: &str,
    now: Timestamp,
    database: &Path,
) -> Value {
    let mut value = serde_json::to_value(&projection).expect("show projection serializes");
    let lanes = visibility::show_lanes(&value, view, database);
    let object = value.as_object_mut().expect("show projection is an object");
    object.insert("view".into(), json!(view));
    object.insert("observed_at".into(), json!(now));
    object.insert("mutation_armed".into(), json!(view != "status"));
    object.insert(
        "locators".into(),
        json!({
            "full": ["loop-engine", "--database", database, "--json", "show", projection.run_id, "--view", "full"],
            "history": ["loop-engine", "--database", database, "--json", "history", projection.run_id],
            "artifact_root": projection.initial_input.get("artifact_root"),
            "context": "full.context", "invocations": "full.work_slot_invocations",
            "evaluations": "full.evaluation_history"
        }),
    );
    if view == "full" {
        insert_visibility_lanes(object, lanes.clone());
        return value;
    }
    for key in [
        "initial_input",
        "context",
        "binding_amendments",
        "effective_bindings",
        "evaluation_history",
        "change_report",
        "workflow_graph",
    ] {
        object.remove(key);
    }
    let active: Vec<_> = projection
        .work_slot_invocations
        .iter()
        .filter(|row| {
            matches!(
                row.status,
                core::ProjectedInvocationStatus::Running | core::ProjectedInvocationStatus::Overrun
            )
        })
        .collect();
    object.insert(
        "work_slot_invocations".into(),
        json!(active
            .iter()
            .map(|row| json!({
                "invocation_id": row.invocation_id, "slot_id": row.slot_id,
                "status": row.status, "subject": row.subject, "started_at": row.started_at,
                "allowed_time_ms": row.allowed_time_ms, "elapsed_ms": row.elapsed_ms,
                "remaining_allowed_ms": row.remaining_allowed_ms,
                "overlay_meaning": row.overlay_meaning, "capture_dir": row.capture_dir,
                "ownership": row.ownership, "assignment_selection": row.assignment_selection,
                "inner_workers": row.inner_workers.iter().map(|worker| json!({
                    "assignment_id": worker.assignment_id, "exit_code": worker.exit_code,
                    "selected_attempt": worker.selected_attempt,
                    "selected_output_path": worker.selected_output_path,
                    "selected_output_sha256": worker.selected_output_sha256
                })).collect::<Vec<_>>(),
                "locator": "full.work_slot_invocations"
            }))
            .collect::<Vec<_>>()),
    );
    let current_checks: Vec<_> = projection
        .latest_evaluations
        .iter()
        .filter(|row| row.transition.source == projection.current_state)
        .collect();
    object.insert("latest_evaluations".into(), json!(current_checks));
    object.insert("blocker_assessment".into(), json!({
        "freshness": "unknown",
        "reason": "Persisted checks are historical judgments, not a fresh evaluation of current work. No recent check does not establish no blockers.",
        "sources": current_checks.iter().map(|row| json!({"sequence": row.sequence, "occurred_at": row.occurred_at, "transition": row.transition, "locator": "full.evaluation_history"})).collect::<Vec<_>>()
    }));
    if view == "status" {
        object.remove("current_state_instructions");
        object.remove("action_guidance");
        object.remove("work_slots");
    } else {
        let slots: Vec<_> = projection
            .work_slots
            .iter()
            .filter(|slot| slot.state == projection.current_state)
            .collect();
        object.insert("work_slots".into(), json!(slots));
        let latest: Vec<_> = slots.iter().filter_map(|slot| {
            projection.work_slot_invocations.iter()
                .filter(|row| row.slot_id == slot.id)
                .max_by_key(|row| row.started_at)
                .map(|row| json!({"slot_id": slot.id, "invocation_id": row.invocation_id,
                    "status": row.status, "capture_dir": row.capture_dir,
                    "subject": row.subject, "standing": row.change_report.standing,
                    "freshness": "inspect change report and applicability; execution is not approval",
                    "locator": "full.work_slot_invocations"}))
        }).collect();
        object.insert("next_action".into(), json!(if projection.lifecycle.is_terminal() { "Read-only terminal run; no work or progression is legal." } else { "Perform current instructions externally, append required evidence, then request a shown event." }));
        object.insert("latest_current_slot_execution".into(), json!(latest));
        object.insert("guidance_status".into(), json!(if projection.action_guidance.is_some() { "persisted-provider-guidance" } else { "legacy-normalized-obligations-unknown; follow persisted current_state_instructions and frozen work_slots; inspect full for input/context" }));
    }
    insert_visibility_lanes(object, lanes);
    value
}

fn insert_visibility_lanes(object: &mut serde_json::Map<String, Value>, lanes: Value) {
    object.insert("visibility".into(), lanes.clone());
    if let Some(lanes) = lanes.as_object() {
        for key in [
            "workflow",
            "execution",
            "worker",
            "conformance",
            "acceptance",
            "evidence",
            "freshness",
            "uncertainty",
            "owner_update_guidance",
        ] {
            if let Some(value) = lanes.get(key) {
                object.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(next_action) = lanes.get("next_action") {
            object.insert("next_action_lane".to_owned(), next_action.clone());
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

#[derive(Serialize)]
struct JsonCompleted<'a, T> {
    operation: &'a str,
    status: &'static str,
    result: &'a T,
}

#[derive(Serialize)]
struct JsonIssue<'a> {
    operation: &'a str,
    status: &'static str,
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

/// Render a core operation outcome as one compact, valid JSON document.
pub fn render_operation_json<T: Serialize>(
    operation: &str,
    outcome: &OperationOutcome<T>,
) -> Result<String, CliError> {
    // Serialize the caller-facing DTO directly.  The former map path first
    // converted a completed DTO to Value and then serialized that Value again,
    // which was measurable in the repeated full-show journey.
    match outcome {
        OperationOutcome::Completed(value) => serde_json::to_string(&JsonCompleted {
            operation,
            status: "completed",
            result: value,
        }),
        OperationOutcome::Rejected(issue) => serde_json::to_string(&JsonIssue {
            operation,
            status: "rejected",
            code: &issue.code,
            message: &issue.message,
            details: issue.details.as_ref(),
        }),
        OperationOutcome::Error(issue) => serde_json::to_string(&JsonIssue {
            operation,
            status: "error",
            code: &issue.code,
            message: &issue.message,
            details: issue.details.as_ref(),
        }),
    }
    .map_err(|error| {
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
                + "Stable worker output data uses {\"origin\":{\"kind\":\"selected-assignment-output\",\"id\":\"INVOCATION_ID\",\"assignment_id\":\"ASSIGNMENT_ID\"}};\n"
                + "the engine resolves the selected attempt and capture metadata. Evidence reuse uses\n"
                + "kind evidence-applicability with {origin,target,attesting_driver,reason}.\n"
        }
        Some("event") => "Usage: loop-engine [options] event <run-id> <event> [--driver-act JSON | --advice-exception JSON | --override JSON]\n\nA future opted-in driver act records author, reason, changed artifacts, and unchanged intent/design/plan revisions; normal checked provider gates still run. Advice exception only excuses exactly named unanswered due advice occasions. --override remains a separate broader exception.\n".to_owned(),
        Some("show") => {
            "Usage: loop-engine [options] show [--compact] <run-id> [--view action|status|full]\n\n".to_owned()
                + "Default action reveals current instructions and obligations. Full includes all checked evaluations.\n"
                + "Action/full arm mutation; --compact aliases the non-arming status view, including JSON.\n"
                + "All views are provider-free. Full-payload consumers must select --view full.\n"
        }
        Some("history") => "Usage: loop-engine [options] history <run-id>\n".to_owned(),
        Some("explore") => "Usage: loop-engine [--database DB] explore RUN_ID\n\nRead-only interactive workflow navigator. Requires a TTY; arrows or j/k move the one-column list, [ and ] scroll detail, g/G jump, and q exits.\n".to_owned(),
        Some("terminate") => "Usage: loop-engine [options] terminate <run-id>\n".to_owned(),
        Some("list") => "Usage: loop-engine [options] list\n".to_owned(),
        _ => "Usage: loop-engine [options] COMMAND\n\nCommands:\n  start PROVIDER JSON [LABEL]\n  list\n  show RUN_ID\n  append RUN_ID KIND JSON\n  event RUN_ID EVENT\n  history RUN_ID\n  terminate RUN_ID\n  read RUN_ID --kind KIND\n  explore RUN_ID\n  monitor --run RUN_ID --database PATH\n\nGlobal options:\n  --json, -j\n  --database PATH\n  --config PATH\n  --timeout-ms MILLISECONDS\n  --help, -h\n  --version, -V\n".to_owned(),
    }
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
    fn external_work_retired_commands_are_unknown() {
        let help = execute(["--help"]);
        for command in [
            "invoke",
            "amend-binding",
            "cancel-invocation",
            "invocation-progress",
            "preview-bindings",
            "wait-invocation",
            "stdin-exec",
            "fan-out",
            "fan-out-join",
            "fan-out-worker",
            "fan-out-barrier",
            "recover-output",
            "recovery-preview",
            "advise",
            "capture-command",
            "capture-matrix",
            "capture-abort",
        ] {
            assert!(
                !help.stdout.contains(command),
                "retired command in help: {command}"
            );
            for args in [vec![command], vec![command, "--help"]] {
                let result = execute(args);
                assert_eq!(result.exit_code, EXIT_INVALID_INVOCATION, "{command}");
                assert!(
                    result.stderr.contains("unknown operation"),
                    "{command}: {}",
                    result.stderr
                );
            }
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
    fn parser_advice_exception_is_event_scoped_and_separate_from_override() {
        let parsed = parse_args([
            "event",
            "run-1",
            "revise",
            "--advice-exception",
            r#"{"state_visit":0,"owner":"owner","reason":"unavailable","occasion_ids":["review"]}"#,
        ])
        .expect("scoped advice exception should parse");
        let ParsedRequest::Operation {
            command:
                PrimaryCommand::Event {
                    advice_exception: Some(exception),
                    override_attestation: None,
                    ..
                },
            ..
        } = parsed
        else {
            panic!("expected event-scoped advice exception");
        };
        assert_eq!(exception.state_visit, 0);
        assert_eq!(exception.occasion_ids, vec!["review"]);

        for args in [
            vec![
                "show",
                "run-1",
                "--advice-exception",
                r#"{"state_visit":0,"owner":"owner","reason":"r","occasion_ids":["x"]}"#,
            ],
            vec![
                "event",
                "run-1",
                "revise",
                "--advice-exception",
                r#"{"state_visit":0,"owner":"owner","reason":"r","occasion_ids":["x"]}"#,
                "--override",
                r#"{"state_visit":0,"owner":"owner","reason":"r"}"#,
            ],
        ] {
            assert_eq!(parse_args(args).unwrap_err().code, "invalid-invocation");
        }
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
            let ParsedRequest::Operation { options, .. } =
                parse_args([selector, "show", "--compact", "run-1"])
                    .expect("compact status supports JSON")
            else {
                panic!("expected operation")
            };
            assert_eq!(options.show_view, "status");
        }

        let other_error =
            parse_args(["list", "--compact"]).expect_err("compact must remain local to show");
        assert_eq!(other_error.code, "invalid-invocation");
        assert!(other_error.message.contains("only valid"));

        assert!(parse_args(["show", "run-1", "--view", "full"]).is_ok());
        assert!(parse_args(["show", "run-1", "--view", "wrong"]).is_err());
        assert!(parse_args(["show", "run-1", "--view=full"]).is_ok());
        assert!(parse_args(["list", "--view", "action"]).is_err());
        assert!(parse_args(["show", "run-1", "--compact", "--view", "full"]).is_err());
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
            "--view".to_owned(),
            "full".to_owned(),
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
            "--view".to_owned(),
            "full".to_owned(),
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
            "--view".to_owned(),
            "full".to_owned(),
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
