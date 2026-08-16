//! SQLite implementation of the semantic persistence port.
//!
//! The adapter deliberately stores the workflow, provider association, and
//! input as JSON snapshots.  Current state is authoritative in `runs`; the
//! immutable semantic records live in `context_records` and
//! `history_entries`.  In particular, checked evaluations are reconstructed
//! from transition history rather than written to a second evaluations table.

use loop_core::{
    AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
    CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
    CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult, ContextRecord,
    CreateRunRequest, CreateRunResult, CreateWorkSlotInvocationRequest,
    CreateWorkSlotInvocationResult, DurableEvaluation, DurableEvaluationResult, HistoryAction,
    HistoryEntry, InvocationId, Lifecycle, Persistence, PersistenceConflict, PersistenceError,
    PersistenceFailure, PersistenceRejection, RecordDenialRequest, RecordDenialResult, Run, RunId,
    RunSummary, SemanticSequence, ShowData, StateId, TerminateRequest, TerminateResult, Timestamp,
    TransitionHistoryOutcome, WaiterWrittenStatus, WorkSlotId, WorkSlotInvocation,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

const BUSY_TIMEOUT_MS: u64 = 5_000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id                           TEXT PRIMARY KEY NOT NULL,
    label                        TEXT,
    workflow_id                  TEXT NOT NULL,
    workflow_json                TEXT NOT NULL,
    provider_association_json    TEXT NOT NULL,
    initial_input_json           TEXT NOT NULL,
    current_state                TEXT NOT NULL,
    lifecycle                    TEXT NOT NULL CHECK (lifecycle IN ('active', 'final', 'terminated')),
    control_revision             INTEGER NOT NULL CHECK (control_revision >= 0),
    last_sequence                INTEGER NOT NULL CHECK (last_sequence >= 1),
    created_at                   INTEGER NOT NULL,
    provider                     TEXT,
    artifact_root                TEXT
);

CREATE TABLE IF NOT EXISTS context_records (
    run_id                       TEXT NOT NULL,
    record_id                    TEXT NOT NULL,
    sequence                     INTEGER NOT NULL CHECK (sequence >= 1),
    kind                         TEXT NOT NULL,
    data_json                    TEXT NOT NULL,
    created_at                   INTEGER NOT NULL,
    PRIMARY KEY (run_id, record_id),
    UNIQUE (run_id, sequence),
    FOREIGN KEY (run_id) REFERENCES runs (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS history_entries (
    run_id                       TEXT NOT NULL,
    sequence                     INTEGER NOT NULL CHECK (sequence >= 1),
    occurred_at                  INTEGER NOT NULL,
    action_json                  TEXT NOT NULL,
    PRIMARY KEY (run_id, sequence),
    FOREIGN KEY (run_id) REFERENCES runs (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS context_records_by_run_sequence
    ON context_records (run_id, sequence);
CREATE INDEX IF NOT EXISTS history_entries_by_run_sequence
    ON history_entries (run_id, sequence);

CREATE TABLE IF NOT EXISTS work_slot_invocations (
    run_id                       TEXT NOT NULL,
    invocation_id                TEXT NOT NULL,
    slot_id                      TEXT NOT NULL,
    binding_json                 TEXT NOT NULL,
    instruction_digest           TEXT NOT NULL,
    subject                      TEXT NOT NULL,
    waiter_pid                   INTEGER NOT NULL CHECK (waiter_pid >= 0),
    started_at                   INTEGER NOT NULL,
    allowed_time_ms              INTEGER NOT NULL CHECK (allowed_time_ms >= 0),
    status                       TEXT CHECK (status IS NULL OR status IN ('succeeded', 'failed')),
    exit_code                    INTEGER,
    completed_at                 INTEGER,
    PRIMARY KEY (run_id, invocation_id),
    FOREIGN KEY (run_id) REFERENCES runs (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS slot_subjects (
    run_id                       TEXT NOT NULL,
    slot_id                      TEXT NOT NULL,
    subject                      TEXT NOT NULL,
    PRIMARY KEY (run_id, slot_id),
    FOREIGN KEY (run_id) REFERENCES runs (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS work_slot_invocations_by_run
    ON work_slot_invocations (run_id, started_at, invocation_id);
"#;

/// A synchronous, file-backed SQLite implementation of [`Persistence`].
///
/// Each instance owns one configured SQLite connection.  The connection is
/// protected by a mutex because the core port is synchronous and takes `&self`;
/// separate instances can therefore be opened against the same database for
/// cross-process-style conditional-write tests.
pub struct SqlitePersistence {
    connection: Mutex<Connection>,
}

impl SqlitePersistence {
    /// Open (or create) a SQLite database at `path` and bootstrap its schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let connection = Connection::open(path).map_err(sqlite_failure)?;
        Self::from_connection(connection)
    }

    /// Open an in-memory database.  This is useful for fast adapter tests;
    /// file-backed callers should use [`Self::open`] when restart durability
    /// or multiple independent instances are required.
    pub fn open_in_memory() -> Result<Self, PersistenceError> {
        let connection = Connection::open_in_memory().map_err(sqlite_failure)?;
        Self::from_connection(connection)
    }

    /// Configure an existing rusqlite connection and bootstrap the schema.
    ///
    /// This is intentionally an adapter constructor rather than a generic
    /// transaction escape hatch: all semantic writes still go through the
    /// [`Persistence`] methods below.
    pub fn from_connection(connection: Connection) -> Result<Self, PersistenceError> {
        configure_connection(&connection).map_err(sqlite_failure)?;
        ensure_run_catalog_columns(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PersistenceError> {
        self.connection.lock().map_err(|_| {
            PersistenceError::failure(PersistenceFailure::new(
                "sqlite-lock",
                "SQLite connection mutex was poisoned",
            ))
        })
    }
}

impl Persistence for SqlitePersistence {
    fn create_run(&self, request: CreateRunRequest) -> Result<CreateRunResult, PersistenceError> {
        let workflow_json = encode_json(&request.workflow, "workflow")?;
        let provider_json = encode_json(&request.provider_association, "provider association")?;
        let initial_input_json = encode_json(&request.initial_input, "initial input")?;
        let lifecycle = lifecycle_name(request.lifecycle);
        let initial_sequence = SemanticSequence::new(1);
        let initial_revision = loop_core::ControlRevision::from_u64(0);
        let history = HistoryEntry::run_created(initial_sequence, request.created_at);

        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            transaction
                .execute(
                    "INSERT INTO runs (
                        id, label, workflow_id, workflow_json,
                        provider_association_json, initial_input_json,
                        current_state, lifecycle, control_revision,
                        last_sequence, created_at, provider, artifact_root
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        request.id.as_str(),
                        request.label,
                        request.workflow.id.as_str(),
                        workflow_json,
                        provider_json,
                        initial_input_json,
                        request.initial_state.as_str(),
                        lifecycle,
                        to_sqlite_i64(initial_revision.as_u64(), "control revision")?,
                        to_sqlite_i64(initial_sequence.as_u64(), "semantic sequence")?,
                        request.created_at.as_unix_millis(),
                        request.provider,
                        request.artifact_root,
                    ],
                )
                .map_err(sqlite_failure)?;
            insert_history(&transaction, &request.id, &history)?;
            upsert_slot_subjects(&transaction, &request.id, &request.slot_subjects)?;

            let run = Run::new(
                request.id,
                request.label,
                request.workflow,
                request.provider_association,
                request.initial_input,
                request.initial_state,
                request.lifecycle,
                initial_revision,
                initial_sequence,
                request.created_at,
            );
            Ok(CreateRunResult { run, history })
        })();
        finish_transaction(transaction, result)
    }

    fn append_context(
        &self,
        request: AppendContextRequest,
    ) -> Result<AppendContextResult, PersistenceError> {
        let data_json = encode_json(&request.data, "context data")?;
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;
            require_active(&run)?;

            let sequence = next_sequence(run.last_sequence)?;
            let context = ContextRecord::new(
                request.record_id.clone(),
                request.kind.clone(),
                request.data.clone(),
                sequence,
                request.created_at,
            );
            transaction
                .execute(
                    "INSERT INTO context_records (
                        run_id, record_id, sequence, kind, data_json, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        request.run_id.as_str(),
                        request.record_id.as_str(),
                        to_sqlite_i64(sequence.as_u64(), "semantic sequence")?,
                        request.kind,
                        data_json,
                        request.created_at.as_unix_millis(),
                    ],
                )
                .map_err(sqlite_failure)?;

            let history = HistoryEntry::context_appended(
                sequence,
                request.created_at,
                request.record_id.clone(),
            );
            insert_history(&transaction, &request.run_id, &history)?;
            update_last_sequence(&transaction, &request.run_id, sequence)?;
            let run = load_required_run(&transaction, &request.run_id)?;
            Ok(AppendContextResult {
                run,
                context,
                history,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn commit_transition(
        &self,
        request: CommitTransitionRequest,
    ) -> Result<CommitTransitionResult, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;
            if !run.lifecycle.is_active() {
                return Err(PersistenceError::conflict(
                    PersistenceConflict::LifecycleMismatch {
                        expected: Lifecycle::Active,
                        observed: run.lifecycle,
                    },
                ));
            }
            verify_revision_and_source(
                &run,
                request.expected_control_revision,
                &request.expected_source_state,
            )?;

            let sequence = next_sequence(run.last_sequence)?;
            let revision = next_revision(run.control_revision)?;
            let occurred_at = current_timestamp()?;
            let history = HistoryEntry::transition(
                sequence,
                occurred_at,
                request.transition.clone(),
                TransitionHistoryOutcome::Committed,
            );
            transaction
                .execute(
                    "UPDATE runs
                     SET current_state = ?1,
                         lifecycle = ?2,
                         control_revision = ?3,
                         last_sequence = ?4
                     WHERE id = ?5",
                    params![
                        request.transition.target.as_str(),
                        lifecycle_name(request.resulting_lifecycle),
                        to_sqlite_i64(revision.as_u64(), "control revision")?,
                        to_sqlite_i64(sequence.as_u64(), "semantic sequence")?,
                        request.run_id.as_str(),
                    ],
                )
                .map_err(sqlite_failure)?;
            insert_history(&transaction, &request.run_id, &history)?;
            upsert_slot_subjects(&transaction, &request.run_id, &request.slot_subjects)?;
            let run = load_required_run(&transaction, &request.run_id)?;
            Ok(CommitTransitionResult { run, history })
        })();
        finish_transaction(transaction, result)
    }

    fn record_denial(
        &self,
        request: RecordDenialRequest,
    ) -> Result<RecordDenialResult, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;
            if !run.lifecycle.is_active() {
                return Err(PersistenceError::conflict(
                    PersistenceConflict::LifecycleMismatch {
                        expected: Lifecycle::Active,
                        observed: run.lifecycle,
                    },
                ));
            }
            verify_revision_and_source(
                &run,
                request.expected_control_revision,
                &request.expected_source_state,
            )?;

            let sequence = next_sequence(run.last_sequence)?;
            let occurred_at = current_timestamp()?;
            let feedback = request.feedback.clone();
            let evaluation = DurableEvaluation::deny(
                request.transition.clone(),
                feedback.clone(),
                sequence,
                occurred_at,
            );
            let history = HistoryEntry::transition(
                sequence,
                occurred_at,
                request.transition,
                TransitionHistoryOutcome::Denied { feedback },
            );
            update_last_sequence(&transaction, &request.run_id, sequence)?;
            insert_history(&transaction, &request.run_id, &history)?;
            let run = load_required_run(&transaction, &request.run_id)?;
            Ok(RecordDenialResult {
                run,
                evaluation,
                history,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn terminate(&self, request: TerminateRequest) -> Result<TerminateResult, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;
            require_active(&run)?;

            let sequence = next_sequence(run.last_sequence)?;
            let revision = next_revision(run.control_revision)?;
            let occurred_at = current_timestamp()?;
            let history = HistoryEntry::terminated(sequence, occurred_at);
            transaction
                .execute(
                    "UPDATE runs
                     SET lifecycle = 'terminated',
                         control_revision = ?1,
                         last_sequence = ?2
                     WHERE id = ?3",
                    params![
                        to_sqlite_i64(revision.as_u64(), "control revision")?,
                        to_sqlite_i64(sequence.as_u64(), "semantic sequence")?,
                        request.run_id.as_str(),
                    ],
                )
                .map_err(sqlite_failure)?;
            insert_history(&transaction, &request.run_id, &history)?;
            let run = load_required_run(&transaction, &request.run_id)?;
            Ok(TerminateResult { run, history })
        })();
        finish_transaction(transaction, result)
    }

    fn load_authoritative_run(&self, run_id: &RunId) -> Result<Run, PersistenceError> {
        let connection = self.lock()?;
        load_required_run(&connection, run_id)
    }

    fn list_runs(&self) -> Result<Vec<RunSummary>, PersistenceError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, label, workflow_id, lifecycle, current_state, provider, artifact_root
                 FROM runs
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(sqlite_failure)?;
        let rows = statement
            .query_map([], |row| {
                let lifecycle: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    lifecycle,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(sqlite_failure)?;

        rows.map(|row| {
            let (id, label, workflow_id, lifecycle, current_state, provider, artifact_root) =
                row.map_err(sqlite_failure)?;
            Ok(RunSummary {
                id: RunId::new(id),
                label,
                workflow_id: loop_core::WorkflowId::new(workflow_id),
                lifecycle: parse_lifecycle(&lifecycle)?,
                current_state: StateId::new(current_state),
                provider,
                artifact_root,
            })
        })
        .collect()
    }

    fn load_context_records(&self, run_id: &RunId) -> Result<Vec<ContextRecord>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        read_context_records(&connection, run_id)
    }

    fn load_history(&self, run_id: &RunId) -> Result<Vec<HistoryEntry>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        read_history_entries(&connection, run_id)
    }

    fn load_checked_evaluations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<DurableEvaluation>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        read_checked_evaluations(&connection, run_id)
    }

    fn load_checked_evaluation_snapshot(
        &self,
        request: CheckedEvaluationSnapshotRequest,
    ) -> Result<CheckedEvaluationSnapshot, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(sqlite_failure)?;
        let result = (|| {
            let run = load_required_run(&transaction, &request.run_id)?;
            require_active(&run)?;
            if run.current_state != request.transition.source
                || !run
                    .workflow
                    .transitions
                    .iter()
                    .any(|candidate| candidate == &request.transition)
            {
                return Err(PersistenceError::conflict(
                    PersistenceConflict::ExactTransitionUnavailable {
                        expected: request.transition.clone(),
                        observed_current_state: run.current_state.clone(),
                    },
                ));
            }

            let context = read_context_records(&transaction, &request.run_id)?;
            let checked_evaluations = read_checked_evaluations(&transaction, &request.run_id)?;
            Ok(CheckedEvaluationSnapshot {
                observed_control_revision: run.control_revision,
                transition: request.transition,
                run,
                context,
                checked_evaluations,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn load_show_data(&self, run_id: &RunId) -> Result<ShowData, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(sqlite_failure)?;
        let result = (|| {
            let run = load_required_run(&transaction, run_id)?;
            let context = read_context_records(&transaction, run_id)?;
            let checked_evaluations = read_checked_evaluations(&transaction, run_id)?;
            Ok(ShowData {
                run,
                context,
                checked_evaluations,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn create_work_slot_invocation(
        &self,
        request: CreateWorkSlotInvocationRequest,
    ) -> Result<CreateWorkSlotInvocationResult, PersistenceError> {
        let binding_json = encode_json(&request.binding, "work slot binding")?;
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;
            require_active(&run)?;

            let sequence = next_sequence(run.last_sequence)?;
            let invocation = WorkSlotInvocation::new(
                request.invocation_id.clone(),
                request.slot_id.clone(),
                request.binding.clone(),
                request.instruction_digest.clone(),
                request.subject.clone(),
                request.waiter_pid,
                request.started_at,
                request.allowed_time_ms,
                None,
                None,
                None,
            );
            transaction
                .execute(
                    "INSERT INTO work_slot_invocations (
                        run_id, invocation_id, slot_id, binding_json,
                        instruction_digest, subject, waiter_pid, started_at,
                        allowed_time_ms, status, exit_code, completed_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, NULL)",
                    params![
                        request.run_id.as_str(),
                        request.invocation_id.as_str(),
                        request.slot_id.as_str(),
                        binding_json,
                        request.instruction_digest,
                        request.subject,
                        to_sqlite_i64(u64::from(request.waiter_pid), "waiter pid")?,
                        request.started_at.as_unix_millis(),
                        to_sqlite_i64(request.allowed_time_ms, "allowed time ms")?,
                    ],
                )
                .map_err(sqlite_failure)?;

            let history = HistoryEntry::invocation_started(
                sequence,
                request.started_at,
                request.invocation_id.clone(),
            );
            insert_history(&transaction, &request.run_id, &history)?;
            update_last_sequence(&transaction, &request.run_id, sequence)?;
            Ok(CreateWorkSlotInvocationResult {
                invocation,
                history,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn complete_work_slot_invocation(
        &self,
        request: CompleteWorkSlotInvocationRequest,
    ) -> Result<CompleteWorkSlotInvocationResult, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;

            let changed = transaction
                .execute(
                    "UPDATE work_slot_invocations
                     SET status = ?1, exit_code = ?2, completed_at = ?3
                     WHERE run_id = ?4 AND invocation_id = ?5 AND status IS NULL",
                    params![
                        waiter_status_name(request.status),
                        request.exit_code,
                        request.completed_at.as_unix_millis(),
                        request.run_id.as_str(),
                        request.invocation_id.as_str(),
                    ],
                )
                .map_err(sqlite_failure)?;
            if changed != 1 {
                return match load_one_work_slot_invocation(
                    &transaction,
                    &request.run_id,
                    &request.invocation_id,
                )? {
                    Some(existing) if existing.status.is_some() => Err(PersistenceError::conflict(
                        PersistenceConflict::InvocationAlreadyTerminal {
                            invocation_id: request.invocation_id.clone(),
                        },
                    )),
                    Some(_) | None => Err(PersistenceError::failure(PersistenceFailure::new(
                        "invocation-not-found",
                        format!(
                            "invocation `{}` was not found on run `{}`",
                            request.invocation_id, request.run_id
                        ),
                    ))),
                };
            }

            let sequence = next_sequence(run.last_sequence)?;
            let history = HistoryEntry::invocation_status_changed(
                sequence,
                request.completed_at,
                request.invocation_id.clone(),
                request.status,
            );
            insert_history(&transaction, &request.run_id, &history)?;
            update_last_sequence(&transaction, &request.run_id, sequence)?;
            let invocation = load_one_work_slot_invocation(
                &transaction,
                &request.run_id,
                &request.invocation_id,
            )?
            .ok_or_else(|| {
                PersistenceError::failure(PersistenceFailure::new(
                    "invocation-not-found",
                    format!(
                        "invocation `{}` was not found on run `{}` after terminal write",
                        request.invocation_id, request.run_id
                    ),
                ))
            })?;
            Ok(CompleteWorkSlotInvocationResult {
                invocation,
                history,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn get_current_slot_subject(
        &self,
        run_id: &RunId,
        slot_id: &WorkSlotId,
    ) -> Result<Option<String>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        connection
            .query_row(
                "SELECT subject FROM slot_subjects WHERE run_id = ?1 AND slot_id = ?2",
                params![run_id.as_str(), slot_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_failure)
    }

    fn set_current_slot_subject(
        &self,
        run_id: &RunId,
        slot_id: &WorkSlotId,
        subject: String,
    ) -> Result<(), PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let _ = load_required_run(&transaction, run_id)?;
            upsert_slot_subject(&transaction, run_id, slot_id, &subject)?;
            Ok(())
        })();
        finish_transaction(transaction, result)
    }

    fn load_work_slot_invocations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<WorkSlotInvocation>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        read_work_slot_invocations(&connection, run_id)
    }
}

fn upsert_slot_subjects(
    transaction: &Transaction<'_>,
    run_id: &RunId,
    subjects: &[(WorkSlotId, String)],
) -> Result<(), PersistenceError> {
    for (slot_id, subject) in subjects {
        upsert_slot_subject(transaction, run_id, slot_id, subject)?;
    }
    Ok(())
}

fn upsert_slot_subject(
    transaction: &Transaction<'_>,
    run_id: &RunId,
    slot_id: &WorkSlotId,
    subject: &str,
) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "INSERT INTO slot_subjects (run_id, slot_id, subject)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (run_id, slot_id) DO UPDATE SET subject = excluded.subject",
            params![run_id.as_str(), slot_id.as_str(), subject],
        )
        .map_err(sqlite_failure)?;
    Ok(())
}

fn configure_connection(connection: &Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA synchronous = FULL;
         PRAGMA busy_timeout = 5000;",
    )?;
    // `journal_mode` is a query pragma.  Reading its result also makes a
    // failure to enter WAL visible to the constructor instead of silently
    // continuing with a weaker journal mode.
    let _: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    connection.execute_batch(SCHEMA)
}

fn ensure_run_catalog_columns(connection: &Connection) -> Result<(), PersistenceError> {
    let columns = run_table_columns(connection)?;
    if !columns.iter().any(|name| name == "provider") {
        connection
            .execute("ALTER TABLE runs ADD COLUMN provider TEXT", [])
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "artifact_root") {
        connection
            .execute("ALTER TABLE runs ADD COLUMN artifact_root TEXT", [])
            .map_err(sqlite_failure)?;
    }
    Ok(())
}

fn run_table_columns(connection: &Connection) -> Result<Vec<String>, PersistenceError> {
    let mut statement = connection
        .prepare("PRAGMA table_info('runs')")
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_failure)?;
    rows.map(|row| row.map_err(sqlite_failure)).collect()
}

fn begin_immediate(connection: &mut Connection) -> Result<Transaction<'_>, PersistenceError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_failure)
}

fn finish_transaction<T>(
    transaction: Transaction<'_>,
    result: Result<T, PersistenceError>,
) -> Result<T, PersistenceError> {
    match result {
        Ok(value) => transaction.commit().map(|()| value).map_err(sqlite_failure),
        Err(error) => {
            let _ = transaction.rollback();
            Err(error)
        }
    }
}

#[derive(Debug)]
struct StoredRun {
    id: String,
    label: Option<String>,
    workflow_json: String,
    provider_association_json: String,
    initial_input_json: String,
    current_state: String,
    lifecycle: String,
    control_revision: i64,
    last_sequence: i64,
    created_at: i64,
}

fn load_raw_run(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Option<StoredRun>, PersistenceError> {
    connection
        .query_row(
            "SELECT id, label, workflow_json, provider_association_json,
                    initial_input_json, current_state, lifecycle,
                    control_revision, last_sequence, created_at
             FROM runs WHERE id = ?1",
            params![run_id.as_str()],
            |row| {
                Ok(StoredRun {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    workflow_json: row.get(2)?,
                    provider_association_json: row.get(3)?,
                    initial_input_json: row.get(4)?,
                    current_state: row.get(5)?,
                    lifecycle: row.get(6)?,
                    control_revision: row.get(7)?,
                    last_sequence: row.get(8)?,
                    created_at: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(sqlite_failure)
}

fn load_required_run(connection: &Connection, run_id: &RunId) -> Result<Run, PersistenceError> {
    let raw = load_raw_run(connection, run_id)?
        .ok_or_else(|| PersistenceError::not_found(run_id.clone()))?;
    decode_run(raw)
}

fn decode_run(raw: StoredRun) -> Result<Run, PersistenceError> {
    let workflow = decode_json(&raw.workflow_json, "workflow")?;
    let provider_association = decode_json(&raw.provider_association_json, "provider association")?;
    let initial_input = decode_json(&raw.initial_input_json, "initial input")?;
    let control_revision = from_sqlite_u64(raw.control_revision, "control revision")?;
    let last_sequence = from_sqlite_u64(raw.last_sequence, "semantic sequence")?;
    Ok(Run::new(
        RunId::new(raw.id),
        raw.label,
        workflow,
        loop_core::ProviderAssociation::new(provider_association),
        initial_input,
        StateId::new(raw.current_state),
        parse_lifecycle(&raw.lifecycle)?,
        control_revision.into(),
        last_sequence.into(),
        Timestamp::from_unix_millis(raw.created_at),
    ))
}

fn require_active(run: &Run) -> Result<(), PersistenceError> {
    if run.lifecycle.is_active() {
        Ok(())
    } else {
        Err(PersistenceError::rejected(
            PersistenceRejection::RunNotActive {
                run_id: run.id.clone(),
                lifecycle: run.lifecycle,
            },
        ))
    }
}

fn verify_revision_and_source(
    run: &Run,
    expected_revision: loop_core::ControlRevision,
    expected_source: &StateId,
) -> Result<(), PersistenceError> {
    if run.control_revision != expected_revision {
        return Err(PersistenceError::conflict(
            PersistenceConflict::ControlRevisionMismatch {
                expected: expected_revision,
                observed: run.control_revision,
            },
        ));
    }
    if &run.current_state != expected_source {
        return Err(PersistenceError::conflict(
            PersistenceConflict::SourceStateMismatch {
                expected: expected_source.clone(),
                observed: run.current_state.clone(),
            },
        ));
    }
    Ok(())
}

fn update_last_sequence(
    transaction: &Transaction<'_>,
    run_id: &RunId,
    sequence: SemanticSequence,
) -> Result<(), PersistenceError> {
    let changed = transaction
        .execute(
            "UPDATE runs SET last_sequence = ?1 WHERE id = ?2",
            params![
                to_sqlite_i64(sequence.as_u64(), "semantic sequence")?,
                run_id.as_str()
            ],
        )
        .map_err(sqlite_failure)?;
    if changed != 1 {
        return Err(sqlite_failure("updating run sequence affected no run"));
    }
    Ok(())
}

fn insert_history(
    transaction: &Transaction<'_>,
    run_id: &RunId,
    history: &HistoryEntry,
) -> Result<(), PersistenceError> {
    let action_json = encode_json(&history.action, "history action")?;
    transaction
        .execute(
            "INSERT INTO history_entries (run_id, sequence, occurred_at, action_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run_id.as_str(),
                to_sqlite_i64(history.sequence.as_u64(), "semantic sequence")?,
                history.occurred_at.as_unix_millis(),
                action_json,
            ],
        )
        .map_err(sqlite_failure)?;
    Ok(())
}

fn read_context_records(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Vec<ContextRecord>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT record_id, sequence, kind, data_json, created_at
             FROM context_records
             WHERE run_id = ?1
             ORDER BY sequence ASC",
        )
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(sqlite_failure)?;

    rows.map(|row| {
        let (id, sequence, kind, data_json, created_at) = row.map_err(sqlite_failure)?;
        Ok(ContextRecord::new(
            id,
            kind,
            decode_json(&data_json, "context data")?,
            from_sqlite_u64(sequence, "semantic sequence")?.into(),
            Timestamp::from_unix_millis(created_at),
        ))
    })
    .collect()
}

fn read_history_entries(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Vec<HistoryEntry>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT sequence, occurred_at, action_json
             FROM history_entries
             WHERE run_id = ?1
             ORDER BY sequence ASC",
        )
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sqlite_failure)?;

    rows.map(|row| {
        let (sequence, occurred_at, action_json) = row.map_err(sqlite_failure)?;
        Ok(HistoryEntry::new(
            from_sqlite_u64(sequence, "semantic sequence")?.into(),
            Timestamp::from_unix_millis(occurred_at),
            decode_json(&action_json, "history action")?,
        ))
    })
    .collect()
}

fn read_checked_evaluations(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Vec<DurableEvaluation>, PersistenceError> {
    read_history_entries(connection, run_id)?
        .into_iter()
        .filter_map(|entry| match entry.action {
            HistoryAction::Transition {
                transition,
                outcome,
            } if transition.kind.is_checked() => {
                let evaluation = match outcome {
                    TransitionHistoryOutcome::Committed => DurableEvaluation {
                        transition,
                        result: DurableEvaluationResult::Allow,
                        sequence: entry.sequence,
                        occurred_at: entry.occurred_at,
                    },
                    TransitionHistoryOutcome::Denied { feedback } => DurableEvaluation {
                        transition,
                        result: DurableEvaluationResult::Deny { feedback },
                        sequence: entry.sequence,
                        occurred_at: entry.occurred_at,
                    },
                };
                Some(Ok(evaluation))
            }
            _ => None,
        })
        .collect()
}

fn next_sequence(last_sequence: SemanticSequence) -> Result<SemanticSequence, PersistenceError> {
    last_sequence
        .as_u64()
        .checked_add(1)
        .map(SemanticSequence::new)
        .ok_or_else(|| {
            PersistenceError::failure(PersistenceFailure::new(
                "sqlite-sequence-overflow",
                "semantic sequence cannot advance beyond u64::MAX",
            ))
        })
}

fn next_revision(
    revision: loop_core::ControlRevision,
) -> Result<loop_core::ControlRevision, PersistenceError> {
    revision
        .as_u64()
        .checked_add(1)
        .map(loop_core::ControlRevision::from_u64)
        .ok_or_else(|| {
            PersistenceError::failure(PersistenceFailure::new(
                "sqlite-revision-overflow",
                "control revision cannot advance beyond u64::MAX",
            ))
        })
}

fn current_timestamp() -> Result<Timestamp, PersistenceError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            PersistenceError::failure(PersistenceFailure::new(
                "sqlite-clock",
                format!("system clock is before Unix epoch: {error}"),
            ))
        })?;
    let millis = i64::try_from(duration.as_millis()).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-clock",
            "system clock timestamp does not fit SQLite INTEGER",
        ))
    })?;
    Ok(Timestamp::from_unix_millis(millis))
}

fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

fn parse_lifecycle(value: &str) -> Result<Lifecycle, PersistenceError> {
    match value {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        _ => Err(PersistenceError::failure(PersistenceFailure::new(
            "sqlite-invalid-lifecycle",
            format!("unknown lifecycle `{value}` in runs"),
        ))),
    }
}

fn encode_json<T: Serialize>(value: &T, field: &str) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|error| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-serialization",
            format!("could not serialize {field}: {error}"),
        ))
    })
}

fn decode_json<T: DeserializeOwned>(value: &str, field: &str) -> Result<T, PersistenceError> {
    serde_json::from_str(value).map_err(|error| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-deserialization",
            format!("could not deserialize {field}: {error}"),
        ))
    })
}

fn to_sqlite_i64(value: u64, field: &str) -> Result<i64, PersistenceError> {
    i64::try_from(value).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-integer-overflow",
            format!("{field} does not fit SQLite INTEGER"),
        ))
    })
}

fn from_sqlite_u64(value: i64, field: &str) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-invalid-integer",
            format!("{field} is negative in SQLite"),
        ))
    })
}

fn waiter_status_name(status: WaiterWrittenStatus) -> &'static str {
    match status {
        WaiterWrittenStatus::Succeeded => "succeeded",
        WaiterWrittenStatus::Failed => "failed",
    }
}

fn parse_waiter_status(value: &str) -> Result<WaiterWrittenStatus, PersistenceError> {
    match value {
        "succeeded" => Ok(WaiterWrittenStatus::Succeeded),
        "failed" => Ok(WaiterWrittenStatus::Failed),
        _ => Err(PersistenceError::failure(PersistenceFailure::new(
            "sqlite-invalid-invocation-status",
            format!("unknown waiter-written status `{value}`"),
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_work_slot_invocation(
    invocation_id: String,
    slot_id: String,
    binding_json: String,
    instruction_digest: String,
    subject: String,
    waiter_pid: i64,
    started_at: i64,
    allowed_time_ms: i64,
    status: Option<String>,
    exit_code: Option<i64>,
    completed_at: Option<i64>,
) -> Result<WorkSlotInvocation, PersistenceError> {
    let waiter_pid = u32::try_from(from_sqlite_u64(waiter_pid, "waiter pid")?).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-integer-overflow",
            "waiter pid does not fit u32",
        ))
    })?;
    let allowed_time_ms = from_sqlite_u64(allowed_time_ms, "allowed time ms")?;
    let status = status.as_deref().map(parse_waiter_status).transpose()?;
    let exit_code = exit_code
        .map(|code| {
            i32::try_from(code).map_err(|_| {
                PersistenceError::failure(PersistenceFailure::new(
                    "sqlite-integer-overflow",
                    "exit code does not fit i32",
                ))
            })
        })
        .transpose()?;
    Ok(WorkSlotInvocation::new(
        InvocationId::new(invocation_id),
        WorkSlotId::new(slot_id),
        decode_json(&binding_json, "work slot binding")?,
        instruction_digest,
        subject,
        waiter_pid,
        Timestamp::from_unix_millis(started_at),
        allowed_time_ms,
        status,
        exit_code,
        completed_at.map(Timestamp::from_unix_millis),
    ))
}

type InvocationRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

fn invocation_row_from_query(row: &rusqlite::Row<'_>) -> rusqlite::Result<InvocationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}

fn load_one_work_slot_invocation(
    connection: &Connection,
    run_id: &RunId,
    invocation_id: &InvocationId,
) -> Result<Option<WorkSlotInvocation>, PersistenceError> {
    connection
        .query_row(
            "SELECT invocation_id, slot_id, binding_json, instruction_digest, subject,
                    waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at
             FROM work_slot_invocations
             WHERE run_id = ?1 AND invocation_id = ?2",
            params![run_id.as_str(), invocation_id.as_str()],
            invocation_row_from_query,
        )
        .optional()
        .map_err(sqlite_failure)?
        .map(
            |(
                invocation_id,
                slot_id,
                binding_json,
                instruction_digest,
                subject,
                waiter_pid,
                started_at,
                allowed_time_ms,
                status,
                exit_code,
                completed_at,
            )| {
                decode_work_slot_invocation(
                    invocation_id,
                    slot_id,
                    binding_json,
                    instruction_digest,
                    subject,
                    waiter_pid,
                    started_at,
                    allowed_time_ms,
                    status,
                    exit_code,
                    completed_at,
                )
            },
        )
        .transpose()
}

fn read_work_slot_invocations(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Vec<WorkSlotInvocation>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT invocation_id, slot_id, binding_json, instruction_digest, subject,
                    waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at
             FROM work_slot_invocations
             WHERE run_id = ?1
             ORDER BY started_at ASC, invocation_id ASC",
        )
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str()], invocation_row_from_query)
        .map_err(sqlite_failure)?;

    rows.map(|row| {
        let (
            invocation_id,
            slot_id,
            binding_json,
            instruction_digest,
            subject,
            waiter_pid,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
        ) = row.map_err(sqlite_failure)?;
        decode_work_slot_invocation(
            invocation_id,
            slot_id,
            binding_json,
            instruction_digest,
            subject,
            waiter_pid,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
        )
    })
    .collect()
}

fn sqlite_failure(error: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new("sqlite", error.to_string()))
}
