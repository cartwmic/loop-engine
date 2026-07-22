//! Rich diagnostics and source-chain presentation (T127).
//!
//! Maps integration and core failure values into structured DTOs and human
//! stderr/stdout-adjacent lines. Presentation only — exit codes, dispatch, and
//! policy belong to later tasks.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use loop_engine_core::model::attempt::ProviderRole;
use loop_engine_core::model::bounded::DIAGNOSTIC_ENCODED_BYTES;
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::reason::Reason;
use loop_engine_integrations::configuration::ConfigurationError;
use loop_engine_integrations::persistence::{
    CorruptionContext, CorruptionDiagnostic, CorruptionError, PersistenceError,
};
use loop_engine_integrations::provider_process::ProcessError;
use loop_engine_integrations::trace::{TraceError, TraceIoPhase};
use serde::Serialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

/// Frozen structured CLI schema version ([cli-contract.md]).
///
/// [cli-contract.md]: ../../../docs/cli-contract.md
pub const SCHEMA_VERSION: u64 = 1;

/// Pre-dispatch failure phase labels frozen in [cli-contract.md].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreDispatchPhase {
    TraceInit,
    Platform,
    Config,
    Persistence,
    Parse,
    Usage,
}

impl PreDispatchPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TraceInit => "trace_init",
            Self::Platform => "platform",
            Self::Config => "config",
            Self::Persistence => "persistence",
            Self::Parse => "parse",
            Self::Usage => "usage",
        }
    }
}

/// Correlates one CLI invocation with its operational trace when available.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvocationCorrelation {
    pub request_id: Option<String>,
    pub trace_path: Option<String>,
}

impl InvocationCorrelation {
    pub fn new(request_id: Option<String>, trace_path: Option<String>) -> Self {
        Self {
            request_id,
            trace_path,
        }
    }

    pub fn with_trace(request_id: impl Into<String>, trace_path: impl Into<String>) -> Self {
        Self {
            request_id: Some(request_id.into()),
            trace_path: Some(trace_path.into()),
        }
    }
}

/// One structured diagnostics entry for outcome envelopes or ancillary stderr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticEntryDto {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

/// Structured pre-dispatch failure object written to stderr in JSON mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreDispatchFailureDto {
    pub schema_version: u64,
    pub phase: PreDispatchPhase,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_chain: Option<Vec<String>>,
}

impl PreDispatchFailureDto {
    pub fn new(
        phase: PreDispatchPhase,
        message: impl Into<String>,
        correlation: InvocationCorrelation,
        source_chain: Vec<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            phase,
            message: bound_diagnostic_text(message.into()),
            request_id: correlation.request_id,
            trace: correlation.trace_path,
            source_chain: non_empty_chain(source_chain),
        }
    }
}

/// Reason summary paired with diagnostics for human rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonPresentation {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DiagnosticRenderError {
    #[error("diagnostic context must be a JSON object")]
    InvalidDiagnosticContext,
    #[error("structured pre-dispatch payload exceeds {max} UTF-8 bytes (actual {actual})")]
    PreDispatchTooLarge { max: usize, actual: usize },
}

/// Converts one core diagnostic into the structured CLI diagnostics entry shape.
pub fn diagnostic_entry_from_core(
    diagnostic: &Diagnostic,
) -> Result<DiagnosticEntryDto, DiagnosticRenderError> {
    let mut entry = DiagnosticEntryDto {
        code: diagnostic.code().to_owned(),
        message: diagnostic.message().to_owned(),
        context: None,
    };
    if let Some(path) = diagnostic.path() {
        let mut context = Map::new();
        context.insert("path".into(), json!(path));
        entry.context = Some(context);
    }
    Ok(entry)
}

/// Converts core diagnostics into structured CLI entries preserving order.
pub fn diagnostic_entries_from_core(
    diagnostics: &[Diagnostic],
) -> Result<Vec<DiagnosticEntryDto>, DiagnosticRenderError> {
    diagnostics.iter().map(diagnostic_entry_from_core).collect()
}

pub fn reason_presentation(reason: &Reason) -> ReasonPresentation {
    ReasonPresentation {
        code: reason.code().code().to_owned(),
        message: reason.message().to_owned(),
    }
}

pub fn parse_failure(
    message: impl Into<String>,
    correlation: InvocationCorrelation,
    source_chain: Vec<String>,
) -> PreDispatchFailureDto {
    PreDispatchFailureDto::new(PreDispatchPhase::Parse, message, correlation, source_chain)
}

pub fn usage_failure(
    message: impl Into<String>,
    correlation: InvocationCorrelation,
    source_chain: Vec<String>,
) -> PreDispatchFailureDto {
    PreDispatchFailureDto::new(PreDispatchPhase::Usage, message, correlation, source_chain)
}

pub fn platform_failure(
    message: impl Into<String>,
    correlation: InvocationCorrelation,
    source_chain: Vec<String>,
) -> PreDispatchFailureDto {
    PreDispatchFailureDto::new(
        PreDispatchPhase::Platform,
        message,
        correlation,
        source_chain,
    )
}

pub fn configuration_failure(
    error: &ConfigurationError,
    correlation: InvocationCorrelation,
) -> PreDispatchFailureDto {
    PreDispatchFailureDto::new(
        PreDispatchPhase::Config,
        configuration_summary(error),
        correlation,
        configuration_source_chain(error),
    )
}

pub fn persistence_failure(
    error: &PersistenceError,
    correlation: InvocationCorrelation,
) -> PreDispatchFailureDto {
    let (message, source_chain) = persistence_predispatch_payload(error);
    PreDispatchFailureDto::new(
        PreDispatchPhase::Persistence,
        message,
        correlation,
        source_chain,
    )
}

/// Maps a pre-dispatch corruption error into a persistence-phase stderr object.
pub fn corruption_pre_dispatch(
    error: &CorruptionError,
    correlation: InvocationCorrelation,
) -> Option<PreDispatchFailureDto> {
    if !error.phase.is_pre_dispatch() {
        return None;
    }
    Some(PreDispatchFailureDto::new(
        PreDispatchPhase::Persistence,
        corruption_summary(error),
        correlation,
        error.source_chain().to_vec(),
    ))
}

/// Maps post-dispatch corruption findings into outcome diagnostics entries.
pub fn corruption_diagnostics(
    error: &CorruptionError,
) -> Result<Vec<DiagnosticEntryDto>, DiagnosticRenderError> {
    error
        .diagnostics
        .iter()
        .map(corruption_diagnostic_entry)
        .collect()
}

pub fn trace_init_failure(error: &TraceError) -> PreDispatchFailureDto {
    PreDispatchFailureDto::new(
        PreDispatchPhase::TraceInit,
        trace_init_summary(error),
        InvocationCorrelation::default(),
        trace_init_source_chain(error),
    )
}

/// Builds a `trace.sink_failure` diagnostic for post-dispatch envelopes or stderr.
pub fn trace_sink_failure_diagnostic(error: &TraceError, after_commit: bool) -> DiagnosticEntryDto {
    let mut context = Map::new();
    context.insert("errno".into(), json!(trace_failure_errno(error)));
    context.insert("phase".into(), json!(trace_failure_phase(error)));
    context.insert("after_commit".into(), json!(after_commit));
    DiagnosticEntryDto {
        code: "trace.sink_failure".into(),
        message: bound_diagnostic_text(format!(
            "operational trace sink failed during {} ({})",
            trace_failure_phase(error),
            trace_failure_errno(error)
        )),
        context: Some(context),
    }
}

/// Provider invocation diagnostic with bounded context; full payloads stay in trace.
pub fn provider_invocation_diagnostic(
    role: ProviderRole,
    message: impl Into<String>,
    failure_code: &str,
    registration_id: Option<&str>,
) -> DiagnosticEntryDto {
    let mut context = Map::new();
    context.insert("role".into(), json!(provider_role_label(role)));
    context.insert("failure_code".into(), json!(failure_code));
    if let Some(registration_id) = registration_id {
        context.insert("registration_id".into(), json!(registration_id));
    }
    DiagnosticEntryDto {
        code: "provider.invocation".into(),
        message: bound_diagnostic_text(message.into()),
        context: Some(context),
    }
}

pub fn provider_invocation_from_process_error(
    role: ProviderRole,
    error: &ProcessError,
    registration_id: Option<&str>,
) -> DiagnosticEntryDto {
    provider_invocation_diagnostic(
        role,
        error.to_string(),
        process_failure_code(error),
        registration_id,
    )
}

/// Renders one canonical JSON object for structured pre-dispatch stderr.
pub fn render_pre_dispatch_json(
    failure: &PreDispatchFailureDto,
) -> Result<String, DiagnosticRenderError> {
    let value = serde_json::to_value(failure).expect("pre-dispatch dto serializes");
    let rendered = canonical_json(&value);
    if rendered.len() > DIAGNOSTIC_ENCODED_BYTES {
        return Err(DiagnosticRenderError::PreDispatchTooLarge {
            max: DIAGNOSTIC_ENCODED_BYTES,
            actual: rendered.len(),
        });
    }
    Ok(rendered)
}

/// Renders rich human pre-dispatch stderr, including trace-init-only messaging.
pub fn render_pre_dispatch_human(failure: &PreDispatchFailureDto) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Error: {}", failure.message));
    lines.push(format!("Phase: {}", failure.phase.as_str()));
    append_correlation_human(
        &mut lines,
        &InvocationCorrelation {
            request_id: failure.request_id.clone(),
            trace_path: failure.trace.clone(),
        },
        failure.phase,
    );
    append_source_chain_human(&mut lines, failure.source_chain.as_deref().unwrap_or(&[]));
    lines.join("\n")
}

/// Renders human diagnostics for a dispatched outcome failure.
pub fn render_outcome_failure_human(
    correlation: &InvocationCorrelation,
    reason: &ReasonPresentation,
    diagnostics: &[DiagnosticEntryDto],
    source_chain: &[String],
) -> String {
    let mut lines = Vec::new();
    lines.push("Outcome: error".to_string());
    lines.push(format!("Reason: {} — {}", reason.code, reason.message));
    append_correlation_human(&mut lines, correlation, PreDispatchPhase::Persistence);
    append_diagnostics_human(&mut lines, diagnostics);
    append_source_chain_human(&mut lines, source_chain);
    lines.join("\n")
}

/// Renders one diagnostic entry for human mode.
pub fn render_diagnostic_entry_human(entry: &DiagnosticEntryDto) -> String {
    let mut line = format!("- {}: {}", entry.code, entry.message);
    if let Some(context) = &entry.context {
        for (key, value) in context {
            let _ = write!(line, "\n    {key}: {}", render_context_value(value));
        }
    }
    line
}

fn append_correlation_human(
    lines: &mut Vec<String>,
    correlation: &InvocationCorrelation,
    phase: PreDispatchPhase,
) {
    if let Some(request_id) = &correlation.request_id {
        lines.push(format!("Request ID: {request_id}"));
    }
    if let Some(trace_path) = &correlation.trace_path {
        lines.push(format!("Trace: {trace_path}"));
        lines.push(trace_payload_hint());
        return;
    }
    if phase == PreDispatchPhase::TraceInit {
        lines.push(trace_init_stderr_hint());
    }
}

fn append_source_chain_human(lines: &mut Vec<String>, source_chain: &[String]) {
    if source_chain.is_empty() {
        return;
    }
    lines.push("Source chain:".into());
    for entry in source_chain {
        lines.push(format!("  {entry}"));
    }
}

fn append_diagnostics_human(lines: &mut Vec<String>, diagnostics: &[DiagnosticEntryDto]) {
    if diagnostics.is_empty() {
        return;
    }
    lines.push("Diagnostics:".into());
    for entry in diagnostics {
        lines.push(render_diagnostic_entry_human(entry));
    }
}

fn corruption_diagnostic_entry(
    diagnostic: &CorruptionDiagnostic,
) -> Result<DiagnosticEntryDto, DiagnosticRenderError> {
    let context = corruption_context_map(&diagnostic.context);
    Ok(DiagnosticEntryDto {
        code: diagnostic.code.to_owned(),
        message: bound_diagnostic_text(diagnostic.message.clone()),
        context: non_empty_map(context),
    })
}

fn corruption_context_map(context: &CorruptionContext) -> Map<String, Value> {
    context
        .as_map()
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect()
}

fn corruption_summary(error: &CorruptionError) -> String {
    error
        .primary()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| error.to_string())
}

fn configuration_summary(error: &ConfigurationError) -> String {
    match error {
        ConfigurationError::Malformed { message, .. } => {
            bound_diagnostic_text(format!("configuration is malformed: {message}"))
        }
        ConfigurationError::UnsupportedVersion { actual, .. } => bound_diagnostic_text(format!(
            "configuration uses schema version {actual}; supported version is 1"
        )),
        ConfigurationError::TooLarge { max, actual, .. } => bound_diagnostic_text(format!(
            "configuration file exceeds {max} bytes (actual {actual})"
        )),
        _ => bound_diagnostic_text(error.to_string()),
    }
}

fn configuration_source_chain(error: &ConfigurationError) -> Vec<String> {
    match error {
        ConfigurationError::Inspect { path, source } => vec![bound_diagnostic_text(format!(
            "inspect {}: {source}",
            path.display()
        ))],
        ConfigurationError::Read { path, source } => vec![bound_diagnostic_text(format!(
            "read {}: {source}",
            path.display()
        ))],
        ConfigurationError::Malformed { path, message } => vec![bound_diagnostic_text(format!(
            "{}: {message}",
            path.display()
        ))],
        ConfigurationError::UnsupportedVersion { path, actual } => vec![bound_diagnostic_text(
            format!("{}: schema_version {actual}", path.display()),
        )],
        ConfigurationError::TooLarge { path, max, actual } => vec![bound_diagnostic_text(format!(
            "{}: {actual} bytes exceeds {max}",
            path.display()
        ))],
        ConfigurationError::RelativePath(path) => {
            vec![bound_diagnostic_text(format!("relative path: {path}"))]
        }
        ConfigurationError::PathTooLong { max, actual } => vec![bound_diagnostic_text(format!(
            "path exceeds {max} UTF-8 bytes (actual {actual})"
        ))],
        ConfigurationError::CurrentDirectory(source) => {
            vec![bound_diagnostic_text(source.to_string())]
        }
        _ => vec![bound_diagnostic_text(error.to_string())],
    }
}

fn persistence_predispatch_payload(error: &PersistenceError) -> (String, Vec<String>) {
    use PersistenceError::*;
    match error {
        CreateDirectory { path, source } => (
            bound_diagnostic_text("failed to create persistence directory"),
            vec![bound_diagnostic_text(format!(
                "{}: {source}",
                path.display()
            ))],
        ),
        Open { path, source } => (
            bound_diagnostic_text("failed to open persistence store"),
            vec![bound_diagnostic_text(format!(
                "{}: {source}",
                path.display()
            ))],
        ),
        PragmaRead { pragma, source } => (
            bound_diagnostic_text(format!("failed to read pragma {pragma}")),
            vec![bound_diagnostic_text(source.to_string())],
        ),
        Pragma { pragma, source } => (
            bound_diagnostic_text(format!("failed to apply persistence pragma {pragma}")),
            vec![bound_diagnostic_text(source.to_string())],
        ),
        FutureSchema {
            supported,
            observed,
        } => (
            bound_diagnostic_text(format!(
                "database schema version {observed} exceeds supported version {supported}"
            )),
            vec![bound_diagnostic_text(format!(
                "schema version {observed} > {supported}"
            ))],
        ),
        Migration { message } => (
            bound_diagnostic_text("database migration failed"),
            vec![bound_diagnostic_text(message.clone())],
        ),
        SchemaMismatch {
            object_type,
            name,
            kind,
        } => (
            bound_diagnostic_text("database schema shape does not match bundled migration"),
            vec![bound_diagnostic_text(format!(
                "{object_type} {name} {kind:?}"
            ))],
        ),
        SchemaInventoryProbe { source } => (
            bound_diagnostic_text("failed to verify database schema inventory"),
            vec![bound_diagnostic_text(source.to_string())],
        ),
        MetadataKeyMissing { key } => (
            bound_diagnostic_text(format!("integration metadata key {key} is missing")),
            vec![bound_diagnostic_text(format!("metadata key {key} missing"))],
        ),
        MetadataKeyInvalidLength {
            key,
            expected,
            actual,
        } => (
            bound_diagnostic_text(format!("integration metadata key {key} has invalid length")),
            vec![bound_diagnostic_text(format!(
                "expected {expected} bytes, observed {actual}"
            ))],
        ),
        MetadataRead { source } => (
            bound_diagnostic_text("failed to read integration metadata"),
            vec![bound_diagnostic_text(source.to_string())],
        ),
        InvalidUserVersion { observed } => (
            bound_diagnostic_text(format!("invalid SQLite user_version {observed}")),
            vec![bound_diagnostic_text(format!("user_version {observed}"))],
        ),
    }
}

fn trace_init_summary(error: &TraceError) -> String {
    match error {
        TraceError::BudgetExhausted {
            required,
            available,
        } => bound_diagnostic_text(format!(
            "trace directory budget is exhausted (required {required}, available {available})"
        )),
        TraceError::FileLimit { max } => {
            bound_diagnostic_text(format!("trace file would exceed {max} bytes"))
        }
        TraceError::ReservationExhausted => bound_diagnostic_text("trace reservation is exhausted"),
        TraceError::Io {
            path,
            phase,
            source,
        } => bound_diagnostic_text(format!(
            "trace I/O failed during {phase} at {}: {source}",
            path.display()
        )),
        TraceError::Collision(path) => {
            bound_diagnostic_text(format!("trace path collision: {}", path.display()))
        }
        _ => bound_diagnostic_text(error.to_string()),
    }
}

fn trace_init_source_chain(error: &TraceError) -> Vec<String> {
    match error {
        TraceError::Io {
            path,
            phase,
            source,
        } => vec![bound_diagnostic_text(format!(
            "{} during {phase}: {source}",
            path.display()
        ))],
        TraceError::MalformedSidecar(path) => vec![bound_diagnostic_text(format!(
            "reservation sidecar malformed: {}",
            path.display()
        ))],
        TraceError::Collision(path) => vec![bound_diagnostic_text(path.display().to_string())],
        _ => vec![bound_diagnostic_text(error.to_string())],
    }
}

fn trace_failure_errno(error: &TraceError) -> &'static str {
    match error {
        TraceError::Io { source, .. } => errno_name(source),
        TraceError::FileLimit { .. } => "EFBIG",
        TraceError::BudgetExhausted { .. } | TraceError::ReservationExhausted => "ENOSPC",
        TraceError::SinkFailed => "EIO",
        TraceError::Collision(_) => "EEXIST",
        TraceError::ReservedPayloadField(_) | TraceError::Serialize(_) => "EINVAL",
        TraceError::MalformedSidecar(_) => "EIO",
        TraceError::NoProviderReservation => "EINVAL",
    }
}

fn trace_failure_phase(error: &TraceError) -> &'static str {
    match error {
        TraceError::Io { phase, .. } => trace_io_phase_label(*phase),
        _ => "write",
    }
}

fn trace_io_phase_label(phase: TraceIoPhase) -> &'static str {
    match phase {
        TraceIoPhase::Write => "write",
        TraceIoPhase::Flush => "flush",
        TraceIoPhase::Fsync => "fsync",
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const RAW_EFBIG: i32 = 27;

fn errno_name(error: &std::io::Error) -> &'static str {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if error.raw_os_error() == Some(RAW_EFBIG) {
        return "EFBIG";
    }
    match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::UnexpectedEof => "EIO",
        _ => "EIO",
    }
}

fn process_failure_code(error: &ProcessError) -> &'static str {
    match error {
        ProcessError::RequestOversized { .. } => "resource.exhausted",
        ProcessError::ExecutableNotFound(_) => "provider.executable.not_found",
        ProcessError::Spawn(_)
        | ProcessError::TimeoutOutOfRange(_)
        | ProcessError::Stdin(_)
        | ProcessError::Stream(_)
        | ProcessError::Termination(_) => "provider.spawn.failed",
        ProcessError::Timeout => "provider.timeout",
        ProcessError::Crash(_) => "provider.crash",
        ProcessError::Signal(_) => "provider.signal",
        ProcessError::NonZero(_) => "provider.nonzero_exit",
        ProcessError::StdoutOversized { .. } => "provider.protocol.oversized",
        ProcessError::InvalidUtf8 => "provider.protocol.invalid_utf8",
        ProcessError::Malformed(_) => "provider.protocol.malformed",
    }
}

fn provider_role_label(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

fn trace_payload_hint() -> String {
    "Full provider, persistence, and request/outcome payloads are recorded in the operational trace."
        .into()
}

fn trace_init_stderr_hint() -> String {
    "No operational trace was created. Inspect stderr for trace-init failure detail; bounded payloads are not duplicated here."
        .into()
}

fn render_context_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => canonical_json(other),
    }
}

fn non_empty_chain(chain: Vec<String>) -> Option<Vec<String>> {
    if chain.is_empty() { None } else { Some(chain) }
}

fn non_empty_map(map: Map<String, Value>) -> Option<Map<String, Value>> {
    if map.is_empty() { None } else { Some(map) }
}

fn bound_diagnostic_text(text: impl Into<String>) -> String {
    let text = text.into();
    if text.len() <= DIAGNOSTIC_ENCODED_BYTES {
        return text;
    }
    let mut end = DIAGNOSTIC_ENCODED_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&canonical_value(value)).expect("canonical json")
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .iter()
                .map(|(key, value)| (key.as_str(), canonical_value(value)))
                .collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use loop_engine_core::model::diagnostic::Diagnostic;
    use loop_engine_integrations::persistence::PersistenceError;

    #[test]
    fn core_diagnostic_path_becomes_structured_context() {
        let diagnostic = Diagnostic::new(
            "persistence.failed",
            "referenced inputs file is missing",
            Some("/inputs/name".into()),
        )
        .unwrap();
        let entry = diagnostic_entry_from_core(&diagnostic).unwrap();
        assert_eq!(entry.code, "persistence.failed");
        assert_eq!(
            entry.context.as_ref().and_then(|ctx| ctx.get("path")),
            Some(&json!("/inputs/name"))
        );
    }

    #[test]
    fn trace_init_human_mentions_stderr_only_record() {
        let failure = trace_init_failure(&TraceError::ReservationExhausted);
        let human = render_pre_dispatch_human(&failure);
        assert!(human.contains("trace_init"));
        assert!(human.contains("No operational trace was created"));
        assert!(!human.contains("Trace:"));
    }

    #[test]
    fn persistence_migration_pre_dispatch_matches_contract_fields() {
        let correlation = InvocationCorrelation::with_trace(
            "01J9X3K2M4N5P6Q7R8S9T0V7C",
            "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V7C.jsonl",
        );
        let failure = persistence_failure(
            &PersistenceError::Migration {
                message: "migration 0002_add_indexes: UNIQUE constraint failed".into(),
            },
            correlation,
        );
        let json = render_pre_dispatch_json(&failure).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["phase"], "persistence");
        assert_eq!(value["message"], "database migration failed");
        assert_eq!(
            value["source_chain"][0],
            "migration 0002_add_indexes: UNIQUE constraint failed"
        );
        let human = render_pre_dispatch_human(&failure);
        assert!(human.contains("Trace:"));
        assert!(human.contains("operational trace"));
    }
}
