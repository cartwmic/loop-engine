use super::support::{
    E2eSandbox, count_journal_entries, parse_correlated_trace, parse_correlated_value,
    parse_structured_stdout,
};

fn add_args<'a>(handle: &'a str, executable: &'a str, cwd: &'a str) -> [&'a str; 7] {
    [
        "provider",
        "add",
        handle,
        "--exec",
        executable,
        "--working-directory",
        cwd,
    ]
}

#[test]
fn add_persists_stable_registration_and_duplicate_rejects_without_mutation() {
    let sandbox = E2eSandbox::new();
    let cwd = sandbox.provider_cwd().to_str().expect("provider cwd utf8");

    let added = sandbox
        .runner()
        .run_json("provider-add", &add_args("alpha", "/bin/false", cwd));
    assert_eq!(added.exit_code, Some(0));
    assert!(added.stderr.is_empty());
    let added = parse_structured_stdout(&added.stdout).expect("add envelope");
    assert_eq!(added.value["operation"], "provider.add");
    assert_eq!(added.value["outcome"], "completed");
    let registration_id = added.value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned();

    let trace = parse_correlated_trace(&added, &sandbox.traces_dir()).expect("add trace");
    let start = &trace.events[0];
    assert!(start.get("argv").is_none());
    assert!(start.get("argv_truncated").is_none());
    assert_eq!(
        start["argv_digest"]
            .as_str()
            .expect("application argv digest")
            .len(),
        64
    );
    let keys = trace
        .events
        .iter()
        .map(|event| {
            (
                event["category"].as_str().unwrap_or_default(),
                event["event"].as_str().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            ("invocation", "start"),
            ("invocation", "request"),
            ("persistence", "intent"),
            ("persistence", "commit"),
            ("invocation", "outcome"),
            ("invocation", "finish"),
        ]
    );
    assert!(
        trace
            .events
            .iter()
            .all(|event| event["category"] != "provider")
    );
    let request_event = &trace.events[1];
    assert_eq!(request_event["operation"], "provider.add");
    assert_eq!(request_event["request"]["handle"], "alpha");
    assert_eq!(request_event["request"]["executable"], "/bin/false");
    assert!(request_event["request"].is_object());
    assert_eq!(trace.events[4]["envelope"], added.value);
    assert_eq!(trace.events[5]["exit_code"], 0);

    let duplicate = sandbox.runner().run_json(
        "provider-add-duplicate",
        &add_args("alpha", "/bin/true", cwd),
    );
    assert_eq!(duplicate.exit_code, Some(2));
    let duplicate = parse_structured_stdout(&duplicate.stdout).expect("duplicate envelope");
    assert_eq!(
        duplicate.value["reason"]["code"],
        "catalog.handle.duplicate"
    );
    let duplicate_trace =
        parse_correlated_trace(&duplicate, &sandbox.traces_dir()).expect("duplicate trace");
    assert!(duplicate_trace.events.iter().any(|event| {
        event["category"] == "persistence"
            && event["event"] == "rollback"
            && event["operation"] == "provider.add"
    }));
    assert!(duplicate_trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "outcome"
            && event["envelope"]["outcome"] == "rejected"
    }));

    let listed = sandbox
        .runner()
        .run_json("provider-list-after-add", &["provider", "list"]);
    assert_eq!(listed.exit_code, Some(0));
    let listed = parse_structured_stdout(&listed.stdout).expect("list envelope");
    let items = listed.value["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["registration"]["id"], registration_id);
    assert_eq!(items[0]["config"]["executable"], "/bin/false");

    assert_eq!(
        count_journal_entries(&sandbox.state_db_path()).expect("journal count"),
        0
    );
}

#[test]
fn add_resolves_relative_paths_lexically_against_caller_cwd() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_json(
        "provider-add-relative-path",
        &[
            "provider",
            "add",
            "relative",
            "--exec",
            "./vendor/../bin/provider",
            "--working-directory",
            "work/./fixture",
        ],
    );
    assert_eq!(invocation.exit_code, Some(0));
    assert!(invocation.stderr.is_empty());
    let added = parse_structured_stdout(&invocation.stdout).expect("add envelope");
    let trace = parse_correlated_value(&added.value, &sandbox.traces_dir()).expect("add trace");
    assert!(trace.events[0].get("argv").is_none());
    assert_eq!(
        trace.events[0]["argv_digest"]
            .as_str()
            .expect("argv digest")
            .len(),
        64
    );

    let listed = sandbox
        .runner()
        .run_json("provider-list-relative-path", &["provider", "list"]);
    assert_eq!(listed.exit_code, Some(0));
    let listed = parse_structured_stdout(&listed.stdout).expect("list envelope");
    let item = &listed.value["data"]["items"][0];
    let process_cwd = sandbox
        .caller_cwd()
        .canonicalize()
        .expect("process-visible caller cwd");
    assert_eq!(
        item["config"]["executable"],
        process_cwd.join("bin/provider").display().to_string()
    );
    assert_eq!(
        item["config"]["working_directory"],
        process_cwd.join("work/fixture").display().to_string()
    );
    assert_eq!(
        count_journal_entries(&sandbox.state_db_path()).expect("journal count"),
        0
    );
}

#[test]
fn add_human_output_uses_same_authoritative_result() {
    let sandbox = E2eSandbox::new();
    let cwd = sandbox.provider_cwd().to_str().expect("provider cwd utf8");
    let added = sandbox
        .runner()
        .run_human("provider-add-human", &add_args("human", "/bin/false", cwd));
    assert_eq!(added.exit_code, Some(0));
    assert!(added.stderr.is_empty());
    let stdout = String::from_utf8(added.stdout).expect("human stdout utf8");
    assert!(stdout.contains("provider.add"));
    assert!(stdout.contains("completed"));
    assert!(stdout.contains("Registration ID:"));
}
