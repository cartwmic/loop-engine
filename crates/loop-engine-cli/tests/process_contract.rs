//! Byte-level stdout/stderr and exit-code contract tests (T128).

#[path = "../src/diagnostics.rs"]
pub mod diagnostics;

#[path = "../src/render/mod.rs"]
pub mod render;

#[path = "../src/exit.rs"]
pub mod exit;

use std::path::PathBuf;

use diagnostics::{
    InvocationCorrelation, PreDispatchPhase, SCHEMA_VERSION, configuration_failure, parse_failure,
    trace_init_failure, usage_failure,
};
use exit::{
    ByteDestination, EXIT_COMPLETED, EXIT_ERROR, EXIT_PRE_DISPATCH, EXIT_REJECTED, ProcessOutput,
    RenderFormat, exit_code_for_outcome, finalize_dispatched_outcome,
    finalize_dispatched_render_failure, finalize_driver_help, finalize_driver_list_operations,
    finalize_driver_version, finalize_pre_dispatch,
};
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::ids::{EventId, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{
    EvidenceRecordedStatus, OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot,
};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::requestable::RequestableEvent;
use loop_engine_core::operations::catalog::OperationId;
use loop_engine_integrations::configuration::ConfigurationError;
use loop_engine_integrations::trace::TraceError;
use render::dto::OutcomeRenderRequest;
use render::human::render_human_outcome;
use render::json::render_structured_outcome;
use serde_json::Value;

const REQUEST_ID: &str = "01J9X3K2M4N5P6Q7R8S9T0V1W";
const TRACE_PATH: &str =
    "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl";
const VERSION: &str = "0.1.0";

fn correlation() -> InvocationCorrelation {
    InvocationCorrelation::with_trace(REQUEST_ID, TRACE_PATH)
}

fn sample_operations() -> [(&'static str, &'static str); 2] {
    [
        ("provider.add", "provider add <HANDLE>"),
        ("run.show", "run show <RUN-ID>"),
    ]
}

fn sample_usage() -> &'static str {
    "loop-engine — workflow control plane\n\nUsage:\n  loop-engine [OPTIONS]\n"
}

fn completed_outcome() -> PublicOutcome {
    let run = RunSnapshot {
        run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
        label: Some("checkout-redesign".into()),
        lifecycle: Lifecycle::Active,
        current_state: StateId::parse("explore").unwrap(),
        state_changed: false,
    };
    let requestable = vec![RequestableEvent {
        event: EventId::parse("intent-ready").unwrap(),
        target: StateId::parse("explore").unwrap(),
        required_gates: vec![],
    }];
    PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(Some(run), Some(requestable), None).unwrap(),
        vec![],
    )
    .unwrap()
}

fn rejected_outcome() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Rejected,
        Some(Reason::new(ReasonCode::GateFailed, "One or more required gates failed").unwrap()),
        OutcomeData::new(
            None,
            None,
            Some(EvidenceRecordedStatus {
                inline: true,
                selected_associations: true,
                provider: true,
            }),
        )
        .unwrap(),
        vec![],
    )
    .unwrap()
}

fn error_outcome() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Error,
        Some(
            Reason::new(
                ReasonCode::ProviderTimeout,
                "Provider process exceeded configured timeout",
            )
            .unwrap(),
        ),
        OutcomeData::new(None, None, None).unwrap(),
        vec![
            Diagnostic::new(
                "provider.invocation",
                "Role describe timed out after 60 seconds",
                None,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn assert_empty_stderr(output: &ProcessOutput) {
    assert!(
        output.stderr.is_empty(),
        "expected empty stderr, got {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_empty_stdout(output: &ProcessOutput) {
    assert!(
        output.stdout.is_empty(),
        "expected empty stdout, got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

fn assert_ends_with_single_newline(bytes: &[u8]) {
    assert!(bytes.ends_with(b"\n"), "payload must end with a newline");
    assert!(
        !bytes.ends_with(b"\n\n"),
        "payload must not end with duplicate newlines"
    );
}

fn assert_exactly_one_trailing_newline(bytes: &[u8]) {
    assert!(
        bytes.ends_with(b"\n"),
        "payload must end with exactly one newline terminator"
    );
    assert_eq!(
        bytes.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "payload must contain exactly one newline"
    );
}

fn assert_single_json_line(bytes: &[u8]) {
    assert_exactly_one_trailing_newline(bytes);
    let text = std::str::from_utf8(bytes).expect("utf-8 payload");
    let object = text.trim_end_matches('\n');
    assert!(
        !object.is_empty(),
        "structured payload must not be an empty object line"
    );
    serde_json::from_str::<Value>(object).expect("single parseable JSON object");
}

fn parse_json_line(bytes: &[u8]) -> Value {
    assert_single_json_line(bytes);
    let text = std::str::from_utf8(bytes).expect("utf-8 payload");
    serde_json::from_str(text.trim_end_matches('\n')).expect("json line")
}

fn assert_process_contract(
    output: &ProcessOutput,
    expected_exit: i32,
    stdout: Option<&[u8]>,
    stderr: Option<&[u8]>,
) {
    assert_eq!(output.exit_code, expected_exit);
    if let Some(expected) = stdout {
        assert_eq!(output.stdout, expected);
    }
    if let Some(expected) = stderr {
        assert_eq!(output.stderr, expected);
    }
}

#[test]
fn exit_code_mapping_for_all_public_outcome_classes() {
    assert_eq!(
        exit_code_for_outcome(OutcomeClass::Completed),
        EXIT_COMPLETED
    );
    assert_eq!(exit_code_for_outcome(OutcomeClass::Rejected), EXIT_REJECTED);
    assert_eq!(exit_code_for_outcome(OutcomeClass::Error), EXIT_ERROR);
}

#[test]
fn dispatched_completed_json_is_single_stdout_object_exit_zero() {
    let outcome = completed_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Json, &request).unwrap();
    assert_process_contract(&output, EXIT_COMPLETED, None, Some(&[]));
    assert_eq!(output.stdout_destination(), Some(ByteDestination::Stdout));
    assert_eq!(output.stderr_destination(), None);
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert_eq!(value["outcome"], "completed");
    assert!(value["reason"].is_null());
    assert_eq!(value["operation"], "run.show");
}

#[test]
fn dispatched_rejected_json_is_single_stdout_object_exit_two() {
    let outcome = rejected_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.request").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Json, &request).unwrap();
    assert_process_contract(&output, EXIT_REJECTED, None, Some(&[]));
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["outcome"], "rejected");
    assert_eq!(value["reason"]["code"], "gate.failed");
}

#[test]
fn dispatched_error_json_is_single_stdout_object_exit_one() {
    let outcome = error_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.create").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Json, &request).unwrap();
    assert_process_contract(&output, EXIT_ERROR, None, Some(&[]));
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["outcome"], "error");
    assert_eq!(value["reason"]["code"], "provider.timeout");
    assert_eq!(value["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn dispatched_completed_human_is_stdout_only_exit_zero() {
    let outcome = completed_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Human, &request).unwrap();
    assert_process_contract(&output, EXIT_COMPLETED, None, Some(&[]));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("Operation: run.show\n"));
    assert!(text.contains("Outcome: completed\n"));
    assert!(text.contains("Run: 01J9X3K2M4N5P6Q7R8S9T0V2X\n"));
    assert!(text.contains("Requestable events:\n  intent-ready\n"));
    assert!(text.contains("Request ID: 01J9X3K2M4N5P6Q7R8S9T0V1W\n"));
    assert!(text.ends_with(&format!("Trace: {TRACE_PATH}\n")));
}

#[test]
fn dispatched_rejected_human_is_stdout_only_exit_two() {
    let outcome = rejected_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.request").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Human, &request).unwrap();
    assert_process_contract(&output, EXIT_REJECTED, None, Some(&[]));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("Operation: run.request\n"));
    assert!(text.contains("Outcome: rejected\n"));
    assert!(text.contains("Reason: gate.failed — One or more required gates failed\n"));
    assert!(text.contains("Evidence recorded: yes\n"));
}

#[test]
fn dispatched_error_human_is_stdout_only_exit_one() {
    let outcome = error_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.create").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let output = finalize_dispatched_outcome(RenderFormat::Human, &request).unwrap();
    assert_process_contract(&output, EXIT_ERROR, None, Some(&[]));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("Operation: run.create\n"));
    assert!(text.contains("Outcome: error\n"));
    assert!(
        text.contains("Reason: provider.timeout — Provider process exceeded configured timeout\n")
    );
    assert!(text.contains(
        "Diagnostics:\n- provider.invocation: Role describe timed out after 60 seconds\n"
    ));
}

#[test]
fn human_stdout_matches_renderer_plus_single_newline() {
    let outcome = completed_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let rendered = render_human_outcome(&request).unwrap();
    assert!(!rendered.ends_with('\n'));
    let output = finalize_dispatched_outcome(RenderFormat::Human, &request).unwrap();
    assert_eq!(output.stdout, format!("{rendered}\n").into_bytes());
}

#[test]
fn structured_json_stdout_matches_renderer_plus_single_newline() {
    let outcome = completed_outcome();
    let request = OutcomeRenderRequest::new(
        OperationId::parse("run.show").unwrap(),
        REQUEST_ID,
        TRACE_PATH,
        &outcome,
    );
    let rendered = render_structured_outcome(&request).unwrap();
    assert!(!rendered.contains('\n'));
    let output = finalize_dispatched_outcome(RenderFormat::Json, &request).unwrap();
    assert_eq!(output.stdout, format!("{rendered}\n").into_bytes());
}

#[test]
fn pre_dispatch_parse_json_is_empty_stdout_rich_stderr_exit_sixty_four() {
    let failure = parse_failure(
        "unknown flag --limt",
        correlation(),
        vec!["run list: unrecognized flag --limt".into()],
    );
    let output = finalize_pre_dispatch(RenderFormat::Json, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    assert_empty_stdout(&output);
    assert_eq!(output.stderr_destination(), Some(ByteDestination::Stderr));
    assert_single_json_line(&output.stderr);
    let value = parse_json_line(&output.stderr);
    assert_eq!(value["phase"], "parse");
    assert_eq!(value["message"], "unknown flag --limt");
    assert_eq!(value["request_id"], REQUEST_ID);
    assert_eq!(value["trace"], TRACE_PATH);
}

#[test]
fn pre_dispatch_parse_human_is_empty_stdout_rich_stderr_exit_sixty_four() {
    let failure = parse_failure(
        "unknown flag --limt",
        correlation(),
        vec!["run list: unrecognized flag --limt".into()],
    );
    let output = finalize_pre_dispatch(RenderFormat::Human, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    assert_empty_stdout(&output);
    assert_ends_with_single_newline(&output.stderr);
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.starts_with("Error: unknown flag --limt\n"));
    assert!(text.contains("Phase: parse\n"));
    assert!(text.contains("Request ID: 01J9X3K2M4N5P6Q7R8S9T0V1W\n"));
    assert!(text.contains("Source chain:\n  run list: unrecognized flag --limt\n"));
}

#[test]
fn pre_dispatch_config_json_is_empty_stdout_exit_sixty_four() {
    let failure = configuration_failure(
        &ConfigurationError::Malformed {
            path: PathBuf::from("/home/alice/.config/loop-engine/config.toml"),
            message: "unexpected key `providers` at line 4 column 1".into(),
        },
        correlation(),
    );
    let output = finalize_pre_dispatch(RenderFormat::Json, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    let value = parse_json_line(&output.stderr);
    assert_eq!(value["phase"], "config");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("unexpected key `providers`")
    );
}

#[test]
fn pre_dispatch_usage_human_is_empty_stdout_exit_sixty_four() {
    let failure = usage_failure(
        "missing application command",
        correlation(),
        vec!["expected an application subcommand after global flags".into()],
    );
    let output = finalize_pre_dispatch(RenderFormat::Human, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("Phase: usage\n"));
    assert!(text.contains("Error: missing application command\n"));
}

#[test]
fn pre_dispatch_trace_init_json_has_no_trace_or_request_id() {
    let failure = trace_init_failure(&TraceError::BudgetExhausted {
        required: 16_777_216,
        available: 0,
    });
    assert_eq!(failure.phase, PreDispatchPhase::TraceInit);
    assert!(failure.request_id.is_none());
    assert!(failure.trace.is_none());
    let output = finalize_pre_dispatch(RenderFormat::Json, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    let value = parse_json_line(&output.stderr);
    assert_eq!(value["phase"], "trace_init");
    assert!(value.get("request_id").is_none());
    assert!(value.get("trace").is_none());
}

#[test]
fn pre_dispatch_trace_init_human_mentions_no_operational_trace() {
    let failure = trace_init_failure(&TraceError::BudgetExhausted {
        required: 16_777_216,
        available: 0,
    });
    let output = finalize_pre_dispatch(RenderFormat::Human, &failure).unwrap();
    assert_process_contract(&output, EXIT_PRE_DISPATCH, Some(&[]), None);
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("Phase: trace_init\n"));
    assert!(text.contains("No operational trace was created"));
    assert!(!text.contains("Trace:"));
}

#[test]
fn driver_help_human_is_stdout_only_exit_zero() {
    let output = finalize_driver_help(RenderFormat::Human, sample_usage(), REQUEST_ID, TRACE_PATH);
    assert_process_contract(
        &output,
        EXIT_COMPLETED,
        Some(sample_usage().as_bytes()),
        Some(&[]),
    );
    assert_eq!(output.stdout_destination(), Some(ByteDestination::Stdout));
}

#[test]
fn driver_help_json_is_single_stdout_object_exit_zero() {
    let output = finalize_driver_help(RenderFormat::Json, sample_usage(), REQUEST_ID, TRACE_PATH);
    assert_process_contract(&output, EXIT_COMPLETED, None, Some(&[]));
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["kind"], "help");
    assert_eq!(value["usage"], sample_usage());
    assert_eq!(value["request_id"], REQUEST_ID);
    assert_eq!(value["trace"], TRACE_PATH);
}

#[test]
fn driver_version_human_is_stdout_only_exit_zero() {
    let expected = format!("loop-engine {VERSION}\n");
    let output = finalize_driver_version(RenderFormat::Human, VERSION, REQUEST_ID, TRACE_PATH);
    assert_process_contract(
        &output,
        EXIT_COMPLETED,
        Some(expected.as_bytes()),
        Some(&[]),
    );
}

#[test]
fn driver_version_json_is_single_stdout_object_exit_zero() {
    let output = finalize_driver_version(RenderFormat::Json, VERSION, REQUEST_ID, TRACE_PATH);
    assert_process_contract(&output, EXIT_COMPLETED, None, Some(&[]));
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["kind"], "version");
    assert_eq!(value["name"], "loop-engine");
    assert_eq!(value["version"], VERSION);
}

#[test]
fn driver_list_operations_json_is_single_stdout_object_exit_zero() {
    let operations = sample_operations();
    let output =
        finalize_driver_list_operations(RenderFormat::Json, &operations, REQUEST_ID, TRACE_PATH);
    assert_process_contract(&output, EXIT_COMPLETED, None, Some(&[]));
    assert_single_json_line(&output.stdout);
    let value = parse_json_line(&output.stdout);
    assert_eq!(value["kind"], "operation_list");
    assert_eq!(value["operations"].as_array().unwrap().len(), 2);
}

#[test]
fn driver_list_operations_human_is_stdout_only_exit_zero() {
    let operations = sample_operations();
    let expected = "provider.add\tprovider add <HANDLE>\nrun.show\trun show <RUN-ID>\n";
    let output =
        finalize_driver_list_operations(RenderFormat::Human, &operations, REQUEST_ID, TRACE_PATH);
    assert_process_contract(
        &output,
        EXIT_COMPLETED,
        Some(expected.as_bytes()),
        Some(&[]),
    );
}

#[test]
fn dispatched_render_failure_is_stderr_only_exit_one() {
    let output = finalize_dispatched_render_failure(
        RenderFormat::Json,
        "structured CLI envelope exceeds 4194304 UTF-8 bytes",
    );
    assert_process_contract(&output, EXIT_ERROR, Some(&[]), None);
    assert_single_json_line(&output.stderr);
    let value = parse_json_line(&output.stderr);
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("structured CLI envelope exceeds")
    );
}

#[test]
fn dispatched_render_failure_human_is_stderr_only_exit_one() {
    let output = finalize_dispatched_render_failure(
        RenderFormat::Human,
        "failed to render structured outcome envelope",
    );
    assert_process_contract(&output, EXIT_ERROR, Some(&[]), None);
    assert_eq!(
        output.stderr,
        b"Error: failed to render structured outcome envelope\n"
    );
}

#[test]
fn pre_dispatch_never_writes_partial_json_to_stdout() {
    let failure = parse_failure("bad argv", correlation(), vec!["bad argv".into()]);
    let output = finalize_pre_dispatch(RenderFormat::Json, &failure).unwrap();
    assert_empty_stdout(&output);
    assert_single_json_line(&output.stderr);
}

#[test]
fn dispatched_outcomes_never_duplicate_structured_object_on_stderr() {
    for format in [RenderFormat::Human, RenderFormat::Json] {
        let outcome = completed_outcome();
        let request = OutcomeRenderRequest::new(
            OperationId::parse("run.show").unwrap(),
            REQUEST_ID,
            TRACE_PATH,
            &outcome,
        );
        let output = finalize_dispatched_outcome(format, &request).unwrap();
        assert_empty_stderr(&output);
        if matches!(format, RenderFormat::Json) {
            assert_single_json_line(&output.stdout);
        }
    }
}
