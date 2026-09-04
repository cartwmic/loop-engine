//! Stateless subprocess implementation of the core provider gateway.
//!
//! A gateway invocation is deliberately one-shot: every `describe` and
//! `evaluate` call creates a new child process, writes one JSON request to its
//! stdin, and interprets one JSON response from stdout.  Stderr is drained so
//! diagnostics cannot block a provider, but it is never parsed as protocol.

use crate::provider::ProviderInvocation;
use loop_core::{
    AllowResponse, EvaluationFeedback, EvaluationRequest, EvaluationResult, ProviderAssociation,
    ProviderError, ProviderGateway, Workflow,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default upper bound for one provider operation.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Polling keeps the implementation synchronous without introducing another
/// runtime or a process-wait dependency.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Stateless subprocess provider gateway.
///
/// The timeout applies independently to every fresh provider process.  No
/// process, output, or provider state is retained between calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubprocessProviderGateway {
    timeout: Duration,
}

impl Default for SubprocessProviderGateway {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT)
    }
}

impl SubprocessProviderGateway {
    /// Construct a gateway with the supplied per-operation timeout.
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Construct a gateway with the supplied per-operation timeout.
    pub const fn with_timeout(timeout: Duration) -> Self {
        Self::new(timeout)
    }

    /// Return the timeout applied to each provider call.
    pub const fn timeout(self) -> Duration {
        self.timeout
    }

    /// Return the default timeout used by [`Default`].
    pub const fn default_timeout() -> Duration {
        DEFAULT_TIMEOUT
    }

    fn call(
        &self,
        provider: &ProviderAssociation,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ProviderError> {
        let invocation = ProviderInvocation::from_association(provider).map_err(|error| {
            ProviderError::execution("invalid-provider-association", error.to_string())
        })?;

        let mut command = Command::new(&invocation.command);
        command
            .args(&invocation.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            ProviderError::execution(
                "provider-spawn-failed",
                format!("could not start `{}`: {error}", invocation.command),
            )
        })?;

        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                terminate_child(&mut child);
                return Err(ProviderError::execution(
                    "provider-stdin-unavailable",
                    "provider process did not expose stdin",
                ));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_child(&mut child);
                return Err(ProviderError::execution(
                    "provider-stdout-unavailable",
                    "provider process did not expose stdout",
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_child(&mut child);
                return Err(ProviderError::execution(
                    "provider-stderr-unavailable",
                    "provider process did not expose stderr",
                ));
            }
        };

        // Writing stdin and draining both output streams concurrently prevents
        // a provider that emits diagnostics or a large response from
        // deadlocking the supervising thread.
        let writer = thread::spawn(move || write_request(stdin, request));
        let stdout_reader = thread::spawn(move || read_stream(stdout));
        let stderr_reader = thread::spawn(move || read_stream(stderr));

        let status = match wait_for_exit(&mut child, self.timeout) {
            Ok(WaitOutcome::Exited(status)) => status,
            Ok(WaitOutcome::TimedOut) => {
                let _ = join_writer(writer);
                let diagnostics = join_reader(stderr_reader).unwrap_or_default();
                let _ = join_reader(stdout_reader);
                return Err(ProviderError::timeout(timeout_message(
                    self.timeout,
                    &diagnostics,
                )));
            }
            Err(error) => {
                terminate_child(&mut child);
                let _ = join_writer(writer);
                let _ = join_reader(stdout_reader);
                let _ = join_reader(stderr_reader);
                return Err(error);
            }
        };

        let stdout = join_reader(stdout_reader)?;
        let diagnostics = join_reader(stderr_reader)?;
        let write_result = join_writer(writer)?;

        // A non-zero exit is a process failure even if the process happened to
        // emit bytes on stdout.  Stderr is diagnostic context only.
        if !status.success() {
            return Err(ProviderError::execution(
                "provider-exited-nonzero",
                process_failure_message(status, &diagnostics),
            ));
        }
        if let Err(error) = write_result {
            // A provider is allowed to emit its one response without reading
            // the request to completion (for example, a deterministic
            // protocol-error fixture).  Once it exited successfully, a
            // BrokenPipe is therefore not itself a protocol response; parse
            // stdout below and classify that response precisely.  Other write
            // failures still mean the request could not be delivered.
            if error.kind() != io::ErrorKind::BrokenPipe {
                return Err(ProviderError::execution(
                    "provider-stdin-failed",
                    format!("could not send provider request: {error}"),
                ));
            }
        }

        Ok(stdout)
    }
}

impl ProviderGateway for SubprocessProviderGateway {
    fn describe(
        &self,
        provider: &ProviderAssociation,
        initial_input: Option<&Value>,
    ) -> Result<Workflow, ProviderError> {
        let request = serde_json::to_vec(&DescribeRequest {
            operation: "describe",
            initial_input,
        })
        .map_err(|error| {
            ProviderError::execution(
                "provider-request-serialization-failed",
                format!("could not serialize describe request: {error}"),
            )
        })?;
        let stdout = self.call(provider, request)?;
        let value = parse_json_response(&stdout, "describe")?;
        serde_json::from_value(value).map_err(|error| {
            ProviderError::invalid_response(format!("describe response is not a workflow: {error}"))
        })
    }

    fn evaluate(
        &self,
        provider: &ProviderAssociation,
        request: EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError> {
        let request = EvaluateRequest::from_core(request);
        let request = serde_json::to_vec(&request).map_err(|error| {
            ProviderError::execution(
                "provider-request-serialization-failed",
                format!("could not serialize evaluate request: {error}"),
            )
        })?;
        let stdout = self.call(provider, request)?;
        let value = parse_json_response(&stdout, "evaluate")?;
        parse_evaluation_response(value)
    }
}

#[derive(Serialize)]
struct DescribeRequest<'a> {
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_input: Option<&'a Value>,
}

/// The wire envelope is intentionally flat.  It carries only the fields in
/// `EvaluationRequest`; there is no raw run-history field to accidentally
/// forward to a provider.
#[derive(Serialize)]
struct EvaluateRequest {
    operation: &'static str,
    workflow: Workflow,
    initial_input: Value,
    context: Vec<loop_core::ContextRecord>,
    transition: loop_core::Transition,
    prior_evaluations: Vec<loop_core::DurableEvaluation>,
}

impl EvaluateRequest {
    fn from_core(request: EvaluationRequest) -> Self {
        let EvaluationRequest {
            workflow,
            initial_input,
            mut context,
            transition,
            prior_evaluations,
        } = request;

        // Core normally constructs these collections in durable order.  Keep
        // the gateway defensive as it is also a public integration adapter:
        // the wire protocol must never depend on a caller's incidental vector
        // ordering, and unrelated/unchecked records must not leak into exact
        // transition lineage.
        context.sort_by_key(|record| record.sequence);
        let mut prior_evaluations = prior_evaluations
            .into_iter()
            .filter(|evaluation| {
                evaluation.transition.kind.is_checked()
                    && evaluation.transition.same_lineage(&transition)
            })
            .collect::<Vec<_>>();
        prior_evaluations.sort_by_key(|evaluation| evaluation.sequence);

        Self {
            operation: "evaluate",
            workflow,
            initial_input,
            context,
            transition,
            prior_evaluations,
        }
    }
}

fn parse_json_response(bytes: &[u8], operation: &str) -> Result<Value, ProviderError> {
    serde_json::from_slice(bytes).map_err(|error| {
        ProviderError::malformed_response(format!(
            "{operation} response is not valid JSON: {error}"
        ))
    })
}

fn parse_evaluation_response(value: Value) -> Result<EvaluationResult, ProviderError> {
    let object = value.as_object().ok_or_else(|| {
        ProviderError::invalid_response("evaluate response must be a JSON object")
    })?;
    let result = object
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderError::invalid_response(
                "evaluate response must contain a string `result` field",
            )
        })?;

    match result {
        "allow" => parse_allow_response(object),
        "unsupported" => {
            require_exact_keys(object, &["result"], "unsupported response")?;
            Ok(EvaluationResult::Unsupported)
        }
        "deny" => parse_deny_response(object),
        other => Err(ProviderError::invalid_response(format!(
            "unknown evaluate result `{other}`"
        ))),
    }
}

fn parse_allow_response(object: &Map<String, Value>) -> Result<EvaluationResult, ProviderError> {
    require_allowed_keys(object, &["result", "context_append"], "allow response")?;
    if object.is_empty() || object.len() > 2 {
        return Err(ProviderError::invalid_response(
            "allow response contains unexpected or missing fields",
        ));
    }

    let mut response = AllowResponse::empty();
    if let Some(context_append) = object.get("context_append") {
        response.context_append = Some(serde_json::from_value(context_append.clone()).map_err(
            |error| {
                ProviderError::invalid_response(format!(
                    "allow response `context_append` is invalid: {error}"
                ))
            },
        )?);
    }
    Ok(EvaluationResult::from(response))
}

fn parse_deny_response(object: &Map<String, Value>) -> Result<EvaluationResult, ProviderError> {
    require_exact_keys(object, &["result", "feedback"], "deny response")?;
    let feedback = object
        .get("feedback")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ProviderError::invalid_response("deny response `feedback` must be a JSON object")
        })?;
    require_allowed_keys(feedback, &["code", "message", "details"], "deny feedback")?;

    let code = feedback
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderError::invalid_response("deny feedback must contain a string `code` field")
        })?;
    let message = feedback
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderError::invalid_response("deny feedback must contain a string `message` field")
        })?;

    let mut evaluation_feedback = EvaluationFeedback::new(code, message);
    if let Some(details) = feedback.get("details") {
        evaluation_feedback.details = Some(details.clone());
    }
    Ok(EvaluationResult::Deny {
        feedback: evaluation_feedback,
    })
}

fn require_exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
    description: &str,
) -> Result<(), ProviderError> {
    require_allowed_keys(object, expected, description)?;
    if object.len() != expected.len() {
        return Err(ProviderError::invalid_response(format!(
            "{description} contains unexpected or duplicate fields"
        )));
    }
    Ok(())
}

fn require_allowed_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    description: &str,
) -> Result<(), ProviderError> {
    if let Some(unexpected) = object
        .keys()
        .find(|key| !allowed.iter().any(|candidate| candidate == key))
    {
        return Err(ProviderError::invalid_response(format!(
            "{description} contains unexpected field `{unexpected}`"
        )));
    }
    Ok(())
}

enum WaitOutcome {
    Exited(ExitStatus),
    TimedOut,
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<WaitOutcome, ProviderError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(WaitOutcome::Exited(status)),
            Ok(None) if started.elapsed() >= timeout => {
                terminate_child(child);
                return Ok(WaitOutcome::TimedOut);
            }
            Ok(None) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                thread::sleep(remaining.min(WAIT_POLL_INTERVAL));
            }
            Err(error) => {
                terminate_child(child);
                return Err(ProviderError::execution(
                    "provider-wait-failed",
                    format!("could not wait for provider process: {error}"),
                ));
            }
        }
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn write_request(mut stdin: impl Write, request: Vec<u8>) -> io::Result<()> {
    stdin.write_all(&request)
}

fn read_stream(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader(handle: JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>, ProviderError> {
    match handle.join() {
        Ok(result) => result.map_err(|error| {
            ProviderError::execution(
                "provider-output-read-failed",
                format!("could not read provider output: {error}"),
            )
        }),
        Err(_) => Err(ProviderError::execution(
            "provider-output-reader-panicked",
            "provider output reader terminated unexpectedly",
        )),
    }
}

fn join_writer(handle: JoinHandle<io::Result<()>>) -> Result<io::Result<()>, ProviderError> {
    match handle.join() {
        Ok(result) => Ok(result),
        Err(_) => Err(ProviderError::execution(
            "provider-input-writer-panicked",
            "provider input writer terminated unexpectedly",
        )),
    }
}

fn process_failure_message(status: ExitStatus, diagnostics: &[u8]) -> String {
    let status = status.code().map_or_else(
        || "terminated by signal".to_owned(),
        |code| format!("exit code {code}"),
    );
    let diagnostics = diagnostic_text(diagnostics);
    if diagnostics.is_empty() {
        format!("provider process {status}")
    } else {
        format!("provider process {status}; stderr: {diagnostics}")
    }
}

fn timeout_message(timeout: Duration, diagnostics: &[u8]) -> String {
    let diagnostics = diagnostic_text(diagnostics);
    if diagnostics.is_empty() {
        format!("provider did not exit within {} ms", timeout.as_millis())
    } else {
        format!(
            "provider did not exit within {} ms; stderr: {diagnostics}",
            timeout.as_millis()
        )
    }
}

fn diagnostic_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{
        ContextRecord, DurableEvaluation, EvaluationFeedback, ProviderAssociation,
        SemanticSequence, State, Timestamp, Transition, Workflow,
    };
    use serde_json::json;

    fn workflow() -> Workflow {
        Workflow::new(
            "fixture",
            "start",
            vec![
                State::new("start", "Start", "Begin", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::checked("start", "finish", "done")],
        )
    }

    #[test]
    fn evaluate_wire_request_filters_and_orders_lineage() {
        let transition = Transition::checked("start", "finish", "done");
        let unrelated = Transition::checked("start", "other", "done");
        let request = EvaluateRequest::from_core(EvaluationRequest::new(
            workflow(),
            json!({"goal": "test"}),
            vec![
                ContextRecord::new(
                    "second",
                    "note",
                    json!(2),
                    SemanticSequence::new(4),
                    Timestamp::from_unix_millis(4),
                ),
                ContextRecord::new(
                    "first",
                    "note",
                    json!(1),
                    SemanticSequence::new(2),
                    Timestamp::from_unix_millis(2),
                ),
            ],
            transition.clone(),
            vec![
                DurableEvaluation::allow(
                    transition.clone(),
                    SemanticSequence::new(8),
                    Timestamp::from_unix_millis(8),
                ),
                DurableEvaluation::deny(
                    unrelated,
                    EvaluationFeedback::new("other", "other"),
                    SemanticSequence::new(3),
                    Timestamp::from_unix_millis(3),
                ),
                DurableEvaluation::deny(
                    transition,
                    EvaluationFeedback::new("first", "first"),
                    SemanticSequence::new(5),
                    Timestamp::from_unix_millis(5),
                ),
            ],
        ));

        assert_eq!(request.context[0].id.as_str(), "first");
        assert_eq!(request.context[1].id.as_str(), "second");
        assert_eq!(request.prior_evaluations.len(), 2);
        assert_eq!(
            request.prior_evaluations[0].sequence,
            SemanticSequence::new(5)
        );
        assert_eq!(
            request.prior_evaluations[1].sequence,
            SemanticSequence::new(8)
        );
    }

    #[test]
    fn evaluation_response_requires_exact_semantic_shapes() {
        assert!(matches!(
            parse_evaluation_response(json!({"result": "allow"})),
            Ok(EvaluationResult::Allow)
        ));
        assert!(matches!(
            parse_evaluation_response(json!({"result": "unsupported"})),
            Ok(EvaluationResult::Unsupported)
        ));
        assert!(matches!(
            parse_evaluation_response(json!({
                "result": "deny",
                "feedback": {"code": "x", "message": "y", "details": null}
            })),
            Ok(EvaluationResult::Deny { .. })
        ));
        assert!(matches!(
            parse_evaluation_response(json!({"result": "allow", "details": true})),
            Err(ProviderError::InvalidResponse { .. })
        ));
        assert!(matches!(
            parse_evaluation_response(json!({
                "result": "deny",
                "feedback": {"code": "x"}
            })),
            Err(ProviderError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn allow_response_parses_opaque_context_append_and_rejects_malformed_shapes() {
        let parsed = parse_evaluation_response(json!({
            "result": "allow",
            "context_append": {
                "kind": "",
                "data": ["opaque", null]
            }
        }))
        .unwrap();
        assert_eq!(
            parsed.context_append().map(|effect| effect.kind.as_str()),
            Some("")
        );
        assert_eq!(
            parsed.context_append().unwrap().data,
            json!(["opaque", null])
        );

        for response in [
            json!({"result": "allow", "context_append": {"data": {}}}),
            json!({"result": "allow", "context_append": null}),
            json!({"result": "allow", "context_append": {"kind": "snapshot", "data": {}, "extra": true}}),
            json!({"result": "deny", "feedback": {"code": "blocked", "message": "no"}, "context_append": {"kind": "snapshot", "data": {}}}),
            json!({"result": "unsupported", "context_append": {"kind": "snapshot", "data": {}}}),
        ] {
            assert!(matches!(
                parse_evaluation_response(response),
                Err(ProviderError::InvalidResponse { .. })
            ));
        }
    }

    #[test]
    fn diagnostic_text_never_changes_protocol_data() {
        assert_eq!(diagnostic_text(b"  diagnostic\n"), "diagnostic");
    }

    #[test]
    fn invalid_association_is_execution_error() {
        let gateway = SubprocessProviderGateway::default();
        let association = ProviderAssociation::new(json!({"command": 42, "args": []}));
        let error = gateway.describe(&association, None).unwrap_err();
        assert!(matches!(error, ProviderError::Execution { .. }));
    }
}
