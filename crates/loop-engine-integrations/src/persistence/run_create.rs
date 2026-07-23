//! Atomic SQLite run creation (T108).
//!
//! Rechecks enabled registration and expected `config_revision` inside one
//! `BEGIN IMMEDIATE` transaction, then commits run authority, immutable graph
//! snapshot/inputs, sequence allocator, and success-only creation journal
//! together.

use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::persistence_commands::{
    AppendAnnotationCommand, AppendCompatibilityAttemptCommand, AppendEvidenceCommand,
    AppendGuidanceAttemptCommand, CommitStatus, CreateRunCommand, ReplaceLabelCommand,
    TerminateCommit, TerminateRunCommand,
};
use loop_engine_core::capabilities::run_writer::RunWriter;
use loop_engine_core::model::attempt::{
    AttemptFacts, JournalExtension, ProviderFact, ProviderRole,
};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::journal::{
    JournalDraft, JournalEncodedSizes, JournalEntry, JournalEntryKind, JournalError, StateFact,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::RunInputs;
use loop_engine_core::model::version::JournalSequence;
use rusqlite::{Connection, Error as SqliteError, params};
use serde_json::{Value, json};
use thiserror::Error;

use crate::persistence::connect_with_pragmas;
use crate::persistence::error::{CommitOutcomeError, PersistenceError};
use crate::persistence::sqlite::commit::{
    JournalRowExpectation, RunCreateCommitExpectation, RunCreateRowExpectation,
    finish_committed_transaction,
};
use crate::persistence::traced::{
    MutationClass, OptionalTraceSink, SemanticOutcome, WriteExecution, WriteTraceSession,
    close_write, committed_or_unconfirmed, rollback_open_transaction, run_create_error_semantic,
};
use crate::provider_protocol::canonical::{graph_bytes, value_from_core};
use crate::sha256_digest::sha256_label;

/// SQLite run writer. T108 implements [`RunWriter::create`]; remaining mutation
/// methods return [`RunCreateError::UnsupportedOperation`] until later tasks
/// combine writers.
#[derive(Debug, Clone)]
pub struct SqliteRunWriter {
    path: PathBuf,
    trace: OptionalTraceSink,
}

/// Persistence failure surfaced to core orchestration for run creation.
#[derive(Debug, Error)]
pub enum RunCreateError {
    #[error("provider registration configuration is stale")]
    StaleProviderConfig,
    #[error("run writer operation is not implemented: {0}")]
    UnsupportedOperation(&'static str),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("journal preparation failed: {0}")]
    JournalPreparation(String),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("graph snapshot encoding failed: {0}")]
    GraphEncoding(String),
    #[error("run inputs encoding failed: {0}")]
    InputsEncoding(String),
    #[error(
        "stored graph revision does not match run authority: stored {stored}, computed {computed}"
    )]
    GraphRevisionMismatch { stored: String, computed: String },
    #[error("creation journal draft does not match run authority")]
    JournalAuthorityMismatch,
    #[error("sqlite write failed: {0}")]
    Sqlite(String),
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl CommitOutcomeError for RunCreateError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

impl SqliteRunWriter {
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

    fn connect(&self) -> Result<Connection, RunCreateError> {
        connect_with_pragmas(&self.path).map_err(RunCreateError::from)
    }

    fn create_impl(
        &self,
        command: CreateRunCommand,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<CommitStatus, RunCreateError> {
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = conn.execute("BEGIN IMMEDIATE", []).map_err(sqlite_err) {
            return WriteExecution::no_transaction(error);
        }
        let result = create_in_transaction(&conn, command, trace);
        match result {
            Ok((status, expectation)) => committed_or_unconfirmed(finish_committed_transaction(
                &self.path,
                conn,
                status,
                |read| expectation.verify(read),
                sqlite_err,
                || RunCreateError::CommitOutcomeUnverified,
                || RunCreateError::CommitIntegrityFailure,
                RunCreateError::from,
            )),
            Err(error) => rollback_open_transaction(&conn, error),
        }
    }
}

impl RunWriter for SqliteRunWriter {
    type Error = RunCreateError;

    fn create(&self, command: CreateRunCommand) -> Result<CommitStatus, Self::Error> {
        close_write(
            &self.trace,
            "run.create",
            MutationClass::RunCreate,
            |trace| self.create_impl(command, trace),
            |_| SemanticOutcome::Completed,
            run_create_error_semantic,
        )
    }

    fn append_evidence(
        &self,
        _command: AppendEvidenceCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunCreateError::UnsupportedOperation("append_evidence"))
    }

    fn append_annotation(
        &self,
        _command: AppendAnnotationCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunCreateError::UnsupportedOperation("append_annotation"))
    }

    fn replace_label(&self, _command: ReplaceLabelCommand) -> Result<CommitStatus, Self::Error> {
        Err(RunCreateError::UnsupportedOperation("replace_label"))
    }

    fn terminate(&self, _command: TerminateRunCommand) -> Result<TerminateCommit, Self::Error> {
        Err(RunCreateError::UnsupportedOperation("terminate"))
    }

    fn append_guidance_attempt(
        &self,
        _command: AppendGuidanceAttemptCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunCreateError::UnsupportedOperation(
            "append_guidance_attempt",
        ))
    }

    fn append_compatibility_attempt(
        &self,
        _command: AppendCompatibilityAttemptCommand,
    ) -> Result<CommitStatus, Self::Error> {
        Err(RunCreateError::UnsupportedOperation(
            "append_compatibility_attempt",
        ))
    }
}

fn create_in_transaction(
    conn: &Connection,
    command: CreateRunCommand,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CommitStatus, RunCreateCommitExpectation), RunCreateError> {
    validate_command(&command)?;
    let expected_config_revision = command.expected_config_revision();
    if let Some(session) = trace {
        session.version_check_catalog(expected_config_revision);
    }
    recheck_registration(
        conn,
        command.run().registration_id().as_str(),
        expected_config_revision,
    )?;

    let (run, expected_config_revision, creation_entry) = command.into_parts();
    let graph_json = stored_graph_json(&run)?;
    let inputs_json = stored_inputs_json(run.inputs())?;
    let state = state_fact_from_run(&run);
    let (journal_entry, journal_payload) = prepare_creation_journal(creation_entry, state)?;
    let created_at = format_observed_at(journal_entry.observed_at());
    insert_run(
        conn,
        &run,
        expected_config_revision,
        &graph_json,
        &inputs_json,
        &created_at,
    )?;

    insert_sequence(conn, run.id().as_str())?;

    insert_journal(conn, run.id().as_str(), &journal_entry, &journal_payload)?;

    let expectation = RunCreateCommitExpectation {
        run: RunCreateRowExpectation {
            run_id: run.id().as_str().to_owned(),
            registration_id: run.registration_id().as_str().to_owned(),
            config_revision_at_create: expected_config_revision,
            current_state: run.current_state().as_str().to_owned(),
            lifecycle: lifecycle_label(run.lifecycle()).to_owned(),
            workflow_state_version: run.workflow_state_version().value(),
            lifecycle_version: run.lifecycle_version().value(),
            label: run.label().map(str::to_owned),
            graph_revision: run.graph_revision().as_str().to_owned(),
            graph_json: graph_json.to_owned(),
            inputs_json: inputs_json.to_owned(),
        },
        next_sequence: 2,
        journal: JournalRowExpectation {
            run_id: run.id().as_str().to_owned(),
            sequence: journal_entry.sequence().value(),
            outcome: outcome_label(journal_entry.outcome()).to_owned(),
            payload: journal_payload.clone(),
        },
    };

    Ok((
        CommitStatus {
            committed: true,
            state_changed: false,
            workflow_state_version: run.workflow_state_version(),
            lifecycle_version: run.lifecycle_version(),
        },
        expectation,
    ))
}

fn validate_command(command: &CreateRunCommand) -> Result<(), RunCreateError> {
    if command.creation_entry().run_id() != command.run().id() {
        return Err(RunCreateError::JournalAuthorityMismatch);
    }
    if command.creation_entry().kind() != JournalEntryKind::RunCreated {
        return Err(RunCreateError::JournalAuthorityMismatch);
    }
    if command.creation_entry().outcome() != OutcomeClass::Completed {
        return Err(RunCreateError::JournalAuthorityMismatch);
    }
    if !matches!(
        command.creation_entry().extension(),
        JournalExtension::RunCreated { graph_revision } if graph_revision == command.run().graph_revision()
    ) {
        return Err(RunCreateError::JournalAuthorityMismatch);
    }
    Ok(())
}

fn recheck_registration(
    conn: &Connection,
    registration_id: &str,
    expected_config_revision: u64,
) -> Result<(), RunCreateError> {
    let row: Result<(i64, i64), SqliteError> = conn.query_row(
        "SELECT enabled, config_revision
         FROM provider_registrations
         WHERE registration_id = ?1",
        params![registration_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match row {
        Ok((enabled, revision)) if enabled == 1 && revision as u64 == expected_config_revision => {
            Ok(())
        }
        Ok(_) | Err(SqliteError::QueryReturnedNoRows) => Err(RunCreateError::StaleProviderConfig),
        Err(source) => Err(RunCreateError::Sqlite(source.to_string())),
    }
}

fn state_fact_from_run(run: &Run) -> StateFact {
    StateFact {
        state: run.current_state().clone(),
        lifecycle: run.lifecycle(),
        workflow_state_version: run.workflow_state_version(),
        lifecycle_version: run.lifecycle_version(),
    }
}

fn stored_graph_json(run: &Run) -> Result<String, RunCreateError> {
    let validated = ValidatedGraph::validate(run.graph().clone())
        .map_err(|error| RunCreateError::GraphEncoding(error.to_string()))?;
    let projection = SemanticGraphProjection::from_validated(&validated);
    let bytes = graph_bytes(&projection)
        .map_err(|error| RunCreateError::GraphEncoding(error.to_string()))?;
    let computed = sha256_label(&bytes);
    if computed != run.graph_revision().as_str() {
        return Err(RunCreateError::GraphRevisionMismatch {
            stored: run.graph_revision().as_str().to_owned(),
            computed,
        });
    }
    String::from_utf8(bytes).map_err(|error| RunCreateError::GraphEncoding(error.to_string()))
}

fn stored_inputs_json(inputs: &RunInputs) -> Result<String, RunCreateError> {
    let object = inputs
        .values()
        .iter()
        .map(|(name, value)| Ok((name.as_str().to_owned(), value_from_core(value))))
        .collect::<Result<serde_json::Map<_, _>, RunCreateError>>()?;
    serde_json::to_string(&Value::Object(object))
        .map_err(|error| RunCreateError::InputsEncoding(error.to_string()))
}

fn prepare_creation_journal(
    draft: JournalDraft,
    state: StateFact,
) -> Result<(JournalEntry, String), RunCreateError> {
    use loop_engine_core::model::bounded::JOURNAL_ENTRY_ENCODED_BYTES;

    let sequence = JournalSequence::first();
    let mut bootstrap_sizes = component_sizes_from_attempt(draft.attempt())?;
    bootstrap_sizes.entry = JOURNAL_ENTRY_ENCODED_BYTES;
    let bootstrap =
        draft
            .clone()
            .finalize(sequence, state.clone(), state.clone(), bootstrap_sizes)?;
    let wire = encode_journal_wire(&bootstrap)?;
    if wire.len() > JOURNAL_ENTRY_ENCODED_BYTES {
        return Err(RunCreateError::JournalPreparation(format!(
            "encoded journal exceeds aggregate bound: {} > {}",
            wire.len(),
            JOURNAL_ENTRY_ENCODED_BYTES
        )));
    }
    let encoded_sizes = encoded_sizes_from_wire(&wire, draft.attempt())?;
    let entry = draft.finalize(sequence, state.clone(), state.clone(), encoded_sizes)?;
    let final_wire = encode_journal_wire(&entry)?;
    if final_wire != wire {
        return Err(RunCreateError::JournalPreparation(
            "journal wire changed across size finalization".into(),
        ));
    }
    Ok((entry, final_wire))
}

fn insert_run(
    conn: &Connection,
    run: &Run,
    config_revision_at_create: u64,
    graph_json: &str,
    inputs_json: &str,
    created_at: &str,
) -> Result<(), RunCreateError> {
    conn.execute(
        "INSERT INTO runs (
            run_id, registration_id, config_revision_at_create, current_state, lifecycle,
            workflow_state_version, lifecycle_version, label_version, label, graph_revision,
            canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, 1, ?10, ?11, ?12)",
        params![
            run.id().as_str(),
            run.registration_id().as_str(),
            i64::try_from(config_revision_at_create)
                .map_err(|_| RunCreateError::Sqlite("config revision overflow".into()))?,
            run.current_state().as_str(),
            lifecycle_label(run.lifecycle()),
            i64::try_from(run.workflow_state_version().value())
                .map_err(|_| RunCreateError::Sqlite("workflow version overflow".into()))?,
            i64::try_from(run.lifecycle_version().value())
                .map_err(|_| RunCreateError::Sqlite("lifecycle version overflow".into()))?,
            run.label(),
            run.graph_revision().as_str(),
            graph_json,
            inputs_json,
            created_at,
        ],
    )
    .map_err(sqlite_err)?;
    Ok(())
}

fn insert_sequence(conn: &Connection, run_id: &str) -> Result<(), RunCreateError> {
    conn.execute(
        "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
        params![run_id],
    )
    .map_err(sqlite_err)?;
    Ok(())
}

fn insert_journal(
    conn: &Connection,
    run_id: &str,
    entry: &JournalEntry,
    payload: &str,
) -> Result<(), RunCreateError> {
    conn.execute(
        "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            run_id,
            i64::try_from(entry.sequence().value())
                .map_err(|_| RunCreateError::Sqlite("journal sequence overflow".into()))?,
            outcome_label(entry.outcome()),
            payload,
        ],
    )
    .map_err(sqlite_err)?;
    Ok(())
}

fn component_sizes_from_attempt(
    attempt: Option<&AttemptFacts>,
) -> Result<JournalEncodedSizes, RunCreateError> {
    let mut sizes = JournalEncodedSizes::default();
    let Some(attempt) = attempt else {
        return Ok(sizes);
    };
    if !attempt.provider_observations.is_empty() {
        sizes.provider_observations = json_encoded_len(&encode_provider_observations(
            &attempt.provider_observations,
        )?)?;
    }
    if let Some(associations) = &attempt.evidence_associations {
        sizes.evidence_associations =
            json_encoded_len(&encode_evidence_associations(associations)?)?;
    }
    if let Some(facts) = &attempt.gate_verdict_facts {
        sizes.gate_verdict_facts = json_encoded_len(&encode_gate_verdict_facts(facts)?)?;
    }
    if !attempt.diagnostics.is_empty() {
        sizes.diagnostics = json_encoded_len(&encode_diagnostics(&attempt.diagnostics)?)?;
    }
    if let Some(note) = &attempt.note {
        sizes.note = note.as_str().len();
    }
    if let Some(actor) = &attempt.actor {
        sizes.actor = json_encoded_len(&encode_actor(actor)?)?;
    }
    Ok(sizes)
}

fn encoded_sizes_from_wire(
    wire: &str,
    attempt: Option<&AttemptFacts>,
) -> Result<JournalEncodedSizes, RunCreateError> {
    let payload: Value = serde_json::from_str(wire)
        .map_err(|error| RunCreateError::JournalPreparation(error.to_string()))?;
    let sizes = JournalEncodedSizes {
        entry: wire.len(),
        evidence_associations: optional_json_len(payload.get("evidence_associations"))?,
        provider_observations: optional_json_len(payload.get("provider_observations"))?,
        gate_verdict_facts: optional_json_len(payload.get("gate_verdict_facts"))?,
        diagnostics: optional_json_len(payload.get("diagnostics"))?,
        note: payload
            .get("note")
            .and_then(Value::as_str)
            .map(str::len)
            .unwrap_or(0),
        actor: optional_json_len(payload.get("actor"))?,
    };
    if attempt.is_some() && sizes.entry == 0 {
        return Err(RunCreateError::JournalPreparation(
            "encoded journal entry must be non-empty".into(),
        ));
    }
    Ok(sizes)
}

fn encode_journal_wire(entry: &JournalEntry) -> Result<String, RunCreateError> {
    let mut payload = json!({
        "journal_schema_version": 1,
        "sequence": entry.sequence().value(),
        "run_id": entry.run_id().as_str(),
        "ts": format_observed_at(entry.observed_at()),
        "operation": entry.operation(),
        "request_id": entry.request_id().as_str(),
        "entry_kind": entry_kind_label(entry.kind()),
        "outcome": outcome_label(entry.outcome()),
        "reason": encode_reason(entry.reason())?,
        "state_before": state_fact_json(entry.state_before()),
        "state_after": state_fact_json(entry.state_after()),
    });
    if let Some(attempt) = entry.attempt() {
        if !attempt.provider_observations.is_empty() {
            payload["provider_observations"] =
                encode_provider_observations(&attempt.provider_observations)?;
        }
        if let Some(associations) = &attempt.evidence_associations {
            payload["evidence_associations"] = encode_evidence_associations(associations)?;
        }
        if let Some(facts) = &attempt.gate_verdict_facts {
            payload["gate_verdict_facts"] = encode_gate_verdict_facts(facts)?;
        }
        if let Some(recorded) = attempt.evidence_recorded {
            payload["evidence_recorded"] = json!({
                "inline": recorded.inline,
                "selected_associations": recorded.selected_associations,
                "provider": recorded.provider,
            });
        }
        if let Some(note) = &attempt.note {
            payload["note"] = Value::String(note.as_str().to_owned());
        }
        if let Some(actor) = &attempt.actor {
            payload["actor"] = encode_actor(actor)?;
        }
        if let Some(sequence) = attempt.corrects_sequence {
            payload["corrects_sequence"] = Value::Number(sequence.value().into());
        }
        if !attempt.diagnostics.is_empty() {
            payload["diagnostics"] = encode_diagnostics(&attempt.diagnostics)?;
        }
    }
    match entry.extension() {
        JournalExtension::RunCreated { graph_revision } => {
            payload["graph_revision"] = Value::String(graph_revision.as_str().to_owned());
        }
        JournalExtension::EvidenceAdded { added } => {
            if let Some(added) = added {
                payload["evidence_id"] = Value::String(added.evidence_id.as_str().to_owned());
                payload["kind"] = Value::String(added.kind.as_str().to_owned());
                payload["locator"] = Value::String(added.locator.as_str().to_owned());
                if let Some(digest) = &added.digest {
                    payload["digest"] = Value::String(digest.as_str().to_owned());
                }
            }
        }
        JournalExtension::LabelChanged { change } => {
            if let Some(change) = change {
                payload["label_before"] = optional_label_json(&change.label_before);
                payload["label_after"] = optional_label_json(&change.label_after);
            }
        }
        JournalExtension::GuidanceAttempt { guidance_text } => {
            if let Some(text) = guidance_text {
                payload["guidance_text"] = Value::String(text.as_str().to_owned());
            }
        }
        JournalExtension::CompatibilityAttempt { findings } => {
            if let Some(findings) = findings {
                payload["findings"] = encode_compatibility_findings(findings)?;
            }
        }
        JournalExtension::Annotation
        | JournalExtension::TransitionAttempt
        | JournalExtension::RunTerminated => {}
    }
    serde_json::to_string(&payload)
        .map_err(|error| RunCreateError::JournalPreparation(error.to_string()))
}

fn json_encoded_len(value: &Value) -> Result<usize, RunCreateError> {
    serde_json::to_string(value)
        .map(|encoded| encoded.len())
        .map_err(|error| RunCreateError::JournalPreparation(error.to_string()))
}

fn optional_json_len(value: Option<&Value>) -> Result<usize, RunCreateError> {
    match value {
        None => Ok(0),
        Some(value) => json_encoded_len(value),
    }
}

fn encode_reason(
    reason: Option<&loop_engine_core::model::reason::Reason>,
) -> Result<Value, RunCreateError> {
    match reason {
        None => Ok(Value::Null),
        Some(reason) => Ok(json!({
            "code": reason.code().code(),
            "message": reason.message(),
        })),
    }
}

fn state_fact_json(state: &StateFact) -> Value {
    json!({
        "state": state.state.as_str(),
        "lifecycle": lifecycle_label(state.lifecycle),
        "workflow_state_version": state.workflow_state_version.value(),
        "lifecycle_version": state.lifecycle_version.value(),
    })
}

fn encode_provider_observations(facts: &[ProviderFact]) -> Result<Value, RunCreateError> {
    let values = facts
        .iter()
        .map(encode_provider_fact)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(values))
}

fn encode_provider_fact(fact: &ProviderFact) -> Result<Value, RunCreateError> {
    let mut value = json!({
        "registration_id": fact.registration_id.as_str(),
        "config_revision": fact.config_revision,
        "role": provider_role_label(fact.role),
        "invocation_id": fact.invocation_id.as_str(),
        "executable": fact.executable.as_str(),
        "outcome": outcome_label(fact.outcome),
    });
    if let DigestObservation::Observed(digest) = &fact.digest {
        value["executable_digest"] = Value::String(digest.as_str().to_owned());
    }
    if let Some(version) = &fact.provider_version {
        value["provider_version"] = Value::String(version.as_str().to_owned());
    }
    if let Some(protocol_major) = fact.protocol_major {
        value["protocol_major"] = Value::Number(protocol_major.into());
    }
    Ok(value)
}

fn encode_evidence_associations(
    associations: &loop_engine_core::model::attempt::EvidenceAssociations,
) -> Result<Value, RunCreateError> {
    let mut value = json!({});
    if !associations.inline.is_empty() {
        value["inline"] = Value::Array(
            associations
                .inline
                .iter()
                .map(|record| {
                    Ok(json!({
                        "evidence_id": record.id().as_str(),
                        "kind": record.kind().as_str(),
                        "locator": record.locator(),
                    }))
                })
                .collect::<Result<_, RunCreateError>>()?,
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
    Ok(value)
}

fn encode_gate_verdict_facts(
    facts: &loop_engine_core::model::attempt::GateVerdictFacts,
) -> Result<Value, RunCreateError> {
    let mut value = json!({
        "event": facts.event.as_str(),
        "gate_ids": facts
            .gate_ids
            .iter()
            .map(|gate| gate.as_str().to_owned())
            .collect::<Vec<_>>(),
    });
    match &facts.result {
        loop_engine_core::model::attempt::GateVerdictResult::Verdicts(verdicts) => {
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
        loop_engine_core::model::attempt::GateVerdictResult::Incompatibility(diagnostic) => {
            value["incompatibility"] = json!({
                "code": diagnostic.code(),
                "message": diagnostic.message(),
            });
        }
        loop_engine_core::model::attempt::GateVerdictResult::EvaluationError(diagnostics) => {
            value["evaluation_error"] = encode_diagnostics(diagnostics.as_slice())?;
        }
    }
    Ok(value)
}

fn encode_diagnostics(
    diagnostics: &[loop_engine_core::model::diagnostic::Diagnostic],
) -> Result<Value, RunCreateError> {
    Ok(Value::Array(
        diagnostics
            .iter()
            .map(|diagnostic| {
                let mut value = json!({
                    "code": diagnostic.code(),
                    "message": diagnostic.message(),
                });
                if let Some(path) = diagnostic.path() {
                    value["path"] = Value::String(path.to_owned());
                }
                value
            })
            .collect(),
    ))
}

fn encode_actor(
    actor: &loop_engine_core::model::annotation::ActorMetadata,
) -> Result<Value, RunCreateError> {
    Ok(value_from_core(actor.value()))
}

fn encode_compatibility_findings(
    findings: &loop_engine_core::model::compatibility::CompatibilityFindings,
) -> Result<Value, RunCreateError> {
    Ok(Value::Array(
        findings
            .as_slice()
            .iter()
            .map(|finding| {
                let status = match finding.status() {
                    loop_engine_core::model::compatibility::CompatibilityStatus::Compatible => {
                        "compatible"
                    }
                    loop_engine_core::model::compatibility::CompatibilityStatus::Incompatible => {
                        "incompatible"
                    }
                    loop_engine_core::model::compatibility::CompatibilityStatus::Unknown => {
                        "unknown"
                    }
                };
                let mut row = json!({
                    "capability": finding.capability(),
                    "status": status,
                });
                if let Some(diagnostic) = finding.diagnostics().first() {
                    row["message"] = Value::String(diagnostic.message().to_owned());
                }
                row
            })
            .collect(),
    ))
}

fn optional_label_json(
    label: &Option<
        loop_engine_core::model::bounded::BoundedText<
            { loop_engine_core::model::bounded::RUN_LABEL_UTF8_BYTES },
        >,
    >,
) -> Value {
    match label {
        None => Value::Null,
        Some(label) => Value::String(label.as_str().to_owned()),
    }
}

fn lifecycle_label(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

fn outcome_label(outcome: OutcomeClass) -> &'static str {
    match outcome {
        OutcomeClass::Completed => "completed",
        OutcomeClass::Rejected => "rejected",
        OutcomeClass::Error => "error",
    }
}

fn entry_kind_label(kind: JournalEntryKind) -> &'static str {
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

fn provider_role_label(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

fn format_observed_at(at: loop_engine_core::model::time::ObservedAt) -> String {
    at.as_timestamp()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn sqlite_err(source: SqliteError) -> RunCreateError {
    RunCreateError::Sqlite(source.to_string())
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::provider_catalog::{
        CatalogMutation, ProviderCatalog, ProviderConfig,
    };
    use loop_engine_core::capabilities::run_writer::RunWriter;
    use loop_engine_core::model::attempt::{
        AttemptFacts, JournalExtension, ProviderFact, ProviderRole,
    };
    use loop_engine_core::model::graph::{State, WorkflowGraph};
    use loop_engine_core::model::graph_projection::SemanticGraphProjection;
    use loop_engine_core::model::graph_validation::ValidatedGraph;
    use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use loop_engine_core::model::ids::{
        GraphRevision, ProviderHandle, RegistrationId, RequestId, RunId, StateId,
    };
    use loop_engine_core::model::journal::JournalDraft;
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::model::outcome::OutcomeClass;
    use loop_engine_core::model::provider::DigestObservation;
    use loop_engine_core::model::run::Run;
    use loop_engine_core::model::run_input::InputDeclarations;
    use loop_engine_core::model::time::ObservedAt;
    use rusqlite::{Connection, params};
    use rusqlite_migration::{M, Migrations};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::TempDir;

    use super::{RunCreateError, SqliteRunWriter};
    use crate::persistence::provider_catalog::SqliteProviderCatalog;
    use crate::persistence::records::GV01_CANONICAL_GRAPH_JSON;
    use crate::persistence::sqlite::open_at;
    use crate::provider_protocol::canonical::graph_bytes;
    use crate::sha256_digest::sha256_label;

    fn test_writer() -> (
        MutexGuard<'static, ()>,
        TempDir,
        SqliteRunWriter,
        RegistrationId,
    ) {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        let migrations = Migrations::new(vec![M::up(include_str!(
            "../../migrations/0001_initial.sql"
        ))]);
        open_at(&path, &migrations, 1).unwrap();
        let registration_id =
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap();
        let catalog = SqliteProviderCatalog::new(path.clone());
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("provider-a").unwrap(),
                config: ProviderConfig::new("/bin/provider", vec![], "/work", 60).unwrap(),
            })
            .unwrap();
        (guard, dir, SqliteRunWriter::new(path), registration_id)
    }

    fn gv01_graph() -> ValidatedGraph {
        let state = State::new(
            StateId::parse("draft").unwrap(),
            false,
            StaticGuidance::Text(
                loop_engine_core::model::bounded::BoundedText::non_empty(
                    "static_guidance",
                    "Prepare the change.",
                )
                .unwrap(),
            ),
            None,
        );
        ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
            StateId::parse("draft").unwrap(),
            vec![state],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        ))
        .unwrap()
    }

    fn initial_final_graph() -> ValidatedGraph {
        let state = State::new(
            StateId::parse("done").unwrap(),
            true,
            StaticGuidance::NoneRequired,
            None,
        );
        ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
            StateId::parse("done").unwrap(),
            vec![state],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        ))
        .unwrap()
    }

    fn graph_revision(graph: &ValidatedGraph) -> GraphRevision {
        let projection = SemanticGraphProjection::from_validated(graph);
        GraphRevision::parse(sha256_label(
            &graph_bytes(&projection).expect("graph bytes"),
        ))
        .unwrap()
    }

    fn describe_fact(registration_id: &RegistrationId, config_revision: u64) -> ProviderFact {
        ProviderFact::new(
            registration_id.clone(),
            config_revision,
            ProviderRole::Describe,
            RequestId::parse("pv-describe-001").unwrap(),
            "/bin/provider",
            OutcomeClass::Completed,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap()
    }

    fn creation_command(
        run_id: &str,
        registration_id: &RegistrationId,
        graph: ValidatedGraph,
        revision: GraphRevision,
        config_revision: u64,
    ) -> loop_engine_core::capabilities::persistence_commands::CreateRunCommand {
        let run = Run::create(
            RunId::parse(run_id).unwrap(),
            registration_id.clone(),
            graph,
            revision.clone(),
            Default::default(),
            None,
        )
        .unwrap();
        let draft = JournalDraft::new(
            run.id().clone(),
            ObservedAt::parse("2026-07-17T14:00:00.123Z").unwrap(),
            "run.create",
            RequestId::parse("01J9X3K2M4N5P6Q7R8S9T0V1W").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(AttemptFacts {
                provider_observations: vec![describe_fact(registration_id, config_revision)],
                ..AttemptFacts::default()
            }),
            JournalExtension::RunCreated {
                graph_revision: revision,
            },
        )
        .unwrap();
        loop_engine_core::capabilities::persistence_commands::CreateRunCommand::for_test(
            run,
            config_revision,
            draft,
        )
    }

    fn persisted_counts(conn: &Connection, run_id: &str) -> (i64, i64, i64) {
        let runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        let journal: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        let sequences: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_journal_sequences WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        (runs, journal, sequences)
    }

    #[test]
    fn create_commits_run_sequence_and_creation_journal() {
        let (_guard, _dir, writer, registration_id) = test_writer();
        let graph = gv01_graph();
        let revision = graph_revision(&graph);
        let command = creation_command(
            "019f0000-0000-7000-8000-000000000101",
            &registration_id,
            graph,
            revision.clone(),
            1,
        );
        let status = writer.create(command).unwrap();
        assert!(status.committed);
        assert!(!status.state_changed);
        assert_eq!(status.workflow_state_version.value(), 1);
        assert_eq!(status.lifecycle_version.value(), 1);

        let conn = Connection::open(writer.path()).unwrap();
        let (runs, journal, sequences) =
            persisted_counts(&conn, "019f0000-0000-7000-8000-000000000101");
        assert_eq!((runs, journal, sequences), (1, 1, 1));
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = ?1",
                params!["019f0000-0000-7000-8000-000000000101"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "active");
        let graph_json: String = conn
            .query_row(
                "SELECT graph_canonical_projection_json FROM runs WHERE run_id = ?1",
                params!["019f0000-0000-7000-8000-000000000101"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(graph_json, GV01_CANONICAL_GRAPH_JSON);
        let stored_revision: String = conn
            .query_row(
                "SELECT graph_revision FROM runs WHERE run_id = ?1",
                params!["019f0000-0000-7000-8000-000000000101"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_revision, revision.as_str());
        let next_sequence: i64 = conn
            .query_row(
                "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
                params!["019f0000-0000-7000-8000-000000000101"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(next_sequence, 2);
    }

    #[test]
    fn create_supports_initial_final_lifecycle() {
        let (_guard, _dir, writer, registration_id) = test_writer();
        let graph = initial_final_graph();
        let revision = graph_revision(&graph);
        let command = creation_command(
            "019f0000-0000-7000-8000-000000000102",
            &registration_id,
            graph,
            revision,
            1,
        );
        assert_eq!(command.run().lifecycle(), Lifecycle::Final);
        writer.create(command).unwrap();
        let conn = Connection::open(writer.path()).unwrap();
        let lifecycle: String = conn
            .query_row(
                "SELECT lifecycle FROM runs WHERE run_id = ?1",
                params!["019f0000-0000-7000-8000-000000000102"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lifecycle, "final");
    }

    #[test]
    fn stale_config_revision_update_disable_restore_write_nothing() {
        let (_guard, _dir, writer, registration_id) = test_writer();
        let graph = gv01_graph();
        let revision = graph_revision(&graph);
        let run_id = "019f0000-0000-7000-8000-000000000103";

        let mut command =
            creation_command(run_id, &registration_id, graph.clone(), revision.clone(), 1);
        let conn = Connection::open(writer.path()).unwrap();
        conn.execute(
            "UPDATE provider_registrations SET config_revision = 2, updated_at = '2026-07-17T13:00:00.000Z' WHERE registration_id = ?1",
            params![registration_id.as_str()],
        )
        .unwrap();
        assert!(matches!(
            writer.create(command.clone()),
            Err(RunCreateError::StaleProviderConfig)
        ));
        assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));

        conn.execute(
            "UPDATE provider_registrations SET config_revision = 1, updated_at = '2026-07-17T13:00:00.000Z' WHERE registration_id = ?1",
            params![registration_id.as_str()],
        )
        .unwrap();
        conn.execute(
            "UPDATE provider_registrations SET enabled = 0, handle = NULL, config_revision = 2, updated_at = '2026-07-17T13:00:00.000Z' WHERE registration_id = ?1",
            params![registration_id.as_str()],
        )
        .unwrap();
        command = command.with_expected_config_revision(1);
        assert!(matches!(
            writer.create(command.clone()),
            Err(RunCreateError::StaleProviderConfig)
        ));
        assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));

        conn.execute(
            "UPDATE provider_registrations SET enabled = 1, handle = 'provider-a', config_revision = 3, updated_at = '2026-07-17T13:00:00.000Z' WHERE registration_id = ?1",
            params![registration_id.as_str()],
        )
        .unwrap();
        command = command.with_expected_config_revision(2);
        assert!(matches!(
            writer.create(command),
            Err(RunCreateError::StaleProviderConfig)
        ));
        assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
    }

    #[test]
    fn missing_registration_is_stale_without_writes() {
        let (_guard, _dir, writer, registration_id) = test_writer();
        let graph = gv01_graph();
        let revision = graph_revision(&graph);
        let command = creation_command(
            "019f0000-0000-7000-8000-000000000104",
            &registration_id,
            graph,
            revision,
            1,
        );
        let conn = Connection::open(writer.path()).unwrap();
        conn.execute(
            "DELETE FROM provider_registrations WHERE registration_id = ?1",
            params![registration_id.as_str()],
        )
        .unwrap();
        assert!(matches!(
            writer.create(command),
            Err(RunCreateError::StaleProviderConfig)
        ));
        assert_eq!(
            persisted_counts(&conn, "019f0000-0000-7000-8000-000000000104"),
            (0, 0, 0)
        );
    }

    fn install_abort_trigger(conn: &Connection, trigger_name: &str, table: &str, run_id: &str) {
        conn.execute_batch(&format!(
            "CREATE TRIGGER {trigger_name}
             BEFORE INSERT ON {table}
             WHEN NEW.run_id = '{run_id}'
             BEGIN
               SELECT RAISE(ABORT, 'injected failure');
             END"
        ))
        .unwrap();
    }

    #[test]
    fn sqlite_abort_triggers_roll_back_run_creation_atomically() {
        let (_guard, _dir, writer, registration_id) = test_writer();
        let graph = gv01_graph();
        let revision = graph_revision(&graph);
        let run_id = "019f0000-0000-7000-8000-000000000105";
        let command = creation_command(run_id, &registration_id, graph, revision, 1);
        let conn = Connection::open(writer.path()).unwrap();

        for (trigger_name, table) in [
            ("reject_run_insert", "runs"),
            ("reject_sequence_insert", "run_journal_sequences"),
            ("reject_journal_insert", "journal_entries"),
        ] {
            install_abort_trigger(&conn, trigger_name, table, run_id);
            assert!(
                matches!(
                    writer.create(command.clone()),
                    Err(RunCreateError::Sqlite(_))
                ),
                "expected sqlite abort at {table} insert"
            );
            assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
            conn.execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
                .unwrap();
        }
    }
}
