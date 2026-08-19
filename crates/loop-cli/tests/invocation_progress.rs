use loop_core::{
    CreateRunRequest, CreateWorkSlotInvocationRequest, Lifecycle, Persistence, ProviderAssociation,
    State, Timestamp, Transition, WaiterWrittenStatus, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
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
        .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
            run_id,
            invocation_id,
            "slot-1",
            WorkSlotBinding::new("echo", vec!["ok".to_owned()]),
            "digest",
            "subject",
            1,
            Timestamp::from_unix_millis(1_000),
            60_000,
            capture_dir,
        ))
        .expect("create invocation");
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
        .output()
        .expect("run invocation-progress")
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
