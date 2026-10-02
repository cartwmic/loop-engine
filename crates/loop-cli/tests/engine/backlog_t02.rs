#![cfg(unix)]
use super::bounded_process::CommandExt;
use loop_core::{
    CompleteWorkSlotInvocationRequest, CreateRunRequest, CreateWorkSlotInvocationRequest,
    InnerWorker, Lifecycle, Persistence, ProviderAssociation, State, Timestamp, Transition,
    WaiterWrittenStatus, WorkSlot, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

#[test]
fn backlog_t02_optional_repository_effect_distinguishes_missing_task_record_public_show() {
    let root = tempdir().expect("change-report fixture");
    let database = root.path().join("loop.sqlite");
    let artifact_root = root.path().join("artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let workflow = Workflow::new(
        "backlog-t02-report",
        "start",
        vec![State::new("start", "Start", "Review", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")]);
    let persistence = SqlitePersistence::open(&database).expect("open report database");
    persistence
        .create_run(CreateRunRequest::new(
            "report-run",
            Some("report".to_owned()),
            workflow,
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            json!({"artifact_root": artifact_root.to_string_lossy()}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            Some(artifact_root.to_string_lossy().into_owned()),
        ))
        .expect("create report run");
    persistence
        .load_show_data(&"report-run".into())
        .expect("observe report run");
    persistence
        .set_current_slot_subject(
            &"report-run".into(),
            &"review".into(),
            "subject-v1".to_owned(),
        )
        .expect("report subject");

    let mut task = InnerWorker::new("/bin/worker", vec!["--task".to_owned()], 0);
    task.assignment_id = "task-a".to_owned();
    task.task_definition = Some(json!({"id": "task-a"}));
    task.task_packet = Some(json!("artifact-root\n---\npacket"));
    task.dependencies = Some(Vec::new());
    task.routed_inputs = Some(json!([]));
    // Deliberately leave the optional effect absent on both present records.
    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                "report-run",
                "inv-task",
                "review",
                WorkSlotBinding::new("/bin/worker", vec!["--task".to_owned()]),
                "digest",
                "subject-v1",
                0,
                Timestamp::from_unix_millis(2),
                1000,
                artifact_root.join("capture").to_string_lossy(),
            )
            .with_frozen_run_identity(json!({
                "provider": {"identity": "provider-v1"},
                "input": {"artifact_root": artifact_root.to_string_lossy()}
            })),
        )
        .expect("create task invocation");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "report-run",
            "inv-task",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(3),
            vec![task.clone()],
        ))
        .expect("complete task invocation");

    let unchanged = show_full_report(&database, "report-run");
    let dimensions = &unchanged["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(dimensions["repository_effect"]["changed"], false);

    let connection = Connection::open(&database).expect("open report mutation database");
    // Mutations below exercise the supported legacy inline snapshot, which
    // retains an independent original worker. New digest-backed snapshots
    // correctly reject changes to their bound worker payload as corruption.
    connection
        .execute(
            "UPDATE work_slot_invocations SET completion_snapshot_json = ?1 WHERE invocation_id = 'inv-task'",
            [serde_json::to_string(&vec![task.clone()]).expect("original worker snapshot JSON")],
        )
        .expect("seed legacy comparison snapshot");
    let mut changed_effect = task.clone();
    changed_effect.repository_effect = Some(json!({"changed": true}));
    connection
        .execute(
            "UPDATE work_slot_invocations SET inner_workers_json = ?1 WHERE invocation_id = 'inv-task'",
            [serde_json::to_string(&vec![changed_effect]).expect("changed worker JSON")],
        )
        .expect("mutate optional effect");
    let changed = show_full_report(&database, "report-run");
    let changed_dimensions = &changed["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(changed_dimensions["repository_effect"]["changed"], true);

    connection
        .execute(
            "UPDATE work_slot_invocations SET inner_workers_json = '[]' WHERE invocation_id = 'inv-task'",
            [],
        )
        .expect("remove current task record");
    let missing = show_full_report(&database, "report-run");
    let missing_dimensions = &missing["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(missing_dimensions["repository_effect"]["changed"], true);
    assert_eq!(missing_dimensions["task_definition"]["changed"], true);
}

fn show_full_report(database: &Path, run_id: &str) -> Value {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database UTF-8"),
            "--json",
            "show",
            "--view",
            "full",
            run_id,
        ])
        .bounded_output("backlog_t02 report show")
        .expect("report show process");
    assert!(
        output.status.success(),
        "report show stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("report show JSON")
}
