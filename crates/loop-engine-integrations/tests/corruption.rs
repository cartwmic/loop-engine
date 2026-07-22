//! Persistence corruption diagnostics integration tests (T117).

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::PageRequest;
use loop_engine_core::capabilities::audit_export::{AuditExporter, ExportTarget};
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::bounded::{
    COLLECTION_PAGE_DATA_BUDGET_BYTES, COLLECTION_PAGE_DEFAULT_COUNT,
};
use loop_engine_core::model::ids::RunId;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_integrations::export::{ExportError, SqliteAuditExporter};
use loop_engine_integrations::persistence::{
    CorruptionDiagnostic, CorruptionError, CorruptionKind, CorruptionPhase,
    LogicalAuthoritySnapshot, SqliteHistoryReads, SqliteRunReads, SqliteStore,
    classify_persistence_error, inspect_file_header, inspect_logical_store, inspect_open_readonly,
    integrity_key_hash, physical_fixture_sha256,
};
use rusqlite::Connection;
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/corruption");
const REGISTRATION_ID: &str = "019f0000-0000-7000-8000-000000000001";
const RUN_ID: &str = "019f0000-0000-7000-8000-000000000101";
const MISSING_RUN_ID: &str = "019f0000-0000-7000-8000-000000000999";

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    observed_at: ObservedAt,
}

impl TimeSource for FixedClock {
    type Error = Infallible;

    fn now(&self) -> Result<ObservedAt, Self::Error> {
        Ok(self.observed_at)
    }
}

struct LiveHarness {
    _root: TempDir,
    db_path: PathBuf,
}

impl LiveHarness {
    fn seeded() -> Self {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        SqliteStore::open(&db_path).unwrap();
        apply_sql_file(&Connection::open(&db_path).unwrap(), "seed_baseline.sql");
        Self {
            _root: root,
            db_path,
        }
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.db_path).unwrap()
    }

    fn logical_snapshot(&self) -> LogicalAuthoritySnapshot {
        LogicalAuthoritySnapshot::capture(&self.connection()).unwrap()
    }

    fn assert_authority_unchanged(&self, before: &LogicalAuthoritySnapshot) {
        let after = self.logical_snapshot();
        LogicalAuthoritySnapshot::assert_unchanged(before, &after).expect("authority unchanged");
    }

    fn with_constraints_bypassed<F>(&self, apply: F)
    where
        F: FnOnce(&Connection),
    {
        let conn = self.connection();
        conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        apply(&conn);
        conn.execute_batch("PRAGMA ignore_check_constraints = OFF; PRAGMA foreign_keys = ON;")
            .unwrap();
    }
}

fn apply_sql_file(conn: &Connection, name: &str) {
    let sql = fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
    conn.execute_batch(&sql).unwrap();
}

fn insert_annotation_rows(conn: &Connection, start: u64, end: u64) {
    let state = r#"{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1}"#;
    for seq in start..=end {
        let payload = format!(
            r#"{{"journal_schema_version":1,"sequence":{seq},"run_id":"{RUN_ID}","ts":"2026-07-17T13:00:00.000Z","operation":"run.annotate","request_id":"req-annotate-{seq:03}","entry_kind":"annotation","outcome":"completed","reason":null,"state_before":{state},"state_after":{state},"note":"page filler {seq}"}}"#,
        );
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json) VALUES (?1, ?2, 'completed', ?3)",
            rusqlite::params![RUN_ID, i64::try_from(seq).unwrap(), payload],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE run_journal_sequences SET next_sequence = ?1 WHERE run_id = ?2",
        rusqlite::params![i64::try_from(end + 1).unwrap(), RUN_ID],
    )
    .unwrap();
}

fn copy_immutable_fixture(name: &str) -> (TempDir, PathBuf, String) {
    let root = TempDir::new().unwrap();
    let source = LiveHarness::seeded();
    let dest = root.path().join(name);
    fs::copy(&source.db_path, &dest).unwrap();
    let physical_hash = physical_fixture_sha256(&dest).unwrap();
    (root, dest, physical_hash)
}

fn corrupt_header_bytes(path: &Path) -> String {
    let mut bytes = fs::read(path).unwrap();
    bytes[0] = b'X';
    fs::write(path, bytes).unwrap();
    physical_fixture_sha256(path).unwrap()
}

fn expect_corruption<F>(phase: CorruptionPhase, kind: CorruptionKind, mut f: F)
where
    F: FnMut() -> Result<(), CorruptionError>,
{
    let error = f().expect_err("expected corruption error");
    assert_eq!(error.phase, phase);
    assert!(
        error.diagnostics.iter().any(|d| d.kind == kind),
        "expected {kind:?}, got {:?}",
        error.diagnostics
    );
    assert!(
        error
            .source_chain()
            .iter()
            .all(|entry| !entry.contains("\"mac\"")),
        "source chain must not expose raw MAC material"
    );
}

fn diagnostic_for<'a>(
    error: &'a CorruptionError,
    kind: CorruptionKind,
    field: Option<&str>,
) -> &'a CorruptionDiagnostic {
    error
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.kind == kind
                && field.is_none_or(|expected| {
                    diagnostic.context.as_map().get("field") == Some(&expected.to_string())
                })
        })
        .unwrap_or_else(|| {
            panic!(
                "expected {kind:?} field={field:?}, got {:?}",
                error.diagnostics
            )
        })
}

fn expect_corruption_deterministic<F>(
    phase: CorruptionPhase,
    kind: CorruptionKind,
    field: Option<&str>,
    code: &str,
    mut inspect: F,
) where
    F: FnMut() -> Result<(), CorruptionError>,
{
    let error = inspect().expect_err("expected corruption error");
    assert_eq!(error.phase, phase);
    let diagnostic = diagnostic_for(&error, kind, field);
    assert_eq!(diagnostic.code, code);
    assert!(
        error
            .source_chain()
            .iter()
            .all(|entry| !entry.contains("\"mac\"")),
        "source chain must not expose raw MAC material"
    );
}

#[test]
fn corrupt_header_fixture_classified_without_logical_mutation() {
    let (_root, path, physical_before) = copy_immutable_fixture("header-corrupt.db");
    let physical_after = corrupt_header_bytes(&path);
    assert_ne!(
        physical_before, physical_after,
        "intentional header corruption changes bytes"
    );

    expect_corruption(
        CorruptionPhase::Open,
        CorruptionKind::MalformedDatabaseHeader,
        || inspect_file_header(&path),
    );

    let readonly_error = inspect_open_readonly(&path).expect_err("readonly inspect should fail");
    assert_eq!(
        readonly_error.primary().unwrap().kind,
        CorruptionKind::MalformedDatabaseHeader
    );
}

#[test]
fn not_a_database_bytes_classified_on_readonly_open() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("not-a-db");
    fs::write(&path, b"not a sqlite database file").unwrap();
    let physical_hash = physical_fixture_sha256(&path).unwrap();

    expect_corruption(
        CorruptionPhase::Open,
        CorruptionKind::MalformedDatabaseHeader,
        || inspect_file_header(&path),
    );

    let error = inspect_open_readonly(&path).expect_err("readonly inspect should fail");
    assert!(
        matches!(
            error.primary().unwrap().kind,
            CorruptionKind::MalformedDatabaseHeader | CorruptionKind::NotADatabase
        ),
        "unexpected kind: {:?}",
        error.primary().unwrap().kind
    );
    assert_eq!(physical_fixture_sha256(&path).unwrap(), physical_hash);
}

#[test]
fn future_schema_detected_without_mutating_live_store() {
    let harness = LiveHarness::seeded();
    {
        let conn = harness.connection();
        conn.execute_batch("PRAGMA user_version = 99").unwrap();
    }
    let before = harness.logical_snapshot();

    let error = SqliteStore::open(&harness.db_path).expect_err("future schema rejected");
    let classified = classify_persistence_error(
        &error,
        CorruptionPhase::Migration,
        vec!["startup migration".into()],
    );
    assert_eq!(
        classified.primary().unwrap().kind,
        CorruptionKind::SchemaFutureVersion
    );
    assert!(classified.phase.is_pre_dispatch());

    let conn = harness.connection();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
            .unwrap(),
        99
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn integrity_key_truncation_detected_at_inspection() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "integrity_key_truncated.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::IntegrityKeyInvalidLength,
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn graph_digest_mismatch_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "graph_digest_mismatch.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::RowGraphDigestMismatch,
        || inspect_logical_store(&harness.connection()),
    );

    let reads = SqliteRunReads::new(&harness.db_path);
    let run_id = RunId::parse(RUN_ID).unwrap();
    assert!(reads.get(&run_id).is_err());

    harness.assert_authority_unchanged(&before);
}

#[test]
fn unsupported_lifecycle_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "unsupported_lifecycle.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::RowUnsupportedEnum,
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn orphan_registration_binding_detected() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "orphan_registration_binding.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::RegistrationReferentialIntegrity,
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_sequence_gap_detected() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_sequence_gap.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalSequenceDiscontinuity,
        || inspect_logical_store(&harness.connection()),
    );

    let history = SqliteHistoryReads::new(&harness.db_path);
    let run_id = RunId::parse(RUN_ID).unwrap();
    assert!(
        history
            .history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .is_err()
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_missing_allocator_detected_for_authoritative_run() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_missing_allocator.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalSequenceAllocatorMismatch,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_deleted_creation_detected_for_authoritative_run() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_deleted_creation.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_wrong_creation_kind_detected_for_authoritative_run() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_wrong_creation_kind.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn orphan_evidence_association_detected() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "orphan_evidence_association.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::EvidenceAssociationIntegrity,
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn failed_export_leaves_logical_authority_unchanged() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_sequence_gap.sql");
    let before = harness.logical_snapshot();

    let exporter = SqliteAuditExporter::with_clock(
        harness.db_path.clone(),
        FixedClock {
            observed_at: ObservedAt::parse("2026-07-17T15:00:00.000Z").unwrap(),
        },
    );
    let export_dir = harness._root.path().join("export-fail");
    fs::create_dir(&export_dir).unwrap();
    let target = ExportTarget::parse(export_dir.to_str().unwrap()).unwrap();
    let run_id = RunId::parse(RUN_ID).unwrap();

    let error = exporter
        .export_consistent(&run_id, &target)
        .expect_err("export must fail on corrupt journal");
    assert!(matches!(error, ExportError::PersistenceFailed { .. }));

    harness.assert_authority_unchanged(&before);
}

#[test]
fn logical_snapshot_tracks_user_version_integrity_hash_and_inventory() {
    let harness = LiveHarness::seeded();
    let snapshot = harness.logical_snapshot();
    assert_eq!(snapshot.user_version, 1);
    assert!(snapshot.integrity_key_hash.starts_with("sha256:"));
    assert_eq!(snapshot.tables.len(), 7);
    for table in [
        "integration_metadata",
        "provider_registrations",
        "runs",
        "run_journal_sequences",
        "evidence",
        "journal_entries",
        "evidence_associations",
    ] {
        assert!(snapshot.tables.contains_key(table), "missing table {table}");
    }
}

#[test]
fn diagnostics_render_for_cli_phase_mapping() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "graph_digest_mismatch.sql");
    let error = inspect_logical_store(&harness.connection()).expect_err("corruption");
    assert_eq!(error.phase, CorruptionPhase::Read);
    assert!(!error.phase.is_pre_dispatch());
    assert_eq!(error.reason_code(), "persistence.failed");
    assert!(!error.source_chain().is_empty());

    let core = error.to_core_diagnostics().unwrap();
    assert!(!core.is_empty());
    assert!(core[0].code().starts_with("persistence.corruption."));
}

#[test]
fn journal_missing_required_field_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_missing_ts.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_invalid_state_fact_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_invalid_state_fact.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_self_correction_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_self_correction.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_future_correction_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_future_correction.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_unsupported_gate_result_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    apply_sql_file(&harness.connection(), "journal_unsupported_gate_result.sql");
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn journal_tail_corruption_detected_via_paged_history_reads() {
    let harness = LiveHarness::seeded();
    let conn = harness.connection();
    let tail_sequence = u64::from(COLLECTION_PAGE_DEFAULT_COUNT) + 2;
    insert_annotation_rows(&conn, 2, tail_sequence);
    conn.execute(
        "UPDATE journal_entries SET encoded_payload_json = ?1 WHERE run_id = ?2 AND sequence = ?3",
        rusqlite::params![
            format!(
                r#"{{"journal_schema_version":1,"sequence":{tail_sequence},"run_id":"{RUN_ID}","operation":"run.annotate","request_id":"req-annotate-tail","entry_kind":"annotation","outcome":"completed","reason":null,"state_before":{{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1}},"state_after":{{"state":"draft","lifecycle":"active","workflow_state_version":1,"lifecycle_version":1}},"note":"missing ts"}}"#,
            ),
            RUN_ID,
            i64::try_from(tail_sequence).unwrap(),
        ],
    )
    .unwrap();
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::JournalPayloadInconsistent,
        || inspect_logical_store(&harness.connection()),
    );

    let history = SqliteHistoryReads::new(&harness.db_path);
    let run_id = RunId::parse(RUN_ID).unwrap();
    let first_page = history
        .history(
            &run_id,
            &PageRequest::new(
                COLLECTION_PAGE_DEFAULT_COUNT,
                COLLECTION_PAGE_DATA_BUDGET_BYTES,
                None,
                (),
            )
            .unwrap(),
        )
        .expect("leading page should decode before tail corruption");
    assert_eq!(
        first_page.rows.len(),
        usize::from(COLLECTION_PAGE_DEFAULT_COUNT)
    );
    let cursor = first_page
        .next_cursor
        .expect("tail row must remain beyond the first page window");
    assert!(
        history
            .history(
                &run_id,
                &PageRequest::new(
                    COLLECTION_PAGE_DEFAULT_COUNT,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    Some(cursor),
                    (),
                )
                .unwrap(),
            )
            .is_err(),
        "tail corruption must surface when continuing from first-unreturned cursor"
    );

    harness.assert_authority_unchanged(&before);
}

#[test]
fn malformed_evidence_metadata_source_and_timestamp_detected() {
    struct Case {
        label: &'static str,
        sql: &'static str,
        kind: CorruptionKind,
        field: &'static str,
        code: &'static str,
    }

    const CASES: &[Case] = &[
        Case {
            label: "malformed metadata_json",
            sql: "UPDATE evidence SET metadata_json = '{' WHERE run_id = ?1 AND evidence_id = 'evidence-1'",
            kind: CorruptionKind::RowMalformedJson,
            field: "metadata_json",
            code: "persistence.corruption.row.malformed_json",
        },
        Case {
            label: "unsupported source",
            sql: "UPDATE evidence SET source = 'unknown' WHERE run_id = ?1 AND evidence_id = 'evidence-1'",
            kind: CorruptionKind::RowUnsupportedEnum,
            field: "source",
            code: "persistence.corruption.row.unsupported_enum",
        },
        Case {
            label: "invalid created_at timestamp",
            sql: "UPDATE evidence SET created_at = 'not-a-timestamp' WHERE run_id = ?1 AND evidence_id = 'evidence-1'",
            kind: CorruptionKind::RowBoundedSemanticValue,
            field: "created_at",
            code: "persistence.corruption.row.bounded_semantic",
        },
    ];

    for case in CASES {
        let harness = LiveHarness::seeded();
        harness.with_constraints_bypassed(|conn| {
            conn.execute(case.sql, [RUN_ID])
                .unwrap_or_else(|error| panic!("{}: {error}", case.label));
        });
        let before = harness.logical_snapshot();

        expect_corruption_deterministic(
            CorruptionPhase::Read,
            case.kind,
            Some(case.field),
            case.code,
            || inspect_logical_store(&harness.connection()),
        );
        harness.assert_authority_unchanged(&before);
    }
}

#[test]
fn evidence_row_orphaned_from_run_detected() {
    let harness = LiveHarness::seeded();
    harness.with_constraints_bypassed(|conn| {
        conn.execute(
            "UPDATE evidence SET run_id = ?1 WHERE run_id = ?2 AND evidence_id = 'evidence-1'",
            rusqlite::params![MISSING_RUN_ID, RUN_ID],
        )
        .unwrap();
    });
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::EvidenceAssociationIntegrity,
        || inspect_logical_store(&harness.connection()),
    );
    let error = inspect_logical_store(&harness.connection()).expect_err("corruption");
    let diagnostic = diagnostic_for(&error, CorruptionKind::EvidenceAssociationIntegrity, None);
    assert!(
        diagnostic.message.contains(MISSING_RUN_ID),
        "expected orphan evidence diagnostic to name missing run, got {}",
        diagnostic.message
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn orphan_journal_and_allocator_rows_detected_without_authoritative_run() {
    let harness = LiveHarness::seeded();
    harness.with_constraints_bypassed(|conn| {
        conn.execute(
            "DELETE FROM evidence_associations WHERE run_id = ?1",
            [RUN_ID],
        )
        .unwrap();
        conn.execute("DELETE FROM evidence WHERE run_id = ?1", [RUN_ID])
            .unwrap();
        conn.execute("DELETE FROM runs WHERE run_id = ?1", [RUN_ID])
            .unwrap();
    });
    let before = harness.logical_snapshot();

    let error = inspect_logical_store(&harness.connection()).expect_err("corruption");
    assert_eq!(error.phase, CorruptionPhase::Read);
    diagnostic_for(&error, CorruptionKind::JournalPayloadInconsistent, None);
    diagnostic_for(
        &error,
        CorruptionKind::JournalSequenceAllocatorMismatch,
        None,
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn negative_signed_fields_produce_deterministic_version_diagnostics() {
    struct Case {
        label: &'static str,
        sql: &'static str,
        kind: CorruptionKind,
        field: &'static str,
        code: &'static str,
    }

    const CASES: &[Case] = &[
        Case {
            label: "provider config_revision",
            sql: "UPDATE provider_registrations SET config_revision = -1 WHERE registration_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "config_revision",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "provider timeout_seconds",
            sql: "UPDATE provider_registrations SET timeout_seconds = -1 WHERE registration_id = ?1",
            kind: CorruptionKind::RowBoundedSemanticValue,
            field: "timeout_seconds",
            code: "persistence.corruption.row.bounded_semantic",
        },
        Case {
            label: "run config_revision_at_create",
            sql: "UPDATE runs SET config_revision_at_create = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "config_revision_at_create",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "run workflow_state_version",
            sql: "UPDATE runs SET workflow_state_version = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "workflow_state_version",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "run lifecycle_version",
            sql: "UPDATE runs SET lifecycle_version = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "lifecycle_version",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "run label_version",
            sql: "UPDATE runs SET label_version = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "label_version",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "run canonical_graph_version",
            sql: "UPDATE runs SET canonical_graph_version = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "canonical_graph_version",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "journal sequence",
            sql: "UPDATE journal_entries SET sequence = -1 WHERE run_id = ?1 AND sequence = 1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "sequence",
            code: "persistence.corruption.row.invalid_version",
        },
        Case {
            label: "journal allocator next_sequence",
            sql: "UPDATE run_journal_sequences SET next_sequence = -1 WHERE run_id = ?1",
            kind: CorruptionKind::RowInvalidVersion,
            field: "next_sequence",
            code: "persistence.corruption.row.invalid_version",
        },
    ];

    for case in CASES {
        let harness = LiveHarness::seeded();
        if case.field == "sequence" {
            harness.with_constraints_bypassed(|conn| {
                conn.execute(
                    "DELETE FROM evidence_associations WHERE run_id = ?1",
                    [RUN_ID],
                )
                .unwrap();
            });
        }
        harness.with_constraints_bypassed(|conn| {
            let result = if case.sql.contains("provider_registrations") {
                conn.execute(case.sql, [REGISTRATION_ID])
            } else {
                conn.execute(case.sql, [RUN_ID])
            };
            result.unwrap_or_else(|error| panic!("{}: {error}", case.label));
        });
        let before = harness.logical_snapshot();

        expect_corruption_deterministic(
            CorruptionPhase::Read,
            case.kind,
            Some(case.field),
            case.code,
            || inspect_logical_store(&harness.connection()),
        );
        harness.assert_authority_unchanged(&before);
    }
}

#[test]
fn past_schema_detected_without_mutating_live_store() {
    let harness = LiveHarness::seeded();
    harness.with_constraints_bypassed(|conn| {
        conn.execute_batch("PRAGMA user_version = 0").unwrap();
    });
    let before = harness.logical_snapshot();

    expect_corruption(
        CorruptionPhase::Read,
        CorruptionKind::SchemaIncompatible,
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn provider_enabled_non_boolean_detected_without_store_mutation() {
    let harness = LiveHarness::seeded();
    harness.with_constraints_bypassed(|conn| {
        conn.execute(
            "UPDATE provider_registrations SET enabled = 2 WHERE registration_id = ?1",
            [REGISTRATION_ID],
        )
        .unwrap();
    });
    let before = harness.logical_snapshot();

    expect_corruption_deterministic(
        CorruptionPhase::Read,
        CorruptionKind::RowUnsupportedEnum,
        Some("enabled"),
        "persistence.corruption.row.unsupported_enum",
        || inspect_logical_store(&harness.connection()),
    );
    harness.assert_authority_unchanged(&before);
}

#[test]
fn future_schema_short_circuits_before_authoritative_table_scan() {
    let harness = LiveHarness::seeded();
    let conn = harness.connection();
    let integrity_before = integrity_key_hash(&conn).unwrap();

    harness.with_constraints_bypassed(|conn| {
        conn.execute_batch("PRAGMA user_version = 99").unwrap();
        conn.execute("DROP TABLE runs", []).unwrap();
    });

    let error = inspect_logical_store(&harness.connection()).expect_err("future schema");
    assert_eq!(error.phase, CorruptionPhase::Read);
    assert_eq!(error.diagnostics.len(), 1);
    assert_eq!(
        error.primary().unwrap().kind,
        CorruptionKind::SchemaFutureVersion
    );
    assert!(
        !error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == CorruptionKind::SchemaIncompatible),
        "future schema must not fall through to candidate-table inventory"
    );
    assert!(
        error
            .source_chain()
            .iter()
            .any(|entry| entry.contains("user_version")),
        "expected early user_version classification, got {:?}",
        error.source_chain()
    );

    let conn = harness.connection();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
            .unwrap(),
        99
    );
    assert_eq!(integrity_key_hash(&conn).unwrap(), integrity_before);
    assert!(
        !conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'runs'")
            .unwrap()
            .exists([])
            .unwrap(),
        "runs table must remain removed to prove scan short-circuit"
    );
}

#[test]
fn clean_store_passes_full_inspection() {
    let harness = LiveHarness::seeded();
    inspect_logical_store(&harness.connection()).expect("clean seeded store");
    inspect_open_readonly(&harness.db_path).expect("clean readonly open");
}
