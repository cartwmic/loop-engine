#[allow(
    dead_code,
    reason = "focused target exercises representative diagnostic paths"
)]
#[path = "../src/diagnostics.rs"]
mod diagnostics;

use std::io;
use std::path::PathBuf;

use diagnostics::{
    InvocationCorrelation, PreDispatchPhase, SCHEMA_VERSION, configuration_failure,
    corruption_diagnostics, corruption_pre_dispatch, diagnostic_entries_from_core,
    diagnostic_entry_from_core, parse_failure, persistence_failure, provider_invocation_diagnostic,
    provider_invocation_from_process_error, reason_presentation, render_outcome_failure_human,
    render_pre_dispatch_human, render_pre_dispatch_json, trace_init_failure,
    trace_sink_failure_diagnostic,
};
use loop_engine_core::model::attempt::ProviderRole;
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_integrations::configuration::ConfigurationError;
use loop_engine_integrations::persistence::{
    CorruptionContext, CorruptionDiagnostic, CorruptionError, CorruptionKind, CorruptionPhase,
    PersistenceError,
};
use loop_engine_integrations::provider_process::ProcessError;
use loop_engine_integrations::trace::{TraceError, TraceIoPhase};
use serde_json::{Value, json};

#[test]
fn structured_parse_failure_matches_cli_contract_example() {
    let correlation = InvocationCorrelation::with_trace(
        "01J9X3K2M4N5P6Q7R8S9T0V5A",
        "/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V5A.jsonl",
    );
    let failure = parse_failure(
        "unknown flag --limt",
        correlation,
        vec!["run list: unrecognized flag --limt".into()],
    );
    let rendered = render_pre_dispatch_json(&failure).unwrap();
    let value: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert_eq!(value["phase"], "parse");
    assert_eq!(value["message"], "unknown flag --limt");
    assert_eq!(value["request_id"], "01J9X3K2M4N5P6Q7R8S9T0V5A");
    assert!(value["trace"].as_str().unwrap().ends_with(".jsonl"));
    assert_eq!(
        value["source_chain"],
        json!(["run list: unrecognized flag --limt"])
    );
}

#[test]
fn nested_configuration_failure_exposes_key_path_in_source_chain() {
    let correlation =
        InvocationCorrelation::with_trace("01J9X3K2M4N5P6Q7R8S9T0V1W", "/tmp/trace.jsonl");
    let failure = configuration_failure(
        &ConfigurationError::Malformed {
            path: PathBuf::from("/home/alice/.config/loop-engine/config.toml"),
            message: "unexpected key `providers` at line 4 column 1".into(),
        },
        correlation,
    );
    let human = render_pre_dispatch_human(&failure);
    assert!(human.contains("Phase: config"));
    assert!(human.contains("config.toml"));
    assert!(human.contains("unexpected key `providers`"));
    let json = render_pre_dispatch_json(&failure).unwrap();
    assert!(json.contains("config"));
}

#[test]
fn nested_persistence_create_failure_preserves_source_chain() {
    let correlation = InvocationCorrelation::with_trace("req-open", "/tmp/open-trace.jsonl");
    let failure = persistence_failure(
        &PersistenceError::CreateDirectory {
            path: PathBuf::from("/tmp/state.db"),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
        },
        correlation,
    );
    let human = render_pre_dispatch_human(&failure);
    assert!(human.contains("failed to create persistence directory"));
    assert!(human.contains("state.db"));
    assert!(human.contains("Source chain:"));
}

#[test]
fn trace_init_structured_failure_has_no_trace_or_request_id() {
    let failure = trace_init_failure(&TraceError::BudgetExhausted {
        required: 16_777_216,
        available: 0,
    });
    assert!(failure.request_id.is_none());
    assert!(failure.trace.is_none());
    assert_eq!(failure.phase, PreDispatchPhase::TraceInit);
    let human = render_pre_dispatch_human(&failure);
    assert!(human.contains("trace directory budget is exhausted"));
    assert!(human.contains("No operational trace was created"));
}

#[test]
fn provider_invocation_diagnostic_points_to_trace_not_payload_repeat() {
    let entry = provider_invocation_diagnostic(
        ProviderRole::Describe,
        "Role describe timed out after 60 seconds",
        "provider.timeout",
        Some("019f6e88-b403-73a6-89f9-ebfe668b417e"),
    );
    assert_eq!(entry.code, "provider.invocation");
    assert_eq!(entry.context.as_ref().unwrap()["role"], json!("describe"));
    assert_eq!(
        entry.context.as_ref().unwrap()["failure_code"],
        json!("provider.timeout")
    );
    let human = render_outcome_failure_human(
        &InvocationCorrelation::with_trace("req", "/tmp/trace.jsonl"),
        &reason_presentation(
            &Reason::new(
                ReasonCode::ProviderTimeout,
                "Provider process exceeded configured timeout",
            )
            .unwrap(),
        ),
        std::slice::from_ref(&entry),
        &[],
    );
    assert!(human.contains("operational trace"));
    assert!(!human.contains("stdout"));
}

#[test]
fn process_error_maps_to_provider_invocation_context() {
    let entry = provider_invocation_from_process_error(
        ProviderRole::Describe,
        &ProcessError::Timeout,
        None,
    );
    assert_eq!(
        entry.context.as_ref().unwrap()["failure_code"],
        json!("provider.timeout")
    );
}

#[test]
fn corruption_pre_dispatch_vs_post_dispatch_split() {
    let pre = CorruptionError::single(
        CorruptionPhase::Open,
        CorruptionDiagnostic::new(
            CorruptionKind::IntegrityKeyMissing,
            "persistence.corruption.integrity_key_missing",
            "integration metadata key integrity_key is missing",
            CorruptionContext::default(),
        ),
        vec!["metadata key integrity_key missing".into()],
    );
    let post = CorruptionError::single(
        CorruptionPhase::Read,
        CorruptionDiagnostic::new(
            CorruptionKind::JournalSequenceDiscontinuity,
            "persistence.corruption.journal.sequence_gap",
            "journal sequence gap detected",
            CorruptionContext::default(),
        ),
        vec!["journal sequence 7 missing".into()],
    );

    let correlation = InvocationCorrelation::with_trace("req", "/tmp/trace.jsonl");
    let pre_dispatch = corruption_pre_dispatch(&pre, correlation.clone()).expect("open phase");
    assert_eq!(pre_dispatch.phase, PreDispatchPhase::Persistence);
    assert!(corruption_pre_dispatch(&post, correlation).is_none());

    let post_entries = corruption_diagnostics(&post).unwrap();
    assert_eq!(post_entries.len(), 1);
    assert!(post_entries[0].code.starts_with("persistence.corruption."));
}

#[test]
fn trace_sink_failure_diagnostic_carries_errno_phase_and_after_commit() {
    let entry = trace_sink_failure_diagnostic(
        &TraceError::Io {
            path: PathBuf::from("/tmp/trace.jsonl"),
            phase: TraceIoPhase::Fsync,
            source: io::ErrorKind::Other.into(),
        },
        true,
    );
    assert_eq!(entry.code, "trace.sink_failure");
    assert_eq!(entry.context.as_ref().unwrap()["phase"], json!("fsync"));
    assert_eq!(entry.context.as_ref().unwrap()["after_commit"], json!(true));
}

#[test]
fn core_diagnostics_round_trip_preserves_order_and_context() {
    let diagnostics = vec![
        Diagnostic::new("provider.invocation", "first", None).unwrap(),
        Diagnostic::new("persistence.failed", "second", Some("/inputs/name".into())).unwrap(),
    ];
    let entries = diagnostic_entries_from_core(&diagnostics).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[1].context.as_ref().and_then(|ctx| ctx.get("path")),
        Some(&json!("/inputs/name"))
    );
    let round = diagnostic_entry_from_core(
        &Diagnostic::new(
            entries[1].code.clone(),
            entries[1].message.clone(),
            entries[1]
                .context
                .as_ref()
                .and_then(|ctx| ctx.get("path"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(round.context, entries[1].context);
}

#[test]
fn source_chain_root_cause_renders_last_without_payload_duplication() {
    let failure = parse_failure(
        "invalid run command",
        InvocationCorrelation::with_trace("req", "/tmp/trace.jsonl"),
        vec![
            "run request: missing required argument <EVENT>".into(),
            "clap: required argument missing".into(),
        ],
    );
    let human = render_pre_dispatch_human(&failure);
    let chain_start = human.find("Source chain:").unwrap();
    let chain_section = &human[chain_start..];
    assert!(chain_section.contains("run request: missing required argument <EVENT>"));
    assert!(chain_section.contains("clap: required argument missing"));
    assert!(human.contains("Full provider, persistence, and request/outcome payloads"));
}
