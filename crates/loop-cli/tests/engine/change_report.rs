use super::bounded_process::CommandExt;
use loop_core::{
    CreateRunRequest, Lifecycle, Persistence, ProviderAssociation, State, Timestamp, Transition,
    WorkSlot, Workflow,
};
use loop_integrations::SqlitePersistence;
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

fn show(database: &std::path::Path) -> Value {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database path"),
            "--json",
            "show",
            "--view",
            "full",
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
fn external_work_full_show_keeps_pending_history_unsampled() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    persistence
        .create_run(CreateRunRequest::new(
            "report-run",
            None,
            workflow(),
            ProviderAssociation::new(json!({"identity":"provider-v1"})),
            json!({}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            None,
        ))
        .expect("run");
    persistence
        .load_show_data(&"report-run".into())
        .expect("observe");
    rusqlite::Connection::open(&database).unwrap().execute(
        r#"INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json, instruction_digest, subject, waiter_pid, started_at, allowed_time_ms) VALUES ('report-run', 'pending', 'review', '{"command":"external-driver","args":["opaque"]}', 'digest', 'subject', 0, 2, 1000)"#,
        [],
    ).unwrap();
    let result = show(&database);
    let row = &result["result"]["work_slot_invocations"][0];
    assert_eq!(row["invocation_id"], "pending");
    assert_eq!(row["status"], "unknown");
    assert_eq!(row["binding"]["command"], "external-driver");
    assert!(row["overlay_meaning"]
        .as_str()
        .unwrap()
        .contains("unsampled"));
    assert_eq!(result["result"]["current_state_instructions"], "review");

    // Retired origins refuse even when supplied on applicability records.
    for kind in ["review", "evidence-applicability"] {
        let output = Command::new(workspace_integration::binary("loop-engine"))
            .args(["--database", database.to_str().unwrap(), "--json", "append", "report-run",
                "--kind", kind, "--data", r#"{"origin":{"kind":"selected-assignment-output","id":"pending","assignment_id":"worker"}}"#])
            .bounded_output("retired origin").unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("selected-assignment-output-retired")
        );
    }
    // Historical pending work cannot prevent an externally driven departure.
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--json",
            "event",
            "report-run",
            "finish",
        ])
        .bounded_output("external departure")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        show(&database)["result"]["work_slot_invocations"][0]["status"],
        "unknown"
    );
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--json",
            "terminate",
            "report-run",
        ])
        .bounded_output("external termination")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    drop(persistence);
    let reopened = show(&database);
    assert_eq!(reopened["result"]["lifecycle"], "terminated");
    assert_eq!(
        reopened["result"]["work_slot_invocations"][0]["invocation_id"],
        "pending"
    );
}
