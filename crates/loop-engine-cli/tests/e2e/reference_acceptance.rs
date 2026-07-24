use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::support::{
    CliRunner, E2eSandbox, ProviderAddArgs, parse_correlated_value, parse_structured_stdout,
    reference_provider_executable, workspace_root,
};

fn invoke(sandbox: &E2eSandbox, label: &str, args: Vec<String>, expected_exit: i32) -> Value {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_json(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(expected_exit),
        "{label}: stderr={} stdout={}",
        String::from_utf8_lossy(&invocation.stderr),
        String::from_utf8_lossy(&invocation.stdout),
    );
    assert!(invocation.stderr.is_empty(), "{label}: dispatched stderr");
    let document = parse_structured_stdout(&invocation.stdout).expect("one structured document");
    let trace = parse_correlated_value(&document.value, &sandbox.traces_dir())
        .expect("correlated production trace");
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "request"
            && event["operation"] == document.value["operation"]
    }));
    assert!(trace.events.iter().any(|event| {
        event["category"] == "invocation"
            && event["event"] == "outcome"
            && event["envelope"] == document.value
    }));
    document.value
}

fn add_reference_provider(sandbox: &E2eSandbox, handle: &str, argv: &[&str]) -> String {
    let args = ProviderAddArgs {
        handle: handle.to_owned(),
        exec: reference_provider_executable().clone(),
        working_directory: sandbox.provider_cwd().to_path_buf(),
        args: argv.iter().map(|value| (*value).to_owned()).collect(),
        timeout_seconds: 60,
    }
    .to_cli_args();
    let result = invoke(sandbox, &format!("reference-add-{handle}"), args, 0);
    result["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

fn update_reference_provider(sandbox: &E2eSandbox, handle: &str, argv: &[&str]) -> Value {
    let mut args = vec![
        "provider".into(),
        "update".into(),
        handle.into(),
        "--exec".into(),
        reference_provider_executable().display().to_string(),
    ];
    for value in argv {
        args.push(format!("--arg={value}"));
    }
    args.extend([
        "--working-directory".into(),
        sandbox.provider_cwd().display().to_string(),
        "--timeout".into(),
        "60".into(),
    ]);
    invoke(sandbox, &format!("reference-update-{handle}"), args, 0)
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture destination");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn artifact_root(sandbox: &E2eSandbox, name: &str, happy: bool) -> PathBuf {
    let root = sandbox.caller_cwd().join(name);
    fs::create_dir_all(&root).expect("create artifact root");
    if happy {
        copy_tree(
            &workspace_root()
                .join("test-support/providers/reference-provider/fixtures/artifacts/happy-path"),
            &root,
        );
    }
    root
}

fn create_reference_run(
    sandbox: &E2eSandbox,
    registration: &str,
    label: &str,
    artifacts: &Path,
) -> String {
    let inputs = sandbox.caller_cwd().join(format!("{label}-inputs.json"));
    fs::write(
        &inputs,
        serde_json::to_vec(&json!({
            "artifact_root": artifacts.display().to_string(),
            "change_id": label,
        }))
        .expect("serialize inputs"),
    )
    .expect("write inputs");
    let result = invoke(
        sandbox,
        &format!("reference-create-{label}"),
        vec![
            "run".into(),
            "create".into(),
            registration.into(),
            "--label".into(),
            label.into(),
            "--inputs".into(),
            inputs.display().to_string(),
        ],
        0,
    );
    result["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned()
}

fn evidence_ids(sandbox: &E2eSandbox, run_id: &str) -> Vec<String> {
    let result = invoke(
        sandbox,
        "reference-evidence-list",
        vec![
            "run".into(),
            "evidence".into(),
            "list".into(),
            run_id.into(),
        ],
        0,
    );
    result["data"]["items"]
        .as_array()
        .expect("evidence items")
        .iter()
        .map(|item| item["id"].as_str().expect("evidence id").to_owned())
        .collect()
}

fn request(
    sandbox: &E2eSandbox,
    label: &str,
    run_id: &str,
    event: &str,
    select_existing: bool,
    extra: &[String],
    expected_exit: i32,
) -> Value {
    let mut args = vec!["run".into(), "request".into(), run_id.into(), event.into()];
    if select_existing {
        for id in evidence_ids(sandbox, run_id) {
            args.extend(["--evidence-id".into(), id]);
        }
    }
    args.extend_from_slice(extra);
    invoke(sandbox, label, args, expected_exit)
}

fn assert_state(result: &Value, outcome: &str, state: &str) {
    assert_eq!(result["outcome"], outcome);
    assert_eq!(result["data"]["run"]["state"], state);
}

fn write_review(root: &Path, name: &str, revision: &str, subject: &str, verdict: &str) {
    fs::write(
        root.join("reviews").join(name),
        serde_json::to_vec(&json!({
            "revision": revision,
            "subject_revision": subject,
            "verdict": verdict,
        }))
        .expect("serialize review"),
    )
    .expect("write review");
}

#[test]
fn reference_behaviors_1_through_4_creation_happy_path_and_rejections() {
    let sandbox = E2eSandbox::new();
    let artifacts = artifact_root(&sandbox, "reference-happy", true);
    let registration = add_reference_provider(&sandbox, "reference-happy", &[]);
    let run_id = create_reference_run(&sandbox, &registration, "reference-happy", &artifacts);

    let shown = invoke(
        &sandbox,
        "reference-safe-show",
        vec!["run".into(), "show".into(), run_id.clone()],
        0,
    );
    assert_state(&shown, "completed", "explore");
    assert_eq!(
        shown["data"]["inputs"]["artifact_root"],
        artifacts.display().to_string()
    );
    let graph = invoke(
        &sandbox,
        "reference-stored-graph",
        vec!["run".into(), "graph".into(), run_id.clone()],
        0,
    );
    assert_eq!(graph["data"]["graph"]["initial_state_id"], "explore");

    for (index, (event, expected)) in [
        ("intent-ready", "design"),
        ("design-ready", "design-review"),
        ("approved", "plan"),
        ("plan-ready", "plan-review"),
        ("approved", "implement"),
        ("implementation-ready", "implementation-review"),
        ("approved", "validation"),
        ("passed", "end"),
    ]
    .into_iter()
    .enumerate()
    {
        let result = request(
            &sandbox,
            &format!("reference-happy-{index}"),
            &run_id,
            event,
            true,
            &[],
            0,
        );
        assert_state(&result, "completed", expected);
    }
    let final_show = invoke(
        &sandbox,
        "reference-final-show",
        vec!["run".into(), "show".into(), run_id],
        0,
    );
    assert_eq!(final_show["data"]["run"]["lifecycle"], "final");

    for (name, body) in [("missing", None), ("invalid", Some(b"not-json".as_slice()))] {
        let sandbox = E2eSandbox::new();
        let artifacts = artifact_root(&sandbox, &format!("reference-{name}"), false);
        if let Some(body) = body {
            fs::write(artifacts.join("intent.json"), body).expect("write invalid intent");
        }
        let registration = add_reference_provider(&sandbox, &format!("reference-{name}"), &[]);
        let run_id = create_reference_run(
            &sandbox,
            &registration,
            &format!("reference-{name}"),
            &artifacts,
        );
        let rejected = request(
            &sandbox,
            &format!("reference-{name}-request"),
            &run_id,
            "intent-ready",
            false,
            &[],
            2,
        );
        assert_state(&rejected, "rejected", "explore");
        assert_eq!(rejected["reason"]["code"], "gate.failed");
        let history = invoke(
            &sandbox,
            &format!("reference-{name}-history"),
            vec!["run".into(), "history".into(), run_id],
            0,
        );
        assert!(
            history["data"]["items"]
                .as_array()
                .is_some_and(|items| items.len() >= 2)
        );
    }
}

#[test]
fn reference_behaviors_5_through_9_revision_cycles_and_verdict_consistency() {
    let sandbox = E2eSandbox::new();
    let root = artifact_root(&sandbox, "reference-revisions", true);
    let registration = add_reference_provider(&sandbox, "reference-revisions", &[]);
    let run_id = create_reference_run(&sandbox, &registration, "reference-revisions", &root);

    assert_state(
        &request(
            &sandbox,
            "revision-intent",
            &run_id,
            "intent-ready",
            true,
            &[],
            0,
        ),
        "completed",
        "design",
    );
    assert_state(
        &request(
            &sandbox,
            "revision-design",
            &run_id,
            "design-ready",
            true,
            &[],
            0,
        ),
        "completed",
        "design-review",
    );

    let mismatch = request(
        &sandbox,
        "revision-design-mismatch",
        &run_id,
        "changes-requested",
        true,
        &[],
        2,
    );
    assert_state(&mismatch, "rejected", "design-review");
    write_review(&root, "design-review.json", "2", "1", "changes_requested");
    assert_state(
        &request(
            &sandbox,
            "revision-design-back",
            &run_id,
            "changes-requested",
            true,
            &[],
            0,
        ),
        "completed",
        "design",
    );
    fs::write(
        root.join("design.json"),
        br#"{"revision":"2","intent_revision":"1"}"#,
    )
    .expect("revise design");
    write_review(&root, "design-review.json", "3", "2", "approved");
    for (label, event, state) in [
        ("revision-design-again", "design-ready", "design-review"),
        ("revision-design-approved", "approved", "plan"),
    ] {
        assert_state(
            &request(&sandbox, label, &run_id, event, true, &[], 0),
            "completed",
            state,
        );
    }
    fs::write(
        root.join("plan.json"),
        br#"{"revision":"1","subject_revision":"2"}"#,
    )
    .expect("link plan to revised design");
    assert_state(
        &request(
            &sandbox,
            "revision-plan",
            &run_id,
            "plan-ready",
            true,
            &[],
            0,
        ),
        "completed",
        "plan-review",
    );

    write_review(&root, "plan-review.json", "2", "1", "changes_requested");
    assert_state(
        &request(
            &sandbox,
            "revision-plan-back",
            &run_id,
            "changes-requested",
            true,
            &[],
            0,
        ),
        "completed",
        "plan",
    );
    fs::write(
        root.join("plan.json"),
        br#"{"revision":"2","subject_revision":"2"}"#,
    )
    .expect("revise plan");
    write_review(&root, "plan-review.json", "3", "2", "approved");
    for (label, event, state) in [
        ("revision-plan-again", "plan-ready", "plan-review"),
        ("revision-plan-approved", "approved", "implement"),
    ] {
        assert_state(
            &request(&sandbox, label, &run_id, event, true, &[], 0),
            "completed",
            state,
        );
    }
    fs::write(
        root.join("implementation.json"),
        br#"{"revision":"1","plan_revision":"2"}"#,
    )
    .expect("link implementation to revised plan");
    assert_state(
        &request(
            &sandbox,
            "revision-implementation",
            &run_id,
            "implementation-ready",
            true,
            &[],
            0,
        ),
        "completed",
        "implementation-review",
    );

    write_review(
        &root,
        "implementation-review.json",
        "2",
        "1",
        "changes_requested",
    );
    assert_state(
        &request(
            &sandbox,
            "revision-implementation-back",
            &run_id,
            "changes-requested",
            true,
            &[],
            0,
        ),
        "completed",
        "implement",
    );
    fs::write(
        root.join("implementation.json"),
        br#"{"revision":"2","plan_revision":"2"}"#,
    )
    .expect("revise implementation");
    write_review(&root, "implementation-review.json", "3", "2", "approved");
    for (label, event, state) in [
        (
            "revision-implementation-again",
            "implementation-ready",
            "implementation-review",
        ),
        ("revision-implementation-approved", "approved", "validation"),
    ] {
        assert_state(
            &request(&sandbox, label, &run_id, event, true, &[], 0),
            "completed",
            state,
        );
    }

    fs::write(
        root.join("validation.json"),
        br#"{"revision":"2","verdict":"failed"}"#,
    )
    .expect("fail validation");
    assert_state(
        &request(
            &sandbox,
            "revision-validation-back",
            &run_id,
            "failed",
            true,
            &[],
            0,
        ),
        "completed",
        "implement",
    );
    fs::write(
        root.join("validation.json"),
        br#"{"revision":"3","verdict":"passed"}"#,
    )
    .expect("pass validation later");

    let evidence = invoke(
        &sandbox,
        "revision-evidence-list-final",
        vec!["run".into(), "evidence".into(), "list".into(), run_id],
        0,
    );
    let items = evidence["data"]["items"]
        .as_array()
        .expect("evidence items");
    assert!(items.iter().any(|item| item["id"] == "design-document-1"));
    assert!(items.iter().any(|item| item["id"] == "design-document-2"));
}

#[test]
fn reference_behaviors_10_through_13_evidence_restart_drift_and_compatibility() {
    let sandbox = E2eSandbox::new();
    let root = artifact_root(&sandbox, "reference-drift", true);
    let registration = add_reference_provider(&sandbox, "reference-drift", &[]);
    let run_id = create_reference_run(&sandbox, &registration, "reference-drift", &root);
    let baseline_graph = invoke(
        &sandbox,
        "reference-drift-graph-before",
        vec!["run".into(), "graph".into(), run_id.clone()],
        0,
    );
    request(
        &sandbox,
        "reference-drift-intent",
        &run_id,
        "intent-ready",
        true,
        &[],
        0,
    );
    request(
        &sandbox,
        "reference-drift-design",
        &run_id,
        "design-ready",
        true,
        &[],
        0,
    );
    let first_ids = evidence_ids(&sandbox, &run_id);
    assert!(first_ids.iter().any(|id| id == "design-document-1"));

    write_review(&root, "design-review.json", "2", "1", "changes_requested");
    request(
        &sandbox,
        "reference-drift-design-back",
        &run_id,
        "changes-requested",
        true,
        &[],
        0,
    );
    fs::write(
        root.join("design.json"),
        br#"{"revision":"2","intent_revision":"1"}"#,
    )
    .expect("revise same-path design");
    request(
        &sandbox,
        "reference-drift-design-revised",
        &run_id,
        "design-ready",
        true,
        &[],
        0,
    );
    let revised_ids = evidence_ids(&sandbox, &run_id);
    assert!(revised_ids.iter().any(|id| id == "design-document-1"));
    assert!(revised_ids.iter().any(|id| id == "design-document-2"));

    let updated = update_reference_provider(
        &sandbox,
        "reference-drift",
        &[
            "--provider-version=reference-provider/2.0.0-test",
            "--describe-graph=v2",
        ],
    );
    assert_eq!(updated["data"]["registration"]["id"], registration);
    let after_graph = invoke(
        &sandbox,
        "reference-drift-graph-after",
        vec!["run".into(), "graph".into(), run_id.clone()],
        0,
    );
    assert_eq!(
        after_graph["data"]["graph"],
        baseline_graph["data"]["graph"]
    );

    update_reference_provider(
        &sandbox,
        "reference-drift",
        &[
            "--provider-version=reference-provider/3.0.0-test",
            "--compat=incompatible",
            "--gate-incompatible",
        ],
    );
    let compatibility = invoke(
        &sandbox,
        "reference-drift-compatibility",
        vec!["run".into(), "compatibility".into(), run_id.clone()],
        0,
    );
    assert_eq!(compatibility["outcome"], "completed");
    assert!(
        compatibility["data"]["findings"]
            .as_array()
            .is_some_and(|items| { items.iter().any(|item| item["status"] == "incompatible") })
    );
    let denied = request(
        &sandbox,
        "reference-drift-incompatible-request",
        &run_id,
        "approved",
        true,
        &[],
        2,
    );
    assert_eq!(denied["reason"]["code"], "compatibility.unsupported");
    assert_state(&denied, "rejected", "design-review");
    assert_state(
        &invoke(
            &sandbox,
            "reference-drift-inspectable",
            vec!["run".into(), "show".into(), run_id.clone()],
            0,
        ),
        "completed",
        "design-review",
    );
    invoke(
        &sandbox,
        "reference-drift-annotatable",
        vec![
            "run".into(),
            "annotate".into(),
            run_id.clone(),
            "--note".into(),
            "incompatibility observed".into(),
        ],
        0,
    );
    let terminated = invoke(
        &sandbox,
        "reference-drift-terminable",
        vec!["run".into(), "terminate".into(), run_id],
        0,
    );
    assert_eq!(terminated["data"]["run"]["lifecycle"], "terminated");
}

#[test]
fn reference_behaviors_14_through_17_guidance_neutrality_journal_and_interaction() {
    let sandbox = E2eSandbox::new();
    let root = artifact_root(&sandbox, "reference-interaction", true);
    let registration =
        add_reference_provider(&sandbox, "reference-interaction", &["--guidance=recommend"]);
    let first = create_reference_run(&sandbox, &registration, "reference-actor-one", &root);
    let second = create_reference_run(&sandbox, &registration, "reference-actor-two", &root);

    for (run_id, actor_name) in [(&first, "operator-one"), (&second, "operator-two")] {
        let actor = sandbox.caller_cwd().join(format!("{actor_name}.json"));
        fs::write(
            &actor,
            serde_json::to_vec(&json!({"name": actor_name})).unwrap(),
        )
        .expect("write actor");
        invoke(
            &sandbox,
            &format!("reference-annotate-{actor_name}"),
            vec![
                "run".into(),
                "annotate".into(),
                run_id.clone(),
                "--note".into(),
                "actor-neutral context".into(),
                "--actor".into(),
                actor.display().to_string(),
            ],
            0,
        );
        let advanced = request(
            &sandbox,
            &format!("reference-actor-request-{actor_name}"),
            run_id,
            "intent-ready",
            true,
            &[],
            0,
        );
        assert_state(&advanced, "completed", "design");
    }

    let show = invoke(
        &sandbox,
        "reference-cold-handoff",
        vec!["run".into(), "show".into(), first.clone()],
        0,
    );
    assert_eq!(show["data"]["static_guidance"]["kind"], "text");
    assert_eq!(show["data"]["live_guidance"], "supported");
    let guidance = invoke(
        &sandbox,
        "reference-live-guidance",
        vec!["run".into(), "guidance".into(), first.clone()],
        0,
    );
    assert_eq!(guidance["outcome"], "completed");
    assert!(guidance["data"]["guidance"].as_str().is_some());
    let after_guidance = invoke(
        &sandbox,
        "reference-guidance-no-authority",
        vec!["run".into(), "show".into(), first.clone()],
        0,
    );
    assert_eq!(after_guidance["data"]["run"]["state"], "design");

    let other_cwd = sandbox.caller_cwd().join("handoff-cwd");
    fs::create_dir_all(&other_cwd).expect("create handoff cwd");
    let handoff = CliRunner::from_cwd(&sandbox, &other_cwd)
        .run_json("reference-other-cwd", &["run", "show", &first]);
    assert_eq!(handoff.exit_code, Some(0));
    let handoff_doc = parse_structured_stdout(&handoff.stdout).expect("handoff envelope");
    assert_eq!(handoff_doc.value["data"]["run"]["id"], first);

    let labeled = invoke(
        &sandbox,
        "reference-active-label",
        vec![
            "run".into(),
            "label".into(),
            first.clone(),
            "--set".into(),
            "reference-handoff".into(),
        ],
        0,
    );
    assert_eq!(labeled["data"]["run"]["label"], "reference-handoff");
    let terminated = invoke(
        &sandbox,
        "reference-interaction-terminate",
        vec!["run".into(), "terminate".into(), first.clone()],
        0,
    );
    assert_eq!(terminated["data"]["run"]["lifecycle"], "terminated");
    let annotated = invoke(
        &sandbox,
        "reference-terminal-annotation",
        vec![
            "run".into(),
            "annotate".into(),
            first.clone(),
            "--note".into(),
            "terminal handoff note".into(),
        ],
        0,
    );
    assert_eq!(annotated["data"]["run"]["lifecycle"], "terminated");
    let denied_label = invoke(
        &sandbox,
        "reference-terminal-label-denied",
        vec![
            "run".into(),
            "label".into(),
            first.clone(),
            "--set".into(),
            "reopened".into(),
        ],
        2,
    );
    assert_eq!(denied_label["reason"]["code"], "run.lifecycle.terminal");

    let history = invoke(
        &sandbox,
        "reference-journal-state-consistency",
        vec!["run".into(), "history".into(), second],
        0,
    );
    assert!(
        history["data"]["items"]
            .as_array()
            .is_some_and(|items| items.len() >= 3)
    );
}

#[test]
fn reference_behaviors_18_through_21_attempt_resolution_automation_and_visibility() {
    let sandbox = E2eSandbox::new();
    let root = artifact_root(&sandbox, "reference-visibility", true);
    let registration = add_reference_provider(&sandbox, "reference-visibility", &[]);
    let run_id = create_reference_run(&sandbox, &registration, "reference-visibility", &root);
    let inline = sandbox.caller_cwd().join("reference-inline-evidence.json");
    fs::write(
        &inline,
        br#"[{"id":"caller-observation-1","kind":"observation","locator":"opaque:caller"}]"#,
    )
    .expect("write inline evidence");

    let completed = request(
        &sandbox,
        "reference-attempt-completed",
        &run_id,
        "intent-ready",
        true,
        &["--evidence".into(), inline.display().to_string()],
        0,
    );
    assert_eq!(completed["data"]["evidence_recorded"]["inline"], true);
    assert_eq!(completed["data"]["evidence_recorded"]["provider"], true);

    let unknown_inline = sandbox.caller_cwd().join("reference-unknown-evidence.json");
    fs::write(
        &unknown_inline,
        br#"[{"id":"caller-observation-2","kind":"observation","locator":"opaque:unknown"}]"#,
    )
    .expect("write unknown evidence");
    let unknown = request(
        &sandbox,
        "reference-attempt-unknown",
        &run_id,
        "unknown-event",
        true,
        &["--evidence".into(), unknown_inline.display().to_string()],
        2,
    );
    assert_eq!(unknown["reason"]["code"], "event.unknown");
    assert_eq!(unknown["data"]["evidence_recorded"]["inline"], true);

    update_reference_provider(
        &sandbox,
        "reference-visibility",
        &["--gate-evaluation-error"],
    );
    let provider_error = request(
        &sandbox,
        "reference-attempt-provider-error",
        &run_id,
        "design-ready",
        true,
        &[],
        1,
    );
    assert_eq!(provider_error["outcome"], "error");
    assert_eq!(
        provider_error["reason"]["code"],
        "provider.evaluation_error"
    );
    let provider_trace = parse_correlated_value(&provider_error, &sandbox.traces_dir())
        .expect("provider-error trace");
    assert!(provider_trace.events.iter().any(|event| {
        event["category"] == "provider"
            && event["event"] == "start"
            && event["role"] == "evaluate_gates"
    }));

    let terminated = invoke(
        &sandbox,
        "reference-visibility-terminate",
        vec!["run".into(), "terminate".into(), run_id.clone()],
        0,
    );
    assert_eq!(terminated["data"]["run"]["lifecycle"], "terminated");
    let terminal = request(
        &sandbox,
        "reference-attempt-terminal",
        &run_id,
        "design-ready",
        true,
        &[],
        2,
    );
    assert_eq!(terminal["reason"]["code"], "run.lifecycle.terminal");

    let evidence = evidence_ids(&sandbox, &run_id);
    assert!(evidence.iter().any(|id| id == "caller-observation-1"));
    assert!(evidence.iter().any(|id| id == "caller-observation-2"));
    let history = invoke(
        &sandbox,
        "reference-visibility-history",
        vec!["run".into(), "history".into(), run_id],
        0,
    );
    let items = history["data"]["items"].as_array().expect("history items");
    assert!(items.iter().any(|item| item["outcome"] == "completed"));
    assert!(items.iter().any(|item| item["outcome"] == "rejected"));
    assert!(items.iter().any(|item| item["outcome"] == "error"));
}
