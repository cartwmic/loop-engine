use super::support;

use serde_json::json;
use support::{
    base_request, checked, config_artifact_root, context_json, invoke, load_fixture, load_profile,
    response, TestDir,
};

#[test]
fn historical_shipped_example_evidence_is_refused_against_current_frozen_profile() {
    let artifacts = TestDir::new("r25-artifacts");
    artifacts.write_json("intent.json", &load_fixture("intent-good.json"));
    let fixture = load_fixture("example-evidence.json");
    assert_eq!(fixture["kind"], "review-evidence");
    assert!(fixture["data"].is_object());
    let profile = load_profile("standard");
    assert_eq!(fixture["data"]["config_version"], "standard-10");
    assert_ne!(fixture["data"]["config_version"], profile["config_version"]);
    let current_version = profile["config_version"].clone();
    let config = config_artifact_root(profile, &artifacts);

    let mut request = base_request(
        config,
        checked("intent-review", "approved", "intent-adversarial-review"),
    );
    request["context"] = json!([
        context_json(
            fixture["kind"].as_str().expect("fixture kind"),
            fixture["data"].clone(),
            1,
        ),
        context_json(
            "finding-ledger",
            support::finding_ledger("intent-review", "intent.json", "r15", json!([])),
            2,
        )
    ]);
    let output = invoke(request);

    support::assert_exit(&output, 0);
    let output = response(&output);
    assert_eq!(output["result"], "deny");
    assert_eq!(
        output["feedback"]["code"],
        "software-change-review-incomplete"
    );
    let details = &output["feedback"]["details"];
    assert_eq!(details["phase"], "evidence");
    // Original typed diagnostic-array assertion; the historical example remains
    // noncounting input, never rewritten into current-config review evidence.
    assert!(details["diagnostics"].is_array());
    assert!(details["diagnostics"]
        .as_array()
        .expect("current axis diagnostics")
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().expect("axis diagnostics"))
        .any(|diagnostic| diagnostic["category"] == "missing"));
    assert!(details["informational"]
        .as_array()
        .expect("informational axis diagnostics")
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().expect("axis diagnostics"))
        .any(|diagnostic| diagnostic["category"] == "stale_config"
            && diagnostic["evidence_version"] == fixture["data"]["config_version"]
            && diagnostic["run_version"] == current_version));
}
