use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use software_change_provider::{names_for_capture_root, write_locator, DaguLocator};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-dagu-resolver-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_software-change"))
}

fn dummy_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/run_plan_graph_dummy.py")
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

fn write_version_stub(directory: &Path, version_line: &str) -> PathBuf {
    let path = directory.join("dagu");
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' '{version_line}'\n"),
    )
    .expect("write dagu stub");
    let mut permissions = fs::metadata(&path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod dagu stub");
    path
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fixture(label: &str) -> (TestDir, PathBuf, PathBuf, PathBuf) {
    let dir = TestDir::new(label);
    let artifact_root = dir.path().join("artifacts");
    let capture_dir = dir.path().join("captures").join("inv-1");
    let receipt_dir = dir.path().join("receipts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::create_dir_all(&capture_dir).expect("capture dir");
    fs::create_dir_all(&receipt_dir).expect("receipt dir");
    fs::write(
        artifact_root.join("plan.json"),
        serde_json::to_vec(&json!({
            "tasks": [{"id": "task-a"}],
            "dependency_graph": []
        }))
        .expect("plan JSON"),
    )
    .expect("write plan.json");
    (dir, artifact_root, capture_dir, receipt_dir)
}

fn task_worker(receipt: &Path) -> String {
    serde_json::to_string(&json!({
        "command": "python3",
        "args": [
            dummy_script().to_string_lossy(),
            "--receipt-dir",
            receipt.to_string_lossy(),
            "--write-report"
        ]
    }))
    .expect("worker JSON")
}

fn invoke_graph(worker: &str, packet: &Value, path: OsString) -> Output {
    let mut child = Command::new(bin())
        .args(["run-plan-graph", "--task-worker", worker])
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run-plan-graph should spawn");
    let write_result = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(packet).expect("packet JSON"));
    let output = child
        .wait_with_output()
        .expect("run-plan-graph process should exit");
    if let Err(error) = write_result {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write failure: {error}"
        );
    }
    output
}

fn packet(artifact_root: &Path, capture_dir: &Path) -> Value {
    json!({
        "run_id": "run-1",
        "slot_id": "implement",
        "artifact_root": artifact_root.to_string_lossy(),
        "instruction_body": "Do the work",
        "capture_dir": capture_dir.to_string_lossy(),
    })
}

fn task_stdout(capture_dir: &Path) -> PathBuf {
    capture_dir.join("task-a").join("stdout")
}

#[test]
fn stub_reporting_2_14_0_resolves() {
    let (_dir, artifact_root, capture_dir, receipt_dir) = fixture("ok");
    let stub_dir = TestDir::new("ok-stub");
    let stub = stub_dir.path().join("dagu");
    fs::write(
        &stub,
        "#!/bin/sh\ncase \"$1\" in\n  version|--version) printf '2.14.0\\n'; exit 0 ;;\n  *) printf 'stub-not-real-dagu\\n' >&2; exit 42 ;;\nesac\n",
    )
    .expect("write version-ok stub");
    let mut permissions = fs::metadata(&stub).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub, permissions).expect("chmod dagu stub");
    let worker = task_worker(&receipt_dir);
    let output = invoke_graph(
        &worker,
        &packet(&artifact_root, &capture_dir),
        prepend_path(stub_dir.path()),
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "a version-ok stub is not a real dagu start; stdout={}",
        stdout_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(
        !stderr.contains("reports version 2.14.0"),
        "2.14.0 stub must pass the version gate, stderr={stderr}"
    );
    assert!(
        stderr.contains("stub-not-real-dagu") || stderr.contains("dagu validate"),
        "version-ok stub should fail at validate/start, stderr={stderr}"
    );
    assert!(
        !task_stdout(&capture_dir).exists(),
        "version-ok stub must not spawn a worker"
    );
    assert!(stub.exists());
}

#[test]
fn stub_reporting_2_13_0_is_rejected_with_required_version() {
    let (_dir, artifact_root, capture_dir, receipt_dir) = fixture("old");
    let stub_dir = TestDir::new("old-stub");
    let stub = write_version_stub(stub_dir.path(), "2.13.0");
    let worker = task_worker(&receipt_dir);
    let output = invoke_graph(
        &worker,
        &packet(&artifact_root, &capture_dir),
        prepend_path(stub_dir.path()),
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout_text(&output));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(stderr.contains("2.13.0"), "{stderr}");
    assert!(
        stderr.contains(stub.to_str().expect("utf-8 stub")),
        "{stderr}"
    );
    assert!(
        !task_stdout(&capture_dir).exists(),
        "too-old dagu must not write capture_dir/<task_id>/stdout"
    );
}

#[test]
fn missing_path_dagu_fails_before_task_stdout() {
    let (_dir, artifact_root, capture_dir, receipt_dir) = fixture("missing");
    let worker = task_worker(&receipt_dir);
    let output = invoke_graph(
        &worker,
        &packet(&artifact_root, &capture_dir),
        path_without_dagu(),
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout_text(&output));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(stderr.contains("PATH lookup found nothing"), "{stderr}");
    assert!(
        !task_stdout(&capture_dir).exists(),
        "missing dagu must not write capture_dir/<task_id>/stdout"
    );
}

#[test]
fn write_locator_uses_isolated_home_and_unique_plan_graph_names() {
    let first_dir = TestDir::new("capture-one");
    let second_dir = TestDir::new("capture-two");
    let first = first_dir.path().join("capture-one");
    let second = second_dir.path().join("capture-two");
    fs::create_dir_all(&first).expect("mkdir first");
    fs::create_dir_all(&second).expect("mkdir second");

    let (first_dag, first_run) = names_for_capture_root(&first).expect("first names");
    let (second_dag, second_run) = names_for_capture_root(&second).expect("second names");
    let first_locator = write_locator(&first, &first_dag, &first_run).expect("write first locator");
    let second_locator =
        write_locator(&second, &second_dag, &second_run).expect("write second locator");

    let first_path = first.join("dagu-locator.json");
    let parsed: DaguLocator =
        serde_json::from_slice(&fs::read(&first_path).expect("read locator")).expect("json");
    let keys: serde_json::Map<String, Value> =
        serde_json::from_slice(&fs::read(&first_path).expect("read locator")).expect("object");
    assert_eq!(keys.len(), 3);
    assert!(keys.contains_key("dagu_home"));
    assert!(keys.contains_key("dag_name"));
    assert!(keys.contains_key("run_name"));
    assert_eq!(parsed, first_locator);
    assert!(Path::new(&parsed.dagu_home).is_absolute());
    assert_eq!(
        Path::new(&parsed.dagu_home),
        fs::canonicalize(first.join("dagu-home")).expect("canonicalize home")
    );
    assert_eq!(parsed.dag_name, "plan-graph-capture-one");
    assert_eq!(parsed.run_name, "plan-graph-capture-one");
    assert!(!parsed.dag_name.is_empty());
    assert!(!parsed.run_name.is_empty());
    assert!(!second_locator.dag_name.is_empty());
    assert!(!second_locator.run_name.is_empty());
    assert_eq!(second_locator.dag_name, "plan-graph-capture-two");
    assert_eq!(second_locator.run_name, "plan-graph-capture-two");
    assert_ne!(first_locator.dag_name, second_locator.dag_name);
    assert_ne!(first_locator.run_name, second_locator.run_name);
    assert_ne!(first_locator.dagu_home, second_locator.dagu_home);
    assert_eq!(
        Path::new(&second_locator.dagu_home),
        fs::canonicalize(second.join("dagu-home")).expect("canonicalize second home")
    );
    assert!(first.join("dagu-home").is_dir());
    assert!(second.join("dagu-locator.json").is_file());
}

#[test]
fn names_for_long_invocation_dir_stay_under_dagu_limit() {
    let first_dir = TestDir::new("long-one");
    let second_dir = TestDir::new("long-two");
    let first = first_dir
        .path()
        .join("invocation-1787044324400584000-1-89864");
    let second = second_dir
        .path()
        .join("invocation-1787044324400584000-1-89865");
    fs::create_dir_all(&first).expect("mkdir first");
    fs::create_dir_all(&second).expect("mkdir second");
    let (first_dag, first_run) = names_for_capture_root(&first).expect("first names");
    let (second_dag, _) = names_for_capture_root(&second).expect("second names");
    assert_eq!(first_dag, first_run);
    assert!(first_dag.starts_with("plan-graph-"));
    assert!(first_dag.len() < 40, "{first_dag}");
    assert!(second_dag.len() < 40, "{second_dag}");
    assert_ne!(first_dag, second_dag);
}

#[test]
fn main_rs_declares_mod_dagu_and_keeps_stdin_exec_dispatch() {
    let source = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("read main.rs");
    assert!(
        source.contains("mod dagu;"),
        "main.rs must declare mod dagu"
    );
    assert!(
        source.contains("stdin-exec"),
        "main.rs must keep the hidden stdin-exec dispatch"
    );
}
