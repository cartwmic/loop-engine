//! Integration tests for the traced operation dispatcher (T124).

#[path = "../src/diagnostics.rs"]
pub mod diagnostics;

#[path = "../src/render/mod.rs"]
pub mod render;

#[path = "../src/composition.rs"]
pub mod composition;

#[path = "../src/dispatch.rs"]
pub mod dispatch;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use composition::TraceCorrelation;
use dispatch::{
    DispatchError, DispatchTraceFailurePhase, TracedDispatchInput, TracedOperationResult,
    dispatch_traced_operation,
};
use loop_engine_core::model::ids::{EventId, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::requestable::RequestableEvent;
use loop_engine_core::operations::catalog::OperationId;
use loop_engine_integrations::trace::{
    TRACE_INIT_RESERVATION_BYTES, TraceCategory, TraceEvent, TraceWriter,
};
use render::dto::{SCHEMA_VERSION, STRUCTURED_CLI_ENVELOPE_BYTES};
use serde_json::{Value, json};

const REQUEST_ID: &str = "01J9X3K2M4N5P6Q7R8S9T0V1W";

fn read_events(path: &Path) -> Vec<Value> {
    let content = fs::read_to_string(path).expect("trace file");
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonl line"))
        .collect()
}

fn write_invocation_start(writer: &Arc<Mutex<TraceWriter>>, request_id: &str) {
    let mut payload = BTreeMap::new();
    payload.insert("format".into(), json!("json"));
    payload.insert("platform".into(), json!("test-target"));
    payload.insert("argv_digest".into(), json!("deadbeef"));
    payload.insert("argv_byte_length".into(), json!(12));
    let event = TraceEvent::new(request_id, TraceCategory::Invocation, "start", payload);
    writer
        .lock()
        .expect("trace writer")
        .write(&event)
        .expect("invocation.start");
}

fn leave_trace_reservation(writer: &Arc<Mutex<TraceWriter>>, path: &Path, target_remaining: usize) {
    let used = usize::try_from(fs::metadata(path).expect("trace metadata").len())
        .expect("trace length fits usize");
    let available = usize::try_from(TRACE_INIT_RESERVATION_BYTES)
        .expect("reservation fits usize")
        .checked_sub(used)
        .expect("used trace bytes fit reservation");

    let mut locked = writer.lock().expect("trace writer");
    let request_id = locked.request_id().to_owned();
    let mut payload = BTreeMap::new();
    payload.insert("padding".into(), json!(""));
    let template = TraceEvent::new(
        request_id.clone(),
        TraceCategory::Trace,
        "test.reservation.consume",
        payload,
    );
    let framing = serde_json::to_vec(&template)
        .expect("trace event serializes")
        .len()
        + 1;
    let padding_len = available
        .checked_sub(target_remaining + framing)
        .expect("target leaves room for filler event");

    let mut payload = BTreeMap::new();
    payload.insert("padding".into(), json!("x".repeat(padding_len)));
    let event = TraceEvent::new(
        request_id,
        TraceCategory::Trace,
        "test.reservation.consume",
        payload,
    );
    locked.write(&event).expect("consume trace reservation");
}

fn open_trace() -> (tempfile::TempDir, TraceCorrelation, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = TraceWriter::create(dir.path(), REQUEST_ID).expect("trace writer");
    let path = writer.path().to_path_buf();
    let trace = TraceCorrelation::adopt(writer);
    write_invocation_start(&trace.writer(), REQUEST_ID);
    (dir, trace, path)
}

fn sample_completed_outcome() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(
            Some(RunSnapshot {
                run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
                label: None,
                lifecycle: Lifecycle::Active,
                current_state: StateId::parse("explore").unwrap(),
                state_changed: false,
            }),
            Some(vec![RequestableEvent {
                event: EventId::parse("intent-ready").unwrap(),
                target: StateId::parse("explore").unwrap(),
                required_gates: vec![],
            }]),
            None,
        )
        .unwrap(),
        vec![],
    )
    .unwrap()
}

fn sample_rejected_outcome() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Rejected,
        Some(
            Reason::new(
                ReasonCode::InputRejected,
                "run_id is not a valid identifier",
            )
            .unwrap(),
        ),
        OutcomeData::new(None, None, None).unwrap(),
        vec![],
    )
    .unwrap()
}

fn sample_error_outcome() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Error,
        Some(
            Reason::new(
                ReasonCode::ProviderTimeout,
                "Provider process exceeded configured timeout",
            )
            .unwrap(),
        ),
        OutcomeData::new(None, None, None).unwrap(),
        vec![],
    )
    .unwrap()
}

fn invocation_events(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|event| event["category"] == "invocation")
        .collect()
}

fn assert_envelope_fields(envelope: &Value) {
    for field in [
        "schema_version",
        "operation",
        "request_id",
        "trace",
        "outcome",
        "reason",
        "data",
        "diagnostics",
    ] {
        assert!(envelope.get(field).is_some(), "missing `{field}`");
    }
}

#[test]
fn completed_rejected_and_error_outcomes_correlate_with_trace_and_envelope() {
    let cases = [
        (
            OutcomeClass::Completed,
            "completed",
            sample_completed_outcome(),
            OperationId::parse("run.show").unwrap(),
            json!({"run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X"}),
        ),
        (
            OutcomeClass::Rejected,
            "rejected",
            sample_rejected_outcome(),
            OperationId::parse("run.create").unwrap(),
            json!({"graph": "linear"}),
        ),
        (
            OutcomeClass::Error,
            "error",
            sample_error_outcome(),
            OperationId::parse("run.create").unwrap(),
            json!({"graph": "linear"}),
        ),
    ];

    for (expected_class, wire_outcome, outcome, operation, request) in cases {
        let (_dir, trace, path) = open_trace();
        let delivery = dispatch_traced_operation(
            &trace,
            TracedDispatchInput {
                operation,
                request: request.clone(),
                operation_data: json!({}),
            },
            || {
                Ok(TracedOperationResult {
                    outcome: outcome.clone(),
                    after_commit: false,
                })
            },
        )
        .expect("dispatch");

        assert_eq!(delivery.outcome_class(), expected_class);
        assert_eq!(delivery.request_id(), REQUEST_ID);
        assert_eq!(delivery.trace_path(), path.to_string_lossy());

        let envelope = delivery.structured_envelope();
        assert_eq!(envelope["schema_version"], SCHEMA_VERSION);
        assert_eq!(envelope["operation"], operation.as_str());
        assert_eq!(envelope["request_id"], REQUEST_ID);
        assert_eq!(envelope["trace"], delivery.trace_path());
        assert_eq!(envelope["outcome"], wire_outcome);

        let events = read_events(&path);
        let invocation = invocation_events(&events);
        assert_eq!(
            invocation
                .iter()
                .map(|event| event["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["start", "request", "outcome"]
        );

        let request_event = invocation
            .iter()
            .find(|event| event["event"] == "request")
            .expect("invocation.request");
        assert_eq!(request_event["request_id"], REQUEST_ID);
        assert_eq!(request_event["operation"], operation.as_str());
        assert_eq!(request_event["request"], request);

        let outcome_event = invocation
            .iter()
            .find(|event| event["event"] == "outcome")
            .expect("invocation.outcome");
        assert_eq!(outcome_event["request_id"], REQUEST_ID);
        assert_envelope_fields(&outcome_event["envelope"]);
        assert_eq!(outcome_event["envelope"], *envelope);
    }
}

#[test]
fn request_and_full_envelope_each_appear_once_in_trace() {
    let (_dir, trace, path) = open_trace();
    let request = json!({"run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X"});
    let delivery = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: request.clone(),
            operation_data: json!({}),
        },
        || {
            Ok(TracedOperationResult {
                outcome: sample_completed_outcome(),
                after_commit: false,
            })
        },
    )
    .expect("dispatch");

    let events = read_events(&path);
    let request_events: Vec<_> = events
        .iter()
        .filter(|event| event["category"] == "invocation" && event["event"] == "request")
        .collect();
    let outcome_events: Vec<_> = events
        .iter()
        .filter(|event| event["category"] == "invocation" && event["event"] == "outcome")
        .collect();

    assert_eq!(request_events.len(), 1);
    assert_eq!(outcome_events.len(), 1);
    assert_eq!(request_events[0]["request"], request);
    assert_eq!(
        outcome_events[0]["envelope"],
        *delivery.structured_envelope()
    );

    let serialized = serde_json::to_string(&events).expect("trace serializes");
    assert_eq!(serialized.matches("\"envelope\"").count(), 1);
}

#[test]
fn execute_once_canary_rejects_double_dispatch() {
    let (_dir, trace, path) = open_trace();
    let executions = Cell::new(0);
    let delivery = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: json!({"run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X"}),
            operation_data: json!({}),
        },
        || {
            executions.set(executions.get() + 1);
            Ok(TracedOperationResult {
                outcome: sample_completed_outcome(),
                after_commit: false,
            })
        },
    )
    .expect("dispatch");

    assert_eq!(executions.get(), 1);
    assert_eq!(delivery.outcome_class(), OutcomeClass::Completed);
    let events = read_events(&path);
    assert_eq!(
        invocation_events(&events)
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["start", "request", "outcome"]
    );
}

#[test]
fn invalid_request_shape_rejects_before_execute() {
    let (_dir, trace, _path) = open_trace();
    let executions = Cell::new(0);
    let error = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: json!(["not-an-object"]),
            operation_data: json!({}),
        },
        || {
            executions.set(executions.get() + 1);
            Ok(TracedOperationResult {
                outcome: sample_completed_outcome(),
                after_commit: false,
            })
        },
    )
    .unwrap_err();

    assert!(matches!(error, DispatchError::InvalidRequestShape));
    assert_eq!(executions.get(), 0);
}

#[test]
fn oversized_request_rejects_before_execute() {
    let (_dir, trace, _path) = open_trace();
    let executions = Cell::new(0);
    let oversized = json!({
        "payload": "x".repeat(STRUCTURED_CLI_ENVELOPE_BYTES + 1),
    });
    let error = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: oversized,
            operation_data: json!({}),
        },
        || {
            executions.set(executions.get() + 1);
            Ok(TracedOperationResult {
                outcome: sample_completed_outcome(),
                after_commit: false,
            })
        },
    )
    .unwrap_err();

    assert!(matches!(error, DispatchError::RequestTooLarge { .. }));
    assert_eq!(executions.get(), 0);
}

#[test]
fn request_trace_sink_failure_preserves_authoritative_outcome_and_diagnostic() {
    let (_dir, trace, path) = open_trace();
    leave_trace_reservation(&trace.writer(), &path, 64 * 1024);

    let delivery = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: json!({
                "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
                "bounded_input": "x".repeat(128 * 1024),
            }),
            operation_data: json!({}),
        },
        || {
            Ok(TracedOperationResult {
                outcome: sample_completed_outcome(),
                after_commit: false,
            })
        },
    )
    .expect("dispatch");

    assert_eq!(delivery.outcome_class(), OutcomeClass::Completed);
    assert_eq!(delivery.trace_failures().len(), 1);
    assert_eq!(
        delivery.trace_failures()[0].phase,
        DispatchTraceFailurePhase::Request
    );
    assert!(!delivery.trace_failures()[0].after_commit);

    let diagnostics = delivery.structured_envelope()["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "trace.sink_failure");
    assert_eq!(diagnostics[0]["context"]["after_commit"], json!(false));

    let events = read_events(&path);
    assert!(
        events
            .iter()
            .all(|event| event["event"] != "request" || event["category"] != "invocation")
    );
    assert!(
        events
            .iter()
            .any(|event| event["category"] == "trace" && event["event"] == "sink_failure")
    );
}

#[test]
fn outcome_trace_sink_failure_preserves_authoritative_class_and_after_commit() {
    let (_dir, trace, path) = open_trace();
    let writer = trace.writer();

    let delivery = dispatch_traced_operation(
        &trace,
        TracedDispatchInput {
            operation: OperationId::parse("run.show").unwrap(),
            request: json!({"run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X"}),
            operation_data: json!({}),
        },
        || {
            leave_trace_reservation(&writer, &path, 64);
            Ok(TracedOperationResult {
                outcome: sample_error_outcome(),
                after_commit: true,
            })
        },
    )
    .expect("dispatch");

    assert_eq!(delivery.outcome_class(), OutcomeClass::Error);
    assert_eq!(delivery.structured_envelope()["outcome"], "error");
    assert_eq!(delivery.trace_failures().len(), 1);
    assert_eq!(
        delivery.trace_failures()[0].phase,
        DispatchTraceFailurePhase::Outcome
    );
    assert!(delivery.trace_failures()[0].after_commit);

    let diagnostics = delivery.structured_envelope()["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    assert!(
        diagnostics
            .iter()
            .any(|entry| entry["code"] == "trace.sink_failure"
                && entry["context"]["after_commit"] == json!(true))
    );

    let events = read_events(&path);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["category"] == "invocation" && event["event"] == "request")
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .all(|event| event["event"] != "outcome" || event["category"] != "invocation")
    );
}
