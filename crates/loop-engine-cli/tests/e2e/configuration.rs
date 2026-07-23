use std::fs;

use crate::support::{
    E2eSandbox, invoke_json, parse_pre_dispatch_stderr, parse_structured_stdout,
    scenario_provider_executable,
};

fn write_config(path: &std::path::Path, format: &str, timeout: u64) {
    fs::write(
        path,
        format!(
            "schema_version = 1\n[defaults]\nformat = \"{format}\"\ntimeout_seconds = {timeout}\n"
        ),
    )
    .unwrap();
}

#[test]
fn production_configuration_precedence_and_nearest_ancestor_are_observable() {
    let sandbox = E2eSandbox::new();
    write_config(&sandbox.config_path(), "json", 7);

    let global = sandbox
        .runner()
        .run_human("config-global-format", &["provider", "list"]);
    assert_eq!(global.exit_code, Some(0));
    let global_document = parse_structured_stdout(&global.stdout).expect("global JSON default");
    assert_eq!(global_document.value["operation"], "provider.list");

    let project_root = sandbox.caller_cwd().join("project");
    let child = project_root.join("a/b");
    fs::create_dir_all(&child).unwrap();
    write_config(&project_root.join(".loop-engine.toml"), "human", 9);
    write_config(&project_root.join("a/.loop-engine.toml"), "json", 11);

    let nearest = sandbox
        .runner_from(&child)
        .run_human("config-nearest", &["provider", "list"]);
    assert_eq!(nearest.exit_code, Some(0));
    parse_structured_stdout(&nearest.stdout).expect("nearest ancestor selected");

    let cli = sandbox.runner_from(&child).run_human(
        "config-cli-wins",
        &["--format", "human", "provider", "list"],
    );
    assert_eq!(cli.exit_code, Some(0));
    assert!(
        String::from_utf8_lossy(&cli.stdout)
            .starts_with("Operation: provider.list\nOutcome: completed\n")
    );

    let executable = scenario_provider_executable().display().to_string();
    let cwd = sandbox.provider_cwd().display().to_string();
    let added = sandbox.runner_from(&child).run_json(
        "config-timeout-default",
        &[
            "provider",
            "add",
            "configured",
            "--exec",
            &executable,
            "--working-directory",
            &cwd,
            "--arg",
            "--scenario",
            "--arg",
            "graph-linear",
        ],
    );
    assert_eq!(added.exit_code, Some(0));
    let listed = sandbox
        .runner_from(&child)
        .run_json("config-timeout-list", &["provider", "list"]);
    let listed = parse_structured_stdout(&listed.stdout).unwrap();
    assert_eq!(
        listed.value["data"]["items"][0]["config"]["timeout_seconds"],
        11
    );
}

#[test]
fn built_in_timeout_applies_without_configuration() {
    let sandbox = E2eSandbox::new();
    let executable = scenario_provider_executable().display().to_string();
    let cwd = sandbox.provider_cwd().display().to_string();
    let added = sandbox.runner().run_json(
        "config-built-in-timeout-add",
        &[
            "provider",
            "add",
            "built-in",
            "--exec",
            &executable,
            "--working-directory",
            &cwd,
            "--arg",
            "--scenario",
            "--arg",
            "graph-linear",
        ],
    );
    assert_eq!(added.exit_code, Some(0));
    let listed = sandbox
        .runner()
        .run_json("config-built-in-timeout-list", &["provider", "list"]);
    let listed = parse_structured_stdout(&listed.stdout).unwrap();
    assert_eq!(
        listed.value["data"]["items"][0]["config"]["timeout_seconds"],
        60
    );
}

#[test]
fn malformed_and_forbidden_configuration_fail_before_dispatch_without_state() {
    for (label, contents) in [
        ("malformed", "schema_version = [\n"),
        (
            "forbidden",
            "schema_version = 1\ndatabase = \"elsewhere.db\"\n[defaults]\nformat = \"json\"\n",
        ),
    ] {
        let sandbox = E2eSandbox::new();
        fs::write(sandbox.config_path(), contents).unwrap();
        let invocation = sandbox.runner().run_json(label, &["provider", "list"]);
        assert_eq!(invocation.exit_code, Some(64));
        assert!(invocation.stdout.is_empty());
        let failure = parse_pre_dispatch_stderr(&invocation.stderr).expect("config failure");
        assert_eq!(failure.value["phase"], "config");
        assert!(!sandbox.state_db_path().exists());
        let trace = fs::read_to_string(failure.value["trace"].as_str().unwrap()).unwrap();
        assert!(!trace.contains("\"event\":\"request\""));
        assert!(!trace.contains("\"category\":\"provider\""));
        assert!(!trace.contains("\"category\":\"persistence\""));
    }
}

#[test]
fn machine_state_is_cwd_independent_and_project_defaults_do_not_rebind_runs() {
    let sandbox = E2eSandbox::new();
    let first = sandbox.caller_cwd().join("first");
    let second = sandbox.caller_cwd().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();

    let executable = scenario_provider_executable().display().to_string();
    let provider_cwd = sandbox.provider_cwd().display().to_string();
    let add = sandbox.runner_from(&first).run_json(
        "cwd-add",
        &[
            "provider",
            "add",
            "stable",
            "--exec",
            &executable,
            "--working-directory",
            &provider_cwd,
            "--arg",
            "--scenario",
            "--arg",
            "graph-linear",
        ],
    );
    let add = parse_structured_stdout(&add.stdout).unwrap();
    let registration_id = add.value["data"]["registration"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let list = sandbox
        .runner_from(&second)
        .run_json("cwd-list", &["provider", "list"]);
    let list = parse_structured_stdout(&list.stdout).unwrap();
    assert_eq!(
        list.value["data"]["items"][0]["registration"]["id"],
        registration_id
    );

    let create = invoke_json(
        &sandbox,
        "cwd-create",
        &["run".into(), "create".into(), registration_id.clone()],
        0,
    );
    let run_id = create.document.value["data"]["run"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    fs::write(
        second.join(".loop-engine.toml"),
        "schema_version = 1\n[defaults]\nprovider = \"different\"\n",
    )
    .unwrap();
    let history = sandbox
        .runner_from(&second)
        .run_json("cwd-history", &["run", "history", &run_id]);
    let history = parse_structured_stdout(&history.stdout).unwrap();
    assert_eq!(
        history.value["data"]["items"][0]["provider_observations"][0]["registration_id"],
        registration_id
    );
}
