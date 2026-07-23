use std::fs;
use std::path::Path;

use serde_json::Value;

use super::support::{
    E2eSandbox, count_journal_entries, parse_correlated_value, parse_structured_stdout,
    scenario_provider_executable, set_provider_registration_command,
    tombstone_provider_registration,
};

fn scenario_provider() -> &'static Path {
    scenario_provider_executable().as_path()
}

fn invoke_json(sandbox: &E2eSandbox, label: &str, args: Vec<String>, expected_exit: i32) -> Value {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_json(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(expected_exit),
        "stderr: {}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    let document = parse_structured_stdout(&invocation.stdout).expect("structured outcome");
    let trace = parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated operation trace");
    if document.value["operation"] == "run.request" {
        assert!(trace.events.iter().any(|event| {
            event["category"] == "persistence" && event["operation"] == "run.request"
        }));
        assert!(trace.events.iter().any(|event| {
            event["category"] == "invocation"
                && event["event"] == "request"
                && event["operation"] == "run.request"
        }));
        assert!(trace.events.iter().any(|event| {
            event["category"] == "invocation"
                && event["event"] == "outcome"
                && event["envelope"]["operation"] == "run.request"
        }));
        for start in trace.events.iter().filter(|event| {
            event["category"] == "provider"
                && event["event"] == "start"
                && event["role"] == "evaluate_gates"
        }) {
            assert!(trace.events.iter().any(|event| {
                event["category"] == "provider"
                    && matches!(event["event"].as_str(), Some("finish" | "failure"))
                    && event["role"] == "evaluate_gates"
                    && event["invocation_id"] == start["invocation_id"]
            }));
        }
    }
    document.value
}

fn add_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    scenario: &str,
    extra_args: &[String],
) -> String {
    let mut args = vec![
        "provider".into(),
        "add".into(),
        handle.into(),
        "--exec".into(),
        scenario_provider().display().to_string(),
        "--working-directory".into(),
        sandbox.provider_cwd().display().to_string(),
        "--arg=--scenario".into(),
        format!("--arg={scenario}"),
    ];
    args.extend(extra_args.iter().map(|arg| format!("--arg={arg}")));
    args.extend(["--timeout".into(), "60".into()]);
    let value = invoke_json(sandbox, "checkpoint-d-provider-add", args, 0);
    value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

fn create_run(sandbox: &E2eSandbox, registration: &str, label: &str) -> String {
    let inputs = sandbox.caller_cwd().join(format!("{label}-inputs.json"));
    fs::write(&inputs, r#"{"ticket":"LE-1"}"#).unwrap();
    let value = invoke_json(
        sandbox,
        "checkpoint-d-run-create",
        vec![
            "run".into(),
            "create".into(),
            registration.into(),
            "--label".into(),
            label.into(),
            "--inputs".into(),
            inputs.display().to_string(),
        ],
        0,
    );
    value["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned()
}

fn request(
    sandbox: &E2eSandbox,
    label: &str,
    run_id: &str,
    event: &str,
    extra: &[String],
    exit: i32,
) -> Value {
    let mut args = vec!["run".into(), "request".into(), run_id.into(), event.into()];
    args.extend_from_slice(extra);
    let value = invoke_json(sandbox, label, args, exit);
    assert_eq!(value["operation"], "run.request");
    value
}

fn ledger_role_count(path: &Path, role: &str) -> usize {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["role"] == role)
        .count()
}

#[test]
fn run_request_gate_free_completion_unknown_and_terminal_attempts_are_authoritative() {
    let sandbox = E2eSandbox::new();
    let registration = add_provider(&sandbox, "linear-request", "graph-linear", &[]);
    let run_id = create_run(&sandbox, &registration, "linear-request");

    let before = count_journal_entries(&sandbox.state_db_path()).unwrap();
    let unknown = request(
        &sandbox,
        "request-unknown",
        &run_id,
        "missing",
        &["--note".into(), "unknown event note".into()],
        2,
    );
    assert_eq!(unknown["outcome"], "rejected");
    assert_eq!(unknown["reason"]["code"], "event.unknown");
    assert_eq!(unknown["data"]["run"]["state"], "start");
    assert_eq!(unknown["data"]["run"]["state_changed"], false);
    assert_eq!(
        count_journal_entries(&sandbox.state_db_path()).unwrap(),
        before + 1
    );

    let advanced = request(
        &sandbox,
        "request-advance",
        &run_id,
        "advance",
        &["--note".into(), "advance note".into()],
        0,
    );
    assert_eq!(advanced["outcome"], "completed");
    assert_eq!(advanced["data"]["run"]["state"], "middle");
    assert_eq!(advanced["data"]["run"]["state_changed"], true);
    assert_eq!(
        advanced["data"]["requestable_events"],
        serde_json::json!(["finish"])
    );

    let finished = request(&sandbox, "request-finish", &run_id, "finish", &[], 0);
    assert_eq!(finished["outcome"], "completed");
    assert_eq!(finished["data"]["run"]["state"], "done");
    assert_eq!(finished["data"]["run"]["lifecycle"], "final");
    assert_eq!(
        finished["data"]["requestable_events"],
        serde_json::json!([])
    );

    let terminal = request(&sandbox, "request-terminal", &run_id, "finish", &[], 2);
    assert_eq!(terminal["outcome"], "rejected");
    assert_eq!(terminal["reason"]["code"], "run.lifecycle.terminal");
    assert_eq!(terminal["data"]["run"]["state"], "done");
    assert_eq!(terminal["data"]["run"]["state_changed"], false);

    let history = invoke_json(
        &sandbox,
        "request-history-authority",
        vec![
            "run".into(),
            "history".into(),
            run_id,
            "--limit".into(),
            "10".into(),
        ],
        0,
    );
    let items = history["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[1]["entry_kind"], "transition.attempt");
    assert_eq!(items[1]["request_id"], unknown["request_id"]);
    assert_eq!(items[1]["outcome"], "rejected");
    assert_eq!(items[1]["reason"]["code"], "event.unknown");
    assert_eq!(items[1]["transition"]["event"], "missing");
    assert_eq!(items[1]["transition"]["applied"], false);
    assert_eq!(items[1]["note"], "unknown event note");
    assert_eq!(items[2]["request_id"], advanced["request_id"]);
    assert_eq!(items[2]["outcome"], "completed");
    assert_eq!(items[2]["transition"]["event"], "advance");
    assert_eq!(items[2]["transition"]["source_state"], "start");
    assert_eq!(items[2]["transition"]["target_state"], "middle");
    assert_eq!(items[2]["transition"]["applied"], true);
    assert_eq!(items[2]["note"], "advance note");
    assert_eq!(items[3]["request_id"], finished["request_id"]);
    assert_eq!(items[3]["transition"]["event"], "finish");
    assert_eq!(items[3]["transition"]["applied"], true);
    assert_eq!(items[4]["request_id"], terminal["request_id"]);
    assert_eq!(items[4]["outcome"], "rejected");
    assert_eq!(items[4]["reason"]["code"], "run.lifecycle.terminal");
    assert_eq!(items[4]["transition"]["applied"], false);
}

#[test]
fn run_request_gated_verdicts_and_inline_evidence_close_committed_outcomes() {
    for (index, scenario, expected_outcome, expected_reason, expected_exit) in [
        (0, "gate-pass", "completed", None, 0),
        (1, "gate-fail", "rejected", Some("gate.failed"), 2),
        (2, "gate-mixed", "rejected", Some("gate.failed"), 2),
        (
            3,
            "gate-exact-set-violation",
            "error",
            Some("provider.protocol.malformed"),
            1,
        ),
        (
            4,
            "gate-incompatible",
            "rejected",
            Some("compatibility.unsupported"),
            2,
        ),
        (
            5,
            "gate-evaluation-error",
            "error",
            Some("provider.evaluation_error"),
            1,
        ),
    ] {
        let sandbox = E2eSandbox::new();
        let registration = add_provider(&sandbox, &format!("gated-{index}"), scenario, &[]);
        let run_id = create_run(&sandbox, &registration, &format!("gated-{index}"));
        let result = request(
            &sandbox,
            &format!("request-gated-{index}"),
            &run_id,
            "approve",
            &[],
            expected_exit,
        );
        assert_eq!(result["outcome"], expected_outcome);
        match expected_reason {
            Some(code) => assert_eq!(result["reason"]["code"], code),
            None => assert!(result["reason"].is_null()),
        }
        let expected_state = if expected_outcome == "completed" {
            "approved"
        } else {
            "draft"
        };
        assert_eq!(result["data"]["run"]["state"], expected_state);
        if index == 1 {
            let human = sandbox.runner().run_human(
                "request-gate-fail-human",
                &["run", "request", &run_id, "approve"],
            );
            assert_eq!(human.exit_code, Some(2));
            assert!(human.stderr.is_empty());
            let stdout = String::from_utf8(human.stdout).unwrap();
            assert!(stdout.contains("Operation: run.request"));
            assert!(stdout.contains("Outcome: rejected"));
            assert!(stdout.contains("Reason: gate.failed"));
            assert!(stdout.contains("State: draft"));
        }
    }

    let sandbox = E2eSandbox::new();
    let registration = add_provider(&sandbox, "caller-evidence", "gate-caller-evidence", &[]);
    let run_id = create_run(&sandbox, &registration, "caller-evidence");
    let path = sandbox.caller_cwd().join("inline-evidence.json");
    fs::write(
        &path,
        r#"[{"id":"caller-evidence-1","kind":"report","locator":"opaque:not-dereferenced","metadata":{"score":1}}]"#,
    )
    .unwrap();
    let with_inline = request(
        &sandbox,
        "request-inline-evidence",
        &run_id,
        "approve",
        &["--evidence".into(), path.display().to_string()],
        0,
    );
    assert_eq!(with_inline["outcome"], "completed");
    assert_eq!(with_inline["data"]["evidence_recorded"]["inline"], true);
}

#[test]
fn run_request_invalid_inline_and_tombstoned_provider_fail_closed_with_journaled_attempts() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("provider-ledger.jsonl");
    let registration = add_provider(
        &sandbox,
        "strict-inline",
        "gate-pass",
        &["--ledger-path".into(), ledger.display().to_string()],
    );
    let run_id = create_run(&sandbox, &registration, "strict-inline");
    let gate_calls_before = ledger_role_count(&ledger, "evaluate_gates");
    let invalid = sandbox.caller_cwd().join("invalid-evidence.json");
    fs::write(
        &invalid,
        r#"[{"id":"duplicate","kind":"report","locator":"opaque:x"},{"id":"duplicate","kind":"report","locator":"opaque:y"}]"#,
    )
    .unwrap();
    let before = count_journal_entries(&sandbox.state_db_path()).unwrap();
    let rejected = request(
        &sandbox,
        "request-invalid-inline",
        &run_id,
        "approve",
        &["--evidence".into(), invalid.display().to_string()],
        2,
    );
    assert_eq!(rejected["outcome"], "rejected");
    assert_eq!(rejected["reason"]["code"], "evidence.invalid");
    assert_eq!(rejected["data"]["evidence_recorded"]["inline"], false);
    assert_eq!(
        ledger_role_count(&ledger, "evaluate_gates"),
        gate_calls_before
    );
    assert_eq!(
        count_journal_entries(&sandbox.state_db_path()).unwrap(),
        before + 1
    );

    tombstone_provider_registration(&sandbox.state_db_path(), &registration).unwrap();
    let tombstoned = request(
        &sandbox,
        "request-tombstoned-provider",
        &run_id,
        "approve",
        &[],
        1,
    );
    assert_eq!(tombstoned["outcome"], "error");
    assert_eq!(tombstoned["reason"]["code"], "provider.tombstoned");
    assert_eq!(
        ledger_role_count(&ledger, "evaluate_gates"),
        gate_calls_before
    );

    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("gate-free-ledger.jsonl");
    let registration = add_provider(
        &sandbox,
        "gate-free-tombstone",
        "graph-linear",
        &["--ledger-path".into(), ledger.display().to_string()],
    );
    let run_id = create_run(&sandbox, &registration, "gate-free-tombstone");
    tombstone_provider_registration(&sandbox.state_db_path(), &registration).unwrap();
    let gate_free = request(
        &sandbox,
        "request-gate-free-tombstone",
        &run_id,
        "advance",
        &[],
        0,
    );
    assert_eq!(gate_free["outcome"], "completed");
    assert_eq!(gate_free["data"]["run"]["state"], "middle");
    assert_eq!(ledger_role_count(&ledger, "evaluate_gates"), 0);
}

#[test]
fn run_request_self_loop_cycle_and_terminated_lifecycle_preserve_graph_semantics() {
    let sandbox = E2eSandbox::new();
    let registration = add_provider(&sandbox, "self-loop", "graph-self-loop", &[]);
    let run_id = create_run(&sandbox, &registration, "self-loop");
    let self_loop = request(&sandbox, "request-self-loop", &run_id, "checkpoint", &[], 0);
    assert_eq!(self_loop["outcome"], "completed");
    assert_eq!(self_loop["data"]["run"]["state"], "draft");
    assert_eq!(self_loop["data"]["run"]["state_changed"], false);
    assert_eq!(
        self_loop["data"]["requestable_events"],
        serde_json::json!(["checkpoint"])
    );

    let sandbox = E2eSandbox::new();
    let registration = add_provider(&sandbox, "cycle", "graph-cycle", &[]);
    let run_id = create_run(&sandbox, &registration, "cycle");
    let forward = request(
        &sandbox,
        "request-cycle-forward",
        &run_id,
        "forward",
        &[],
        0,
    );
    assert_eq!(forward["data"]["run"]["state"], "b");
    let back = request(&sandbox, "request-cycle-back", &run_id, "back", &[], 0);
    assert_eq!(back["data"]["run"]["state"], "a");

    let terminated = invoke_json(
        &sandbox,
        "terminate-before-request",
        vec!["run".into(), "terminate".into(), run_id.clone()],
        0,
    );
    assert_eq!(terminated["outcome"], "completed");
    let denied = request(&sandbox, "request-terminated", &run_id, "forward", &[], 2);
    assert_eq!(denied["outcome"], "rejected");
    assert_eq!(denied["reason"]["code"], "run.lifecycle.terminal");
    assert_eq!(denied["data"]["run"]["lifecycle"], "terminated");
    assert_eq!(denied["data"]["requestable_events"], serde_json::json!([]));
}

#[test]
fn run_request_representative_provider_failures_are_journaled_without_state_change() {
    for (index, scenario, expected_reason) in [
        (0, "process-malformed-json", "provider.protocol.malformed"),
        (1, "process-timeout", "provider.timeout"),
    ] {
        let sandbox = E2eSandbox::new();
        let registration = add_provider(
            &sandbox,
            &format!("provider-failure-{index}"),
            "gate-pass",
            &[],
        );
        let run_id = create_run(
            &sandbox,
            &registration,
            &format!("provider-failure-{index}"),
        );
        set_provider_registration_command(
            &sandbox.state_db_path(),
            &registration,
            &["--scenario", scenario],
            1,
        )
        .unwrap();
        let before = count_journal_entries(&sandbox.state_db_path()).unwrap();
        let failed = request(
            &sandbox,
            &format!("request-provider-failure-{index}"),
            &run_id,
            "approve",
            &[],
            1,
        );
        assert_eq!(failed["outcome"], "error");
        assert_eq!(failed["reason"]["code"], expected_reason);
        assert_eq!(failed["data"]["run"]["state"], "draft");
        assert_eq!(failed["data"]["run"]["state_changed"], false);
        assert_eq!(
            count_journal_entries(&sandbox.state_db_path()).unwrap(),
            before + 1
        );
    }
}

#[test]
fn run_request_stale_cas_uses_post_transaction_snapshot_and_never_applies_transition() {
    let sandbox = E2eSandbox::new();
    let barrier = sandbox.caller_cwd().join("request-barrier");
    let registration = add_provider(
        &sandbox,
        "stale-request",
        "gate-pass",
        &[
            "--barrier-dir".into(),
            barrier.display().to_string(),
            "--barrier-id".into(),
            "request".into(),
            "--barrier-action".into(),
            "reached".into(),
        ],
    );

    let run_id = std::thread::scope(|scope| {
        let worker = scope.spawn(|| create_run(&sandbox, &registration, "stale-request"));
        let reached = barrier.join("request/reached");
        while fs::read_dir(&reached)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
        {
            std::thread::yield_now();
        }
        fs::write(barrier.join("request/release"), b"release").unwrap();
        worker.join().unwrap()
    });
    fs::remove_file(barrier.join("request/release")).unwrap();
    fs::remove_dir_all(barrier.join("request/reached")).unwrap();

    let stale = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            request(
                &sandbox,
                "request-stale-cas",
                &run_id,
                "approve",
                &["--note".into(), "stale attempt".into()],
                1,
            )
        });
        let reached = barrier.join("request/reached");
        while fs::read_dir(&reached)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
        {
            std::thread::yield_now();
        }
        let terminated = invoke_json(
            &sandbox,
            "terminate-during-request",
            vec!["run".into(), "terminate".into(), run_id.clone()],
            0,
        );
        assert_eq!(terminated["data"]["run"]["lifecycle"], "terminated");
        fs::write(barrier.join("request/release"), b"release").unwrap();
        worker.join().unwrap()
    });

    assert_eq!(stale["outcome"], "error");
    assert_eq!(stale["reason"]["code"], "state.stale_version");
    assert_eq!(stale["data"]["run"]["state"], "draft");
    assert_eq!(stale["data"]["run"]["lifecycle"], "terminated");
    assert_eq!(stale["data"]["run"]["state_changed"], false);
    assert_eq!(stale["data"]["requestable_events"], serde_json::json!([]));
    assert_eq!(stale["data"]["evidence_recorded"]["provider"], false);
}
