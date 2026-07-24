//! Integration tests for structured outcome rendering and published schema parity (T125).

#[path = "../src/render/dto.rs"]
mod dto;
#[path = "../src/render/json.rs"]
mod json;

use std::collections::BTreeSet;
use std::path::Path;

use dto::{OutcomeRenderRequest, SCHEMA_VERSION, STRUCTURED_CLI_ENVELOPE_BYTES};
use json::{build_outcome_envelope, render_structured_outcome};
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::ids::{EventId, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::requestable::RequestableEvent;
use loop_engine_core::operations::catalog::{OperationId, PLANNED_OPERATION_IDS};
use serde_json::Value;

fn schema_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/cli/v1/outcome.schema.json")
}

fn load_schema() -> Value {
    serde_json::from_slice(&std::fs::read(schema_path()).unwrap()).unwrap()
}

#[test]
fn published_schema_enums_match_core_catalog_and_taxonomy() {
    let schema = load_schema();
    let schema_ops: Vec<&str> = schema["properties"]["operation"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(schema_ops, PLANNED_OPERATION_IDS);

    let schema_codes: Vec<&str> = schema["$defs"]["ReasonCode"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let mut core_codes: Vec<&str> = ReasonCode::ALL.iter().map(|code| code.code()).collect();
    core_codes.sort_unstable();
    let mut sorted_schema_codes = schema_codes.clone();
    sorted_schema_codes.sort_unstable();
    assert_eq!(sorted_schema_codes, core_codes);
}

#[test]
fn published_schema_reason_classes_partition_taxonomy() {
    let schema = load_schema();
    let all_codes = schema["$defs"]["ReasonCode"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let class_codes = |index: usize| {
        schema["allOf"][index]["then"]["properties"]["reason"]["allOf"][1]["properties"]
            ["code"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>()
    };
    let rejected = class_codes(1);
    let error = class_codes(2);

    assert!(rejected.is_disjoint(&error));
    assert_eq!(
        rejected.union(&error).copied().collect::<BTreeSet<_>>(),
        all_codes
    );
    assert!(rejected.contains("gate.failed"));
    assert!(!error.contains("gate.failed"));
    assert!(error.contains("provider.evaluation_error"));
    assert!(!rejected.contains("provider.evaluation_error"));
}

#[test]
fn contract_examples_render_with_eight_required_top_level_fields() {
    let run = RunSnapshot {
        run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
        label: Some("checkout-redesign".into()),
        lifecycle: Lifecycle::Active,
        current_state: StateId::parse("explore").unwrap(),
        state_changed: false,
    };
    let requestable = vec![RequestableEvent {
        event: EventId::parse("intent-ready").unwrap(),
        target: StateId::parse("explore").unwrap(),
        required_gates: vec![],
    }];
    let outcome = PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(Some(run), Some(requestable), None).unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V1W",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl",
        &outcome,
    );
    let rendered = render_structured_outcome(&request).unwrap();
    assert!(!rendered.contains('\n'));
    assert!(rendered.len() <= STRUCTURED_CLI_ENVELOPE_BYTES);

    let value: Value = serde_json::from_str(&rendered).unwrap();
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
        assert!(value.get(field).is_some(), "missing `{field}`");
    }
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert_eq!(value["outcome"], "completed");
    assert!(value["reason"].is_null());
    assert_eq!(
        value["data"]["requestable_events"],
        serde_json::json!(["intent-ready"])
    );
}

#[test]
fn core_diagnostic_path_renders_as_context_path_not_parsed_json() {
    let diagnostic = Diagnostic::new(
        "provider.invocation",
        "Role describe timed out after 60 seconds",
        Some(r#"{"role":"describe","timeout_seconds":60}"#.into()),
    )
    .unwrap();
    let outcome = PublicOutcome::new(
        OutcomeClass::Error,
        Some(
            Reason::new(
                ReasonCode::ProviderTimeout,
                "Provider process exceeded configured timeout",
            )
            .unwrap(),
        ),
        OutcomeData::new(None, None, None).unwrap(),
        vec![diagnostic],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.create").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V4Z",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V4Z.jsonl",
        &outcome,
    );
    let value = build_outcome_envelope(&request).unwrap();
    assert_eq!(
        value["diagnostics"][0]["context"],
        serde_json::json!({"path": r#"{"role":"describe","timeout_seconds":60}"#})
    );
}

#[test]
fn terminal_run_requires_empty_requestable_events_in_renderer_output() {
    let run = RunSnapshot {
        run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
        label: Some("checkout-redesign".into()),
        lifecycle: Lifecycle::Final,
        current_state: StateId::parse("shipped").unwrap(),
        state_changed: false,
    };
    let outcome = PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(Some(run), Some(vec![]), None).unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V6B",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V6B.jsonl",
        &outcome,
    );
    let value: Value = serde_json::from_str(&render_structured_outcome(&request).unwrap()).unwrap();
    assert_eq!(value["data"]["run"]["lifecycle"], "final");
    assert_eq!(value["data"]["requestable_events"], serde_json::json!([]));
}

#[test]
fn renderer_output_excludes_provider_and_trace_stream_fields() {
    let outcome = PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(None, None, None).unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("provider.list").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0AAA",
        "/tmp/traces/01J9X3K2M4N5P6Q7R8S9T0AAA.jsonl",
        &outcome,
    )
    .with_operation_data(serde_json::json!({"items": []}));
    let rendered = render_structured_outcome(&request).unwrap();
    assert!(!rendered.contains("provider.stdout"));
    assert!(!rendered.contains("trace_schema_version"));
}
