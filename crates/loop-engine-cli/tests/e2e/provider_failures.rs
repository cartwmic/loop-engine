use std::path::PathBuf;

use crate::support::{
    E2eSandbox, ProviderAddArgs, add_scenario_provider, count_journal_entries, count_runs,
    create_run, invoke_json, scenario_provider_executable, set_provider_registration_command,
    tombstone_provider_registration,
};

fn assert_provider_failure(sandbox: &E2eSandbox, target: &str, label: &str, reason: &str) {
    let failed = invoke_json(
        sandbox,
        label,
        &["provider".into(), "check".into(), target.into()],
        1,
    );
    assert_eq!(failed.document.value["outcome"], "error");
    assert_eq!(failed.document.value["reason"]["code"], reason);
    assert!(
        failed
            .trace
            .events
            .iter()
            .any(|event| { event["category"] == "provider" && event["event"] == "start" })
    );
    assert!(failed.trace.events.iter().any(|event| {
        event["category"] == "provider"
            && matches!(event["event"].as_str(), Some("finish" | "failure"))
    }));
}

#[test]
fn provider_check_closes_the_shared_execution_failure_cross_product() {
    let sandbox = E2eSandbox::new();
    let missing = ProviderAddArgs {
        handle: "missing-a2".into(),
        exec: PathBuf::from("/definitely/missing/loop-engine-provider"),
        working_directory: sandbox.provider_cwd().to_path_buf(),
        args: vec![],
        timeout_seconds: 2,
    }
    .to_cli_args();
    let missing = invoke_json(&sandbox, "failure-add-missing", &missing, 0);
    let missing_id = missing.document.value["data"]["registration"]["id"]
        .as_str()
        .unwrap();
    assert_provider_failure(
        &sandbox,
        missing_id,
        "failure-missing",
        "provider.executable.not_found",
    );

    for (index, (scenario, reason)) in [
        ("process-malformed-json", "provider.protocol.malformed"),
        ("process-extra-stdout", "provider.protocol.malformed"),
        ("process-missing-stdout", "provider.protocol.malformed"),
        ("process-wrong-major", "provider.protocol.unsupported_major"),
        ("process-nonzero-exit", "provider.nonzero_exit"),
        ("process-signal", "provider.crash"),
        ("process-timeout", "provider.timeout"),
        ("process-oversized-stdout", "provider.protocol.oversized"),
        ("process-invalid-utf8", "provider.protocol.invalid_utf8"),
    ]
    .into_iter()
    .enumerate()
    {
        let provider = add_scenario_provider(&sandbox, &format!("failure-{index}"), scenario, &[]);
        assert_provider_failure(
            &sandbox,
            &provider,
            &format!("failure-check-{index}"),
            reason,
        );
    }

    let stderr_provider =
        add_scenario_provider(&sandbox, "failure-stderr", "process-oversized-stderr", &[]);
    let checked = invoke_json(
        &sandbox,
        "failure-stderr-check",
        &["provider".into(), "check".into(), stderr_provider],
        0,
    );
    let finish = checked
        .trace
        .events
        .iter()
        .find(|event| event["category"] == "provider" && event["event"] == "finish")
        .unwrap();
    assert_eq!(finish["stderr_truncated"], true);
}

#[test]
fn other_provider_invoking_alpha_operations_have_atomic_failure_rows() {
    let sandbox = E2eSandbox::new();
    let create_provider =
        add_scenario_provider(&sandbox, "failure-create", "process-malformed-json", &[]);
    let failed_create = invoke_json(
        &sandbox,
        "failure-create-run",
        &["run".into(), "create".into(), create_provider],
        1,
    );
    assert_eq!(
        failed_create.document.value["reason"]["code"],
        "provider.protocol.malformed"
    );
    assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 0);
    assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 0);

    let gated_provider = add_scenario_provider(&sandbox, "failure-request", "gate-pass", &[]);
    let gated_run = create_run(&sandbox, &gated_provider, "failure-request-run");
    set_provider_registration_command(
        &sandbox.state_db_path(),
        &gated_provider,
        &["--scenario", "process-timeout"],
        1,
    )
    .unwrap();
    let failed_request = invoke_json(
        &sandbox,
        "failure-request-timeout",
        &[
            "run".into(),
            "request".into(),
            gated_run.clone(),
            "approve".into(),
        ],
        1,
    );
    assert_eq!(
        failed_request.document.value["reason"]["code"],
        "provider.timeout"
    );
    let shown = invoke_json(
        &sandbox,
        "failure-request-show",
        &["run".into(), "show".into(), gated_run],
        0,
    );
    assert_eq!(shown.document.value["data"]["run"]["state"], "draft");
}

#[test]
fn gate_free_operations_remain_usable_without_provider_authority() {
    let sandbox = E2eSandbox::new();
    let provider = add_scenario_provider(&sandbox, "failure-gate-free", "graph-linear", &[]);
    let run = create_run(&sandbox, &provider, "failure-gate-free-run");
    tombstone_provider_registration(&sandbox.state_db_path(), &provider).unwrap();

    let requested = invoke_json(
        &sandbox,
        "failure-gate-free-request",
        &[
            "run".into(),
            "request".into(),
            run.clone(),
            "advance".into(),
        ],
        0,
    );
    assert_eq!(requested.document.value["outcome"], "completed");
    assert!(
        !requested
            .trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );

    for (label, args) in [
        ("show", vec!["run", "show", &run]),
        ("history", vec!["run", "history", &run]),
        ("list", vec!["run", "list", "--all"]),
    ] {
        let refs = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let read = invoke_json(&sandbox, &format!("failure-gate-free-{label}"), &refs, 0);
        assert_eq!(read.document.value["outcome"], "completed");
        assert!(
            !read
                .trace
                .events
                .iter()
                .any(|event| event["category"] == "provider")
        );
    }

    assert!(scenario_provider_executable().is_file());
}
