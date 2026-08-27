use super::bounded_process::CommandExt;
use loop_core::{
    AppendContextRequest, CompleteWorkSlotInvocationRequest, CreateRunRequest,
    CreateWorkSlotInvocationRequest, InnerWorker, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WorkSlot, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::process::Command;
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "carry-workflow",
        "start",
        vec![State::new("start", "Start", "work", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")])
}

fn worker() -> InnerWorker {
    let mut worker = InnerWorker::new("/bin/worker", vec!["--review".to_owned()], 0);
    worker.assignment_id = "axis-a".to_owned();
    worker.selected_attempt = Some(1);
    worker.selected_output_sha256 = Some("sha256:originating".to_owned());
    worker.selected_output_path = Some("axis-a/attempts/1/stdout".to_owned());
    worker.declared_output_contract = Some(json!({"type": "object"}));
    worker
}

fn cli(database: &std::path::Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args(["--database", database.to_str().unwrap(), "--json"])
        .args(args)
        .bounded_output("loop-engine carry")
        .expect("loop-engine");
    let value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    (
        output.status.code().unwrap_or(-1),
        value,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn append_carry_acts_read_report_and_preserve_provenance() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).unwrap();
    let initial_input = json!({
        "policy": "v1",
        "work_slot_bindings": {
            "review": {"command": "/bin/worker", "args": ["--review"]}
        }
    });
    persistence
        .create_run(CreateRunRequest::new(
            "carry-run",
            None,
            workflow(),
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            initial_input.clone(),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            None,
        ))
        .unwrap();
    persistence.load_show_data(&"carry-run".into()).unwrap();
    persistence
        .set_current_slot_subject(&"carry-run".into(), &"review".into(), "subject-v1".into())
        .unwrap();
    persistence
        .append_context(AppendContextRequest::new(
            "carry-run",
            "original-evidence",
            "review-evidence",
            json!({
                "gate": "review",
                "policy_id": "axis-a",
                "result": "pass",
                "findings": "",
                "subject": "subject-v1",
                "subject_revision": "subject-v1",
                "config_version": "v1",
                "author": {"name": "reviewer-a", "kind": "agent"}
            }),
            Timestamp::from_unix_millis(2),
        ))
        .unwrap();

    let invocation = CreateWorkSlotInvocationRequest::new(
        "carry-run",
        "invocation-1",
        "review",
        WorkSlotBinding::new("/bin/worker", vec!["--review".to_owned()]),
        "instruction-v1",
        "subject-v1",
        0,
        Timestamp::from_unix_millis(3),
        1000,
        directory.path().join("capture").to_string_lossy(),
    )
    .with_frozen_run_identity(json!({
        "provider": {"identity": "provider-v1"},
        "input": initial_input
    }));
    persistence.create_work_slot_invocation(invocation).unwrap();
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "carry-run",
            "invocation-1",
            loop_core::WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(4),
            vec![worker()],
        ))
        .unwrap();

    let (_, fabricated, _) = cli(
        &database,
        &[
            "append",
            "carry-run",
            "review-evidence",
            r#"{"originating_output":{"invocation_id":"invocation-1","assignment_id":"not-selected","selected_attempt":1,"sha256":"sha256:originating","path":"axis-a/attempts/1/stdout"}}"#,
        ],
    );
    assert_eq!(fabricated["status"], "rejected");
    assert_eq!(fabricated["code"], "selected-output-linkage-refused");
    let (_, linked, _) = cli(
        &database,
        &[
            "append",
            "carry-run",
            "review-evidence",
            r#"{"originating_output":{"invocation_id":"invocation-1","assignment_id":"axis-a","selected_attempt":1,"sha256":"sha256:originating","path":"axis-a/attempts/1/stdout"}}"#,
        ],
    );
    assert_eq!(linked["status"], "completed");

    // A clean report permits unchanged-carry and its output names the guidance.
    let (_, clean, output) = cli(
        &database,
        &[
            "append",
            "carry-run",
            "unchanged-carry",
            r#"{"source_record_id":"original-evidence","invocation_id":"invocation-1","assignment_id":"axis-a","attesting_driver":{"name":"driver-1","kind":"human"}}"#,
        ],
    );
    assert_eq!(clean["status"], "completed", "{output}");
    assert!(output.contains("change report"));
    assert!(output.contains("unchanged-carry"));

    let (_, show, _) = cli(&database, &["show", "carry-run"]);
    let judgment = &show["result"]["change_report"]["assignments"][0];
    assert_eq!(judgment["standing"], true);
    assert_eq!(judgment["carry_act"], "unchanged-carry");
    assert_eq!(judgment["attesting_driver"]["name"], "driver-1");
    assert_eq!(judgment["originating_output_sha256"], "sha256:originating");
    let carried_context = show["result"]["context"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(
        carried_context["data"]["author"],
        json!({"name": "reviewer-a", "kind": "agent"})
    );
    assert_eq!(
        carried_context["data"]["loop_engine_carry"]["originating_output_path"],
        "axis-a/attempts/1/stdout"
    );
    assert_eq!(
        carried_context["data"]["loop_engine_carry"]["originating_attempt"],
        1
    );
    assert!(carried_context["data"]["loop_engine_carry"]["attested_dimensions"].is_object());

    persistence
        .set_current_slot_subject(&"carry-run".into(), &"review".into(), "subject-v2".into())
        .unwrap();
    let (_, refused, _) = cli(
        &database,
        &[
            "append",
            "carry-run",
            "unchanged-carry",
            r#"{"source_record_id":"original-evidence","invocation_id":"invocation-1","assignment_id":"axis-a","attesting_driver":{"name":"driver-2","kind":"human"}}"#,
        ],
    );
    assert_eq!(refused["status"], "rejected");
    assert_eq!(refused["code"], "carry-refused");
    assert!(refused["message"]
        .as_str()
        .unwrap()
        .contains("subject_bytes"));
    let (_, drifted_show, _) = cli(&database, &["show", "carry-run"]);
    assert_eq!(
        drifted_show["result"]["change_report"]["assignments"][0]["standing"], false,
        "a prior unchanged carry must not bless later drift"
    );

    let (_, overridden, output) = cli(
        &database,
        &[
            "append",
            "carry-run",
            "override-carry",
            r#"{"source_record_id":"original-evidence","invocation_id":"invocation-1","assignment_id":"axis-a","attesting_driver":{"name":"driver-3","kind":"human"},"overridden_inputs":["subject_bytes"]}"#,
        ],
    );
    assert_eq!(overridden["status"], "completed", "{output}");
    assert!(output.contains("override-carry"));
    let (_, show, _) = cli(&database, &["show", "carry-run"]);
    let judgment = &show["result"]["change_report"]["assignments"][0];
    assert_eq!(judgment["carry_act"], "override-carry");
    assert_eq!(judgment["standing"], true);
    assert_eq!(judgment["overridden_inputs"], json!(["subject_bytes"]));

    persistence
        .set_current_slot_subject(&"carry-run".into(), &"review".into(), "subject-v3".into())
        .unwrap();
    let (_, drifted_again, _) = cli(&database, &["show", "carry-run"]);
    assert_eq!(
        drifted_again["result"]["change_report"]["assignments"][0]["standing"], false,
        "an override carry attests exact dimensions, not every later change with the same name"
    );
}
