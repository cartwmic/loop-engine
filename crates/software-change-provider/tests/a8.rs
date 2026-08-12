#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use support::{axis_config, valid_metadata, Engine, TestDir};

#[test]
fn engine_show_projects_allow_over_prior_deny_for_same_transition() {
    let artifacts = TestDir::new("a8-artifacts");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let state = TestDir::new("a8-state");
    let engine = Engine::new(state.path().join("a8.sqlite"));
    engine.start_ok("a8", axis_config(&artifacts, "axis"));

    let denied = engine.event("a8", "intent-ready");
    let denied_issue = match denied {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected first evidence denial, got {other:?}"),
    };
    assert_eq!(denied_issue.code, "software-change-review-incomplete");
    let first_show = engine.show("a8");
    assert_eq!(first_show.latest_evaluations.len(), 1);
    assert!(first_show.latest_evaluations[0].is_deny());

    engine.append_evidence(
        "a8",
        "a8-pass",
        "intent",
        "axis",
        "pass",
        "",
        "reviewer-a8",
        "agent",
        "intent.json",
        "1",
        "test-1",
    );
    assert!(matches!(
        engine.event("a8", "intent-ready"),
        OperationOutcome::Completed(_)
    ));

    let final_show = engine.show("a8");
    assert_eq!(final_show.latest_evaluations.len(), 1);
    assert!(final_show.latest_evaluations[0].is_allow());
    assert_eq!(final_show.current_state.as_str(), "design");
}
