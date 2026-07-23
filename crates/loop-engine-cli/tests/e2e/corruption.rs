use crate::support::{
    CorruptionKind, E2eSandbox, corrupt_database, parse_correlated_trace, parse_correlated_value,
    parse_pre_dispatch_stderr, parse_structured_stdout,
};

#[test]
fn corruption_families_fail_closed_through_public_cli_with_correlated_traces() {
    for (index, kind) in [
        CorruptionKind::MalformedDatabaseHeader,
        CorruptionKind::NotADatabase,
        CorruptionKind::SchemaFutureVersion,
        CorruptionKind::IntegrityKeyMissing,
        CorruptionKind::IntegrityKeyInvalidLength,
        CorruptionKind::SqlitePhysicalCorruption,
    ]
    .into_iter()
    .enumerate()
    {
        let sandbox = E2eSandbox::new();
        corrupt_database(&sandbox.state_db_path(), kind).expect("install corruption");
        let invocation = sandbox
            .runner()
            .run_json(&format!("corruption-{index}"), &["provider", "list"]);
        assert!(matches!(invocation.exit_code, Some(1 | 64)));

        if invocation.stdout.is_empty() {
            let failure = parse_pre_dispatch_stderr(&invocation.stderr)
                .unwrap_or_else(|error| panic!("corruption {kind:?} stderr: {error}"));
            assert_eq!(failure.value["phase"], "persistence");
            assert!(
                failure.value["message"]
                    .as_str()
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("corrupt")
                    || !failure.value["source_chain"].as_array().unwrap().is_empty()
            );
            parse_correlated_value(&failure.value, &sandbox.traces_dir())
                .expect("corruption trace");
        } else {
            assert!(invocation.stderr.is_empty());
            let document = parse_structured_stdout(&invocation.stdout).unwrap();
            assert_eq!(document.value["outcome"], "error");
            assert!(
                document.value["reason"]["code"]
                    .as_str()
                    .unwrap()
                    .starts_with("persistence.")
            );
            parse_correlated_trace(&document, &sandbox.traces_dir()).expect("corruption trace");
        }
    }
}
