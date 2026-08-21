//! Argv and stdin contracts plus the Dagu-backed executor for `run-plan-graph`.

use crate::dagu::{names_for_capture_root, resolve_dagu, write_locator};
use crate::schema::{self, CheckResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Default concurrent ordinary plan-task cap when `--max-active` is omitted.
/// Emitted as Dagu `max_active_steps`. The summarizer is not a concurrent peer.
pub(crate) const MAX_CONCURRENCY: usize = 4;

const DEFAULT_WORKER_COMMAND: &str = "pi";
const DEFAULT_WORKER_ARGS: &[&str] = &["--print", "--no-skills", "--no-extensions"];
const SUMMARIZER_STEP: &str = "summarizer";
const REPORT_FILE: &str = "implementation-report.json";
const SUMMARY_FILE: &str = "summary.json";
const REQUIRED_REPORT_KEYS: &[&str] = &[
    "revision",
    "author",
    "plan_revision",
    "coverage",
    "summary",
    "changed_surface",
    "validation",
];
const SUMMARIZER_ASSIGNMENT: &str = "Write artifact_root/implementation-report.json for this invocation only. You are the sole writer of that filename. plan_revision must equal the revision of the plan.json at plan_path. Do not concatenate worker stdout. Do not append review-evidence. Ordinary plan tasks must not write that filename.";

/// Worker argv: JSON object with exactly string `command` and array-of-string `args`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerCli {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

/// Parsed argv after the `run-plan-graph` command name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunPlanGraphArgs {
    pub(crate) worker: WorkerCli,
    pub(crate) max_active: usize,
    pub(crate) working_directory: PathBuf,
}

/// Bound-worker stdin packet.  Exactly the five engine invoke keys; no extras.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvokePacket {
    pub(crate) run_id: String,
    pub(crate) slot_id: String,
    pub(crate) artifact_root: String,
    pub(crate) instruction_body: String,
    pub(crate) capture_dir: String,
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

/// Default inner worker when `--task-worker` is omitted:
/// `pi --print --no-skills --no-extensions`.
/// Does not pass `--no-context-files` or `--tools`.
pub(crate) fn default_task_worker() -> WorkerCli {
    WorkerCli {
        command: DEFAULT_WORKER_COMMAND.to_owned(),
        args: DEFAULT_WORKER_ARGS
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect(),
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

/// Parse the five-key engine invoke packet from stdin JSON.
pub(crate) fn parse_invoke_packet(raw: &str) -> Result<InvokePacket, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "invoke packet must be a JSON object with exactly `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`: {error}"
        ))
    })
}

/// Parse argv tokens after the `run-plan-graph` command name.
///
/// Required once: `--working-directory ABS`, an existing absolute directory
/// selected and maintained by the caller.  The supplied path is preserved.
/// Optional once: `--task-worker JSON`.  Omitted flag yields [`default_task_worker`].
/// Optional once: `--max-active N` with decimal integer N >= 1.  Omitted flag
/// yields [`MAX_CONCURRENCY`].  Unknown flags, leftover positionals, and
/// repeated flags are errors.  There is no `--max-concurrency` flag.
pub(crate) fn parse_run_plan_graph_args<I, S>(args: I) -> Result<RunPlanGraphArgs, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|token| token.as_ref().to_owned())
        .collect::<Vec<String>>();
    let mut worker = None;
    let mut max_active = None;
    let mut working_directory = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if let Some(raw) = strip_option(token, "--working-directory") {
            if working_directory.is_some() {
                return Err(ParseError::new(
                    "`--working-directory` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--working-directory")?.to_owned(),
            };
            working_directory = Some(validate_working_directory(&raw)?);
            continue;
        }
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
        if let Some(raw) = strip_option(token, "--max-active") {
            if max_active.is_some() {
                return Err(ParseError::new(
                    "`--max-active` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--max-active")?.to_owned(),
            };
            max_active = Some(parse_max_active(&raw)?);
            continue;
        }
        if token.starts_with('-') {
            return Err(ParseError::new(format!("unknown option `{token}`")));
        }
        return Err(ParseError::new(format!(
            "unexpected argument `{token}` for run-plan-graph"
        )));
    }
    let working_directory = working_directory.ok_or_else(|| {
        ParseError::new(
            "missing required option `--working-directory` (working directory was omitted)",
        )
    })?;
    Ok(RunPlanGraphArgs {
        worker: worker.unwrap_or_else(default_task_worker),
        max_active: max_active.unwrap_or(MAX_CONCURRENCY),
        working_directory,
    })
}

fn validate_working_directory(raw: &str) -> Result<PathBuf, ParseError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ParseError::new(format!(
            "`--working-directory` value `{raw}` is relative; expected an absolute directory"
        )));
    }
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => Ok(path),
        Ok(_) => Err(ParseError::new(format!(
            "`--working-directory` value `{raw}` is not a directory"
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(ParseError::new(format!(
            "`--working-directory` value `{raw}` is nonexistent"
        ))),
        Err(error) => Err(ParseError::new(format!(
            "`--working-directory` value `{raw}` is not a directory: {error}"
        ))),
    }
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

fn parse_max_active(raw: &str) -> Result<usize, ParseError> {
    raw.parse::<usize>()
        .ok()
        .filter(|value| *value >= 1)
        .ok_or_else(|| {
            ParseError::new(format!(
                "`--max-active` requires a decimal integer >= 1, got `{raw}`"
            ))
        })
}

/// Read the invoke packet from stdin, schedule `plan.json`, and waitpid Dagu.
pub(crate) fn execute(args: &RunPlanGraphArgs) -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("could not read invoke packet: {error}");
        return 2;
    }
    match execute_from_packet(args, &input) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            error.exit_code()
        }
    }
}

#[derive(Deserialize)]
struct PlanDocument {
    #[serde(default)]
    revision: Option<String>,
    tasks: Vec<Value>,
    dependency_graph: Vec<DependencyEdge>,
}

#[derive(Deserialize)]
struct DependencyEdge {
    from: String,
    to: String,
}

struct PlanGraph {
    revision: String,
    order: Vec<String>,
    tasks: HashMap<String, Value>,
    predecessors: HashMap<String, HashSet<String>>,
}

struct PreparedStep {
    name: String,
    stdin_path: String,
    stdout_path: String,
    stderr_path: String,
    depends: Vec<String>,
}

#[derive(Serialize)]
struct CaptureWorker<'a> {
    command: &'a str,
    args: &'a [String],
    exit_code: i32,
    stdout_path: &'a str,
    stderr_path: &'a str,
}

#[derive(Serialize)]
struct CaptureSummary<'a> {
    workers: Vec<CaptureWorker<'a>>,
}

#[derive(Serialize)]
struct ArtifactRootContext<'a> {
    artifact_root: &'a str,
}

#[derive(Serialize)]
struct SummarizerLocation<'a> {
    artifact_root: &'a str,
    capture_dir: &'a str,
    plan_path: &'a str,
}

#[derive(Clone, Copy)]
struct StepOutcome {
    started: bool,
    exit_code: Option<i32>,
}

fn execute_from_packet(args: &RunPlanGraphArgs, raw_packet: &str) -> Result<(), ExecuteError> {
    let packet =
        parse_invoke_packet(raw_packet).map_err(|error| ExecuteError::usage(error.to_string()))?;
    let _ = (&packet.run_id, &packet.slot_id, &packet.instruction_body);
    if packet.capture_dir.is_empty() {
        return Err(ExecuteError::usage(
            "invoke packet capture_dir must be a non-empty path",
        ));
    }
    let artifact_root = absolute_from_cwd(&packet.artifact_root)?;
    let capture_root = absolute_from_cwd(&packet.capture_dir)?;
    delete_stale_report(&artifact_root)?;
    let plan_path = artifact_root.join("plan.json");
    let plan_raw = fs::read_to_string(&plan_path).map_err(|error| {
        ExecuteError::failed(format!("could not read {}: {error}", plan_path.display()))
    })?;
    let plan = parse_plan(&plan_raw)?;
    let dagu = resolve_dagu().map_err(|error| ExecuteError::failed(error.to_string()))?;
    run_dagu_graph(
        args,
        &dagu,
        &artifact_root,
        &plan_path,
        &capture_root,
        &plan,
    )
}

fn delete_stale_report(artifact_root: &Path) -> Result<(), ExecuteError> {
    let report = artifact_root.join(REPORT_FILE);
    match fs::remove_file(&report) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ExecuteError::failed(format!(
            "could not delete leftover {}: {error}",
            report.display()
        ))),
    }
}

fn absolute_from_cwd(raw: &str) -> Result<PathBuf, ExecuteError> {
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
        if id == SUMMARIZER_STEP {
            return Err(ExecuteError::failed(
                "plan.json task id `summarizer` collides with the summarizer step",
            ));
        }
        if !is_dagu_safe_task_id(&id) {
            return Err(ExecuteError::failed(format!(
                "plan.json task id `{id}` is not Dagu-safe [A-Za-z0-9_-]"
            )));
        }
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
        revision: document.revision.unwrap_or_default(),
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

fn run_dagu_graph(
    args: &RunPlanGraphArgs,
    dagu: &Path,
    artifact_root: &Path,
    plan_path: &Path,
    capture_root: &Path,
    plan: &PlanGraph,
) -> Result<(), ExecuteError> {
    fs::create_dir_all(capture_root).map_err(|error| {
        ExecuteError::failed(format!(
            "could not create {}: {error}",
            capture_root.display()
        ))
    })?;
    let (dag_name, run_name) = names_for_capture_root(capture_root)
        .map_err(|error| ExecuteError::failed(error.to_string()))?;
    let locator = write_locator(capture_root, &dag_name, &run_name)
        .map_err(|error| ExecuteError::failed(error.to_string()))?;
    let home = PathBuf::from(&locator.dagu_home);
    write_isolated_home_files(&home)?;
    let software_change = software_change_exe()?;
    let software_change = path_to_string(&software_change);

    let mut steps = Vec::new();
    for id in &plan.order {
        let out_dir = task_capture_dir(capture_root, id)?;
        fs::create_dir_all(&out_dir).map_err(|error| {
            ExecuteError::failed(format!("could not create {}: {error}", out_dir.display()))
        })?;
        let task = plan.tasks.get(id).expect("plan order id exists in tasks");
        let stdin_path = out_dir.join("stdin");
        let stdin_text = task_stdin(artifact_root, task)?;
        fs::write(&stdin_path, stdin_text).map_err(|error| {
            ExecuteError::failed(format!("could not write {}: {error}", stdin_path.display()))
        })?;
        let mut depends: Vec<String> = plan
            .predecessors
            .get(id)
            .map(|preds| {
                plan.order
                    .iter()
                    .filter(|pred| preds.contains(*pred))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        depends.sort();
        steps.push(PreparedStep {
            name: id.clone(),
            stdin_path: path_to_string(&stdin_path),
            stdout_path: path_to_string(&out_dir.join("stdout")),
            stderr_path: path_to_string(&out_dir.join("stderr")),
            depends,
        });
    }

    let summarizer_dir = task_capture_dir(capture_root, SUMMARIZER_STEP)?;
    fs::create_dir_all(&summarizer_dir).map_err(|error| {
        ExecuteError::failed(format!(
            "could not create {}: {error}",
            summarizer_dir.display()
        ))
    })?;
    let summarizer_stdin_path = summarizer_dir.join("stdin");
    let summarizer_stdin = summarizer_stdin(artifact_root, capture_root, plan_path)?;
    fs::write(&summarizer_stdin_path, summarizer_stdin).map_err(|error| {
        ExecuteError::failed(format!(
            "could not write {}: {error}",
            summarizer_stdin_path.display()
        ))
    })?;
    let summarizer = PreparedStep {
        name: SUMMARIZER_STEP.to_owned(),
        stdin_path: path_to_string(&summarizer_stdin_path),
        stdout_path: path_to_string(&summarizer_dir.join("stdout")),
        stderr_path: path_to_string(&summarizer_dir.join("stderr")),
        depends: plan.order.clone(),
    };

    let yaml = emit_graph_yaml(
        &dag_name,
        &software_change,
        &args.worker,
        &args.working_directory,
        &steps,
        &summarizer,
        args.max_active,
    );
    let dags_dir = home.join("dags");
    fs::create_dir_all(&dags_dir).map_err(|error| {
        ExecuteError::failed(format!("could not create {}: {error}", dags_dir.display()))
    })?;
    let yaml_path = dags_dir.join(format!("{dag_name}.yaml"));
    fs::write(&yaml_path, yaml).map_err(|error| {
        ExecuteError::failed(format!("could not write {}: {error}", yaml_path.display()))
    })?;

    run_dagu_cli(
        dagu,
        &[
            "validate",
            "--dagu-home",
            &locator.dagu_home,
            &path_to_string(&yaml_path),
        ],
        false,
    )?;

    let start_result = run_dagu_cli(
        dagu,
        &[
            "start",
            "--quiet",
            "--dagu-home",
            &locator.dagu_home,
            "--name",
            &dag_name,
            "--run-id",
            &run_name,
            &path_to_string(&yaml_path),
        ],
        true,
    );
    let start_ok = start_result.is_ok();
    let outcomes = step_outcomes(&home);

    let summary_error = write_plan_summary(capture_root, &args.worker, plan, &steps, &outcomes);
    let summarizer_ok = summarizer_succeeded(&outcomes, capture_root, start_ok);
    let report_error = if summarizer_ok {
        validate_fresh_report(artifact_root, &plan.revision)
    } else {
        Err(ExecuteError::failed(
            "summarizer step did not exit 0".to_owned(),
        ))
    };

    if let Err(error) = summary_error {
        if start_ok && report_error.is_ok() {
            return Err(error);
        }
    }
    start_result?;
    report_error
}

fn summarizer_succeeded(
    outcomes: &HashMap<String, StepOutcome>,
    capture_root: &Path,
    start_ok: bool,
) -> bool {
    if let Some(outcome) = outcomes.get(SUMMARIZER_STEP) {
        return outcome.exit_code == Some(0);
    }
    start_ok && capture_root.join(SUMMARIZER_STEP).join("stdout").is_file()
}

fn write_plan_summary(
    capture_root: &Path,
    worker: &WorkerCli,
    plan: &PlanGraph,
    steps: &[PreparedStep],
    outcomes: &HashMap<String, StepOutcome>,
) -> Result<(), ExecuteError> {
    let mut workers = Vec::new();
    for (id, step) in plan.order.iter().zip(steps.iter()) {
        let stdout_path = Path::new(&step.stdout_path);
        let stderr_path = Path::new(&step.stderr_path);
        let outcome = outcomes.get(id).copied();
        let started = stdout_path.is_file()
            || stderr_path.is_file()
            || outcome.map(|item| item.started).unwrap_or(false);
        if !started {
            continue;
        }
        if let Some(parent) = stdout_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !stdout_path.exists() {
            let _ = fs::write(stdout_path, b"");
        }
        if !stderr_path.exists() {
            let _ = fs::write(stderr_path, b"");
        }
        let exit_code = outcome.and_then(|item| item.exit_code).unwrap_or(1);
        workers.push(CaptureWorker {
            command: &worker.command,
            args: &worker.args,
            exit_code,
            stdout_path: &step.stdout_path,
            stderr_path: &step.stderr_path,
        });
    }
    let path = capture_root.join(SUMMARY_FILE);
    let bytes = serde_json::to_vec_pretty(&CaptureSummary { workers }).map_err(|error| {
        ExecuteError::failed(format!(
            "could not serialize capture summary {}: {error}",
            path.display()
        ))
    })?;
    fs::write(&path, bytes).map_err(|error| {
        ExecuteError::failed(format!(
            "could not write capture summary {}: {error}",
            path.display()
        ))
    })?;
    Ok(())
}

fn validate_fresh_report(artifact_root: &Path, plan_revision: &str) -> Result<(), ExecuteError> {
    let report_path = artifact_root.join(REPORT_FILE);
    let raw = fs::read(&report_path).map_err(|error| {
        ExecuteError::failed(format!(
            "missing {} after summarizer succeeded: {error}",
            report_path.display()
        ))
    })?;
    let value: Value = serde_json::from_slice(&raw).map_err(|error| {
        ExecuteError::failed(format!(
            "{} is not valid JSON: {error}",
            report_path.display()
        ))
    })?;
    match load_frozen_report_schema(artifact_root) {
        Some(schema) => match schema::check(&schema, &value) {
            CheckResult::Valid => {}
            CheckResult::SchemaInvalid(_) => required_keys_valid(&value)?,
            CheckResult::InstanceInvalid(report) => {
                return Err(ExecuteError::failed(format!(
                    "{} failed schema validation: {}",
                    report_path.display(),
                    report
                        .violations()
                        .iter()
                        .map(|item| item.to_string())
                        .collect::<Vec<_>>()
                        .join("; ")
                )))
            }
        },
        None => required_keys_valid(&value)?,
    }
    let reported = value
        .get("plan_revision")
        .and_then(Value::as_str)
        .unwrap_or("");
    if reported != plan_revision {
        return Err(ExecuteError::failed(format!(
            "{} plan_revision `{reported}` does not match plan.json revision `{plan_revision}`",
            report_path.display()
        )));
    }
    Ok(())
}

fn load_frozen_report_schema(artifact_root: &Path) -> Option<Value> {
    for name in ["artifact_schemas.json", "initial_input.json"] {
        let path = artifact_root.join(name);
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(schema) = extract_report_schema(&value) {
            return Some(schema);
        }
    }
    None
}

fn extract_report_schema(value: &Value) -> Option<Value> {
    value.get(REPORT_FILE).cloned().or_else(|| {
        value
            .get("artifact_schemas")
            .and_then(|schemas| schemas.get(REPORT_FILE).cloned())
    })
}

fn required_keys_valid(value: &Value) -> Result<(), ExecuteError> {
    let object = value
        .as_object()
        .ok_or_else(|| ExecuteError::failed(format!("{REPORT_FILE} must be a JSON object")))?;
    for key in REQUIRED_REPORT_KEYS {
        if !object.contains_key(*key) {
            return Err(ExecuteError::failed(format!(
                "{REPORT_FILE} is missing required key `{key}`"
            )));
        }
        if is_empty_required_value(&object[*key]) {
            return Err(ExecuteError::failed(format!(
                "{REPORT_FILE} required key `{key}` is empty"
            )));
        }
    }
    Ok(())
}

fn is_empty_required_value(value: &Value) -> bool {
    match value {
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object.is_empty(),
        Value::Null => true,
        _ => false,
    }
}

fn write_isolated_home_files(home: &Path) -> Result<(), ExecuteError> {
    let base = home.join("base.yaml");
    fs::write(&base, "type: graph\n").map_err(|error| {
        ExecuteError::failed(format!(
            "could not write isolated dagu base.yaml `{}`: {error}",
            base.display()
        ))
    })?;
    let config = home.join("config.yaml");
    fs::write(&config, "auth:\n  mode: none\n").map_err(|error| {
        ExecuteError::failed(format!(
            "could not write isolated dagu config.yaml `{}`: {error}",
            config.display()
        ))
    })?;
    Ok(())
}

fn software_change_exe() -> Result<PathBuf, ExecuteError> {
    let path = std::env::current_exe().map_err(|error| {
        ExecuteError::failed(format!(
            "could not resolve software-change executable: {error}"
        ))
    })?;
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

fn run_dagu_cli(dagu: &Path, args: &[&str], allow_nonzero: bool) -> Result<(), ExecuteError> {
    let output = Command::new(dagu)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            ExecuteError::failed(format!(
                "could not run `{} {}`: {error}",
                dagu.display(),
                args.join(" ")
            ))
        })?;
    if output.status.success() {
        return Ok(());
    }
    let detail = dagu_failure_detail(&output);
    let verb = args.first().copied().unwrap_or("command");
    if allow_nonzero {
        return Err(ExecuteError::failed(format!(
            "dagu {verb} did not complete successfully{detail}"
        )));
    }
    Err(ExecuteError::failed(format!("dagu {verb} failed{detail}")))
}

fn dagu_failure_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = stderr.trim();
    if !message.is_empty() {
        return format!(": {message}");
    }
    let message = stdout.trim();
    if !message.is_empty() {
        return format!(": {message}");
    }
    if let Some(code) = output.status.code() {
        format!(" (exit {code})")
    } else {
        " (terminated by signal)".to_owned()
    }
}

fn emit_graph_yaml(
    dag_name: &str,
    software_change: &str,
    worker: &WorkerCli,
    working_directory: &Path,
    steps: &[PreparedStep],
    summarizer: &PreparedStep,
    max_active: usize,
) -> String {
    let _ = dag_name;
    let mut yaml = String::from("type: graph\nworking_dir: ");
    yaml.push_str(&yaml_double_quoted(&path_to_string(working_directory)));
    yaml.push_str("\nmax_active_steps: ");
    yaml.push_str(&max_active.to_string());
    yaml.push_str("\nsteps:\n");
    for step in steps.iter().chain(std::iter::once(summarizer)) {
        yaml.push_str("  - name: ");
        yaml.push_str(&yaml_double_quoted(&step.name));
        yaml.push('\n');
        yaml.push_str("    action: exec\n");
        if !step.depends.is_empty() {
            yaml.push_str("    depends:\n");
            for dep in &step.depends {
                yaml.push_str("      - ");
                yaml.push_str(&yaml_double_quoted(dep));
                yaml.push('\n');
            }
        }
        yaml.push_str("    with:\n");
        yaml.push_str("      command: ");
        yaml.push_str(&yaml_double_quoted(software_change));
        yaml.push('\n');
        yaml.push_str("      args:\n");
        let mut args = vec![
            "stdin-exec".to_owned(),
            "--exit-mode".to_owned(),
            "propagate".to_owned(),
            "--stdin-file".to_owned(),
            step.stdin_path.clone(),
            "--".to_owned(),
            worker.command.clone(),
        ];
        args.extend(worker.args.iter().cloned());
        for arg in args {
            yaml.push_str("        - ");
            yaml.push_str(&yaml_double_quoted(&arg));
            yaml.push('\n');
        }
        yaml.push_str("    stdout: ");
        yaml.push_str(&yaml_double_quoted(&step.stdout_path));
        yaml.push('\n');
        yaml.push_str("    stderr: ");
        yaml.push_str(&yaml_double_quoted(&step.stderr_path));
        yaml.push('\n');
    }
    yaml
}

fn yaml_double_quoted(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn task_stdin(artifact_root: &Path, task: &Value) -> Result<String, ExecuteError> {
    let location = serde_json::to_string(&ArtifactRootContext {
        artifact_root: &path_to_string(artifact_root),
    })
    .map_err(|error| ExecuteError::failed(format!("could not serialize location JSON: {error}")))?;
    let task_json = serde_json::to_string(task).map_err(|error| {
        ExecuteError::failed(format!("could not serialize task record: {error}"))
    })?;
    Ok(format!("{location}\n---\n\n{task_json}"))
}

fn summarizer_stdin(
    artifact_root: &Path,
    capture_root: &Path,
    plan_path: &Path,
) -> Result<String, ExecuteError> {
    let location = serde_json::to_string(&SummarizerLocation {
        artifact_root: &path_to_string(artifact_root),
        capture_dir: &path_to_string(capture_root),
        plan_path: &path_to_string(plan_path),
    })
    .map_err(|error| {
        ExecuteError::failed(format!(
            "could not serialize summarizer location JSON: {error}"
        ))
    })?;
    Ok(format!("{location}\n---\n\n{SUMMARIZER_ASSIGNMENT}"))
}

fn is_dagu_safe_task_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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

fn task_capture_dir(capture_root: &Path, task_id: &str) -> Result<PathBuf, ExecuteError> {
    if task_id != SUMMARIZER_STEP && !is_dagu_safe_task_id(task_id) {
        return Err(ExecuteError::failed(format!(
            "plan.json task id `{task_id}` is not Dagu-safe [A-Za-z0-9_-]"
        )));
    }
    if !is_safe_task_id(task_id) {
        return Err(ExecuteError::failed(format!(
            "plan.json task id `{task_id}` is not a single path-safe component"
        )));
    }
    let out_dir = capture_root.join(task_id);
    let capture_root = lexical_path(capture_root);
    let out_dir_lex = lexical_path(&out_dir);
    if !out_dir_lex.starts_with(&capture_root) {
        return Err(ExecuteError::failed(format!(
            "task capture path for `{task_id}` escapes capture_dir"
        )));
    }
    Ok(out_dir)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn step_outcomes(dagu_home: &Path) -> HashMap<String, StepOutcome> {
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
    let mut outcomes = HashMap::new();
    for node in nodes {
        let name = node
            .get("step")
            .and_then(|step| step.get("name"))
            .and_then(Value::as_str)
            .or_else(|| node.get("name").and_then(Value::as_str));
        let Some(name) = name else {
            continue;
        };
        outcomes.insert(name.to_owned(), outcome_from_node(node));
    }
    outcomes
}

fn outcome_from_node(node: &Value) -> StepOutcome {
    let status = node.get("status").and_then(Value::as_u64).unwrap_or(0);
    let started_at = node.get("startedAt").and_then(Value::as_str).unwrap_or("");
    let error = node.get("error").and_then(Value::as_str).unwrap_or("");
    let started = status == 2 || status == 4 || !started_at.is_empty();
    let exit_code = match status {
        4 => Some(0),
        2 => Some(parse_exit_status(error).unwrap_or(1)),
        _ => None,
    };
    StepOutcome { started, exit_code }
}

fn parse_exit_status(error: &str) -> Option<i32> {
    let marker = "exit status ";
    let rest = error.split(marker).nth(1)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn latest_status_jsonl(dagu_home: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_worker_json() -> &'static str {
        r#"{"command":"echo","args":["hello"]}"#
    }

    fn valid_packet_json() -> &'static str {
        r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","capture_dir":"/tmp/captures/inv-1"}"#
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
    fn valid_invoke_packet_parses_five_string_keys() {
        let packet = parse_invoke_packet(valid_packet_json()).expect("valid packet");
        assert_eq!(packet.run_id, "run-1");
        assert_eq!(packet.slot_id, "slot-1");
        assert_eq!(packet.artifact_root, "/tmp/artifacts");
        assert_eq!(packet.instruction_body, "Do the work");
        assert_eq!(packet.capture_dir, "/tmp/captures/inv-1");
    }

    #[test]
    fn invoke_packet_extra_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","capture_dir":"/tmp/captures/inv-1","extra":"no"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_missing_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_four_keys_without_capture_dir_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn omitted_task_worker_yields_default_pi_print() {
        let parsed = parse_run_plan_graph_args(["--working-directory", "/tmp"])
            .expect("omitted --task-worker");
        let worker = parsed.worker;
        assert_eq!(worker, default_task_worker());
        assert_eq!(parsed.max_active, MAX_CONCURRENCY);
        assert_eq!(parsed.max_active, 4);
        assert_eq!(parsed.working_directory, PathBuf::from("/tmp"));
        assert_eq!(worker.command, "pi");
        assert_eq!(
            worker.args,
            vec![
                "--print".to_owned(),
                "--no-skills".to_owned(),
                "--no-extensions".to_owned(),
            ]
        );
        assert!(!worker.args.iter().any(|arg| arg == "--no-context-files"));
        assert!(!worker
            .args
            .iter()
            .any(|arg| arg == "--tools" || arg.starts_with("--tools=")));
    }

    #[test]
    fn default_task_worker_is_sandboxed_pi_print() {
        let worker = default_task_worker();
        assert_eq!(worker.command, "pi");
        assert_eq!(
            worker.args,
            vec![
                "--print".to_owned(),
                "--no-skills".to_owned(),
                "--no-extensions".to_owned(),
            ]
        );
        assert!(!worker
            .args
            .iter()
            .any(|arg| arg.contains("--no-context-files")));
        assert!(!worker.args.iter().any(|arg| arg.contains("--tools")));
    }

    #[test]
    fn one_task_worker_replaces_default() {
        let parsed = parse_run_plan_graph_args([
            "--working-directory",
            "/tmp",
            "--task-worker",
            valid_worker_json(),
        ])
        .expect("one --task-worker");
        assert_eq!(parsed.worker.command, "echo");
        assert_eq!(parsed.worker.args, vec!["hello".to_owned()]);
        assert_eq!(parsed.max_active, MAX_CONCURRENCY);
    }

    #[test]
    fn two_task_worker_flags_fail() {
        let result = parse_run_plan_graph_args([
            "--working-directory",
            "/tmp",
            "--task-worker",
            valid_worker_json(),
            "--task-worker",
            valid_worker_json(),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn working_directory_is_required_and_validated_without_canonicalizing() {
        let root = std::env::temp_dir().join(format!(
            "software-change-working-directory-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("working-directory test directory");
        let file = root.join("not-a-directory");
        fs::write(&file, b"file").expect("working-directory test file");
        let missing = root.join("missing");
        let root = root.to_string_lossy().into_owned();
        let missing = missing.to_string_lossy().into_owned();

        let omitted = parse_run_plan_graph_args(&[] as &[&str]).expect_err("omitted directory");
        assert!(omitted.to_string().contains("omitted"), "{omitted}");

        let relative = parse_run_plan_graph_args(["--working-directory", "relative/checkout"])
            .expect_err("relative directory");
        assert!(relative.to_string().contains("relative"), "{relative}");
        assert!(
            relative.to_string().contains("relative/checkout"),
            "{relative}"
        );

        let nonexistent = parse_run_plan_graph_args(["--working-directory", &missing])
            .expect_err("nonexistent directory");
        assert!(
            nonexistent.to_string().contains("nonexistent"),
            "{nonexistent}"
        );
        assert!(nonexistent.to_string().contains(&missing), "{nonexistent}");

        let not_directory = parse_run_plan_graph_args([
            "--working-directory",
            file.to_str().expect("test file UTF-8"),
        ])
        .expect_err("file directory");
        assert!(
            not_directory.to_string().contains("not a directory"),
            "{not_directory}"
        );
        assert!(not_directory
            .to_string()
            .contains(file.to_str().expect("test file UTF-8")));

        let separate = parse_run_plan_graph_args(["--working-directory", &root])
            .expect("separated working-directory");
        assert_eq!(separate.working_directory, PathBuf::from(&root));
        let equals = parse_run_plan_graph_args(vec![format!("--working-directory={root}")])
            .expect("equals working-directory");
        assert_eq!(equals.working_directory, PathBuf::from(&root));
        let duplicate = parse_run_plan_graph_args(vec![
            "--working-directory".to_owned(),
            root.clone(),
            format!("--working-directory={root}"),
        ])
        .expect_err("duplicate working-directory");
        assert!(
            duplicate.to_string().contains("at most once"),
            "{duplicate}"
        );

        let _ = fs::remove_file(file);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn leftover_unknown_arg_fails() {
        assert!(parse_run_plan_graph_args(["leftover"]).is_err());
        assert!(parse_run_plan_graph_args(["--max-concurrency", "8"]).is_err());
    }

    #[test]
    fn max_active_n_is_parsed_and_omitted_stays_four() {
        let omitted = parse_run_plan_graph_args(["--working-directory", "/tmp"])
            .expect("omitted --max-active");
        assert_eq!(omitted.max_active, 4);
        let set = parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "2"])
            .expect("--max-active 2");
        assert_eq!(set.max_active, 2);
        assert_eq!(set.worker, default_task_worker());
        let equals = parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active=3"])
            .expect("--max-active=3");
        assert_eq!(equals.max_active, 3);
        let with_worker = parse_run_plan_graph_args([
            "--working-directory",
            "/tmp",
            "--task-worker",
            valid_worker_json(),
            "--max-active",
            "8",
        ])
        .expect("worker plus --max-active");
        assert_eq!(with_worker.max_active, 8);
        assert_eq!(with_worker.worker.command, "echo");
    }

    #[test]
    fn max_active_zero_missing_non_integer_and_repeat_fail() {
        assert!(
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "0"])
                .is_err()
        );
        assert!(
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active"]).is_err()
        );
        assert!(
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "nope"])
                .is_err()
        );
        assert!(
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "1.5"])
                .is_err()
        );
        assert!(
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "-1"])
                .is_err()
        );
        assert!(parse_run_plan_graph_args([
            "--working-directory",
            "/tmp",
            "--max-active",
            "2",
            "--max-active",
            "3"
        ])
        .is_err());
        assert!(parse_run_plan_graph_args([
            "--working-directory",
            "/tmp",
            "--max-active=2",
            "--max-active=4"
        ])
        .is_err());
    }

    #[test]
    fn max_concurrency_is_four() {
        assert_eq!(MAX_CONCURRENCY, 4);
    }

    #[test]
    fn task_ids_must_be_dagu_safe_single_path_components() {
        assert!(is_dagu_safe_task_id("cli-argv-contracts"));
        assert!(is_dagu_safe_task_id("a"));
        assert!(is_safe_task_id("cli-argv-contracts"));
        assert!(!is_dagu_safe_task_id("../../escaped"));
        assert!(!is_dagu_safe_task_id("foo/bar"));
        assert!(!is_dagu_safe_task_id("task.with.dots"));
        assert!(!is_safe_task_id("../../escaped"));
        assert!(!is_safe_task_id("foo/bar"));
        assert!(!is_safe_task_id(".."));
        assert!(!is_safe_task_id("."));
        assert!(!is_safe_task_id(""));
        assert!(!is_safe_task_id("foo\\bar"));
    }

    #[test]
    fn emitted_yaml_has_max_active_steps_and_no_continue_on() {
        let worker = WorkerCli {
            command: "python3".to_owned(),
            args: vec!["worker.py".to_owned()],
        };
        let task = PreparedStep {
            name: "task-a".to_owned(),
            stdin_path: "/tmp/cap/task-a/stdin".to_owned(),
            stdout_path: "/tmp/cap/task-a/stdout".to_owned(),
            stderr_path: "/tmp/cap/task-a/stderr".to_owned(),
            depends: Vec::new(),
        };
        let summarizer = PreparedStep {
            name: "summarizer".to_owned(),
            stdin_path: "/tmp/cap/summarizer/stdin".to_owned(),
            stdout_path: "/tmp/cap/summarizer/stdout".to_owned(),
            stderr_path: "/tmp/cap/summarizer/stderr".to_owned(),
            depends: vec!["task-a".to_owned()],
        };
        let yaml = emit_graph_yaml(
            "plan-graph-inv-1",
            "/abs/software-change",
            &worker,
            Path::new("/tmp/selected-checkout"),
            &[task],
            &summarizer,
            MAX_CONCURRENCY,
        );
        assert!(
            yaml.starts_with("type: graph\nworking_dir: \"/tmp/selected-checkout\"\n"),
            "{yaml}"
        );
        assert_eq!(yaml.matches("working_dir:").count(), 1, "{yaml}");
        assert!(yaml.contains("max_active_steps: 4"), "{yaml}");
        assert!(yaml.contains("action: exec"), "{yaml}");
        assert!(yaml.contains("name: \"task-a\""), "{yaml}");
        assert!(yaml.contains("name: \"summarizer\""), "{yaml}");
        assert!(
            yaml.contains("    depends:\n      - \"task-a\"\n"),
            "{yaml}"
        );
        assert!(yaml.contains("stdin-exec"), "{yaml}");
        assert!(yaml.contains("--exit-mode"), "{yaml}");
        assert!(yaml.contains("propagate"), "{yaml}");
        assert!(!yaml.contains("continue_on"), "{yaml}");
        assert!(!yaml.contains("retry_policy"), "{yaml}");
        assert!(!yaml.contains("instruction_body"), "{yaml}");
    }

    #[test]
    fn emitted_yaml_escapes_working_directory_once_at_graph_level() {
        let worker = WorkerCli {
            command: "python3".to_owned(),
            args: vec!["worker.py".to_owned()],
        };
        let yaml = emit_sample(
            &worker,
            Path::new("/tmp/checkout with \\\\slash and \"quote\""),
            &["task-a"],
            MAX_CONCURRENCY,
        );
        let expected = format!(
            "working_dir: {}",
            yaml_double_quoted("/tmp/checkout with \\\\slash and \"quote\"")
        );
        assert!(yaml.contains(&expected), "{yaml}");
        assert_eq!(yaml.matches("working_dir:").count(), 1, "{yaml}");
        assert!(!yaml.contains("    working_dir:"), "{yaml}");

        let parsed = parse_run_plan_graph_args(["--working-directory", "/tmp"]).expect("omitted");
        let yaml = emit_sample(
            &parsed.worker,
            Path::new("/tmp/selected-checkout"),
            &["task-a"],
            parsed.max_active,
        );
        assert!(yaml.contains("max_active_steps: 4"), "{yaml}");
        assert!(
            yaml.contains("    depends:\n      - \"task-a\"\n"),
            "{yaml}"
        );
    }

    #[test]
    fn omitted_parse_emits_max_active_steps_four() {
        let parsed = parse_run_plan_graph_args(["--working-directory", "/tmp"]).expect("omitted");
        let yaml = emit_sample(
            &parsed.worker,
            Path::new("/tmp/selected-checkout"),
            &["task-a"],
            parsed.max_active,
        );
        assert!(yaml.contains("max_active_steps: 4"), "{yaml}");
        assert!(
            yaml.contains("    depends:\n      - \"task-a\"\n"),
            "{yaml}"
        );
    }

    #[test]
    fn max_active_two_emits_max_active_steps_two() {
        let parsed =
            parse_run_plan_graph_args(["--working-directory", "/tmp", "--max-active", "2"])
                .expect("N=2");
        assert_eq!(parsed.max_active, 2);
        let yaml = emit_sample(
            &parsed.worker,
            Path::new("/tmp/selected-checkout"),
            &["task-a", "task-b"],
            parsed.max_active,
        );
        assert!(yaml.contains("max_active_steps: 2"), "{yaml}");
        assert!(!yaml.contains("max_active_steps: 4"), "{yaml}");
        let summarizer = yaml
            .split("name: \"summarizer\"")
            .nth(1)
            .expect("summarizer step");
        assert!(
            summarizer.contains("    depends:\n      - \"task-a\"\n      - \"task-b\"\n"),
            "{yaml}"
        );
    }

    fn emit_sample(
        worker: &WorkerCli,
        working_directory: &Path,
        task_names: &[&str],
        max_active: usize,
    ) -> String {
        let steps: Vec<PreparedStep> = task_names
            .iter()
            .map(|name| PreparedStep {
                name: (*name).to_owned(),
                stdin_path: format!("/tmp/cap/{name}/stdin"),
                stdout_path: format!("/tmp/cap/{name}/stdout"),
                stderr_path: format!("/tmp/cap/{name}/stderr"),
                depends: Vec::new(),
            })
            .collect();
        let summarizer = PreparedStep {
            name: "summarizer".to_owned(),
            stdin_path: "/tmp/cap/summarizer/stdin".to_owned(),
            stdout_path: "/tmp/cap/summarizer/stdout".to_owned(),
            stderr_path: "/tmp/cap/summarizer/stderr".to_owned(),
            depends: task_names.iter().map(|name| (*name).to_owned()).collect(),
        };
        emit_graph_yaml(
            "plan-graph-inv-1",
            "/abs/software-change",
            worker,
            working_directory,
            &steps,
            &summarizer,
            max_active,
        )
    }

    #[test]
    fn parse_plan_rejects_unknown_ids_cycles_and_summarizer_collision() {
        let unknown = parse_plan(
            r#"{"tasks":[{"id":"a"}],"dependency_graph":[{"from":"a","to":"missing"}]}"#,
        );
        assert!(unknown.is_err());
        let cycle = parse_plan(
            r#"{"tasks":[{"id":"a"},{"id":"b"}],"dependency_graph":[{"from":"a","to":"b"},{"from":"b","to":"a"}]}"#,
        );
        assert!(cycle.is_err());
        let collision = parse_plan(r#"{"tasks":[{"id":"summarizer"}],"dependency_graph":[]}"#);
        assert!(collision.is_err());
    }
}
