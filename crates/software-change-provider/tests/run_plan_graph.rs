use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-run-plan-graph-{label}-{}-{suffix}",
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

fn task_worker(receipt: &Path, extra: &[&str]) -> String {
    let mut args = vec![
        dummy_script().to_string_lossy().into_owned(),
        "--receipt-dir".to_owned(),
        receipt.to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().map(|token| (*token).to_owned()));
    serde_json::to_string(&json!({ "command": "python3", "args": args })).expect("worker JSON")
}

fn invoke_protocol(stdin: &[u8]) -> Output {
    let mut child = Command::new(bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("software-change binary should spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write protocol stdin");
    child
        .wait_with_output()
        .expect("protocol process should exit")
}

fn invoke_graph(worker: &str, packet: &Value, current_dir: Option<&Path>) -> Output {
    let mut command = Command::new(bin());
    command
        .args(["run-plan-graph", "--task-worker", worker])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().expect("run-plan-graph should spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(packet).expect("packet JSON"))
        .expect("write invoke packet");
    child
        .wait_with_output()
        .expect("run-plan-graph process should exit")
}

fn packet(run_id: &str, slot_id: &str, artifact_root: &str, body: &str) -> Value {
    json!({
        "run_id": run_id,
        "slot_id": slot_id,
        "artifact_root": artifact_root,
        "instruction_body": body,
        "capture_dir": capture_dir_for_root(Path::new(artifact_root)).to_string_lossy(),
    })
}

fn capture_dir_for_root(artifact_root: &Path) -> PathBuf {
    artifact_root
        .parent()
        .map(|parent| parent.join("captures").join("inv-1"))
        .unwrap_or_else(|| PathBuf::from("captures").join("inv-1"))
}

fn read_capture_summary(artifact_root: &Path) -> Value {
    let path = capture_dir_for_root(artifact_root).join("summary.json");
    serde_json::from_slice(
        &fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("summary json {}: {error}", path.display()))
}

fn write_plan(artifact_root: &Path, plan: &Value) {
    fs::write(
        artifact_root.join("plan.json"),
        serde_json::to_vec_pretty(plan).expect("plan JSON"),
    )
    .expect("write plan.json");
}

fn read_trimmed(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .trim()
        .to_owned()
}

fn read_f64(path: &Path) -> f64 {
    read_trimmed(path)
        .parse()
        .unwrap_or_else(|error| panic!("parse {} as f64: {error}", path.display()))
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn fixture(label: &str) -> (TestDir, PathBuf, PathBuf) {
    let dir = TestDir::new(label);
    let artifact_root = dir.path().join("artifacts");
    let receipt_dir = dir.path().join("receipts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::create_dir_all(&receipt_dir).expect("receipt dir");
    (dir, artifact_root, receipt_dir)
}

#[test]
fn independent_tasks_overlap_under_concurrency_cap() {
    let (_dir, artifact_root, receipt_dir) = fixture("overlap");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}],
            "dependency_graph": []
        }),
    );
    let spawn_marker = artifact_root.join("spawned");
    let worker = task_worker(
        &receipt_dir,
        &[
            "--sleep",
            "0.4",
            "--write-report",
            "--wait-peers",
            "2",
            "--spawn-marker",
            spawn_marker.to_str().expect("utf-8 marker"),
        ],
    );
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(read_trimmed(&receipt_dir.join("a.overlap")), "1");
    assert_eq!(read_trimmed(&receipt_dir.join("b.overlap")), "1");
    let start_a = read_f64(&receipt_dir.join("a.start"));
    let end_a = read_f64(&receipt_dir.join("a.end"));
    let start_b = read_f64(&receipt_dir.join("b.start"));
    let end_b = read_f64(&receipt_dir.join("b.end"));
    assert!(
        start_a < end_b && start_b < end_a,
        "independent tasks should overlap: a=[{start_a}, {end_a}] b=[{start_b}, {end_b}]"
    );
    let capture_root = capture_dir_for_root(&artifact_root);
    assert!(capture_root.join("a").join("stdout").is_file());
    assert!(capture_root.join("b").join("stdout").is_file());
    assert!(!artifact_root.join("run-plan-graph").exists());
    let captured = read_capture_summary(&artifact_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0]["command"], "python3");
    assert_eq!(workers[1]["command"], "python3");
    assert_eq!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
}

#[test]
fn dependent_task_waits_until_predecessor_succeeds() {
    let (_dir, artifact_root, receipt_dir) = fixture("dependent");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}],
            "dependency_graph": [{"from": "a", "to": "b"}]
        }),
    );
    let worker = task_worker(&receipt_dir, &["--sleep", "0.25", "--write-report"]);
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let end_a = read_f64(&receipt_dir.join("a.end"));
    let start_b = read_f64(&receipt_dir.join("b.start"));
    assert!(
        start_b >= end_a,
        "b should wait for a: end_a={end_a} start_b={start_b}"
    );
}

#[test]
fn unknown_edge_and_cycle_exit_nonzero_before_spawn() {
    let (_dir, artifact_root, receipt_dir) = fixture("unknown-edge");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}],
            "dependency_graph": [{"from": "a", "to": "missing"}]
        }),
    );
    let spawn_marker = artifact_root.join("spawned");
    let worker = task_worker(
        &receipt_dir,
        &[
            "--spawn-marker",
            spawn_marker.to_str().expect("utf-8 marker"),
            "--write-report",
        ],
    );
    let unknown = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(unknown.status.code(), Some(0));
    assert!(!spawn_marker.exists(), "unknown edge must not spawn");
    assert!(!artifact_root.join("run-plan-graph").exists());
    assert!(receipt_dir.read_dir().expect("receipts").next().is_none());

    let (_cycle_dir, cycle_root, cycle_receipts) = fixture("cycle");
    write_plan(
        &cycle_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}],
            "dependency_graph": [
                {"from": "a", "to": "b"},
                {"from": "b", "to": "a"}
            ]
        }),
    );
    let cycle_marker = cycle_root.join("spawned");
    let cycle_worker = task_worker(
        &cycle_receipts,
        &[
            "--spawn-marker",
            cycle_marker.to_str().expect("utf-8 marker"),
            "--write-report",
        ],
    );
    let cycle = invoke_graph(
        &cycle_worker,
        &packet(
            "run-1",
            "implement",
            &cycle_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(cycle.status.code(), Some(0));
    assert!(!cycle_marker.exists(), "cycle must not spawn");
    assert!(!cycle_root.join("run-plan-graph").exists());
    assert!(cycle_receipts
        .read_dir()
        .expect("receipts")
        .next()
        .is_none());
}

#[test]
fn task_worker_dummy_records_locked_stdin_layout() {
    let (dir, artifact_root, receipt_dir) = fixture("stdin-layout");
    let task = json!({
        "id": "task-a",
        "objective": "Do A",
        "dependencies": [],
        "source_of_truth": ["design.json"],
        "deliverables": ["a.rs"],
        "out_of_scope": [],
        "validation": ["cargo test"],
        "handoff": "A is done"
    });
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [task.clone()],
            "dependency_graph": []
        }),
    );

    let relative_root = "artifacts";
    let worker = task_worker(&receipt_dir, &["--write-report"]);
    let output = invoke_graph(
        &worker,
        &packet("run-1", "implement", relative_root, "Implement the plan."),
        Some(dir.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let recorded = fs::read_to_string(receipt_dir.join("task-a.stdin")).expect("recorded stdin");
    let captured = fs::read_to_string(
        capture_dir_for_root(&artifact_root)
            .join("task-a")
            .join("stdout"),
    )
    .expect("captured stdout");
    assert_eq!(recorded, captured);
    assert!(!artifact_root.join("run-plan-graph").exists());

    let abs_root = artifact_root
        .canonicalize()
        .unwrap_or(artifact_root.clone());
    let prefix = format!(
        "run_id: run-1\nslot_id: implement\nartifact_root: {}\n\n## instruction_body\nImplement the plan.\n\n## task\n",
        abs_root.display()
    );
    let prefix_display = format!(
        "run_id: run-1\nslot_id: implement\nartifact_root: {}\n\n## instruction_body\nImplement the plan.\n\n## task\n",
        artifact_root.display()
    );
    let used_prefix = if recorded.starts_with(&prefix) {
        prefix
    } else if recorded.starts_with(&prefix_display) {
        prefix_display
    } else {
        panic!(
            "stdin layout prefix mismatch.\nrecorded:\n{recorded}\nexpected one of:\n{prefix}\n{prefix_display}"
        );
    };
    let task_raw = recorded
        .strip_prefix(&used_prefix)
        .expect("task section")
        .trim();
    let parsed: Value = serde_json::from_str(task_raw).expect("task JSON");
    assert_eq!(parsed, task);

    let artifact_line = recorded
        .lines()
        .find(|line| line.starts_with("artifact_root: "))
        .expect("artifact_root line");
    let recorded_root = PathBuf::from(artifact_line.trim_start_matches("artifact_root: "));
    assert!(
        recorded_root.is_absolute(),
        "artifact_root must be absolute, got {}",
        recorded_root.display()
    );
}

#[test]
fn failing_sibling_is_reaped_before_runner_exits() {
    let (_dir, artifact_root, receipt_dir) = fixture("reap");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "fail"}, {"id": "slow"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(
        &receipt_dir,
        &["--sleep", "0.5", "--fail-task", "fail", "--wait-peers", "2"],
    );
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(
        receipt_dir.join("slow.end").exists(),
        "slow sibling must finish before runner exits; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pid: u32 = read_trimmed(&receipt_dir.join("slow.pid"))
        .parse()
        .expect("slow pid");
    assert!(
        !pid_alive(pid),
        "slow sibling pid {pid} must be reaped before run-plan-graph exits"
    );
    let captured = read_capture_summary(&artifact_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_ne!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
}

#[test]
fn missing_implementation_report_after_successes_exits_nonzero() {
    let (_dir, artifact_root, receipt_dir) = fixture("missing-report");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &[]);
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(
        receipt_dir.join("a.end").exists(),
        "task should have succeeded"
    );
    assert!(!artifact_root.join("implementation-report.json").is_file());
    let captured = read_capture_summary(&artifact_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["exit_code"], 0);
    assert!(capture_dir_for_root(&artifact_root)
        .join("a")
        .join("stdout")
        .is_file());
}

#[test]
fn describe_and_evaluate_stdin_protocol_does_not_reach_the_executor() {
    let describe = invoke_protocol(br#"{"operation":"describe"}"#);
    assert_eq!(describe.status.code(), Some(0));
    assert!(!describe.stdout.is_empty());
    let describe_err = String::from_utf8_lossy(&describe.stderr);
    assert!(
        !describe_err.contains("plan.json") && !describe_err.contains("invoke packet"),
        "describe stderr: {describe_err}"
    );

    let evaluate = invoke_protocol(br#"{"operation":"evaluate"}"#);
    assert_ne!(evaluate.status.code(), Some(0));
    let evaluate_err = String::from_utf8_lossy(&evaluate.stderr);
    assert!(
        !evaluate_err.contains("plan.json") && !evaluate_err.contains("invoke packet"),
        "evaluate must stay on the protocol path, stderr: {evaluate_err}"
    );
}

#[test]
fn leftover_args_after_run_plan_graph_flags_are_errors() {
    let output = Command::new(bin())
        .args(["run-plan-graph", "leftover"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run leftover args");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("leftover"));
}

#[test]
fn missing_plan_json_exits_nonzero_without_spawn() {
    let (_dir, artifact_root, receipt_dir) = fixture("missing-plan");
    let spawn_marker = artifact_root.join("spawned");
    let worker = task_worker(
        &receipt_dir,
        &[
            "--spawn-marker",
            spawn_marker.to_str().expect("utf-8 marker"),
        ],
    );
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(!spawn_marker.exists());
    assert!(receipt_dir.read_dir().expect("receipts").next().is_none());
}

#[test]
fn path_escaping_task_id_exits_nonzero_without_writing_outside_artifact_root() {
    let (dir, artifact_root, receipt_dir) = fixture("escape-id");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "../../escaped"}],
            "dependency_graph": []
        }),
    );
    let spawn_marker = artifact_root.join("spawned");
    let worker = task_worker(
        &receipt_dir,
        &[
            "--spawn-marker",
            spawn_marker.to_str().expect("utf-8 marker"),
            "--write-report",
        ],
    );
    let output = invoke_graph(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let escaped = dir.path().join("escaped");
    assert!(
        !escaped.exists() && !escaped.join("stdout").exists(),
        "task id ../../escaped must not create {}",
        escaped.display()
    );
    assert!(!spawn_marker.exists());
    assert!(!artifact_root.join("run-plan-graph").exists());
    assert!(receipt_dir.read_dir().expect("receipts").next().is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("path-safe") || stderr.contains("escapes"),
        "{stderr}"
    );
}
