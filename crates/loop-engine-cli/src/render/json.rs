use std::collections::BTreeMap;

use loop_engine_core::model::outcome::{OutcomeClass, PublicOutcome};
use serde_json::{Map, Value, json};

use super::dto::{
    OutcomeRenderError, OutcomeRenderRequest, SCHEMA_VERSION, STRUCTURED_CLI_ENVELOPE_BYTES,
    lifecycle_wire, merge_operation_data, outcome_wire, render_diagnostics,
    validate_invocation_fields,
};

/// Builds the structured outcome envelope as a JSON value without serializing it.
pub fn build_outcome_envelope(
    request: &OutcomeRenderRequest<'_>,
) -> Result<Value, OutcomeRenderError> {
    validate_invocation_fields(request.request_id, request.trace_path)?;

    let core_data = build_core_data(request.outcome)?;
    let data = merge_operation_data(core_data, &request.operation_data)?;

    let mut envelope = Map::new();
    envelope.insert("schema_version".into(), json!(SCHEMA_VERSION));
    envelope.insert("operation".into(), json!(request.operation.as_str()));
    envelope.insert("request_id".into(), json!(request.request_id));
    envelope.insert("trace".into(), json!(request.trace_path));
    envelope.insert(
        "outcome".into(),
        json!(outcome_wire(request.outcome.class())),
    );
    envelope.insert("reason".into(), render_reason(request.outcome)?);
    envelope.insert("data".into(), data);
    envelope.insert(
        "diagnostics".into(),
        render_diagnostics(request.outcome.diagnostics()),
    );

    Ok(Value::Object(envelope))
}

/// Renders exactly one canonical UTF-8 JSON object for stdout structured mode.
pub fn render_structured_outcome(
    request: &OutcomeRenderRequest<'_>,
) -> Result<String, OutcomeRenderError> {
    let envelope = build_outcome_envelope(request)?;
    let rendered = canonical_json(&envelope);
    ensure_envelope_bound(&rendered)?;
    Ok(rendered)
}

/// Renders the envelope as UTF-8 bytes after enforcing the stdout byte bound.
pub fn render_structured_outcome_bytes(
    request: &OutcomeRenderRequest<'_>,
) -> Result<Vec<u8>, OutcomeRenderError> {
    let rendered = render_structured_outcome(request)?;
    Ok(rendered.into_bytes())
}

fn build_core_data(outcome: &PublicOutcome) -> Result<Value, OutcomeRenderError> {
    let data = outcome.data();
    let mut object = Map::new();

    if let Some(run) = data.run() {
        object.insert(
            "run".into(),
            json!({
                "id": run.run_id.as_str(),
                "label": run.label,
                "lifecycle": lifecycle_wire(run.lifecycle),
                "state": run.current_state.as_str(),
                "state_changed": run.state_changed,
            }),
        );
    }

    if let Some(events) = data.requestable_events() {
        let names = events
            .iter()
            .map(|event| Value::String(event.event.as_str().to_owned()))
            .collect::<Vec<_>>();
        object.insert("requestable_events".into(), Value::Array(names));
    }

    if let Some(status) = data.evidence_recorded() {
        object.insert(
            "evidence_recorded".into(),
            json!({
                "inline": status.inline,
                "selected_associations": status.selected_associations,
                "provider": status.provider,
            }),
        );
    }

    Ok(Value::Object(object))
}

fn render_reason(outcome: &PublicOutcome) -> Result<Value, OutcomeRenderError> {
    match outcome.class() {
        OutcomeClass::Completed => Ok(Value::Null),
        OutcomeClass::Rejected | OutcomeClass::Error => {
            let reason = outcome
                .reason()
                .expect("core PublicOutcome guarantees reason for non-completed classes");
            Ok(json!({
                "code": reason.code().code(),
                "message": reason.message(),
            }))
        }
    }
}

fn ensure_envelope_bound(rendered: &str) -> Result<(), OutcomeRenderError> {
    if rendered.len() > STRUCTURED_CLI_ENVELOPE_BYTES {
        return Err(OutcomeRenderError::EnvelopeTooLarge {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: rendered.len(),
        });
    }
    Ok(())
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
mod tests {
    use std::path::Path;

    use loop_engine_core::model::diagnostic::Diagnostic;
    use loop_engine_core::model::ids::{EventId, RunId, StateId};
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::model::outcome::{
        EvidenceRecordedStatus, OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot,
    };
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::model::requestable::RequestableEvent;
    use loop_engine_core::operations::catalog::{OperationId, PLANNED_OPERATION_IDS};
    use serde_json::{Value, json};

    use super::{
        OutcomeRenderRequest, SCHEMA_VERSION, STRUCTURED_CLI_ENVELOPE_BYTES,
        build_outcome_envelope, canonical_json, render_structured_outcome,
        render_structured_outcome_bytes,
    };

    fn envelope_has_required_fields(value: &Value) {
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
            assert!(
                value.get(field).is_some(),
                "missing required field `{field}`"
            );
        }
    }

    #[allow(clippy::too_many_arguments, reason = "table-shaped contract fixture")]
    fn assert_contract_example(
        operation: &str,
        request_id: &str,
        trace: &str,
        outcome: OutcomeClass,
        reason_code: Option<ReasonCode>,
        run: Option<RunSnapshot>,
        requestable_events: Option<Vec<RequestableEvent>>,
        evidence_recorded: Option<EvidenceRecordedStatus>,
        diagnostics: Vec<Diagnostic>,
        operation_data: Value,
        expected_outcome: &str,
        expected_reason_code: Option<&str>,
        expected_requestable: Option<&[&str]>,
    ) {
        let outcome_data = OutcomeData::new(run, requestable_events, evidence_recorded).unwrap();
        let reason = reason_code.map(|code| Reason::new(code, "summary").unwrap());
        let public = PublicOutcome::new(outcome, reason, outcome_data, diagnostics).unwrap();
        let request = OutcomeRenderRequest::new(
            OperationId::parse(operation).unwrap(),
            request_id,
            trace,
            &public,
        )
        .with_operation_data(operation_data);

        let rendered = render_structured_outcome(&request).unwrap();
        assert_eq!(
            rendered,
            canonical_json(&build_outcome_envelope(&request).unwrap())
        );
        assert!(rendered.len() <= STRUCTURED_CLI_ENVELOPE_BYTES);

        let value: Value = serde_json::from_str(&rendered).unwrap();
        envelope_has_required_fields(&value);
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["request_id"], request_id);
        assert_eq!(value["trace"], trace);
        assert_eq!(value["outcome"], expected_outcome);
        if let Some(code) = expected_reason_code {
            assert_eq!(value["reason"]["code"], code);
        } else {
            assert!(value["reason"].is_null());
        }
        if let Some(events) = expected_requestable {
            let actual = value["data"]["requestable_events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item.as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(actual, events);
        }
    }

    #[test]
    fn contract_completed_run_show_example_shape() {
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
        assert_contract_example(
            "run.show",
            "01J9X3K2M4N5P6Q7R8S9T0V1W",
            "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl",
            OutcomeClass::Completed,
            None,
            Some(run),
            Some(requestable),
            None,
            vec![],
            json!({}),
            "completed",
            None,
            Some(&["intent-ready"]),
        );
    }

    #[test]
    fn contract_terminal_run_show_requires_empty_requestable_events() {
        let run = RunSnapshot {
            run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
            label: Some("checkout-redesign".into()),
            lifecycle: Lifecycle::Final,
            current_state: StateId::parse("shipped").unwrap(),
            state_changed: false,
        };
        assert_contract_example(
            "run.show",
            "01J9X3K2M4N5P6Q7R8S9T0V6B",
            "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V6B.jsonl",
            OutcomeClass::Completed,
            None,
            Some(run),
            Some(vec![]),
            None,
            vec![],
            json!({}),
            "completed",
            None,
            Some(&[]),
        );
    }

    #[test]
    fn contract_rejected_run_request_example_shape() {
        let run = RunSnapshot {
            run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
            label: Some("checkout-redesign".into()),
            lifecycle: Lifecycle::Active,
            current_state: StateId::parse("design-review").unwrap(),
            state_changed: false,
        };
        let requestable = vec![
            RequestableEvent {
                event: EventId::parse("approved").unwrap(),
                target: StateId::parse("done").unwrap(),
                required_gates: vec![],
            },
            RequestableEvent {
                event: EventId::parse("changes-requested").unwrap(),
                target: StateId::parse("design-review").unwrap(),
                required_gates: vec![],
            },
        ];
        assert_contract_example(
            "run.request",
            "01J9X3K2M4N5P6Q7R8S9T0V3Y",
            "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V3Y.jsonl",
            OutcomeClass::Rejected,
            Some(ReasonCode::GateFailed),
            Some(run),
            Some(requestable),
            Some(EvidenceRecordedStatus {
                inline: true,
                selected_associations: true,
                provider: true,
            }),
            vec![],
            json!({}),
            "rejected",
            Some("gate.failed"),
            Some(&["approved", "changes-requested"]),
        );
    }

    #[test]
    fn contract_error_run_create_example_shape() {
        let diagnostic = Diagnostic::new(
            "provider.invocation",
            "Role describe timed out after 60 seconds",
            Some(r#"{"role":"describe","timeout_seconds":60}"#.into()),
        )
        .unwrap();
        let outcome_data = OutcomeData::new(None, None, None).unwrap();
        let public = PublicOutcome::new(
            OutcomeClass::Error,
            Some(
                Reason::new(
                    ReasonCode::ProviderTimeout,
                    "Provider process exceeded configured timeout",
                )
                .unwrap(),
            ),
            outcome_data,
            vec![diagnostic],
        )
        .unwrap();
        let request = OutcomeRenderRequest::new(
            OperationId::parse("run.create").unwrap(),
            "01J9X3K2M4N5P6Q7R8S9T0V4Z",
            "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V4Z.jsonl",
            &public,
        );
        let rendered = render_structured_outcome(&request).unwrap();
        let value: Value = serde_json::from_str(&rendered).unwrap();
        envelope_has_required_fields(&value);
        assert_eq!(value["outcome"], "error");
        assert_eq!(value["reason"]["code"], "provider.timeout");
        assert_eq!(value["data"], json!({}));
        assert_eq!(
            value["diagnostics"][0]["context"],
            json!({"path": r#"{"role":"describe","timeout_seconds":60}"#})
        );
    }

    #[test]
    fn renderer_emits_one_json_object_without_provider_or_trace_streams() {
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
        .with_operation_data(json!({"items": []}));
        let rendered = render_structured_outcome(&request).unwrap();
        assert!(!rendered.contains('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&rendered)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            8
        );
        assert!(!rendered.contains("provider.stdout"));
        assert!(!rendered.contains("trace_schema_version"));
    }

    #[test]
    fn bytes_renderer_matches_string_renderer() {
        let outcome = PublicOutcome::new(
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunNotFound, "missing").unwrap()),
            OutcomeData::new(None, None, None).unwrap(),
            vec![],
        )
        .unwrap();
        let request = OutcomeRenderRequest::new(
            OperationId::parse("run.show").unwrap(),
            "01J9X3K2M4N5P6Q7R8S9T0BBB",
            "/tmp/traces/01J9X3K2M4N5P6Q7R8S9T0BBB.jsonl",
            &outcome,
        );
        let rendered = render_structured_outcome(&request).unwrap();
        let bytes = render_structured_outcome_bytes(&request).unwrap();
        assert_eq!(rendered.as_bytes(), bytes.as_slice());
    }

    #[test]
    fn published_schema_matches_renderer_markers() {
        let schema_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/cli/v1/outcome.schema.json");
        let schema: Value = serde_json::from_slice(&std::fs::read(schema_path).unwrap()).unwrap();
        assert_eq!(
            schema["properties"]["schema_version"]["const"],
            SCHEMA_VERSION
        );
        assert_eq!(
            schema["properties"]["operation"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            PLANNED_OPERATION_IDS.len()
        );
        assert_eq!(
            schema["properties"]["outcome"]["enum"],
            json!(["completed", "rejected", "error"])
        );
        assert_eq!(
            schema["x-loop-engine-bound-markers"]["envelope"],
            "structured_cli_envelope_bytes"
        );
    }

    #[test]
    fn diagnostic_path_string_maps_to_context_path_field() {
        let diagnostic = Diagnostic::new("bad", "message", Some("not-json-object".into())).unwrap();
        let public = PublicOutcome::new(
            OutcomeClass::Error,
            Some(Reason::new(ReasonCode::PersistenceFailed, "failed").unwrap()),
            OutcomeData::new(None, None, None).unwrap(),
            vec![diagnostic],
        )
        .unwrap();
        let request = OutcomeRenderRequest::new(
            OperationId::parse("run.export").unwrap(),
            "01J9X3K2M4N5P6Q7R8S9T0DDD",
            "/tmp/traces/01J9X3K2M4N5P6Q7R8S9T0DDD.jsonl",
            &public,
        );
        let value: Value =
            serde_json::from_str(&render_structured_outcome(&request).unwrap()).unwrap();
        assert_eq!(
            value["diagnostics"][0]["context"],
            json!({"path": "not-json-object"})
        );
    }

    #[test]
    fn all_planned_operations_and_outcome_classes_keep_required_fields() {
        for operation in PLANNED_OPERATION_IDS {
            for (class, reason) in [
                (OutcomeClass::Completed, None),
                (OutcomeClass::Rejected, Some(ReasonCode::RunNotFound)),
                (OutcomeClass::Error, Some(ReasonCode::PersistenceFailed)),
            ] {
                let reason = reason.map(|code| Reason::new(code, "summary").unwrap());
                let public = PublicOutcome::new(
                    class,
                    reason,
                    OutcomeData::new(None, None, None).unwrap(),
                    vec![],
                )
                .unwrap();
                let request = OutcomeRenderRequest::new(
                    OperationId::parse(operation).unwrap(),
                    "01J9X3K2M4N5P6Q7R8S9T0CCC",
                    "/tmp/traces/01J9X3K2M4N5P6Q7R8S9T0CCC.jsonl",
                    &public,
                );
                let value = build_outcome_envelope(&request).unwrap();
                envelope_has_required_fields(&value);
                assert_eq!(value["operation"], *operation);
            }
        }
    }
}
