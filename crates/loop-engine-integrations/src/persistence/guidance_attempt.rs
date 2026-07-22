//! Atomic per-run guidance attempt persistence (T114).
//!
//! Journal-only writes: authoritative `runs` columns and evidence inventory are never
//! mutated. Registration-wide `provider.check` does not use this writer.

use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::persistence_commands::{
    AppendGuidanceAttemptCommand, CommitStatus,
};
use loop_engine_core::model::attempt::{
    AttemptFacts, EvidenceAssociations, JournalExtension, ProviderFact, ProviderRole,
};
use loop_engine_core::model::bounded::{
    JOURNAL_ENTRY_ENCODED_BYTES, JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES,
    JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES, JOURNAL_PROVIDER_FACTS_ENCODED_BYTES, Value,
};
use loop_engine_core::model::compatibility::CompatibilityStatus;
use loop_engine_core::model::compatibility::{CompatibilityFinding, CompatibilityFindings};
use loop_engine_core::model::evidence::EvidenceRecord;
use loop_engine_core::model::ids::{RunId, StateId};
use loop_engine_core::model::journal::{
    JournalDraft, JournalEncodedSizes, JournalEntry, JournalEntryKind, JournalError, StateFact,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::reason::Reason;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, params};
use serde_json::{Value as JsonValue, json};
use thiserror::Error;

use super::error::CommitOutcomeError;
use super::records::JOURNAL_PAYLOAD_SCHEMA_VERSION;
use super::sqlite::commit::{
    EvidenceAssociationExpectation, JournalBundleExpectation, JournalRowExpectation,
    RunAuthoritativeExpectation, finish_committed_transaction,
};
use super::sqlite::connect_with_pragmas;
use super::traced::{
    MutationClass, OptionalTraceSink, SemanticOutcome, WriteExecution, WriteTraceSession,
    close_write, committed_or_unconfirmed, guidance_attempt_error_semantic,
    rollback_open_transaction,
};

/// SQLite-backed atomic guidance-attempt writer.
#[derive(Clone)]
pub struct GuidanceAttemptWriter {
    path: PathBuf,
    trace: OptionalTraceSink,
}

impl std::fmt::Debug for GuidanceAttemptWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuidanceAttemptWriter")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Extras encoded into compatibility-attempt payloads; guidance leaves these unset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JournalEncodeExtras {
    pub observed_executable_drift: Option<bool>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuidanceAttemptError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("journal command run_id does not match draft")]
    RunIdMismatch,
    #[error("journal finalize rejected draft: {0}")]
    JournalFinalize(JournalError),
    #[error("encoded journal entry exceeds bound ({actual} > {max})")]
    EntryTooLarge { actual: usize, max: usize },
    #[error("journal sequence allocator missing for run")]
    SequenceMissing,
    #[error("database constraint violation")]
    Constraint,
    #[error("persistence write failed: {detail}")]
    Persistence { detail: String },
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl CommitOutcomeError for GuidanceAttemptError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum AttemptWriteError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("journal command run_id does not match draft")]
    RunIdMismatch,
    #[error("journal finalize rejected draft: {0}")]
    JournalFinalize(JournalError),
    #[error("encoded journal entry exceeds bound ({actual} > {max})")]
    EntryTooLarge { actual: usize, max: usize },
    #[error("journal sequence allocator missing for run")]
    SequenceMissing,
    #[error("database constraint violation")]
    Constraint,
    #[error("persistence write failed: {detail}")]
    Persistence { detail: String },
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl From<AttemptWriteError> for GuidanceAttemptError {
    fn from(error: AttemptWriteError) -> Self {
        match error {
            AttemptWriteError::NotFound { run_id } => Self::NotFound { run_id },
            AttemptWriteError::RunIdMismatch => Self::RunIdMismatch,
            AttemptWriteError::JournalFinalize(error) => Self::JournalFinalize(error),
            AttemptWriteError::EntryTooLarge { actual, max } => Self::EntryTooLarge { actual, max },
            AttemptWriteError::SequenceMissing => Self::SequenceMissing,
            AttemptWriteError::Constraint => Self::Constraint,
            AttemptWriteError::Persistence { detail } => Self::Persistence { detail },
            AttemptWriteError::CommitOutcomeUnverified => Self::CommitOutcomeUnverified,
            AttemptWriteError::CommitIntegrityFailure => Self::CommitIntegrityFailure,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptPersistedJournal {
    pub sequence: u64,
    pub outcome: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthoritativeRunRow {
    pub run_id: RunId,
    pub current_state: StateId,
    pub lifecycle: Lifecycle,
    pub workflow_state_version: WorkflowStateVersion,
    pub lifecycle_version: LifecycleVersion,
    pub label: Option<String>,
    pub label_version: u64,
}

impl GuidanceAttemptWriter {
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

    /// Append one guidance attempt journal entry under `BEGIN IMMEDIATE`.
    pub fn append_guidance_attempt(
        &self,
        command: AppendGuidanceAttemptCommand,
    ) -> Result<CommitStatus, GuidanceAttemptError> {
        close_write(
            &self.trace,
            "run.guidance",
            MutationClass::RunMutation,
            |trace| {
                self.append_guidance_attempt_impl(command, trace)
                    .map_ok(|(status, outcome)| {
                        (status, SemanticOutcome::from_outcome_class(outcome))
                    })
            },
            |(_, semantic)| *semantic,
            guidance_attempt_error_semantic,
        )
        .map(|(status, _)| status)
    }

    fn append_guidance_attempt_impl(
        &self,
        command: AppendGuidanceAttemptCommand,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(CommitStatus, OutcomeClass), GuidanceAttemptError> {
        if let Err(error) = validate_guidance_command(&command) {
            return WriteExecution::no_transaction(error);
        }
        let expected_lifecycle = command.expected_lifecycle_version().value();
        let run_id = command.run_id().as_str();
        let conn = match connect_with_pragmas(self.path()).map_err(map_persistence) {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = conn
            .execute("BEGIN IMMEDIATE", [])
            .map_err(|error| GuidanceAttemptError::from(map_sqlite_persistence(error)))
        {
            return WriteExecution::no_transaction(error);
        }
        let result = (|| {
            let row = load_authoritative_run(&conn, command.run_id())?;
            if let Some(session) = trace {
                session.version_check_run_cas(run_id, None, Some(expected_lifecycle));
            }
            let draft = select_attempt_draft(
                row.lifecycle,
                command.journal_entry(),
                command.terminal_rejection_entry(),
            )
            .clone();
            let outcome = draft.outcome();
            let (status, journal, associations) = append_journal_attempt(
                &conn,
                command.run_id(),
                draft,
                JournalEncodeExtras::default(),
                &row,
            )?;
            let expectation =
                attempt_commit_expectation(command.run_id(), &row, &journal, associations);
            Ok(((status, outcome), expectation))
        })();
        commit_attempt_transaction(self.path(), conn, result).map_err(GuidanceAttemptError::from)
    }
}

pub(crate) fn select_attempt_draft<'a>(
    lifecycle: Lifecycle,
    journal_entry: &'a JournalDraft,
    terminal_rejection_entry: &'a JournalDraft,
) -> &'a JournalDraft {
    if lifecycle.is_terminal() {
        terminal_rejection_entry
    } else {
        journal_entry
    }
}

pub(crate) fn append_journal_attempt(
    conn: &Connection,
    run_id: &RunId,
    draft: JournalDraft,
    extras: JournalEncodeExtras,
    row: &AuthoritativeRunRow,
) -> Result<
    (
        CommitStatus,
        AttemptPersistedJournal,
        Vec<EvidenceAssociationExpectation>,
    ),
    AttemptWriteError,
> {
    if draft.run_id().as_str() != run_id.as_str() {
        return Err(AttemptWriteError::RunIdMismatch);
    }
    let sequence = allocate_sequence(conn, run_id)?;
    let state = state_fact(row);
    let (entry, payload, outcome) = prepare_journal_entry(draft, sequence, &state, extras)?;
    insert_journal_row(conn, run_id, &entry, &payload, &outcome)?;
    let associations = insert_selected_associations(conn, run_id, sequence, entry.attempt())?;
    let journal = AttemptPersistedJournal {
        sequence: entry.sequence().value(),
        outcome,
        payload,
    };
    Ok((
        CommitStatus {
            committed: true,
            state_changed: false,
            workflow_state_version: row.workflow_state_version,
            lifecycle_version: row.lifecycle_version,
        },
        journal,
        associations,
    ))
}

fn validate_guidance_command(
    command: &AppendGuidanceAttemptCommand,
) -> Result<(), GuidanceAttemptError> {
    validate_draft_run_id(command.run_id(), command.journal_entry())?;
    validate_draft_run_id(command.run_id(), command.terminal_rejection_entry())?;
    Ok(())
}

pub(crate) fn validate_draft_run_id(
    run_id: &RunId,
    draft: &JournalDraft,
) -> Result<(), AttemptWriteError> {
    if draft.run_id().as_str() == run_id.as_str() {
        Ok(())
    } else {
        Err(AttemptWriteError::RunIdMismatch)
    }
}

pub(crate) fn load_authoritative_run(
    conn: &Connection,
    run_id: &RunId,
) -> Result<AuthoritativeRunRow, AttemptWriteError> {
    conn.query_row(
        "SELECT r.run_id, r.current_state, r.lifecycle, r.workflow_state_version,
                r.lifecycle_version, r.label, r.label_version
         FROM runs r WHERE r.run_id = ?1",
        params![run_id.as_str()],
        |row| {
            Ok(AuthoritativeRunRow {
                run_id: RunId::parse(row.get::<_, String>(0)?).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        0,
                        "run_id".into(),
                        rusqlite::types::Type::Text,
                    )
                })?,
                current_state: StateId::parse(row.get::<_, String>(1)?).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "current_state".into(),
                        rusqlite::types::Type::Text,
                    )
                })?,
                lifecycle: parse_lifecycle(row.get::<_, String>(2)?)?,
                workflow_state_version: WorkflowStateVersion::try_from(read_u64_column(row, 3)?)
                    .map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "workflow_state_version".into(),
                            rusqlite::types::Type::Integer,
                        )
                    })?,
                lifecycle_version: LifecycleVersion::try_from(read_u64_column(row, 4)?).map_err(
                    |_| {
                        rusqlite::Error::InvalidColumnType(
                            4,
                            "lifecycle_version".into(),
                            rusqlite::types::Type::Integer,
                        )
                    },
                )?,
                label: row.get(5)?,
                label_version: read_u64_column(row, 6)?,
            })
        },
    )
    .optional()
    .map_err(map_sqlite_persistence)?
    .ok_or_else(|| AttemptWriteError::NotFound {
        run_id: run_id.clone(),
    })
}

fn read_u64_column(row: &rusqlite::Row<'_>, index: usize) -> Result<u64, rusqlite::Error> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::InvalidColumnType(
            index,
            "unsigned integer".into(),
            rusqlite::types::Type::Integer,
        )
    })
}

fn parse_lifecycle(value: String) -> Result<Lifecycle, rusqlite::Error> {
    match value.as_str() {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        _ => Err(rusqlite::Error::InvalidColumnType(
            2,
            "lifecycle".into(),
            rusqlite::types::Type::Text,
        )),
    }
}

pub(crate) fn state_fact(row: &AuthoritativeRunRow) -> StateFact {
    StateFact {
        state: row.current_state.clone(),
        lifecycle: row.lifecycle,
        workflow_state_version: row.workflow_state_version,
        lifecycle_version: row.lifecycle_version,
    }
}

fn allocate_sequence(
    conn: &Connection,
    run_id: &RunId,
) -> Result<JournalSequence, AttemptWriteError> {
    let next: i64 = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_persistence)?
        .ok_or(AttemptWriteError::SequenceMissing)?;
    let updated = conn
        .execute(
            "UPDATE run_journal_sequences SET next_sequence = next_sequence + 1 WHERE run_id = ?1 AND next_sequence = ?2",
            params![run_id.as_str(), next],
        )
        .map_err(map_sqlite_persistence)?;
    if updated != 1 {
        return Err(AttemptWriteError::Constraint);
    }
    let sequence = u64::try_from(next).map_err(|_| AttemptWriteError::Constraint)?;
    JournalSequence::try_from(sequence).map_err(|_| AttemptWriteError::Constraint)
}

fn initial_encoded_sizes(attempt: Option<&AttemptFacts>) -> JournalEncodedSizes {
    let attempt = attempt.cloned().unwrap_or_default();
    JournalEncodedSizes {
        entry: 512,
        evidence_associations: usize::from(attempt.evidence_associations.is_some()) * 64,
        provider_observations: if attempt.provider_observations.is_empty() {
            0
        } else {
            128
        },
        gate_verdict_facts: usize::from(attempt.gate_verdict_facts.is_some()) * 64,
        diagnostics: if attempt.diagnostics.is_empty() {
            0
        } else {
            64
        },
        note: attempt
            .note
            .as_ref()
            .map(|note| note.as_str().len().max(1))
            .unwrap_or(0),
        actor: usize::from(attempt.actor.is_some()) * 32,
    }
}

fn prepare_journal_entry(
    draft: JournalDraft,
    sequence: JournalSequence,
    state: &StateFact,
    extras: JournalEncodeExtras,
) -> Result<(JournalEntry, String, String), AttemptWriteError> {
    let preview = draft
        .clone()
        .finalize(
            sequence,
            state.clone(),
            state.clone(),
            initial_encoded_sizes(draft.attempt()),
        )
        .map_err(AttemptWriteError::JournalFinalize)?;
    let payload_value = build_journal_value_from_entry(&preview, extras);
    let sizes = measure_sizes(&payload_value, preview.attempt())?;
    let entry = draft
        .finalize(sequence, state.clone(), state.clone(), sizes)
        .map_err(AttemptWriteError::JournalFinalize)?;
    let payload_value = build_journal_value_from_entry(&entry, extras);
    let payload =
        serde_json::to_string(&payload_value).map_err(|error| AttemptWriteError::Persistence {
            detail: error.to_string(),
        })?;
    if payload.len() > JOURNAL_ENTRY_ENCODED_BYTES {
        return Err(AttemptWriteError::EntryTooLarge {
            actual: payload.len(),
            max: JOURNAL_ENTRY_ENCODED_BYTES,
        });
    }
    let outcome = outcome_wire(entry.outcome()).to_owned();
    Ok((entry, payload, outcome))
}

fn insert_journal_row(
    conn: &Connection,
    run_id: &RunId,
    entry: &JournalEntry,
    payload: &str,
    outcome: &str,
) -> Result<(), AttemptWriteError> {
    let sequence =
        i64::try_from(entry.sequence().value()).map_err(|_| AttemptWriteError::Constraint)?;
    conn.execute(
        "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![run_id.as_str(), sequence, outcome, payload],
    )
    .map_err(map_sqlite_write)?;
    Ok(())
}

fn insert_selected_associations(
    conn: &Connection,
    run_id: &RunId,
    sequence: JournalSequence,
    attempt: Option<&AttemptFacts>,
) -> Result<Vec<EvidenceAssociationExpectation>, AttemptWriteError> {
    let Some(associations) = attempt.and_then(|facts| facts.evidence_associations.as_ref()) else {
        return Ok(Vec::new());
    };
    let mut expected = Vec::with_capacity(associations.selected_ids.len());
    for evidence_id in &associations.selected_ids {
        let sequence_value =
            i64::try_from(sequence.value()).map_err(|_| AttemptWriteError::Constraint)?;
        conn.execute(
            "INSERT INTO evidence_associations (run_id, journal_sequence, evidence_id, event_id, gate_id)
             VALUES (?1, ?2, ?3, NULL, NULL)",
            params![run_id.as_str(), sequence_value, evidence_id.as_str()],
        )
        .map_err(map_sqlite_write)?;
        expected.push(EvidenceAssociationExpectation {
            run_id: run_id.as_str().to_owned(),
            journal_sequence: sequence.value(),
            evidence_id: evidence_id.as_str().to_owned(),
            event_id: None,
            gate_id: None,
        });
    }
    Ok(expected)
}

pub(crate) fn attempt_commit_expectation(
    run_id: &RunId,
    row: &AuthoritativeRunRow,
    journal: &AttemptPersistedJournal,
    associations: Vec<EvidenceAssociationExpectation>,
) -> JournalBundleExpectation {
    JournalBundleExpectation {
        run_changed: false,
        run: RunAuthoritativeExpectation {
            run_id: run_id.as_str().to_owned(),
            current_state: row.current_state.as_str().to_owned(),
            lifecycle: lifecycle_wire(row.lifecycle).to_owned(),
            workflow_state_version: row.workflow_state_version.value(),
            lifecycle_version: row.lifecycle_version.value(),
            label: row.label.clone(),
            label_version: row.label_version,
            next_sequence: journal.sequence + 1,
        },
        journal: JournalRowExpectation {
            run_id: run_id.as_str().to_owned(),
            sequence: journal.sequence,
            outcome: journal.outcome.clone(),
            payload: journal.payload.clone(),
        },
        evidence_ids: Vec::new(),
        associations,
    }
}

pub(crate) fn commit_attempt_transaction<T>(
    path: &Path,
    conn: Connection,
    result: Result<(T, JournalBundleExpectation), AttemptWriteError>,
) -> WriteExecution<T, AttemptWriteError> {
    match result {
        Ok((value, expectation)) => committed_or_unconfirmed(finish_committed_transaction(
            path,
            conn,
            value,
            |read| expectation.verify(read),
            map_sqlite_persistence,
            || AttemptWriteError::CommitOutcomeUnverified,
            || AttemptWriteError::CommitIntegrityFailure,
            |error| AttemptWriteError::Persistence {
                detail: error.to_string(),
            },
        )),
        Err(error) => rollback_open_transaction(&conn, error),
    }
}

fn build_journal_value_from_entry(entry: &JournalEntry, extras: JournalEncodeExtras) -> JsonValue {
    build_journal_value_from_parts(
        draft_parts_from_entry(entry),
        entry.sequence(),
        entry.state_before(),
        entry.state_after(),
        extras,
    )
}

struct DraftParts<'a> {
    run_id: &'a RunId,
    observed_at: ObservedAt,
    operation: &'a str,
    request_id: &'a loop_engine_core::model::ids::RequestId,
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    reason: Option<&'a Reason>,
    attempt: Option<&'a AttemptFacts>,
    extension: &'a JournalExtension,
}

fn draft_parts_from_entry<'a>(entry: &'a JournalEntry) -> DraftParts<'a> {
    DraftParts {
        run_id: entry.run_id(),
        observed_at: entry.observed_at(),
        operation: entry.operation(),
        request_id: entry.request_id(),
        kind: entry.kind(),
        outcome: entry.outcome(),
        reason: entry.reason(),
        attempt: entry.attempt(),
        extension: entry.extension(),
    }
}

fn build_journal_value_from_parts(
    draft: DraftParts<'_>,
    sequence: JournalSequence,
    state_before: &StateFact,
    state_after: &StateFact,
    extras: JournalEncodeExtras,
) -> JsonValue {
    let mut root = json!({
        "journal_schema_version": JOURNAL_PAYLOAD_SCHEMA_VERSION,
        "sequence": sequence.value(),
        "run_id": draft.run_id.as_str(),
        "ts": format_ts(draft.observed_at),
        "operation": draft.operation,
        "request_id": draft.request_id.as_str(),
        "entry_kind": entry_kind_wire(draft.kind),
        "outcome": outcome_wire(draft.outcome),
        "reason": encode_reason(draft.reason),
        "state_before": state_fact_json(state_before),
        "state_after": state_fact_json(state_after),
    });
    if let Some(attempt) = draft.attempt {
        merge_attempt_fields(&mut root, attempt);
    }
    merge_extension_fields(&mut root, draft.extension);
    if let Some(drift) = extras.observed_executable_drift {
        root["observed_executable_drift"] = json!(drift);
    }
    root
}

fn merge_attempt_fields(root: &mut JsonValue, attempt: &AttemptFacts) {
    if !attempt.provider_observations.is_empty() {
        root["provider_observations"] = json!(
            attempt
                .provider_observations
                .iter()
                .map(encode_provider_fact)
                .collect::<Vec<_>>()
        );
    }
    if let Some(associations) = &attempt.evidence_associations {
        root["evidence_associations"] = encode_evidence_associations(associations);
    }
    if let Some(recorded) = attempt.evidence_recorded {
        root["evidence_recorded"] = encode_evidence_recorded(recorded);
    }
    if let Some(note) = &attempt.note {
        root["note"] = json!(note.as_str());
    }
    if let Some(actor) = &attempt.actor {
        root["actor"] = core_value_to_json(actor.value());
    }
    if let Some(corrects) = attempt.corrects_sequence {
        root["corrects_sequence"] = json!(corrects.value());
    }
    if !attempt.diagnostics.is_empty() {
        root["diagnostics"] = json!(
            attempt
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    json!({
                        "code": diagnostic.code(),
                        "message": diagnostic.message(),
                        "path": diagnostic.path(),
                    })
                })
                .collect::<Vec<_>>()
        );
    }
}

fn merge_extension_fields(root: &mut JsonValue, extension: &JournalExtension) {
    match extension {
        JournalExtension::GuidanceAttempt {
            guidance_text: Some(text),
        } => {
            root["guidance_text"] = json!(text.as_str());
        }
        JournalExtension::CompatibilityAttempt {
            findings: Some(findings),
        } => {
            root["findings"] = encode_findings(findings);
        }
        _ => {}
    }
}

fn encode_findings(findings: &CompatibilityFindings) -> JsonValue {
    json!(
        findings
            .as_slice()
            .iter()
            .map(encode_finding)
            .collect::<Vec<_>>()
    )
}

fn encode_finding(finding: &CompatibilityFinding) -> JsonValue {
    let mut value = json!({
        "capability": finding.capability(),
        "status": compatibility_status_wire(finding.status()),
    });
    if let Some(message) = finding.diagnostics().first().map(|d| d.message()) {
        value["message"] = json!(message);
    }
    value
}

fn encode_evidence_associations(associations: &EvidenceAssociations) -> JsonValue {
    json!({
        "inline": associations.inline.iter().map(encode_inline_evidence).collect::<Vec<_>>(),
        "selected_ids": associations.selected_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
        "provider_recorded_ids": associations.provider_recorded_ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
    })
}

fn encode_inline_evidence(record: &EvidenceRecord) -> JsonValue {
    let mut value = json!({
        "evidence_id": record.id().as_str(),
        "kind": record.kind().as_str(),
        "locator": record.locator(),
    });
    if let Some(digest) = record.digest() {
        value["digest"] = json!(digest);
    }
    value
}

fn encode_evidence_recorded(recorded: EvidenceRecordedStatus) -> JsonValue {
    json!({
        "inline": recorded.inline,
        "selected_associations": recorded.selected_associations,
        "provider": recorded.provider,
    })
}

fn encode_provider_fact(fact: &ProviderFact) -> JsonValue {
    let mut value = json!({
        "registration_id": fact.registration_id.as_str(),
        "config_revision": fact.config_revision,
        "role": provider_role_wire(fact.role),
        "invocation_id": fact.invocation_id.as_str(),
        "executable": fact.executable.as_str(),
        "outcome": outcome_wire(fact.outcome),
    });
    match &fact.digest {
        DigestObservation::Observed(digest) => {
            value["executable_digest"] = json!(digest.as_str());
        }
        DigestObservation::Unavailable => {}
    }
    if let Some(version) = &fact.provider_version {
        value["provider_version"] = json!(version.as_str());
    }
    if let Some(major) = fact.protocol_major {
        value["protocol_major"] = json!(major);
    }
    value
}

fn encode_reason(reason: Option<&Reason>) -> JsonValue {
    match reason {
        None => JsonValue::Null,
        Some(reason) => json!({
            "code": reason.code().code(),
            "message": reason.message(),
        }),
    }
}

fn state_fact_json(fact: &StateFact) -> JsonValue {
    json!({
        "state": fact.state.as_str(),
        "lifecycle": lifecycle_wire(fact.lifecycle),
        "workflow_state_version": fact.workflow_state_version.value(),
        "lifecycle_version": fact.lifecycle_version.value(),
    })
}

fn format_ts(at: ObservedAt) -> String {
    at.as_timestamp()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
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

fn provider_role_wire(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

fn compatibility_status_wire(status: CompatibilityStatus) -> &'static str {
    match status {
        CompatibilityStatus::Compatible => "compatible",
        CompatibilityStatus::Incompatible => "incompatible",
        CompatibilityStatus::Unknown => "unknown",
    }
}

fn measure_sizes(
    value: &JsonValue,
    attempt: Option<&AttemptFacts>,
) -> Result<JournalEncodedSizes, AttemptWriteError> {
    let entry = serde_json::to_string(value)
        .map_err(|error| AttemptWriteError::Persistence {
            detail: error.to_string(),
        })?
        .len();
    let component_len = |key: &str| -> usize {
        value
            .get(key)
            .map(|part| {
                serde_json::to_string(part)
                    .map(|encoded| encoded.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    };
    let sizes = JournalEncodedSizes {
        entry,
        evidence_associations: component_len("evidence_associations"),
        provider_observations: component_len("provider_observations"),
        gate_verdict_facts: component_len("gate_verdict_facts"),
        diagnostics: component_len("diagnostics"),
        note: value
            .get("note")
            .and_then(|value| value.as_str())
            .map(str::len)
            .unwrap_or(0),
        actor: component_len("actor"),
    };
    if sizes.evidence_associations > JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES
        || sizes.provider_observations > JOURNAL_PROVIDER_FACTS_ENCODED_BYTES
        || sizes.gate_verdict_facts > JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES
    {
        return Err(AttemptWriteError::EntryTooLarge {
            actual: entry,
            max: JOURNAL_ENTRY_ENCODED_BYTES,
        });
    }
    if attempt.is_some() && sizes.entry == 0 {
        return Err(AttemptWriteError::EntryTooLarge {
            actual: 0,
            max: JOURNAL_ENTRY_ENCODED_BYTES,
        });
    }
    Ok(sizes)
}

fn core_value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(value) => json!(*value),
        Value::Number(value) => json!(value.value()),
        Value::String(value) => json!(value),
        Value::Array(values) => JsonValue::Array(values.iter().map(core_value_to_json).collect()),
        Value::Object(values) => {
            let mut object = serde_json::Map::new();
            for (key, value) in values {
                object.insert(key.clone(), core_value_to_json(value));
            }
            JsonValue::Object(object)
        }
    }
}

fn map_sqlite_persistence(error: SqliteError) -> AttemptWriteError {
    AttemptWriteError::Persistence {
        detail: error.to_string(),
    }
}

fn map_sqlite_write(error: SqliteError) -> AttemptWriteError {
    if matches!(error, SqliteError::SqliteFailure(_, _)) {
        AttemptWriteError::Constraint
    } else {
        AttemptWriteError::Persistence {
            detail: error.to_string(),
        }
    }
}

fn map_persistence(error: super::error::PersistenceError) -> GuidanceAttemptError {
    GuidanceAttemptError::Persistence {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::persistence_commands::AppendGuidanceAttemptCommand;
    use loop_engine_core::model::attempt::{
        AttemptFacts, JournalExtension, ProviderFact, ProviderRole,
    };
    use loop_engine_core::model::bounded::BoundedText;
    use loop_engine_core::model::ids::{RegistrationId, RequestId, RunId};
    use loop_engine_core::model::journal::JournalDraft;
    use loop_engine_core::model::outcome::OutcomeClass;
    use loop_engine_core::model::provider::DigestObservation;
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::model::time::ObservedAt;
    use loop_engine_core::model::version::LifecycleVersion;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::{GuidanceAttemptWriter, select_attempt_draft};
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::sqlite::open_at;

    const MINIMAL_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":true,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"none"}}],"transitions":[]}"#;

    fn graph_revision() -> String {
        use crate::persistence::mapping;
        use crate::persistence::records::RunRecord;
        let record = RunRecord {
            run_id: "019f0000-0000-7000-8000-000000000000".into(),
            registration_id: "019f0000-0000-7000-8000-000000000001".into(),
            config_revision_at_create: 1,
            current_state: "draft".into(),
            lifecycle: "active".into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: None,
            graph_revision:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            canonical_graph_version: 1,
            graph_canonical_projection_json: MINIMAL_GRAPH_JSON.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:00.000Z".into(),
        };
        match mapping::run_from_record(&record) {
            Err(crate::persistence::mapping::MappingError::GraphDigestMismatch {
                computed,
                ..
            }) => computed,
            other => panic!("unexpected mapping result: {other:?}"),
        }
    }

    fn seed_registration(conn: &Connection, registration_id: &str) {
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
             ) VALUES (?1, 'provider', 1, 1, '/bin/provider', '[]', '/work', 60,
                       '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            params![registration_id],
        )
        .unwrap();
    }

    fn seed_run(
        conn: &Connection,
        run_id: &str,
        registration_id: &str,
        lifecycle: &str,
        workflow_version: u64,
        lifecycle_version: u64,
    ) {
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
             ) VALUES (?1, ?2, 1, 'draft', ?3, ?4, ?5, 1, ?6, 1, ?7, '{}', '2026-07-17T12:00:01.000Z')",
            params![
                run_id,
                registration_id,
                lifecycle,
                i64::try_from(workflow_version).unwrap(),
                i64::try_from(lifecycle_version).unwrap(),
                graph_revision(),
                MINIMAL_GRAPH_JSON,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
            params![run_id],
        )
        .unwrap();
    }

    fn open_store() -> (TempDir, Connection, String) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        let store = open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        let registration_id = "019f0000-0000-7000-8000-000000000001";
        seed_registration(store.connection(), registration_id);
        (dir, store.into_connection(), registration_id.to_owned())
    }

    fn provider_fact(role: ProviderRole) -> ProviderFact {
        ProviderFact::new(
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
            1,
            role,
            RequestId::parse("019f0000-0000-7000-8000-000000000201").unwrap(),
            "/bin/provider",
            OutcomeClass::Completed,
            DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap(),
            Some("1.0.0".into()),
            Some(1),
        )
        .unwrap()
    }

    fn guidance_draft(
        run_id: &str,
        outcome: OutcomeClass,
        reason: Option<Reason>,
        extension: JournalExtension,
        attempt: Option<AttemptFacts>,
    ) -> JournalDraft {
        JournalDraft::new(
            RunId::parse(run_id).unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.guidance",
            RequestId::parse("019f0000-0000-7000-8000-000000000301").unwrap(),
            outcome,
            reason,
            attempt,
            extension,
        )
        .unwrap()
    }

    fn completed_guidance_command(run_id: &str) -> AppendGuidanceAttemptCommand {
        let text = BoundedText::non_empty("guidance_text", "Review rollback risks.").unwrap();
        let attempt = AttemptFacts {
            provider_observations: vec![provider_fact(ProviderRole::LiveGuidance)],
            ..AttemptFacts::default()
        };
        let journal_entry = guidance_draft(
            run_id,
            OutcomeClass::Completed,
            None,
            JournalExtension::GuidanceAttempt {
                guidance_text: Some(text),
            },
            Some(attempt.clone()),
        );
        let terminal_rejection_entry = guidance_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(attempt),
        );
        AppendGuidanceAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            LifecycleVersion::initial(),
            journal_entry,
            terminal_rejection_entry,
        )
    }

    fn read_run_versions(conn: &Connection, run_id: &str) -> (i64, i64, String) {
        conn.query_row(
            "SELECT workflow_state_version, lifecycle_version, lifecycle FROM runs WHERE run_id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn atomic_append_increments_sequence_without_state_mutation() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000101";
        seed_run(&conn, run_id, &registration_id, "active", 3, 2);
        let before = read_run_versions(&conn, run_id);
        let writer = GuidanceAttemptWriter::new(_dir.path().join("state.db"));
        let status = writer
            .append_guidance_attempt(completed_guidance_command(run_id))
            .unwrap();
        assert!(!status.state_changed);
        assert_eq!(status.workflow_state_version.value(), 3);
        assert_eq!(status.lifecycle_version.value(), 2);
        let after = read_run_versions(&conn, run_id);
        assert_eq!(before, after);
        let (sequence, outcome, payload): (i64, String, String) = conn
            .query_row(
                "SELECT sequence, outcome, encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(sequence, 2);
        assert_eq!(outcome, "completed");
        assert!(payload.contains("\"entry_kind\":\"guidance.attempt\""));
        assert!(payload.contains("Review rollback risks."));
        let next: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(next, 3);
    }

    #[test]
    fn persistence_failure_rolls_back_without_partial_journal() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000102";
        seed_run(&conn, run_id, &registration_id, "active", 1, 1);
        conn.execute_batch(&format!(
            "CREATE TRIGGER reject_guidance_insert
             BEFORE INSERT ON journal_entries
             WHEN NEW.run_id = '{run_id}'
             BEGIN
               SELECT RAISE(ABORT, 'injected failure');
             END"
        ))
        .unwrap();
        let writer = GuidanceAttemptWriter::new(_dir.path().join("state.db"));
        let error = writer
            .append_guidance_attempt(completed_guidance_command(run_id))
            .unwrap_err();
        assert!(matches!(
            error,
            super::GuidanceAttemptError::Constraint
                | super::GuidanceAttemptError::Persistence { .. }
        ));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let next: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(next, 2);
    }

    #[test]
    fn missing_provider_post_lookup_is_persisted_from_supplied_journal() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000103";
        seed_run(&conn, run_id, &registration_id, "active", 1, 1);
        let journal_entry = guidance_draft(
            run_id,
            OutcomeClass::Error,
            Some(
                Reason::new(
                    ReasonCode::ProviderRegistrationMissing,
                    "registration unavailable for invocation",
                )
                .unwrap(),
            ),
            JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(AttemptFacts::default()),
        );
        let terminal_rejection_entry = guidance_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(AttemptFacts::default()),
        );
        let command = AppendGuidanceAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            LifecycleVersion::initial(),
            journal_entry,
            terminal_rejection_entry,
        );
        GuidanceAttemptWriter::new(_dir.path().join("state.db"))
            .append_guidance_attempt(command)
            .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1 AND sequence = 2",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("provider.registration.missing"));
        assert!(!payload.contains("provider_observations"));
    }

    #[test]
    fn terminal_lifecycle_selects_terminal_rejection_entry() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000104";
        seed_run(&conn, run_id, &registration_id, "final", 1, 1);
        let command = completed_guidance_command(run_id);
        let selected = select_attempt_draft(
            loop_engine_core::model::lifecycle::Lifecycle::Final,
            command.journal_entry(),
            command.terminal_rejection_entry(),
        );
        assert_eq!(selected.outcome(), OutcomeClass::Rejected);
        GuidanceAttemptWriter::new(_dir.path().join("state.db"))
            .append_guidance_attempt(command)
            .unwrap();
        let (outcome, payload): (String, String) = conn
            .query_row(
                "SELECT outcome, encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(outcome, "rejected");
        assert!(payload.contains("run.lifecycle.terminal"));
        assert!(!payload.contains("Review rollback risks."));
    }

    #[test]
    fn unsupported_guidance_rejection_preserves_semantics_without_provider_facts() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000105";
        seed_run(&conn, run_id, &registration_id, "active", 1, 1);
        let journal_entry = guidance_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::GuidanceUnsupported, "stored unsupported").unwrap()),
            JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(AttemptFacts::default()),
        );
        let terminal_rejection_entry = journal_entry.clone();
        let command = AppendGuidanceAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            LifecycleVersion::initial(),
            journal_entry,
            terminal_rejection_entry,
        );
        GuidanceAttemptWriter::new(_dir.path().join("state.db"))
            .append_guidance_attempt(command)
            .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("guidance.unsupported"));
        assert!(!payload.contains("provider_observations"));
    }

    #[test]
    fn per_run_writer_is_not_registration_wide_provider_check() {
        // Registration-wide `provider.check --active-runs` never appends per-run journal rows
        // ([journal-contract.md] § Operation journal obligations). These writers accept only
        // single-run Append*AttemptCommand values scoped to one `run_id`.
        let command = completed_guidance_command("019f0000-0000-7000-8000-000000000101");
        assert_eq!(command.journal_entry().operation(), "run.guidance");
        assert_ne!(command.journal_entry().operation(), "provider.check");
    }
}
