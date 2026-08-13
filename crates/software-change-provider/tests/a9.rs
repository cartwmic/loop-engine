#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use serde_json::json;
use support::{axis_config, valid_metadata, Engine, TestDir};

#[test]
fn engine_distinguishes_inaccessible_root_provider_failure_from_review_deny_and_missing_binary() {
    let state = TestDir::new("a9-state");

    let inaccessible_artifacts = TestDir::new("a9-inaccessible-artifacts");
    let mut inaccessible_config = axis_config(&inaccessible_artifacts, "axis");
    inaccessible_config["artifact_root"] = json!(state
        .path()
        .join("missing-artifact-root")
        .to_string_lossy()
        .to_string());
    let inaccessible = Engine::new(state.path().join("inaccessible.sqlite"));
    inaccessible.start_ok("inaccessible", inaccessible_config);
    let inaccessible_outcome = inaccessible.event("inaccessible", "intent-ready");
    let inaccessible_issue = match inaccessible_outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected inaccessible-root error, got {other:?}"),
    };
    assert_eq!(inaccessible_issue.code, "provider-execution-failed");
    assert!(inaccessible_issue
        .message
        .contains("provider-exited-nonzero"));
    assert!(inaccessible_issue.message.contains("artifact_root"));
    assert_eq!(
        inaccessible.current_state("inaccessible").as_str(),
        "explore"
    );
    assert!(inaccessible
        .show("inaccessible")
        .latest_evaluations
        .is_empty());

    let review_artifacts = TestDir::new("a9-review-artifacts");
    review_artifacts.write_json("intent.json", &valid_metadata("1"));
    let review = Engine::new(state.path().join("review.sqlite"));
    review.start_ok("review-deny", axis_config(&review_artifacts, "axis"));
    let review_outcome = review.event("review-deny", "intent-ready");
    let review_issue = match review_outcome {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected review denial, got {other:?}"),
    };
    assert_eq!(review_issue.code, "software-change-review-incomplete");
    assert_eq!(review.current_state("review-deny").as_str(), "explore");
    assert_eq!(review.show("review-deny").latest_evaluations.len(), 1);

    let missing_binary = state.path().join("provider-does-not-exist");
    let unavailable = Engine::with_command(state.path().join("missing.sqlite"), missing_binary);
    let unavailable_outcome = unavailable.start(
        "missing-binary",
        json!({
            "config_version": "none",
            "review_policies": {}
        }),
    );
    let unavailable_issue = match unavailable_outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected missing-provider error, got {other:?}"),
    };
    assert_eq!(unavailable_issue.code, "provider-execution-failed");
    assert!(unavailable_issue.message.contains("provider-spawn-failed"));
    assert_ne!(
        inaccessible_issue.message, unavailable_issue.message,
        "engine must preserve distinct provider incapacity diagnostics"
    );
}

#[test]
fn engine_classifies_stale_config_evidence_as_review_denial() {
    let state = TestDir::new("a9-stale-config-state");
    let artifacts = TestDir::new("a9-stale-config-artifacts");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let engine = Engine::new(state.path().join("stale-config.sqlite"));
    engine.start_ok("stale-config", axis_config(&artifacts, "axis"));

    engine.append_evidence(
        "stale-config",
        "stale-config-evidence",
        "intent",
        "axis",
        "pass",
        "",
        "reviewer-a9",
        "agent",
        "intent.json",
        "1",
        "stale-test-version",
    );

    let outcome = engine.event("stale-config", "intent-ready");
    let issue = match outcome {
        OperationOutcome::Rejected(issue) => issue,
        OperationOutcome::Error(issue) => {
            panic!("expected stale config policy denial, got evaluation error {issue:?}")
        }
        other => panic!("expected stale config policy denial, got {other:?}"),
    };
    assert_eq!(issue.code, "software-change-review-incomplete");
    let details = issue.details.as_ref().expect("evidence denial details");
    assert_eq!(details["phase"], "evidence");
    let axis = details["diagnostics"]
        .as_array()
        .expect("axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("axis diagnostic");
    assert!(axis["diagnostics"]
        .as_array()
        .expect("blocking diagnostics")
        .iter()
        .all(|diagnostic| diagnostic["category"] != "stale_config"));
    let informational_axis = details["informational"]
        .as_array()
        .expect("informational axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("informational axis diagnostic");
    let stale_config = informational_axis["diagnostics"]
        .as_array()
        .expect("informational diagnostics")
        .iter()
        .find(|diagnostic| diagnostic["category"] == "stale_config")
        .expect("stale config diagnostic");
    assert_eq!(stale_config["evidence_version"], "stale-test-version");
    assert_eq!(stale_config["run_version"], "test-1");
    assert_eq!(engine.current_state("stale-config").as_str(), "explore");
    let shown = engine.show("stale-config");
    assert_eq!(shown.latest_evaluations.len(), 1);
    assert!(shown.latest_evaluations[0].is_deny());
}
