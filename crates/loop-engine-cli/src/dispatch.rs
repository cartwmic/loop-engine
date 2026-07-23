//! Traced application operation dispatcher (T124).
//!
//! Single generic choke point for one core operation per user intent. Emits
//! `invocation.request` and `invocation.outcome` with exact trace correlation.
//! `invocation.start` and `invocation.finish` remain startup/exit responsibility.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use loop_engine_core::model::outcome::{OutcomeClass, OutcomeError, PublicOutcome};
use loop_engine_core::operations::catalog::OperationId;
use loop_engine_integrations::trace::{
    TraceCategory, TraceError, TraceEvent, TraceIoPhase, TraceWriter,
};
use serde_json::{Value, json};
use thiserror::Error;

use crate::composition::TraceCorrelation;
use crate::diagnostics::{DiagnosticEntryDto, trace_sink_failure_diagnostic};
use crate::render::dto::{OutcomeRenderError, OutcomeRenderRequest, STRUCTURED_CLI_ENVELOPE_BYTES};
use crate::render::json::build_outcome_envelope;

/// Bounded request payload and operation-specific envelope extensions for one dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedDispatchInput {
    pub operation: OperationId,
    pub request: Value,
    pub operation_data: Value,
}

/// Authoritative operation result plus persistence commit disposition for trace sink truthfulness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedOperationResult {
    pub outcome: PublicOutcome,
    pub after_commit: bool,
}

/// Which dispatcher trace write failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchTraceFailurePhase {
    Request,
    Outcome,
}

/// Recorded post-init trace sink failure at the dispatch boundary.
#[derive(Debug)]
pub struct DispatchTraceFailure {
    pub phase: DispatchTraceFailurePhase,
    pub after_commit: bool,
    pub error: TraceError,
}

/// Bounded delivery artifact for the exit layer without outcome reclassification.
#[derive(Debug)]
pub struct DispatchDelivery {
    operation: OperationId,
    request_id: String,
    trace_path: String,
    outcome: PublicOutcome,
    operation_data: Value,
    structured_envelope: Value,
    trace_failures: Vec<DispatchTraceFailure>,
}

impl DispatchDelivery {
    pub fn operation(&self) -> OperationId {
        self.operation
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn trace_path(&self) -> &str {
        &self.trace_path
    }

    pub fn outcome(&self) -> &PublicOutcome {
        &self.outcome
    }

    pub fn outcome_class(&self) -> OutcomeClass {
        self.outcome.class()
    }

    pub fn operation_data(&self) -> &Value {
        &self.operation_data
    }

    pub fn structured_envelope(&self) -> &Value {
        &self.structured_envelope
    }

    pub fn trace_failures(&self) -> &[DispatchTraceFailure] {
        &self.trace_failures
    }

    pub fn render_request(&self) -> OutcomeRenderRequest<'_> {
        OutcomeRenderRequest {
            operation: self.operation,
            request_id: &self.request_id,
            trace_path: &self.trace_path,
            outcome: &self.outcome,
            operation_data: self.operation_data.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("operation request must be a JSON object")]
    InvalidRequestShape,
    #[error("operation request exceeds {max} UTF-8 bytes when encoded for trace (actual {actual})")]
    RequestTooLarge { max: usize, actual: usize },
    #[error(transparent)]
    Outcome(#[from] OutcomeError),
    #[error(transparent)]
    Envelope(#[from] OutcomeRenderError),
}

/// Executes exactly one supplied operation, emitting bounded request/outcome trace events once each.
pub fn dispatch_traced_operation<F>(
    trace: &TraceCorrelation,
    input: TracedDispatchInput,
    execute: F,
) -> Result<DispatchDelivery, DispatchError>
where
    F: FnOnce() -> Result<TracedOperationResult, OutcomeError>,
{
    let operation_data = input.operation_data.clone();
    dispatch_traced_operation_with_data(trace, input, || {
        execute().map(|result| (result, operation_data))
    })
}

/// Dynamic-data variant used by production routes whose response data is produced by execution.
pub fn dispatch_traced_operation_with_data<F>(
    trace: &TraceCorrelation,
    input: TracedDispatchInput,
    execute: F,
) -> Result<DispatchDelivery, DispatchError>
where
    F: FnOnce() -> Result<(TracedOperationResult, Value), OutcomeError>,
{
    validate_dispatch_input(&input)?;

    let request_id = trace.request_id().to_owned();
    let trace_path = trace_path_string(trace.writer().lock().expect("trace writer").path());
    let writer = trace.writer();
    let mut trace_failures = Vec::new();

    record_trace_write(
        &writer,
        &mut trace_failures,
        DispatchTraceFailurePhase::Request,
        false,
        write_invocation_request(&writer, &request_id, input.operation, &input.request),
    );

    let (
        TracedOperationResult {
            outcome,
            after_commit,
        },
        operation_data,
    ) = execute()?;

    let mut structured_envelope = build_outcome_envelope(&OutcomeRenderRequest {
        operation: input.operation,
        request_id: &request_id,
        trace_path: &trace_path,
        outcome: &outcome,
        operation_data: operation_data.clone(),
    })?;
    append_trace_failure_diagnostics(&mut structured_envelope, &trace_failures);
    ensure_envelope_bound(&structured_envelope)?;

    let outcome_write = write_invocation_outcome(&writer, &request_id, &structured_envelope);
    if let Err(error) = outcome_write {
        let failure = DispatchTraceFailure {
            phase: DispatchTraceFailurePhase::Outcome,
            after_commit,
            error,
        };
        emit_trace_sink_failure(&writer, &failure.error, failure.after_commit);
        append_diagnostic(
            &mut structured_envelope,
            &trace_sink_failure_diagnostic(&failure.error, failure.after_commit),
        );
        ensure_envelope_bound(&structured_envelope)?;
        trace_failures.push(failure);
    }

    Ok(DispatchDelivery {
        operation: input.operation,
        request_id,
        trace_path,
        outcome,
        operation_data,
        structured_envelope,
        trace_failures,
    })
}

fn record_trace_write(
    writer: &Arc<Mutex<TraceWriter>>,
    trace_failures: &mut Vec<DispatchTraceFailure>,
    phase: DispatchTraceFailurePhase,
    after_commit: bool,
    result: Result<(), TraceError>,
) {
    if let Err(error) = result {
        emit_trace_sink_failure(writer, &error, after_commit);
        trace_failures.push(DispatchTraceFailure {
            phase,
            after_commit,
            error,
        });
    }
}

fn write_invocation_request(
    writer: &Arc<Mutex<TraceWriter>>,
    request_id: &str,
    operation: OperationId,
    request: &Value,
) -> Result<(), TraceError> {
    let mut payload = BTreeMap::new();
    payload.insert("operation".into(), json!(operation.as_str()));
    payload.insert("request".into(), request.clone());
    write_invocation_event(writer, request_id, "request", payload)
}

fn write_invocation_outcome(
    writer: &Arc<Mutex<TraceWriter>>,
    request_id: &str,
    envelope: &Value,
) -> Result<(), TraceError> {
    let mut payload = BTreeMap::new();
    payload.insert("envelope".into(), envelope.clone());
    write_invocation_event(writer, request_id, "outcome", payload)
}

fn write_invocation_event(
    writer: &Arc<Mutex<TraceWriter>>,
    request_id: &str,
    event: &str,
    payload: BTreeMap<String, Value>,
) -> Result<(), TraceError> {
    let trace_event = TraceEvent::new(request_id, TraceCategory::Invocation, event, payload);
    writer
        .lock()
        .map_err(|_| TraceError::SinkFailed)?
        .write(&trace_event)
        .map(|_| ())
}

fn emit_trace_sink_failure(
    writer: &Arc<Mutex<TraceWriter>>,
    error: &TraceError,
    after_commit: bool,
) {
    let mut payload = BTreeMap::new();
    payload.insert("errno".into(), json!(trace_failure_errno(error)));
    payload.insert("phase".into(), json!(trace_failure_phase(error)));
    payload.insert("after_commit".into(), json!(after_commit));
    if let Ok(mut locked) = writer.lock() {
        let event = TraceEvent::new(
            locked.request_id(),
            TraceCategory::Trace,
            "sink_failure",
            payload,
        );
        let _ = locked.write(&event);
    }
}

fn append_trace_failure_diagnostics(envelope: &mut Value, trace_failures: &[DispatchTraceFailure]) {
    for failure in trace_failures {
        append_diagnostic(
            envelope,
            &trace_sink_failure_diagnostic(&failure.error, failure.after_commit),
        );
    }
}

fn append_diagnostic(envelope: &mut Value, entry: &DiagnosticEntryDto) {
    let Some(diagnostics) = envelope
        .as_object_mut()
        .and_then(|object| object.get_mut("diagnostics"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    diagnostics.push(
        serde_json::to_value(entry).expect("diagnostic entry serializes for outcome envelope"),
    );
}

fn validate_dispatch_input(input: &TracedDispatchInput) -> Result<(), DispatchError> {
    if !input.request.is_object() {
        return Err(DispatchError::InvalidRequestShape);
    }
    let encoded = serde_json::to_string(&input.request).expect("request value serializes");
    if encoded.len() > STRUCTURED_CLI_ENVELOPE_BYTES {
        return Err(DispatchError::RequestTooLarge {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: encoded.len(),
        });
    }
    Ok(())
}

fn ensure_envelope_bound(envelope: &Value) -> Result<(), OutcomeRenderError> {
    let rendered = serde_json::to_string(envelope).expect("outcome envelope serializes");
    if rendered.len() > STRUCTURED_CLI_ENVELOPE_BYTES {
        return Err(OutcomeRenderError::EnvelopeTooLarge {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: rendered.len(),
        });
    }
    Ok(())
}

fn trace_path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
        TraceError::Io {
            phase: TraceIoPhase::Write,
            ..
        } => "write",
        TraceError::Io {
            phase: TraceIoPhase::Flush,
            ..
        } => "flush",
        TraceError::Io {
            phase: TraceIoPhase::Fsync,
            ..
        } => "fsync",
        _ => "write",
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
        std::io::ErrorKind::StorageFull => "ENOSPC",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::ReadOnlyFilesystem => "EROFS",
        _ => "EIO",
    }
}
