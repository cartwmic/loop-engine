use super::bounded_process::CommandExt;
use loop_core::{
    AppendContextRequest, CompleteWorkSlotInvocationRequest, CreateRunRequest,
    CreateWorkSlotInvocationRequest, InnerWorker, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WaiterWrittenStatus, WorkSlot, Workflow,
};
use loop_integrations::SqlitePersistence;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::process::Command;
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "report-workflow",
        "start",
        vec![State::new("start", "Start", "review", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")
        .with_stdin_context_kinds(vec!["routed".to_owned()])])
}

fn worker(assignment: &str) -> InnerWorker {
    let mut worker = InnerWorker::new("/bin/worker", vec!["--frozen".to_owned()], 0);
    worker.assignment_id = assignment.to_owned();
    worker.declared_output_contract = Some(json!({"required": ["result"]}));
    worker
}

fn show(database: &std::path::Path) -> Value {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database path"),
            "--json",
            "show",
            "report-run",
        ])
        .bounded_output("loop-engine change-report")
        .expect("show");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("show JSON")
}

#[test]
fn show_projects_durable_change_report_without_capture_files() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    let run = CreateRunRequest::new(
        "report-run",
        Some("report".to_owned()),
        workflow(),
        ProviderAssociation::new(json!({"identity": "provider-v1"})),
        json!({
            "policy": "policy-v1",
            "artifact_root": directory.path().join("missing").to_string_lossy()
        }),
        "start",
        Lifecycle::Active,
        Timestamp::from_unix_millis(1),
        "provider-v1",
        None,
    );
    persistence.create_run(run).expect("create");
    persistence
        .load_show_data(&"report-run".into())
        .expect("observe");
    persistence
        .set_current_slot_subject(
            &"report-run".into(),
            &"review".into(),
            "subject-v1".to_owned(),
        )
        .expect("subject");
    let routed = loop_core::ContextRecord::new(
        "ctx-1",
        "routed",
        json!({"bytes": "input-v1"}),
        2.into(),
        Timestamp::from_unix_millis(2),
    );
    persistence
        .append_context(AppendContextRequest::new(
            "report-run",
            "ctx-1",
            "routed",
            json!({"bytes": "input-v1"}),
            Timestamp::from_unix_millis(2),
        ))
        .expect("append initial routed input");

    let judgment = CreateWorkSlotInvocationRequest::new(
        "report-run",
        "inv-judgment",
        "review",
        loop_core::WorkSlotBinding::new("/bin/worker", vec!["--review".to_owned()]),
        "instruction-digest",
        "subject-v1",
        0,
        Timestamp::from_unix_millis(3),
        1000,
        directory.path().join("does-not-exist").to_string_lossy(),
    )
    .with_routed_inputs(vec![routed.clone()])
    .with_frozen_run_identity(
        json!({"provider": {"identity": "provider-v1"}, "input": {"policy": "policy-v1"}}),
    );
    persistence
        .create_work_slot_invocation(judgment)
        .expect("create judgment");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "report-run",
            "inv-judgment",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(4),
            vec![worker("reviewer-1")],
        ))
        .expect("complete judgment");

    let mut task_worker = worker("task-a");
    task_worker.task_definition = Some(json!({"id": "task-a", "change": "one"}));
    task_worker.task_packet = Some(json!("artifact-root\n---\npacket"));
    task_worker.dependencies = Some(vec!["task-parent".to_owned()]);
    task_worker.routed_inputs = Some(json!([]));
    task_worker.repository_effect = Some(json!({"recorded": "task-a-only"}));
    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                "report-run", "inv-plan", "review",
                loop_core::WorkSlotBinding::new("/bin/worker", vec!["--plan".to_owned()]),
                "instruction-digest", "subject-v1", 0, Timestamp::from_unix_millis(6), 1000,
                directory.path().join("also-missing").to_string_lossy(),
            )
            .with_frozen_run_identity(json!({"provider": {"identity": "provider-v1"}, "input": {"policy": "policy-v1", "artifact_root": directory.path().join("missing").to_string_lossy()}})),
        )
        .expect("create plan result");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "report-run",
            "inv-plan",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(7),
            vec![task_worker.clone()],
        ))
        .expect("complete plan result");

    let first = show(&database);
    assert_eq!(
        first["result"]["change_report"]["assignments"][0]["subject_revision"],
        "subject-v1"
    );
    assert_eq!(
        first["result"]["work_slot_invocations"][0]["change_report"]["dimensions"]["routed_inputs"]
            ["changed"],
        false
    );
    assert_eq!(
        first["result"]["work_slot_invocations"][0]["change_report"]["dimensions"]
            ["declared_output_contract"]["changed"],
        false
    );
    let plan_report = &first["result"]["change_report"]["plan_task_results"][0]["dimensions"];
    for dimension in [
        "task_definition",
        "task_packet",
        "dependencies",
        "routed_inputs",
        "worker_binding",
        "repository_effect",
    ] {
        assert_eq!(plan_report[dimension]["changed"], false, "{dimension}");
    }
    assert!(first["result"]["work_slot_invocations"][0]["capture_dir"]
        .as_str()
        .unwrap()
        .contains("does-not-exist"));
    assert_eq!(first, show(&database));

    // A file left in the shared directory is deliberately not a task result.
    // The report stays unchanged because it uses only the task's recorded
    // repository effect, never a checkout scan.
    let shared_remainder = directory.path().join("shared-working-directory");
    std::fs::create_dir_all(&shared_remainder).expect("shared directory");
    std::fs::write(shared_remainder.join("remainder.txt"), b"not task-a").expect("remainder");
    let after_shared_remainder = show(&database);
    assert_eq!(
        after_shared_remainder["result"]["change_report"]["plan_task_results"][0]["dimensions"]
            ["repository_effect"]["changed"],
        false
    );
    assert_eq!(after_shared_remainder, first);

    let mut mutations = Vec::new();
    let mut changed = task_worker.clone();
    changed.task_definition = Some(json!({"id": "task-a", "change": "two"}));
    mutations.push(("task_definition", changed));
    let mut changed = task_worker.clone();
    changed.task_packet = Some(json!("different packet"));
    mutations.push(("task_packet", changed));
    let mut changed = task_worker.clone();
    changed.dependencies = Some(vec!["other-parent".to_owned()]);
    mutations.push(("dependencies", changed));
    let mut changed = task_worker.clone();
    changed.routed_inputs = Some(json!([{"new": true}]));
    mutations.push(("routed_inputs", changed));
    let mut changed = task_worker.clone();
    changed.args = vec!["--changed-binding".to_owned()];
    mutations.push(("worker_binding", changed));
    let mut changed = task_worker.clone();
    changed.repository_effect = Some(json!({"recorded": "different-effect"}));
    mutations.push(("repository_effect", changed));
    for (dimension, mutated) in mutations {
        let connection = Connection::open(&database).expect("open mutation database");
        connection
            .execute(
                "UPDATE work_slot_invocations SET inner_workers_json = ?1 WHERE invocation_id = 'inv-plan'",
                [serde_json::to_string(&vec![mutated]).expect("worker JSON")],
            )
            .expect("mutate durable result");
        let report = show(&database)["result"]["change_report"]["plan_task_results"][0]
            ["dimensions"]
            .clone();
        assert_eq!(report[dimension]["changed"], true, "{dimension}");
        connection
            .execute(
                "UPDATE work_slot_invocations SET inner_workers_json = completion_snapshot_json WHERE invocation_id = 'inv-plan'",
                [],
            )
            .expect("restore durable result");
    }

    // A durable subject revision and a new routed record are visible without
    // touching the missing capture directory or starting the provider.
    persistence
        .set_current_slot_subject(
            &"report-run".into(),
            &"review".into(),
            "subject-v2".to_owned(),
        )
        .expect("revise subject");
    persistence
        .load_show_data(&"report-run".into())
        .expect("re-observe");
    persistence
        .append_context(AppendContextRequest::new(
            "report-run",
            "ctx-2",
            "routed",
            json!({"bytes": "input-v2"}),
            Timestamp::from_unix_millis(5),
        ))
        .expect("append routed input");
    let changed = show(&database);
    let report = &changed["result"]["work_slot_invocations"][0]["change_report"]["dimensions"];
    assert_eq!(report["subject_bytes"]["changed"], true);
    assert_eq!(report["routed_inputs"]["changed"], true);

    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                "report-run",
                "inv-empty",
                "review",
                loop_core::WorkSlotBinding::new("/bin/worker", vec!["--empty".to_owned()]),
                "instruction-digest",
                "subject-v2",
                0,
                Timestamp::from_unix_millis(8),
                1000,
                directory.path().join("empty").to_string_lossy(),
            )
            .with_frozen_run_identity(json!({
                "provider": {"identity": "provider-v1"},
                "input": {"policy": "policy-v1"}
            })),
        )
        .expect("create empty result");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "report-run",
            "inv-empty",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(9),
            Vec::new(),
        ))
        .expect("complete empty result");
    let empty = show(&database);
    let empty_report = &empty["result"]["work_slot_invocations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["invocation_id"] == "inv-empty")
        .expect("empty invocation")["change_report"]["dimensions"];
    for dimension in [
        "subject_bytes",
        "worker_assignment",
        "frozen_binding",
        "governing_policy_configuration",
        "declared_output_contract",
        "routed_inputs",
    ] {
        assert_eq!(empty_report[dimension]["changed"], true, "{dimension}");
    }
    assert_eq!(
        empty_report,
        &show(&database)["result"]["work_slot_invocations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["invocation_id"] == "inv-empty")
            .expect("repeated empty invocation")["change_report"]["dimensions"]
    );
}
