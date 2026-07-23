//! Black-box E2E harness self-tests (T143–T145).
//!
//! Exercises substrate helpers only. The production driver exposes zero application
//! routes; these tests do not claim application-outcome coverage.

mod support;

#[path = "e2e/checkpoint_b.rs"]
mod checkpoint_b;
#[path = "e2e/checkpoint_c.rs"]
mod checkpoint_c;
#[path = "e2e/provider_add.rs"]
mod provider_add;
#[path = "e2e/provider_list.rs"]
mod provider_list;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use support::{
    CliInvocation, CorruptionKind, E2eSandbox, ProviderAddArgs, ProviderConfigFile,
    ProviderExecutableError, RuntimeCoverageRecorder, StructuredDocument, StructuredParseError,
    TombstonedRegistrationSetup, TraceParseError, apply_initial_migration, corrupt_database,
    harness_fixture_db_path, insert_tombstoned_registration, install_trace_fixture,
    parse_correlated_trace, parse_pre_dispatch_stderr, parse_structured_stdout,
    provider_manifest_path, read_trace_events, reference_provider_manifest_path, require_sqlite3,
    resolve_provider_executable_path, scenario_provider_manifest_path, trace_fixture_path,
    validate_tombstoned_registration,
};

#[cfg(unix)]
use support::{run_with_rlimit_fsize, verify_rlimit_blocks_writes};

const REQUEST_ID: &str = "01J9X3K2M4N5P6Q7R8S9T0V1W";
const STRUCTURED_CLI_ENVELOPE_BYTES: usize = 4_194_304;

fn structured_envelope(trace_path: &Path) -> StructuredDocument {
    StructuredDocument {
        value: json!({
            "schema_version": 1,
            "operation": "run.show",
            "request_id": REQUEST_ID,
            "trace": trace_path.display().to_string(),
            "outcome": "completed",
            "reason": null,
            "data": {},
            "diagnostics": []
        }),
        raw: Vec::new(),
    }
}

// --- T143: isolated sandbox ---

#[test]
fn sandbox_exposes_two_independent_private_roots() {
    let first = E2eSandbox::new();
    let second = E2eSandbox::new();

    assert_ne!(first.loop_engine_home(), second.loop_engine_home());
    assert_ne!(first.caller_cwd(), second.caller_cwd());
    assert_ne!(first.provider_cwd(), second.provider_cwd());
    assert!(first.traces_dir().starts_with(first.loop_engine_home()));
    assert!(
        second
            .state_db_path()
            .starts_with(second.loop_engine_home())
    );
    assert!(first.transcripts_dir().exists());
    assert!(second.transcripts_dir().exists());
    assert!(first.config_path().starts_with(first.loop_engine_home()));
}

#[test]
fn sandbox_runner_records_fresh_process_transcript() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_human("help", &["--help"]);
    assert!(invocation.transcript_path.exists());
    assert_eq!(
        invocation.env.get("LOOP_ENGINE_HOME").map(String::as_str),
        Some(sandbox.loop_engine_home().to_str().unwrap(),)
    );
    for key in E2eSandbox::isolated_env_removals() {
        if *key != "LOOP_ENGINE_HOME" {
            assert!(!invocation.env.contains_key(*key));
        }
    }
}

// --- T144: CLI runner + structured parser ---

#[test]
fn cli_runner_help_via_fresh_production_process() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_human("help", &["--help"]);
    assert_fresh_cli_invocation(&invocation, 0);
    let stdout = String::from_utf8(invocation.stdout).expect("help stdout utf8");
    assert!(stdout.contains("Global options"));
}

#[test]
fn cli_runner_version_via_fresh_production_process() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_human("version", &["--version"]);
    assert_fresh_cli_invocation(&invocation, 0);
    assert_eq!(
        String::from_utf8(invocation.stdout).expect("version stdout utf8"),
        "loop-engine 0.1.0\n",
    );
}

#[test]
fn cli_runner_invalid_argv_emits_pre_dispatch_json() {
    let sandbox = E2eSandbox::new();
    let invocation = sandbox.runner().run_json("invalid-argv", &["--not-a-flag"]);
    assert_fresh_cli_invocation(&invocation, 64);
    assert!(invocation.stdout.is_empty());

    let failure = parse_pre_dispatch_stderr(&invocation.stderr).expect("pre-dispatch json");
    assert_eq!(failure.value["phase"], "parse");
    assert!(failure.value.get("schema_version").is_some());
}

#[test]
fn structured_parser_accepts_valid_single_object_envelope() {
    let payload = br#"{"schema_version":1,"operation":"run.show","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","trace":"/tmp/trace.jsonl","outcome":"completed","reason":null,"data":{},"diagnostics":[]}"#;
    let mut bytes = payload.to_vec();
    bytes.push(b'\n');
    let document = parse_structured_stdout(&bytes).expect("valid envelope");
    assert_eq!(document.value["operation"], "run.show");
}

#[test]
fn structured_parser_rejects_missing_trailing_newline() {
    let bytes = br#"{"schema_version":1}"#;
    let error = parse_structured_stdout(bytes).unwrap_err();
    assert_eq!(error, StructuredParseError::NewlineBoundary);
}

#[test]
fn structured_parser_rejects_root_non_object() {
    let bytes = b"[1,2,3]\n";
    let error = parse_structured_stdout(bytes).unwrap_err();
    assert_eq!(error, StructuredParseError::RootNotObject);
}

#[test]
fn structured_parser_rejects_trailing_extra_content() {
    let bytes = b"{\"schema_version\":1}\n{\"extra\":true}\n";
    let error = parse_structured_stdout(bytes).unwrap_err();
    assert_eq!(error, StructuredParseError::TrailingContent);
}

#[test]
fn structured_parser_rejects_duplicate_object_keys() {
    let bytes = br#"{"schema_version":1,"schema_version":2}"#;
    let mut payload = bytes.to_vec();
    payload.push(b'\n');
    let error = parse_structured_stdout(&payload).unwrap_err();
    assert!(matches!(error, StructuredParseError::DuplicateKey { .. }));
}

#[test]
fn structured_parser_rejects_malformed_json() {
    let bytes = b"{not-json}\n";
    let error = parse_structured_stdout(bytes).unwrap_err();
    assert!(matches!(error, StructuredParseError::Malformed(_)));
}

#[test]
fn structured_parser_rejects_oversized_payload() {
    let mut bytes = vec![b'x'; STRUCTURED_CLI_ENVELOPE_BYTES];
    bytes.push(b'\n');
    let error = parse_structured_stdout(&bytes).unwrap_err();
    assert_eq!(
        error,
        StructuredParseError::Oversized {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: STRUCTURED_CLI_ENVELOPE_BYTES + 1,
        }
    );
}

// --- T145: trace parser ---

#[test]
fn trace_parser_correlates_request_id_from_fixture_file() {
    let sandbox = E2eSandbox::new();
    let trace_path =
        install_trace_fixture("01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl", &sandbox.traces_dir())
            .expect("install correlated fixture");
    let document = structured_envelope(&trace_path);
    let parsed = parse_correlated_trace(&document, &sandbox.traces_dir()).expect("correlate trace");
    assert_eq!(parsed.request_id, REQUEST_ID);
    assert_eq!(parsed.path, trace_path);
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(
        read_trace_events(&trace_path)
            .expect("fixture readable")
            .len(),
        2,
    );
    assert!(trace_fixture_path("01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl").exists());
}

#[test]
fn trace_parser_allows_unrelated_prior_traces_in_sandbox() {
    let sandbox = E2eSandbox::new();
    let primary = install_trace_fixture(
        "stale-sibling/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl",
        &sandbox.traces_dir(),
    )
    .expect("install primary fixture");
    install_trace_fixture(
        "stale-sibling/01J9X3K2M4N5P6Q7R8S9T0V2X.jsonl",
        &sandbox.traces_dir(),
    )
    .expect("install unrelated prior trace");

    let document = structured_envelope(&primary);
    let parsed = parse_correlated_trace(&document, &sandbox.traces_dir()).expect("correlate trace");
    assert_eq!(parsed.request_id, REQUEST_ID);
}

#[test]
fn trace_parser_rejects_referenced_trace_request_id_mismatch() {
    let sandbox = E2eSandbox::new();
    let trace_path = sandbox.traces_dir().join(format!("{REQUEST_ID}.jsonl"));
    let mismatched = fs::read_to_string(trace_fixture_path(
        "stale-sibling/01J9X3K2M4N5P6Q7R8S9T0V2X.jsonl",
    ))
    .expect("read mismatched fixture");
    fs::write(&trace_path, mismatched).expect("install mismatched referenced trace");

    let document = structured_envelope(&trace_path);
    let error = parse_correlated_trace(&document, &sandbox.traces_dir()).unwrap_err();
    assert!(matches!(error, TraceParseError::RequestIdMismatch { .. }));
}

#[test]
fn trace_parser_rejects_duplicate_object_keys_in_jsonl() {
    let sandbox = E2eSandbox::new();
    let trace_path = sandbox.traces_dir().join(format!("{REQUEST_ID}.jsonl"));
    fs::write(
        &trace_path,
        format!("{{\"request_id\":\"{REQUEST_ID}\",\"request_id\":\"duplicate\"}}\n"),
    )
    .expect("write duplicate-key trace");

    let document = structured_envelope(&trace_path);
    let error = parse_correlated_trace(&document, &sandbox.traces_dir()).unwrap_err();
    assert!(matches!(error, TraceParseError::DuplicateKey { .. }));
}

// --- T145: runtime coverage recorder ---

#[test]
fn runtime_coverage_recorder_stays_empty_without_app_routes() {
    let sandbox = E2eSandbox::new();
    let mut recorder = RuntimeCoverageRecorder::new();
    assert!(recorder.is_empty());

    let help = sandbox.runner().run_human("help", &["--help"]);
    recorder.observe_invocation(None, None, None);
    assert!(recorder.is_empty());

    let list = sandbox
        .runner()
        .run_json("list-ops", &["--list-operations"]);
    let driver_list = parse_structured_stdout(&list.stdout).expect("driver list json");
    assert_eq!(driver_list.value["kind"], "operation_list");
    recorder.observe_stdout(&driver_list);
    assert!(recorder.is_empty());
    assert!(recorder.e2e_operations().is_empty());
    assert!(recorder.trace_operations().is_empty());

    let invalid = sandbox.runner().run_json("invalid", &["--not-a-flag"]);
    let failure = parse_pre_dispatch_stderr(&invalid.stderr).expect("pre-dispatch");
    recorder.observe_stderr(&failure);
    assert!(recorder.is_empty());

    assert_eq!(help.exit_code, Some(0));
}

#[test]
fn runtime_coverage_recorder_observes_real_shaped_trace_operations() {
    let sandbox = E2eSandbox::new();
    let trace_path =
        install_trace_fixture("01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl", &sandbox.traces_dir())
            .expect("install request fixture");
    let mut contents = fs::read_to_string(&trace_path).expect("read request fixture");
    contents.push_str(
        &serde_json::to_string(&json!({
            "trace_schema_version": 1,
            "ts": "2026-07-17T10:00:00.050Z",
            "request_id": REQUEST_ID,
            "category": "invocation",
            "event": "outcome",
            "envelope": {
                "schema_version": 1,
                "operation": "run.export",
                "request_id": REQUEST_ID,
                "trace": trace_path.display().to_string(),
                "outcome": "completed",
                "reason": null,
                "data": {},
                "diagnostics": []
            }
        }))
        .expect("serialize outcome event"),
    );
    contents.push('\n');
    fs::write(&trace_path, contents).expect("append outcome fixture line");

    let document = structured_envelope(&trace_path);
    let parsed = parse_correlated_trace(&document, &sandbox.traces_dir()).expect("correlate trace");
    let mut recorder = RuntimeCoverageRecorder::new();
    recorder.observe_trace(&parsed);
    assert_eq!(
        recorder.trace_operations(),
        vec!["run.export".to_owned(), "run.show".to_owned()]
    );
    assert!(recorder.e2e_operations().is_empty());
}

// --- T145: provider config helper ---

#[test]
fn provider_config_helper_writes_json_and_cli_argv() {
    let sandbox = E2eSandbox::new();
    let config_path = sandbox.provider_cwd().join("scenario-provider.json");
    let executable = sandbox.provider_cwd().join("scenario-provider");
    let config = ProviderConfigFile::scenario(
        config_path.clone(),
        "scenario",
        &executable,
        sandbox.provider_cwd(),
        "linear-success",
        30,
    );
    config.write().expect("write provider config");
    assert!(config_path.exists());

    let args = config.provider_add_args(None);
    let cli = args.to_cli_args();
    assert_eq!(
        args,
        ProviderAddArgs {
            handle: "scenario".to_owned(),
            exec: executable.clone(),
            working_directory: sandbox.provider_cwd().to_path_buf(),
            args: vec!["--scenario".to_owned(), "linear-success".to_owned()],
            timeout_seconds: 30,
        }
    );
    assert_eq!(cli[0], "provider");
    assert_eq!(cli[1], "add");
    assert_eq!(cli[2], "scenario");
    assert!(cli.windows(2).any(|pair| pair == ["--arg", "--scenario"]));
    assert!(cli.contains(&"linear-success".to_owned()));

    let override_exec = sandbox.provider_cwd().join("override");
    let overridden = config.provider_add_args(Some(&override_exec));
    assert_eq!(overridden.exec, override_exec);

    assert!(provider_manifest_path("scenario-provider").ends_with("scenario-provider/Cargo.toml"));
    assert!(scenario_provider_manifest_path().ends_with("scenario-provider/Cargo.toml"));
    assert!(reference_provider_manifest_path().ends_with("reference-provider/Cargo.toml"));
}

#[test]
fn provider_executable_resolver_uses_standalone_crate_target() {
    let package = "scenario-provider";
    let binary = "scenario-provider";
    let manifest = scenario_provider_manifest_path();
    let invocation_cwd =
        std::env::current_dir().unwrap_or_else(|_| provider_manifest_path(package).join("../.."));
    let cargo_target_dir = std::env::var("CARGO_TARGET_DIR").ok().map(PathBuf::from);
    let expected_base = match cargo_target_dir.as_deref() {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => invocation_cwd.join(path),
        None => manifest.parent().expect("manifest parent").join("target"),
    };
    let expected_candidate = expected_base
        .join("debug")
        .join(format!("{binary}{}", std::env::consts::EXE_SUFFIX));

    match resolve_provider_executable_path(package, binary) {
        Ok(path) => {
            assert!(
                path.is_absolute(),
                "canonical provider path must be absolute"
            );
            assert_eq!(
                path,
                expected_candidate
                    .canonicalize()
                    .unwrap_or(expected_candidate)
            );
        }
        Err(ProviderExecutableError::BinaryNotBuilt {
            package: err_package,
            binary: err_binary,
            expected,
            manifest: err_manifest,
        }) => {
            assert_eq!(err_package, package);
            assert_eq!(err_binary, binary);
            assert_eq!(expected, expected_candidate);
            assert_eq!(err_manifest, manifest);
            assert!(
                !expected.exists(),
                "missing provider executable must not silently fall back to a source-tree path"
            );
            let message = ProviderExecutableError::BinaryNotBuilt {
                package: err_package,
                binary: err_binary,
                expected: expected.clone(),
                manifest: err_manifest,
            }
            .to_string();
            assert!(message.contains(&expected.display().to_string()));
            assert!(message.contains("cargo build --manifest-path"));
            assert!(message.contains("--locked"));
            assert!(message.contains(&manifest.display().to_string()));
        }
    }
}

// --- T145: sqlite fixture helpers ---

#[test]
fn sqlite_fixture_migration_tombstone_and_corruption_smoke() {
    require_sqlite3().expect("sqlite3 executable is a required harness prerequisite");

    let sandbox = E2eSandbox::new();
    let db_path = harness_fixture_db_path(sandbox.loop_engine_home(), "harness-smoke");
    assert_ne!(db_path, sandbox.state_db_path());
    assert!(
        db_path
            .components()
            .any(|component| component.as_os_str() == "harness-fixtures")
    );
    apply_initial_migration(&db_path).expect("apply initial migration");
    assert!(db_path.exists());

    let setup = TombstonedRegistrationSetup {
        registration_id: "01J9X3K2M4N5P6Q7R8S9T0V3Y".to_owned(),
        config_revision: 7,
    };
    insert_tombstoned_registration(&db_path, &setup).expect("insert tombstone");
    let observed = validate_tombstoned_registration(&db_path, &setup.registration_id)
        .expect("validate tombstone");
    assert_eq!(observed, setup);

    let _supported_corruptions = [
        CorruptionKind::MalformedDatabaseHeader,
        CorruptionKind::NotADatabase,
        CorruptionKind::SchemaFutureVersion,
        CorruptionKind::IntegrityKeyMissing,
        CorruptionKind::IntegrityKeyInvalidLength,
        CorruptionKind::SqlitePhysicalCorruption,
    ];
    let corruption_path = harness_fixture_db_path(sandbox.loop_engine_home(), "harness-corrupt");
    corrupt_database(&corruption_path, CorruptionKind::NotADatabase).expect("corrupt database");
    let header = fs::read(&corruption_path).expect("read corrupted db");
    assert_eq!(&header, b"definitely not a database");
}

// --- T145: Unix RLIMIT_FSIZE wrapper ---

#[cfg(unix)]
#[test]
fn rlimit_fsize_wrapper_self_test_without_production_branches() {
    let sandbox = E2eSandbox::new();
    verify_rlimit_blocks_writes(512).expect("ulimit wrapper blocks oversized writes");

    let output = run_with_rlimit_fsize(&sandbox, "rlimit-version", &["--version"], 1_048_576)
        .expect("spawn loop-engine under generous rlimit");
    assert_eq!(output.exit_code, Some(0));
    assert!(output.transcript_path.exists());
    assert!(
        output
            .transcript_path
            .starts_with(sandbox.transcripts_dir())
    );
    let document = parse_structured_stdout(&output.stdout).expect("version json envelope");
    assert_eq!(document.value["kind"], "version");
    assert_eq!(document.value["version"], "0.1.0");

    let tight = run_with_rlimit_fsize(&sandbox, "rlimit-tight", &["--version"], 512)
        .expect("spawn loop-engine under tight rlimit");
    assert!(
        tight.exit_code.is_some(),
        "tight rlimit invocation should terminate deterministically"
    );
    assert!(tight.transcript_path.exists());
}

fn assert_fresh_cli_invocation(invocation: &CliInvocation, expected_exit: i32) {
    assert_eq!(invocation.exit_code, Some(expected_exit));
    assert_eq!(
        invocation.argv.first().map(String::as_str),
        Some("loop-engine")
    );
    assert!(
        invocation.transcript_path.starts_with(
            invocation
                .env
                .get("LOOP_ENGINE_HOME")
                .expect("LOOP_ENGINE_HOME")
        )
    );
    assert!(invocation.transcript_path.exists());
}
