//! Public process-incarnation and completion-reconciliation regressions.
//!
//! These tests use real Loop Engine CLI subprocesses and only test-owned
//! processes/files.  Numeric PID/PGID values are deliberately retained in the
//! fixture so the assertions exercise the native identity boundary.

use super::bounded_process::CommandExt;
use loop_core::{
    CreateRunRequest, CreateWorkSlotInvocationRequest, Lifecycle, OwnedExecution, Persistence,
    ProcessIdentity, ProviderAssociation, State, Transition, WorkSlot, Workflow,
};
use loop_integrations::ownership::{self, Admission};
use loop_integrations::SqlitePersistence;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn make_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write fixture executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");
    }
}

fn toml_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_cli(output: Output, phase: &str) -> Value {
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{phase} did not emit JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "{phase} failed: {value}; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    value
}

fn run_cli_output(root: &Path, args: &[String], phase: &str) -> Output {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command
        .current_dir(root)
        .env_remove(ownership::OWNERSHIP_ENV)
        .args(args);
    command
        .bounded_output(phase)
        .unwrap_or_else(|error| panic!("{phase} could not run: {error}"))
}

fn run_cli(root: &Path, args: &[String], phase: &str) -> Value {
    parse_cli(run_cli_output(root, args, phase), phase)
}

fn launch_blocked_invocation(root: &Path, run_id: &str) -> (PathBuf, PathBuf, String) {
    let (database, artifacts, config, worker) = write_cli_fixture(root);
    let mut start = common_args(&database, &config);
    start.extend([
        "start".to_owned(),
        "fixture".to_owned(),
        serde_json::to_string(&json!({
            "artifact_root": artifacts,
            "work_slot_bindings": {
                "slot": {"command": worker, "args": []}
            }
        }))
        .expect("start input"),
        "--id".to_owned(),
        run_id.to_owned(),
    ]);
    assert_eq!(
        run_cli(root, &start, "backlog_t03 start")["status"],
        "completed"
    );

    let mut show = common_args(&database, &config);
    show.extend([
        "show".to_owned(),
        run_id.to_owned(),
        "--view".to_owned(),
        "full".to_owned(),
    ]);
    run_cli(root, &show, "backlog_t03 arm invoke");

    let mut invoke = common_args(&database, &config);
    invoke.extend(["invoke".to_owned(), run_id.to_owned(), "slot".to_owned()]);
    let launched = run_cli(root, &invoke, "backlog_t03 invoke");
    let capture = PathBuf::from(
        launched["result"]["capture_dir"]
            .as_str()
            .expect("capture directory"),
    );
    let ready = root.join("worker-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "worker did not start");
        thread::sleep(Duration::from_millis(10));
    }
    let ownership_file = capture.join("ownership/ownership.json");
    while !ownership_file.is_file() {
        assert!(Instant::now() < deadline, "ownership was not published");
        thread::sleep(Duration::from_millis(10));
    }

    (
        database,
        capture,
        launched["result"]["invocation_id"]
            .as_str()
            .expect("invocation id")
            .to_owned(),
    )
}

fn cancel_args(database: &Path, run_id: &str, invocation_id: &str) -> Vec<String> {
    vec![
        "--database".to_owned(),
        database.to_string_lossy().into_owned(),
        "--json".to_owned(),
        "cancel-invocation".to_owned(),
        run_id.to_owned(),
        invocation_id.to_owned(),
    ]
}

#[test]
fn backlog_t03_unpublished_marker_is_ignored_by_public_cancellation() {
    let root = tempfile::tempdir().expect("unpublished-marker fixture tempdir");
    let run_id = "unpublished-marker-cancel";
    let (database, capture, invocation_id) = launch_blocked_invocation(root.path(), run_id);
    let ownership_dir = capture.join("ownership");
    let unpublished = ownership_dir.join("child-unpublished.json.tmp");
    fs::write(&unpublished, []).expect("write unpublished ownership marker");

    let cancelled = run_cli(
        root.path(),
        &cancel_args(&database, run_id, &invocation_id),
        "backlog_t03 unpublished cancellation",
    );
    assert_eq!(cancelled["result"]["status"], "failed");
    assert_eq!(cancelled["result"]["cancelled"], true);
    assert!(
        unpublished.is_file(),
        "cleanup must not rewrite the fixture"
    );
    assert!(
        !ownership::live_owned_work(capture.to_str().expect("capture path"))
            .expect("post-cancellation liveness")
    );
    assert!(!ownership::cleanup_pending(
        capture.to_str().expect("capture path")
    ));
}

#[test]
fn backlog_t03_malformed_published_marker_refuses_until_repaired() {
    let root = tempfile::tempdir().expect("malformed-marker fixture tempdir");
    let run_id = "malformed-marker-cancel";
    let (database, capture, invocation_id) = launch_blocked_invocation(root.path(), run_id);
    let ownership_dir = capture.join("ownership");
    let malformed = ownership_dir.join("child-malformed.json");
    fs::write(&malformed, []).expect("write malformed published ownership marker");

    let rejected_output = run_cli_output(
        root.path(),
        &cancel_args(&database, run_id, &invocation_id),
        "backlog_t03 malformed cancellation",
    );
    let rejected: Value = serde_json::from_slice(&rejected_output.stdout)
        .expect("malformed-marker cancellation JSON");
    assert_eq!(rejected_output.status.code(), Some(10));
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["code"], "ownership-unavailable");
    assert!(!ownership_dir.join("stop.json").exists());

    let owned: OwnedExecution =
        serde_json::from_slice(&fs::read(ownership_dir.join("ownership.json")).expect("ownership"))
            .expect("decode ownership");
    let root_identity = owned.root_identity.clone().expect("root identity");
    assert!(
        ownership::process_identity_matches(owned.root_pid, Some(&root_identity))
            .expect("verify genuine work remains")
    );

    let root_process = ownership::read_process(owned.root_pid)
        .expect("read genuine root")
        .expect("genuine root remains live");
    let repair = Admission::acquire(&ownership_dir).expect("repair marker admission");
    repair
        .write(
            "child-malformed.json",
            &json!({
                "root_pid": owned.root_pid,
                "parent_pid": root_process.parent_pid,
                "process_group_id": owned.process_group_id,
                "identity": root_process.identity,
            }),
        )
        .expect("repair published ownership marker");
    drop(repair);

    let cancelled = run_cli(
        root.path(),
        &cancel_args(&database, run_id, &invocation_id),
        "backlog_t03 repaired cancellation",
    );
    assert_eq!(cancelled["result"]["status"], "failed");
    assert_eq!(cancelled["result"]["cancelled"], true);
    assert!(
        !ownership::live_owned_work(capture.to_str().expect("capture path"))
            .expect("post-repair cancellation liveness")
    );
    assert!(!ownership::cleanup_pending(
        capture.to_str().expect("capture path")
    ));
}

fn common_args(database: &Path, config: &Path) -> Vec<String> {
    vec![
        "--database".to_owned(),
        database.to_string_lossy().into_owned(),
        "--config".to_owned(),
        config.to_string_lossy().into_owned(),
        "--json".to_owned(),
    ]
}

fn write_cli_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let database = root.join("loop.sqlite");
    let artifacts = root.join("artifacts");
    let provider = root.join("provider.py");
    let worker = root.join("worker.py");
    fs::create_dir_all(&artifacts).expect("artifact root");
    make_executable(
        &provider,
        r#"#!/usr/bin/env python3
import json, sys

request = json.load(sys.stdin)
if request.get("operation") == "describe":
    print(json.dumps({
        "id": "backlog-t03",
        "initial_state": "start",
        "states": [
            {"id": "start", "title": "Start", "instructions": "Run the worker", "final": False},
            {"id": "done", "title": "Done", "instructions": "Finished", "final": True},
        ],
        "transitions": [
            {"source": "start", "event": "finish", "target": "done", "kind": "check-free"}
        ],
        "work_slots": [{"id": "slot", "state": "start", "event": "finish"}],
    }))
else:
    print(json.dumps({"result": "allow"}))
"#,
    );
    make_executable(
        &worker,
        &format!(
            r#"#!/usr/bin/env python3
import json, pathlib, sys, time
packet = json.load(sys.stdin)
root = pathlib.Path({root:?})
(root / "worker-ready").write_text(json.dumps(packet))
while not (root / "worker-release").exists():
    time.sleep(0.01)
print("worker completed execution", flush=True)
raise SystemExit(7)
"#,
            root = root.to_string_lossy().to_string()
        ),
    );
    let config = root.join("providers.toml");
    fs::write(
        &config,
        format!(
            "[providers.fixture]\ncommand = \"{}\"\n",
            toml_quote(&provider.to_string_lossy())
        ),
    )
    .expect("provider catalog");
    (database, artifacts, config, worker)
}

#[test]
fn backlog_t03_native_identity_rejects_stale_numeric_owner_without_signaling_canary() {
    let root = tempfile::tempdir().expect("identity fixture tempdir");
    let ready = root.path().join("canary-ready");
    let signal_log = root.path().join("canary-signals");
    let canary = root.path().join("canary.py");
    make_executable(
        &canary,
        &format!(
            r#"#!/usr/bin/env python3
import pathlib, signal, time, os
ready = pathlib.Path({ready:?})
log = pathlib.Path({log:?})
def received(signum, frame):
    with log.open("a") as stream:
        stream.write(str(signum) + "\n")
signal.signal(signal.SIGTERM, received)
signal.signal(signal.SIGCONT, received)
ready.write_text(str(os.getpid()))
while True:
    time.sleep(0.05)
"#,
            ready = ready.to_string_lossy().to_string(),
            log = signal_log.to_string_lossy().to_string()
        ),
    );
    let mut command = Command::new(&canary);
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove(ownership::OWNERSHIP_ENV);
    super::bounded_process::prepare_process_group(&mut command);
    let mut child = command.spawn().expect("spawn test canary");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "canary did not start");
        thread::sleep(Duration::from_millis(10));
    }

    let identity = ownership::read_process_identity(child.id())
        .expect("read canary identity")
        .expect("canary identity");
    let process = ownership::read_process(child.id())
        .expect("read canary process")
        .expect("canary process");
    let stale = ProcessIdentity::new(
        identity.pid,
        identity.boot_id.clone(),
        identity.start_time.saturating_sub(1).max(1),
    );
    let ownership_dir = root.path().join("capture").join("ownership");
    fs::create_dir_all(&ownership_dir).expect("ownership directory");
    let admission = Admission::acquire(&ownership_dir).expect("ownership admission");
    admission
        .publish(&OwnedExecution {
            root_pid: identity.pid,
            process_group_id: process.process_group_id,
            root_identity: Some(stale.clone()),
            admission_directory: ownership_dir.clone(),
            graph_locator: None,
        })
        .expect("publish stale ownership");
    drop(admission);

    let current = ownership::read_processes().expect("native process snapshot");
    let resolved = ownership::discover_owned_processes(&ownership_dir, &current)
        .expect("resolve stale ownership");
    assert!(resolved.identity_usable);
    assert!(
        resolved.live.is_empty(),
        "stale owner adopted canary: {resolved:?}"
    );
    assert!(!ownership::live_owned_work(
        root.path().join("capture").to_str().expect("capture path")
    )
    .expect("stale ownership liveness"));
    assert!(
        !ownership::process_identity_matches(identity.pid, Some(&stale))
            .expect("stale identity comparison")
    );
    assert!(!ownership::signal_process(&stale, 15).expect("stale signal comparison"));
    thread::sleep(Duration::from_millis(100));
    assert!(
        !signal_log.exists(),
        "stale owner signaled unrelated canary"
    );

    // Drive the same stale ownership through the public cancellation command.
    // A separately recorded live child keeps cleanup necessary; the stale root
    // number is the unrelated canary and must remain untouched.
    let mut owned_child_command = Command::new("/bin/sleep");
    owned_child_command
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    super::bounded_process::prepare_process_group(&mut owned_child_command);
    let mut owned_child = owned_child_command.spawn().expect("spawn owned child");
    let child_identity = ownership::read_process_identity(owned_child.id())
        .expect("read owned child identity")
        .expect("owned child identity");
    let owned_child_pid = owned_child.id();
    let child_process = ownership::read_process(owned_child_pid)
        .expect("read owned child process")
        .expect("owned child process");
    let owned_child_waiter = thread::spawn(move || owned_child.wait());
    let marker_admission = Admission::acquire(&ownership_dir).expect("child marker admission");
    marker_admission
        .write(
            &format!("child-{}.json", owned_child_pid),
            &json!({
                "root_pid": owned_child_pid,
                "parent_pid": std::process::id(),
                "process_group_id": child_process.process_group_id,
                "identity": child_identity,
            }),
        )
        .expect("record owned child identity");
    drop(marker_admission);

    let database = root.path().join("stale-cancel.sqlite");
    let workflow = Workflow::new(
        "backlog-t03-cancel",
        "start",
        vec![State::new("start", "Start", "Cancel work", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("slot", "start", "finish")]);
    let persistence = SqlitePersistence::open(&database).expect("open stale cancel database");
    persistence
        .create_run(CreateRunRequest::new(
            "stale-cancel",
            None,
            workflow,
            ProviderAssociation::new(json!({"provider":"backlog-t03"})),
            json!({"artifact_root":root.path().to_string_lossy()}),
            "start",
            Lifecycle::Active,
            loop_core::Timestamp::from_unix_millis(1),
            "backlog-t03",
            None,
        ))
        .expect("create stale cancel run");
    persistence
        .load_show_data(&"stale-cancel".into())
        .expect("observe stale cancel run");
    persistence
        .set_current_slot_subject(&"stale-cancel".into(), &"slot".into(), "subject".to_owned())
        .expect("set stale cancel subject");
    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                "stale-cancel",
                "stale-invocation",
                "slot",
                loop_core::WorkSlotBinding::new("/bin/sleep", vec!["30".to_owned()]),
                "digest",
                "subject",
                identity.pid,
                loop_core::Timestamp::from_unix_millis(2),
                30_000,
                root.path().join("capture").to_string_lossy(),
            )
            .with_waiter_identity(stale.clone()),
        )
        .expect("create stale cancel invocation");

    let cancel_args = vec![
        "--database".to_owned(),
        database.to_string_lossy().into_owned(),
        "--json".to_owned(),
        "cancel-invocation".to_owned(),
        "stale-cancel".to_owned(),
        "stale-invocation".to_owned(),
    ];
    let cancelled = run_cli(root.path(), &cancel_args, "backlog_t03 stale cancellation");
    assert_eq!(cancelled["result"]["status"], "failed");
    assert!(cancelled["result"]["cancelled"] == true);
    assert!(
        ownership::process_identity_matches(identity.pid, Some(&identity))
            .expect("canary remains live")
    );
    assert!(!signal_log.exists(), "public cancellation signaled canary");
    assert!(!ownership::live_owned_work(
        root.path().join("capture").to_str().expect("capture path")
    )
    .expect("post-cancellation owned work"));
    owned_child_waiter
        .join()
        .expect("owned child wait thread")
        .expect("reap cancelled owned child");

    child.kill().expect("stop test canary");
    let _ = child.wait();
}

#[test]
fn backlog_t03_root_loss_retains_recorded_child_incarnation() {
    let root = tempfile::tempdir().expect("root-loss fixture tempdir");
    let ownership_dir = root.path().join("ownership");
    fs::create_dir_all(&ownership_dir).expect("ownership directory");
    let root_identity = ProcessIdentity::new(4_000_001, "test-boot", 1);
    let child_identity = ProcessIdentity::new(4_000_002, "test-boot", 2);
    fs::write(
        ownership_dir.join("ownership.json"),
        serde_json::to_vec(&OwnedExecution {
            root_pid: root_identity.pid,
            process_group_id: 4_000_000,
            root_identity: Some(root_identity.clone()),
            admission_directory: ownership_dir.clone(),
            graph_locator: None,
        })
        .expect("ownership JSON"),
    )
    .expect("write ownership");
    fs::write(
        ownership_dir.join("child-4000002.json"),
        serde_json::to_vec(&json!({
            "root_pid": child_identity.pid,
            "parent_pid": root_identity.pid,
            "process_group_id": 4_000_000,
            "identity": child_identity,
        }))
        .expect("child JSON"),
    )
    .expect("write child identity");

    let resolved = ownership::discover_owned_processes(
        &ownership_dir,
        &[loop_integrations::ownership::ProcessRecord {
            identity: ProcessIdentity::new(4_000_002, "test-boot", 2),
            parent_pid: root_identity.pid,
            process_group_id: 4_000_000,
            state: "R".to_owned(),
        }],
    )
    .expect("resolve root-loss ownership");
    assert!(resolved.identity_usable);
    assert_eq!(resolved.live.len(), 1);
    assert_eq!(resolved.live[0].identity.pid, child_identity.pid);
    assert!(resolved.anchored_groups.contains(&4_000_000));
}

#[test]
fn backlog_t03_completion_receipt_does_not_promote_uncommitted_catalog_row() {
    let root = tempfile::tempdir().expect("completion fixture tempdir");
    let (database, artifacts, config, worker) = write_cli_fixture(root.path());
    let mut start = common_args(&database, &config);
    start.extend([
        "start".to_owned(),
        "fixture".to_owned(),
        serde_json::to_string(&json!({
            "artifact_root": artifacts,
            "work_slot_bindings": {
                "slot": {"command": worker, "args": []}
            }
        }))
        .expect("start input"),
        "--id".to_owned(),
        "receipt-reconciliation".to_owned(),
    ]);
    assert_eq!(
        run_cli(root.path(), &start, "backlog_t03 start")["status"],
        "completed"
    );

    let mut show = common_args(&database, &config);
    show.extend([
        "show".to_owned(),
        "receipt-reconciliation".to_owned(),
        "--view".to_owned(),
        "full".to_owned(),
    ]);
    run_cli(root.path(), &show, "backlog_t03 arm invoke");

    let mut invoke = common_args(&database, &config);
    invoke.extend([
        "invoke".to_owned(),
        "receipt-reconciliation".to_owned(),
        "slot".to_owned(),
    ]);
    let launched = run_cli(root.path(), &invoke, "backlog_t03 invoke");
    let capture = PathBuf::from(
        launched["result"]["capture_dir"]
            .as_str()
            .expect("capture directory"),
    );
    let ready = root.path().join("worker-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "worker did not start");
        thread::sleep(Duration::from_millis(10));
    }

    let invocation_id = launched["result"]["invocation_id"]
        .as_str()
        .expect("invocation id")
        .to_owned();
    let connection = Connection::open(&database).expect("open completion fixture database");
    let (waiter_pid, waiter_identity_json): (i64, Option<String>) = connection
        .query_row(
            "SELECT waiter_pid, waiter_identity_json FROM work_slot_invocations WHERE invocation_id = ?1",
            [&invocation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read waiter identity");
    let waiter_identity: ProcessIdentity =
        serde_json::from_str(&waiter_identity_json.expect("new invocation waiter identity"))
            .expect("decode waiter identity");
    assert_eq!(waiter_identity.pid, waiter_pid as u32);

    // Hold the catalog write transaction while the real waiter completes its
    // worker. It writes the engine-owned completion receipt first, then blocks
    // at the SQLite transaction boundary. Killing that CLI process leaves an
    // honest incomplete catalog row and a retained receipt.
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold completion transaction");
    fs::write(root.path().join("worker-release"), b"release").expect("release worker");
    let receipt_path = capture.join("ownership/waiter-completion.json");
    let receipt_deadline = Instant::now() + Duration::from_secs(10);
    while !receipt_path.is_file() {
        assert!(
            Instant::now() < receipt_deadline,
            "waiter did not retain completion receipt"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ownership::process_identity_matches(waiter_pid as u32, Some(&waiter_identity))
            .expect("verify waiter before interruption")
    );
    let killed = unsafe { libc::kill(waiter_pid as libc::pid_t, libc::SIGKILL) };
    assert_eq!(killed, 0, "could not interrupt waiter {waiter_pid}");
    // Rollback the deliberately interrupted transaction after the waiter is
    // gone; no catalog mutation from the blocked CLI can commit.
    drop(connection);
    let mut observed = None;
    for _ in 0..300 {
        let value = run_cli(root.path(), &show, "backlog_t03 completion inspection");
        let row = value["result"]["work_slot_invocations"]
            .as_array()
            .expect("invocation views")
            .iter()
            .find(|row| row["invocation_id"] == invocation_id)
            .expect("completion invocation");
        if row["status"] == "failed"
            && row["ownership"]["live_owned_work"] == false
            && row["ownership"]["cleanup_pending"] == false
            && receipt_path.is_file()
            && !ownership::process_identity_matches(waiter_pid as u32, Some(&waiter_identity))
                .expect("check interrupted waiter")
        {
            observed = Some(row.clone());
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let row = observed.expect("interrupted completion was not observed");
    // bookends:LE-129 — public show retains the real failed outcome and
    // cleanup barrier after the completion receipt cannot commit its row.
    assert_eq!(row["status"], "failed");
    assert_eq!(row["exit_code"], Value::Null);
    assert_eq!(row["ownership"]["cleanup_pending"], false);

    let connection = Connection::open(&database).expect("reopen completion fixture database");
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM work_slot_invocations WHERE invocation_id = ?1",
            [&invocation_id],
            |row| row.get(0),
        )
        .expect("read catalog status");
    assert_eq!(status, None, "failed transaction invented a terminal row");
    let receipt: Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("completion receipt"))
            .expect("completion receipt JSON");
    assert_eq!(receipt["exit_code"], 7);

    let mut history = common_args(&database, &config);
    history.extend(["history".to_owned(), "receipt-reconciliation".to_owned()]);
    let history = run_cli(root.path(), &history, "backlog_t03 completion history");
    assert_eq!(
        history["result"]
            .as_array()
            .expect("history rows")
            .iter()
            .filter(|entry| entry["action"]["kind"] == "invocation_status_changed")
            .count(),
        0,
        "uncommitted completion created semantic history"
    );
}
