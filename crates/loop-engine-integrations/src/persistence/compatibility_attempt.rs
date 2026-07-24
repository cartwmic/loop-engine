//! Atomic per-run compatibility attempt persistence (T114).
//!
//! Journal-only writes: no workflow/lifecycle/version/latch mutation and no evidence
//! inventory changes. Registration-wide `provider.check --active-runs` never routes here.

use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::persistence_commands::{
    AppendCompatibilityAttemptCommand, AttemptCommit, CommittedRunSnapshot,
};
use loop_engine_core::capabilities::run_writer::CompatibilityAttemptWriter as CompatibilityAttemptWriterPort;
use loop_engine_core::model::ids::RunId;
use thiserror::Error;

use super::error::CommitOutcomeError;
use super::guidance_attempt::{
    AttemptWriteError, JournalEncodeExtras, append_journal_attempt, attempt_commit_expectation,
    commit_attempt_transaction, load_authoritative_run, select_attempt_draft,
    validate_draft_run_id,
};
use super::sqlite::connect_with_pragmas;
use super::traced::{
    MutationClass, OptionalTraceSink, SemanticOutcome, WriteExecution, WriteTraceSession,
    close_write, compatibility_attempt_error_semantic,
};
use loop_engine_core::model::outcome::OutcomeClass;

/// SQLite-backed atomic compatibility-attempt writer.
#[derive(Clone)]
pub struct CompatibilityAttemptWriter {
    path: PathBuf,
    trace: OptionalTraceSink,
}

impl CompatibilityAttemptWriterPort for CompatibilityAttemptWriter {
    type Error = CompatibilityAttemptError;

    fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error> {
        CompatibilityAttemptWriter::append_compatibility_attempt(self, command)
    }
}

impl std::fmt::Debug for CompatibilityAttemptWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompatibilityAttemptWriter")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompatibilityAttemptError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("journal command run_id does not match draft")]
    RunIdMismatch,
    #[error("journal finalize rejected draft: {0}")]
    JournalFinalize(loop_engine_core::model::journal::JournalError),
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

impl CommitOutcomeError for CompatibilityAttemptError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

impl From<AttemptWriteError> for CompatibilityAttemptError {
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

impl CompatibilityAttemptWriter {
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

    /// Append one compatibility attempt journal entry under `BEGIN IMMEDIATE`.
    pub fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<AttemptCommit, CompatibilityAttemptError> {
        close_write(
            &self.trace,
            "run.compatibility",
            MutationClass::RunMutation,
            |trace| {
                self.append_compatibility_attempt_impl(command, trace)
                    .map_ok(|(commit, outcome)| {
                        (commit, SemanticOutcome::from_outcome_class(outcome))
                    })
            },
            |(_, semantic)| *semantic,
            compatibility_attempt_error_semantic,
        )
        .map(|(status, _)| status)
    }

    fn append_compatibility_attempt_impl(
        &self,
        command: AppendCompatibilityAttemptCommand,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(AttemptCommit, OutcomeClass), CompatibilityAttemptError> {
        if let Err(error) = validate_compatibility_command(&command) {
            return WriteExecution::no_transaction(error);
        }
        let expected_workflow = command.expected_workflow_version();
        let expected_lifecycle = command.expected_lifecycle_version();
        let run_id = command.run_id().as_str();
        let conn = match connect_with_pragmas(self.path()).map_err(map_persistence) {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = conn
            .execute("BEGIN IMMEDIATE", [])
            .map_err(map_sqlite_persistence)
        {
            return WriteExecution::no_transaction(error);
        }
        let extras = JournalEncodeExtras {
            observed_executable_drift: command.observed_drift(),
        };
        let result = (|| {
            let row = load_authoritative_run(&conn, command.run_id())?;
            if let Some(session) = trace {
                session.version_check_run_cas(
                    run_id,
                    Some(expected_workflow.value()),
                    Some(expected_lifecycle.value()),
                );
            }
            let draft = select_attempt_draft(
                &row,
                expected_workflow,
                expected_lifecycle,
                command.journal_entry(),
                command.terminal_rejection_entry(),
                command.stale_error_entry(),
            )
            .clone();
            let outcome = draft.outcome();
            let reason = draft.reason().cloned();
            let (status, journal, associations) =
                append_journal_attempt(&conn, command.run_id(), draft, extras, &row)?;
            let expectation =
                attempt_commit_expectation(command.run_id(), &row, &journal, associations);
            let commit = AttemptCommit {
                commit: status,
                outcome,
                reason,
                run: CommittedRunSnapshot {
                    lifecycle: row.lifecycle,
                    current_state: row.current_state.clone(),
                    label: row.label.clone(),
                },
            };
            Ok(((commit, outcome), expectation))
        })();
        commit_attempt_transaction(self.path(), conn, result)
            .map_err(CompatibilityAttemptError::from)
    }
}

fn validate_compatibility_command(
    command: &AppendCompatibilityAttemptCommand,
) -> Result<(), CompatibilityAttemptError> {
    validate_draft_run_id(command.run_id(), command.journal_entry())
        .map_err(CompatibilityAttemptError::from)?;
    validate_draft_run_id(command.run_id(), command.terminal_rejection_entry())
        .map_err(CompatibilityAttemptError::from)?;
    validate_draft_run_id(command.run_id(), command.stale_error_entry())
        .map_err(CompatibilityAttemptError::from)?;
    Ok(())
}

fn map_sqlite_persistence(error: rusqlite::Error) -> CompatibilityAttemptError {
    CompatibilityAttemptError::Persistence {
        detail: error.to_string(),
    }
}

fn map_persistence(error: super::error::PersistenceError) -> CompatibilityAttemptError {
    CompatibilityAttemptError::Persistence {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::persistence_commands::AppendCompatibilityAttemptCommand;
    use loop_engine_core::model::attempt::{
        AttemptFacts, JournalExtension, ProviderFact, ProviderRole,
    };
    use loop_engine_core::model::compatibility::{
        CompatibilityFinding, CompatibilityFindings, CompatibilityReport, CompatibilityStatus,
    };
    use loop_engine_core::model::ids::{RegistrationId, RequestId, RunId};
    use loop_engine_core::model::journal::JournalDraft;
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::model::outcome::OutcomeClass;
    use loop_engine_core::model::provider::DigestObservation;
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::model::time::ObservedAt;
    use loop_engine_core::model::version::{LifecycleVersion, WorkflowStateVersion};
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::CompatibilityAttemptWriter;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::sqlite::open_at;

    const MINIMAL_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"none"}}],"transitions":[]}"#;

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

    fn provider_fact(digest_hex: &str) -> ProviderFact {
        ProviderFact::new(
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
            1,
            ProviderRole::CheckCompatibility,
            RequestId::parse("019f0000-0000-7000-8000-000000000401").unwrap(),
            "/bin/provider",
            OutcomeClass::Completed,
            DigestObservation::observed(format!("sha256:{digest_hex}")).unwrap(),
            Some("1.0.0".into()),
            Some(1),
        )
        .unwrap()
    }

    fn compatibility_draft(
        run_id: &str,
        outcome: OutcomeClass,
        reason: Option<Reason>,
        extension: JournalExtension,
        attempt: Option<AttemptFacts>,
    ) -> JournalDraft {
        JournalDraft::new(
            RunId::parse(run_id).unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.compatibility",
            RequestId::parse("019f0000-0000-7000-8000-000000000501").unwrap(),
            outcome,
            reason,
            attempt,
            extension,
        )
        .unwrap()
    }

    fn completed_compatibility_command(
        run_id: &str,
        digest_hex: &str,
        observed_drift: Option<bool>,
    ) -> AppendCompatibilityAttemptCommand {
        completed_compatibility_command_with_versions(
            run_id,
            digest_hex,
            observed_drift,
            WorkflowStateVersion::initial(),
            LifecycleVersion::initial(),
        )
    }

    fn completed_compatibility_command_with_versions(
        run_id: &str,
        digest_hex: &str,
        observed_drift: Option<bool>,
        expected_workflow: WorkflowStateVersion,
        expected_lifecycle: LifecycleVersion,
    ) -> AppendCompatibilityAttemptCommand {
        let findings = CompatibilityFindings::new(vec![
            CompatibilityFinding::new("live_guidance", CompatibilityStatus::Incompatible, vec![])
                .unwrap(),
        ])
        .unwrap();
        let attempt = AttemptFacts {
            provider_observations: vec![provider_fact(digest_hex)],
            ..AttemptFacts::default()
        };
        let journal_entry = compatibility_draft(
            run_id,
            OutcomeClass::Completed,
            None,
            JournalExtension::CompatibilityAttempt {
                findings: Some(findings),
            },
            Some(attempt.clone()),
        );
        let terminal_rejection_entry = compatibility_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt.clone()),
        );
        let stale_error_entry = compatibility_draft(
            run_id,
            OutcomeClass::Error,
            Some(Reason::new(ReasonCode::StateStaleVersion, "stale workflow state").unwrap()),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt),
        );
        AppendCompatibilityAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            expected_workflow,
            expected_lifecycle,
            observed_drift,
            journal_entry,
            terminal_rejection_entry,
            stale_error_entry,
        )
    }

    fn read_run_versions(conn: &Connection, run_id: &str) -> (i64, i64) {
        conn.query_row(
            "SELECT workflow_state_version, lifecycle_version FROM runs WHERE run_id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn atomic_append_records_findings_and_provider_observation() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000201";
        seed_run(&conn, run_id, &registration_id, "active", 5, 2);
        let before = read_run_versions(&conn, run_id);
        let writer = CompatibilityAttemptWriter::new(_dir.path().join("state.db"));
        let status = writer
            .append_compatibility_attempt(completed_compatibility_command_with_versions(
                run_id,
                &"c".repeat(64),
                Some(false),
                WorkflowStateVersion::try_from(5).unwrap(),
                LifecycleVersion::try_from(2).unwrap(),
            ))
            .unwrap();
        assert!(!status.commit.state_changed);
        assert_eq!(status.commit.workflow_state_version.value(), 5);
        assert_eq!(status.commit.lifecycle_version.value(), 2);
        assert_eq!(status.outcome, OutcomeClass::Completed);
        assert_eq!(read_run_versions(&conn, run_id), before);
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1 AND sequence = 2",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("\"entry_kind\":\"compatibility.attempt\""));
        assert!(payload.contains("\"role\":\"check_compatibility\""));
        assert!(payload.contains(&format!("sha256:{}", "c".repeat(64))));
        assert!(payload.contains("\"capability\":\"live_guidance\""));
        assert!(payload.contains("\"status\":\"incompatible\""));
        let evidence_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence_count, 0);
    }

    #[test]
    fn drift_observation_is_persisted_without_state_mutation() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000202";
        seed_run(&conn, run_id, &registration_id, "active", 1, 1);
        CompatibilityAttemptWriter::new(_dir.path().join("state.db"))
            .append_compatibility_attempt(completed_compatibility_command(
                run_id,
                &"d".repeat(64),
                Some(true),
            ))
            .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("\"observed_executable_drift\":true"));
        assert!(payload.contains(&format!("sha256:{}", "d".repeat(64))));
        let (workflow, lifecycle) = read_run_versions(&conn, run_id);
        assert_eq!((workflow, lifecycle), (1, 1));
    }

    #[test]
    fn missing_provider_post_lookup_is_persisted_from_supplied_journal() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000203";
        seed_run(&conn, run_id, &registration_id, "active", 1, 1);
        let journal_entry = compatibility_draft(
            run_id,
            OutcomeClass::Error,
            Some(
                Reason::new(
                    ReasonCode::ProviderRegistrationMissing,
                    "registration unavailable for invocation",
                )
                .unwrap(),
            ),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts::default()),
        );
        let terminal_rejection_entry = compatibility_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunLifecycleTerminal, "terminal lifecycle").unwrap()),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts::default()),
        );
        let command = AppendCompatibilityAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            WorkflowStateVersion::initial(),
            LifecycleVersion::initial(),
            None,
            journal_entry,
            terminal_rejection_entry.clone(),
            terminal_rejection_entry,
        );
        CompatibilityAttemptWriter::new(_dir.path().join("state.db"))
            .append_compatibility_attempt(command)
            .unwrap();
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("provider.registration.missing"));
        assert!(!payload.contains("provider_observations"));
    }

    #[test]
    fn terminal_lifecycle_appends_terminal_rejection_branch() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000204";
        seed_run(&conn, run_id, &registration_id, "terminated", 1, 2);
        let commit = CompatibilityAttemptWriter::new(_dir.path().join("state.db"))
            .append_compatibility_attempt(completed_compatibility_command(
                run_id,
                &"e".repeat(64),
                None,
            ))
            .unwrap();
        assert_eq!(commit.outcome, OutcomeClass::Rejected);
        assert_eq!(
            commit.reason.as_ref().map(|reason| reason.code()),
            Some(ReasonCode::RunLifecycleTerminal)
        );
        assert_eq!(commit.run.lifecycle, Lifecycle::Terminated);
        let (outcome, payload): (String, String) = conn
            .query_row(
                "SELECT outcome, encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(outcome, "rejected");
        assert!(payload.contains("run.lifecycle.terminal"));
        assert!(!payload.contains("\"findings\""));
    }

    #[test]
    fn workflow_state_version_race_selects_stale_error_entry() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000205";
        seed_run(&conn, run_id, &registration_id, "active", 2, 1);
        let commit = CompatibilityAttemptWriter::new(_dir.path().join("state.db"))
            .append_compatibility_attempt(completed_compatibility_command(
                run_id,
                &"f".repeat(64),
                None,
            ))
            .unwrap();
        assert_eq!(commit.outcome, OutcomeClass::Error);
        assert_eq!(
            commit.reason.as_ref().map(|reason| reason.code()),
            Some(ReasonCode::StateStaleVersion)
        );
        assert_eq!(commit.run.lifecycle, Lifecycle::Active);
        assert_eq!(commit.run.current_state.as_str(), "draft");
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("state.stale_version"));
        assert!(!payload.contains("\"findings\""));
    }

    #[test]
    fn initially_terminal_attempt_keeps_ordinary_terminal_reason() {
        let (_dir, conn, registration_id) = open_store();
        let run_id = "019f0000-0000-7000-8000-000000000206";
        seed_run(&conn, run_id, &registration_id, "terminated", 1, 1);
        let attempt = AttemptFacts::default();
        let ordinary = compatibility_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(
                Reason::new(
                    ReasonCode::RunLifecycleTerminal,
                    "run lifecycle is terminal",
                )
                .unwrap(),
            ),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt.clone()),
        );
        let raced = compatibility_draft(
            run_id,
            OutcomeClass::Rejected,
            Some(
                Reason::new(
                    ReasonCode::RunLifecycleTerminal,
                    "run lifecycle changed before compatibility committed",
                )
                .unwrap(),
            ),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt.clone()),
        );
        let stale = compatibility_draft(
            run_id,
            OutcomeClass::Error,
            Some(Reason::new(ReasonCode::StateStaleVersion, "stale workflow state").unwrap()),
            JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt),
        );
        let command = AppendCompatibilityAttemptCommand::for_test(
            RunId::parse(run_id).unwrap(),
            WorkflowStateVersion::initial(),
            LifecycleVersion::initial(),
            None,
            ordinary,
            raced,
            stale,
        );
        let commit = CompatibilityAttemptWriter::new(_dir.path().join("state.db"))
            .append_compatibility_attempt(command)
            .unwrap();
        assert_eq!(
            commit.reason.as_ref().map(|reason| reason.message()),
            Some("run lifecycle is terminal")
        );
        let payload: String = conn
            .query_row(
                "SELECT encoded_payload_json FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.contains("run lifecycle is terminal"));
        assert!(!payload.contains("run lifecycle changed before compatibility committed"));
    }

    #[test]
    fn registration_wide_provider_check_does_not_use_per_run_writer() {
        let command = completed_compatibility_command(
            "019f0000-0000-7000-8000-000000000201",
            &"f".repeat(64),
            None,
        );
        assert_eq!(command.journal_entry().operation(), "run.compatibility");
        assert_ne!(command.journal_entry().operation(), "provider.check");
        assert!(
            CompatibilityReport::findings(vec![
                CompatibilityFinding::new("gates", CompatibilityStatus::Compatible, vec![])
                    .unwrap()
            ])
            .is_ok()
        );
    }
}
