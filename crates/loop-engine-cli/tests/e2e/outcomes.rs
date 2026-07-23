use crate::support::{
    E2eSandbox, add_scenario_provider, parse_pre_dispatch_stderr, parse_structured_stdout,
};

fn assert_human(invocation: &crate::support::CliInvocation, operation: &str, outcome: &str) {
    let stdout = std::str::from_utf8(&invocation.stdout).expect("human stdout UTF-8");
    assert!(stdout.contains(&format!("Operation: {operation}")));
    assert!(stdout.contains(&format!("Outcome: {outcome}")));
    assert!(invocation.stderr.is_empty());
}

#[test]
fn human_and_structured_renderers_preserve_outcome_and_exit_taxonomy() {
    let sandbox = E2eSandbox::new();

    let completed_json = sandbox
        .runner()
        .run_json("outcome-completed-json", &["provider", "list"]);
    assert_eq!(completed_json.exit_code, Some(0));
    let completed = parse_structured_stdout(&completed_json.stdout).unwrap();
    assert_eq!(completed.value["operation"], "provider.list");
    assert_eq!(completed.value["outcome"], "completed");
    assert!(completed_json.stderr.is_empty());
    assert!(completed_json.stdout.ends_with(b"\n"));
    assert!(!completed_json.stdout[..completed_json.stdout.len() - 1].contains(&b'\n'));

    let completed_human = sandbox
        .runner()
        .run_human("outcome-completed-human", &["provider", "list"]);
    assert_eq!(completed_human.exit_code, Some(0));
    assert_human(&completed_human, "provider.list", "completed");

    let absent = "019f8f00-0000-7000-8000-000000000001";
    let rejected_json = sandbox
        .runner()
        .run_json("outcome-rejected-json", &["run", "show", absent]);
    assert_eq!(rejected_json.exit_code, Some(2));
    let rejected = parse_structured_stdout(&rejected_json.stdout).unwrap();
    assert_eq!(rejected.value["outcome"], "rejected");
    let reason = rejected.value["reason"]["code"]
        .as_str()
        .unwrap()
        .to_owned();

    let rejected_human = sandbox
        .runner()
        .run_human("outcome-rejected-human", &["run", "show", absent]);
    assert_eq!(rejected_human.exit_code, Some(2));
    assert_human(&rejected_human, "run.show", "rejected");
    assert!(String::from_utf8_lossy(&rejected_human.stdout).contains(&reason));

    let malformed =
        add_scenario_provider(&sandbox, "outcome-malformed", "process-malformed-json", &[]);
    let error_json = sandbox
        .runner()
        .run_json("outcome-error-json", &["provider", "check", &malformed]);
    assert_eq!(error_json.exit_code, Some(1));
    let error = parse_structured_stdout(&error_json.stdout).unwrap();
    assert_eq!(error.value["outcome"], "error");
    assert_eq!(error.value["reason"]["code"], "provider.protocol.malformed");

    let error_human = sandbox
        .runner()
        .run_human("outcome-error-human", &["provider", "check", &malformed]);
    assert_eq!(error_human.exit_code, Some(1));
    assert_human(&error_human, "provider.check", "error");
    assert!(String::from_utf8_lossy(&error_human.stdout).contains("provider.protocol.malformed"));

    let parse_json = sandbox
        .runner()
        .run_json("outcome-parse-json", &["unknown"]);
    assert_eq!(parse_json.exit_code, Some(64));
    assert!(parse_json.stdout.is_empty());
    let failure = parse_pre_dispatch_stderr(&parse_json.stderr).unwrap();
    assert!(matches!(
        failure.value["phase"].as_str(),
        Some("parse" | "usage")
    ));

    let parse_human = sandbox
        .runner()
        .run_human("outcome-parse-human", &["unknown"]);
    assert_eq!(parse_human.exit_code, Some(64));
    assert!(parse_human.stdout.is_empty());
    assert!(!parse_human.stderr.is_empty());
}
