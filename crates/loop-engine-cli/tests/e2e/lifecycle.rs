use crate::support::{E2eSandbox, add_scenario_provider, create_run, invoke_json};

fn show(sandbox: &E2eSandbox, run_id: &str, label: &str) -> serde_json::Value {
    invoke_json(
        sandbox,
        label,
        &["run".into(), "show".into(), run_id.into()],
        0,
    )
    .document
    .value
}

#[test]
fn lifecycle_family_is_stable_across_fresh_processes() {
    let sandbox = E2eSandbox::new();
    let linear = add_scenario_provider(&sandbox, "life-linear", "graph-linear", &[]);
    let active = create_run(&sandbox, &linear, "life-active");
    assert_eq!(
        show(&sandbox, &active, "life-active-show")["data"]["run"]["lifecycle"],
        "active"
    );

    for event in ["advance", "finish"] {
        let requested = invoke_json(
            &sandbox,
            &format!("life-{event}"),
            &["run".into(), "request".into(), active.clone(), event.into()],
            0,
        );
        assert_eq!(requested.document.value["outcome"], "completed");
    }
    let completed = show(&sandbox, &active, "life-completed-show");
    assert_eq!(completed["data"]["run"]["lifecycle"], "final");
    let final_rejection = invoke_json(
        &sandbox,
        "life-completed-request",
        &[
            "run".into(),
            "request".into(),
            active.clone(),
            "finish".into(),
        ],
        2,
    );
    assert_eq!(
        final_rejection.document.value["reason"]["code"],
        "run.lifecycle.terminal"
    );

    let initial = add_scenario_provider(&sandbox, "life-initial", "graph-initial-final", &[]);
    let initial_run = create_run(&sandbox, &initial, "life-initial");
    assert_eq!(
        show(&sandbox, &initial_run, "life-initial-show")["data"]["run"]["lifecycle"],
        "final"
    );

    let terminated_run = create_run(&sandbox, &linear, "life-terminated");
    let terminated = invoke_json(
        &sandbox,
        "life-terminate",
        &[
            "run".into(),
            "terminate".into(),
            terminated_run.clone(),
            "--note".into(),
            "operator stop".into(),
        ],
        0,
    );
    assert_eq!(terminated.document.value["outcome"], "completed");
    assert_eq!(
        show(&sandbox, &terminated_run, "life-terminated-show")["data"]["run"]["lifecycle"],
        "terminated"
    );
    let repeated = invoke_json(
        &sandbox,
        "life-terminate-repeat",
        &["run".into(), "terminate".into(), terminated_run.clone()],
        2,
    );
    assert_eq!(
        repeated.document.value["reason"]["code"],
        "run.lifecycle.terminal"
    );

    let history = invoke_json(
        &sandbox,
        "life-history",
        &["run".into(), "history".into(), terminated_run],
        0,
    );
    let entries = history.document.value["data"]["items"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["entry_kind"], "run.created");
    assert_eq!(entries[1]["entry_kind"], "run.terminated");
    assert_eq!(entries[2]["outcome"], "rejected");
}
