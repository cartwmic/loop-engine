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
    let transition = checked("explore", "intent-ready", "design");
    let pass = context_json(
        "review-evidence",
        support::evidence(
            "intent",
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

    let mut first_request = base_request(config.clone(), transition.clone());
    first_request["context"] = json!([pass.clone()]);
    let first = invoke(first_request);
    support::assert_exit(&first, 0);
    assert_eq!(response(&first), json!({"result": "allow"}));

    artifacts.write_json("intent.json", &valid_metadata("2"));
    let mut stale_request = base_request(config.clone(), transition.clone());
    stale_request["context"] = json!([pass.clone()]);
    let stale = invoke(stale_request);
    support::assert_exit(&stale, 0);
    let stale_value = response(&stale);
    assert_eq!(
        stale_value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    let axis = stale_value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("axis diagnostic");
    let stale_diagnostic = axis["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .find(|diagnostic| diagnostic["category"] == "stale")
        .expect("stale diagnostic");
    assert_eq!(stale_diagnostic["evidence_revision"], "1");
    assert_eq!(stale_diagnostic["current_revision"], "2");

    let current_pass = context_json(
        "review-evidence",
        support::evidence(
            "intent",
            "axis",
            "pass",
            "",
            "reviewer-a5",
            "agent",
            "intent.json",
            "2",
            "test-1",
        ),
        2,
    );
    let mut current_request = base_request(config, transition);
    current_request["context"] = json!([pass, current_pass]);
    let current = invoke(current_request);
    support::assert_exit(&current, 0);
    assert_eq!(response(&current), json!({"result": "allow"}));
}
