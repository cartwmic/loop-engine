use super::bounded_process::CommandExt;
use loop_core::{
    CompleteWorkSlotInvocationRequest, CreateRunRequest, CreateWorkSlotInvocationRequest,
    Lifecycle, Persistence, ProviderAssociation, State, Timestamp, Transition, WaiterWrittenStatus,
    WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "workflow",
        "start",
        vec![State::new("start", "Start", "Do the work", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
}

fn seed_run(database: &Path, run_id: &str, invocation_id: &str, capture_dir: &str) {
    seed_run_at(database, run_id, invocation_id, capture_dir, 1_000);
}

fn seed_run_at(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    capture_dir: &str,
    started_at: i64,
) {
    seed_run_at_with_pid(database, run_id, invocation_id, capture_dir, started_at, 1);
}

fn seed_run_at_with_pid(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    capture_dir: &str,
    started_at: i64,
    waiter_pid: u32,
) {
    let persistence = SqlitePersistence::open(database).expect("open sqlite");
    persistence
        .create_run(CreateRunRequest::new(
            run_id,
            Some(format!("label-{run_id}")),
            workflow(),
            ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
            json!({"objective": "progress"}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(100),
            "test-provider",
            Some("/allocated/run-dir".to_owned()),
        ))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
    persistence
        .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            "slot-1",
            WorkSlotBinding::new("echo", vec!["ok".to_owned()]),
            "digest",
            "subject",
            waiter_pid,
            Timestamp::from_unix_millis(started_at),
            60_000,
            capture_dir,
        ))
        .expect("create invocation");
}

fn complete_invocation(database: &Path, run_id: &str, invocation_id: &str, completed_at: i64) {
    let persistence = SqlitePersistence::open(database).expect("open sqlite");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(completed_at),
            Vec::new(),
        ))
        .expect("complete invocation");
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis() as i64
}

fn load_status(database: &Path, run_id: &str) -> Option<WaiterWrittenStatus> {
    let persistence = SqlitePersistence::open(database).expect("reopen sqlite");
    persistence
        .load_work_slot_invocations(&run_id.into())
        .expect("load invocations")
        .into_iter()
        .next()
        .and_then(|invocation| invocation.status)
}

fn prepend_path(directory: &Path) -> OsString {
    let mut dirs = vec![directory.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&existing));
    }
    env::join_paths(dirs).expect("join PATH")
}

fn path_without_dagu() -> OsString {
    let mut dirs = Vec::new();
    if let Some(existing) = env::var_os("PATH") {
        for dir in env::split_paths(&existing) {
            if dir.join("dagu").is_file() {
                continue;
            }
            dirs.push(dir);
        }
    }
    dirs.push(PathBuf::from("/bin"));
    dirs.push(PathBuf::from("/usr/bin"));
    env::join_paths(dirs).expect("join PATH without dagu")
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write stub");
    let mut permissions = fs::metadata(path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod stub");
}

fn write_status_ok_stub(directory: &Path) {
    write_executable(
        &directory.join("dagu"),
        "#!/bin/sh\n\
case \"$1\" in\n\
  version|--version) printf '2.14.0\\n'; exit 0 ;;\n\
  status)\n\
    printf 'human dagu status must not become the snapshot\\n'\n\
    printf 'running\\n'\n\
    exit 0\n\
    ;;\n\
  *) printf 'unexpected dagu argv\\n' >&2; exit 42 ;;\n\
esac\n",
    );
}

fn run_progress(database: &Path, run_id: &str, path: OsString) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--json",
            "--database",
            database.to_str().expect("utf-8 database"),
            "invocation-progress",
            run_id,
        ])
        .env("PATH", path)
        .bounded_output("loop-engine invocation-progress")
        .expect("run invocation-progress")
}

fn run_show_compact(database: &Path, run_id: &str, path: OsString) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--database",
            database.to_str().expect("utf-8 database"),
            "show",
            "--compact",
            run_id,
        ])
        .env("PATH", path)
        .bounded_output("loop-engine show --compact")
        .expect("run compact show")
}

fn run_show_json(database: &Path, run_id: &str, path: OsString) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args([
            "--json",
            "--database",
            database.to_str().expect("utf-8 database"),
            "show",
            run_id,
        ])
        .env("PATH", path)
        .bounded_output("loop-engine --json show")
        .expect("run JSON show")
}

fn write_graph_fixture(capture: &Path) -> PathBuf {
    let home = capture.join("dagu-home");
    let dags = home.join("dags");
    let status_dir = home.join("data").join("dag-run").join("live");
    fs::create_dir_all(&dags).expect("dags");
    fs::create_dir_all(&status_dir).expect("status dir");
    let locator = json!({
        "dagu_home": home.to_string_lossy(),
        "dag_name": "fanout-progress",
        "run_name": "fanout-progress",
    });
    fs::write(
        capture.join("dagu-locator.json"),
        serde_json::to_vec(&locator).expect("locator json"),
    )
    .expect("write locator");
    fs::write(
        dags.join("fanout-progress.yaml"),
        concat!(
            "type: graph\n",
            "steps:\n",
            "  - name: \"w0\"\n",
            "    action: exec\n",
            "  - name: \"w1\"\n",
            "    action: exec\n",
            "  - name: \"join\"\n",
            "    action: exec\n",
        ),
    )
    .expect("write yaml");
    fs::write(
        status_dir.join("status.jsonl"),
        concat!(
            r#"{"nodes":[{"status":0,"step":{"name":"w0"}}]}"#,
            "\n",
            r#"{"nodes":[{"status":1,"step":{"name":"w0"},"startedAt":"2024-01-01T00:00:00Z"},{"status":4,"name":"join","startedAt":"2024-01-01T00:00:01Z"}]}"#,
            "\n",
        ),
    )
    .expect("write status.jsonl");
    home
}

#[test]
fn public_compact_show_covers_running_completed_and_unavailable_progress() {
    let root = tempdir().expect("tempdir");
    let database = root.path().join("loop.db");
    let stub_dir = root.path().join("bin");
    fs::create_dir_all(&stub_dir).expect("stub dir");
    write_status_ok_stub(&stub_dir);
    let dagu_path = prepend_path(&stub_dir);

    let running_capture = root.path().join("running-capture");
    fs::create_dir_all(&running_capture).expect("running capture");
    write_graph_fixture(&running_capture);
    let mut waiter = Command::new("/bin/sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("running waiter");
    seed_run_at_with_pid(
        &database,
        "run-compact-running",
        "inv-compact-running",
        running_capture.to_str().expect("utf8 running capture"),
        now_millis() - 1_000,
        waiter.id(),
    );
    let running = run_show_compact(&database, "run-compact-running", dagu_path.clone());
    assert!(
        running.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&running.stderr),
        String::from_utf8_lossy(&running.stdout)
    );
    let running_text = String::from_utf8_lossy(&running.stdout);
    assert!(running_text.contains("completed show --compact"));
    assert!(running_text.contains("lifecycle: active"));
    assert!(running_text.contains("state: start (Start)"));
    assert!(running_text.contains("requestable events: finish -> start (check-free)"));
    assert!(running_text.contains("latest checked result: none"));
    assert!(
        running_text.contains("invocation: inv-compact-running slot=slot-1 status=running"),
        "compact running output:\n{running_text}"
    );
    assert!(running_text.contains(
        "inner progress (Dagu helper liveness): steps=3 not_started=1 running=1 reaped=1"
    ));

    std::thread::sleep(Duration::from_millis(20));
    let running_again = run_show_compact(&database, "run-compact-running", dagu_path.clone());
    assert!(running_again.status.success());
    assert_eq!(
        running.stdout, running_again.stdout,
        "compact output must not change with observation time for unchanged running state"
    );
    assert!(load_status(&database, "run-compact-running").is_none());

    let running_progress = run_progress(&database, "run-compact-running", dagu_path.clone());
    assert!(running_progress.status.success());
    let running_payload: Value =
        serde_json::from_slice(&running_progress.stdout).expect("running progress JSON");
    assert_eq!(running_payload["status"], "completed");
    assert_eq!(
        running_payload["result"]["invocation_id"],
        "inv-compact-running"
    );
    assert_eq!(
        running_payload["result"]["graph"]["steps"][0]["state"],
        "running"
    );
    assert_eq!(
        running_payload["result"]["graph"]["steps"][1]["state"],
        "not_started"
    );
    assert_eq!(
        running_payload["result"]["graph"]["steps"][2]["state"],
        "reaped"
    );
    waiter.kill().expect("stop running waiter");
    waiter.wait().expect("reap running waiter");

    let completed_capture = root.path().join("completed-capture");
    fs::create_dir_all(&completed_capture).expect("completed capture");
    let completed_home = write_graph_fixture(&completed_capture);
    fs::write(
        completed_home
            .join("data")
            .join("dag-run")
            .join("live")
            .join("status.jsonl"),
        r#"{"nodes":[{"status":4,"step":{"name":"w0"},"startedAt":"2024-01-01T00:00:00Z"},{"status":4,"step":{"name":"w1"},"startedAt":"2024-01-01T00:00:00Z"},{"status":4,"step":{"name":"join"},"startedAt":"2024-01-01T00:00:01Z"}]}
"#,
    )
    .expect("completed status.jsonl");
    seed_run_at(
        &database,
        "run-compact-completed",
        "inv-compact-completed",
        completed_capture.to_str().expect("utf8 completed capture"),
        now_millis() - 10_000,
    );
    complete_invocation(
        &database,
        "run-compact-completed",
        "inv-compact-completed",
        now_millis(),
    );
    let completed = run_show_compact(&database, "run-compact-completed", dagu_path.clone());
    assert!(
        completed.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&completed.stderr),
        String::from_utf8_lossy(&completed.stdout)
    );
    let completed_text = String::from_utf8_lossy(&completed.stdout);
    assert!(completed_text
        .contains("invocation: inv-compact-completed slot=slot-1 status=succeeded exit_code=0"));
    assert!(completed_text.contains(
        "inner progress (Dagu helper liveness): steps=3 not_started=0 running=0 reaped=3"
    ));
    assert_eq!(
        load_status(&database, "run-compact-completed"),
        Some(WaiterWrittenStatus::Succeeded)
    );
    let completed_progress = run_progress(&database, "run-compact-completed", dagu_path.clone());
    assert!(completed_progress.status.success());
    let completed_payload: Value =
        serde_json::from_slice(&completed_progress.stdout).expect("completed progress JSON");
    assert_eq!(completed_payload["status"], "completed");
    assert_eq!(
        completed_payload["result"]["graph"]["steps"][0]["state"],
        "reaped"
    );
    assert_eq!(
        completed_payload["result"]["graph"]["steps"][1]["state"],
        "reaped"
    );
    assert_eq!(
        completed_payload["result"]["graph"]["steps"][2]["state"],
        "reaped"
    );

    let detailed = run_show_json(&database, "run-compact-completed", dagu_path);
    assert!(
        detailed.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&detailed.stderr),
        String::from_utf8_lossy(&detailed.stdout)
    );
    let detailed_payload: Value = serde_json::from_slice(&detailed.stdout).expect("show JSON");
    assert_eq!(detailed_payload["operation"], "show");
    assert_eq!(detailed_payload["status"], "completed");
    assert_eq!(
        detailed_payload["result"]["run_id"],
        "run-compact-completed"
    );
    assert!(detailed_payload["result"].get("compact").is_none());

    let unavailable_capture = root.path().join("unavailable-capture");
    fs::create_dir_all(&unavailable_capture).expect("unavailable capture");
    write_graph_fixture(&unavailable_capture);
    seed_run_at(
        &database,
        "run-compact-unavailable",
        "inv-compact-unavailable",
        unavailable_capture
            .to_str()
            .expect("utf8 unavailable capture"),
        now_millis() - 10_000,
    );
    complete_invocation(
        &database,
        "run-compact-unavailable",
        "inv-compact-unavailable",
        now_millis(),
    );
    let unavailable = run_show_compact(&database, "run-compact-unavailable", path_without_dagu());
    assert!(
        unavailable.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&unavailable.stderr),
        String::from_utf8_lossy(&unavailable.stdout)
    );
    let unavailable_text = String::from_utf8_lossy(&unavailable.stdout);
    assert!(unavailable_text.contains("status=succeeded exit_code=0"));
    assert!(unavailable_text.contains("inner progress: unavailable [dagu-unavailable]"));
    assert!(load_status(&database, "run-compact-unavailable").is_some());
}

#[test]
fn public_compact_json_combination_is_a_clear_parse_error() {
    for selector in [
        "--json",
        "--machine-readable",
        "-j",
        "--format=json",
        "--output=json",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
            .args([selector, "show", "--compact", "run-1"])
            .bounded_output("loop-engine incompatible compact JSON")
            .expect("run incompatible compact JSON");
        assert_eq!(output.status.code(), Some(2), "selector {selector}");
        let payload: Value = serde_json::from_slice(&output.stdout).expect("JSON invocation error");
        assert_eq!(payload["status"], "invalid-invocation");
        assert!(payload["message"].as_str().unwrap().contains("human-only"));
        assert!(payload["message"].as_str().unwrap().contains("--json"));
    }
}

#[test]
fn cli_json_without_locator_omits_graph_and_does_not_write_status() {
    let root = tempdir().expect("tempdir");
    let database = root.path().join("loop.db");
    let capture = root.path().join("capture");
    fs::create_dir_all(&capture).expect("capture");
    seed_run(
        &database,
        "run-progress",
        "inv-progress",
        capture.to_str().expect("utf8 capture"),
    );

    let output = run_progress(&database, "run-progress", env::var_os("PATH").unwrap());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_eq!(payload["operation"], "invocation-progress");
    assert_eq!(payload["status"], "completed");
    let result = payload["result"].as_object().expect("result");
    assert_eq!(result["run_id"], "run-progress");
    assert_eq!(result["invocation_id"], "inv-progress");
    assert_eq!(result["slot_id"], "slot-1");
    assert_eq!(
        result["capture_dir"],
        capture.to_string_lossy().into_owned()
    );
    assert!(!result.contains_key("graph"));
    assert_eq!(result["traces"], json!([]));
    assert!(!result.contains_key("overlay"));
    assert!(!result.contains_key("inner_workers"));
    assert!(load_status(&database, "run-progress").is_none());
}

#[test]
fn cli_json_with_locator_maps_jsonl_liveness_not_dagu_stdout() {
    let root = tempdir().expect("tempdir");
    let database = root.path().join("loop.db");
    let capture = root.path().join("capture");
    fs::create_dir_all(&capture).expect("capture");
    write_graph_fixture(&capture);
    seed_run(
        &database,
        "run-graph",
        "inv-graph",
        capture.to_str().expect("utf8 capture"),
    );
    let stub_dir = root.path().join("bin");
    fs::create_dir_all(&stub_dir).expect("stub dir");
    write_status_ok_stub(&stub_dir);

    let output = run_progress(&database, "run-graph", prepend_path(&stub_dir));
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_eq!(payload["operation"], "invocation-progress");
    assert_eq!(payload["status"], "completed");
    let result = &payload["result"];
    assert_eq!(result["invocation_id"], "inv-graph");
    assert_eq!(
        result["capture_dir"],
        capture.to_string_lossy().into_owned()
    );
    let graph = result["graph"].as_object().expect("graph present");
    assert_eq!(graph["locator"]["dag_name"], "fanout-progress");
    assert_eq!(graph["locator"]["run_name"], "fanout-progress");
    assert!(graph["locator"]["dagu_home"].as_str().is_some());
    assert_eq!(graph["locator"].as_object().expect("locator").len(), 3);
    let steps = graph["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["name"], "w0");
    assert_eq!(steps[0]["state"], "running");
    assert_eq!(steps[1]["name"], "w1");
    assert_eq!(steps[1]["state"], "not_started");
    assert_eq!(steps[2]["name"], "join");
    assert_eq!(steps[2]["state"], "reaped");
    assert!(!result.as_object().expect("result").contains_key("overlay"));
    assert!(!result
        .as_object()
        .expect("result")
        .contains_key("inner_workers"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("human dagu status must not become the snapshot"),
        "dagu human stdout leaked into snapshot: {stdout}"
    );
    assert!(load_status(&database, "run-graph").is_none());
}

#[test]
fn missing_dagu_with_locator_fails_this_command_only() {
    let root = tempdir().expect("tempdir");
    let database = root.path().join("loop.db");
    let capture = root.path().join("capture");
    fs::create_dir_all(&capture).expect("capture");
    write_graph_fixture(&capture);
    seed_run(
        &database,
        "run-missing-dagu",
        "inv-1",
        capture.to_str().expect("utf8 capture"),
    );

    let output = run_progress(&database, "run-missing-dagu", path_without_dagu());
    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "dagu-unavailable");
    assert!(load_status(&database, "run-missing-dagu").is_none());
}
