use loop_core::{
    ContextRecord, DurableEvaluation, EvaluationFeedback, EvaluationRequest, EvaluationResult,
    ProviderAssociation, ProviderError, ProviderGateway, SemanticSequence, State, Timestamp,
    Transition, Workflow,
};
use loop_integrations::{ProviderInvocation, SubprocessProviderGateway};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};
use tempfile::{tempdir, TempDir};

const WORKFLOW_JSON: &str = r#"{"id":"fixture","initial_state":"start","states":[{"id":"start","title":"Start","instructions":"Begin","final":false},{"id":"done","title":"Done","instructions":"Finished","final":true}],"transitions":[{"source":"start","event":"finish","target":"done","kind":"checked"}]}"#;

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

fn context(id: &str, sequence: u64, value: i64) -> ContextRecord {
    ContextRecord::new(
        id,
        "observation",
        json!({"value": value}),
        SemanticSequence::new(sequence),
        Timestamp::from_unix_millis(sequence as i64),
    )
}

fn association(args: &[String]) -> ProviderAssociation {
    ProviderInvocation::new("/bin/sh", args.iter().cloned()).to_association()
}

fn fixture_with_script(
    _directory: &TempDir,
    script: &str,
    extra_args: &[String],
) -> ProviderAssociation {
    let mut args = vec!["-c".to_owned(), script.to_owned(), "fixture".to_owned()];
    args.extend(extra_args.iter().cloned());
    association(&args)
}

fn capture_response(
    directory: &TempDir,
    response: &str,
) -> (ProviderAssociation, std::path::PathBuf) {
    let request_path = directory.path().join("request.json");
    let script = r#"cat > "$1"; printf '%s' "$2""#;
    let provider = fixture_with_script(
        directory,
        script,
        &[
            request_path.to_string_lossy().into_owned(),
            response.to_owned(),
        ],
    );
    (provider, request_path)
}

fn captured_request(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("fixture captured stdin"))
        .expect("fixture received JSON")
}

#[test]
fn describe_sends_only_operation_and_receives_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let (provider, request_path) = capture_response(&directory, WORKFLOW_JSON);
    let gateway = SubprocessProviderGateway::new(Duration::from_secs(2));

    let described = gateway.describe(&provider)?;

    assert_eq!(described, workflow());
    assert_eq!(
        captured_request(&request_path),
        json!({"operation": "describe"})
    );
    Ok(())
}

#[test]
fn evaluate_sends_stored_inputs_ordered_context_and_exact_lineage_only(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let (provider, request_path) = capture_response(&directory, r#"{"result":"allow"}"#);
    let gateway = SubprocessProviderGateway::new(Duration::from_secs(2));
    let selected = Transition::checked("start", "finish", "done");
    let unrelated = Transition::checked("start", "other", "done");
    let prior = DurableEvaluation::deny(
        selected.clone(),
        EvaluationFeedback::new("needs-work", "Fix the issue"),
        SemanticSequence::new(7),
        Timestamp::from_unix_millis(7),
    );
    let unrelated_evaluation = DurableEvaluation::allow(
        unrelated,
        SemanticSequence::new(3),
        Timestamp::from_unix_millis(3),
    );
    let request = EvaluationRequest::new(
        workflow(),
        json!({"objective": "preserve"}),
        vec![context("second", 5, 2), context("first", 2, 1)],
        selected.clone(),
        vec![prior.clone(), unrelated_evaluation],
    );

    let result = gateway.evaluate(&provider, request)?;

    assert_eq!(result, EvaluationResult::Allow);
    let wire = captured_request(&request_path);
    assert_eq!(wire["operation"], "evaluate");
    assert_eq!(wire["workflow"], serde_json::to_value(workflow())?);
    assert_eq!(wire["initial_input"], json!({"objective": "preserve"}));
    assert_eq!(
        wire["context"],
        serde_json::to_value(vec![context("first", 2, 1), context("second", 5, 2)])?
    );
    assert_eq!(wire["transition"], serde_json::to_value(selected)?);
    assert_eq!(
        wire["prior_evaluations"],
        json!([serde_json::to_value(prior)?])
    );
    assert!(wire.get("history").is_none());
    assert!(wire.get("raw_history").is_none());
    assert!(wire.get("run").is_none());
    Ok(())
}

#[test]
fn recognize_allow_deny_without_details_deny_with_opaque_details_and_unsupported(
) -> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (r#"{"result":"allow"}"#, EvaluationResult::Allow),
        (
            r#"{"result":"deny","feedback":{"code":"blocked","message":"Needs work"}}"#,
            EvaluationResult::Deny {
                feedback: EvaluationFeedback::new("blocked", "Needs work"),
            },
        ),
        (
            r#"{"result":"deny","feedback":{"code":"blocked","message":"Needs work","details":{"axis":["security",true]}}}"#,
            EvaluationResult::Deny {
                feedback: EvaluationFeedback::new("blocked", "Needs work")
                    .with_details(json!({"axis": ["security", true]})),
            },
        ),
        (r#"{"result":"unsupported"}"#, EvaluationResult::Unsupported),
    ];

    for (response, expected) in cases {
        let directory = tempdir()?;
        let (provider, _) = capture_response(&directory, response);
        let gateway = SubprocessProviderGateway::new(Duration::from_secs(2));
        let request = EvaluationRequest::new(
            workflow(),
            json!({}),
            Vec::new(),
            Transition::checked("start", "finish", "done"),
            Vec::new(),
        );
        assert_eq!(gateway.evaluate(&provider, request)?, expected);
    }
    Ok(())
}

#[test]
fn stderr_is_diagnostic_and_not_protocol_data() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let request_path = directory.path().join("request.json");
    let script = r#"cat > "$1"; printf '%s' 'diagnostic' >&2; printf '%s' '{"result":"allow"}'"#;
    let provider = fixture_with_script(
        &directory,
        script,
        &[request_path.to_string_lossy().into_owned()],
    );

    let result = SubprocessProviderGateway::new(Duration::from_secs(2)).evaluate(
        &provider,
        EvaluationRequest::new(
            workflow(),
            json!({}),
            Vec::new(),
            Transition::checked("start", "finish", "done"),
            Vec::new(),
        ),
    )?;

    assert_eq!(result, EvaluationResult::Allow);
    Ok(())
}

#[test]
fn non_zero_exit_is_execution_error() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let script = r#"cat >/dev/null; printf '%s' 'failure diagnostics' >&2; exit 23"#;
    let provider = fixture_with_script(&directory, script, &[]);
    let error = SubprocessProviderGateway::new(Duration::from_secs(2))
        .evaluate(
            &provider,
            EvaluationRequest::new(
                workflow(),
                json!({}),
                Vec::new(),
                Transition::checked("start", "finish", "done"),
                Vec::new(),
            ),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ProviderError::Execution { ref code, ref message }
            if code == "provider-exited-nonzero" && message.contains("failure diagnostics")
    ));
    Ok(())
}

#[test]
fn malformed_json_and_invalid_protocol_are_distinguished() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempdir()?;
    let (malformed_provider, _) = capture_response(&directory, "{");
    let gateway = SubprocessProviderGateway::new(Duration::from_secs(2));
    let request = EvaluationRequest::new(
        workflow(),
        json!({}),
        Vec::new(),
        Transition::checked("start", "finish", "done"),
        Vec::new(),
    );
    let malformed = gateway
        .evaluate(&malformed_provider, request.clone())
        .unwrap_err();
    assert!(matches!(malformed, ProviderError::MalformedResponse { .. }));

    let directory = tempdir()?;
    let (invalid_provider, _) = capture_response(&directory, r#"{"result":"allow","extra":true}"#);
    let invalid = gateway.evaluate(&invalid_provider, request).unwrap_err();
    assert!(matches!(invalid, ProviderError::InvalidResponse { .. }));
    Ok(())
}

#[test]
fn timeout_terminates_an_unresponsive_provider() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let script = r#"cat >/dev/null; while :; do :; done"#;
    let provider = fixture_with_script(&directory, script, &[]);
    let gateway = SubprocessProviderGateway::new(Duration::from_millis(40));

    let error = gateway
        .evaluate(
            &provider,
            EvaluationRequest::new(
                workflow(),
                json!({}),
                Vec::new(),
                Transition::checked("start", "finish", "done"),
                Vec::new(),
            ),
        )
        .unwrap_err();

    assert!(matches!(error, ProviderError::Timeout { .. }));
    Ok(())
}

#[test]
fn every_gateway_call_starts_a_fresh_process() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let request_path = directory.path().join("request.json");
    let count_path = directory.path().join("count");
    let script = r#"cat > "$1"; count=0; if [ -f "$2" ]; then count=$(cat "$2"); fi; count=$((count + 1)); printf '%s' "$count" > "$2"; if grep -q '"operation":"describe"' "$1"; then printf '%s' '{"id":"fixture","initial_state":"start","states":[{"id":"start","title":"Start","instructions":"Begin","final":false},{"id":"done","title":"Done","instructions":"Finished","final":true}],"transitions":[{"source":"start","event":"finish","target":"done","kind":"checked"}]}'; else printf '%s' '{"result":"allow"}'; fi"#;
    let provider = fixture_with_script(
        &directory,
        script,
        &[
            request_path.to_string_lossy().into_owned(),
            count_path.to_string_lossy().into_owned(),
        ],
    );
    let gateway = SubprocessProviderGateway::new(Duration::from_secs(2));

    gateway.describe(&provider)?;
    gateway.evaluate(
        &provider,
        EvaluationRequest::new(
            workflow(),
            json!({}),
            Vec::new(),
            Transition::checked("start", "finish", "done"),
            Vec::new(),
        ),
    )?;

    assert_eq!(fs::read_to_string(count_path)?, "2");
    Ok(())
}
