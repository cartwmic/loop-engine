//! Semantic integration tests using golden fixtures and subprocess transport.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    let raw = fs::read_to_string(path.as_ref()).expect("read fixture");
    serde_json::from_str(&raw).expect("parse fixture json")
}

fn config_fixture(name: &str) -> Value {
    read_json(fixture_root().join(format!("config/{name}.json")))
}

fn version_fixture(name: &str) -> Value {
    read_json(fixture_root().join(format!("versions/{name}.json")))
}

fn string_argv(values: &Value) -> Vec<String> {
    values
        .as_array()
        .expect("argv array")
        .iter()
        .map(|value| value.as_str().expect("argv entry").to_string())
        .collect()
}

fn process_argv_from(fixture: &Value) -> Vec<String> {
    string_argv(&fixture["process_argv"])
}

fn registration_argv_from(fixture: &Value) -> Vec<Value> {
    fixture["registration_argv"]
        .as_array()
        .expect("registration argv array")
        .clone()
}

fn apply_registration_argv(request: &mut Value, fixture: &Value) {
    request["registration"]["argv"] = json!(registration_argv_from(fixture));
}

fn invoke(request: Value, extra_argv: &[&str]) -> Value {
    let bin = env!("CARGO_BIN_EXE_reference-provider");
    let mut command = Command::new(bin);
    command.args(extra_argv);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn provider");
    std::io::Write::write_all(
        &mut child.stdin.take().expect("stdin"),
        serde_json::to_string(&request).unwrap().as_bytes(),
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "provider failed: {:?}",
        output.status
    );
    serde_json::from_slice(&output.stdout).expect("parse provider stdout")
}

fn invoke_with_config_fixture(request: Value, fixture: &Value) -> Value {
    let process_argv = process_argv_from(fixture);
    let process_argv_refs: Vec<&str> = process_argv.iter().map(String::as_str).collect();
    invoke(request, &process_argv_refs)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn gate_request(
    artifact_root: &Path,
    event: &str,
    required_gate_ids: &[&str],
    registration_argv: Value,
) -> Value {
    json!({
        "protocol_major": 1,
        "role": "evaluate_gates",
        "invocation_id": format!("gate-{event}"),
        "registration": {
            "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
            "config_revision": 1,
            "executable": "/opt/providers/reference-provider",
            "argv": registration_argv,
            "working_directory": "/opt/providers",
            "timeout_seconds": 60
        },
        "payload": {
            "snapshot": {
                "run_id": "run-1",
                "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
                "graph_revision": "sha256:test",
                "lifecycle": "active",
                "current_state": "explore",
                "workflow_state_version": 1,
                "lifecycle_version": 1,
                "inputs": {
                    "artifact_root": artifact_root.display().to_string()
                },
                "stored_graph": {
                    "canonical_graph_version": 1,
                    "initial_state_id": "explore",
                    "input_declarations": [],
                    "live_guidance_supported": true,
                    "states": [],
                    "transitions": []
                }
            },
            "event": event,
            "required_gate_ids": required_gate_ids,
            "selected_evidence": [],
            "inline_evidence": []
        }
    })
}

#[test]
fn describe_matches_golden_topology() {
    let request = read_json(fixture_root().join("requests/describe-request.json"));
    let result = invoke(request, &[]);
    let golden = read_json(fixture_root().join("golden/graph-topology.json"));

    let graph = &result["result"]["graph"];
    assert_eq!(graph["initial_state"], golden["initial_state"]);
    assert_eq!(
        graph["live_guidance_supported"],
        golden["live_guidance_supported"]
    );

    for state in golden["states"].as_array().unwrap() {
        assert!(
            graph["states"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate["id"] == *state),
            "missing state {state}"
        );
    }

    for edge in golden["transitions"].as_array().unwrap() {
        let source = &edge[0];
        let event = &edge[1];
        let target = &edge[2];
        let gates = &edge[3];
        assert!(
            graph["transitions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|transition| {
                    transition["source_state"] == *source
                        && transition["event"] == *event
                        && transition["target_state"] == *target
                        && transition["gate_ids"] == *gates
                }),
            "missing transition {edge:?}"
        );
    }
}

#[test]
fn validate_inputs_accepts_fixture_request() {
    let request = read_json(fixture_root().join("requests/validate-inputs-accepted.json"));
    let result = invoke(request, &[]);
    assert_eq!(result["result"]["kind"], "accepted");
}

#[test]
fn validate_inputs_rejects_missing_required_input() {
    let request = read_json(fixture_root().join("requests/validate-inputs-rejected.json"));
    let result = invoke(request, &[]);
    assert_eq!(result["result"]["kind"], "rejected");
    assert_eq!(result["result"]["diagnostics"][0]["code"], "input.required");
}

#[test]
fn happy_path_gate_sequence_passes() {
    let artifact_root = std::env::temp_dir().join("reference-provider-happy-path");
    let _ = fs::remove_dir_all(&artifact_root);
    copy_dir(&fixture_root().join("artifacts/happy-path"), &artifact_root);

    let gates = [
        ("intent-ready", vec!["intent-ready"]),
        ("design-ready", vec!["design-ready"]),
        ("approved", vec!["design-review-approved"]),
        ("plan-ready", vec!["plan-ready"]),
        ("approved", vec!["plan-review-approved"]),
        ("implementation-ready", vec!["implementation-ready"]),
        ("approved", vec!["implementation-review-approved"]),
        ("passed", vec!["validation-passed"]),
    ];

    for (event, required_gate_ids) in gates {
        let request = gate_request(&artifact_root, event, &required_gate_ids, json!([]));
        let result = invoke(request, &[]);
        assert_eq!(result["result"]["kind"], "verdicts", "event {event}");
        assert!(
            result["result"]["verdicts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|verdict| verdict["passed"] == true),
            "event {event} failed: {result}"
        );
        let evidence = result["result"]["evidence"]
            .as_array()
            .expect("expected provider evidence");
        let digest = evidence[0]["digest"]
            .as_str()
            .expect("engine-owned digest must be present");
        assert!(digest.starts_with("sha256:"));
    }
}

#[test]
fn version_fixture_echoes_provider_version_without_faking_digest() {
    let version = version_fixture("default-v1");
    let request = read_json(fixture_root().join("requests/describe-request.json"));
    let result = invoke_with_config_fixture(request.clone(), &version);
    assert_eq!(
        result["provider_version"],
        version["process_argv"][0]
            .as_str()
            .unwrap()
            .strip_prefix("--provider-version=")
            .unwrap()
    );

    let alternate = version_fixture("alternate-v2-build");
    let alternate_result = invoke_with_config_fixture(request, &alternate);
    assert_eq!(
        alternate_result["provider_version"],
        "reference-provider/2.0.0-test"
    );
    assert_eq!(
        alternate_result["result"]["graph"]["metadata"]["workflow_version"],
        alternate["graph_metadata_version"]
    );
}

#[test]
fn describe_graph_v2_fixture_only_affects_new_describe_output() {
    let graph_config = config_fixture("describe-graph-v2");
    let stored =
        read_json(fixture_root().join("requests/check-compatibility-incompatible.json"))["payload"]
            ["stored_graph"]
            .clone();
    let stored_snapshot = stored.clone();

    let describe_request = read_json(fixture_root().join("requests/describe-request.json"));
    let baseline = invoke(describe_request.clone(), &[]);
    assert!(
        !baseline["result"]["graph"]["input_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "policy_root")
    );

    let v2 = invoke_with_config_fixture(describe_request, &graph_config);
    assert!(
        v2["result"]["graph"]["input_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "policy_root")
    );
    assert_eq!(v2["result"]["graph"]["metadata"]["workflow_version"], "2");

    let mut compat_request =
        read_json(fixture_root().join("requests/check-compatibility-incompatible.json"));
    compat_request["registration"]["argv"] = json!([]);
    compat_request["payload"]["stored_graph"] = stored_snapshot;
    let compat = invoke_with_config_fixture(compat_request, &graph_config);
    assert_eq!(compat["result"]["capabilities"][0]["status"], "compatible");
    assert_eq!(stored, stored);
}

#[test]
fn unsupported_stored_gate_request_returns_incompatible_not_failed_verdict() {
    let request =
        read_json(fixture_root().join("requests/evaluate-gates-unsupported-stored-gate.json"));
    let result = invoke(request, &[]);
    assert_eq!(result["result"]["kind"], "incompatible");
    assert_eq!(
        result["result"]["diagnostics"][0]["code"],
        "compatibility.unsupported"
    );
    assert!(
        result["result"]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("legacy-intent-ready")
    );
    assert!(result["result"].get("verdicts").is_none());
}

#[test]
fn compatibility_incompatible_config_fixture_reports_finding() {
    let config = config_fixture("compat-incompatible");
    let mut request =
        read_json(fixture_root().join("requests/check-compatibility-incompatible.json"));
    apply_registration_argv(&mut request, &config);
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "findings");
    assert_eq!(
        result["result"]["capabilities"][0]["status"],
        "incompatible"
    );
}

#[test]
fn compatibility_evaluation_error_config_fixture_returns_evaluation_error() {
    let config = config_fixture("compat-evaluation-error");
    let mut request = read_json(fixture_root().join("requests/describe-request.json"));
    request["role"] = json!("check_compatibility");
    request["payload"] = json!({
        "stored_graph": {
            "canonical_graph_version": 1,
            "initial_state_id": "explore",
            "input_declarations": [],
            "live_guidance_supported": true,
            "states": [],
            "transitions": [{
                "event_id": "intent-ready",
                "gate_ids": ["intent-ready"],
                "source_state_id": "explore",
                "target_state_id": "design"
            }]
        },
        "capabilities": ["evaluate_gates"]
    });
    apply_registration_argv(&mut request, &config);
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "evaluation_error");
}

#[test]
fn gate_incompatible_config_fixture_returns_incompatible_result() {
    let config = config_fixture("gate-incompatible");
    let artifact_root = std::env::temp_dir().join("reference-provider-gate-incompatible");
    let _ = fs::remove_dir_all(&artifact_root);
    copy_dir(&fixture_root().join("artifacts/happy-path"), &artifact_root);

    let request = gate_request(
        &artifact_root,
        "intent-ready",
        &["intent-ready"],
        json!(registration_argv_from(&config)),
    );
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "incompatible");
}

#[test]
fn gate_evaluation_error_config_fixture_returns_evaluation_error() {
    let config = config_fixture("gate-evaluation-error");
    let request = gate_request(
        &PathBuf::from("/tmp/artifacts"),
        "intent-ready",
        &["intent-ready"],
        json!(registration_argv_from(&config)),
    );
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "evaluation_error");
}

#[test]
fn guidance_recommend_config_fixture_is_advisory_only() {
    let config = config_fixture("guidance-recommend");
    let request = json!({
        "protocol_major": 1,
        "role": "live_guidance",
        "invocation_id": "guidance-1",
        "registration": {
            "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
            "config_revision": 1,
            "executable": "/opt/providers/reference-provider",
            "argv": registration_argv_from(&config),
            "working_directory": "/opt/providers",
            "timeout_seconds": 60
        },
        "payload": {
            "snapshot": {
                "run_id": "run-1",
                "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
                "graph_revision": "sha256:test",
                "lifecycle": "active",
                "current_state": "design",
                "workflow_state_version": 1,
                "lifecycle_version": 1,
                "inputs": {"artifact_root": "/tmp/artifacts"},
                "stored_graph": {
                    "canonical_graph_version": 1,
                    "initial_state_id": "explore",
                    "input_declarations": [],
                    "live_guidance_supported": true,
                    "states": [],
                    "transitions": []
                }
            },
            "selected_evidence": [{
                "id": "intent-document-1",
                "kind": "intent-document",
                "locator": "file:///tmp/artifacts/intent.json"
            }]
        }
    });
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "guidance");
    assert!(
        result["result"]["text"]
            .as_str()
            .unwrap()
            .contains("advisory only")
    );
}

#[test]
fn guidance_evaluation_error_config_fixture_returns_evaluation_error() {
    let config = config_fixture("guidance-evaluation-error");
    let request = json!({
        "protocol_major": 1,
        "role": "live_guidance",
        "invocation_id": "guidance-error",
        "registration": {
            "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
            "config_revision": 1,
            "executable": "/opt/providers/reference-provider",
            "argv": registration_argv_from(&config),
            "working_directory": "/opt/providers",
            "timeout_seconds": 60
        },
        "payload": {
            "snapshot": {
                "run_id": "run-1",
                "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
                "graph_revision": "sha256:test",
                "lifecycle": "active",
                "current_state": "design",
                "workflow_state_version": 1,
                "lifecycle_version": 1,
                "inputs": {"artifact_root": "/tmp/artifacts"},
                "stored_graph": {
                    "canonical_graph_version": 1,
                    "initial_state_id": "explore",
                    "input_declarations": [],
                    "live_guidance_supported": true,
                    "states": [],
                    "transitions": []
                }
            },
            "selected_evidence": []
        }
    });
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "evaluation_error");
}

#[test]
fn invalid_json_artifact_subprocess_fails_gate_without_incompatible() {
    let artifact_root = std::env::temp_dir().join("reference-provider-invalid-json");
    let _ = fs::remove_dir_all(&artifact_root);
    fs::create_dir_all(&artifact_root).unwrap();
    fs::write(artifact_root.join("intent.json"), "not-json").unwrap();

    let request = gate_request(&artifact_root, "intent-ready", &["intent-ready"], json!([]));
    let result = invoke(request, &[]);
    assert_eq!(result["result"]["kind"], "verdicts");
    assert_eq!(result["result"]["verdicts"][0]["passed"], false);
}

#[test]
fn malformed_evidence_config_fixture_emits_invalid_provider_evidence() {
    let config = config_fixture("malformed-evidence");
    let artifact_root = std::env::temp_dir().join("reference-provider-malformed-evidence");
    let _ = fs::remove_dir_all(&artifact_root);
    copy_dir(&fixture_root().join("artifacts/happy-path"), &artifact_root);

    let request = gate_request(
        &artifact_root,
        "intent-ready",
        &["intent-ready"],
        json!(registration_argv_from(&config)),
    );
    let result = invoke_with_config_fixture(request, &config);
    assert_eq!(result["result"]["kind"], "verdicts");
    assert_eq!(result["result"]["evidence"][0]["id"], "");
    assert_eq!(result["result"]["evidence"][0]["kind"], "invalid");
}

#[test]
fn design_revision_cycle_subprocess_preserves_append_only_evidence_ids() {
    let artifact_root = std::env::temp_dir().join("reference-provider-design-revision");
    let _ = fs::remove_dir_all(&artifact_root);
    fs::create_dir_all(&artifact_root).unwrap();
    fs::write(
        artifact_root.join("intent.json"),
        serde_json::to_string(&json!({"revision":"1","summary":"intent"})).unwrap(),
    )
    .unwrap();
    fs::write(
        artifact_root.join("design.json"),
        serde_json::to_string(&json!({"revision":"1","intent_revision":"1"})).unwrap(),
    )
    .unwrap();

    let first = gate_request(&artifact_root, "design-ready", &["design-ready"], json!([]));
    let first_result = invoke(first, &[]);
    assert_eq!(first_result["result"]["kind"], "verdicts");
    assert_eq!(first_result["result"]["verdicts"][0]["passed"], true);
    let first_id = first_result["result"]["evidence"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(first_id, "design-document-1");
    assert_eq!(
        first_result["result"]["evidence"][0]["kind"],
        "design-document"
    );

    fs::write(
        artifact_root.join("design.json"),
        serde_json::to_string(&json!({"revision":"2","intent_revision":"1"})).unwrap(),
    )
    .unwrap();

    let mut second = gate_request(&artifact_root, "design-ready", &["design-ready"], json!([]));
    second["payload"]["selected_evidence"] = json!([{
        "id": first_id,
        "kind": "design-document",
        "locator": format!("file://{}/design.json", artifact_root.display())
    }]);
    let second_result = invoke(second, &[]);
    assert_eq!(second_result["result"]["kind"], "verdicts");
    assert_eq!(second_result["result"]["verdicts"][0]["passed"], true);
    assert_eq!(
        second_result["result"]["evidence"][0]["id"],
        "design-document-2"
    );
    assert_ne!(
        second_result["result"]["evidence"][0]["id"]
            .as_str()
            .unwrap(),
        first_id
    );
    assert_eq!(
        second_result["result"]["evidence"][0]["kind"],
        "design-document"
    );
}

#[test]
fn design_ready_subprocess_rejects_subject_revision_fallback_link() {
    let artifact_root = std::env::temp_dir().join("reference-provider-design-link");
    let _ = fs::remove_dir_all(&artifact_root);
    fs::create_dir_all(&artifact_root).unwrap();
    fs::write(
        artifact_root.join("intent.json"),
        serde_json::to_string(&json!({"revision":"1","summary":"intent"})).unwrap(),
    )
    .unwrap();
    fs::write(
        artifact_root.join("design.json"),
        serde_json::to_string(&json!({"revision":"1","subject_revision":"1"})).unwrap(),
    )
    .unwrap();

    let request = gate_request(&artifact_root, "design-ready", &["design-ready"], json!([]));
    let result = invoke(request, &[]);
    assert_eq!(result["result"]["kind"], "verdicts");
    assert_eq!(result["result"]["verdicts"][0]["passed"], false);
}
