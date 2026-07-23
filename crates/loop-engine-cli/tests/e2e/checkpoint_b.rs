use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use super::support::{
    E2eSandbox, count_journal_entries, count_runs, parse_correlated_trace, parse_correlated_value,
    parse_structured_stdout, scenario_provider_executable, tombstone_provider_registration,
};

fn scenario_provider() -> &'static Path {
    scenario_provider_executable().as_path()
}

fn run_json(sandbox: &E2eSandbox, label: &str, args: Vec<String>) -> Value {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_json(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    let document = parse_structured_stdout(&invocation.stdout).expect("structured outcome");
    parse_correlated_trace(&document, &sandbox.traces_dir()).expect("correlated operation trace");
    document.value
}

fn add_configured_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    executable: &Path,
    provider_args: &[&str],
    timeout_seconds: u64,
) -> String {
    let mut args = vec![
        "provider".into(),
        "add".into(),
        handle.into(),
        "--exec".into(),
        executable.display().to_string(),
        "--working-directory".into(),
        sandbox.provider_cwd().display().to_string(),
    ];
    args.extend(provider_args.iter().map(|arg| format!("--arg={arg}")));
    args.extend(["--timeout".into(), timeout_seconds.to_string()]);
    let value = run_json(sandbox, "checkpoint-b-provider-add", args);
    value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

fn add_scenario_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    scenario: &str,
    timeout_seconds: u64,
) -> String {
    add_configured_provider(
        sandbox,
        handle,
        scenario_provider(),
        &["--scenario", scenario],
        timeout_seconds,
    )
}

fn add_provider(sandbox: &E2eSandbox, handle: &str) -> String {
    add_scenario_provider(sandbox, handle, "graph-linear", 60)
}

fn run_provider_check_failure(
    sandbox: &E2eSandbox,
    label: &str,
    registration_id: &str,
    expected_reason: &str,
) -> Value {
    let invocation = sandbox
        .runner()
        .run_json(label, &["provider", "check", registration_id]);
    assert_eq!(
        invocation.exit_code,
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    let document = parse_structured_stdout(&invocation.stdout).expect("structured provider error");
    parse_correlated_trace(&document, &sandbox.traces_dir()).expect("correlated provider trace");
    assert_eq!(document.value["outcome"], "error");
    assert_eq!(document.value["reason"]["code"], expected_reason);
    document.value
}

fn create_run(sandbox: &E2eSandbox, target: &str, label: &str) -> Value {
    let input_path = sandbox.caller_cwd().join(format!("{label}-inputs.json"));
    fs::write(&input_path, r#"{"ticket":"LE-1"}"#).expect("write run inputs");
    run_json(
        sandbox,
        "checkpoint-b-run-create",
        vec![
            "run".into(),
            "create".into(),
            target.into(),
            "--label".into(),
            label.into(),
            "--inputs".into(),
            input_path.display().to_string(),
        ],
    )
}

fn run_id(value: &Value) -> String {
    value["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned()
}

#[test]
fn provider_check_invokes_conformance_and_active_run_compatibility_with_correlated_trace() {
    let sandbox = E2eSandbox::new();
    let registration_id = add_provider(&sandbox, "checkable");
    for index in 0..10 {
        let created = create_run(&sandbox, &registration_id, &format!("checked-{index}"));
        assert_eq!(created["operation"], "run.create");
        assert_eq!(created["outcome"], "completed");
    }

    let checked = run_json(
        &sandbox,
        "provider-check-active",
        vec![
            "provider".into(),
            "check".into(),
            registration_id.clone(),
            "--active-runs".into(),
            "--limit".into(),
            "9".into(),
        ],
    );
    assert_eq!(checked["operation"], "provider.check");
    assert_eq!(checked["outcome"], "completed");
    assert_eq!(checked["data"]["conformance"]["graph_status"], "valid");
    assert_eq!(checked["data"]["provider_calls"], 10);
    assert_eq!(checked["data"]["items"].as_array().unwrap().len(), 9);
    let cursor = checked["data"]["next_cursor"]
        .as_str()
        .expect("active-run continuation")
        .to_owned();

    let resumed = run_json(
        &sandbox,
        "provider-check-active-resume",
        vec![
            "provider".into(),
            "check".into(),
            registration_id,
            "--active-runs".into(),
            "--cursor".into(),
            cursor,
            "--limit".into(),
            "9".into(),
        ],
    );
    assert_eq!(resumed["data"]["provider_calls"], 2);
    assert_eq!(resumed["data"]["items"].as_array().unwrap().len(), 1);
    assert!(resumed["data"].get("next_cursor").is_none());

    let trace =
        parse_correlated_value(&checked, &sandbox.traces_dir()).expect("provider check trace");
    assert!(
        trace
            .events
            .iter()
            .any(|event| { event["category"] == "provider" && event["event"] == "start" })
    );
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence" && event["operation"] == "provider.check"
    }));
}

#[test]
fn provider_check_closes_shared_process_failure_family_and_bounds_stderr_trace() {
    let sandbox = E2eSandbox::new();
    let missing = add_configured_provider(
        &sandbox,
        "missing-exec",
        Path::new("/definitely/missing/loop-engine-provider"),
        &[],
        60,
    );
    run_provider_check_failure(
        &sandbox,
        "provider-check-missing-exec",
        &missing,
        "provider.executable.not_found",
    );

    let failures = [
        (
            "malformed",
            "process-malformed-json",
            "provider.protocol.malformed",
            60,
        ),
        (
            "extra-output",
            "process-extra-stdout",
            "provider.protocol.malformed",
            60,
        ),
        (
            "missing-output",
            "process-missing-stdout",
            "provider.protocol.malformed",
            60,
        ),
        (
            "wrong-major",
            "process-wrong-major",
            "provider.protocol.unsupported_major",
            60,
        ),
        (
            "nonzero",
            "process-nonzero-exit",
            "provider.nonzero_exit",
            60,
        ),
        ("crash", "process-signal", "provider.crash", 60),
        ("timeout", "process-timeout", "provider.timeout", 1),
        (
            "oversized-output",
            "process-oversized-stdout",
            "provider.protocol.oversized",
            60,
        ),
        (
            "invalid-utf8",
            "process-invalid-utf8",
            "provider.protocol.invalid_utf8",
            60,
        ),
    ];
    for (handle, scenario, reason, timeout) in failures {
        let registration = add_scenario_provider(&sandbox, handle, scenario, timeout);
        run_provider_check_failure(
            &sandbox,
            &format!("provider-check-{handle}"),
            &registration,
            reason,
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let signal_script = sandbox.caller_cwd().join("signal-provider.sh");
        fs::write(&signal_script, "#!/bin/sh\ncat >/dev/null\nkill -TERM $$\n").unwrap();
        fs::set_permissions(&signal_script, fs::Permissions::from_mode(0o700)).unwrap();
        let signal = add_configured_provider(&sandbox, "signal", &signal_script, &[], 60);
        run_provider_check_failure(
            &sandbox,
            "provider-check-signal",
            &signal,
            "provider.signal",
        );
    }

    let stderr_registration =
        add_scenario_provider(&sandbox, "oversized-stderr", "process-oversized-stderr", 60);
    let checked = run_json(
        &sandbox,
        "provider-check-oversized-stderr",
        vec!["provider".into(), "check".into(), stderr_registration],
    );
    let trace = parse_correlated_value(&checked, &sandbox.traces_dir()).unwrap();
    let finish = trace
        .events
        .iter()
        .find(|event| event["category"] == "provider" && event["event"] == "finish")
        .expect("provider finish event");
    assert_eq!(finish["stderr_truncated"], true);
    assert!(finish["stderr_byte_length"].as_u64().unwrap() > 1_000_000);
}

#[test]
fn run_create_and_list_persist_across_processes_and_apply_lifecycle_filters() {
    let sandbox = E2eSandbox::new();
    let registration_id = add_provider(&sandbox, "creator");
    let active = create_run(&sandbox, &registration_id, "active");
    let create_trace = parse_correlated_value(&active, &sandbox.traces_dir()).unwrap();
    assert!(
        create_trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    assert!(
        create_trace.events.iter().any(|event| {
            event["category"] == "persistence" && event["operation"] == "run.create"
        })
    );
    let terminal = create_run(&sandbox, &registration_id, "terminal");
    let terminal_id = run_id(&terminal);
    tombstone_provider_registration(&sandbox.state_db_path(), &registration_id)
        .expect("tombstone provider after creating runs");

    let terminated = run_json(
        &sandbox,
        "run-terminate-for-list",
        vec![
            "run".into(),
            "terminate".into(),
            terminal_id,
            "--note".into(),
            "done".into(),
        ],
    );
    assert_eq!(terminated["outcome"], "completed");

    let active_page = run_json(
        &sandbox,
        "run-list-active",
        vec!["run".into(), "list".into()],
    );
    let list_trace = parse_correlated_value(&active_page, &sandbox.traces_dir()).unwrap();
    assert!(
        !list_trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    assert!(
        list_trace.events.iter().any(|event| {
            event["category"] == "persistence" && event["operation"] == "run.list"
        })
    );
    let active_items = active_page["data"]["items"].as_array().unwrap();
    assert_eq!(active_items.len(), 1);
    assert_eq!(active_items[0]["run_id"], run_id(&active));
    assert_eq!(active_items[0]["lifecycle"], "active");

    let terminal_page = run_json(
        &sandbox,
        "run-list-terminal",
        vec!["run".into(), "list".into(), "--terminal".into()],
    );
    let terminal_items = terminal_page["data"]["items"].as_array().unwrap();
    assert_eq!(terminal_items.len(), 1);
    assert_eq!(terminal_items[0]["lifecycle"], "terminated");

    let all_page = run_json(
        &sandbox,
        "run-list-all",
        vec![
            "run".into(),
            "list".into(),
            "--all".into(),
            "--limit".into(),
            "1".into(),
        ],
    );
    assert_eq!(all_page["data"]["items"].as_array().unwrap().len(), 1);
    let cursor = all_page["data"]["next_cursor"]
        .as_str()
        .expect("run-list continuation")
        .to_owned();
    let resumed = run_json(
        &sandbox,
        "run-list-all-resume",
        vec![
            "run".into(),
            "list".into(),
            "--all".into(),
            "--cursor".into(),
            cursor,
            "--limit".into(),
            "1".into(),
        ],
    );
    assert_eq!(resumed["data"]["items"].as_array().unwrap().len(), 1);
    assert!(resumed["data"].get("next_cursor").is_none());
}

#[test]
fn run_create_covers_zero_valid_rejected_error_and_representative_process_failures() {
    let sandbox = E2eSandbox::new();

    let zero_provider = add_scenario_provider(&sandbox, "zero-input", "graph-cycle", 60);
    let zero = run_json(
        &sandbox,
        "run-create-zero-input",
        vec!["run".into(), "create".into(), zero_provider],
    );
    assert_eq!(zero["outcome"], "completed");
    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 1);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 1);

    let accepted_provider =
        add_scenario_provider(&sandbox, "accepted-input", "input-required-accepted", 60);
    let accepted = run_json(
        &sandbox,
        "run-create-provider-accepted-missing-required",
        vec!["run".into(), "create".into(), accepted_provider],
    );
    assert_eq!(accepted["outcome"], "completed");
    let accepted_id = run_id(&accepted);
    let listed = run_json(
        &sandbox,
        "run-list-provider-accepted-missing-required",
        vec!["run".into(), "list".into(), "--all".into()],
    );
    assert!(
        listed["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["run_id"] == accepted_id)
    );
    let terminated = run_json(
        &sandbox,
        "run-terminate-provider-accepted-missing-required",
        vec!["run".into(), "terminate".into(), accepted_id],
    );
    assert_eq!(terminated["outcome"], "completed");
    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 2);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 3);

    let rejected_provider =
        add_scenario_provider(&sandbox, "rejected-input", "input-invalid-rejected", 60);
    let rejected_inputs = sandbox.caller_cwd().join("rejected-inputs.json");
    fs::write(&rejected_inputs, r#"{"ticket":"invalid"}"#).unwrap();
    let rejected = sandbox.runner().run_json(
        "run-create-rejected-input",
        &[
            "run",
            "create",
            &rejected_provider,
            "--inputs",
            rejected_inputs.to_str().unwrap(),
        ],
    );
    assert_eq!(rejected.exit_code, Some(2));
    let rejected = parse_structured_stdout(&rejected.stdout).unwrap();
    parse_correlated_trace(&rejected, &sandbox.traces_dir()).unwrap();
    assert_eq!(rejected.value["outcome"], "rejected");
    assert_eq!(rejected.value["reason"]["code"], "input.rejected");

    let evaluation_provider =
        add_scenario_provider(&sandbox, "input-evaluation", "input-evaluation-error", 60);
    let evaluation = sandbox.runner().run_json(
        "run-create-input-evaluation",
        &[
            "run",
            "create",
            &evaluation_provider,
            "--inputs",
            rejected_inputs.to_str().unwrap(),
        ],
    );
    assert_eq!(evaluation.exit_code, Some(1));
    let evaluation = parse_structured_stdout(&evaluation.stdout).unwrap();
    parse_correlated_trace(&evaluation, &sandbox.traces_dir()).unwrap();
    assert_eq!(evaluation.value["outcome"], "error");
    assert_eq!(
        evaluation.value["reason"]["code"],
        "provider.evaluation_error"
    );

    for (handle, scenario, timeout, reason) in [
        ("create-timeout", "process-timeout", 1, "provider.timeout"),
        (
            "create-malformed",
            "process-malformed-json",
            60,
            "provider.protocol.malformed",
        ),
    ] {
        let registration = add_scenario_provider(&sandbox, handle, scenario, timeout);
        let invocation = sandbox.runner().run_json(
            &format!("run-create-{handle}"),
            &["run", "create", &registration],
        );
        assert_eq!(invocation.exit_code, Some(1));
        let document = parse_structured_stdout(&invocation.stdout).unwrap();
        parse_correlated_trace(&document, &sandbox.traces_dir()).unwrap();
        assert_eq!(document.value["outcome"], "error");
        assert_eq!(document.value["reason"]["code"], reason);
    }

    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 2);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 3);
}

#[test]
fn run_create_rejects_executable_drift_between_description_and_validation() {
    let sandbox = E2eSandbox::new();
    let provider_copy = sandbox.caller_cwd().join("drifting-provider");
    let provider_replacement = sandbox.caller_cwd().join("drifting-provider-next");
    fs::copy(scenario_provider(), &provider_copy).unwrap();
    fs::copy(scenario_provider(), &provider_replacement).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&provider_replacement)
        .unwrap()
        .write_all(b"\0")
        .unwrap();
    let barrier_root = sandbox.caller_cwd().join("drift-barrier");
    let barrier_text = barrier_root.to_str().unwrap();
    let registration = add_configured_provider(
        &sandbox,
        "drifting",
        &provider_copy,
        &[
            "--scenario",
            "graph-linear",
            "--barrier-dir",
            barrier_text,
            "--barrier-id",
            "create",
            "--barrier-action",
            "reached",
        ],
        60,
    );
    let inputs = sandbox.caller_cwd().join("drifting-inputs.json");
    fs::write(&inputs, r#"{"ticket":"LE-DRIFT"}"#).unwrap();

    let invocation = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            sandbox.runner().run_json(
                "run-create-digest-drift",
                &[
                    "run",
                    "create",
                    &registration,
                    "--inputs",
                    inputs.to_str().unwrap(),
                ],
            )
        });
        let reached = barrier_root.join("create/reached");
        while fs::read_dir(&reached)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
        {
            std::thread::yield_now();
        }
        fs::rename(&provider_replacement, &provider_copy).unwrap();
        fs::write(barrier_root.join("create/release"), b"release").unwrap();
        worker.join().unwrap()
    });

    assert_eq!(invocation.exit_code, Some(1));
    let document = parse_structured_stdout(&invocation.stdout).unwrap();
    parse_correlated_trace(&document, &sandbox.traces_dir()).unwrap();
    assert_eq!(document.value["outcome"], "error");
    assert_eq!(document.value["reason"]["code"], "provider.drift.detected");
    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 0);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 0);
}

#[test]
fn run_terminate_rejects_repeat_and_history_returns_complete_persisted_entries() {
    let sandbox = E2eSandbox::new();
    let registration_id = add_provider(&sandbox, "historian");
    let created = create_run(&sandbox, &registration_id, "history");
    let id = run_id(&created);
    tombstone_provider_registration(&sandbox.state_db_path(), &registration_id)
        .expect("tombstone provider before provider-free operations");

    let first = run_json(
        &sandbox,
        "run-terminate-first",
        vec![
            "run".into(),
            "terminate".into(),
            id.clone(),
            "--note".into(),
            "closed".into(),
        ],
    );
    assert_eq!(first["operation"], "run.terminate");
    assert_eq!(first["outcome"], "completed");
    assert_eq!(first["data"]["run"]["lifecycle"], "terminated");
    let terminate_trace = parse_correlated_value(&first, &sandbox.traces_dir()).unwrap();
    assert!(
        !terminate_trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    assert!(terminate_trace.events.iter().any(|event| {
        event["category"] == "persistence" && event["operation"] == "run.terminate"
    }));

    let repeat_invocation = sandbox.runner().run_json(
        "run-terminate-repeat",
        &["run", "terminate", &id, "--note", "again"],
    );
    assert_eq!(repeat_invocation.exit_code, Some(2));
    let repeat = parse_structured_stdout(&repeat_invocation.stdout)
        .expect("repeat termination outcome")
        .value;
    assert_eq!(repeat["outcome"], "rejected");
    assert_eq!(repeat["reason"]["code"], "run.lifecycle.terminal");
    assert_eq!(repeat["data"]["run"]["lifecycle"], "terminated");

    let history = run_json(
        &sandbox,
        "run-history",
        vec![
            "run".into(),
            "history".into(),
            id.clone(),
            "--limit".into(),
            "2".into(),
        ],
    );
    assert_eq!(history["operation"], "run.history");
    let history_trace = parse_correlated_value(&history, &sandbox.traces_dir()).unwrap();
    assert!(
        !history_trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    assert!(history_trace.events.iter().any(|event| {
        event["category"] == "persistence" && event["operation"] == "run.history"
    }));
    let items = history["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["entry_kind"], "run.created");
    assert_eq!(items[1]["entry_kind"], "run.terminated");
    assert_eq!(items[1]["note"], "closed");
    let cursor = history["data"]["next_cursor"]
        .as_str()
        .expect("history continuation")
        .to_owned();
    let resumed = run_json(
        &sandbox,
        "run-history-resume",
        vec![
            "run".into(),
            "history".into(),
            id,
            "--cursor".into(),
            cursor,
            "--limit".into(),
            "2".into(),
        ],
    );
    let resumed_items = resumed["data"]["items"].as_array().unwrap();
    assert_eq!(resumed_items.len(), 1);
    assert_eq!(resumed_items[0]["outcome"], "rejected");
    assert_eq!(resumed_items[0]["note"], "again");
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 3);
}

#[test]
fn run_create_strict_input_failure_precedes_provider_and_creates_no_journal() {
    let sandbox = E2eSandbox::new();
    let cwd = sandbox.provider_cwd().to_str().expect("provider cwd utf8");
    let added = sandbox.runner().run_json(
        "strict-input-provider",
        &[
            "provider",
            "add",
            "never-run",
            "--exec",
            "/bin/false",
            "--working-directory",
            cwd,
        ],
    );
    assert_eq!(added.exit_code, Some(0));

    let input_path = sandbox.caller_cwd().join("duplicate-inputs.json");
    fs::write(&input_path, r#"{"ticket":"one","ticket":"two"}"#).unwrap();
    let invocation = sandbox.runner().run_json(
        "run-create-duplicate-inputs",
        &[
            "run",
            "create",
            "never-run",
            "--inputs",
            input_path.to_str().unwrap(),
        ],
    );
    assert_eq!(invocation.exit_code, Some(64));
    assert!(invocation.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invocation.stderr).contains("duplicate object key"));
    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 0);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 0);
}
