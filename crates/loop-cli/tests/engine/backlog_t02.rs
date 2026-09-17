//! Public regression coverage for selected plan-task recovery.
//!
//! These cases drive fresh `loop-engine` and `software-change` processes. The
//! task worker is a test-owned script; it deliberately omits the optional
//! `repository_effect` field so the engine must distinguish that omission from
//! a missing task record.

use super::bounded_process::CommandExt;
use loop_core::{
    CompleteWorkSlotInvocationRequest, CreateRunRequest, CreateWorkSlotInvocationRequest,
    InnerWorker, Lifecycle, Persistence, ProviderAssociation, State, Timestamp, Transition,
    WaiterWrittenStatus, WorkSlot, WorkSlotBinding, Workflow,
};
use loop_integrations::SqlitePersistence;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

struct PlanFixture {
    _root: TempDir,
    database: PathBuf,
    repository: PathBuf,
    artifact_root: PathBuf,
    receipt_dir: PathBuf,
    worker: PathBuf,
    provider: PathBuf,
}

impl PlanFixture {
    fn new(label: &str) -> Self {
        let root = tempdir().expect("plan fixture tempdir");
        let root_path = root.path();
        let repository = root_path.join(format!("{label}-repository"));
        let artifact_root = root_path.join(format!("{label}-artifacts"));
        let receipt_dir = root_path.join(format!("{label}-receipts"));
        let worker = root_path.join(format!("{label}-worker.py"));
        fs::create_dir_all(&repository).expect("repository directory");
        fs::create_dir_all(&artifact_root).expect("artifact root");
        fs::create_dir_all(&receipt_dir).expect("receipt directory");
        init_repository(&repository);
        fs::write(artifact_root.join("intent.json"), br#"{"revision":"1"}"#)
            .expect("intent document");
        fs::write(artifact_root.join("design.json"), br#"{"revision":"1"}"#)
            .expect("design document");
        write_plan(&artifact_root, "plan-r1");
        write_recovery_worker(&worker);
        Self {
            database: root_path.join(format!("{label}.sqlite")),
            _root: root,
            repository,
            artifact_root,
            receipt_dir,
            worker,
            provider: workspace_integration::binary("software-change"),
        }
    }

    fn binding(&self) -> Value {
        let worker = serde_json::to_string(&json!({
            "command": "python3",
            "args": [
                self.worker.to_string_lossy(),
                self.receipt_dir.to_string_lossy()
            ]
        }))
        .expect("worker binding JSON");
        json!({
            "command": self.provider.to_string_lossy(),
            "args": [
                "run-plan-graph",
                "--working-directory",
                self.repository.to_string_lossy(),
                "--task-worker",
                worker
            ]
        })
    }

    fn seed(&self, run_id: &str) {
        let binding = self.binding();
        let workflow = Workflow::new(
            "backlog-t02",
            "start",
            vec![
                State::new("start", "Start", "Implement the plan", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        )
        .with_work_slots(vec![WorkSlot::new("implement", "start", "finish")
            .with_stdin_context_kinds(vec!["recovery-context".to_owned()])]);
        let initial_input = json!({
            "artifact_root": self.artifact_root.to_string_lossy(),
            "work_slot_bindings": {"implement": binding}
        });
        let persistence = SqlitePersistence::open(&self.database).expect("open fixture database");
        persistence
            .create_run(CreateRunRequest::new(
                run_id,
                Some("backlog-t02".to_owned()),
                workflow,
                ProviderAssociation::new(json!({
                    "command": self.provider.to_string_lossy(),
                    "args": []
                })),
                initial_input,
                "start",
                Lifecycle::Active,
                Timestamp::from_unix_millis(1),
                "software-change",
                Some(self.artifact_root.to_string_lossy().into_owned()),
            ))
            .expect("create fixture run");
        persistence
            .load_show_data(&run_id.into())
            .expect("observe fixture run");
        persistence
            .set_current_slot_subject(&run_id.into(), &"implement".into(), "subject-r1".to_owned())
            .expect("set fixture subject");
    }
}

struct OpaqueRecoveryFixture {
    _root: TempDir,
    database: PathBuf,
    config: PathBuf,
    artifact_root: PathBuf,
    worker: PathBuf,
}

impl OpaqueRecoveryFixture {
    fn new() -> Self {
        let root = tempdir().expect("opaque recovery tempdir");
        let root_path = root.path();
        let artifact_root = root_path.join("artifacts");
        let worker = root_path.join("worker.py");
        let provider = root_path.join("provider.py");
        let config = root_path.join("providers.toml");
        fs::create_dir_all(&artifact_root).expect("opaque artifact root");
        fs::write(
            &provider,
            r#"#!/usr/bin/env python3
import json
import sys

request = json.load(sys.stdin)
if request.get("operation") == "describe":
    print(json.dumps({
        "id": "backlog-t02-opaque",
        "initial_state": "start",
        "states": [
            {"id": "start", "title": "Start", "instructions": "Run worker", "final": False},
            {"id": "done", "title": "Done", "instructions": "Finished", "final": True}
        ],
        "transitions": [
            {"source": "start", "event": "finish", "target": "done", "kind": "check-free"}
        ],
        "work_slots": [{"id": "slot", "state": "start", "event": "finish"}]
    }))
else:
    print(json.dumps({"result": "allow"}))
"#,
        )
        .expect("write opaque provider");
        fs::write(
            &worker,
            r#"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

packet = json.load(sys.stdin)
artifact_root = Path(packet["artifact_root"])
count_path = artifact_root / "opaque-worker-count"
count = int(count_path.read_text()) if count_path.exists() else 0
count += 1
count_path.write_text(str(count))
(artifact_root / f"opaque-executed-{count}").write_text(json.dumps(packet))
if count == 1:
    capture_dir = Path(packet["capture_dir"])
    (capture_dir / "summary.json").write_text(json.dumps({
        "workers": [{
            "assignment_id": "c",
            "command": sys.argv[0],
            "args": [],
            "exit_code": 0
        }]
    }))
    raise SystemExit(0)
# This admitted opaque replacement really ran, but lost its completion summary.
raise SystemExit(1)
"#,
        )
        .expect("write opaque worker");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&provider, &worker] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                    .expect("make opaque fixture executable");
            }
        }
        fs::write(
            &config,
            format!(
                "[providers.fixture]\ncommand = \"{}\"\n",
                toml_quote(&provider.to_string_lossy())
            ),
        )
        .expect("write opaque provider catalog");
        Self {
            database: root_path.join("loop.sqlite"),
            config,
            artifact_root,
            worker,
            _root: root,
        }
    }

    fn initial_input(&self) -> Value {
        json!({
            "artifact_root": self.artifact_root.to_string_lossy(),
            "work_slot_bindings": {
                "slot": {"command": self.worker.to_string_lossy(), "args": []}
            }
        })
    }
}

fn toml_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn init_repository(repository: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "backlog-t02"],
        vec!["config", "user.email", "backlog-t02@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(repository)
            .status()
            .expect("run git setup")
            .success());
    }
    fs::write(repository.join(".baseline"), b"baseline\n").expect("repository baseline");
    for args in [vec!["add", "-A"], vec!["commit", "-qm", "baseline"]] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(repository)
            .status()
            .expect("commit git setup")
            .success());
    }
}

fn write_plan(artifact_root: &Path, revision: &str) {
    let plan = json!({
        "revision": revision,
        "tasks": [
            {"id": "a", "objective": "A", "dependencies": []},
            {"id": "b", "objective": "B", "dependencies": ["a"]},
            {"id": "c", "objective": "C", "dependencies": ["b"]}
        ],
        "dependency_graph": [
            {"from": "a", "to": "b"},
            {"from": "b", "to": "c"}
        ]
    });
    fs::write(
        artifact_root.join("plan.json"),
        serde_json::to_vec_pretty(&plan).expect("plan JSON"),
    )
    .expect("write plan");
}

fn write_recovery_worker(path: &Path) {
    fs::write(
        path,
        r#"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

separator = "\n---\n\n"
receipt_dir = Path(sys.argv[1])
raw = sys.stdin.read()
location_raw, rest = raw.split(separator, 1)
location = json.loads(location_raw)
if rest.startswith("Write artifact_root/implementation-report.json"):
    task_id = "summarizer"
else:
    task = json.loads(rest)
    task_id = task["id"]
count_path = receipt_dir / (task_id + ".count")
count = int(count_path.read_text()) if count_path.exists() else 0
count += 1
receipt_dir.mkdir(parents=True, exist_ok=True)
count_path.write_text(str(count))
sys.stdout.write(raw)
sys.stdout.flush()
if task_id == "c" and count == 1:
    raise SystemExit(1)
if task_id == "summarizer":
    artifact_root = Path(location["artifact_root"])
    plan_path = Path(location["plan_path"])
    plan = json.loads(plan_path.read_text())
    report = {
        "revision": "report-r1",
        "author": {"name": "backlog-t02-worker", "kind": "script"},
        "plan_revision": plan["revision"],
        "coverage": {
            "commit": "backlog-t02",
            "documents": [{"path": "plan.json", "revision": plan["revision"]}]
        },
        "summary": "selected recovery report",
        "changed_surface": [".baseline"],
        "validation": ["backlog_t02_plan_graph_recovery_public_cli"]
    }
    (artifact_root / "implementation-report.json").write_text(
        json.dumps(report) + "\n"
    )
"#,
    )
    .expect("write recovery worker");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("make recovery worker executable");
    }
}

fn run_invoke(fixture: &PlanFixture, run_id: &str, input: Option<Value>) -> (Output, Value) {
    run_invoke_with_controls(fixture, run_id, input, None)
}

fn run_invoke_preview(fixture: &PlanFixture, run_id: &str, input: Value) -> (Output, Value) {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.args([
        "--database",
        fixture.database.to_str().expect("database UTF-8"),
        "--json",
        "--input",
        &serde_json::to_string(&input).expect("input JSON"),
        "invoke",
        run_id,
        "implement",
        "--preview",
    ]);
    let output = command
        .bounded_output("backlog_t02 loop-engine invoke preview")
        .expect("invoke preview process");
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invoke preview did not return JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, value)
}

fn run_invoke_with_controls(
    fixture: &PlanFixture,
    run_id: &str,
    input: Option<Value>,
    controls: Option<Value>,
) -> (Output, Value) {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.args([
        "--database",
        fixture.database.to_str().expect("database UTF-8"),
        "--json",
    ]);
    if let Some(input) = input {
        command.args([
            "--input",
            &serde_json::to_string(&input).expect("input JSON"),
        ]);
    }
    if let Some(controls) = controls {
        command.args([
            "--controls",
            &serde_json::to_string(&controls).expect("controls JSON"),
        ]);
    }
    command.args(["invoke", run_id, "implement"]);
    let output = command
        .bounded_output("backlog_t02 loop-engine invoke")
        .expect("invoke process");
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invoke did not return JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, value)
}

fn show_full(fixture: &PlanFixture, run_id: &str) -> Value {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            fixture.database.to_str().expect("database UTF-8"),
            "--json",
            "show",
            "--view",
            "full",
            run_id,
        ])
        .bounded_output("backlog_t02 loop-engine show")
        .expect("show process");
    assert!(
        output.status.success(),
        "show stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("show JSON")
}

fn load_invocations(fixture: &PlanFixture, run_id: &str) -> Vec<loop_core::WorkSlotInvocation> {
    load_invocations_at(&fixture.database, run_id)
}

fn load_invocations_at(database: &Path, run_id: &str) -> Vec<loop_core::WorkSlotInvocation> {
    let persistence = SqlitePersistence::open(database).expect("reopen fixture database");
    persistence
        .load_work_slot_invocations(&run_id.into())
        .expect("load fixture invocations")
}

fn wait_for_invocation(
    fixture: &PlanFixture,
    run_id: &str,
    expected_count: usize,
) -> Vec<loop_core::WorkSlotInvocation> {
    wait_for_invocation_at(&fixture.database, run_id, expected_count)
}

fn wait_for_invocation_at(
    database: &Path,
    run_id: &str,
    expected_count: usize,
) -> Vec<loop_core::WorkSlotInvocation> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let invocations = load_invocations_at(database, run_id);
        if invocations.len() >= expected_count
            && invocations
                .get(expected_count - 1)
                .is_some_and(|invocation| invocation.status.is_some())
        {
            return invocations;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for invocation {expected_count}: {invocations:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn backlog_t02_optional_repository_effect_distinguishes_missing_task_record_public_show() {
    let root = tempdir().expect("change-report fixture");
    let database = root.path().join("loop.sqlite");
    let artifact_root = root.path().join("artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let workflow = Workflow::new(
        "backlog-t02-report",
        "start",
        vec![State::new("start", "Start", "Review", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")]);
    let persistence = SqlitePersistence::open(&database).expect("open report database");
    persistence
        .create_run(CreateRunRequest::new(
            "report-run",
            Some("report".to_owned()),
            workflow,
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            json!({"artifact_root": artifact_root.to_string_lossy()}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            Some(artifact_root.to_string_lossy().into_owned()),
        ))
        .expect("create report run");
    persistence
        .load_show_data(&"report-run".into())
        .expect("observe report run");
    persistence
        .set_current_slot_subject(
            &"report-run".into(),
            &"review".into(),
            "subject-v1".to_owned(),
        )
        .expect("report subject");

    let mut task = InnerWorker::new("/bin/worker", vec!["--task".to_owned()], 0);
    task.assignment_id = "task-a".to_owned();
    task.task_definition = Some(json!({"id": "task-a"}));
    task.task_packet = Some(json!("artifact-root\n---\npacket"));
    task.dependencies = Some(Vec::new());
    task.routed_inputs = Some(json!([]));
    // Deliberately leave the optional effect absent on both present records.
    persistence
        .create_work_slot_invocation(
            CreateWorkSlotInvocationRequest::new(
                "report-run",
                "inv-task",
                "review",
                WorkSlotBinding::new("/bin/worker", vec!["--task".to_owned()]),
                "digest",
                "subject-v1",
                0,
                Timestamp::from_unix_millis(2),
                1000,
                artifact_root.join("capture").to_string_lossy(),
            )
            .with_frozen_run_identity(json!({
                "provider": {"identity": "provider-v1"},
                "input": {"artifact_root": artifact_root.to_string_lossy()}
            })),
        )
        .expect("create task invocation");
    persistence
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "report-run",
            "inv-task",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(3),
            vec![task.clone()],
        ))
        .expect("complete task invocation");

    let unchanged = show_full_report(&database, "report-run");
    let dimensions = &unchanged["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(dimensions["repository_effect"]["changed"], false);

    let connection = Connection::open(&database).expect("open report mutation database");
    let mut changed_effect = task.clone();
    changed_effect.repository_effect = Some(json!({"changed": true}));
    connection
        .execute(
            "UPDATE work_slot_invocations SET inner_workers_json = ?1 WHERE invocation_id = 'inv-task'",
            [serde_json::to_string(&vec![changed_effect]).expect("changed worker JSON")],
        )
        .expect("mutate optional effect");
    let changed = show_full_report(&database, "report-run");
    let changed_dimensions = &changed["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(changed_dimensions["repository_effect"]["changed"], true);

    connection
        .execute(
            "UPDATE work_slot_invocations SET inner_workers_json = '[]' WHERE invocation_id = 'inv-task'",
            [],
        )
        .expect("remove current task record");
    let missing = show_full_report(&database, "report-run");
    let missing_dimensions = &missing["result"]["work_slot_invocations"][0]["change_report"]
        ["plan_task_results"][0]["dimensions"];
    assert_eq!(missing_dimensions["repository_effect"]["changed"], true);
    assert_eq!(missing_dimensions["task_definition"]["changed"], true);
}

fn public_opaque_start(fixture: &OpaqueRecoveryFixture, run_id: &str) -> Value {
    let input = serde_json::to_string(&fixture.initial_input()).expect("opaque input JSON");
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            fixture.database.to_str().expect("opaque database UTF-8"),
            "--config",
            fixture.config.to_str().expect("opaque config UTF-8"),
            "--json",
            "start",
            "fixture",
            &input,
            "--id",
            run_id,
        ])
        .bounded_output("backlog_t02 opaque start")
        .expect("opaque start process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "opaque start did not return JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(output.status.code(), Some(0), "opaque start: {value}");
    assert_eq!(value["status"], "completed", "opaque start: {value}");
    value
}

fn public_opaque_invoke(
    fixture: &OpaqueRecoveryFixture,
    run_id: &str,
    input: Option<Value>,
) -> (Output, Value) {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.args([
        "--database",
        fixture.database.to_str().expect("opaque database UTF-8"),
        "--config",
        fixture.config.to_str().expect("opaque config UTF-8"),
        "--json",
    ]);
    let input = input.map(|value| serde_json::to_string(&value).expect("opaque invoke JSON"));
    if let Some(input) = input.as_deref() {
        command.args(["--input", input]);
    }
    command.args(["invoke", run_id, "slot"]);
    let output = command
        .bounded_output("backlog_t02 opaque invoke")
        .expect("opaque invoke process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "opaque invoke did not return JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, value)
}

fn show_full_report(database: &Path, run_id: &str) -> Value {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database UTF-8"),
            "--json",
            "show",
            "--view",
            "full",
            run_id,
        ])
        .bounded_output("backlog_t02 report show")
        .expect("report show process");
    assert!(
        output.status.success(),
        "report show stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("report show JSON")
}

#[test]
fn backlog_t02_plan_graph_recovery_reuses_b_before_retry_and_reaches_current_checkpoint() {
    let fixture = PlanFixture::new("recovery");
    let run_id = "backlog-t02-recovery";
    fixture.seed(run_id);

    let (first_output, first) = run_invoke(&fixture, run_id, None);
    assert_eq!(first_output.status.code(), Some(0), "first invoke: {first}");
    assert_eq!(first["status"], "completed");
    let first_invocations = wait_for_invocation(&fixture, run_id, 1);
    assert_eq!(
        first_invocations[0].status,
        Some(WaiterWrittenStatus::Failed)
    );

    let first_show = show_full(&fixture, run_id);
    let first_view = first_show["result"]["work_slot_invocations"]
        .as_array()
        .expect("first invocation views")
        .iter()
        .find(|view| view["invocation_id"] == first["result"]["invocation_id"])
        .expect("first invocation view");
    let b = first_view["change_report"]["plan_task_results"]
        .as_array()
        .expect("first plan task report")
        .iter()
        .find(|result| result["assignment_id"] == "b")
        .expect("b report");
    assert_eq!(b["dimensions"]["repository_effect"]["changed"], false);
    assert_eq!(b["standing"], true, "b must stand before retry: {b}");

    let (force_output, force_fresh) = run_invoke_with_controls(
        &fixture,
        run_id,
        Some(json!({"plan_revision": "plan-r1", "task_roots": ["c"]})),
        Some(json!({"force_fresh": true})),
    );
    assert_ne!(
        force_output.status.code(),
        Some(0),
        "force-fresh reused b: {force_fresh}"
    );
    assert!(
        force_fresh["message"]
            .as_str()
            .unwrap_or_default()
            .contains("force-fresh selected execution cannot reuse"),
        "unexpected force-fresh refusal: {force_fresh}"
    );
    assert_eq!(
        load_invocations(&fixture, run_id).len(),
        1,
        "pre-admission refusal must not create a replacement invocation"
    );
    let after_force = show_full(&fixture, run_id);
    let after_force_b = after_force["result"]["work_slot_invocations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["invocation_id"] == first["result"]["invocation_id"])
        .unwrap()["change_report"]["plan_task_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["assignment_id"] == "b")
        .unwrap();
    assert_eq!(
        after_force_b["standing"], true,
        "rejected force-fresh attempt displaced b standing: {after_force_b}"
    );

    let (retry_output, retry) = run_invoke(
        &fixture,
        run_id,
        Some(json!({"plan_revision": "plan-r1", "task_roots": ["c"]})),
    );
    assert_eq!(retry_output.status.code(), Some(0), "retry invoke: {retry}");
    assert_eq!(retry["status"], "completed");
    let invocations = wait_for_invocation(&fixture, run_id, 2);
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].status, Some(WaiterWrittenStatus::Failed));
    assert_eq!(invocations[1].status, Some(WaiterWrittenStatus::Succeeded));

    let after_retry = show_full(&fixture, run_id);
    let first_after_retry = after_retry["result"]["work_slot_invocations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["invocation_id"] == first["result"]["invocation_id"])
        .unwrap();
    for task_id in ["a", "b"] {
        let result = first_after_retry["change_report"]["plan_task_results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|result| result["assignment_id"] == task_id)
            .unwrap();
        assert_eq!(result["standing"], true, "unchanged {task_id} was retired");
    }
    let old_c = first_after_retry["change_report"]["plan_task_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["assignment_id"] == "c")
        .unwrap();
    assert_eq!(old_c["standing"], false, "replaced c was resurrected");
    let retry_view = after_retry["result"]["work_slot_invocations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["invocation_id"] == retry["result"]["invocation_id"])
        .unwrap();
    let retry_c = retry_view["change_report"]["plan_task_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["assignment_id"] == "c")
        .unwrap();
    assert_eq!(retry_c["standing"], true);

    // A later selected recovery must not retire unchanged results from the
    // earlier full invocation. Re-preparing the same selected recovery is a
    // second public selection and must succeed without launching anything.
    let (repeat_output, repeat) = run_invoke_preview(
        &fixture,
        run_id,
        json!({"plan_revision": "plan-r1", "task_roots": ["c"]}),
    );
    assert_eq!(
        repeat_output.status.code(),
        Some(0),
        "repeat preview: {repeat}"
    );
    assert_eq!(repeat["status"], "completed");
    assert_eq!(load_invocations(&fixture, run_id).len(), 2);
    assert_eq!(read_count(&fixture.receipt_dir, "c"), 2);

    assert_eq!(read_count(&fixture.receipt_dir, "a"), 1);
    assert_eq!(read_count(&fixture.receipt_dir, "b"), 1);
    assert_eq!(read_count(&fixture.receipt_dir, "c"), 2);
    assert_eq!(read_count(&fixture.receipt_dir, "summarizer"), 1);

    let results: Value = serde_json::from_slice(
        &fs::read(fixture.artifact_root.join("plan-task-results.json")).expect("plan task results"),
    )
    .expect("plan task results JSON");
    let results = results["results"].as_array().expect("result rows");
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result["exit_code"] == 0));
    assert_eq!(
        results
            .iter()
            .map(|result| result["assignment_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );

    let report: Value = serde_json::from_slice(
        &fs::read(fixture.artifact_root.join("implementation-report.json"))
            .expect("current implementation report"),
    )
    .expect("implementation report JSON");
    assert_eq!(report["revision"], "report-r1");
    assert_eq!(report["plan_revision"], "plan-r1");
    let checkpoint: Value = serde_json::from_slice(
        &fs::read(fixture.artifact_root.join("implementation-checkpoint.json"))
            .expect("current implementation checkpoint"),
    )
    .expect("implementation checkpoint JSON");
    assert_eq!(checkpoint["phase"], "implementation");
    assert_eq!(checkpoint["documents"]["plan_revision"], "plan-r1");
    assert!(!checkpoint["report"]["sha256"].as_str().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn backlog_t02_opaque_failed_replacement_without_summary_does_not_revive_old_success() {
    let fixture = OpaqueRecoveryFixture::new();
    let run_id = "backlog-t02-opaque-replacement";
    public_opaque_start(&fixture, run_id);

    let first_show = show_full_report(&fixture.database, run_id);
    assert_eq!(first_show["result"]["current_state"], "start");
    let (first_output, first) = public_opaque_invoke(&fixture, run_id, None);
    assert_eq!(first_output.status.code(), Some(0), "first invoke: {first}");
    assert_eq!(first["status"], "completed");
    let first_invocations = wait_for_invocation_at(&fixture.database, run_id, 1);
    assert_eq!(
        first_invocations[0].status,
        Some(WaiterWrittenStatus::Succeeded)
    );
    assert!(fixture.artifact_root.join("opaque-executed-1").is_file());

    let first_complete = show_full_report(&fixture.database, run_id);
    let first_assignment = first_complete["result"]["change_report"]["assignments"]
        .as_array()
        .expect("first assignment report")
        .iter()
        .find(|assignment| assignment["assignment_id"] == "c")
        .expect("first c assignment");
    assert_eq!(
        first_assignment["standing"], true,
        "normal completion: {first_assignment}"
    );

    let before_retry = show_full_report(&fixture.database, run_id);
    let (retry_output, retry) = public_opaque_invoke(
        &fixture,
        run_id,
        Some(json!({"opaque": {"assignment": "c"}})),
    );
    assert_eq!(
        retry_output.status.code(),
        Some(0),
        "retry admission: {retry}"
    );
    assert_eq!(retry["status"], "completed");
    let invocations = wait_for_invocation_at(&fixture.database, run_id, 2);
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[1].status, Some(WaiterWrittenStatus::Failed));
    assert!(invocations[1].inner_workers.is_empty());
    assert!(fixture.artifact_root.join("opaque-executed-2").is_file());
    assert!(
        !Path::new(
            retry["result"]["capture_dir"]
                .as_str()
                .expect("retry capture")
        )
        .join("summary.json")
        .exists(),
        "replacement unexpectedly retained a summary"
    );

    let after_retry = show_full_report(&fixture.database, run_id);
    let old_assignment = after_retry["result"]["change_report"]["assignments"]
        .as_array()
        .expect("after-retry assignment report")
        .iter()
        .find(|assignment| assignment["assignment_id"] == "c")
        .expect("after-retry c assignment");
    assert_eq!(
        old_assignment["standing"], false,
        "a real failed opaque replacement with no summary revived the old success: {old_assignment}; before={before_retry}"
    );
    assert_eq!(
        after_retry["result"]["work_slot_invocations"][1]["inner_workers"],
        json!([])
    );
}

#[test]
fn backlog_t02_prepare_facade_rejects_invalid_selection_before_invoke_or_capture() {
    let fixture = PlanFixture::new("invalid-selection");
    let run_id = "backlog-t02-invalid-selection";
    fixture.seed(run_id);
    let (output, value) = run_invoke(
        &fixture,
        run_id,
        Some(json!({"plan_revision": "plan-r1", "task_roots": ["missing"]})),
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "invalid selection admitted: {value}"
    );
    assert_eq!(
        value["status"], "error",
        "invalid selection response: {value}"
    );
    assert!(
        value["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown task"),
        "facade refusal did not preserve the selection error: {value}"
    );
    assert!(
        load_invocations(&fixture, run_id).is_empty(),
        "invalid facade preparation created an invocation"
    );
    assert!(
        !fixture.artifact_root.join("work-slot-captures").exists(),
        "invalid facade preparation admitted a capture directory"
    );
    assert!(
        !fixture.receipt_dir.join("a.count").exists()
            && !fixture.receipt_dir.join("b.count").exists()
            && !fixture.receipt_dir.join("c.count").exists()
            && !fixture.receipt_dir.join("summarizer.count").exists(),
        "invalid facade preparation started a task worker"
    );
}

fn read_count(receipt_dir: &Path, task_id: &str) -> u32 {
    fs::read_to_string(receipt_dir.join(format!("{task_id}.count")))
        .unwrap_or_else(|error| panic!("missing {task_id} count: {error}"))
        .parse()
        .unwrap_or_else(|error| panic!("invalid {task_id} count: {error}"))
}
