//! SQLite atomic run mutation transactions (T111).
//!
//! Operation-specific `BEGIN IMMEDIATE` write paths for evidence append, annotation,
//! label replacement, and run termination. Each path re-reads authoritative lifecycle
//! and version columns, allocates journal sequence, validates exact encoded sizes,
//! and commits state/evidence/journal changes atomically.

use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::persistence_commands::{
    AppendAnnotationCommand, AppendEvidenceCommand, CommitStatus, CommittedRunSnapshot,
    ReplaceLabelCommand, TerminateCommit, TerminateRunCommand,
};
use loop_engine_core::capabilities::run_writer::RunWriter;
use loop_engine_core::model::attempt::JournalExtension;
use loop_engine_core::model::bounded::Value as CoreValue;
use loop_engine_core::model::evidence::EvidenceRecord;
use loop_engine_core::model::ids::{RunId, StateId};
use loop_engine_core::model::journal::{
    JournalDraft, JournalEncodedSizes, JournalEntry, JournalError, StateFact,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::reason::ReasonCode;
use loop_engine_core::model::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, params};
use serde_json::{Map, Value, json};
use thiserror::Error;

use super::error::{CommitOutcomeError, PersistenceError};
use super::mapping::{self, MappingError};
use super::records::{JOURNAL_PAYLOAD_SCHEMA_VERSION, JournalRecord};
use super::sqlite::commit::{
    EvidenceAssociationExpectation, JournalBundleExpectation, JournalRowExpectation,
    RunAuthoritativeExpectation, finish_committed_transaction,
};
use super::sqlite::connect_with_pragmas;
use super::traced::{
    MutationClass, OptionalTraceSink, SemanticOutcome, WriteExecution, WriteTraceSession,
    close_write, committed_or_unconfirmed, rollback_open_transaction, run_mutation_error_semantic,
};

const SQL_BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

const SQL_LOAD_RUN: &str = "SELECT current_state, lifecycle, workflow_state_version,
    lifecycle_version, label_version, label
    FROM runs WHERE run_id = ?1";

const SQL_LOAD_SEQUENCE: &str = "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1";

const SQL_BUMP_SEQUENCE: &str =
    "UPDATE run_journal_sequences SET next_sequence = ?1 WHERE run_id = ?2";

const SQL_JOURNAL_EXISTS: &str =
    "SELECT 1 FROM journal_entries WHERE run_id = ?1 AND sequence = ?2 LIMIT 1";

const SQL_EVIDENCE_EXISTS: &str =
    "SELECT 1 FROM evidence WHERE run_id = ?1 AND evidence_id = ?2 LIMIT 1";

const SQL_INSERT_EVIDENCE: &str = "INSERT INTO evidence (
    run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

const SQL_INSERT_JOURNAL: &str = "INSERT INTO journal_entries (
    run_id, sequence, outcome, encoded_payload_json
) VALUES (?1, ?2, ?3, ?4)";

const SQL_UPDATE_LABEL: &str =
    "UPDATE runs SET label = ?1, label_version = label_version + 1 WHERE run_id = ?2";

const SQL_TERMINATE_ACTIVE: &str = "UPDATE runs
    SET lifecycle = 'terminated', lifecycle_version = ?1
    WHERE run_id = ?2 AND lifecycle = 'active' AND lifecycle_version = ?3";

/// SQLite-backed atomic run mutation writer for evidence, annotation, label, and termination.
#[derive(Clone)]
pub struct SqliteRunMutations {
    path: PathBuf,
    trace: OptionalTraceSink,
}

impl std::fmt::Debug for SqliteRunMutations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteRunMutations")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Typed persistence errors for run mutation transactions.
#[derive(Debug, Error)]
pub enum RunMutationError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("journal command run_id does not match authoritative run")]
    RunIdMismatch,
    #[error("stored run data is corrupt: {message}")]
    Corrupt { message: String },
    #[error("evidence id already exists for this run")]
    EvidenceDuplicate,
    #[error("correction link references missing or invalid journal sequence")]
    InvalidCorrectionLink,
    #[error("journal branch does not match authoritative lifecycle/version")]
    JournalBranchMismatch,
    #[error("journal validation failed")]
    Journal(#[from] JournalError),
    #[error("row mapping failed")]
    Mapping(#[from] MappingError),
    #[error("database constraint violation")]
    Constraint,
    #[error("lifecycle version exhausted")]
    VersionExhausted,
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl CommitOutcomeError for RunMutationError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthoritativeRunSnapshot {
    current_state: StateId,
    lifecycle: Lifecycle,
    workflow_state_version: WorkflowStateVersion,
    lifecycle_version: LifecycleVersion,
    label_version: u64,
    label: Option<String>,
    next_journal_sequence: u64,
}

impl SqliteRunMutations {
    /// Untraced bootstrap constructor (tests and internal wiring without an operational trace).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_trace(path, OptionalTraceSink::none())
    }

    pub fn with_trace(path: impl Into<PathBuf>, trace: OptionalTraceSink) -> Self {
        Self {
            path: path.into(),
            trace,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_evidence(
        &self,
        command: AppendEvidenceCommand,
    ) -> Result<CommitStatus, RunMutationError> {
        close_write(
            &self.trace,
            "run.evidence.add",
            MutationClass::RunMutation,
            |trace| {
                self.execute_append_evidence(command, trace)
                    .map_ok(|(status, outcome)| {
                        (status, SemanticOutcome::from_outcome_class(outcome))
                    })
            },
            |(_, semantic)| *semantic,
            run_mutation_error_semantic,
        )
        .map(|(status, _)| status)
    }

    pub fn append_annotation(
        &self,
        command: AppendAnnotationCommand,
    ) -> Result<CommitStatus, RunMutationError> {
        close_write(
            &self.trace,
            "run.annotate",
            MutationClass::RunMutation,
            |trace| {
                self.execute_append_annotation(command, trace)
                    .map_ok(|(status, outcome)| {
                        (status, SemanticOutcome::from_outcome_class(outcome))
                    })
            },
            |(_, semantic)| *semantic,
            run_mutation_error_semantic,
        )
        .map(|(status, _)| status)
    }

    pub fn replace_label(
        &self,
        command: ReplaceLabelCommand,
    ) -> Result<CommitStatus, RunMutationError> {
        close_write(
            &self.trace,
            "run.label",
            MutationClass::RunMutation,
            |trace| {
                self.execute_replace_label(command, trace)
                    .map_ok(|(status, outcome)| {
                        (status, SemanticOutcome::from_outcome_class(outcome))
                    })
            },
            |(_, semantic)| *semantic,
            run_mutation_error_semantic,
        )
        .map(|(status, _)| status)
    }

    pub fn terminate(
        &self,
        command: TerminateRunCommand,
    ) -> Result<TerminateCommit, RunMutationError> {
        close_write(
            &self.trace,
            "run.terminate",
            MutationClass::RunMutation,
            |trace| self.execute_terminate(command, trace),
            |commit| SemanticOutcome::from_outcome_class(commit.outcome),
            run_mutation_error_semantic,
        )
    }

    fn execute_append_evidence(
        &self,
        command: AppendEvidenceCommand,
        _trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(CommitStatus, OutcomeClass), RunMutationError> {
        if let Err(error) = ensure_append_evidence_run_ids(&command) {
            return WriteExecution::no_transaction(error);
        }
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = begin_immediate(&conn) {
            return WriteExecution::no_transaction(error);
        }
        let result = (|| {
            let snapshot = load_authoritative_run(&conn, command.run_id())?;
            let state_before = state_fact(&snapshot);
            let state_after = state_before.clone();
            let branch = select_evidence_append_branch(&command, &conn)?;
            let sequence = allocate_sequence(&conn, command.run_id(), &snapshot)?;
            let (draft, evidence_ids) = match branch {
                EvidenceAppendBranch::Success { evidence, draft } => {
                    insert_evidence(&conn, command.run_id(), &evidence)?;
                    (draft, vec![evidence.id().as_str().to_owned()])
                }
                EvidenceAppendBranch::DuplicateRejection { draft } => (draft, Vec::new()),
            };
            let journal_outcome = draft.outcome();
            let journal = persist_journal(
                &conn,
                command.run_id(),
                draft,
                sequence,
                state_before,
                state_after,
            )?;
            finalize_sequence(
                &conn,
                command.run_id(),
                sequence,
                snapshot.next_journal_sequence,
            )?;
            let expectation = journal_bundle_expectation(
                command.run_id(),
                &snapshot,
                &journal,
                evidence_ids,
                Vec::new(),
                false,
            );
            Ok((
                (
                    commit_status(
                        false,
                        snapshot.workflow_state_version,
                        snapshot.lifecycle_version,
                    ),
                    journal_outcome,
                ),
                expectation,
            ))
        })();
        commit_mutation(&self.path, conn, result)
    }

    fn execute_append_annotation(
        &self,
        command: AppendAnnotationCommand,
        _trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(CommitStatus, OutcomeClass), RunMutationError> {
        if let Err(error) = ensure_draft_run_id(command.run_id(), command.journal_entry()) {
            return WriteExecution::no_transaction(error);
        }
        let journal_outcome = command.journal_entry().outcome();
        let (run_id, corrects_sequence, journal_entry) = command.into_parts();
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = begin_immediate(&conn) {
            return WriteExecution::no_transaction(error);
        }
        let result = (|| {
            let snapshot = load_authoritative_run(&conn, &run_id)?;
            let state_before = state_fact(&snapshot);
            let state_after = state_before.clone();
            let sequence = allocate_sequence(&conn, &run_id, &snapshot)?;
            if let Some(corrects) = corrects_sequence {
                validate_correction_link(&conn, &run_id, corrects, sequence)?;
            }
            let journal = persist_journal(
                &conn,
                &run_id,
                journal_entry,
                sequence,
                state_before,
                state_after,
            )?;
            finalize_sequence(&conn, &run_id, sequence, snapshot.next_journal_sequence)?;
            let expectation = journal_bundle_expectation(
                &run_id,
                &snapshot,
                &journal,
                Vec::new(),
                Vec::new(),
                false,
            );
            Ok((
                (
                    commit_status(
                        false,
                        snapshot.workflow_state_version,
                        snapshot.lifecycle_version,
                    ),
                    journal_outcome,
                ),
                expectation,
            ))
        })();
        commit_mutation(&self.path, conn, result)
    }

    fn execute_replace_label(
        &self,
        command: ReplaceLabelCommand,
        _trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(CommitStatus, OutcomeClass), RunMutationError> {
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = begin_immediate(&conn) {
            return WriteExecution::no_transaction(error);
        }
        let result = (|| {
            let snapshot = load_authoritative_run(&conn, command.run_id())?;
            let label_before = snapshot
                .label
                .as_ref()
                .map(|value| {
                    loop_engine_core::model::bounded::BoundedText::non_empty(
                        "run_label",
                        value.clone(),
                    )
                })
                .transpose()
                .map_err(|error| RunMutationError::Corrupt {
                    message: error.to_string(),
                })?;
            let (run_id, label, completed, terminal_rejection) =
                command.into_transaction_parts(label_before)?;
            ensure_draft_run_id(&run_id, &completed)?;
            ensure_draft_run_id(&run_id, &terminal_rejection)?;
            let state_before = state_fact(&snapshot);
            let mut state_after = state_before.clone();
            let sequence = allocate_sequence(&conn, &run_id, &snapshot)?;
            let (draft, label_changed) = if snapshot.lifecycle == Lifecycle::Active {
                let label_value = label.as_ref().map(|value| value.as_str());
                conn.execute(SQL_UPDATE_LABEL, params![label_value, run_id.as_str()])
                    .map_err(map_sqlite_error)?;
                (completed, true)
            } else if snapshot.lifecycle.is_terminal() {
                (terminal_rejection, false)
            } else {
                return Err(RunMutationError::Corrupt {
                    message: format!("unsupported lifecycle {:?}", snapshot.lifecycle),
                });
            };
            let journal_outcome = draft.outcome();
            if label_changed {
                state_after = state_fact(&load_authoritative_run(&conn, &run_id)?);
            }
            let journal =
                persist_journal(&conn, &run_id, draft, sequence, state_before, state_after)?;
            finalize_sequence(&conn, &run_id, sequence, snapshot.next_journal_sequence)?;
            let refreshed = load_authoritative_run(&conn, &run_id)?;
            let expectation = journal_bundle_expectation(
                &run_id,
                &refreshed,
                &journal,
                Vec::new(),
                Vec::new(),
                label_changed,
            );
            Ok((
                (
                    commit_status(
                        false,
                        refreshed.workflow_state_version,
                        refreshed.lifecycle_version,
                    ),
                    journal_outcome,
                ),
                expectation,
            ))
        })();
        commit_mutation(&self.path, conn, result)
    }

    fn execute_terminate(
        &self,
        command: TerminateRunCommand,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<TerminateCommit, RunMutationError> {
        let (
            command_run_id,
            expected_lifecycle_version,
            completed_entry,
            terminal_rejection_entry,
            stale_error_entry,
        ) = command.into_parts();
        for entry in [
            &completed_entry,
            &terminal_rejection_entry,
            &stale_error_entry,
        ] {
            if let Err(error) = ensure_draft_run_id(&command_run_id, entry) {
                return WriteExecution::no_transaction(error);
            }
        }
        let expected_lifecycle = expected_lifecycle_version.value();
        let run_id = command_run_id.as_str();
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = begin_immediate(&conn) {
            return WriteExecution::no_transaction(error);
        }
        let result = (|| {
            let snapshot = load_authoritative_run(&conn, &command_run_id)?;
            let state_before = state_fact(&snapshot);
            let sequence = allocate_sequence(&conn, &command_run_id, &snapshot)?;
            let branch = classify_terminate_branch(&snapshot, expected_lifecycle_version);
            let (draft, state_after, _lifecycle_changed) = match branch {
                TerminateBranch::Completed => {
                    if let Some(session) = trace {
                        session.version_check_run_cas(run_id, None, Some(expected_lifecycle));
                    }
                    let next_value = snapshot
                        .lifecycle_version
                        .value()
                        .checked_add(1)
                        .ok_or(RunMutationError::VersionExhausted)?;
                    let next_lifecycle = LifecycleVersion::try_from(next_value)
                        .map_err(|_| RunMutationError::VersionExhausted)?;
                    let updated = conn
                        .execute(
                            SQL_TERMINATE_ACTIVE,
                            params![
                                next_lifecycle.value() as i64,
                                command_run_id.as_str(),
                                expected_lifecycle_version.value() as i64,
                            ],
                        )
                        .map_err(map_sqlite_error)?;
                    if updated == 0 {
                        return Err(select_terminate_journal_mismatch(
                            &snapshot,
                            expected_lifecycle_version,
                        ));
                    }
                    let mut after = state_before.clone();
                    after.lifecycle = Lifecycle::Terminated;
                    after.lifecycle_version = next_lifecycle;
                    (completed_entry, after, true)
                }
                TerminateBranch::TerminalRejection => {
                    if !matches_terminal_rejection(&terminal_rejection_entry) {
                        return Err(RunMutationError::JournalBranchMismatch);
                    }
                    (terminal_rejection_entry, state_before.clone(), false)
                }
                TerminateBranch::StaleRejection => {
                    if !matches_stale_rejection(&stale_error_entry) {
                        return Err(RunMutationError::JournalBranchMismatch);
                    }
                    (stale_error_entry, state_before.clone(), false)
                }
            };
            let journal_outcome = draft.outcome();
            let journal = persist_journal(
                &conn,
                &command_run_id,
                draft,
                sequence,
                state_before,
                state_after,
            )?;
            finalize_sequence(
                &conn,
                &command_run_id,
                sequence,
                snapshot.next_journal_sequence,
            )?;
            let run_changed = matches!(branch, TerminateBranch::Completed);
            let refreshed = load_authoritative_run(&conn, &command_run_id)?;
            let expectation = journal_bundle_expectation(
                &command_run_id,
                &refreshed,
                &journal,
                Vec::new(),
                Vec::new(),
                run_changed,
            );
            Ok((
                TerminateCommit {
                    commit: CommitStatus {
                        committed: true,
                        state_changed: false,
                        workflow_state_version: refreshed.workflow_state_version,
                        lifecycle_version: refreshed.lifecycle_version,
                    },
                    outcome: journal_outcome,
                    run: CommittedRunSnapshot {
                        lifecycle: refreshed.lifecycle,
                        current_state: refreshed.current_state,
                        label: refreshed.label,
                    },
                },
                expectation,
            ))
        })();
        commit_mutation(&self.path, conn, result)
    }

    fn connect(&self) -> Result<Connection, RunMutationError> {
        connect_with_pragmas(&self.path).map_err(RunMutationError::from)
    }
}

impl RunWriter for SqliteRunMutations {
    type Error = RunMutationError;

    fn create(
        &self,
        _command: loop_engine_core::capabilities::persistence_commands::CreateRunCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunMutationError::Corrupt {
            message: "run.create persistence is not implemented on SqliteRunMutations".into(),
        })
    }

    fn append_evidence(&self, command: AppendEvidenceCommand) -> Result<CommitStatus, Self::Error> {
        SqliteRunMutations::append_evidence(self, command)
    }

    fn append_annotation(
        &self,
        command: AppendAnnotationCommand,
    ) -> Result<CommitStatus, Self::Error> {
        SqliteRunMutations::append_annotation(self, command)
    }

    fn replace_label(&self, command: ReplaceLabelCommand) -> Result<CommitStatus, Self::Error> {
        SqliteRunMutations::replace_label(self, command)
    }

    fn terminate(&self, command: TerminateRunCommand) -> Result<TerminateCommit, Self::Error> {
        SqliteRunMutations::terminate(self, command)
    }

    fn append_guidance_attempt(
        &self,
        _command: loop_engine_core::capabilities::persistence_commands::AppendGuidanceAttemptCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunMutationError::Corrupt {
            message: "guidance persistence is not implemented on SqliteRunMutations".into(),
        })
    }

    fn append_compatibility_attempt(
        &self,
        _command: loop_engine_core::capabilities::persistence_commands::AppendCompatibilityAttemptCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunMutationError::Corrupt {
            message: "compatibility persistence is not implemented on SqliteRunMutations".into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminateBranch {
    Completed,
    TerminalRejection,
    StaleRejection,
}

struct PersistedJournal {
    sequence: u64,
    outcome: String,
    payload: String,
}

fn classify_terminate_branch(
    snapshot: &AuthoritativeRunSnapshot,
    expected: LifecycleVersion,
) -> TerminateBranch {
    if snapshot.lifecycle.is_terminal() {
        TerminateBranch::TerminalRejection
    } else if snapshot.lifecycle_version != expected {
        TerminateBranch::StaleRejection
    } else if snapshot.lifecycle == Lifecycle::Active {
        TerminateBranch::Completed
    } else {
        TerminateBranch::TerminalRejection
    }
}

fn select_terminate_journal_mismatch(
    snapshot: &AuthoritativeRunSnapshot,
    expected: LifecycleVersion,
) -> RunMutationError {
    if snapshot.lifecycle.is_terminal() || snapshot.lifecycle_version != expected {
        RunMutationError::JournalBranchMismatch
    } else {
        RunMutationError::Corrupt {
            message: "terminate CAS update affected zero rows".into(),
        }
    }
}

enum EvidenceAppendBranch {
    Success {
        evidence: EvidenceRecord,
        draft: JournalDraft,
    },
    DuplicateRejection {
        draft: JournalDraft,
    },
}

fn select_evidence_append_branch(
    command: &AppendEvidenceCommand,
    conn: &Connection,
) -> Result<EvidenceAppendBranch, RunMutationError> {
    let Some(evidence) = command.evidence().cloned() else {
        if matches_pre_resolved_rejection(command.duplicate_rejection_entry()) {
            return Ok(EvidenceAppendBranch::DuplicateRejection {
                draft: command.duplicate_rejection_entry().clone(),
            });
        }
        return Err(RunMutationError::JournalBranchMismatch);
    };

    let duplicate = evidence_exists(conn, command.run_id(), evidence.id().as_str())?;

    if duplicate {
        if matches_evidence_duplicate_rejection(command.duplicate_rejection_entry()) {
            return Ok(EvidenceAppendBranch::DuplicateRejection {
                draft: command.duplicate_rejection_entry().clone(),
            });
        }
        return Err(RunMutationError::JournalBranchMismatch);
    }

    if matches_evidence_success(command.completed_entry(), &evidence) {
        Ok(EvidenceAppendBranch::Success {
            evidence,
            draft: command.completed_entry().clone(),
        })
    } else {
        Err(RunMutationError::JournalBranchMismatch)
    }
}

fn matches_evidence_success(draft: &JournalDraft, evidence: &EvidenceRecord) -> bool {
    draft.outcome() == OutcomeClass::Completed
        && matches!(
            draft.extension(),
            JournalExtension::EvidenceAdded { added: Some(added) }
                if added.evidence_id == *evidence.id()
                    && added.kind == *evidence.kind()
                    && added.locator.as_str() == evidence.locator()
                    && added.digest.as_ref().map(|value| value.as_str()) == evidence.digest()
        )
}

fn matches_pre_resolved_rejection(draft: &JournalDraft) -> bool {
    matches!(
        draft.extension(),
        JournalExtension::EvidenceAdded { added: None }
    )
}

fn matches_evidence_duplicate_rejection(draft: &JournalDraft) -> bool {
    draft.outcome() == OutcomeClass::Rejected
        && draft.reason().map(|reason| reason.code()) == Some(ReasonCode::EvidenceInvalid)
        && matches!(
            draft.extension(),
            JournalExtension::EvidenceAdded { added: None }
        )
}

fn matches_terminal_rejection(draft: &JournalDraft) -> bool {
    draft.outcome() == OutcomeClass::Rejected
        && draft.reason().map(|reason| reason.code()) == Some(ReasonCode::RunLifecycleTerminal)
}

fn matches_stale_rejection(draft: &JournalDraft) -> bool {
    draft.outcome() == OutcomeClass::Error
        && draft.reason().map(|reason| reason.code()) == Some(ReasonCode::StateStaleVersion)
}

fn ensure_append_evidence_run_ids(command: &AppendEvidenceCommand) -> Result<(), RunMutationError> {
    ensure_draft_run_id(command.run_id(), command.duplicate_rejection_entry())?;
    if command.evidence().is_some() {
        ensure_draft_run_id(command.run_id(), command.completed_entry())?;
    }
    Ok(())
}

fn ensure_draft_run_id(run_id: &RunId, draft: &JournalDraft) -> Result<(), RunMutationError> {
    if draft.run_id() == run_id {
        Ok(())
    } else {
        Err(RunMutationError::RunIdMismatch)
    }
}

fn commit_mutation<T>(
    path: &Path,
    conn: Connection,
    result: Result<(T, JournalBundleExpectation), RunMutationError>,
) -> WriteExecution<T, RunMutationError> {
    match result {
        Ok((value, expectation)) => committed_or_unconfirmed(finish_committed_transaction(
            path,
            conn,
            value,
            |read| expectation.verify(read),
            map_sqlite_error,
            || RunMutationError::CommitOutcomeUnverified,
            || RunMutationError::CommitIntegrityFailure,
            RunMutationError::from,
        )),
        Err(error) => rollback_open_transaction(&conn, error),
    }
}

fn journal_bundle_expectation(
    run_id: &RunId,
    snapshot: &AuthoritativeRunSnapshot,
    journal: &PersistedJournal,
    evidence_ids: Vec<String>,
    associations: Vec<EvidenceAssociationExpectation>,
    run_changed: bool,
) -> JournalBundleExpectation {
    JournalBundleExpectation {
        run_changed,
        run: RunAuthoritativeExpectation {
            run_id: run_id.as_str().to_owned(),
            current_state: snapshot.current_state.as_str().to_owned(),
            lifecycle: lifecycle_label(snapshot.lifecycle).to_owned(),
            workflow_state_version: snapshot.workflow_state_version.value(),
            lifecycle_version: snapshot.lifecycle_version.value(),
            label: snapshot.label.clone(),
            label_version: snapshot.label_version,
            next_sequence: journal.sequence + 1,
        },
        journal: JournalRowExpectation {
            run_id: run_id.as_str().to_owned(),
            sequence: journal.sequence,
            outcome: journal.outcome.clone(),
            payload: journal.payload.clone(),
        },
        evidence_ids,
        associations,
    }
}

fn begin_immediate(conn: &Connection) -> Result<(), RunMutationError> {
    conn.execute(SQL_BEGIN_IMMEDIATE, [])
        .map_err(map_sqlite_error)
        .map(|_| ())
}

fn load_authoritative_run(
    conn: &Connection,
    run_id: &RunId,
) -> Result<AuthoritativeRunSnapshot, RunMutationError> {
    let run_row = conn
        .query_row(SQL_LOAD_RUN, params![run_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .optional()
        .map_err(map_sqlite_error)?
        .ok_or_else(|| RunMutationError::NotFound {
            run_id: run_id.clone(),
        })?;
    let next_journal_sequence = conn
        .query_row(SQL_LOAD_SEQUENCE, params![run_id.as_str()], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map_err(map_sqlite_error)?
        .ok_or_else(|| RunMutationError::Corrupt {
            message: "run_journal_sequences row missing".into(),
        })?;
    if next_journal_sequence <= 0 {
        return Err(RunMutationError::Corrupt {
            message: "journal sequence must be positive".into(),
        });
    }
    let workflow_state_version =
        WorkflowStateVersion::try_from(run_row.2 as u64).map_err(|_| {
            RunMutationError::Corrupt {
                message: "invalid workflow_state_version".into(),
            }
        })?;
    let lifecycle_version =
        LifecycleVersion::try_from(run_row.3 as u64).map_err(|_| RunMutationError::Corrupt {
            message: "invalid lifecycle_version".into(),
        })?;
    Ok(AuthoritativeRunSnapshot {
        current_state: StateId::parse(run_row.0).map_err(|error| RunMutationError::Corrupt {
            message: error.to_string(),
        })?,
        lifecycle: parse_lifecycle(&run_row.1)?,
        workflow_state_version,
        lifecycle_version,
        label_version: run_row.4 as u64,
        label: run_row.5,
        next_journal_sequence: next_journal_sequence as u64,
    })
}

fn parse_lifecycle(value: &str) -> Result<Lifecycle, RunMutationError> {
    match value {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        other => Err(RunMutationError::Corrupt {
            message: format!("unsupported lifecycle {other}"),
        }),
    }
}

fn lifecycle_label(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

fn state_fact(snapshot: &AuthoritativeRunSnapshot) -> StateFact {
    StateFact {
        state: snapshot.current_state.clone(),
        lifecycle: snapshot.lifecycle,
        workflow_state_version: snapshot.workflow_state_version,
        lifecycle_version: snapshot.lifecycle_version,
    }
}

fn commit_status(
    state_changed: bool,
    workflow_state_version: WorkflowStateVersion,
    lifecycle_version: LifecycleVersion,
) -> CommitStatus {
    CommitStatus {
        committed: true,
        state_changed,
        workflow_state_version,
        lifecycle_version,
    }
}

fn allocate_sequence(
    _conn: &Connection,
    _run_id: &RunId,
    snapshot: &AuthoritativeRunSnapshot,
) -> Result<JournalSequence, RunMutationError> {
    JournalSequence::try_from(snapshot.next_journal_sequence).map_err(|_| {
        RunMutationError::Corrupt {
            message: "invalid journal sequence".into(),
        }
    })
}

fn finalize_sequence(
    conn: &Connection,
    run_id: &RunId,
    sequence: JournalSequence,
    prior_next: u64,
) -> Result<(), RunMutationError> {
    let expected = sequence
        .next_sequence()
        .ok_or(RunMutationError::Corrupt {
            message: "journal sequence overflow".into(),
        })?
        .value();
    if prior_next != sequence.value() {
        return Err(RunMutationError::Corrupt {
            message: "sequence allocation diverged from authoritative row".into(),
        });
    }
    conn.execute(SQL_BUMP_SEQUENCE, params![expected as i64, run_id.as_str()])
        .map_err(map_sqlite_error)
        .map(|_| ())?;
    Ok(())
}

fn validate_correction_link(
    conn: &Connection,
    run_id: &RunId,
    corrects: JournalSequence,
    allocated: JournalSequence,
) -> Result<(), RunMutationError> {
    if corrects >= allocated {
        return Err(RunMutationError::InvalidCorrectionLink);
    }
    let exists = conn
        .query_row(
            SQL_JOURNAL_EXISTS,
            params![run_id.as_str(), corrects.value() as i64],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_sqlite_error)?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(RunMutationError::InvalidCorrectionLink)
    }
}

fn evidence_exists(
    conn: &Connection,
    run_id: &RunId,
    evidence_id: &str,
) -> Result<bool, RunMutationError> {
    Ok(conn
        .query_row(
            SQL_EVIDENCE_EXISTS,
            params![run_id.as_str(), evidence_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_sqlite_error)?
        .is_some())
}

fn insert_evidence(
    conn: &Connection,
    run_id: &RunId,
    evidence: &EvidenceRecord,
) -> Result<(), RunMutationError> {
    let row = mapping::evidence_record_row(run_id, evidence)?;
    conn.execute(
        SQL_INSERT_EVIDENCE,
        params![
            row.run_id,
            row.evidence_id,
            row.kind,
            row.locator,
            row.digest,
            row.media_type,
            row.metadata_json,
            row.source,
            row.created_at,
        ],
    )
    .map_err(map_sqlite_error)
    .map(|_| ())
}

fn persist_journal(
    conn: &Connection,
    run_id: &RunId,
    draft: JournalDraft,
    sequence: JournalSequence,
    state_before: StateFact,
    state_after: StateFact,
) -> Result<PersistedJournal, RunMutationError> {
    let (entry, payload_json, outcome) =
        finalize_journal_entry(draft, sequence, state_before, state_after)?;
    mapping::validate_journal_record(&JournalRecord {
        run_id: run_id.as_str().to_owned(),
        sequence: entry.sequence().value(),
        outcome: outcome.clone(),
        encoded_payload_json: payload_json.clone(),
    })
    .map_err(|error| RunMutationError::Corrupt {
        message: error.to_string(),
    })?;
    conn.execute(
        SQL_INSERT_JOURNAL,
        params![
            run_id.as_str(),
            entry.sequence().value() as i64,
            outcome,
            payload_json,
        ],
    )
    .map_err(map_sqlite_error)
    .map(|_| ())?;
    Ok(PersistedJournal {
        sequence: entry.sequence().value(),
        outcome,
        payload: payload_json,
    })
}

fn finalize_journal_entry(
    draft: JournalDraft,
    sequence: JournalSequence,
    state_before: StateFact,
    state_after: StateFact,
) -> Result<(JournalEntry, String, String), RunMutationError> {
    let encoded_sizes = measure_encoded_sizes(&draft, sequence, &state_before, &state_after)?;
    let entry = draft.finalize(sequence, state_before, state_after, encoded_sizes)?;
    let payload = encode_journal_entry(&entry)?;
    if payload.len() != entry.encoded_size() {
        return Err(RunMutationError::Corrupt {
            message: "journal encoded size mismatch after finalize".into(),
        });
    }
    let outcome = outcome_label(entry.outcome()).to_owned();
    Ok((entry, payload, outcome))
}

fn measure_encoded_sizes(
    draft: &JournalDraft,
    sequence: JournalSequence,
    state_before: &StateFact,
    state_after: &StateFact,
) -> Result<JournalEncodedSizes, JournalError> {
    let provisional = provisional_encoded_sizes(draft);
    let entry = draft.clone().finalize(
        sequence,
        state_before.clone(),
        state_after.clone(),
        provisional,
    )?;
    let payload = encode_journal_entry(&entry)?;
    Ok(measured_encoded_sizes(&entry, &payload))
}

fn provisional_encoded_sizes(draft: &JournalDraft) -> JournalEncodedSizes {
    let attempt = draft.attempt();
    let marker = |present: bool| if present { 1 } else { 0 };
    JournalEncodedSizes {
        entry: 1,
        evidence_associations: marker(
            attempt
                .and_then(|facts| facts.evidence_associations.as_ref())
                .is_some(),
        ),
        provider_observations: marker(
            attempt.is_some_and(|facts| !facts.provider_observations.is_empty()),
        ),
        gate_verdict_facts: marker(
            attempt
                .and_then(|facts| facts.gate_verdict_facts.as_ref())
                .is_some(),
        ),
        diagnostics: marker(attempt.is_some_and(|facts| !facts.diagnostics.is_empty())),
        note: marker(attempt.and_then(|facts| facts.note.as_ref()).is_some()),
        actor: marker(attempt.and_then(|facts| facts.actor.as_ref()).is_some()),
    }
}

fn measured_encoded_sizes(entry: &JournalEntry, payload_json: &str) -> JournalEncodedSizes {
    let wire: Value =
        serde_json::from_str(payload_json).expect("journal payload must be valid JSON");
    JournalEncodedSizes {
        entry: payload_json.len(),
        evidence_associations: component_len(&wire, "evidence_associations"),
        provider_observations: component_len(&wire, "provider_observations"),
        gate_verdict_facts: component_len(&wire, "gate_verdict_facts"),
        diagnostics: diagnostics_len(entry.attempt()),
        note: wire
            .get("note")
            .and_then(Value::as_str)
            .map(str::len)
            .unwrap_or(0),
        actor: component_len(&wire, "actor"),
    }
}

fn component_len(wire: &Value, field: &str) -> usize {
    wire.get(field)
        .map(|value| serde_json::to_string(value).unwrap_or_default().len())
        .unwrap_or(0)
}

fn diagnostics_len(attempt: Option<&loop_engine_core::model::attempt::AttemptFacts>) -> usize {
    attempt
        .map(|facts| {
            facts
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    json!({
                        "code": diagnostic.code(),
                        "message": diagnostic.message(),
                        "path": diagnostic.path(),
                    })
                })
                .map(|value| serde_json::to_string(&value).unwrap_or_default().len())
                .sum()
        })
        .unwrap_or(0)
}

/// Render one authoritative journal entry using persisted wire vocabulary.
pub fn journal_entry_value(entry: &JournalEntry) -> Result<Value, JournalError> {
    let encoded = encode_journal_entry(entry)?;
    serde_json::from_str(&encoded).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_entry",
        })
    })
}

fn encode_journal_entry(entry: &JournalEntry) -> Result<String, JournalError> {
    encode_journal_entry_from_parts(
        entry.sequence(),
        entry.run_id(),
        mapping::format_observed_at(&entry.observed_at()),
        entry.operation(),
        entry.request_id().as_str(),
        entry.kind(),
        entry.outcome(),
        entry.reason(),
        entry.state_before(),
        entry.state_after(),
        entry.attempt(),
        entry.extension(),
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_journal_entry_from_parts(
    sequence: JournalSequence,
    run_id: &RunId,
    ts: String,
    operation: &str,
    request_id: &str,
    kind: loop_engine_core::model::journal::JournalEntryKind,
    outcome: OutcomeClass,
    reason: Option<&loop_engine_core::model::reason::Reason>,
    state_before: &StateFact,
    state_after: &StateFact,
    attempt: Option<&loop_engine_core::model::attempt::AttemptFacts>,
    extension: &loop_engine_core::model::attempt::JournalExtension,
) -> Result<String, JournalError> {
    let mut payload = Map::new();
    payload.insert(
        "journal_schema_version".into(),
        json!(JOURNAL_PAYLOAD_SCHEMA_VERSION),
    );
    payload.insert("sequence".into(), json!(sequence.value()));
    payload.insert("run_id".into(), json!(run_id.as_str()));
    payload.insert("ts".into(), json!(ts));
    payload.insert("operation".into(), json!(operation));
    payload.insert("request_id".into(), json!(request_id));
    payload.insert("entry_kind".into(), json!(entry_kind_label(kind)));
    payload.insert("outcome".into(), json!(outcome_label(outcome)));
    payload.insert(
        "reason".into(),
        match reason {
            None => Value::Null,
            Some(value) => json!({
                "code": value.code().code(),
                "message": value.message(),
            }),
        },
    );
    payload.insert("state_before".into(), state_fact_json(state_before));
    payload.insert("state_after".into(), state_fact_json(state_after));
    if let Some(facts) = attempt {
        if let Some(note) = &facts.note {
            payload.insert("note".into(), json!(note.as_str()));
        }
        if let Some(actor) = &facts.actor {
            payload.insert("actor".into(), core_value_to_json(actor.value()));
        }
        if let Some(corrects) = facts.corrects_sequence {
            payload.insert("corrects_sequence".into(), json!(corrects.value()));
        }
        if let Some(associations) = &facts.evidence_associations {
            payload.insert(
                "evidence_associations".into(),
                serde_json::from_str(&encode_evidence_associations(associations)?).unwrap(),
            );
        }
        if let Some(recorded) = facts.evidence_recorded {
            payload.insert(
                "evidence_recorded".into(),
                json!({
                    "inline": recorded.inline,
                    "selected_associations": recorded.selected_associations,
                    "provider": recorded.provider,
                }),
            );
        }
        if !facts.provider_observations.is_empty() {
            payload.insert(
                "provider_observations".into(),
                serde_json::from_str(&encode_provider_observations(&facts.provider_observations)?)
                    .unwrap(),
            );
        }
        if let Some(gate_facts) = &facts.gate_verdict_facts {
            payload.insert(
                "gate_verdict_facts".into(),
                serde_json::from_str(&encode_gate_verdict_facts(gate_facts)?).unwrap(),
            );
        }
        if let Some(transition) = &facts.transition {
            payload.insert(
                "transition".into(),
                json!({
                    "event": transition.event.as_str(),
                    "source_state": transition.source.as_str(),
                    "target_state": transition.target.as_ref().map(|value| value.as_str()),
                    "applied": transition.applied,
                }),
            );
        }
        if !facts.diagnostics.is_empty() {
            payload.insert(
                "diagnostics".into(),
                serde_json::from_str(&encode_diagnostics(&facts.diagnostics)?).unwrap(),
            );
        }
    }
    match extension {
        loop_engine_core::model::attempt::JournalExtension::RunCreated { graph_revision } => {
            payload.insert("graph_revision".into(), json!(graph_revision.as_str()));
        }
        loop_engine_core::model::attempt::JournalExtension::EvidenceAdded { added } => {
            if let Some(added) = added {
                payload.insert("evidence_id".into(), json!(added.evidence_id.as_str()));
                payload.insert("kind".into(), json!(added.kind.as_str()));
                payload.insert("locator".into(), json!(added.locator.as_str()));
                if let Some(digest) = &added.digest {
                    payload.insert("digest".into(), json!(digest.as_str()));
                }
            }
        }
        loop_engine_core::model::attempt::JournalExtension::LabelChanged { change } => {
            if let Some(change) = change {
                payload.insert(
                    "label_before".into(),
                    match &change.label_before {
                        None => Value::Null,
                        Some(value) => json!(value.as_str()),
                    },
                );
                payload.insert(
                    "label_after".into(),
                    match &change.label_after {
                        None => Value::Null,
                        Some(value) => json!(value.as_str()),
                    },
                );
            }
        }
        loop_engine_core::model::attempt::JournalExtension::GuidanceAttempt { guidance_text } => {
            if let Some(text) = guidance_text {
                payload.insert("guidance_text".into(), json!(text.as_str()));
            }
        }
        loop_engine_core::model::attempt::JournalExtension::CompatibilityAttempt { findings } => {
            if let Some(findings) = findings {
                payload.insert(
                    "findings".into(),
                    json!(
                        findings
                            .as_slice()
                            .iter()
                            .map(|entry| {
                                json!({
                                    "capability": entry.capability(),
                                    "status": compatibility_status_label(entry.status()),
                                    "message": entry
                                        .diagnostics()
                                        .first()
                                        .map(|diagnostic| diagnostic.message()),
                                })
                            })
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        loop_engine_core::model::attempt::JournalExtension::Annotation
        | loop_engine_core::model::attempt::JournalExtension::TransitionAttempt
        | loop_engine_core::model::attempt::JournalExtension::RunTerminated => {}
    }
    serde_json::to_string(&Value::Object(payload)).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_entry",
        })
    })
}

fn state_fact_json(fact: &StateFact) -> Value {
    json!({
        "state": fact.state.as_str(),
        "lifecycle": lifecycle_label(fact.lifecycle),
        "workflow_state_version": fact.workflow_state_version.value(),
        "lifecycle_version": fact.lifecycle_version.value(),
    })
}

fn entry_kind_label(kind: loop_engine_core::model::journal::JournalEntryKind) -> &'static str {
    use loop_engine_core::model::journal::JournalEntryKind;
    match kind {
        JournalEntryKind::RunCreated => "run.created",
        JournalEntryKind::EvidenceAdded => "evidence.added",
        JournalEntryKind::Annotation => "annotation",
        JournalEntryKind::LabelChanged => "label.changed",
        JournalEntryKind::TransitionAttempt => "transition.attempt",
        JournalEntryKind::GuidanceAttempt => "guidance.attempt",
        JournalEntryKind::CompatibilityAttempt => "compatibility.attempt",
        JournalEntryKind::RunTerminated => "run.terminated",
    }
}

fn outcome_label(outcome: OutcomeClass) -> &'static str {
    match outcome {
        OutcomeClass::Completed => "completed",
        OutcomeClass::Rejected => "rejected",
        OutcomeClass::Error => "error",
    }
}

fn encode_evidence_associations(
    associations: &loop_engine_core::model::attempt::EvidenceAssociations,
) -> Result<String, JournalError> {
    let payload = json!({
        "inline": associations.inline.iter().map(|record| {
            json!({
                "evidence_id": record.id().as_str(),
                "kind": record.kind().as_str(),
                "locator": record.locator(),
            })
        }).collect::<Vec<_>>(),
        "selected_ids": associations.selected_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
        "provider_recorded_ids": associations.provider_recorded_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
    });
    serde_json::to_string(&payload).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_evidence_associations",
        })
    })
}

fn encode_provider_observations(
    observations: &[loop_engine_core::model::attempt::ProviderFact],
) -> Result<String, JournalError> {
    let payload = observations
        .iter()
        .map(|observation| {
            let mut object = Map::new();
            object.insert(
                "registration_id".into(),
                json!(observation.registration_id.as_str()),
            );
            object.insert("config_revision".into(), json!(observation.config_revision));
            object.insert("role".into(), json!(provider_role_label(observation.role)));
            object.insert(
                "invocation_id".into(),
                json!(observation.invocation_id.as_str()),
            );
            object.insert("executable".into(), json!(observation.executable.as_str()));
            object.insert("outcome".into(), json!(outcome_label(observation.outcome)));
            match &observation.digest {
                loop_engine_core::model::provider::DigestObservation::Observed(digest) => {
                    object.insert("executable_digest".into(), json!(digest.as_str()));
                }
                loop_engine_core::model::provider::DigestObservation::Unavailable => {}
            }
            if let Some(version) = &observation.provider_version {
                object.insert("provider_version".into(), json!(version.as_str()));
            }
            if let Some(major) = observation.protocol_major {
                object.insert("protocol_major".into(), json!(major));
            }
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&payload).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_provider_facts",
        })
    })
}

fn encode_gate_verdict_facts(
    facts: &loop_engine_core::model::attempt::GateVerdictFacts,
) -> Result<String, JournalError> {
    use loop_engine_core::model::attempt::GateVerdictResult;
    let mut payload = Map::new();
    payload.insert("event".into(), json!(facts.event.as_str()));
    payload.insert(
        "gate_ids".into(),
        json!(
            facts
                .gate_ids
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
        ),
    );
    match &facts.result {
        GateVerdictResult::Verdicts(verdicts) => {
            payload.insert(
                "verdicts".into(),
                json!(
                    verdicts
                        .iter()
                        .map(|verdict| {
                            json!({
                                "gate_id": verdict.gate_id.as_str(),
                                "status": if verdict.passed { "pass" } else { "fail" },
                                "message": verdict.message.as_ref().map(|value| value.as_str()),
                            })
                        })
                        .collect::<Vec<_>>()
                ),
            );
        }
        GateVerdictResult::Incompatibility(diagnostic) => {
            payload.insert(
                "incompatibility".into(),
                json!({
                    "code": diagnostic.code(),
                    "message": diagnostic.message(),
                    "path": diagnostic.path(),
                }),
            );
        }
        GateVerdictResult::EvaluationError(diagnostics) => {
            payload.insert(
                "evaluation_error".into(),
                json!(
                    diagnostics
                        .as_slice()
                        .iter()
                        .map(|diagnostic| {
                            json!({
                                "code": diagnostic.code(),
                                "message": diagnostic.message(),
                                "path": diagnostic.path(),
                            })
                        })
                        .collect::<Vec<_>>()
                ),
            );
        }
    }
    serde_json::to_string(&Value::Object(payload)).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_gate_verdict_facts",
        })
    })
}

fn encode_diagnostics(
    diagnostics: &[loop_engine_core::model::diagnostic::Diagnostic],
) -> Result<String, JournalError> {
    let payload = diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "code": diagnostic.code(),
                "message": diagnostic.message(),
                "path": diagnostic.path(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&payload).map_err(|_| {
        JournalError::Bound(loop_engine_core::model::bounded::BoundError::InvalidType {
            field: "journal_diagnostics",
        })
    })
}

fn compatibility_status_label(
    status: loop_engine_core::model::compatibility::CompatibilityStatus,
) -> &'static str {
    use loop_engine_core::model::compatibility::CompatibilityStatus;
    match status {
        CompatibilityStatus::Compatible => "compatible",
        CompatibilityStatus::Incompatible => "incompatible",
        CompatibilityStatus::Unknown => "unknown",
    }
}

fn provider_role_label(role: loop_engine_core::model::attempt::ProviderRole) -> &'static str {
    use loop_engine_core::model::attempt::ProviderRole;
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

fn core_value_to_json(value: &CoreValue) -> Value {
    match value {
        CoreValue::Null => Value::Null,
        CoreValue::Bool(value) => json!(*value),
        CoreValue::Number(number) => json!(number.value()),
        CoreValue::String(value) => json!(value),
        CoreValue::Array(values) => {
            json!(values.iter().map(core_value_to_json).collect::<Vec<_>>())
        }
        CoreValue::Object(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                map.insert(key.clone(), core_value_to_json(value));
            }
            Value::Object(map)
        }
    }
}

fn map_sqlite_error(error: SqliteError) -> RunMutationError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        RunMutationError::Constraint
    } else {
        RunMutationError::Corrupt {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::persistence_commands::AppendEvidenceCommand;
    use loop_engine_core::model::annotation::{ActorMetadata, Note};
    use loop_engine_core::model::attempt::{
        AttemptFacts, EvidenceAddedFact, EvidenceAssociations, JournalExtension, LabelChangeFact,
    };
    use loop_engine_core::model::bounded::{BoundedText, Value as CoreValue};
    use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
    use loop_engine_core::model::ids::{EvidenceId, EvidenceKind, RequestId, RunId};
    use loop_engine_core::model::journal::JournalDraft;
    use loop_engine_core::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::model::run::Run;
    use loop_engine_core::model::time::ObservedAt;
    use loop_engine_core::model::version::JournalSequence;
    use loop_engine_core::operations::{evidence_add, run_annotate, run_label, run_terminate};
    use rusqlite::{Connection, params};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::mapping;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::records::{GV01_CANONICAL_GRAPH_JSON, GV01_GRAPH_REVISION, RunRecord};
    use crate::persistence::run_reads::SqliteRunReads;
    use crate::persistence::sqlite::open_at;

    const INITIAL_FINAL_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"done","input_declarations":[],"live_guidance_supported":false,"states":[{"final":true,"id":"done","static_guidance":{"kind":"none"}}],"transitions":[]}"#;

    fn graph_revision_for(
        canonical_graph_json: &str,
        current_state: &str,
        lifecycle: &str,
    ) -> String {
        let record = RunRecord {
            run_id: "019f0000-0000-7000-8000-000000000000".into(),
            registration_id: "reg-1".into(),
            config_revision_at_create: 1,
            current_state: current_state.into(),
            lifecycle: lifecycle.into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: None,
            graph_revision:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            canonical_graph_version: 1,
            graph_canonical_projection_json: canonical_graph_json.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:00.000Z".into(),
        };
        match mapping::run_from_record(&record) {
            Err(mapping::MappingError::GraphDigestMismatch { computed, .. }) => computed,
            Ok(run) => run.graph_revision().as_str().to_owned(),
            other => panic!("unexpected mapping result: {other:?}"),
        }
    }

    fn test_mutations() -> (TempDir, SqliteRunMutations) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        (dir, SqliteRunMutations::new(path))
    }

    fn insert_registration(conn: &Connection, registration_id: &str) {
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
            ) VALUES (?1, 'provider-a', 1, 1, '/bin/provider', '[]', '/work', 60,
                      '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            params![registration_id],
        )
        .unwrap();
    }

    fn insert_run(conn: &Connection, run_id: &str, registration_id: &str, lifecycle: &str) {
        let (current_state, graph_json, graph_revision) = if lifecycle == "final" {
            (
                "done",
                INITIAL_FINAL_GRAPH_JSON,
                graph_revision_for(INITIAL_FINAL_GRAPH_JSON, "done", "final"),
            )
        } else {
            (
                "draft",
                GV01_CANONICAL_GRAPH_JSON,
                GV01_GRAPH_REVISION.to_owned(),
            )
        };
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES (?1, ?2, 1, ?3, ?4, 1, 1, 1, NULL, ?5, 1, ?6, '{}', '2026-07-17T12:00:00.000Z')",
            params![
                run_id,
                registration_id,
                current_state,
                lifecycle,
                graph_revision,
                graph_json,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
            params![run_id],
        )
        .unwrap();
    }

    fn seed_journal(conn: &Connection, run_id: &str, sequence: i64) {
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
             VALUES (?1, ?2, 'completed', '{\"journal_schema_version\":1,\"sequence\":1,\"run_id\":\"x\",\"outcome\":\"completed\"}')",
            params![run_id, sequence],
        )
        .unwrap();
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = ?1 WHERE run_id = ?2",
            params![sequence + 1, run_id],
        )
        .unwrap();
    }

    fn evidence_attempt() -> AttemptFacts {
        AttemptFacts {
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        }
    }

    fn evidence_record(id: &str) -> EvidenceRecord {
        EvidenceRecord::new(
            EvidenceId::parse(id).unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            "opaque:locator",
            None,
            None,
            None,
            EvidenceSource::Caller,
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
        )
        .unwrap()
    }

    fn evidence_draft(run_id: &RunId, evidence: &EvidenceRecord) -> JournalDraft {
        JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            RequestId::parse("request-evidence").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(evidence_attempt()),
            JournalExtension::EvidenceAdded {
                added: Some(EvidenceAddedFact {
                    evidence_id: evidence.id().clone(),
                    kind: evidence.kind().clone(),
                    locator: BoundedText::opaque_non_empty("evidence_locator", evidence.locator())
                        .unwrap(),
                    digest: None,
                }),
            },
        )
        .unwrap()
    }

    fn annotation_draft(
        run_id: &RunId,
        note: Option<Note>,
        actor: Option<ActorMetadata>,
        corrects: Option<JournalSequence>,
    ) -> JournalDraft {
        JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.annotate",
            RequestId::parse("request-annotate").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(AttemptFacts {
                note,
                actor,
                corrects_sequence: corrects,
                ..AttemptFacts::default()
            }),
            JournalExtension::Annotation,
        )
        .unwrap()
    }

    fn label_drafts(run_id: &RunId, label_after: Option<&str>) -> (JournalDraft, JournalDraft) {
        let label_after = label_after
            .map(|value| BoundedText::non_empty("run_label", value.to_string()).unwrap());
        let completed = JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.label",
            RequestId::parse("request-label").unwrap(),
            OutcomeClass::Completed,
            None,
            None,
            JournalExtension::LabelChanged {
                change: Some(LabelChangeFact {
                    label_before: None,
                    label_after: label_after.clone(),
                }),
            },
        )
        .unwrap();
        let rejected = JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.label",
            RequestId::parse("request-label").unwrap(),
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            None,
            JournalExtension::LabelChanged { change: None },
        )
        .unwrap();
        (completed, rejected)
    }

    fn terminate_drafts(run_id: &RunId) -> (JournalDraft, JournalDraft, JournalDraft) {
        let completed = JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.terminate",
            RequestId::parse("request-terminate").unwrap(),
            OutcomeClass::Completed,
            None,
            None,
            JournalExtension::RunTerminated,
        )
        .unwrap();
        let rejected = JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.terminate",
            RequestId::parse("request-terminate").unwrap(),
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            None,
            JournalExtension::RunTerminated,
        )
        .unwrap();
        let stale = JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.terminate",
            RequestId::parse("request-terminate").unwrap(),
            OutcomeClass::Error,
            Some(Reason::new(ReasonCode::StateStaleVersion, "stale lifecycle").unwrap()),
            None,
            JournalExtension::RunTerminated,
        )
        .unwrap();
        (completed, rejected, stale)
    }

    fn evidence_duplicate_rejection_draft(run_id: &RunId) -> JournalDraft {
        JournalDraft::new(
            run_id.clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            RequestId::parse("request-evidence-dup").unwrap(),
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::EvidenceInvalid, "duplicate evidence id").unwrap()),
            Some(evidence_attempt()),
            JournalExtension::EvidenceAdded { added: None },
        )
        .unwrap()
    }

    fn append_evidence_command(run: &Run, evidence: EvidenceRecord) -> AppendEvidenceCommand {
        let completed = evidence_draft(run.id(), &evidence);
        let rejected = evidence_duplicate_rejection_draft(run.id());
        evidence_add::command(run, evidence, completed, rejected).unwrap()
    }

    #[test]
    fn append_evidence_commits_on_terminal_lifecycle_without_version_bump() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "terminated");
        let run = run_from_reads(&writer, &RunId::parse("run-1").unwrap());
        let evidence = evidence_record("evidence-1");
        let command = append_evidence_command(&run, evidence);
        let status = writer.append_evidence(command).unwrap();
        assert!(!status.state_changed);
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn append_annotation_accepts_correction_link_to_prior_sequence() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        seed_journal(&conn, "run-1", 1);
        let run_id = RunId::parse("run-1").unwrap();
        let note = Note::new("clarification").unwrap();
        let corrects = Some(JournalSequence::try_from(1).unwrap());
        let command = run_annotate::command(
            &run_from_reads(&writer, &run_id),
            Some(note.clone()),
            None,
            corrects,
            annotation_draft(&run_id, Some(note), None, corrects),
        )
        .unwrap()
        .expect("annotation command");
        writer.append_annotation(command).unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("\"corrects_sequence\":1"));
    }

    #[test]
    fn append_annotation_rejects_missing_correction_target() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let note = Note::new("clarification").unwrap();
        let command = AppendAnnotationCommand::for_test(
            run_id,
            Some(note.clone()),
            None,
            Some(JournalSequence::try_from(1).unwrap()),
            annotation_draft(
                &RunId::parse("run-1").unwrap(),
                Some(note),
                None,
                Some(JournalSequence::try_from(1).unwrap()),
            ),
        );
        assert!(matches!(
            writer.append_annotation(command),
            Err(RunMutationError::InvalidCorrectionLink)
        ));
    }

    #[test]
    fn replace_label_on_terminal_run_appends_rejection_without_label_change() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "final");
        conn.execute("UPDATE runs SET label = 'kept' WHERE run_id = 'run-1'", [])
            .unwrap();
        let run_id = RunId::parse("run-1").unwrap();
        let (completed, rejected) = label_drafts(&run_id, Some("next"));
        let command = run_label::command(
            &run_from_reads(&writer, &run_id),
            Some("next".into()),
            completed,
            rejected,
        )
        .unwrap();
        let status = writer.replace_label(command).unwrap();
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);
        let label: String = conn
            .query_row("SELECT label FROM runs WHERE run_id = 'run-1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(label, "kept");
        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "rejected");
    }

    #[test]
    fn replace_label_on_active_run_updates_label_without_workflow_version_bump() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let (completed, rejected) = label_drafts(&run_id, Some("next"));
        let command = run_label::command(
            &run_from_reads(&writer, &run_id),
            Some("next".into()),
            completed,
            rejected,
        )
        .unwrap();
        let status = writer.replace_label(command).unwrap();
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);
        let (label, label_version): (String, i64) = conn
            .query_row(
                "SELECT label, label_version FROM runs WHERE run_id = 'run-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(label, "next");
        assert_eq!(label_version, 2);
    }

    #[test]
    fn terminate_on_active_run_bumps_lifecycle_version_only() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let (completed, rejected, stale) = terminate_drafts(&run_id);
        let command = run_terminate::command(&run, None, completed, rejected, stale).unwrap();
        conn.execute(
            "UPDATE runs SET label = 'authoritative', label_version = 2 WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();
        let status = writer.terminate(command).unwrap();
        assert_eq!(status.outcome, OutcomeClass::Completed);
        assert_eq!(status.run.lifecycle, Lifecycle::Terminated);
        assert_eq!(status.run.current_state.as_str(), "draft");
        assert_eq!(status.run.label.as_deref(), Some("authoritative"));
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        assert_eq!(status.commit.lifecycle_version.value(), 2);
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "terminated");
    }

    #[test]
    fn terminate_returns_authoritative_final_snapshot_after_concurrent_lifecycle_change() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let (completed, rejected, stale) = terminate_drafts(&run_id);
        let command = run_terminate::command(&run, None, completed, rejected, stale).unwrap();
        conn.execute(
            "UPDATE runs SET lifecycle = 'final', lifecycle_version = 2 WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();

        let status = writer.terminate(command).unwrap();

        assert_eq!(status.outcome, OutcomeClass::Rejected);
        assert_eq!(status.run.lifecycle, Lifecycle::Final);
        assert_eq!(status.run.current_state.as_str(), "draft");
        assert_eq!(status.commit.lifecycle_version.value(), 2);
    }

    #[test]
    fn terminate_on_terminal_run_appends_rejection_without_mutation() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "terminated");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let (completed, rejected, stale) = terminate_drafts(&run_id);
        let command = run_terminate::command(&run, None, completed, rejected, stale).unwrap();
        let status = writer.terminate(command).unwrap();
        assert_eq!(status.outcome, OutcomeClass::Rejected);
        assert_eq!(status.run.lifecycle, Lifecycle::Terminated);
        assert_eq!(status.commit.lifecycle_version.value(), 1);
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "terminated");
        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "rejected");
    }

    #[test]
    fn terminate_stale_lifecycle_appends_error_without_mutation() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let (completed, rejected, stale) = terminate_drafts(&run_id);
        let command = run_terminate::command(&run, None, completed, rejected, stale).unwrap();
        conn.execute(
            "UPDATE runs SET lifecycle_version = 2 WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();
        let status = writer.terminate(command).unwrap();
        assert_eq!(status.outcome, OutcomeClass::Error);
        assert_eq!(status.run.lifecycle, Lifecycle::Active);
        assert_eq!(status.commit.lifecycle_version.value(), 2);
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "active");
        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "error");
    }

    fn run_from_reads(
        writer: &SqliteRunMutations,
        run_id: &RunId,
    ) -> loop_engine_core::model::run::Run {
        SqliteRunReads::new(writer.path()).get(run_id).unwrap()
    }

    fn install_abort_trigger(conn: &Connection, sql: &str) {
        conn.execute(sql, []).unwrap();
    }

    fn assert_append_evidence_rolled_back(conn: &Connection, case: &str) {
        let evidence_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let journal_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let sequence: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence_count, 0, "{case}");
        assert_eq!(journal_count, 0, "{case}");
        assert_eq!(sequence, 2, "{case}");
    }

    #[test]
    fn append_evidence_abort_rolls_back_without_partial_writes() {
        let trigger_cases = [
            (
                "before_evidence_insert",
                "CREATE TRIGGER abort_before_evidence BEFORE INSERT ON evidence
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_journal_insert",
                "CREATE TRIGGER abort_before_journal BEFORE INSERT ON journal_entries
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_sequence_finalize",
                "CREATE TRIGGER abort_before_sequence BEFORE UPDATE ON run_journal_sequences
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
        ];
        for (case, trigger_sql) in trigger_cases {
            let (_dir, writer) = test_mutations();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run(&conn, "run-1", "reg-1", "active");
            install_abort_trigger(&conn, trigger_sql);
            let run = run_from_reads(&writer, &RunId::parse("run-1").unwrap());
            let evidence = evidence_record("evidence-1");
            let command = append_evidence_command(&run, evidence);
            assert!(writer.append_evidence(command).is_err(), "{case}");
            assert_append_evidence_rolled_back(&conn, case);
        }
    }

    #[test]
    fn append_evidence_pre_resolved_rejection_appends_journal_without_insert() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let command =
            evidence_add::rejected_command(&run, evidence_duplicate_rejection_draft(&run_id))
                .unwrap();
        let status = writer.append_evidence(command).unwrap();
        assert!(!status.state_changed);
        let evidence_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence_count, 0);
        assert_eq!(outcome, "rejected");
    }

    #[test]
    fn append_evidence_duplicate_id_appends_rejection_journal_without_partial_write() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let evidence = evidence_record("evidence-1");
        writer
            .append_evidence(append_evidence_command(&run, evidence.clone()))
            .unwrap();
        let status = writer
            .append_evidence(append_evidence_command(&run, evidence.clone()))
            .unwrap();
        assert!(!status.state_changed);
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);
        let evidence_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let (outcome, reason): (String, Option<String>) = conn
            .query_row(
                "SELECT outcome, json_extract(encoded_payload_json, '$.reason.code')
                 FROM journal_entries WHERE run_id = 'run-1' AND sequence = 3",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(evidence_count, 1);
        assert_eq!(outcome, "rejected");
        assert_eq!(reason.as_deref(), Some("evidence.invalid"));
    }

    #[test]
    fn append_evidence_duplicate_then_fresh_id_is_visible_in_history() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let run = run_from_reads(&writer, &run_id);
        let first = evidence_record("evidence-1");
        writer
            .append_evidence(append_evidence_command(&run, first.clone()))
            .unwrap();
        writer
            .append_evidence(append_evidence_command(&run, first))
            .unwrap();
        writer
            .append_evidence(append_evidence_command(&run, evidence_record("evidence-2")))
            .unwrap();
        let journal_rows: Vec<(i64, String)> = conn
            .prepare(
                "SELECT sequence, outcome FROM journal_entries
                 WHERE run_id = 'run-1' ORDER BY sequence",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let evidence_ids: Vec<String> = conn
            .prepare("SELECT evidence_id FROM evidence WHERE run_id = 'run-1' ORDER BY evidence_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            journal_rows,
            vec![
                (2, "completed".to_owned()),
                (3, "rejected".to_owned()),
                (4, "completed".to_owned()),
            ]
        );
        assert_eq!(evidence_ids, vec!["evidence-1", "evidence-2"]);
    }

    #[test]
    fn append_annotation_rejects_self_and_future_correction_links() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        seed_journal(&conn, "run-1", 1);
        let run_id = RunId::parse("run-1").unwrap();
        let note = Note::new("clarification").unwrap();
        let self_link = AppendAnnotationCommand::for_test(
            run_id.clone(),
            Some(note.clone()),
            None,
            Some(JournalSequence::try_from(2).unwrap()),
            annotation_draft(
                &run_id,
                Some(note.clone()),
                None,
                Some(JournalSequence::try_from(2).unwrap()),
            ),
        );
        assert!(matches!(
            writer.append_annotation(self_link),
            Err(RunMutationError::InvalidCorrectionLink)
        ));
        seed_journal(&conn, "run-1", 2);
        let future_link = AppendAnnotationCommand::for_test(
            run_id.clone(),
            Some(note),
            None,
            Some(JournalSequence::try_from(4).unwrap()),
            annotation_draft(
                &run_id,
                Some(Note::new("future correction").unwrap()),
                None,
                Some(JournalSequence::try_from(4).unwrap()),
            ),
        );
        assert!(matches!(
            writer.append_annotation(future_link),
            Err(RunMutationError::InvalidCorrectionLink)
        ));
    }

    #[test]
    fn append_annotation_on_final_run_preserves_workflow_versions() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "final");
        let run_id = RunId::parse("run-1").unwrap();
        let note = Note::new("note").unwrap();
        let command = run_annotate::command(
            &run_from_reads(&writer, &run_id),
            Some(note.clone()),
            None,
            None,
            annotation_draft(&run_id, Some(note), None, None),
        )
        .unwrap()
        .expect("annotation command");
        let status = writer.append_annotation(command).unwrap();
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);
    }

    fn assert_append_annotation_rolled_back(conn: &Connection, case: &str) {
        let journal_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let sequence: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(journal_count, 0, "{case}");
        assert_eq!(sequence, 2, "{case}");
    }

    #[test]
    fn append_annotation_abort_rolls_back_without_partial_writes() {
        let trigger_cases = [
            (
                "before_journal_insert",
                "CREATE TRIGGER abort_before_journal BEFORE INSERT ON journal_entries
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_sequence_finalize",
                "CREATE TRIGGER abort_before_sequence BEFORE UPDATE ON run_journal_sequences
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
        ];
        for (case, trigger_sql) in trigger_cases {
            let (_dir, writer) = test_mutations();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run(&conn, "run-1", "reg-1", "active");
            install_abort_trigger(&conn, trigger_sql);
            let run_id = RunId::parse("run-1").unwrap();
            let note = Note::new("note").unwrap();
            let command = run_annotate::command(
                &run_from_reads(&writer, &run_id),
                Some(note.clone()),
                None,
                None,
                annotation_draft(&run_id, Some(note), None, None),
            )
            .unwrap()
            .expect("annotation command");
            assert!(writer.append_annotation(command).is_err(), "{case}");
            assert_append_annotation_rolled_back(&conn, case);
        }
    }

    #[test]
    fn append_annotation_with_actor_metadata_commits_journal() {
        let (_dir, writer) = test_mutations();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1", "active");
        let run_id = RunId::parse("run-1").unwrap();
        let actor = ActorMetadata::new(CoreValue::Object(BTreeMap::from([(
            "kind".into(),
            CoreValue::String("agent".into()),
        )])))
        .unwrap();
        let command = run_annotate::command(
            &run_from_reads(&writer, &run_id),
            None,
            Some(actor.clone()),
            None,
            annotation_draft(&run_id, None, Some(actor), None),
        )
        .unwrap()
        .expect("annotation command");
        writer.append_annotation(command).unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("\"actor\""));
    }

    fn assert_replace_label_rolled_back(conn: &Connection, case: &str) {
        let (label, label_version): (Option<String>, i64) = conn
            .query_row(
                "SELECT label, label_version FROM runs WHERE run_id = 'run-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let journal_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let sequence: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(label.is_none(), "{case}");
        assert_eq!(label_version, 1, "{case}");
        assert_eq!(journal_count, 0, "{case}");
        assert_eq!(sequence, 2, "{case}");
    }

    #[test]
    fn replace_label_abort_rolls_back_without_partial_writes() {
        let trigger_cases = [
            (
                "before_label_update",
                "CREATE TRIGGER abort_before_label BEFORE UPDATE OF label, label_version ON runs
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_journal_insert",
                "CREATE TRIGGER abort_before_journal BEFORE INSERT ON journal_entries
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_sequence_finalize",
                "CREATE TRIGGER abort_before_sequence BEFORE UPDATE ON run_journal_sequences
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
        ];
        for (case, trigger_sql) in trigger_cases {
            let (_dir, writer) = test_mutations();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run(&conn, "run-1", "reg-1", "active");
            install_abort_trigger(&conn, trigger_sql);
            let run_id = RunId::parse("run-1").unwrap();
            let (completed, rejected) = label_drafts(&run_id, Some("next"));
            let command = run_label::command(
                &run_from_reads(&writer, &run_id),
                Some("next".into()),
                completed,
                rejected,
            )
            .unwrap();
            assert!(writer.replace_label(command).is_err(), "{case}");
            assert_replace_label_rolled_back(&conn, case);
        }
    }

    fn assert_terminate_rolled_back(conn: &Connection, case: &str) {
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let lifecycle_version: i64 = conn
            .query_row(
                "SELECT lifecycle_version FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let journal_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let sequence: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "active", "{case}");
        assert_eq!(lifecycle_version, 1, "{case}");
        assert_eq!(journal_count, 0, "{case}");
        assert_eq!(sequence, 2, "{case}");
    }

    #[test]
    fn terminate_abort_rolls_back_without_partial_writes() {
        let trigger_cases = [
            (
                "before_lifecycle_update",
                "CREATE TRIGGER abort_before_lifecycle BEFORE UPDATE OF lifecycle, lifecycle_version ON runs
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_journal_insert",
                "CREATE TRIGGER abort_before_journal BEFORE INSERT ON journal_entries
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
            (
                "before_sequence_finalize",
                "CREATE TRIGGER abort_before_sequence BEFORE UPDATE ON run_journal_sequences
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            ),
        ];
        for (case, trigger_sql) in trigger_cases {
            let (_dir, writer) = test_mutations();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run(&conn, "run-1", "reg-1", "active");
            install_abort_trigger(&conn, trigger_sql);
            let run_id = RunId::parse("run-1").unwrap();
            let run = run_from_reads(&writer, &run_id);
            let (completed, rejected, stale) = terminate_drafts(&run_id);
            let command = run_terminate::command(&run, None, completed, rejected, stale).unwrap();
            assert!(writer.terminate(command).is_err(), "{case}");
            assert_terminate_rolled_back(&conn, case);
        }
    }
}
