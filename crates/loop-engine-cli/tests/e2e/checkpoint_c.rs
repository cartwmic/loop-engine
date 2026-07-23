use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::support::{
    E2eSandbox, parse_correlated_value, parse_structured_stdout, reference_provider_executable,
    scenario_provider_executable, set_run_projection_state, tombstone_provider_registration,
};

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
