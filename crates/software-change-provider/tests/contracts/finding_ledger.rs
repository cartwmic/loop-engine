use super::bounded_process::CommandExt;
use super::support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::process::Command;
use support::{
    axis_config, base_request, checked, config_artifact_root, context_json, invoke, invoke_in_dir,
    load_fixture, load_profile, provider_binary, response, TestDir,
};

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn source(id: &str) -> Value {
    json!({
        "kind": "context-record",
        "id": format!("source-{id}"),
    })
}

fn finding(_root: &TestDir, id: &str, statement: &str) -> Value {
    json!({
        "id": id,
        "source": source(id),
        "policy_id": "axis",
        "statement": statement,
        "disposition": "rejected",
        "reason": "driver triage rejected the candidate",
        "owner_phase": null,
        "task_ids": [],
        "review_axes": [],
        "status": "recorded"
    })
}

fn ledger(gate: &str, subject: &str, revision: &str, findings: Value) -> Value {
    json!({
        "schema_version": "1",
        "gate": gate,
        "subject": subject,
        "subject_revision": revision,
        "author": {"name": "driver", "kind": "agent"},
        "findings": findings
    })
}

fn pass_evidence() -> Value {
    json!({
        "gate": "intent-review",
        "policy_id": "axis",
        "result": "pass",
        "findings": "",
        "author": {"name": "reviewer", "kind": "agent"},
        "subject": "intent.json",
        "subject_revision": "1",
        "config_version": "test-1"
    })
}

fn evaluate(root: &TestDir, mut records: Vec<Value>) -> Value {
    let mut source_records = Vec::new();
    let mut source_ids = std::collections::BTreeSet::new();
    for record in &records {
        let Some(data) = record["data"].as_object() else {
            continue;
        };
        let Some(findings) = data.get("findings").and_then(Value::as_array) else {
            continue;
        };
        let gate = data["gate"].as_str().unwrap_or("intent-review");
        let subject = data["subject"].as_str().unwrap_or("intent.json");
        let revision = data["subject_revision"].as_str().unwrap_or("1");
        for finding in findings {
            let Some(source_id) = finding["source"]["id"].as_str() else {
                continue;
            };
            if !source_id.starts_with("source-") || !source_ids.insert(source_id.to_owned()) {
                continue;
            }
            let statement = finding["statement"].as_str().unwrap_or("candidate");
            let accepted_unresolved =
                finding["disposition"] == "accepted" && finding["status"] == "unresolved";
            source_records.push(json!({
                "id": source_id,
                "kind": "review-evidence",
                "data": {
                    "gate": gate,
                    "policy_id": finding["policy_id"],
                    "result": if accepted_unresolved { "fail" } else { "pass" },
                    "findings": if accepted_unresolved { statement } else { "" },
                    "author": {"name": "reviewer", "kind": "agent"},
                    "subject": subject,
                    "subject_revision": revision,
                    "config_version": "test-1"
                },
                "sequence": 0,
                "created_at": 0
            }));
        }
    }
    records.splice(0..0, source_records);
    let mut request = base_request(
        axis_config(root, "axis"),
        checked("intent-review", "approved", "design"),
    );
    request["context"] = Value::Array(records);
    let output = invoke(request);
    support::assert_exit(&output, 0);
    response(&output)
}

fn ledger_record(sequence: u64, data: Value) -> Value {
    context_json("finding-ledger", data, sequence)
}

#[test]
fn malformed_closed_shape_and_illegal_combinations_fail_closed() {
    let root = TestDir::new("ledger-malformed");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );

    let mut unknown = ledger("intent-review", "intent.json", "1", json!([]));
    unknown["extra"] = json!(true);
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(2, unknown),
        ],
    );
    assert_eq!(
        result["feedback"]["code"],
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");

    let mut illegal = finding(&root, "F-illegal", "candidate");
    illegal["disposition"] = json!("rejected");
    illegal["status"] = json!("unresolved");
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([illegal])),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
}

#[test]
fn duplicate_and_changed_stable_ids_fail_closed() {
    let root = TestDir::new("ledger-continuity");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );

    let duplicate = finding(&root, "F-duplicate", "same");
    let mut duplicate_again = duplicate.clone();
    duplicate_again["reason"] = json!("a different driver reason");
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger(
                    "intent-review",
                    "intent.json",
                    "1",
                    json!([duplicate, duplicate_again]),
                ),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");

    let first = ledger(
        "intent-review",
        "intent.json",
        "1",
        json!([finding(&root, "F-stable", "old")]),
    );
    let second = ledger(
        "intent-review",
        "intent.json",
        "1",
        json!([finding(&root, "F-stable", "new")]),
    );
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(2, first),
            ledger_record(3, second),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
}

#[test]
fn omitted_accepted_unresolved_finding_requires_explicit_disposition() {
    let root = TestDir::new("ledger-omitted-unresolved");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );

    let mut accepted = finding(&root, "F-one", "candidate");
    accepted["disposition"] = json!("accepted");
    accepted["status"] = json!("unresolved");
    accepted["owner_phase"] = json!("design");
    accepted["review_axes"] = json!(["axis"]);
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([accepted])),
            ),
            ledger_record(3, ledger("intent-review", "intent.json", "1", json!([]))),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
    assert!(result["feedback"]["details"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason.as_str().unwrap().contains("F-one")));

    // A new subject revision starts a new latest-snapshot view; it must not be
    // mistaken for an omitted same-revision disposition.
    root.write_json(
        "intent.json",
        &json!({"revision": "2", "author": {"name": "owner", "kind": "human"}}),
    );
    let mut revised_pass = pass_evidence();
    revised_pass["subject_revision"] = json!("2");
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", revised_pass, 1),
            ledger_record(
                2,
                ledger(
                    "intent-review",
                    "intent.json",
                    "1",
                    json!([{
                        "id": "F-two",
                        "source": source("F-two"),
                        "policy_id": "axis",
                        "statement": "candidate",
                        "disposition": "accepted",
                        "reason": "driver accepted the candidate",
                        "owner_phase": "design",
                        "task_ids": [],
                        "review_axes": ["axis"],
                        "status": "unresolved"
                    }]),
                ),
            ),
            ledger_record(3, ledger("intent-review", "intent.json", "2", json!([]))),
        ],
    );
    assert_eq!(result, json!({"result": "allow"}));
}

#[test]
fn reappearing_subject_revision_requires_explicit_disposition() {
    let root = TestDir::new("ledger-reappearing-revision-omission");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );

    let mut accepted = finding(&root, "F-one", "candidate");
    accepted["disposition"] = json!("accepted");
    accepted["status"] = json!("unresolved");
    accepted["owner_phase"] = json!("design");
    accepted["review_axes"] = json!(["axis"]);
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([accepted])),
            ),
            ledger_record(3, ledger("intent-review", "intent.json", "2", json!([]))),
            ledger_record(4, ledger("intent-review", "intent.json", "1", json!([]))),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
    assert!(result["feedback"]["details"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason.as_str().unwrap().contains("F-one")));
}

#[test]
fn reappearing_subject_revision_accepts_explicit_resolved_or_stale_disposition() {
    for (status, owner_phase) in [("resolved", "design"), ("stale", "plan")] {
        let root = TestDir::new(&format!("ledger-reappearing-revision-{status}"));
        root.write_json(
            "intent.json",
            &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
        );

        let mut unresolved = finding(&root, "F-one", "candidate");
        unresolved["disposition"] = json!("accepted");
        unresolved["status"] = json!("unresolved");
        unresolved["owner_phase"] = json!("design");
        unresolved["review_axes"] = json!(["axis"]);
        let mut dispositioned = unresolved.clone();
        dispositioned["status"] = json!(status);
        dispositioned["owner_phase"] = json!(owner_phase);
        let result = evaluate(
            &root,
            vec![
                context_json("review-evidence", pass_evidence(), 1),
                ledger_record(
                    2,
                    ledger("intent-review", "intent.json", "1", json!([unresolved])),
                ),
                ledger_record(3, ledger("intent-review", "intent.json", "2", json!([]))),
                ledger_record(
                    4,
                    ledger("intent-review", "intent.json", "1", json!([dispositioned])),
                ),
            ],
        );
        assert_eq!(result, json!({"result": "allow"}));
    }
}

#[test]
fn stale_subject_unknown_identifiers_and_raw_digest_fail_closed() {
    let root = TestDir::new("ledger-freshness");
    root.write_json(
        "intent.json",
        &json!({"revision": "2", "author": {"name": "owner", "kind": "human"}}),
    );

    let stale = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(2, ledger("intent-review", "intent.json", "1", json!([]))),
        ],
    );
    assert_eq!(stale["feedback"]["details"]["status"], "stale_subject");

    root.write_json("plan.json", &json!({"tasks": [{"id": "known"}]}));
    let mut unknown = finding(&root, "F-unknown", "candidate");
    unknown["disposition"] = json!("accepted");
    unknown["status"] = json!("unresolved");
    unknown["owner_phase"] = json!("implementation");
    unknown["task_ids"] = json!(["missing"]);
    unknown["review_axes"] = json!(["missing-axis"]);
    let unknown_result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "2", json!([unknown])),
            ),
        ],
    );
    assert_eq!(unknown_result["feedback"]["details"]["status"], "malformed");

    let mut bad_digest = ledger(
        "intent-review",
        "intent.json",
        "2",
        json!([finding(&root, "F-digest", "candidate")]),
    );
    bad_digest["findings"][0]["source"]["output_sha256"] =
        json!("sha256:1111111111111111111111111111111111111111111111111111111111111111");
    let digest_result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(2, bad_digest),
        ],
    );
    assert_eq!(digest_result["feedback"]["details"]["status"], "malformed");
}

#[test]
fn missing_finding_source_context_is_rejected_before_progression() {
    let root = TestDir::new("ledger-missing-source");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let mut missing = finding(&root, "F-missing", "candidate");
    missing["source"] = json!({"kind": "context-record", "id": "missing-evidence"});
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([missing])),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
    assert!(result["feedback"]["details"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason.as_str().unwrap().contains("missing context record")));
}

#[test]
fn legacy_verbose_finding_source_is_rejected() {
    let root = TestDir::new("ledger-attempts");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let worker = root.path().join("work-slot-captures/intent-review/inv-1/0");
    fs::create_dir_all(worker.join("attempts/1")).unwrap();
    fs::create_dir_all(worker.join("attempts/2")).unwrap();
    fs::write(worker.join("attempts/1/stdout"), b"selected").unwrap();
    fs::write(worker.join("attempts/1/stderr"), b"").unwrap();
    fs::write(worker.join("attempts/2/stdout"), b"other").unwrap();
    fs::write(worker.join("attempts/2/stderr"), b"").unwrap();
    let manifest = json!({
        "schema_version": "1",
        "attempts": [
            {"number": 1, "stdout_sha256": digest(b"selected"), "stderr_sha256": digest(b""), "validation_errors": []},
            {"number": 2, "stdout_sha256": digest(b"other"), "stderr_sha256": digest(b""), "validation_errors": ["bad"]}
        ],
        "selected_attempt": 1,
        "exhausted": false
    });
    fs::write(
        worker.join("attempts.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let source = json!({
        "kind": "work-slot",
        "invocation_id": "inv-1",
        "worker_index": 0,
        "attempt": 2,
        "output_sha256": digest(b"other")
    });
    let mut rejected = finding(&root, "F-attempt", "candidate");
    rejected["source"] = source;
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([rejected])),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");

    let mut selected = finding(&root, "F-selected", "candidate");
    selected["source"] = json!({
        "kind": "work-slot",
        "invocation_id": "inv-1",
        "worker_index": 0,
        "attempt": 1,
        "output_sha256": digest(b"selected")
    });
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger(
                    "intent-review",
                    "intent.json",
                    "1",
                    json!([selected.clone()]),
                ),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");

    // The old per-finding attempt/digest coordinate is not a second source
    // path, even when its capture bytes happen to be present.
    fs::write(worker.join("attempts/2/stdout"), b"tampered").unwrap();
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([selected])),
            ),
        ],
    );
    assert_eq!(result["feedback"]["details"]["status"], "malformed");
}

#[test]
fn report_ledger_derives_current_checkpoint_instead_of_copying_state() {
    let root = TestDir::new("ledger-checkpoint");
    for (subject, fixture) in [
        ("intent.json", "intent-good.json"),
        ("design.json", "design-good.json"),
        ("plan.json", "plan-good.json"),
        (
            "implementation-report.json",
            "implementation-report-good.json",
        ),
        ("validation-report.json", "validation-report-good.json"),
    ] {
        root.write_json(subject, &load_fixture(fixture));
    }
    let repository = root.path().join("repository");
    fs::create_dir_all(&repository).unwrap();
    fs::write(repository.join("marker.txt"), b"baseline\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "software-change test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repository)
            .status()
            .unwrap()
            .success());
    }
    let checkpoint = Command::new(provider_binary())
        .args([
            "checkpoint",
            "--phase",
            "implementation",
            "--artifact-root",
            root.path().to_str().unwrap(),
            "--working-directory",
            repository.to_str().unwrap(),
        ])
        .current_dir(&repository)
        .bounded_output("software-change finding-ledger checkpoint")
        .unwrap();
    assert!(
        checkpoint.status.success(),
        "checkpoint stderr: {:?}",
        checkpoint.stderr
    );
    let config = config_artifact_root(load_profile("high-rigor"), &root);
    let mut request = base_request(
        config,
        checked(
            "implementation-review",
            "approved",
            "implementation-adversarial-review",
        ),
    );
    let mut context = Vec::new();
    for (sequence, axis) in [
        "tasks-actually-done",
        "no-scope-creep",
        "design-faithful-final",
    ]
    .into_iter()
    .enumerate()
    {
        context.push(context_json(
            "review-evidence",
            support::evidence(
                "implementation-review",
                axis,
                "pass",
                "",
                &format!("reviewer-{axis}"),
                "agent",
                "implementation-report.json",
                "r15",
                "high-rigor-7",
            ),
            (sequence + 1) as u64,
        ));
    }
    let report_ledger = ledger(
        "implementation-review",
        "implementation-report.json",
        "r15",
        json!([]),
    );
    context.push(context_json("finding-ledger", report_ledger, 10));
    request["context"] = Value::Array(context);
    let output = invoke_in_dir(request.clone(), &repository);
    support::assert_exit(&output, 0);
    assert_eq!(response(&output), json!({"result": "allow"}));

    fs::write(repository.join("marker.txt"), b"changed\n").unwrap();
    // Do not regenerate the checkpoint: its immutable repository identity is
    // the evidence that the report/ledger is stale after this edit.
    let output = invoke_in_dir(request, &repository);
    support::assert_exit(&output, 0);
    let value = response(&output);
    assert_eq!(
        value["feedback"]["code"],
        "software-change-checkpoint-invalid"
    );
    assert_eq!(value["feedback"]["details"]["phase"], "checkpoint");
}

#[test]
fn advisory_proposal_is_inert_until_driver_appends_a_ledger() {
    let root = TestDir::new("ledger-proposal-inert");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            context_json(
                "advisory-finding-proposal",
                json!({
                    "schema_version": "1",
                    "proposals": [{
                        "candidate_source_ids": ["source-1"],
                        "proposed_disposition": "accepted",
                        "proposed_reason": "suggestion",
                        "proposed_owner_phase": "implementation",
                        "proposed_task_ids": ["task-a"],
                        "proposed_review_axes": ["axis"],
                        "rationale": "advisory only"
                    }]
                }),
                2,
            ),
        ],
    );
    assert_eq!(
        result["feedback"]["code"],
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(result["feedback"]["details"]["status"], "missing");
}

#[test]
fn accepted_unresolved_entries_must_match_current_failing_evidence() {
    let root = TestDir::new("ledger-set-agreement");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );

    let mut accepted = finding(&root, "F-accepted", "candidate");
    accepted["disposition"] = json!("accepted");
    accepted["status"] = json!("unresolved");
    accepted["owner_phase"] = json!("design");
    accepted["review_axes"] = json!(["axis"]);
    let mut fail = pass_evidence();
    fail["result"] = json!("fail");
    fail["findings"] = json!("candidate");

    let matching = evaluate(
        &root,
        vec![
            context_json("review-evidence", fail, 1),
            ledger_record(
                2,
                ledger(
                    "intent-review",
                    "intent.json",
                    "1",
                    json!([accepted.clone()]),
                ),
            ),
        ],
    );
    assert_eq!(
        matching["feedback"]["code"],
        "software-change-review-incomplete"
    );
    assert_eq!(matching["feedback"]["details"]["phase"], "evidence");

    let mut pass = pass_evidence();
    pass["findings"] = json!("");
    let mismatch = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass, 1),
            ledger_record(
                2,
                ledger("intent-review", "intent.json", "1", json!([accepted])),
            ),
        ],
    );
    assert_eq!(
        mismatch["feedback"]["code"],
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(mismatch["feedback"]["details"]["status"], "set_mismatch");
}

#[test]
fn rejected_advisory_resolved_and_stale_entries_have_empty_blocking_set() {
    let root = TestDir::new("ledger-dispositions");
    root.write_json(
        "intent.json",
        &json!({"revision": "1", "author": {"name": "owner", "kind": "human"}}),
    );
    let rejected = finding(&root, "F-rejected", "rejected");
    let mut advisory = finding(&root, "F-advisory", "advisory");
    advisory["disposition"] = json!("advisory");
    advisory["status"] = json!("stale");
    let mut resolved = finding(&root, "F-resolved", "resolved");
    resolved["disposition"] = json!("accepted");
    resolved["status"] = json!("resolved");
    resolved["owner_phase"] = json!("design");
    let mut stale = finding(&root, "F-stale", "stale");
    stale["disposition"] = json!("accepted");
    stale["status"] = json!("stale");
    stale["owner_phase"] = json!("plan");
    let result = evaluate(
        &root,
        vec![
            context_json("review-evidence", pass_evidence(), 1),
            ledger_record(
                2,
                ledger(
                    "intent-review",
                    "intent.json",
                    "1",
                    json!([rejected, advisory, resolved, stale]),
                ),
            ),
        ],
    );
    assert_eq!(result, json!({"result": "allow"}));
}
