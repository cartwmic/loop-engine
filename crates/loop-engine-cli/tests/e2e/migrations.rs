use std::fs;

use crate::support::{E2eSandbox, execute_sql, invoke_json};

const FROZEN_V1: &str = include_str!("../../../../test-support/sqlite/v1.sql");

#[test]
fn frozen_v1_database_opens_and_mutates_only_through_production_cli() {
    let sandbox = E2eSandbox::new();
    execute_sql(&sandbox.state_db_path(), FROZEN_V1).expect("apply frozen v1 fixture");
    let before = fs::metadata(sandbox.state_db_path()).unwrap().len();

    let listed = invoke_json(
        &sandbox,
        "migration-v1-list",
        &["provider".into(), "list".into()],
        0,
    );
    assert_eq!(listed.document.value["outcome"], "completed");
    assert!(
        listed.document.value["data"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let executable = crate::support::scenario_provider_executable()
        .display()
        .to_string();
    let cwd = sandbox.provider_cwd().display().to_string();
    let added = invoke_json(
        &sandbox,
        "migration-v1-add",
        &[
            "provider".into(),
            "add".into(),
            "migrated".into(),
            "--exec".into(),
            executable,
            "--working-directory".into(),
            cwd,
            "--arg".into(),
            "--scenario".into(),
            "--arg".into(),
            "graph-linear".into(),
        ],
        0,
    );
    assert_eq!(added.document.value["outcome"], "completed");
    let registration_id = added.document.value["data"]["registration"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(fs::metadata(sandbox.state_db_path()).unwrap().len() >= before);

    let reopened = invoke_json(
        &sandbox,
        "migration-v1-reopen-list",
        &["provider".into(), "list".into()],
        0,
    );
    let item = &reopened.document.value["data"]["items"][0];
    assert_eq!(item["registration"]["id"], registration_id);
    assert_eq!(item["registration"]["handle"], "migrated");
    assert_eq!(item["config"]["timeout_seconds"], 60);
}

#[test]
fn frozen_v1_fixture_is_standalone_and_explicitly_versioned() {
    assert!(FROZEN_V1.contains("PRAGMA user_version = 1;"));
    assert!(FROZEN_V1.contains("CREATE TABLE provider_registrations"));
    assert!(FROZEN_V1.contains("CREATE TABLE journal_entries"));
    assert!(!FROZEN_V1.contains("IF NOT EXISTS"));
}
