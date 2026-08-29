use super::bounded_process::CommandExt;
use loop_core::{
    AppendContextRequest, CompleteWorkSlotInvocationRequest, CreateRunRequest,
    CreateWorkSlotInvocationRequest, InnerWorker, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WaiterWrittenStatus, WorkSlot, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "stable-reference-workflow",
        "start",
        vec![State::new("start", "Start", "work", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")])
}

fn selected_worker(assignment: &str, path: &Path) -> InnerWorker {
    let mut worker = InnerWorker::new("/bin/worker", vec!["--review".to_owned()], 0);
    worker.assignment_id = assignment.to_owned();
    worker.selected_attempt = Some(2);
    worker.selected_output_sha256 = Some("sha256:selected-output".to_owned());
    worker.selected_output_path = Some(path.to_string_lossy().into_owned());
    worker.declared_output_contract = Some(json!({"type": "object"}));
    worker
}

fn outputless_worker(assignment: &str) -> InnerWorker {
    let mut worker = InnerWorker::new("/bin/worker", vec!["--review".to_owned()], 0);
    worker.assignment_id = assignment.to_owned();
    worker
}

fn create_run(persistence: &SqlitePersistence, run_id: &str) {
    persistence
        .create_run(CreateRunRequest::new(
            run_id,
            None,
            workflow(),
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            json!({
                "policy": "v1",
                "work_slot_bindings": {
                    "review": {"command": "/bin/worker", "args": ["--review"]}
                }
            }),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            None,
        ))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
    persistence
        .set_current_slot_subject(&run_id.into(), &"review".into(), "subject-v1".to_owned())
        .expect("set subject");
}

fn create_completed_invocation(
    persistence: &SqlitePersistence,
    run_id: &str,
    invocation_id: &str,
    capture_dir: &Path,
    workers: Vec<InnerWorker>,
) {
    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                run_id,
                invocation_id,
                "review",
                WorkSlotBinding::new("/bin/worker", vec!["--review".to_owned()]),
                "instruction-v1",
                "subject-v1",
                0,
                Timestamp::from_unix_millis(3),
                1_000,
                capture_dir.to_string_lossy(),
            )
            .with_frozen_run_identity(json!({
                "provider": {"identity": "provider-v1"},
                "input": {
                    "policy": "v1",
                    "work_slot_bindings": {
                        "review": {"command": "/bin/worker", "args": ["--review"]}
                    }
                }
            })),
        )
        .expect("create invocation");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(4),
            workers,
        ))
        .expect("complete invocation");
}

fn cli(database: &Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database path"),
            "--json",
        ])
        .args(args)
        .bounded_output("loop-engine stable references")
        .expect("loop-engine");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    (output.status.code().unwrap_or(-1), value, stdout)
}

fn show(database: &Path) -> Value {
    let (_, value, _) = cli(database, &["show", "stable-run"]);
    value
}

#[test]
fn append_resolves_selected_assignment_from_durable_state_only() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let capture = directory.path().join("capture");
    create_run(
        &SqlitePersistence::open(&database).expect("sqlite"),
        "stable-run",
    );
    let persistence = SqlitePersistence::open(&database).expect("reopen sqlite");
    create_completed_invocation(
        &persistence,
        "stable-run",
        "invocation-1",
        &capture,
        vec![selected_worker(
            "axis-a",
            &capture.join("axis-a/attempts/2/stdout"),
        )],
    );

    let (_, linked, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"axis":"axis-a","result":"pass","findings":"","origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"axis-a"}}"#,
        ],
    );
    assert_eq!(linked["status"], "completed");
    let context = &linked["result"]["context"];
    assert_eq!(context["data"]["origin"]["id"], "invocation-1");
    assert_eq!(context["data"]["origin"]["assignment_id"], "axis-a");
    assert_eq!(
        context["data"]["loop_engine_origin"],
        json!({
            "invocation_id": "invocation-1",
            "assignment_id": "axis-a",
            "selected_attempt": 2,
            "selected_output_sha256": "sha256:selected-output",
            "selected_output_path": capture.join("axis-a/attempts/2/stdout").to_string_lossy(),
            "capture_dir": capture.to_string_lossy(),
            "command": "/bin/worker",
            "args": ["--review"],
            "binding": {"command": "/bin/worker", "args": ["--review"]}
        })
    );
}

#[test]
fn append_rejects_missing_cross_run_unknown_and_outputless_origins() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    let capture = directory.path().join("capture");
    create_completed_invocation(
        &persistence,
        "stable-run",
        "invocation-1",
        &capture,
        vec![selected_worker(
            "axis-a",
            &capture.join("axis-a/attempts/2/stdout"),
        )],
    );
    create_completed_invocation(
        &persistence,
        "stable-run",
        "invocation-empty",
        &directory.path().join("empty-capture"),
        vec![outputless_worker("axis-empty")],
    );

    let cases = [
        (
            "missing-assignment",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-1"}}"#,
        ),
        (
            "unknown-invocation",
            r#"{"origin":{"kind":"selected-assignment-output","id":"missing","assignment_id":"axis-a"}}"#,
        ),
        (
            "unknown-assignment",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"missing"}}"#,
        ),
        (
            "outputless",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-empty","assignment_id":"axis-empty"}}"#,
        ),
    ];
    for (record_id, data) in cases {
        let (_, result, _) = cli(
            &database,
            &[
                "append",
                "stable-run",
                "review-evidence",
                data,
                "--record-id",
                record_id,
            ],
        );
        assert_eq!(result["status"], "rejected", "{record_id}: {result}");
        assert_eq!(
            result["code"], "selected-output-linkage-refused",
            "{result}"
        );
    }

    create_run(&persistence, "other-run");
    create_completed_invocation(
        &persistence,
        "other-run",
        "foreign-invocation",
        &directory.path().join("foreign-capture"),
        vec![outputless_worker("foreign")],
    );
    let (_, cross_run, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"origin":{"kind":"selected-assignment-output","id":"foreign-invocation","assignment_id":"foreign"}}"#,
            "--record-id",
            "cross-run",
        ],
    );
    assert_eq!(cross_run["status"], "rejected");
    assert_eq!(cross_run["code"], "selected-output-linkage-refused");
}

#[test]
fn append_rejects_caller_spoof_and_legacy_new_operations() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    let capture = directory.path().join("capture");
    create_completed_invocation(
        &persistence,
        "stable-run",
        "invocation-1",
        &capture,
        vec![selected_worker(
            "axis-a",
            &capture.join("axis-a/attempts/2/stdout"),
        )],
    );

    let (_, spoofed, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"axis-a"},"loop_engine_origin":{"invocation_id":"fake","assignment_id":"axis-a","selected_attempt":99,"selected_output_sha256":"sha256:fake","selected_output_path":"capture/fake","capture_dir":"/tmp/fake","command":"fake","args":[],"binding":{"command":"fake","args":[]}}}"#,
            "--record-id",
            "spoofed",
        ],
    );
    assert_eq!(spoofed["status"], "rejected");
    assert_eq!(spoofed["code"], "selected-output-linkage-refused");

    for kind in ["unchanged-carry", "override-carry"] {
        let (_, retired, _) = cli(
            &database,
            &[
                "append",
                "stable-run",
                kind,
                r#"{"source_record_id":"old","invocation_id":"invocation-1","assignment_id":"axis-a","attesting_driver":{"name":"driver"}}"#,
            ],
        );
        assert_eq!(retired["status"], "rejected", "{retired}");
        assert_eq!(retired["code"], "carry-refused", "{retired}");
    }

    let (_, verbose, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"originating_output":{"invocation_id":"invocation-1","assignment_id":"axis-a","selected_attempt":2,"sha256":"sha256:selected-output","path":"capture/axis-a/attempts/2/stdout"}}"#,
        ],
    );
    assert_eq!(verbose["status"], "rejected");
    assert_eq!(verbose["code"], "selected-output-linkage-refused");
}

#[test]
fn evidence_applicability_is_concise_same_run_context_and_stays_visible() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    persistence
        .append_context(AppendContextRequest::new(
            "stable-run",
            "original-evidence",
            "review-evidence",
            json!({"axis": "axis-a", "result": "pass", "findings": "", "author": {"name": "reviewer"}}),
            Timestamp::from_unix_millis(2),
        ))
        .expect("append source evidence");

    let (_, applicability, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "evidence-applicability",
            r#"{"origin":{"kind":"context-record","id":"original-evidence"},"target":{"subject":"review.json","revision":"2","checkpoint":"checkpoint-2"},"attesting_driver":{"name":"driver-1","kind":"human"},"reason":"The reviewed policy and subject remain applicable."}"#,
            "--record-id",
            "applicability-1",
        ],
    );
    assert_eq!(applicability["status"], "completed", "{applicability}");
    assert_eq!(
        applicability["result"]["context"]["kind"],
        "evidence-applicability"
    );
    assert_eq!(
        applicability["result"]["context"]["data"],
        json!({
            "origin": {"kind": "context-record", "id": "original-evidence"},
            "target": {"subject": "review.json", "revision": "2", "checkpoint": "checkpoint-2"},
            "attesting_driver": {"name": "driver-1", "kind": "human"},
            "reason": "The reviewed policy and subject remain applicable."
        })
    );

    let (_, missing, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "evidence-applicability",
            r#"{"origin":{"kind":"context-record","id":"missing"},"target":{"revision":"2"},"attesting_driver":{"name":"driver"},"reason":"reason"}"#,
        ],
    );
    assert_eq!(missing["status"], "rejected");
    assert_eq!(missing["code"], "evidence-applicability-refused");

    let shown = show(&database);
    let contexts = shown["result"]["context"].as_array().expect("contexts");
    assert!(contexts.iter().any(|record| {
        record["id"] == "original-evidence" && record["data"]["author"]["name"] == "reviewer"
    }));
    let current = contexts
        .iter()
        .find(|record| record["id"] == "applicability-1")
        .expect("applicability context");
    assert_eq!(current["data"]["origin"]["id"], "original-evidence");
    assert!(current["data"].get("overridden_inputs").is_none());
    assert!(current["data"].get("attested_dimensions").is_none());
}

#[test]
fn detailed_compact_and_history_are_capture_free_and_terminal_history_remains_readable() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    let capture = directory.path().join("capture");
    create_completed_invocation(
        &persistence,
        "stable-run",
        "invocation-1",
        &capture,
        vec![selected_worker(
            "axis-a",
            &capture.join("axis-a/attempts/2/stdout"),
        )],
    );
    // No selected stdout file is created. Neither projection needs to read it.
    let (_, linked, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"axis-a"}}"#,
        ],
    );
    assert_eq!(linked["status"], "completed");

    let detailed = show(&database);
    assert_eq!(detailed["status"], "completed");
    let worker = &detailed["result"]["work_slot_invocations"][0]["inner_workers"][0];
    assert_eq!(worker["selected_attempt"], 2);
    assert_eq!(worker["selected_output_sha256"], "sha256:selected-output");
    assert_eq!(
        worker["selected_output_path"],
        capture
            .join("axis-a/attempts/2/stdout")
            .to_string_lossy()
            .into_owned()
    );
    assert_eq!(
        detailed["result"]["context"][0]["data"]["loop_engine_origin"]["command"],
        "/bin/worker"
    );

    let compact = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database path"),
            "--compact",
            "show",
            "stable-run",
        ])
        .bounded_output("loop-engine compact stable references")
        .expect("compact show");
    assert!(compact.status.success(), "{compact:?}");
    assert!(String::from_utf8_lossy(&compact.stdout).contains("completed show --compact"));

    let (_, terminated, _) = cli(&database, &["terminate", "stable-run"]);
    assert_eq!(terminated["status"], "completed", "{terminated}");
    let (_, history, _) = cli(&database, &["history", "stable-run"]);
    assert_eq!(history["status"], "completed");
    let entries = history["result"].as_array().expect("history entries");
    assert!(entries
        .iter()
        .any(|entry| entry["action"]["kind"] == "context_appended"));
    assert!(entries
        .iter()
        .any(|entry| entry["action"]["kind"] == "terminated"));

    let (_, after_terminal, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "note",
            "{}",
            "--record-id",
            "after-terminal",
        ],
    );
    assert_eq!(after_terminal["status"], "rejected");
    assert_eq!(after_terminal["code"], "run-not-active");
}

#[test]
fn core_legacy_carry_requests_decode_but_are_not_executed() {
    let request = serde_json::from_value::<AppendContextRequest>(json!({
        "run_id": "run-1",
        "record_id": "record-1",
        "kind": "unchanged-carry",
        "data": {},
        "created_at": 1,
        "carry": {
            "source_record_id": "source",
            "invocation_id": "invocation",
            "assignment_id": "axis-a",
            "act": "unchanged",
            "attesting_driver": {"name": "driver"}
        }
    }))
    .expect("legacy request remains decodable");
    assert!(request.carry.is_some());
}
