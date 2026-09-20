//! Public contract-v3 validation-stage regression coverage.

use super::bounded_process::run_with_stdin;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "software-change-backlog-t05-validation-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("temporary root");
    root
}

fn provider() -> PathBuf {
    workspace_integration::binary("software-change")
}

fn engine() -> PathBuf {
    workspace_integration::binary("loop-engine")
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON bytes")).expect("write JSON");
}

fn engine_call(database: &Path, cwd: &Path, args: &[String]) -> Value {
    let output = Command::new(engine())
        .current_dir(cwd)
        .arg("--database")
        .arg(database)
        .arg("--json")
        .args(args)
        .output()
        .expect("engine process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "engine output is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "engine command failed: args={args:?}, value={value}, stderr={:?}",
        output.stderr
    );
    value
}

fn show(database: &Path, cwd: &Path, run_id: &str) -> Value {
    engine_call(
        database,
        cwd,
        &["show".into(), "--view".into(), "full".into(), run_id.into()],
    )
}

fn observe_event(database: &Path, cwd: &Path, run_id: &str, event: &str) -> Value {
    let _ = show(database, cwd, run_id);
    engine_call(
        database,
        cwd,
        &["event".into(), run_id.into(), event.into()],
    )
}

fn append(database: &Path, cwd: &Path, run_id: &str, kind: &str, id: &str, data: &Value) {
    let _ = show(database, cwd, run_id);
    let result = engine_call(
        database,
        cwd,
        &[
            "append".into(),
            run_id.into(),
            "--kind".into(),
            kind.into(),
            "--record-id".into(),
            id.into(),
            serde_json::to_string(data).expect("record JSON"),
        ],
    );
    assert_eq!(result["status"], "completed", "append failed: {result}");
}

fn validation_schema() -> Value {
    let path = workspace_integration::package_root("software-change-provider")
        .join("data/validation-report-schema.json");
    serde_json::from_slice(&fs::read(path).expect("validation report schema"))
        .expect("validation report schema JSON")
}

fn worker_schema(stage: &str, author: &str, include_verdicts: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["review_stage", "author", "judgments"],
        "properties": {
            "review_stage": {"type": "string", "enum": ["individual", "aggregate"], "const": stage},
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
                "minItems": 1,
                "maxItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["axis", "result", "findings"],
                    "properties": {
                        "axis": {"type": "string", "const": "delivery"},
                        "result": {"type": "string", "const": "pass"},
                        "findings": {"type": "string", "const": ""}
                    }
                },
                "allOf": [{"contains": {
                    "type": "object",
                    "required": ["axis"],
                    "properties": {"axis": {"const": "delivery"}}
                }}]
            }
        }
    });
    if include_verdicts {
        schema["required"]
            .as_array_mut()
            .expect("schema required")
            .push(json!("validation_verdicts"));
        schema["properties"]["validation_verdicts"] = json!({
            "type": "array",
            "minItems": 1,
            "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["record_id", "kind", "data"],
                "properties": {
                    "record_id": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "enum": ["criterion-verdict", "goal-verdict"]},
                    "data": {"type": "object"}
                }
            }
        });
    }
    schema
}

fn worker_script() -> &'static str {
    r##"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

packet = sys.stdin.read()
location = json.loads(packet.split("---", 1)[0].strip().splitlines()[-1])
stage = sys.argv[2]
author = {"name": sys.argv[1], "kind": "agent"}
log = Path(sys.argv[3])
context = location.get("context", [])
with log.open("a", encoding="utf-8") as stream:
    stream.write(json.dumps({
        "stage": stage,
        "author": author,
        "review_evidence_ids": [
            record["id"] for record in context
            if record.get("kind") == "review-evidence"
        ],
    }) + "\n")

result = {
    "review_stage": stage,
    "author": author,
    "judgments": [{"axis": "delivery", "result": "pass", "findings": ""}],
}
if stage == "aggregate":
    root = Path(location["artifact_root"])
    report = json.loads((root / "validation-report.json").read_text(encoding="utf-8"))
    author_number = 0 if author["name"].endswith("a") else 1
    verdicts = []
    for criterion in report["criteria"]:
        record_id = criterion["verdict_ids"][author_number]
        verdicts.append({
            "record_id": record_id,
            "kind": "criterion-verdict",
            "data": {
                "criterion_id": criterion["criterion_id"],
                "subject": "validation-report.json",
                "subject_revision": report["revision"],
                "checkpoint": "validation-checkpoint.json",
                "author": author,
                "result": "pass",
                "findings": [],
                "evidence_context_ids": report["command_evidence_ids"],
            },
        })
    record_id = report["goal_verdict_ids"][author_number]
    verdicts.append({
        "record_id": record_id,
        "kind": "goal-verdict",
        "data": {
            "subject": "validation-report.json",
            "subject_revision": report["revision"],
            "checkpoint": "validation-checkpoint.json",
            "author": author,
            "result": "pass",
            "findings": [],
            "evidence_context_ids": report["command_evidence_ids"],
        },
    })
    result["validation_verdicts"] = verdicts
print(json.dumps(result, separators=(",", ":")))
"##
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("worker script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("worker metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("worker permissions");
    }
}

fn worker(author: &str, stage: &str, script: &Path, log: &Path) -> Value {
    json!({
        "command": script.to_string_lossy(),
        "args": [author, stage, log.to_string_lossy()],
        "preamble": "backlog_t05 validation worker",
        "full_output_schema": worker_schema(stage, author, stage == "aggregate")
    })
}

fn provider_output(mut command: Command, input: &Value) -> Output {
    command.stdin(Stdio::piped());
    let bytes = serde_json::to_vec(input).expect("provider input JSON");
    run_with_stdin(
        &mut command,
        "software-change backlog_t05 validation",
        &bytes,
    )
    .expect("provider process")
    .output
}

fn checkpoint(root: &Path, repository: &Path, phase: &str) {
    let output = Command::new(provider())
        .current_dir(repository)
        .args([
            "checkpoint",
            "--phase",
            phase,
            "--artifact-root",
            root.to_str().expect("artifact root"),
            "--working-directory",
            repository.to_str().expect("repository"),
        ])
        .output()
        .expect("checkpoint process");
    assert!(
        output.status.success(),
        "{phase} checkpoint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn backlog_t05_high_validation_uses_aggregate_only_criterion_rows() {
    let root = temp_root();
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

    let script = root.join("validation-worker.py");
    write_executable(&script, worker_script());
    let log_a = root.join("reviewer-a.jsonl");
    let log_b = root.join("reviewer-b.jsonl");
    let config_version = "backlog-t05-high-validation";
    let workers = [
        worker("reviewer-a", "individual", &script, &log_a),
        worker("reviewer-b", "individual", &script, &log_b),
        worker("reviewer-a", "aggregate", &script, &log_a),
        worker("reviewer-b", "aggregate", &script, &log_b),
    ];
    let mut fan_out_args = vec!["fan-out".into(), "--max-active".into(), "2".into()];
    for (index, worker) in workers.iter().enumerate() {
        if index == 2 {
            fan_out_args.push("--then".into());
        }
        fan_out_args.push("--worker".into());
        fan_out_args.push(serde_json::to_string(worker).expect("worker JSON"));
    }

    let profile = root.join("profile.json");
    write_json(
        &profile,
        &json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 2, "goal_required_authors": 2},
            "config_version": config_version,
            "review_policies": {
                "validation-review": [
                    {"id": "delivery", "description": "delivery", "review_stage": "individual", "required_authors": 2},
                    {"id": "delivery", "description": "delivery", "review_stage": "aggregate", "required_authors": 2}
                ],
                "validation-adversarial-review": [
                    {"id": "delivery", "description": "challenge", "review_stage": "aggregate", "required_authors": 2}
                ]
            },
            "artifact_schemas": {"validation-report.json": validation_schema()},
            "work_slot_bindings": {
                "validation-review": {
                    "command": engine().to_string_lossy(),
                    "args": fan_out_args
                }
            }
        }),
    );
    let providers = root.join("providers.toml");
    fs::write(
        &providers,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            provider().to_string_lossy()
        ),
    )
    .expect("provider catalog");
    let database = root.join("run.sqlite");
    let run_id = "backlog-t05-high-validation";
    let started = engine_call(
        &database,
        &repository,
        &[
            "--config".into(),
            providers.to_string_lossy().into_owned(),
            "start".into(),
            "--id".into(),
            run_id.into(),
            "software-change".into(),
            format!("@{}", profile.display()),
            "high validation".into(),
        ],
    );
    let artifact_root = PathBuf::from(
        started["result"]["run"]["initial_input"]["artifact_root"]
            .as_str()
            .expect("artifact root"),
    );
    fs::create_dir_all(&artifact_root).expect("allocated artifact root");
    write_json(
        &artifact_root.join("intent.json"),
        &json!({
            "revision": "intent-1",
            "author": {"name": "owner", "kind": "human"},
            "acceptance": [{"id": "AC-1", "statement": "the named proof succeeds"}]
        }),
    );
    write_json(
        &artifact_root.join("design.json"),
        &json!({"revision": "design-1", "author": {"name": "owner", "kind": "human"}}),
    );
    write_json(
        &artifact_root.join("plan.json"),
        &json!({
            "revision": "plan-1",
            "author": {"name": "owner", "kind": "human"},
            "tasks": [],
            "proof_commands": [{
                "id": "proof",
                "command": "python3",
                "args": ["-c", "print('backlog_t05 proof')"],
                "owner": "driver",
                "obligation": "the retained proof command succeeds"
            }]
        }),
    );
    write_json(
        &artifact_root.join("implementation-report.json"),
        &json!({
            "revision": "implementation-1",
            "author": {"name": "owner", "kind": "human"}
        }),
    );
    write_json(
        &artifact_root.join("reconciliation.json"),
        &super::reconciliation_fixture(),
    );

    for event in ["intent-ready", "design-ready", "plan-ready"] {
        let result = observe_event(&database, &repository, run_id, event);
        assert_eq!(result["status"], "completed", "{event}: {result}");
    }
    checkpoint(&artifact_root, &repository, "implementation");
    let result = observe_event(&database, &repository, run_id, "implementation-ready");
    assert_eq!(
        result["status"], "completed",
        "implementation-ready: {result}"
    );
    let result = observe_event(&database, &repository, run_id, "reconciliation-ready");
    assert_eq!(
        result["status"], "completed",
        "reconciliation-ready: {result}"
    );

    let validation_show = show(&database, &repository, run_id);
    let validation = provider_output(
        {
            let mut command = Command::new(provider());
            command.current_dir(&repository).args([
                "run-validation",
                "--engine",
                engine().to_str().expect("engine"),
                "--working-directory",
                repository.to_str().expect("repository"),
                "--revision",
                "validation-1",
            ]);
            command
        },
        &validation_show,
    );
    assert_eq!(
        validation.status.code(),
        Some(0),
        "validation: {validation:?}"
    );
    let validation_result: Value =
        serde_json::from_slice(&validation.stdout).expect("validation JSON");
    assert_eq!(validation_result["commands_passed"], true);
    let report: Value = serde_json::from_slice(
        &fs::read(artifact_root.join("validation-report.json")).expect("validation report"),
    )
    .expect("validation report JSON");
    let prechosen_ids = report["criteria"][0]["verdict_ids"]
        .as_array()
        .expect("criterion IDs")
        .iter()
        .chain(
            report["goal_verdict_ids"]
                .as_array()
                .expect("goal IDs")
                .iter(),
        )
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let before = show(&database, &repository, run_id);
    let before_context = before["result"]["context"].as_array().expect("context");
    assert!(prechosen_ids.iter().all(|id| {
        !before_context
            .iter()
            .any(|record| record["id"].as_str() == Some(id))
    }));
    for candidate in validation_result["command_candidates"]
        .as_array()
        .expect("command candidates")
    {
        append(
            &database,
            &repository,
            run_id,
            candidate["kind"].as_str().expect("command kind"),
            candidate["record_id"].as_str().expect("command ID"),
            &candidate["data"],
        );
    }
    checkpoint(&artifact_root, &repository, "validation");
    let result = observe_event(&database, &repository, run_id, "validation-ready");
    assert_eq!(result["status"], "completed", "validation-ready: {result}");

    let _ = show(&database, &repository, run_id);
    let invoked = engine_call(
        &database,
        &repository,
        &["invoke".into(), run_id.into(), "validation-review".into()],
    );
    let invocation_id = invoked["result"]["invocation_id"]
        .as_str()
        .expect("invocation ID")
        .to_owned();
    let terminal = (0..300)
        .find_map(|_| {
            let current = show(&database, &repository, run_id);
            let invocation = current["result"]["work_slot_invocations"]
                .as_array()
                .expect("invocations")
                .iter()
                .find(|row| row["invocation_id"] == invocation_id)
                .cloned()?;
            if matches!(
                invocation["status"].as_str(),
                Some("succeeded") | Some("failed")
            ) && invocation
                .get("completed_at")
                .is_some_and(|value| !value.is_null())
            {
                Some((current, invocation))
            } else {
                thread::sleep(Duration::from_millis(50));
                None
            }
        })
        .expect("validation review invocation completion");
    assert_eq!(
        terminal.1["status"], "succeeded",
        "invocation: {}",
        terminal.1
    );

    let candidate_output = provider_output(
        {
            let mut command = Command::new(provider());
            command.arg("review-candidates");
            command
        },
        &terminal.0,
    );
    assert_eq!(
        candidate_output.status.code(),
        Some(0),
        "candidates: {candidate_output:?}"
    );
    let candidates: Value =
        serde_json::from_slice(&candidate_output.stdout).expect("candidate JSON");
    let candidates = candidates["candidates"].as_array().expect("candidates");
    let verdicts = candidates
        .iter()
        .filter(|candidate| candidate["status"] == "verdict-ready")
        .collect::<Vec<_>>();
    assert_eq!(
        verdicts.len(),
        4,
        "two aggregate criterion/goal sets: {candidates:?}"
    );
    assert!(candidates
        .iter()
        .filter(|candidate| candidate["status"] == "ready")
        .all(
            |candidate| candidate["origin"]["assignment_id"] == "worker-0"
                || candidate["origin"]["assignment_id"] == "worker-1"
                || candidate["origin"]["assignment_id"] == "worker-2"
                || candidate["origin"]["assignment_id"] == "worker-3"
        ));
    assert!(verdicts.iter().all(|candidate| {
        candidate["origin"]["assignment_id"] == "worker-2"
            || candidate["origin"]["assignment_id"] == "worker-3"
    }));

    for candidate in candidates {
        match candidate["status"].as_str() {
            Some("ready") => {
                let data = json!({
                    "gate": "validation-review",
                    "policy_id": candidate["axis"],
                    "review_stage": candidate["review_stage"],
                    "result": candidate["result"],
                    "findings": candidate["findings"],
                    "author": candidate["author"],
                    "subject": "validation-report.json",
                    "subject_revision": report["revision"],
                    "config_version": config_version,
                    "origin": candidate["origin"]
                });
                append(
                    &database,
                    &repository,
                    run_id,
                    "review-evidence",
                    &format!("axis-{}", candidate["origin"]["assignment_id"]),
                    &data,
                );
            }
            Some("verdict-ready") => {
                let mut data = candidate["data"].clone();
                data["origin"] = candidate["origin"].clone();
                append(
                    &database,
                    &repository,
                    run_id,
                    candidate["kind"].as_str().expect("verdict kind"),
                    candidate["record_id"].as_str().expect("verdict ID"),
                    &data,
                );
            }
            other => panic!("unexpected candidate status {other:?}: {candidate}"),
        }
    }
    append(
        &database,
        &repository,
        run_id,
        "finding-ledger",
        "validation-ledger",
        &json!({
            "schema_version": "1",
            "gate": "validation-review",
            "subject": "validation-report.json",
            "subject_revision": report["revision"],
            "author": {"name": "driver", "kind": "agent"},
            "findings": []
        }),
    );
    let approved = observe_event(&database, &repository, run_id, "approved");
    assert_eq!(
        approved["status"], "completed",
        "validation approval: {approved}"
    );
    assert_eq!(
        approved["result"]["run"]["current_state"],
        "validation-adversarial-review"
    );

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
    assert_eq!(logs.len(), 4);
    assert!(logs
        .iter()
        .filter(|row| row["stage"] == "individual")
        .all(|row| row["review_evidence_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)));
    assert!(logs
        .iter()
        .filter(|row| row["stage"] == "aggregate")
        .all(|row| row["review_evidence_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)));
}
