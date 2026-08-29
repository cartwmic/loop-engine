use serde_json::json;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn invoke(input: &[u8]) -> Output {
    let mut child = Command::new(workspace_integration::binary("research"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("research binary should spawn");
    child
        .stdin
        .take()
        .expect("research stdin should be available")
        .write_all(input)
        .expect("request should reach research");
    child
        .wait_with_output()
        .expect("research process should exit")
}

#[test]
fn describe_matches_committed_snapshot_byte_for_byte() {
    let output = invoke(br#"{"operation":"describe"}"#);

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, include_bytes!("snapshots/describe.json"));
}

#[test]
fn unknown_describe_envelope_field_exits_with_protocol_error() {
    let output = invoke(br#"{"operation":"describe","unexpected":true}"#);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field `unexpected`"));
}

#[test]
fn describe_with_and_without_initial_input_return_the_same_workflow_bytes() {
    let without = invoke(br#"{"operation":"describe"}"#);
    let with = invoke(
        br#"{"operation":"describe","initial_input":{"review_policies":{"design-review":["axis"]},"objective":"ignored"}}"#,
    );

    assert!(without.status.success(), "stderr: {:?}", without.stderr);
    assert!(with.status.success(), "stderr: {:?}", with.stderr);
    assert_eq!(without.stdout, with.stdout);
    assert_eq!(without.stdout, include_bytes!("snapshots/describe.json"));
}

#[test]
fn unknown_operation_exits_with_protocol_error() {
    let output = invoke(br#"{"operation":"unknown"}"#);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported provider operation"));
}

#[test]
fn malformed_json_exits_with_protocol_error() {
    let output = invoke(br#"{"operation":"describe""#);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("malformed JSON request"));
}

#[test]
fn unknown_evaluate_envelope_field_exits_with_protocol_error() {
    let request = json!({
        "operation": "evaluate",
        "workflow": {
            "id": "research",
            "initial_state": "scope",
            "states": [],
            "transitions": []
        },
        "initial_input": {},
        "context": [],
        "transition": {
            "source": "scope",
            "event": "scoped",
            "target": "gather",
            "kind": "checked"
        },
        "prior_evaluations": [],
        "unexpected": true
    });
    let request = serde_json::to_vec(&request).expect("request should serialize");
    let output = invoke(&request);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field `unexpected`"));
}

#[test]
fn evaluate_without_review_policies_exits_with_shipped_config_error() {
    let request = json!({
        "operation": "evaluate",
        "workflow": {
            "id": "research",
            "initial_state": "scope",
            "states": [],
            "transitions": []
        },
        "initial_input": {},
        "context": [],
        "transition": {
            "source": "scope",
            "event": "scoped",
            "target": "gather",
            "kind": "checked"
        },
        "prior_evaluations": []
    });
    let request = serde_json::to_vec(&request).expect("request should serialize");
    let output = invoke(&request);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("standard"));
}
