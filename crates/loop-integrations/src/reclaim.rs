//! Explicit reclaim of bulky legacy work-slot invocation payloads.
//!
//! Earlier releases stored worker commands, prompts and duplicated completion
//! snapshots on every invocation row. Reclaim clears those columns, only for
//! final or terminated runs, to the empty values the catalog reader already
//! accepts, then VACUUMs so the catalog file shrinks. `binding_json` and every
//! other column stay intact, so cleaned runs remain listable and showable.
//! Nothing here runs implicitly.

use rusqlite::{Connection, ErrorCode, OpenFlags};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Rows that still hold bulky payload in any reclaimable column.
const NOT_EMPTY: &str = "(w.inner_workers_json != '[]'
    OR w.completion_snapshot_json != '[]'
    OR w.routed_inputs_json != '[]'
    OR w.invocation_input_json IS NOT NULL
    OR w.assignment_selection_json IS NOT NULL
    OR w.frozen_run_identity_json IS NOT NULL)";

/// Bytes the reclaimable columns hold beyond their empty values.
const PAYLOAD_BYTES: &str = "(length(CAST(w.inner_workers_json AS BLOB))
    + length(CAST(w.completion_snapshot_json AS BLOB))
    + length(CAST(w.routed_inputs_json AS BLOB))
    + coalesce(length(CAST(w.invocation_input_json AS BLOB)), 0)
    + coalesce(length(CAST(w.assignment_selection_json AS BLOB)), 0)
    + coalesce(length(CAST(w.frozen_run_identity_json AS BLOB)), 0) - 6)";

const FINISHED: &str = "r.lifecycle IN ('final', 'terminated')";

/// One finished run that holds reclaimable payload.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReclaimRun {
    pub run_id: String,
    pub lifecycle: String,
    pub invocation_rows: u64,
    pub reclaimable_bytes: u64,
}

/// Preview of what apply would clear.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReclaimPreview {
    pub database: PathBuf,
    pub runs: Vec<ReclaimRun>,
    pub invocation_rows: u64,
    pub reclaimable_bytes: u64,
    pub file_bytes: u64,
    pub wal_bytes: u64,
}

/// Outcome of an applied reclaim.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReclaimApplied {
    pub database: PathBuf,
    pub runs: Vec<ReclaimRun>,
    pub invocation_rows_cleared: u64,
    pub before: FileSizes,
    pub after: FileSizes,
}

/// Catalog file sizes, main database plus write-ahead log.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct FileSizes {
    pub file_bytes: u64,
    pub wal_bytes: u64,
}

/// A reclaim failure with a stable code and a message naming the cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimError {
    pub code: &'static str,
    pub message: String,
}

impl ReclaimError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// `ENOSPC` (no space left) and `EFBIG` (file size limit) on Linux and macOS.
const NO_ROOM_ERRNOS: [i32; 2] = [28, 27];

/// Classify a failure on an open connection, consulting the operating system
/// error so that a write refused for lack of room reads as full storage even
/// when SQLite reports a generic I/O error.
fn classify_on(connection: &Connection, step: &str, error: rusqlite::Error) -> ReclaimError {
    if error.sqlite_error_code() == Some(ErrorCode::SystemIoFailure) {
        // SAFETY: the handle belongs to this live connection and the call only
        // reads the last OS error SQLite recorded for it.
        let errno = unsafe { rusqlite::ffi::sqlite3_system_errno(connection.handle()) };
        if NO_ROOM_ERRNOS.contains(&errno) {
            return ReclaimError::new(
                "catalog-storage-full",
                format!("{step}: not enough storage to finish ({error}, os error {errno}); the catalog is consistent, any column clearing already committed stays, and reclaim can be retried with more free space"),
            );
        }
    }
    classify(step, error)
}

fn classify(step: &str, error: rusqlite::Error) -> ReclaimError {
    let code = error.sqlite_error_code();
    match code {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => ReclaimError::new(
            "catalog-busy",
            format!("{step}: another process is holding the catalog ({error}); nothing further was changed and it is safe to retry"),
        ),
        Some(ErrorCode::DiskFull) => ReclaimError::new(
            "catalog-storage-full",
            format!("{step}: not enough storage to finish ({error}); the catalog is consistent, any column clearing already committed stays, and reclaim can be retried with more free space"),
        ),
        Some(ErrorCode::SystemIoFailure | ErrorCode::CannotOpen) => ReclaimError::new(
            "catalog-io-error",
            format!("{step}: storage I/O failed ({error}); the catalog is consistent and reclaim can be retried"),
        ),
        _ => ReclaimError::new("catalog-reclaim-failed", format!("{step}: {error}")),
    }
}

fn sizes(database: &Path) -> FileSizes {
    let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut wal = database.as_os_str().to_owned();
    wal.push("-wal");
    FileSizes {
        file_bytes: size(database),
        wal_bytes: size(Path::new(&wal)),
    }
}

fn open(database: &Path, flags: OpenFlags) -> Result<Connection, ReclaimError> {
    if !database.is_file() {
        return Err(ReclaimError::new(
            "catalog-not-found",
            format!("catalog `{}` does not exist", database.display()),
        ));
    }
    let connection = Connection::open_with_flags(database, flags)
        .map_err(|error| classify("open catalog", error))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| classify("set busy timeout", error))?;
    Ok(connection)
}

fn candidate_runs(connection: &Connection) -> Result<Vec<ReclaimRun>, ReclaimError> {
    let sql = format!(
        "SELECT r.id, r.lifecycle, COUNT(*), SUM({PAYLOAD_BYTES})
         FROM runs r JOIN work_slot_invocations w ON w.run_id = r.id
         WHERE {FINISHED} AND {NOT_EMPTY}
         GROUP BY r.id, r.lifecycle ORDER BY r.id"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| classify("inspect catalog", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ReclaimRun {
                run_id: row.get(0)?,
                lifecycle: row.get(1)?,
                invocation_rows: row.get::<_, i64>(2)?.max(0) as u64,
                reclaimable_bytes: row.get::<_, i64>(3)?.max(0) as u64,
            })
        })
        .map_err(|error| classify("inspect catalog", error))?;
    rows.collect::<Result<_, _>>()
        .map_err(|error| classify("inspect catalog", error))
}

/// Report what apply would clear without writing to the catalog.
pub fn preview(database: &Path) -> Result<ReclaimPreview, ReclaimError> {
    let connection = open(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let runs = candidate_runs(&connection)?;
    let file = sizes(database);
    Ok(ReclaimPreview {
        database: database.to_path_buf(),
        invocation_rows: runs.iter().map(|run| run.invocation_rows).sum(),
        reclaimable_bytes: runs.iter().map(|run| run.reclaimable_bytes).sum(),
        runs,
        file_bytes: file.file_bytes,
        wal_bytes: file.wal_bytes,
    })
}

/// Clear finished runs' bulky payload columns, then shrink the catalog file.
pub fn apply(database: &Path) -> Result<ReclaimApplied, ReclaimError> {
    let mut connection = open(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let before = sizes(database);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| classify("begin reclaim", error))?;
    let runs = candidate_runs(&transaction)?;
    let cleared = transaction
        .execute(
            &format!(
                "UPDATE work_slot_invocations AS w
                 SET inner_workers_json = '[]',
                     completion_snapshot_json = '[]',
                     routed_inputs_json = '[]',
                     invocation_input_json = NULL,
                     assignment_selection_json = NULL,
                     frozen_run_identity_json = NULL
                 WHERE w.run_id IN (SELECT r.id FROM runs r WHERE {FINISHED}) AND {NOT_EMPTY}"
            ),
            [],
        )
        .map_err(|error| classify_on(&transaction, "clear legacy payloads", error))?;
    transaction
        .commit()
        .map_err(|error| classify("commit reclaim", error))?;
    connection
        .execute_batch("VACUUM")
        .map_err(|error| classify_on(&connection, "shrink catalog (VACUUM)", error))?;
    let busy: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|error| classify_on(&connection, "truncate write-ahead log", error))?;
    if busy != 0 {
        return Err(ReclaimError::new(
            "catalog-busy",
            "truncate write-ahead log: another connection holds a WAL snapshot; legacy column clearing already committed, but shrinking is incomplete; release the reader and retry reclaim",
        ));
    }
    drop(connection);
    Ok(ReclaimApplied {
        database: database.to_path_buf(),
        runs,
        invocation_rows_cleared: cleared as u64,
        before,
        after: sizes(database),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqlitePersistence;
    use loop_core::{
        CreateRunRequest, Lifecycle, Persistence, ProviderAssociation, State, Timestamp,
        Transition, Workflow,
    };
    use serde_json::json;

    fn create(adapter: &SqlitePersistence, id: &str) {
        let workflow = Workflow::new(
            "reclaim-test",
            "start",
            vec![
                State::new("start", "Start", "Begin", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::checked("start", "finish", "done")],
        );
        adapter
            .create_run(CreateRunRequest::new(
                id,
                None,
                workflow,
                ProviderAssociation::new(json!({"provider": "test"})),
                json!({}),
                "start",
                Lifecycle::Active,
                Timestamp::from_unix_millis(1),
                "test",
                None,
            ))
            .expect("create run");
    }

    fn seed(path: &Path, run_id: &str, lifecycle: &str) {
        let connection = Connection::open(path).expect("open");
        connection
            .execute(
                "UPDATE runs SET lifecycle = ?1 WHERE id = ?2",
                (lifecycle, run_id),
            )
            .expect("lifecycle");
        let worker =
            json!([{"command": "pi", "args": ["x".repeat(50_000)], "exit_code": 0}]).to_string();
        for index in 0..3 {
            connection
                .execute(
                    "INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json,
                         instruction_digest, subject, waiter_pid, started_at, allowed_time_ms,
                         capture_dir, inner_workers_json, invocation_input_json, completion_snapshot_json)
                     VALUES (?1, ?2, 'slot', '{\"command\":\"pi\",\"args\":[]}', 'sha256:x', 'subject',
                             0, 1, 1000, '/capture', ?3, '{\"prompt\":\"p\"}', ?3)",
                    (run_id, format!("{run_id}-{index}"), &worker),
                )
                .expect("seed");
        }
    }

    fn payload(path: &Path, run_id: &str) -> Vec<(String, String, Option<String>)> {
        let connection = Connection::open(path).expect("open");
        let mut statement = connection
            .prepare(
                "SELECT inner_workers_json, completion_snapshot_json, invocation_input_json
                 FROM work_slot_invocations WHERE run_id = ?1 ORDER BY invocation_id",
            )
            .expect("prepare");
        statement
            .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    #[test]
    fn preview_writes_nothing_and_apply_clears_only_finished_runs() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("loop.db");
        {
            let adapter = SqlitePersistence::open(&path).expect("open");
            for id in ["final-run", "terminated-run", "active-run"] {
                create(&adapter, id);
            }
        }
        seed(&path, "final-run", "final");
        seed(&path, "terminated-run", "terminated");
        seed(&path, "active-run", "active");
        let main_before = std::fs::read(&path).expect("read");
        let preview = preview(&path).expect("preview");
        assert_eq!(std::fs::read(&path).expect("read"), main_before);
        let ids: Vec<_> = preview.runs.iter().map(|run| run.run_id.as_str()).collect();
        assert_eq!(ids, ["final-run", "terminated-run"]);
        assert_eq!(preview.invocation_rows, 6);
        assert!(preview.reclaimable_bytes > 6 * 100_000);

        let active_before = payload(&path, "active-run");
        let applied = apply(&path).expect("apply");
        assert_eq!(applied.invocation_rows_cleared, 6);
        assert!(applied.after.file_bytes < applied.before.file_bytes);
        for run in ["final-run", "terminated-run"] {
            for row in payload(&path, run) {
                assert_eq!(row, ("[]".into(), "[]".into(), None));
            }
        }
        assert_eq!(payload(&path, "active-run"), active_before);
        // Cleaned runs still decode through the same reader show uses.
        let adapter = SqlitePersistence::open(&path).expect("reopen");
        for id in ["final-run", "terminated-run", "active-run"] {
            adapter
                .load_show_data(&id.into())
                .expect("show data decodes");
        }
        assert!(preview_after(&path).runs.is_empty());
    }

    fn preview_after(path: &Path) -> ReclaimPreview {
        preview(path).expect("preview after apply")
    }

    #[test]
    fn missing_catalog_and_held_lock_report_clear_codes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("absent.db");
        assert_eq!(preview(&missing).unwrap_err().code, "catalog-not-found");
        let path = directory.path().join("loop.db");
        {
            let adapter = SqlitePersistence::open(&path).expect("open");
            create(&adapter, "final-run");
        }
        seed(&path, "final-run", "final");
        let holder = Connection::open(&path).expect("holder");
        holder.execute_batch("BEGIN IMMEDIATE").expect("lock");
        let error = apply(&path).unwrap_err();
        assert_eq!(error.code, "catalog-busy");
        assert!(error.message.contains("retry"));
        holder.execute_batch("ROLLBACK").expect("unlock");
        assert_eq!(payload(&path, "final-run").len(), 3);
        assert!(payload(&path, "final-run").iter().all(|row| row.0 != "[]"));
    }
}
