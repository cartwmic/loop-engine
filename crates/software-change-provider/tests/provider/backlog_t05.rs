use super::bounded_process::run_with_stdin;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

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
    staged_evidence(id, "axis", stage, author, "1", "backlog-t05", sequence)
}

fn staged_evidence(
    id: &str,
    policy_id: &str,
    stage: &str,
    author: &str,
    subject_revision: &str,
    config_version: &str,
    sequence: u64,
) -> Value {
    json!({
        "id": id,
        "kind": "review-evidence",
        "data": {
            "gate": "intent-review",
            "policy_id": policy_id,
            "review_stage": stage,
            "result": "pass",
            "findings": "",
            "author": {"name": author, "kind": "agent"},
            "subject": "intent.json",
            "subject_revision": subject_revision,
            "config_version": config_version
        },
        "sequence": sequence,
        "created_at": sequence
    })
}

fn ledger(sequence: u64) -> Value {
    ledger_at("1", sequence)
}

fn ledger_at(revision: &str, sequence: u64) -> Value {
    json!({
        "id": "ledger",
        "kind": "finding-ledger",
        "data": {
            "schema_version": "1",
            "gate": "intent-review",
            "subject": "intent.json",
            "subject_revision": revision,
            "author": {"name": "driver", "kind": "agent"},
            "findings": []
        },
        "sequence": sequence,
        "created_at": sequence
    })
}

fn evaluate_high_at(root: &Path, context: Vec<Value>) -> Output {
    let input = high_config(root);
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
    evaluate_high_at(&root, context)
}

#[test]
// bookends:LE-130 — this public protocol test asserts separate high-rigor
// stages and independent author identities.
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
// bookends:LE-130 — old aggregate applicability cannot replace fresh aggregate review.
fn backlog_t05_old_aggregate_applicability_cannot_satisfy_corrected_stage() {
    let root = temp_root("old-aggregate-carry");
    fs::write(
        root.join("intent.json"),
        serde_json::to_vec(&json!({
            "revision": "2",
            "author": {"name": "owner", "kind": "human"}
        }))
        .expect("intent JSON"),
    )
    .expect("intent artifact");
    let context = vec![
        staged_evidence(
            "individual-a",
            "axis",
            "individual",
            "reviewer-a",
            "2",
            "backlog-t05",
            1,
        ),
        staged_evidence(
            "individual-b",
            "axis",
            "individual",
            "reviewer-b",
            "2",
            "backlog-t05",
            2,
        ),
        staged_evidence(
            "old-aggregate-a",
            "axis",
            "aggregate",
            "reviewer-a",
            "1",
            "backlog-t05",
            3,
        ),
        staged_evidence(
            "old-aggregate-b",
            "axis",
            "aggregate",
            "reviewer-b",
            "1",
            "backlog-t05",
            4,
        ),
        json!({
            "id": "carry-aggregate-a",
            "kind": "evidence-applicability",
            "data": {
                "origin": {"kind": "context-record", "id": "old-aggregate-a"},
                "target": {"subject": "intent.json", "revision": "2", "checkpoint": null},
                "attesting_driver": {"name": "driver", "kind": "agent"},
                "reason": "old aggregate evidence was incorrectly claimed applicable"
            },
            "sequence": 5,
            "created_at": 5
        }),
        json!({
            "id": "carry-aggregate-b",
            "kind": "evidence-applicability",
            "data": {
                "origin": {"kind": "context-record", "id": "old-aggregate-b"},
                "target": {"subject": "intent.json", "revision": "2", "checkpoint": null},
                "attesting_driver": {"name": "driver", "kind": "agent"},
                "reason": "old aggregate evidence was incorrectly claimed applicable"
            },
            "sequence": 6,
            "created_at": 6
        }),
        ledger_at("2", 7),
    ];
    let output = evaluate_high_at(&root, context);
    assert_eq!(
        output.status.code(),
        Some(0),
        "old aggregate carry: {output:?}"
    );
    assert!(
        output.stderr.is_empty(),
        "old aggregate carry panicked: {output:?}"
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("structured deny response");
    assert_eq!(value["result"], "deny");
    assert_eq!(
        value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert!(value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|axis| {
            axis["review_stage"] == "aggregate"
                && axis["axis"] == "axis"
                && axis["diagnostics"].as_array().is_some_and(|diagnostics| {
                    diagnostics.iter().any(|diagnostic| {
                        diagnostic["category"] == "unverified"
                            && diagnostic
                                .to_string()
                                .contains("aggregate applicability cannot be carried")
                    })
                })
        }));
}

#[test]
// bookends:LE-130 — stage-aware evidence cannot satisfy a different stage or axis.
fn backlog_t05_wrong_stage_axis_applicability_returns_structured_deny() {
    // crates/software-change-provider/docs/prd.md:208, 210, 216 require
    // explicit stage-aware applicability and reject cross-stage coverage.
    let root = temp_root("wrong-stage-axis");
    fs::write(
        root.join("intent.json"),
        serde_json::to_vec(&json!({
            "revision": "1",
            "author": {"name": "owner", "kind": "human"}
        }))
        .expect("intent JSON"),
    )
    .expect("intent artifact");
    let config_version = "backlog-t05-wrong-stage-axis";
    let input = json!({
        "contract_version": 3,
        "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
        "config_version": config_version,
        "artifact_root": root.to_string_lossy().to_string(),
        "review_policies": {
            "intent-review": [
                {"id": "axis", "description": "axis", "review_stage": "individual", "required_authors": 1},
                {"id": "different-aggregate-axis", "description": "axis", "review_stage": "aggregate", "required_authors": 1}
            ]
        },
        "artifact_schemas": {"intent.json": metadata_schema()}
    });
    let workflow = describe(&input);
    let output = provider(json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": input,
        "context": [
            staged_evidence(
                "aggregate-source",
                "axis",
                "aggregate",
                "reviewer-a",
                "1",
                config_version,
                1,
            ),
            {
                "id": "aggregate-applicability",
                "kind": "evidence-applicability",
                "data": {
                    "origin": {"kind": "context-record", "id": "aggregate-source"},
                    "target": {"subject": "intent.json", "revision": "1", "checkpoint": null},
                    "attesting_driver": {"name": "driver", "kind": "agent"},
                    "reason": "retained aggregate evidence was claimed applicable"
                },
                "sequence": 2,
                "created_at": 2
            },
            ledger(3)
        ],
        "transition": {"source": "intent-review", "event": "approved", "target": "design", "kind": "checked"},
        "prior_evaluations": []
    }));
    assert_eq!(
        output.status.code(),
        Some(0),
        "wrong-stage evaluation: {:?}",
        output
    );
    assert!(
        output.stderr.is_empty(),
        "wrong-stage evaluation panicked: {:?}",
        output
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("structured deny response");
    assert_eq!(value["result"], "deny");
    assert_eq!(
        value["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert!(value["feedback"]["details"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|axis| {
            axis["review_stage"] == "individual"
                && axis["axis"] == "axis"
                && axis["diagnostics"].as_array().is_some_and(|diagnostics| {
                    diagnostics.iter().any(|diagnostic| {
                        diagnostic["category"] == "malformed"
                            && diagnostic.to_string().contains("aggregate")
                    })
                })
        }));
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

// bookends:LE-131 — this public provider evaluation rejects a current plan
// task without criterion_ids with a structured criterion-reference diagnostic.
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

struct Ac40Binaries {
    engine: PathBuf,
    provider: PathBuf,
}

fn ac40_engine_call(
    binaries: &Ac40Binaries,
    database: &Path,
    cwd: &Path,
    args: &[String],
) -> Value {
    let output = Command::new(&binaries.engine)
        .current_dir(cwd)
        .arg("--database")
        .arg(database)
        .arg("--json")
        .args(args)
        .output()
        .expect("engine process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "engine output is not JSON: {error}; args={args:?}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success() || value["status"] == "rejected",
        "engine command failed: args={args:?}, value={value}, stderr={:?}",
        output.stderr
    );
    value
}

fn ac40_show(binaries: &Ac40Binaries, database: &Path, repository: &Path, run_id: &str) -> Value {
    ac40_engine_call(
        binaries,
        database,
        repository,
        &["show", "--view", "full", run_id]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
}

fn ac40_event(
    binaries: &Ac40Binaries,
    database: &Path,
    repository: &Path,
    run_id: &str,
    event: &str,
) -> Value {
    let _ = ac40_show(binaries, database, repository, run_id);
    ac40_engine_call(
        binaries,
        database,
        repository,
        &["event", run_id, event]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
}

fn ac40_append(
    binaries: &Ac40Binaries,
    database: &Path,
    repository: &Path,
    run_id: &str,
    kind: &str,
    record_id: &str,
    data: &Value,
) -> Value {
    let _ = ac40_show(binaries, database, repository, run_id);
    let args = vec![
        "append".to_owned(),
        run_id.to_owned(),
        "--kind".to_owned(),
        kind.to_owned(),
        "--record-id".to_owned(),
        record_id.to_owned(),
        serde_json::to_string(data).expect("record JSON"),
    ];
    let result = ac40_engine_call(binaries, database, repository, &args);
    assert_eq!(result["status"], "completed", "append failed: {result}");
    result
}

fn ac40_wait_for_invocation(
    binaries: &Ac40Binaries,
    database: &Path,
    repository: &Path,
    run_id: &str,
    invocation_id: &str,
) -> Value {
    for _ in 0..300 {
        let shown = ac40_show(binaries, database, repository, run_id);
        let invocation = shown["result"]["work_slot_invocations"]
            .as_array()
            .expect("invocations")
            .iter()
            .find(|row| row["invocation_id"] == invocation_id);
        if let Some(invocation) = invocation {
            if matches!(
                invocation["status"].as_str(),
                Some("succeeded") | Some("failed")
            ) && invocation
                .get("completed_at")
                .is_some_and(|value| !value.is_null())
            {
                return shown;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("invocation {invocation_id} did not complete");
}

fn ac40_review_candidates(binaries: &Ac40Binaries, show: &Value) -> Vec<Value> {
    let bytes = serde_json::to_vec(show).expect("show JSON");
    let mut command = Command::new(&binaries.provider);
    command.arg("review-candidates");
    let output = run_with_stdin(
        &mut command,
        "software-change backlog_t05 AC40 candidates",
        &bytes,
    )
    .expect("review-candidates process")
    .output;
    assert!(
        output.status.success(),
        "review-candidates failed: stdout={:?}; stderr={:?}",
        output.stdout,
        output.stderr
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("candidate JSON");
    value["candidates"]
        .as_array()
        .expect("candidate array")
        .clone()
}

fn ac40_subject_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["revision", "author", "affected"],
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "affected": {"type": "string", "enum": ["needs-correction", "corrected"]},
            "author": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                },
                "required": ["name", "kind"],
                "additionalProperties": false
            }
        }
    })
}

fn ac40_worker_schema(stage: &str, author: &str, axes: &[&str]) -> Value {
    let axis_values = axes.iter().map(|axis| json!(axis)).collect::<Vec<_>>();
    let all_of = axes
        .iter()
        .map(|axis| {
            json!({
                "contains": {
                    "type": "object",
                    "required": ["axis"],
                    "properties": {"axis": {"const": axis}}
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["review_stage", "author", "judgments"],
        "properties": {
            "review_stage": {"type": "string", "const": stage},
            "author": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "const": "agent"}
                },
                "const": {"name": author, "kind": "agent"}
            },
            "judgments": {
                "type": "array",
                "minItems": axes.len(),
                "maxItems": axes.len(),
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["axis", "result", "findings"],
                    "properties": {
                        "axis": {"type": "string", "enum": axis_values},
                        "result": {"type": "string", "enum": ["pass", "fail"]},
                        "findings": {"type": "string"}
                    },
                    "oneOf": [
                        {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
                        {"properties": {"result": {"const": "fail"}, "findings": {"minLength": 1}}}
                    ]
                },
                "allOf": all_of
            }
        }
    })
}

fn ac40_worker_script() -> &'static str {
    r##"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

packet = sys.stdin.read()
def value(prefix, default):
    return next((line[len(prefix):] for line in packet.splitlines() if line.startswith(prefix)), default)

location = json.loads(packet.split("---", 1)[0].strip().splitlines()[-1])
stage = value("review_stage: ", "aggregate")
author = value("required_author_claim: ", sys.argv[1])
policies = json.loads(value("assigned_policies: ", "[]"))
root = Path(location["artifact_root"])
artifact = json.loads((root / "intent.json").read_text(encoding="utf-8"))
revision = artifact["revision"]
affected = artifact["affected"]
log = Path(sys.argv[2])
with log.open("a", encoding="utf-8") as stream:
    stream.write(json.dumps({
        "stage": stage,
        "author": author,
        "revision": revision,
        "affected": affected,
        "axes": [policy["id"] for policy in policies],
    }) + "\n")

judgments = []
for policy in policies:
    axis = policy["id"]
    failing_initial_individual = (
        stage == "individual" and axis == "affected" and affected == "needs-correction"
    )
    judgments.append({
        "axis": axis,
        "result": "fail" if failing_initial_individual else "pass",
        "findings": "affected requires correction" if failing_initial_individual else "",
    })
print(json.dumps({
    "review_stage": stage,
    "author": {"name": author, "kind": "agent"},
    "judgments": judgments,
}, separators=(",", ":")))
"##
}

fn ac40_write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("worker script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("worker metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("worker permissions");
    }
}

fn ac40_write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON bytes")).expect("JSON file");
}

fn ac40_worker(script: &Path, log: &Path, author: &str, stage: &str, axes: &[&str]) -> Value {
    let policies = axes
        .iter()
        .map(|axis| json!({"id": axis, "description": format!("{axis} axis")}))
        .collect::<Vec<_>>();
    json!({
        "command": script,
        "args": [author, log],
        "preamble": format!(
            "FROZEN REVIEW ASSIGNMENT\nreview_stage: {stage}\nassigned_policies: {}\nrequired_author_claim: {author}\n",
            serde_json::to_string(&policies).expect("policy JSON")
        ),
        "full_output_schema": ac40_worker_schema(stage, author, axes)
    })
}

fn ac40_record_id(prefix: &str, candidate: &Value) -> String {
    format!(
        "{prefix}-{}-{}-{}",
        candidate["review_stage"].as_str().expect("candidate stage"),
        candidate["author"]["name"]
            .as_str()
            .expect("candidate author"),
        candidate["axis"].as_str().expect("candidate axis")
    )
}

#[allow(clippy::too_many_arguments)]
fn ac40_append_candidate(
    binaries: &Ac40Binaries,
    database: &Path,
    repository: &Path,
    run_id: &str,
    candidate: &Value,
    revision: &str,
    config_version: &str,
    prefix: &str,
) -> String {
    assert_eq!(
        candidate["status"], "ready",
        "unexpected candidate: {candidate}"
    );
    let record_id = ac40_record_id(prefix, candidate);
    let data = json!({
        "gate": "intent-review",
        "policy_id": candidate["axis"],
        "review_stage": candidate["review_stage"],
        "result": candidate["result"],
        "findings": candidate["findings"],
        "author": candidate["author"],
        "subject": "intent.json",
        "subject_revision": revision,
        "config_version": config_version,
        "origin": candidate["origin"]
    });
    ac40_append(
        binaries,
        database,
        repository,
        run_id,
        "review-evidence",
        &record_id,
        &data,
    );
    record_id
}

fn ac40_append_applicability(
    binaries: &Ac40Binaries,
    database: &Path,
    repository: &Path,
    run_id: &str,
    record_id: &str,
    source_id: &str,
    revision: &str,
) {
    ac40_append(
        binaries,
        database,
        repository,
        run_id,
        "evidence-applicability",
        record_id,
        &json!({
            "origin": {"kind": "context-record", "id": source_id},
            "target": {"subject": "intent.json", "revision": revision, "checkpoint": null},
            "attesting_driver": {"name": "backlog_t05-driver", "kind": "agent"},
            "reason": "unaffected individual evidence remains applicable to corrected intent"
        }),
    );
}

fn ac40_finding(source_id: &str, status: &str, reason: &str) -> Value {
    json!({
        "id": "F-ac40-affected",
        "source": {"kind": "context-record", "id": source_id},
        "policy_id": "affected",
        "statement": "affected requires correction",
        "disposition": "accepted",
        "reason": reason,
        "owner_phase": "intent",
        "task_ids": [],
        "review_axes": ["affected"],
        "status": status
    })
}

fn ac40_ledger(revision: &str, findings: Value) -> Value {
    json!({
        "schema_version": "1",
        "gate": "intent-review",
        "subject": "intent.json",
        "subject_revision": revision,
        "author": {"name": "backlog_t05-driver", "kind": "agent"},
        "findings": findings
    })
}

#[test]
// bookends:LE-130 — this public run proves high-rigor stage coverage,
// independent authors, affected-axis re-review, and fresh aggregate review.
fn backlog_t05_high_correction_requires_fresh_aggregate_and_carries_only_unaffected_individuals() {
    let binaries = Ac40Binaries {
        engine: workspace_integration::binary("loop-engine"),
        provider: workspace_integration::binary("software-change"),
    };
    let root = temp_root("high-correction");
    let repository = root.join("repository");
    fs::create_dir_all(&repository).expect("repository");
    fs::write(repository.join("marker.txt"), "baseline\n").expect("marker");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "backlog_t05"],
        vec!["config", "user.email", "backlog_t05@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(Command::new("git")
            .current_dir(&repository)
            .args(args)
            .status()
            .expect("git process")
            .success());
    }

    let script = root.join("review-worker.py");
    let log_a = root.join("reviewer-a.jsonl");
    let log_b = root.join("reviewer-b.jsonl");
    ac40_write_executable(&script, ac40_worker_script());
    let workers = [
        ac40_worker(&script, &log_a, "reviewer-a", "individual", &["affected"]),
        ac40_worker(&script, &log_a, "reviewer-a", "individual", &["unaffected"]),
        ac40_worker(&script, &log_b, "reviewer-b", "individual", &["affected"]),
        ac40_worker(&script, &log_b, "reviewer-b", "individual", &["unaffected"]),
        ac40_worker(
            &script,
            &log_a,
            "reviewer-a",
            "aggregate",
            &["affected", "unaffected"],
        ),
        ac40_worker(
            &script,
            &log_b,
            "reviewer-b",
            "aggregate",
            &["affected", "unaffected"],
        ),
    ];
    let mut fan_out_args = vec![
        "fan-out".to_owned(),
        "--max-active".to_owned(),
        "2".to_owned(),
    ];
    for (index, worker) in workers.iter().enumerate() {
        if index == 4 {
            fan_out_args.push("--then".to_owned());
        }
        fan_out_args.push("--worker".to_owned());
        fan_out_args.push(serde_json::to_string(worker).expect("worker JSON"));
    }
    let config_version = "backlog-t05-ac40-high";
    let profile = root.join("profile.json");
    ac40_write_json(
        &profile,
        &json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": config_version,
            "review_policies": {
                "intent-review": [
                    {"id": "affected", "description": "affected", "review_stage": "individual", "required_authors": 2},
                    {"id": "unaffected", "description": "unaffected", "review_stage": "individual", "required_authors": 2},
                    {"id": "affected", "description": "affected", "review_stage": "aggregate", "required_authors": 2},
                    {"id": "unaffected", "description": "unaffected", "review_stage": "aggregate", "required_authors": 2}
                ]
            },
            "artifact_schemas": {"intent.json": ac40_subject_schema()},
            "work_slot_bindings": {
                "intent-review": {
                    "command": binaries.engine.to_string_lossy(),
                    "args": fan_out_args,
                    "context_filter": {"command": binaries.provider.to_string_lossy(), "args": ["commission"]}
                }
            }
        }),
    );
    let providers = root.join("providers.toml");
    fs::write(
        &providers,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            binaries.provider.to_string_lossy()
        ),
    )
    .expect("provider catalog");
    let database = root.join("run.sqlite");
    let run_id = "backlog-t05-high-correction";
    let started = ac40_engine_call(
        &binaries,
        &database,
        &repository,
        &[
            "--config".to_owned(),
            providers.to_string_lossy().into_owned(),
            "start".to_owned(),
            "--id".to_owned(),
            run_id.to_owned(),
            "software-change".to_owned(),
            format!("@{}", profile.display()),
            "backlog t05 high correction".to_owned(),
        ],
    );
    assert_eq!(started["status"], "completed", "start: {started}");
    let artifact_root = PathBuf::from(
        started["result"]["run"]["initial_input"]["artifact_root"]
            .as_str()
            .expect("artifact root"),
    );
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let intent_path = artifact_root.join("intent.json");
    let initial_intent = json!({
        "revision": "1",
        "author": {"name": "owner", "kind": "human"},
        "affected": "needs-correction"
    });
    ac40_write_json(&intent_path, &initial_intent);
    let initial_intent_bytes = fs::read(&intent_path).expect("initial intent bytes");

    assert_eq!(
        ac40_event(&binaries, &database, &repository, run_id, "intent-ready")["result"]["run"]
            ["current_state"],
        "intent-review"
    );
    let _ = ac40_show(&binaries, &database, &repository, run_id);
    let initial_invoke = ac40_engine_call(
        &binaries,
        &database,
        &repository,
        &[
            "invoke".to_owned(),
            run_id.to_owned(),
            "intent-review".to_owned(),
        ],
    );
    assert_eq!(
        initial_invoke["status"], "completed",
        "initial invoke: {initial_invoke}"
    );
    let initial_invocation_id = initial_invoke["result"]["invocation_id"]
        .as_str()
        .expect("initial invocation ID")
        .to_owned();
    let initial_show = ac40_wait_for_invocation(
        &binaries,
        &database,
        &repository,
        run_id,
        &initial_invocation_id,
    );
    let initial_invocation = initial_show["result"]["work_slot_invocations"]
        .as_array()
        .expect("initial invocations")
        .iter()
        .find(|row| row["invocation_id"] == initial_invocation_id)
        .expect("initial invocation");
    assert_eq!(
        initial_invocation["status"], "succeeded",
        "initial invocation"
    );
    let initial_candidates = ac40_review_candidates(&binaries, &initial_show)
        .into_iter()
        .filter(|candidate| candidate["origin"]["id"] == initial_invocation_id)
        .collect::<Vec<_>>();
    assert_eq!(initial_candidates.len(), 8, "initial candidate rows");
    assert_eq!(
        initial_candidates
            .iter()
            .filter(|candidate| candidate["review_stage"] == "individual")
            .count(),
        4
    );
    assert_eq!(
        initial_candidates
            .iter()
            .filter(|candidate| candidate["review_stage"] == "aggregate")
            .count(),
        4
    );
    assert_eq!(
        initial_candidates
            .iter()
            .filter(|candidate| {
                candidate["review_stage"] == "individual"
                    && candidate["axis"] == "affected"
                    && candidate["result"] == "fail"
            })
            .count(),
        2,
        "the uncorrected affected field must produce two individual findings"
    );
    let affected_failure_source = initial_candidates
        .iter()
        .find(|candidate| {
            candidate["review_stage"] == "individual"
                && candidate["author"]["name"] == "reviewer-a"
                && candidate["axis"] == "affected"
        })
        .map(|candidate| ac40_record_id("initial", candidate))
        .expect("affected failure source");
    for candidate in initial_candidates
        .iter()
        .filter(|candidate| candidate["review_stage"] == "individual")
    {
        ac40_append_candidate(
            &binaries,
            &database,
            &repository,
            run_id,
            candidate,
            "1",
            config_version,
            "initial",
        );
    }
    ac40_append(
        &binaries,
        &database,
        &repository,
        run_id,
        "finding-ledger",
        "initial-empty-ledger",
        &ac40_ledger("1", json!([])),
    );
    let missing_initial_aggregate =
        ac40_event(&binaries, &database, &repository, run_id, "approved");
    assert_eq!(missing_initial_aggregate["status"], "rejected");
    assert_eq!(
        missing_initial_aggregate["code"],
        "software-change-review-incomplete"
    );
    assert!(missing_initial_aggregate
        .to_string()
        .contains("\"review_stage\":\"aggregate\""));
    assert!(missing_initial_aggregate
        .to_string()
        .contains("\"category\":\"missing\""));

    for candidate in initial_candidates
        .iter()
        .filter(|candidate| candidate["review_stage"] == "aggregate")
    {
        ac40_append_candidate(
            &binaries,
            &database,
            &repository,
            run_id,
            candidate,
            "1",
            config_version,
            "initial",
        );
    }
    ac40_append(
        &binaries,
        &database,
        &repository,
        run_id,
        "finding-ledger",
        "initial-unresolved-ledger",
        &ac40_ledger(
            "1",
            json!([ac40_finding(
                &affected_failure_source,
                "unresolved",
                "retain the material finding until the intent is corrected"
            )]),
        ),
    );
    let unresolved = ac40_event(&binaries, &database, &repository, run_id, "approved");
    assert_eq!(
        unresolved["status"], "rejected",
        "unresolved finding must block"
    );
    assert_eq!(unresolved["code"], "software-change-finding-ledger-invalid");
    assert!(unresolved.to_string().contains("accepted_unresolved"));

    let revised = ac40_event(&binaries, &database, &repository, run_id, "revise");
    assert_eq!(revised["status"], "completed", "revise: {revised}");
    assert_eq!(revised["result"]["run"]["current_state"], "explore");

    let corrected_intent = json!({
        "revision": "2",
        "author": {"name": "owner", "kind": "human"},
        "affected": "corrected"
    });
    ac40_write_json(&intent_path, &corrected_intent);
    let corrected_intent_bytes = fs::read(&intent_path).expect("corrected intent bytes");
    assert_ne!(
        initial_intent_bytes, corrected_intent_bytes,
        "correction must change the subject bytes"
    );
    assert_eq!(corrected_intent["revision"], "2");
    assert_eq!(corrected_intent["affected"], "corrected");
    let ready_again = ac40_event(&binaries, &database, &repository, run_id, "intent-ready");
    assert_eq!(
        ready_again["status"], "completed",
        "intent-ready correction: {ready_again}"
    );
    assert_eq!(
        ready_again["result"]["run"]["current_state"],
        "intent-review"
    );

    for author in ["reviewer-a", "reviewer-b"] {
        let source = format!("initial-individual-{author}-unaffected");
        ac40_append_applicability(
            &binaries,
            &database,
            &repository,
            run_id,
            &format!("carry-individual-{author}"),
            &source,
            "2",
        );
    }
    ac40_append(
        &binaries,
        &database,
        &repository,
        run_id,
        "finding-ledger",
        "corrected-unresolved-ledger",
        &ac40_ledger(
            "2",
            json!([ac40_finding(
                &affected_failure_source,
                "unresolved",
                "the corrected subject is reviewed before this finding is resolved"
            )]),
        ),
    );

    let selected_ids = "worker-0,worker-2,worker-4,worker-5";
    let _ = ac40_show(&binaries, &database, &repository, run_id);
    let corrected_invoke = ac40_engine_call(
        &binaries,
        &database,
        &repository,
        &[
            "invoke".to_owned(),
            run_id.to_owned(),
            "intent-review".to_owned(),
            "--assignments".to_owned(),
            selected_ids.to_owned(),
        ],
    );
    assert_eq!(
        corrected_invoke["status"], "completed",
        "corrected invoke: {corrected_invoke}"
    );
    let corrected_invocation_id = corrected_invoke["result"]["invocation_id"]
        .as_str()
        .expect("corrected invocation ID")
        .to_owned();
    let corrected_show = ac40_wait_for_invocation(
        &binaries,
        &database,
        &repository,
        run_id,
        &corrected_invocation_id,
    );
    let corrected_invocation = corrected_show["result"]["work_slot_invocations"]
        .as_array()
        .expect("corrected invocations")
        .iter()
        .find(|row| row["invocation_id"] == corrected_invocation_id)
        .expect("corrected invocation");
    assert_eq!(
        corrected_invocation["status"], "succeeded",
        "corrected invocation"
    );
    assert_eq!(
        corrected_invocation["inner_workers"]
            .as_array()
            .expect("corrected workers")
            .iter()
            .map(|worker| worker["assignment_id"].as_str().expect("assignment ID"))
            .collect::<Vec<_>>(),
        selected_ids.split(',').collect::<Vec<_>>()
    );
    let corrected_candidates = ac40_review_candidates(&binaries, &corrected_show)
        .into_iter()
        .filter(|candidate| candidate["origin"]["id"] == corrected_invocation_id)
        .collect::<Vec<_>>();
    assert_eq!(corrected_candidates.len(), 6, "corrected candidate rows");
    assert_eq!(
        corrected_candidates
            .iter()
            .filter(|candidate| candidate["review_stage"] == "individual")
            .count(),
        2
    );
    assert_eq!(
        corrected_candidates
            .iter()
            .filter(|candidate| candidate["review_stage"] == "aggregate")
            .count(),
        4
    );
    assert_eq!(
        corrected_candidates
            .iter()
            .filter(|candidate| {
                candidate["review_stage"] == "individual"
                    && candidate["axis"] == "affected"
                    && candidate["result"] == "pass"
            })
            .count(),
        2,
        "the corrected affected field must clear both selected individual findings"
    );
    for candidate in corrected_candidates
        .iter()
        .filter(|candidate| candidate["review_stage"] == "individual")
    {
        ac40_append_candidate(
            &binaries,
            &database,
            &repository,
            run_id,
            candidate,
            "2",
            config_version,
            "corrected-individual",
        );
    }
    for candidate in corrected_candidates
        .iter()
        .filter(|candidate| candidate["review_stage"] == "aggregate")
    {
        ac40_append_candidate(
            &binaries,
            &database,
            &repository,
            run_id,
            candidate,
            "2",
            config_version,
            "corrected-aggregate",
        );
    }

    let unresolved_after_review = ac40_event(&binaries, &database, &repository, run_id, "approved");
    assert_eq!(
        unresolved_after_review["status"], "rejected",
        "accepted unresolved finding must block after fresh selected reviews"
    );
    assert_eq!(
        unresolved_after_review["code"],
        "software-change-finding-ledger-invalid"
    );
    assert!(unresolved_after_review
        .to_string()
        .contains("accepted_unresolved"));
    assert!(unresolved_after_review
        .to_string()
        .contains(&affected_failure_source));

    ac40_append(
        &binaries,
        &database,
        &repository,
        run_id,
        "finding-ledger",
        "corrected-resolved-ledger",
        &ac40_ledger(
            "2",
            json!([ac40_finding(
                &affected_failure_source,
                "resolved",
                "the corrected affected field was verified by fresh review"
            )]),
        ),
    );
    let approved = ac40_event(&binaries, &database, &repository, run_id, "approved");
    assert_eq!(
        approved["status"], "completed",
        "corrected approval: {approved}"
    );
    assert_eq!(approved["result"]["run"]["current_state"], "design");

    let logs = [log_a, log_b]
        .into_iter()
        .flat_map(|path| {
            fs::read_to_string(path)
                .expect("worker log")
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).expect("worker log JSON"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(logs.len(), 10, "initial six plus corrected four workers");
    let initial_logs = logs
        .iter()
        .filter(|log| log["revision"] == "1")
        .collect::<Vec<_>>();
    assert_eq!(initial_logs.len(), 6);
    assert!(initial_logs
        .iter()
        .all(|log| log["affected"] == "needs-correction"));
    let corrected_logs = logs
        .iter()
        .filter(|log| log["revision"] == "2")
        .collect::<Vec<_>>();
    assert_eq!(corrected_logs.len(), 4);
    assert!(corrected_logs
        .iter()
        .all(|log| log["affected"] == "corrected"));
    assert_eq!(
        corrected_logs
            .iter()
            .filter(|log| log["stage"] == "individual")
            .count(),
        2
    );
    assert!(corrected_logs
        .iter()
        .filter(|log| log["stage"] == "individual")
        .all(|log| log["axes"] == json!(["affected"])));
    assert_eq!(
        corrected_logs
            .iter()
            .filter(|log| log["stage"] == "aggregate")
            .count(),
        2
    );
    assert!(corrected_logs
        .iter()
        .filter(|log| log["stage"] == "aggregate")
        .all(|log| log["axes"] == json!(["affected", "unaffected"])));
}
