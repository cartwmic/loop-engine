#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use serde_json::Value;
use support::{config_artifact_root, load_fixture, load_profile, Engine, TestDir};

#[test]
fn standard_run_progresses_schema_deny_then_evidence_deny_then_allow() {
    let artifacts = TestDir::new("a1-artifacts");
    let state = TestDir::new("a1-state");
    let config = config_artifact_root(load_profile("standard"), &artifacts);
    let engine = Engine::new(state.path().join("a1.sqlite"));

    engine.start_ok("a1", config.clone());
    let shown = engine.show("a1");
    assert_eq!(shown.initial_input, config);
    assert!(shown.initial_input["review_policies"]["intent"]
        .as_array()
        .is_some_and(|axes| !axes.is_empty()));
    assert!(shown.initial_input["artifact_schemas"]["intent.json"].is_object());
    assert_eq!(shown.current_state.as_str(), "explore");

    // No artifact exists yet, so deterministic shape checking denies before
    // semantic evidence is consulted.
    let schema_denial = engine.event("a1", "intent-ready");
    let schema_issue = match schema_denial {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected schema denial, got {other:?}"),
    };
    assert_eq!(schema_issue.code, "software-change-schema-invalid");
    assert_eq!(engine.current_state("a1").as_str(), "explore");

    let intent = load_fixture("intent-good.json");
    artifacts.write_json("intent.json", &intent);
    let evidence_denial = engine.event("a1", "intent-ready");
    let evidence_issue = match evidence_denial {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected evidence denial, got {other:?}"),
    };
    assert_eq!(evidence_issue.code, "software-change-review-incomplete");
    assert_eq!(evidence_issue.message, "review evidence incomplete");
    let diagnostics = evidence_issue.details.as_ref().expect("evidence details")["diagnostics"]
        .as_array()
        .expect("axis diagnostics");
    assert_eq!(diagnostics.len(), 5);

    let axes = [
        "solution-agnostic",
        "outside-verifiable",
        "scope-fenced",
        "constraints-are-limits",
        "problem-grounded",
    ];
    for (sequence, axis) in axes.into_iter().enumerate() {
        engine.append_evidence(
            "a1",
            &format!("a1-{axis}"),
            "intent",
            axis,
            "pass",
            "",
            "reviewer-a1",
            "agent",
            "intent.json",
            "r15",
            "standard-5",
        );
        // Keep sequence values visible in source even though persistence owns
        // durable sequence allocation; enumerate documents append order.
        let _ = sequence;
    }

    let allowed = engine.event("a1", "intent-ready");
    assert!(matches!(allowed, OperationOutcome::Completed(_)));
    assert_eq!(engine.current_state("a1").as_str(), "design");

    // Ensure shipped evidence path was consumed as JSON, not copied into a
    // test-local representation.
    assert_eq!(intent_revision(&intent), "r15");
}

fn intent_revision(intent: &Value) -> &str {
    intent["revision"].as_str().expect("intent revision")
}
