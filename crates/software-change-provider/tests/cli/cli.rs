use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn invoke(arguments: &[&str], stdin: &[u8]) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    command.args(arguments);
    let completed =
        super::bounded_process::run_with_stdin(&mut command, "software-change CLI protocol", stdin)
            .expect("software-change process should exit");
    if let Some(error) = completed.stdin_error {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write failure: {error}"
        );
    }
    completed.output
}

#[test]
fn help_flags_return_conventional_stdout_without_reading_protocol() {
    for flag in ["--help", "-h"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("software-change"));
        assert!(help.contains("describe"));
        assert!(help.contains("evaluate"));
        assert!(help.contains("data-dump"));
        assert!(help.contains("run-plan-graph"));
        assert!(help.contains("review-candidates"));
        assert!(help.contains('4'));
    }
}

#[test]
fn version_flags_report_workspace_package_version_on_stdout() {
    for flag in ["--version", "-V"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("software-change {}\n", env!("CARGO_PKG_VERSION")),
            "flag={flag}"
        );
    }
}

#[test]
fn no_arguments_keep_stdin_protocol_and_unsupported_arguments_keep_error_taxonomy() {
    let describe = invoke(&[], br#"{"operation":"describe"}"#);
    assert_eq!(describe.status.code(), Some(0));
    assert!(!describe.stdout.is_empty());

    let unsupported = invoke(&["unknown"], b"");
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(unsupported.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported command"));
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn candidate_temp_dir() -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "software-change-review-candidates-cli-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("candidate temp directory");
    path
}

fn review_contract(axis: &str, author: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"type": "string", "const": axis},
            "author": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string"},
                    "kind": {"type": "string"}
                },
                "const": {"name": author, "kind": "agent"}
            },
            "result": {"type": "string"},
            "findings": {"type": "string"}
        },
        "oneOf": [
            {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
            {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}}
        ]
    })
}

fn candidate_worker(
    assignment_id: &str,
    contract: Value,
    selected_attempt: Option<u32>,
    selected_output_path: Option<&PathBuf>,
    selected_output_sha256: Option<String>,
) -> Value {
    json!({
        "assignment_id": assignment_id,
        "command": "/bin/reviewer",
        "args": [],
        "exit_code": 0,
        "selected_attempt": selected_attempt,
        "selected_output_path": selected_output_path.map(|path| path.to_string_lossy().to_string()),
        "selected_output_sha256": selected_output_sha256,
        "declared_output_contract": contract
    })
}

#[test]
fn review_candidates_cli_reads_selected_bytes_without_rewriting_captures() {
    let capture = candidate_temp_dir();
    let selected = capture.join("0/attempts/2/stdout");
    fs::create_dir_all(capture.join("0/attempts/1")).expect("first attempt parent");
    fs::create_dir_all(selected.parent().expect("selected parent")).expect("selected parent");
    fs::create_dir_all(capture.join("1")).expect("exhausted parent");
    let selected_bytes =
        br#"{"axis":"axis","author":{"name":"reviewer","kind":"agent"},"result":"fail","findings":"concrete failure"}"#;
    fs::write(
        capture.join("0/attempts/1/stdout"),
        b"first malformed attempt",
    )
    .expect("first attempt");
    fs::write(&selected, selected_bytes).expect("selected attempt");
    fs::write(
        capture.join("1/attempts.json"),
        br#"{"schema_version":"1","attempts":[{"number":1,"validation_errors":["bad"]},{"number":2,"validation_errors":["still bad"]}],"selected_attempt":null,"exhausted":true}"#,
    )
    .expect("exhaustion manifest");

    let show = json!({
        "operation": "show",
        "status": "completed",
        "result": {
            "workflow_id": "software-change",
            "initial_input": {"review_policies": {"design-review": [{"id": "axis"}]}},
            "work_slot_invocations": [{
                "invocation_id": "invocation-1",
                "slot_id": "design-review",
                "status": "failed",
                "capture_dir": capture.to_string_lossy(),
                "inner_workers": [
                    candidate_worker(
                        "assignment-ready",
                        review_contract("axis", "reviewer"),
                        Some(2),
                        Some(&selected),
                        Some(format!("sha256:{:x}", Sha256::digest(selected_bytes))),
                    ),
                    candidate_worker(
                        "assignment-exhausted",
                        review_contract("axis", "reviewer"),
                        None,
                        None,
                        None,
                    )
                ]
            }]
        }
    });
    let input = serde_json::to_vec(&show).expect("show JSON");
    let first = invoke(&["review-candidates"], &input);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert!(first.stderr.is_empty(), "{first:?}");
    let second = invoke(&["review-candidates"], &input);
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    assert_eq!(first.stdout, second.stdout);

    let output: Value = serde_json::from_slice(&first.stdout).expect("candidate JSON");
    assert_eq!(output["schema_version"], "1");
    let candidates = output["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["status"], "ready");
    assert_eq!(candidates[0]["origin"]["id"], "invocation-1");
    assert_eq!(candidates[0]["origin"]["assignment_id"], "assignment-ready");
    assert_eq!(candidates[0]["result"], "fail");
    assert_eq!(candidates[0]["findings"], "concrete failure");
    assert_eq!(candidates[1]["status"], "exhausted");
    assert!(candidates[1]["axis"].is_null());
    assert!(candidates[1]["author"].is_null());
    assert!(candidates[1]["diagnostic"].as_str().is_some());
    assert!(candidates[0]["origin"]
        .get("selected_output_path")
        .is_none());
    assert!(candidates[0]["origin"]
        .get("selected_output_sha256")
        .is_none());
    assert_eq!(fs::read(&selected).expect("selected bytes"), selected_bytes);
    assert_eq!(
        fs::read(capture.join("0/attempts/1/stdout")).expect("first bytes"),
        b"first malformed attempt"
    );
    assert_eq!(
        fs::read(capture.join("1/attempts.json")).expect("manifest bytes"),
        br#"{"schema_version":"1","attempts":[{"number":1,"validation_errors":["bad"]},{"number":2,"validation_errors":["still bad"]}],"selected_attempt":null,"exhausted":true}"#
    );
    let _ = fs::remove_dir_all(capture);
}

#[test]
fn review_candidates_cli_rejects_non_show_input() {
    let output = invoke(&["review-candidates"], br#"{"operation":"evaluate"}"#);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ordinary completed show"));
}

#[test]
fn review_candidates_cli_rejects_foreign_workflow() {
    let input = json!({
        "operation": "show",
        "status": "completed",
        "result": {"workflow_id": "research"}
    });
    let output = invoke(
        &["review-candidates"],
        &serde_json::to_vec(&input).expect("show JSON"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("software-change"));
}
