//! SQLite implementation of the semantic persistence port.
//!
//! The adapter deliberately stores the workflow, provider association, and
//! input as JSON snapshots.  Current state is authoritative in `runs`; the
//! immutable semantic records live in `context_records` and
//! `history_entries`.  In particular, checked evaluations are reconstructed
//! from transition history rather than written to a second evaluations table.

#[path = "sqlite_observation.rs"]
mod sqlite_observation;
#[path = "sqlite_reads.rs"]
mod sqlite_reads;

use loop_core::{
    AppendAdviceAttemptRequest, AppendContextRequest, AppendContextResult,
    CheckedEvaluationSnapshot, CheckedEvaluationSnapshotRequest, CommitTransitionRequest,
    CommitTransitionResult, CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult,
    ContextRecord, CreateRunRequest, CreateRunResult, CreateWorkSlotInvocationRequest,
    CreateWorkSlotInvocationResult, DriverActEvidence, DurableEvaluation, DurableEvaluationResult,
    HistoryAction, HistoryEntry, InnerWorker, InvocationId, Lifecycle, Persistence,
    PersistenceConflict, PersistenceError, PersistenceFailure, PersistenceRejection,
    RecordDenialRequest, RecordDenialResult, Run, RunId, RunSummary, SemanticSequence, ShowData,
    StateId, TerminateRequest, TerminateResult, Timestamp, TransitionHistoryOutcome,
    WaiterWrittenStatus, WorkSlotId, WorkSlotInvocation,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

const BUSY_TIMEOUT_MS: u64 = 5_000;
const COMPLETION_SNAPSHOT_REF_PREFIX: &str = "@inner-workers-sha256:";

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
    observed_control_revision    INTEGER CHECK (observed_control_revision >= 0),
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
    state_visit                  INTEGER NOT NULL DEFAULT 0,
    waiter_pid                   INTEGER NOT NULL CHECK (waiter_pid >= 0),
    waiter_identity_json         TEXT,
    started_at                   INTEGER NOT NULL,
    allowed_time_ms              INTEGER NOT NULL CHECK (allowed_time_ms >= 0),
    status                       TEXT CHECK (status IS NULL OR status IN ('succeeded', 'failed')),
    exit_code                    INTEGER,
    completed_at                 INTEGER,
    capture_dir                  TEXT NOT NULL DEFAULT '',
    inner_workers_json           TEXT NOT NULL DEFAULT '[]',
    assignment_selection_json    TEXT,
    invocation_input_json        TEXT,
    routed_inputs_json           TEXT NOT NULL DEFAULT '[]',
    frozen_run_identity_json     TEXT,
    completion_snapshot_json     TEXT NOT NULL DEFAULT '[]',
    assignment_labels_json      TEXT NOT NULL DEFAULT '[]',
    assignment_facts_version    INTEGER NOT NULL DEFAULT 0,
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
CREATE INDEX IF NOT EXISTS work_slot_invocations_by_pending
    ON work_slot_invocations (run_id, status, started_at, invocation_id);

CREATE TABLE IF NOT EXISTS work_slot_assignment_facts (
    run_id                       TEXT NOT NULL,
    invocation_id                TEXT NOT NULL,
    slot_id                      TEXT NOT NULL,
    subject                      TEXT NOT NULL,
    assignment_id                TEXT NOT NULL,
    title                        TEXT,
    role                         TEXT,
    execution_state              TEXT NOT NULL,
    conformance_state            TEXT NOT NULL,
    conformance_error            TEXT,
    conformance_error_truncated  INTEGER NOT NULL DEFAULT 0,
    facts_complete               INTEGER NOT NULL DEFAULT 0,
    attempts_path                TEXT,
    PRIMARY KEY (run_id, invocation_id, assignment_id),
    FOREIGN KEY (run_id, invocation_id) REFERENCES work_slot_invocations (run_id, invocation_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS work_slot_assignment_attempts (
    run_id                       TEXT NOT NULL,
    invocation_id                TEXT NOT NULL,
    assignment_id                TEXT NOT NULL,
    attempt_number               INTEGER NOT NULL CHECK (attempt_number >= 1),
    failed                       INTEGER NOT NULL CHECK (failed IN (0, 1)),
    errors_json                  TEXT NOT NULL DEFAULT '[]',
    errors_truncated             INTEGER NOT NULL DEFAULT 0,
    stdout_path                  TEXT,
    stderr_path                  TEXT,
    PRIMARY KEY (run_id, invocation_id, assignment_id, attempt_number),
    FOREIGN KEY (run_id, invocation_id, assignment_id)
        REFERENCES work_slot_assignment_facts (run_id, invocation_id, assignment_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS assignment_facts_by_subject
    ON work_slot_assignment_facts (run_id, slot_id, subject, assignment_id, invocation_id);
CREATE INDEX IF NOT EXISTS assignment_facts_by_assignment
    ON work_slot_assignment_facts (run_id, assignment_id, invocation_id);
CREATE INDEX IF NOT EXISTS assignment_attempts_by_assignment
    ON work_slot_assignment_attempts (run_id, assignment_id, attempt_number, invocation_id);
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

/// Bounded provider-free target selector for semantic history, assignment
/// attempts/errors, and original captured stream bytes.
#[derive(Clone, Debug, Default)]
pub struct TargetedReadRequest {
    pub kind: String,
    pub assignment_id: Option<String>,
    pub invocation_id: Option<String>,
    pub stream: Option<String>,
    pub attempt: Option<u32>,
    /// Numeric sequence cursor for history, row offset for assignment/attempt pages.
    pub cursor: Option<String>,
    /// Original-stream byte offset.
    pub offset: u64,
    pub limit: u32,
}

impl SqlitePersistence {
    /// Open (or create) a SQLite database at `path` and bootstrap its schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let connection = Connection::open(path).map_err(sqlite_failure)?;
        Self::from_connection(connection)
    }

    /// Open an existing catalog for one deadline-bounded observation without
    /// running schema setup or migrations. Status is read-only; action mode may
    /// arm the observed revision but still performs no provider call.
    pub fn open_for_observation(
        path: impl AsRef<Path>,
        deadline: std::time::Instant,
        arm: bool,
    ) -> Result<Self, PersistenceError> {
        let flags = if arm {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let connection = Connection::open_with_flags(path, flags).map_err(sqlite_failure)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(status_read_deadline());
        }
        connection.busy_timeout(remaining).map_err(sqlite_failure)?;
        connection
            .progress_handler(1_000, Some(move || std::time::Instant::now() >= deadline))
            .map_err(sqlite_failure)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
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
        ensure_work_slot_invocation_columns(&connection)?;
        ensure_work_slot_assignment_columns(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Read one cancellation target without decoding unrelated invocation
    /// captures. Callers still perform current-visit and ownership checks.
    pub fn read_targeted(
        &self,
        run_id: &RunId,
        request: &TargetedReadRequest,
        deadline: std::time::Instant,
    ) -> Result<serde_json::Value, PersistenceError> {
        let mut connection = self.lock_until(deadline)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(status_read_deadline());
        }
        connection.busy_timeout(remaining).map_err(sqlite_failure)?;
        connection
            .progress_handler(1_000, Some(move || std::time::Instant::now() >= deadline))
            .map_err(sqlite_failure)?;
        let result = sqlite_reads::read_targeted(&mut connection, run_id, request, deadline);
        let _ = connection.progress_handler(0, None::<fn() -> bool>);
        let _ = connection.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS));
        result
    }

    pub fn read_bounded_observation(
        &self,
        run_id: &RunId,
        deadline: std::time::Instant,
        arm: bool,
    ) -> Result<serde_json::Value, PersistenceError> {
        let mut connection = self.lock_until(deadline)?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(status_read_deadline());
        }
        connection.busy_timeout(remaining).map_err(sqlite_failure)?;
        let result =
            sqlite_observation::read_bounded_observation(&mut connection, run_id, deadline, arm);
        let _ = connection.progress_handler(0, None::<fn() -> bool>);
        let _ = connection.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS));
        result
    }

    pub fn load_work_slot_invocation(
        &self,
        run_id: &RunId,
        invocation_id: &InvocationId,
    ) -> Result<Option<WorkSlotInvocation>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        load_one_work_slot_invocation(&connection, run_id, invocation_id)
    }

    /// Called only after the active controller verified disappearance, including
    /// zombies. The stop marker remains blocking across a crash between the
    /// catalog commit and the acknowledgment file; retry can finish that write.
    pub fn acknowledge_cancellation(
        &self,
        run_id: &RunId,
        invocation_id: &InvocationId,
        completed_at: Timestamp,
        acknowledgment: &loop_core::CancellationAcknowledgment,
        deadline: std::time::Instant,
    ) -> Result<(), PersistenceError> {
        let fail = |error: std::io::Error| {
            PersistenceError::failure(PersistenceFailure::new(
                "cancellation-acknowledgment-failed",
                error.to_string(),
            ))
        };
        let row = self
            .load_work_slot_invocation(run_id, invocation_id)?
            .ok_or_else(|| {
                PersistenceError::failure(PersistenceFailure::new(
                    "invocation-not-found",
                    "cancellation target missing",
                ))
            })?;
        let directory = crate::ownership::directory(&row.capture_dir);
        let admission =
            crate::ownership::Admission::acquire_until(&directory, deadline).map_err(fail)?;
        if !admission.stopped()
            || !directory.join("cleanup-verified.json").is_file()
            || crate::ownership::live_owned_work(&row.capture_dir).map_err(fail)?
        {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "cancellation-cleanup-pending",
                "cleanup has not been verified",
            )));
        }
        if std::time::Instant::now() >= deadline {
            return Err(fail(std::io::Error::other(
                "cancellation acknowledgment deadline",
            )));
        }
        let receipt = std::fs::read(directory.join("waiter-completion.json"))
            .ok()
            .map(|bytes| serde_json::from_slice::<crate::ownership::CompletionReceipt>(&bytes))
            .transpose()
            .map_err(|error| fail(std::io::Error::other(error)))?;
        let mut connection = self.lock()?;
        connection
            .busy_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(sqlite_failure)?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let run = load_required_run(&transaction, run_id)?;
            if std::time::Instant::now() >= deadline {
                return Err(fail(std::io::Error::other(
                    "cancellation acknowledgment deadline",
                )));
            }
            let cancelled_workers = receipt
                .as_ref()
                .map(|r| r.inner_workers.clone())
                .unwrap_or_default();
            let cancelled_workers_json =
                encode_json(&cancelled_workers, "cancelled inner workers")?;
            let completion_snapshot = completion_snapshot_reference(&cancelled_workers_json);
            let changed = transaction
                .execute(
                    "UPDATE work_slot_invocations SET status = 'failed', exit_code = ?1,
                 completed_at = ?2, inner_workers_json = ?3, completion_snapshot_json = ?4
                 WHERE run_id = ?5 AND invocation_id = ?6 AND status IS NULL",
                    params![
                        receipt.as_ref().map(|r| r.exit_code),
                        completed_at.as_unix_millis(),
                        cancelled_workers_json,
                        completion_snapshot,
                        run_id.as_str(),
                        invocation_id.as_str()
                    ],
                )
                .map_err(sqlite_failure)?;
            if changed == 1 {
                let sequence = next_sequence(run.last_sequence)?;
                let history = HistoryEntry::invocation_status_changed(
                    sequence,
                    completed_at,
                    invocation_id.clone(),
                    WaiterWrittenStatus::Failed,
                );
                insert_history(&transaction, run_id, &history)?;
                update_last_sequence(&transaction, run_id, sequence)?;
            } else if row.status != Some(WaiterWrittenStatus::Failed) {
                return Err(PersistenceError::failure(PersistenceFailure::new(
                    "cancellation-conflict",
                    "target is not pending cancellation",
                )));
            }
            Ok(())
        })();
        finish_transaction(transaction, result)?;
        admission
            .write("cleanup-acknowledged.json", acknowledgment)
            .map_err(fail)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PersistenceError> {
        self.connection.lock().map_err(|_| {
            PersistenceError::failure(PersistenceFailure::new(
                "sqlite-lock",
                "SQLite connection mutex was poisoned",
            ))
        })
    }

    fn lock_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<MutexGuard<'_, Connection>, PersistenceError> {
        loop {
            match self.connection.try_lock() {
                Ok(connection) => return Ok(connection),
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(PersistenceError::failure(PersistenceFailure::new(
                        "sqlite-lock",
                        "SQLite connection mutex was poisoned",
                    )))
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(status_read_deadline());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        }
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
                        observed_control_revision, last_sequence, created_at,
                        provider, artifact_root
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13)",
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

    fn amend_binding(
        &self,
        run_id: &RunId,
        slot_id: &WorkSlotId,
        request: loop_core::operations::amend_binding::Request,
        now: Timestamp,
    ) -> Result<HistoryEntry, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, run_id)?
                .ok_or_else(|| PersistenceError::not_found(run_id.clone()))?;
            let run = load_required_run(&transaction, run_id)?;
            require_active(&run)?;
            require_observed(&raw)?;
            verify_revision_and_source(
                &run,
                loop_core::ControlRevision::from_u64(request.state_visit),
                &run.current_state,
            )?;
            if !run
                .workflow
                .work_slots
                .iter()
                .any(|slot| slot.id == *slot_id)
            {
                return Err(PersistenceError::failure(PersistenceFailure::new(
                    "unknown-work-slot",
                    format!("unknown work slot `{slot_id}`"),
                )));
            }
            let original = loop_core::effective_binding(&run, slot_id)
                .transpose()
                .map_err(|message| {
                    PersistenceError::failure(PersistenceFailure::new(
                        "invalid-work-slot-binding",
                        message,
                    ))
                })?;
            let sequence = next_sequence(run.last_sequence)?;
            let history = HistoryEntry::new(
                sequence,
                now,
                HistoryAction::BindingAmended {
                    amendment: loop_core::BindingAmendment {
                        slot_id: slot_id.clone(),
                        state_visit: request.state_visit,
                        owner: request.owner,
                        reason: request.reason,
                        original,
                        effective: request.binding,
                    },
                },
            );
            insert_history(&transaction, run_id, &history)?;
            update_last_sequence(&transaction, run_id, sequence)?;
            Ok(history)
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
            let run = decode_run(raw.clone())?;
            require_active(&run)?;
            require_observed(&raw)?;

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

    fn append_advice_attempt(
        &self,
        request: AppendAdviceAttemptRequest,
    ) -> Result<AppendContextResult, PersistenceError> {
        let data_json = encode_json(&request.data, "advice attempt")?;
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let run = load_required_run(&transaction, &request.run_id)?;
            let sequence = next_sequence(run.last_sequence)?;
            let context = ContextRecord::new(
                request.record_id.clone(),
                loop_core::ADVICE_CAPTURE_KIND,
                request.data,
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
                        loop_core::ADVICE_CAPTURE_KIND,
                        data_json,
                        request.created_at.as_unix_millis(),
                    ],
                )
                .map_err(sqlite_failure)?;
            let history =
                HistoryEntry::context_appended(sequence, request.created_at, request.record_id);
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

    fn load_run_artifact_root(&self, run_id: &RunId) -> Result<Option<String>, PersistenceError> {
        let connection = self.lock()?;
        let artifact_root = connection
            .query_row(
                "SELECT artifact_root FROM runs WHERE id = ?1",
                params![run_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(sqlite_failure)?;
        artifact_root.ok_or_else(|| PersistenceError::not_found(run_id.clone()))
    }

    fn commit_transition(
        &self,
        request: CommitTransitionRequest,
    ) -> Result<CommitTransitionResult, PersistenceError> {
        if request.exception.as_ref().is_some_and(|exception| {
            exception.attestation.state_visit != request.expected_control_revision.as_u64()
                || exception.attestation.owner.trim().is_empty()
                || exception.attestation.reason.trim().is_empty()
        }) {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "invalid-override",
                "override requires the expected visit and nonempty owner/reason",
            )));
        }
        if request.exception.is_some()
            && (request.advice_exception.is_some() || request.driver_act.is_some())
            || request.advice_exception.is_some() && request.driver_act.is_some()
        {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "sqlite-incompatible-event-exceptions",
                "driver acts, advice exceptions, and general event overrides cannot be combined",
            )));
        }
        if request.driver_act.as_ref().is_some_and(|act| {
            request.transition.kind.is_check_free()
                || act.state_visit != request.expected_control_revision.as_u64()
                || act.request.author.name.trim().is_empty()
                || !matches!(
                    act.request.author.kind.as_str(),
                    "human" | "agent" | "script"
                )
                || act.request.reason.trim().is_empty()
                || act.request.changed_artifacts.is_empty()
                || act
                    .request
                    .unchanged_documents
                    .intent_revision
                    .trim()
                    .is_empty()
                || act
                    .request
                    .unchanged_documents
                    .design_revision
                    .trim()
                    .is_empty()
                || act
                    .request
                    .unchanged_documents
                    .plan_revision
                    .trim()
                    .is_empty()
        }) {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "invalid-driver-act",
                "driver act must be attributed to this checked state visit and include its rationale and unchanged document revisions",
            )));
        }
        if request.advice_exception.as_ref().is_some_and(|exception| {
            exception.state_visit != request.expected_control_revision.as_u64()
                || exception.owner.trim().is_empty()
                || exception.reason.trim().is_empty()
                || exception.occasion_ids.is_empty()
                || exception.occasion_ids.iter().any(|id| id.trim().is_empty())
                || exception
                    .occasion_ids
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != exception.occasion_ids.len()
        }) {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "invalid-advice-exception",
                "advice exception requires the expected visit, unique occasions, and nonempty owner/reason",
            )));
        }
        if request.context_append.is_some()
            && (request.transition.kind.is_check_free() || request.exception.is_some())
        {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "sqlite-invalid-context-effect",
                "provider context append effects require a checked transition",
            )));
        }

        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw.clone())?;
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
            if let Some(act) = request.driver_act.as_ref() {
                validate_driver_act_commit(&transaction, &run, &request, act)?;
            }

            require_observed(&raw)?;

            let first_sequence = next_sequence(run.last_sequence)?;
            let revision = next_revision(run.control_revision)?;
            let occurred_at = current_timestamp()?;
            let transition_sequence = if let Some(effect) = request.context_append.as_ref() {
                let record_id = generated_context_effect_id(&request.run_id, first_sequence);
                let data_json = encode_json(&effect.data, "context effect data")?;
                transaction
                    .execute(
                        "INSERT INTO context_records (
                            run_id, record_id, sequence, kind, data_json, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            request.run_id.as_str(),
                            record_id.as_str(),
                            to_sqlite_i64(first_sequence.as_u64(), "semantic sequence")?,
                            effect.kind,
                            data_json,
                            occurred_at.as_unix_millis(),
                        ],
                    )
                    .map_err(sqlite_failure)?;
                let context_history =
                    HistoryEntry::context_appended(first_sequence, occurred_at, record_id);
                insert_history(&transaction, &request.run_id, &context_history)?;
                next_sequence(first_sequence)?
            } else {
                first_sequence
            };
            let history = HistoryEntry::transition(
                transition_sequence,
                occurred_at,
                request.transition.clone(),
                match (
                    request.exception.clone(),
                    request.advice_exception.clone(),
                    request.driver_act.clone(),
                ) {
                    (Some(exception), None, None) => {
                        TransitionHistoryOutcome::Overridden { exception }
                    }
                    (None, Some(exception), None) => {
                        TransitionHistoryOutcome::AdviceException { exception }
                    }
                    (None, None, Some(act)) => TransitionHistoryOutcome::DriverAct { act },
                    (None, None, None) => TransitionHistoryOutcome::Committed,
                    _ => unreachable!("mutually exclusive exceptions checked above"),
                },
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
                        to_sqlite_i64(transition_sequence.as_u64(), "semantic sequence")?,
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
            let run = decode_run(raw.clone())?;
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
            require_observed(&raw)?;

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
            let run = decode_run(raw.clone())?;
            require_active(&run)?;
            require_observed(&raw)?;

            // Check the same ownership barrier as state departure while holding
            // the mutation transaction, so termination cannot strand cancellation.
            for invocation in read_work_slot_invocations(&transaction, &request.run_id)? {
                if loop_core::invocation_owns_work(
                    &invocation,
                    self.invocation_waiter_alive(&invocation),
                ) {
                    return Err(PersistenceError::rejected(
                        PersistenceRejection::LiveOwnedWork {
                            run_id: request.run_id.clone(),
                            invocation_id: invocation.invocation_id,
                        },
                    ));
                }
            }

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

    fn observation_is_current(
        &self,
        run_id: &RunId,
        control_revision: loop_core::ControlRevision,
    ) -> Result<bool, PersistenceError> {
        let connection = self.lock()?;
        let observed = connection
            .query_row(
                "SELECT observed_control_revision = control_revision
                 FROM runs WHERE id = ?1 AND control_revision = ?2",
                params![
                    run_id.as_str(),
                    to_sqlite_i64(control_revision.as_u64(), "control revision")?,
                ],
                |row| row.get::<_, Option<bool>>(0),
            )
            .optional()
            .map_err(sqlite_failure)?;
        Ok(observed.flatten().unwrap_or(false))
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
                id: RunId::new(id.clone()),
                label,
                workflow_id: loop_core::WorkflowId::new(workflow_id),
                override_summary: loop_core::OverrideSummary::from_history(
                    &read_history_entries(&connection, &RunId::from(id.clone()))?,
                    parse_lifecycle(&lifecycle)?,
                ),
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

    fn load_transition_history(
        &self,
        run_id: &RunId,
    ) -> Result<Option<Vec<HistoryEntry>>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, occurred_at, action_json FROM history_entries
                 WHERE run_id=?1 AND json_extract(action_json, '$.kind')='transition'
                 ORDER BY sequence LIMIT 4097",
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
        let mut history = Vec::new();
        for row in rows {
            let (sequence, occurred_at, action_json) = row.map_err(sqlite_failure)?;
            history.push(HistoryEntry::new(
                from_sqlite_u64(sequence, "semantic sequence")?.into(),
                Timestamp::from_unix_millis(occurred_at),
                decode_json(&action_json, "history action")?,
            ));
        }
        if history.len() > 4096 {
            return Ok(None);
        }
        Ok(Some(history))
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

    fn load_status_data(&self, run_id: &RunId) -> Result<ShowData, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(sqlite_failure)?;
        let result = (|| {
            Ok(ShowData {
                run: load_required_run(&transaction, run_id)?,
                context: read_context_records(&transaction, run_id)?,
                checked_evaluations: read_checked_evaluations(&transaction, run_id)?,
            })
        })();
        finish_transaction(transaction, result)
    }

    fn load_show_data(&self, run_id: &RunId) -> Result<ShowData, PersistenceError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_failure)?;
        let result = (|| {
            let _run = load_required_run(&transaction, run_id)?;
            let context = read_context_records(&transaction, run_id)?;
            let checked_evaluations = read_checked_evaluations(&transaction, run_id)?;
            transaction
                .execute(
                    "UPDATE runs
                     SET observed_control_revision = control_revision
                     WHERE id = ?1",
                    params![run_id.as_str()],
                )
                .map_err(sqlite_failure)?;
            let run = load_required_run(&transaction, run_id)?;
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
            let run = decode_run(raw.clone())?;
            require_active(&run)?;
            require_observed(&raw)?;

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
                request.capture_dir.clone(),
                Vec::new(),
            )
            .with_state_visit(request.state_visit)
            .with_routed_inputs(request.routed_inputs.clone())
            .with_assignment_labels(request.assignment_labels.clone())
            .with_assignment_selection(request.assignment_selection.clone())
            .with_waiter_identity_opt(request.waiter_identity.clone());
            let mut invocation = invocation;
            invocation.controls = request.controls.clone();
            let invocation = match request.frozen_run_identity.clone() {
                Some(identity) => invocation.with_frozen_run_identity(identity),
                None => invocation,
            };
            transaction
                .execute(
                    "INSERT INTO work_slot_invocations (
                        run_id, invocation_id, slot_id, binding_json,
                        instruction_digest, subject, state_visit, waiter_pid, waiter_identity_json, started_at,
                        allowed_time_ms, status, exit_code, completed_at,
                        capture_dir, inner_workers_json, assignment_selection_json,
                        invocation_input_json, routed_inputs_json, frozen_run_identity_json,
                        completion_snapshot_json, controls_json, assignment_labels_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                    params![
                        request.run_id.as_str(),
                        request.invocation_id.as_str(),
                        request.slot_id.as_str(),
                        binding_json,
                        request.instruction_digest,
                        request.subject,
                        to_sqlite_i64(request.state_visit, "state visit")?,
                        to_sqlite_i64(u64::from(request.waiter_pid), "waiter pid")?,
                        request
                            .waiter_identity
                            .as_ref()
                            .map(|identity| encode_json(identity, "waiter identity"))
                            .transpose()?,
                        request.started_at.as_unix_millis(),
                        to_sqlite_i64(request.allowed_time_ms, "allowed time ms")?,
                        request.capture_dir,
                        encode_json(&Vec::<InnerWorker>::new(), "inner workers")?,
                        request
                            .assignment_selection
                            .as_ref()
                            .map(|selection| encode_json(selection, "assignment selection"))
                            .transpose()?,
                        request
                            .invocation_input
                            .as_ref()
                            .map(|input| encode_json(input, "invocation input"))
                            .transpose()?,
                        encode_json(&request.routed_inputs, "routed inputs")?,
                        request
                            .frozen_run_identity
                            .as_ref()
                            .map(|identity| encode_json(identity, "frozen run identity"))
                            .transpose()?,
                        encode_json(&Vec::<InnerWorker>::new(), "completion snapshot")?,
                        request.controls.as_ref().map(|controls| encode_json(controls, "invocation controls")).transpose()?,
                        encode_json(&request.assignment_labels, "assignment labels")?,
                    ],
                )
                .map_err(sqlite_failure)?;
            for label in &request.assignment_labels {
                if request
                    .assignment_selection
                    .as_ref()
                    .is_some_and(|selection| !selection.iter().any(|id| id == &label.assignment_id))
                {
                    continue;
                }
                transaction
                    .execute(
                        "INSERT INTO work_slot_assignment_facts (
                        run_id, invocation_id, slot_id, subject, assignment_id, title, role,
                        execution_state, conformance_state
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'unknown', 'unknown')",
                        params![
                            request.run_id.as_str(),
                            request.invocation_id.as_str(),
                            request.slot_id.as_str(),
                            request.subject,
                            label.assignment_id,
                            label.title,
                            label.role
                        ],
                    )
                    .map_err(sqlite_failure)?;
            }

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
        let inner_workers_json = encode_json(&request.inner_workers, "inner workers")?;
        let completion_snapshot = completion_snapshot_reference(&inner_workers_json);
        let assignment_facts_version = i64::from(
            !request.inner_workers.is_empty()
                && request
                    .inner_workers
                    .iter()
                    .all(|worker| !worker.assignment_id.is_empty()),
        );
        // Same short lock as helper admission and the future cancellation
        // controller. Keep it through the terminal transaction: neither side
        // can turn a cancellation-admitted exit into ordinary success.
        let invocation = self.load_work_slot_invocation(&request.run_id, &request.invocation_id)?;
        let admission = invocation
            .as_ref()
            .filter(|row| row.ownership.is_some())
            .map(|row| {
                crate::ownership::Admission::acquire(&crate::ownership::directory(&row.capture_dir))
            })
            .transpose()
            .map_err(|error| {
                PersistenceError::failure(PersistenceFailure::new(
                    "ownership-admission-failed",
                    error.to_string(),
                ))
            })?;
        let current = self.load_work_slot_invocation(&request.run_id, &request.invocation_id)?;
        if let Some(admission) = admission.as_ref().filter(|admission| {
            current.as_ref().is_some_and(|row| {
                row.status.is_none()
                    || (admission.stopped() && row.status == Some(WaiterWrittenStatus::Failed))
            })
        }) {
            admission
                .record_completion(&crate::ownership::CompletionReceipt {
                    exit_code: request.exit_code,
                    inner_workers: request.inner_workers.clone(),
                })
                .map_err(|error| {
                    PersistenceError::failure(PersistenceFailure::new(
                        "ownership-completion-failed",
                        error.to_string(),
                    ))
                })?;
            if admission.stopped() {
                // Cleanup can win just before the released waiter gets CPU.
                // Preserve its later actual waitpid/capture facts, never change
                // the controller's failed outcome or completion timestamp.
                if current
                    .as_ref()
                    .is_some_and(|row| row.status == Some(WaiterWrittenStatus::Failed))
                {
                    let connection = self.lock()?;
                    connection.execute(
                        "UPDATE work_slot_invocations SET exit_code = ?1, inner_workers_json = ?2,
                         completion_snapshot_json = ?3 WHERE run_id = ?4 AND invocation_id = ?5 AND status = 'failed'",
                        params![request.exit_code, inner_workers_json, completion_snapshot,
                            request.run_id.as_str(), request.invocation_id.as_str()],
                    ).map_err(sqlite_failure)?;
                }
                return Err(PersistenceError::failure(PersistenceFailure::new(
                    "cancellation-cleanup-pending",
                    "actual waiter exit retained; only verified cancellation cleanup may finalize this invocation",
                )));
            }
        }
        let mut connection = self.lock()?;
        let transaction = begin_immediate(&mut connection)?;
        let result = (|| {
            let raw = load_raw_run(&transaction, &request.run_id)?
                .ok_or_else(|| PersistenceError::not_found(request.run_id.clone()))?;
            let run = decode_run(raw)?;

            let changed = transaction
                .execute(
                    "UPDATE work_slot_invocations
                     SET status = ?1, exit_code = ?2, completed_at = ?3, inner_workers_json = ?4,
                         completion_snapshot_json = ?5, assignment_facts_version = ?6
                     WHERE run_id = ?7 AND invocation_id = ?8 AND status IS NULL",
                    params![
                        waiter_status_name(request.status),
                        request.exit_code,
                        request.completed_at.as_unix_millis(),
                        inner_workers_json,
                        completion_snapshot,
                        assignment_facts_version,
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
            sqlite_observation::insert_assignment_facts(
                &transaction,
                &request.run_id,
                &invocation,
                &request.inner_workers,
            )?;
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

    fn load_invocation_progress_candidates(
        &self,
        run_id: &RunId,
        invocation_id: Option<&InvocationId>,
    ) -> Result<Vec<WorkSlotInvocation>, PersistenceError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1100);
        let connection = self.lock_until(deadline)?;
        connection
            .busy_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(sqlite_failure)?;
        connection
            .progress_handler(1_000, Some(move || std::time::Instant::now() >= deadline))
            .map_err(sqlite_failure)?;
        let result = (|| {
            let _ = load_required_run(&connection, run_id)?;
            if let Some(invocation_id) = invocation_id {
                let selected = load_progress_invocation(&connection, run_id, invocation_id)?;
                return Ok(selected.into_iter().collect());
            }
            let pending = connection
                .query_row(
                    "SELECT COUNT(*) FROM work_slot_invocations WHERE run_id=?1 AND status IS NULL",
                    params![run_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(sqlite_failure)?;
            if pending > 20 {
                return Err(PersistenceError::failure(PersistenceFailure::new(
                    "invocation-progress-too-many-pending",
                    "more than 20 pending invocations require explicit --invocation selection",
                )));
            }
            let mut identities = Vec::new();
            if pending > 0 {
                let mut statement = connection.prepare(
                    "SELECT invocation_id FROM work_slot_invocations WHERE run_id=?1 AND status IS NULL
                     ORDER BY started_at DESC, invocation_id DESC LIMIT 20"
                ).map_err(sqlite_failure)?;
                let rows = statement
                    .query_map(params![run_id.as_str()], |row| row.get::<_, String>(0))
                    .map_err(sqlite_failure)?;
                for row in rows {
                    identities.push(InvocationId::from(row.map_err(sqlite_failure)?));
                }
            }
            let latest = connection
                .query_row(
                    "SELECT invocation_id FROM work_slot_invocations WHERE run_id=?1
                 ORDER BY started_at DESC, invocation_id DESC LIMIT 1",
                    params![run_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sqlite_failure)?;
            if let Some(latest) = latest {
                let latest = InvocationId::from(latest);
                if !identities.contains(&latest) {
                    identities.push(latest);
                }
            }
            let mut candidates = Vec::with_capacity(identities.len());
            for identity in identities {
                if let Some(row) = load_progress_invocation(&connection, run_id, &identity)? {
                    candidates.push(row);
                }
            }
            Ok(candidates)
        })();
        let _ = connection.progress_handler(0, None::<fn() -> bool>);
        let _ = connection.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS));
        result
    }

    fn invocation_waiter_alive(&self, invocation: &WorkSlotInvocation) -> bool {
        crate::ownership::process_identity_matches(
            invocation.waiter_pid,
            invocation.waiter_identity.as_ref(),
        )
        .unwrap_or(false)
    }
}

fn validate_driver_act_commit(
    transaction: &Transaction<'_>,
    run: &Run,
    request: &CommitTransitionRequest,
    act: &DriverActEvidence,
) -> Result<(), PersistenceError> {
    let Some(slot) = run.workflow.work_slots.iter().find(|slot| {
        slot.id == act.slot_id
            && slot.state == run.current_state
            && slot.event == request.transition.event
    }) else {
        return Err(PersistenceError::failure(PersistenceFailure::new(
            "driver-act-slot-mismatch",
            "driver act does not name the current event's bound work slot",
        )));
    };
    if !slot.driver_act_allowed || act.state_visit != run.control_revision.as_u64() {
        return Err(PersistenceError::failure(PersistenceFailure::new(
            "driver-act-not-opted-in",
            "driver act is not allowed on this frozen slot visit",
        )));
    }
    let binding = loop_core::effective_binding(run, &slot.id)
        .and_then(Result::ok)
        .ok_or_else(|| {
            PersistenceError::failure(PersistenceFailure::new(
                "driver-act-requires-bound-slot",
                "driver act requires a valid frozen slot binding",
            ))
        })?;
    let state = run
        .workflow
        .states
        .iter()
        .find(|state| state.id == slot.state)
        .ok_or_else(|| {
            PersistenceError::failure(PersistenceFailure::new(
                "invalid-run",
                "driver-act work slot names a missing source state",
            ))
        })?;
    let instruction = loop_core::instruction_digest(&state.instructions);
    let binding_bytes = serde_json::to_vec(&binding).map_err(|error| {
        PersistenceError::failure(PersistenceFailure::new(
            "driver-act-binding-invalid",
            error.to_string(),
        ))
    })?;
    let binding_digest = format!("sha256:{:x}", Sha256::digest(binding_bytes));
    let current_subject = transaction
        .query_row(
            "SELECT subject FROM slot_subjects WHERE run_id=?1 AND slot_id=?2",
            params![run.id.as_str(), slot.id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite_failure)?;
    if act.instruction_digest != instruction
        || act.binding_sha256 != binding_digest
        || current_subject.as_deref() != Some(act.current_subject.as_str())
    {
        return Err(PersistenceError::failure(PersistenceFailure::new(
            "driver-act-identity-mismatch",
            "driver act binding, instruction, or current visit subject changed before commit",
        )));
    }
    Ok(())
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
    if !columns
        .iter()
        .any(|name| name == "observed_control_revision")
    {
        connection
            .execute(
                "ALTER TABLE runs ADD COLUMN observed_control_revision INTEGER",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    Ok(())
}

fn ensure_work_slot_invocation_columns(connection: &Connection) -> Result<(), PersistenceError> {
    let columns = work_slot_invocation_table_columns(connection)?;
    if !columns.iter().any(|name| name == "waiter_identity_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN waiter_identity_json TEXT",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "capture_dir") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN capture_dir TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "inner_workers_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN inner_workers_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns
        .iter()
        .any(|name| name == "assignment_selection_json")
    {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN assignment_selection_json TEXT",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "invocation_input_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN invocation_input_json TEXT",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "controls_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN controls_json TEXT",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "routed_inputs_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN routed_inputs_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns
        .iter()
        .any(|name| name == "frozen_run_identity_json")
    {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN frozen_run_identity_json TEXT",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns
        .iter()
        .any(|name| name == "completion_snapshot_json")
    {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN completion_snapshot_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "state_visit") {
        connection.execute(
            "ALTER TABLE work_slot_invocations ADD COLUMN state_visit INTEGER NOT NULL DEFAULT 0",
            [],
        ).map_err(sqlite_failure)?;
    }
    if !columns.iter().any(|name| name == "assignment_labels_json") {
        connection
            .execute(
                "ALTER TABLE work_slot_invocations ADD COLUMN assignment_labels_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(sqlite_failure)?;
    }
    if !columns
        .iter()
        .any(|name| name == "assignment_facts_version")
    {
        connection.execute(
            "ALTER TABLE work_slot_invocations ADD COLUMN assignment_facts_version INTEGER NOT NULL DEFAULT 0", [],
        ).map_err(sqlite_failure)?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS work_slot_invocations_by_subject
         ON work_slot_invocations (run_id, slot_id, subject, status, assignment_facts_version)",
            [],
        )
        .map_err(sqlite_failure)?;
    Ok(())
}

fn ensure_work_slot_assignment_columns(connection: &Connection) -> Result<(), PersistenceError> {
    let facts = table_columns(connection, "work_slot_assignment_facts")?;
    if !facts
        .iter()
        .any(|name| name == "conformance_error_truncated")
    {
        connection.execute(
            "ALTER TABLE work_slot_assignment_facts ADD COLUMN conformance_error_truncated INTEGER NOT NULL DEFAULT 0", [],
        ).map_err(sqlite_failure)?;
    }
    if !facts.iter().any(|name| name == "facts_complete") {
        connection.execute(
            "ALTER TABLE work_slot_assignment_facts ADD COLUMN facts_complete INTEGER NOT NULL DEFAULT 0", [],
        ).map_err(sqlite_failure)?;
    }
    let attempts = table_columns(connection, "work_slot_assignment_attempts")?;
    if !attempts.iter().any(|name| name == "errors_truncated") {
        connection.execute(
            "ALTER TABLE work_slot_assignment_attempts ADD COLUMN errors_truncated INTEGER NOT NULL DEFAULT 0", [],
        ).map_err(sqlite_failure)?;
    }
    Ok(())
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, PersistenceError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info('{table}')"))
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_failure)?;
    rows.map(|row| row.map_err(sqlite_failure)).collect()
}

fn work_slot_invocation_table_columns(
    connection: &Connection,
) -> Result<Vec<String>, PersistenceError> {
    let mut statement = connection
        .prepare("PRAGMA table_info('work_slot_invocations')")
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_failure)?;
    rows.map(|row| row.map_err(sqlite_failure)).collect()
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

#[derive(Clone, Debug)]
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
    observed_control_revision: Option<i64>,
}

fn load_raw_run(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Option<StoredRun>, PersistenceError> {
    connection
        .query_row(
            "SELECT id, label, workflow_json, provider_association_json,
                    initial_input_json, current_state, lifecycle,
                    control_revision, last_sequence, created_at,
                    observed_control_revision
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
                    observed_control_revision: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(sqlite_failure)
}

fn load_required_run(connection: &Connection, run_id: &RunId) -> Result<Run, PersistenceError> {
    let raw = load_raw_run(connection, run_id)?
        .ok_or_else(|| PersistenceError::not_found(run_id.clone()))?;
    let mut run = decode_run(raw)?;
    let history = read_history_entries(connection, run_id)?;
    run.override_summary = loop_core::OverrideSummary::from_history(&history, run.lifecycle);
    run.binding_amendments = history
        .into_iter()
        .filter_map(|entry| match entry.action {
            HistoryAction::BindingAmended { amendment } => Some(amendment),
            _ => None,
        })
        .collect();
    Ok(run)
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

fn require_observed(raw: &StoredRun) -> Result<(), PersistenceError> {
    let observed = raw
        .observed_control_revision
        .and_then(|value| u64::try_from(value).ok());
    let current = u64::try_from(raw.control_revision).ok();
    if observed == current {
        Ok(())
    } else {
        Err(PersistenceError::rejected(
            PersistenceRejection::RunNotObserved {
                run_id: RunId::new(raw.id.clone()),
            },
        ))
    }
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

fn generated_context_effect_id(
    run_id: &RunId,
    sequence: SemanticSequence,
) -> loop_core::ContextRecordId {
    loop_core::ContextRecordId::new(format!("engine-context-effect-{run_id}-{sequence}"))
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
                    TransitionHistoryOutcome::Overridden { .. } => return None,
                    TransitionHistoryOutcome::Committed
                    | TransitionHistoryOutcome::AdviceException { .. }
                    | TransitionHistoryOutcome::DriverAct { .. } => DurableEvaluation {
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
fn completion_snapshot_reference(inner_workers_json: &str) -> String {
    let digest = Sha256::digest(inner_workers_json.as_bytes());
    format!("{COMPLETION_SNAPSHOT_REF_PREFIX}{digest:x}")
}

fn decode_completion_snapshot(
    snapshot: &str,
    inner_workers_json: &str,
) -> Result<(Vec<InnerWorker>, Option<String>), PersistenceError> {
    if let Some(expected) = snapshot.strip_prefix(COMPLETION_SNAPSHOT_REF_PREFIX) {
        let actual = format!("{:x}", Sha256::digest(inner_workers_json.as_bytes()));
        if expected != actual {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "sqlite-completion-snapshot-mismatch",
                "worker payload no longer matches its completion digest",
            )));
        }
        return Ok((Vec::new(), Some(actual)));
    }
    Ok((decode_json(snapshot, "completion snapshot")?, None))
}

#[allow(clippy::too_many_arguments)]
fn decode_work_slot_invocation(
    invocation_id: String,
    slot_id: String,
    binding_json: String,
    instruction_digest: String,
    subject: String,
    waiter_pid: i64,
    waiter_identity_json: Option<String>,
    started_at: i64,
    allowed_time_ms: i64,
    status: Option<String>,
    exit_code: Option<i64>,
    completed_at: Option<i64>,
    capture_dir: String,
    inner_workers_json: String,
    assignment_selection_json: Option<String>,
    invocation_input_json: Option<String>,
    routed_inputs_json: String,
    frozen_run_identity_json: Option<String>,
    completion_snapshot_json: String,
    controls_json: Option<String>,
    assignment_labels_json: String,
    state_visit: i64,
) -> Result<WorkSlotInvocation, PersistenceError> {
    let waiter_pid = u32::try_from(from_sqlite_u64(waiter_pid, "waiter pid")?).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-integer-overflow",
            "waiter pid does not fit u32",
        ))
    })?;
    let allowed_time_ms = from_sqlite_u64(allowed_time_ms, "allowed time ms")?;
    let state_visit = from_sqlite_u64(state_visit, "state visit")?;
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
    let (recorded_workers, completion_snapshot_sha256) =
        decode_completion_snapshot(&completion_snapshot_json, &inner_workers_json)?;
    let mut invocation = WorkSlotInvocation::new(
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
        capture_dir,
        decode_json(&inner_workers_json, "inner workers")?,
    )
    .with_state_visit(state_visit)
    .with_routed_inputs(decode_json(&routed_inputs_json, "routed inputs")?)
    .with_waiter_identity_opt(
        waiter_identity_json
            .map(|json| decode_json(&json, "waiter identity"))
            .transpose()?,
    )
    .with_recorded_inner_workers(recorded_workers)
    .with_completion_snapshot_sha256(completion_snapshot_sha256)
    .with_assignment_labels(decode_json(&assignment_labels_json, "assignment labels")?)
    .with_assignment_selection(
        assignment_selection_json
            .map(|json| decode_json(&json, "assignment selection"))
            .transpose()?,
    )
    .with_invocation_input(
        invocation_input_json
            .map(|json| decode_json(&json, "invocation input"))
            .transpose()?,
    );
    if let Some(identity_json) = frozen_run_identity_json {
        invocation = invocation
            .with_frozen_run_identity(decode_json(&identity_json, "frozen run identity")?);
    }
    invocation.controls = controls_json
        .map(|raw| decode_json(&raw, "invocation controls"))
        .transpose()?;
    if !invocation.capture_dir.is_empty() {
        if let Some(execution) =
            crate::ownership::read_ownership(&invocation.capture_dir).map_err(|error| {
                PersistenceError::failure(PersistenceFailure::new(
                    "ownership-read-failed",
                    error.to_string(),
                ))
            })?
        {
            invocation.ownership = Some(loop_core::ExecutionOwnershipState {
                execution,
                live_owned_work: crate::ownership::live_owned_work(&invocation.capture_dir)
                    .unwrap_or(true),
                cleanup_pending: crate::ownership::cleanup_pending(&invocation.capture_dir),
                cancellation: crate::ownership::cancellation_state(&invocation.capture_dir)
                    .map_err(|error| {
                        PersistenceError::failure(PersistenceFailure::new(
                            "cancellation-read-failed",
                            error.to_string(),
                        ))
                    })?,
            });
        }
    }
    Ok(invocation)
}

type InvocationRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    i64,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
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
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
        row.get(19)?,
        row.get(20)?,
        row.get(21)?,
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
                    waiter_pid, waiter_identity_json, started_at, allowed_time_ms, status,
                    exit_code, completed_at, capture_dir, inner_workers_json,
                    assignment_selection_json, invocation_input_json, routed_inputs_json,
                    frozen_run_identity_json, completion_snapshot_json, controls_json,
                    assignment_labels_json, state_visit
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
                waiter_identity_json,
                started_at,
                allowed_time_ms,
                status,
                exit_code,
                completed_at,
                capture_dir,
                inner_workers_json,
                assignment_selection_json,
                invocation_input_json,
                routed_inputs_json,
                frozen_run_identity_json,
                completion_snapshot_json,
                controls_json,
                assignment_labels_json,
                state_visit,
            )| {
                decode_work_slot_invocation(
                    invocation_id,
                    slot_id,
                    binding_json,
                    instruction_digest,
                    subject,
                    waiter_pid,
                    waiter_identity_json,
                    started_at,
                    allowed_time_ms,
                    status,
                    exit_code,
                    completed_at,
                    capture_dir,
                    inner_workers_json,
                    assignment_selection_json,
                    invocation_input_json,
                    routed_inputs_json,
                    frozen_run_identity_json,
                    completion_snapshot_json,
                    controls_json,
                    assignment_labels_json,
                    state_visit,
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
                    waiter_pid, waiter_identity_json, started_at, allowed_time_ms, status,
                    exit_code, completed_at, capture_dir, inner_workers_json,
                    assignment_selection_json, invocation_input_json, routed_inputs_json,
                    frozen_run_identity_json, completion_snapshot_json, controls_json,
                    assignment_labels_json, state_visit
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
            waiter_identity_json,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
            capture_dir,
            inner_workers_json,
            assignment_selection_json,
            invocation_input_json,
            routed_inputs_json,
            frozen_run_identity_json,
            completion_snapshot_json,
            controls_json,
            assignment_labels_json,
            state_visit,
        ) = row.map_err(sqlite_failure)?;
        decode_work_slot_invocation(
            invocation_id,
            slot_id,
            binding_json,
            instruction_digest,
            subject,
            waiter_pid,
            waiter_identity_json,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
            capture_dir,
            inner_workers_json,
            assignment_selection_json,
            invocation_input_json,
            routed_inputs_json,
            frozen_run_identity_json,
            completion_snapshot_json,
            controls_json,
            assignment_labels_json,
            state_visit,
        )
    })
    .collect()
}

fn load_progress_invocation(
    connection: &Connection,
    run_id: &RunId,
    invocation_id: &InvocationId,
) -> Result<Option<WorkSlotInvocation>, PersistenceError> {
    let row = connection
        .query_row(
            "SELECT invocation_id, slot_id, subject, waiter_pid, waiter_identity_json,
                started_at, allowed_time_ms, status, exit_code, completed_at, capture_dir, state_visit
         FROM work_slot_invocations WHERE run_id=?1 AND invocation_id=?2",
            params![run_id.as_str(), invocation_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_failure)?;
    let Some((
        invocation_id,
        slot_id,
        subject,
        waiter_pid,
        identity_json,
        started_at,
        allowed_time_ms,
        status,
        exit_code,
        completed_at,
        capture_dir,
        state_visit,
    )) = row
    else {
        return Ok(None);
    };
    let waiter_pid = u32::try_from(from_sqlite_u64(waiter_pid, "waiter pid")?).map_err(|_| {
        PersistenceError::failure(PersistenceFailure::new(
            "sqlite-integer-overflow",
            "waiter pid does not fit u32",
        ))
    })?;
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
    let identity = identity_json
        .map(|raw| decode_json(&raw, "waiter identity"))
        .transpose()?;
    let mut invocation = WorkSlotInvocation::new(
        InvocationId::new(invocation_id),
        WorkSlotId::new(slot_id),
        loop_core::WorkSlotBinding::new(String::new(), Vec::new()),
        String::new(),
        subject,
        waiter_pid,
        Timestamp::from_unix_millis(started_at),
        from_sqlite_u64(allowed_time_ms, "allowed time ms")?,
        status,
        exit_code,
        completed_at.map(Timestamp::from_unix_millis),
        capture_dir,
        Vec::new(),
    )
    .with_state_visit(from_sqlite_u64(state_visit, "state visit")?)
    .with_waiter_identity_opt(identity);
    if !invocation.capture_dir.is_empty() {
        if let Some(execution) =
            crate::ownership::read_ownership(&invocation.capture_dir).map_err(|error| {
                PersistenceError::failure(PersistenceFailure::new(
                    "ownership-read-failed",
                    error.to_string(),
                ))
            })?
        {
            invocation.ownership = Some(loop_core::ExecutionOwnershipState {
                execution,
                live_owned_work: crate::ownership::live_owned_work(&invocation.capture_dir)
                    .unwrap_or(true),
                cleanup_pending: crate::ownership::cleanup_pending(&invocation.capture_dir),
                cancellation: crate::ownership::cancellation_state(&invocation.capture_dir)
                    .map_err(|error| {
                        PersistenceError::failure(PersistenceFailure::new(
                            "cancellation-read-failed",
                            error.to_string(),
                        ))
                    })?,
            });
        }
    }
    Ok(Some(invocation))
}

fn status_read_deadline() -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new(
        "status-read-deadline",
        "bounded observation could not finish before its deadline",
    ))
}

fn sqlite_failure(error: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new("sqlite", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{
        ContextAppendEffect, EvaluationRequest, EvaluationResult, InnerWorker, Lifecycle,
        ProviderAssociation, ProviderError, ProviderGateway, State, Transition,
        WaiterWrittenStatus, WorkSlotBinding, Workflow,
    };
    use serde_json::json;
    use tempfile::tempdir;

    fn workflow() -> Workflow {
        Workflow::new(
            "test-workflow",
            "start",
            vec![State::new("start", "Start", "Begin", false)],
            vec![Transition::check_free("start", "finish", "start")],
        )
    }

    fn create_run(id: &str) -> CreateRunRequest {
        CreateRunRequest::new(
            id,
            Some(format!("label-{id}")),
            workflow(),
            ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
            json!({"objective": "durable"}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(100),
            "test-provider",
            Some("/allocated/run-dir".to_owned()),
        )
    }

    #[test]
    fn advice_attempt_is_immutable_run_history_without_arming_primary_work() {
        let adapter = SqlitePersistence::open_in_memory().expect("open sqlite");
        adapter
            .create_run(create_run("run-advice-capture"))
            .expect("create run");
        assert_eq!(
            adapter
                .load_run_artifact_root(&"run-advice-capture".into())
                .expect("load run artifact root"),
            Some("/allocated/run-dir".to_owned())
        );

        let appended = adapter
            .append_advice_attempt(AppendAdviceAttemptRequest::new(
                "run-advice-capture",
                "advice-attempt-1",
                json!({"attempt_id":"advice-attempt-1","typed_result":null,"exit_code":7}),
                Timestamp::from_unix_millis(200),
            ))
            .expect("retain advice attempt without show or slot invocation");
        assert_eq!(appended.context.kind, loop_core::ADVICE_CAPTURE_KIND);
        assert_eq!(appended.context.id.as_str(), "advice-attempt-1");
        assert_eq!(appended.context.data["exit_code"], 7);
        assert_eq!(appended.run.current_state.as_str(), "start");
        assert_eq!(appended.run.last_sequence.as_u64(), 2);
        assert!(!adapter
            .observation_is_current(&"run-advice-capture".into(), 0_u64.into())
            .expect("observe arming remains separate"));
        assert!(adapter
            .load_work_slot_invocations(&"run-advice-capture".into())
            .expect("primary slots remain empty")
            .is_empty());
        let history = adapter
            .load_history(&"run-advice-capture".into())
            .expect("load immutable history");
        assert!(matches!(
            history.last().map(|entry| &entry.action),
            Some(HistoryAction::ContextAppended { context_record_id }) if context_record_id.as_str() == "advice-attempt-1"
        ));
    }

    fn checked_event_workflow() -> Workflow {
        Workflow::new(
            "checked-event-test-workflow",
            "start",
            vec![
                State::new("start", "Start", "Begin", false),
                State::new("middle", "Middle", "Continue", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::checked("start", "approve", "middle")],
        )
    }

    fn checked_event_run(id: &str) -> CreateRunRequest {
        CreateRunRequest::new(
            id,
            Some(format!("label-{id}")),
            checked_event_workflow(),
            ProviderAssociation::new(json!({"provider": "static-test-gateway"})),
            json!({"objective": "atomic checked event"}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(100),
            "static-test-gateway",
            None,
        )
    }

    struct StaticGateway {
        result: EvaluationResult,
    }

    impl StaticGateway {
        fn new(result: EvaluationResult) -> Self {
            Self { result }
        }
    }

    impl ProviderGateway for StaticGateway {
        fn describe(
            &self,
            _provider: &ProviderAssociation,
            _initial_input: Option<&serde_json::Value>,
        ) -> Result<Workflow, ProviderError> {
            Ok(checked_event_workflow())
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            _request: EvaluationRequest,
        ) -> Result<EvaluationResult, ProviderError> {
            Ok(self.result.clone())
        }
    }

    #[test]
    fn outer_checked_event_persists_effect_allow_and_transition_atomically() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("checked-event-atomic.sqlite");
        let effect = ContextAppendEffect::new(
            "accepted-intent-snapshot",
            json!({"schema_version": 1, "intent_revision": "r1"}),
        );

        {
            let adapter = SqlitePersistence::open(&path).expect("open sqlite");
            adapter
                .create_run(checked_event_run("run-checked-event-atomic"))
                .expect("create run");
            adapter
                .load_show_data(&"run-checked-event-atomic".into())
                .expect("observe run");

            let outcome = loop_core::operations::event::execute(
                loop_core::operations::event::Request::new("run-checked-event-atomic", "approve"),
                &StaticGateway::new(EvaluationResult::allow_with_context_append(effect.clone())),
                &adapter,
            );
            assert!(outcome.is_completed(), "checked event failed: {outcome:?}");
        }

        let reopened = SqlitePersistence::open(&path).expect("reopen sqlite");
        let run = reopened
            .load_authoritative_run(&"run-checked-event-atomic".into())
            .expect("load committed run");
        assert_eq!(run.current_state.as_str(), "middle");
        assert_eq!(run.lifecycle, Lifecycle::Active);
        assert_eq!(run.control_revision.as_u64(), 1);
        assert_eq!(run.last_sequence.as_u64(), 3);

        let context = reopened
            .load_context_records(&"run-checked-event-atomic".into())
            .expect("load effect");
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].kind, effect.kind);
        assert_eq!(context[0].data, effect.data);
        assert_eq!(context[0].sequence.as_u64(), 2);
        assert!(context[0].id.as_str().starts_with("engine-context-effect-"));
        assert!(!context[0].id.as_str().contains("accepted-intent-snapshot"));

        let history = reopened
            .load_history(&"run-checked-event-atomic".into())
            .expect("load history");
        assert_eq!(history.len(), 3);
        assert!(matches!(
            history[1].action,
            loop_core::HistoryAction::ContextAppended { .. }
        ));
        assert!(matches!(
            history[2].action,
            loop_core::HistoryAction::Transition {
                outcome: loop_core::TransitionHistoryOutcome::Committed,
                ..
            }
        ));

        let evaluations = reopened
            .load_checked_evaluations(&"run-checked-event-atomic".into())
            .expect("load allow evaluation");
        assert_eq!(evaluations.len(), 1);
        assert!(evaluations[0].is_allow());
        assert_eq!(evaluations[0].sequence.as_u64(), 3);
    }

    #[test]
    fn create_and_complete_roundtrip_stores_capture_dir_and_inner_workers() {
        let adapter = SqlitePersistence::open_in_memory().expect("open in-memory sqlite");
        adapter
            .create_run(create_run("run-capture-roundtrip"))
            .expect("create run");
        adapter
            .load_show_data(&"run-capture-roundtrip".into())
            .expect("observe run");
        let created = adapter
            .create_work_slot_invocation(
                CreateWorkSlotInvocationRequest::new(
                    "run-capture-roundtrip",
                    "inv-1",
                    "slot-1",
                    WorkSlotBinding::new("/bin/sh", vec!["-c".to_owned(), "exit 0".to_owned()]),
                    "digest",
                    "subject-a",
                    1,
                    Timestamp::from_unix_millis(500),
                    1_000,
                    "/captures/slot-1/inv-1",
                )
                .with_controls(loop_core::InvocationControls {
                    max_active: std::num::NonZeroUsize::new(2),
                    force_fresh: true,
                }),
            )
            .expect("create invocation");
        assert_eq!(created.invocation.capture_dir, "/captures/slot-1/inv-1");
        assert!(created.invocation.inner_workers.is_empty());
        assert!(created.invocation.status.is_none());

        let completed = adapter
            .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
                "run-capture-roundtrip",
                "inv-1",
                WaiterWrittenStatus::Succeeded,
                0,
                Timestamp::from_unix_millis(900),
                vec![InnerWorker::new("dummy", vec!["--fail".to_owned()], 7)],
            ))
            .expect("complete invocation");
        assert_eq!(
            completed.invocation.status,
            Some(WaiterWrittenStatus::Succeeded)
        );
        assert_eq!(completed.invocation.exit_code, Some(0));
        assert_eq!(completed.invocation.capture_dir, "/captures/slot-1/inv-1");
        assert_eq!(completed.invocation.inner_workers.len(), 1);
        assert_eq!(completed.invocation.inner_workers[0].command, "dummy");
        assert_eq!(
            completed.invocation.inner_workers[0].args,
            vec!["--fail".to_owned()]
        );
        assert_eq!(completed.invocation.inner_workers[0].exit_code, 7);

        let loaded = adapter
            .load_work_slot_invocations(&"run-capture-roundtrip".into())
            .expect("load invocations");
        assert_eq!(loaded[0].capture_dir, "/captures/slot-1/inv-1");
        assert_eq!(loaded[0].status, Some(WaiterWrittenStatus::Succeeded));
        assert_eq!(loaded[0].exit_code, Some(0));
        assert_eq!(loaded[0].inner_workers[0].exit_code, 7);
        assert_eq!(loaded[0].controls, created.invocation.controls);
        assert!(loaded[0].controls.as_ref().unwrap().force_fresh);
    }

    #[test]
    fn bounded_observation_and_targeted_reads_are_revision_scoped_and_exact() {
        use loop_core::{AssignmentLabel, WorkSlot, WorkerAttempt};
        use std::time::{Duration, Instant};

        let directory = tempdir().expect("tempdir");
        let database = directory.path().join("observation.sqlite");
        let adapter = SqlitePersistence::open(&database).expect("open sqlite");
        let workflow = workflow().with_work_slots(vec![WorkSlot::new("slot-1", "start", "finish")]);
        let run = CreateRunRequest::new(
            "run-observation",
            None,
            workflow,
            ProviderAssociation::new(json!({"identity":"fixture"})),
            json!({"artifact_root":directory.path().join("artifacts").to_string_lossy(),
                   "work_slot_bindings":{"slot-1":{"command":"worker","args":[]}}}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "fixture",
            None,
        );
        adapter.create_run(run).expect("create run");
        let passive = adapter
            .read_bounded_observation(
                &"run-observation".into(),
                Instant::now() + Duration::from_secs(1),
                false,
            )
            .expect("non-arming status");
        assert_eq!(passive["mutation_armed"], false);
        assert_eq!(passive["run"]["current_state"], "start");
        assert!(!adapter
            .observation_is_current(&"run-observation".into(), 0_u64.into())
            .unwrap());
        adapter
            .load_show_data(&"run-observation".into())
            .expect("arm action view");
        adapter
            .set_current_slot_subject(
                &"run-observation".into(),
                &"slot-1".into(),
                "revision-old".into(),
            )
            .unwrap();

        let capture = directory.path().join("capture");
        let attempt_one = capture.join("0/attempts/1");
        let attempt_two = capture.join("0/attempts/2");
        std::fs::create_dir_all(&attempt_one).unwrap();
        std::fs::create_dir_all(&attempt_two).unwrap();
        std::fs::write(attempt_one.join("stdout"), b"old\0bytes").unwrap();
        std::fs::write(attempt_one.join("stderr"), b"old error").unwrap();
        std::fs::write(attempt_two.join("stdout"), b"A\0BCDE").unwrap();
        std::fs::write(attempt_two.join("stderr"), b"current error").unwrap();

        let make_request = |id: &str, subject: &str, capture_dir: &str| {
            let started_at = if id == "inv-old" { 10 } else { 20 };
            CreateWorkSlotInvocationRequest::new(
                "run-observation",
                id,
                "slot-1",
                WorkSlotBinding::new("worker", vec![]),
                "digest",
                subject,
                0,
                Timestamp::from_unix_millis(started_at),
                1000,
                capture_dir,
            )
            .with_assignment_labels(vec![AssignmentLabel {
                assignment_id: "worker-0".into(),
                title: "Axis A".into(),
                role: "reviewer Sol".into(),
            }])
        };
        adapter
            .create_work_slot_invocation(make_request(
                "inv-old",
                "revision-old",
                capture.to_str().unwrap(),
            ))
            .unwrap();
        let mut old_worker = InnerWorker::new("worker", vec![], 0);
        old_worker.assignment_id = "worker-0".into();
        old_worker.conformance_status = Some("failed".into());
        old_worker.conformance_error = Some("older revision failed".into());
        old_worker.attempts = vec![WorkerAttempt {
            number: 1,
            failed: true,
            errors: vec!["older revision failed".into()],
            stdout_path: Some("0/attempts/1/stdout".into()),
            stderr_path: Some("0/attempts/1/stderr".into()),
        }];
        adapter
            .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
                "run-observation",
                "inv-old",
                WaiterWrittenStatus::Failed,
                1,
                Timestamp::from_unix_millis(20),
                vec![old_worker],
            ))
            .unwrap();

        adapter
            .set_current_slot_subject(
                &"run-observation".into(),
                &"slot-1".into(),
                "revision-current".into(),
            )
            .unwrap();
        adapter
            .create_work_slot_invocation(make_request(
                "inv-current",
                "revision-current",
                capture.to_str().unwrap(),
            ))
            .unwrap();
        let active = adapter
            .read_bounded_observation(
                &"run-observation".into(),
                Instant::now() + Duration::from_secs(1),
                false,
            )
            .expect("running status sample");
        assert!(
            active["assignments"]["items"][0]["conformance"]["attempt_count"].is_null(),
            "an uncompleted invocation must not fabricate a zero attempt count"
        );
        let mut current_worker = InnerWorker::new("worker", vec![], 0);
        current_worker.assignment_id = "worker-0".into();
        current_worker.stdout_path = Some("0/stdout".into());
        current_worker.stderr_path = Some("0/stderr".into());
        current_worker.conformance_status = Some("failed".into());
        current_worker.conformance_error = Some("current retry exhausted".into());
        current_worker.attempts = vec![
            WorkerAttempt {
                number: 1,
                failed: true,
                errors: vec!["first validation failure".into()],
                stdout_path: Some("0/attempts/1/stdout".into()),
                stderr_path: Some("0/attempts/1/stderr".into()),
            },
            WorkerAttempt {
                number: 2,
                failed: true,
                errors: vec!["second validation failure".into()],
                stdout_path: Some("0/attempts/2/stdout".into()),
                stderr_path: Some("0/attempts/2/stderr".into()),
            },
        ];
        adapter
            .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
                "run-observation",
                "inv-current",
                WaiterWrittenStatus::Failed,
                1,
                Timestamp::from_unix_millis(30),
                vec![current_worker],
            ))
            .unwrap();

        let snapshot = adapter
            .read_bounded_observation(
                &"run-observation".into(),
                Instant::now() + Duration::from_secs(1),
                false,
            )
            .expect("bounded status");
        let assignment = &snapshot["assignments"]["items"][0];
        assert_eq!(assignment["assignment_id"], "worker-0");
        assert_eq!(assignment["title"], "Axis A");
        assert_eq!(assignment["role"], "reviewer Sol");
        assert_eq!(assignment["subject_revision"], "revision-current");
        assert_eq!(assignment["conformance"]["attempt_count"], 2);
        assert_eq!(assignment["conformance"]["repeated_failure_count"], 2);
        assert_eq!(assignment["state"], "exhausted");
        assert_eq!(assignment["acceptance"]["state"], "unknown");

        let page = adapter
            .read_targeted(
                &"run-observation".into(),
                &TargetedReadRequest {
                    kind: "assignment".into(),
                    assignment_id: Some("worker-0".into()),
                    limit: 10,
                    ..TargetedReadRequest::default()
                },
                Instant::now() + Duration::from_secs(1),
            )
            .expect("assignment history page");
        assert_eq!(page["schema_version"], 1);
        assert!(page["delta_sequence"].as_u64().is_some());
        assert_eq!(page["total"], 2);
        assert_eq!(page["items"][0]["subject_revision"], "revision-old");
        assert_eq!(page["items"][1]["subject_revision"], "revision-current");
        let failures = adapter
            .read_targeted(
                &"run-observation".into(),
                &TargetedReadRequest {
                    kind: "error".into(),
                    assignment_id: Some("worker-0".into()),
                    invocation_id: Some("inv-current".into()),
                    limit: 10,
                    ..TargetedReadRequest::default()
                },
                Instant::now() + Duration::from_secs(1),
            )
            .expect("error page");
        assert_eq!(failures["total"], 2);
        assert_eq!(
            failures["items"][0]["errors"][0],
            "first validation failure"
        );

        let stream = adapter
            .read_targeted(
                &"run-observation".into(),
                &TargetedReadRequest {
                    kind: "stdout".into(),
                    assignment_id: Some("worker-0".into()),
                    invocation_id: Some("inv-current".into()),
                    attempt: Some(2),
                    offset: 1,
                    limit: 3,
                    ..TargetedReadRequest::default()
                },
                Instant::now() + Duration::from_secs(1),
            )
            .expect("original stdout byte range");
        assert_eq!(stream["schema_version"], 1);
        assert!(stream["delta_sequence"].as_u64().is_some());
        assert_eq!(stream["encoding"], "hex");
        assert_eq!(stream["data"], "004243");
        assert_eq!(stream["total_bytes"], 6);
        assert_eq!(stream["next_offset"], 4);
        assert_eq!(stream["truncated"], true);
        assert!(
            adapter
                .read_targeted(
                    &"run-observation".into(),
                    &TargetedReadRequest {
                        kind: "stdout".into(),
                        assignment_id: Some("worker-0".into()),
                        invocation_id: Some("inv-current".into()),
                        attempt: Some(2),
                        offset: 99,
                        limit: 3,
                        ..TargetedReadRequest::default()
                    },
                    Instant::now() + Duration::from_secs(1)
                )
                .is_err(),
            "invalid byte offset must not be erased"
        );
    }

    #[test]
    fn completion_payload_reference_avoids_scaled_sqlite_row_limit_and_preserves_raw_worker() {
        use rusqlite::limits::Limit;

        let adapter = SqlitePersistence::open_in_memory().expect("open sqlite");
        adapter
            .create_run(create_run("run-scaled-completion"))
            .expect("create run");
        adapter
            .load_show_data(&"run-scaled-completion".into())
            .expect("observe run");
        adapter
            .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
                "run-scaled-completion",
                "inv-scaled",
                "slot-1",
                WorkSlotBinding::new("worker", vec!["--frozen".into()]),
                "digest",
                "subject-v1",
                1,
                Timestamp::from_unix_millis(10),
                1000,
                "/captures/scaled",
            ))
            .expect("create invocation");
        adapter
            .lock()
            .unwrap()
            .set_limit(Limit::SQLITE_LIMIT_LENGTH, 75_000)
            .expect("scaled SQLite length limit");
        let mut worker = InnerWorker::new("worker", vec!["--frozen".into()], 0);
        worker.assignment_id = "task-one".into();
        worker.task_packet = Some(serde_json::Value::String("x".repeat(48_000)));
        let completed = adapter
            .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
                "run-scaled-completion",
                "inv-scaled",
                WaiterWrittenStatus::Succeeded,
                0,
                Timestamp::from_unix_millis(20),
                vec![worker.clone()],
            ))
            .expect("complete under scaled row limit");
        assert_eq!(completed.invocation.inner_workers, vec![worker.clone()]);
        assert!(completed.invocation.recorded_inner_workers.is_empty());
        assert!(completed.invocation.completion_snapshot_sha256.is_some());

        let loaded = adapter
            .load_work_slot_invocation(&"run-scaled-completion".into(), &"inv-scaled".into())
            .expect("load selected invocation")
            .expect("invocation exists");
        assert_eq!(loaded.inner_workers, vec![worker]);
        assert_eq!(
            loaded.inner_workers[0]
                .task_packet
                .as_ref()
                .unwrap()
                .as_str()
                .unwrap()
                .len(),
            48_000
        );
        let connection = adapter.lock().unwrap();
        let (payload_bytes, reference_bytes): (i64, i64) = connection.query_row(
            "SELECT length(inner_workers_json), length(completion_snapshot_json) FROM work_slot_invocations WHERE run_id='run-scaled-completion' AND invocation_id='inv-scaled'",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).expect("inspect compact reference");
        assert!(payload_bytes > 48_000);
        assert!(
            payload_bytes + 48_000 > 75_000,
            "two worker payload copies would exceed the scaled per-row limit"
        );
        assert!(
            reference_bytes < 128,
            "redundant snapshot remains a digest reference, not a second payload"
        );
    }

    #[test]
    fn opening_pre_change_schema_alters_and_writes_new_invocation() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("legacy-invocations.sqlite");
        {
            let connection = Connection::open(&path).expect("open legacy sqlite");
            connection
                .execute_batch(
                    "CREATE TABLE runs (
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
                        created_at                   INTEGER NOT NULL
                    );
                    CREATE TABLE work_slot_invocations (
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
                    );",
                )
                .expect("create pre-change schema");
            connection.execute(
                "INSERT INTO runs (id, workflow_id, workflow_json, provider_association_json, initial_input_json, current_state, lifecycle, control_revision, last_sequence, created_at) VALUES ('historic', 'test-workflow', ?1, '{}', '{}', 'start', 'active', 0, 1, 100)",
                [serde_json::to_string(&workflow()).unwrap()],
            ).expect("retain the pre-change parent run");
            connection.execute(
                "INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json, instruction_digest, subject, waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at) VALUES ('historic', 'old-attempt', 'slot-1', ?1, 'original-digest', 'original-visit', 1, 100, 250, 'failed', 7, 200)",
                [r#"{"command":"old-worker","args":["original-model"]}"#],
            ).expect("retain an actual pre-change invocation row");
        }

        let adapter = SqlitePersistence::open(&path).expect("open after alter");
        let historical =
            read_work_slot_invocations(&adapter.lock().unwrap(), &"historic".into()).unwrap();
        assert_eq!(
            historical[0].binding,
            WorkSlotBinding::new("old-worker", vec!["original-model".into()])
        );
        assert_eq!(historical[0].status, Some(WaiterWrittenStatus::Failed));
        assert_eq!(historical[0].exit_code, Some(7));
        assert_eq!(historical[0].allowed_time_ms, 250);
        assert!(historical[0].controls.is_none());
        assert!(historical[0].binding.context_filter.is_none());
        adapter
            .create_run(create_run("run-legacy-alter"))
            .expect("create run after alter");
        adapter
            .load_show_data(&"run-legacy-alter".into())
            .expect("observe run");
        let created = adapter
            .create_work_slot_invocation(CreateWorkSlotInvocationRequest::new(
                "run-legacy-alter",
                "inv-legacy",
                "slot-1",
                WorkSlotBinding::new("/bin/sh", vec!["-c".to_owned(), "exit 0".to_owned()]),
                "digest",
                "subject-a",
                1,
                Timestamp::from_unix_millis(500),
                1_000,
                "/captures/slot-1/inv-legacy",
            ))
            .expect("create invocation after alter");
        assert_eq!(
            created.invocation.capture_dir,
            "/captures/slot-1/inv-legacy"
        );
        assert!(created.invocation.inner_workers.is_empty());
    }
}
