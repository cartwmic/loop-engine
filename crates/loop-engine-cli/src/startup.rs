use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use loop_engine_core::capabilities::id_generator::IdGenerator;

use crate::args::GlobalCli;
use crate::driver_catalog::DRIVER_OPERATION_IDS;
use loop_engine_core::model::bounded::FILESYSTEM_PATH_UTF8_BYTES;
use loop_engine_integrations::configuration::{
    ConfigurationError, MachinePaths, discover_project_config, load_optional,
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
    let mut session = TraceSession {
        writer: Some(writer),
        trace_path,
        request_id,
        format,
    };

    if let Err(error) = session.write_start(&argv) {
        let _ = session.close();
        return emit_trace_init_failure(
            format,
            TraceInitFailure {
                message: trace_init_message(&error),
                request_id: Some(session.request_id.clone()),
                trace: Some(session.trace_path.clone()),
                source_chain: vec![error.to_string()],
            },
        );
    }

    match GlobalCli::try_parse() {
        Ok(cli) => dispatch(session, &paths, cli),
        Err(error) => fail_parse(session, &error),
    }
}

fn dispatch(session: TraceSession, paths: &MachinePaths, cli: GlobalCli) -> i32 {
    let format = match parse_render_format(&cli.format) {
        Ok(format) => format,
        Err(message) => {
            let owned = message.to_owned();
            return fail_parse_with_message(session, "parse", &owned, vec![owned.clone()]);
        }
    };
    let session = session.with_format(format);

    if cli.help {
        return driver_help(session);
    }
    if cli.version {
        return driver_version(session);
    }
    if cli.list_operations {
        return driver_list_operations(session);
    }
    if !cli.rest.is_empty() {
        if let Err(error) = load_configuration(paths) {
            return fail_config(session, &error);
        }
        if !is_supported_platform() {
            return fail_platform(session);
        }
        return fail_parse_with_message(
            session,
            "usage",
            "application commands are not exposed yet",
            vec![
                "no application subcommands are registered in this build".to_owned(),
                format!("unexpected arguments: {}", cli.rest.join(" ")),
            ],
        );
    }

    fail_parse_with_message(
        session,
        "usage",
        "missing application command",
        vec!["expected an application subcommand after global flags".to_owned()],
    )
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
        .map(|id| (id, ""))
        .collect()
}

fn write_driver_metadata(
    session: &mut TraceSession,
    kind: &str,
    render: impl FnOnce() -> io::Result<()>,
) -> Result<(), i32> {
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

    fn write_start(&mut self, argv: &[String]) -> Result<(), TraceError> {
        let (argv_values, argv_truncated, argv_byte_length) = capture_argv(argv);
        let mut payload = BTreeMap::new();
        payload.insert("format".into(), json!(render_format_label(self.format)));
        payload.insert("platform".into(), json!(platform()));
        payload.insert("argv".into(), json!(argv_values));
        payload.insert("argv_truncated".into(), json!(argv_truncated));
        payload.insert("argv_byte_length".into(), json!(argv_byte_length));
        let event = TraceEvent::new(
            self.request_id.clone(),
            TraceCategory::Invocation,
            "start",
            payload,
        );
        self.write_event(&event)
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

fn load_configuration(paths: &MachinePaths) -> Result<(), ConfigurationError> {
    load_optional(&paths.global_config)?;
    let cwd = std::env::current_dir().map_err(ConfigurationError::CurrentDirectory)?;
    if let Some(project_path) = discover_project_config(&cwd)? {
        load_optional(&project_path)?;
    }
    Ok(())
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
        "Application subcommands are not registered in this build.",
        "Use --list-operations to see currently exposed application operations.",
        "",
    ]
    .join("\n")
}

fn trace_path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
