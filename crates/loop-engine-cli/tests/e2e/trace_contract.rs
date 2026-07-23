use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::support::{E2eSandbox, invoke_json, parse_pre_dispatch_stderr, parse_structured_stdout};

#[test]
fn production_traces_are_private_correlated_and_bounded() {
    let sandbox = E2eSandbox::new();
    let invocation = invoke_json(
        &sandbox,
        "trace-contract-list",
        &["provider".into(), "list".into()],
        0,
    );
    let directory_mode = fs::metadata(sandbox.traces_dir())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let file_mode = fs::metadata(&invocation.trace.path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(directory_mode, 0o700);
    assert_eq!(file_mode, 0o600);
    assert!(invocation.trace.path.starts_with(sandbox.traces_dir()));
    assert_eq!(
        invocation.trace.path.file_stem().unwrap().to_str().unwrap(),
        invocation.document.value["request_id"].as_str().unwrap()
    );

    let starts = invocation
        .trace
        .events
        .iter()
        .filter(|event| event["category"] == "invocation" && event["event"] == "start")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    assert!(starts[0].get("argv").is_none());
    assert!(
        starts[0]["argv_digest"]
            .as_str()
            .is_some_and(|value| value.len() == 64)
    );
    assert_eq!(
        invocation
            .trace
            .events
            .iter()
            .filter(|event| event["category"] == "invocation" && event["event"] == "finish")
            .count(),
        1
    );
}

#[test]
fn closed_trace_rotation_evicts_oldest_and_retains_exact_bound() {
    let sandbox = E2eSandbox::new();
    for index in 0..100 {
        fs::write(sandbox.traces_dir().join(format!("{index:03}.jsonl")), b"").unwrap();
    }
    invoke_json(
        &sandbox,
        "trace-rotation",
        &["provider".into(), "list".into()],
        0,
    );
    let traces = fs::read_dir(sandbox.traces_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    assert_eq!(traces.len(), 100);
    assert!(!sandbox.traces_dir().join("000.jsonl").exists());
    assert!(sandbox.traces_dir().join("099.jsonl").exists());
}

#[test]
fn trace_initialization_failure_precedes_parse_and_dispatch() {
    let sandbox = E2eSandbox::new();
    fs::remove_dir(sandbox.traces_dir()).unwrap();
    fs::write(sandbox.traces_dir(), b"not a directory").unwrap();
    let failed = sandbox
        .runner()
        .run_json("trace-init-failure", &["unknown"]);
    assert_ne!(failed.exit_code, Some(0));
    assert!(failed.stdout.is_empty());
    let failure = parse_pre_dispatch_stderr(&failed.stderr).unwrap();
    assert_eq!(failure.value["phase"], "trace_init");
    assert!(failure.value["request_id"].as_str().is_some());
    assert!(failure.value["trace"].is_null());

    assert!(parse_structured_stdout(&failed.stderr).is_ok());
}
