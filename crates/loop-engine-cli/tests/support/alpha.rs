//! Shared production-process setup for cross-operation alpha acceptance.

use std::path::Path;

use super::cli::{StructuredDocument, parse_structured_stdout};
use super::provider::{ProviderAddArgs, scenario_provider_executable};
use super::sandbox::E2eSandbox;
use super::trace::{ParsedTrace, parse_correlated_trace};

pub struct AlphaInvocation {
    pub document: StructuredDocument,
    pub trace: ParsedTrace,
}

pub fn invoke_json(
    sandbox: &E2eSandbox,
    label: &str,
    args: &[String],
    expected_exit: i32,
) -> AlphaInvocation {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invocation = sandbox.runner().run_json(label, &refs);
    assert_eq!(
        invocation.exit_code,
        Some(expected_exit),
        "unexpected exit for {label}; stderr={} stdout={}",
        String::from_utf8_lossy(&invocation.stderr),
        String::from_utf8_lossy(&invocation.stdout),
    );
    assert!(
        invocation.stderr.is_empty(),
        "dispatched stderr for {label}"
    );
    let document = parse_structured_stdout(&invocation.stdout)
        .unwrap_or_else(|error| panic!("parse structured stdout for {label}: {error}"));
    let trace = parse_correlated_trace(&document, &sandbox.traces_dir())
        .unwrap_or_else(|error| panic!("parse correlated trace for {label}: {error}"));
    AlphaInvocation { document, trace }
}

pub fn add_scenario_provider(
    sandbox: &E2eSandbox,
    handle: &str,
    scenario: &str,
    extra_args: &[(&str, &Path)],
) -> String {
    let mut argv = vec!["--scenario".to_owned(), scenario.to_owned()];
    for (flag, value) in extra_args {
        argv.push((*flag).to_owned());
        argv.push(value.display().to_string());
    }
    let args = ProviderAddArgs {
        handle: handle.to_owned(),
        exec: scenario_provider_executable().clone(),
        working_directory: sandbox.provider_cwd().to_path_buf(),
        args: argv,
        timeout_seconds: 2,
    }
    .to_cli_args();
    let invocation = invoke_json(sandbox, &format!("add-{handle}"), &args, 0);
    assert_eq!(invocation.document.value["outcome"], "completed");
    invocation.document.value["data"]["registration"]["id"]
        .as_str()
        .expect("registration id")
        .to_owned()
}

pub fn create_run(sandbox: &E2eSandbox, target: &str, label: &str) -> String {
    let invocation = invoke_json(
        sandbox,
        label,
        &[
            "run".into(),
            "create".into(),
            target.into(),
            "--label".into(),
            label.into(),
        ],
        0,
    );
    assert_eq!(invocation.document.value["outcome"], "completed");
    invocation.document.value["data"]["run"]["id"]
        .as_str()
        .expect("run id")
        .to_owned()
}
