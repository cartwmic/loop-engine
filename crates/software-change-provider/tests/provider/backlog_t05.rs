use super::bounded_process::run_with_stdin;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn provider(request: Value) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    let bytes = serde_json::to_vec(&request).expect("request JSON");
    run_with_stdin(&mut command, "software-change backlog_t05", &bytes)
        .expect("provider process")
        .output
}

fn temp_root(label: &str) -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "software-change-backlog-t05-{label}-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary root");
    path
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
                    "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                },
                "required": ["name", "kind"],
                "additionalProperties": false
            }
        },
        "required": ["revision", "author"],
        "additionalProperties": false
    })
}

fn high_config(root: &Path) -> Value {
    json!({
        "contract_version": 3,
        "criterion_policy": {"required_authors": 2, "goal_required_authors": 2},
        "config_version": "backlog-t05",
        "artifact_root": root.to_string_lossy().to_string(),
        "review_policies": {
            "intent-review": [
                {"id": "axis", "description": "axis", "review_stage": "individual", "required_authors": 2},
                {"id": "axis", "description": "axis", "review_stage": "aggregate", "required_authors": 2}
            ]
        },
        "artifact_schemas": {"intent.json": metadata_schema()}
    })
}

fn describe(input: &Value) -> Value {
    let output = provider(json!({"operation": "describe", "initial_input": input}));
    assert_eq!(output.status.code(), Some(0), "describe: {:?}", output);
    serde_json::from_slice(&output.stdout).expect("describe JSON")
}

fn evidence(id: &str, stage: &str, author: &str, sequence: u64) -> Value {
    json!({
        "id": id,
        "kind": "review-evidence",
        "data": {
            "gate": "intent-review",
            "policy_id": "axis",
            "review_stage": stage,
            "result": "pass",
            "findings": "",
            "author": {"name": author, "kind": "agent"},
            "subject": "intent.json",
            "subject_revision": "1",
            "config_version": "backlog-t05"
        },
        "sequence": sequence,
        "created_at": sequence
    })
}

fn ledger(sequence: u64) -> Value {
    json!({
        "id": "ledger",
        "kind": "finding-ledger",
        "data": {
            "schema_version": "1",
            "gate": "intent-review",
            "subject": "intent.json",
            "subject_revision": "1",
            "author": {"name": "driver", "kind": "agent"},
            "findings": []
        },
        "sequence": sequence,
        "created_at": sequence
    })
}

fn evaluate_high(context: Vec<Value>) -> Output {
    let root = temp_root("stages");
    fs::write(
        root.join("intent.json"),
        serde_json::to_vec(&json!({
            "revision": "1",
            "author": {"name": "owner", "kind": "human"}
        }))
        .expect("intent JSON"),
    )
    .expect("intent artifact");
    let input = high_config(&root);
    let workflow = describe(&input);
    provider(json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": input,
        "context": context,
        "transition": {"source": "intent-review", "event": "approved", "target": "design", "kind": "checked"},
        "prior_evaluations": []
    }))
}

#[test]
fn backlog_t05_high_requires_both_stages_and_same_author_identities() {
    let missing = evaluate_high(vec![
        evidence("individual-a", "individual", "reviewer-a", 1),
        evidence("individual-b", "individual", "reviewer-b", 2),
        ledger(3),
    ]);
    assert_eq!(missing.status.code(), Some(0), "missing: {:?}", missing);
    let missing_value: Value = serde_json::from_slice(&missing.stdout).expect("missing response");
    assert_eq!(missing_value["result"], "deny");
    assert!(missing_value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|axis| axis["review_stage"] == "aggregate"));

    let mixed = evaluate_high(vec![
        evidence("individual-a", "individual", "reviewer-a", 1),
        evidence("individual-b", "individual", "reviewer-b", 2),
        evidence("aggregate-a", "aggregate", "reviewer-a", 3),
        evidence("aggregate-c", "aggregate", "reviewer-c", 4),
        ledger(5),
    ]);
    let mixed_value: Value = serde_json::from_slice(&mixed.stdout).expect("mixed response");
    assert_eq!(mixed_value["result"], "deny");
    assert!(mixed_value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|axis| axis["diagnostics"]
            .as_array()
            .expect("axis diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["category"] == "mixed_authors")));

    let good = evaluate_high(vec![
        evidence("individual-a", "individual", "reviewer-a", 1),
        evidence("individual-b", "individual", "reviewer-b", 2),
        evidence("aggregate-a", "aggregate", "reviewer-a", 3),
        evidence("aggregate-b", "aggregate", "reviewer-b", 4),
        ledger(5),
    ]);
    assert_eq!(good.status.code(), Some(0), "good: {:?}", good);
    assert_eq!(
        serde_json::from_slice::<Value>(&good.stdout).unwrap(),
        json!({"result": "allow"})
    );
}

#[test]
fn backlog_t05_v2_checked_evaluation_is_explicitly_unsupported() {
    let input = json!({
        "contract_version": 2,
        "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
        "config_version": "historical-v2",
        "review_policies": {}
    });
    let workflow = describe(&input);
    let output = provider(json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": input,
        "context": [],
        "transition": {"source": "explore", "event": "intent-ready", "target": "design", "kind": "checked"},
        "prior_evaluations": []
    }));
    assert_eq!(output.status.code(), Some(0), "unsupported: {:?}", output);
    assert_eq!(output.stdout, br#"{"result":"unsupported"}"#);
}

#[test]
fn backlog_t05_v3_requires_current_nonempty_plan_task_criteria() {
    let root = temp_root("task-criteria");
    fs::write(
        root.join("intent.json"),
        serde_json::to_vec(&json!({
            "revision": "intent-1",
            "acceptance": [{"id": "AC-1", "statement": "observable outcome"}]
        }))
        .expect("intent JSON"),
    )
    .expect("intent artifact");
    let input = json!({
        "contract_version": 3,
        "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
        "config_version": "task-criteria-v3",
        "artifact_root": root.to_string_lossy().to_string(),
        "review_policies": {},
        "artifact_schemas": {
            "intent.json": {
                "type": "object",
                "properties": {
                    "revision": {"type": "string", "minLength": 1},
                    "acceptance": {"type": "array", "minItems": 1, "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "minLength": 1},
                            "statement": {"type": "string", "minLength": 1}
                        },
                        "required": ["id", "statement"],
                        "additionalProperties": false
                    }}
                },
                "required": ["revision", "acceptance"],
                "additionalProperties": false
            },
            "plan.json": {
                "type": "object",
                "properties": {
                    "revision": {"type": "string", "minLength": 1},
                    "intent_revision": {"type": "string", "minLength": 1},
                    "tasks": {"type": "array", "minItems": 1, "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "minLength": 1},
                            "criterion_ids": {"type": "array", "minItems": 1, "items": {"type": "string", "minLength": 1}}
                        },
                        "required": ["id"],
                        "additionalProperties": false
                    }}
                },
                "required": ["revision", "intent_revision", "tasks"],
                "additionalProperties": false
            }
        },
        "revision_links": [{"from": "plan.json", "field": "intent_revision", "to": "intent.json"}]
    });
    fs::write(
        root.join("plan.json"),
        serde_json::to_vec(&json!({
            "revision": "plan-1",
            "intent_revision": "intent-1",
            "tasks": [{"id": "task-without-criteria"}]
        }))
        .expect("plan JSON"),
    )
    .expect("plan artifact");
    let workflow = describe(&input);
    let output = provider(json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": input,
        "context": [],
        "transition": {"source": "plan", "event": "plan-ready", "target": "implement", "kind": "checked"},
        "prior_evaluations": []
    }));
    assert_eq!(output.status.code(), Some(0), "task criteria: {:?}", output);
    let value: Value = serde_json::from_slice(&output.stdout).expect("task criteria response");
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(value["feedback"]["details"]["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .any(|violation| violation["rule"] == "criterion-reference"));
}
