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
    assert!(fixture["data"].is_object());
    let config = config_artifact_root(load_profile("standard"), &artifacts);

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
    assert!(output["feedback"]["details"]["diagnostics"].is_array());
}
