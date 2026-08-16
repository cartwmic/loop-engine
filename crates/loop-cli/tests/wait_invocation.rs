use loop_core::{
    CreateRunRequest, CreateWorkSlotInvocationRequest, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WaiterWrittenStatus, WorkSlotBinding, Workflow,
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
    let persistence = SqlitePersistence::open(database).expect("open sqlite");
    persistence
        .create_run(create_request(run_id))
        .expect("create run");
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
        ))
        .expect("create running invocation");
}

fn load_invocation_status(
    database: &Path,
    run_id: &str,
) -> (Option<WaiterWrittenStatus>, Option<i32>) {
    let persistence = SqlitePersistence::open(database).expect("reopen sqlite");
    let invocations = persistence
        .load_work_slot_invocations(&run_id.into())
        .expect("load invocations");
    assert_eq!(invocations.len(), 1);
    (invocations[0].status, invocations[0].exit_code)
}

fn spawn_wait_invocation(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    envelope: &Value,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "wait-invocation",
            run_id,
            invocation_id,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wait-invocation");
    {
        let mut stdin = child.stdin.take().expect("waiter stdin");
        stdin
            .write_all(&serde_json::to_vec(envelope).expect("envelope json"))
            .expect("write waiter envelope");
    }
    child.wait_with_output().expect("wait for waiter")
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
    let (status, exit_code) = load_invocation_status(&database, "run-wait-ok");
    assert_eq!(status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(exit_code, Some(0));
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
    let (status, exit_code) = load_invocation_status(&database, "run-wait-fail");
    assert_eq!(status, Some(WaiterWrittenStatus::Failed));
    assert_eq!(exit_code, Some(7));
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

    let mut waiter = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "wait-invocation",
            "run-wait-ppid",
            "inv-wait-ppid",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wait-invocation");
    let waiter_pid = waiter.id();
    {
        let mut stdin = waiter.stdin.take().expect("waiter stdin");
        stdin
            .write_all(&serde_json::to_vec(&envelope).expect("envelope json"))
            .expect("write waiter envelope");
    }
    let status = waiter.wait().expect("wait for waiter");
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
    let output = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .arg("--help")
        .output()
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
    let help = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .arg("--help")
        .output()
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

    let missing_args = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .arg("wait-invocation")
        .output()
        .expect("run wait-invocation without args");
    let code = missing_args.status.code();
    assert!(
        code == Some(0) || code == Some(2),
        "hidden wait-invocation must be accepted (exit 0 or 2 if missing args), got {code:?}: {:?}",
        String::from_utf8_lossy(&missing_args.stderr)
    );
    let unknown = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .arg("not-a-primary")
        .output()
        .expect("run unknown operation");
    let unknown_stderr = String::from_utf8_lossy(&unknown.stderr);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(
        unknown_stderr.contains("start, list, show, append, event, history, terminate, or invoke"),
        "unknown primary list changed: {unknown_stderr}"
    );
}
