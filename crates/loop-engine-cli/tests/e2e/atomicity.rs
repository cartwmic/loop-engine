use std::fs;

use crate::support::{
    E2eSandbox, add_scenario_provider, count_evidence_associations, count_evidence_records,
    count_journal_entries, count_runs, create_run, execute_sql, invoke_json,
};

fn install_fault(sandbox: &E2eSandbox, timing: &str, table: &str) {
    execute_sql(
        &sandbox.state_db_path(),
        &format!(
            "CREATE TRIGGER fault {timing} ON {table} BEGIN SELECT RAISE(ABORT, 'wp4 fault'); END;"
        ),
    )
    .unwrap();
}

fn remove_fault(sandbox: &E2eSandbox) {
    execute_sql(&sandbox.state_db_path(), "DROP TRIGGER fault;").unwrap();
}

#[test]
fn provider_add_and_run_create_roll_back_at_every_durable_boundary() {
    let provider_fault = E2eSandbox::new();
    invoke_json(
        &provider_fault,
        "atomicity-initialize",
        &["provider".into(), "list".into()],
        0,
    );
    install_fault(&provider_fault, "BEFORE INSERT", "provider_registrations");
    let executable = crate::support::scenario_provider_executable()
        .display()
        .to_string();
    let cwd = provider_fault.provider_cwd().display().to_string();
    let failed = invoke_json(
        &provider_fault,
        "atomicity-provider-add",
        &[
            "provider".into(),
            "add".into(),
            "faulted".into(),
            "--exec".into(),
            executable,
            "--working-directory".into(),
            cwd,
        ],
        1,
    );
    assert_eq!(failed.document.value["outcome"], "error");
    assert_eq!(
        failed.document.value["reason"]["code"],
        "persistence.failed"
    );
    remove_fault(&provider_fault);
    let listed = invoke_json(
        &provider_fault,
        "atomicity-provider-list",
        &["provider".into(), "list".into()],
        0,
    );
    assert!(
        listed.document.value["data"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    for (index, (timing, table)) in [
        ("BEFORE INSERT", "runs"),
        ("BEFORE INSERT", "run_journal_sequences"),
        ("BEFORE INSERT", "journal_entries"),
    ]
    .into_iter()
    .enumerate()
    {
        let sandbox = E2eSandbox::new();
        let provider = add_scenario_provider(
            &sandbox,
            &format!("create-fault-{index}"),
            "graph-linear",
            &[],
        );
        install_fault(&sandbox, timing, table);
        let failed = invoke_json(
            &sandbox,
            &format!("atomicity-create-{index}"),
            &["run".into(), "create".into(), provider],
            1,
        );
        assert_eq!(failed.document.value["outcome"], "error");
        remove_fault(&sandbox);
        assert_eq!(count_runs(&sandbox.state_db_path()).unwrap(), 0);
        assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 0);
    }
}

#[test]
fn termination_rolls_back_state_sequence_and_journal_together() {
    for (index, (timing, table)) in [
        ("BEFORE UPDATE", "runs"),
        ("BEFORE UPDATE", "run_journal_sequences"),
        ("BEFORE INSERT", "journal_entries"),
    ]
    .into_iter()
    .enumerate()
    {
        let sandbox = E2eSandbox::new();
        let provider = add_scenario_provider(
            &sandbox,
            &format!("terminate-fault-{index}"),
            "graph-linear",
            &[],
        );
        let run = create_run(&sandbox, &provider, &format!("terminate-fault-{index}"));
        install_fault(&sandbox, timing, table);
        let failed = invoke_json(
            &sandbox,
            &format!("atomicity-terminate-{index}"),
            &["run".into(), "terminate".into(), run.clone()],
            1,
        );
        assert_eq!(failed.document.value["outcome"], "error");
        remove_fault(&sandbox);
        let shown = invoke_json(
            &sandbox,
            &format!("atomicity-terminate-show-{index}"),
            &["run".into(), "show".into(), run],
            0,
        );
        assert_eq!(shown.document.value["data"]["run"]["lifecycle"], "active");
        assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 1);
    }
}

#[test]
fn gated_request_rolls_back_evidence_association_state_sequence_and_journal() {
    for (index, (timing, table)) in [
        ("BEFORE INSERT", "evidence"),
        ("BEFORE INSERT", "evidence_associations"),
        ("BEFORE UPDATE", "runs"),
        ("BEFORE UPDATE", "run_journal_sequences"),
        ("BEFORE INSERT", "journal_entries"),
    ]
    .into_iter()
    .enumerate()
    {
        let sandbox = E2eSandbox::new();
        let provider = add_scenario_provider(
            &sandbox,
            &format!("request-fault-{index}"),
            "gate-caller-evidence",
            &[],
        );
        let run = create_run(&sandbox, &provider, &format!("request-fault-{index}"));
        let evidence = sandbox.caller_cwd().join("evidence.json");
        fs::write(
            &evidence,
            r#"[{"id":"fault-evidence","kind":"report","locator":"opaque:fault","metadata":{}}]"#,
        )
        .unwrap();
        install_fault(&sandbox, timing, table);
        let failed = invoke_json(
            &sandbox,
            &format!("atomicity-request-{index}"),
            &[
                "run".into(),
                "request".into(),
                run.clone(),
                "approve".into(),
                "--evidence".into(),
                evidence.display().to_string(),
            ],
            1,
        );
        assert_eq!(failed.document.value["outcome"], "error");
        remove_fault(&sandbox);
        let shown = invoke_json(
            &sandbox,
            &format!("atomicity-request-show-{index}"),
            &["run".into(), "show".into(), run],
            0,
        );
        assert_eq!(shown.document.value["data"]["run"]["state"], "draft");
        assert_eq!(count_journal_entries(&sandbox.state_db_path()).unwrap(), 1);
        assert_eq!(count_evidence_records(&sandbox.state_db_path()).unwrap(), 0);
        assert_eq!(
            count_evidence_associations(&sandbox.state_db_path()).unwrap(),
            0
        );
    }
}
