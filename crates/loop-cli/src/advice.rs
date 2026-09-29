//! Separate run-scoped generic advice command and immutable capture.

use super::{
    expand_tilde, new_context_id, now_timestamp, open_persistence, render_operation, resolve_paths,
    CliError, CliOptions, Execution, OutputFormat,
};
use loop_core::{
    self as core, AdviceCommandConfig, AdviceRequest, AdviceResponse, AppendAdviceAttemptRequest,
    OperationOutcome, OutcomeIssue, Persistence, RunId, ADVICE_CAPTURE_KIND,
    ADVICE_COMMAND_INPUT_KEY,
};
use loop_integrations::capture::{execute_row_with_stdin, Row};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
struct AdviceOrigin<'a> {
    kind: &'static str,
    run_id: &'a RunId,
    attempt_id: &'a str,
    context_record_id: &'a str,
    capture_dir: String,
}

#[derive(Serialize)]
struct CapturedStream<'a> {
    path: &'a str,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct AdviceOutput<'a> {
    attempt_id: &'a str,
    origin: AdviceOrigin<'a>,
    response: &'a AdviceResponse,
    capture_dir: String,
    input: CapturedStream<'a>,
    stdout: CapturedStream<'a>,
    stderr: CapturedStream<'a>,
    receipt: String,
}

struct AttemptCapture {
    root: PathBuf,
    input_relative: String,
    stdout_relative: String,
    stderr_relative: String,
    receipt_relative: String,
}

fn error<T>(code: impl Into<String>, message: impl Into<String>) -> OperationOutcome<T> {
    OperationOutcome::error(code, message)
}

fn rejected<T>(code: impl Into<String>, message: impl Into<String>) -> OperationOutcome<T> {
    OperationOutcome::rejected(code, message)
}

fn render<T: Serialize>(output: OutputFormat, outcome: &OperationOutcome<T>) -> Execution {
    render_operation("advise", output, outcome)
}

pub(super) fn execute(
    options: CliOptions,
    run_id: RunId,
    request_source: String,
    _operation_started_at: std::time::Instant,
) -> Execution {
    let output = options.output;
    let paths = match resolve_paths(&options) {
        Ok(paths) => paths,
        Err(error) => return super::render_operation_error("advise", output, error),
    };
    let persistence = match open_persistence(&paths.database) {
        Ok(persistence) => persistence,
        Err(error) => return super::render_operation_error("advise", output, error),
    };
    let run = match persistence.load_authoritative_run(&run_id) {
        Ok(run) => run,
        Err(error) => {
            return render(
                output,
                &core::OperationOutcome::<AdviceOutput>::error(error.code(), error.to_string()),
            )
        }
    };
    let Some(config_value) = run
        .initial_input
        .as_object()
        .and_then(|input| input.get(ADVICE_COMMAND_INPUT_KEY))
    else {
        return render(
            output,
            &rejected::<AdviceOutput>(
                "advice-disabled",
                format!("run `{run_id}` has no frozen `{ADVICE_COMMAND_INPUT_KEY}` command"),
            ),
        );
    };
    let config = match serde_json::from_value::<AdviceCommandConfig>(config_value.clone()) {
        Ok(config) => config,
        Err(error) => {
            return render(
                output,
                &rejected::<AdviceOutput>(
                    "invalid-advice-command",
                    format!("frozen advice command configuration is invalid: {error}"),
                ),
            )
        }
    };
    if let Err(message) = config.validate() {
        return render(
            output,
            &rejected::<AdviceOutput>("invalid-advice-command", message),
        );
    }
    let artifact_root = match persistence.load_run_artifact_root(&run_id) {
        Ok(Some(path)) if !path.trim().is_empty() => expand_tilde(Path::new(&path)),
        Ok(_) => {
            return render(
                output,
                &error::<AdviceOutput>(
                    "advice-capture-root-unavailable",
                    format!(
                        "run `{run_id}` has no recorded artifact root for immutable advice capture"
                    ),
                ),
            )
        }
        Err(error) => {
            return render(
                output,
                &core::OperationOutcome::<AdviceOutput>::error(error.code(), error.to_string()),
            )
        }
    };

    let request_bytes = match read_request_source(&request_source) {
        Ok(bytes) => bytes,
        Err(error) => {
            return super::render_operation_error(
                "advise",
                output,
                CliError::new("advice-input-read-failed", error.to_string()),
            )
        }
    };
    let attempt_id = new_context_id().replace("context-", "advice-");
    let capture = match create_capture(&artifact_root, &attempt_id, &request_bytes) {
        Ok(capture) => capture,
        Err(error) => {
            return render(
                output,
                &core::OperationOutcome::<AdviceOutput>::error(
                    "advice-capture-create-failed",
                    format!("could not create run-owned advice capture: {error}"),
                ),
            )
        }
    };
    let argv = std::iter::once(config.command.clone())
        .chain(config.args.iter().cloned())
        .collect::<Vec<_>>();
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            return persist_failure(
                output,
                &persistence,
                &run_id,
                &attempt_id,
                &config,
                &argv,
                &request_bytes,
                &capture,
                None,
                None,
                Some(format!("could not determine current directory: {error}")),
                Some("advice-cwd-unavailable".to_owned()),
                None,
                Some(false),
            )
        }
    };
    if request_bytes.len() as u64 > config.max_request_bytes {
        let message = format!(
            "advice request is {} bytes; frozen limit is {}",
            request_bytes.len(),
            config.max_request_bytes
        );
        let _ = create_refused_attempt(&capture, &argv, &cwd, &request_bytes, &config, &message);
        return persist_failure(
            output,
            &persistence,
            &run_id,
            &attempt_id,
            &config,
            &argv,
            &request_bytes,
            &capture,
            None,
            Some(&cwd),
            Some(message),
            Some("advice-request-too-large".to_owned()),
            None,
            Some(false),
        );
    }
    let request = match AdviceRequest::parse(&request_bytes) {
        Ok(request) => Some(request),
        Err(message) => {
            let _ =
                create_refused_attempt(&capture, &argv, &cwd, &request_bytes, &config, &message);
            return persist_failure(
                output,
                &persistence,
                &run_id,
                &attempt_id,
                &config,
                &argv,
                &request_bytes,
                &capture,
                None,
                Some(&cwd),
                Some(message),
                Some("invalid-advice-request".to_owned()),
                None,
                Some(false),
            );
        }
    };
    let request = request.expect("validated request is present");

    let row = Row {
        id: attempt_id.clone(),
        argv: argv.clone(),
        environment: BTreeMap::new(),
        inherit_environment: Vec::new(),
        timeout_ms: config.timeout_ms,
        obligations: vec!["generic typed advice response".to_owned()],
    };
    let attempt_dir = capture.root.join("attempts").join("1");
    let receipt_result =
        execute_row_with_stdin(&row, &cwd, &attempt_dir, &capture.root, &request_bytes);
    let receipt = match receipt_result {
        Ok(receipt) => Some(receipt),
        Err(error) => {
            let command_started = captured_command_started(&capture.root);
            return persist_failure(
                output,
                &persistence,
                &run_id,
                &attempt_id,
                &config,
                &argv,
                &request_bytes,
                &capture,
                None,
                Some(&cwd),
                Some(format!("capture executor failed: {error}")),
                Some("advice-capture-execution-failed".to_owned()),
                Some(&request),
                command_started,
            );
        }
    };
    let stdout_bytes = fs::read(attempt_dir.join("stdout")).unwrap_or_default();
    let process_error = process_failure(
        receipt.as_ref().expect("receipt was captured"),
        &config,
        stdout_bytes.len() as u64,
    );
    if let Some(message) = process_error {
        return persist_failure(
            output,
            &persistence,
            &run_id,
            &attempt_id,
            &config,
            &argv,
            &request_bytes,
            &capture,
            receipt.as_ref(),
            Some(&cwd),
            Some(message.clone()),
            Some("advice-command-failed".to_owned()),
            Some(&request),
            Some(true),
        );
    }
    let response = match AdviceResponse::parse(&stdout_bytes) {
        Ok(response) => response,
        Err(message) => {
            return persist_failure(
                output,
                &persistence,
                &run_id,
                &attempt_id,
                &config,
                &argv,
                &request_bytes,
                &capture,
                receipt.as_ref(),
                Some(&cwd),
                Some(message),
                Some("invalid-advice-response".to_owned()),
                Some(&request),
                Some(true),
            )
        }
    };
    if let Err(message) = request.validate_response(&response) {
        return persist_failure(
            output,
            &persistence,
            &run_id,
            &attempt_id,
            &config,
            &argv,
            &request_bytes,
            &capture,
            receipt.as_ref(),
            Some(&cwd),
            Some(message),
            Some("invalid-advice-response".to_owned()),
            Some(&request),
            Some(true),
        );
    }

    let origin = AdviceOrigin {
        kind: ADVICE_CAPTURE_KIND,
        run_id: &run_id,
        attempt_id: &attempt_id,
        context_record_id: &attempt_id,
        capture_dir: capture.root.to_string_lossy().into_owned(),
    };
    let typed_result = match serde_json::to_value(&response) {
        Ok(value) => value,
        Err(error) => {
            return persist_failure(
                output,
                &persistence,
                &run_id,
                &attempt_id,
                &config,
                &argv,
                &request_bytes,
                &capture,
                receipt.as_ref(),
                Some(&cwd),
                Some(format!(
                    "could not serialize validated advice result: {error}"
                )),
                Some("advice-result-serialization-failed".to_owned()),
                Some(&request),
                Some(true),
            )
        }
    };
    if let Err(error) = write_immutable_json(&capture.root.join("typed-result.json"), &typed_result)
    {
        return persist_failure(
            output,
            &persistence,
            &run_id,
            &attempt_id,
            &config,
            &argv,
            &request_bytes,
            &capture,
            receipt.as_ref(),
            Some(&cwd),
            Some(format!("could not retain typed advice result: {error}")),
            Some("advice-capture-write-failed".to_owned()),
            Some(&request),
            Some(true),
        );
    }
    let data = attempt_data(
        &run_id,
        &attempt_id,
        &config,
        &argv,
        &request_bytes,
        &capture,
        Some(&request),
        receipt.as_ref(),
        Some(typed_result),
        None,
        Some(true),
        Some(&cwd),
    );
    if let Err(error) = persistence.append_advice_attempt(AppendAdviceAttemptRequest::new(
        run_id.clone(),
        attempt_id.clone(),
        data,
        now_timestamp(),
    )) {
        let issue = OutcomeIssue::new(error.code(), error.to_string()).with_details(json!({
            "attempt_id": attempt_id,
            "capture_dir": capture.root,
            "origin": origin,
        }));
        return render(
            output,
            &OperationOutcome::<AdviceOutput>::error_with_issue(issue),
        );
    }

    let result = AdviceOutput {
        attempt_id: &attempt_id,
        origin,
        response: &response,
        capture_dir: capture.root.to_string_lossy().into_owned(),
        input: stream(&capture.input_relative, &request_bytes),
        stdout: stream(&capture.stdout_relative, &stdout_bytes),
        stderr: stream(
            &capture.stderr_relative,
            &fs::read(capture.root.join(&capture.stderr_relative)).unwrap_or_default(),
        ),
        receipt: capture.receipt_relative.clone(),
    };
    render(output, &OperationOutcome::completed(result))
}

#[expect(
    clippy::too_many_arguments,
    reason = "failure recording needs the distinct capture, request, receipt and command-start facts"
)]
fn persist_failure(
    output: OutputFormat,
    persistence: &impl Persistence,
    run_id: &RunId,
    attempt_id: &str,
    config: &AdviceCommandConfig,
    argv: &[String],
    request_bytes: &[u8],
    capture: &AttemptCapture,
    receipt: Option<&Value>,
    cwd: Option<&Path>,
    message: Option<String>,
    code: Option<String>,
    request: Option<&AdviceRequest>,
    command_started: Option<bool>,
) -> Execution {
    let stdout_bytes = fs::read(capture.root.join(&capture.stdout_relative)).unwrap_or_default();
    let stderr_bytes = fs::read(capture.root.join(&capture.stderr_relative)).unwrap_or_default();
    let typed_result = Value::Null;
    let _ = write_immutable_json(&capture.root.join("typed-result.json"), &typed_result);
    let data = attempt_data(
        run_id,
        attempt_id,
        config,
        argv,
        request_bytes,
        capture,
        request,
        receipt,
        None,
        message.clone(),
        command_started,
        cwd,
    );
    if let Err(error) = persistence.append_advice_attempt(AppendAdviceAttemptRequest::new(
        run_id.clone(),
        attempt_id.to_owned(),
        data,
        now_timestamp(),
    )) {
        let issue = OutcomeIssue::new(error.code(), error.to_string()).with_details(json!({
            "attempt_id": attempt_id,
            "capture_dir": capture.root,
            "advice_failure": message,
            "capture_write_error": error.to_string(),
        }));
        return render(
            output,
            &OperationOutcome::<AdviceOutput>::error_with_issue(issue),
        );
    }
    let details = json!({
        "attempt_id": attempt_id,
        "origin": {"kind": ADVICE_CAPTURE_KIND, "run_id": run_id, "attempt_id": attempt_id, "context_record_id": attempt_id},
        "capture_dir": capture.root,
        "input": capture.input_relative,
        "stdout": capture.stdout_relative,
        "stderr": capture.stderr_relative,
        "receipt": capture.receipt_relative,
        "command_started": command_started,
        "stdout_bytes": stdout_bytes.len(),
        "stderr_bytes": stderr_bytes.len(),
        "exit_code": receipt.and_then(|receipt| receipt["exit_code"].as_i64()),
        "timed_out": receipt.and_then(|receipt| receipt["timed_out"].as_bool()),
    });
    let issue = OutcomeIssue::new(
        code.unwrap_or_else(|| "advice-command-failed".to_owned()),
        message.unwrap_or_else(|| "advice attempt failed".to_owned()),
    )
    .with_details(details);
    render(
        output,
        &OperationOutcome::<AdviceOutput>::error_with_issue(issue),
    )
}

fn captured_command_started(capture_root: &Path) -> Option<bool> {
    let state = fs::read(capture_root.join("state.json")).ok()?;
    let state: Value = serde_json::from_slice(&state).ok()?;
    Some(state["root_pid"].as_u64().is_some())
}

fn process_failure(
    receipt: &Value,
    config: &AdviceCommandConfig,
    stdout_bytes: u64,
) -> Option<String> {
    if receipt["timed_out"] == true {
        return Some(format!(
            "advice command timed out after {}ms",
            config.timeout_ms
        ));
    }
    if receipt["aborted"] == true {
        return Some("advice command capture was aborted".to_owned());
    }
    if receipt["cleanup"] != "complete" {
        return Some("advice command capture cleanup is unresolved".to_owned());
    }
    for field in ["spawn_error", "capture_error", "stdin_error"] {
        if let Some(message) = receipt[field]
            .as_str()
            .filter(|message| !message.is_empty())
        {
            return Some(format!("advice command {field}: {message}"));
        }
    }
    if receipt["exit_code"].as_i64() != Some(0) {
        return Some(format!(
            "advice command exited with {:?}",
            receipt["exit_code"]
        ));
    }
    if stdout_bytes > config.max_response_bytes {
        return Some(format!(
            "advice response is {stdout_bytes} bytes; frozen limit is {}",
            config.max_response_bytes
        ));
    }
    None
}

fn read_request_source(source: &str) -> io::Result<Vec<u8>> {
    let path = source
        .strip_prefix('@')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "expected @REQUEST_FILE"))?;
    fs::read(expand_tilde(Path::new(path)))
}

fn create_capture(
    root: &Path,
    attempt_id: &str,
    request_bytes: &[u8],
) -> io::Result<AttemptCapture> {
    fs::create_dir_all(root)?;
    let advice_root = root.join("advice");
    fs::create_dir_all(&advice_root)?;
    let root = advice_root.join(attempt_id);
    fs::create_dir(&root)?;
    fs::create_dir(root.join("attempts"))?;
    write_immutable_bytes(&root.join("request.json"), request_bytes)?;
    Ok(AttemptCapture {
        root,
        input_relative: "request.json".to_owned(),
        stdout_relative: "attempts/1/stdout".to_owned(),
        stderr_relative: "attempts/1/stderr".to_owned(),
        receipt_relative: "attempts/1/receipt.json".to_owned(),
    })
}

fn create_refused_attempt(
    capture: &AttemptCapture,
    argv: &[String],
    cwd: &Path,
    request_bytes: &[u8],
    config: &AdviceCommandConfig,
    reason: &str,
) -> io::Result<()> {
    let attempt = capture.root.join("attempts/1");
    fs::create_dir_all(&attempt)?;
    write_immutable_bytes(&attempt.join("stdin"), request_bytes)?;
    write_immutable_bytes(&attempt.join("stdout"), b"")?;
    write_immutable_bytes(&attempt.join("stderr"), b"")?;
    let started_at = timestamp_seconds();
    let started = json!({
        "id": attempt_id_from_root(&capture.root),
        "argv": argv,
        "cwd": cwd,
        "settings": {"timeout_ms": config.timeout_ms, "max_request_bytes": config.max_request_bytes, "max_response_bytes": config.max_response_bytes},
        "stdin": "stdin",
        "stdin_bytes": request_bytes.len(),
        "stdin_sha256": digest(request_bytes),
        "started_at": started_at,
        "not_executed": reason,
    });
    write_immutable_json(&attempt.join("started.json"), &started)?;
    let stdout = b"";
    let stderr = b"";
    let receipt = json!({
        "id": attempt_id_from_root(&capture.root),
        "argv": argv,
        "cwd": cwd,
        "settings": {"timeout_ms": config.timeout_ms, "max_request_bytes": config.max_request_bytes, "max_response_bytes": config.max_response_bytes},
        "started_at": started_at,
        "finished_at": timestamp_seconds(),
        "wall_seconds": 0,
        "exit_code": null,
        "signal": null,
        "timed_out": false,
        "aborted": false,
        "spawn_error": null,
        "capture_error": null,
        "cleanup": "complete",
        "stdout": "stdout",
        "stderr": "stderr",
        "stdout_sha256": digest(stdout),
        "stderr_sha256": digest(stderr),
        "stdin": "stdin",
        "stdin_bytes": request_bytes.len(),
        "stdin_sha256": digest(request_bytes),
        "stdin_error": null,
        "not_executed": reason,
    });
    write_immutable_json(&attempt.join("receipt.json"), &receipt)
}

fn attempt_id_from_root(root: &Path) -> &str {
    root.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("advice-attempt")
}

#[expect(
    clippy::too_many_arguments,
    reason = "persisted advice attempts retain independent command, capture, receipt and typed-result facts"
)]
fn attempt_data(
    run_id: &RunId,
    attempt_id: &str,
    config: &AdviceCommandConfig,
    argv: &[String],
    request_bytes: &[u8],
    capture: &AttemptCapture,
    request: Option<&AdviceRequest>,
    receipt: Option<&Value>,
    typed_result: Option<Value>,
    failure: Option<String>,
    command_started: Option<bool>,
    cwd: Option<&Path>,
) -> Value {
    let stdout_bytes = fs::read(capture.root.join(&capture.stdout_relative)).unwrap_or_default();
    let stderr_bytes = fs::read(capture.root.join(&capture.stderr_relative)).unwrap_or_default();
    let origin = json!({"kind": ADVICE_CAPTURE_KIND, "run_id": run_id, "attempt_id": attempt_id, "context_record_id": attempt_id, "capture_dir": capture.root});
    let typed_result_origin = if typed_result.is_some() {
        origin.clone()
    } else {
        Value::Null
    };
    json!({
        "schema_version": 1,
        "attempt_id": attempt_id,
        "origin": origin,
        "typed_result_origin": typed_result_origin,
        "capture_dir": capture.root,
        "status": if failure.is_some() {"failed"} else {"completed"},
        "request": request,
        "command": config.command,
        "argv": argv,
        "limits": {"timeout_ms": config.timeout_ms, "request_bytes": config.max_request_bytes, "response_bytes": config.max_response_bytes},
        "command_started": command_started,
        "cwd": cwd,
        "input": {"path": capture.input_relative, "bytes": request_bytes.len(), "sha256": digest(request_bytes)},
        "stdout": {"path": capture.stdout_relative, "bytes": stdout_bytes.len(), "sha256": digest(&stdout_bytes)},
        "stderr": {"path": capture.stderr_relative, "bytes": stderr_bytes.len(), "sha256": digest(&stderr_bytes)},
        "receipt": capture.receipt_relative,
        "exit_code": receipt.and_then(|receipt| receipt["exit_code"].as_i64()),
        "timed_out": receipt.and_then(|receipt| receipt["timed_out"].as_bool()),
        "typed_result": typed_result,
        "validation_error": failure,
    })
}

fn stream<'a>(relative: &'a str, bytes: &[u8]) -> CapturedStream<'a> {
    CapturedStream {
        path: relative,
        bytes: bytes.len() as u64,
        sha256: digest(bytes),
    }
}

fn write_immutable_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn write_immutable_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default()
}
