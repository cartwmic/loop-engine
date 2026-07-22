use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use scenario_provider::scenarios::{all_scenario_names, scenario_fixture_category};
use scenario_provider::test_support::TempDir;
use serde_json::Value;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scenario-provider"))
}

fn registration() -> Value {
    serde_json::json!({
        "registration_id": "019f0000-0000-7000-8000-000000000001",
        "config_revision": 1,
        "executable": "/tmp/scenario-provider",
        "argv": [],
        "working_directory": "/tmp",
        "timeout_seconds": 60
    })
}

fn request(role: &str, invocation_id: &str, payload: Value) -> String {
    serde_json::json!({
        "protocol_major": 1,
        "role": role,
        "invocation_id": invocation_id,
        "registration": registration(),
        "payload": payload,
    })
    .to_string()
}

fn run_provider(
    args: &[&str],
    role: &str,
    invocation_id: &str,
    payload: Value,
) -> std::process::Output {
    let mut child = bin()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let body = request(role, invocation_id, payload);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn wait_for_path(path: &std::path::Path, deadline: Instant) {
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "path timeout: {}",
            path.display()
        );
        thread::yield_now();
    }
}

#[test]
fn describe_graph_linear_preserves_invocation_and_role() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/graphs/linear.json")).unwrap();
    let output = run_provider(
        &["--scenario", "graph-linear"],
        "describe",
        "inv-linear",
        fixture["request"]["payload"].clone(),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["protocol_major"], 1);
    assert_eq!(response["role"], "describe");
    assert_eq!(response["invocation_id"], "inv-linear");
    assert_eq!(
        response["result"]["graph"]["initial_state"],
        fixture["expected"]["result"]["graph"]["initial_state"]
    );
}

#[test]
fn validate_inputs_required_rejected_matches_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/inputs/required-rejected.json")).unwrap();
    let output = run_provider(
        &["--scenario", "input-required-rejected"],
        "validate_inputs",
        "inv-input",
        fixture["request_payload"].clone(),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["kind"], "rejected");
    assert_eq!(
        response["result"]["diagnostics"][0]["code"],
        fixture["expected_result"]["diagnostics"][0]["code"]
    );
}

#[test]
fn gate_mixed_matches_fixture_verdicts() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/roles/gate-mixed.json")).unwrap();
    let output = run_provider(
        &["--scenario", "gate-mixed"],
        "evaluate_gates",
        "inv-gates",
        fixture["request_payload"].clone(),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["verdicts"], fixture["expected_verdicts"]);
}

#[test]
fn gate_provider_evidence_duplicate_returns_two_same_ids() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../fixtures/roles/gate-provider-evidence-duplicate.json"
    ))
    .unwrap();
    let output = run_provider(
        &["--scenario", "gate-provider-evidence-duplicate"],
        "evaluate_gates",
        "inv-dup",
        fixture["request_payload"].clone(),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let evidence = response["result"]["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0]["id"], evidence[1]["id"]);
}

#[test]
fn gate_provider_evidence_collision_matches_selected_id() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../fixtures/roles/gate-provider-evidence-collision.json"
    ))
    .unwrap();
    let output = run_provider(
        &["--scenario", "gate-provider-evidence-collision"],
        "evaluate_gates",
        "inv-collision",
        fixture["request_payload"].clone(),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["result"]["evidence"][0]["id"],
        fixture["request_payload"]["selected_evidence"][0]["id"]
    );
}

#[test]
fn graph_build_drift_selector_uses_persisted_ordinal() {
    let temp = TempDir::new("graph-build-drift");
    let ordinal = temp.path().join("ordinal");
    let first = run_provider(
        &[
            "--scenario",
            "graph-build-drift",
            "--ordinal-path",
            ordinal.to_str().unwrap(),
        ],
        "describe",
        "inv-a",
        serde_json::json!({}),
    );
    let second = run_provider(
        &[
            "--scenario",
            "graph-build-drift",
            "--ordinal-path",
            ordinal.to_str().unwrap(),
        ],
        "describe",
        "inv-b",
        serde_json::json!({}),
    );
    assert!(first.status.success());
    assert!(second.status.success());
    let first_graph: Value = serde_json::from_slice(&first.stdout).unwrap();
    let second_graph: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        first_graph["result"]["graph"]["initial_state"],
        "build-a-v1"
    );
    assert_eq!(
        second_graph["result"]["graph"]["initial_state"],
        "build-b-v2"
    );
}

#[test]
fn malformed_role_payload_maps_to_evaluation_error_without_panic() {
    let output = run_provider(
        &["--scenario", "gate-pass"],
        "evaluate_gates",
        "inv-bad",
        serde_json::json!({"invalid": true}),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["kind"], "evaluation_error");
    assert_eq!(
        response["result"]["diagnostics"][0]["code"],
        "provider.protocol.malformed"
    );
}

#[test]
fn process_malformed_json_emits_invalid_stdout_after_consuming_stdin() {
    let output = run_provider(
        &["--scenario", "process-malformed-json"],
        "describe",
        "inv-malformed",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
}

#[test]
fn process_nonzero_exit_after_valid_stdout() {
    let output = run_provider(
        &["--scenario", "process-nonzero-exit"],
        "describe",
        "inv-nonzero",
        serde_json::json!({}),
    );
    assert_eq!(output.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["protocol_major"], 1);
}

#[test]
fn process_extra_stdout_appends_trailing_bytes() {
    let output = run_provider(
        &["--scenario", "process-extra-stdout"],
        "describe",
        "inv-extra",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
    assert!(output.stdout.windows(5).any(|window| window == b"EXTRA"));
}

#[test]
fn process_missing_stdout_emits_empty_stream() {
    let output = run_provider(
        &["--scenario", "process-missing-stdout"],
        "describe",
        "inv-missing",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn process_wrong_major_emits_unsupported_protocol_major() {
    let output = run_provider(
        &["--scenario", "process-wrong-major"],
        "describe",
        "inv-major",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["protocol_major"], 2);
}

#[test]
fn process_oversized_stdout_exceeds_bound() {
    let output = run_provider(
        &["--scenario", "process-oversized-stdout"],
        "describe",
        "inv-oversized",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(output.stdout.len() > 1_048_576);
}

#[test]
fn process_oversized_stderr_exceeds_bound_with_valid_stdout() {
    let output = run_provider(
        &["--scenario", "process-oversized-stderr"],
        "describe",
        "inv-stderr",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_ok());
    assert!(output.stderr.len() > 1_048_576);
}

#[test]
fn process_invalid_utf8_stdout_is_not_valid_utf8() {
    let output = run_provider(
        &["--scenario", "process-invalid-utf8"],
        "describe",
        "inv-utf8",
        serde_json::json!({}),
    );
    assert!(output.status.success());
    assert!(std::str::from_utf8(&output.stdout).is_err());
}

#[test]
fn process_signal_aborts_without_success() {
    let output = run_provider(
        &["--scenario", "process-signal"],
        "describe",
        "inv-signal",
        serde_json::json!({}),
    );
    assert!(!output.status.success());
}

#[test]
fn process_timeout_can_be_killed_with_bounded_cleanup() {
    let mut child = bin()
        .args(["--scenario", "process-timeout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request("describe", "inv-timeout", serde_json::json!({})).as_bytes())
        .unwrap();
    let alive_deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < alive_deadline {
        assert!(
            child.try_wait().unwrap().is_none(),
            "timeout scenario exited before bounded kill"
        );
        thread::yield_now();
    }
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
fn ledger_records_invocation_append_only_with_request_facts() {
    let temp = TempDir::new("ledger-append");
    let ledger = temp.path().join("invocations.jsonl");
    for id in ["first", "second"] {
        let output = run_provider(
            &[
                "--scenario",
                "graph-linear",
                "--ledger-path",
                ledger.to_str().unwrap(),
            ],
            "describe",
            id,
            serde_json::json!({}),
        );
        assert!(output.status.success());
    }
    let lines: Vec<Value> = std::fs::read_to_string(&ledger)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["invocation_id"], "first");
    assert_eq!(lines[1]["invocation_id"], "second");
    assert_eq!(lines[0]["request"]["protocol_major"], 1);
    assert_eq!(lines[0]["request"]["role"], "describe");
    assert!(lines[0].get("registration").is_none());
}

#[test]
fn concurrent_subprocess_ledger_integrity() {
    let temp = TempDir::new("ledger-concurrent");
    let ledger = temp.path().join("invocations.jsonl");
    let ledger_arg = ledger.to_str().unwrap().to_string();
    let handles: Vec<_> = (0..8)
        .map(|index| {
            let ledger_arg = ledger_arg.clone();
            thread::spawn(move || {
                let output = run_provider(
                    &["--scenario", "graph-linear", "--ledger-path", &ledger_arg],
                    "describe",
                    &format!("inv-{index}"),
                    serde_json::json!({}),
                );
                assert!(output.status.success());
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let lines: Vec<Value> = std::fs::read_to_string(&ledger)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 8);
    let mut ids: Vec<_> = lines
        .iter()
        .map(|line| line["invocation_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8);
}

#[test]
fn barrier_reached_waits_for_explicit_release() {
    let temp = TempDir::new("barrier-release");
    let barrier_dir = temp.path().join("barriers");
    let mut reached = bin()
        .args([
            "--scenario",
            "graph-linear",
            "--barrier-dir",
            barrier_dir.to_str().unwrap(),
            "--barrier-id",
            "overlap",
            "--barrier-action",
            "reached",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    reached
        .stdin
        .take()
        .unwrap()
        .write_all(request("describe", "inv-barrier", serde_json::json!({})).as_bytes())
        .unwrap();
    let reached_marker = barrier_dir.join("overlap/reached/inv-barrier");
    wait_for_path(&reached_marker, Instant::now() + Duration::from_secs(2));
    assert!(reached.try_wait().unwrap().is_none());
    let release_output = run_provider(
        &[
            "--scenario",
            "graph-linear",
            "--barrier-dir",
            barrier_dir.to_str().unwrap(),
            "--barrier-id",
            "overlap",
            "--barrier-action",
            "release",
        ],
        "describe",
        "inv-release",
        serde_json::json!({}),
    );
    assert!(release_output.status.success());
    let output = reached.wait_with_output().unwrap();
    assert!(output.status.success());
}

#[test]
fn barrier_kill_while_waiting_leaves_cleanup_path() {
    let temp = TempDir::new("barrier-kill");
    let barrier_dir = temp.path().join("barriers");
    let mut reached = bin()
        .args([
            "--scenario",
            "graph-linear",
            "--barrier-dir",
            barrier_dir.to_str().unwrap(),
            "--barrier-id",
            "kill",
            "--barrier-action",
            "reached",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    reached
        .stdin
        .take()
        .unwrap()
        .write_all(request("describe", "inv-kill", serde_json::json!({})).as_bytes())
        .unwrap();
    let reached_marker = barrier_dir.join("kill/reached/inv-kill");
    wait_for_path(&reached_marker, Instant::now() + Duration::from_secs(2));
    reached.kill().unwrap();
    let status = reached.wait().unwrap();
    assert!(!status.success());
    std::fs::write(barrier_dir.join("kill/release"), b"release").unwrap();
    std::fs::remove_dir_all(barrier_dir.join("kill")).unwrap();
    assert!(!barrier_dir.join("kill").exists());
}

#[test]
fn all_documented_scenarios_are_recognized() {
    for name in all_scenario_names() {
        let status = bin()
            .args(["--scenario", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .unwrap();
        assert!(
            !status.success(),
            "scenario {name} should require stdin request before succeeding"
        );
    }
}

#[test]
fn fixture_indexes_reference_concrete_golden_vectors() {
    for name in all_scenario_names() {
        let category = scenario_fixture_category(name).expect("category");
        let index_path = format!("../fixtures/{category}/scenario-index.jsonl");
        let index = match category {
            "graphs" => include_str!("../fixtures/graphs/scenario-index.jsonl"),
            "inputs" => include_str!("../fixtures/inputs/scenario-index.jsonl"),
            "roles" => include_str!("../fixtures/roles/scenario-index.jsonl"),
            "process" => include_str!("../fixtures/process/scenario-index.jsonl"),
            _ => index_path.as_str(),
        };
        let mut found = false;
        for line in index.lines() {
            let entry: Value = serde_json::from_str(line).unwrap();
            if entry["scenario"] == *name {
                found = true;
                assert!(entry.get("golden").is_some(), "missing golden for {name}");
            }
        }
        assert!(found, "scenario {name} missing from index");
    }
}
