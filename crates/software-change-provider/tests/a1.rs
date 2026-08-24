#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use serde_json::{json, Value};
use support::{config_artifact_root, load_fixture, load_profile, Engine, TestDir};

#[test]
fn standard_run_progresses_schema_deny_then_evidence_deny_then_allow() {
    let artifacts = TestDir::new("a1-artifacts");
    let state = TestDir::new("a1-state");
    let config = config_artifact_root(load_profile("standard"), &artifacts);
    let engine = Engine::new(state.path().join("a1.sqlite"));
    let config_version = config["config_version"]
        .as_str()
        .expect("standard config_version")
        .to_owned();

    engine.start_ok("a1", config.clone());
    let shown = engine.show("a1");
    assert_eq!(shown.initial_input, config);
    assert!(shown.initial_input["review_policies"]["intent-review"]
        .as_array()
        .is_some_and(|axes| !axes.is_empty()));
    assert!(shown.initial_input["artifact_schemas"]["intent.json"].is_object());
    assert_eq!(shown.current_state.as_str(), "explore");

    // Draft ready is schema and links only; missing artifact denies before
    // review evidence or finding-ledger records are consulted.
    let schema_denial = engine.event("a1", "intent-ready");
    let schema_issue = match schema_denial {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected schema denial, got {other:?}"),
    };
    assert_eq!(schema_issue.code, "software-change-schema-invalid");
    assert_eq!(engine.current_state("a1").as_str(), "explore");

    let intent = load_fixture("intent-good.json");
    artifacts.write_json("intent.json", &intent);
    let ready = engine.event("a1", "intent-ready");
    assert!(
        matches!(ready, OperationOutcome::Completed(_)),
        "expected draft ready to allow after schema pass, got {ready:?}"
    );
    assert_eq!(engine.current_state("a1").as_str(), "intent-review");

    let evidence_denial = engine.event("a1", "approved");
    let evidence_issue = match evidence_denial {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected evidence denial, got {other:?}"),
    };
    assert_eq!(
        evidence_issue.code,
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(
        evidence_issue.message,
        "finding ledger missing or malformed"
    );
    assert_eq!(
        evidence_issue.details.as_ref().expect("ledger details")["status"],
        "missing"
    );

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
            "intent-review",
            axis,
            "pass",
            "",
            "reviewer-a1",
            "agent",
            "intent.json",
            "r15",
            &config_version,
        );
        let _ = sequence;
    }

    let findings_denial = engine.event("a1", "approved");
    let findings_issue = match findings_denial {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected finding-ledger denial, got {other:?}"),
    };
    assert_eq!(
        findings_issue.code,
        "software-change-finding-ledger-invalid"
    );

    engine.append_finding_ledger(
        "a1",
        "a1-accepted",
        "intent-review",
        "intent.json",
        "r15",
        json!([]),
    );

    let allowed = engine.event("a1", "approved");
    assert!(matches!(allowed, OperationOutcome::Completed(_)));
    assert_eq!(
        engine.current_state("a1").as_str(),
        "intent-adversarial-review"
    );

    // Ensure shipped evidence path was consumed as JSON, not copied into a
    // test-local representation.
    assert_eq!(intent_revision(&intent), "r15");
}

fn intent_revision(intent: &Value) -> &str {
    intent["revision"].as_str().expect("intent revision")
}
