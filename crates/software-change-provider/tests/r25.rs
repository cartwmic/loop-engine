#[path = "support/mod.rs"]
mod support;

use serde_json::json;
use support::{
    base_request, checked, config_artifact_root, context_json, invoke, load_fixture, load_profile,
    response, TestDir,
};

#[test]
fn shipped_example_evidence_record_is_consumed_by_production_evaluation_parser() {
    let artifacts = TestDir::new("r25-artifacts");
    artifacts.write_json("intent.json", &load_fixture("intent-good.json"));
    let fixture = load_fixture("example-evidence.json");
    assert_eq!(fixture["kind"], "review-evidence");
    let evidence = &fixture["data"];
    let gate = evidence["gate"].as_str().expect("fixture gate");
    assert_eq!(gate, "intent");
    let policy_id = evidence["policy_id"].as_str().expect("fixture policy id");
    let config_version = evidence["config_version"]
        .as_str()
        .expect("fixture config version");
    let config = config_artifact_root(load_profile("standard"), &artifacts);
    assert_eq!(config["config_version"], config_version);
    assert!(config["review_policies"][gate]
        .as_array()
        .expect("shipped gate axes")
        .iter()
        .any(|axis| axis["id"] == policy_id));

    let mut request = base_request(config, checked("explore", "intent-ready", "design"));
    request["context"] = json!([context_json(
        fixture["kind"].as_str().expect("fixture kind"),
        fixture["data"].clone(),
        1,
    )]);
    let output = invoke(request);

    support::assert_exit(&output, 0);
    let output = response(&output);
    assert_eq!(output["result"], "deny");
    let diagnostics = output["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("axis diagnostics");
    assert!(diagnostics.iter().all(|axis| axis["axis"] != policy_id));
}
