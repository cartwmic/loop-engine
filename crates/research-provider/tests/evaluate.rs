mod support;

use serde_json::{json, Value};
use support::{
    assert_exit, base_request, checked, context_record, evidence, invoke, metadata_schema,
    response, transition, valid_metadata, TestDir,
};

fn config_with_schema(subject: &str, root: Option<&TestDir>) -> Value {
    let mut config = json!({
        "config_version": "test-1",
        "review_policies": {},
        "artifact_schemas": {subject: metadata_schema()}
    });
    if let Some(root) = root {
        config["artifact_root"] = root.root_value();
    }
    config
}

fn config_with_verify_axis(root: &TestDir) -> Value {
    json!({
        "config_version": "test-1",
        "artifact_root": root.root_value(),
        "review_policies": {
            "verify": [{"id": "axis", "description": "test axis"}]
        },
        "artifact_schemas": {"verification.json": metadata_schema()}
    })
}

#[test]
fn schema_deny_reports_every_simultaneous_structural_violation() {
    let root = TestDir::new("schema-deny");
    root.write_json(
        "brief.json",
        &json!({
            "author": {"name": "", "kind": "unknown"},
            "unexpected": true
        }),
    );
    let output = invoke(base_request(
        config_with_schema("brief.json", Some(&root)),
        checked("scope", "scoped", "gather"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["result"], "deny");
    assert_eq!(value["feedback"]["code"], "research-schema-invalid");
    assert_eq!(
        value["feedback"]["details"]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|violation| json!({
                "path": violation["path"],
                "rule": violation["rule"]
            }))
            .collect::<Vec<_>>(),
        vec![
            json!({"path": "", "rule": "required"}),
            json!({"path": "/author/kind", "rule": "enum"}),
            json!({"path": "/author/name", "rule": "minLength"}),
            json!({"path": "/unexpected", "rule": "additionalProperties"}),
        ]
    );
}

#[test]
fn revision_link_mismatch_is_schema_deny_naming_both_artifacts() {
    let root = TestDir::new("link-mismatch");
    let mut config = json!({
        "config_version": "test-1",
        "artifact_root": root.root_value(),
        "review_policies": {},
        "artifact_schemas": {
            "sources.json": metadata_schema(),
            "brief.json": metadata_schema()
        },
        "revision_links": [{
            "from": "sources.json",
            "field": "brief_revision",
            "to": "brief.json"
        }]
    });
    config["artifact_schemas"]["sources.json"]["properties"]["brief_revision"] =
        json!({"type": "string", "minLength": 1});
    root.write_json(
        "sources.json",
        &json!({
            "revision": "s1",
            "author": {"name": "owner", "kind": "human"},
            "brief_revision": "b1"
        }),
    );
    root.write_json("brief.json", &valid_metadata("b2"));
    let output = invoke(base_request(
        config,
        checked("gather", "gathered", "verify"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "research-schema-invalid");
    let message = value["feedback"]["details"]["violations"][0]["message"]
        .as_str()
        .unwrap();
    assert!(message.contains("sources.json"), "{message}");
    assert!(message.contains("brief.json"), "{message}");
}

#[test]
fn stale_evidence_denies_verified() {
    let root = TestDir::new("stale-evidence");
    root.write_json("verification.json", &valid_metadata("2"));
    let mut request = base_request(
        config_with_verify_axis(&root),
        checked("verify", "verified", "synthesize"),
    );
    request["context"] = json!([context_record(evidence(
        "verify",
        "axis",
        "pass",
        "",
        "reviewer",
        "agent",
        "verification.json",
        "1",
        "test-1"
    ))]);
    let output = invoke(request);
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "research-review-incomplete");
    assert!(value["feedback"]["details"]["informational"]
        .as_array()
        .unwrap()
        .iter()
        .any(|axis| axis["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["category"] == "stale")));
}

#[test]
fn self_authored_evidence_denies_verified() {
    let root = TestDir::new("self-authored");
    root.write_json("verification.json", &valid_metadata("1"));
    let mut request = base_request(
        config_with_verify_axis(&root),
        checked("verify", "verified", "synthesize"),
    );
    request["context"] = json!([context_record(evidence(
        "verify",
        "axis",
        "pass",
        "",
        "owner",
        "human",
        "verification.json",
        "1",
        "test-1"
    ))]);
    let output = invoke(request);
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "research-review-incomplete");
}

#[test]
fn incomplete_evidence_denies_verified() {
    let root = TestDir::new("incomplete-evidence");
    root.write_json("verification.json", &valid_metadata("1"));
    let output = invoke(base_request(
        config_with_verify_axis(&root),
        checked("verify", "verified", "synthesize"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "research-review-incomplete");
    assert!(value["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|axis| axis["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["category"] == "missing")));
}

#[test]
fn independent_pass_allows_verified() {
    let root = TestDir::new("independent-pass");
    root.write_json("verification.json", &valid_metadata("1"));
    let mut request = base_request(
        config_with_verify_axis(&root),
        checked("verify", "verified", "synthesize"),
    );
    request["context"] = json!([context_record(evidence(
        "verify",
        "axis",
        "pass",
        "",
        "reviewer",
        "agent",
        "verification.json",
        "1",
        "test-1"
    ))]);
    let output = invoke(request);
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn check_free_revise_allows_without_artifacts() {
    let output = invoke(base_request(
        json!({"config_version": "test-1", "review_policies": {}}),
        transition("gather", "revise", "scope", "check-free"),
    ));
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn unsupported_tuple_returns_exact_unsupported_result() {
    let output = invoke(base_request(
        json!({"config_version": "none", "review_policies": {}}),
        checked("scope", "wrong-event", "gather"),
    ));
    assert_exit(&output, 0);
    assert_eq!(output.stdout, br#"{"result":"unsupported"}"#);
}
