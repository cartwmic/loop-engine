use std::collections::BTreeSet;

use crate::support::{
    E2eSandbox, ProviderAddArgs, RuntimeCoverageRecorder, invoke_json, parse_pre_dispatch_stderr,
    parse_structured_stdout, scenario_provider_executable,
};

fn observe(recorder: &mut RuntimeCoverageRecorder, invocation: &crate::support::AlphaInvocation) {
    recorder.observe_invocation(Some(&invocation.document), None, Some(&invocation.trace));
    let events = &invocation.trace.events;
    let request = events
        .iter()
        .filter(|event| event["category"] == "invocation" && event["event"] == "request")
        .collect::<Vec<_>>();
    let outcome = events
        .iter()
        .filter(|event| event["category"] == "invocation" && event["event"] == "outcome")
        .collect::<Vec<_>>();
    assert_eq!(request.len(), 1, "one invocation.request");
    assert_eq!(outcome.len(), 1, "one invocation.outcome");
    assert_eq!(
        outcome[0]["envelope"], invocation.document.value,
        "trace outcome must contain exact stdout envelope",
    );
    let operation = invocation.document.value["operation"]
        .as_str()
        .expect("outcome operation");
    assert_eq!(request[0]["operation"], operation);
    assert!(
        events
            .iter()
            .any(|event| { event["category"] == "persistence" && event["operation"] == operation }),
        "{operation} must emit its own persistence boundary trace",
    );
    assert!(
        events
            .iter()
            .any(|event| { event["category"] == "invocation" && event["event"] == "start" })
    );
    assert!(
        events
            .iter()
            .any(|event| { event["category"] == "invocation" && event["event"] == "finish" })
    );

    let encoded = serde_json::to_string(events).expect("trace serializes");
    assert!(!encoded.contains("LOOP_ENGINE_HOME"));
    assert!(!encoded.contains("HTTP_PROXY"));
    let start = events
        .iter()
        .find(|event| event["category"] == "invocation" && event["event"] == "start")
        .expect("invocation start");
    assert!(start.get("argv").is_none());
    assert!(start["argv_digest"].as_str().is_some());
}

#[test]
fn alpha_catalog_has_independent_runtime_and_trace_closure() {
    let sandbox = E2eSandbox::new();
    let mut recorder = RuntimeCoverageRecorder::new();

    let add_args = ProviderAddArgs {
        handle: "alpha".into(),
        exec: scenario_provider_executable().clone(),
        working_directory: sandbox.provider_cwd().to_path_buf(),
        args: vec!["--scenario".into(), "graph-linear".into()],
        timeout_seconds: 2,
    }
    .to_cli_args();
    let add = invoke_json(&sandbox, "closure-add", &add_args, 0);
    observe(&mut recorder, &add);
    let registration_id = add.document.value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned();

    let list = invoke_json(
        &sandbox,
        "closure-provider-list",
        &["provider".into(), "list".into()],
        0,
    );
    observe(&mut recorder, &list);

    let check = invoke_json(
        &sandbox,
        "closure-provider-check",
        &["provider".into(), "check".into(), registration_id.clone()],
        0,
    );
    observe(&mut recorder, &check);

    let create = invoke_json(
        &sandbox,
        "closure-run-create",
        &[
            "run".into(),
            "create".into(),
            registration_id.clone(),
            "--label".into(),
            "closure".into(),
        ],
        0,
    );
    observe(&mut recorder, &create);
    let run_id = create.document.value["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned();

    let list = invoke_json(
        &sandbox,
        "closure-run-list",
        &["run".into(), "list".into()],
        0,
    );
    observe(&mut recorder, &list);

    let show = invoke_json(
        &sandbox,
        "closure-run-show",
        &["run".into(), "show".into(), run_id.clone()],
        0,
    );
    observe(&mut recorder, &show);

    let graph = invoke_json(
        &sandbox,
        "closure-run-graph",
        &["run".into(), "graph".into(), run_id.clone()],
        0,
    );
    observe(&mut recorder, &graph);

    let request = invoke_json(
        &sandbox,
        "closure-run-request",
        &[
            "run".into(),
            "request".into(),
            run_id.clone(),
            "advance".into(),
        ],
        0,
    );
    observe(&mut recorder, &request);

    let evidence_add = invoke_json(
        &sandbox,
        "closure-run-evidence-add",
        &[
            "run".into(),
            "evidence".into(),
            "add".into(),
            run_id.clone(),
            "--kind".into(),
            "artifact".into(),
            "--ref".into(),
            "opaque:closure".into(),
        ],
        0,
    );
    observe(&mut recorder, &evidence_add);

    let evidence_list = invoke_json(
        &sandbox,
        "closure-run-evidence-list",
        &[
            "run".into(),
            "evidence".into(),
            "list".into(),
            run_id.clone(),
        ],
        0,
    );
    observe(&mut recorder, &evidence_list);

    let annotate = invoke_json(
        &sandbox,
        "closure-run-annotate",
        &[
            "run".into(),
            "annotate".into(),
            run_id.clone(),
            "--note".into(),
            "closure annotation".into(),
        ],
        0,
    );
    observe(&mut recorder, &annotate);

    let label = invoke_json(
        &sandbox,
        "closure-run-label",
        &[
            "run".into(),
            "label".into(),
            run_id.clone(),
            "--set".into(),
            "closure-renamed".into(),
        ],
        0,
    );
    observe(&mut recorder, &label);

    let guidance = invoke_json(
        &sandbox,
        "closure-run-guidance",
        &["run".into(), "guidance".into(), run_id.clone()],
        2,
    );
    observe(&mut recorder, &guidance);

    let compatibility = invoke_json(
        &sandbox,
        "closure-run-compatibility",
        &["run".into(), "compatibility".into(), run_id.clone()],
        0,
    );
    observe(&mut recorder, &compatibility);

    let history = invoke_json(
        &sandbox,
        "closure-run-history",
        &["run".into(), "history".into(), run_id.clone()],
        0,
    );
    observe(&mut recorder, &history);

    let export_dir = sandbox.caller_cwd().join("closure-export");
    let export = invoke_json(
        &sandbox,
        "closure-run-export",
        &[
            "run".into(),
            "export".into(),
            run_id.clone(),
            "--output".into(),
            export_dir.display().to_string(),
        ],
        0,
    );
    observe(&mut recorder, &export);

    let terminate = invoke_json(
        &sandbox,
        "closure-run-terminate",
        &["run".into(), "terminate".into(), run_id],
        0,
    );
    observe(&mut recorder, &terminate);

    let update = invoke_json(
        &sandbox,
        "closure-provider-update",
        &[
            "provider".into(),
            "update".into(),
            "alpha".into(),
            "--exec".into(),
            scenario_provider_executable().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-linear".into(),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--timeout".into(),
            "2".into(),
        ],
        0,
    );
    observe(&mut recorder, &update);

    let rename = invoke_json(
        &sandbox,
        "closure-provider-rename",
        &[
            "provider".into(),
            "rename".into(),
            "alpha".into(),
            "alpha-renamed".into(),
        ],
        0,
    );
    observe(&mut recorder, &rename);

    let disable_warning = invoke_json(
        &sandbox,
        "closure-provider-disable-warning",
        &["provider".into(), "disable".into(), "alpha-renamed".into()],
        0,
    );
    observe(&mut recorder, &disable_warning);
    let ack = disable_warning.document.value["data"]["ack_token"]
        .as_str()
        .expect("disable acknowledgement")
        .to_owned();
    let disable = invoke_json(
        &sandbox,
        "closure-provider-disable",
        &[
            "provider".into(),
            "disable".into(),
            "alpha-renamed".into(),
            "--allow-active-runs".into(),
            ack,
        ],
        0,
    );
    observe(&mut recorder, &disable);

    let restore = invoke_json(
        &sandbox,
        "closure-provider-restore",
        &[
            "provider".into(),
            "restore".into(),
            registration_id,
            "--handle".into(),
            "alpha-restored".into(),
            "--exec".into(),
            scenario_provider_executable().display().to_string(),
            "--working-directory".into(),
            sandbox.provider_cwd().display().to_string(),
            "--arg=--scenario".into(),
            "--arg=graph-linear".into(),
            "--timeout".into(),
            "2".into(),
        ],
        0,
    );
    observe(&mut recorder, &restore);

    let catalog = loop_engine_core::operations::catalog::PLANNED_OPERATION_IDS
        .iter()
        .map(|operation| (*operation).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        recorder
            .e2e_operations()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        catalog,
        "operations observed from actual runtime outcomes must exactly cover the core catalog",
    );
    assert_eq!(
        recorder
            .trace_operations()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        catalog,
        "operations observed from actual correlated traces must exactly cover the core catalog",
    );
}

#[test]
fn production_surface_excludes_non_goals_and_unexposed_operations() {
    let sandbox = E2eSandbox::new();
    let listed = sandbox
        .runner()
        .run_json("closure-list-operations", &["--list-operations"]);
    assert_eq!(listed.exit_code, Some(0));
    assert!(listed.stderr.is_empty());
    let document = parse_structured_stdout(&listed.stdout).expect("operation list");
    let observed = document.value["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .map(|row| row["id"].as_str().expect("operation id").to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed,
        loop_engine_core::operations::catalog::OperationId::planned()
            .map(|operation| operation.as_str().to_owned())
            .collect()
    );

    for (index, argv) in [
        vec!["run", "delete", "not-a-run"],
        vec!["run", "reopen", "not-a-run"],
        vec!["run", "import", "snapshot.json"],
        vec!["provider", "discover"],
        vec!["daemon", "start"],
        vec!["agent", "run"],
    ]
    .into_iter()
    .enumerate()
    {
        let invocation = sandbox
            .runner()
            .run_json(&format!("excluded-{index}"), &argv);
        assert_eq!(invocation.exit_code, Some(64));
        assert!(invocation.stdout.is_empty());
        let failure = parse_pre_dispatch_stderr(&invocation.stderr).expect("structured failure");
        assert!(matches!(
            failure.value["phase"].as_str(),
            Some("parse" | "usage")
        ));
        let trace = std::fs::read_to_string(failure.value["trace"].as_str().expect("trace path"))
            .expect("read trace");
        assert!(!trace.contains("\"event\":\"request\""));
        assert!(!trace.contains("\"event\":\"outcome\""));
    }

    let help = sandbox.runner().run_human("closure-help", &["--help"]);
    let help = String::from_utf8(help.stdout).expect("help utf8");
    for forbidden in [
        "daemon",
        "agent",
        "replay",
        "sandbox",
        "discovery",
        "import",
        "reopen",
        "delete",
        "sdk",
    ] {
        assert!(
            !help.to_ascii_lowercase().contains(forbidden),
            "help exposed {forbidden}"
        );
    }
}
