use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-provider-evaluate-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary artifact root");
        Self { path }
    }

    fn root_value(&self) -> Value {
        json!(self.path.to_string_lossy().to_string())
    }

    fn write_json(&self, subject: &str, value: &Value) {
        fs::write(self.path.join(subject), serde_json::to_vec(value).unwrap())
            .expect("write artifact JSON");
    }

    fn write_text(&self, subject: &str, value: &str) {
        fs::write(self.path.join(subject), value).expect("write artifact text");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn transition(source: &str, event: &str, target: &str, kind: &str) -> Value {
    json!({
        "source": source,
        "event": event,
        "target": target,
        "kind": kind
    })
}

fn checked(source: &str, event: &str, target: &str) -> Value {
    transition(source, event, target, "checked")
}

fn check_free(source: &str, event: &str, target: &str) -> Value {
    transition(source, event, target, "check-free")
}

fn described_workflow(initial_input: &Value) -> Value {
    let output = run_provider(json!({
        "operation": "describe",
        "initial_input": initial_input
    }));
    assert_exit(&output, 0);
    response(&output)
}

fn base_request(initial_input: Value, transition: Value) -> Value {
    let workflow = described_workflow(&initial_input);
    json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": initial_input,
        "context": [],
        "transition": transition,
        "prior_evaluations": []
    })
}

fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "author": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {
                        "type": "string",
                        "enum": ["human", "agent", "script"]
                    }
                },
                "required": ["name", "kind"],
                "additionalProperties": false
            }
        },
        "required": ["revision", "author"],
        "additionalProperties": false
    })
}

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

fn config_with_axis(root: &TestDir) -> Value {
    json!({
        "config_version": "test-1",
        "artifact_root": root.root_value(),
        "review_policies": {
            "intent-review": [{"id": "axis", "description": "test axis"}]
        },
        "artifact_schemas": {"intent.json": metadata_schema()}
    })
}

fn valid_metadata(revision: &str) -> Value {
    json!({
        "revision": revision,
        "author": {"name": "owner", "kind": "human"}
    })
}

fn criterion_intent_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "acceptance": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "statement"],
                    "properties": {
                        "id": {"type": "string", "pattern": "^AC-[1-9][0-9]*$"},
                        "statement": {"type": "string", "minLength": 1}
                    },
                    "additionalProperties": false
                },
                "minItems": 1
            }
        },
        "required": ["revision", "acceptance"],
        "additionalProperties": false
    })
}

fn criterion_design_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "intent_revision": {"type": "string", "minLength": 1},
            "coverage": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["acceptance", "delivered_by"],
                    "properties": {
                        "acceptance": {"type": "string", "minLength": 1},
                        "delivered_by": {"type": "string", "minLength": 1},
                        "criterion_id": {"type": "string", "pattern": "^AC-[1-9][0-9]*$"}
                    },
                    "additionalProperties": false
                },
                "minItems": 1
            }
        },
        "required": ["revision", "intent_revision", "coverage"],
        "additionalProperties": false
    })
}

fn criterion_config(root: &TestDir) -> Value {
    json!({
        "config_version": "criteria-test-1",
        "artifact_root": root.root_value(),
        "review_policies": {},
        "artifact_schemas": {
            "intent.json": criterion_intent_schema(),
            "design.json": criterion_design_schema()
        },
        "revision_links": [{
            "from": "design.json",
            "field": "intent_revision",
            "to": "intent.json"
        }]
    })
}

fn criterion_intent(revision: &str, criteria: Value) -> Value {
    json!({"revision": revision, "acceptance": criteria})
}

fn criterion_design(revision: &str, intent_revision: &str, coverage: Value) -> Value {
    json!({
        "revision": revision,
        "intent_revision": intent_revision,
        "coverage": coverage
    })
}

fn pre_8_criterion_config(root: &TestDir) -> Value {
    json!({
        "config_version": "criteria-pre-8",
        "artifact_root": root.root_value(),
        "review_policies": {},
        "artifact_schemas": {
            "intent.json": {
                "type": "object",
                "properties": {
                    "revision": {"type": "string", "minLength": 1},
                    "acceptance": {
                        "type": "array",
                        "items": {"type": "string", "minLength": 1},
                        "minItems": 1
                    }
                },
                "required": ["revision", "acceptance"],
                "additionalProperties": false
            },
            "design.json": {
                "type": "object",
                "properties": {
                    "revision": {"type": "string", "minLength": 1},
                    "intent_revision": {"type": "string", "minLength": 1},
                    "coverage": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "acceptance": {"type": "string", "minLength": 1},
                                "delivered_by": {"type": "string", "minLength": 1}
                            },
                            "required": ["acceptance", "delivered_by"]
                        },
                        "minItems": 1
                    }
                },
                "required": ["revision", "intent_revision", "coverage"],
                "additionalProperties": false
            }
        },
        "revision_links": [{
            "from": "design.json",
            "field": "intent_revision",
            "to": "intent.json"
        }]
    })
}

fn context_record(data: Value) -> Value {
    json!({
        "id": "context-1",
        "kind": "review-evidence",
        "data": data,
        "sequence": 1,
        "created_at": 1
    })
}

fn finding_ledger_record(gate: &str, subject: &str, revision: &str, findings: Value) -> Value {
    json!({
        "id": "ledger-1",
        "kind": "finding-ledger",
        "data": {
            "schema_version": "1",
            "gate": gate,
            "subject": subject,
            "subject_revision": revision,
            "author": {"name": "driver", "kind": "agent"},
            "findings": findings
        },
        "sequence": 2,
        "created_at": 2
    })
}

fn passing_evidence() -> Value {
    context_record(json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": "pass",
        "findings": "",
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1"
    }))
}

#[test]
fn evaluate_accepts_concise_selected_origin_and_engine_metadata_only() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let capture = root.path.join("capture");
    let selected = capture.join("worker-0/attempts/1/stdout");
    fs::create_dir_all(selected.parent().unwrap()).unwrap();
    let raw = serde_json::to_vec(&json!({
        "axis": "axis",
        "author": {"name": "reviewer", "kind": "agent"},
        "result": "pass",
        "findings": ""
    }))
    .unwrap();
    fs::write(&selected, &raw).unwrap();
    let evidence = json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": "pass",
        "findings": "",
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1",
        "origin": {"kind": "selected-assignment-output", "id": "invocation-1", "assignment_id": "worker-0"},
        "loop_engine_origin": {
            "invocation_id": "invocation-1",
            "assignment_id": "worker-0",
            "selected_attempt": 1,
            "selected_output_sha256": format!("sha256:{:x}", Sha256::digest(&raw)),
            "selected_output_path": selected.to_string_lossy(),
            "capture_dir": capture.to_string_lossy(),
            "command": "/bin/worker",
            "args": ["--review"],
            "binding": {"command": "/bin/worker", "args": ["--review"]}
        }
    });
    let mut request = base_request(
        config_with_axis(&root),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        {"id": "evidence-1", "kind": "review-evidence", "data": evidence, "sequence": 1, "created_at": 1},
        finding_ledger_record("intent-review", "intent.json", "1", json!([]))
    ]);
    let output = run_provider(request);
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

fn run_provider(request: Value) -> Output {
    run_provider_in(
        &workspace_integration::package_root("software-change-provider"),
        request,
    )
}

fn run_provider_in(directory: &Path, request: Value) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    command.current_dir(directory);
    let request = serde_json::to_vec(&request).expect("serialize provider request");
    super::bounded_process::run_with_stdin(
        &mut command,
        "software-change evaluate protocol",
        &request,
    )
    .expect("wait for provider")
    .output
}

fn response(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "provider stdout is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stderr={:?}",
        output.stderr
    );
}

fn criterion_violation_rules(output: &Output) -> Vec<String> {
    response(output)["feedback"]["details"]["violations"]
        .as_array()
        .expect("criterion schema violations")
        .iter()
        .map(|violation| {
            violation["rule"]
                .as_str()
                .expect("criterion violation rule")
                .to_owned()
        })
        .collect()
}

#[test]
fn zero_obligation_allows_without_artifact_root() {
    let output = run_provider(base_request(
        json!({"config_version": "none", "review_policies": {}}),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn schema_deny_reports_every_simultaneous_structural_violation() {
    let root = TestDir::new();
    root.write_json(
        "intent.json",
        &json!({
            "author": {"name": "", "kind": "unknown"},
            "unexpected": true
        }),
    );
    let output = run_provider(base_request(
        config_with_schema("intent.json", Some(&root)),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["result"], "deny");
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
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
fn schema_deny_is_byte_identical_when_only_context_varies() {
    let root = TestDir::new();
    let mut artifact = valid_metadata("1");
    artifact["unexpected"] = json!(true);
    root.write_json("intent.json", &artifact);
    let config = config_with_schema("intent.json", Some(&root));
    let transition = checked("explore", "intent-ready", "design");

    let first = run_provider(base_request(config.clone(), transition.clone()));
    let mut second_request = base_request(config, transition);
    second_request["context"] = json!([context_record(json!({"untrusted": "ignored"}))]);
    let second = run_provider(second_request);

    assert_exit(&first, 0);
    assert_exit(&second, 0);
    assert_eq!(first.stdout, second.stdout);
    let value = response(&first);
    assert_eq!(value["result"], "deny");
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert_eq!(value["feedback"]["message"], "not judged: fix shape first");
    assert!(!value["feedback"]["details"]["violations"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn approval_transition_rechecks_current_subject_after_ready_pass() {
    let root = TestDir::new();
    root.write_json("design.json", &valid_metadata("1"));
    let mut config = config_with_schema("design.json", Some(&root));
    config["review_policies"]["design-review"] =
        json!([{"id": "axis", "description": "test axis"}]);

    let ready = run_provider(base_request(
        config.clone(),
        checked("design", "design-ready", "design-review"),
    ));
    assert_exit(&ready, 0);
    assert_eq!(response(&ready), json!({"result": "allow"}));

    let mut malformed = valid_metadata("1");
    malformed["unexpected"] = json!(true);
    root.write_json("design.json", &malformed);
    let approval = run_provider(base_request(
        config,
        checked("design-review", "approved", "plan"),
    ));
    assert_exit(&approval, 0);
    assert_eq!(approval.status.code(), Some(0));
    assert_eq!(
        response(&approval)["feedback"]["code"],
        "software-change-schema-invalid"
    );
}

#[test]
fn prior_denials_are_flat_and_accumulate_across_requests() {
    let root = TestDir::new();
    root.write_json("intent.json", &json!("not an object"));
    let config = config_with_schema("intent.json", Some(&root));
    let transition = checked("explore", "intent-ready", "design");

    let first = run_provider(base_request(config.clone(), transition.clone()));
    let first_value = response(&first);
    let mut second_request = base_request(config, transition.clone());
    second_request["prior_evaluations"] = json!([{
        "transition": transition,
        "result": {"result": "deny", "feedback": first_value["feedback"].clone()},
        "sequence": 7,
        "occurred_at": 7
    }]);
    let second = run_provider(second_request);
    assert_exit(&second, 0);
    let details = &response(&second)["feedback"]["details"];
    let prior = details["prior_denials"]
        .as_array()
        .expect("prior denials array");
    assert_eq!(prior.len(), 1);
    assert_eq!(
        prior[0],
        json!({
            "sequence": 7,
            "code": "software-change-schema-invalid",
            "message": "not judged: fix shape first"
        })
    );
    assert!(prior[0].get("details").is_none());
}

#[test]
fn unsupported_tuple_returns_exact_unsupported_result() {
    let output = run_provider(base_request(
        json!({"config_version": "none", "review_policies": {}}),
        checked("explore", "wrong-event", "design"),
    ));
    assert_exit(&output, 0);
    assert_eq!(output.stdout, br#"{"result":"unsupported"}"#);
}

#[test]
fn missing_review_policies_is_evaluation_error_naming_shipped_configs() {
    let output = run_provider(base_request(
        json!({"config_version": "test-1"}),
        checked("explore", "intent-ready", "intent-review"),
    ));
    assert_exit(&output, 1);
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("minimal"));
    assert!(stderr.contains("standard"));
    assert!(stderr.contains("high-rigor"));
}

#[test]
fn evidence_phase_denies_with_configured_axis_diagnostics() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let mut request = base_request(
        config_with_axis(&root),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([finding_ledger_record(
        "intent-review",
        "intent.json",
        "1",
        json!([])
    )]);
    let output = run_provider(request);
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(
        value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert_eq!(value["feedback"]["message"], "review evidence incomplete");
    assert!(value["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|axis| axis["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["category"] == "missing")));
}

#[test]
fn mixed_current_blocker_and_stale_context_separates_feedback_projection() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("2"));
    let config = config_with_axis(&root);
    let transition = checked("intent-review", "approved", "design");
    let mut request = base_request(config, transition);
    request["context"] = json!([
        context_record(json!({
            "gate": "intent-review",
            "policy_id": "axis",
            "result": "fail",
            "findings": "current blocker",
            "author": {"name": "current-reviewer", "kind": "agent"},
            "subject": "intent.json",
            "subject_revision": "2",
            "config_version": "test-1"
        })),
        context_record(json!({
            "gate": "intent-review",
            "policy_id": "axis",
            "result": "pass",
            "findings": "",
            "author": {"name": "stale-reviewer", "kind": "agent"},
            "subject": "intent.json",
            "subject_revision": "1",
            "config_version": "test-1"
        })),
        finding_ledger_record("intent-review", "intent.json", "2", json!([]))
    ]);

    let output = run_provider(request);
    assert_exit(&output, 0);
    let details = &response(&output)["feedback"]["details"];
    assert_eq!(details["phase"], "finding-ledger");
    assert_eq!(details["status"], "set_mismatch");
    assert!(details["accepted_unresolved"]
        .as_array()
        .expect("accepted-unresolved set")
        .is_empty());
    assert_eq!(
        details["failing_evidence"][0]["statement"],
        "current blocker"
    );
}

#[test]
fn every_checked_reference_route_is_accepted_when_obligations_are_empty() {
    let artifacts = TestDir::new();
    for name in ["intent.json", "design.json", "plan.json"] {
        artifacts.write_json(name, &json!({"revision": "1"}));
    }
    artifacts.write_json("implementation-report.json", &json!({"revision": "1"}));
    artifacts.write_json("validation-report.json", &json!({"revision": "1"}));
    let repository = TestDir::new();
    fs::write(repository.path.join("marker.txt"), b"baseline\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "software-change evaluate"],
        vec!["config", "user.email", "evaluate@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repository.path)
            .status()
            .unwrap()
            .success());
    }
    for phase in ["implementation", "validation"] {
        let output = Command::new(workspace_integration::binary("software-change"))
            .args([
                "checkpoint",
                "--phase",
                phase,
                "--artifact-root",
                artifacts.path.to_str().unwrap(),
                "--working-directory",
                repository.path.to_str().unwrap(),
            ])
            .current_dir(&repository.path)
            .bounded_output("software-change evaluate checkpoint")
            .expect("run checkpoint");
        assert!(output.status.success(), "checkpoint {phase} failed");
    }
    let input = json!({
        "config_version": "none",
        "review_policies": {},
        "artifact_root": artifacts.root_value()
    });
    let routes = [
        ("explore", "intent-ready", "design"),
        ("design", "design-ready", "plan"),
        ("plan", "plan-ready", "implement"),
        ("implement", "implementation-ready", "validation"),
        ("validation", "passed", "end"),
    ];
    for (source, event, target) in routes {
        let output = run_provider_in(
            &repository.path,
            base_request(input.clone(), checked(source, event, target)),
        );
        assert_exit(&output, 0);
        assert_eq!(
            response(&output),
            json!({"result": "allow"}),
            "{source} {event}"
        );
    }
}

#[test]
fn each_single_field_tuple_mismatch_is_unsupported() {
    let initial_input = json!({"config_version": "none", "review_policies": {}});
    let workflow = described_workflow(&initial_input);
    let routes = [
        ("explore", "intent-ready", "design"),
        ("design", "design-ready", "plan"),
        ("plan", "plan-ready", "implement"),
        ("implement", "implementation-ready", "validation"),
        ("validation", "passed", "end"),
    ];
    for (source, event, target) in routes {
        let variants = [
            checked("wrong", event, target),
            checked(source, "wrong", target),
            checked(source, event, "wrong"),
            transition(source, event, target, "check-free"),
        ];
        for transition in variants {
            let output = run_provider(json!({
                "operation": "evaluate",
                "workflow": workflow,
                "initial_input": initial_input,
                "context": [],
                "transition": transition,
                "prior_evaluations": []
            }));
            assert_exit(&output, 0);
            assert_eq!(response(&output), json!({"result": "unsupported"}));
        }
    }
}

#[test]
fn absent_artifact_is_schema_deny() {
    let root = TestDir::new();
    let output = run_provider(base_request(
        config_with_schema("intent.json", Some(&root)),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(value["feedback"]["details"]["violations"][0]["message"]
        .as_str()
        .unwrap()
        .contains("work not yet authored"));
}

#[test]
fn unparseable_artifact_is_schema_deny() {
    let root = TestDir::new();
    root.write_text("intent.json", "not JSON");
    let output = run_provider(base_request(
        config_with_schema("intent.json", Some(&root)),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(value["feedback"]["details"]["violations"][0]["message"]
        .as_str()
        .unwrap()
        .contains("not parseable JSON"));
}

#[test]
fn revision_link_mismatch_is_schema_deny_naming_both_artifacts() {
    let root = TestDir::new();
    let mut config = json!({
        "config_version": "test-1",
        "artifact_root": root.root_value(),
        "review_policies": {},
        "artifact_schemas": {
            "design.json": metadata_schema(),
            "intent.json": metadata_schema()
        },
        "revision_links": [{
            "from": "design.json",
            "field": "intent_revision",
            "to": "intent.json"
        }]
    });
    root.write_json(
        "design.json",
        &json!({
            "revision": "d1",
            "author": {"name": "owner", "kind": "human"},
            "intent_revision": "i1"
        }),
    );
    root.write_json("intent.json", &valid_metadata("i2"));
    // Keep source schema closed-field semantics from hiding link behavior: the
    // source schema in this inline config explicitly permits the link field.
    config["artifact_schemas"]["design.json"]["properties"]["intent_revision"] =
        json!({"type": "string", "minLength": 1});
    let output = run_provider(base_request(
        config,
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    let message = value["feedback"]["details"]["violations"][0]["message"]
        .as_str()
        .unwrap();
    assert!(message.contains("design.json"));
    assert!(message.contains("intent.json"));
}

#[test]
fn inaccessible_artifact_root_is_evaluation_error() {
    let missing_root = Path::new("/tmp/software-change-provider-root-that-does-not-exist-9f5e");
    let mut config = config_with_schema("intent.json", None);
    config["artifact_root"] = json!(missing_root.to_string_lossy().to_string());
    let output = run_provider(base_request(
        config,
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 1);
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact_root"));
}

#[test]
fn malformed_config_classes_exit_one_without_stdout_result() {
    let cases = vec![
        (
            "unknown top-level",
            json!({"config_version": "x", "review_policies": {}, "typo": true}),
        ),
        ("missing config version", json!({"review_policies": {}})),
        (
            "empty config version",
            json!({"config_version": "", "review_policies": {}}),
        ),
        (
            "unknown gate",
            json!({"config_version": "x", "review_policies": {"nope": []}}),
        ),
        (
            "bad required authors",
            json!({
                "config_version": "x",
                "review_policies": {"intent-review": [{"id": "axis", "description": "x", "required_authors": 0}]}
            }),
        ),
        (
            "unknown artifact",
            json!({"config_version": "x", "review_policies": {}, "artifact_schemas": {"nope.json": {"type": "object"}}}),
        ),
        (
            "bad schema keyword",
            json!({"config_version": "x", "review_policies": {}, "artifact_schemas": {"intent.json": {"type": "object", "nope": true}}}),
        ),
        (
            "axes without schema",
            json!({"config_version": "x", "review_policies": {"intent-review": [{"id": "axis", "description": "x"}]}}),
        ),
        (
            "links without schemas",
            json!({"config_version": "x", "review_policies": {}, "revision_links": [{"from": "design.json", "field": "intent_revision", "to": "intent.json"}]}),
        ),
        (
            "malformed link shape",
            json!({"config_version": "x", "review_policies": {}, "revision_links": [{"from": "design.json", "to": "intent.json"}]}),
        ),
    ];
    for (name, config) in cases {
        let workflow = described_workflow(&config);
        let transition = workflow["transitions"]
            .as_array()
            .expect("described transitions")
            .iter()
            .find(|edge| edge["kind"] == "checked")
            .cloned()
            .expect("checked hop");
        let output = run_provider(json!({
            "operation": "evaluate",
            "workflow": workflow,
            "initial_input": config,
            "context": [],
            "transition": transition,
            "prior_evaluations": []
        }));
        assert_exit(&output, 1);
        assert!(output.stdout.is_empty(), "{name} emitted stdout");
    }
}

#[test]
fn check_free_revise_allows_without_evidence_or_finding_ledger() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let output = run_provider(base_request(
        config_with_axis(&root),
        check_free("intent-review", "revise", "explore"),
    ));
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn review_approved_denied_without_current_revision_finding_ledger() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let mut request = base_request(
        config_with_axis(&root),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([passing_evidence()]);
    let output = run_provider(request);
    assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(
        value["feedback"]["code"],
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(value["feedback"]["details"]["status"], "missing");
}

#[test]
fn empty_finding_ledger_array_allows_review_approved() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let mut request = base_request(
        config_with_axis(&root),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        passing_evidence(),
        finding_ledger_record("intent-review", "intent.json", "1", json!([]))
    ]);
    let output = run_provider(request);
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn malformed_finding_ledger_blocks_until_superseded() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let config = config_with_axis(&root);
    let transition = checked("intent-review", "approved", "design");

    let mut malformed =
        finding_ledger_record("intent-review", "intent.json", "1", json!("not-an-array"));
    malformed["sequence"] = json!(2);
    let mut blocked = base_request(config.clone(), transition.clone());
    blocked["context"] = json!([passing_evidence(), malformed.clone()]);
    let blocked_output = run_provider(blocked);
    assert_exit(&blocked_output, 0);
    assert_eq!(
        response(&blocked_output)["feedback"]["details"]["status"],
        "malformed"
    );

    let mut later = finding_ledger_record("intent-review", "intent.json", "1", json!([]));
    later["id"] = json!("accepted-later");
    later["sequence"] = json!(3);
    later["created_at"] = json!(3);
    malformed["id"] = json!("accepted-malformed");
    let mut superseded = base_request(config, transition);
    superseded["context"] = json!([passing_evidence(), malformed, later]);
    let output = run_provider(superseded);
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn finding_ledger_for_prior_revision_does_not_satisfy_after_bump() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("2"));
    let mut request = base_request(
        config_with_axis(&root),
        checked("intent-review", "approved", "design"),
    );
    let mut evidence = passing_evidence();
    evidence["data"]["subject_revision"] = json!("2");
    request["context"] = json!([
        evidence,
        finding_ledger_record("intent-review", "intent.json", "1", json!([]))
    ]);
    let output = run_provider(request);
    assert_exit(&output, 0);
    let details = &response(&output)["feedback"]["details"];
    assert_eq!(details["status"], "stale_subject");
    assert_eq!(details["record_revision"], "1");
    assert_eq!(details["current_revision"], "2");
}

#[test]
fn same_policy_id_on_parent_and_adversarial_gates_aggregates_independently() {
    let root = TestDir::new();
    root.write_json("intent.json", &valid_metadata("1"));
    let config = json!({
        "config_version": "test-1",
        "artifact_root": root.root_value(),
        "review_policies": {
            "intent-review": [{"id": "axis", "description": "parent axis"}],
            "intent-adversarial-review": [{"id": "axis", "description": "adversarial axis"}]
        },
        "artifact_schemas": {"intent.json": metadata_schema()}
    });
    let parent_pass = context_record(json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": "pass",
        "findings": "",
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1"
    }));
    let parent_findings = finding_ledger_record("intent-review", "intent.json", "1", json!([]));
    let adversarial_findings =
        finding_ledger_record("intent-adversarial-review", "intent.json", "1", json!([]));

    let mut parent = base_request(
        config.clone(),
        checked("intent-review", "approved", "intent-adversarial-review"),
    );
    parent["context"] = json!([parent_pass.clone(), parent_findings.clone()]);
    let parent_output = run_provider(parent);
    assert_exit(&parent_output, 0);
    assert_eq!(response(&parent_output), json!({"result": "allow"}));

    let mut adversarial = base_request(
        config,
        checked("intent-adversarial-review", "approved", "design"),
    );
    adversarial["context"] = json!([parent_pass, parent_findings, adversarial_findings]);
    let adversarial_output = run_provider(adversarial);
    assert_exit(&adversarial_output, 0);
    let value = response(&adversarial_output);
    assert_eq!(
        value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert!(value["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|axis| axis["axis"] == "axis"));
}

#[test]
fn pre_8_scalar_intent_acceptance_uses_the_frozen_schema_contract() {
    let root = TestDir::new();
    root.write_json(
        "intent.json",
        &json!({"revision": "i1", "acceptance": ["first"]}),
    );
    let output = run_provider(base_request(
        pre_8_criterion_config(&root),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn pre_8_downstream_schema_does_not_activate_criterion_references() {
    let root = TestDir::new();
    root.write_json(
        "intent.json",
        &json!({"revision": "i1", "acceptance": ["first"]}),
    );
    let config = pre_8_criterion_config(&root);
    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([{
                "acceptance": "first",
                "delivered_by": "part",
                "criterion_id": "AC-1"
            }]),
        ),
    );
    let output = run_provider(base_request(
        config,
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));
}

#[test]
fn intent_criterion_identity_rejects_duplicate_ids_and_schema_malformed_records() {
    let root = TestDir::new();
    let config = criterion_config(&root);
    root.write_json(
        "intent.json",
        &criterion_intent(
            "i1",
            json!([
                {"id": "AC-1", "statement": "first"},
                {"id": "AC-1", "statement": "second"}
            ]),
        ),
    );
    let duplicate = run_provider(base_request(
        config.clone(),
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&duplicate, 0);
    assert_eq!(
        response(&duplicate)["feedback"]["code"],
        "software-change-schema-invalid"
    );
    assert_eq!(
        criterion_violation_rules(&duplicate),
        vec!["criterion-identity"]
    );
    assert!(
        response(&duplicate)["feedback"]["details"]["violations"][0]["message"]
            .as_str()
            .unwrap()
            .contains("duplicate criterion ID")
    );

    root.write_json(
        "intent.json",
        &criterion_intent("i1", json!([{"id": "AC-0", "statement": ""}])),
    );
    let malformed = run_provider(base_request(
        config,
        checked("explore", "intent-ready", "design"),
    ));
    assert_exit(&malformed, 0);
    assert_eq!(
        response(&malformed)["feedback"]["code"],
        "software-change-schema-invalid"
    );
    let rules = criterion_violation_rules(&malformed);
    assert!(rules.contains(&"pattern".to_owned()));
    assert!(rules.contains(&"minLength".to_owned()));
}

#[test]
fn downstream_criterion_references_are_optional_but_current_and_unique() {
    let root = TestDir::new();
    let config = criterion_config(&root);
    root.write_json(
        "intent.json",
        &criterion_intent("i1", json!([{"id": "AC-1", "statement": "first"}])),
    );

    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([{"acceptance": "first", "delivered_by": "part"}]),
        ),
    );
    let optional = run_provider(base_request(
        config.clone(),
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&optional, 0);
    assert_eq!(response(&optional), json!({"result": "allow"}));

    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([{"acceptance": "first", "delivered_by": "part", "criterion_id": "AC-1"}]),
        ),
    );
    let known = run_provider(base_request(
        config.clone(),
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&known, 0);
    assert_eq!(response(&known), json!({"result": "allow"}));

    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([{"acceptance": "malformed", "delivered_by": "part", "criterion_id": "AC-01"}]),
        ),
    );
    let malformed = run_provider(base_request(
        config.clone(),
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&malformed, 0);
    assert!(criterion_violation_rules(&malformed).contains(&"pattern".to_owned()));

    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([
                {"acceptance": "first", "delivered_by": "part", "criterion_id": "AC-1"},
                {"acceptance": "same", "delivered_by": "other", "criterion_id": "AC-1"}
            ]),
        ),
    );
    let duplicate = run_provider(base_request(
        config.clone(),
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&duplicate, 0);
    assert_eq!(
        criterion_violation_rules(&duplicate),
        vec!["criterion-reference"]
    );
    assert!(
        response(&duplicate)["feedback"]["details"]["violations"][0]["message"]
            .as_str()
            .unwrap()
            .contains("duplicate criterion reference")
    );

    root.write_json(
        "design.json",
        &criterion_design(
            "d1",
            "i1",
            json!([{"acceptance": "unknown", "delivered_by": "part", "criterion_id": "AC-2"}]),
        ),
    );
    let unknown = run_provider(base_request(
        config,
        checked("design", "design-ready", "plan"),
    ));
    assert_exit(&unknown, 0);
    assert_eq!(
        criterion_violation_rules(&unknown),
        vec!["criterion-reference"]
    );
    assert!(
        response(&unknown)["feedback"]["details"]["violations"][0]["message"]
            .as_str()
            .unwrap()
            .contains("not present in the current intent")
    );
}
