use loop_core::{
    CreateRunRequest, CreateWorkSlotInvocationRequest, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WorkSlot, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "test-workflow",
        "start",
        vec![
            State::new("start", "Start", "Begin the work", false),
            State::new("middle", "Middle", "Continue", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::checked("start", "approve", "middle"),
            Transition::checked("start", "retry", "start"),
            Transition::check_free("middle", "finish", "done"),
        ],
    )
    .with_work_slots(vec![WorkSlot::new("slot-1", "start", "approve")])
}

fn create_request(id: &str, bindings: Option<Value>, artifact_root: &str) -> CreateRunRequest {
    let mut initial_input = json!({
        "objective": "durable",
        "artifact_root": artifact_root,
    });
    if let Some(bindings) = bindings {
        initial_input
            .as_object_mut()
            .expect("object initial_input")
            .insert("work_slot_bindings".to_owned(), bindings);
    }
    CreateRunRequest::new(
        id,
        Some(format!("label-{id}")),
        workflow(),
        ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
        initial_input,
        "start",
        Lifecycle::Active,
        Timestamp::from_unix_millis(100),
        "test-provider",
        Some(artifact_root.to_owned()),
    )
}

fn slot_binding(command: &str, args: Vec<String>) -> Value {
    json!({
        "slot-1": {
            "command": command,
            "args": args,
        }
    })
}

fn seed_run(
    database: &Path,
    run_id: &str,
    bindings: Option<Value>,
    artifact_root: &str,
    subject: Option<&str>,
) {
    let persistence = SqlitePersistence::open(database).expect("open sqlite");
    persistence
        .create_run(create_request(run_id, bindings, artifact_root))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
    if let Some(subject) = subject {
        persistence
            .set_current_slot_subject(&run_id.into(), &"slot-1".into(), subject.to_owned())
            .expect("set current slot subject");
    }
}

fn load_invocations(database: &Path, run_id: &str) -> Vec<loop_core::WorkSlotInvocation> {
    let persistence = SqlitePersistence::open(database).expect("reopen sqlite");
    persistence
        .load_work_slot_invocations(&run_id.into())
        .expect("load invocations")
}

fn run_invoke(
    database: &Path,
    extra: &[&str],
    run_id: &str,
    slot_id: &str,
) -> std::process::Output {
    let mut args = vec![
        "--database".to_owned(),
        database.to_str().expect("utf-8 database path").to_owned(),
        "--json".to_owned(),
    ];
    args.extend(extra.iter().map(|value| (*value).to_owned()));
    args.extend(["invoke".to_owned(), run_id.to_owned(), slot_id.to_owned()]);
    Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args(args)
        .output()
        .expect("run invoke")
}

fn wait_until_terminal(
    database: &Path,
    run_id: &str,
    expected_count: usize,
    timeout: Duration,
) -> loop_core::WorkSlotInvocation {
    let started = Instant::now();
    loop {
        let invocations = load_invocations(database, run_id);
        if invocations.len() >= expected_count {
            if let Some(latest) = invocations.last() {
                if latest.status.is_some() {
                    return latest.clone();
                }
            }
        }
        if started.elapsed() > timeout {
            panic!(
                "timed out waiting for terminal invocation; last={:?}",
                load_invocations(database, run_id)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn terminate_pid(pid: u32) {
    if pid <= 1 {
        return;
    }
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[test]
fn invoke_fan_out_subset_starts_only_selected_assignment_and_records_it() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let worker_zero = directory.path().join("worker-zero.started");
    let worker_one = directory.path().join("worker-one.started");
    let worker = |label: &str, receipt: &Path| {
        json!({
            "command": "/bin/sh",
            "args": [
                "-c",
                format!(r#"printf '%s' '{label}' > "$1"; exit 0"#),
                "_",
                receipt.to_string_lossy(),
            ]
        })
        .to_string()
    };
    let binding = json!({
        "slot-1": {
            "command": env!("CARGO_BIN_EXE_loop-engine"),
            "args": [
                "fan-out",
                "--worker", worker("zero", &worker_zero),
                "--worker", worker("one", &worker_one),
            ]
        }
    });
    seed_run(
        &database,
        "run-subset",
        Some(binding.clone()),
        &directory.path().to_string_lossy(),
        Some("subject-1"),
    );

    let output = run_invoke(
        &database,
        &["--assignment", "worker-1"],
        "run-subset",
        "slot-1",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let invocation = wait_until_terminal(&database, "run-subset", 1, Duration::from_secs(10));
    assert_eq!(
        invocation.assignment_selection,
        Some(vec!["worker-1".to_owned()])
    );
    assert_eq!(
        invocation.binding,
        loop_core::WorkSlotBinding::new(
            env!("CARGO_BIN_EXE_loop-engine"),
            vec![
                "fan-out".to_owned(),
                "--worker".to_owned(),
                worker("zero", &worker_zero),
                "--worker".to_owned(),
                worker("one", &worker_one),
            ],
        )
    );
    assert!(!worker_zero.exists(), "unselected worker started");
    assert_eq!(
        std::fs::read_to_string(&worker_one).expect("selected receipt"),
        "one"
    );
    assert_eq!(invocation.inner_workers.len(), 1);
    assert_eq!(invocation.inner_workers[0].assignment_id, "worker-1");

    let shown = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "--json",
            "show",
            "run-subset",
        ])
        .output()
        .expect("show subset invocation");
    assert!(shown.status.success(), "{shown:?}");
    let shown: Value = serde_json::from_slice(&shown.stdout).expect("show json");
    assert_eq!(
        shown["result"]["work_slot_invocations"][0]["assignment_selection"],
        json!(["worker-1"])
    );
}

#[test]
fn invoke_fan_out_omitted_selection_starts_every_assignment() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let worker_zero = directory.path().join("worker-zero.started");
    let worker_one = directory.path().join("worker-one.started");
    let worker = |label: &str, receipt: &Path| {
        json!({
            "command": "/bin/sh",
            "args": [
                "-c",
                format!(r#"printf '%s' '{label}' > "$1"; exit 0"#),
                "_",
                receipt.to_string_lossy(),
            ]
        })
        .to_string()
    };
    let binding = json!({
        "slot-1": {
            "command": env!("CARGO_BIN_EXE_loop-engine"),
            "args": [
                "fan-out",
                "--worker", worker("zero", &worker_zero),
                "--worker", worker("one", &worker_one),
            ]
        }
    });
    seed_run(
        &database,
        "run-full-selection",
        Some(binding),
        &directory.path().to_string_lossy(),
        Some("subject-1"),
    );

    let output = run_invoke(&database, &[], "run-full-selection", "slot-1");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let invocation =
        wait_until_terminal(&database, "run-full-selection", 1, Duration::from_secs(10));
    assert_eq!(invocation.assignment_selection, None);
    assert_eq!(std::fs::read_to_string(&worker_zero).unwrap(), "zero");
    assert_eq!(std::fs::read_to_string(&worker_one).unwrap(), "one");
    assert_eq!(
        invocation
            .inner_workers
            .iter()
            .map(|worker| worker.assignment_id.as_str())
            .collect::<Vec<_>>(),
        vec!["worker-0", "worker-1"]
    );
}

#[test]
fn invoke_invalid_assignment_selections_refuse_before_any_process_starts() {
    for (run_id, extra, expected_code) in [
        (
            "run-empty-selection",
            vec!["--assignments=[]"],
            "empty-assignment-selection",
        ),
        (
            "run-unknown-selection",
            vec!["--assignment", "missing"],
            "unknown-assignment",
        ),
        (
            "run-duplicate-selection",
            vec!["--assignment", "worker-0", "--assignment", "worker-0"],
            "duplicate-assignment-selection",
        ),
    ] {
        let directory = tempdir().expect("tempdir");
        let database = directory.path().join("loop.db");
        let worker_zero = directory.path().join("worker-zero.started");
        let worker_one = directory.path().join("worker-one.started");
        let worker = |receipt: &Path| {
            json!({
                "command": "/bin/sh",
                "args": [
                    "-c",
                    "printf started > \"$1\"; exit 0",
                    "_",
                    receipt.to_string_lossy(),
                ]
            })
            .to_string()
        };
        let binding = json!({
            "slot-1": {
                "command": env!("CARGO_BIN_EXE_loop-engine"),
                "args": [
                    "fan-out",
                    "--worker", worker(&worker_zero),
                    "--worker", worker(&worker_one),
                ]
            }
        });
        seed_run(
            &database,
            run_id,
            Some(binding),
            &directory.path().to_string_lossy(),
            Some("subject-1"),
        );

        let output = run_invoke(&database, &extra, run_id, "slot-1");
        assert_eq!(output.status.code(), Some(10), "{output:?}");
        let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
        assert_eq!(parsed["status"], "rejected");
        assert_eq!(parsed["code"], expected_code);
        assert!(load_invocations(&database, run_id).is_empty());
        assert!(
            !worker_zero.exists(),
            "worker zero started for {expected_code}"
        );
        assert!(
            !worker_one.exists(),
            "worker one started for {expected_code}"
        );
    }
}

#[test]
fn invoke_selection_on_opaque_binding_refuses_before_process_start() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let receipt = directory.path().join("opaque.started");
    let binding = slot_binding(
        "/bin/sh",
        vec![
            "-c".to_owned(),
            "printf started > \"$1\"; exit 0".to_owned(),
            "_".to_owned(),
            receipt.to_string_lossy().into_owned(),
        ],
    );
    seed_run(
        &database,
        "run-opaque-selection",
        Some(binding),
        &directory.path().to_string_lossy(),
        Some("subject-1"),
    );

    let output = run_invoke(
        &database,
        &["--assignment", "worker-0"],
        "run-opaque-selection",
        "slot-1",
    );
    assert_eq!(output.status.code(), Some(10), "{output:?}");
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(parsed["status"], "rejected");
    assert_eq!(parsed["code"], "assignments-not-enumerable");
    assert!(load_invocations(&database, "run-opaque-selection").is_empty());
    assert!(
        !receipt.exists(),
        "opaque worker started before selection refusal"
    );
}

#[test]
fn invoke_unbound_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    seed_run(
        &database,
        "run-unbound",
        None,
        "/tmp/artifacts",
        Some("subject-1"),
    );
    let output = run_invoke(&database, &[], "run-unbound", "slot-1");
    assert_eq!(output.status.code(), Some(10), "{output:?}");
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(parsed["status"], "rejected");
    assert_eq!(parsed["code"], "unbound-work-slot");
    assert!(load_invocations(&database, "run-unbound").is_empty());
}

#[test]
fn invoke_unknown_slot_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    seed_run(
        &database,
        "run-unknown",
        Some(slot_binding(
            "sh",
            vec!["-c".to_owned(), "exit 0".to_owned()],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );
    let output = run_invoke(&database, &[], "run-unknown", "no-such-slot");
    assert_eq!(output.status.code(), Some(10), "{output:?}");
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(parsed["status"], "rejected");
    assert_eq!(parsed["code"], "unknown-work-slot");
    assert!(load_invocations(&database, "run-unknown").is_empty());
}

#[test]
fn invoke_already_running_live_waiter_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    seed_run(
        &database,
        "run-running",
        Some(slot_binding(
            "sh",
            vec!["-c".to_owned(), "sleep 30".to_owned()],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );
    let first = run_invoke(&database, &[], "run-running", "slot-1");
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let second = run_invoke(&database, &[], "run-running", "slot-1");
    assert_eq!(second.status.code(), Some(10), "{second:?}");
    let parsed: Value = serde_json::from_slice(&second.stdout).expect("json stdout");
    assert_eq!(parsed["status"], "rejected");
    assert_eq!(parsed["code"], "work-slot-already-running");
    let invocations = load_invocations(&database, "run-running");
    assert_eq!(invocations.len(), 1);
    terminate_pid(invocations[0].waiter_pid);
}

#[test]
fn invoke_overlay_overrun_is_not_already_running() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let packet_file = directory.path().join("packet.json");
    seed_run(
        &database,
        "run-overrun",
        Some(slot_binding(
            "sh",
            vec![
                "-c".to_owned(),
                "cat > \"$1\"; exit 0".to_owned(),
                "_".to_owned(),
                packet_file.to_string_lossy().into_owned(),
            ],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );

    let mut live = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn live pid");
    let live_pid = live.id();

    let persistence = SqlitePersistence::open(&database).expect("open sqlite");
    persistence
        .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
            "run-overrun",
            "inv-old",
            "slot-1",
            WorkSlotBinding::new("sh", vec!["-c".to_owned(), "sleep 60".to_owned()]),
            "digest",
            "subject-1",
            live_pid,
            Timestamp::from_unix_millis(now_millis() - 10_000),
            1,
            String::new(),
        ))
        .expect("create overrun record");

    let output = run_invoke(&database, &[], "run-overrun", "slot-1");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let invocations = load_invocations(&database, "run-overrun");
    assert_eq!(invocations.len(), 2);

    wait_until_terminal(&database, "run-overrun", 2, Duration::from_secs(5));
    terminate_pid(live_pid);
    let _ = live.kill();
    let _ = live.wait();
}

#[test]
fn invoke_happy_path_writes_worker_packet_without_command() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let packet_file = directory.path().join("packet.json");
    let artifact_root = directory.path().to_string_lossy().into_owned();
    seed_run(
        &database,
        "run-happy",
        Some(slot_binding(
            "sh",
            vec![
                "-c".to_owned(),
                "cat > \"$1\"; exit 0".to_owned(),
                "_".to_owned(),
                packet_file.to_string_lossy().into_owned(),
            ],
        )),
        &artifact_root,
        Some("subject-1"),
    );

    let output = run_invoke(&database, &[], "run-happy", "slot-1");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(parsed["status"], "completed");
    assert_eq!(parsed["result"]["slot_id"], "slot-1");
    assert!(parsed["result"].get("waiter_pid").is_none());
    let capture_dir = parsed["result"]["capture_dir"]
        .as_str()
        .expect("capture_dir string")
        .to_owned();
    assert!(
        capture_dir.starts_with(&format!("{artifact_root}/work-slot-captures/slot-1/")),
        "capture_dir {capture_dir} should include invocation id under the slot"
    );
    assert!(Path::new(&capture_dir).is_dir());

    wait_until_terminal(&database, "run-happy", 1, Duration::from_secs(5));
    let captured = std::fs::read_to_string(&packet_file).expect("read worker packet file");
    let packet: Value = serde_json::from_str(&captured).expect("worker stdin json");
    let object = packet.as_object().expect("packet object");
    assert_eq!(object.len(), 5);
    assert!(object.contains_key("run_id"));
    assert!(object.contains_key("slot_id"));
    assert!(object.contains_key("artifact_root"));
    assert!(object.contains_key("instruction_body"));
    assert!(object.contains_key("capture_dir"));
    assert_eq!(
        packet,
        json!({
            "run_id": "run-happy",
            "slot_id": "slot-1",
            "artifact_root": artifact_root,
            "instruction_body": "Begin the work",
            "capture_dir": capture_dir,
        })
    );
    assert!(
        packet.get("command").is_none(),
        "worker packet must not contain command: {captured}"
    );
    assert!(
        !captured.contains("\"command\""),
        "worker packet must not contain command: {captured}"
    );
}

#[test]
fn invoke_worker_ppid_equals_waiter_pid_not_invoke() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let ppid_file = directory.path().join("ppid.txt");
    seed_run(
        &database,
        "run-ppid",
        Some(slot_binding(
            "sh",
            vec![
                "-c".to_owned(),
                "printf %s \"$PPID\" > \"$1\"; exit 0".to_owned(),
                "_".to_owned(),
                ppid_file.to_string_lossy().into_owned(),
            ],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );

    let child = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "--json",
            "invoke",
            "run-ppid",
            "slot-1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn invoke");
    let invoke_pid = child.id();
    let output = child.wait_with_output().expect("wait for invoke");
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    wait_until_terminal(&database, "run-ppid", 1, Duration::from_secs(5));
    let waiter_pid = load_invocations(&database, "run-ppid")[0].waiter_pid;
    let recorded = std::fs::read_to_string(&ppid_file).expect("read ppid file");
    assert_eq!(recorded, waiter_pid.to_string());
    assert_ne!(recorded, invoke_pid.to_string());
}

#[test]
fn invoke_allowed_time_ms_equals_timeout_ms_flag() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let packet_file = directory.path().join("packet.json");
    seed_run(
        &database,
        "run-timeout",
        Some(slot_binding(
            "sh",
            vec![
                "-c".to_owned(),
                "cat > \"$1\"; exit 0".to_owned(),
                "_".to_owned(),
                packet_file.to_string_lossy().into_owned(),
            ],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );
    let output = run_invoke(
        &database,
        &["--timeout-ms", "12345"],
        "run-timeout",
        "slot-1",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(parsed["result"]["allowed_time_ms"], 12345);
    wait_until_terminal(&database, "run-timeout", 1, Duration::from_secs(5));
    let invocations = load_invocations(&database, "run-timeout");
    assert_eq!(invocations[0].allowed_time_ms, 12_345);
}

#[test]
fn invoke_packet_is_not_present_on_worker_argv() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let argv_file = directory.path().join("argv.txt");
    seed_run(
        &database,
        "run-argv",
        Some(slot_binding(
            "sh",
            vec![
                "-c".to_owned(),
                "cat > /dev/null; printf %s \"$#|$0|$*\" > \"$1\"; exit 0".to_owned(),
                "_".to_owned(),
                argv_file.to_string_lossy().into_owned(),
            ],
        )),
        "/tmp/artifacts",
        Some("subject-1"),
    );
    let output = run_invoke(&database, &[], "run-argv", "slot-1");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    wait_until_terminal(&database, "run-argv", 1, Duration::from_secs(5));
    let argv = std::fs::read_to_string(&argv_file).expect("read argv file");
    assert!(
        !argv.contains("instruction_body"),
        "packet must not be on worker argv: {argv}"
    );
    assert!(
        !argv.contains("run-argv"),
        "packet must not be on worker argv: {argv}"
    );
    assert!(
        !argv.contains("artifact_root"),
        "packet must not be on worker argv: {argv}"
    );
}

#[test]
fn elapsed_time_overrun_then_retry_show_history_gate() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.db");
    let artifact_root = directory.path().to_string_lossy().into_owned();
    seed_run(
        &database,
        "run-elapsed-overrun",
        Some(slot_binding(
            "sh",
            vec!["-c".to_owned(), "sleep 30".to_owned()],
        )),
        &artifact_root,
        Some("subject-1"),
    );

    struct KillWaiters {
        database: std::path::PathBuf,
        run_id: String,
    }
    impl Drop for KillWaiters {
        fn drop(&mut self) {
            for invocation in load_invocations(&self.database, &self.run_id) {
                terminate_pid(invocation.waiter_pid);
            }
        }
    }
    let _guard = KillWaiters {
        database: database.clone(),
        run_id: "run-elapsed-overrun".to_owned(),
    };

    let first = run_invoke(
        &database,
        &["--timeout-ms", "50"],
        "run-elapsed-overrun",
        "slot-1",
    );
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let first_invocations = load_invocations(&database, "run-elapsed-overrun");
    assert_eq!(first_invocations.len(), 1);
    assert!(
        first_invocations[0].status.is_none(),
        "first invoke must return while waiter is still unwritten: {:?}",
        first_invocations[0]
    );

    std::thread::sleep(Duration::from_millis(80));

    let show = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "--json",
            "show",
            "run-elapsed-overrun",
        ])
        .output()
        .expect("run show");
    assert_eq!(show.status.code(), Some(0), "{show:?}");
    let show_json: Value = serde_json::from_slice(&show.stdout).expect("show json");
    let invocations = show_json["result"]["work_slot_invocations"]
        .as_array()
        .expect("work_slot_invocations");
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0]["status"], "overrun");
    assert!(
        invocations[0].get("waiter_pid").is_none(),
        "waiter_pid must be absent: {}",
        invocations[0]
    );
    let invocation_id = invocations[0]["invocation_id"]
        .as_str()
        .expect("invocation_id")
        .to_owned();
    let slots = show_json["result"]["work_slots"]
        .as_array()
        .expect("work_slots");
    for slot in slots {
        let object = slot.as_object().expect("work_slots entry");
        assert_eq!(
            object.len(),
            3,
            "work_slots entries have only id/state/event: {slot}"
        );
        assert!(object.contains_key("id"), "{slot}");
        assert!(object.contains_key("state"), "{slot}");
        assert!(object.contains_key("event"), "{slot}");
    }

    let history = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "--json",
            "history",
            "run-elapsed-overrun",
        ])
        .output()
        .expect("run history");
    assert_eq!(history.status.code(), Some(0), "{history:?}");
    let history_json: Value = serde_json::from_slice(&history.stdout).expect("history json");
    let entries = history_json["result"].as_array().expect("history result");
    let mut saw_started = false;
    for entry in entries {
        let action = &entry["action"];
        let kind = action["kind"].as_str();
        if kind == Some("invocation_started") && action["invocation_id"] == invocation_id {
            saw_started = true;
        }
        assert_ne!(
            kind,
            Some("overlay_overrun"),
            "must not add overlay-overrun as a HistoryAction: {entry}"
        );
        if kind == Some("invocation_status_changed")
            && action["invocation_id"] == invocation_id
            && action["status"] == "succeeded"
        {
            panic!(
                "history must not contain waiter-written succeeded for {invocation_id}: {entry}"
            );
        }
    }
    assert!(
        saw_started,
        "history must contain InvocationStarted for {invocation_id}: {history_json}"
    );

    let event = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database path"),
            "--json",
            "event",
            "run-elapsed-overrun",
            "approve",
        ])
        .output()
        .expect("run event");
    assert_eq!(event.status.code(), Some(10), "{event:?}");
    let event_json: Value = serde_json::from_slice(&event.stdout).expect("event json");
    assert_eq!(event_json["status"], "rejected");
    assert_eq!(event_json["code"], "bound-slot-invocation-required");

    let second = run_invoke(&database, &[], "run-elapsed-overrun", "slot-1");
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    let parsed_second: Value = serde_json::from_slice(&second.stdout).expect("second invoke json");
    assert_eq!(parsed_second["status"], "completed");
    assert_ne!(parsed_second["code"], "work-slot-already-running");
}
