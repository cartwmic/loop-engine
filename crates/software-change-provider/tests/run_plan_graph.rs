use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
    invoke_graph_with(worker, packet, current_dir, &[])
}

fn invoke_graph_with(
    worker: &str,
    packet: &Value,
    current_dir: Option<&Path>,
    extra: &[&str],
) -> Output {
    let working_directory = current_dir
        .map(Path::to_path_buf)
        .or_else(|| {
            packet
                .get("artifact_root")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .expect("packet must identify an explicit working directory");
    assert!(working_directory.is_absolute());
    assert!(working_directory.is_dir());
    let mut command = Command::new(bin());
    command
        .args([
            "run-plan-graph",
            "--working-directory",
            working_directory.to_str().expect("working directory UTF-8"),
            "--task-worker",
            worker,
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().expect("run-plan-graph should spawn");
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

fn packet(run_id: &str, slot_id: &str, artifact_root: &str, body: &str) -> Value {
    json!({
        "run_id": run_id,
        "slot_id": slot_id,
        "artifact_root": artifact_root,
        "instruction_body": body,
        "capture_dir": capture_dir_for_root(Path::new(artifact_root)).to_string_lossy(),
    })
}

fn packet_with_capture(
    run_id: &str,
    slot_id: &str,
    artifact_root: &str,
    body: &str,
    capture_dir: &Path,
) -> Value {
    json!({
        "run_id": run_id,
        "slot_id": slot_id,
        "artifact_root": artifact_root,
        "instruction_body": body,
        "capture_dir": capture_dir.to_string_lossy(),
    })
}

fn capture_dir_for_root(artifact_root: &Path) -> PathBuf {
    artifact_root
        .parent()
        .map(|parent| parent.join("captures").join("inv-1"))
        .unwrap_or_else(|| PathBuf::from("captures").join("inv-1"))
}

fn read_capture_summary(artifact_root: &Path) -> Value {
    read_summary_at(&capture_dir_for_root(artifact_root))
}

fn read_summary_at(capture_root: &Path) -> Value {
    let path = capture_root.join("summary.json");
    serde_json::from_slice(
        &fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("summary json {}: {error}", path.display()))
}

fn write_plan(artifact_root: &Path, plan: &Value) {
    let mut plan = plan.clone();
    if plan.get("revision").is_none() {
        plan["revision"] = json!("1");
    }
    fs::write(
        artifact_root.join("plan.json"),
        serde_json::to_vec_pretty(&plan).expect("plan JSON"),
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

fn emitted_yaml(capture_root: &Path) -> String {
    let dags = capture_root.join("dagu-home").join("dags");
    let entry = fs::read_dir(&dags)
        .unwrap_or_else(|error| panic!("read {}: {error}", dags.display()))
        .flatten()
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .unwrap_or_else(|| panic!("missing DAG yaml in {}", dags.display()));
    fs::read_to_string(entry.path()).expect("read yaml")
}

fn valid_leftover_report(plan_revision: &str) -> Value {
    json!({
        "revision": "9",
        "author": {"name": "leftover", "kind": "agent"},
        "plan_revision": plan_revision,
        "coverage": {
            "commit": "leftover",
            "documents": [{"path": "plan.json", "revision": plan_revision}]
        },
        "summary": "planted leftover must not satisfy",
        "changed_surface": ["leftover"],
        "validation": ["leftover"]
    })
}

fn max_overlap(intervals: &[(f64, f64)]) -> usize {
    let mut events: Vec<(f64, i32)> = Vec::with_capacity(intervals.len() * 2);
    for (start, end) in intervals {
        events.push((*start, 1));
        events.push((*end, -1));
    }
    events.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut current = 0i32;
    let mut max = 0usize;
    for (_, delta) in events {
        current += delta;
        max = max.max(current as usize);
    }
    max
}

#[test]
fn invalid_working_directory_fails_before_dagu_or_worker_launch() {
    let (dir, _artifact_root, receipt_dir) = fixture("invalid-working-directory");
    let marker = dir.path().join("worker-started");
    let worker = task_worker(
        &receipt_dir,
        &["--spawn-marker", marker.to_str().expect("marker UTF-8")],
    );
    let file = dir.path().join("not-a-directory");
    fs::write(&file, b"file").expect("write non-directory path");
    let missing = dir.path().join("missing");
    let cases = vec![
        (Vec::<String>::new(), "omitted", "".to_owned()),
        (
            vec![
                "--working-directory".to_owned(),
                "relative/checkout".to_owned(),
            ],
            "relative",
            "relative/checkout".to_owned(),
        ),
        (
            vec![
                "--working-directory".to_owned(),
                missing.to_string_lossy().into_owned(),
            ],
            "nonexistent",
            missing.to_string_lossy().into_owned(),
        ),
        (
            vec![
                "--working-directory".to_owned(),
                file.to_string_lossy().into_owned(),
            ],
            "not a directory",
            file.to_string_lossy().into_owned(),
        ),
    ];
    for (extra, category, supplied) in cases {
        let output = Command::new(bin())
            .arg("run-plan-graph")
            .arg("--task-worker")
            .arg(&worker)
            .args(&extra)
            .output()
            .expect("invalid run-plan-graph should spawn");
        assert_eq!(output.status.code(), Some(2), "{category}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(category), "{category}: {stderr}");
        if !supplied.is_empty() {
            assert!(stderr.contains(&supplied), "{category}: {stderr}");
        }
        assert!(!marker.exists(), "{category} started the dummy worker");
    }
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
    let yaml = emitted_yaml(&capture_root);
    assert!(yaml.contains("max_active_steps: 4"), "{yaml}");
    assert!(!yaml.contains("continue_on"), "{yaml}");
    assert!(!yaml.contains("retry_policy"), "{yaml}");
}

#[test]
fn max_active_two_emits_that_cap_and_summarizer_depends_on_every_task() {
    let (_dir, artifact_root, receipt_dir) = fixture("max-active-two");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "task-a"}, {"id": "task-b"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--write-report"]);
    let output = invoke_graph_with(
        &worker,
        &packet(
            "run-1",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
        &["--max-active", "2"],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let yaml = emitted_yaml(&capture_dir_for_root(&artifact_root));
    assert!(yaml.contains("max_active_steps: 2"), "{yaml}");
    assert!(!yaml.contains("max_active_steps: 4"), "{yaml}");
    let summarizer = yaml
        .split("name: \"summarizer\"")
        .nth(1)
        .expect("summarizer step");
    assert!(summarizer.contains("      - \"task-a\""), "{yaml}");
    assert!(summarizer.contains("      - \"task-b\""), "{yaml}");
    assert!(
        summarizer.contains("    depends:\n      - \"task-a\"\n      - \"task-b\"\n"),
        "{yaml}"
    );
}

#[test]
fn five_independent_tasks_never_exceed_cap_four() {
    let (_dir, artifact_root, receipt_dir) = fixture("cap-four");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [
                {"id": "t1"},
                {"id": "t2"},
                {"id": "t3"},
                {"id": "t4"},
                {"id": "t5"}
            ],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--sleep", "0.45", "--write-report"]);
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
    let intervals: Vec<(f64, f64)> = (1..=5)
        .map(|index| {
            let id = format!("t{index}");
            (
                read_f64(&receipt_dir.join(format!("{id}.start"))),
                read_f64(&receipt_dir.join(format!("{id}.end"))),
            )
        })
        .collect();
    let overlap = max_overlap(&intervals);
    assert!(
        overlap <= 4,
        "5 independent dummy sleeps must not exceed 4 concurrent, got {overlap}: {intervals:?}"
    );
    assert!(
        overlap >= 2,
        "independent tasks should overlap: {intervals:?}"
    );
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
    assert!(!capture_dir_for_root(&artifact_root)
        .join("a")
        .join("stdout")
        .exists());
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
    assert!(!capture_dir_for_root(&cycle_root)
        .join("a")
        .join("stdout")
        .exists());
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
    assert!(
        !recorded.contains("instruction_body"),
        "location JSON must not include instruction_body:\n{recorded}"
    );
    assert!(
        !recorded.contains("Implement the plan."),
        "duty must be the task object only:\n{recorded}"
    );

    let (location_raw, rest) = recorded
        .split_once("\n---\n\n")
        .expect("compact location JSON plus duty separator");
    let location: Value = serde_json::from_str(location_raw).expect("location JSON");
    let keys: Vec<&str> = location
        .as_object()
        .expect("location object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["artifact_root"]);
    let recorded_root = PathBuf::from(location["artifact_root"].as_str().expect("artifact_root"));
    assert!(
        recorded_root.is_absolute(),
        "artifact_root must be absolute, got {}",
        recorded_root.display()
    );
    let parsed: Value = serde_json::from_str(rest).expect("task JSON");
    assert_eq!(parsed, task);
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
    let capture_root = capture_dir_for_root(&artifact_root);
    assert!(!capture_root.join("summarizer").join("stdout").exists());
    let captured = read_capture_summary(&artifact_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_ne!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
    assert_eq!(workers[0]["command"], "python3");
    assert!(workers[0]["args"].is_array());
    assert!(Path::new(workers[0]["stdout_path"].as_str().expect("stdout")).is_file());
    assert!(Path::new(workers[0]["stderr_path"].as_str().expect("stderr")).is_file());
}

#[test]
fn failing_task_prevents_downstream_receipt() {
    let (_dir, artifact_root, receipt_dir) = fixture("downstream");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "fail"}, {"id": "down"}],
            "dependency_graph": [{"from": "fail", "to": "down"}]
        }),
    );
    let worker = task_worker(&receipt_dir, &["--fail-task", "fail"]);
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
    assert!(receipt_dir.join("fail.end").exists());
    assert!(
        !receipt_dir.join("down.start").exists(),
        "downstream task must not start after a failed predecessor"
    );
    assert!(!capture_dir_for_root(&artifact_root)
        .join("down")
        .join("stdout")
        .exists());
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
fn leftover_report_is_deleted_and_does_not_satisfy() {
    let (_dir, artifact_root, receipt_dir) = fixture("leftover");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}],
            "dependency_graph": []
        }),
    );
    let leftover = valid_leftover_report("1");
    fs::write(
        artifact_root.join("implementation-report.json"),
        serde_json::to_vec_pretty(&leftover).expect("leftover JSON"),
    )
    .expect("plant leftover report");
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
    assert_ne!(
        output.status.code(),
        Some(0),
        "planted leftover must not satisfy; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(receipt_dir.join("a.end").exists());
    if artifact_root.join("implementation-report.json").is_file() {
        let current: Value = serde_json::from_slice(
            &fs::read(artifact_root.join("implementation-report.json")).expect("read leftover"),
        )
        .expect("leftover json");
        assert_ne!(
            current, leftover,
            "start-of-run delete must drop the planted file"
        );
    }
}

#[test]
fn summarizer_dummy_writes_matching_report_and_exits_zero() {
    let (_dir, artifact_root, receipt_dir) = fixture("summarizer-ok");
    write_plan(
        &artifact_root,
        &json!({
            "revision": "7",
            "tasks": [{"id": "a"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--write-report"]);
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
    let report: Value = serde_json::from_slice(
        &fs::read(artifact_root.join("implementation-report.json")).expect("report"),
    )
    .expect("report json");
    assert_eq!(report["plan_revision"], "7");
    assert!(capture_dir_for_root(&artifact_root)
        .join("summarizer")
        .join("stdout")
        .is_file());
    let summarizer_stdin =
        fs::read_to_string(receipt_dir.join("summarizer.stdin")).expect("summarizer stdin");
    let (location_raw, assignment) = summarizer_stdin
        .split_once("\n---\n\n")
        .expect("summarizer compact location plus assignment");
    let location: Value = serde_json::from_str(location_raw).expect("summarizer location JSON");
    let keys: Vec<&str> = location
        .as_object()
        .expect("location object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["artifact_root", "capture_dir", "plan_path"]);
    assert!(Path::new(location["artifact_root"].as_str().expect("artifact_root")).is_absolute());
    assert!(Path::new(location["capture_dir"].as_str().expect("capture_dir")).is_absolute());
    assert!(location["plan_path"]
        .as_str()
        .expect("plan_path")
        .ends_with("plan.json"));
    assert_eq!(
        assignment,
        "Write artifact_root/implementation-report.json for this invocation only. You are the sole writer of that filename. plan_revision must equal the revision of the plan.json at plan_path. Do not concatenate worker stdout. Do not append review-evidence. Ordinary plan tasks must not write that filename."
    );
}

#[test]
fn failing_task_writes_summary_without_summarizer() {
    let (_dir, artifact_root, receipt_dir) = fixture("fail-summary");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "boom"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--fail-task", "boom"]);
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
    let capture_root = capture_dir_for_root(&artifact_root);
    assert!(
        !capture_root.join("summarizer").join("stdout").exists(),
        "summarizer must not start after a task failure"
    );
    let captured = read_summary_at(&capture_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["command"], "python3");
    assert!(workers[0]["args"].is_array());
    assert_ne!(workers[0]["exit_code"], 0);
    let stdout = PathBuf::from(workers[0]["stdout_path"].as_str().expect("stdout_path"));
    let stderr = PathBuf::from(workers[0]["stderr_path"].as_str().expect("stderr_path"));
    assert!(stdout.is_file(), "{}", stdout.display());
    assert!(stderr.is_file(), "{}", stderr.display());
}

#[test]
fn summarizer_killed_still_writes_summary_for_plan_tasks() {
    let (_dir, artifact_root, receipt_dir) = fixture("summarizer-kill");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--summarizer-kill"]);
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
        "killed summarizer must be nonzero; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(receipt_dir.join("a.end").exists());
    assert!(receipt_dir.join("b.end").exists());
    assert!(!artifact_root.join("implementation-report.json").is_file());
    let captured = read_capture_summary(&artifact_root);
    let workers = captured["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
    assert_eq!(workers[0]["command"], "python3");
    assert!(Path::new(workers[0]["stdout_path"].as_str().expect("stdout")).is_file());
    assert!(Path::new(workers[1]["stderr_path"].as_str().expect("stderr")).is_file());
}

#[test]
fn live_locator_exists_and_second_capture_dir_is_isolated() {
    let dir = TestDir::new("live-locator");
    let artifact_root = dir.path().join("artifacts");
    let receipt_dir = dir.path().join("receipts");
    let first_capture = dir.path().join("captures").join("inv-live-one");
    let second_capture = dir.path().join("captures").join("inv-live-two");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::create_dir_all(&receipt_dir).expect("receipt dir");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "long"}],
            "dependency_graph": []
        }),
    );
    let worker = task_worker(&receipt_dir, &["--sleep", "1.2", "--write-report"]);
    let packet = packet_with_capture(
        "run-1",
        "implement",
        &artifact_root.to_string_lossy(),
        "Do the work",
        &first_capture,
    );
    let mut child = Command::new(bin())
        .args([
            "run-plan-graph",
            "--working-directory",
            artifact_root.to_str().expect("artifact root UTF-8"),
            "--task-worker",
            &worker,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn live run-plan-graph");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(&packet).expect("packet"))
        .expect("write packet");

    let locator_path = first_capture.join("dagu-locator.json");
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if locator_path.is_file() && first_capture.join("long").join("stdout").is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "locator must be observed while the overlay-equivalent process is still running"
    );
    let locator: Value = serde_json::from_slice(&fs::read(&locator_path).expect("read locator"))
        .expect("locator json");
    let object = locator.as_object().expect("locator object");
    assert_eq!(object.len(), 3);
    assert!(object.contains_key("dagu_home"));
    assert!(object.contains_key("dag_name"));
    assert!(object.contains_key("run_name"));
    let dagu_home = locator["dagu_home"].as_str().expect("dagu_home");
    let dag_name = locator["dag_name"].as_str().expect("dag_name");
    let run_name = locator["run_name"].as_str().expect("run_name");
    assert!(!dagu_home.is_empty());
    assert!(!dag_name.is_empty());
    assert!(!run_name.is_empty());
    assert!(Path::new(dagu_home).is_absolute());
    assert_eq!(
        Path::new(dagu_home),
        fs::canonicalize(first_capture.join("dagu-home")).expect("canonicalize first home")
    );
    assert_eq!(dag_name, "plan-graph-inv-live-one");

    let output = child.wait_with_output().expect("wait live");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_locator_after = fs::read(&locator_path).expect("reread first locator");
    let first_long_stdout =
        fs::read(first_capture.join("long").join("stdout")).expect("first stdout");

    let second = invoke_graph(
        &worker,
        &packet_with_capture(
            "run-2",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
            &second_capture,
        ),
        None,
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_locator: Value = serde_json::from_slice(
        &fs::read(second_capture.join("dagu-locator.json")).expect("second locator"),
    )
    .expect("second locator json");
    assert_eq!(
        second_locator["dag_name"]
            .as_str()
            .expect("second dag_name"),
        "plan-graph-inv-live-two"
    );
    assert_ne!(
        second_locator["dag_name"], locator["dag_name"],
        "second capture_dir must not reuse dag_name"
    );
    assert_ne!(
        second_locator["dagu_home"], locator["dagu_home"],
        "second invocation must use a fresh isolated home"
    );
    assert_eq!(
        fs::read(&locator_path).expect("first locator remains"),
        first_locator_after
    );
    assert_eq!(
        fs::read(first_capture.join("long").join("stdout")).expect("first stdout remains"),
        first_long_stdout
    );
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
fn max_concurrency_and_invalid_max_active_are_parse_errors() {
    let cases: &[&[&str]] = &[
        &["run-plan-graph", "--max-concurrency", "8"],
        &["run-plan-graph", "--max-active", "0"],
        &["run-plan-graph", "--max-active"],
        &["run-plan-graph", "--max-active", "2", "--max-active", "3"],
    ];
    for args in cases {
        let output = Command::new(bin())
            .args(*args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("run parse-error argv");
        assert_eq!(
            output.status.code(),
            Some(2),
            "args={args:?} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn default_task_worker_is_pi_print_without_tools_or_no_context_files() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/run_plan_graph.rs"))
            .expect("read run_plan_graph.rs");
    assert!(
        source.contains("const DEFAULT_WORKER_COMMAND: &str = \"pi\";"),
        "default --task-worker command must remain pi"
    );
    assert!(
        source.contains(
            "const DEFAULT_WORKER_ARGS: &[&str] = &[\"--print\", \"--no-skills\", \"--no-extensions\"];"
        ),
        "default --task-worker args must remain --print --no-skills --no-extensions"
    );

    let help = Command::new(bin()).args(["--help"]).output().expect("help");
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("--task-worker"), "{text}");
    assert!(text.contains("--max-active"), "{text}");
    assert!(
        text.contains("omitted means 4 ordinary tasks"),
        "help must say omitted --max-active means 4 ordinary tasks: {text}"
    );
    assert!(
        !text.contains("--no-context-files"),
        "help must not advertise --no-context-files: {text}"
    );
    assert!(
        !text.contains("--max-concurrency"),
        "help must not advertise --max-concurrency: {text}"
    );
    assert!(
        !text.contains("stdin-exec"),
        "help must keep stdin-exec hidden: {text}"
    );
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
        stderr.contains("path-safe") || stderr.contains("escapes") || stderr.contains("Dagu-safe"),
        "{stderr}"
    );
}
