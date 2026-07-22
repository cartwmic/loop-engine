//! Semantic parity tests for human outcome rendering (T126).

#[path = "../src/render/mod.rs"]
mod render;

use std::collections::BTreeMap;

use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::ids::{EventId, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{
    EvidenceRecordedStatus, OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot,
};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::requestable::RequestableEvent;
use loop_engine_core::operations::catalog::{OperationId, PLANNED_OPERATION_IDS};
use render::dto::{OutcomeRenderRequest, STRUCTURED_CLI_ENVELOPE_BYTES};
use render::human::{render_human_outcome, render_human_outcome_bytes};
use render::json::{build_outcome_envelope, render_structured_outcome};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutcomeFacts {
    operation: String,
    outcome: String,
    reason_code: Option<String>,
    reason_message: Option<String>,
    request_id: String,
    trace: String,
    run_id: Option<String>,
    run_label: Option<String>,
    run_lifecycle: Option<String>,
    run_state: Option<String>,
    state_changed: Option<bool>,
    requestable_events: Option<Vec<String>>,
    evidence_recorded: Option<BTreeMap<String, bool>>,
    provider_executed: Option<bool>,
    findings: Vec<(String, String, Option<String>)>,
    gate_verdicts: Vec<(String, String, Option<String>)>,
    diagnostics: Vec<(String, String, Option<String>)>,
}

impl OutcomeFacts {
    fn from_envelope(envelope: &Value) -> Self {
        let data = envelope.get("data").and_then(Value::as_object);
        let run = data
            .and_then(|data| data.get("run"))
            .and_then(Value::as_object);

        let reason = envelope.get("reason");
        let (reason_code, reason_message) = if reason.is_some_and(Value::is_null) {
            (None, None)
        } else {
            (
                reason
                    .and_then(|value| value.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                reason
                    .and_then(|value| value.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        };

        Self {
            operation: envelope["operation"].as_str().unwrap().to_owned(),
            outcome: envelope["outcome"].as_str().unwrap().to_owned(),
            reason_code,
            reason_message,
            request_id: envelope["request_id"].as_str().unwrap().to_owned(),
            trace: envelope["trace"].as_str().unwrap().to_owned(),
            run_id: run
                .and_then(|run| run.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            run_label: run
                .and_then(|run| run.get("label"))
                .and_then(|value| value.as_str().map(str::to_owned)),
            run_lifecycle: run
                .and_then(|run| run.get("lifecycle"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            run_state: run
                .and_then(|run| run.get("state"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            state_changed: run
                .and_then(|run| run.get("state_changed"))
                .and_then(Value::as_bool),
            requestable_events: data
                .and_then(|data| data.get("requestable_events"))
                .map(parse_string_array),
            evidence_recorded: data
                .and_then(|data| data.get("evidence_recorded"))
                .map(parse_bool_map),
            provider_executed: data
                .and_then(|data| data.get("provider_executed"))
                .and_then(Value::as_bool),
            findings: data
                .and_then(|data| data.get("findings"))
                .map(parse_findings)
                .unwrap_or_default(),
            gate_verdicts: data
                .and_then(|data| data.get("gate_verdicts"))
                .map(parse_gate_verdicts)
                .unwrap_or_default(),
            diagnostics: envelope
                .get("diagnostics")
                .map(parse_diagnostics)
                .unwrap_or_default(),
        }
    }

    fn from_human(human: &str) -> Self {
        let mut facts = OutcomeFacts {
            operation: String::new(),
            outcome: String::new(),
            reason_code: None,
            reason_message: None,
            request_id: String::new(),
            trace: String::new(),
            run_id: None,
            run_label: None,
            run_lifecycle: None,
            run_state: None,
            state_changed: None,
            requestable_events: None,
            evidence_recorded: None,
            provider_executed: None,
            findings: Vec::new(),
            gate_verdicts: Vec::new(),
            diagnostics: Vec::new(),
        };

        let mut in_requestable = false;
        let mut in_findings = false;
        let mut in_gate_verdicts = false;
        let mut in_diagnostics = false;

        for line in human.lines() {
            if let Some(value) = line.strip_prefix("Operation: ") {
                facts.operation = value.to_owned();
                continue;
            }
            if let Some(value) = line.strip_prefix("Outcome: ") {
                facts.outcome = value.to_owned();
                continue;
            }
            if let Some(value) = line.strip_prefix("Reason: ") {
                let (code, message) = value
                    .split_once(" — ")
                    .map(|(code, message)| (code.to_owned(), message.to_owned()))
                    .unwrap_or((value.to_owned(), String::new()));
                facts.reason_code = Some(code);
                if !message.is_empty() {
                    facts.reason_message = Some(message);
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("Request ID: ") {
                facts.request_id = value.to_owned();
                continue;
            }
            if let Some(value) = line.strip_prefix("Trace: ") {
                facts.trace = value.to_owned();
                continue;
            }
            if let Some(value) = line.strip_prefix("Run: ") {
                facts.run_id = Some(value.to_owned());
                continue;
            }
            if let Some(value) = line.strip_prefix("Label: ") {
                facts.run_label = Some(value.to_owned());
                continue;
            }
            if let Some(value) = line.strip_prefix("Lifecycle: ") {
                facts.run_lifecycle = Some(value.to_owned());
                continue;
            }
            if let Some(value) = line.strip_prefix("State: ") {
                if let Some(state) = value.strip_suffix(" (unchanged)") {
                    facts.run_state = Some(state.to_owned());
                    facts.state_changed = Some(false);
                } else {
                    facts.run_state = Some(value.to_owned());
                }
                continue;
            }
            if line == "State changed: yes" {
                facts.state_changed = Some(true);
                continue;
            }
            if line == "Requestable events:" {
                in_requestable = true;
                in_findings = false;
                in_gate_verdicts = false;
                in_diagnostics = false;
                facts.requestable_events = Some(Vec::new());
                continue;
            }
            if line == "Findings:" {
                in_findings = true;
                in_requestable = false;
                in_gate_verdicts = false;
                in_diagnostics = false;
                continue;
            }
            if line == "Gate verdicts:" {
                in_gate_verdicts = true;
                in_requestable = false;
                in_findings = false;
                in_diagnostics = false;
                continue;
            }
            if line == "Diagnostics:" {
                in_diagnostics = true;
                in_requestable = false;
                in_findings = false;
                in_gate_verdicts = false;
                continue;
            }
            if let Some(value) = line.strip_prefix("Provider executed: ") {
                facts.provider_executed = Some(value == "yes");
                continue;
            }
            if line == "Evidence recorded: yes" {
                facts.evidence_recorded = Some(evidence_map(true, true, true));
                continue;
            }
            if line == "Evidence recorded: no" {
                facts.evidence_recorded = Some(evidence_map(false, false, false));
                continue;
            }
            if let Some(value) = line.strip_prefix("Submitted inline evidence recorded: ") {
                let mut map = facts
                    .evidence_recorded
                    .take()
                    .unwrap_or_else(|| evidence_map(false, false, false));
                map.insert("inline".into(), value == "yes");
                facts.evidence_recorded = Some(map);
                continue;
            }
            if let Some(value) = line.strip_prefix("Selected evidence associations recorded: ") {
                let mut map = facts
                    .evidence_recorded
                    .take()
                    .unwrap_or_else(|| evidence_map(false, false, false));
                map.insert("selected_associations".into(), value == "yes");
                facts.evidence_recorded = Some(map);
                continue;
            }
            if let Some(value) = line.strip_prefix("Provider evidence recorded: ") {
                let mut map = facts
                    .evidence_recorded
                    .take()
                    .unwrap_or_else(|| evidence_map(false, false, false));
                map.insert("provider".into(), value == "yes");
                facts.evidence_recorded = Some(map);
                continue;
            }

            if in_requestable {
                if let Some(name) = line.strip_prefix("  ") {
                    facts
                        .requestable_events
                        .get_or_insert_with(Vec::new)
                        .push(name.to_owned());
                }
                continue;
            }

            if in_findings {
                if let Some(message) = line.strip_prefix("    ")
                    && let Some(last) = facts.findings.last_mut()
                {
                    last.2 = Some(message.to_owned());
                } else if let Some(rest) = line.strip_prefix("  ")
                    && let Some((key, status)) = rest.split_once(": ")
                {
                    facts
                        .findings
                        .push((key.to_owned(), status.to_owned(), None));
                }
                continue;
            }

            if in_gate_verdicts {
                if let Some(message) = line.strip_prefix("    ")
                    && let Some(last) = facts.gate_verdicts.last_mut()
                {
                    last.2 = Some(message.to_owned());
                } else if let Some(rest) = line.strip_prefix("  ")
                    && let Some((gate, verdict)) = rest.split_once(": ")
                {
                    facts
                        .gate_verdicts
                        .push((gate.to_owned(), verdict.to_owned(), None));
                }
                continue;
            }

            if in_diagnostics {
                if let Some(rest) = line.strip_prefix("- ") {
                    if let Some((code, message)) = rest.split_once(": ") {
                        facts
                            .diagnostics
                            .push((code.to_owned(), message.to_owned(), None));
                    }
                } else if let Some(path) = line.strip_prefix("    path: ")
                    && let Some(last) = facts.diagnostics.last_mut()
                {
                    last.2 = Some(path.to_owned());
                }
            }
        }

        if facts.state_changed.is_none()
            && facts.outcome == "completed"
            && facts.run_state.is_some()
        {
            facts.state_changed = Some(false);
        }

        facts
    }
}

fn evidence_map(inline: bool, selected: bool, provider: bool) -> BTreeMap<String, bool> {
    BTreeMap::from([
        ("inline".into(), inline),
        ("selected_associations".into(), selected),
        ("provider".into(), provider),
    ])
}

fn parse_string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn parse_bool_map(value: &Value) -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            if let Some(flag) = value.as_bool() {
                map.insert(key.clone(), flag);
            }
        }
    }
    map
}

fn parse_findings(value: &Value) -> Vec<(String, String, Option<String>)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|object| {
            let key = ["capability", "key", "id", "name"]
                .into_iter()
                .find_map(|field| object.get(field).and_then(Value::as_str))
                .unwrap_or("")
                .to_owned();
            let status = object
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let message = object
                .get("message")
                .or_else(|| object.get("detail"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            (key, status, message)
        })
        .collect()
}

fn parse_gate_verdicts(value: &Value) -> Vec<(String, String, Option<String>)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|object| {
            let gate = object
                .get("gate")
                .or_else(|| object.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let verdict = object
                .get("verdict")
                .or_else(|| object.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let message = object
                .get("message")
                .or_else(|| object.get("detail"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            (gate, verdict, message)
        })
        .collect()
}

fn parse_diagnostics(value: &Value) -> Vec<(String, String, Option<String>)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|object| {
            let code = object
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let message = object
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let path = object
                .get("context")
                .and_then(Value::as_object)
                .and_then(|context| context.get("path"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            (code, message, path)
        })
        .collect()
}

fn assert_semantic_parity(request: &OutcomeRenderRequest<'_>) {
    let envelope = build_outcome_envelope(request).unwrap();
    let human = render_human_outcome(request).unwrap();
    assert!(human.len() <= STRUCTURED_CLI_ENVELOPE_BYTES);
    assert!(!human.contains("provider.stdout"));
    assert!(!human.contains("trace_schema_version"));

    let structured = OutcomeFacts::from_envelope(&envelope);
    let rendered = OutcomeFacts::from_human(&human);
    assert_eq!(rendered, structured);

    let json_rendered = render_structured_outcome(request).unwrap();
    assert_ne!(human, json_rendered);
}

#[test]
fn contract_completed_run_show_matches_structured_facts() {
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
    assert_semantic_parity(&request);
}

#[test]
fn terminal_run_show_requires_empty_requestable_events() {
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
    assert_semantic_parity(&request);
}

#[test]
fn contract_rejected_run_request_matches_structured_facts() {
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
    let outcome = PublicOutcome::new(
        OutcomeClass::Rejected,
        Some(Reason::new(ReasonCode::GateFailed, "One or more required gates failed").unwrap()),
        OutcomeData::new(
            Some(run),
            Some(requestable),
            Some(EvidenceRecordedStatus {
                inline: true,
                selected_associations: true,
                provider: true,
            }),
        )
        .unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.request").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V3Y",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V3Y.jsonl",
        &outcome,
    )
    .with_operation_data(json!({
        "gate_verdicts": [
            {"gate": "design-is-complete", "verdict": "pass"},
            {
                "gate": "risks-are-addressed",
                "verdict": "fail",
                "message": "Missing rollback strategy."
            }
        ]
    }));
    assert_semantic_parity(&request);
}

#[test]
fn contract_error_run_create_matches_structured_facts() {
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
    assert_semantic_parity(&request);
}

#[test]
fn compatibility_and_provider_execution_shapes_match_structured_facts() {
    let run = RunSnapshot {
        run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
        label: Some("checkout-redesign".into()),
        lifecycle: Lifecycle::Active,
        current_state: StateId::parse("design-review").unwrap(),
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
        OperationId::parse("run.compatibility").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V7D",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V7D.jsonl",
        &outcome,
    )
    .with_operation_data(json!({
        "provider_executed": true,
        "findings": [
            {"capability": "event.approved", "status": "compatible"},
            {
                "capability": "guidance.live",
                "status": "incompatible",
                "message": "stored guidance contract no longer supported"
            }
        ]
    }));
    assert_semantic_parity(&request);
}

#[test]
fn active_graph_rows_match_structured_facts() {
    let outcome = PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(None, None, None).unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("provider.check").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0V7E",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V7E.jsonl",
        &outcome,
    )
    .with_operation_data(json!({
        "items": [
            {
                "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
                "graph_revision": "sha256:44cd...",
                "status": "incompatible",
                "message": "unsupported gate: implementation.has-review"
            }
        ]
    }));
    assert_semantic_parity(&request);

    let human = render_human_outcome(&request).unwrap();
    assert!(human.contains("Active graphs:"));
    assert!(human.contains("sha256:44cd..."));
    assert!(human.contains("unsupported gate: implementation.has-review"));
}

#[test]
fn all_planned_operations_and_outcome_classes_keep_semantic_parity() {
    for operation in PLANNED_OPERATION_IDS {
        for (class, reason) in [
            (OutcomeClass::Completed, None),
            (OutcomeClass::Rejected, Some(ReasonCode::RunNotFound)),
            (OutcomeClass::Error, Some(ReasonCode::PersistenceFailed)),
        ] {
            let reason = reason.map(|code| Reason::new(code, "summary").unwrap());
            let outcome = PublicOutcome::new(
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
                &outcome,
            );
            assert_semantic_parity(&request);
        }
    }
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
    let rendered = render_human_outcome(&request).unwrap();
    let bytes = render_human_outcome_bytes(&request).unwrap();
    assert_eq!(rendered.as_bytes(), bytes.as_slice());
}

#[test]
fn presentation_bound_rejects_oversized_output_without_truncation() {
    let outcome = PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(None, None, None).unwrap(),
        vec![],
    )
    .unwrap();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        "01J9X3K2M4N5P6Q7R8S9T0FFF",
        "/tmp/traces/01J9X3K2M4N5P6Q7R8S9T0FFF.jsonl",
        &outcome,
    )
    .with_operation_data(json!({
        "guidance": "x".repeat(STRUCTURED_CLI_ENVELOPE_BYTES)
    }));
    let err = render_human_outcome(&request).unwrap_err().to_string();
    assert!(err.contains("exceeds"));
}
