//! Catalog-reading `invocation-progress` command.
//!
//! Opens the run catalog, selects one invocation, and prints a JSON snapshot of
//! that invocation's capture_dir graph liveness and already-associated traces.
//! Show remains the overlay authority. This command does not write overlay or
//! invocation status.

use crate::dagu::{locator_path, resolve_dagu, DaguLocator};
use loop_core::{
    project_invocation_status, InvocationId, OperationOutcome, Persistence,
    ProjectedInvocationStatus, RunId, Timestamp, WorkSlotId, WorkSlotInvocation,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{
    now_timestamp, open_persistence, render_operation, render_operation_error, resolve_paths,
    CliError, CliOptions, Execution, DEFAULT_PROVIDER_TIMEOUT,
};

const OPERATION: &str = "invocation-progress";
const STEP_NAME_PREFIX: &str = "  - name: ";

/// Caller-facing progress snapshot. Overlay fields stay on `show`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InvocationProgressSnapshot {
    pub run_id: RunId,
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub capture_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<GraphProgress>,
    pub traces: Vec<ProgressTrace>,
}

/// Locator-backed per-step helper liveness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphProgress {
    pub locator: DaguLocator,
    pub steps: Vec<GraphStep>,
}

/// One inventoried Dagu step and its helper liveness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GraphStep {
    pub name: String,
    pub state: GraphStepState,
}

/// Dagu step-helper liveness. Reaped means the helper finished, not overlay
/// success and not inner waitpid 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphStepState {
    NotStarted,
    Running,
    Reaped,
}

/// An already-associated sidecar or session file. Bodies are not read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProgressTrace {
    pub path: String,
    pub kind: TraceKind,
    pub last_modified_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

/// How a named trace is associated with the invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    Sidecar,
    Session,
}

/// Open the catalog, select one invocation, and render a progress snapshot.
pub(crate) fn execute_invocation_progress(
    options: CliOptions,
    run_id: RunId,
    invocation_id: Option<InvocationId>,
) -> Execution {
    let output = options.output;
    let timeout = options.provider_timeout.unwrap_or(DEFAULT_PROVIDER_TIMEOUT);
    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return render_operation_error(OPERATION, output, error),
    };
    let persistence = match open_persistence(&paths.database) {
        Ok(persistence) => persistence,
        Err(error) => return render_operation_error(OPERATION, output, error),
    };
    match collect_snapshot(
        &persistence,
        &run_id,
        invocation_id.as_ref(),
        timeout,
        now_timestamp(),
        waiter_alive,
    ) {
        Ok(snapshot) => render_operation(OPERATION, output, &OperationOutcome::completed(snapshot)),
        Err(error) => render_operation_error(OPERATION, output, error),
    }
}

pub(crate) fn collect_snapshot_for_invocation<P>(
    persistence: &P,
    run_id: &RunId,
    invocation_id: &InvocationId,
    timeout: Duration,
    now: Timestamp,
) -> Result<InvocationProgressSnapshot, CliError>
where
    P: Persistence + ?Sized,
{
    collect_snapshot(
        persistence,
        run_id,
        Some(invocation_id),
        timeout,
        now,
        waiter_alive,
    )
}

fn collect_snapshot<P, F>(
    persistence: &P,
    run_id: &RunId,
    invocation_id: Option<&InvocationId>,
    timeout: Duration,
    now: Timestamp,
    waiter_alive: F,
) -> Result<InvocationProgressSnapshot, CliError>
where
    P: Persistence + ?Sized,
    F: Fn(u32) -> bool,
{
    let invocations = persistence
        .load_work_slot_invocations(run_id)
        .map_err(|error| CliError::new(error.code(), error.to_string()))?;
    let selected = select_invocation(&invocations, invocation_id, now, waiter_alive)?;
    let capture_dir = selected.capture_dir.clone();
    if capture_dir.is_empty() || !Path::new(&capture_dir).is_dir() {
        return Err(CliError::new(
            "capture-dir-missing",
            format!("capture directory `{capture_dir}` is missing"),
        ));
    }
    let capture_path = Path::new(&capture_dir);
    let graph = read_graph(capture_path, timeout)?;
    let traces = enumerate_traces(capture_path)?;
    Ok(InvocationProgressSnapshot {
        run_id: run_id.clone(),
        invocation_id: selected.invocation_id.clone(),
        slot_id: selected.slot_id.clone(),
        capture_dir,
        graph,
        traces,
    })
}

fn select_invocation<'a, F>(
    invocations: &'a [WorkSlotInvocation],
    explicit: Option<&InvocationId>,
    now: Timestamp,
    waiter_alive: F,
) -> Result<&'a WorkSlotInvocation, CliError>
where
    F: Fn(u32) -> bool,
{
    if invocations.is_empty() {
        return Err(CliError::new(
            "no-invocations",
            "run has no work-slot invocations",
        ));
    }
    if let Some(invocation_id) = explicit {
        return invocations
            .iter()
            .find(|invocation| invocation.invocation_id == *invocation_id)
            .ok_or_else(|| {
                CliError::new(
                    "invocation-not-found",
                    format!("invocation `{invocation_id}` was not found on the run"),
                )
            });
    }
    let running: Vec<&WorkSlotInvocation> = invocations
        .iter()
        .filter(|invocation| {
            project_invocation_status(invocation, now, waiter_alive(invocation.waiter_pid))
                == ProjectedInvocationStatus::Running
        })
        .collect();
    if running.len() == 1 {
        return Ok(running[0]);
    }
    invocations
        .iter()
        .max_by_key(|invocation| invocation.started_at)
        .ok_or_else(|| CliError::new("no-invocations", "run has no work-slot invocations"))
}

fn read_graph(capture_dir: &Path, timeout: Duration) -> Result<Option<GraphProgress>, CliError> {
    let Some(locator) = load_locator(capture_dir)? else {
        return Ok(None);
    };
    let inventory = inventory_steps(&locator)?;
    let dagu =
        resolve_dagu().map_err(|error| CliError::new("dagu-unavailable", error.to_string()))?;
    spawn_dagu_status(&dagu, &locator, timeout)?;
    Ok(Some(graph_progress(locator, inventory)))
}

fn graph_progress(locator: DaguLocator, inventory: Vec<String>) -> GraphProgress {
    let nodes = node_states_from_status_jsonl(Path::new(&locator.dagu_home));
    let steps = inventory
        .into_iter()
        .map(|name| GraphStep {
            state: nodes
                .get(&name)
                .copied()
                .unwrap_or(GraphStepState::NotStarted),
            name,
        })
        .collect();
    GraphProgress { locator, steps }
}

fn load_locator(capture_dir: &Path) -> Result<Option<DaguLocator>, CliError> {
    let path = locator_path(capture_dir);
    match fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CliError::new(
            "malformed-dagu-locator",
            format!("could not read dagu locator `{}`: {error}", path.display()),
        )),
        Ok(bytes) => parse_strict_locator(&bytes).map(Some),
    }
}

fn parse_strict_locator(bytes: &[u8]) -> Result<DaguLocator, CliError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CliError::new(
            "malformed-dagu-locator",
            format!("dagu-locator.json is not valid JSON: {error}"),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        CliError::new(
            "malformed-dagu-locator",
            "dagu-locator.json must be an object with dagu_home, dag_name, and run_name",
        )
    })?;
    if object.len() != 3
        || !object.contains_key("dagu_home")
        || !object.contains_key("dag_name")
        || !object.contains_key("run_name")
    {
        return Err(CliError::new(
            "malformed-dagu-locator",
            "dagu-locator.json must have exactly the three non-empty string keys dagu_home, dag_name, and run_name",
        ));
    }
    let dagu_home = required_locator_string(object, "dagu_home")?;
    let dag_name = required_locator_string(object, "dag_name")?;
    let run_name = required_locator_string(object, "run_name")?;
    Ok(DaguLocator {
        dagu_home,
        dag_name,
        run_name,
    })
}

fn required_locator_string(object: &Map<String, Value>, key: &str) -> Result<String, CliError> {
    let value = object.get(key).ok_or_else(|| {
        CliError::new(
            "malformed-dagu-locator",
            format!("dagu-locator.json is missing `{key}`"),
        )
    })?;
    let Some(text) = value.as_str() else {
        return Err(CliError::new(
            "malformed-dagu-locator",
            format!("dagu-locator.json key `{key}` must be a string"),
        ));
    };
    if text.is_empty() {
        return Err(CliError::new(
            "malformed-dagu-locator",
            format!("dagu-locator.json key `{key}` must be a non-empty string"),
        ));
    }
    Ok(text.to_owned())
}

fn inventory_steps(locator: &DaguLocator) -> Result<Vec<String>, CliError> {
    let yaml_path = Path::new(&locator.dagu_home)
        .join("dags")
        .join(format!("{}.yaml", locator.dag_name));
    let yaml = fs::read_to_string(&yaml_path).map_err(|error| {
        CliError::new(
            "dagu-graph-unavailable",
            format!(
                "could not read emitted graph `{}`: {error}",
                yaml_path.display()
            ),
        )
    })?;
    Ok(inventory_step_names(&yaml))
}

fn inventory_step_names(yaml: &str) -> Vec<String> {
    yaml.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(STEP_NAME_PREFIX)?;
            parse_yaml_double_quoted(rest.trim_end())
        })
        .collect()
}

fn parse_yaml_double_quoted(raw: &str) -> Option<String> {
    let mut chars = raw.chars();
    if chars.next() != Some('"') {
        return None;
    }
    let mut out = String::new();
    let mut escape = false;
    for ch in chars {
        if escape {
            out.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

fn spawn_dagu_status(
    dagu: &Path,
    locator: &DaguLocator,
    timeout: Duration,
) -> Result<(), CliError> {
    run_bounded_command(
        dagu,
        &[
            "status",
            "--dagu-home",
            &locator.dagu_home,
            "--run-id",
            &locator.run_name,
            &locator.dag_name,
        ],
        timeout,
    )
}

fn run_bounded_command(command: &Path, args: &[&str], timeout: Duration) -> Result<(), CliError> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            CliError::new(
                "dagu-helper-failed",
                format!(
                    "could not spawn `{} {}`: {error}",
                    command.display(),
                    args.join(" ")
                ),
            )
        })?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                let detail = dagu_failure_detail(status.code(), stderr.trim());
                return Err(CliError::new(
                    "dagu-helper-failed",
                    format!("dagu status failed{detail}"),
                ));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CliError::new(
                        "dagu-helper-timeout",
                        format!("dagu status exceeded timeout of {}ms", timeout.as_millis()),
                    ));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                return Err(CliError::new(
                    "dagu-helper-failed",
                    format!("could not wait for dagu status: {error}"),
                ));
            }
        }
    }
}

fn dagu_failure_detail(code: Option<i32>, stderr: &str) -> String {
    if !stderr.is_empty() {
        return format!(": {stderr}");
    }
    if let Some(code) = code {
        format!(" (exit {code})")
    } else {
        " (terminated by signal)".to_owned()
    }
}

fn node_states_from_status_jsonl(dagu_home: &Path) -> HashMap<String, GraphStepState> {
    let Some(status_path) = latest_status_jsonl(dagu_home) else {
        return HashMap::new();
    };
    let Ok(raw) = fs::read_to_string(&status_path) else {
        return HashMap::new();
    };
    let Some(line) = raw.lines().rev().find(|line| !line.trim().is_empty()) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return HashMap::new();
    };
    let Some(nodes) = value.get("nodes").and_then(Value::as_array) else {
        return HashMap::new();
    };
    let mut states = HashMap::new();
    for node in nodes {
        let name = node
            .get("step")
            .and_then(|step| step.get("name"))
            .and_then(Value::as_str)
            .or_else(|| node.get("name").and_then(Value::as_str));
        let Some(name) = name else {
            continue;
        };
        let status = node.get("status").and_then(Value::as_u64);
        let started_at = node.get("startedAt").and_then(Value::as_str).unwrap_or("");
        states.insert(name.to_owned(), graph_state_from_node(status, started_at));
    }
    states
}

fn latest_status_jsonl(dagu_home: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let mut stack = vec![dagu_home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().is_some_and(|name| name == "status.jsonl") {
                let Ok(mtime) = entry.metadata().and_then(|meta| meta.modified()) else {
                    continue;
                };
                if best.as_ref().is_none_or(|(time, _)| mtime >= *time) {
                    best = Some((mtime, path));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

fn graph_state_from_node(status: Option<u64>, started_at: &str) -> GraphStepState {
    let status = status.unwrap_or(0);
    if matches!(status, 2..=4) {
        return GraphStepState::Reaped;
    }
    if status == 5 {
        return if started_at.is_empty() {
            GraphStepState::NotStarted
        } else {
            GraphStepState::Reaped
        };
    }
    if status == 1 || !started_at.is_empty() {
        GraphStepState::Running
    } else {
        GraphStepState::NotStarted
    }
}

fn enumerate_traces(capture_dir: &Path) -> Result<Vec<ProgressTrace>, CliError> {
    let mut worker_dirs = Vec::new();
    let entries = fs::read_dir(capture_dir).map_err(|error| {
        CliError::new(
            "capture-dir-missing",
            format!(
                "could not read capture directory `{}`: {error}",
                capture_dir.display()
            ),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CliError::new(
                "capture-dir-missing",
                format!(
                    "could not read capture directory `{}`: {error}",
                    capture_dir.display()
                ),
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            worker_dirs.push(path);
        }
    }
    worker_dirs.sort();
    let mut traces = Vec::new();
    for worker_dir in worker_dirs {
        let Some(dir_name) = worker_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let step = worker_step_name(dir_name);
        let sidecar = worker_dir.join("inner_exit.json");
        if sidecar.is_file() {
            traces.push(ProgressTrace {
                path: path_to_string(&sidecar),
                kind: TraceKind::Sidecar,
                last_modified_ms: unix_mtime_ms(&sidecar)?,
                step: Some(step.clone()),
            });
        }
        let sessions = worker_dir.join("sessions");
        if !sessions.is_dir() {
            continue;
        }
        let mut session_files = Vec::new();
        let session_entries = fs::read_dir(&sessions).map_err(|error| {
            CliError::new(
                "capture-dir-missing",
                format!(
                    "could not read sessions directory `{}`: {error}",
                    sessions.display()
                ),
            )
        })?;
        for entry in session_entries {
            let entry = entry.map_err(|error| {
                CliError::new(
                    "capture-dir-missing",
                    format!(
                        "could not read sessions directory `{}`: {error}",
                        sessions.display()
                    ),
                )
            })?;
            let path = entry.path();
            if path.is_file() {
                session_files.push(path);
            }
        }
        session_files.sort();
        for path in session_files {
            traces.push(ProgressTrace {
                path: path_to_string(&path),
                kind: TraceKind::Session,
                last_modified_ms: unix_mtime_ms(&path)?,
                step: Some(step.clone()),
            });
        }
    }
    Ok(traces)
}

fn worker_step_name(dir_name: &str) -> String {
    if let Ok(index) = dir_name.parse::<u64>() {
        if index.to_string() == dir_name {
            return format!("w{index}");
        }
    }
    dir_name.to_owned()
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn unix_mtime_ms(path: &Path) -> Result<u64, CliError> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::new(
            "capture-dir-missing",
            format!("could not stat `{}`: {error}", path.display()),
        )
    })?;
    let modified = metadata.modified().map_err(|error| {
        CliError::new(
            "capture-dir-missing",
            format!("could not read mtime of `{}`: {error}", path.display()),
        )
    })?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64)
}

fn waiter_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { unix_signal::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
mod unix_signal {
    extern "C" {
        pub fn kill(pid: i32, sig: i32) -> i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{execute, EXIT_COMPLETED, EXIT_ERROR};
    use loop_core::{
        CreateRunRequest, CreateWorkSlotInvocationRequest, Lifecycle, ProviderAssociation, State,
        Transition, WaiterWrittenStatus, WorkSlotBinding, Workflow,
    };
    use loop_integrations::SqlitePersistence;
    use serde_json::json;
    use std::fs;

    fn sample_invocation(
        id: &str,
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        waiter_pid: u32,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            id,
            "slot-1",
            WorkSlotBinding::new("echo", vec!["ok".to_owned()]),
            "digest",
            "subject",
            waiter_pid,
            Timestamp::from_unix_millis(started_at),
            allowed_time_ms,
            status,
            None,
            None,
            "/tmp/capture",
            Vec::new(),
        )
    }

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![State::new("start", "Start", "Do the work", false)],
            vec![Transition::check_free("start", "finish", "start")],
        )
    }

    fn seed_run(
        persistence: &SqlitePersistence,
        run_id: &str,
        invocation_id: &str,
        started_at: i64,
        waiter_pid: u32,
        capture_dir: &str,
    ) {
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

    #[test]
    fn snapshot_serde_omits_overlay_and_inner_workers() {
        let snapshot = InvocationProgressSnapshot {
            run_id: "run-1".into(),
            invocation_id: "inv-1".into(),
            slot_id: "slot-1".into(),
            capture_dir: "/tmp/capture".to_owned(),
            graph: None,
            traces: Vec::new(),
        };
        let value = serde_json::to_value(&snapshot).expect("serialize snapshot");
        let object = value.as_object().expect("object");
        assert!(object.contains_key("run_id"));
        assert!(object.contains_key("invocation_id"));
        assert!(object.contains_key("slot_id"));
        assert!(object.contains_key("capture_dir"));
        assert!(object.contains_key("traces"));
        assert!(!object.contains_key("graph"));
        assert!(!object.contains_key("overlay"));
        assert!(!object.contains_key("overlay_meaning"));
        assert!(!object.contains_key("elapsed_ms"));
        assert!(!object.contains_key("remaining_allowed_ms"));
        assert!(!object.contains_key("inner_workers"));
        assert_eq!(object.get("traces"), Some(&json!([])));
    }

    #[test]
    fn graph_omitted_when_locator_is_absent() {
        let root = tempfile::tempdir().expect("tempdir");
        let capture = root.path().join("capture");
        fs::create_dir_all(&capture).expect("capture dir");
        let graph = load_locator(&capture).expect("locator read");
        assert!(graph.is_none());
        let traces = enumerate_traces(&capture).expect("traces");
        assert!(traces.is_empty());
    }

    #[test]
    fn traces_name_existing_sidecar_and_session_files_without_reading_bodies() {
        let root = tempfile::tempdir().expect("tempdir");
        let capture = root.path().join("capture");
        let worker = capture.join("0");
        let sessions = worker.join("sessions");
        fs::create_dir_all(&sessions).expect("sessions");
        let sidecar = worker.join("inner_exit.json");
        let session = sessions.join("session.bin");
        fs::write(&sidecar, "not-json-and-must-not-be-parsed").expect("sidecar");
        fs::write(&session, "not-a-pi-session-body").expect("session");
        fs::write(worker.join("stdout"), "worker stdout must be ignored").expect("stdout");
        fs::write(worker.join("stderr"), "worker stderr must be ignored").expect("stderr");
        let named = capture.join("summarizer");
        fs::create_dir_all(&named).expect("named worker");
        fs::write(named.join("inner_exit.json"), "{}").expect("named sidecar");

        let traces = enumerate_traces(&capture).expect("traces");
        assert_eq!(traces.len(), 3);
        assert_eq!(traces[0].kind, TraceKind::Sidecar);
        assert_eq!(traces[0].step.as_deref(), Some("w0"));
        assert!(traces[0].path.ends_with("inner_exit.json"));
        assert_eq!(traces[1].kind, TraceKind::Session);
        assert_eq!(traces[1].step.as_deref(), Some("w0"));
        assert!(traces[1].path.ends_with("session.bin"));
        assert_eq!(traces[2].kind, TraceKind::Sidecar);
        assert_eq!(traces[2].step.as_deref(), Some("summarizer"));
        assert!(traces.iter().all(|trace| trace.last_modified_ms > 0));
        assert!(traces.iter().all(|trace| !trace.path.ends_with("stdout")));
        assert!(traces.iter().all(|trace| !trace.path.ends_with("stderr")));
    }

    #[test]
    fn selection_prefers_unique_running_then_latest_started_at() {
        let now = Timestamp::from_unix_millis(2_000);
        let running = sample_invocation("inv-running", 1_000, 5_000, None, 1);
        let later_done = sample_invocation(
            "inv-later",
            9_000,
            5_000,
            Some(WaiterWrittenStatus::Succeeded),
            2,
        );
        let waiter = |pid: u32| pid == 1;
        let pair = [running.clone(), later_done.clone()];
        let selected = select_invocation(&pair, None, now, waiter).expect("unique running");
        assert_eq!(selected.invocation_id.as_str(), "inv-running");

        let older = sample_invocation(
            "inv-old",
            1_000,
            5_000,
            Some(WaiterWrittenStatus::Failed),
            3,
        );
        let newer = sample_invocation(
            "inv-new",
            8_000,
            5_000,
            Some(WaiterWrittenStatus::Succeeded),
            4,
        );
        let completed = [older, newer];
        let selected =
            select_invocation(&completed, None, now, |_| false).expect("latest started_at");
        assert_eq!(selected.invocation_id.as_str(), "inv-new");

        let first = sample_invocation("inv-a", 1_000, 5_000, None, 1);
        let second = sample_invocation("inv-b", 1_500, 5_000, None, 1);
        let both_running = [first, second];
        let selected = select_invocation(&both_running, None, now, |_| true)
            .expect("two running fall back to latest");
        assert_eq!(selected.invocation_id.as_str(), "inv-b");

        let explicit_id = InvocationId::from("inv-later");
        let selected =
            select_invocation(&pair, Some(&explicit_id), now, waiter).expect("explicit id");
        assert_eq!(selected.invocation_id.as_str(), "inv-later");
    }

    #[test]
    fn selection_errors_for_empty_and_unknown_id() {
        let now = Timestamp::from_unix_millis(1);
        let empty = select_invocation(&[], None, now, |_| false).expect_err("empty");
        assert_eq!(empty.code, "no-invocations");
        let record = sample_invocation("inv-1", 1, 5_000, None, 1);
        let missing = select_invocation(&[record], Some(&"missing".into()), now, |_| false)
            .expect_err("unknown id");
        assert_eq!(missing.code, "invocation-not-found");
    }

    #[test]
    fn malformed_locator_errors_without_inventing_graph() {
        let extra = json!({
            "dagu_home": "/tmp/home",
            "dag_name": "dag",
            "run_name": "run",
            "extra": "nope"
        });
        let error = parse_strict_locator(extra.to_string().as_bytes()).expect_err("extra key");
        assert_eq!(error.code, "malformed-dagu-locator");

        let empty = json!({
            "dagu_home": "/tmp/home",
            "dag_name": "",
            "run_name": "run"
        });
        let error = parse_strict_locator(empty.to_string().as_bytes()).expect_err("empty");
        assert_eq!(error.code, "malformed-dagu-locator");

        let missing = json!({
            "dagu_home": "/tmp/home",
            "dag_name": "dag"
        });
        let error = parse_strict_locator(missing.to_string().as_bytes()).expect_err("two keys");
        assert_eq!(error.code, "malformed-dagu-locator");

        let valid = json!({
            "dagu_home": "/tmp/home",
            "dag_name": "dag",
            "run_name": "run"
        });
        let locator = parse_strict_locator(valid.to_string().as_bytes()).expect("valid locator");
        assert_eq!(locator.dag_name, "dag");
    }

    #[test]
    fn inventory_reads_emitted_name_lines() {
        let yaml = concat!(
            "type: graph\n",
            "steps:\n",
            "  - name: \"w0\"\n",
            "    action: exec\n",
            "    depends:\n",
            "      - \"ignored\"\n",
            "  - name: \"join\"\n",
            "  - name: \"summarizer\"\n",
        );
        assert_eq!(
            inventory_step_names(yaml),
            vec!["w0".to_owned(), "join".to_owned(), "summarizer".to_owned()]
        );
    }

    #[test]
    fn graph_state_maps_dagu_node_integers() {
        assert_eq!(graph_state_from_node(None, ""), GraphStepState::NotStarted);
        assert_eq!(
            graph_state_from_node(Some(0), ""),
            GraphStepState::NotStarted
        );
        assert_eq!(
            graph_state_from_node(Some(5), ""),
            GraphStepState::NotStarted
        );
        assert_eq!(graph_state_from_node(Some(1), ""), GraphStepState::Running);
        assert_eq!(
            graph_state_from_node(Some(0), "2024-01-01T00:00:00Z"),
            GraphStepState::Running
        );
        assert_eq!(graph_state_from_node(Some(2), ""), GraphStepState::Reaped);
        assert_eq!(graph_state_from_node(Some(3), ""), GraphStepState::Reaped);
        assert_eq!(graph_state_from_node(Some(4), ""), GraphStepState::Reaped);
        assert_eq!(
            graph_state_from_node(Some(5), "2024-01-01T00:00:00Z"),
            GraphStepState::Reaped
        );
    }

    #[test]
    fn json_progress_without_locator_yields_capture_dir_and_does_not_write_status() {
        let root = tempfile::tempdir().expect("tempdir");
        let database = root.path().join("loop.db");
        let capture = root.path().join("capture");
        fs::create_dir_all(&capture).expect("capture");
        let persistence = SqlitePersistence::open(&database).expect("open sqlite");
        seed_run(
            &persistence,
            "run-progress",
            "inv-progress",
            1_000,
            1,
            capture.to_str().expect("utf8 capture"),
        );

        let execution = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "invocation-progress".to_owned(),
            "run-progress".to_owned(),
        ]);
        assert_eq!(execution.exit_code, EXIT_COMPLETED, "{}", execution.stdout);
        let payload: Value = serde_json::from_str(&execution.stdout).expect("json envelope");
        assert_eq!(payload["operation"], "invocation-progress");
        assert_eq!(payload["status"], "completed");
        let result = payload["result"].as_object().expect("result object");
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
        assert!(!result.contains_key("overlay_meaning"));
        assert!(!result.contains_key("elapsed_ms"));
        assert!(!result.contains_key("remaining_allowed_ms"));
        assert!(!result.contains_key("inner_workers"));

        let loaded = persistence
            .load_work_slot_invocations(&"run-progress".into())
            .expect("reload invocations");
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].status.is_none());
        assert!(loaded[0].inner_workers.is_empty());
    }

    #[test]
    fn unknown_run_no_invocations_and_unknown_id_are_errors() {
        let root = tempfile::tempdir().expect("tempdir");
        let database = root.path().join("loop.db");
        let persistence = SqlitePersistence::open(&database).expect("open sqlite");
        persistence
            .create_run(CreateRunRequest::new(
                "empty-run",
                None,
                workflow(),
                ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
                json!({}),
                "start",
                Lifecycle::Active,
                Timestamp::from_unix_millis(1),
                "test-provider",
                Some("/allocated/run-dir".to_owned()),
            ))
            .expect("create empty run");
        let capture = root.path().join("capture");
        fs::create_dir_all(&capture).expect("capture");
        seed_run(
            &persistence,
            "run-with-inv",
            "inv-known",
            1_000,
            1,
            capture.to_str().expect("utf8"),
        );
        let db = database.to_string_lossy().into_owned();

        let unknown_run = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            db.clone(),
            "invocation-progress".to_owned(),
            "missing-run".to_owned(),
        ]);
        assert_eq!(unknown_run.exit_code, EXIT_ERROR);
        let payload: Value = serde_json::from_str(&unknown_run.stdout).expect("json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "run-not-found");

        let no_invocations = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            db.clone(),
            "invocation-progress".to_owned(),
            "empty-run".to_owned(),
        ]);
        assert_eq!(no_invocations.exit_code, EXIT_ERROR);
        let payload: Value = serde_json::from_str(&no_invocations.stdout).expect("json");
        assert_eq!(payload["code"], "no-invocations");

        let unknown_id = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            db,
            "invocation-progress".to_owned(),
            "run-with-inv".to_owned(),
            "missing-inv".to_owned(),
        ]);
        assert_eq!(unknown_id.exit_code, EXIT_ERROR);
        let payload: Value = serde_json::from_str(&unknown_id.stdout).expect("json");
        assert_eq!(payload["code"], "invocation-not-found");

        let loaded = persistence
            .load_work_slot_invocations(&"run-with-inv".into())
            .expect("reload");
        assert!(loaded[0].status.is_none());
    }

    #[test]
    fn malformed_locator_fails_the_command_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let database = root.path().join("loop.db");
        let capture = root.path().join("capture");
        fs::create_dir_all(&capture).expect("capture");
        fs::write(
            capture.join("dagu-locator.json"),
            r#"{"dagu_home":"/tmp/home","dag_name":"dag"}"#,
        )
        .expect("malformed locator");
        let persistence = SqlitePersistence::open(&database).expect("open sqlite");
        seed_run(
            &persistence,
            "run-bad-locator",
            "inv-1",
            1_000,
            1,
            capture.to_str().expect("utf8"),
        );

        let execution = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "invocation-progress".to_owned(),
            "run-bad-locator".to_owned(),
        ]);
        assert_eq!(execution.exit_code, EXIT_ERROR);
        let payload: Value = serde_json::from_str(&execution.stdout).expect("json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "malformed-dagu-locator");
        let loaded = persistence
            .load_work_slot_invocations(&"run-bad-locator".into())
            .expect("reload");
        assert!(loaded[0].status.is_none());
    }

    #[test]
    fn deleted_capture_dir_fails_the_command_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let database = root.path().join("loop.db");
        let capture = root.path().join("capture");
        fs::create_dir_all(&capture).expect("capture");
        let persistence = SqlitePersistence::open(&database).expect("open sqlite");
        seed_run(
            &persistence,
            "run-deleted",
            "inv-1",
            1_000,
            1,
            capture.to_str().expect("utf8"),
        );
        fs::remove_dir_all(&capture).expect("delete capture");

        let execution = execute([
            "--json".to_owned(),
            "--database".to_owned(),
            database.to_string_lossy().into_owned(),
            "invocation-progress".to_owned(),
            "run-deleted".to_owned(),
        ]);
        assert_eq!(execution.exit_code, EXIT_ERROR);
        let payload: Value = serde_json::from_str(&execution.stdout).expect("json");
        assert_eq!(payload["code"], "capture-dir-missing");
        let loaded = persistence
            .load_work_slot_invocations(&"run-deleted".into())
            .expect("reload");
        assert!(loaded[0].status.is_none());
    }

    #[test]
    fn status_jsonl_maps_latest_line_and_missing_inventory_steps_are_not_started() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("dagu-home");
        let nested = home.join("data").join("runs").join("one");
        fs::create_dir_all(&nested).expect("status dir");
        fs::write(
            nested.join("status.jsonl"),
            concat!(
                r#"{"nodes":[{"status":4,"step":{"name":"stale"}}]}"#, 
                "\n",
                r#"{"nodes":[{"status":1,"step":{"name":"w0"},"startedAt":"t"},{"status":4,"name":"w1","startedAt":"t"},{"status":5,"step":{"name":"join"}}]}"#, 
                "\n",
            ),
        )
        .expect("status.jsonl");

        let locator = DaguLocator {
            dagu_home: home.to_string_lossy().into_owned(),
            dag_name: "fanout-test".to_owned(),
            run_name: "fanout-test".to_owned(),
        };
        let graph = graph_progress(
            locator,
            vec![
                "w0".to_owned(),
                "w1".to_owned(),
                "join".to_owned(),
                "summarizer".to_owned(),
            ],
        );
        assert_eq!(graph.steps.len(), 4);
        assert_eq!(graph.steps[0].name, "w0");
        assert_eq!(graph.steps[0].state, GraphStepState::Running);
        assert_eq!(graph.steps[1].name, "w1");
        assert_eq!(graph.steps[1].state, GraphStepState::Reaped);
        assert_eq!(graph.steps[2].name, "join");
        assert_eq!(graph.steps[2].state, GraphStepState::NotStarted);
        assert_eq!(graph.steps[3].name, "summarizer");
        assert_eq!(graph.steps[3].state, GraphStepState::NotStarted);
        assert!(graph.steps.iter().all(|step| step.name != "stale"));
    }

    #[test]
    fn bounded_helper_timeout_fails_without_parsing_stdout() {
        let error = run_bounded_command(Path::new("/bin/sleep"), &["2"], Duration::from_millis(50))
            .expect_err("timeout");
        assert_eq!(error.code, "dagu-helper-timeout");

        run_bounded_command(Path::new("/usr/bin/true"), &[], Duration::from_secs(1))
            .or_else(|_| run_bounded_command(Path::new("/bin/true"), &[], Duration::from_secs(1)))
            .expect("true should succeed");
    }
}
