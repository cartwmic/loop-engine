use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use loop_engine_core::model::bounded::SQLITE_BUSY_TIMEOUT_MS;
use rusqlite::{Connection, OpenFlags};
use rusqlite_migration::Migrations;

use super::error::PersistenceError;
use super::migrations::{
    SUPPORTED_SCHEMA_VERSION, bundled_migrations, run_startup_schema_pipeline,
};
use super::traced::OptionalTraceSink;

pub const INTEGRATION_METADATA_TABLE: &str = "integration_metadata";
pub const INTEGRITY_KEY_ROW_KEY: &str = "integrity_key";
pub const INTEGRITY_KEY_BYTE_LENGTH: usize = 32;

/// Owning handle to an opened, migrated, pragma-configured SQLite store.
#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
    path: PathBuf,
}

impl SqliteStore {
    /// Open (and create when absent) the installation database at `path`.
    ///
    /// Creates the parent directory with mode `0700` when missing. Applies connection-local
    /// `busy_timeout`, serialized startup schema preflight/migration/postcheck before any
    /// write-affecting pragmas, persistent connection pragmas, and the integration integrity key
    /// row.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        open_at(
            path.as_ref(),
            &bundled_migrations(),
            SUPPORTED_SCHEMA_VERSION,
        )
    }

    /// Open the store; on pre-dispatch failure emit `invocation.error` when `trace` is enabled.
    pub fn open_traced(
        path: impl AsRef<Path>,
        trace: OptionalTraceSink,
    ) -> Result<Self, PersistenceError> {
        match Self::open(path) {
            Ok(store) => Ok(store),
            Err(error) => {
                trace.emit_predispatch_persistence_error(&error);
                Err(error)
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn into_connection(self) -> Connection {
        self.conn
    }
}

pub(crate) fn open_at<'m>(
    path: &Path,
    migrations: &Migrations<'m>,
    supported_version: u32,
) -> Result<SqliteStore, PersistenceError> {
    ensure_parent_directory(path)?;
    let mut conn = connect(path)?;
    apply_busy_timeout(&conn)?;
    run_startup_schema_pipeline(&mut conn, migrations, supported_version)?;
    apply_persistent_connection_pragmas(&conn)?;
    verify_integration_metadata(&conn)?;
    Ok(SqliteStore {
        conn,
        path: path.to_path_buf(),
    })
}

fn ensure_parent_directory(path: &Path) -> Result<(), PersistenceError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || parent.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|source| PersistenceError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })
}

fn connect(path: &Path) -> Result<Connection, PersistenceError> {
    Connection::open(path).map_err(|source| PersistenceError::Open {
        path: path.to_path_buf(),
        source,
    })
}

/// Opens a file-backed connection and applies the persistence pragma contract.
///
/// Does not create parent directories, run migrations, or verify metadata. Intended
/// for later adapters that need additional connections (for example post-commit
/// verification) without duplicating pragma setup.
pub fn connect_with_pragmas(path: &Path) -> Result<Connection, PersistenceError> {
    let conn = connect(path)?;
    apply_connection_pragmas(&conn)?;
    Ok(conn)
}

/// Opens an existing database read-only and applies safe read-side pragmas.
///
/// Does not create the file, run migrations, or execute write-affecting pragmas.
/// Verifies `journal_mode` is `wal` via query rather than assignment.
pub fn connect_read_only_with_pragmas(path: &Path) -> Result<Connection, PersistenceError> {
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            PersistenceError::Open {
                path: path.to_path_buf(),
                source,
            }
        })?;
    apply_read_only_connection_pragmas(&conn)?;
    Ok(conn)
}

const JOURNAL_MODE_WAL_RETRY_BACKOFF: Duration = Duration::from_millis(1);

fn is_journal_mode_contention(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
    )
}

fn journal_mode_not_wal_error(mode: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
        Some(format!("journal_mode returned {mode:?}, expected wal")),
    )
}

fn verify_journal_mode_wal(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if mode.eq_ignore_ascii_case("wal") {
        Ok(())
    } else {
        Err(journal_mode_not_wal_error(&mode))
    }
}

fn set_journal_mode_wal(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if mode.eq_ignore_ascii_case("wal") {
        Ok(())
    } else {
        Err(journal_mode_not_wal_error(&mode))
    }
}

fn apply_journal_mode_wal_with_retry(conn: &Connection) -> Result<(), PersistenceError> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
        .unwrap_or_else(Instant::now);

    loop {
        match set_journal_mode_wal(conn) {
            Ok(()) => return Ok(()),
            Err(source) if is_journal_mode_contention(&source) => {
                if Instant::now() >= deadline {
                    return Err(PersistenceError::Pragma {
                        pragma: "journal_mode",
                        source,
                    });
                }
                std::thread::sleep(JOURNAL_MODE_WAL_RETRY_BACKOFF);
            }
            Err(source) => {
                return Err(PersistenceError::Pragma {
                    pragma: "journal_mode",
                    source,
                });
            }
        }
    }
}

fn apply_busy_timeout(conn: &Connection) -> Result<(), PersistenceError> {
    conn.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
        .map_err(|source| PersistenceError::Pragma {
            pragma: "busy_timeout",
            source,
        })
}

fn apply_persistent_connection_pragmas(conn: &Connection) -> Result<(), PersistenceError> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA synchronous = FULL;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|source| PersistenceError::Pragma {
        pragma: "connection",
        source,
    })?;
    apply_journal_mode_wal_with_retry(conn)
}

fn apply_read_only_connection_pragmas(conn: &Connection) -> Result<(), PersistenceError> {
    apply_busy_timeout(conn)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA synchronous = FULL;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|source| PersistenceError::Pragma {
        pragma: "connection",
        source,
    })?;
    verify_journal_mode_wal(conn).map_err(|source| PersistenceError::Pragma {
        pragma: "journal_mode",
        source,
    })
}

pub(crate) fn apply_connection_pragmas(conn: &Connection) -> Result<(), PersistenceError> {
    apply_busy_timeout(conn)?;
    apply_persistent_connection_pragmas(conn)
}

pub(crate) fn verify_integration_metadata(conn: &Connection) -> Result<(), PersistenceError> {
    let length: Result<i64, rusqlite::Error> = conn.query_row(
        &format!("SELECT length(value) FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
        [INTEGRITY_KEY_ROW_KEY],
        |row| row.get(0),
    );
    match length {
        Ok(len) if len == INTEGRITY_KEY_BYTE_LENGTH as i64 => Ok(()),
        Ok(len) => Err(PersistenceError::MetadataKeyInvalidLength {
            key: INTEGRITY_KEY_ROW_KEY,
            expected: INTEGRITY_KEY_BYTE_LENGTH,
            actual: len as usize,
        }),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(PersistenceError::MetadataKeyMissing {
            key: INTEGRITY_KEY_ROW_KEY,
        }),
        Err(source) => Err(PersistenceError::MetadataRead { source }),
    }
}

pub(crate) mod commit {
    use std::path::Path;

    use rusqlite::{Connection, OptionalExtension, params};

    use super::{PersistenceError, connect_read_only_with_pragmas};

    /// Authoritative read result after commit I/O failure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CommitBundlePresence {
        Complete,
        Absent,
        Partial,
    }

    pub fn merge_presence(parts: impl IntoIterator<Item = bool>) -> CommitBundlePresence {
        let mut any = false;
        let mut all = true;
        for present in parts {
            any |= present;
            all &= present;
        }
        match (any, all) {
            (false, _) => CommitBundlePresence::Absent,
            (true, true) => CommitBundlePresence::Complete,
            (true, false) => CommitBundlePresence::Partial,
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct JournalRowExpectation {
        pub run_id: String,
        pub sequence: u64,
        pub outcome: String,
        pub payload: String,
    }

    impl JournalRowExpectation {
        pub fn is_present(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            conn.query_row(
                "SELECT outcome, encoded_payload_json FROM journal_entries
                 WHERE run_id = ?1 AND sequence = ?2",
                params![
                    self.run_id,
                    i64::try_from(self.sequence).unwrap_or(i64::MAX)
                ],
                |row| {
                    let outcome: String = row.get(0)?;
                    let payload: String = row.get(1)?;
                    Ok(outcome == self.outcome && payload == self.payload)
                },
            )
            .optional()
            .map(|opt| opt.unwrap_or(false))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunAuthoritativeExpectation {
        pub run_id: String,
        pub current_state: String,
        pub lifecycle: String,
        pub workflow_state_version: u64,
        pub lifecycle_version: u64,
        pub label: Option<String>,
        pub label_version: u64,
        pub next_sequence: u64,
    }

    impl RunAuthoritativeExpectation {
        pub fn is_present(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            conn.query_row(
                "SELECT r.current_state, r.lifecycle, r.workflow_state_version, r.lifecycle_version,
                        r.label, r.label_version, s.next_sequence
                 FROM runs r
                 JOIN run_journal_sequences s ON s.run_id = r.run_id
                 WHERE r.run_id = ?1",
                params![self.run_id],
                |row| {
                    let current_state: String = row.get(0)?;
                    let lifecycle: String = row.get(1)?;
                    let wf: i64 = row.get(2)?;
                    let lv: i64 = row.get(3)?;
                    let label: Option<String> = row.get(4)?;
                    let label_version: i64 = row.get(5)?;
                    let next_seq: i64 = row.get(6)?;
                    Ok(
                        current_state == self.current_state
                            && lifecycle == self.lifecycle
                            && wf as u64 == self.workflow_state_version
                            && lv as u64 == self.lifecycle_version
                            && label == self.label
                            && label_version as u64 == self.label_version
                            && next_seq as u64 == self.next_sequence,
                    )
                },
            )
            .optional()
            .map(|opt| opt.unwrap_or(false))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunCreateRowExpectation {
        pub run_id: String,
        pub registration_id: String,
        pub config_revision_at_create: u64,
        pub current_state: String,
        pub lifecycle: String,
        pub workflow_state_version: u64,
        pub lifecycle_version: u64,
        pub label: Option<String>,
        pub graph_revision: String,
        pub graph_json: String,
        pub inputs_json: String,
    }

    impl RunCreateRowExpectation {
        pub fn is_present(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            conn.query_row(
                "SELECT registration_id, config_revision_at_create, current_state, lifecycle,
                        workflow_state_version, lifecycle_version, label, graph_revision,
                        graph_canonical_projection_json, inputs_json
                 FROM runs WHERE run_id = ?1",
                params![self.run_id],
                |row| {
                    let registration_id: String = row.get(0)?;
                    let config_revision: i64 = row.get(1)?;
                    let current_state: String = row.get(2)?;
                    let lifecycle: String = row.get(3)?;
                    let wf: i64 = row.get(4)?;
                    let lv: i64 = row.get(5)?;
                    let label: Option<String> = row.get(6)?;
                    let graph_revision: String = row.get(7)?;
                    let graph_json: String = row.get(8)?;
                    let inputs_json: String = row.get(9)?;
                    Ok(registration_id == self.registration_id
                        && config_revision as u64 == self.config_revision_at_create
                        && current_state == self.current_state
                        && lifecycle == self.lifecycle
                        && wf as u64 == self.workflow_state_version
                        && lv as u64 == self.lifecycle_version
                        && label == self.label
                        && graph_revision == self.graph_revision
                        && graph_json == self.graph_json
                        && inputs_json == self.inputs_json)
                },
            )
            .optional()
            .map(|opt| opt.unwrap_or(false))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunCreateCommitExpectation {
        pub run: RunCreateRowExpectation,
        pub next_sequence: u64,
        pub journal: JournalRowExpectation,
    }

    impl RunCreateCommitExpectation {
        pub fn verify(&self, conn: &Connection) -> Result<CommitBundlePresence, rusqlite::Error> {
            let sequence_present = conn
                .query_row(
                    "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
                    params![self.run.run_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .map(|next| next as u64 == self.next_sequence)
                .unwrap_or(false);
            Ok(merge_presence([
                self.run.is_present(conn)?,
                sequence_present,
                self.journal.is_present(conn)?,
            ]))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct EvidenceAssociationExpectation {
        pub run_id: String,
        pub journal_sequence: u64,
        pub evidence_id: String,
        pub event_id: Option<String>,
        pub gate_id: Option<String>,
    }

    impl EvidenceAssociationExpectation {
        pub fn is_present(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            conn.query_row(
                "SELECT event_id, gate_id FROM evidence_associations
                 WHERE run_id = ?1 AND journal_sequence = ?2 AND evidence_id = ?3",
                params![
                    self.run_id,
                    i64::try_from(self.journal_sequence).unwrap_or(i64::MAX),
                    self.evidence_id,
                ],
                |row| {
                    let event_id: Option<String> = row.get(0)?;
                    let gate_id: Option<String> = row.get(1)?;
                    Ok(event_id == self.event_id && gate_id == self.gate_id)
                },
            )
            .optional()
            .map(|opt| opt.unwrap_or(false))
        }
    }

    pub fn evidence_rows_present(
        conn: &Connection,
        run_id: &str,
        evidence_ids: &[String],
    ) -> Result<bool, rusqlite::Error> {
        for evidence_id in evidence_ids {
            let present = conn
                .query_row(
                    "SELECT 1 FROM evidence WHERE run_id = ?1 AND evidence_id = ?2 LIMIT 1",
                    params![run_id, evidence_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !present {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn any_expected_evidence_present(
        conn: &Connection,
        run_id: &str,
        evidence_ids: &[String],
    ) -> Result<bool, rusqlite::Error> {
        for evidence_id in evidence_ids {
            let present = conn
                .query_row(
                    "SELECT 1 FROM evidence WHERE run_id = ?1 AND evidence_id = ?2 LIMIT 1",
                    params![run_id, evidence_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if present {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct JournalBundleExpectation {
        /// Whether the attempted mutation changed authoritative run fields.
        pub run_changed: bool,
        pub run: RunAuthoritativeExpectation,
        pub journal: JournalRowExpectation,
        pub evidence_ids: Vec<String>,
        pub associations: Vec<EvidenceAssociationExpectation>,
    }

    impl JournalBundleExpectation {
        pub fn verify(&self, conn: &Connection) -> Result<CommitBundlePresence, rusqlite::Error> {
            let journal_present = self.journal.is_present(conn)?;
            let evidence_complete =
                evidence_rows_present(conn, &self.run.run_id, &self.evidence_ids)?;
            let association_presence: Result<Vec<bool>, rusqlite::Error> = self
                .associations
                .iter()
                .map(|association| association.is_present(conn))
                .collect();
            let association_presence = association_presence?;
            let associations_complete = association_presence.iter().all(|&present| present);
            let any_association = association_presence.iter().any(|&present| present);
            let any_evidence =
                any_expected_evidence_present(conn, &self.run.run_id, &self.evidence_ids)?;
            let run_present = self.run.is_present(conn)?;
            let any_bundle = journal_present || any_evidence || any_association;

            if journal_present {
                if run_present && evidence_complete && associations_complete {
                    Ok(CommitBundlePresence::Complete)
                } else {
                    Ok(CommitBundlePresence::Partial)
                }
            } else if any_bundle || (self.run_changed && run_present) {
                Ok(CommitBundlePresence::Partial)
            } else {
                Ok(CommitBundlePresence::Absent)
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CatalogRegistrationExpectation {
        pub registration_id: String,
        pub handle: Option<String>,
        pub enabled: bool,
        pub config_revision: u64,
        pub executable: String,
        pub argv_json: String,
        pub working_directory: String,
        pub timeout_seconds: u64,
        pub should_exist: bool,
    }

    impl CatalogRegistrationExpectation {
        pub fn verify(&self, conn: &Connection) -> Result<CommitBundlePresence, rusqlite::Error> {
            let row_present = self.is_present(conn)?;
            if !self.should_exist {
                return Ok(if row_present {
                    CommitBundlePresence::Partial
                } else {
                    CommitBundlePresence::Absent
                });
            }
            if !row_present {
                return Ok(CommitBundlePresence::Absent);
            }
            if self.active_handle_ok(conn)? {
                Ok(CommitBundlePresence::Complete)
            } else {
                Ok(CommitBundlePresence::Partial)
            }
        }

        fn is_present(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            conn.query_row(
                "SELECT handle, enabled, config_revision, executable, argv_json,
                        working_directory, timeout_seconds
                 FROM provider_registrations WHERE registration_id = ?1",
                params![self.registration_id],
                |row| {
                    let handle: Option<String> = row.get(0)?;
                    let enabled: i64 = row.get(1)?;
                    let config_revision: i64 = row.get(2)?;
                    let executable: String = row.get(3)?;
                    let argv_json: String = row.get(4)?;
                    let working_directory: String = row.get(5)?;
                    let timeout_seconds: i64 = row.get(6)?;
                    Ok(handle == self.handle
                        && (enabled != 0) == self.enabled
                        && config_revision as u64 == self.config_revision
                        && executable == self.executable
                        && argv_json == self.argv_json
                        && working_directory == self.working_directory
                        && timeout_seconds as u64 == self.timeout_seconds)
                },
            )
            .optional()
            .map(|opt| opt.unwrap_or(false))
        }

        fn active_handle_ok(&self, conn: &Connection) -> Result<bool, rusqlite::Error> {
            if !self.enabled {
                return Ok(true);
            }
            let Some(handle) = &self.handle else {
                return Ok(true);
            };
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM provider_registrations WHERE enabled = 1 AND handle = ?1",
                params![handle],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        }
    }

    /// Abandon the failed writer and verify durable post-state on a fresh connection.
    pub fn verify_after_commit_error<E, F>(
        path: &Path,
        verify: F,
        map_open: impl FnOnce(PersistenceError) -> E,
        map_verify: impl FnOnce(rusqlite::Error) -> E,
    ) -> Result<CommitBundlePresence, E>
    where
        F: FnOnce(&Connection) -> Result<CommitBundlePresence, rusqlite::Error>,
    {
        let read_conn = connect_read_only_with_pragmas(path).map_err(map_open)?;
        verify(&read_conn).map_err(map_verify)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_committed_transaction<T, F, E>(
        path: &Path,
        conn: Connection,
        success: T,
        verify: F,
        map_commit_io: impl FnOnce(rusqlite::Error) -> E,
        map_unverified: impl FnOnce() -> E,
        map_integrity: impl FnOnce() -> E,
        map_read: impl FnOnce(PersistenceError) -> E,
    ) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<CommitBundlePresence, rusqlite::Error>,
    {
        finish_committed_transaction_using(
            path,
            conn,
            success,
            verify,
            map_commit_io,
            map_unverified,
            map_integrity,
            map_read,
            |conn| conn.execute("COMMIT", []).map(|_| ()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_committed_transaction_using<T, F, E, C>(
        path: &Path,
        conn: Connection,
        success: T,
        verify: F,
        map_commit_io: impl FnOnce(rusqlite::Error) -> E,
        map_unverified: impl FnOnce() -> E,
        map_integrity: impl FnOnce() -> E,
        map_read: impl FnOnce(PersistenceError) -> E,
        commit: C,
    ) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<CommitBundlePresence, rusqlite::Error>,
        C: FnOnce(&Connection) -> Result<(), rusqlite::Error>,
    {
        match commit(&conn) {
            Ok(()) => Ok(success),
            Err(_source) => {
                drop(conn);
                let presence = verify_after_commit_error(path, verify, map_read, map_commit_io)?;
                match presence {
                    CommitBundlePresence::Complete => Ok(success),
                    CommitBundlePresence::Absent => Err(map_unverified()),
                    CommitBundlePresence::Partial => Err(map_integrity()),
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rusqlite::Connection;

        #[test]
        fn merge_presence_classifies_complete_absent_and_partial() {
            assert_eq!(merge_presence([true, true]), CommitBundlePresence::Complete);
            assert_eq!(merge_presence([false, false]), CommitBundlePresence::Absent);
            assert_eq!(merge_presence([true, false]), CommitBundlePresence::Partial);
        }

        #[test]
        fn journal_bundle_expectation_absent_when_unchanged_run_matches_without_bundle() {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                     run_id TEXT PRIMARY KEY,
                     current_state TEXT NOT NULL,
                     lifecycle TEXT NOT NULL,
                     workflow_state_version INTEGER NOT NULL,
                     lifecycle_version INTEGER NOT NULL,
                     label TEXT,
                     label_version INTEGER NOT NULL
                 );
                 CREATE TABLE run_journal_sequences (
                     run_id TEXT PRIMARY KEY,
                     next_sequence INTEGER NOT NULL
                 );
                 CREATE TABLE journal_entries (
                     run_id TEXT NOT NULL,
                     sequence INTEGER NOT NULL,
                     outcome TEXT NOT NULL,
                     encoded_payload_json TEXT NOT NULL,
                     PRIMARY KEY (run_id, sequence)
                 );
                 INSERT INTO runs VALUES ('run-1', 'draft', 'active', 1, 1, NULL, 1);
                 INSERT INTO run_journal_sequences VALUES ('run-1', 2);",
            )
            .unwrap();
            let expectation = JournalBundleExpectation {
                run_changed: false,
                run: RunAuthoritativeExpectation {
                    run_id: "run-1".into(),
                    current_state: "draft".into(),
                    lifecycle: "active".into(),
                    workflow_state_version: 1,
                    lifecycle_version: 1,
                    label: None,
                    label_version: 1,
                    next_sequence: 2,
                },
                journal: JournalRowExpectation {
                    run_id: "run-1".into(),
                    sequence: 2,
                    outcome: "completed".into(),
                    payload: "{}".into(),
                },
                evidence_ids: Vec::new(),
                associations: Vec::new(),
            };
            assert_eq!(
                expectation.verify(&conn).unwrap(),
                CommitBundlePresence::Absent
            );
        }

        #[test]
        fn journal_bundle_expectation_partial_when_run_changed_without_journal() {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                     run_id TEXT PRIMARY KEY,
                     current_state TEXT NOT NULL,
                     lifecycle TEXT NOT NULL,
                     workflow_state_version INTEGER NOT NULL,
                     lifecycle_version INTEGER NOT NULL,
                     label TEXT,
                     label_version INTEGER NOT NULL
                 );
                 CREATE TABLE run_journal_sequences (
                     run_id TEXT PRIMARY KEY,
                     next_sequence INTEGER NOT NULL
                 );
                 CREATE TABLE journal_entries (
                     run_id TEXT NOT NULL,
                     sequence INTEGER NOT NULL,
                     outcome TEXT NOT NULL,
                     encoded_payload_json TEXT NOT NULL,
                     PRIMARY KEY (run_id, sequence)
                 );
                 INSERT INTO runs VALUES ('run-1', 'done', 'final', 2, 1, NULL, 1);
                 INSERT INTO run_journal_sequences VALUES ('run-1', 3);",
            )
            .unwrap();
            let expectation = JournalBundleExpectation {
                run_changed: true,
                run: RunAuthoritativeExpectation {
                    run_id: "run-1".into(),
                    current_state: "done".into(),
                    lifecycle: "final".into(),
                    workflow_state_version: 2,
                    lifecycle_version: 1,
                    label: None,
                    label_version: 1,
                    next_sequence: 3,
                },
                journal: JournalRowExpectation {
                    run_id: "run-1".into(),
                    sequence: 2,
                    outcome: "completed".into(),
                    payload: "{}".into(),
                },
                evidence_ids: Vec::new(),
                associations: Vec::new(),
            };
            assert_eq!(
                expectation.verify(&conn).unwrap(),
                CommitBundlePresence::Partial
            );
        }

        fn simulated_commit_io_error() -> rusqlite::Error {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
                Some("test commit I/O failure".into()),
            )
        }

        fn row_presence(conn: &Connection) -> Result<CommitBundlePresence, rusqlite::Error> {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))?;
            Ok(if count == 1 {
                CommitBundlePresence::Complete
            } else {
                CommitBundlePresence::Absent
            })
        }

        #[test]
        fn commit_error_before_commit_verifies_absent_on_fresh_connection() {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("state.db");
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE t (id INTEGER PRIMARY KEY);
                 BEGIN IMMEDIATE;
                 INSERT INTO t (id) VALUES (1);",
            )
            .unwrap();

            let result = finish_committed_transaction_using(
                &path,
                conn,
                "committed",
                row_presence,
                |error| format!("read: {error}"),
                || "unverified".to_owned(),
                || "integrity".to_owned(),
                |error| format!("open: {error}"),
                |_| Err(simulated_commit_io_error()),
            );

            assert_eq!(result, Err("unverified".to_owned()));
        }

        #[test]
        fn commit_error_after_commit_verifies_complete_on_fresh_connection() {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("state.db");
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE t (id INTEGER PRIMARY KEY);
                 BEGIN IMMEDIATE;
                 INSERT INTO t (id) VALUES (1);",
            )
            .unwrap();

            let result = finish_committed_transaction_using(
                &path,
                conn,
                "committed",
                row_presence,
                |error| format!("read: {error}"),
                || "unverified".to_owned(),
                || "integrity".to_owned(),
                |error| format!("open: {error}"),
                |conn| {
                    conn.execute("COMMIT", [])?;
                    Err(simulated_commit_io_error())
                },
            );

            assert_eq!(result, Ok("committed"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    use rusqlite_migration::{M, Migrations};
    use tempfile::TempDir;

    use super::{
        INTEGRITY_KEY_BYTE_LENGTH, INTEGRITY_KEY_ROW_KEY, apply_connection_pragmas,
        connect_read_only_with_pragmas, open_at, verify_integration_metadata,
    };
    use crate::persistence::error::{PersistenceError, SchemaMismatchKind};
    use crate::persistence::migrations::{
        SUPPORTED_SCHEMA_VERSION, V1_SCHEMA_INDEXES, V1_SCHEMA_TABLES, bundled_migrations,
        read_user_version,
    };

    fn read_pragma_i64(conn: &rusqlite::Connection, pragma: &str) -> i64 {
        conn.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
            .unwrap()
    }

    fn read_pragma_text(conn: &rusqlite::Connection, pragma: &str) -> String {
        conn.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
            .unwrap()
    }

    fn schema_object_exists(conn: &rusqlite::Connection, object_type: &str, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
            rusqlite::params![object_type, name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    #[test]
    fn open_empty_store_applies_migration_and_metadata() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        assert!(!path.exists());

        let store = open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        assert!(path.exists());
        verify_integration_metadata(store.connection()).unwrap();
        assert_eq!(read_user_version(store.connection()).unwrap(), 1);
    }

    #[test]
    fn open_latest_store_is_idempotent() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");

        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        let second = open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        assert_eq!(read_user_version(second.connection()).unwrap(), 1);
        verify_integration_metadata(second.connection()).unwrap();
    }

    #[test]
    fn connection_pragmas_match_persistence_contract() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        apply_connection_pragmas(&conn).unwrap();

        assert_eq!(read_pragma_i64(&conn, "foreign_keys"), 1);
        assert_eq!(read_pragma_text(&conn, "journal_mode"), "wal");
        assert_eq!(read_pragma_i64(&conn, "synchronous"), 2);
        assert_eq!(read_pragma_i64(&conn, "busy_timeout"), 5_000);
        assert_eq!(read_pragma_i64(&conn, "temp_store"), 2);
    }

    #[test]
    fn read_only_connection_applies_safe_pragmas_without_writes() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();

        let conn = connect_read_only_with_pragmas(&path).unwrap();
        assert_eq!(read_pragma_i64(&conn, "foreign_keys"), 1);
        assert_eq!(read_pragma_text(&conn, "journal_mode"), "wal");
        assert_eq!(read_pragma_i64(&conn, "synchronous"), 2);
        assert_eq!(read_pragma_i64(&conn, "busy_timeout"), 5_000);
        assert_eq!(read_pragma_i64(&conn, "temp_store"), 2);
        assert!(conn.execute("CREATE TABLE leak (id INTEGER)", []).is_err());
    }

    #[test]
    fn future_schema_version_is_rejected_without_mutation() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();

        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 2").unwrap();
        }

        let error = open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::FutureSchema {
                supported: 1,
                observed: 2
            }
        ));

        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_user_version(&conn).unwrap(), 2);
    }

    #[test]
    fn future_schema_rejects_before_journal_mode_wal_mutation() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA user_version = 2;")
                .unwrap();
            assert_eq!(read_pragma_text(&conn, "journal_mode"), "delete");
        }

        let error = open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::FutureSchema {
                supported: 1,
                observed: 2,
            }
        ));

        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_pragma_text(&conn, "journal_mode"), "delete");
        assert_eq!(read_user_version(&conn).unwrap(), 2);
    }

    #[test]
    fn incomplete_v1_schema_shape_fails_closed_for_each_table() {
        for &table in V1_SCHEMA_TABLES {
            let directory = TempDir::new().unwrap();
            let path = directory.path().join("state.db");
            open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();

            {
                let conn = rusqlite::Connection::open(&path).unwrap();
                conn.execute_batch(&format!(
                    "PRAGMA foreign_keys = OFF;
                     DROP TABLE {table};
                     PRAGMA foreign_keys = ON;"
                ))
                .unwrap();
            }

            let error =
                open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap_err();
            assert!(
                matches!(
                    error,
                    PersistenceError::SchemaMismatch {
                        object_type: "table",
                        name,
                        kind: SchemaMismatchKind::Missing,
                    } if name == table
                ),
                "unexpected error for missing table {table}: {error}"
            );

            let conn = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(read_user_version(&conn).unwrap(), 1);
            assert!(!schema_object_exists(&conn, "table", table));
        }
    }

    #[test]
    fn incomplete_v1_schema_shape_fails_closed_for_each_index() {
        for &index in V1_SCHEMA_INDEXES {
            let directory = TempDir::new().unwrap();
            let path = directory.path().join("state.db");
            open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();

            {
                let conn = rusqlite::Connection::open(&path).unwrap();
                conn.execute_batch(&format!("DROP INDEX {index};")).unwrap();
            }

            let error =
                open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap_err();
            assert!(
                matches!(
                    error,
                    PersistenceError::SchemaMismatch {
                        object_type: "index",
                        name,
                        kind: SchemaMismatchKind::Missing,
                    } if name == index
                ),
                "unexpected error for missing index {index}: {error}"
            );

            let conn = rusqlite::Connection::open(&path).unwrap();
            assert_eq!(read_user_version(&conn).unwrap(), 1);
            assert!(!schema_object_exists(&conn, "index", index));
        }
    }

    #[test]
    fn failed_migration_rolls_back_on_empty_store() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let migrations = Migrations::new(vec![M::up("NOT VALID SQL SYNTAX")]);

        let error = open_at(&path, &migrations, 1).unwrap_err();
        assert!(matches!(error, PersistenceError::Migration { .. }));

        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_user_version(&conn).unwrap(), 0);
    }

    #[test]
    fn failed_migration_rolls_back_all_pending_steps() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let migrations = Migrations::new(vec![
            M::up(
                "CREATE TABLE migration_marker (id INTEGER PRIMARY KEY);
                 INSERT INTO migration_marker (id) VALUES (1);",
            ),
            M::up("THIS IS NOT VALID SQL"),
        ]);

        let error = open_at(&path, &migrations, 2).unwrap_err();
        assert!(matches!(error, PersistenceError::Migration { .. }));

        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_user_version(&conn).unwrap(), 0);
        let marker_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'migration_marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_exists, 0);
    }

    #[test]
    fn concurrent_opens_apply_migration_once() {
        let directory = Arc::new(TempDir::new().unwrap());
        let path = directory.path().join("state.db");
        const CONCURRENT_OPENS: usize = 4;
        const BARRIER_WAVES: usize = 5;

        for _ in 0..BARRIER_WAVES {
            let barrier = Arc::new(Barrier::new(CONCURRENT_OPENS));
            thread::scope(|scope| {
                let mut handles = Vec::new();
                for _ in 0..CONCURRENT_OPENS {
                    let path = path.clone();
                    let barrier = Arc::clone(&barrier);
                    handles.push(scope.spawn(move || {
                        barrier.wait();
                        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap()
                    }));
                }
                for handle in handles {
                    handle.join().unwrap();
                }
            });
        }

        let conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_user_version(&conn).unwrap(), 1);
        verify_integration_metadata(&conn).unwrap();
        let integrity_length: i64 = conn
            .query_row(
                "SELECT length(value) FROM integration_metadata WHERE key = ?1",
                [INTEGRITY_KEY_ROW_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(integrity_length, INTEGRITY_KEY_BYTE_LENGTH as i64);
    }
}
