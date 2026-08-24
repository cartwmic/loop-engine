#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use serde_json::json;
use support::{axis_config, valid_metadata, Engine, TestDir};

#[test]
fn engine_show_projects_allow_over_prior_deny_for_same_transition() {
    let artifacts = TestDir::new("a8-artifacts");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let state = TestDir::new("a8-state");
    let engine = Engine::new(state.path().join("a8.sqlite"));
    engine.start_ok("a8", axis_config(&artifacts, "axis"));

    let ready = engine.event("a8", "intent-ready");
    assert!(
        matches!(ready, OperationOutcome::Completed(_)),
        "draft ready is schema-only, got {ready:?}"
    );
    assert_eq!(engine.current_state("a8").as_str(), "intent-review");

    let denied = engine.event("a8", "approved");
    let denied_issue = match denied {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected first evidence denial, got {other:?}"),
    };
    assert_eq!(denied_issue.code, "software-change-finding-ledger-invalid");
    let first_show = engine.show("a8");
    assert!(first_show
        .latest_evaluations
        .iter()
        .any(|evaluation| evaluation.is_deny()));

    engine.append_evidence(
        "a8",
        "a8-pass",
        "intent-review",
        "axis",
        "pass",
        "",
        "reviewer-a8",
        "agent",
        "intent.json",
        "1",
        "test-1",
    );
    engine.append_finding_ledger(
        "a8",
        "a8-accepted",
        "intent-review",
        "intent.json",
        "1",
        json!([]),
    );
    assert!(matches!(
        engine.event("a8", "approved"),
        OperationOutcome::Completed(_)
    ));

    let final_show = engine.show("a8");
    assert!(final_show
        .latest_evaluations
        .iter()
        .any(|evaluation| evaluation.is_allow()));
    assert_eq!(final_show.current_state.as_str(), "design");
}
