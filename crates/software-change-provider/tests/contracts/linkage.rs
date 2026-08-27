use super::support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{axis_config, base_request, checked, context_json, invoke, response, TestDir};

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn ledger() -> Value {
    json!({
        "schema_version": "1",
        "gate": "intent-review",
        "subject": "intent.json",
        "subject_revision": "1",
        "author": {"name": "driver", "kind": "agent"},
        "repository_state": null,
        "findings": []
    })
}

fn linked_evidence(root: &TestDir, result: &str, findings: &str) -> Value {
    let bytes = serde_json::to_vec(&json!({
        "axis": "axis",
        "author": {"name": "reviewer", "kind": "agent"},
        "result": result,
        "findings": findings
    }))
    .unwrap();
    root.write_text("review.stdout", std::str::from_utf8(&bytes).unwrap());
    let data = json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": result,
        "findings": findings,
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1",
        "originating_output": {
            "invocation_id": "invocation-1",
            "assignment_id": "worker-0",
            "selected_attempt": 1,
            "sha256": digest(&bytes),
            "path": root.path().join("review.stdout").to_string_lossy()
        }
    });
    data
}

fn evaluate(root: &TestDir, evidence: Value) -> Value {
    let mut request = base_request(
        axis_config(root, "axis"),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        context_json("review-evidence", evidence, 1),
        context_json("finding-ledger", ledger(), 2)
    ]);
    let output = invoke(request);
    support::assert_exit(&output, 0);
    response(&output)
}

#[test]
fn linked_judgment_must_agree_with_originating_bytes() {
    let root = TestDir::new("linkage-agreement");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let matching = evaluate(&root, linked_evidence(&root, "pass", ""));
    assert_eq!(matching, json!({"result": "allow"}));

    let mut opposite = linked_evidence(&root, "pass", "");
    opposite["result"] = json!("fail");
    opposite["findings"] = json!("opposite claim");
    let refused = evaluate(&root, opposite);
    assert_eq!(
        refused["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));
}

#[test]
fn linked_judgment_fails_unverified_when_origin_bytes_change_or_disappear() {
    let root = TestDir::new("linkage-unverified");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let evidence = linked_evidence(&root, "pass", "");
    root.write_text("review.stdout", "changed");
    let refused = evaluate(&root, evidence.clone());
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));

    let evidence = linked_evidence(&root, "pass", "");
    std::fs::remove_file(root.path().join("review.stdout")).unwrap();
    let refused = evaluate(&root, evidence);
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));
}
