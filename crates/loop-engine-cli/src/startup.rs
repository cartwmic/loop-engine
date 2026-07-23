use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use loop_engine_core::capabilities::id_generator::IdGenerator;

use crate::args::{GlobalCli, parse_planned_application};
use crate::composition::{
    LoadedConfiguration, TraceCorrelation, build_application_from_configuration,
};
use crate::driver_catalog::DRIVER_OPERATION_IDS;
use crate::execution::{
    PreparedApplicationCommand, execute_application_command, prepare_application_command,
};
use crate::exit::exit_code_for_outcome;
use crate::render::human::render_human_envelope;
use loop_engine_core::model::bounded::FILESYSTEM_PATH_UTF8_BYTES;
use loop_engine_integrations::configuration::{
    CliDefaults, ConfigurationError, MachinePaths, OutputFormat,
};
use loop_engine_integrations::trace::{TraceCategory, TraceError, TraceEvent, TraceWriter};
use loop_engine_integrations::uuid_ids::UuidV7Generator;
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SUPPORTED_PLATFORMS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];
const EXIT_PRE_DISPATCH: i32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderFormat {
    Human,
    Json,
}

struct TraceSession {
    writer: Option<TraceWriter>,
    trace_path: PathBuf,
    request_id: String,
    format: RenderFormat,
    argv: Vec<String>,
    start_written: bool,
}

struct TraceInitFailure {
    message: String,
    request_id: Option<String>,
    trace: Option<PathBuf>,
    source_chain: Vec<String>,
}

pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    let format = detect_format(&argv);

    let request_id = UuidV7Generator
        .request_id()
        .expect("UUID request id generation is infallible")
        .to_string();

    let paths = match MachinePaths::from_environment() {
        Ok(paths) => paths,
        Err(error) => {
            return emit_early_pre_dispatch_failure(
                format,
                "config",
                configuration_message(&error),
                source_chain(&error),
            );
        }
    };

    let writer = match TraceWriter::create(&paths.traces, &request_id) {
        Ok(writer) => writer,
        Err(error) => {
            return emit_trace_init_failure(
                format,
                TraceInitFailure {
                    message: trace_init_message(&error),
                    request_id: Some(request_id),
                    trace: None,
                    source_chain: vec![error.to_string()],
                },
            );
        }
    };

    let trace_path = writer.path().to_owned();
    let session = TraceSession {
        writer: Some(writer),
        trace_path,
        request_id,
        format,
        argv,
        start_written: false,
    };

    match GlobalCli::try_parse() {
        Ok(cli) => dispatch(session, &paths, cli),
        Err(error) => fail_parse(session, &error),
    }
}

fn dispatch(session: TraceSession, paths: &MachinePaths, cli: GlobalCli) -> i32 {
    let explicit_format = match cli.format.as_deref().map(parse_render_format).transpose() {
        Ok(format) => format,
        Err(message) => {
            let owned = message.to_owned();
            return fail_parse_with_message(session, "parse", &owned, vec![owned.clone()]);
        }
    };
    let session = match explicit_format {
        Some(format) => session.with_format(format),
        None => session,
    };

    if cli.help {
        return driver_help(session);
    }
    if cli.version {
        return driver_version(session);
    }
    if cli.list_operations {
        return driver_list_operations(session);
    }
    if cli.rest.is_empty() {
        return fail_parse_with_message(
            session,
            "usage",
            "missing application command",
            vec!["expected an application subcommand after global flags".to_owned()],
        );
    }

    let cli_defaults = CliDefaults {
        format: explicit_format.map(|format| match format {
            RenderFormat::Human => OutputFormat::Human,
            RenderFormat::Json => OutputFormat::Json,
        }),
        ..CliDefaults::default()
    };
    let configuration = match crate::composition::load_configuration(paths, &cli_defaults) {
        Ok(configuration) => configuration,
        Err(error) => return fail_config(session, &error),
    };
    let session = session.with_format(match configuration.defaults.format {
        OutputFormat::Human => RenderFormat::Human,
        OutputFormat::Json => RenderFormat::Json,
    });

    let command = match parse_planned_application(&cli.rest) {
        Ok(command) => command,
        Err(error) => {
            let message = error.to_string();
            return fail_parse_with_message(session, "parse", &message, vec![message.clone()]);
        }
    };
    let operation = command.operation_id();
    if !DRIVER_OPERATION_IDS.contains(&operation.as_str()) {
        return fail_parse_with_message(
            session,
            "usage",
            "application command is not exposed",
            vec![format!(
                "operation {} is not registered in this build",
                operation.as_str()
            )],
        );
    }
    if !is_supported_platform() {
        return fail_platform(session);
    }
    let home = std::env::var_os("HOME");
    let command = match prepare_application_command(
        command,
        &configuration.caller_cwd,
        home.as_deref(),
        configuration.defaults.timeout_seconds,
    ) {
        Ok(command) => command,
        Err(error) => {
            let message = error.to_string();
            return fail_parse_with_message(session, "parse", &message, vec![message.clone()]);
        }
    };
    dispatch_application(session, configuration, command)
}

fn dispatch_application(
    mut session: TraceSession,
    configuration: LoadedConfiguration,
    command: PreparedApplicationCommand,
) -> i32 {
    if let Err(error) = session.write_start_digest() {
        return fail_sink(&mut session, error);
    }
    let writer = session
        .writer
        .take()
        .expect("startup trace remains owned until application composition");
    let trace = TraceCorrelation::adopt(writer);
    let finish_trace = trace.clone();
    let application = match build_application_from_configuration(configuration, trace) {
        Ok(application) => application,
        Err(error) => {
            return fail_adopted_application(
                session,
                finish_trace,
                "persistence",
                &error.to_string(),
            );
        }
    };

    let delivery = match execute_application_command(&application, command) {
        Ok(delivery) => delivery,
        Err(error) => {
            drop(application);
            let _ = finish_adopted_trace(finish_trace, 1);
            eprintln!("application dispatch failed: {error}");
            return 1;
        }
    };
    let outcome_exit_code = exit_code_for_outcome(delivery.outcome_class());
    drop(application);

    let rendered =
        match session.format {
            RenderFormat::Json => serde_json::to_string(delivery.structured_envelope())
                .map_err(|error| error.to_string()),
            RenderFormat::Human => render_human_envelope(delivery.structured_envelope())
                .map_err(|error| error.to_string()),
        };
    let output = rendered
        .and_then(|rendered| write_stdout_text(&rendered).map_err(|error| error.to_string()));
    let process_exit_code = if output.is_ok() { outcome_exit_code } else { 1 };
    if let Err(error) = finish_adopted_trace(finish_trace, process_exit_code) {
        eprintln!("trace sink failure after dispatch: {error}");
    }

    match output {
        Ok(()) => outcome_exit_code,
        Err(error) => {
            eprintln!("failed to render dispatched outcome: {error}");
            1
        }
    }
}

fn fail_adopted_application(
    session: TraceSession,
    trace: TraceCorrelation,
    phase: &str,
    message: &str,
) -> i32 {
    if let Ok(mut writer) = trace.try_into_writer() {
        let mut payload = BTreeMap::new();
        payload.insert("phase".into(), json!(phase));
        payload.insert("message".into(), json!(message));
        let event = TraceEvent::new(
            session.request_id.clone(),
            TraceCategory::Invocation,
            "error",
            payload,
        );
        let _ = writer.write(&event);
        let mut finish_payload = BTreeMap::new();
        finish_payload.insert("exit_code".into(), json!(EXIT_PRE_DISPATCH));
        let finish = TraceEvent::new(
            session.request_id.clone(),
            TraceCategory::Invocation,
            "finish",
            finish_payload,
        );
        let _ = writer.write(&finish);
        let _ = writer.close();
    }
    emit_pre_dispatch_failure(
        session.format,
        json!({
            "schema_version": 1,
            "phase": phase,
            "message": message,
            "request_id": session.request_id,
            "trace": trace_path_string(&session.trace_path),
            "source_chain": [message],
        }),
    );
    EXIT_PRE_DISPATCH
}

fn finish_adopted_trace(trace: TraceCorrelation, exit_code: i32) -> Result<(), TraceError> {
    let mut writer = trace
        .try_into_writer()
        .map_err(|_| TraceError::SinkFailed)?;
    let mut payload = BTreeMap::new();
    payload.insert("exit_code".into(), json!(exit_code));
    let event = TraceEvent::new(
        writer.request_id(),
        TraceCategory::Invocation,
        "finish",
        payload,
    );
    writer.write(&event)?;
    writer.close()
}

fn driver_help(mut session: TraceSession) -> i32 {
    let usage = help_usage();
    let format = session.format;
    let request_id = session.request_id.clone();
    let trace_path = trace_path_string(&session.trace_path);
    if let Err(code) = write_driver_metadata(&mut session, "help", || match format {
        RenderFormat::Human => {
            print!("{usage}");
            Ok(())
        }
        RenderFormat::Json => write_stdout_json(&json!({
            "schema_version": 1,
            "kind": "help",
            "usage": usage,
            "request_id": request_id,
            "trace": trace_path,
        })),
    }) {
        return code;
    }
    finish(session, 0)
}

fn driver_version(mut session: TraceSession) -> i32 {
    let format = session.format;
    let request_id = session.request_id.clone();
    let trace_path = trace_path_string(&session.trace_path);
    if let Err(code) = write_driver_metadata(&mut session, "version", || match format {
        RenderFormat::Human => {
            println!("loop-engine {VERSION}");
            Ok(())
        }
        RenderFormat::Json => write_stdout_json(&json!({
            "schema_version": 1,
            "kind": "version",
            "name": "loop-engine",
            "version": VERSION,
            "request_id": request_id,
            "trace": trace_path,
        })),
    }) {
        return code;
    }
    finish(session, 0)
}

fn driver_list_operations(mut session: TraceSession) -> i32 {
    let format = session.format;
    let request_id = session.request_id.clone();
    let trace_path = trace_path_string(&session.trace_path);
    let operations = driver_catalog_operations();
    if let Err(code) = write_driver_metadata(&mut session, "list_operations", || match format {
        RenderFormat::Human => {
            for (id, argv) in &operations {
                println!("{id}\t{argv}");
            }
            Ok(())
        }
        RenderFormat::Json => {
            let rows = operations
                .iter()
                .map(|(id, argv)| json!({ "id": id, "argv": argv }))
                .collect::<Vec<_>>();
            write_stdout_json(&json!({
                "schema_version": 1,
                "kind": "operation_list",
                "operations": rows,
                "request_id": request_id,
                "trace": trace_path,
            }))
        }
    }) {
        return code;
    }
    finish(session, 0)
}

/// Operation rows for `--list-operations`, derived from the production driver catalog.
fn driver_catalog_operations() -> Vec<(&'static str, &'static str)> {
    DRIVER_OPERATION_IDS
        .iter()
        .copied()
        .map(|id| {
            let argv = match id {
                "provider.add" => {
                    "provider add <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]"
                }
                "provider.list" => {
                    "provider list [--enabled] [--tombstoned] [--active-runs-for <REGISTRATION-ID>] [--cursor <CURSOR>] [--limit <COUNT>]"
                }
                "provider.check" => {
                    "provider check <TARGET> [--active-runs] [--cursor <CURSOR>] [--limit <COUNT>]"
                }
                "run.create" => "run create <TARGET> [--label <LABEL>] [--inputs <PATH>]",
                "run.list" => {
                    "run list [--terminal] [--all] [--cursor <CURSOR>] [--limit <COUNT>]"
                }
                "run.terminate" => "run terminate <RUN-ID> [--note <TEXT>]",
                "run.history" => {
                    "run history <RUN-ID> [--cursor <CURSOR>] [--limit <COUNT>]"
                }
                "run.show" => "run show <RUN-ID>",
                "run.request" => {
                    "run request <RUN-ID> <EVENT> [--evidence-id <ID> ...] [--evidence <PATH>] [--note <TEXT>]"
                }
                _ => unreachable!("driver catalog entry must own an argv template"),
            };
            (id, argv)
        })
        .collect()
}

fn write_driver_metadata(
    session: &mut TraceSession,
    kind: &str,
    render: impl FnOnce() -> io::Result<()>,
) -> Result<(), i32> {
    session
        .write_start_raw()
        .map_err(|error| fail_sink(session, error))?;
    let mut payload = BTreeMap::new();
    payload.insert("kind".into(), json!(kind));
    let event = TraceEvent::new(
        session.request_id.clone(),
        TraceCategory::Driver,
        "metadata",
        payload,
    );
    session
        .write_event(&event)
        .map_err(|error| fail_sink(session, error))?;
    render().map_err(|error| {
        fail_pre_dispatch(
            session,
            "platform",
            format!("failed to write driver metadata to stdout: {error}"),
            vec![error.to_string()],
        )
    })
}

fn fail_parse(session: TraceSession, error: &clap::Error) -> i32 {
    let (phase, message, chain) = match error.kind() {
        clap::error::ErrorKind::DisplayHelp
        | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            ("usage", error.to_string(), vec![error.to_string()])
        }
        clap::error::ErrorKind::DisplayVersion => {
            ("parse", error.to_string(), vec![error.to_string()])
        }
        _ => ("parse", error.to_string(), vec![error.to_string()]),
    };
    fail_parse_with_message(session, phase, &message, chain)
}

fn fail_parse_with_message(
    session: TraceSession,
    phase: &str,
    message: &str,
    source_chain: Vec<String>,
) -> i32 {
    let mut session = session;
    if let Err(error) = session.write_start_raw() {
        return fail_sink(&mut session, error);
    }
    let mut payload = BTreeMap::new();
    payload.insert("phase".into(), json!(phase));
    payload.insert("message".into(), json!(message));
    if !source_chain.is_empty() {
        payload.insert("source_chain".into(), json!(source_chain.clone()));
    }
    let event = TraceEvent::new(
        session.request_id.clone(),
        TraceCategory::Parse,
        "failure",
        payload,
    );
    if let Err(error) = session.write_event(&event) {
        return fail_sink(&mut session, error);
    }
    emit_pre_dispatch_failure(
        session.format,
        json!({
            "schema_version": 1,
            "phase": phase,
            "message": message,
            "request_id": session.request_id,
            "trace": trace_path_string(&session.trace_path),
            "source_chain": source_chain,
        }),
    );
    finish(session, EXIT_PRE_DISPATCH)
}

fn fail_config(session: TraceSession, error: &ConfigurationError) -> i32 {
    let message = configuration_message(error);
    let chain = source_chain(error);
    fail_invocation_error(session, "config", &message, chain)
}

fn fail_platform(session: TraceSession) -> i32 {
    let message = format!(
        "unsupported platform {}; supported targets are {}",
        platform(),
        SUPPORTED_PLATFORMS.join(", ")
    );
    fail_invocation_error(
        session,
        "platform",
        &message,
        vec![format!("detected target: {}", platform())],
    )
}

fn fail_invocation_error(
    session: TraceSession,
    phase: &str,
    message: &str,
    source_chain: Vec<String>,
) -> i32 {
    let mut session = session;
    if let Err(error) = session.write_start_raw() {
        return fail_sink(&mut session, error);
    }
    let mut payload = BTreeMap::new();
    payload.insert("phase".into(), json!(phase));
    payload.insert("message".into(), json!(message));
    if !source_chain.is_empty() {
        payload.insert("source_chain".into(), json!(source_chain.clone()));
    }
    let event = TraceEvent::new(
        session.request_id.clone(),
        TraceCategory::Invocation,
        "error",
        payload,
    );
    if let Err(error) = session.write_event(&event) {
        return fail_sink(&mut session, error);
    }
    emit_pre_dispatch_failure(
        session.format,
        json!({
            "schema_version": 1,
            "phase": phase,
            "message": message,
            "request_id": session.request_id,
            "trace": trace_path_string(&session.trace_path),
            "source_chain": source_chain,
        }),
    );
    finish(session, EXIT_PRE_DISPATCH)
}

fn fail_sink(session: &mut TraceSession, error: TraceError) -> i32 {
    let _ = session.close();
    emit_pre_dispatch_failure(
        session.format,
        json!({
            "schema_version": 1,
            "phase": "trace_init",
            "message": trace_init_message(&error),
            "request_id": session.request_id,
            "trace": trace_path_string(&session.trace_path),
            "source_chain": [error.to_string()],
        }),
    );
    EXIT_PRE_DISPATCH
}

fn fail_pre_dispatch(
    session: &TraceSession,
    phase: &str,
    message: String,
    source_chain: Vec<String>,
) -> i32 {
    emit_pre_dispatch_failure(
        session.format,
        json!({
            "schema_version": 1,
            "phase": phase,
            "message": message,
            "request_id": session.request_id,
            "trace": trace_path_string(&session.trace_path),
            "source_chain": source_chain,
        }),
    );
    EXIT_PRE_DISPATCH
}

fn finish(mut session: TraceSession, exit_code: i32) -> i32 {
    let mut payload = BTreeMap::new();
    payload.insert("exit_code".into(), json!(exit_code));
    let event = TraceEvent::new(
        session.request_id.clone(),
        TraceCategory::Invocation,
        "finish",
        payload,
    );
    if let Err(error) = session.write_event(&event) {
        return fail_sink(&mut session, error);
    }
    if let Err(error) = session.close() {
        return fail_sink(&mut session, error);
    }
    exit_code
}

impl TraceSession {
    fn with_format(self, format: RenderFormat) -> Self {
        Self { format, ..self }
    }

    fn write_start_raw(&mut self) -> Result<(), TraceError> {
        if self.start_written {
            return Ok(());
        }
        let (argv_values, argv_truncated, argv_byte_length) = capture_argv(&self.argv);
        let mut payload = self.start_payload(argv_byte_length);
        payload.insert("argv".into(), json!(argv_values));
        payload.insert("argv_truncated".into(), json!(argv_truncated));
        self.write_start_payload(payload)
    }

    fn write_start_digest(&mut self) -> Result<(), TraceError> {
        if self.start_written {
            return Ok(());
        }
        let argv_byte_length = self.argv.iter().map(String::len).sum();
        let joined = self.argv.join("\0");
        let mut payload = self.start_payload(argv_byte_length);
        payload.insert(
            "argv_digest".into(),
            json!(loop_engine_integrations::sha256_digest::sha256_hex(
                joined.as_bytes()
            )),
        );
        self.write_start_payload(payload)
    }

    fn start_payload(&self, argv_byte_length: usize) -> BTreeMap<String, Value> {
        let mut payload = BTreeMap::new();
        payload.insert("format".into(), json!(render_format_label(self.format)));
        payload.insert("platform".into(), json!(platform()));
        payload.insert("argv_byte_length".into(), json!(argv_byte_length));
        payload
    }

    fn write_start_payload(&mut self, payload: BTreeMap<String, Value>) -> Result<(), TraceError> {
        let event = TraceEvent::new(
            self.request_id.clone(),
            TraceCategory::Invocation,
            "start",
            payload,
        );
        self.write_event(&event)?;
        self.start_written = true;
        Ok(())
    }

    fn write_event(&mut self, event: &TraceEvent) -> Result<(), TraceError> {
        self.writer
            .as_mut()
            .expect("trace writer remains open until finish")
            .write(event)
            .map(|_| ())
    }

    fn close(&mut self) -> Result<(), TraceError> {
        match self.writer.take() {
            Some(writer) => writer.close(),
            None => Ok(()),
        }
    }
}

fn capture_argv(argv: &[String]) -> (Vec<Value>, bool, usize) {
    let argv_byte_length = argv.iter().map(String::len).sum();
    let mut argv_truncated = false;
    let mut captured = Vec::with_capacity(argv.len());
    for arg in argv {
        if arg.len() > FILESYSTEM_PATH_UTF8_BYTES {
            argv_truncated = true;
            captured.push(Value::String(truncate_utf8(
                arg,
                FILESYSTEM_PATH_UTF8_BYTES,
            )));
        } else {
            captured.push(Value::String(arg.clone()));
        }
    }
    (captured, argv_truncated, argv_byte_length)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn detect_format(argv: &[String]) -> RenderFormat {
    let mut index = 1;
    while index < argv.len() {
        match argv[index].as_str() {
            "--format" => {
                if let Some(value) = argv.get(index + 1) {
                    return parse_render_format(value).unwrap_or(RenderFormat::Human);
                }
            }
            value if value.starts_with("--format=") => {
                let value = value.trim_start_matches("--format=");
                return parse_render_format(value).unwrap_or(RenderFormat::Human);
            }
            "--" => break,
            _ => {}
        }
        index += 1;
    }
    RenderFormat::Human
}

fn parse_render_format(value: &str) -> Result<RenderFormat, &'static str> {
    match value {
        "human" => Ok(RenderFormat::Human),
        "json" => Ok(RenderFormat::Json),
        _ => Err("format must be human or json"),
    }
}

fn render_format_label(format: RenderFormat) -> &'static str {
    match format {
        RenderFormat::Human => "human",
        RenderFormat::Json => "json",
    }
}

fn platform() -> String {
    let environment = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{environment}", std::env::consts::ARCH)
}

fn is_supported_platform() -> bool {
    SUPPORTED_PLATFORMS.contains(&platform().as_str())
}

fn help_usage() -> String {
    [
        "loop-engine — workflow control plane",
        "",
        "Usage:",
        "  loop-engine [OPTIONS]",
        "",
        "Global options:",
        "  -h, --help               Print usage help",
        "      --version            Print version information",
        "      --list-operations    List currently exposed application operations",
        "      --format <human|json>  Output rendering mode (default: human)",
        "",
        "Provider catalog foundation commands are available in this build.",
        "Use --list-operations to see currently exposed application operations.",
        "",
    ]
    .join("\n")
}

fn trace_path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_stdout_text(value: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(value.as_bytes())?;
    stdout.write_all(b"\n")
}

fn write_stdout_json(value: &Value) -> io::Result<()> {
    serde_json::to_writer(io::stdout(), value)?;
    io::stdout().write_all(b"\n")
}

fn emit_early_pre_dispatch_failure(
    format: RenderFormat,
    phase: &str,
    message: String,
    source_chain: Vec<String>,
) -> i32 {
    emit_pre_dispatch_failure(
        format,
        json!({
            "schema_version": 1,
            "phase": phase,
            "message": message,
            "request_id": Value::Null,
            "trace": Value::Null,
            "source_chain": source_chain,
        }),
    );
    EXIT_PRE_DISPATCH
}

fn emit_trace_init_failure(format: RenderFormat, failure: TraceInitFailure) -> i32 {
    emit_pre_dispatch_failure(
        format,
        json!({
            "schema_version": 1,
            "phase": "trace_init",
            "message": failure.message,
            "request_id": failure.request_id,
            "trace": failure.trace.as_ref().map(|path| trace_path_string(path)),
            "source_chain": failure.source_chain,
        }),
    );
    EXIT_PRE_DISPATCH
}

fn emit_pre_dispatch_failure(format: RenderFormat, payload: Value) {
    match format {
        RenderFormat::Human => {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("pre-dispatch failure");
            let _ = writeln!(io::stderr(), "Error: {message}");
            if let Some(request_id) = payload.get("request_id").and_then(Value::as_str) {
                let _ = writeln!(io::stderr(), "Request ID: {request_id}");
            }
            if let Some(trace) = payload.get("trace").and_then(Value::as_str) {
                let _ = writeln!(io::stderr(), "Trace: {trace}");
            }
            if let Some(chain) = payload.get("source_chain").and_then(Value::as_array) {
                for entry in chain {
                    if let Some(line) = entry.as_str() {
                        let _ = writeln!(io::stderr(), "  caused by: {line}");
                    }
                }
            }
        }
        RenderFormat::Json => {
            let _ = serde_json::to_writer(io::stderr(), &payload);
            let _ = io::stderr().write_all(b"\n");
        }
    }
}

fn trace_init_message(error: &TraceError) -> String {
    format!("failed to initialize operational trace: {error}")
}

fn configuration_message(error: &ConfigurationError) -> String {
    error.to_string()
}

fn source_chain(error: &ConfigurationError) -> Vec<String> {
    vec![error.to_string()]
}
