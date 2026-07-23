//! Black-box E2E harness support (T143–T145). Crate-private to integration tests.

mod alpha;
mod cli;
mod coverage;
mod provider;
mod sandbox;
mod sqlite_fixture;
mod strict_json;
mod trace;

#[cfg(unix)]
mod rlimit;

pub(crate) use alpha::{AlphaInvocation, add_scenario_provider, create_run, invoke_json};
pub(crate) use cli::{
    CliInvocation, StructuredDocument, StructuredParseError, parse_pre_dispatch_stderr,
    parse_structured_stdout,
};
pub(crate) use coverage::RuntimeCoverageRecorder;
pub(crate) use provider::{
    ProviderAddArgs, ProviderConfigFile, ProviderExecutableError, provider_manifest_path,
    reference_provider_executable, reference_provider_manifest_path,
    resolve_provider_executable_path, scenario_provider_executable,
    scenario_provider_manifest_path,
};
pub(crate) use sandbox::E2eSandbox;
pub(crate) use sqlite_fixture::{
    CorruptionKind, TombstonedRegistrationSetup, apply_initial_migration, corrupt_database,
    count_evidence_associations, count_evidence_records, count_journal_entries, count_runs,
    execute_sql, harness_fixture_db_path, insert_provider_registrations,
    insert_tombstoned_registration, require_sqlite3, set_provider_registration_command,
    set_run_projection_state, tombstone_provider_registration, validate_tombstoned_registration,
};
pub(crate) use trace::{
    TraceParseError, install_trace_fixture, parse_correlated_trace, parse_correlated_value,
    read_trace_events, trace_fixture_path,
};

#[cfg(unix)]
pub(crate) use rlimit::{run_with_rlimit_fsize, verify_rlimit_blocks_writes};
