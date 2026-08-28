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
        "findings": []
    })
}

fn selected_path(root: &TestDir) -> std::path::PathBuf {
    root.path()
        .join("selected-capture/worker-0/attempts/1/stdout")
}

fn linked_evidence(root: &TestDir, result: &str, findings: &str) -> Value {
    let capture = root.path().join("selected-capture");
    let selected = selected_path(root);
    std::fs::create_dir_all(selected.parent().unwrap()).unwrap();
    let bytes = serde_json::to_vec(&json!({
        "axis": "axis",
        "author": {"name": "reviewer", "kind": "agent"},
        "result": result,
        "findings": findings
    }))
    .unwrap();
    std::fs::write(&selected, &bytes).unwrap();
    json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": result,
        "findings": findings,
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1",
        "origin": {
            "kind": "selected-assignment-output",
            "id": "invocation-1",
            "assignment_id": "worker-0"
        },
        "loop_engine_origin": {
            "invocation_id": "invocation-1",
            "assignment_id": "worker-0",
            "selected_attempt": 1,
            "selected_output_sha256": digest(&bytes),
            "selected_output_path": selected.to_string_lossy(),
            "capture_dir": capture.to_string_lossy(),
            "command": "/bin/worker",
            "args": ["--review"],
            "binding": {"command": "/bin/worker", "args": ["--review"]}
        }
    })
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
fn concise_origin_judgment_must_agree_with_engine_selected_bytes() {
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
fn concise_origin_fails_unverified_when_selected_bytes_change_or_disappear() {
    let root = TestDir::new("linkage-unverified");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let evidence = linked_evidence(&root, "pass", "");
    std::fs::write(selected_path(&root), b"changed").unwrap();
    let refused = evaluate(&root, evidence.clone());
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));

    let evidence = linked_evidence(&root, "pass", "");
    std::fs::remove_file(selected_path(&root)).unwrap();
    let refused = evaluate(&root, evidence);
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));
}

#[test]
fn legacy_verbose_evidence_linkage_is_not_a_parallel_provider_path() {
    let root = TestDir::new("legacy-linkage");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let mut evidence = linked_evidence(&root, "pass", "");
    evidence["originating_output"] = json!({
        "invocation_id": "invocation-1",
        "assignment_id": "worker-0",
        "selected_attempt": 1,
        "sha256": evidence["loop_engine_origin"]["selected_output_sha256"],
        "path": selected_path(&root).to_string_lossy()
    });
    evidence.as_object_mut().unwrap().remove("origin");
    evidence
        .as_object_mut()
        .unwrap()
        .remove("loop_engine_origin");
    let refused = evaluate(&root, evidence);
    assert!(refused["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|diagnostic| diagnostic["category"] == "unverified"));
}

#[test]
fn applicability_resolves_original_evidence_and_keeps_attestation_separate() {
    let root = TestDir::new("applicability");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let mut request = base_request(
        axis_config(&root, "axis"),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        context_json(
            "review-evidence",
            json!({
                "gate": "intent-review",
                "policy_id": "axis",
                "result": "pass",
                "findings": "",
                "author": {"name": "original-reviewer", "kind": "agent"},
                "subject": "intent.json",
                "subject_revision": "1",
                "config_version": "test-1"
            }),
            1,
        ),
        context_json(
            "evidence-applicability",
            json!({
                "origin": {"kind": "context-record", "id": "context-1"},
                "target": {"subject": "intent.json", "revision": "1", "checkpoint": null},
                "attesting_driver": {"name": "driver", "kind": "human"},
                "reason": "The original review still applies to this target."
            }),
            2,
        ),
        context_json("finding-ledger", ledger(), 3)
    ]);
    // Keep the declared revision while changing ordinary artifact content:
    // applicability is accepted because the driver declared it, not because
    // the provider inferred semantic equivalence from this edit.
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner-after-edit", "kind": "human"}}),
    );
    let output = invoke(request);
    support::assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));

    let mut stale = base_request(
        axis_config(&root, "axis"),
        checked("intent-review", "approved", "design"),
    );
    stale["context"] = json!([
        context_json(
            "review-evidence",
            json!({
                "gate": "intent-review",
                "policy_id": "axis",
                "result": "pass",
                "findings": "",
                "author": {"name": "original-reviewer", "kind": "agent"},
                "subject": "intent.json",
                "subject_revision": "1",
                "config_version": "test-1"
            }),
            1,
        ),
        context_json(
            "evidence-applicability",
            json!({
                "origin": {"kind": "context-record", "id": "context-1"},
                "target": {"subject": "design.json", "revision": "1", "checkpoint": null},
                "attesting_driver": {"name": "driver", "kind": "human"},
                "reason": "wrong target"
            }),
            2,
        ),
        context_json("finding-ledger", ledger(), 3)
    ]);
    let output = invoke(stale);
    support::assert_exit(&output, 0);
    assert_eq!(
        response(&output)["feedback"]["code"],
        "software-change-review-incomplete"
    );
}
