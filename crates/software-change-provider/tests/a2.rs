#[path = "support/mod.rs"]
mod support;

use loop_core::{Lifecycle, OperationOutcome};
use serde_json::json;
use support::{Engine, TestDir};

#[test]
fn missing_policies_errors_on_first_check_and_leaves_engine_state_unchanged() {
    let state = TestDir::new("a2-missing-state");
    let engine = Engine::new(state.path().join("missing.sqlite"));
    engine.start_ok("missing-policies", json!({"config_version": "standard-1"}));

    let outcome = engine.event("missing-policies", "intent-ready");
    let issue = match outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected evaluation error, got {other:?}"),
    };
    assert!(issue.message.contains("minimal"));
    assert!(issue.message.contains("standard"));
    assert!(issue.message.contains("high-rigor"));

    let run = engine.authoritative("missing-policies");
    assert_eq!(run.current_state.as_str(), "explore");
    assert_eq!(run.last_sequence.as_u64(), 1);
    assert!(engine
        .show("missing-policies")
        .latest_evaluations
        .is_empty());
}

#[test]
fn explicitly_empty_policies_walk_to_end_without_artifact_root() {
    let state = TestDir::new("a2-empty-state");
    let engine = Engine::new(state.path().join("empty.sqlite"));
    let input = json!({"config_version": "none", "review_policies": {}});
    engine.start_ok("empty-policies", input.clone());

    let shown = engine.show("empty-policies");
    assert_eq!(shown.initial_input, input);
    assert!(shown.initial_input.get("artifact_root").is_none());
    assert!(shown.initial_input.get("artifact_schemas").is_none());
    assert!(shown
        .initial_input
        .get("review_policies")
        .and_then(|value| value.as_object())
        .is_some_and(|policies| policies.is_empty()));

    for event in [
        "intent-ready",
        "design-ready",
        "approved",
        "plan-ready",
        "approved",
        "implementation-ready",
        "approved",
        "passed",
    ] {
        let result = engine.event("empty-policies", event);
        assert!(
            matches!(result, OperationOutcome::Completed(_)),
            "{event}: {result:?}"
        );
    }

    assert_eq!(engine.current_state("empty-policies").as_str(), "end");
    assert_eq!(engine.lifecycle("empty-policies"), Lifecycle::Final);
}
