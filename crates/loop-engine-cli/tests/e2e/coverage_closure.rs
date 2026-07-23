use std::collections::BTreeSet;

use crate::support::{
    E2eSandbox, ProviderAddArgs, RuntimeCoverageRecorder, invoke_json, parse_pre_dispatch_stderr,
    parse_structured_stdout, scenario_provider_executable,
};

const ALPHA: [&str; 9] = [
    "provider.add",
    "provider.list",
    "provider.check",
    "run.create",
    "run.list",
    "run.terminate",
    "run.show",
    "run.request",
    "run.history",
];

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
            registration_id,
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

    let history = invoke_json(
        &sandbox,
        "closure-run-history",
        &["run".into(), "history".into(), run_id.clone()],
        0,
    );
    observe(&mut recorder, &history);

    let terminate = invoke_json(
        &sandbox,
        "closure-run-terminate",
        &["run".into(), "terminate".into(), run_id],
        0,
    );
    observe(&mut recorder, &terminate);

    let expected = ALPHA
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        recorder
            .e2e_operations()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        expected,
    );
    assert_eq!(
        recorder
            .trace_operations()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        expected,
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
    assert_eq!(observed, ALPHA.map(str::to_owned).into_iter().collect());

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
