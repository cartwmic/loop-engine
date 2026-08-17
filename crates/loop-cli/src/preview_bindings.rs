//! Inspect `work_slot_bindings` without starting a run.
//!
//! `preview-bindings` expands nested `--worker` / `--task-worker` JSON with the
//! same `{command, args}` parse as `fan-out`, lists `--model` values, and warns
//! on inspectable risks. It opens no database. Zero-worker fan-out is an error;
//! warnings alone are not. A pi worker with `--no-extensions`/`-ne` and no
//! `-e`/`--extension` is a warning; missing `--no-extensions` is not.

use crate::fan_out::{parse_worker_cli_json, WorkerCli};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub(crate) const DEFAULT_INVOKE_TIMEOUT_WARNING: &str =
    "invoke allowed_time_ms defaults to 30000 unless the caller passes --timeout-ms";

/// Substring of the warning when a pi worker has `--no-extensions`/`-ne` and no `-e`/`--extension`.
pub(crate) const PI_NO_EXTENSIONS_WITHOUT_E: &str = "has --no-extensions and no -e";

const REVIEW_TOOLS: [&str; 4] = ["find", "grep", "ls", "read"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewError {
    pub(crate) message: String,
}

impl PreviewError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Expanded binding preview.  Not a run-state envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PreviewReport {
    pub(crate) bindings: Vec<SlotPreview>,
    pub(crate) models: Vec<String>,
    pub(crate) warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SlotPreview {
    pub(crate) slot_id: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) workers: Vec<PreviewWorker>,
    pub(crate) models: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PreviewWorker {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

impl From<WorkerCli> for PreviewWorker {
    fn from(worker: WorkerCli) -> Self {
        Self {
            command: worker.command,
            args: worker.args,
        }
    }
}

/// Load the JSON operand: omitted reads `stdin`, `@FILE` reads that path,
/// otherwise the operand is inline JSON.
pub(crate) fn load_source<R: Read>(
    operand: Option<&str>,
    stdin: R,
) -> Result<String, PreviewError> {
    match operand {
        None => read_to_string(stdin, "stdin"),
        Some(raw) => match raw.strip_prefix('@') {
            Some(path) => {
                if path.is_empty() {
                    return Err(PreviewError::new(
                        "preview-bindings file path after `@` is empty",
                    ));
                }
                fs::read_to_string(Path::new(path))
                    .map_err(|error| PreviewError::new(format!("could not read `{path}`: {error}")))
            }
            None => Ok(raw.to_owned()),
        },
    }
}

fn read_to_string<R: Read>(mut reader: R, source: &str) -> Result<String, PreviewError> {
    let mut buf = String::new();
    reader
        .read_to_string(&mut buf)
        .map_err(|error| PreviewError::new(format!("could not read {source}: {error}")))?;
    Ok(buf)
}

/// Expand bindings from JSON text. `warn_default_timeout` is true when the
/// caller did not pass `--timeout-ms`.
pub(crate) fn preview(
    raw: &str,
    warn_default_timeout: bool,
) -> Result<PreviewReport, PreviewError> {
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        PreviewError::new(format!("preview-bindings input is not valid JSON: {error}"))
    })?;
    let map = bindings_map(value)?;
    let mut slot_ids = map.keys().cloned().collect::<Vec<_>>();
    slot_ids.sort();

    let mut bindings = Vec::new();
    let mut models = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    for slot_id in slot_ids {
        let value = map
            .get(&slot_id)
            .expect("sorted keys come from the map")
            .clone();
        let slot = preview_slot(&slot_id, value)?;
        for model in &slot.models {
            push_unique(&mut models, model.clone());
        }
        collect_slot_warnings(&slot, &mut warnings);
        if is_fan_out_binding(&slot.args) && worker_flag_count(&slot.args) == 0 {
            errors.push(format!(
                "fan-out binding `{slot_id}` has zero --worker entries"
            ));
        }
        bindings.push(slot);
    }

    if warn_default_timeout {
        push_unique(&mut warnings, DEFAULT_INVOKE_TIMEOUT_WARNING.to_owned());
    }

    Ok(PreviewReport {
        bindings,
        models,
        warnings,
        errors,
    })
}

fn bindings_map(value: Value) -> Result<Map<String, Value>, PreviewError> {
    let Value::Object(object) = value else {
        return Err(PreviewError::new(
            "preview-bindings JSON must be a work_slot_bindings map or an object containing that key",
        ));
    };
    if let Some(bindings) = object.get("work_slot_bindings") {
        return match bindings {
            Value::Object(map) => Ok(map.clone()),
            _ => Err(PreviewError::new(
                "work_slot_bindings must be an object map of slot_id to {command, args}",
            )),
        };
    }
    Ok(object)
}

fn preview_slot(slot_id: &str, value: Value) -> Result<SlotPreview, PreviewError> {
    let binding: WorkerCli = serde_json::from_value(value).map_err(|error| {
        PreviewError::new(format!(
            "work_slot_bindings[{slot_id}] must be an object with exactly string `command` and array-of-string `args`: {error}"
        ))
    })?;
    let workers = nested_workers(slot_id, &binding.args)?;
    let mut models = models_in_argv(&binding.args);
    for worker in &workers {
        for model in models_in_argv(&worker.args) {
            push_unique(&mut models, model);
        }
    }
    Ok(SlotPreview {
        slot_id: slot_id.to_owned(),
        command: binding.command,
        args: binding.args,
        workers: workers.into_iter().map(PreviewWorker::from).collect(),
        models,
    })
}

fn nested_workers(slot_id: &str, args: &[String]) -> Result<Vec<WorkerCli>, PreviewError> {
    let mut workers = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if let Some((flag, raw)) = nested_worker_json(args, &mut index)? {
            let worker = parse_worker_cli_json(&raw).map_err(|error| {
                PreviewError::new(format!(
                    "nested {flag} for slot `{slot_id}` is not a {{command, args}} object: {error}"
                ))
            })?;
            workers.push(worker);
            continue;
        }
        let _ = token;
        index += 1;
    }
    Ok(workers)
}

fn nested_worker_json(
    args: &[String],
    index: &mut usize,
) -> Result<Option<(&'static str, String)>, PreviewError> {
    let token = &args[*index];
    if token == "--worker" || token == "--task-worker" {
        let flag = if token == "--worker" {
            "--worker"
        } else {
            "--task-worker"
        };
        let raw = args.get(*index + 1).cloned().ok_or_else(|| {
            PreviewError::new(format!("{flag} requires a JSON object {{command, args}}"))
        })?;
        if raw.starts_with('-') && raw != "-" {
            return Err(PreviewError::new(format!(
                "{flag} requires a JSON object {{command, args}}"
            )));
        }
        *index += 2;
        return Ok(Some((flag, raw)));
    }
    if let Some(raw) = token.strip_prefix("--worker=") {
        *index += 1;
        return Ok(Some(("--worker", raw.to_owned())));
    }
    if let Some(raw) = token.strip_prefix("--task-worker=") {
        *index += 1;
        return Ok(Some(("--task-worker", raw.to_owned())));
    }
    Ok(None)
}

fn collect_slot_warnings(slot: &SlotPreview, warnings: &mut Vec<String>) {
    inspect_command(
        &slot.command,
        &slot.args,
        is_fan_out_binding(&slot.args),
        warnings,
    );
    let fan_out = is_fan_out_binding(&slot.args);
    for worker in &slot.workers {
        inspect_command(&worker.command, &worker.args, fan_out, warnings);
    }
}

fn inspect_command(
    command: &str,
    args: &[String],
    under_fan_out: bool,
    warnings: &mut Vec<String>,
) {
    if !command.contains('/') {
        push_unique(
            warnings,
            format!("command `{command}` is a PATH name, not an absolute path"),
        );
    }
    if !is_pi(command) {
        return;
    }
    if !has_token(args, "--model") {
        push_unique(
            warnings,
            format!("command `{command}` is pi and args have no --model"),
        );
    }
    if !has_bool_flag(args, "--no-skills", "-ns") {
        push_unique(warnings, format!("pi worker `{command}` lacks --no-skills"));
    }
    if has_bool_flag(args, "--no-extensions", "-ne") && !has_extension_flag(args) {
        push_unique(
            warnings,
            format!("pi worker `{command}` {PI_NO_EXTENSIONS_WITHOUT_E}"),
        );
    }
    if under_fan_out && !tools_is_review_readonly(args) {
        push_unique(
            warnings,
            format!(
                "fan-out pi worker `{command}` lacks --tools whose comma-separated value is exactly read,grep,find,ls"
            ),
        );
    }
}

fn is_pi(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        == "pi"
}

fn is_fan_out_binding(args: &[String]) -> bool {
    args.iter().any(|token| token == "fan-out")
}

fn worker_flag_count(args: &[String]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if token == "--worker" {
            count += 1;
            index += 2;
            continue;
        }
        if token.starts_with("--worker=") {
            count += 1;
        }
        index += 1;
    }
    count
}

fn models_in_argv(args: &[String]) -> Vec<String> {
    let mut models = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--model" {
            if let Some(value) = args.get(index + 1) {
                push_unique(&mut models, value.clone());
                index += 2;
                continue;
            }
        }
        index += 1;
    }
    models
}

fn has_token(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|token| token == flag || token.starts_with(&format!("{flag}=")))
}

fn has_bool_flag(args: &[String], long: &str, short: &str) -> bool {
    args.iter()
        .any(|token| token == long || token == short || token.starts_with(&format!("{long}=")))
}

fn has_extension_flag(args: &[String]) -> bool {
    args.iter()
        .any(|token| token == "-e" || token == "--extension" || token.starts_with("--extension="))
}

fn tools_is_review_readonly(args: &[String]) -> bool {
    let Some(value) = tools_value(args) else {
        return false;
    };
    let set: BTreeSet<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    set == BTreeSet::from(REVIEW_TOOLS)
}

fn tools_value(args: &[String]) -> Option<&str> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--tools" {
            return args.get(index + 1).map(String::as_str);
        }
        if let Some(value) = args[index].strip_prefix("--tools=") {
            return Some(value);
        }
        index += 1;
    }
    None
}

fn push_unique(items: &mut Vec<String>, item: String) {
    if !items.iter().any(|existing| existing == &item) {
        items.push(item);
    }
}

pub(crate) fn load_from_stdin_or_operand(operand: Option<&str>) -> Result<String, PreviewError> {
    load_source(operand, io::stdin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{execute, EXIT_COMPLETED, EXIT_INVALID_INVOCATION};
    use serde_json::json;
    use std::io::Cursor;

    fn worker_json(command: &str, args: &[&str]) -> String {
        json!({"command": command, "args": args}).to_string()
    }

    fn fan_out_binding(workers: &[String]) -> Value {
        let mut args = vec!["fan-out".to_owned()];
        for worker in workers {
            args.push("--worker".to_owned());
            args.push(worker.clone());
        }
        json!({"command": "loop-engine", "args": args})
    }

    #[test]
    fn zero_worker_fan_out_binding_exits_nonzero_and_creates_no_sqlite_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = directory.path().join("must-not-exist.sqlite");
        let operand =
            json!({"design-review": {"command": "loop-engine", "args": ["fan-out"]}}).to_string();
        let result = execute([
            "preview-bindings",
            &operand,
            "--database",
            database.to_str().expect("utf-8 path"),
        ]);
        assert_eq!(result.exit_code, EXIT_INVALID_INVOCATION);
        assert!(
            result.stderr.contains("zero --worker") || result.stdout.contains("zero --worker"),
            "stdout={} stderr={}",
            result.stdout,
            result.stderr
        );
        assert!(!database.exists(), "preview must not open the database");
        assert!(!directory.path().join("must-not-exist.sqlite-wal").exists());
        assert!(!directory.path().join("must-not-exist.sqlite-shm").exists());
    }

    #[test]
    fn one_worker_fan_out_lists_command_args_and_unpinned_path_pi_warns_exit_0() {
        let worker = worker_json("pi", &["--print"]);
        let operand = json!({"design-review": fan_out_binding(&[worker])}).to_string();
        let result = execute(["preview-bindings", &operand]);
        assert_eq!(result.exit_code, EXIT_COMPLETED, "{}", result.stderr);
        let report: Value =
            serde_json::from_str(result.stdout.trim()).expect("preview JSON report");
        let workers = report["bindings"][0]["workers"]
            .as_array()
            .expect("workers array");
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0]["command"], "pi");
        assert_eq!(workers[0]["args"], json!(["--print"]));
        let warnings = report["warnings"].as_array().expect("warnings");
        let joined = warnings
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("no --model"), "{joined}");
        assert!(joined.contains("PATH name"), "{joined}");
        assert!(joined.contains(DEFAULT_INVOKE_TIMEOUT_WARNING), "{joined}");
        assert!(report.get("errors").is_none() || report["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn malformed_json_exits_nonzero() {
        let result = execute(["preview-bindings", "not-json"]);
        assert_eq!(result.exit_code, EXIT_INVALID_INVOCATION);
    }

    #[test]
    fn work_slot_bindings_wrapper_key_is_accepted() {
        let worker = worker_json(
            "pi",
            &[
                "--print",
                "--model",
                "grok",
                "--no-skills",
                "--no-extensions",
                "--tools",
                "read,grep,find,ls",
            ],
        );
        let operand = json!({
            "config_version": "ignored",
            "work_slot_bindings": {
                "design-review": fan_out_binding(&[worker])
            }
        })
        .to_string();
        let report = preview(&operand, false).expect("preview");
        assert!(report.errors.is_empty());
        assert_eq!(report.bindings.len(), 1);
        assert_eq!(report.bindings[0].slot_id, "design-review");
        assert_eq!(report.models, vec!["grok".to_owned()]);
        assert!(!report
            .warnings
            .iter()
            .any(|warning| warning.contains("no --model")));
        assert!(!report.warnings.iter().any(|warning| warning
            .contains("lacks --tools whose comma-separated value is exactly read,grep,find,ls")));
    }

    #[test]
    fn omitted_task_worker_is_not_invented() {
        let operand = json!({
            "implement": {
                "command": "software-change",
                "args": ["run-plan-graph"]
            }
        })
        .to_string();
        let report = preview(&operand, false).expect("preview");
        assert!(report.errors.is_empty());
        assert!(report.bindings[0].workers.is_empty());
        assert!(!report
            .warnings
            .iter()
            .any(|warning| warning.contains("pi worker")));
    }

    #[test]
    fn task_worker_json_is_expanded() {
        let worker = worker_json("pi", &["--print", "--no-skills", "--no-extensions"]);
        let operand = json!({
            "implement": {
                "command": "software-change",
                "args": ["run-plan-graph", "--task-worker", worker]
            }
        })
        .to_string();
        let report = preview(&operand, true).expect("preview");
        assert!(report.errors.is_empty());
        assert_eq!(report.bindings[0].workers.len(), 1);
        assert_eq!(report.bindings[0].workers[0].command, "pi");
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("no --model")));
        assert!(!report.warnings.iter().any(|warning| warning
            .contains("lacks --tools whose comma-separated value is exactly read,grep,find,ls")));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning == DEFAULT_INVOKE_TIMEOUT_WARNING));
    }

    #[test]
    fn timeout_ms_suppresses_default_timeout_warning() {
        let operand = json!({"explore": {"command": "/bin/echo", "args": []}}).to_string();
        let with_default = preview(&operand, true).expect("preview");
        let without = preview(&operand, false).expect("preview");
        assert!(with_default
            .warnings
            .iter()
            .any(|warning| warning == DEFAULT_INVOKE_TIMEOUT_WARNING));
        assert!(!without
            .warnings
            .iter()
            .any(|warning| warning == DEFAULT_INVOKE_TIMEOUT_WARNING));
    }

    #[test]
    fn at_file_operand_reads_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("bindings.json");
        let worker = worker_json("echo", &[]);
        fs::write(
            &path,
            json!({"design-review": fan_out_binding(&[worker])}).to_string(),
        )
        .expect("write");
        let operand = format!("@{}", path.display());
        let source = load_source(Some(&operand), Cursor::new(Vec::new())).expect("load");
        let report = preview(&source, false).expect("preview");
        assert!(report.errors.is_empty());
        assert_eq!(report.bindings[0].workers[0].command, "echo");
    }

    #[test]
    fn omitted_operand_reads_stdin() {
        let json = json!({"slot": {"command": "/bin/echo", "args": []}}).to_string();
        let source = load_source(None, Cursor::new(json.clone())).expect("load");
        assert_eq!(source, json);
    }

    #[test]
    fn extra_binding_fields_are_malformed() {
        let operand = json!({
            "slot": {"command": "echo", "args": [], "extra": true}
        })
        .to_string();
        assert!(preview(&operand, false).is_err());
    }

    #[test]
    fn array_input_is_malformed() {
        assert!(preview("[1]", false).is_err());
    }

    #[test]
    fn help_lists_preview_bindings_under_other_commands_not_operations() {
        let help = execute(["--help"]);
        assert_eq!(help.exit_code, EXIT_COMPLETED);
        let (operations, other) = help
            .stdout
            .split_once("Other commands:")
            .expect("help must have Other commands");
        assert!(operations.contains("Operations:"));
        assert!(
            !operations.contains("preview-bindings"),
            "preview-bindings must not be a ninth primary operation: {operations}"
        );
        assert!(
            other.contains("preview-bindings"),
            "preview-bindings must be under Other commands: {other}"
        );
        let command_help = execute(["preview-bindings", "--help"]);
        assert_eq!(command_help.exit_code, EXIT_COMPLETED);
        assert!(command_help.stdout.contains("preview-bindings"));
        assert!(command_help.stdout.contains("[JSON|@FILE]"));
    }

    #[test]
    fn execute_timeout_ms_suppresses_default_timeout_warning() {
        let operand = json!({"explore": {"command": "/bin/echo", "args": []}}).to_string();
        let with_default = execute(["preview-bindings", &operand]);
        assert_eq!(
            with_default.exit_code, EXIT_COMPLETED,
            "{}",
            with_default.stderr
        );
        let with_report: Value =
            serde_json::from_str(with_default.stdout.trim()).expect("preview JSON");
        let with_warnings = with_report["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            with_warnings.contains(DEFAULT_INVOKE_TIMEOUT_WARNING),
            "{with_warnings}"
        );

        let with_timeout = execute(["--timeout-ms", "60000", "preview-bindings", &operand]);
        assert_eq!(
            with_timeout.exit_code, EXIT_COMPLETED,
            "{}",
            with_timeout.stderr
        );
        let timeout_report: Value =
            serde_json::from_str(with_timeout.stdout.trim()).expect("preview JSON");
        let timeout_warnings = timeout_report["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !timeout_warnings.contains(DEFAULT_INVOKE_TIMEOUT_WARNING),
            "{timeout_warnings}"
        );
    }

    #[test]
    fn extra_operand_is_invalid_invocation() {
        let result = execute(["preview-bindings", "{}", "extra"]);
        assert_eq!(result.exit_code, EXIT_INVALID_INVOCATION);
    }

    fn joined_warnings(report: &PreviewReport) -> String {
        report.warnings.join("\n")
    }

    #[test]
    fn pi_no_extensions_without_e_warns() {
        let operand = json!({
            "implement": {
                "command": "pi",
                "args": ["--print", "--no-skills", "--no-extensions"]
            }
        })
        .to_string();
        let report = preview(&operand, false).expect("preview");
        let joined = joined_warnings(&report);
        assert!(joined.contains(PI_NO_EXTENSIONS_WITHOUT_E), "{joined}");
    }

    #[test]
    fn pi_no_extensions_with_e_does_not_warn_missing_e_or_missing_no_extensions() {
        let operand = json!({
            "implement": {
                "command": "pi",
                "args": [
                    "--print",
                    "--no-skills",
                    "--no-extensions",
                    "-e",
                    "/tmp/cursor",
                    "-e",
                    "/tmp/claude-bridge",
                    "--model",
                    "x"
                ]
            }
        })
        .to_string();
        let report = preview(&operand, false).expect("preview");
        let joined = joined_warnings(&report);
        assert!(!joined.contains(PI_NO_EXTENSIONS_WITHOUT_E), "{joined}");
        assert!(!joined.contains("lacks --no-extensions"), "{joined}");
        assert!(!joined.contains("no --model"), "{joined}");
    }

    #[test]
    fn pi_without_no_extensions_does_not_warn_about_missing_no_extensions() {
        let operand = json!({
            "implement": {
                "command": "pi",
                "args": ["--print", "--no-skills", "--model", "x"]
            }
        })
        .to_string();
        let report = preview(&operand, false).expect("preview");
        let joined = joined_warnings(&report);
        assert!(!joined.contains("lacks --no-extensions"), "{joined}");
        assert!(!joined.contains(PI_NO_EXTENSIONS_WITHOUT_E), "{joined}");
        assert!(!joined.contains("lacks --no-skills"), "{joined}");
    }

    #[test]
    fn fan_out_pi_no_extensions_without_e_exits_0_and_warns() {
        let worker = worker_json(
            "pi",
            &[
                "--print",
                "--no-skills",
                "--no-extensions",
                "--tools",
                "read,grep,find,ls",
                "--model",
                "x",
            ],
        );
        let operand = json!({"design-review": fan_out_binding(&[worker])}).to_string();
        let result = execute(["preview-bindings", &operand]);
        assert_eq!(result.exit_code, EXIT_COMPLETED, "{}", result.stderr);
        let report: Value =
            serde_json::from_str(result.stdout.trim()).expect("preview JSON report");
        let joined = report["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains(PI_NO_EXTENSIONS_WITHOUT_E), "{joined}");
    }
}
