#[path = "support/mod.rs"]
mod support;

use serde_json::json;
use support::{
    axis_config, base_request, checked, context_json, invoke, response, valid_metadata, TestDir,
};

#[test]
fn revision_bump_retires_old_pass_until_current_revision_is_reviewed() {
    let artifacts = TestDir::new("a5-artifacts");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let config = axis_config(&artifacts, "axis");
    let transition = checked("intent-review", "approved", "design");
    let pass = context_json(
        "review-evidence",
        support::evidence(
            "intent-review",
            "axis",
            "pass",
            "",
            "reviewer-a5",
            "agent",
            "intent.json",
            "1",
            "test-1",
        ),
        1,
    );
    let accepted_v1 = context_json(
        "accepted-findings",
        support::accepted_findings("intent-review", "intent.json", "1", json!([])),
        2,
    );

    let mut first_request = base_request(config.clone(), transition.clone());
    first_request["context"] = json!([pass.clone(), accepted_v1.clone()]);
    let first = invoke(first_request);
    support::assert_exit(&first, 0);
    assert_eq!(response(&first), json!({"result": "allow"}));

    artifacts.write_json("intent.json", &valid_metadata("2"));
    let mut stale_request = base_request(config.clone(), transition.clone());
    stale_request["context"] = json!([pass.clone(), accepted_v1.clone()]);
    let stale = invoke(stale_request);
    support::assert_exit(&stale, 0);
    let stale_value = response(&stale);
    assert_eq!(
        stale_value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    let blocking_axis = stale_value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("blocking axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("blocking axis diagnostic");
    assert!(blocking_axis["diagnostics"]
        .as_array()
        .expect("blocking diagnostics")
        .iter()
        .all(|diagnostic| diagnostic["category"] != "stale"));
    let informational_axis = stale_value["feedback"]["details"]["informational"]
        .as_array()
        .expect("informational axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("informational axis diagnostic");
    let stale_diagnostic = informational_axis["diagnostics"]
        .as_array()
        .expect("informational diagnostics")
        .iter()
        .find(|diagnostic| diagnostic["category"] == "stale")
        .expect("stale diagnostic");
    assert_eq!(stale_diagnostic["evidence_revision"], "1");
    assert_eq!(stale_diagnostic["current_revision"], "2");

    let current_pass = context_json(
        "review-evidence",
        support::evidence(
            "intent-review",
            "axis",
            "pass",
            "",
            "reviewer-a5",
            "agent",
            "intent.json",
            "2",
            "test-1",
        ),
        3,
    );
    let accepted_v2 = context_json(
        "accepted-findings",
        support::accepted_findings("intent-review", "intent.json", "2", json!([])),
        4,
    );
    let mut current_request = base_request(config, transition);
    current_request["context"] = json!([pass, accepted_v1, current_pass, accepted_v2]);
    let current = invoke(current_request);
    support::assert_exit(&current, 0);
    assert_eq!(response(&current), json!({"result": "allow"}));
}
