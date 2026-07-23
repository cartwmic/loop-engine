use super::support::{
    E2eSandbox, count_journal_entries, insert_provider_registrations, parse_correlated_trace,
    parse_pre_dispatch_stderr, parse_structured_stdout, tombstone_provider_registration,
};

fn add(sandbox: &E2eSandbox, handle: &str) -> String {
    let cwd = sandbox.provider_cwd().to_str().expect("provider cwd utf8");
    let invocation = sandbox.runner().run_json(
        &format!("add-{handle}"),
        &[
            "provider",
            "add",
            handle,
            "--exec",
            "/bin/false",
            "--working-directory",
            cwd,
        ],
    );
    assert_eq!(invocation.exit_code, Some(0));
    let document = parse_structured_stdout(&invocation.stdout).expect("add envelope");
    document.value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

fn list(sandbox: &E2eSandbox, label: &str, args: &[&str]) -> serde_json::Value {
    let invocation = sandbox.runner().run_json(label, args);
    assert_eq!(
        invocation.exit_code,
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    parse_structured_stdout(&invocation.stdout)
        .expect("list envelope")
        .value
}

fn insert_registrations(sandbox: &E2eSandbox, prefix: &str, count: usize, argv_json: &str) {
    insert_provider_registrations(&sandbox.state_db_path(), prefix, count, argv_json)
        .expect("insert provider registration fixtures");
}

#[test]
fn list_filters_tombstones_and_zero_active_runs_without_provider_execution() {
    let sandbox = E2eSandbox::new();
    let tombstoned_id = add(&sandbox, "retired");
    tombstone_provider_registration(&sandbox.state_db_path(), &tombstoned_id)
        .expect("tombstone provider fixture");
    let enabled_id = add(&sandbox, "enabled");

    let enabled = list(&sandbox, "list-enabled", &["provider", "list", "--enabled"]);
    let enabled_items = enabled["data"]["items"].as_array().expect("enabled items");
    assert_eq!(enabled_items.len(), 1);
    assert_eq!(enabled_items[0]["registration"]["id"], enabled_id);

    let tombstoned = list(
        &sandbox,
        "list-tombstoned",
        &["provider", "list", "--tombstoned"],
    );
    let tombstoned_items = tombstoned["data"]["items"]
        .as_array()
        .expect("tombstoned items");
    assert_eq!(tombstoned_items.len(), 1);
    assert_eq!(tombstoned_items[0]["registration"]["id"], tombstoned_id);
    assert_eq!(tombstoned_items[0]["registration"]["enabled"], false);
    assert!(tombstoned_items[0]["config"].is_null());

    let all = list(
        &sandbox,
        "list-all",
        &["provider", "list", "--enabled", "--tombstoned"],
    );
    assert_eq!(all["data"]["items"].as_array().unwrap().len(), 2);

    let active = sandbox.runner().run_json(
        "list-active-zero",
        &["provider", "list", "--active-runs-for", &enabled_id],
    );
    assert_eq!(active.exit_code, Some(0));
    let active = parse_structured_stdout(&active.stdout).expect("active envelope");
    assert_eq!(active.value["data"]["items"], serde_json::json!([]));
    assert!(active.value["data"].get("next_cursor").is_none());
    let trace = parse_correlated_trace(&active, &sandbox.traces_dir()).expect("active trace");
    assert!(
        trace
            .events
            .iter()
            .all(|event| event["category"] != "provider")
    );
    assert!(trace.events.iter().all(|event| {
        event["category"] != "persistence" || event["operation"] == "provider.list"
    }));
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
            ("persistence", "read_complete"),
            ("invocation", "outcome"),
            ("invocation", "finish"),
        ]
    );
    assert_eq!(trace.events[1]["operation"], "provider.list");
    assert_eq!(trace.events[1]["request"]["active_runs_for"], enabled_id);
    assert!(trace.events[1]["request"].is_object());
    assert_eq!(trace.events[4]["envelope"], active.value);
    assert_eq!(trace.events[5]["exit_code"], 0);

    assert_eq!(
        count_journal_entries(&sandbox.state_db_path()).expect("journal count"),
        0
    );
}

#[test]
fn list_count_pages_progress_and_tampered_cursor_rejects() {
    let sandbox = E2eSandbox::new();
    for handle in ["page-a", "page-b", "page-c", "page-d", "page-e"] {
        add(&sandbox, handle);
    }

    let first = list(
        &sandbox,
        "list-page-first",
        &["provider", "list", "--limit", "2"],
    );
    let first_items = first["data"]["items"].as_array().expect("first items");
    assert_eq!(first_items.len(), 2);
    let cursor = first["data"]["next_cursor"]
        .as_str()
        .expect("first cursor")
        .to_owned();

    let second = list(
        &sandbox,
        "list-page-second",
        &["provider", "list", "--limit", "2", "--cursor", &cursor],
    );
    let second_items = second["data"]["items"].as_array().expect("second items");
    assert_eq!(second_items.len(), 2);
    assert_ne!(first_items, second_items);
    let second_cursor = second["data"]["next_cursor"]
        .as_str()
        .expect("second cursor");
    assert_ne!(cursor, second_cursor);

    let mismatched = sandbox.runner().run_json(
        "list-filter-mismatched-cursor",
        &["provider", "list", "--tombstoned", "--cursor", &cursor],
    );
    assert_eq!(mismatched.exit_code, Some(2));
    let mismatched = parse_structured_stdout(&mismatched.stdout).expect("mismatch envelope");
    assert_eq!(mismatched.value["reason"]["code"], "cursor.invalid");

    let mut tampered = cursor;
    let replacement = if tampered.ends_with('A') { 'B' } else { 'A' };
    tampered.pop();
    tampered.push(replacement);
    let rejected = sandbox.runner().run_json(
        "list-tampered-cursor",
        &["provider", "list", "--cursor", &tampered],
    );
    assert_eq!(rejected.exit_code, Some(2));
    let rejected = parse_structured_stdout(&rejected.stdout).expect("rejected envelope");
    assert_eq!(rejected.value["reason"]["code"], "cursor.invalid");
}

#[test]
fn list_enforces_default_and_max_count_ceilings() {
    let sandbox = E2eSandbox::new();
    add(&sandbox, "count-seed");
    insert_registrations(&sandbox, "count", 1_001, "[]");

    let default_page = list(&sandbox, "list-default-count", &["provider", "list"]);
    assert_eq!(default_page["data"]["items"].as_array().unwrap().len(), 100);
    assert!(default_page["data"]["next_cursor"].is_string());

    let max_page = list(
        &sandbox,
        "list-max-count",
        &["provider", "list", "--limit", "1000"],
    );
    assert_eq!(max_page["data"]["items"].as_array().unwrap().len(), 1_000);
    let cursor = max_page["data"]["next_cursor"]
        .as_str()
        .expect("max-page cursor");
    let final_page = list(
        &sandbox,
        "list-after-max-count",
        &["provider", "list", "--limit", "1000", "--cursor", cursor],
    );
    assert_eq!(final_page["data"]["items"].as_array().unwrap().len(), 2);
    assert!(final_page["data"].get("next_cursor").is_none());

    let over_max = sandbox.runner().run_json(
        "list-over-max-count",
        &["provider", "list", "--limit", "1001"],
    );
    assert_eq!(over_max.exit_code, Some(64));
    assert!(over_max.stdout.is_empty());
    let failure = parse_pre_dispatch_stderr(&over_max.stderr).expect("limit failure");
    assert_eq!(failure.value["phase"], "parse");
}

#[test]
fn list_stops_before_byte_budget_and_resumes_without_record_truncation() {
    let sandbox = E2eSandbox::new();
    add(&sandbox, "byte-seed");
    let element = "x".repeat(16_000);
    let argv_json = serde_json::to_string(&vec![
        &element, &element, &element, &element, &element, &element, &element, &element,
    ])
    .expect("large argv json");
    insert_registrations(&sandbox, "byte", 30, &argv_json);

    let mut cursor: Option<String> = None;
    let mut observed_large_rows = 0;
    let mut page_count = 0;
    loop {
        let mut args = vec!["provider", "list", "--limit", "100"];
        if let Some(value) = cursor.as_deref() {
            args.extend(["--cursor", value]);
        }
        let page = list(&sandbox, &format!("list-byte-page-{page_count}"), &args);
        let items = page["data"]["items"].as_array().expect("byte-page items");
        assert!(!items.is_empty());
        assert!(
            items.len() < 31,
            "byte budget must stop before count ceiling"
        );
        for item in items {
            if item["registration"]["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("byte-"))
            {
                let argv = item["config"]["argv"].as_array().expect("complete argv");
                assert_eq!(argv.len(), 8);
                assert!(
                    argv.iter()
                        .all(|value| value.as_str().unwrap().len() == 16_000)
                );
                observed_large_rows += 1;
            }
        }
        page_count += 1;
        cursor = page["data"]["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        assert!(page_count < 5, "cursor must make bounded progress");
    }
    assert_eq!(observed_large_rows, 30);
    assert!(page_count >= 2);
}
