//! Centralized persistence corruption diagnostics and classification (T117).
//!
//! Rich error taxonomy for malformed databases, row DTO violations, referential
//! integrity, journal sequence continuity, schema version, and integrity-key
//! failures. Never repairs, defaults, or deletes store content.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use loop_engine_core::model::diagnostic::Diagnostic;
use rusqlite::{Connection, Error as SqliteError, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::sha256_digest::sha256_label;

use super::error::PersistenceError;
use super::history::{HistoryReadError, validate_journal_record_semantics};
use super::mapping::{self, MappingError};
use super::migrations::SUPPORTED_SCHEMA_VERSION;
use super::records::{EvidenceRecordRow, JournalRecord, ProviderRegistrationRecord, RunRecord};

const INTEGRATION_METADATA_TABLE: &str = "integration_metadata";
const INTEGRITY_KEY_ROW_KEY: &str = "integrity_key";
const INTEGRITY_KEY_BYTE_LENGTH: usize = 32;

const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";
const INSPECTION_SAVEPOINT: &str = "inspect_logical_store";

/// Stable diagnostic codes for CLI envelope and trace correlation.
pub mod codes {
    pub const MALFORMED_HEADER: &str = "persistence.corruption.malformed_header";
    pub const NOT_A_DATABASE: &str = "persistence.corruption.not_a_database";
    pub const SQLITE_PHYSICAL: &str = "persistence.corruption.sqlite_physical";
    pub const SCHEMA_FUTURE: &str = "persistence.corruption.schema_future";
    pub const SCHEMA_INCOMPATIBLE: &str = "persistence.corruption.schema_incompatible";
    pub const INTEGRITY_KEY_MISSING: &str = "persistence.corruption.integrity_key_missing";
    pub const INTEGRITY_KEY_INVALID: &str = "persistence.corruption.integrity_key_invalid";
    pub const ROW_MALFORMED_JSON: &str = "persistence.corruption.row.malformed_json";
    pub const ROW_UNSUPPORTED_VERSION: &str = "persistence.corruption.row.unsupported_version";
    pub const ROW_UNSUPPORTED_ENUM: &str = "persistence.corruption.row.unsupported_enum";
    pub const ROW_GRAPH_DIGEST: &str = "persistence.corruption.row.graph_digest_mismatch";
    pub const ROW_GRAPH_SEMANTICS: &str = "persistence.corruption.row.graph_semantics";
    pub const ROW_LIFECYCLE_STATE: &str = "persistence.corruption.row.lifecycle_state";
    pub const ROW_INVALID_VERSION: &str = "persistence.corruption.row.invalid_version";
    pub const ROW_BOUNDED_VALUE: &str = "persistence.corruption.row.bounded_semantic";
    pub const REGISTRATION_REFERENTIAL: &str =
        "persistence.corruption.referential.registration_binding";
    pub const EVIDENCE_ASSOCIATION: &str =
        "persistence.corruption.referential.evidence_association";
    pub const JOURNAL_SEQUENCE_GAP: &str = "persistence.corruption.journal.sequence_gap";
    pub const JOURNAL_SEQUENCE_ALLOCATOR: &str =
        "persistence.corruption.journal.sequence_allocator";
    pub const JOURNAL_PAYLOAD_INCONSISTENT: &str =
        "persistence.corruption.journal.payload_inconsistent";
}

/// Operation boundary where corruption was detected (maps to CLI `phase` / trace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptionPhase {
    /// Pre-dispatch open (`phase`: `persistence`, exit `64`).
    Open,
    /// Pre-dispatch migration (`phase`: `persistence`, exit `64`).
    Migration,
    /// Post-dispatch read path (`outcome`: `error`, `reason.code`: `persistence.failed`).
    Read,
    /// Post-dispatch write path.
    Write,
    /// Post-dispatch export snapshot read.
    Export,
}

impl CorruptionPhase {
    pub fn is_pre_dispatch(self) -> bool {
        matches!(self, Self::Open | Self::Migration)
    }

    pub fn cli_phase_label(self) -> &'static str {
        match self {
            Self::Open | Self::Migration => "persistence",
            Self::Read | Self::Write | Self::Export => "operation",
        }
    }
}

/// Semantic corruption category for classification and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptionKind {
    MalformedDatabaseHeader,
    NotADatabase,
    SqlitePhysicalCorruption,
    SchemaFutureVersion,
    SchemaIncompatible,
    IntegrityKeyMissing,
    IntegrityKeyInvalidLength,
    RowMalformedJson,
    RowUnsupportedVersion,
    RowUnsupportedEnum,
    RowGraphDigestMismatch,
    RowGraphSemantics,
    RowInvalidLifecycleState,
    RowInvalidVersion,
    RowBoundedSemanticValue,
    RegistrationReferentialIntegrity,
    EvidenceAssociationIntegrity,
    JournalSequenceDiscontinuity,
    JournalSequenceAllocatorMismatch,
    JournalPayloadInconsistent,
}

/// Bounded structured context for one corruption finding (no secrets or raw MAC).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CorruptionContext {
    values: BTreeMap<String, String>,
}

impl CorruptionContext {
    pub fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl fmt::Display) -> Self {
        self.values.insert(key.into(), value.to_string());
        self
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

/// One actionable corruption diagnostic suitable for CLI `diagnostics[]` rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorruptionDiagnostic {
    pub kind: CorruptionKind,
    pub code: &'static str,
    pub message: String,
    pub context: CorruptionContext,
}

impl CorruptionDiagnostic {
    pub fn new(
        kind: CorruptionKind,
        code: &'static str,
        message: impl Into<String>,
        context: CorruptionContext,
    ) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            context,
        }
    }

    pub fn to_core_diagnostic(
        &self,
    ) -> Result<Diagnostic, loop_engine_core::model::bounded::BoundError> {
        let path = if self.context.values.is_empty() {
            None
        } else {
            Some(canonical_context_json(&self.context))
        };
        Diagnostic::new(self.code, &self.message, path)
    }
}

/// Rich corruption error preserving a sanitized source chain (root cause last).
#[derive(Debug, Error)]
#[error("{summary}")]
pub struct CorruptionError {
    pub phase: CorruptionPhase,
    pub diagnostics: Vec<CorruptionDiagnostic>,
    source_chain: Vec<String>,
    summary: String,
}

impl CorruptionError {
    pub fn single(
        phase: CorruptionPhase,
        diagnostic: CorruptionDiagnostic,
        source_chain: Vec<String>,
    ) -> Self {
        let summary = diagnostic.message.clone();
        Self {
            phase,
            diagnostics: vec![diagnostic],
            source_chain: sanitize_source_chain(source_chain),
            summary,
        }
    }

    pub fn multiple(
        phase: CorruptionPhase,
        diagnostics: Vec<CorruptionDiagnostic>,
        source_chain: Vec<String>,
    ) -> Self {
        let summary = diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "persistence store corruption detected".into());
        Self {
            phase,
            diagnostics,
            source_chain: sanitize_source_chain(source_chain),
            summary,
        }
    }

    pub fn primary(&self) -> Option<&CorruptionDiagnostic> {
        self.diagnostics.first()
    }

    pub fn source_chain(&self) -> &[String] {
        &self.source_chain
    }

    pub fn reason_code(&self) -> &'static str {
        "persistence.failed"
    }

    pub fn to_core_diagnostics(
        &self,
    ) -> Result<Vec<Diagnostic>, loop_engine_core::model::bounded::BoundError> {
        self.diagnostics
            .iter()
            .map(CorruptionDiagnostic::to_core_diagnostic)
            .collect()
    }
}

/// Per-table authoritative row inventory for logical authority comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInventory {
    pub row_count: u64,
    pub content_digest: String,
}

/// Logical authority snapshot of a live WAL-backed database.
///
/// Captures schema version, integrity-key hash (never raw key material), and
/// canonical content digests for every authoritative table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalAuthoritySnapshot {
    pub user_version: u32,
    pub integrity_key_hash: String,
    pub tables: BTreeMap<String, TableInventory>,
}

impl LogicalAuthoritySnapshot {
    pub fn capture(conn: &Connection) -> Result<Self, CorruptionError> {
        let user_version = read_user_version(conn)?;
        let integrity_key_hash = integrity_key_fingerprint(conn)?;
        let tables = inventory_all_tables(conn)?;
        Ok(Self {
            user_version,
            integrity_key_hash,
            tables,
        })
    }

    pub fn assert_unchanged(before: &Self, after: &Self) -> Result<(), CorruptionError> {
        if before == after {
            Ok(())
        } else {
            Err(CorruptionError::multiple(
                CorruptionPhase::Read,
                vec![CorruptionDiagnostic::new(
                    CorruptionKind::SqlitePhysicalCorruption,
                    codes::SQLITE_PHYSICAL,
                    "logical authority changed after failed persistence operation",
                    CorruptionContext::new()
                        .with("before_user_version", before.user_version)
                        .with("after_user_version", after.user_version)
                        .with("before_integrity_key_hash", &before.integrity_key_hash)
                        .with("after_integrity_key_hash", &after.integrity_key_hash),
                )],
                vec!["authority snapshot mismatch".into()],
            ))
        }
    }
}

/// SHA-256 over immutable on-disk bytes of a copied fixture (never live WAL files).
pub fn physical_fixture_sha256(path: &Path) -> Result<String, std::io::Error> {
    let bytes = std::fs::read(path)?;
    Ok(sha256_label(&bytes))
}

/// Classify SQLite open/read errors including malformed header and not-a-database.
pub fn classify_sqlite_error(
    error: &SqliteError,
    phase: CorruptionPhase,
    source_chain: Vec<String>,
) -> CorruptionError {
    if let Some(code) = error.sqlite_error_code() {
        match code {
            rusqlite::ErrorCode::NotADatabase => {
                return CorruptionError::single(
                    phase,
                    CorruptionDiagnostic::new(
                        CorruptionKind::NotADatabase,
                        codes::NOT_A_DATABASE,
                        "file is not a SQLite database",
                        CorruptionContext::new(),
                    ),
                    source_chain,
                );
            }
            rusqlite::ErrorCode::DatabaseCorrupt => {
                return CorruptionError::single(
                    phase,
                    CorruptionDiagnostic::new(
                        CorruptionKind::SqlitePhysicalCorruption,
                        codes::SQLITE_PHYSICAL,
                        "SQLite reported database corruption",
                        CorruptionContext::new().with("sqlite_message", error.to_string()),
                    ),
                    source_chain,
                );
            }
            _ => {}
        }
    }
    CorruptionError::single(
        phase,
        CorruptionDiagnostic::new(
            CorruptionKind::SqlitePhysicalCorruption,
            codes::SQLITE_PHYSICAL,
            error.to_string(),
            CorruptionContext::new(),
        ),
        source_chain,
    )
}

/// Classify integration open-path persistence errors (schema, integrity key).
pub fn classify_persistence_error(
    error: &PersistenceError,
    phase: CorruptionPhase,
    source_chain: Vec<String>,
) -> CorruptionError {
    let diagnostic = match error {
        PersistenceError::FutureSchema {
            supported,
            observed,
        } => CorruptionDiagnostic::new(
            CorruptionKind::SchemaFutureVersion,
            codes::SCHEMA_FUTURE,
            format!("database schema version {observed} exceeds supported version {supported}"),
            CorruptionContext::new()
                .with("supported_schema_version", *supported)
                .with("observed_schema_version", *observed),
        ),
        PersistenceError::InvalidUserVersion { observed } => CorruptionDiagnostic::new(
            CorruptionKind::SchemaIncompatible,
            codes::SCHEMA_INCOMPATIBLE,
            format!("invalid SQLite user_version {observed}"),
            CorruptionContext::new().with("observed_schema_version", *observed),
        ),
        PersistenceError::MetadataKeyMissing { key } => CorruptionDiagnostic::new(
            CorruptionKind::IntegrityKeyMissing,
            codes::INTEGRITY_KEY_MISSING,
            format!("integration metadata key {key} is missing"),
            CorruptionContext::new().with("metadata_key", *key),
        ),
        PersistenceError::MetadataKeyInvalidLength {
            key,
            expected,
            actual,
        } => CorruptionDiagnostic::new(
            CorruptionKind::IntegrityKeyInvalidLength,
            codes::INTEGRITY_KEY_INVALID,
            format!("integration metadata key {key} has length {actual}; expected {expected}"),
            CorruptionContext::new()
                .with("metadata_key", *key)
                .with("expected_length", *expected)
                .with("actual_length", *actual),
        ),
        PersistenceError::Open { path, source } => {
            if is_malformed_sqlite_header(path) {
                return CorruptionError::single(
                    phase,
                    CorruptionDiagnostic::new(
                        CorruptionKind::MalformedDatabaseHeader,
                        codes::MALFORMED_HEADER,
                        "SQLite database header is malformed or truncated",
                        CorruptionContext::new().with("path", path.display()),
                    ),
                    push_chain(source_chain, source.to_string()),
                );
            }
            return classify_sqlite_error(source, phase, push_chain(source_chain, "open".into()));
        }
        other => CorruptionDiagnostic::new(
            CorruptionKind::SchemaIncompatible,
            codes::SCHEMA_INCOMPATIBLE,
            other.to_string(),
            CorruptionContext::new(),
        ),
    };
    CorruptionError::single(phase, diagnostic, source_chain)
}

/// Classify row DTO / mapping corruption detected during decode.
pub fn classify_mapping_error(
    error: &MappingError,
    phase: CorruptionPhase,
    context: CorruptionContext,
    source_chain: Vec<String>,
) -> CorruptionError {
    let (kind, code, message) = match error {
        MappingError::MalformedJson { field, message } => (
            CorruptionKind::RowMalformedJson,
            codes::ROW_MALFORMED_JSON,
            format!("malformed JSON in {field}: {message}"),
        ),
        MappingError::UnsupportedVersion { field, value } => (
            CorruptionKind::RowUnsupportedVersion,
            codes::ROW_UNSUPPORTED_VERSION,
            format!("unsupported version in {field}: {value}"),
        ),
        MappingError::UnsupportedEnum { field, value } => (
            CorruptionKind::RowUnsupportedEnum,
            codes::ROW_UNSUPPORTED_ENUM,
            format!("unsupported enum in {field}: {value}"),
        ),
        MappingError::GraphDigestMismatch { stored, computed } => (
            CorruptionKind::RowGraphDigestMismatch,
            codes::ROW_GRAPH_DIGEST,
            format!("graph digest mismatch: stored {stored}, computed {computed}"),
        ),
        MappingError::GraphSemantics { message } => (
            CorruptionKind::RowGraphSemantics,
            codes::ROW_GRAPH_SEMANTICS,
            format!("graph semantics invalid: {message}"),
        ),
        MappingError::InvalidLifecycleState { message } => (
            CorruptionKind::RowInvalidLifecycleState,
            codes::ROW_LIFECYCLE_STATE,
            format!("invalid lifecycle or state: {message}"),
        ),
        MappingError::InvalidVersion { field, message } => (
            CorruptionKind::RowInvalidVersion,
            codes::ROW_INVALID_VERSION,
            format!("invalid version in {field}: {message}"),
        ),
        MappingError::BoundedSemanticValue { field, message } => (
            CorruptionKind::RowBoundedSemanticValue,
            codes::ROW_BOUNDED_VALUE,
            format!("bounded semantic value rejected at {field}: {message}"),
        ),
    };
    let mut ctx = context;
    if let MappingError::MalformedJson { field, .. }
    | MappingError::UnsupportedVersion { field, .. }
    | MappingError::UnsupportedEnum { field, .. }
    | MappingError::InvalidVersion { field, .. }
    | MappingError::BoundedSemanticValue { field, .. } = error
    {
        ctx = ctx.with("field", *field);
    }
    CorruptionError::single(
        phase,
        CorruptionDiagnostic::new(kind, code, message, ctx),
        source_chain,
    )
}

/// Inspect a database file header without opening through rusqlite migrations.
pub fn inspect_file_header(path: &Path) -> Result<(), CorruptionError> {
    let bytes = std::fs::read(path).map_err(|error| {
        CorruptionError::single(
            CorruptionPhase::Open,
            CorruptionDiagnostic::new(
                CorruptionKind::MalformedDatabaseHeader,
                codes::MALFORMED_HEADER,
                format!("unable to read database file: {error}"),
                CorruptionContext::new().with("path", path.display()),
            ),
            vec![error.to_string()],
        )
    })?;
    if bytes.len() < SQLITE_HEADER.len() {
        return Err(CorruptionError::single(
            CorruptionPhase::Open,
            CorruptionDiagnostic::new(
                CorruptionKind::MalformedDatabaseHeader,
                codes::MALFORMED_HEADER,
                "SQLite database header is truncated",
                CorruptionContext::new()
                    .with("path", path.display())
                    .with("header_bytes", bytes.len()),
            ),
            vec!["header shorter than SQLite magic".into()],
        ));
    }
    if &bytes[..SQLITE_HEADER.len()] != SQLITE_HEADER {
        return Err(CorruptionError::single(
            CorruptionPhase::Open,
            CorruptionDiagnostic::new(
                CorruptionKind::MalformedDatabaseHeader,
                codes::MALFORMED_HEADER,
                "SQLite database header magic mismatch",
                CorruptionContext::new().with("path", path.display()),
            ),
            vec!["expected SQLite format 3 magic".into()],
        ));
    }
    Ok(())
}

/// Attempt read-only open and run full logical corruption inspection.
pub fn inspect_open_readonly(path: &Path) -> Result<(), CorruptionError> {
    inspect_file_header(path)?;
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Open, vec!["readonly open".into()])
        })?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Open,
                vec!["pragma foreign_keys".into()],
            )
        })?;
    inspect_logical_store(&conn)
}

/// Scan all authoritative tables for logical corruption.
pub fn inspect_logical_store(conn: &Connection) -> Result<(), CorruptionError> {
    let guard = InspectionSnapshotGuard::begin(conn)?;
    let result = inspect_logical_store_scanned(conn);
    match result {
        Ok(()) => {
            guard.release()?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn inspect_logical_store_scanned(conn: &Connection) -> Result<(), CorruptionError> {
    let user_version = read_user_version(conn)?;
    if user_version > SUPPORTED_SCHEMA_VERSION {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::SchemaFutureVersion,
                codes::SCHEMA_FUTURE,
                format!(
                    "database schema version {user_version} exceeds supported version {SUPPORTED_SCHEMA_VERSION}"
                ),
                CorruptionContext::new()
                    .with("supported_schema_version", SUPPORTED_SCHEMA_VERSION)
                    .with("observed_schema_version", user_version),
            ),
            vec!["pragma user_version".into()],
        ));
    }
    if user_version < SUPPORTED_SCHEMA_VERSION {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::SchemaIncompatible,
                codes::SCHEMA_INCOMPATIBLE,
                format!(
                    "database schema version {user_version} is below supported version {SUPPORTED_SCHEMA_VERSION}"
                ),
                CorruptionContext::new()
                    .with("supported_schema_version", SUPPORTED_SCHEMA_VERSION)
                    .with("observed_schema_version", user_version),
            ),
            vec!["pragma user_version".into()],
        ));
    }

    let mut findings = Vec::new();
    let mut chain = Vec::new();

    if let Err(error) = verify_integrity_key_row(conn) {
        if let Some(diagnostic) = error.primary().cloned() {
            findings.push(diagnostic);
        } else {
            return Err(error);
        }
    }

    let run_ids = load_run_ids(conn)?;
    findings.extend(validate_provider_registrations(conn)?);
    findings.extend(validate_runs(conn)?);
    findings.extend(validate_evidence_rows(conn, &run_ids)?);
    findings.extend(validate_orphan_referential_rows(conn, &run_ids)?);
    findings.extend(validate_journal_integrity(conn)?);
    findings.extend(validate_evidence_associations(conn)?);
    findings.extend(validate_foreign_key_violations(conn)?);

    if findings.is_empty() {
        Ok(())
    } else {
        chain.push(format!("{} logical corruption finding(s)", findings.len()));
        Err(CorruptionError::multiple(
            CorruptionPhase::Read,
            findings,
            chain,
        ))
    }
}

/// Validate per-run journal allocator, creation entry, and contiguous sequences.
pub fn validate_journal_sequences_for_run(
    conn: &Connection,
    run_id: &str,
) -> Result<(), CorruptionError> {
    let next_sequence: Option<i64> = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["run_journal_sequences lookup".into()],
            )
        })?;

    let Some(allocator_next_sequence) = next_sequence else {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalSequenceAllocatorMismatch,
                codes::JOURNAL_SEQUENCE_ALLOCATOR,
                format!("run_journal_sequences row missing for run {run_id}"),
                CorruptionContext::new().with("run_id", run_id),
            ),
            vec!["run_journal_sequences lookup".into()],
        ));
    };
    if let Err(diagnostic) = decode_version_i64(
        "next_sequence",
        allocator_next_sequence,
        CorruptionContext::new().with("run_id", run_id),
    ) {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            diagnostic,
            vec!["run_journal_sequences lookup".into()],
        ));
    }

    let records = load_journal_records(conn, run_id)?;
    if records.is_empty() {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!(
                    "run {run_id} has no journal entries; sequence 1 creation entry is required"
                ),
                CorruptionContext::new().with("run_id", run_id),
            ),
            vec!["journal_entries creation entry".into()],
        ));
    }

    let mut expected = 1u64;
    for record in &records {
        if record.sequence != expected {
            let (kind, code, message) = if expected == 1 {
                (
                    CorruptionKind::JournalPayloadInconsistent,
                    codes::JOURNAL_PAYLOAD_INCONSISTENT,
                    format!(
                        "run {run_id} is missing sequence 1 creation journal entry; expected 1, found {}",
                        record.sequence
                    ),
                )
            } else {
                (
                    CorruptionKind::JournalSequenceDiscontinuity,
                    codes::JOURNAL_SEQUENCE_GAP,
                    format!(
                        "journal sequence discontinuity for run {run_id}: expected {expected}, found {}",
                        record.sequence
                    ),
                )
            };
            return Err(CorruptionError::single(
                CorruptionPhase::Read,
                CorruptionDiagnostic::new(
                    kind,
                    code,
                    message,
                    CorruptionContext::new()
                        .with("run_id", run_id)
                        .with("expected_sequence", expected)
                        .with("observed_sequence", record.sequence),
                ),
                vec!["journal_entries ordered scan".into()],
            ));
        }
        if record.sequence == 1 {
            validate_creation_journal_entry(record, run_id)?;
        }
        if let Err(error) = mapping::validate_journal_record(record) {
            return Err(classify_mapping_error(
                &error,
                CorruptionPhase::Read,
                CorruptionContext::new()
                    .with("run_id", run_id)
                    .with("sequence", record.sequence),
                vec!["journal payload validation".into()],
            ));
        }
        expected = expected.saturating_add(1);
    }

    let expected_next = i64::try_from(expected).unwrap_or(i64::MAX);
    if allocator_next_sequence != expected_next {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalSequenceAllocatorMismatch,
                codes::JOURNAL_SEQUENCE_ALLOCATOR,
                format!(
                    "run_journal_sequences.next_sequence {allocator_next_sequence} does not match journal tail {expected_next} for run {run_id}"
                ),
                CorruptionContext::new()
                    .with("run_id", run_id)
                    .with("allocator_next_sequence", allocator_next_sequence)
                    .with("expected_next_sequence", expected_next),
            ),
            vec!["run_journal_sequences consistency".into()],
        ));
    }

    Ok(())
}

fn decode_version_i64(
    field: &'static str,
    value: i64,
    context: CorruptionContext,
) -> Result<u64, CorruptionDiagnostic> {
    if value <= 0 {
        return Err(CorruptionDiagnostic::new(
            CorruptionKind::RowInvalidVersion,
            codes::ROW_INVALID_VERSION,
            format!("invalid version in {field}: must be positive"),
            context.with("field", field).with("observed_value", value),
        ));
    }
    u64::try_from(value).map_err(|_| {
        CorruptionDiagnostic::new(
            CorruptionKind::RowInvalidVersion,
            codes::ROW_INVALID_VERSION,
            format!("invalid version in {field}: exceeds representable range"),
            context.with("field", field).with("observed_value", value),
        )
    })
}

fn decode_positive_bounded_i64(
    field: &'static str,
    value: i64,
    context: CorruptionContext,
) -> Result<u64, CorruptionDiagnostic> {
    if value <= 0 {
        return Err(CorruptionDiagnostic::new(
            CorruptionKind::RowBoundedSemanticValue,
            codes::ROW_BOUNDED_VALUE,
            format!("bounded semantic value rejected at {field}: must be positive"),
            context.with("field", field).with("observed_value", value),
        ));
    }
    u64::try_from(value).map_err(|_| {
        CorruptionDiagnostic::new(
            CorruptionKind::RowBoundedSemanticValue,
            codes::ROW_BOUNDED_VALUE,
            format!("bounded semantic value rejected at {field}: exceeds representable range"),
            context.with("field", field).with("observed_value", value),
        )
    })
}

fn decode_provider_enabled(
    value: i64,
    context: CorruptionContext,
) -> Result<bool, CorruptionDiagnostic> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CorruptionDiagnostic::new(
            CorruptionKind::RowUnsupportedEnum,
            codes::ROW_UNSUPPORTED_ENUM,
            format!("unsupported enum in enabled: {value}"),
            context
                .with("field", "enabled")
                .with("observed_value", value),
        )),
    }
}

struct InspectionSnapshotGuard<'conn> {
    conn: &'conn Connection,
    released: bool,
}

impl<'conn> InspectionSnapshotGuard<'conn> {
    fn begin(conn: &'conn Connection) -> Result<Self, CorruptionError> {
        conn.execute(&format!("SAVEPOINT {INSPECTION_SAVEPOINT}"), [])
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["inspect_logical_store savepoint".into()],
                )
            })?;
        Ok(Self {
            conn,
            released: false,
        })
    }

    fn release(mut self) -> Result<(), CorruptionError> {
        self.conn
            .execute(&format!("RELEASE SAVEPOINT {INSPECTION_SAVEPOINT}"), [])
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["inspect_logical_store release".into()],
                )
            })?;
        self.released = true;
        Ok(())
    }
}

impl Drop for InspectionSnapshotGuard<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {INSPECTION_SAVEPOINT}; RELEASE SAVEPOINT {INSPECTION_SAVEPOINT};"
            ));
        }
    }
}

fn load_run_ids(conn: &Connection) -> Result<BTreeSet<String>, CorruptionError> {
    let mut statement = conn
        .prepare("SELECT run_id FROM runs ORDER BY run_id ASC")
        .map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Read, vec!["runs id index".into()])
        })?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs id index rows".into()],
            )
        })?;
    let mut run_ids = BTreeSet::new();
    for run_id in rows {
        run_ids.insert(run_id.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs id index row".into()],
            )
        })?);
    }
    Ok(run_ids)
}

fn validate_provider_registrations(
    conn: &Connection,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn
        .prepare(
            "SELECT registration_id, handle, enabled, config_revision, executable, argv_json,
                    working_directory, timeout_seconds, created_at, updated_at
             FROM provider_registrations
             ORDER BY registration_id ASC",
        )
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["provider_registrations scan".into()],
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["provider_registrations row decode".into()],
            )
        })?;
    for row in rows {
        let (
            registration_id,
            handle,
            enabled_raw,
            config_revision_raw,
            executable,
            argv_json,
            working_directory,
            timeout_seconds_raw,
            created_at,
            updated_at,
        ) = row.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["provider_registrations row".into()],
            )
        })?;
        let context = CorruptionContext::new().with("registration_id", &registration_id);
        let mut row_findings = Vec::new();
        let enabled = match decode_provider_enabled(enabled_raw, context.clone()) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                row_findings.push(diagnostic);
                None
            }
        };
        let config_revision =
            match decode_version_i64("config_revision", config_revision_raw, context.clone()) {
                Ok(value) => Some(value),
                Err(diagnostic) => {
                    row_findings.push(diagnostic);
                    None
                }
            };
        let timeout_seconds = match decode_positive_bounded_i64(
            "timeout_seconds",
            timeout_seconds_raw,
            context.clone(),
        ) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                row_findings.push(diagnostic);
                None
            }
        };
        if let (Some(config_revision), Some(timeout_seconds), Some(enabled)) =
            (config_revision, timeout_seconds, enabled)
        {
            let record = ProviderRegistrationRecord {
                registration_id,
                handle,
                enabled,
                config_revision,
                executable,
                argv_json,
                working_directory,
                timeout_seconds,
                created_at,
                updated_at,
            };
            if let Err(error) = mapping::registration_from_record(&record) {
                row_findings.push(
                    classify_mapping_error(
                        &error,
                        CorruptionPhase::Read,
                        CorruptionContext::new().with("registration_id", &record.registration_id),
                        vec!["provider_registrations mapping".into()],
                    )
                    .primary()
                    .cloned()
                    .expect("mapping classification yields diagnostic"),
                );
            }
            if let Err(error) = mapping::config_from_record(&record) {
                row_findings.push(
                    classify_mapping_error(
                        &error,
                        CorruptionPhase::Read,
                        CorruptionContext::new().with("registration_id", &record.registration_id),
                        vec!["provider_registrations config mapping".into()],
                    )
                    .primary()
                    .cloned()
                    .expect("mapping classification yields diagnostic"),
                );
            }
        }
        findings.extend(row_findings);
    }
    Ok(findings)
}

fn validate_runs(conn: &Connection) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn
        .prepare(
            "SELECT run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                    workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                    canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
             FROM runs
             ORDER BY run_id ASC",
        )
        .map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Read, vec!["runs scan".into()])
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
            ))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs row decode".into()],
            )
        })?;

    let mut registration_ids = BTreeMap::new();
    {
        let mut statement = conn
            .prepare("SELECT registration_id FROM provider_registrations")
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["provider_registrations index".into()],
                )
            })?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["provider_registrations index rows".into()],
                )
            })?;
        for id in rows {
            registration_ids.insert(
                id.map_err(|source| {
                    classify_sqlite_error(
                        &source,
                        CorruptionPhase::Read,
                        vec!["provider_registrations index id".into()],
                    )
                })?,
                (),
            );
        }
    }

    for row in rows {
        let (
            run_id,
            registration_id,
            config_revision_at_create_raw,
            current_state,
            lifecycle,
            workflow_state_version_raw,
            lifecycle_version_raw,
            label_version_raw,
            label,
            graph_revision,
            canonical_graph_version_raw,
            graph_canonical_projection_json,
            inputs_json,
            created_at,
        ) = row.map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Read, vec!["runs row".into()])
        })?;
        let context = CorruptionContext::new().with("run_id", &run_id);
        let mut row_findings = Vec::new();
        if !registration_ids.contains_key(&registration_id) {
            row_findings.push(CorruptionDiagnostic::new(
                CorruptionKind::RegistrationReferentialIntegrity,
                codes::REGISTRATION_REFERENTIAL,
                format!("run {run_id} references missing registration {registration_id}"),
                context.clone().with("registration_id", &registration_id),
            ));
        }
        let config_revision_at_create = match decode_version_i64(
            "config_revision_at_create",
            config_revision_at_create_raw,
            context.clone(),
        ) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                row_findings.push(diagnostic);
                None
            }
        };
        let workflow_state_version = match decode_version_i64(
            "workflow_state_version",
            workflow_state_version_raw,
            context.clone(),
        ) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                row_findings.push(diagnostic);
                None
            }
        };
        let lifecycle_version =
            match decode_version_i64("lifecycle_version", lifecycle_version_raw, context.clone()) {
                Ok(value) => Some(value),
                Err(diagnostic) => {
                    row_findings.push(diagnostic);
                    None
                }
            };
        let label_version =
            match decode_version_i64("label_version", label_version_raw, context.clone()) {
                Ok(value) => Some(value),
                Err(diagnostic) => {
                    row_findings.push(diagnostic);
                    None
                }
            };
        let canonical_graph_version = match decode_version_i64(
            "canonical_graph_version",
            canonical_graph_version_raw,
            context.clone(),
        ) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                row_findings.push(diagnostic);
                None
            }
        };
        if let (
            Some(config_revision_at_create),
            Some(workflow_state_version),
            Some(lifecycle_version),
            Some(label_version),
            Some(canonical_graph_version),
        ) = (
            config_revision_at_create,
            workflow_state_version,
            lifecycle_version,
            label_version,
            canonical_graph_version,
        ) {
            let record = RunRecord {
                run_id,
                registration_id,
                config_revision_at_create,
                current_state,
                lifecycle,
                workflow_state_version,
                lifecycle_version,
                label_version,
                label,
                graph_revision,
                canonical_graph_version,
                graph_canonical_projection_json,
                inputs_json,
                created_at,
            };
            if let Err(error) = mapping::run_from_record(&record) {
                row_findings.push(
                    classify_mapping_error(
                        &error,
                        CorruptionPhase::Read,
                        CorruptionContext::new().with("run_id", &record.run_id),
                        vec!["runs mapping".into()],
                    )
                    .primary()
                    .cloned()
                    .expect("mapping classification yields diagnostic"),
                );
            }
        }
        findings.extend(row_findings);
    }
    Ok(findings)
}

fn validate_evidence_rows(
    conn: &Connection,
    run_ids: &BTreeSet<String>,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn
        .prepare(
            "SELECT run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
             FROM evidence
             ORDER BY run_id ASC, evidence_id ASC",
        )
        .map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Read, vec!["evidence scan".into()])
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok(EvidenceRecordRow {
                run_id: row.get(0)?,
                evidence_id: row.get(1)?,
                kind: row.get(2)?,
                locator: row.get(3)?,
                digest: row.get(4)?,
                media_type: row.get(5)?,
                metadata_json: row.get(6)?,
                source: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["evidence row decode".into()],
            )
        })?;
    for row in rows {
        let record = row.map_err(|source| {
            classify_sqlite_error(&source, CorruptionPhase::Read, vec!["evidence row".into()])
        })?;
        let context = CorruptionContext::new()
            .with("run_id", &record.run_id)
            .with("evidence_id", &record.evidence_id);
        if !run_ids.contains(&record.run_id) {
            findings.push(CorruptionDiagnostic::new(
                CorruptionKind::EvidenceAssociationIntegrity,
                codes::EVIDENCE_ASSOCIATION,
                format!(
                    "evidence row references missing run ({}, {})",
                    record.run_id, record.evidence_id
                ),
                context,
            ));
            continue;
        }
        if let Err(error) = mapping::evidence_from_record(&record) {
            findings.push(
                classify_mapping_error(
                    &error,
                    CorruptionPhase::Read,
                    context,
                    vec!["evidence mapping".into()],
                )
                .primary()
                .cloned()
                .expect("mapping classification yields diagnostic"),
            );
        }
    }
    Ok(findings)
}

fn validate_orphan_referential_rows(
    conn: &Connection,
    run_ids: &BTreeSet<String>,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();

    let mut journal_statement = conn
        .prepare("SELECT run_id, sequence FROM journal_entries ORDER BY run_id ASC, sequence ASC")
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["journal_entries orphan scan".into()],
            )
        })?;
    let journal_rows = journal_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["journal_entries orphan row decode".into()],
            )
        })?;
    for row in journal_rows {
        let (run_id, sequence_raw) = row.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["journal_entries orphan row".into()],
            )
        })?;
        if !run_ids.contains(&run_id) {
            findings.push(CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!("orphan journal entry references missing run ({run_id}, {sequence_raw})"),
                CorruptionContext::new()
                    .with("run_id", &run_id)
                    .with("sequence", sequence_raw),
            ));
        }
    }

    let mut allocator_statement = conn
        .prepare("SELECT run_id, next_sequence FROM run_journal_sequences ORDER BY run_id ASC")
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["run_journal_sequences orphan scan".into()],
            )
        })?;
    let allocator_rows = allocator_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["run_journal_sequences orphan row decode".into()],
            )
        })?;
    for row in allocator_rows {
        let (run_id, next_sequence_raw) = row.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["run_journal_sequences orphan row".into()],
            )
        })?;
        if !run_ids.contains(&run_id) {
            findings.push(CorruptionDiagnostic::new(
                CorruptionKind::JournalSequenceAllocatorMismatch,
                codes::JOURNAL_SEQUENCE_ALLOCATOR,
                format!(
                    "orphan sequence allocator references missing run ({run_id}, {next_sequence_raw})"
                ),
                CorruptionContext::new()
                    .with("run_id", &run_id)
                    .with("next_sequence", next_sequence_raw),
            ));
        }
    }

    Ok(findings)
}

fn validate_foreign_key_violations(
    conn: &Connection,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn.prepare("PRAGMA foreign_key_check").map_err(|source| {
        classify_sqlite_error(
            &source,
            CorruptionPhase::Read,
            vec!["foreign_key_check".into()],
        )
    })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["foreign_key_check row decode".into()],
            )
        })?;
    for row in rows {
        let (table, rowid, parent, fkid) = row.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["foreign_key_check row".into()],
            )
        })?;
        findings.push(classify_foreign_key_violation(&table, rowid, &parent, fkid));
    }
    Ok(findings)
}

fn classify_foreign_key_violation(
    table: &str,
    rowid: i64,
    parent: &str,
    fkid: i64,
) -> CorruptionDiagnostic {
    let context = CorruptionContext::new()
        .with("table", table)
        .with("rowid", rowid)
        .with("parent_table", parent)
        .with("foreign_key_index", fkid);
    match table {
        "runs" => CorruptionDiagnostic::new(
            CorruptionKind::RegistrationReferentialIntegrity,
            codes::REGISTRATION_REFERENTIAL,
            format!("foreign key violation in runs row {rowid}: missing parent in {parent}"),
            context,
        ),
        "journal_entries" => CorruptionDiagnostic::new(
            CorruptionKind::JournalPayloadInconsistent,
            codes::JOURNAL_PAYLOAD_INCONSISTENT,
            format!(
                "foreign key violation in journal_entries row {rowid}: missing parent in {parent}"
            ),
            context,
        ),
        "run_journal_sequences" => CorruptionDiagnostic::new(
            CorruptionKind::JournalSequenceAllocatorMismatch,
            codes::JOURNAL_SEQUENCE_ALLOCATOR,
            format!(
                "foreign key violation in run_journal_sequences row {rowid}: missing parent in {parent}"
            ),
            context,
        ),
        "evidence" | "evidence_associations" => CorruptionDiagnostic::new(
            CorruptionKind::EvidenceAssociationIntegrity,
            codes::EVIDENCE_ASSOCIATION,
            format!("foreign key violation in {table} row {rowid}: missing parent in {parent}"),
            context,
        ),
        other => CorruptionDiagnostic::new(
            CorruptionKind::RegistrationReferentialIntegrity,
            codes::REGISTRATION_REFERENTIAL,
            format!("foreign key violation in {other} row {rowid}: missing parent in {parent}"),
            context,
        ),
    }
}

fn validate_journal_integrity(
    conn: &Connection,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn
        .prepare("SELECT run_id FROM runs ORDER BY run_id ASC")
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs journal invariant scan".into()],
            )
        })?;
    let run_ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs journal invariant scan".into()],
            )
        })?;
    for run_id in run_ids {
        let run_id = run_id.map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["runs journal invariant scan".into()],
            )
        })?;
        if let Err(error) = validate_journal_sequences_for_run(conn, &run_id) {
            if let Some(diagnostic) = error.primary() {
                findings.push(diagnostic.clone());
            } else {
                return Err(error);
            }
        }
        if let Err(error) = validate_journal_semantics_for_run(conn, &run_id) {
            if let Some(diagnostic) = error.primary() {
                findings.push(diagnostic.clone());
            } else {
                return Err(error);
            }
        }
    }
    Ok(findings)
}

fn validate_journal_semantics_for_run(
    conn: &Connection,
    run_id: &str,
) -> Result<(), CorruptionError> {
    let records = load_journal_records(conn, run_id)?;
    for record in &records {
        if let Err(error) = validate_journal_record_semantics(conn, record) {
            return Err(classify_history_read_error(
                &error,
                run_id,
                vec!["journal payload decode".into()],
            ));
        }
    }
    Ok(())
}

fn classify_history_read_error(
    error: &HistoryReadError,
    run_id: &str,
    source_chain: Vec<String>,
) -> CorruptionError {
    let context = CorruptionContext::new().with("run_id", run_id);
    match error {
        HistoryReadError::Corrupt { message } => CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!("stored journal data is corrupt: {message}"),
                context,
            ),
            source_chain,
        ),
        HistoryReadError::NotFound { .. } => CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!("journal payload decode failed: {error}"),
                context,
            ),
            source_chain,
        ),
        HistoryReadError::Persistence(persistence) => classify_persistence_error(
            persistence,
            CorruptionPhase::Read,
            push_chain(source_chain, "journal payload decode".into()),
        ),
        HistoryReadError::Page(page) => CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!("journal payload paging failure: {page}"),
                context,
            ),
            source_chain,
        ),
        HistoryReadError::Bound(bound) => CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::RowBoundedSemanticValue,
                codes::ROW_BOUNDED_VALUE,
                format!("journal payload bound violation: {bound}"),
                context,
            ),
            source_chain,
        ),
    }
}

fn validate_evidence_associations(
    conn: &Connection,
) -> Result<Vec<CorruptionDiagnostic>, CorruptionError> {
    let mut findings = Vec::new();
    let mut statement = conn
        .prepare(
            "SELECT ea.run_id, ea.journal_sequence, ea.evidence_id, ea.event_id, ea.gate_id
             FROM evidence_associations ea
             ORDER BY ea.run_id ASC, ea.journal_sequence ASC, ea.evidence_id ASC",
        )
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["evidence_associations scan".into()],
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["evidence_associations row decode".into()],
            )
        })?;

    for row in rows {
        let (run_id, journal_sequence, evidence_id, _event_id, _gate_id) =
            row.map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["evidence_associations row".into()],
                )
            })?;

        let context = CorruptionContext::new()
            .with("run_id", &run_id)
            .with("evidence_id", &evidence_id);
        if let Err(diagnostic) =
            decode_version_i64("journal_sequence", journal_sequence, context.clone())
        {
            findings.push(diagnostic);
            continue;
        }

        let journal_exists: bool = conn
            .query_row(
                "SELECT 1 FROM journal_entries WHERE run_id = ?1 AND sequence = ?2",
                rusqlite::params![run_id, journal_sequence],
                |_| Ok(true),
            )
            .optional()
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["journal_entries association lookup".into()],
                )
            })?
            .unwrap_or(false);
        if !journal_exists {
            findings.push(CorruptionDiagnostic::new(
                CorruptionKind::EvidenceAssociationIntegrity,
                codes::EVIDENCE_ASSOCIATION,
                format!(
                    "evidence association references missing journal entry ({run_id}, {journal_sequence})"
                ),
                CorruptionContext::new()
                    .with("run_id", &run_id)
                    .with("journal_sequence", journal_sequence)
                    .with("evidence_id", &evidence_id),
            ));
        }

        let evidence_exists: bool = conn
            .query_row(
                "SELECT 1 FROM evidence WHERE run_id = ?1 AND evidence_id = ?2",
                rusqlite::params![run_id, evidence_id],
                |_| Ok(true),
            )
            .optional()
            .map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["evidence association lookup".into()],
                )
            })?
            .unwrap_or(false);
        if !evidence_exists {
            findings.push(CorruptionDiagnostic::new(
                CorruptionKind::EvidenceAssociationIntegrity,
                codes::EVIDENCE_ASSOCIATION,
                format!(
                    "evidence association references missing evidence ({run_id}, {evidence_id})"
                ),
                CorruptionContext::new()
                    .with("run_id", &run_id)
                    .with("journal_sequence", journal_sequence)
                    .with("evidence_id", &evidence_id),
            ));
        }
    }
    Ok(findings)
}

fn validate_creation_journal_entry(
    record: &JournalRecord,
    run_id: &str,
) -> Result<(), CorruptionError> {
    let payload: Value = serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
        CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!("run {run_id} sequence 1 creation payload is malformed: {error}"),
                CorruptionContext::new()
                    .with("run_id", run_id)
                    .with("sequence", 1),
            ),
            vec!["journal_entries creation entry".into()],
        )
    })?;
    let entry_kind = payload
        .get("entry_kind")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    let operation = payload
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    if entry_kind != "run.created" || operation != "run.create" {
        return Err(CorruptionError::single(
            CorruptionPhase::Read,
            CorruptionDiagnostic::new(
                CorruptionKind::JournalPayloadInconsistent,
                codes::JOURNAL_PAYLOAD_INCONSISTENT,
                format!(
                    "run {run_id} sequence 1 must be creation entry (operation run.create, entry_kind run.created)"
                ),
                CorruptionContext::new()
                    .with("run_id", run_id)
                    .with("sequence", 1)
                    .with("observed_entry_kind", entry_kind)
                    .with("observed_operation", operation),
            ),
            vec!["journal_entries creation entry".into()],
        ));
    }
    Ok(())
}

fn load_journal_records(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<JournalRecord>, CorruptionError> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, sequence, outcome, encoded_payload_json
             FROM journal_entries
             WHERE run_id = ?1
             ORDER BY sequence ASC",
        )
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["journal_entries load".into()],
            )
        })?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Read,
                vec!["journal_entries row decode".into()],
            )
        })?;
    let mut records = Vec::new();
    for row in rows {
        let (run_id_value, sequence_raw, outcome, encoded_payload_json) =
            row.map_err(|source| {
                classify_sqlite_error(
                    &source,
                    CorruptionPhase::Read,
                    vec!["journal_entries row".into()],
                )
            })?;
        let sequence = decode_version_i64(
            "sequence",
            sequence_raw,
            CorruptionContext::new().with("run_id", run_id),
        )
        .map_err(|diagnostic| {
            CorruptionError::single(
                CorruptionPhase::Read,
                diagnostic,
                vec!["journal_entries row decode".into()],
            )
        })?;
        records.push(JournalRecord {
            run_id: run_id_value,
            sequence,
            outcome,
            encoded_payload_json,
        });
    }
    Ok(records)
}

fn verify_integrity_key_row(conn: &Connection) -> Result<(), CorruptionError> {
    let length: Result<i64, SqliteError> = conn.query_row(
        &format!("SELECT length(value) FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
        [INTEGRITY_KEY_ROW_KEY],
        |row| row.get(0),
    );
    match length {
        Ok(len) if len == INTEGRITY_KEY_BYTE_LENGTH as i64 => Ok(()),
        Ok(len) => Err(classify_persistence_error(
            &PersistenceError::MetadataKeyInvalidLength {
                key: INTEGRITY_KEY_ROW_KEY,
                expected: INTEGRITY_KEY_BYTE_LENGTH,
                actual: len as usize,
            },
            CorruptionPhase::Open,
            vec!["integration_metadata integrity_key".into()],
        )),
        Err(SqliteError::QueryReturnedNoRows) => Err(classify_persistence_error(
            &PersistenceError::MetadataKeyMissing {
                key: INTEGRITY_KEY_ROW_KEY,
            },
            CorruptionPhase::Open,
            vec!["integration_metadata integrity_key".into()],
        )),
        Err(source) => Err(classify_sqlite_error(
            &source,
            CorruptionPhase::Open,
            vec!["integration_metadata read".into()],
        )),
    }
}

pub fn integrity_key_hash(conn: &Connection) -> Result<String, CorruptionError> {
    let bytes = read_integrity_key_bytes(conn)?;
    if bytes.len() != INTEGRITY_KEY_BYTE_LENGTH {
        return Err(classify_persistence_error(
            &PersistenceError::MetadataKeyInvalidLength {
                key: INTEGRITY_KEY_ROW_KEY,
                expected: INTEGRITY_KEY_BYTE_LENGTH,
                actual: bytes.len(),
            },
            CorruptionPhase::Read,
            vec!["integration_metadata integrity_key".into()],
        ));
    }
    Ok(sha256_label(&bytes))
}

fn integrity_key_fingerprint(conn: &Connection) -> Result<String, CorruptionError> {
    Ok(sha256_label(&read_integrity_key_bytes(conn)?))
}

fn read_integrity_key_bytes(conn: &Connection) -> Result<Vec<u8>, CorruptionError> {
    conn.query_row(
        &format!("SELECT value FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
        [INTEGRITY_KEY_ROW_KEY],
        |row| row.get(0),
    )
    .map_err(|source| match source {
        SqliteError::QueryReturnedNoRows => classify_persistence_error(
            &PersistenceError::MetadataKeyMissing {
                key: INTEGRITY_KEY_ROW_KEY,
            },
            CorruptionPhase::Read,
            vec!["integration_metadata integrity_key".into()],
        ),
        other => classify_sqlite_error(
            &other,
            CorruptionPhase::Read,
            vec!["integration_metadata integrity_key".into()],
        ),
    })
}

fn read_user_version(conn: &Connection) -> Result<u32, CorruptionError> {
    let version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .map_err(|source| {
            classify_sqlite_error(
                &source,
                CorruptionPhase::Open,
                vec!["pragma user_version".into()],
            )
        })?;
    if version < 0 {
        return Err(classify_persistence_error(
            &PersistenceError::InvalidUserVersion { observed: version },
            CorruptionPhase::Open,
            vec!["pragma user_version".into()],
        ));
    }
    Ok(version as u32)
}

fn inventory_all_tables(
    conn: &Connection,
) -> Result<BTreeMap<String, TableInventory>, CorruptionError> {
    let mut tables = BTreeMap::new();
    for (name, query) in AUTHORITATIVE_TABLE_QUERIES {
        tables.insert(name.to_string(), inventory_table(conn, name, query)?);
    }
    Ok(tables)
}

const AUTHORITATIVE_TABLE_QUERIES: &[(&str, &str)] = &[
    (
        "integration_metadata",
        "SELECT key, length(value) FROM integration_metadata ORDER BY key ASC",
    ),
    (
        "provider_registrations",
        "SELECT registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
         FROM provider_registrations ORDER BY registration_id ASC",
    ),
    (
        "runs",
        "SELECT run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
         FROM runs ORDER BY run_id ASC",
    ),
    (
        "run_journal_sequences",
        "SELECT run_id, next_sequence FROM run_journal_sequences ORDER BY run_id ASC",
    ),
    (
        "evidence",
        "SELECT run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
         FROM evidence ORDER BY run_id ASC, evidence_id ASC",
    ),
    (
        "journal_entries",
        "SELECT run_id, sequence, outcome, encoded_payload_json
         FROM journal_entries ORDER BY run_id ASC, sequence ASC",
    ),
    (
        "evidence_associations",
        "SELECT run_id, journal_sequence, evidence_id, event_id, gate_id
         FROM evidence_associations ORDER BY run_id ASC, journal_sequence ASC, evidence_id ASC",
    ),
];

fn inventory_table(
    conn: &Connection,
    table: &str,
    query: &str,
) -> Result<TableInventory, CorruptionError> {
    let mut statement = conn.prepare(query).map_err(|source| {
        classify_sqlite_error(
            &source,
            CorruptionPhase::Read,
            vec![format!("{table} inventory prepare")],
        )
    })?;
    let mut rows = Vec::new();
    let mapped = statement.query([]).map_err(|source| {
        classify_sqlite_error(
            &source,
            CorruptionPhase::Read,
            vec![format!("{table} inventory query")],
        )
    })?;
    let mut mapped = mapped;
    while let Some(row) = mapped.next().map_err(|source| {
        classify_sqlite_error(
            &source,
            CorruptionPhase::Read,
            vec![format!("{table} inventory row")],
        )
    })? {
        let column_count = row.as_ref().column_count();
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            let value = row
                .get::<_, rusqlite::types::Value>(index)
                .map_err(|source| {
                    classify_sqlite_error(
                        &source,
                        CorruptionPhase::Read,
                        vec![format!("{table} inventory decode")],
                    )
                })?;
            values.push(sqlite_value_to_json(value));
        }
        rows.push(Value::Array(values));
    }
    let payload = json!({ "rows": rows });
    let digest = sha256_label(canonical_json_string(&payload).as_bytes());
    Ok(TableInventory {
        row_count: u64::try_from(rows.len()).unwrap_or(u64::MAX),
        content_digest: digest,
    })
}

fn sqlite_value_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => json!(v),
        rusqlite::types::Value::Real(v) => json!(v),
        rusqlite::types::Value::Text(v) => Value::String(v),
        rusqlite::types::Value::Blob(v) => Value::String(format!("blob:sha256:{}", sha256_hex(&v))),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&canonicalize_json(value)).expect("canonical json")
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .iter()
                .map(|(key, value)| (key.as_str(), canonicalize_json(value)))
                .collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

fn canonical_context_json(context: &CorruptionContext) -> String {
    canonical_json_string(&json!(context.values))
}

fn is_malformed_sqlite_header(path: &Path) -> bool {
    inspect_file_header(path).is_err()
}

fn push_chain(mut chain: Vec<String>, item: String) -> Vec<String> {
    chain.push(item);
    chain
}

fn sanitize_source_chain(chain: Vec<String>) -> Vec<String> {
    chain
        .into_iter()
        .map(|entry| redact_secrets(&entry))
        .collect()
}

fn redact_secrets(message: &str) -> String {
    if message.contains("integrity_key") && message.len() > 64 {
        return "integration metadata read failed".into();
    }
    if message.contains("\"mac\"") || message.contains("HMAC") {
        return "integrity MAC verification failed".into();
    }
    message.to_string()
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn source_chain_redacts_mac_like_content() {
        let chain = sanitize_source_chain(vec![
            "cursor verify".into(),
            "HMAC tag mismatch for payload".into(),
        ]);
        assert_eq!(chain[1], "integrity MAC verification failed");
    }

    #[test]
    fn classify_mapping_error_preserves_field_context() {
        let error = MappingError::UnsupportedEnum {
            field: "lifecycle",
            value: "paused".into(),
        };
        let classified = classify_mapping_error(
            &error,
            CorruptionPhase::Read,
            CorruptionContext::new().with("run_id", "run-1"),
            vec!["runs mapping".into()],
        );
        let diagnostic = classified.primary().unwrap();
        assert_eq!(diagnostic.kind, CorruptionKind::RowUnsupportedEnum);
        assert_eq!(
            diagnostic.context.as_map().get("field").map(String::as_str),
            Some("lifecycle")
        );
    }
}
