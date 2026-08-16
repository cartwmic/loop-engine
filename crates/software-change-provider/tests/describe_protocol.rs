use serde_json::json;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn invoke(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_software-change"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("software-change binary should spawn");
    child
        .stdin
        .take()
        .expect("software-change stdin should be available")
        .write_all(input)
        .expect("request should reach software-change");
    child
        .wait_with_output()
        .expect("software-change process should exit")
}

#[test]
fn describe_matches_committed_snapshot_byte_for_byte() {
    let output = invoke(br#"{"operation":"describe"}"#);

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, include_bytes!("snapshots/describe.json"));
}

#[test]
fn describe_work_slots_are_exactly_the_locked_checked_edges() {
    let output = invoke(br#"{"operation":"describe"}"#);

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let workflow: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("describe JSON");
    let actual: Vec<(String, String, String)> = workflow["work_slots"]
        .as_array()
        .expect("work_slots array")
        .iter()
        .map(|slot| {
            (
                slot["id"].as_str().expect("slot id").to_owned(),
                slot["state"].as_str().expect("slot state").to_owned(),
                slot["event"].as_str().expect("slot event").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            (
                "explore-intent".to_owned(),
                "explore".to_owned(),
                "intent-ready".to_owned(),
            ),
            (
                "design-draft".to_owned(),
                "design".to_owned(),
                "design-ready".to_owned(),
            ),
            (
                "design-review".to_owned(),
                "design-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "plan-draft".to_owned(),
                "plan".to_owned(),
                "plan-ready".to_owned(),
            ),
            (
                "plan-review".to_owned(),
                "plan-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "implement".to_owned(),
                "implement".to_owned(),
                "implementation-ready".to_owned(),
            ),
            (
                "implementation-review".to_owned(),
                "implementation-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "validate".to_owned(),
                "validation".to_owned(),
                "passed".to_owned(),
            ),
        ]
    );
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
            "id": "software-change",
            "initial_state": "explore",
            "states": [],
            "transitions": []
        },
        "initial_input": {},
        "context": [],
        "transition": {
            "source": "explore",
            "event": "intent-ready",
            "target": "design",
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
            "id": "software-change",
            "initial_state": "explore",
            "states": [],
            "transitions": []
        },
        "initial_input": {},
        "context": [],
        "transition": {
            "source": "explore",
            "event": "intent-ready",
            "target": "design",
            "kind": "checked"
        },
        "prior_evaluations": []
    });
    let request = serde_json::to_vec(&request).expect("request should serialize");
    let output = invoke(&request);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("minimal"));
    assert!(stderr.contains("standard"));
    assert!(stderr.contains("high-rigor"));
}
