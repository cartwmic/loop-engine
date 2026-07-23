mod compatibility_attempt;
pub mod corruption;
mod error;
mod event_attempt;
mod evidence_reads;
pub(crate) mod export_snapshot;
mod guidance_attempt;
mod history;
pub mod mapping;
mod migrations;
mod provider_catalog;
pub mod records;
mod run_create;
mod run_mutations;
mod run_reads;
mod sqlite;
pub(crate) mod traced;

pub use compatibility_attempt::{CompatibilityAttemptError, CompatibilityAttemptWriter};
pub use corruption::{
    CorruptionContext, CorruptionDiagnostic, CorruptionError, CorruptionKind, CorruptionPhase,
    LogicalAuthoritySnapshot, TableInventory, classify_mapping_error, classify_persistence_error,
    classify_sqlite_error, inspect_file_header, inspect_logical_store, inspect_open_readonly,
    integrity_key_hash, physical_fixture_sha256, validate_journal_sequences_for_run,
};
pub use error::PersistenceError;
pub use event_attempt::{EventAttemptPersistenceError, SqliteEventAttemptWriter};
pub use evidence_reads::{EvidenceReadError, SqliteEvidenceReads};
pub use guidance_attempt::{GuidanceAttemptError, GuidanceAttemptWriter};
pub use history::{HistoryReadError, SqliteHistoryReads};
pub use migrations::SUPPORTED_SCHEMA_VERSION;
pub use provider_catalog::{CatalogPersistenceError, DisableWarningsPage, SqliteProviderCatalog};
pub use run_create::{RunCreateError, SqliteRunWriter};
pub use run_mutations::{RunMutationError, SqliteRunMutations, journal_entry_value};
pub use run_reads::{RawRunRow, RunReadError, SqliteRunReads};
pub use sqlite::{SqliteStore, connect_with_pragmas};
pub use traced::{
    MutationClass, OptionalTraceSink, PersistenceTraceFailure, PersistenceTraceSink,
    ReadCompleteExtras, ReadTraceSession, SemanticOutcome, WriteTraceSession,
};
