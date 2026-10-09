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
    CommitTransitionResult, ContextRecord, CreateRunRequest, CreateRunResult, DurableEvaluation,
    DurableEvaluationResult, HistoryAction, HistoryEntry, InnerWorker, InvocationId, Lifecycle,
    Persistence, PersistenceConflict, PersistenceError, PersistenceFailure, PersistenceRejection,
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
    /// captures. This is a historical metadata-only read.
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
        if request.exception.is_some() && request.advice_exception.is_some() {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "sqlite-incompatible-event-exceptions",
                "advice exceptions and general event overrides cannot be combined",
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
        if request.capture_append.is_some() && request.transition.kind.is_check_free() {
            return Err(PersistenceError::failure(PersistenceFailure::new(
                "sqlite-invalid-context-effect",
                "provider capture effects require a checked transition",
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

            require_observed(&raw)?;

            let first_sequence = next_sequence(run.last_sequence)?;
            let revision = next_revision(run.control_revision)?;
            let occurred_at = current_timestamp()?;
            // Provider effects (allow effect, then capture effect) each take one
            // semantic sequence before the transition, in this transaction.
            let mut transition_sequence = first_sequence;
            for effect in [
                request.context_append.as_ref(),
                request.capture_append.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                let sequence = transition_sequence;
                let record_id = generated_context_effect_id(&request.run_id, sequence);
                let data_json = encode_json(&effect.data, "context effect data")?;
                transaction
                    .execute(
                        "INSERT INTO context_records (
                            run_id, record_id, sequence, kind, data_json, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            request.run_id.as_str(),
                            record_id.as_str(),
                            to_sqlite_i64(sequence.as_u64(), "semantic sequence")?,
                            effect.kind,
                            data_json,
                            occurred_at.as_unix_millis(),
                        ],
                    )
                    .map_err(sqlite_failure)?;
                let context_history =
                    HistoryEntry::context_appended(sequence, occurred_at, record_id);
                insert_history(&transaction, &request.run_id, &context_history)?;
                transition_sequence = next_sequence(sequence)?;
            }
            let history = HistoryEntry::transition(
                transition_sequence,
                occurred_at,
                request.transition.clone(),
                match (request.exception.clone(), request.advice_exception.clone()) {
                    (Some(exception), None) => TransitionHistoryOutcome::Overridden { exception },
                    (None, Some(exception)) => {
                        TransitionHistoryOutcome::AdviceException { exception }
                    }
                    (None, None) => TransitionHistoryOutcome::Committed,
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

    fn load_work_slot_invocations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<WorkSlotInvocation>, PersistenceError> {
        let connection = self.lock()?;
        let _ = load_required_run(&connection, run_id)?;
        read_work_slot_invocations(&connection, run_id)
    }
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
        return Err(PersistenceError::failure(PersistenceFailure::new(
            "sqlite",
            "updating run sequence affected no run",
        )));
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

fn status_read_deadline() -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new(
        "status-read-deadline",
        "bounded observation could not finish before its deadline",
    ))
}

fn sqlite_failure(error: rusqlite::Error) -> PersistenceError {
    if matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    ) {
        return PersistenceError::failure(PersistenceFailure::new(
            "catalog-busy",
            format!("another process is holding the catalog ({error}); retry the command after it releases the catalog"),
        ));
    }
    PersistenceError::failure(PersistenceFailure::new("sqlite", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{
        ContextAppendEffect, EvaluationRequest, EvaluationResult, Lifecycle, ProviderAssociation,
        ProviderError, ProviderGateway, State, Transition, Workflow,
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

    struct CapturingGateway {
        result: EvaluationResult,
        capture: Result<Option<ContextAppendEffect>, ProviderError>,
        calls: std::cell::RefCell<Vec<loop_core::CaptureExit>>,
    }

    impl ProviderGateway for CapturingGateway {
        fn describe(
            &self,
            _provider: &ProviderAssociation,
            _initial_input: Option<&serde_json::Value>,
        ) -> Result<Workflow, ProviderError> {
            Ok(checked_event_workflow().with_capture(true))
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            _request: EvaluationRequest,
        ) -> Result<EvaluationResult, ProviderError> {
            Ok(self.result.clone())
        }

        fn capture(
            &self,
            _provider: &ProviderAssociation,
            _request: EvaluationRequest,
            exit: loop_core::CaptureExit,
        ) -> Result<Option<ContextAppendEffect>, ProviderError> {
            self.calls.borrow_mut().push(exit);
            self.capture.clone()
        }
    }

    fn capture_run(path: &std::path::Path, id: &str, capture: bool) -> SqlitePersistence {
        let adapter = SqlitePersistence::open(path).expect("open sqlite");
        let mut request = checked_event_run(id);
        request.workflow = checked_event_workflow().with_capture(capture);
        adapter.create_run(request).expect("create run");
        adapter.load_show_data(&id.into()).expect("observe run");
        adapter
    }

    #[test]
    fn capture_effect_commits_after_allow_effect_and_before_transition() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("capture-allow.sqlite");
        let adapter = capture_run(&path, "run-capture-allow", true);
        let allow = ContextAppendEffect::new("allow-effect", json!({"a": 1}));
        let retained = ContextAppendEffect::new("retained-artifacts", json!({"b": 2}));
        let gateway = CapturingGateway {
            result: EvaluationResult::allow_with_context_append(allow.clone()),
            capture: Ok(Some(retained.clone())),
            calls: Default::default(),
        };
        let outcome = loop_core::operations::event::execute(
            loop_core::operations::event::Request::new("run-capture-allow", "approve"),
            &gateway,
            &adapter,
        );
        assert!(outcome.is_completed(), "checked event failed: {outcome:?}");
        assert_eq!(
            *gateway.calls.borrow(),
            vec![loop_core::CaptureExit::Checked]
        );

        let context = adapter
            .load_context_records(&"run-capture-allow".into())
            .expect("context");
        assert_eq!(context.len(), 2);
        assert_eq!(
            (context[0].kind.as_str(), context[0].sequence.as_u64()),
            ("allow-effect", 2)
        );
        assert_eq!(
            (context[1].kind.as_str(), context[1].sequence.as_u64()),
            ("retained-artifacts", 3)
        );
        assert_eq!(context[1].data, retained.data);
        let history = adapter
            .load_history(&"run-capture-allow".into())
            .expect("history");
        assert_eq!(history.len(), 4);
        assert!(matches!(
            history[3].action,
            loop_core::HistoryAction::Transition {
                outcome: loop_core::TransitionHistoryOutcome::Committed,
                ..
            }
        ));
        let run = adapter
            .load_authoritative_run(&"run-capture-allow".into())
            .expect("run");
        assert_eq!(run.current_state.as_str(), "middle");
        assert_eq!(run.last_sequence.as_u64(), 4);
    }

    #[test]
    fn workflow_without_capture_never_asks_the_provider() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("capture-off.sqlite");
        let adapter = capture_run(&path, "run-capture-off", false);
        let gateway = CapturingGateway {
            result: EvaluationResult::allow(),
            capture: Ok(Some(ContextAppendEffect::new(
                "retained-artifacts",
                json!({}),
            ))),
            calls: Default::default(),
        };
        let outcome = loop_core::operations::event::execute(
            loop_core::operations::event::Request::new("run-capture-off", "approve"),
            &gateway,
            &adapter,
        );
        assert!(outcome.is_completed(), "checked event failed: {outcome:?}");
        assert!(gateway.calls.borrow().is_empty());
        assert!(adapter
            .load_context_records(&"run-capture-off".into())
            .expect("context")
            .is_empty());
    }

    #[test]
    fn override_of_checked_transition_commits_capture_effect() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("capture-override.sqlite");
        let adapter = capture_run(&path, "run-capture-override", true);
        let retained = ContextAppendEffect::new("retained-artifacts", json!({"exit": "override"}));
        let gateway = CapturingGateway {
            result: EvaluationResult::deny(loop_core::EvaluationFeedback::new("no", "never asked")),
            capture: Ok(Some(retained.clone())),
            calls: Default::default(),
        };
        let outcome = loop_core::operations::event::execute(
            loop_core::operations::event::Request::new("run-capture-override", "approve")
                .with_override(loop_core::StateVisitAttestation {
                    state_visit: 0,
                    owner: "owner".into(),
                    reason: "test override".into(),
                }),
            &gateway,
            &adapter,
        );
        assert!(outcome.is_completed(), "override failed: {outcome:?}");
        assert_eq!(
            *gateway.calls.borrow(),
            vec![loop_core::CaptureExit::Override]
        );
        let context = adapter
            .load_context_records(&"run-capture-override".into())
            .expect("context");
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].data, retained.data);
        let history = adapter
            .load_history(&"run-capture-override".into())
            .expect("history");
        assert!(matches!(
            history.last().expect("transition").action,
            loop_core::HistoryAction::Transition {
                outcome: loop_core::TransitionHistoryOutcome::Overridden { .. },
                ..
            }
        ));
    }

    #[test]
    fn capture_failure_refuses_the_event_and_commits_nothing() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("capture-fail.sqlite");
        let adapter = capture_run(&path, "run-capture-fail", true);
        let gateway = CapturingGateway {
            result: EvaluationResult::allow(),
            capture: Err(ProviderError::timeout("capture timed out")),
            calls: Default::default(),
        };
        let outcome = loop_core::operations::event::execute(
            loop_core::operations::event::Request::new("run-capture-fail", "approve"),
            &gateway,
            &adapter,
        );
        assert!(
            !outcome.is_completed(),
            "capture failure must refuse: {outcome:?}"
        );
        let run = adapter
            .load_authoritative_run(&"run-capture-fail".into())
            .expect("run");
        assert_eq!(run.current_state.as_str(), "start");
        assert_eq!(run.control_revision.as_u64(), 0);
        assert!(adapter
            .load_context_records(&"run-capture-fail".into())
            .expect("context")
            .is_empty());
    }

    #[test]
    fn external_work_opens_legacy_invocations_without_rewriting_rows() {
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
            loop_core::WorkSlotBinding::new("old-worker", vec!["original-model".into()])
        );
        assert_eq!(
            historical[0].status,
            Some(loop_core::WaiterWrittenStatus::Failed)
        );
        assert_eq!(historical[0].exit_code, Some(7));
        assert_eq!(historical[0].allowed_time_ms, 250);
        assert!(historical[0].controls.is_none());
        assert!(historical[0].binding.context_filter.is_none());
    }
}
