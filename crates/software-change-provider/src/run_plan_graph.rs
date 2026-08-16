//! Argv and stdin contracts plus the DAG executor for `run-plan-graph`.

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// Hard cap on concurrent inner-worker processes for `run-plan-graph`.
pub(crate) const MAX_CONCURRENCY: usize = 4;

const DEFAULT_WORKER_COMMAND: &str = "pi";
const DEFAULT_WORKER_ARG: &str = "--print";

/// Worker argv: JSON object with exactly string `command` and array-of-string `args`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerCli {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

/// Bound-worker stdin packet.  Exactly the four engine invoke keys; no extras.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvokePacket {
    pub(crate) run_id: String,
    pub(crate) slot_id: String,
    pub(crate) artifact_root: String,
    pub(crate) instruction_body: String,
}

/// Parse failure for worker JSON, invoke packets, or `run-plan-graph` argv.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

enum ExecuteError {
    Usage(String),
    Failed(String),
}

impl ExecuteError {
    fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl fmt::Display for ExecuteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Default inner worker when `--task-worker` is omitted: `pi --print`.
pub(crate) fn default_task_worker() -> WorkerCli {
    WorkerCli {
        command: DEFAULT_WORKER_COMMAND.to_owned(),
        args: vec![DEFAULT_WORKER_ARG.to_owned()],
    }
}

/// Parse one worker CLI JSON object (`{command, args}`).
pub(crate) fn parse_worker_cli_json(raw: &str) -> Result<WorkerCli, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "worker CLI JSON must be an object with exactly string `command` and array-of-string `args`: {error}"
        ))
    })
}

/// Parse the four-key engine invoke packet from stdin JSON.
pub(crate) fn parse_invoke_packet(raw: &str) -> Result<InvokePacket, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "invoke packet must be a JSON object with exactly `run_id`, `slot_id`, `artifact_root`, and `instruction_body`: {error}"
        ))
    })
}

/// Parse argv tokens after the `run-plan-graph` command name.
///
/// Optional once: `--task-worker JSON`.  Omitted flag yields [`default_task_worker`].
/// Unknown flags, leftover positionals, and repeated `--task-worker` are errors.
/// There is no `--max-concurrency` flag; concurrency is [`MAX_CONCURRENCY`].
pub(crate) fn parse_run_plan_graph_args<I, S>(args: I) -> Result<WorkerCli, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|token| token.as_ref().to_owned())
        .collect::<Vec<String>>();
    let mut worker = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if let Some(raw) = strip_option(token, "--task-worker") {
            if worker.is_some() {
                return Err(ParseError::new(
                    "`--task-worker` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--task-worker")?.to_owned(),
            };
            worker = Some(parse_worker_cli_json(&raw)?);
            continue;
        }
        if token.starts_with('-') {
            return Err(ParseError::new(format!("unknown option `{token}`")));
        }
        return Err(ParseError::new(format!(
            "unexpected argument `{token}` for run-plan-graph"
        )));
    }
    Ok(worker.unwrap_or_else(default_task_worker))
}

fn strip_option(token: &str, option: &str) -> Option<Option<String>> {
    if token == option {
        return Some(None);
    }
    let prefix = format!("{option}=");
    token.strip_prefix(&prefix).map(|raw| Some(raw.to_owned()))
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, ParseError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| ParseError::new(format!("option `{option}` requires a value")))?;
    if value.starts_with('-') && value != "-" {
        return Err(ParseError::new(format!(
            "option `{option}` requires a value"
        )));
    }
    *index += 1;
    Ok(value)
}

/// Read the invoke packet from stdin, schedule `plan.json`, and reap every spawned worker.
pub(crate) fn execute(worker: &WorkerCli) -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("could not read invoke packet: {error}");
        return 2;
    }
    match execute_from_packet(worker, &input) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            error.exit_code()
        }
    }
}

#[derive(Deserialize)]
struct PlanDocument {
    tasks: Vec<Value>,
    dependency_graph: Vec<DependencyEdge>,
}

#[derive(Deserialize)]
struct DependencyEdge {
    from: String,
    to: String,
}

struct PlanGraph {
    order: Vec<String>,
    tasks: HashMap<String, Value>,
    predecessors: HashMap<String, HashSet<String>>,
}

struct RunningTask {
    id: String,
    child: Child,
}

fn execute_from_packet(worker: &WorkerCli, raw_packet: &str) -> Result<(), ExecuteError> {
    let packet =
        parse_invoke_packet(raw_packet).map_err(|error| ExecuteError::usage(error.to_string()))?;
    let artifact_root = absolute_artifact_root(&packet.artifact_root)?;
    let plan_path = artifact_root.join("plan.json");
    let plan_raw = fs::read_to_string(&plan_path).map_err(|error| {
        ExecuteError::failed(format!("could not read {}: {error}", plan_path.display()))
    })?;
    let plan = parse_plan(&plan_raw)?;
    run_schedule(worker, &packet, &artifact_root, &plan)
}

fn absolute_artifact_root(raw: &str) -> Result<PathBuf, ExecuteError> {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        let cwd = std::env::current_dir().map_err(|error| {
            ExecuteError::failed(format!("could not read current directory: {error}"))
        })?;
        Ok(cwd.join(path))
    }
}

fn parse_plan(raw: &str) -> Result<PlanGraph, ExecuteError> {
    let document: PlanDocument = serde_json::from_str(raw).map_err(|error| {
        ExecuteError::failed(format!("plan.json is not a valid plan document: {error}"))
    })?;
    let mut order = Vec::new();
    let mut tasks = HashMap::new();
    let mut ids = HashSet::new();
    for task in document.tasks {
        let id = task
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ExecuteError::failed("plan.json task is missing a string `id`"))?
            .to_owned();
        if !is_safe_task_id(&id) {
            return Err(ExecuteError::failed(format!(
                "plan.json task id `{id}` is not a single path-safe component"
            )));
        }
        if !ids.insert(id.clone()) {
            return Err(ExecuteError::failed(format!(
                "plan.json has duplicate task id `{id}`"
            )));
        }
        order.push(id.clone());
        tasks.insert(id, task);
    }
    if order.is_empty() {
        return Err(ExecuteError::failed("plan.json has no tasks"));
    }

    let mut predecessors: HashMap<String, HashSet<String>> = order
        .iter()
        .cloned()
        .map(|id| (id, HashSet::new()))
        .collect();
    let mut successors: HashMap<String, Vec<String>> =
        order.iter().cloned().map(|id| (id, Vec::new())).collect();

    for edge in &document.dependency_graph {
        if !ids.contains(&edge.from) || !ids.contains(&edge.to) {
            return Err(ExecuteError::failed(format!(
                "dependency_graph edge names unknown task id (`{}` -> `{}`)",
                edge.from, edge.to
            )));
        }
        predecessors
            .get_mut(&edge.to)
            .expect("edge `to` is a known task id")
            .insert(edge.from.clone());
        successors
            .get_mut(&edge.from)
            .expect("edge `from` is a known task id")
            .push(edge.to.clone());
    }

    if has_cycle(&order, &successors) {
        return Err(ExecuteError::failed(
            "dependency_graph contains a cycle".to_owned(),
        ));
    }

    Ok(PlanGraph {
        order,
        tasks,
        predecessors,
    })
}

fn has_cycle(order: &[String], successors: &HashMap<String, Vec<String>>) -> bool {
    fn visit(
        node: &str,
        successors: &HashMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) -> bool {
        if visited.contains(node) {
            return false;
        }
        if !visiting.insert(node.to_owned()) {
            return true;
        }
        if let Some(nexts) = successors.get(node) {
            for next in nexts {
                if visiting.contains(next) || visit(next, successors, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(node);
        visited.insert(node.to_owned());
        false
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    order
        .iter()
        .any(|id| visit(id, successors, &mut visiting, &mut visited))
}

fn run_schedule(
    worker: &WorkerCli,
    packet: &InvokePacket,
    artifact_root: &Path,
    plan: &PlanGraph,
) -> Result<(), ExecuteError> {
    let mut pending: HashSet<String> = plan.order.iter().cloned().collect();
    let mut succeeded: HashSet<String> = HashSet::new();
    let mut running: Vec<RunningTask> = Vec::new();
    let mut failed = false;
    let mut failure_message = String::from("inner task worker failed");

    loop {
        let mut index = 0;
        while index < running.len() {
            match running[index].child.try_wait() {
                Ok(Some(status)) => {
                    let finished = running.swap_remove(index);
                    if status.success() {
                        succeeded.insert(finished.id);
                    } else {
                        failed = true;
                        failure_message = format!(
                            "inner task worker for `{}` exited unsuccessfully",
                            finished.id
                        );
                    }
                }
                Ok(None) => index += 1,
                Err(error) => {
                    let finished = running.swap_remove(index);
                    failed = true;
                    failure_message = format!(
                        "could not wait for inner task worker `{}`: {error}",
                        finished.id
                    );
                }
            }
        }

        if failed {
            if running.is_empty() {
                return Err(ExecuteError::failed(failure_message));
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        while running.len() < MAX_CONCURRENCY {
            let Some(id) = next_runnable(&plan.order, &pending, &succeeded, &plan.predecessors)
            else {
                break;
            };
            pending.remove(&id);
            let task = plan
                .tasks
                .get(&id)
                .expect("runnable task exists in the plan");
            match spawn_task(worker, packet, artifact_root, &id, task) {
                Ok(child) => running.push(RunningTask { id, child }),
                Err(error) => {
                    failed = true;
                    failure_message = error.to_string();
                    break;
                }
            }
        }

        if running.is_empty() {
            if failed {
                return Err(ExecuteError::failed(failure_message));
            }
            if succeeded.len() == plan.order.len() {
                let report = artifact_root.join("implementation-report.json");
                if !report.is_file() {
                    return Err(ExecuteError::failed(format!(
                        "missing {} after all tasks succeeded",
                        report.display()
                    )));
                }
                return Ok(());
            }
            return Err(ExecuteError::failed(
                "no runnable plan tasks remain before the graph is complete".to_owned(),
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn next_runnable(
    order: &[String],
    pending: &HashSet<String>,
    succeeded: &HashSet<String>,
    predecessors: &HashMap<String, HashSet<String>>,
) -> Option<String> {
    order.iter().find_map(|id| {
        if !pending.contains(id) {
            return None;
        }
        let ready = predecessors
            .get(id)
            .map(|preds| preds.iter().all(|pred| succeeded.contains(pred)))
            .unwrap_or(true);
        ready.then(|| id.clone())
    })
}

fn spawn_task(
    worker: &WorkerCli,
    packet: &InvokePacket,
    artifact_root: &Path,
    task_id: &str,
    task: &Value,
) -> Result<Child, ExecuteError> {
    let out_dir = task_capture_dir(artifact_root, task_id)?;
    fs::create_dir_all(&out_dir).map_err(|error| {
        ExecuteError::failed(format!("could not create {}: {error}", out_dir.display()))
    })?;
    let stdout_path = out_dir.join("stdout");
    let stderr_path = out_dir.join("stderr");
    let stdout = File::create(&stdout_path).map_err(|error| {
        ExecuteError::failed(format!(
            "could not create {}: {error}",
            stdout_path.display()
        ))
    })?;
    let stderr = File::create(&stderr_path).map_err(|error| {
        ExecuteError::failed(format!(
            "could not create {}: {error}",
            stderr_path.display()
        ))
    })?;
    let stdin_text = inner_stdin(packet, artifact_root, task)?;
    let mut child = Command::new(&worker.command)
        .args(&worker.args)
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| {
            ExecuteError::failed(format!(
                "could not spawn inner task worker `{}` for `{task_id}`: {error}",
                worker.command
            ))
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_text.as_bytes());
    }
    Ok(child)
}

fn is_safe_task_id(id: &str) -> bool {
    if id.is_empty() || id.contains('\0') || id.contains('/') || id.contains('\\') {
        return false;
    }
    let mut components = Path::new(id).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => name == std::ffi::OsStr::new(id),
        _ => false,
    }
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
            }
            Component::Normal(name) => out.push(name),
        }
    }
    out
}

fn task_capture_dir(artifact_root: &Path, task_id: &str) -> Result<PathBuf, ExecuteError> {
    if !is_safe_task_id(task_id) {
        return Err(ExecuteError::failed(format!(
            "plan.json task id `{task_id}` is not a single path-safe component"
        )));
    }
    let capture_root = artifact_root.join("run-plan-graph");
    let out_dir = capture_root.join(task_id);
    let capture_root = lexical_path(&capture_root);
    let out_dir_lex = lexical_path(&out_dir);
    if !out_dir_lex.starts_with(&capture_root) {
        return Err(ExecuteError::failed(format!(
            "task capture path for `{task_id}` escapes artifact_root"
        )));
    }
    Ok(out_dir)
}

fn inner_stdin(
    packet: &InvokePacket,
    artifact_root: &Path,
    task: &Value,
) -> Result<String, ExecuteError> {
    let task_json = serde_json::to_string(task).map_err(|error| {
        ExecuteError::failed(format!("could not serialize task record: {error}"))
    })?;
    Ok(format!(
        "run_id: {}\nslot_id: {}\nartifact_root: {}\n\n## instruction_body\n{}\n\n## task\n{}\n",
        packet.run_id,
        packet.slot_id,
        artifact_root.display(),
        packet.instruction_body,
        task_json
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_worker_json() -> &'static str {
        r#"{"command":"echo","args":["hello"]}"#
    }

    fn valid_packet_json() -> &'static str {
        r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work"}"#
    }

    #[test]
    fn valid_worker_json_parses_command_and_args() {
        let worker = parse_worker_cli_json(valid_worker_json()).expect("valid worker JSON");
        assert_eq!(worker.command, "echo");
        assert_eq!(worker.args, vec!["hello".to_owned()]);
    }

    #[test]
    fn worker_json_unknown_field_fails() {
        let raw = r#"{"command":"echo","args":["hello"],"extra":true}"#;
        assert!(parse_worker_cli_json(raw).is_err());
    }

    #[test]
    fn worker_json_non_object_fails() {
        assert!(parse_worker_cli_json(r#"["echo"]"#).is_err());
        assert!(parse_worker_cli_json("\"echo\"").is_err());
    }

    #[test]
    fn worker_json_non_string_command_fails() {
        assert!(parse_worker_cli_json(r#"{"command":1,"args":[]}"#).is_err());
    }

    #[test]
    fn worker_json_non_string_args_entry_fails() {
        assert!(parse_worker_cli_json(r#"{"command":"echo","args":[1]}"#).is_err());
    }

    #[test]
    fn valid_invoke_packet_parses_four_string_keys() {
        let packet = parse_invoke_packet(valid_packet_json()).expect("valid packet");
        assert_eq!(packet.run_id, "run-1");
        assert_eq!(packet.slot_id, "slot-1");
        assert_eq!(packet.artifact_root, "/tmp/artifacts");
        assert_eq!(packet.instruction_body, "Do the work");
    }

    #[test]
    fn invoke_packet_extra_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","extra":"no"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_missing_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn omitted_task_worker_yields_default_pi_print() {
        let worker = parse_run_plan_graph_args(&[] as &[&str]).expect("omitted --task-worker");
        assert_eq!(worker, default_task_worker());
        assert_eq!(worker.command, "pi");
        assert_eq!(worker.args, vec!["--print".to_owned()]);
    }

    #[test]
    fn one_task_worker_replaces_default() {
        let worker = parse_run_plan_graph_args(["--task-worker", valid_worker_json()])
            .expect("one --task-worker");
        assert_eq!(worker.command, "echo");
        assert_eq!(worker.args, vec!["hello".to_owned()]);
    }

    #[test]
    fn two_task_worker_flags_fail() {
        let result = parse_run_plan_graph_args([
            "--task-worker",
            valid_worker_json(),
            "--task-worker",
            valid_worker_json(),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn leftover_unknown_arg_fails() {
        assert!(parse_run_plan_graph_args(["leftover"]).is_err());
        assert!(parse_run_plan_graph_args(["--max-concurrency", "8"]).is_err());
    }

    #[test]
    fn max_concurrency_is_four() {
        assert_eq!(MAX_CONCURRENCY, 4);
    }

    #[test]
    fn task_ids_must_be_single_path_components() {
        assert!(is_safe_task_id("cli-argv-contracts"));
        assert!(is_safe_task_id("a"));
        assert!(!is_safe_task_id("../../escaped"));
        assert!(!is_safe_task_id("foo/bar"));
        assert!(!is_safe_task_id(".."));
        assert!(!is_safe_task_id("."));
        assert!(!is_safe_task_id(""));
        assert!(!is_safe_task_id("foo\\bar"));
    }
}
