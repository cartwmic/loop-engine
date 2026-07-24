use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::support::{
    E2eSandbox, execute_sql, parse_correlated_value, parse_structured_stdout,
    reference_provider_executable, scenario_provider_executable, set_provider_registration_command,
    set_run_projection_state, tombstone_provider_registration,
};

fn wait_for_barrier(reached: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if fs::read_dir(reached)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn scenario_provider() -> &'static Path {
    scenario_provider_executable().as_path()
}

fn reference_provider() -> &'static Path {
    reference_provider_executable().as_path()
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
    let trace = parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated operation trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence" && event["operation"] == document.value["operation"]
    }));
    document.value
}

fn run_json_outcome(
    sandbox: &E2eSandbox,
    label: &str,
    expected_exit: i32,
    args: Vec<String>,
) -> Value {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_json(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(expected_exit),
        "stderr: {}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    let document =
        parse_structured_stdout(&invocation.stdout).expect("structured operation outcome");
    parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated operation trace");
    document.value
}

fn run_human(sandbox: &E2eSandbox, label: &str, operation: &str, args: Vec<String>) -> String {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_human(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&invocation.stderr)
    );
    assert!(invocation.stderr.is_empty());
    let stdout = String::from_utf8(invocation.stdout).expect("human stdout is UTF-8");
    assert!(stdout.contains(&format!("Operation: {operation}")));
    assert!(stdout.contains("Outcome: completed"));
    stdout
}

fn add_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    executable: &Path,
    provider_args: &[String],
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
    args.extend(["--timeout".into(), "60".into()]);
    let value = run_json(sandbox, "checkpoint-c-provider-add", args);
    value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

fn add_scenario_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    scenario: &str,
    ledger: &Path,
) -> String {
    add_provider(
        sandbox,
        handle,
        scenario_provider(),
        &[
            "--scenario".into(),
            scenario.into(),
            "--ledger-path".into(),
            ledger.display().to_string(),
        ],
    )
}

fn create_run(sandbox: &E2eSandbox, target: &str, label: &str, inputs: Option<Value>) -> Value {
    let mut args = vec![
        "run".into(),
        "create".into(),
        target.into(),
        "--label".into(),
        label.into(),
    ];
    if let Some(inputs) = inputs {
        let path = sandbox.caller_cwd().join(format!("{label}-inputs.json"));
        fs::write(&path, serde_json::to_vec(&inputs).unwrap()).expect("write inputs");
        args.extend(["--inputs".into(), path.display().to_string()]);
    }
    run_json(sandbox, "checkpoint-c-run-create", args)
}

fn run_id(value: &Value) -> String {
    value["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned()
}

fn graph(sandbox: &E2eSandbox, label: &str, run_id: &str) -> Value {
    let value = run_json(
        sandbox,
        label,
        vec!["run".into(), "graph".into(), run_id.into()],
    );
    assert_eq!(value["operation"], "run.graph");
    assert_eq!(value["outcome"], "completed");
    let trace = parse_correlated_value(&value, &sandbox.traces_dir()).expect("run.graph trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence"
            && event["operation"] == "run.graph"
            && event["event"] == "read_complete"
            && event["outcome"] == "completed"
    }));
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "request"
            && event["operation"] == "run.graph"
    }));
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "outcome"
            && event["envelope"]["operation"] == "run.graph"
    }));
    assert!(
        !trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    value
}

fn show(sandbox: &E2eSandbox, label: &str, run_id: &str) -> Value {
    let value = run_json(
        sandbox,
        label,
        vec!["run".into(), "show".into(), run_id.into()],
    );
    assert_eq!(value["operation"], "run.show");
    assert_eq!(value["outcome"], "completed");
    let trace = parse_correlated_value(&value, &sandbox.traces_dir()).expect("run.show trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence"
            && event["operation"] == "run.show"
            && event["event"] == "read_complete"
            && event["outcome"] == "completed"
    }));
    assert!(
        trace
            .events
            .iter()
            .any(|event| event["category"] == "invocation" && event["event"] == "start")
    );
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "request"
            && event["operation"] == "run.show"
    }));
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "outcome"
            && event["envelope"]["operation"] == "run.show"
    }));
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation" && event["event"] == "finish" && event["exit_code"] == 0
    }));
    assert!(
        !trace
            .events
            .iter()
            .any(|event| event["category"] == "provider")
    );
    value
}

#[test]
fn provider_update_and_rename_preserve_identity_binding_and_release_old_handle() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("catalog-lifecycle-ledger.jsonl");
    let registration = add_scenario_provider(&sandbox, "mutable", "graph-linear", &ledger);
    let occupied = add_scenario_provider(&sandbox, "occupied", "graph-linear", &ledger);
    let created = create_run(&sandbox, &registration, "stable-binding", None);
    let original_graph = graph(
        &sandbox,
        "checkpoint-c-provider-update-original-graph",
        &run_id(&created),
    );

    let updated = run_json(
        &sandbox,
        "checkpoint-c-provider-update",
        vec![
            "provider".into(),
            "update".into(),
            "mutable".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-initial-final".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--timeout".into(),
            "17".into(),
        ],
    );
    assert_eq!(updated["operation"], "provider.update");
    assert_eq!(updated["data"]["registration"]["id"], registration);
    assert_eq!(updated["data"]["registration"]["config_revision"], 2);
    assert_eq!(updated["data"]["affected_active_runs"], 1);

    let update_missing = sandbox.runner().run_json(
        "checkpoint-c-provider-update-missing",
        &[
            "provider",
            "update",
            "missing-provider",
            "--exec",
            "/missing/provider",
        ],
    );
    assert_eq!(update_missing.exit_code, Some(2));
    let update_missing = parse_structured_stdout(&update_missing.stdout).expect("update rejection");
    assert_eq!(update_missing.value["operation"], "provider.update");
    assert_eq!(update_missing.value["outcome"], "rejected");

    let renamed = run_json(
        &sandbox,
        "checkpoint-c-provider-rename",
        vec![
            "provider".into(),
            "rename".into(),
            registration.clone(),
            "renamed".into(),
        ],
    );
    assert_eq!(renamed["operation"], "provider.rename");
    assert_eq!(renamed["data"]["registration"]["id"], registration);
    assert_eq!(renamed["data"]["registration"]["handle"], "renamed");
    assert_eq!(renamed["data"]["registration"]["config_revision"], 2);

    let rejected = sandbox.runner().run_json(
        "checkpoint-c-provider-rename-occupied",
        &["provider", "rename", "renamed", "occupied"],
    );
    assert_eq!(rejected.exit_code, Some(2));
    let rejected = parse_structured_stdout(&rejected.stdout).expect("rename rejection");
    assert_eq!(rejected.value["operation"], "provider.rename");
    assert_eq!(rejected.value["outcome"], "rejected");

    let replacement = add_scenario_provider(&sandbox, "mutable", "graph-linear", &ledger);
    assert_ne!(replacement, registration);
    assert_ne!(replacement, occupied);
    let stored_graph = graph(
        &sandbox,
        "checkpoint-c-provider-update-stored-graph",
        &run_id(&created),
    );
    assert_eq!(
        stored_graph["data"]["graph_revision"],
        original_graph["data"]["graph_revision"]
    );

    let warning = run_json(
        &sandbox,
        "checkpoint-c-provider-disable-warning",
        vec!["provider".into(), "disable".into(), "renamed".into()],
    );
    assert_eq!(warning["operation"], "provider.disable");
    assert_eq!(warning["data"]["active_run_count"], 1);
    let ack = warning["data"]["ack_token"]
        .as_str()
        .expect("final warning page acknowledgement")
        .to_owned();

    let invalid_ack = sandbox.runner().run_json(
        "checkpoint-c-provider-disable-invalid-ack",
        &[
            "provider",
            "disable",
            "renamed",
            "--allow-active-runs",
            "invalid-ack",
        ],
    );
    assert_eq!(invalid_ack.exit_code, Some(2));
    let invalid_ack = parse_structured_stdout(&invalid_ack.stdout).expect("disable rejection");
    assert_eq!(invalid_ack.value["operation"], "provider.disable");
    assert_eq!(invalid_ack.value["outcome"], "rejected");

    let disabled = run_json(
        &sandbox,
        "checkpoint-c-provider-disable-authorized",
        vec![
            "provider".into(),
            "disable".into(),
            "renamed".into(),
            "--allow-active-runs".into(),
            ack,
        ],
    );
    assert_eq!(disabled["data"]["registration"]["enabled"], false);
    assert_eq!(disabled["data"]["registration"]["id"], registration);

    let after_disable = graph(
        &sandbox,
        "checkpoint-c-provider-disabled-stored-graph",
        &run_id(&created),
    );
    assert_eq!(after_disable["data"], stored_graph["data"]);

    let restored = run_json(
        &sandbox,
        "checkpoint-c-provider-restore",
        vec![
            "provider".into(),
            "restore".into(),
            registration.clone(),
            "--handle".into(),
            "restored".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-linear".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
            "--timeout".into(),
            "19".into(),
        ],
    );
    assert_eq!(restored["operation"], "provider.restore");
    assert_eq!(restored["data"]["registration"]["id"], registration);
    assert_eq!(restored["data"]["registration"]["handle"], "restored");
    assert_eq!(restored["data"]["registration"]["enabled"], true);

    let human_registration =
        add_scenario_provider(&sandbox, "human-mutable", "graph-linear", &ledger);
    run_human(
        &sandbox,
        "checkpoint-c-provider-update-human",
        "provider.update",
        vec![
            "provider".into(),
            "update".into(),
            "human-mutable".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-linear".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
        ],
    );
    run_human(
        &sandbox,
        "checkpoint-c-provider-rename-human",
        "provider.rename",
        vec![
            "provider".into(),
            "rename".into(),
            "human-mutable".into(),
            "human-renamed".into(),
        ],
    );
    let human_warning = run_json(
        &sandbox,
        "checkpoint-c-provider-disable-human-warning",
        vec!["provider".into(), "disable".into(), "human-renamed".into()],
    );
    let human_ack = human_warning["data"]["ack_token"]
        .as_str()
        .expect("zero-impact disable acknowledgement")
        .to_owned();
    run_human(
        &sandbox,
        "checkpoint-c-provider-disable-human",
        "provider.disable",
        vec![
            "provider".into(),
            "disable".into(),
            "human-renamed".into(),
            "--allow-active-runs".into(),
            human_ack,
        ],
    );
    run_human(
        &sandbox,
        "checkpoint-c-provider-restore-human",
        "provider.restore",
        vec![
            "provider".into(),
            "restore".into(),
            human_registration,
            "--handle".into(),
            "human-restored".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-linear".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
        ],
    );
}

#[test]
fn run_graph_projects_stored_graph_across_lifecycle_provider_drift_and_missing_provider() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("graph-provider-ledger.jsonl");
    let registration = add_scenario_provider(&sandbox, "graph-source", "graph-linear", &ledger);
    let active = create_run(&sandbox, &registration, "graph-active", None);
    let final_run = create_run(&sandbox, &registration, "graph-final", None);
    let terminated = create_run(&sandbox, &registration, "graph-terminated", None);

    let active_id = run_id(&active);
    let final_id = run_id(&final_run);
    let terminated_id = run_id(&terminated);
    let baseline = graph(&sandbox, "checkpoint-c-graph-active", &active_id);
    assert_eq!(baseline["data"]["graph"]["initial_state_id"], "start");
    assert_eq!(baseline["data"]["graph"]["canonical_graph_version"], 1);
    assert!(baseline["data"]["graph_revision"].as_str().is_some());

    set_run_projection_state(&sandbox.state_db_path(), &final_id, "done", "final")
        .expect("seed final run");
    run_json(
        &sandbox,
        "checkpoint-c-graph-terminate",
        vec!["run".into(), "terminate".into(), terminated_id.clone()],
    );
    set_provider_registration_command(
        &sandbox.state_db_path(),
        &registration,
        &["--scenario", "graph-initial-final"],
        60,
    )
    .expect("drift current provider configuration");
    tombstone_provider_registration(&sandbox.state_db_path(), &registration)
        .expect("remove current provider availability");
    fs::write(&ledger, []).expect("clear provider ledger before provider-free reads");

    for (label, id) in [
        ("checkpoint-c-graph-active-missing", &active_id),
        ("checkpoint-c-graph-final-missing", &final_id),
        ("checkpoint-c-graph-terminated-missing", &terminated_id),
    ] {
        let projected = graph(&sandbox, label, id);
        assert_eq!(
            projected["data"]["graph_revision"],
            baseline["data"]["graph_revision"]
        );
        assert_eq!(projected["data"]["graph"], baseline["data"]["graph"]);
    }
    assert_eq!(fs::read(&ledger).expect("read provider ledger"), b"");
}

#[test]
fn run_graph_not_found_is_rejected_with_correlated_persistence_trace() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_json(
        "checkpoint-c-graph-not-found",
        &["run", "graph", "missing-run"],
    );
    assert_eq!(invocation.exit_code, Some(2));
    let document = parse_structured_stdout(&invocation.stdout).expect("structured rejection");
    assert_eq!(document.value["operation"], "run.graph");
    assert_eq!(document.value["outcome"], "rejected");
    assert_eq!(document.value["reason"]["code"], "run.not_found");
    let trace = parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated run.graph rejection trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence"
            && event["operation"] == "run.graph"
            && event["event"] == "read_complete"
            && event["outcome"] == "rejected"
    }));
}

#[test]
fn run_show_projects_inputs_guidance_gates_and_provider_free_authoritative_lifecycles() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("provider-ledger.jsonl");

    let reference_registration = add_provider(&sandbox, "reference", reference_provider(), &[]);
    let reference = create_run(
        &sandbox,
        &reference_registration,
        "reference-active",
        Some(json!({"artifact_root": "/work/change", "change_id": "T153"})),
    );

    let linear_registration = add_scenario_provider(&sandbox, "linear", "graph-linear", &ledger);
    let neutral_final = create_run(
        &sandbox,
        &linear_registration,
        "neutral-final",
        Some(json!({"ticket": "LE-153"})),
    );
    let terminated = create_run(
        &sandbox,
        &linear_registration,
        "terminated",
        Some(json!({"ticket": "LE-153-T"})),
    );
    let missing_provider = create_run(
        &sandbox,
        &linear_registration,
        "missing-provider",
        Some(json!({"ticket": "LE-153-M"})),
    );

    let initial_final_registration =
        add_scenario_provider(&sandbox, "initial-final", "graph-initial-final", &ledger);
    let initial_final = create_run(&sandbox, &initial_final_registration, "initial-final", None);

    let zero_final_registration =
        add_scenario_provider(&sandbox, "zero-final", "graph-zero-final", &ledger);
    let zero_final = create_run(&sandbox, &zero_final_registration, "zero-final", None);

    let sink_registration =
        add_scenario_provider(&sandbox, "sink", "graph-non-final-sink", &ledger);
    let sink = create_run(&sandbox, &sink_registration, "non-final-sink", None);

    let neutral_final_id = run_id(&neutral_final);
    set_run_projection_state(&sandbox.state_db_path(), &neutral_final_id, "done", "final")
        .expect("seed reached neutral-final projection");
    let sink_id = run_id(&sink);
    set_run_projection_state(&sandbox.state_db_path(), &sink_id, "sink", "active")
        .expect("seed reached non-final sink projection");

    let terminated_id = run_id(&terminated);
    run_json(
        &sandbox,
        "checkpoint-c-terminate",
        vec!["run".into(), "terminate".into(), terminated_id.clone()],
    );
    tombstone_provider_registration(&sandbox.state_db_path(), &linear_registration)
        .expect("tombstone provider after run creation");

    fs::write(&ledger, []).expect("clear provider invocation ledger before reads");

    let active = show(&sandbox, "checkpoint-c-show-active", &run_id(&reference));
    assert_eq!(active["data"]["run"]["label"], "reference-active");
    assert_eq!(active["data"]["run"]["lifecycle"], "active");
    assert_eq!(active["data"]["run"]["state"], "explore");
    assert_eq!(active["data"]["inputs"]["artifact_root"], "/work/change");
    assert_eq!(active["data"]["inputs"]["change_id"], "T153");
    assert_eq!(active["data"]["static_guidance"]["kind"], "text");
    assert_eq!(
        active["data"]["static_guidance"]["text"],
        "Explore the change context and capture intent."
    );
    assert_eq!(active["data"]["live_guidance"], "supported");
    assert_eq!(active["data"]["selected_evidence"], json!([]));
    assert_eq!(
        active["data"]["requestable_events"],
        json!(["intent-ready"])
    );
    assert_eq!(
        active["data"]["requestable_event_details"],
        json!([{
            "event": "intent-ready",
            "target": "design",
            "required_gates": ["intent-ready"]
        }])
    );
    assert!(
        active["data"]["graph_revision"]
            .as_str()
            .is_some_and(|revision| revision.starts_with("sha256:"))
    );

    let final_run = show(
        &sandbox,
        "checkpoint-c-show-neutral-final",
        &neutral_final_id,
    );
    assert_eq!(final_run["data"]["run"]["lifecycle"], "final");
    assert_eq!(final_run["data"]["run"]["state"], "done");
    assert_eq!(
        final_run["data"]["static_guidance"]["kind"],
        "none_required"
    );
    assert_eq!(final_run["data"]["live_guidance"], "unsupported");
    assert_eq!(final_run["data"]["requestable_events"], json!([]));
    assert_eq!(final_run["data"]["requestable_event_details"], json!([]));

    let initial = show(
        &sandbox,
        "checkpoint-c-show-initial-final",
        &run_id(&initial_final),
    );
    assert_eq!(initial["data"]["run"]["lifecycle"], "final");
    assert_eq!(initial["data"]["run"]["state"], "done");
    assert_eq!(initial["data"]["requestable_events"], json!([]));

    let ongoing = show(
        &sandbox,
        "checkpoint-c-show-zero-final",
        &run_id(&zero_final),
    );
    assert_eq!(ongoing["data"]["run"]["lifecycle"], "active");
    assert_eq!(ongoing["data"]["run"]["state"], "ongoing");
    assert_eq!(ongoing["data"]["requestable_events"], json!(["advance"]));

    let sink = show(&sandbox, "checkpoint-c-show-sink", &sink_id);
    assert_eq!(sink["data"]["run"]["lifecycle"], "active");
    assert_eq!(sink["data"]["run"]["state"], "sink");
    assert_eq!(sink["data"]["requestable_events"], json!([]));

    let terminated = show(&sandbox, "checkpoint-c-show-terminated", &terminated_id);
    assert_eq!(terminated["data"]["run"]["lifecycle"], "terminated");
    assert_eq!(terminated["data"]["requestable_events"], json!([]));
    assert_eq!(terminated["data"]["requestable_event_details"], json!([]));

    let missing = show(
        &sandbox,
        "checkpoint-c-show-missing-provider",
        &run_id(&missing_provider),
    );
    assert_eq!(missing["data"]["run"]["lifecycle"], "active");
    assert_eq!(missing["data"]["inputs"]["ticket"], "LE-153-M");
    assert!(fs::read(&ledger).expect("read provider ledger").is_empty());

    let human = sandbox.runner().run_human(
        "checkpoint-c-show-human",
        &["run", "show", &run_id(&reference)],
    );
    assert_eq!(human.exit_code, Some(0));
    assert!(human.stderr.is_empty());
    let text = String::from_utf8(human.stdout).expect("human output utf8");
    assert!(text.contains("Inputs: {\"artifact_root\":\"/work/change\",\"change_id\":\"T153\"}"));
    assert!(text.contains("Guidance: Explore the change context and capture intent."));
    assert!(text.contains("Live guidance: supported"));
    assert!(text.contains("Selected evidence: none"));
    assert!(text.contains("intent-ready -> design (required gates: intent-ready)"));

    let reference_id = run_id(&reference);
    set_run_projection_state(
        &sandbox.state_db_path(),
        &reference_id,
        "design-review",
        "active",
    )
    .expect("seed multi-event active projection");
    let multi_event = show(&sandbox, "checkpoint-c-show-multi-event", &reference_id);
    assert_eq!(
        multi_event["data"]["requestable_events"],
        json!(["approved", "changes-requested"])
    );
    assert_eq!(
        multi_event["data"]["requestable_event_details"][0]["event"],
        "approved"
    );
    assert_eq!(
        multi_event["data"]["requestable_event_details"][1]["event"],
        "changes-requested"
    );
    assert!(fs::read(&ledger).expect("read provider ledger").is_empty());
}

#[test]
fn run_show_not_found_is_rejected_with_correlated_persistence_trace() {
    let sandbox = E2eSandbox::new();
    let missing = "019f0000-0000-7000-8000-000000000153";
    let invocation = sandbox
        .runner()
        .run_json("checkpoint-c-show-not-found", &["run", "show", missing]);
    assert_eq!(invocation.exit_code, Some(2));
    assert!(invocation.stderr.is_empty());
    let document = parse_structured_stdout(&invocation.stdout).expect("structured error");
    assert_eq!(document.value["operation"], "run.show");
    assert_eq!(document.value["outcome"], "rejected");
    assert_eq!(document.value["reason"]["code"], "run.not_found");
    let trace = parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated error trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "persistence"
            && event["operation"] == "run.show"
            && event["event"] == "read_complete"
            && event["outcome"] == "rejected"
    }));
}

#[test]
fn deferred_run_operations_close_evidence_metadata_provider_and_export_paths() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox
        .caller_cwd()
        .join("deferred-operations-ledger.jsonl");
    let base_registration =
        add_scenario_provider(&sandbox, "deferred-base", "graph-linear", &ledger);
    let created = create_run(&sandbox, &base_registration, "deferred-run", None);
    let id = run_id(&created);

    let metadata = sandbox.caller_cwd().join("evidence-metadata.json");
    fs::write(&metadata, br#"{"owner":"caller","nested":{"rank":1}}"#)
        .expect("write evidence metadata");
    let added = run_json(
        &sandbox,
        "checkpoint-c-evidence-add",
        vec![
            "run".into(),
            "evidence".into(),
            "add".into(),
            id.clone(),
            "--kind".into(),
            "artifact".into(),
            "--ref".into(),
            "opaque:deferred-artifact".into(),
            "--digest".into(),
            format!("sha256:{}", "a".repeat(64)),
            "--media-type".into(),
            "application/json".into(),
            "--metadata".into(),
            metadata.display().to_string(),
        ],
    );
    assert_eq!(added["operation"], "run.evidence.add");
    assert_eq!(added["data"]["run"]["id"], id);
    assert_eq!(added["data"]["evidence_added"], true);
    assert!(added["data"]["requestable_events"].is_array());
    let evidence_id = added["data"]["evidence_id"]
        .as_str()
        .expect("evidence id")
        .to_owned();

    let listed = run_json(
        &sandbox,
        "checkpoint-c-evidence-list",
        vec!["run".into(), "evidence".into(), "list".into(), id.clone()],
    );
    assert_eq!(listed["operation"], "run.evidence.list");
    assert_eq!(listed["data"]["items"][0]["id"], evidence_id);
    assert_eq!(
        listed["data"]["items"][0]["locator"],
        "opaque:deferred-artifact"
    );

    let annotation_noop = run_json(
        &sandbox,
        "checkpoint-c-run-annotate-noop",
        vec!["run".into(), "annotate".into(), id.clone()],
    );
    assert_eq!(annotation_noop["data"]["run"]["id"], id);
    assert_eq!(annotation_noop["data"]["changed"], false);
    assert!(annotation_noop["data"]["requestable_events"].is_array());

    let actor = sandbox.caller_cwd().join("annotation-actor.json");
    fs::write(&actor, br#"{"kind":"operator","name":"local"}"#).expect("write actor metadata");
    let annotated = run_json(
        &sandbox,
        "checkpoint-c-run-annotate",
        vec![
            "run".into(),
            "annotate".into(),
            id.clone(),
            "--note".into(),
            "caller annotation".into(),
            "--actor".into(),
            actor.display().to_string(),
            "--corrects".into(),
            "1".into(),
        ],
    );
    assert_eq!(annotated["operation"], "run.annotate");
    assert_eq!(annotated["data"]["run"]["id"], id);
    assert!(annotated["data"]["requestable_events"].is_array());

    let labeled = run_json(
        &sandbox,
        "checkpoint-c-run-label",
        vec![
            "run".into(),
            "label".into(),
            id.clone(),
            "--set".into(),
            "renamed-deferred-run".into(),
        ],
    );
    assert_eq!(labeled["data"]["run"]["label"], "renamed-deferred-run");

    let export_dir = sandbox.caller_cwd().join("deferred-export");
    let exported = run_json(
        &sandbox,
        "checkpoint-c-run-export",
        vec![
            "run".into(),
            "export".into(),
            id.clone(),
            "--output".into(),
            export_dir.display().to_string(),
        ],
    );
    assert_eq!(exported["operation"], "run.export");
    assert_eq!(
        exported["data"]["export"]["output"],
        export_dir.display().to_string()
    );
    assert_eq!(exported["data"]["export"]["manifest_file"], "manifest.json");
    assert_eq!(exported["data"]["export"]["state_file"], "state.json");
    assert_eq!(exported["data"]["export"]["journal_file"], "journal.jsonl");
    assert!(export_dir.join("manifest.json").is_file());
    assert!(export_dir.join("state.json").is_file());
    assert!(export_dir.join("journal.jsonl").is_file());

    let guidance_registration = add_scenario_provider(
        &sandbox,
        "deferred-guidance",
        "graph-guidance-supported",
        &ledger,
    );
    let guidance_run = create_run(&sandbox, &guidance_registration, "guidance-run", None);
    run_json(
        &sandbox,
        "checkpoint-c-guidance-provider-update",
        vec![
            "provider".into(),
            "update".into(),
            "deferred-guidance".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=guidance-text".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--timeout".into(),
            "60".into(),
        ],
    );
    let guidance = run_json(
        &sandbox,
        "checkpoint-c-run-guidance",
        vec!["run".into(), "guidance".into(), run_id(&guidance_run)],
    );
    assert_eq!(guidance["operation"], "run.guidance");
    assert_eq!(guidance["data"]["run"]["id"], run_id(&guidance_run));
    assert!(guidance["data"]["requestable_events"].is_array());
    assert_eq!(guidance["data"]["provider_executed"], true);
    assert!(guidance["data"]["guidance"].as_str().is_some());

    let compatibility_registration = add_scenario_provider(
        &sandbox,
        "deferred-compatibility",
        "compatibility-all-compatible",
        &ledger,
    );
    let compatibility_run = create_run(
        &sandbox,
        &compatibility_registration,
        "compatibility-run",
        None,
    );
    let compatibility = run_json(
        &sandbox,
        "checkpoint-c-run-compatibility",
        vec![
            "run".into(),
            "compatibility".into(),
            run_id(&compatibility_run),
        ],
    );
    assert_eq!(compatibility["operation"], "run.compatibility");
    assert_eq!(
        compatibility["data"]["run"]["id"],
        run_id(&compatibility_run)
    );
    assert!(compatibility["data"]["requestable_events"].is_array());
    assert_eq!(compatibility["data"]["provider_executed"], true);
    assert!(compatibility["data"]["findings"].as_array().is_some());

    let assert_journal_correlation = |history: &Value, operation: &str, envelope: &Value| {
        let entry = history["data"]["items"]
            .as_array()
            .expect("history items")
            .iter()
            .find(|entry| entry["operation"] == operation)
            .unwrap_or_else(|| panic!("journal contains {operation}"));
        assert_eq!(entry["request_id"], envelope["request_id"]);
    };
    let base_history = run_json(
        &sandbox,
        "checkpoint-c-deferred-base-history",
        vec!["run".into(), "history".into(), id.clone()],
    );
    assert_journal_correlation(&base_history, "run.evidence.add", &added);
    assert_journal_correlation(&base_history, "run.annotate", &annotated);
    assert_journal_correlation(&base_history, "run.label", &labeled);
    let guidance_history = run_json(
        &sandbox,
        "checkpoint-c-deferred-guidance-history",
        vec!["run".into(), "history".into(), run_id(&guidance_run)],
    );
    assert_journal_correlation(&guidance_history, "run.guidance", &guidance);
    let compatibility_history = run_json(
        &sandbox,
        "checkpoint-c-deferred-compatibility-history",
        vec!["run".into(), "history".into(), run_id(&compatibility_run)],
    );
    assert_journal_correlation(&compatibility_history, "run.compatibility", &compatibility);

    let evidence_human = run_human(
        &sandbox,
        "checkpoint-c-evidence-add-human",
        "run.evidence.add",
        vec![
            "run".into(),
            "evidence".into(),
            "add".into(),
            id.clone(),
            "--kind".into(),
            "note".into(),
            "--ref".into(),
            "opaque:human-evidence".into(),
        ],
    );
    assert!(evidence_human.contains("Evidence recorded: yes"));
    run_human(
        &sandbox,
        "checkpoint-c-evidence-list-human",
        "run.evidence.list",
        vec!["run".into(), "evidence".into(), "list".into(), id.clone()],
    );
    run_human(
        &sandbox,
        "checkpoint-c-annotate-human",
        "run.annotate",
        vec![
            "run".into(),
            "annotate".into(),
            id.clone(),
            "--note".into(),
            "human annotation".into(),
        ],
    );
    run_human(
        &sandbox,
        "checkpoint-c-label-human",
        "run.label",
        vec!["run".into(), "label".into(), id.clone(), "--clear".into()],
    );
    run_human(
        &sandbox,
        "checkpoint-c-guidance-human",
        "run.guidance",
        vec!["run".into(), "guidance".into(), run_id(&guidance_run)],
    );
    run_human(
        &sandbox,
        "checkpoint-c-compatibility-human",
        "run.compatibility",
        vec![
            "run".into(),
            "compatibility".into(),
            run_id(&compatibility_run),
        ],
    );
    let export_human = run_human(
        &sandbox,
        "checkpoint-c-export-human",
        "run.export",
        vec![
            "run".into(),
            "export".into(),
            id,
            "--output".into(),
            sandbox
                .caller_cwd()
                .join("deferred-export-human")
                .display()
                .to_string(),
        ],
    );
    assert!(export_human.contains("Output:"));
    assert!(export_human.contains("Manifest: manifest.json"));
    assert!(export_human.contains("State file: state.json"));
    assert!(export_human.contains("Journal file: journal.jsonl"));

    run_json(
        &sandbox,
        "checkpoint-c-guidance-provider-evaluation-error-update",
        vec![
            "provider".into(),
            "update".into(),
            "deferred-guidance".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=guidance-evaluation-error".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
        ],
    );
    let guidance_error = run_json_outcome(
        &sandbox,
        "checkpoint-c-guidance-provider-evaluation-error",
        1,
        vec!["run".into(), "guidance".into(), run_id(&guidance_run)],
    );
    assert_eq!(guidance_error["outcome"], "error");
    assert_eq!(
        guidance_error["reason"]["code"],
        "provider.evaluation_error"
    );
    assert_eq!(guidance_error["data"]["provider_executed"], true);
    assert_eq!(guidance_error["data"]["run"]["id"], run_id(&guidance_run));
    let guidance_error_trace = parse_correlated_value(&guidance_error, &sandbox.traces_dir())
        .expect("guidance error trace");
    assert!(guidance_error_trace.events.iter().any(|event| {
        event["category"] == "provider"
            && event["role"] == "live_guidance"
            && event["event"] == "finish"
    }));

    run_json(
        &sandbox,
        "checkpoint-c-compatibility-provider-evaluation-error-update",
        vec![
            "provider".into(),
            "update".into(),
            "deferred-compatibility".into(),
            "--exec".into(),
            scenario_provider().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=compatibility-evaluation-error".into(),
            "--arg=--ledger-path".into(),
            format!("--arg={}", ledger.display()),
        ],
    );
    let compatibility_error = run_json_outcome(
        &sandbox,
        "checkpoint-c-compatibility-provider-evaluation-error",
        1,
        vec![
            "run".into(),
            "compatibility".into(),
            run_id(&compatibility_run),
        ],
    );
    assert_eq!(compatibility_error["outcome"], "error");
    assert_eq!(
        compatibility_error["reason"]["code"],
        "provider.evaluation_error"
    );
    assert_eq!(compatibility_error["data"]["provider_executed"], true);
    assert_eq!(
        compatibility_error["data"]["run"]["id"],
        run_id(&compatibility_run)
    );
    let compatibility_error_trace =
        parse_correlated_value(&compatibility_error, &sandbox.traces_dir())
            .expect("compatibility error trace");
    assert!(compatibility_error_trace.events.iter().any(|event| {
        event["category"] == "provider"
            && event["role"] == "check_compatibility"
            && event["event"] == "finish"
    }));
}

#[test]
fn provider_attempt_prelaunch_failures_report_provider_not_executed() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("prelaunch-ledger.jsonl");
    let registration = add_scenario_provider(
        &sandbox,
        "prelaunch-failure",
        "graph-guidance-supported",
        &ledger,
    );
    let run = create_run(&sandbox, &registration, "prelaunch-failure", None);
    let missing = sandbox.caller_cwd().join("missing-provider");
    run_json(
        &sandbox,
        "checkpoint-c-prelaunch-update",
        vec![
            "provider".into(),
            "update".into(),
            "prelaunch-failure".into(),
            "--exec".into(),
            missing.display().to_string(),
        ],
    );

    for (label, operation) in [
        ("checkpoint-c-prelaunch-guidance", "guidance"),
        ("checkpoint-c-prelaunch-compatibility", "compatibility"),
    ] {
        let value = run_json_outcome(
            &sandbox,
            label,
            1,
            vec!["run".into(), operation.into(), run_id(&run)],
        );
        assert_eq!(value["reason"]["code"], "provider.executable.not_found");
        assert_eq!(value["data"]["provider_executed"], false);
        assert_eq!(value["data"]["run"]["id"], run_id(&run));
    }
}

#[test]
fn provider_attempt_lifecycle_races_report_committed_terminal_state_and_execution_truth() {
    for (operation, graph_scenario, provider_scenario) in [
        ("guidance", "graph-guidance-supported", "guidance-text"),
        (
            "compatibility",
            "compatibility-all-compatible",
            "compatibility-all-compatible",
        ),
    ] {
        let sandbox = E2eSandbox::new();
        let ledger = sandbox
            .caller_cwd()
            .join(format!("{operation}-race-ledger.jsonl"));
        let barrier = sandbox
            .caller_cwd()
            .join(format!("{operation}-race-barrier"));
        let registration = add_scenario_provider(
            &sandbox,
            &format!("{operation}-race"),
            graph_scenario,
            &ledger,
        );
        let run = create_run(
            &sandbox,
            &registration,
            &format!("{operation}-race-run"),
            None,
        );
        let id = run_id(&run);
        let barrier_text = barrier.display().to_string();
        let ledger_text = ledger.display().to_string();
        set_provider_registration_command(
            &sandbox.state_db_path(),
            &registration,
            &[
                "--scenario",
                provider_scenario,
                "--ledger-path",
                &ledger_text,
                "--barrier-dir",
                &barrier_text,
                "--barrier-id",
                operation,
                "--barrier-action",
                "reached",
            ],
            60,
        )
        .unwrap();

        let raced = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                run_json_outcome(
                    &sandbox,
                    &format!("checkpoint-c-{operation}-lifecycle-race"),
                    2,
                    vec!["run".into(), operation.into(), id.clone()],
                )
            });
            let reached = wait_for_barrier(&barrier.join(operation).join("reached"));
            let terminated = reached.then(|| {
                run_json(
                    &sandbox,
                    &format!("checkpoint-c-{operation}-race-terminate"),
                    vec!["run".into(), "terminate".into(), id.clone()],
                )
            });
            fs::create_dir_all(barrier.join(operation)).unwrap();
            fs::write(barrier.join(operation).join("release"), b"release").unwrap();
            let raced = worker.join().unwrap();
            assert!(reached, "{operation} provider reached barrier");
            assert_eq!(
                terminated.expect("termination after barrier")["data"]["run"]["lifecycle"],
                "terminated"
            );
            raced
        });

        assert_eq!(raced["outcome"], "rejected");
        assert_eq!(raced["reason"]["code"], "run.lifecycle.terminal");
        assert_eq!(
            raced["reason"]["message"],
            format!("run lifecycle changed before {operation} committed")
        );
        assert_eq!(raced["data"]["provider_executed"], true);
        assert_eq!(raced["data"]["run"]["id"], id);
        assert_eq!(raced["data"]["run"]["lifecycle"], "terminated");
        assert_eq!(raced["data"]["run"]["state_changed"], false);
        assert_eq!(raced["data"]["requestable_events"], json!([]));
        if operation == "guidance" {
            assert!(raced["data"]["guidance"].is_null());
        } else {
            assert!(raced["data"]["findings"].is_null());
        }
    }
}

#[test]
fn provider_attempt_workflow_version_races_reject_stale_results() {
    for (operation, graph_scenario, provider_scenario) in [
        ("guidance", "graph-guidance-supported", "guidance-text"),
        (
            "compatibility",
            "compatibility-all-compatible",
            "compatibility-all-compatible",
        ),
    ] {
        let sandbox = E2eSandbox::new();
        let ledger = sandbox
            .caller_cwd()
            .join(format!("{operation}-state-race-ledger.jsonl"));
        let barrier = sandbox
            .caller_cwd()
            .join(format!("{operation}-state-race-barrier"));
        let registration = add_scenario_provider(
            &sandbox,
            &format!("{operation}-state-race"),
            graph_scenario,
            &ledger,
        );
        let run = create_run(
            &sandbox,
            &registration,
            &format!("{operation}-state-race-run"),
            None,
        );
        let id = run_id(&run);
        let barrier_text = barrier.display().to_string();
        let ledger_text = ledger.display().to_string();
        set_provider_registration_command(
            &sandbox.state_db_path(),
            &registration,
            &[
                "--scenario",
                provider_scenario,
                "--ledger-path",
                &ledger_text,
                "--barrier-dir",
                &barrier_text,
                "--barrier-id",
                operation,
                "--barrier-action",
                "reached",
            ],
            60,
        )
        .unwrap();

        let stale = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                run_json_outcome(
                    &sandbox,
                    &format!("checkpoint-c-{operation}-workflow-version-race"),
                    1,
                    vec!["run".into(), operation.into(), id.clone()],
                )
            });
            let reached = wait_for_barrier(&barrier.join(operation).join("reached"));
            if reached {
                execute_sql(
                    &sandbox.state_db_path(),
                    &format!(
                        "UPDATE runs SET workflow_state_version = workflow_state_version + 1 \
                         WHERE run_id = '{id}';"
                    ),
                )
                .unwrap();
            }
            fs::create_dir_all(barrier.join(operation)).unwrap();
            fs::write(barrier.join(operation).join("release"), b"release").unwrap();
            let stale = worker.join().unwrap();
            assert!(reached, "{operation} provider reached barrier");
            stale
        });

        assert_eq!(stale["outcome"], "error");
        assert_eq!(stale["reason"]["code"], "state.stale_version");
        assert_eq!(
            stale["reason"]["message"],
            format!("run state changed before {operation} committed")
        );
        assert_eq!(stale["data"]["provider_executed"], true);
        assert_eq!(stale["data"]["run"]["id"], id);
        assert_eq!(stale["data"]["run"]["lifecycle"], "active");
        assert_eq!(stale["data"]["run"]["state_changed"], false);
        assert_eq!(stale["data"]["workflow_state_version"], 2);
        if operation == "guidance" {
            assert!(stale["data"]["guidance"].is_null());
        } else {
            assert!(stale["data"]["findings"].is_null());
        }
    }
}

#[test]
fn post_provider_persistence_failures_keep_resolved_run_and_execution_truth() {
    for (operation, graph_scenario, provider_scenario) in [
        ("guidance", "graph-guidance-supported", "guidance-text"),
        (
            "compatibility",
            "compatibility-all-compatible",
            "compatibility-all-compatible",
        ),
    ] {
        let sandbox = E2eSandbox::new();
        let ledger = sandbox
            .caller_cwd()
            .join(format!("{operation}-failure-ledger.jsonl"));
        let barrier = sandbox
            .caller_cwd()
            .join(format!("{operation}-failure-barrier"));
        let registration = add_scenario_provider(
            &sandbox,
            &format!("{operation}-failure"),
            graph_scenario,
            &ledger,
        );
        let run = create_run(
            &sandbox,
            &registration,
            &format!("{operation}-failure-run"),
            None,
        );
        let id = run_id(&run);
        let barrier_text = barrier.display().to_string();
        let ledger_text = ledger.display().to_string();
        set_provider_registration_command(
            &sandbox.state_db_path(),
            &registration,
            &[
                "--scenario",
                provider_scenario,
                "--ledger-path",
                &ledger_text,
                "--barrier-dir",
                &barrier_text,
                "--barrier-id",
                operation,
                "--barrier-action",
                "reached",
            ],
            60,
        )
        .unwrap();

        let failed = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                run_json_outcome(
                    &sandbox,
                    &format!("checkpoint-c-{operation}-post-provider-persistence-failure"),
                    1,
                    vec!["run".into(), operation.into(), id.clone()],
                )
            });
            let reached = wait_for_barrier(&barrier.join(operation).join("reached"));
            if reached {
                execute_sql(
                    &sandbox.state_db_path(),
                    "CREATE TRIGGER provider_attempt_fault BEFORE INSERT ON journal_entries \
                     BEGIN SELECT RAISE(ABORT, 'provider attempt fault'); END;",
                )
                .unwrap();
            }
            fs::create_dir_all(barrier.join(operation)).unwrap();
            fs::write(barrier.join(operation).join("release"), b"release").unwrap();
            let failed = worker.join().unwrap();
            assert!(reached, "{operation} provider reached barrier");
            failed
        });

        assert_eq!(failed["outcome"], "error");
        assert_eq!(failed["reason"]["code"], "persistence.failed");
        assert_eq!(failed["data"]["provider_executed"], true);
        assert_eq!(failed["data"]["run"]["id"], id);
        assert_eq!(failed["data"]["run"]["lifecycle"], "active");
        assert_eq!(failed["data"]["run"]["state_changed"], false);
        assert!(failed["data"]["requestable_events"].is_array());
    }
}

#[test]
fn deferred_run_operations_cover_not_found_terminal_invalid_input_and_export_collision() {
    let sandbox = E2eSandbox::new();
    let ledger = sandbox.caller_cwd().join("deferred-negative-ledger.jsonl");
    let registration =
        add_scenario_provider(&sandbox, "deferred-negative", "graph-linear", &ledger);
    let created = create_run(&sandbox, &registration, "deferred-negative", None);
    let id = run_id(&created);

    let invalid_metadata = sandbox.caller_cwd().join("invalid-metadata.json");
    let invalid_actor = sandbox.caller_cwd().join("invalid-actor.json");
    fs::write(&invalid_metadata, "[]").expect("write invalid metadata");
    fs::write(&invalid_actor, "[]").expect("write invalid actor");

    let invalid_evidence = run_json_outcome(
        &sandbox,
        "checkpoint-c-evidence-invalid-metadata",
        2,
        vec![
            "run".into(),
            "evidence".into(),
            "add".into(),
            id.clone(),
            "--kind".into(),
            "report".into(),
            "--ref".into(),
            "opaque:invalid-metadata".into(),
            "--metadata".into(),
            invalid_metadata.display().to_string(),
        ],
    );
    assert_eq!(invalid_evidence["reason"]["code"], "evidence.invalid");
    assert_eq!(invalid_evidence["data"]["run"]["id"], id);
    assert!(invalid_evidence["data"]["requestable_events"].is_array());
    let invalid_annotation = run_json_outcome(
        &sandbox,
        "checkpoint-c-annotate-invalid-actor",
        2,
        vec![
            "run".into(),
            "annotate".into(),
            id.clone(),
            "--note".into(),
            "invalid actor".into(),
            "--actor".into(),
            invalid_actor.display().to_string(),
        ],
    );
    assert_eq!(invalid_annotation["reason"]["code"], "actor.invalid");
    assert_eq!(invalid_annotation["data"]["run"]["id"], id);
    assert!(invalid_annotation["data"]["requestable_events"].is_array());

    run_json(
        &sandbox,
        "checkpoint-c-deferred-negative-terminate",
        vec!["run".into(), "terminate".into(), id.clone()],
    );
    let disable_warning = run_json(
        &sandbox,
        "checkpoint-c-deferred-negative-disable-warning",
        vec![
            "provider".into(),
            "disable".into(),
            "deferred-negative".into(),
        ],
    );
    let disable_ack = disable_warning["data"]["ack_token"]
        .as_str()
        .expect("terminal run leaves zero active impact")
        .to_owned();
    run_json(
        &sandbox,
        "checkpoint-c-deferred-negative-disable",
        vec![
            "provider".into(),
            "disable".into(),
            "deferred-negative".into(),
            "--allow-active-runs".into(),
            disable_ack,
        ],
    );
    for (label, operation, args) in [
        (
            "checkpoint-c-terminal-evidence-add",
            "run.evidence.add",
            vec![
                "run".into(),
                "evidence".into(),
                "add".into(),
                id.clone(),
                "--kind".into(),
                "report".into(),
                "--ref".into(),
                "opaque:terminal".into(),
            ],
        ),
        (
            "checkpoint-c-terminal-evidence-list",
            "run.evidence.list",
            vec!["run".into(), "evidence".into(), "list".into(), id.clone()],
        ),
        (
            "checkpoint-c-terminal-annotate",
            "run.annotate",
            vec![
                "run".into(),
                "annotate".into(),
                id.clone(),
                "--note".into(),
                "terminal annotation".into(),
            ],
        ),
    ] {
        let value = run_json_outcome(&sandbox, label, 0, args);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["outcome"], "completed");
    }
    for (label, operation, args) in [
        (
            "checkpoint-c-terminal-label",
            "run.label",
            vec![
                "run".into(),
                "label".into(),
                id.clone(),
                "--set".into(),
                "blocked".into(),
            ],
        ),
        (
            "checkpoint-c-terminal-guidance",
            "run.guidance",
            vec!["run".into(), "guidance".into(), id.clone()],
        ),
        (
            "checkpoint-c-terminal-compatibility",
            "run.compatibility",
            vec!["run".into(), "compatibility".into(), id.clone()],
        ),
    ] {
        let value = run_json_outcome(&sandbox, label, 2, args);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["reason"]["code"], "run.lifecycle.terminal");
        if matches!(operation, "run.guidance" | "run.compatibility") {
            assert_eq!(value["reason"]["message"], "run lifecycle is terminal");
        }
        assert_eq!(value["data"]["run"]["id"], id);
        assert_eq!(value["data"]["run"]["lifecycle"], "terminated");
        assert_eq!(value["data"]["requestable_events"], json!([]));
        if operation != "run.label" {
            assert_eq!(value["data"]["provider_executed"], false);
        }
    }

    let export_target = sandbox.caller_cwd().join("terminal-export");
    let exported = run_json_outcome(
        &sandbox,
        "checkpoint-c-terminal-export",
        0,
        vec![
            "run".into(),
            "export".into(),
            id.clone(),
            "--output".into(),
            export_target.display().to_string(),
        ],
    );
    assert_eq!(exported["operation"], "run.export");
    let collision = run_json_outcome(
        &sandbox,
        "checkpoint-c-export-collision",
        2,
        vec![
            "run".into(),
            "export".into(),
            id,
            "--output".into(),
            export_target.display().to_string(),
        ],
    );
    assert_eq!(collision["operation"], "run.export");
    assert_eq!(collision["outcome"], "rejected");
    assert_eq!(collision["reason"]["code"], "export.target.not_empty");

    let missing_output = sandbox.caller_cwd().join("missing-export");
    for (label, operation, args) in [
        (
            "checkpoint-c-missing-evidence-add",
            "run.evidence.add",
            vec![
                "run".into(),
                "evidence".into(),
                "add".into(),
                "missing-run".into(),
                "--kind".into(),
                "report".into(),
                "--ref".into(),
                "opaque:missing".into(),
            ],
        ),
        (
            "checkpoint-c-missing-evidence-list",
            "run.evidence.list",
            vec![
                "run".into(),
                "evidence".into(),
                "list".into(),
                "missing-run".into(),
            ],
        ),
        (
            "checkpoint-c-missing-annotate",
            "run.annotate",
            vec![
                "run".into(),
                "annotate".into(),
                "missing-run".into(),
                "--note".into(),
                "missing".into(),
            ],
        ),
        (
            "checkpoint-c-missing-label",
            "run.label",
            vec![
                "run".into(),
                "label".into(),
                "missing-run".into(),
                "--clear".into(),
            ],
        ),
        (
            "checkpoint-c-missing-guidance",
            "run.guidance",
            vec!["run".into(), "guidance".into(), "missing-run".into()],
        ),
        (
            "checkpoint-c-missing-compatibility",
            "run.compatibility",
            vec!["run".into(), "compatibility".into(), "missing-run".into()],
        ),
        (
            "checkpoint-c-missing-export",
            "run.export",
            vec![
                "run".into(),
                "export".into(),
                "missing-run".into(),
                "--output".into(),
                missing_output.display().to_string(),
            ],
        ),
    ] {
        let value = run_json_outcome(&sandbox, label, 2, args);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["reason"]["code"], "run.not_found");
    }
}
