use serde_json::json;
use std::process::{Command, Output};

fn invoke(input: &[u8]) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    super::bounded_process::run_with_stdin(&mut command, "software-change describe protocol", input)
        .expect("software-change process should exit")
        .output
}

#[test]
fn describe_matches_committed_snapshot_byte_for_byte() {
    let output = invoke(br#"{"operation":"describe"}"#);

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(output.stdout, include_bytes!("../snapshots/describe.json"));
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
                "intent-draft".to_owned(),
                "explore".to_owned(),
                "intent-ready".to_owned(),
            ),
            (
                "intent-review".to_owned(),
                "intent-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "intent-adversarial-review".to_owned(),
                "intent-adversarial-review".to_owned(),
                "approved".to_owned(),
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
                "design-adversarial-review".to_owned(),
                "design-adversarial-review".to_owned(),
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
                "plan-adversarial-review".to_owned(),
                "plan-adversarial-review".to_owned(),
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
                "implementation-adversarial-review".to_owned(),
                "implementation-adversarial-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "validation-draft".to_owned(),
                "validation".to_owned(),
                "validation-ready".to_owned(),
            ),
            (
                "validation-review".to_owned(),
                "validation-review".to_owned(),
                "approved".to_owned(),
            ),
            (
                "validation-adversarial-review".to_owned(),
                "validation-adversarial-review".to_owned(),
                "passed".to_owned(),
            ),
        ]
    );
}

#[test]
fn describe_accepts_optional_initial_input() {
    let without = invoke(br#"{"operation":"describe"}"#);
    let with = invoke(br#"{"operation":"describe","initial_input":{"objective":"test"}}"#);

    assert!(without.status.success(), "stderr: {:?}", without.stderr);
    assert!(with.status.success(), "stderr: {:?}", with.stderr);
    assert_eq!(without.stdout, with.stdout);
    assert_eq!(without.stdout, include_bytes!("../snapshots/describe.json"));
}

fn describe_workflow(input: serde_json::Value) -> serde_json::Value {
    let request = serde_json::to_vec(&input).expect("request should serialize");
    let output = invoke(&request);
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("describe JSON")
}

fn state_ids(workflow: &serde_json::Value) -> Vec<&str> {
    workflow["states"]
        .as_array()
        .expect("states")
        .iter()
        .map(|state| state["id"].as_str().expect("state id"))
        .collect()
}

#[test]
fn describe_omitted_review_policies_is_sixteen_state_union() {
    let workflow = describe_workflow(json!({"operation": "describe"}));
    assert_eq!(
        state_ids(&workflow),
        vec![
            "explore",
            "intent-review",
            "intent-adversarial-review",
            "design",
            "design-review",
            "design-adversarial-review",
            "plan",
            "plan-review",
            "plan-adversarial-review",
            "implement",
            "implementation-review",
            "implementation-adversarial-review",
            "validation",
            "validation-review",
            "validation-adversarial-review",
            "end",
        ]
    );
}

#[test]
fn describe_empty_review_policies_omits_reviews_and_uses_passed_on_validation_draft() {
    let workflow = describe_workflow(json!({
        "operation": "describe",
        "initial_input": {"review_policies": {}}
    }));
    assert_eq!(
        state_ids(&workflow),
        vec![
            "explore",
            "design",
            "plan",
            "implement",
            "validation",
            "end",
        ]
    );
    let slots = workflow["work_slots"].as_array().expect("work_slots");
    let validation_draft = slots
        .iter()
        .find(|slot| slot["id"] == "validation-draft")
        .expect("validation-draft");
    assert_eq!(validation_draft["event"], "passed");
    assert!(validation_draft.get("stdin_context_kinds").is_none());
}

#[test]
fn describe_live_review_slots_declare_finding_ledger_and_drafts_omit_it() {
    let workflow = describe_workflow(json!({"operation": "describe"}));
    for slot in workflow["work_slots"].as_array().expect("work_slots") {
        let id = slot["id"].as_str().expect("id");
        let is_review = id.ends_with("-review");
        if is_review || id == "implement" {
            let expected = if id == "implement" {
                json!([
                    "finding-ledger",
                    "review-evidence",
                    "evidence-applicability"
                ])
            } else {
                json!(["finding-ledger"])
            };
            assert_eq!(slot.get("stdin_context_kinds"), Some(&expected), "{id}");
        } else {
            assert!(
                slot.get("stdin_context_kinds").is_none(),
                "non-ledger draft slot {id} must omit stdin_context_kinds"
            );
        }
    }
}

#[test]
fn describe_fails_closed_on_orphan_adversarial_and_unknown_counterpart() {
    let orphan = invoke(
        br#"{"operation":"describe","initial_input":{"review_policies":{"intent-adversarial-review":[{"id":"axis","description":"d"}]}}}"#,
    );
    assert_eq!(orphan.status.code(), Some(2));
    assert!(orphan.stdout.is_empty());
    let orphan_err = String::from_utf8_lossy(&orphan.stderr);
    assert!(orphan_err.contains("empty or absent"), "{orphan_err}");

    let unknown = invoke(
        br#"{"operation":"describe","initial_input":{"review_policies":{"intent-review":[{"id":"parent","description":"d"}],"intent-adversarial-review":[{"id":"other","description":"d"}]}}}"#,
    );
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    let unknown_err = String::from_utf8_lossy(&unknown.stderr);
    assert!(unknown_err.contains("other"), "{unknown_err}");
    assert!(unknown_err.contains("not on"), "{unknown_err}");
}

#[test]
fn unknown_describe_envelope_field_exits_with_protocol_error() {
    let output = invoke(br#"{"operation":"describe","unexpected":true}"#);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field `unexpected`"));
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
    let described = invoke(br#"{"operation":"describe"}"#);
    assert!(described.status.success(), "stderr: {:?}", described.stderr);
    let workflow: serde_json::Value =
        serde_json::from_slice(&described.stdout).expect("union workflow");
    let request = json!({
        "operation": "evaluate",
        "workflow": workflow,
        "initial_input": {},
        "context": [],
        "transition": {
            "source": "explore",
            "event": "intent-ready",
            "target": "intent-review",
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
