//! Atomic SQLite event-attempt transactions (T112–T113).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::event_attempt_writer::EventAttemptWriter;
use loop_engine_core::capabilities::persistence_commands::{
    CommitEventAttemptCommand, CommitStatus, EventAttemptParts, EventCommitBranch,
    EventCommitStatus,
};
use loop_engine_core::model::attempt::{
    AttemptFacts, GateVerdictFacts, GateVerdictResult, JournalExtension, ProviderFact,
    ProviderRole, TransitionFact,
};
use loop_engine_core::model::bounded::{
    ACTOR_METADATA_ENCODED_BYTES, BoundError, DIAGNOSTIC_ENCODED_BYTES,
    DIAGNOSTICS_PER_RESULT_COUNT, JOURNAL_ENTRY_ENCODED_BYTES,
    JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES, JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES,
    JOURNAL_PROVIDER_FACTS_ENCODED_BYTES, NOTE_TEXT_UTF8_BYTES, Value as CoreValue,
};
use loop_engine_core::model::decision::{
    DecisionError, TransitionDecision, resolve_gate_free, resolve_gated,
};
use loop_engine_core::model::gate::{GateEvaluation, GateVerdict};
use loop_engine_core::model::run::Run;
use loop_engine_core::operations::run_request::{EvidenceConflictKind, evidence_conflict_journal};

const fn journal_diagnostic_aggregate_bytes() -> usize {
    match DIAGNOSTIC_ENCODED_BYTES.checked_mul(DIAGNOSTICS_PER_RESULT_COUNT) {
        Some(bytes) => bytes,
        None => panic!("journal diagnostic aggregate exceeds usize"),
    }
}

const JOURNAL_DIAGNOSTIC_AGGREGATE_BYTES: usize = journal_diagnostic_aggregate_bytes();
use loop_engine_core::model::evidence::{EvidenceAssociation, EvidenceRecord};
use loop_engine_core::model::ids::{EventId, EvidenceId, GateId, RunId, StateId};
use loop_engine_core::model::journal::{
    JournalDraft, JournalEncodedSizes, JournalEntry, JournalEntryKind, JournalError, StateFact,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::reason::ReasonCode;
use loop_engine_core::model::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};
use rusqlite::{Connection, Error as SqliteError, params};
use serde_json::{Value, json};
use thiserror::Error;

use super::error::{CommitOutcomeError, PersistenceError};
use super::mapping::{self, MappingError};
use super::records::{JOURNAL_PAYLOAD_SCHEMA_VERSION, RunRecord};
use super::sqlite::commit::{
    EvidenceAssociationExpectation, JournalBundleExpectation, JournalRowExpectation,
    RunAuthoritativeExpectation, finish_committed_transaction,
};
use super::sqlite::connect_with_pragmas;
use super::traced::{
    OptionalTraceSink, WriteExecution, WriteTraceSession, committed_or_unconfirmed,
    event_attempt_error_semantic, event_commit_semantic, finish_traced_event_write,
    rollback_open_transaction,
};

/// SQLite-backed atomic writer for gated and gate-free `run.request` commits.
#[derive(Clone)]
pub struct SqliteEventAttemptWriter {
    path: PathBuf,
    trace: OptionalTraceSink,
}

impl std::fmt::Debug for SqliteEventAttemptWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteEventAttemptWriter")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub enum EventAttemptPersistenceError {
    #[error("run not found")]
    NotFound,
    #[error("evidence is invalid")]
    EvidenceInvalid,
    #[error("resource exhausted")]
    ResourceExhausted,
    #[error("persistence failed")]
    PersistenceFailed,
    #[error("corrupt persistence row: {detail}")]
    Corrupt { detail: String },
    #[error("attempt validation failed: {detail}")]
    Validation { detail: String },
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl CommitOutcomeError for EventAttemptPersistenceError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

impl SqliteEventAttemptWriter {
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

    fn commit_parts(
        &self,
        parts: EventAttemptParts,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<EventCommitStatus, EventAttemptPersistenceError> {
        if let Err(error) = validate_parts(&parts) {
            return WriteExecution::no_transaction(error);
        }
        let conn =
            match connect_with_pragmas(&self.path).map_err(EventAttemptPersistenceError::from) {
                Ok(conn) => conn,
                Err(error) => return WriteExecution::no_transaction(error),
            };
        if let Err(error) = validate_referential_consistency(&conn, &parts) {
            return WriteExecution::no_transaction(error);
        }
        if let Err(error) = conn
            .execute("BEGIN IMMEDIATE", [])
            .map_err(map_sqlite_write_error)
        {
            return WriteExecution::no_transaction(error);
        }
        let expected_workflow_version = parts.expected_workflow_version.value();
        let expected_lifecycle_version = parts.expected_lifecycle_version.value();
        let run_id = parts.run_id.as_str();

        let outcome =
            (|| -> Result<(EventCommitStatus, JournalBundleExpectation), EventAttemptPersistenceError> {
            if let Some(session) = trace {
                session.version_check_run_cas(
                    run_id,
                    Some(expected_workflow_version),
                    Some(expected_lifecycle_version),
                );
            }
            let snapshot = read_run_snapshot(&conn, &parts.run_id)?;

            let versions_match = snapshot.workflow_state_version
                == parts.expected_workflow_version.value()
                && snapshot.lifecycle_version == parts.expected_lifecycle_version.value();

            if versions_match {
                let authoritative_run =
                    reconstruct_authoritative_run(&conn, &parts.run_id, &snapshot)?;
                let transition = parts
                    .journal_entry
                    .attempt()
                    .and_then(|attempt| attempt.transition.as_ref())
                    .ok_or_else(|| EventAttemptPersistenceError::Validation {
                        detail: "journal entry missing transition facts".into(),
                    })?;
                match resolve_gate_free(&authoritative_run, &transition.event) {
                    Err(DecisionError::GatesRequired) => validate_gated_branch(
                        &parts,
                        &snapshot,
                        &authoritative_run,
                        &parts.journal_entry,
                    )?,
                    _ => validate_gate_free_branch(
                        &parts,
                        &snapshot,
                        &authoritative_run,
                        &parts.journal_entry,
                    )?,
                }
            } else {
                validate_stale_branch(&parts.stale_journal_entry)?;
            }

            let conflict = if versions_match {
                existing_evidence_conflict(&conn, &parts)?
            } else {
                None
            };
            let conflict_draft = conflict
                .map(|kind| evidence_conflict_journal(&parts.journal_entry, kind))
                .transpose()
                .map_err(|error| EventAttemptPersistenceError::Validation {
                    detail: format!("invalid evidence-conflict journal: {error}"),
                })?;
            let (branch, draft) = match (versions_match, conflict, conflict_draft.as_ref()) {
                (false, _, _) => (EventCommitBranch::StaleVersions, &parts.stale_journal_entry),
                (true, None, _) => (EventCommitBranch::ExpectedVersions, &parts.journal_entry),
                (true, Some(EvidenceConflictKind::Inline), Some(draft)) => {
                    (EventCommitBranch::InlineEvidenceConflict, draft)
                }
                (true, Some(EvidenceConflictKind::Provider), Some(draft)) => {
                    (EventCommitBranch::ProviderEvidenceConflict, draft)
                }
                (true, Some(_), None) => {
                    return Err(EventAttemptPersistenceError::Validation {
                        detail: "missing evidence-conflict journal".into(),
                    });
                }
            };

            let (state_before, state_after) =
                compute_state_facts(&parts, &snapshot, branch, draft)?;
            let sequence = allocate_sequence(&conn, &parts.run_id)?;
            let (_entry, payload_json, outcome_class) =
                materialize_journal(draft, sequence, state_before.clone(), state_after.clone())?;

            if branch == EventCommitBranch::ExpectedVersions {
                insert_evidence_records(&conn, &parts)?;
                if should_apply_state_mutation(draft, &parts) {
                    apply_run_mutation(&conn, &parts, &state_after)?;
                }
            }

            insert_journal_row(&conn, &parts.run_id, sequence, outcome_class, &payload_json)?;

            if branch == EventCommitBranch::ExpectedVersions && !parts.associations.is_empty() {
                insert_associations(&conn, &parts.run_id, sequence, &parts.associations)?;
            }
            let status = build_commit_status(
                branch,
                draft,
                &state_before,
                &state_after,
            );
            let expectation = event_attempt_bundle_expectation(
                &parts,
                &snapshot,
                branch,
                draft,
                sequence,
                &payload_json,
                outcome_class,
                &state_after,
            );
            Ok((status, expectation))
        })();

        match outcome {
            Ok((status, expectation)) => committed_or_unconfirmed(finish_committed_transaction(
                &self.path,
                conn,
                status,
                |read| expectation.verify(read),
                map_sqlite_read_error,
                || EventAttemptPersistenceError::CommitOutcomeUnverified,
                || EventAttemptPersistenceError::CommitIntegrityFailure,
                EventAttemptPersistenceError::from,
            )),
            Err(error) => rollback_open_transaction(&conn, error),
        }
    }
}

impl EventAttemptWriter for SqliteEventAttemptWriter {
    type Error = EventAttemptPersistenceError;

    fn commit_event_attempt(
        &self,
        command: CommitEventAttemptCommand,
    ) -> Result<EventCommitStatus, Self::Error> {
        let parts = command.into_parts();
        let expected_draft = parts.journal_entry.clone();
        let stale_draft = parts.stale_journal_entry.clone();
        finish_traced_event_write(
            &self.trace,
            "run.request",
            |trace| {
                self.commit_parts(parts, trace).map_ok(|status| {
                    let semantic = event_commit_semantic(&status, &expected_draft, &stale_draft);
                    (status, semantic)
                })
            },
            event_attempt_error_semantic,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunSnapshot {
    current_state: StateId,
    lifecycle: Lifecycle,
    workflow_state_version: u64,
    lifecycle_version: u64,
    label: Option<String>,
    label_version: u64,
}

fn validate_parts(parts: &EventAttemptParts) -> Result<(), EventAttemptPersistenceError> {
    if parts.run_id.as_str() != parts.journal_entry.run_id().as_str()
        || parts.run_id.as_str() != parts.stale_journal_entry.run_id().as_str()
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "journal run_id mismatch".into(),
        });
    }
    if parts.expected_workflow_version.value() == 0 || parts.expected_lifecycle_version.value() == 0
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "expected version must be positive".into(),
        });
    }
    if parts.journal_entry.kind() != JournalEntryKind::TransitionAttempt
        || parts.stale_journal_entry.kind() != JournalEntryKind::TransitionAttempt
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "event attempt requires transition.attempt journal kind".into(),
        });
    }
    Ok(())
}

fn validate_referential_consistency(
    conn: &Connection,
    parts: &EventAttemptParts,
) -> Result<(), EventAttemptPersistenceError> {
    let attempt =
        parts
            .journal_entry
            .attempt()
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "journal entry missing attempt facts".into(),
            })?;
    let associations = attempt.evidence_associations.as_ref().ok_or_else(|| {
        EventAttemptPersistenceError::Validation {
            detail: "journal entry missing evidence associations".into(),
        }
    })?;
    let recorded =
        attempt
            .evidence_recorded
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "journal entry missing evidence_recorded".into(),
            })?;
    if associations.recorded_status() != recorded {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "evidence_recorded does not match associations".into(),
        });
    }

    let inline_ids = parts
        .inline_evidence
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    if inline_ids.len() != parts.inline_evidence.len() {
        return Err(EventAttemptPersistenceError::EvidenceInvalid);
    }
    let provider_ids = parts
        .provider_evidence
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    if provider_ids.len() != parts.provider_evidence.len() {
        return Err(EventAttemptPersistenceError::EvidenceInvalid);
    }

    let expected_count = inline_ids
        .len()
        .saturating_add(provider_ids.len())
        .saturating_add(associations.selected_ids.len());
    let expected_association_ids = inline_ids
        .iter()
        .chain(provider_ids.iter())
        .map(|id| (*id).clone())
        .chain(associations.selected_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let actual_association_ids = parts
        .associations
        .iter()
        .map(|association| association.evidence_id().clone())
        .collect::<BTreeSet<_>>();
    if expected_association_ids.len() != expected_count
        || actual_association_ids.len() != parts.associations.len()
        || parts.associations.len() != expected_count
        || expected_association_ids != actual_association_ids
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "association evidence_id set or category ownership mismatch".into(),
        });
    }

    if !associations.inline.iter().eq(parts.inline_evidence.iter()) {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "inline evidence mismatch between journal and command".into(),
        });
    }
    if associations
        .provider_recorded_ids
        .iter()
        .collect::<BTreeSet<_>>()
        != provider_ids
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "provider evidence mismatch between journal and command".into(),
        });
    }

    for selected in &associations.selected_ids {
        if !evidence_exists(conn, &parts.run_id, selected)? {
            return Err(EventAttemptPersistenceError::EvidenceInvalid);
        }
    }
    Ok(())
}

fn load_run_record(
    conn: &Connection,
    run_id: &RunId,
) -> Result<RunRecord, EventAttemptPersistenceError> {
    conn.query_row(
        "SELECT run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label,
                graph_revision, canonical_graph_version, graph_canonical_projection_json,
                inputs_json, created_at
         FROM runs WHERE run_id = ?1",
        params![run_id.as_str()],
        |row| {
            Ok(RunRecord {
                run_id: row.get(0)?,
                registration_id: row.get(1)?,
                config_revision_at_create: row.get::<_, i64>(2)? as u64,
                current_state: row.get(3)?,
                lifecycle: row.get(4)?,
                workflow_state_version: row.get::<_, i64>(5)? as u64,
                lifecycle_version: row.get::<_, i64>(6)? as u64,
                label_version: row.get::<_, i64>(7)? as u64,
                label: row.get(8)?,
                graph_revision: row.get(9)?,
                canonical_graph_version: row.get::<_, i64>(10)? as u64,
                graph_canonical_projection_json: row.get(11)?,
                inputs_json: row.get(12)?,
                created_at: row.get(13)?,
            })
        },
    )
    .map_err(map_run_lookup_error)
}

fn reconstruct_authoritative_run(
    conn: &Connection,
    run_id: &RunId,
    snapshot: &RunSnapshot,
) -> Result<Run, EventAttemptPersistenceError> {
    let record = load_run_record(conn, run_id)?;
    if record.current_state != snapshot.current_state.as_str() {
        return Err(EventAttemptPersistenceError::Corrupt {
            detail: "run row current_state diverged within transaction".into(),
        });
    }
    let lifecycle = parse_lifecycle(&record.lifecycle)
        .map_err(|detail| EventAttemptPersistenceError::Corrupt { detail })?;
    if lifecycle != snapshot.lifecycle
        || record.workflow_state_version != snapshot.workflow_state_version
        || record.lifecycle_version != snapshot.lifecycle_version
    {
        return Err(EventAttemptPersistenceError::Corrupt {
            detail: "run row versions diverged within transaction".into(),
        });
    }
    mapping::run_from_record(&record).map_err(|error| EventAttemptPersistenceError::Corrupt {
        detail: error.to_string(),
    })
}

fn validate_gate_free_branch(
    parts: &EventAttemptParts,
    snapshot: &RunSnapshot,
    run: &Run,
    draft: &JournalDraft,
) -> Result<(), EventAttemptPersistenceError> {
    if snapshot.current_state != parts.source_state {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "source_state does not match authoritative run row".into(),
        });
    }
    let attempt = draft
        .attempt()
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "journal entry missing attempt facts".into(),
        })?;
    let transition =
        attempt
            .transition
            .as_ref()
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "gate-free attempt requires transition facts".into(),
            })?;
    if transition.source != parts.source_state {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "transition source disagrees with command source_state".into(),
        });
    }
    validate_association_context(parts, &transition.event, &[])?;

    match resolve_gate_free(run, &transition.event) {
        Ok(decision) => match draft.outcome() {
            OutcomeClass::Completed => {
                validate_completed_gate_free(parts, transition, &decision, snapshot)
            }
            OutcomeClass::Rejected | OutcomeClass::Error => {
                validate_pre_resolution_gate_free(parts, draft, transition, &decision, attempt)
            }
        },
        Err(DecisionError::GatesRequired) => Err(EventAttemptPersistenceError::Validation {
            detail: "gate-free attempt draft disagrees with gated transition".into(),
        }),
        Err(error) => match draft.outcome() {
            OutcomeClass::Completed => Err(EventAttemptPersistenceError::Validation {
                detail: "completed attempt disagrees with authoritative event resolution".into(),
            }),
            OutcomeClass::Rejected => validate_rejected_gate_free(parts, draft, transition, &error),
            OutcomeClass::Error => Err(EventAttemptPersistenceError::Validation {
                detail: "gate-free pre-resolution rejection must use rejected outcome".into(),
            }),
        },
    }
}

fn validate_completed_gate_free(
    parts: &EventAttemptParts,
    transition: &TransitionFact,
    decision: &TransitionDecision,
    snapshot: &RunSnapshot,
) -> Result<(), EventAttemptPersistenceError> {
    let target =
        parts
            .target_state
            .as_ref()
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "completed attempt requires target_state".into(),
            })?;
    let target_lifecycle =
        parts
            .target_lifecycle
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "completed attempt requires target_lifecycle".into(),
            })?;
    if transition.event != *decision.event()
        || transition.source != *decision.source()
        || transition.target.as_ref() != Some(decision.target())
        || !transition.applied
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "transition facts inconsistent with authoritative resolution".into(),
        });
    }
    if target != decision.target()
        || target_lifecycle != decision.lifecycle()
        || parts.source_state != *decision.source()
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "command target or lifecycle disagrees with authoritative resolution".into(),
        });
    }
    if target_lifecycle != snapshot.lifecycle && snapshot.lifecycle.is_terminal() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "cannot transition from terminal lifecycle".into(),
        });
    }
    Ok(())
}

fn is_pre_resolution_gate_free_reason(code: ReasonCode) -> bool {
    matches!(
        code,
        ReasonCode::EvidenceInvalid
            | ReasonCode::EvidenceSelectionInvalid
            | ReasonCode::PersistenceFailed
            | ReasonCode::ResourceExhausted
    )
}

fn validate_pre_resolution_gate_free(
    parts: &EventAttemptParts,
    draft: &JournalDraft,
    transition: &TransitionFact,
    decision: &TransitionDecision,
    attempt: &AttemptFacts,
) -> Result<(), EventAttemptPersistenceError> {
    if parts.target_lifecycle.is_some() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution attempt must not carry target_lifecycle".into(),
        });
    }
    if transition.applied {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution attempt cannot apply transition".into(),
        });
    }
    if transition.event != *decision.event() || transition.source != *decision.source() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "transition facts inconsistent with authoritative resolution".into(),
        });
    }
    if transition
        .target
        .as_ref()
        .is_some_and(|target| target != decision.target())
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution transition target must be absent or authoritative".into(),
        });
    }
    if parts.target_state != transition.target {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "command target_state disagrees with transition facts".into(),
        });
    }
    if parts
        .target_state
        .as_ref()
        .is_some_and(|target| target != decision.target())
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution target must be absent or authoritative".into(),
        });
    }
    if !attempt.provider_observations.is_empty() || attempt.gate_verdict_facts.is_some() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution attempt must not carry provider observations".into(),
        });
    }
    let reason = draft
        .reason()
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "pre-resolution attempt requires reason".into(),
        })?;
    if !is_pre_resolution_gate_free_reason(reason.code()) {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution reason code is not permitted for applicable gate-free event"
                .into(),
        });
    }
    if draft.outcome() != reason.code().outcome_class() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-resolution outcome disagrees with reason code".into(),
        });
    }
    Ok(())
}

fn validate_rejected_gate_free(
    parts: &EventAttemptParts,
    draft: &JournalDraft,
    transition: &TransitionFact,
    error: &DecisionError,
) -> Result<(), EventAttemptPersistenceError> {
    if parts.target_lifecycle.is_some() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "rejected attempt must not carry target_lifecycle".into(),
        });
    }
    if transition.applied {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "rejected attempt cannot apply transition".into(),
        });
    }
    if transition.source != parts.source_state {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "transition source disagrees with command source_state".into(),
        });
    }
    let resolved_target = rejection_target(error, transition.event.clone());
    if transition.target != resolved_target || parts.target_state != resolved_target {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "rejection transition target disagrees with authoritative resolution".into(),
        });
    }
    if draft.reason().map(|reason| reason.code()) != Some(rejection_reason_code(error)) {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "rejection draft reason disagrees with authoritative resolution".into(),
        });
    }
    Ok(())
}

fn rejection_target(_error: &DecisionError, _event: EventId) -> Option<StateId> {
    None
}

fn rejection_reason_code(error: &DecisionError) -> ReasonCode {
    match error {
        DecisionError::Terminal => ReasonCode::RunLifecycleTerminal,
        DecisionError::UnknownEvent(_) | DecisionError::AmbiguousEvent(_) => {
            ReasonCode::EventUnknown
        }
        DecisionError::GatesRequired => ReasonCode::GateFailed,
        _ => ReasonCode::GateFailed,
    }
}

fn validate_gated_branch(
    parts: &EventAttemptParts,
    snapshot: &RunSnapshot,
    run: &Run,
    draft: &JournalDraft,
) -> Result<(), EventAttemptPersistenceError> {
    if snapshot.current_state != parts.source_state {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "source_state does not match authoritative run row".into(),
        });
    }
    let attempt = draft
        .attempt()
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "journal entry missing attempt facts".into(),
        })?;
    let transition =
        attempt
            .transition
            .as_ref()
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "gated attempt requires transition facts".into(),
            })?;
    let stored_transition = run
        .graph()
        .transitions()
        .iter()
        .find(|candidate| {
            candidate.source() == run.current_state() && candidate.event() == &transition.event
        })
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "gated attempt event is not authoritative for current state".into(),
        })?;
    if transition.source != *run.current_state() || parts.source_state != *run.current_state() {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "gated transition source disagrees with stored graph".into(),
        });
    }
    validate_association_context(parts, &transition.event, stored_transition.required_gates())?;

    let Some(facts) = &attempt.gate_verdict_facts else {
        return validate_pre_provider_gated(parts, draft, attempt, transition, stored_transition);
    };
    if facts.event != transition.event
        || facts.gate_ids != stored_transition.required_gates()
        || transition.target.as_ref() != Some(stored_transition.target())
        || parts.target_state.as_ref() != Some(stored_transition.target())
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "gate verdict event, gates, or transition target disagree with stored graph"
                .into(),
        });
    }
    let evaluation = gate_evaluation_from_facts(parts, facts)?;
    let provider_outcome = match &facts.result {
        GateVerdictResult::Verdicts(_) => OutcomeClass::Completed,
        GateVerdictResult::Incompatibility(_) => OutcomeClass::Rejected,
        GateVerdictResult::EvaluationError(_) => OutcomeClass::Error,
    };
    if !matches!(attempt.provider_observations.as_slice(), [fact]
        if fact.role == ProviderRole::EvaluateGates && fact.outcome == provider_outcome)
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "gate verdict facts require one matching evaluate_gates provider observation"
                .into(),
        });
    }

    match resolve_gated(run, &transition.event, &evaluation) {
        Ok(decision) => {
            if draft.outcome() != OutcomeClass::Completed
                || draft.reason().is_some()
                || !transition.applied
                || parts.target_lifecycle != Some(decision.lifecycle())
                || parts.target_state.as_ref() != Some(decision.target())
            {
                return Err(EventAttemptPersistenceError::Validation {
                    detail: "completed gated attempt disagrees with authoritative decision".into(),
                });
            }
        }
        Err(DecisionError::GateFailed { .. }) => validate_gated_denial(
            parts,
            draft,
            transition,
            OutcomeClass::Rejected,
            ReasonCode::GateFailed,
        )?,
        Err(DecisionError::Incompatible { .. }) => validate_gated_denial(
            parts,
            draft,
            transition,
            OutcomeClass::Rejected,
            ReasonCode::CompatibilityUnsupported,
        )?,
        Err(DecisionError::EvaluationError { .. }) => validate_gated_denial(
            parts,
            draft,
            transition,
            OutcomeClass::Error,
            ReasonCode::ProviderEvaluationError,
        )?,
        Err(error) => {
            return Err(EventAttemptPersistenceError::Validation {
                detail: format!("gate verdict facts do not resolve authoritatively: {error}"),
            });
        }
    }
    Ok(())
}

fn validate_association_context(
    parts: &EventAttemptParts,
    event: &EventId,
    required_gates: &[GateId],
) -> Result<(), EventAttemptPersistenceError> {
    let provider_ids = parts
        .provider_evidence
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    if parts.associations.iter().all(|association| {
        association.event_id() == Some(event)
            && association
                .gate_id()
                .is_none_or(|gate| required_gates.contains(gate))
            && (!provider_ids.contains(association.evidence_id())
                || association
                    .gate_id()
                    .is_some_and(|gate| required_gates.contains(gate)))
    }) {
        Ok(())
    } else {
        Err(EventAttemptPersistenceError::Validation {
            detail: "evidence association event or gate context is not authoritative".into(),
        })
    }
}

fn gate_evaluation_from_facts(
    parts: &EventAttemptParts,
    facts: &GateVerdictFacts,
) -> Result<GateEvaluation, EventAttemptPersistenceError> {
    match &facts.result {
        GateVerdictResult::Verdicts(verdicts) => Ok(GateEvaluation::verdicts(
            verdicts
                .iter()
                .map(|verdict| {
                    let evidence = parts
                        .provider_evidence
                        .iter()
                        .filter(|record| {
                            parts.associations.iter().any(|association| {
                                association.evidence_id() == record.id()
                                    && association.gate_id() == Some(&verdict.gate_id)
                            })
                        })
                        .cloned()
                        .collect();
                    GateVerdict::new(verdict.gate_id.clone(), verdict.passed, evidence)
                })
                .collect(),
        )),
        GateVerdictResult::Incompatibility(diagnostic) => {
            GateEvaluation::incompatible(vec![diagnostic.clone()]).map_err(Into::into)
        }
        GateVerdictResult::EvaluationError(diagnostics) => {
            GateEvaluation::evaluation_error(diagnostics.as_slice().to_vec()).map_err(Into::into)
        }
    }
}

fn validate_gated_denial(
    parts: &EventAttemptParts,
    draft: &JournalDraft,
    transition: &TransitionFact,
    expected_outcome: OutcomeClass,
    expected_reason: ReasonCode,
) -> Result<(), EventAttemptPersistenceError> {
    if draft.outcome() != expected_outcome
        || draft.reason().map(|reason| reason.code()) != Some(expected_reason)
        || transition.applied
        || parts.target_lifecycle.is_some()
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "gated denial outcome, reason, or mutation shape is not authoritative".into(),
        });
    }
    Ok(())
}

fn validate_pre_provider_gated(
    parts: &EventAttemptParts,
    draft: &JournalDraft,
    attempt: &AttemptFacts,
    transition: &TransitionFact,
    stored_transition: &loop_engine_core::model::transition::Transition,
) -> Result<(), EventAttemptPersistenceError> {
    let reason = draft
        .reason()
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "pre-provider gated attempt requires reason".into(),
        })?;
    let provider_facts_valid = match reason.code() {
        ReasonCode::ProviderProtocolMalformed => {
            matches!(attempt.provider_observations.as_slice(), [] | [_])
        }
        ReasonCode::ProviderEvidenceMalformed => {
            matches!(attempt.provider_observations.as_slice(), [fact]
                if fact.role == ProviderRole::EvaluateGates
                    && matches!(fact.outcome, OutcomeClass::Completed | OutcomeClass::Error))
        }
        _ => {
            matches!(attempt.provider_observations.as_slice(), [])
                || matches!(attempt.provider_observations.as_slice(), [fact]
                    if fact.role == ProviderRole::EvaluateGates
                        && fact.outcome == OutcomeClass::Error)
        }
    };
    if transition.applied
        || parts.target_lifecycle.is_some()
        || !parts.provider_evidence.is_empty()
        || !provider_facts_valid
        || transition.target != parts.target_state
        || transition
            .target
            .as_ref()
            .is_some_and(|target| target != stored_transition.target())
        || draft.outcome() != reason.code().outcome_class()
        || !is_pre_resolution_gated_reason(reason.code())
    {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "pre-provider gated attempt carries unsupported facts or reason".into(),
        });
    }
    Ok(())
}

fn is_pre_resolution_gated_reason(code: ReasonCode) -> bool {
    matches!(
        code,
        ReasonCode::EvidenceInvalid
            | ReasonCode::EvidenceSelectionInvalid
            | ReasonCode::CatalogRegistrationNotFound
            | ReasonCode::ProviderRegistrationMissing
            | ReasonCode::ProviderRegistrationStale
            | ReasonCode::ProviderTombstoned
            | ReasonCode::ProviderSpawnFailed
            | ReasonCode::ProviderExecutableNotFound
            | ReasonCode::ProviderProtocolUnsupportedMajor
            | ReasonCode::ProviderProtocolMalformed
            | ReasonCode::ProviderProtocolOversized
            | ReasonCode::ProviderProtocolInvalidUtf8
            | ReasonCode::ProviderTimeout
            | ReasonCode::ProviderCrash
            | ReasonCode::ProviderNonzeroExit
            | ReasonCode::ProviderSignal
            | ReasonCode::ProviderEvidenceMalformed
            | ReasonCode::PersistenceFailed
            | ReasonCode::ResourceExhausted
    )
}

fn stale_evidence_absent(attempt: &AttemptFacts) -> bool {
    match (&attempt.evidence_associations, attempt.evidence_recorded) {
        (None, None) => true,
        (Some(associations), Some(recorded)) => {
            associations.inline.is_empty()
                && associations.selected_ids.is_empty()
                && associations.provider_recorded_ids.is_empty()
                && !recorded.inline
                && !recorded.selected_associations
                && !recorded.provider
        }
        _ => false,
    }
}

fn validate_stale_branch(draft: &JournalDraft) -> Result<(), EventAttemptPersistenceError> {
    if draft.outcome() != OutcomeClass::Error {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "stale branch requires error outcome".into(),
        });
    }
    if draft.reason().map(|reason| reason.code()) != Some(ReasonCode::StateStaleVersion) {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "stale branch requires state.stale_version reason".into(),
        });
    }
    let attempt = draft
        .attempt()
        .ok_or_else(|| EventAttemptPersistenceError::Validation {
            detail: "stale branch requires attempt facts".into(),
        })?;
    if !stale_evidence_absent(attempt) {
        return Err(EventAttemptPersistenceError::Validation {
            detail: "stale branch must not claim evidence associations or recorded categories"
                .into(),
        });
    }
    Ok(())
}

fn compute_state_facts(
    parts: &EventAttemptParts,
    snapshot: &RunSnapshot,
    branch: EventCommitBranch,
    draft: &JournalDraft,
) -> Result<(StateFact, StateFact), EventAttemptPersistenceError> {
    if branch == EventCommitBranch::StaleVersions {
        let authoritative = state_fact_from_snapshot(snapshot);
        return Ok((authoritative.clone(), authoritative));
    }
    let before = state_fact_from_snapshot(snapshot);
    if draft.outcome() != OutcomeClass::Completed || !should_apply_state_mutation(draft, parts) {
        return Ok((before.clone(), before));
    }
    let target_state =
        parts
            .target_state
            .as_ref()
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "completed mutation requires target_state".into(),
            })?;
    let target_lifecycle =
        parts
            .target_lifecycle
            .ok_or_else(|| EventAttemptPersistenceError::Validation {
                detail: "completed mutation requires target_lifecycle".into(),
            })?;
    let state_changed = *target_state != parts.source_state;
    let lifecycle_changed = target_lifecycle != snapshot.lifecycle;
    let workflow_after = if state_changed {
        let next = parts
            .expected_workflow_version
            .value()
            .checked_add(1)
            .ok_or(EventAttemptPersistenceError::PersistenceFailed)?;
        WorkflowStateVersion::try_from(next)
            .map_err(|_| EventAttemptPersistenceError::PersistenceFailed)?
    } else {
        parts.expected_workflow_version
    };
    let lifecycle_after = if lifecycle_changed {
        let next = parts
            .expected_lifecycle_version
            .value()
            .checked_add(1)
            .ok_or(EventAttemptPersistenceError::PersistenceFailed)?;
        LifecycleVersion::try_from(next)
            .map_err(|_| EventAttemptPersistenceError::PersistenceFailed)?
    } else {
        parts.expected_lifecycle_version
    };
    Ok((
        before.clone(),
        StateFact {
            state: target_state.clone(),
            lifecycle: target_lifecycle,
            workflow_state_version: workflow_after,
            lifecycle_version: lifecycle_after,
        },
    ))
}

fn should_apply_state_mutation(draft: &JournalDraft, parts: &EventAttemptParts) -> bool {
    draft.outcome() == OutcomeClass::Completed
        && parts.target_state.is_some()
        && parts.target_lifecycle.is_some()
}

fn state_fact_from_snapshot(snapshot: &RunSnapshot) -> StateFact {
    StateFact {
        state: snapshot.current_state.clone(),
        lifecycle: snapshot.lifecycle,
        workflow_state_version: WorkflowStateVersion::try_from(snapshot.workflow_state_version)
            .expect("stored workflow version is positive"),
        lifecycle_version: LifecycleVersion::try_from(snapshot.lifecycle_version)
            .expect("stored lifecycle version is positive"),
    }
}

fn read_run_snapshot(
    conn: &Connection,
    run_id: &RunId,
) -> Result<RunSnapshot, EventAttemptPersistenceError> {
    let (
        state_raw,
        lifecycle_raw,
        workflow_state_version,
        lifecycle_version,
        label_version,
        label,
    ): (String, String, i64, i64, i64, Option<String>) = conn
        .query_row(
            "SELECT current_state, lifecycle, workflow_state_version, lifecycle_version,
                    label_version, label
             FROM runs WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(map_run_lookup_error)?;
    Ok(RunSnapshot {
        current_state: StateId::parse(state_raw).map_err(|error| {
            EventAttemptPersistenceError::Corrupt {
                detail: error.to_string(),
            }
        })?,
        lifecycle: parse_lifecycle(&lifecycle_raw)
            .map_err(|detail| EventAttemptPersistenceError::Corrupt { detail })?,
        workflow_state_version: workflow_state_version as u64,
        lifecycle_version: lifecycle_version as u64,
        label,
        label_version: label_version as u64,
    })
}

fn parse_lifecycle(raw: &str) -> Result<Lifecycle, String> {
    match raw {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        other => Err(format!("unsupported lifecycle: {other}")),
    }
}

fn allocate_sequence(
    conn: &Connection,
    run_id: &RunId,
) -> Result<JournalSequence, EventAttemptPersistenceError> {
    let next: i64 = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_run_lookup_error)?;
    if next <= 0 {
        return Err(EventAttemptPersistenceError::Corrupt {
            detail: "run_journal_sequences.next_sequence must be positive".into(),
        });
    }
    let updated = conn
        .execute(
            "UPDATE run_journal_sequences SET next_sequence = next_sequence + 1 WHERE run_id = ?1",
            params![run_id.as_str()],
        )
        .map_err(map_sqlite_write_error)?;
    if updated != 1 {
        return Err(EventAttemptPersistenceError::NotFound);
    }
    JournalSequence::try_from(next as u64).map_err(|_| EventAttemptPersistenceError::Corrupt {
        detail: "allocated journal sequence invalid".into(),
    })
}

fn materialize_journal(
    draft: &JournalDraft,
    sequence: JournalSequence,
    state_before: StateFact,
    state_after: StateFact,
) -> Result<(JournalEntry, String, OutcomeClass), EventAttemptPersistenceError> {
    let provisional = provisional_encoded_sizes(draft);
    let entry = draft
        .clone()
        .finalize(
            sequence,
            state_before.clone(),
            state_after.clone(),
            provisional,
        )
        .map_err(map_journal_error)?;
    let payload_json = encode_journal_entry(&entry)?;
    let encoded_sizes = measured_encoded_sizes(&entry, &payload_json);
    validate_component_bounds(&encoded_sizes)?;
    if encoded_sizes.entry > JOURNAL_ENTRY_ENCODED_BYTES {
        return Err(EventAttemptPersistenceError::ResourceExhausted);
    }
    let entry = draft
        .clone()
        .finalize(sequence, state_before, state_after, encoded_sizes)
        .map_err(map_journal_error)?;
    let payload_json = encode_journal_entry(&entry)?;
    if payload_json.len() != entry.encoded_size() {
        return Err(EventAttemptPersistenceError::Corrupt {
            detail: "encoded journal length does not match declared size".into(),
        });
    }
    if payload_json.len() > JOURNAL_ENTRY_ENCODED_BYTES {
        return Err(EventAttemptPersistenceError::ResourceExhausted);
    }
    Ok((entry, payload_json, draft.outcome()))
}

fn validate_component_bounds(
    sizes: &JournalEncodedSizes,
) -> Result<(), EventAttemptPersistenceError> {
    let check = |actual: usize, max: usize| actual <= max;
    if !check(sizes.entry, JOURNAL_ENTRY_ENCODED_BYTES)
        || !check(
            sizes.evidence_associations,
            JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES,
        )
        || !check(
            sizes.provider_observations,
            JOURNAL_PROVIDER_FACTS_ENCODED_BYTES,
        )
        || !check(
            sizes.gate_verdict_facts,
            JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES,
        )
        || !check(sizes.note, NOTE_TEXT_UTF8_BYTES)
        || !check(sizes.actor, ACTOR_METADATA_ENCODED_BYTES)
        || !check(sizes.diagnostics, JOURNAL_DIAGNOSTIC_AGGREGATE_BYTES)
    {
        return Err(EventAttemptPersistenceError::ResourceExhausted);
    }
    Ok(())
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
        note: entry
            .attempt()
            .and_then(|facts| facts.note.as_ref())
            .map(|note| note.as_str().len())
            .unwrap_or(0),
        actor: component_len(&wire, "actor"),
    }
}

fn component_len(wire: &Value, field: &str) -> usize {
    wire.get(field)
        .map(|value| serde_json::to_string(value).unwrap_or_default().len())
        .unwrap_or(0)
}

fn diagnostics_len(attempt: Option<&AttemptFacts>) -> usize {
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

fn encode_journal_entry(entry: &JournalEntry) -> Result<String, EventAttemptPersistenceError> {
    let wire = build_journal_wire_from_entry(entry)?;
    serde_json::to_string(&wire).map_err(|error| EventAttemptPersistenceError::Corrupt {
        detail: format!("journal JSON encode failed: {error}"),
    })
}

fn build_journal_wire_from_entry(
    entry: &JournalEntry,
) -> Result<Value, EventAttemptPersistenceError> {
    build_journal_wire_fields(
        entry.run_id(),
        entry.sequence(),
        mapping::format_observed_at(&entry.observed_at()),
        entry.operation(),
        entry.request_id(),
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
fn build_journal_wire_fields(
    run_id: &RunId,
    sequence: JournalSequence,
    ts: String,
    operation: &str,
    request_id: &loop_engine_core::model::ids::RequestId,
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    reason: Option<&loop_engine_core::model::reason::Reason>,
    state_before: &StateFact,
    state_after: &StateFact,
    attempt: Option<&AttemptFacts>,
    extension: &JournalExtension,
) -> Result<Value, EventAttemptPersistenceError> {
    let mut value = json!({
        "journal_schema_version": JOURNAL_PAYLOAD_SCHEMA_VERSION,
        "sequence": sequence.value(),
        "run_id": run_id.as_str(),
        "ts": ts,
        "operation": operation,
        "request_id": request_id.as_str(),
        "entry_kind": entry_kind_wire(kind),
        "outcome": outcome_wire(outcome),
        "reason": encode_reason(reason)?,
        "state_before": encode_state_fact(state_before),
        "state_after": encode_state_fact(state_after),
    });
    let object = value
        .as_object_mut()
        .ok_or_else(|| EventAttemptPersistenceError::Corrupt {
            detail: "journal wire root must be object".into(),
        })?;
    if let Some(attempt) = attempt {
        if let Some(transition) = &attempt.transition {
            object.insert("transition".into(), encode_transition(transition));
        }
        if !attempt.provider_observations.is_empty() {
            object.insert(
                "provider_observations".into(),
                encode_provider_observations(&attempt.provider_observations),
            );
        }
        if let Some(gate) = &attempt.gate_verdict_facts {
            object.insert("gate_verdict_facts".into(), encode_gate_verdict_facts(gate));
        }
        if let Some(associations) = &attempt.evidence_associations {
            object.insert(
                "evidence_associations".into(),
                encode_evidence_associations(associations),
            );
        }
        if let Some(recorded) = attempt.evidence_recorded {
            object.insert(
                "evidence_recorded".into(),
                encode_evidence_recorded(recorded),
            );
        }
        if let Some(note) = &attempt.note {
            object.insert("note".into(), Value::String(note.as_str().to_owned()));
        }
        if let Some(actor) = &attempt.actor {
            object.insert("actor".into(), core_value_to_json(actor.value())?);
        }
        if let Some(sequence) = attempt.corrects_sequence {
            object.insert(
                "corrects_sequence".into(),
                Value::Number(sequence.value().into()),
            );
        }
        if !attempt.diagnostics.is_empty() {
            object.insert(
                "diagnostics".into(),
                Value::Array(
                    attempt
                        .diagnostics
                        .iter()
                        .map(|diagnostic| {
                            Ok(json!({
                                "code": diagnostic.code(),
                                "message": diagnostic.message(),
                                "path": diagnostic.path(),
                            }))
                        })
                        .collect::<Result<_, EventAttemptPersistenceError>>()?,
                ),
            );
        }
    }
    match extension {
        JournalExtension::TransitionAttempt => {}
        _ => {
            return Err(EventAttemptPersistenceError::Validation {
                detail: "event attempt writer supports transition attempts only".into(),
            });
        }
    }
    Ok(value)
}

fn encode_state_fact(fact: &StateFact) -> Value {
    json!({
        "state": fact.state.as_str(),
        "lifecycle": lifecycle_wire(fact.lifecycle),
        "workflow_state_version": fact.workflow_state_version.value(),
        "lifecycle_version": fact.lifecycle_version.value(),
    })
}

fn encode_reason(
    reason: Option<&loop_engine_core::model::reason::Reason>,
) -> Result<Value, EventAttemptPersistenceError> {
    match reason {
        None => Ok(Value::Null),
        Some(reason) => Ok(json!({
            "code": reason.code().code(),
            "message": reason.message(),
        })),
    }
}

fn encode_transition(transition: &TransitionFact) -> Value {
    json!({
        "event": transition.event.as_str(),
        "source_state": transition.source.as_str(),
        "target_state": transition.target.as_ref().map(|state| state.as_str()),
        "applied": transition.applied,
    })
}

fn encode_provider_observations(observations: &[ProviderFact]) -> Value {
    Value::Array(
        observations
            .iter()
            .map(|fact| {
                let mut value = json!({
                    "registration_id": fact.registration_id.as_str(),
                    "config_revision": fact.config_revision,
                    "role": provider_role_wire(fact.role),
                    "invocation_id": fact.invocation_id.as_str(),
                    "executable": fact.executable.as_str(),
                    "outcome": outcome_wire(fact.outcome),
                });
                if let DigestObservation::Observed(digest) = &fact.digest {
                    value["executable_digest"] = Value::String(digest.as_str().to_owned());
                }
                if let Some(version) = &fact.provider_version {
                    value["provider_version"] = Value::String(version.as_str().to_owned());
                }
                if let Some(major) = fact.protocol_major {
                    value["protocol_major"] = Value::Number(major.into());
                }
                value
            })
            .collect(),
    )
}

fn encode_gate_verdict_facts(facts: &GateVerdictFacts) -> Value {
    let mut value = json!({
        "event": facts.event.as_str(),
        "gate_ids": facts.gate_ids.iter().map(GateId::as_str).collect::<Vec<_>>(),
    });
    match &facts.result {
        GateVerdictResult::Verdicts(verdicts) => {
            value["verdicts"] = Value::Array(
                verdicts
                    .iter()
                    .map(|verdict| {
                        let mut row = json!({
                            "gate_id": verdict.gate_id.as_str(),
                            "status": if verdict.passed { "pass" } else { "fail" },
                        });
                        if let Some(message) = &verdict.message {
                            row["message"] = Value::String(message.as_str().to_owned());
                        }
                        row
                    })
                    .collect(),
            );
        }
        GateVerdictResult::Incompatibility(diagnostic) => {
            value["incompatibility"] = json!({
                "code": diagnostic.code(),
                "message": diagnostic.message(),
                "path": diagnostic.path(),
            });
        }
        GateVerdictResult::EvaluationError(diagnostics) => {
            value["evaluation_error"] = Value::Array(
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
                    .collect(),
            );
        }
    }
    value
}

fn encode_evidence_associations(
    associations: &loop_engine_core::model::attempt::EvidenceAssociations,
) -> Value {
    let mut value = json!({});
    if !associations.inline.is_empty() {
        value["inline"] = Value::Array(
            associations
                .inline
                .iter()
                .map(|record| {
                    json!({
                        "evidence_id": record.id().as_str(),
                        "kind": record.kind().as_str(),
                        "locator": record.locator(),
                    })
                })
                .collect(),
        );
    }
    if !associations.selected_ids.is_empty() {
        value["selected_ids"] = Value::Array(
            associations
                .selected_ids
                .iter()
                .map(|id| Value::String(id.as_str().to_owned()))
                .collect(),
        );
    }
    if !associations.provider_recorded_ids.is_empty() {
        value["provider_recorded_ids"] = Value::Array(
            associations
                .provider_recorded_ids
                .iter()
                .map(|id| Value::String(id.as_str().to_owned()))
                .collect(),
        );
    }
    value
}

fn encode_evidence_recorded(recorded: EvidenceRecordedStatus) -> Value {
    json!({
        "inline": recorded.inline,
        "selected_associations": recorded.selected_associations,
        "provider": recorded.provider,
    })
}

fn core_value_to_json(value: &CoreValue) -> Result<Value, EventAttemptPersistenceError> {
    Ok(match value {
        CoreValue::Null => Value::Null,
        CoreValue::Bool(value) => Value::Bool(*value),
        CoreValue::Number(value) => {
            Value::Number(serde_json::Number::from_f64(value.value()).ok_or_else(|| {
                EventAttemptPersistenceError::Corrupt {
                    detail: "non-finite actor number".into(),
                }
            })?)
        }
        CoreValue::String(value) => Value::String(value.clone()),
        CoreValue::Array(values) => Value::Array(
            values
                .iter()
                .map(core_value_to_json)
                .collect::<Result<_, _>>()?,
        ),
        CoreValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| core_value_to_json(value).map(|json| (key.clone(), json)))
                .collect::<Result<_, _>>()?,
        ),
    })
}

fn existing_evidence_conflict(
    conn: &Connection,
    parts: &EventAttemptParts,
) -> Result<Option<EvidenceConflictKind>, EventAttemptPersistenceError> {
    for evidence in &parts.provider_evidence {
        if evidence_exists(conn, &parts.run_id, evidence.id())? {
            return Ok(Some(EvidenceConflictKind::Provider));
        }
    }
    for evidence in &parts.inline_evidence {
        if evidence_exists(conn, &parts.run_id, evidence.id())? {
            return Ok(Some(EvidenceConflictKind::Inline));
        }
    }
    Ok(None)
}

fn insert_evidence_records(
    conn: &Connection,
    parts: &EventAttemptParts,
) -> Result<(), EventAttemptPersistenceError> {
    for evidence in parts
        .inline_evidence
        .iter()
        .chain(parts.provider_evidence.iter())
    {
        let row =
            mapping::evidence_record_row(&parts.run_id, evidence).map_err(map_mapping_error)?;
        conn.execute(
            "INSERT INTO evidence (
                run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
        .map_err(map_sqlite_write_error)?;
    }
    Ok(())
}

fn u64_sql(value: u64) -> Result<i64, EventAttemptPersistenceError> {
    i64::try_from(value).map_err(|_| EventAttemptPersistenceError::Corrupt {
        detail: format!("sqlite integer overflow: {value}"),
    })
}

fn apply_run_mutation(
    conn: &Connection,
    parts: &EventAttemptParts,
    state_after: &StateFact,
) -> Result<(), EventAttemptPersistenceError> {
    let updated = conn
        .execute(
            "UPDATE runs
             SET current_state = ?1,
                 lifecycle = ?2,
                 workflow_state_version = ?3,
                 lifecycle_version = ?4
             WHERE run_id = ?5
               AND workflow_state_version = ?6
               AND lifecycle_version = ?7",
            params![
                state_after.state.as_str(),
                lifecycle_wire(state_after.lifecycle),
                u64_sql(state_after.workflow_state_version.value())?,
                u64_sql(state_after.lifecycle_version.value())?,
                parts.run_id.as_str(),
                u64_sql(parts.expected_workflow_version.value())?,
                u64_sql(parts.expected_lifecycle_version.value())?,
            ],
        )
        .map_err(map_sqlite_write_error)?;
    if updated != 1 {
        return Err(EventAttemptPersistenceError::PersistenceFailed);
    }
    Ok(())
}

fn insert_journal_row(
    conn: &Connection,
    run_id: &RunId,
    sequence: JournalSequence,
    outcome: OutcomeClass,
    payload_json: &str,
) -> Result<(), EventAttemptPersistenceError> {
    conn.execute(
        "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            run_id.as_str(),
            u64_sql(sequence.value())?,
            outcome_wire(outcome),
            payload_json,
        ],
    )
    .map_err(map_sqlite_write_error)
    .map(|_| ())
}

fn insert_associations(
    conn: &Connection,
    run_id: &RunId,
    sequence: JournalSequence,
    associations: &[EvidenceAssociation],
) -> Result<(), EventAttemptPersistenceError> {
    for association in associations {
        conn.execute(
            "INSERT INTO evidence_associations (run_id, journal_sequence, evidence_id, event_id, gate_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id.as_str(),
                u64_sql(sequence.value())?,
                association.evidence_id().as_str(),
                association.event_id().map(EventId::as_str),
                association.gate_id().map(GateId::as_str),
            ],
        )
        .map_err(map_sqlite_write_error)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn event_attempt_bundle_expectation(
    parts: &EventAttemptParts,
    snapshot: &RunSnapshot,
    branch: EventCommitBranch,
    draft: &JournalDraft,
    sequence: JournalSequence,
    payload_json: &str,
    outcome_class: OutcomeClass,
    state_after: &StateFact,
) -> JournalBundleExpectation {
    let authoritative_state = if branch == EventCommitBranch::StaleVersions {
        state_fact_from_snapshot(snapshot)
    } else if should_apply_state_mutation(draft, parts) {
        state_after.clone()
    } else {
        state_fact_from_snapshot(snapshot)
    };
    let (evidence_ids, associations) = if branch == EventCommitBranch::ExpectedVersions {
        (
            parts
                .inline_evidence
                .iter()
                .chain(parts.provider_evidence.iter())
                .map(|evidence| evidence.id().as_str().to_owned())
                .collect(),
            parts
                .associations
                .iter()
                .map(|association| EvidenceAssociationExpectation {
                    run_id: parts.run_id.as_str().to_owned(),
                    journal_sequence: sequence.value(),
                    evidence_id: association.evidence_id().as_str().to_owned(),
                    event_id: association
                        .event_id()
                        .map(|event| event.as_str().to_owned()),
                    gate_id: association.gate_id().map(|gate| gate.as_str().to_owned()),
                })
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let next_sequence = sequence.value().saturating_add(1);
    let run_changed =
        branch == EventCommitBranch::ExpectedVersions && should_apply_state_mutation(draft, parts);
    JournalBundleExpectation {
        run_changed,
        run: RunAuthoritativeExpectation {
            run_id: parts.run_id.as_str().to_owned(),
            current_state: authoritative_state.state.as_str().to_owned(),
            lifecycle: lifecycle_wire(authoritative_state.lifecycle).to_owned(),
            workflow_state_version: authoritative_state.workflow_state_version.value(),
            lifecycle_version: authoritative_state.lifecycle_version.value(),
            label: snapshot.label.clone(),
            label_version: snapshot.label_version,
            next_sequence,
        },
        journal: JournalRowExpectation {
            run_id: parts.run_id.as_str().to_owned(),
            sequence: sequence.value(),
            payload: payload_json.to_owned(),
            outcome: outcome_wire(outcome_class).to_owned(),
        },
        evidence_ids,
        associations,
    }
}

fn build_commit_status(
    branch: EventCommitBranch,
    draft: &JournalDraft,
    state_before: &StateFact,
    state_after: &StateFact,
) -> EventCommitStatus {
    EventCommitStatus {
        branch,
        commit: CommitStatus {
            committed: true,
            state_changed: draft.outcome() == OutcomeClass::Completed
                && state_before.state != state_after.state,
            workflow_state_version: state_after.workflow_state_version,
            lifecycle_version: state_after.lifecycle_version,
        },
    }
}

fn evidence_exists(
    conn: &Connection,
    run_id: &RunId,
    evidence_id: &EvidenceId,
) -> Result<bool, EventAttemptPersistenceError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM evidence WHERE run_id = ?1 AND evidence_id = ?2",
            params![run_id.as_str(), evidence_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_sqlite_read_error)?;
    Ok(count == 1)
}

fn outcome_wire(outcome: OutcomeClass) -> &'static str {
    match outcome {
        OutcomeClass::Completed => "completed",
        OutcomeClass::Rejected => "rejected",
        OutcomeClass::Error => "error",
    }
}

fn lifecycle_wire(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

fn entry_kind_wire(kind: JournalEntryKind) -> &'static str {
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

fn provider_role_wire(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

fn map_run_lookup_error(error: SqliteError) -> EventAttemptPersistenceError {
    match error {
        SqliteError::QueryReturnedNoRows => EventAttemptPersistenceError::NotFound,
        _ => EventAttemptPersistenceError::Corrupt {
            detail: error.to_string(),
        },
    }
}

fn map_sqlite_read_error(error: SqliteError) -> EventAttemptPersistenceError {
    if matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt) | Some(rusqlite::ErrorCode::NotADatabase)
    ) {
        EventAttemptPersistenceError::PersistenceFailed
    } else {
        EventAttemptPersistenceError::Corrupt {
            detail: error.to_string(),
        }
    }
}

fn map_sqlite_write_error(error: SqliteError) -> EventAttemptPersistenceError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        let message = error.to_string();
        if message.contains("evidence.") {
            EventAttemptPersistenceError::EvidenceInvalid
        } else {
            EventAttemptPersistenceError::PersistenceFailed
        }
    } else if matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt) | Some(rusqlite::ErrorCode::NotADatabase)
    ) {
        EventAttemptPersistenceError::PersistenceFailed
    } else {
        EventAttemptPersistenceError::Corrupt {
            detail: error.to_string(),
        }
    }
}

fn map_journal_error(error: JournalError) -> EventAttemptPersistenceError {
    match error {
        JournalError::Bound(_) => EventAttemptPersistenceError::ResourceExhausted,
        other => EventAttemptPersistenceError::Journal(other),
    }
}

fn map_mapping_error(error: MappingError) -> EventAttemptPersistenceError {
    EventAttemptPersistenceError::Corrupt {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::persistence_commands::EventAttemptParts;
    use loop_engine_core::model::attempt::{
        AttemptFacts, EvidenceAssociations, GateVerdictFact, GateVerdictFacts, GateVerdictResult,
        JournalExtension, ProviderFact, ProviderRole, TransitionFact,
    };
    use loop_engine_core::model::bounded::{EVIDENCE_LOCATOR_UTF8_BYTES, NOTE_TEXT_UTF8_BYTES};
    use loop_engine_core::model::decision::{DecisionError, TransitionDecision, resolve_gate_free};
    use loop_engine_core::model::evidence::{EvidenceAssociation, EvidenceRecord, EvidenceSource};
    use loop_engine_core::model::ids::{
        EventId, EvidenceId, EvidenceKind, GateId, RegistrationId, RequestId, RunId, StateId,
    };
    use loop_engine_core::model::journal::JournalDraft;
    use loop_engine_core::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
    use loop_engine_core::model::provider::DigestObservation;
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::model::run::Run;
    use loop_engine_core::model::time::ObservedAt;
    use loop_engine_core::model::version::WorkflowStateVersion;
    use rusqlite::{Connection, params};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::run_reads::SqliteRunReads;
    use crate::persistence::sqlite::open_at;
    use crate::persistence::traced::test_support::{event_names, read_events, test_sink};
    use crate::persistence::traced::{OptionalTraceSink, finish_traced_event_write};
    /// Frozen canonical graph for gate-free checkpoint/advance event-attempt tests.
    const REQUEST_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}},{"final":false,"id":"review","static_guidance":{"kind":"text","text":"Review the change."}}],"transitions":[{"event_id":"advance","gate_ids":[],"source_state_id":"draft","target_state_id":"review"},{"event_id":"checkpoint","gate_ids":[],"source_state_id":"draft","target_state_id":"draft"}]}"#;
    const REQUEST_GRAPH_REVISION: &str =
        "sha256:d5b2dc73bbb81d7ce3802c6a1ad3b8ff86f51a40fc61b095a86432d5fc29dc19";
    const GATED_REQUEST_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}},{"final":false,"id":"review","static_guidance":{"kind":"text","text":"Review the change."}}],"transitions":[{"event_id":"advance","gate_ids":["gate-1"],"source_state_id":"draft","target_state_id":"review"},{"event_id":"checkpoint","gate_ids":[],"source_state_id":"draft","target_state_id":"draft"}]}"#;
    const GATED_REQUEST_GRAPH_REVISION: &str =
        "sha256:dbb3c1eb56bf177c305320824c52ec2b4ab195e8403cec8ac6e16a7df2149f8f";

    fn store() -> (
        MutexGuard<'static, ()>,
        TempDir,
        SqliteEventAttemptWriter,
        SqliteRunReads,
    ) {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        (
            guard,
            dir,
            SqliteEventAttemptWriter::new(path.clone()),
            SqliteRunReads::new(path),
        )
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

    fn insert_run_with_graph(
        conn: &Connection,
        run_id: &str,
        registration_id: &str,
        graph_revision: &str,
        graph_json: &str,
    ) {
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES (?1, ?2, 1, 'draft', 'active', 1, 1, 1, ?3, 1, ?4, '{}', '2026-07-17T12:00:00.000Z')",
            params![run_id, registration_id, graph_revision, graph_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
            params![run_id],
        )
        .unwrap();
    }

    fn insert_run(conn: &Connection, run_id: &str, registration_id: &str) {
        insert_run_with_graph(
            conn,
            run_id,
            registration_id,
            REQUEST_GRAPH_REVISION,
            REQUEST_GRAPH_JSON,
        );
    }

    fn gated_pre_provider_error_attempt(run: &Run, event: &EventId) -> AttemptFacts {
        AttemptFacts {
            transition: Some(
                TransitionFact::new(event.clone(), run.current_state().clone(), None, false)
                    .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        }
    }

    fn gated_parts(run: &Run, passed: bool) -> EventAttemptParts {
        let event = EventId::parse("advance").unwrap();
        let target = StateId::parse("review").unwrap();
        let gate = GateId::parse("gate-1").unwrap();
        let provider_fact = ProviderFact::new(
            RegistrationId::parse("reg-1").unwrap(),
            1,
            ProviderRole::EvaluateGates,
            RequestId::parse("provider-request-1").unwrap(),
            "/bin/provider",
            OutcomeClass::Completed,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap();
        let gate_facts = GateVerdictFacts::new(
            event.clone(),
            vec![gate.clone()],
            GateVerdictResult::Verdicts(vec![GateVerdictFact::new(gate, passed, None).unwrap()]),
        )
        .unwrap();
        let attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(target.clone()),
                    passed,
                )
                .unwrap(),
            ),
            provider_observations: vec![provider_fact.clone()],
            gate_verdict_facts: Some(gate_facts.clone()),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        };
        let stale_attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    event,
                    run.current_state().clone(),
                    Some(target.clone()),
                    false,
                )
                .unwrap(),
            ),
            provider_observations: vec![provider_fact],
            gate_verdict_facts: Some(gate_facts),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        };
        let outcome = if passed {
            OutcomeClass::Completed
        } else {
            OutcomeClass::Rejected
        };
        EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state: Some(target),
            target_lifecycle: passed.then_some(Lifecycle::Active),
            inline_evidence: vec![],
            associations: vec![],
            provider_evidence: vec![],
            journal_entry: journal_draft(run, outcome, attempt),
            stale_journal_entry: journal_draft(run, OutcomeClass::Error, stale_attempt),
        }
    }

    fn sample_evidence(id: &str) -> EvidenceRecord {
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

    fn transition_attempt(
        run: &Run,
        event: &EventId,
        target: &StateId,
        _outcome: OutcomeClass,
        applied: bool,
        inline: Vec<EvidenceRecord>,
    ) -> AttemptFacts {
        AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(target.clone()),
                    applied,
                )
                .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations {
                inline: inline.clone(),
                ..EvidenceAssociations::default()
            }),
            evidence_recorded: Some(EvidenceRecordedStatus {
                inline: !inline.is_empty(),
                ..Default::default()
            }),
            ..AttemptFacts::default()
        }
    }

    fn journal_draft(run: &Run, outcome: OutcomeClass, attempt: AttemptFacts) -> JournalDraft {
        let reason = match outcome {
            OutcomeClass::Completed => None,
            OutcomeClass::Rejected => {
                Some(Reason::new(ReasonCode::GateFailed, "gate failed").unwrap())
            }
            OutcomeClass::Error => {
                Some(Reason::new(ReasonCode::StateStaleVersion, "stale").unwrap())
            }
        };
        journal_draft_with_reason(run, outcome, reason, attempt)
    }

    fn journal_draft_with_reason(
        run: &Run,
        outcome: OutcomeClass,
        reason: Option<Reason>,
        attempt: AttemptFacts,
    ) -> JournalDraft {
        JournalDraft::new(
            run.id().clone(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.request",
            RequestId::parse("request-1").unwrap(),
            outcome,
            reason,
            Some(attempt),
            JournalExtension::TransitionAttempt,
        )
        .unwrap()
    }

    fn gate_free_pre_resolution_attempt(
        run: &Run,
        event: &EventId,
        target: Option<StateId>,
    ) -> AttemptFacts {
        AttemptFacts {
            transition: Some(
                TransitionFact::new(event.clone(), run.current_state().clone(), target, false)
                    .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        }
    }

    fn stale_transition_attempt(run: &Run, event: &EventId, target: &StateId) -> AttemptFacts {
        AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(target.clone()),
                    false,
                )
                .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        }
    }

    fn gate_free_parts(
        decision: &TransitionDecision,
        inline: Vec<EvidenceRecord>,
        associations: Vec<EvidenceAssociation>,
        completed_attempt: AttemptFacts,
        stale_attempt: AttemptFacts,
        run: &Run,
    ) -> EventAttemptParts {
        EventAttemptParts {
            run_id: decision.run_id().clone(),
            expected_workflow_version: decision.expected_workflow_version(),
            expected_lifecycle_version: decision.expected_lifecycle_version(),
            source_state: decision.source().clone(),
            target_state: Some(decision.target().clone()),
            target_lifecycle: Some(decision.lifecycle()),
            inline_evidence: inline,
            associations,
            provider_evidence: decision.provider_evidence().to_vec(),
            journal_entry: journal_draft(run, OutcomeClass::Completed, completed_attempt),
            stale_journal_entry: journal_draft(run, OutcomeClass::Error, stale_attempt),
        }
    }

    fn self_loop_parts(run: &Run) -> EventAttemptParts {
        let event = EventId::parse("checkpoint").unwrap();
        let decision = resolve_gate_free(run, &event).unwrap();
        let inline = sample_evidence("inline-1");
        let associations = vec![EvidenceAssociation::new(
            inline.id().clone(),
            Some(event.clone()),
            None,
        )];
        let completed_attempt = transition_attempt(
            run,
            &event,
            decision.target(),
            OutcomeClass::Completed,
            true,
            vec![inline.clone()],
        );
        let stale_attempt = stale_transition_attempt(run, &event, decision.target());
        gate_free_parts(
            &decision,
            vec![inline],
            associations,
            completed_attempt,
            stale_attempt,
            run,
        )
    }

    fn advance_parts(run: &Run) -> EventAttemptParts {
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(run, &event).unwrap();
        let inline = sample_evidence("inline-advance");
        let associations = vec![EvidenceAssociation::new(
            inline.id().clone(),
            Some(event.clone()),
            None,
        )];
        let completed_attempt = transition_attempt(
            run,
            &event,
            decision.target(),
            OutcomeClass::Completed,
            true,
            vec![inline.clone()],
        );
        let stale_attempt = stale_transition_attempt(run, &event, decision.target());
        gate_free_parts(
            &decision,
            vec![inline],
            associations,
            completed_attempt,
            stale_attempt,
            run,
        )
    }

    fn count_table(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn commit_traced(
        writer: &SqliteEventAttemptWriter,
        trace: &OptionalTraceSink,
        parts: EventAttemptParts,
    ) -> Result<EventCommitStatus, EventAttemptPersistenceError> {
        let expected_draft = parts.journal_entry.clone();
        let stale_draft = parts.stale_journal_entry.clone();
        finish_traced_event_write(
            trace,
            "run.request",
            |session| {
                writer.commit_parts(parts, session).map_ok(|status| {
                    (
                        status.clone(),
                        event_commit_semantic(&status, &expected_draft, &stale_draft),
                    )
                })
            },
            event_attempt_error_semantic,
        )
    }

    #[test]
    fn completed_self_loop_commits_without_workflow_version_bump() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let status = writer
            .commit_parts(self_loop_parts(&run), None)
            .into_result()
            .unwrap();
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        let wf: i64 = conn
            .query_row(
                "SELECT workflow_state_version FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wf, 1);
        assert_eq!(count_table(&conn, "evidence"), 1);
        assert_eq!(count_table(&conn, "journal_entries"), 1);
        assert_eq!(count_table(&conn, "evidence_associations"), 1);
    }

    #[test]
    fn repeated_inline_evidence_id_commits_rejected_attempt_without_partial_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        writer
            .commit_parts(self_loop_parts(&run), None)
            .into_result()
            .unwrap();

        let status = writer
            .commit_parts(self_loop_parts(&run), None)
            .into_result()
            .unwrap();
        assert_eq!(status.branch, EventCommitBranch::InlineEvidenceConflict);
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        assert_eq!(count_table(&conn, "evidence"), 1);
        assert_eq!(count_table(&conn, "evidence_associations"), 1);
        assert_eq!(count_table(&conn, "journal_entries"), 2);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "rejected");
        assert_eq!(wire["reason"]["code"], "evidence.invalid");
        assert_eq!(wire["transition"]["applied"], Value::Bool(false));
        assert_eq!(wire["evidence_recorded"]["inline"], Value::Bool(false));
    }

    #[test]
    fn existing_provider_evidence_id_commits_error_attempt_with_provider_fact() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run_with_graph(
            &conn,
            "run-1",
            "reg-1",
            GATED_REQUEST_GRAPH_REVISION,
            GATED_REQUEST_GRAPH_JSON,
        );
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let mut parts = gated_parts(&run, true);
        let evidence = EvidenceRecord::new(
            EvidenceId::parse("provider-existing").unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            "opaque:provider",
            None,
            None,
            None,
            EvidenceSource::Provider,
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
        )
        .unwrap();
        let association = EvidenceAssociation::new(
            evidence.id().clone(),
            Some(EventId::parse("advance").unwrap()),
            Some(GateId::parse("gate-1").unwrap()),
        );
        let recorded_associations = EvidenceAssociations {
            inline: vec![],
            selected_ids: vec![],
            provider_recorded_ids: vec![evidence.id().clone()],
        };
        let mut completed_attempt = parts.journal_entry.attempt().unwrap().clone();
        completed_attempt.evidence_associations = Some(recorded_associations.clone());
        completed_attempt.evidence_recorded = Some(recorded_associations.recorded_status());
        parts.provider_evidence = vec![evidence];
        parts.associations = vec![association];
        parts.journal_entry = journal_draft(&run, OutcomeClass::Completed, completed_attempt);
        insert_evidence_records(&conn, &parts).unwrap();

        let status = writer.commit_parts(parts, None).into_result().unwrap();
        assert_eq!(status.branch, EventCommitBranch::ProviderEvidenceConflict);
        assert!(!status.commit.state_changed);
        assert_eq!(count_table(&conn, "evidence"), 1);
        assert_eq!(count_table(&conn, "evidence_associations"), 0);
        assert_eq!(count_table(&conn, "journal_entries"), 1);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "error");
        assert_eq!(wire["reason"]["code"], "provider.evidence.malformed");
        assert_eq!(wire["transition"]["applied"], Value::Bool(false));
        assert_eq!(wire["provider_observations"].as_array().unwrap().len(), 1);
        assert_eq!(wire["evidence_recorded"]["provider"], Value::Bool(false));
    }

    #[test]
    fn evidence_recorded_status_matches_persisted_bundle() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        writer
            .commit_parts(self_loop_parts(&run), None)
            .into_result()
            .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["evidence_recorded"]["inline"], Value::Bool(true));
        assert_eq!(
            wire["evidence_recorded"]["selected_associations"],
            Value::Bool(false)
        );
        assert_eq!(wire["evidence_recorded"]["provider"], Value::Bool(false));
    }

    #[test]
    fn aggregate_overflow_rejects_without_partial_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("checkpoint").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        let mut inline = Vec::new();
        let locator = format!(
            "opaque:{}",
            "x".repeat(EVIDENCE_LOCATOR_UTF8_BYTES - "opaque:".len())
        );
        for index in 0..400 {
            inline.push(
                EvidenceRecord::new(
                    EvidenceId::parse(format!("inline-{index:04}")).unwrap(),
                    EvidenceKind::parse("artifact").unwrap(),
                    locator.clone(),
                    None,
                    None,
                    None,
                    EvidenceSource::Caller,
                    ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
                )
                .unwrap(),
            );
        }
        let associations = inline
            .iter()
            .map(|evidence| {
                EvidenceAssociation::new(evidence.id().clone(), Some(event.clone()), None)
            })
            .collect::<Vec<_>>();
        let completed_attempt = AttemptFacts {
            note: Some(
                loop_engine_core::model::annotation::Note::new("x".repeat(NOTE_TEXT_UTF8_BYTES))
                    .unwrap(),
            ),
            ..transition_attempt(
                &run,
                &event,
                decision.target(),
                OutcomeClass::Completed,
                true,
                inline.clone(),
            )
        };
        let stale_attempt = stale_transition_attempt(&run, &event, decision.target());
        let parts = gate_free_parts(
            &decision,
            inline,
            associations,
            completed_attempt,
            stale_attempt,
            &run,
        );
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::ResourceExhausted
        ));
        assert_eq!(count_table(&conn, "evidence"), 0);
        assert_eq!(count_table(&conn, "journal_entries"), 0);
    }

    #[test]
    fn state_changing_transition_increments_workflow_version() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let status = writer
            .commit_parts(advance_parts(&run), None)
            .into_result()
            .unwrap();
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 2);
        let current_state: String = conn
            .query_row(
                "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current_state, "review");
    }

    #[test]
    fn stale_branch_appends_journal_without_evidence_or_state_mutation() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let stale_parts = self_loop_parts(&run);
        writer
            .commit_parts(advance_parts(&run), None)
            .into_result()
            .unwrap();
        let status = writer
            .commit_parts(stale_parts, None)
            .into_result()
            .unwrap();
        assert_eq!(status.branch, EventCommitBranch::StaleVersions);
        assert_eq!(count_table(&conn, "evidence"), 1);
        assert_eq!(count_table(&conn, "journal_entries"), 2);
        assert_eq!(count_table(&conn, "evidence_associations"), 1);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 3",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "error");
        assert_eq!(wire["state_before"]["state"], "review");
        assert_eq!(wire["state_after"]["state"], "review");
        assert_eq!(wire["state_before"]["workflow_state_version"], 2);
        let recorded = wire.get("evidence_recorded");
        assert!(
            recorded.is_none()
                || (recorded.unwrap()["inline"] == Value::Bool(false)
                    && recorded.unwrap()["selected_associations"] == Value::Bool(false)
                    && recorded.unwrap()["provider"] == Value::Bool(false))
        );
        if let Some(associations) = wire.get("evidence_associations") {
            for key in ["inline", "selected_ids", "provider_recorded_ids"] {
                assert!(
                    associations
                        .get(key)
                        .is_none_or(|value| { value.as_array().is_some_and(Vec::is_empty) }),
                    "stale journal must not claim {key} evidence"
                );
            }
        }
    }

    #[test]
    fn sqlite_abort_triggers_roll_back_event_attempt_atomically() {
        let cases = [
            (
                "evidence_insert",
                "CREATE TRIGGER abort_event_evidence BEFORE INSERT ON evidence BEGIN SELECT RAISE(ABORT, 'test evidence abort'); END",
                false,
            ),
            (
                "run_update",
                "CREATE TRIGGER abort_event_run BEFORE UPDATE OF current_state ON runs BEGIN SELECT RAISE(ABORT, 'test run abort'); END",
                true,
            ),
            (
                "journal_insert",
                "CREATE TRIGGER abort_event_journal BEFORE INSERT ON journal_entries BEGIN SELECT RAISE(ABORT, 'test journal abort'); END",
                false,
            ),
            (
                "association_insert",
                "CREATE TRIGGER abort_event_association BEFORE INSERT ON evidence_associations BEGIN SELECT RAISE(ABORT, 'test association abort'); END",
                false,
            ),
            (
                "sequence_update",
                "CREATE TRIGGER abort_event_sequence BEFORE UPDATE OF next_sequence ON run_journal_sequences BEGIN SELECT RAISE(ABORT, 'test sequence abort'); END",
                false,
            ),
        ];

        for (boundary, trigger_sql, state_changing) in cases {
            let (_guard, _dir, writer, reads) = store();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run(&conn, "run-1", "reg-1");
            conn.execute_batch(trigger_sql).unwrap();
            let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
            let parts = if state_changing {
                advance_parts(&run)
            } else {
                self_loop_parts(&run)
            };

            writer.commit_parts(parts, None).into_result().unwrap_err();

            assert_eq!(count_table(&conn, "evidence"), 0, "{boundary}");
            assert_eq!(count_table(&conn, "journal_entries"), 0, "{boundary}");
            assert_eq!(count_table(&conn, "evidence_associations"), 0, "{boundary}");
            let (state, version, next_sequence): (String, i64, i64) = conn
                .query_row(
                    "SELECT r.current_state, r.workflow_state_version, s.next_sequence
                     FROM runs r
                     JOIN run_journal_sequences s ON s.run_id = r.run_id
                     WHERE r.run_id = 'run-1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(state, "draft", "{boundary}");
            assert_eq!(version, 1, "{boundary}");
            assert_eq!(next_sequence, 2, "{boundary}");
        }
    }

    #[test]
    fn mismatched_evidence_recorded_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let mut parts = self_loop_parts(&run);
        parts.inline_evidence.clear();
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 0);
    }

    #[test]
    fn wrong_target_forged_command_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        let inline = sample_evidence("inline-forged");
        let associations = vec![EvidenceAssociation::new(
            inline.id().clone(),
            Some(event.clone()),
            None,
        )];
        let forged_target = StateId::parse("draft").unwrap();
        let completed_attempt = transition_attempt(
            &run,
            &event,
            &forged_target,
            OutcomeClass::Completed,
            true,
            vec![inline.clone()],
        );
        let stale_attempt = stale_transition_attempt(&run, &event, decision.target());
        let parts = EventAttemptParts {
            run_id: decision.run_id().clone(),
            expected_workflow_version: decision.expected_workflow_version(),
            expected_lifecycle_version: decision.expected_lifecycle_version(),
            source_state: decision.source().clone(),
            target_state: Some(decision.target().clone()),
            target_lifecycle: Some(decision.lifecycle()),
            inline_evidence: vec![inline],
            associations,
            provider_evidence: decision.provider_evidence().to_vec(),
            journal_entry: journal_draft(&run, OutcomeClass::Completed, completed_attempt),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 0);
        assert_eq!(count_table(&conn, "evidence"), 0);
    }

    #[test]
    fn gate_free_state_derivation_commits_authoritative_target() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        assert_eq!(decision.target().as_str(), "review");
        let status = writer
            .commit_parts(advance_parts(&run), None)
            .into_result()
            .unwrap();
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(status.commit.state_changed);
        let current_state: String = conn
            .query_row(
                "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current_state, "review");
    }

    #[test]
    fn wrong_source_forged_command_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        let inline = sample_evidence("inline-forged-source");
        let associations = vec![EvidenceAssociation::new(
            inline.id().clone(),
            Some(event.clone()),
            None,
        )];
        let forged_source = StateId::parse("review").unwrap();
        let completed_attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    event.clone(),
                    forged_source.clone(),
                    Some(decision.target().clone()),
                    true,
                )
                .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations {
                inline: vec![inline.clone()],
                ..EvidenceAssociations::default()
            }),
            evidence_recorded: Some(EvidenceRecordedStatus {
                inline: true,
                ..Default::default()
            }),
            ..AttemptFacts::default()
        };
        let stale_attempt = stale_transition_attempt(&run, &event, decision.target());
        let parts = EventAttemptParts {
            run_id: decision.run_id().clone(),
            expected_workflow_version: decision.expected_workflow_version(),
            expected_lifecycle_version: decision.expected_lifecycle_version(),
            source_state: forged_source,
            target_state: Some(decision.target().clone()),
            target_lifecycle: Some(decision.lifecycle()),
            inline_evidence: vec![inline],
            associations,
            provider_evidence: decision.provider_evidence().to_vec(),
            journal_entry: journal_draft(&run, OutcomeClass::Completed, completed_attempt),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 0);
        assert_eq!(count_table(&conn, "evidence"), 0);
    }

    #[test]
    fn stale_draft_claiming_evidence_recorded_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        writer
            .commit_parts(advance_parts(&run), None)
            .into_result()
            .unwrap();
        let mut parts = self_loop_parts(&run);
        let inline = sample_evidence("stale-inline");
        parts.stale_journal_entry = journal_draft(
            &run,
            OutcomeClass::Error,
            transition_attempt(
                &run,
                &EventId::parse("checkpoint").unwrap(),
                run.current_state(),
                OutcomeClass::Error,
                false,
                vec![inline],
            ),
        );
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 1);
    }

    #[test]
    fn gate_free_selection_invalid_commits_journal_only_with_unchanged_state() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        let attempt = gate_free_pre_resolution_attempt(&run, &event, None);
        let stale_attempt = stale_transition_attempt(&run, &event, decision.target());
        let parts = EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state: None,
            target_lifecycle: None,
            inline_evidence: vec![],
            associations: vec![],
            provider_evidence: vec![],
            journal_entry: journal_draft_with_reason(
                &run,
                OutcomeClass::Rejected,
                Some(
                    Reason::new(
                        ReasonCode::EvidenceSelectionInvalid,
                        "duplicate selected evidence id",
                    )
                    .unwrap(),
                ),
                attempt,
            ),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let status = writer
            .commit_parts(parts, None)
            .into_result()
            .unwrap_or_else(|error| {
                panic!(
                    "expected journal commit for applicable gate-free selection rejection: {error}"
                )
            });
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        assert_eq!(status.commit.lifecycle_version.value(), 1);
        let current_state: String = conn
            .query_row(
                "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current_state, "draft");
        assert_eq!(count_table(&conn, "journal_entries"), 1);
        assert_eq!(count_table(&conn, "evidence"), 0);
        assert_eq!(count_table(&conn, "evidence_associations"), 0);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "rejected");
        assert_eq!(wire["reason"]["code"], "evidence.selection.invalid");
        assert_eq!(wire["state_before"]["state"], "draft");
        assert_eq!(wire["state_after"]["state"], "draft");
        assert_eq!(wire["state_before"]["workflow_state_version"], 1);
        assert_eq!(wire["state_after"]["workflow_state_version"], 1);
    }

    #[test]
    fn gate_free_persistence_failed_commits_journal_only_with_unchanged_state() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let decision = resolve_gate_free(&run, &event).unwrap();
        let attempt =
            gate_free_pre_resolution_attempt(&run, &event, Some(decision.target().clone()));
        let stale_attempt = stale_transition_attempt(&run, &event, decision.target());
        let parts = EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state: Some(decision.target().clone()),
            target_lifecycle: None,
            inline_evidence: vec![],
            associations: vec![],
            provider_evidence: vec![],
            journal_entry: journal_draft_with_reason(
                &run,
                OutcomeClass::Error,
                Some(
                    Reason::new(
                        ReasonCode::PersistenceFailed,
                        "selected evidence read failed",
                    )
                    .unwrap(),
                ),
                attempt,
            ),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let status = writer
            .commit_parts(parts, None)
            .into_result()
            .unwrap_or_else(|error| {
                panic!(
                    "expected journal commit for applicable gate-free persistence error: {error}"
                )
            });
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        assert_eq!(status.commit.lifecycle_version.value(), 1);
        let current_state: String = conn
            .query_row(
                "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current_state, "draft");
        assert_eq!(count_table(&conn, "journal_entries"), 1);
        assert_eq!(count_table(&conn, "evidence"), 0);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "error");
        assert_eq!(wire["reason"]["code"], "persistence.failed");
        assert_eq!(wire["state_before"]["state"], "draft");
        assert_eq!(wire["state_after"]["state"], "draft");
    }

    #[test]
    fn gate_free_pre_resolution_mismatched_reason_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        let attempt = gate_free_pre_resolution_attempt(&run, &event, None);
        let stale_attempt = stale_transition_attempt(&run, &event, run.current_state());
        let parts = EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state: None,
            target_lifecycle: None,
            inline_evidence: vec![],
            associations: vec![],
            provider_evidence: vec![],
            journal_entry: journal_draft_with_reason(
                &run,
                OutcomeClass::Rejected,
                Some(Reason::new(ReasonCode::GateFailed, "gate failed").unwrap()),
                attempt.clone(),
            ),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let err = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 0);
    }

    #[test]
    fn gated_verdicts_derive_authoritative_completed_and_rejected_branches() {
        for passed in [true, false] {
            let (_guard, _dir, writer, reads) = store();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run_with_graph(
                &conn,
                "run-1",
                "reg-1",
                GATED_REQUEST_GRAPH_REVISION,
                GATED_REQUEST_GRAPH_JSON,
            );
            let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
            let status = writer
                .commit_parts(gated_parts(&run, passed), None)
                .into_result()
                .unwrap();
            assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
            assert_eq!(status.commit.state_changed, passed);
            let state: String = conn
                .query_row(
                    "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, if passed { "review" } else { "draft" });
        }
    }

    #[test]
    fn gated_verdict_event_forgery_rejects_before_write() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run_with_graph(
            &conn,
            "run-1",
            "reg-1",
            GATED_REQUEST_GRAPH_REVISION,
            GATED_REQUEST_GRAPH_JSON,
        );
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let mut parts = gated_parts(&run, true);
        let mut attempt = parts.journal_entry.attempt().unwrap().clone();
        attempt.gate_verdict_facts.as_mut().unwrap().event = EventId::parse("checkpoint").unwrap();
        parts.journal_entry = journal_draft(&run, OutcomeClass::Completed, attempt);
        let error = writer.commit_parts(parts, None).into_result().unwrap_err();
        assert!(matches!(
            error,
            EventAttemptPersistenceError::Validation { .. }
        ));
        assert_eq!(count_table(&conn, "journal_entries"), 0);
    }

    #[test]
    fn gated_pre_provider_error_draft_commits_journal_only_with_unchanged_state() {
        let (_guard, _dir, writer, reads) = store();
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run_with_graph(
            &conn,
            "run-1",
            "reg-1",
            GATED_REQUEST_GRAPH_REVISION,
            GATED_REQUEST_GRAPH_JSON,
        );
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let event = EventId::parse("advance").unwrap();
        assert!(matches!(
            resolve_gate_free(&run, &event),
            Err(DecisionError::GatesRequired)
        ));
        let attempt = gated_pre_provider_error_attempt(&run, &event);
        assert!(
            attempt.provider_observations.is_empty() && attempt.gate_verdict_facts.is_none(),
            "draft shape matched obsolete gate-free predicate"
        );
        let stale_attempt = stale_transition_attempt(&run, &event, run.current_state());
        let parts = EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state: None,
            target_lifecycle: None,
            inline_evidence: vec![],
            associations: vec![],
            provider_evidence: vec![],
            journal_entry: journal_draft_with_reason(
                &run,
                OutcomeClass::Error,
                Some(Reason::new(ReasonCode::PersistenceFailed, "persistence failed").unwrap()),
                attempt,
            ),
            stale_journal_entry: journal_draft(&run, OutcomeClass::Error, stale_attempt),
        };
        let status = writer
            .commit_parts(parts, None)
            .into_result()
            .unwrap_or_else(|error| {
                panic!("expected journal commit, not obsolete gate-free rollback: {error}")
            });
        assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 1);
        assert_eq!(status.commit.lifecycle_version.value(), 1);
        let current_state: String = conn
            .query_row(
                "SELECT current_state FROM runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current_state, "draft");
        assert_eq!(count_table(&conn, "journal_entries"), 1);
        assert_eq!(count_table(&conn, "evidence"), 0);
        assert_eq!(count_table(&conn, "evidence_associations"), 0);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = 'run-1' AND sequence = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let wire: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(wire["outcome"], "error");
        assert_eq!(wire["state_before"]["state"], "draft");
        assert_eq!(wire["state_after"]["state"], "draft");
        assert_eq!(wire["state_before"]["workflow_state_version"], 1);
        assert_eq!(wire["state_after"]["workflow_state_version"], 1);
    }

    #[test]
    fn malformed_provider_fact_and_evidence_result_remain_journalable() {
        for (reason, role) in [
            (
                ReasonCode::ProviderProtocolMalformed,
                ProviderRole::Describe,
            ),
            (
                ReasonCode::ProviderEvidenceMalformed,
                ProviderRole::EvaluateGates,
            ),
        ] {
            let (_guard, _dir, writer, reads) = store();
            let conn = Connection::open(writer.path()).unwrap();
            insert_registration(&conn, "reg-1");
            insert_run_with_graph(
                &conn,
                "run-1",
                "reg-1",
                GATED_REQUEST_GRAPH_REVISION,
                GATED_REQUEST_GRAPH_JSON,
            );
            let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
            let event = EventId::parse("advance").unwrap();
            let target = StateId::parse("review").unwrap();
            let fact = ProviderFact::new(
                RegistrationId::parse("reg-1").unwrap(),
                1,
                role,
                RequestId::parse("provider-request-1").unwrap(),
                "/bin/provider",
                OutcomeClass::Completed,
                DigestObservation::Unavailable,
                None,
                Some(1),
            )
            .unwrap();
            let associations = EvidenceAssociations::default();
            let attempt = AttemptFacts {
                transition: Some(
                    TransitionFact::new(
                        event.clone(),
                        run.current_state().clone(),
                        Some(target.clone()),
                        false,
                    )
                    .unwrap(),
                ),
                provider_observations: vec![fact],
                evidence_associations: Some(associations.clone()),
                evidence_recorded: Some(associations.recorded_status()),
                ..AttemptFacts::default()
            };
            let parts = EventAttemptParts {
                run_id: run.id().clone(),
                expected_workflow_version: run.workflow_state_version(),
                expected_lifecycle_version: run.lifecycle_version(),
                source_state: run.current_state().clone(),
                target_state: Some(target),
                target_lifecycle: None,
                inline_evidence: vec![],
                associations: vec![],
                provider_evidence: vec![],
                journal_entry: journal_draft_with_reason(
                    &run,
                    OutcomeClass::Error,
                    Some(Reason::new(reason, "malformed provider result").unwrap()),
                    attempt.clone(),
                ),
                stale_journal_entry: journal_draft(&run, OutcomeClass::Error, attempt),
            };
            let status = writer
                .commit_parts(parts, None)
                .into_result()
                .unwrap_or_else(|error| {
                    panic!("expected {reason:?} attempt to remain journalable: {error}")
                });
            assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
            assert!(!status.commit.state_changed);
            assert_eq!(count_table(&conn, "journal_entries"), 1);
            assert_eq!(count_table(&conn, "evidence"), 0);
        }
    }

    #[test]
    fn traced_missing_run_emits_intent_version_check_and_rollback() {
        let (_guard, dir, _writer, reads) = store();
        let path = dir.path().join("state.db");
        let (trace_dir, _trace_writer, sink) = test_sink("run-request-missing-row");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let writer = SqliteEventAttemptWriter::with_trace(path, trace.clone());
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let parts = self_loop_parts(&run);
        conn.execute(
            "DELETE FROM run_journal_sequences WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM runs WHERE run_id = 'run-1'", [])
            .unwrap();
        let err = commit_traced(&writer, &trace, parts).unwrap_err();
        assert!(matches!(err, EventAttemptPersistenceError::NotFound));
        let events = read_events(&trace_dir.trace_dir().join("run-request-missing-row.jsonl"));
        assert_eq!(
            event_names(&events),
            vec!["intent", "version_check", "rollback"]
        );
        assert_eq!(events[1]["expected_workflow_version"], 1);
        assert_eq!(events[1]["expected_lifecycle_version"], 1);
        assert_eq!(events[2]["outcome"], "rejected");
    }

    #[test]
    fn traced_stale_version_race_emits_intent_version_check_and_rollback() {
        let (_guard, dir, _writer, reads) = store();
        let path = dir.path().join("state.db");
        let (trace_dir, _trace_writer, sink) = test_sink("run-request-stale-race");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let writer = SqliteEventAttemptWriter::with_trace(path, trace.clone());
        let conn = Connection::open(writer.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let run = reads.get(&RunId::parse("run-1").unwrap()).unwrap();
        let mut parts = self_loop_parts(&run);
        parts.expected_workflow_version = WorkflowStateVersion::try_from(99).unwrap();
        let inline = sample_evidence("stale-race-inline");
        parts.stale_journal_entry = journal_draft(
            &run,
            OutcomeClass::Error,
            transition_attempt(
                &run,
                &EventId::parse("checkpoint").unwrap(),
                run.current_state(),
                OutcomeClass::Error,
                false,
                vec![inline],
            ),
        );
        let err = commit_traced(&writer, &trace, parts).unwrap_err();
        assert!(matches!(
            err,
            EventAttemptPersistenceError::Validation { .. }
        ));
        let events = read_events(&trace_dir.trace_dir().join("run-request-stale-race.jsonl"));
        assert_eq!(
            event_names(&events),
            vec!["intent", "version_check", "rollback"]
        );
        assert_eq!(events[1]["expected_workflow_version"], 99);
        assert_eq!(events[2]["outcome"], "rejected");
        assert_eq!(count_table(&conn, "journal_entries"), 0);
    }
}
