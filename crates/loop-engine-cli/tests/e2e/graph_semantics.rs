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

fn request(
    sandbox: &E2eSandbox,
    run_id: &str,
    event: &str,
    label: &str,
    exit: i32,
) -> serde_json::Value {
    invoke_json(
        sandbox,
        label,
        &["run".into(), "request".into(), run_id.into(), event.into()],
        exit,
    )
    .document
    .value
}

#[test]
fn valid_graph_family_preserves_cycle_self_loop_final_and_sink_semantics() {
    let sandbox = E2eSandbox::new();

    let cycle_provider = add_scenario_provider(&sandbox, "cycle-a4", "graph-cycle", &[]);
    let cycle = create_run(&sandbox, &cycle_provider, "cycle-a4");
    assert_eq!(
        request(&sandbox, &cycle, "forward", "cycle-forward", 0)["outcome"],
        "completed"
    );
    assert_eq!(
        request(&sandbox, &cycle, "back", "cycle-back", 0)["outcome"],
        "completed"
    );
    let cycle_show = show(&sandbox, &cycle, "cycle-show");
    assert_eq!(cycle_show["data"]["run"]["state"], "a");
    assert_eq!(cycle_show["data"]["run"]["lifecycle"], "active");

    let self_provider = add_scenario_provider(&sandbox, "self-a4", "graph-self-loop", &[]);
    let self_loop = create_run(&sandbox, &self_provider, "self-a4");
    assert_eq!(
        request(&sandbox, &self_loop, "checkpoint", "self-request", 0)["outcome"],
        "completed"
    );
    let after = show(&sandbox, &self_loop, "self-after");
    assert_eq!(after["data"]["run"]["state"], "draft");
    assert_eq!(after["data"]["run"]["lifecycle"], "active");
    let self_history = invoke_json(
        &sandbox,
        "self-history",
        &["run".into(), "history".into(), self_loop.clone()],
        0,
    );
    let self_entries = self_history.document.value["data"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(self_entries.len(), 2);
    assert_eq!(self_entries[1]["entry_kind"], "transition.attempt");
    assert_eq!(self_entries[1]["outcome"], "completed");

    let multi_provider = add_scenario_provider(&sandbox, "multi-a4", "graph-multi-final", &[]);
    for event in ["finish-a", "finish-b"] {
        let run = create_run(&sandbox, &multi_provider, &format!("multi-{event}"));
        assert_eq!(
            request(&sandbox, &run, event, &format!("multi-{event}-request"), 0)["outcome"],
            "completed"
        );
        let shown = show(&sandbox, &run, &format!("multi-{event}-show"));
        assert_eq!(shown["data"]["run"]["lifecycle"], "final");
        assert!(
            shown["data"]["requestable_events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    let zero_provider = add_scenario_provider(&sandbox, "zero-a4", "graph-zero-final", &[]);
    let zero = create_run(&sandbox, &zero_provider, "zero-a4");
    assert_eq!(
        request(&sandbox, &zero, "advance", "zero-advance", 0)["outcome"],
        "completed"
    );
    let zero_show = show(&sandbox, &zero, "zero-show");
    assert_eq!(zero_show["data"]["run"]["state"], "review");
    assert_eq!(zero_show["data"]["run"]["lifecycle"], "active");
    assert!(
        zero_show["data"]["requestable_events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        request(&sandbox, &zero, "advance", "zero-no-event", 2)["outcome"],
        "rejected"
    );

    let sink_provider = add_scenario_provider(&sandbox, "sink-a4", "graph-non-final-sink", &[]);
    let sink = create_run(&sandbox, &sink_provider, "sink-a4");
    assert_eq!(
        request(&sandbox, &sink, "fall", "sink-fall", 0)["outcome"],
        "completed"
    );
    let sink_show = show(&sandbox, &sink, "sink-show");
    assert_eq!(sink_show["data"]["run"]["state"], "sink");
    assert_eq!(sink_show["data"]["run"]["lifecycle"], "active");
    assert!(
        sink_show["data"]["requestable_events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn invalid_and_ambiguous_graphs_fail_before_run_creation() {
    for (index, scenario) in [
        "graph-ambiguous-duplicate-state",
        "graph-ambiguous-duplicate-event",
        "graph-structurally-invalid",
        "graph-final-outgoing",
    ]
    .into_iter()
    .enumerate()
    {
        let sandbox = E2eSandbox::new();
        let provider = add_scenario_provider(&sandbox, &format!("invalid-{index}"), scenario, &[]);
        let failed = invoke_json(
            &sandbox,
            &format!("invalid-create-{index}"),
            &["run".into(), "create".into(), provider],
            1,
        );
        assert_eq!(failed.document.value["outcome"], "error");
        assert_eq!(
            failed.document.value["reason"]["code"],
            "provider.graph.invalid"
        );
        let listed = invoke_json(
            &sandbox,
            &format!("invalid-list-{index}"),
            &["run".into(), "list".into(), "--all".into()],
            0,
        );
        assert!(
            listed.document.value["data"]["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
