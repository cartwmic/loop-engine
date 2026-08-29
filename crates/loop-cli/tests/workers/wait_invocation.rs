use super::bounded_process::CommandExt;
use loop_core::{
    CreateRunRequest, CreateWorkSlotInvocationRequest, InnerWorker, Lifecycle, Persistence,
    ProviderAssociation, State, Timestamp, Transition, WaiterWrittenStatus, WorkSlotBinding,
    Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "test-workflow",
        "start",
        vec![
            State::new("start", "Start", "Begin", false),
            State::new("middle", "Middle", "Continue", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::checked("start", "approve", "middle"),
            Transition::checked("start", "retry", "start"),
            Transition::check_free("middle", "finish", "done"),
        ],
    )
}

fn create_request(id: &str) -> CreateRunRequest {
    CreateRunRequest::new(
        id,
        Some(format!("label-{id}")),
        workflow(),
        ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
        json!({"objective": "durable"}),
        "start",
        Lifecycle::Active,
        Timestamp::from_unix_millis(100),
        "test-provider",
        Some("/allocated/run-dir".to_owned()),
    )
}

fn worker_packet(run_id: &str) -> Value {
    json!({
        "run_id": run_id,
        "slot_id": "slot-1",
        "artifact_root": "/tmp/artifacts",
        "instruction_body": "do the work"
    })
}

fn envelope(command: &str, args: Vec<Value>, packet: Value) -> Value {
    json!({
        "command": command,
        "args": args,
        "worker_packet": packet
    })
}

fn seed_running_invocation(database: &Path, run_id: &str, invocation_id: &str) {
    seed_running_invocation_with_capture(database, run_id, invocation_id, String::new());
}

fn seed_running_invocation_with_capture(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    capture_dir: impl Into<String>,
) {
    let persistence = SqlitePersistence::open(database).expect("open sqlite");
    persistence
        .create_run(create_request(run_id))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
    persistence
        .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            "slot-1",
            WorkSlotBinding::new("sh", vec!["-c".to_owned(), "exit 0".to_owned()]),
            "digest",
            "subject",
            1,
            Timestamp::from_unix_millis(500),
            1_000,
            capture_dir,
        ))
        .expect("create running invocation");
}

fn load_invocation(
    database: &Path,
    run_id: &str,
) -> (Option<WaiterWrittenStatus>, Option<i32>, Vec<InnerWorker>) {
    let persistence = SqlitePersistence::open(database).expect("reopen sqlite");
    let invocations = persistence
        .load_work_slot_invocations(&run_id.into())
        .expect("load invocations");
    assert_eq!(invocations.len(), 1);
    (
        invocations[0].status,
        invocations[0].exit_code,
        invocations[0].inner_workers.clone(),
    )
}

fn spawn_wait_invocation(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    envelope: &Value,
) -> std::process::Output {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.args([
        "--database",
        database.to_str().expect("utf-8 database path"),
        "wait-invocation",
        run_id,
        invocation_id,
    ]);
    let completed = super::bounded_process::run_with_stdin(
        &mut command,
        "loop-engine wait-invocation",
        &serde_json::to_vec(envelope).expect("envelope json"),
    )
    .expect("wait for waiter");
    completed.output
}

#[test]
fn wait_invocation_worker_exit_0_stores_succeeded() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    seed_running_invocation(&database, "run-wait-ok", "inv-wait-ok");
    let output = spawn_wait_invocation(
        &database,
        "run-wait-ok",
        "inv-wait-ok",
        &envelope(
            "sh",
            vec![json!("-c"), json!("exit 0")],
            worker_packet("run-wait-ok"),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, exit_code, inner_workers) = load_invocation(&database, "run-wait-ok");
    assert_eq!(status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(exit_code, Some(0));
    assert!(inner_workers.is_empty());
}

#[test]
fn wait_invocation_worker_exit_7_stores_failed() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    seed_running_invocation(&database, "run-wait-fail", "inv-wait-fail");
    let output = spawn_wait_invocation(
        &database,
        "run-wait-fail",
        "inv-wait-fail",
        &envelope(
            "sh",
            vec![json!("-c"), json!("exit 7")],
            worker_packet("run-wait-fail"),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, exit_code, inner_workers) = load_invocation(&database, "run-wait-fail");
    assert_eq!(status, Some(WaiterWrittenStatus::Failed));
    assert_eq!(exit_code, Some(7));
    assert!(inner_workers.is_empty());
}

#[test]
fn wait_invocation_waiter_is_the_worker_parent() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let ppid_file = directory.path().join("ppid.txt");
    seed_running_invocation(&database, "run-wait-ppid", "inv-wait-ppid");

    let envelope = envelope(
        "sh",
        vec![
            json!("-c"),
            json!("printf %s \"$PPID\" > \"$1\"; exit 0"),
            json!("_"),
            json!(ppid_file.to_string_lossy().into_owned()),
        ],
        worker_packet("run-wait-ppid"),
    );

    let mut waiter_command = Command::new(workspace_integration::binary("loop-engine"));
    waiter_command
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "wait-invocation",
            "run-wait-ppid",
            "inv-wait-ppid",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::bounded_process::prepare_process_group(&mut waiter_command);
    let mut waiter = waiter_command.spawn().expect("spawn wait-invocation");
    let waiter_pid = waiter.id();
    {
        let mut stdin = waiter.stdin.take().expect("waiter stdin");
        stdin
            .write_all(&serde_json::to_vec(&envelope).expect("envelope json"))
            .expect("write waiter envelope");
    }
    let status = super::bounded_process::wait_existing(waiter, "loop-engine waiter parent")
        .expect("wait for waiter")
        .status;
    assert!(status.success(), "{status:?}");

    let recorded = std::fs::read_to_string(&ppid_file).expect("read ppid file");
    assert_eq!(recorded, waiter_pid.to_string());
}

#[test]
fn wait_invocation_worker_stdin_is_inner_worker_packet_not_envelope() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let stdin_file = directory.path().join("stdin.json");
    seed_running_invocation(&database, "run-wait-stdin", "inv-wait-stdin");
    let packet = worker_packet("run-wait-stdin");
    let output = spawn_wait_invocation(
        &database,
        "run-wait-stdin",
        "inv-wait-stdin",
        &envelope(
            "sh",
            vec![
                json!("-c"),
                json!("cat > \"$1\"; exit 0"),
                json!("_"),
                json!(stdin_file.to_string_lossy().into_owned()),
            ],
            packet.clone(),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let captured = std::fs::read_to_string(&stdin_file).expect("read worker stdin file");
    let parsed: Value = serde_json::from_str(&captured).expect("worker stdin must be JSON");
    assert_eq!(parsed, packet);
    assert!(
        parsed.get("command").is_none(),
        "worker stdin must not contain the envelope command field: {captured}"
    );
    assert!(
        !captured.contains("\"command\""),
        "worker stdin must not contain envelope command: {captured}"
    );
}

#[test]
fn wait_invocation_help_stdout_does_not_contain_wait_invocation() {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .arg("--help")
        .bounded_output("loop-engine wait-invocation")
        .expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("wait-invocation"),
        "help must not mention hidden wait-invocation: {stdout}"
    );
}

#[test]
fn wait_invocation_unknown_primary_list_unchanged() {
    let help = Command::new(workspace_integration::binary("loop-engine"))
        .arg("--help")
        .bounded_output("loop-engine wait-invocation")
        .expect("run --help");
    let stdout = String::from_utf8_lossy(&help.stdout);
    for operation in [
        "start",
        "list",
        "show",
        "append",
        "event",
        "history",
        "terminate",
        "invoke",
    ] {
        assert!(
            stdout.contains(operation),
            "help operations list missing `{operation}`: {stdout}"
        );
    }
    assert!(
        !stdout.contains("wait-invocation"),
        "help operations list must still omit wait-invocation: {stdout}"
    );

    let missing_args = Command::new(workspace_integration::binary("loop-engine"))
        .arg("wait-invocation")
        .bounded_output("loop-engine wait-invocation")
        .expect("run wait-invocation without args");
    let code = missing_args.status.code();
    assert!(
        code == Some(0) || code == Some(2),
        "hidden wait-invocation must be accepted (exit 0 or 2 if missing args), got {code:?}: {:?}",
        String::from_utf8_lossy(&missing_args.stderr)
    );
    let unknown = Command::new(workspace_integration::binary("loop-engine"))
        .arg("not-a-primary")
        .bounded_output("loop-engine wait-invocation")
        .expect("run unknown operation");
    let unknown_stderr = String::from_utf8_lossy(&unknown.stderr);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(
        unknown_stderr.contains("start, list, show, append, event, history, terminate, or invoke"),
        "unknown primary list changed: {unknown_stderr}"
    );
}

#[test]
fn wait_invocation_copies_well_formed_summary_inner_workers() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let capture_dir = directory.path().join("captures").join("inv-summary");
    std::fs::create_dir_all(&capture_dir).expect("capture dir");
    std::fs::write(
        capture_dir.join("summary.json"),
        serde_json::to_vec(&json!({
            "workers": [
                {
                    "command": "python3",
                    "args": ["a.py"],
                    "exit_code": 0,
                    "stdout_path": "/tmp/0/stdout"
                },
                {
                    "command": "python3",
                    "args": ["b.py"],
                    "exit_code": 7,
                    "stderr_path": "/tmp/1/stderr"
                }
            ]
        }))
        .expect("summary json"),
    )
    .expect("write summary.json");
    seed_running_invocation_with_capture(
        &database,
        "run-wait-summary",
        "inv-wait-summary",
        capture_dir.to_string_lossy().into_owned(),
    );
    let output = spawn_wait_invocation(
        &database,
        "run-wait-summary",
        "inv-wait-summary",
        &envelope(
            "sh",
            vec![json!("-c"), json!("exit 0")],
            worker_packet("run-wait-summary"),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, exit_code, inner_workers) = load_invocation(&database, "run-wait-summary");
    assert_eq!(status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(exit_code, Some(0));
    assert_eq!(
        inner_workers,
        vec![
            InnerWorker::new("python3", vec!["a.py".to_owned()], 0),
            InnerWorker::new("python3", vec!["b.py".to_owned()], 7),
        ]
    );
}

#[test]
fn wait_invocation_missing_summary_stores_empty_inner_workers() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let capture_dir = directory.path().join("captures").join("inv-missing");
    std::fs::create_dir_all(&capture_dir).expect("capture dir");
    seed_running_invocation_with_capture(
        &database,
        "run-wait-missing",
        "inv-wait-missing",
        capture_dir.to_string_lossy().into_owned(),
    );
    let output = spawn_wait_invocation(
        &database,
        "run-wait-missing",
        "inv-wait-missing",
        &envelope(
            "sh",
            vec![json!("-c"), json!("exit 0")],
            worker_packet("run-wait-missing"),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, exit_code, inner_workers) = load_invocation(&database, "run-wait-missing");
    assert_eq!(status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(exit_code, Some(0));
    assert!(inner_workers.is_empty());
}

#[test]
fn wait_invocation_malformed_summary_stores_empty_inner_workers_without_flipping_overlay() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let capture_dir = directory.path().join("captures").join("inv-malformed");
    std::fs::create_dir_all(&capture_dir).expect("capture dir");
    std::fs::write(
        capture_dir.join("summary.json"),
        serde_json::to_vec(&json!({
            "workers": [
                {"command": "python3", "args": ["ok.py"], "exit_code": 0},
                {"command": "python3", "args": []}
            ]
        }))
        .expect("malformed summary json"),
    )
    .expect("write summary.json");
    seed_running_invocation_with_capture(
        &database,
        "run-wait-malformed",
        "inv-wait-malformed",
        capture_dir.to_string_lossy().into_owned(),
    );
    let output = spawn_wait_invocation(
        &database,
        "run-wait-malformed",
        "inv-wait-malformed",
        &envelope(
            "sh",
            vec![json!("-c"), json!("exit 0")],
            worker_packet("run-wait-malformed"),
        ),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, exit_code, inner_workers) = load_invocation(&database, "run-wait-malformed");
    assert_eq!(status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(exit_code, Some(0));
    assert!(
        inner_workers.is_empty(),
        "one malformed worker must store empty inner_workers, got {inner_workers:?}"
    );
}
