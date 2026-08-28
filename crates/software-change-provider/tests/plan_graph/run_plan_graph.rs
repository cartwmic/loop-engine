use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
        fs::write(path.join(".journey-baseline"), b"baseline\n").expect("write baseline");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "software-change run-plan-graph"],
            vec!["config", "user.email", "run-plan@example.invalid"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "baseline"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&path)
                .status()
                .expect("run git")
                .success());
        }
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
    let mut command = Command::new(bin());
    super::bounded_process::run_with_stdin(
        &mut command,
        "software-change plan-graph protocol",
        stdin,
    )
    .expect("protocol process should exit")
    .output
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
    invoke_graph_with_env(worker, packet, current_dir, extra, None)
}

fn invoke_graph_with_env(
    worker: &str,
    packet: &Value,
    current_dir: Option<&Path>,
    extra: &[&str],
    path: Option<&Path>,
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
        .args(extra);
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    if let Some(path) = path {
        // Keep the real toolchain available for checkpoint verification while
        // putting the fake Dagu directory first so resolver probes remain
        // observable and deterministic.
        let mut paths = vec![path.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        let path = std::env::join_paths(paths).expect("join test PATH");
        command.env("PATH", path);
    }
    let completed = super::bounded_process::run_with_stdin(
        &mut command,
        "software-change run-plan-graph",
        &serde_json::to_vec(packet).expect("packet JSON"),
    )
    .expect("run-plan-graph process should exit");
    if let Some(error) = completed.stdin_error {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write failure: {error}"
        );
    }
    completed.output
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
    for name in ["intent.json", "design.json"] {
        let path = artifact_root.join(name);
        if !path.exists() {
            fs::write(path, br#"{"revision":"1"}"#).expect("write checkpoint document");
        }
    }
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
    for name in ["intent.json", "design.json"] {
        fs::write(artifact_root.join(name), br#"{"revision":"1"}"#)
            .expect("write checkpoint document");
    }
    fs::create_dir_all(&receipt_dir).expect("receipt dir");
    (dir, artifact_root, receipt_dir)
}

struct RepairFixture {
    repo: TestDir,
    artifact_root: PathBuf,
    receipt_dir: PathBuf,
    capture_root: PathBuf,
}

impl RepairFixture {
    fn new(label: &str) -> Self {
        let repo = TestDir::new(&format!("repair-{label}"));
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "software-change-repair-artifacts-{}-{suffix}",
            std::process::id()
        ));
        let artifact_root = base.join("artifacts");
        let receipt_dir = base.join("receipts");
        let capture_root = base.join("capture");
        fs::create_dir_all(&artifact_root).expect("repair artifact root");
        fs::create_dir_all(&receipt_dir).expect("repair receipt directory");
        for name in ["intent.json", "design.json"] {
            fs::write(artifact_root.join(name), br#"{"revision":"1"}"#)
                .expect("write repair checkpoint document");
        }
        Self {
            repo,
            artifact_root,
            receipt_dir,
            capture_root,
        }
    }

    fn setup_plan(&self) {
        write_plan(
            &self.artifact_root,
            &json!({
                "revision": "plan-r1",
                "tasks": [{"id": "ordinary"}],
                "dependency_graph": []
            }),
        );
    }

    fn write_report(&self, revision: &str) {
        fs::write(
            self.artifact_root.join("implementation-report.json"),
            serde_json::to_vec_pretty(&json!({
                "revision": revision,
                "author": {"name": "repair-fixture", "kind": "script"},
                "plan_revision": "plan-r1",
                "coverage": {"commit": "fixture", "documents": [{"path": "plan.json", "revision": "plan-r1"}]},
                "summary": "fixture report",
                "changed_surface": ["fixture"],
                "validation": ["fixture"]
            }))
            .expect("report JSON"),
        )
        .expect("write implementation report");
    }

    fn create_checkpoint(&self) {
        let output = Command::new(bin())
            .args([
                "checkpoint",
                "--phase",
                "implementation",
                "--artifact-root",
                self.artifact_root.to_str().expect("artifact root UTF-8"),
                "--working-directory",
                self.repo.path().to_str().expect("repository UTF-8"),
            ])
            .current_dir(self.repo.path())
            .output()
            .expect("create implementation checkpoint");
        assert_eq!(
            output.status.code(),
            Some(0),
            "checkpoint stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn accept_report_revision(&self, revision: &str) {
        self.write_report(revision);
        self.create_checkpoint();
        let bytes = fs::read(self.artifact_root.join("implementation-checkpoint.json"))
            .expect("read historical implementation checkpoint");
        let history = self.artifact_root.join("implementation-proof-history");
        fs::create_dir_all(&history).expect("create implementation proof history");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        fs::write(history.join(format!("{digest}.json")), bytes)
            .expect("write historical implementation checkpoint");
    }

    fn checkpoint_value(&self) -> Value {
        serde_json::from_slice(
            &fs::read(self.artifact_root.join("implementation-checkpoint.json"))
                .expect("read implementation checkpoint"),
        )
        .expect("checkpoint JSON")
    }

    fn current_state(&self) -> String {
        self.checkpoint_value()["repository"]["state_sha256"]
            .as_str()
            .expect("repository state")
            .to_owned()
    }

    fn source(&self, id: &str) -> Value {
        json!({
            "kind": "context-record",
            "id": format!("{id}-evidence"),
        })
    }

    fn context(&self, finding: Value, subject_revision: &str, _state: &str) -> Value {
        let source_id = finding["source"]["id"].as_str().expect("source id");
        let gate = "implementation-review";
        let subject = "implementation-report.json";
        let source = json!({
            "id": source_id,
            "kind": "review-evidence",
            "data": {
                "gate": gate,
                "policy_id": finding["policy_id"],
                "result": "fail",
                "findings": finding["statement"],
                "author": {"name": "reviewer", "kind": "agent"},
                "subject": subject,
                "subject_revision": subject_revision,
                "config_version": "test-1"
            },
            "sequence": 0,
            "created_at": 0
        });
        json!([source, {
            "id": "ledger-repair",
            "kind": "finding-ledger",
            "data": {
                "schema_version": "1",
                "gate": gate,
                "subject": subject,
                "subject_revision": subject_revision,
                "author": {"name": "driver", "kind": "agent"},
                "findings": [finding]
            },
            "sequence": 1,
            "created_at": 1
        }])
    }

    fn finding(&self, id: &str) -> Value {
        json!({
            "id": id,
            "source": self.source(id),
            "policy_id": "implementation-contract",
            "statement": "repair this exact implementation defect",
            "disposition": "accepted",
            "reason": "driver selected the no-task route",
            "owner_phase": "implementation",
            "task_ids": [],
            "review_axes": ["implementation-contract"],
            "status": "unresolved"
        })
    }

    fn packet(&self, invocation_input: Value, context: Value) -> Value {
        let mut packet = packet_with_capture(
            "run-repair",
            "implement",
            self.artifact_root.to_str().expect("artifact root UTF-8"),
            "Implement",
            &self.capture_root,
        );
        packet["invocation_input"] = invocation_input;
        packet["context"] = context;
        packet
    }
}

impl Drop for RepairFixture {
    fn drop(&mut self) {
        if let Some(base) = self.artifact_root.parent() {
            let _ = fs::remove_dir_all(base);
        }
    }
}

#[cfg(unix)]
fn dagu_sentinel(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).expect("dagu sentinel directory");
    fs::write(
        bin_dir.join("dagu"),
        b"#!/bin/sh\nprintf 'probed\\n' >> \"$(dirname \"$0\")/probed\"\nprintf '2.14.0\\n'\n",
    )
    .expect("write dagu sentinel");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(bin_dir.join("dagu"))
        .expect("dagu sentinel metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin_dir.join("dagu"), permissions).expect("chmod dagu sentinel");
    bin_dir.join("probed")
}

fn write_standing_plan_task(artifact_root: &Path, plan_revision: &str, task_id: &str) {
    let task = json!({"id": task_id});
    let packet = format!(
        "{{\"artifact_root\":\"{}\"}}\\n---\\n\\n{}",
        artifact_root.display(),
        task
    );
    let file = json!({
        "schema_version": "1",
        "plan_revision": plan_revision,
        "results": [{
            "assignment_id": task_id,
            "plan_revision": plan_revision,
            "task": task,
            "packet": packet,
            "dependencies": [],
            "worker": {"command": "python3", "args": []},
            "exit_code": 0,
            "repository_effect": null,
            "capture_dir": artifact_root.join("standing").to_string_lossy()
        }]
    });
    fs::write(
        artifact_root.join("plan-task-results.json"),
        serde_json::to_vec_pretty(&file).expect("standing result JSON"),
    )
    .expect("write standing result");
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

#[cfg(unix)]
#[test]
fn bound_invocation_selection_refuses_every_invalid_shape_before_dagu_or_graph() {
    let cases = vec![
        ("malformed", json!("not an object"), Vec::<&str>::new()),
        (
            "extra",
            json!({
                "plan_revision": "plan-r1",
                "task_roots": ["a"],
                "extra": true
            }),
            Vec::<&str>::new(),
        ),
        ("null", Value::Null, Vec::<&str>::new()),
        (
            "wrong-types",
            json!({"plan_revision": 7, "task_roots": ["a"]}),
            Vec::<&str>::new(),
        ),
        (
            "empty-roots",
            json!({"plan_revision": "plan-r1", "task_roots": []}),
            Vec::<&str>::new(),
        ),
        (
            "blank-root",
            json!({"plan_revision": "plan-r1", "task_roots": ["  "]}),
            Vec::<&str>::new(),
        ),
        (
            "duplicate-root",
            json!({"plan_revision": "plan-r1", "task_roots": ["a", "a"]}),
            Vec::<&str>::new(),
        ),
        (
            "unknown-root",
            json!({"plan_revision": "plan-r1", "task_roots": ["missing"]}),
            Vec::<&str>::new(),
        ),
        (
            "stale-plan",
            json!({"plan_revision": "old-plan", "task_roots": ["a"]}),
            Vec::<&str>::new(),
        ),
        (
            "missing-predecessor",
            json!({"plan_revision": "plan-r1", "task_roots": ["b"]}),
            Vec::<&str>::new(),
        ),
        (
            "mixed-selector",
            json!({"plan_revision": "plan-r1", "task_roots": ["a"]}),
            vec!["--task", "a"],
        ),
    ];

    for (label, invocation_input, extra) in cases {
        let (_dir, artifact_root, receipt_dir) = fixture(&format!("bound-invalid-{label}"));
        write_plan(
            &artifact_root,
            &json!({
                "revision": "plan-r1",
                "tasks": [{"id": "a"}, {"id": "b"}],
                "dependency_graph": [{"from": "a", "to": "b"}]
            }),
        );
        let fake_dagu = artifact_root.parent().unwrap().join("fake-dagu-bin");
        let probe_marker = dagu_sentinel(&fake_dagu);
        let mut invoke_packet = packet(
            "run-bound-invalid",
            "implement",
            artifact_root.to_str().unwrap(),
            "Implement",
        );
        invoke_packet["invocation_input"] = invocation_input;
        let output = invoke_graph_with_env(
            &task_worker(&receipt_dir, &["--write-report"]),
            &invoke_packet,
            None,
            &extra,
            Some(&fake_dagu),
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !probe_marker.exists(),
            "{label} probed Dagu: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !capture_dir_for_root(&artifact_root).exists(),
            "{label} emitted a capture graph"
        );
        assert!(!artifact_root.join("implementation-report.json").exists());
        assert!(!artifact_root
            .join("implementation-checkpoint.json")
            .exists());
        assert!(
            receipt_dir
                .read_dir()
                .expect("receipt directory")
                .next()
                .is_none(),
            "{label} started a task worker"
        );
    }
}

#[cfg(unix)]
#[test]
fn bound_repair_selection_refuses_invalid_shapes_before_dagu_or_mutation() {
    let cases = [
        ("null", Value::Null),
        ("string", json!("repair")),
        ("empty", json!({"repair_finding_ids": []})),
        ("blank", json!({"repair_finding_ids": ["  "]})),
        (
            "duplicate",
            json!({"repair_finding_ids": ["F-repair", "F-repair"]}),
        ),
        (
            "extra",
            json!({"repair_finding_ids": ["F-repair"], "extra": true}),
        ),
        (
            "mixed",
            json!({
                "plan_revision": "plan-r1",
                "task_roots": ["ordinary"],
                "repair_finding_ids": ["F-repair"]
            }),
        ),
        ("wrong-type", json!({"repair_finding_ids": [7]})),
    ];
    for (label, invocation_input) in cases {
        let fixture = RepairFixture::new(&format!("invalid-{label}"));
        let fake_dagu = fixture
            .artifact_root
            .parent()
            .unwrap()
            .join("fake-dagu-bin");
        let probe_marker = dagu_sentinel(&fake_dagu);
        let worker = task_worker(
            &fixture.receipt_dir,
            &[
                "--spawn-marker",
                fixture.repo.path().join("worker-started").to_str().unwrap(),
            ],
        );
        let output = invoke_graph_with_env(
            &worker,
            &fixture.packet(invocation_input, json!([])),
            Some(fixture.repo.path()),
            &[],
            Some(&fake_dagu),
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!probe_marker.exists(), "{label} probed Dagu");
        assert!(
            !fixture.capture_root.exists(),
            "{label} created capture data"
        );
        assert!(fixture.receipt_dir.read_dir().unwrap().next().is_none());
        assert!(!fixture.repo.path().join("worker-started").exists());
        assert!(!fixture
            .artifact_root
            .join("implementation-report.json")
            .exists());
        assert!(!fixture
            .artifact_root
            .join("implementation-checkpoint.json")
            .exists());
    }
}

#[cfg(unix)]
#[test]
fn bound_repair_runs_one_captured_generic_worker_and_refreshes_checkpoint() {
    let fixture = RepairFixture::new("valid");
    fixture.setup_plan();
    fixture.write_report("pre-report");
    fixture.create_checkpoint();
    let pre_state = fixture.current_state();
    let finding = fixture.finding("F-repair");
    let context = fixture.context(finding.clone(), "pre-report", &pre_state);
    let effect = fixture.repo.path().join("repair-effect.txt");
    let worker = task_worker(
        &fixture.receipt_dir,
        &[
            "--write-repair-report",
            "--repair-report-revision",
            "post-report",
            "--repair-effect-file",
            effect.to_str().unwrap(),
        ],
    );
    let output = invoke_graph(
        &worker,
        &fixture.packet(json!({"repair_finding_ids": ["F-repair"]}), context),
        Some(fixture.repo.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        effect.is_file(),
        "repair worker must change the Git checkout"
    );
    assert!(fixture.receipt_dir.join("ad-hoc-repair.stdin").is_file());
    assert!(!fixture.receipt_dir.join("ordinary.stdin").exists());
    assert!(!fixture.receipt_dir.join("summarizer.stdin").exists());
    assert!(!fixture
        .artifact_root
        .join("plan-task-results.json")
        .exists());

    let repair_stdin = fs::read_to_string(fixture.receipt_dir.join("ad-hoc-repair.stdin"))
        .expect("repair worker stdin");
    let (location_raw, assignment_raw) = repair_stdin
        .split_once("\n---\n\n")
        .expect("repair location and assignment");
    let location: Value = serde_json::from_str(location_raw).expect("repair location JSON");
    assert_eq!(
        location,
        json!({"artifact_root": fixture.artifact_root.to_string_lossy()})
    );
    let assignment: Value = serde_json::from_str(assignment_raw).expect("repair assignment JSON");
    assert_eq!(assignment["kind"], "ad-hoc-repair");
    assert_eq!(assignment["plan_revision"], "plan-r1");
    assert_eq!(assignment["pre_report_revision"], "pre-report");
    assert_eq!(assignment["pre_repository_state_sha256"], pre_state);
    assert_eq!(assignment["findings"], json!([finding]));
    assert!(assignment["instruction"]
        .as_str()
        .unwrap()
        .contains("fresh report revision"));

    let summary = read_summary_at(&fixture.capture_root);
    let workers = summary["workers"].as_array().expect("repair workers");
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["assignment_id"], "ad-hoc-repair");
    assert!(workers[0].get("task_definition").is_none());
    assert!(workers[0].get("repository_effect").is_none());
    assert_eq!(workers[0]["routed_inputs"], json!([finding]));
    assert_eq!(workers[0]["task_packet"], repair_stdin);
    assert!(workers[0]["selected_output_path"]
        .as_str()
        .unwrap()
        .contains("ad-hoc-repair/attempts/1/stdout"));
    assert_eq!(summary["repair"]["repair_finding_ids"], json!(["F-repair"]));
    assert_eq!(summary["repair"]["pre_report_revision"], "pre-report");
    assert_eq!(summary["repair"]["post_report_revision"], "post-report");
    assert_eq!(summary["repair"]["pre_repository_state_sha256"], pre_state);
    let post_state = summary["repair"]["post_repository_state_sha256"]
        .as_str()
        .expect("post state");
    assert_ne!(post_state, pre_state);
    assert!(fixture
        .artifact_root
        .join("implementation-checkpoint.json")
        .is_file());
    let report: Value = serde_json::from_slice(
        &fs::read(fixture.artifact_root.join("implementation-report.json")).unwrap(),
    )
    .expect("post report JSON");
    assert_eq!(report["revision"], "post-report");
    let yaml = emitted_yaml(&fixture.capture_root);
    assert!(yaml.contains("name: \"ad-hoc-repair\""), "{yaml}");
    assert!(!yaml.contains("summarizer"), "{yaml}");
}

#[test]
fn bound_repair_selection_refuses_non_current_or_non_empty_task_findings_before_dagu() {
    let cases = [
        ("unknown", "unknown", None, None),
        (
            "resolved",
            "F-repair",
            Some(("status", json!("resolved"))),
            None,
        ),
        (
            "rejected",
            "F-repair",
            Some(("disposition", json!("rejected"))),
            None,
        ),
        (
            "advisory",
            "F-repair",
            Some(("disposition", json!("advisory"))),
            None,
        ),
        (
            "wrong-phase",
            "F-repair",
            Some(("owner_phase", json!("plan"))),
            None,
        ),
        (
            "task-routed",
            "F-repair",
            Some(("task_ids", json!(["ordinary"]))),
            None,
        ),
        ("stale-subject", "F-repair", None, Some("old-report")),
        (
            "stale-checkpoint",
            "F-repair",
            None,
            Some("sha256:0000000000000000000000000000000000000000000000000000000000000000"),
        ),
    ];
    for (label, requested_id, mutation, stale) in cases {
        let fixture = RepairFixture::new(label);
        fixture.setup_plan();
        fixture.write_report("pre-report");
        fixture.create_checkpoint();
        let pre_report =
            fs::read(fixture.artifact_root.join("implementation-report.json")).expect("pre report");
        let pre_checkpoint = fs::read(fixture.artifact_root.join("implementation-checkpoint.json"))
            .expect("pre checkpoint");
        let pre_state = fixture.current_state();
        if label == "stale-checkpoint" {
            fs::write(fixture.repo.path().join("marker.txt"), b"changed\n")
                .expect("make checkpoint stale");
        }
        let mut finding = fixture.finding("F-repair");
        if let Some((field, value)) = mutation {
            finding[field] = value;
            if field == "disposition" {
                finding["status"] = json!("recorded");
                finding["owner_phase"] = Value::Null;
                finding["review_axes"] = json!([]);
            }
        }
        let subject_revision = stale
            .filter(|value| value == &"old-report")
            .unwrap_or("pre-report");
        let state = stale
            .filter(|value| value.starts_with("sha256:"))
            .unwrap_or(&pre_state);
        let mut context = fixture.context(finding, subject_revision, state);
        if label == "unknown" {
            context[1]["data"]["findings"][0]["id"] = json!("F-other");
        }
        let fake_dagu = fixture
            .artifact_root
            .parent()
            .unwrap()
            .join("fake-dagu-bin");
        let probe_marker = dagu_sentinel(&fake_dagu);
        let worker = task_worker(
            &fixture.receipt_dir,
            &[
                "--spawn-marker",
                fixture.repo.path().join("worker-started").to_str().unwrap(),
            ],
        );
        let output = invoke_graph_with_env(
            &worker,
            &fixture.packet(json!({"repair_finding_ids": [requested_id]}), context),
            Some(fixture.repo.path()),
            &[],
            Some(&fake_dagu),
        );
        assert_eq!(
            output.status.code(),
            Some(if label == "stale-checkpoint" { 1 } else { 2 }),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!probe_marker.exists(), "{label} probed Dagu");
        assert!(
            !fixture.capture_root.exists(),
            "{label} created capture data"
        );
        assert!(fixture.receipt_dir.read_dir().unwrap().next().is_none());
        assert_eq!(
            fs::read(fixture.artifact_root.join("implementation-report.json")).unwrap(),
            pre_report,
            "{label} changed implementation report"
        );
        assert_eq!(
            fs::read(fixture.artifact_root.join("implementation-checkpoint.json")).unwrap(),
            pre_checkpoint,
            "{label} changed implementation checkpoint"
        );
    }
}

#[cfg(unix)]
#[test]
fn bound_repair_report_revision_collisions_run_worker_but_create_no_post_checkpoint() {
    for (label, collision_revision, historical) in [
        ("immediate-collision", "pre-report", false),
        ("historical-collision", "old-report", true),
    ] {
        let fixture = RepairFixture::new(label);
        fixture.setup_plan();
        if historical {
            fixture.accept_report_revision("old-report");
        }
        fixture.write_report("pre-report");
        fixture.create_checkpoint();
        let pre_state = fixture.current_state();
        let finding = fixture.finding("F-repair");
        let context = fixture.context(finding, "pre-report", &pre_state);
        let worker = task_worker(
            &fixture.receipt_dir,
            &[
                "--write-repair-report",
                "--repair-report-revision",
                collision_revision,
            ],
        );
        let output = invoke_graph(
            &worker,
            &fixture.packet(json!({"repair_finding_ids": ["F-repair"]}), context),
            Some(fixture.repo.path()),
        );
        assert_ne!(output.status.code(), Some(0), "{label} unexpectedly passed");
        assert!(fixture.receipt_dir.join("ad-hoc-repair.stdin").is_file());
        assert!(!fixture.receipt_dir.join("ordinary.stdin").exists());
        assert!(!fixture.receipt_dir.join("summarizer.stdin").exists());
        assert!(!fixture
            .artifact_root
            .join("implementation-checkpoint.json")
            .exists());
        let report: Value = serde_json::from_slice(
            &fs::read(fixture.artifact_root.join("implementation-report.json")).unwrap(),
        )
        .expect("collision report");
        assert_eq!(report["revision"], collision_revision);
        let summary = read_summary_at(&fixture.capture_root);
        assert_eq!(summary["workers"].as_array().unwrap().len(), 1);
        assert_eq!(summary["workers"][0]["exit_code"], 0);
        assert_eq!(summary["repair"]["pre_report_revision"], "pre-report");
        assert!(summary["repair"]["post_report_revision"].is_null());
        assert!(String::from_utf8_lossy(&output.stderr).contains("collides"));
    }
}

#[test]
fn bound_invocation_selection_runs_dependants_with_standing_prerequisite() {
    let (_dir, artifact_root, receipt_dir) = fixture("bound-selection-standing");
    write_plan(
        &artifact_root,
        &json!({
            "revision": "plan-r1",
            "tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}, {"id": "unselected"}],
            "dependency_graph": [
                {"from": "a", "to": "b"},
                {"from": "b", "to": "c"}
            ]
        }),
    );
    write_standing_plan_task(&artifact_root, "plan-r1", "a");
    let mut invoke_packet = packet(
        "run-bound-selection",
        "implement",
        artifact_root.to_str().unwrap(),
        "Implement",
    );
    invoke_packet["invocation_input"] = json!({"plan_revision": "plan-r1", "task_roots": ["b"]});
    invoke_packet["standing_assignment_ids"] = json!(["a"]);
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &invoke_packet,
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(receipt_dir.join("b.stdin").is_file());
    assert!(receipt_dir.join("c.stdin").is_file());
    assert!(receipt_dir.join("summarizer.stdin").is_file());
    assert!(!receipt_dir.join("a.stdin").exists());
    assert!(!receipt_dir.join("unselected.stdin").exists());
    assert!(artifact_root.join("implementation-report.json").is_file());
    assert!(artifact_root
        .join("implementation-checkpoint.json")
        .is_file());

    let selection: Value = serde_json::from_slice(
        &fs::read(capture_dir_for_root(&artifact_root).join("selection.json"))
            .expect("selection record"),
    )
    .expect("selection JSON");
    assert_eq!(selection["requested"], json!(["b"]));
    assert_eq!(selection["tasks"], json!(["b", "c"]));
}

#[test]
fn bound_invocation_input_omitted_runs_full_graph() {
    let (_dir, artifact_root, receipt_dir) = fixture("bound-selection-omitted");
    write_plan(
        &artifact_root,
        &json!({
            "revision": "plan-r1",
            "tasks": [{"id": "a"}, {"id": "b"}],
            "dependency_graph": []
        }),
    );
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &packet(
            "run-bound-full",
            "implement",
            artifact_root.to_str().unwrap(),
            "Implement",
        ),
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for task in ["a", "b", "summarizer"] {
        assert!(receipt_dir.join(format!("{task}.stdin")).is_file());
    }
    let selection: Value = serde_json::from_slice(
        &fs::read(capture_dir_for_root(&artifact_root).join("selection.json"))
            .expect("selection record"),
    )
    .expect("selection JSON");
    assert!(selection["requested"].is_null());
    assert_eq!(selection["tasks"], json!(["a", "b"]));
}

#[test]
fn bound_selection_routes_only_matching_current_implementation_findings() {
    let (dir, artifact_root, receipt_dir) = fixture("bound-selection-findings");
    write_plan(
        &artifact_root,
        &json!({
            "revision": "plan-r1",
            "tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "dependency_graph": [{"from": "b", "to": "c"}]
        }),
    );
    let finding = |id: &str, task_id: &str| {
        json!({
            "id": id,
            "source": {"kind": "context-record", "id": format!("{id}-evidence")},
            "policy_id": "task-sized",
            "statement": id,
            "disposition": "accepted",
            "reason": "driver decision",
            "owner_phase": "implementation",
            "task_ids": [task_id],
            "review_axes": ["task-sized"],
            "status": "unresolved"
        })
    };
    let findings = [
        finding("F-a", "a"),
        finding("F-b", "b"),
        finding("F-c", "c"),
    ];
    let mut context_records = Vec::new();
    for (index, finding) in findings.iter().enumerate() {
        context_records.push(json!({
            "id": finding["source"]["id"],
            "kind": "review-evidence",
            "data": {
                "gate": "plan-review",
                "policy_id": "task-sized",
                "result": "fail",
                "findings": finding["statement"],
                "author": {"name": format!("reviewer-{index}"), "kind": "agent"},
                "subject": "plan.json",
                "subject_revision": "plan-r1",
                "config_version": "test-1"
            },
            "sequence": index,
            "created_at": index
        }));
    }
    context_records.push(json!({
        "id": "ledger-1",
        "kind": "finding-ledger",
        "data": {
            "schema_version": "1",
            "gate": "plan-review",
            "subject": "plan.json",
            "subject_revision": "plan-r1",
            "author": {"name": "driver", "kind": "agent"},
            "findings": findings
        },
        "sequence": 10,
        "created_at": 10
    }));
    let context = Value::Array(context_records);
    let mut invoke_packet = packet(
        "run-bound-findings",
        "implement",
        artifact_root.to_str().unwrap(),
        "Implement",
    );
    invoke_packet["context"] = context;
    invoke_packet["invocation_input"] = json!({"plan_revision": "plan-r1", "task_roots": ["b"]});
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &invoke_packet,
        Some(dir.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (task_id, expected_finding) in [("b", "F-b"), ("c", "F-c")] {
        let raw =
            fs::read_to_string(receipt_dir.join(format!("{task_id}.stdin"))).expect("task packet");
        let (_, task_raw) = raw.split_once("\n---\n\n").expect("task separator");
        let task: Value = serde_json::from_str(task_raw).expect("task JSON");
        let routed = task["finding_context"]
            .as_array()
            .expect("finding_context array");
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0]["id"], expected_finding);
        assert!(routed.iter().all(|entry| entry["id"] != "F-a"));
    }
    assert!(!receipt_dir.join("a.stdin").exists());
}

#[test]
fn subset_plan_graph_refuses_missing_prerequisite_and_runs_dependants() {
    let (_dir, artifact_root, receipt_dir) = fixture("subset-selection");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "dependency_graph": [
                {"from": "a", "to": "b"},
                {"from": "a", "to": "c"}
            ]
        }),
    );

    let refused = invoke_graph_with(
        &task_worker(&receipt_dir, &["--write-report"]),
        &packet(
            "run-subset",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
        &["--task", "b"],
    );
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("missing standing prerequisites"));
    assert!(!receipt_dir.join("b.stdin").exists());

    let first_receipts = artifact_root.parent().unwrap().join("receipts-first");
    fs::create_dir_all(&first_receipts).expect("first receipts");
    let first_capture = artifact_root.parent().unwrap().join("captures/inv-first");
    let first = invoke_graph_with(
        &task_worker(&first_receipts, &["--write-report"]),
        &packet_with_capture(
            "run-subset",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
            &first_capture,
        ),
        None,
        &["--task", "a"],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    for task in ["a", "b", "c"] {
        assert!(first_receipts.join(format!("{task}.stdin")).is_file());
    }

    let second_receipts = artifact_root.parent().unwrap().join("receipts-second");
    fs::create_dir_all(&second_receipts).expect("second receipts");
    let second_capture = artifact_root.parent().unwrap().join("captures/inv-second");
    let second = invoke_graph_with(
        &task_worker(&second_receipts, &["--write-report"]),
        &packet_with_capture(
            "run-subset",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
            &second_capture,
        ),
        None,
        &["--task", "c"],
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(second_receipts.join("c.stdin").is_file());
    assert!(!second_receipts.join("a.stdin").exists());
    assert!(!second_receipts.join("b.stdin").exists());
    let selection: Value = serde_json::from_slice(
        &fs::read(second_capture.join("selection.json")).expect("selection record"),
    )
    .expect("selection JSON");
    assert_eq!(selection["tasks"], json!(["c"]));
    let summary = read_summary_at(&second_capture);
    assert_eq!(summary["workers"][0]["assignment_id"], "c");
    assert_eq!(summary["workers"][0]["selected_attempt"], 1);
    let selected_output = PathBuf::from(
        summary["workers"][0]["selected_output_path"]
            .as_str()
            .expect("selected output path"),
    );
    assert!(selected_output.is_file());
    let mut hasher = Sha256::new();
    hasher.update(fs::read(&selected_output).expect("selected output bytes"));
    assert_eq!(
        summary["workers"][0]["selected_output_sha256"],
        format!("sha256:{:x}", hasher.finalize())
    );
    assert_eq!(
        summary["workers"][0]["dependencies"],
        json!(["a"]),
        "recorded dependencies must retain standing prerequisites even when no Dagu step is spawned"
    );
    assert!(artifact_root.join("plan-task-results.json").is_file());
}

#[test]
fn plan_graph_rejects_invalid_selection_and_missing_auto_dependant_prerequisites() {
    let cases = [
        (vec!["--tasks", ""], "empty selection"),
        (vec!["--task", "unknown"], "unknown selection"),
        (vec!["--task", "a", "--task", "a"], "duplicate selection"),
    ];
    for (extra, label) in cases {
        let fixture_label = label.replace(' ', "-");
        let (_dir, artifact_root, receipt_dir) = fixture(&fixture_label);
        write_plan(
            &artifact_root,
            &json!({
                "tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
                "dependency_graph": [
                    {"from": "a", "to": "c"},
                    {"from": "b", "to": "c"}
                ]
            }),
        );
        let output = invoke_graph_with(
            &task_worker(&receipt_dir, &["--write-report"]),
            &packet(
                "run-invalid-selection",
                "implement",
                &artifact_root.to_string_lossy(),
                "Do the work",
            ),
            None,
            &extra,
        );
        assert_eq!(output.status.code(), Some(2), "{label}: {output:?}");
        assert!(
            receipt_dir
                .read_dir()
                .expect("receipt directory")
                .next()
                .is_none(),
            "{label} started a worker"
        );
    }

    // Selecting A adds C as a dependant, but C also requires B.  B is not
    // auto-included, so the whole selection is refused before Dagu starts.
    let (_dir, artifact_root, receipt_dir) = fixture("missing-auto-dependant-prerequisite");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "dependency_graph": [
                {"from": "a", "to": "c"},
                {"from": "b", "to": "c"}
            ]
        }),
    );
    let output = invoke_graph_with(
        &task_worker(&receipt_dir, &["--write-report"]),
        &packet(
            "run-missing-auto-dependant-prerequisite",
            "implement",
            &artifact_root.to_string_lossy(),
            "Do the work",
        ),
        None,
        &["--task", "a"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("c"));
    assert!(receipt_dir
        .read_dir()
        .expect("receipt directory")
        .next()
        .is_none());
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
            .bounded_output("software-change plan-graph subprocess")
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
fn implementation_context_routes_exact_current_findings_without_widening_task_stdin() {
    let (dir, artifact_root, receipt_dir) = fixture("finding-routing");
    let task_a = json!({
        "id": "task-a",
        "objective": "Do A",
        "dependencies": [],
        "source_of_truth": ["plan.json"],
        "deliverables": ["a"],
        "out_of_scope": [],
        "validation": ["public A"],
        "handoff": "A is done"
    });
    let task_b = json!({
        "id": "task-b",
        "objective": "Do B",
        "dependencies": [],
        "source_of_truth": ["plan.json"],
        "deliverables": ["b"],
        "out_of_scope": [],
        "validation": ["public B"],
        "handoff": "B is done"
    });
    write_plan(
        &artifact_root,
        &json!({
            "revision": "1",
            "tasks": [task_a, task_b],
            "dependency_graph": []
        }),
    );
    let finding =
        |id: &str, disposition: &str, status: &str, owner: Value, tasks: Value, axes: Value| {
            json!({
                "id": id,
                "source": {"kind": "context-record", "id": format!("{id}-evidence")},
                "policy_id": "task-sized",
                "statement": id,
                "disposition": disposition,
                "reason": "driver decision",
                "owner_phase": owner,
                "task_ids": tasks,
                "review_axes": axes,
                "status": status
            })
        };
    let ledger = json!({
        "schema_version": "1",
        "gate": "plan-review",
        "subject": "plan.json",
        "subject_revision": "1",
        "author": {"name": "driver", "kind": "agent"},
        "findings": [
            finding("F-route-a", "accepted", "unresolved", json!("implementation"), json!(["task-a"]), json!(["task-sized"])),
            finding("F-route-b", "accepted", "unresolved", json!("implementation"), json!(["task-b"]), json!(["task-sized"])),
            finding("F-plan-owned", "accepted", "unresolved", json!("plan"), json!(["task-a"]), json!(["task-sized"])),
            finding("F-resolved", "accepted", "resolved", json!("implementation"), json!(["task-a"]), json!(["task-sized"])),
            finding("F-stale", "accepted", "stale", json!("implementation"), json!(["task-a"]), json!(["task-sized"])),
            finding("F-rejected", "rejected", "recorded", Value::Null, json!([]), json!([])),
            finding("F-advisory", "advisory", "recorded", Value::Null, json!([]), json!([]))
        ]
    });
    let findings = ledger["findings"].as_array().unwrap().clone();
    let mut context_records = Vec::new();
    for (index, finding) in findings.iter().enumerate() {
        let accepted_unresolved =
            finding["disposition"] == "accepted" && finding["status"] == "unresolved";
        context_records.push(json!({
            "id": finding["source"]["id"],
            "kind": "review-evidence",
            "data": {
                "gate": "plan-review",
                "policy_id": finding["policy_id"],
                "result": if accepted_unresolved { "fail" } else { "pass" },
                "findings": if accepted_unresolved { finding["statement"].clone() } else { json!("") },
                "author": {"name": format!("reviewer-{index}"), "kind": "agent"},
                "subject": "plan.json",
                "subject_revision": "1",
                "config_version": "test-1"
            },
            "sequence": index,
            "created_at": index
        }));
    }
    context_records.push(json!({
        "id": "ledger-1",
        "kind": "finding-ledger",
        "data": ledger,
        "sequence": 10,
        "created_at": 10
    }));
    context_records.push(json!({
        "id": "proposal-1",
        "kind": "advisory-finding-proposal",
        "data": {"proposals": [{"candidate_source_ids": ["source-1"]}]},
        "sequence": 11,
        "created_at": 11
    }));
    let context = Value::Array(context_records);
    let mut invoke_packet = packet(
        "run-1",
        "implement",
        artifact_root.to_str().unwrap(),
        "Implement",
    );
    invoke_packet["context"] = context;
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &invoke_packet,
        Some(dir.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (task_id, expected_finding) in
        [("task-a", Some("F-route-a")), ("task-b", Some("F-route-b"))]
    {
        let raw_stdin =
            fs::read_to_string(receipt_dir.join(format!("{task_id}.stdin"))).expect("task receipt");
        let (_, task_raw) = raw_stdin.split_once("\n---\n\n").expect("task separator");
        let task: Value = serde_json::from_str(task_raw).expect("task JSON");
        let routed = task["finding_context"]
            .as_array()
            .expect("finding_context array");
        assert_eq!(routed.len(), 1, "exact route for {task_id}");
        assert_eq!(routed[0]["id"], expected_finding.unwrap());
        assert!(routed.iter().all(|entry| entry["id"] != "F-plan-owned"));
        assert!(routed.iter().all(|entry| entry["id"] != "F-resolved"));
        assert!(routed.iter().all(|entry| entry["id"] != "F-stale"));
        assert!(routed.iter().all(|entry| entry["id"] != "F-rejected"));
        assert!(routed.iter().all(|entry| entry["id"] != "F-advisory"));
        let location: Value =
            serde_json::from_str(raw_stdin.split_once("\n---\n\n").unwrap().0).unwrap();
        assert_eq!(
            location
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["artifact_root"]
        );
    }
}

#[test]
fn finding_routing_empty_context_still_runs_full_dag_and_emits_empty_arrays() {
    let (dir, artifact_root, receipt_dir) = fixture("finding-routing-empty");
    write_plan(
        &artifact_root,
        &json!({
            "tasks": [{"id": "task-a"}, {"id": "task-b"}],
            "dependency_graph": []
        }),
    );
    let mut invoke_packet = packet(
        "run-1",
        "implement",
        artifact_root.to_str().unwrap(),
        "Implement",
    );
    invoke_packet["context"] = json!([]);
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &invoke_packet,
        Some(dir.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for task_id in ["task-a", "task-b"] {
        let raw =
            fs::read_to_string(receipt_dir.join(format!("{task_id}.stdin"))).expect("task receipt");
        let (_, task_raw) = raw.split_once("\n---\n\n").expect("task separator");
        let task: Value = serde_json::from_str(task_raw).expect("task JSON");
        assert_eq!(task["finding_context"], json!([]));
    }
}

#[test]
fn finding_routing_packet_context_is_optional_for_legacy_five_key_invocation() {
    let (dir, artifact_root, receipt_dir) = fixture("finding-routing-legacy");
    write_plan(
        &artifact_root,
        &json!({"tasks": [{"id": "task-a"}], "dependency_graph": []}),
    );
    let output = invoke_graph(
        &task_worker(&receipt_dir, &["--write-report"]),
        &packet(
            "run-1",
            "implement",
            artifact_root.to_str().unwrap(),
            "Implement",
        ),
        Some(dir.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = fs::read_to_string(receipt_dir.join("task-a.stdin")).expect("task receipt");
    let (_, task_raw) = raw.split_once("\n---\n\n").expect("task separator");
    let task: Value = serde_json::from_str(task_raw).expect("task JSON");
    assert!(task.get("finding_context").is_none());
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
    let mut command = Command::new(bin());
    command
        .args([
            "run-plan-graph",
            "--working-directory",
            artifact_root.to_str().expect("artifact root UTF-8"),
            "--task-worker",
            &worker,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::bounded_process::prepare_process_group(&mut command);
    let mut child = command.spawn().expect("spawn live run-plan-graph");
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

    let output =
        super::bounded_process::wait_existing(child, "software-change live run-plan-graph")
            .expect("wait live");
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
        .bounded_output("software-change plan-graph subprocess")
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
            .bounded_output("software-change plan-graph subprocess")
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

    let help = Command::new(bin())
        .args(["--help"])
        .bounded_output("software-change plan-graph subprocess")
        .expect("help");
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
