//! Traced persistence boundary (T118).
//!
//! Fail-safe operational trace emission at persistence read/write choke points.
//! Trace failures are diagnostic only and never roll back committed database state.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use loop_engine_core::capabilities::persistence_commands::{EventCommitBranch, EventCommitStatus};
use loop_engine_core::model::bounded::DIAGNOSTIC_ENCODED_BYTES;
use loop_engine_core::model::journal::JournalDraft;
use loop_engine_core::model::outcome::OutcomeClass;
use serde_json::{Value, json};

use crate::trace::{TraceCategory, TraceError, TraceEvent, TraceWriter};

use rusqlite::Connection;

use super::error::{CommitOutcomeError, PersistenceError, commit_outcome_trace_is_rollback};

/// Persistence mutation class recorded on `persistence.intent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationClass {
    Catalog,
    RunCreate,
    RunMutation,
    ReadOnly,
    ExportRead,
}

impl MutationClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::RunCreate => "run_create",
            Self::RunMutation => "run_mutation",
            Self::ReadOnly => "read_only",
            Self::ExportRead => "export_read",
        }
    }
}

/// Semantic outcome class for persistence commit/rollback/read closure events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticOutcome {
    Completed,
    Rejected,
    Error,
}

impl SemanticOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Error => "error",
        }
    }

    pub fn outcome_class(self) -> OutcomeClass {
        match self {
            Self::Completed => OutcomeClass::Completed,
            Self::Rejected => OutcomeClass::Rejected,
            Self::Error => OutcomeClass::Error,
        }
    }

    pub fn from_outcome_class(outcome: OutcomeClass) -> Self {
        match outcome {
            OutcomeClass::Completed => Self::Completed,
            OutcomeClass::Rejected => Self::Rejected,
            OutcomeClass::Error => Self::Error,
        }
    }
}

/// Diagnostic record when trace emission fails after the authoritative DB outcome is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistenceTraceFailure {
    pub message: String,
    pub after_commit: bool,
}

/// Shared fail-safe trace sink for persistence boundaries.
#[derive(Clone)]
pub struct PersistenceTraceSink {
    writer: Arc<Mutex<TraceWriter>>,
    last_failure: Arc<Mutex<Option<PersistenceTraceFailure>>>,
    operation_override: Arc<Mutex<Option<&'static str>>>,
}

impl std::fmt::Debug for PersistenceTraceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistenceTraceSink")
            .field("enabled", &self.is_enabled())
            .finish()
    }
}

impl PersistenceTraceSink {
    pub fn new(writer: Arc<Mutex<TraceWriter>>) -> Self {
        Self {
            writer,
            last_failure: Arc::new(Mutex::new(None)),
            operation_override: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        true
    }

    /// Returns and clears the most recent trace failure recorded by this sink.
    pub fn take_failure(&self) -> Option<PersistenceTraceFailure> {
        self.last_failure
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// Runs `operation` under a temporary operation-id override for ambiguous trait methods.
    pub fn with_operation<R>(&self, operation: &'static str, f: impl FnOnce() -> R) -> R {
        let prior = match self.operation_override.lock() {
            Ok(mut slot) => slot.replace(operation),
            Err(_) => return f(),
        };
        struct ResetOperation<'a> {
            slot: &'a Mutex<Option<&'static str>>,
            prior: Option<&'static str>,
        }
        impl Drop for ResetOperation<'_> {
            fn drop(&mut self) {
                if let Ok(mut slot) = self.slot.lock() {
                    *slot = self.prior;
                }
            }
        }
        let _reset = ResetOperation {
            slot: &self.operation_override,
            prior,
        };
        f()
    }

    fn resolve_operation<'a>(&self, default: &'a str) -> &'a str {
        self.operation_override
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(default)
    }

    fn record_failure(&self, message: String, after_commit: bool) {
        if let Ok(mut slot) = self.last_failure.lock() {
            *slot = Some(PersistenceTraceFailure {
                message,
                after_commit,
            });
        }
    }

    fn try_emit(
        &self,
        event: &str,
        payload: BTreeMap<String, Value>,
        after_commit: bool,
    ) -> Result<(), TraceError> {
        let mut writer = self.writer.lock().map_err(|_| TraceError::SinkFailed)?;
        let trace_event = TraceEvent::new(
            writer.request_id(),
            TraceCategory::Persistence,
            event,
            payload,
        );
        match writer.write(&trace_event) {
            Ok(_) => Ok(()),
            Err(error) => {
                let phase = trace_failure_phase(&error);
                let errno = trace_failure_errno(&error);
                drop(writer);
                self.emit_sink_failure(errno, phase, after_commit);
                self.record_failure(error.to_string(), after_commit);
                Err(error)
            }
        }
    }

    fn emit_sink_failure(&self, errno: &str, phase: &str, after_commit: bool) {
        let mut payload = BTreeMap::new();
        payload.insert("errno".into(), json!(errno));
        payload.insert("phase".into(), json!(phase));
        payload.insert("after_commit".into(), json!(after_commit));
        if let Ok(mut writer) = self.writer.lock() {
            let event = TraceEvent::new(
                writer.request_id(),
                TraceCategory::Trace,
                "sink_failure",
                payload,
            );
            let _ = writer.write(&event);
        }
    }

    fn emit_intent(&self, operation: &str, mutation_class: MutationClass) {
        let mut payload = BTreeMap::new();
        payload.insert("mutation_class".into(), json!(mutation_class.as_str()));
        payload.insert("operation".into(), json!(operation));
        let _ = self.try_emit("intent", payload, false);
    }

    fn emit_version_check_catalog(&self, registration_config_revision: u64) {
        let mut payload = BTreeMap::new();
        payload.insert(
            "registration_config_revision".into(),
            json!(registration_config_revision),
        );
        let _ = self.try_emit("version_check", payload, false);
    }

    fn emit_version_check_run(
        &self,
        run_id: &str,
        expected_workflow_version: Option<u64>,
        expected_lifecycle_version: Option<u64>,
    ) {
        let mut payload = BTreeMap::new();
        payload.insert("run_id".into(), json!(run_id));
        if let Some(version) = expected_workflow_version {
            payload.insert("expected_workflow_version".into(), json!(version));
        }
        if let Some(version) = expected_lifecycle_version {
            payload.insert("expected_lifecycle_version".into(), json!(version));
        }
        let _ = self.try_emit("version_check", payload, false);
    }

    fn emit_commit(&self, operation: &str, outcome: SemanticOutcome, after_commit: bool) {
        let mut payload = BTreeMap::new();
        payload.insert("operation".into(), json!(operation));
        payload.insert("outcome".into(), json!(outcome.as_str()));
        let _ = self.try_emit("commit", payload, after_commit);
    }

    fn emit_rollback(&self, operation: &str, outcome: SemanticOutcome) {
        let mut payload = BTreeMap::new();
        payload.insert("operation".into(), json!(operation));
        payload.insert("outcome".into(), json!(outcome.as_str()));
        let _ = self.try_emit("rollback", payload, false);
    }

    fn emit_read_complete(
        &self,
        operation: &str,
        outcome: SemanticOutcome,
        extras: ReadCompleteExtras,
    ) {
        let mut payload = BTreeMap::new();
        payload.insert("operation".into(), json!(operation));
        payload.insert("outcome".into(), json!(outcome.as_str()));
        extras.apply(&mut payload);
        let _ = self.try_emit("read_complete", payload, false);
    }

    fn emit_read_failure(&self, operation: &str, failure_code: &str, message: Option<&str>) {
        let mut payload = BTreeMap::new();
        payload.insert("operation".into(), json!(operation));
        payload.insert("failure_code".into(), json!(failure_code));
        if let Some(message) = message {
            payload.insert("message".into(), json!(message));
        }
        let _ = self.try_emit("read_failure", payload, false);
    }

    /// Emits pre-dispatch `invocation.error` for persistence open/migration/schema failures.
    ///
    /// Diagnostic only; must not alter the authoritative open error returned to callers.
    pub fn emit_predispatch_persistence_error(&self, error: &PersistenceError) {
        let (message, source_chain) = persistence_predispatch_error_payload(error);
        let mut payload = BTreeMap::new();
        payload.insert("phase".into(), json!("persistence"));
        payload.insert("message".into(), json!(message));
        if !source_chain.is_empty() {
            payload.insert("source_chain".into(), json!(source_chain));
        }
        let _ = self.try_emit_invocation("error", payload);
    }

    fn try_emit_invocation(
        &self,
        event: &str,
        payload: BTreeMap<String, Value>,
    ) -> Result<(), TraceError> {
        let mut writer = self.writer.lock().map_err(|_| TraceError::SinkFailed)?;
        let trace_event = TraceEvent::new(
            writer.request_id(),
            TraceCategory::Invocation,
            event,
            payload,
        );
        match writer.write(&trace_event) {
            Ok(_) => Ok(()),
            Err(error) => {
                let phase = trace_failure_phase(&error);
                let errno = trace_failure_errno(&error);
                drop(writer);
                self.emit_sink_failure(errno, phase, false);
                self.record_failure(error.to_string(), false);
                Err(error)
            }
        }
    }

    /// Opens a write trace session: emits `persistence.intent` (fail-safe).
    pub fn begin_write(
        &self,
        default_operation: &'static str,
        mutation_class: MutationClass,
    ) -> WriteTraceSession<'_> {
        let operation = self.resolve_operation(default_operation);
        self.emit_intent(operation, mutation_class);
        WriteTraceSession {
            sink: self,
            operation,
            mutation_class,
        }
    }

    /// Opens a read trace session: emits `persistence.intent` (fail-safe).
    pub fn begin_read(
        &self,
        default_operation: &'static str,
        mutation_class: MutationClass,
    ) -> ReadTraceSession<'_> {
        let operation = self.resolve_operation(default_operation);
        self.emit_intent(operation, mutation_class);
        ReadTraceSession {
            sink: self,
            operation,
        }
    }
}

/// Optional bounded metadata for `persistence.read_complete`.
#[derive(Debug, Clone, Default)]
pub struct ReadCompleteExtras {
    pub item_count: Option<u64>,
    pub next_cursor_present: Option<bool>,
    pub page_data_byte_length: Option<u64>,
    pub result_digest: Option<String>,
    pub manifest_digest: Option<String>,
    pub artifact_byte_lengths: Option<BTreeMap<String, u64>>,
}

impl ReadCompleteExtras {
    fn apply(self, payload: &mut BTreeMap<String, Value>) {
        if let Some(count) = self.item_count {
            payload.insert("item_count".into(), json!(count));
        }
        if let Some(present) = self.next_cursor_present {
            payload.insert("next_cursor_present".into(), json!(present));
        }
        if let Some(length) = self.page_data_byte_length {
            payload.insert("page_data_byte_length".into(), json!(length));
        }
        if let Some(digest) = self.result_digest {
            payload.insert(
                "result_digest".into(),
                if digest.is_empty() {
                    Value::Null
                } else {
                    Value::String(digest)
                },
            );
        }
        if let Some(digest) = self.manifest_digest {
            payload.insert("manifest_digest".into(), json!(digest));
        }
        if let Some(lengths) = self.artifact_byte_lengths {
            payload.insert("artifact_byte_lengths".into(), json!(lengths));
        }
    }

    pub fn for_page<T>(
        page: &loop_engine_core::capabilities::Page<T>,
        page_data_bytes: u64,
    ) -> Self {
        Self {
            item_count: Some(page.rows.len() as u64),
            next_cursor_present: Some(page.next_cursor.is_some()),
            page_data_byte_length: Some(page_data_bytes),
            result_digest: None,
            manifest_digest: None,
            artifact_byte_lengths: None,
        }
    }
}

pub struct WriteTraceSession<'a> {
    sink: &'a PersistenceTraceSink,
    operation: &'static str,
    mutation_class: MutationClass,
}

impl<'a> WriteTraceSession<'a> {
    pub fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn mutation_class(&self) -> MutationClass {
        self.mutation_class
    }

    pub fn version_check_catalog(&self, registration_config_revision: u64) {
        self.sink
            .emit_version_check_catalog(registration_config_revision);
    }

    pub fn version_check_run(
        &self,
        run_id: &str,
        expected_workflow_version: u64,
        expected_lifecycle_version: u64,
    ) {
        self.version_check_run_cas(
            run_id,
            Some(expected_workflow_version),
            Some(expected_lifecycle_version),
        );
    }

    /// Emits run-scoped CAS fields present on the command (omit unset expected versions).
    pub fn version_check_run_cas(
        &self,
        run_id: &str,
        expected_workflow_version: Option<u64>,
        expected_lifecycle_version: Option<u64>,
    ) {
        self.sink.emit_version_check_run(
            run_id,
            expected_workflow_version,
            expected_lifecycle_version,
        );
    }

    /// Closes the write path after a successful DB commit.
    pub fn finish_committed(self, semantic: SemanticOutcome) {
        self.sink.emit_commit(self.operation, semantic, true);
    }

    /// Closes the write path after a DB rollback.
    pub fn finish_rolled_back(self, semantic: SemanticOutcome) {
        self.sink.emit_rollback(self.operation, semantic);
    }
}

pub struct ReadTraceSession<'a> {
    sink: &'a PersistenceTraceSink,
    operation: &'static str,
}

impl<'a> ReadTraceSession<'a> {
    pub fn finish_completed(self, extras: ReadCompleteExtras) {
        self.sink
            .emit_read_complete(self.operation, SemanticOutcome::Completed, extras);
    }

    pub fn finish_rejected(self, extras: ReadCompleteExtras) {
        self.sink
            .emit_read_complete(self.operation, SemanticOutcome::Rejected, extras);
    }

    pub fn finish_failure(self, failure_code: &'static str, message: Option<&str>) {
        self.sink
            .emit_read_failure(self.operation, failure_code, message);
    }
}

/// Optional trace sink held by persistence adapters; no-op when absent.
#[derive(Clone, Default)]
pub struct OptionalTraceSink {
    pub(crate) inner: Option<PersistenceTraceSink>,
}

impl OptionalTraceSink {
    pub fn none() -> Self {
        Self { inner: None }
    }

    pub fn from_arc(writer: Arc<Mutex<TraceWriter>>) -> Self {
        Self {
            inner: Some(PersistenceTraceSink::new(writer)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn take_failure(&self) -> Option<PersistenceTraceFailure> {
        self.inner
            .as_ref()
            .and_then(PersistenceTraceSink::take_failure)
    }

    pub fn with_operation<R>(&self, operation: &'static str, f: impl FnOnce() -> R) -> R {
        match &self.inner {
            Some(sink) => sink.with_operation(operation, f),
            None => f(),
        }
    }

    pub fn begin_write(
        &self,
        operation: &'static str,
        mutation_class: MutationClass,
    ) -> Option<WriteTraceSession<'_>> {
        self.inner
            .as_ref()
            .map(|sink| sink.begin_write(operation, mutation_class))
    }

    pub fn begin_read(
        &self,
        operation: &'static str,
        mutation_class: MutationClass,
    ) -> Option<ReadTraceSession<'_>> {
        self.inner
            .as_ref()
            .map(|sink| sink.begin_read(operation, mutation_class))
    }

    /// Emits pre-dispatch `invocation.error` when trace is enabled.
    pub fn emit_predispatch_persistence_error(&self, error: &PersistenceError) {
        if let Some(sink) = &self.inner {
            sink.emit_predispatch_persistence_error(error);
        }
    }
}

impl std::fmt::Debug for OptionalTraceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OptionalTraceSink")
            .field("enabled", &self.is_enabled())
            .finish()
    }
}

pub fn catalog_mutation_operation(
    command: &loop_engine_core::capabilities::provider_catalog::CatalogMutation,
) -> &'static str {
    use loop_engine_core::capabilities::provider_catalog::CatalogMutation;
    match command {
        CatalogMutation::Add { .. } => "provider.add",
        CatalogMutation::Update { .. } => "provider.update",
        CatalogMutation::Rename { .. } => "provider.rename",
        CatalogMutation::Disable { .. } => "provider.disable",
        CatalogMutation::Restore { .. } => "provider.restore",
    }
}

/// Transaction disposition for a persistence write boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteTransactionDisposition {
    /// No write transaction was opened (pre-BEGIN validation, connect, or BEGIN failure).
    NoTransaction,
    /// Write transaction committed successfully (including semantic rejections returned as `Ok`).
    Committed,
    /// Explicit `ROLLBACK` succeeded after a started transaction.
    RollbackConfirmed,
    /// Transaction was started but rollback was not confirmed (ROLLBACK I/O failure,
    /// commit I/O with unknown durable outcome, or integrity failure after commit I/O).
    RollbackUnconfirmed,
}

/// Integration-owned write outcome carrying trace rollback eligibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteExecution<T, E> {
    pub result: Result<T, E>,
    pub disposition: WriteTransactionDisposition,
}

impl<T, E> WriteExecution<T, E> {
    pub fn committed(value: T) -> Self {
        Self {
            result: Ok(value),
            disposition: WriteTransactionDisposition::Committed,
        }
    }

    pub fn no_transaction(error: E) -> Self {
        Self {
            result: Err(error),
            disposition: WriteTransactionDisposition::NoTransaction,
        }
    }

    pub fn rollback_confirmed(error: E) -> Self {
        Self {
            result: Err(error),
            disposition: WriteTransactionDisposition::RollbackConfirmed,
        }
    }

    pub fn rollback_unconfirmed(error: E) -> Self {
        Self {
            result: Err(error),
            disposition: WriteTransactionDisposition::RollbackUnconfirmed,
        }
    }

    pub fn map_ok<U>(self, f: impl FnOnce(T) -> U) -> WriteExecution<U, E> {
        WriteExecution {
            result: self.result.map(f),
            disposition: self.disposition,
        }
    }

    pub fn map_err<F>(self, f: impl FnOnce(E) -> F) -> WriteExecution<T, F> {
        WriteExecution {
            result: self.result.map_err(f),
            disposition: self.disposition,
        }
    }

    pub fn into_result(self) -> Result<T, E> {
        self.result
    }

    pub fn should_emit_rollback(&self) -> bool {
        matches!(
            self.disposition,
            WriteTransactionDisposition::RollbackConfirmed
        )
    }
}

/// Classify a successful commit path or an unverified commit I/O outcome.
pub fn committed_or_unconfirmed<T, E>(result: Result<T, E>) -> WriteExecution<T, E> {
    match result {
        Ok(value) => WriteExecution::committed(value),
        Err(error) => WriteExecution::rollback_unconfirmed(error),
    }
}

/// Roll back an open transaction and report whether trace may claim rollback.
pub fn rollback_open_transaction<T, E>(conn: &Connection, error: E) -> WriteExecution<T, E> {
    match conn.execute("ROLLBACK", []) {
        Ok(_) => WriteExecution::rollback_confirmed(error),
        Err(_) => WriteExecution::rollback_unconfirmed(error),
    }
}

pub fn catalog_read_failure(
    error: &crate::persistence::provider_catalog::CatalogPersistenceError,
) -> (&'static str, Option<String>) {
    use crate::persistence::provider_catalog::CatalogPersistenceError;
    match error {
        CatalogPersistenceError::NotFound
        | CatalogPersistenceError::Disabled
        | CatalogPersistenceError::InvalidCursor
        | CatalogPersistenceError::InvalidAck => ("persistence.failed", Some(error.to_string())),
        CatalogPersistenceError::Duplicate
        | CatalogPersistenceError::Occupied
        | CatalogPersistenceError::Stale
        | CatalogPersistenceError::Constraint
        | CatalogPersistenceError::Persistence(_)
        | CatalogPersistenceError::Mapping(_)
        | CatalogPersistenceError::CommitOutcomeUnverified
        | CatalogPersistenceError::CommitIntegrityFailure => {
            ("persistence.failed", Some(error.to_string()))
        }
    }
}

pub fn close_write<T, E>(
    trace: &OptionalTraceSink,
    operation: &'static str,
    mutation_class: MutationClass,
    operation_fn: impl FnOnce(Option<&WriteTraceSession<'_>>) -> WriteExecution<T, E>,
    committed_semantic: impl FnOnce(&T) -> SemanticOutcome,
    rolled_back_semantic: impl FnOnce(&E) -> SemanticOutcome,
) -> Result<T, E>
where
    E: CommitOutcomeError + std::fmt::Display,
{
    if let Some(session) = trace.begin_write(operation, mutation_class) {
        let execution = operation_fn(Some(&session));
        let should_rollback = execution.should_emit_rollback();
        match execution.into_result() {
            Ok(value) => {
                session.finish_committed(committed_semantic(&value));
                Ok(value)
            }
            Err(error) => {
                if should_rollback && commit_outcome_trace_is_rollback(&error) {
                    session.finish_rolled_back(rolled_back_semantic(&error));
                }
                // Rollback-unconfirmed paths retain intent-only partial trace.
                Err(error)
            }
        }
    } else {
        operation_fn(None).into_result()
    }
}

pub fn close_read<T, E>(
    trace: &OptionalTraceSink,
    operation: &'static str,
    mutation_class: MutationClass,
    operation_fn: impl FnOnce() -> Result<T, E>,
    completed: impl FnOnce(&T) -> ReadCompleteExtras,
    rejected: impl FnOnce(&E) -> bool,
    failure: impl FnOnce(&E) -> (&'static str, Option<String>),
) -> Result<T, E> {
    if let Some(session) = trace.begin_read(operation, mutation_class) {
        let result = operation_fn();
        match &result {
            Ok(value) => session.finish_completed(completed(value)),
            Err(error) if rejected(error) => session.finish_rejected(ReadCompleteExtras::default()),
            Err(error) => {
                let (code, message) = failure(error);
                session.finish_failure(code, message.as_deref());
            }
        }
        result
    } else {
        operation_fn()
    }
}

pub fn finish_traced_event_write<E>(
    trace: &OptionalTraceSink,
    operation: &'static str,
    operation_fn: impl FnOnce(
        Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<(EventCommitStatus, SemanticOutcome), E>,
    rolled_back_semantic: impl FnOnce(&E) -> SemanticOutcome,
) -> Result<EventCommitStatus, E>
where
    E: CommitOutcomeError + std::fmt::Display,
{
    if let Some(session) = trace.begin_write(operation, MutationClass::RunMutation) {
        let execution = operation_fn(Some(&session));
        let should_rollback = execution.should_emit_rollback();
        match execution.into_result() {
            Ok((status, semantic)) => {
                session.finish_committed(semantic);
                Ok(status)
            }
            Err(error) => {
                if should_rollback && commit_outcome_trace_is_rollback(&error) {
                    session.finish_rolled_back(rolled_back_semantic(&error));
                }
                // Rollback-unconfirmed paths retain intent-only partial trace.
                Err(error)
            }
        }
    } else {
        operation_fn(None).into_result().map(|(status, _)| status)
    }
}

pub fn journal_draft_semantic(draft: &JournalDraft) -> SemanticOutcome {
    SemanticOutcome::from_outcome_class(draft.outcome())
}

pub fn event_commit_semantic(
    status: &EventCommitStatus,
    expected_draft: &JournalDraft,
    stale_draft: &JournalDraft,
) -> SemanticOutcome {
    match status.branch {
        EventCommitBranch::ExpectedVersions => journal_draft_semantic(expected_draft),
        EventCommitBranch::StaleVersions => journal_draft_semantic(stale_draft),
        EventCommitBranch::InlineEvidenceConflict => SemanticOutcome::Rejected,
        EventCommitBranch::ProviderEvidenceConflict => SemanticOutcome::Error,
    }
}

pub fn run_read_rejected(error: &crate::persistence::run_reads::RunReadError) -> bool {
    matches!(
        error,
        crate::persistence::run_reads::RunReadError::NotFound { .. }
    )
}

pub fn run_read_failure(
    error: &crate::persistence::run_reads::RunReadError,
) -> (&'static str, Option<String>) {
    ("persistence.failed", Some(error.to_string()))
}

pub fn run_mutation_error_semantic(
    error: &crate::persistence::run_mutations::RunMutationError,
) -> SemanticOutcome {
    use crate::persistence::run_mutations::RunMutationError;
    match error {
        RunMutationError::NotFound { .. }
        | RunMutationError::RunIdMismatch
        | RunMutationError::EvidenceDuplicate
        | RunMutationError::InvalidCorrectionLink
        | RunMutationError::JournalBranchMismatch => SemanticOutcome::Rejected,
        _ => SemanticOutcome::Error,
    }
}

pub fn history_read_rejected(error: &crate::persistence::history::HistoryReadError) -> bool {
    use crate::persistence::history::HistoryReadError;
    use loop_engine_core::operations::paging::PagingError;

    matches!(
        error,
        HistoryReadError::NotFound { .. }
            | HistoryReadError::Page(PagingError::CursorBinding | PagingError::CursorVersion)
    )
}

pub fn history_read_failure(
    error: &crate::persistence::history::HistoryReadError,
) -> (&'static str, Option<String>) {
    ("persistence.failed", Some(error.to_string()))
}

pub fn evidence_read_rejected(
    error: &crate::persistence::evidence_reads::EvidenceReadError,
) -> bool {
    matches!(
        error,
        crate::persistence::evidence_reads::EvidenceReadError::NotFound
            | crate::persistence::evidence_reads::EvidenceReadError::Unavailable
    )
}

pub fn evidence_read_failure(
    error: &crate::persistence::evidence_reads::EvidenceReadError,
) -> (&'static str, Option<String>) {
    ("persistence.failed", Some(error.to_string()))
}

pub fn event_attempt_error_semantic(
    error: &crate::persistence::event_attempt::EventAttemptPersistenceError,
) -> SemanticOutcome {
    use crate::persistence::event_attempt::EventAttemptPersistenceError;
    match error {
        EventAttemptPersistenceError::NotFound
        | EventAttemptPersistenceError::EvidenceInvalid
        | EventAttemptPersistenceError::Validation { .. } => SemanticOutcome::Rejected,
        _ => SemanticOutcome::Error,
    }
}

pub fn guidance_attempt_error_semantic(
    error: &crate::persistence::guidance_attempt::GuidanceAttemptError,
) -> SemanticOutcome {
    use crate::persistence::guidance_attempt::GuidanceAttemptError;
    match error {
        GuidanceAttemptError::NotFound { .. } | GuidanceAttemptError::RunIdMismatch => {
            SemanticOutcome::Rejected
        }
        _ => SemanticOutcome::Error,
    }
}

pub fn compatibility_attempt_error_semantic(
    error: &crate::persistence::compatibility_attempt::CompatibilityAttemptError,
) -> SemanticOutcome {
    use crate::persistence::compatibility_attempt::CompatibilityAttemptError;
    match error {
        CompatibilityAttemptError::NotFound { .. } | CompatibilityAttemptError::RunIdMismatch => {
            SemanticOutcome::Rejected
        }
        _ => SemanticOutcome::Error,
    }
}

pub fn run_create_error_semantic(
    error: &crate::persistence::run_create::RunCreateError,
) -> SemanticOutcome {
    use crate::persistence::run_create::RunCreateError;
    match error {
        RunCreateError::StaleProviderConfig
        | RunCreateError::JournalAuthorityMismatch
        | RunCreateError::GraphRevisionMismatch { .. } => SemanticOutcome::Rejected,
        _ => SemanticOutcome::Error,
    }
}

fn bound_diagnostic_text(text: &str) -> String {
    if text.len() <= DIAGNOSTIC_ENCODED_BYTES {
        return text.to_string();
    }
    let mut end = DIAGNOSTIC_ENCODED_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn persistence_predispatch_error_payload(error: &PersistenceError) -> (String, Vec<String>) {
    use PersistenceError::*;
    match error {
        CreateDirectory { path, source } => (
            bound_diagnostic_text("failed to create persistence directory"),
            vec![bound_diagnostic_text(&format!(
                "{}: {source}",
                path.display()
            ))],
        ),
        Open { path, source } => (
            bound_diagnostic_text("failed to open persistence store"),
            vec![bound_diagnostic_text(&format!(
                "{}: {source}",
                path.display()
            ))],
        ),
        PragmaRead { pragma, source } => (
            bound_diagnostic_text(&format!("failed to read pragma {pragma}")),
            vec![bound_diagnostic_text(&source.to_string())],
        ),
        Pragma { pragma, source } => (
            bound_diagnostic_text(&format!("failed to apply persistence pragma {pragma}")),
            vec![bound_diagnostic_text(&source.to_string())],
        ),
        FutureSchema {
            supported,
            observed,
        } => (
            bound_diagnostic_text(&format!(
                "database schema version {observed} exceeds supported version {supported}"
            )),
            vec![bound_diagnostic_text(&format!(
                "schema version {observed} > {supported}"
            ))],
        ),
        Migration { message } => (
            bound_diagnostic_text("database migration failed"),
            vec![bound_diagnostic_text(message)],
        ),
        SchemaMismatch {
            object_type,
            name,
            kind,
        } => (
            bound_diagnostic_text("database schema shape does not match bundled migration"),
            vec![bound_diagnostic_text(&format!(
                "{object_type} {name} {kind:?}"
            ))],
        ),
        SchemaInventoryProbe { source } => (
            bound_diagnostic_text("failed to verify database schema inventory"),
            vec![bound_diagnostic_text(&source.to_string())],
        ),
        MetadataKeyMissing { key } => (
            bound_diagnostic_text(&format!("integration metadata key {key} is missing")),
            vec![bound_diagnostic_text(&format!(
                "metadata key {key} missing"
            ))],
        ),
        MetadataKeyInvalidLength {
            key,
            expected,
            actual,
        } => (
            bound_diagnostic_text(&format!(
                "integration metadata key {key} has invalid length"
            )),
            vec![bound_diagnostic_text(&format!(
                "expected {expected} bytes, observed {actual}"
            ))],
        ),
        MetadataRead { source } => (
            bound_diagnostic_text("failed to read integration metadata"),
            vec![bound_diagnostic_text(&source.to_string())],
        ),
        InvalidUserVersion { observed } => (
            bound_diagnostic_text(&format!("invalid SQLite user_version {observed}")),
            vec![bound_diagnostic_text(&format!("user_version {observed}"))],
        ),
    }
}

pub fn catalog_read_rejected(
    error: &crate::persistence::provider_catalog::CatalogPersistenceError,
) -> bool {
    use crate::persistence::provider_catalog::CatalogPersistenceError;
    matches!(
        error,
        CatalogPersistenceError::NotFound
            | CatalogPersistenceError::Disabled
            | CatalogPersistenceError::InvalidCursor
            | CatalogPersistenceError::InvalidAck
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const RAW_EFBIG: i32 = 27;

fn trace_failure_errno(error: &TraceError) -> &'static str {
    match error {
        TraceError::Io { source, .. } => errno_name(source),
        TraceError::FileLimit { .. } => "EFBIG",
        TraceError::BudgetExhausted { .. } => "ENOSPC",
        TraceError::ReservationExhausted => "ENOSPC",
        TraceError::SinkFailed => "EIO",
        TraceError::Collision(_) => "EEXIST",
        TraceError::ReservedPayloadField(_) => "EINVAL",
        TraceError::Serialize(_) => "EINVAL",
        TraceError::MalformedSidecar(_) => "EIO",
        TraceError::NoProviderReservation => "EINVAL",
    }
}

fn trace_failure_phase(error: &TraceError) -> &'static str {
    match error {
        TraceError::Io { phase, .. } => phase.as_str(),
        _ => "write",
    }
}

fn errno_name(error: &io::Error) -> &'static str {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if error.raw_os_error() == Some(RAW_EFBIG) {
        return "EFBIG";
    }
    match error.kind() {
        io::ErrorKind::StorageFull => "ENOSPC",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::ReadOnlyFilesystem => "EROFS",
        _ => "EIO",
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    pub fn test_sink(
        request_id: &str,
    ) -> (TempTraceDir, Arc<Mutex<TraceWriter>>, PersistenceTraceSink) {
        let dir = TempTraceDir::new();
        let writer = Arc::new(Mutex::new(
            TraceWriter::create(&dir.trace_dir(), request_id).expect("trace writer"),
        ));
        let sink = PersistenceTraceSink::new(writer.clone());
        (dir, writer, sink)
    }

    pub fn read_events(path: &Path) -> Vec<Value> {
        let content = fs::read_to_string(path).expect("trace file");
        content
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("json line"))
            .collect()
    }

    pub fn invocation_event_names(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .filter(|event| event["category"] == "invocation")
            .filter_map(|event| event["event"].as_str().map(str::to_owned))
            .collect()
    }

    pub fn event_names(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .filter(|event| event["category"] == "persistence")
            .filter_map(|event| event["event"].as_str().map(str::to_owned))
            .collect()
    }

    pub struct TempTraceDir {
        root: tempfile::TempDir,
    }

    impl TempTraceDir {
        pub fn new() -> Self {
            Self {
                root: tempfile::TempDir::new().expect("tempdir"),
            }
        }

        pub fn trace_dir(&self) -> PathBuf {
            self.root.path().join("traces")
        }
    }

    /// Consumes the invocation's real trace reservation until further writes are rejected.
    pub fn exhaust_trace_reservation(writer: &Arc<Mutex<TraceWriter>>) {
        let mut writer = writer.lock().expect("trace writer");
        for padding_bytes in (0..=23).rev().map(|shift| 1_usize << shift) {
            loop {
                let mut payload = BTreeMap::new();
                payload.insert(
                    "padding".to_owned(),
                    Value::String("x".repeat(padding_bytes)),
                );
                let event = TraceEvent::new(
                    writer.request_id(),
                    TraceCategory::Trace,
                    "test.reservation.consume",
                    payload,
                );
                if writer.write(&event).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{event_names, read_events, test_sink};
    use super::*;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::run_reads::SqliteRunReads;
    use crate::persistence::sqlite::open_at;
    use crate::trace::{TraceError, TraceIoPhase};
    use loop_engine_core::model::ids::RunId;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn trace_failure_phase_maps_typed_io_phase() {
        let flush = TraceError::io_at(
            "/tmp/trace.jsonl",
            TraceIoPhase::Flush,
            io::ErrorKind::Interrupted.into(),
        );
        assert_eq!(trace_failure_phase(&flush), "flush");

        let fsync = TraceError::io_at(
            "/tmp/trace.jsonl",
            TraceIoPhase::Fsync,
            io::ErrorKind::Other.into(),
        );
        assert_eq!(trace_failure_phase(&fsync), "fsync");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn errno_name_maps_raw_efbig() {
        let error = io::Error::from_raw_os_error(super::RAW_EFBIG);
        assert_eq!(errno_name(&error), "EFBIG");
        assert_eq!(
            trace_failure_errno(&TraceError::io("/tmp/x", error)),
            "EFBIG"
        );
    }

    #[test]
    fn operation_override_does_not_deadlock_trace_reentry() {
        let (trace_dir, _writer, sink) = test_sink("operation-override");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            trace.with_operation("run.history", || {
                let result: Result<(), &str> = close_read(
                    &trace,
                    "run.show",
                    MutationClass::ReadOnly,
                    || Ok(()),
                    |_| ReadCompleteExtras::default(),
                    |_| false,
                    |_| ("persistence.failed", None),
                );
                done_tx.send(result).unwrap();
            });
        });
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(())
        );
        let events = read_events(&trace_dir.trace_dir().join("operation-override.jsonl"));
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["operation"], "run.history");
        assert_eq!(events[1]["operation"], "run.history");
    }

    #[test]
    fn read_success_emits_intent_and_read_complete() {
        let (trace_dir, _writer, sink) = test_sink("read-success");
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("state.db");
        open_at(&db_path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();

        let reads = SqliteRunReads::with_trace(
            db_path,
            OptionalTraceSink {
                inner: Some(sink.clone()),
            },
        );
        let run_id = RunId::parse("019f0000-0000-7000-8000-000000000001").unwrap();
        let error = reads.show(&run_id).expect_err("missing run");
        assert!(matches!(
            error,
            crate::persistence::run_reads::RunReadError::NotFound { .. }
        ));

        let events = read_events(&trace_dir.trace_dir().join("read-success.jsonl"));
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["mutation_class"], "read_only");
        assert_eq!(events[0]["operation"], "run.show");
        assert_eq!(events[1]["outcome"], "rejected");
    }

    #[test]
    fn write_scope_emits_intent_before_operation_closure_runs() {
        let (trace_dir, _writer, sink) = test_sink("write-temporal");
        let trace = OptionalTraceSink {
            inner: Some(sink.clone()),
        };
        let trace_path = trace_dir.trace_dir().join("write-temporal.jsonl");

        let result: Result<(), &str> = close_write(
            &trace,
            "run.annotate",
            MutationClass::RunMutation,
            |_trace| {
                let events = read_events(&trace_path);
                assert_eq!(event_names(&events), vec!["intent"]);
                WriteExecution::rollback_confirmed("simulated failure")
            },
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert_eq!(result, Err("simulated failure"));

        let events = read_events(&trace_path);
        assert_eq!(event_names(&events), vec!["intent", "rollback"]);
        assert_eq!(events[1]["outcome"], "error");
    }

    #[test]
    fn pre_begin_error_emits_intent_only() {
        let (trace_dir, _writer, sink) = test_sink("pre-begin");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let result: Result<(), &str> = close_write(
            &trace,
            "run.guidance",
            MutationClass::RunMutation,
            |_| WriteExecution::no_transaction("validation failed"),
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Rejected,
        );
        assert_eq!(result, Err("validation failed"));
        let events = read_events(&trace_dir.trace_dir().join("pre-begin.jsonl"));
        assert_eq!(event_names(&events), vec!["intent"]);
    }

    #[test]
    fn begin_busy_emits_intent_only() {
        let (trace_dir, _writer, sink) = test_sink("begin-busy");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let result: Result<(), &str> = close_write(
            &trace,
            "run.create",
            MutationClass::RunCreate,
            |_| WriteExecution::no_transaction("database is locked"),
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert_eq!(result, Err("database is locked"));
        let events = read_events(&trace_dir.trace_dir().join("begin-busy.jsonl"));
        assert_eq!(event_names(&events), vec!["intent"]);
    }

    #[test]
    fn real_rollback_failure_closes_without_rollback_claim() {
        let (trace_dir, _writer, sink) = test_sink("rollback-io");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("BEGIN IMMEDIATE", []).unwrap();
        conn.execute("COMMIT", []).unwrap();
        let execution = rollback_open_transaction::<(), &str>(&conn, "work failed");
        assert!(!execution.should_emit_rollback());
        let result = close_write(
            &trace,
            "run.annotate",
            MutationClass::RunMutation,
            |_| execution,
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert_eq!(result, Err("work failed"));
        let events = read_events(&trace_dir.trace_dir().join("rollback-io.jsonl"));
        assert_eq!(event_names(&events), vec!["intent"]);
    }

    #[test]
    fn read_scope_emits_intent_before_operation_closure_runs() {
        let (trace_dir, _writer, sink) = test_sink("read-temporal");
        let trace = OptionalTraceSink {
            inner: Some(sink.clone()),
        };
        let trace_path = trace_dir.trace_dir().join("read-temporal.jsonl");

        let result = close_read(
            &trace,
            "run.show",
            MutationClass::ReadOnly,
            || {
                let events = read_events(&trace_path);
                assert_eq!(event_names(&events), vec!["intent"]);
                Err::<(), &str>("simulated read failure")
            },
            |_| ReadCompleteExtras::default(),
            |_| false,
            |_| ("persistence.failed", Some("simulated read failure".into())),
        );
        assert_eq!(result, Err("simulated read failure"));

        let events = read_events(&trace_path);
        assert_eq!(event_names(&events), vec!["intent", "read_failure"]);
        assert_eq!(events[1]["failure_code"], "persistence.failed");
    }

    #[test]
    fn write_commit_emits_intent_version_check_and_commit() {
        let (_trace_dir, _writer, sink) = test_sink("write-commit");
        let session = sink.begin_write("run.request", MutationClass::RunMutation);
        session.version_check_run("run-1", 3, 2);
        session.finish_committed(SemanticOutcome::Completed);
        let failure = sink.take_failure();
        assert!(failure.is_none());
    }

    #[test]
    fn write_rollback_emits_intent_and_rollback() {
        let (trace_dir, _writer, sink) = test_sink("write-rollback");
        let session = sink.begin_write("run.create", MutationClass::RunCreate);
        session.finish_rolled_back(SemanticOutcome::Error);
        let events = read_events(&trace_dir.trace_dir().join("write-rollback.jsonl"));
        assert_eq!(event_names(&events), vec!["intent", "rollback"]);
        assert_eq!(events[1]["outcome"], "error");
    }

    #[test]
    fn unverified_commit_error_closes_with_intent_only_without_commit_or_rollback() {
        let (trace_dir, _writer, sink) = test_sink("commit-unknown");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let result: Result<(), crate::persistence::run_create::RunCreateError> = close_write(
            &trace,
            "run.create",
            MutationClass::RunCreate,
            |_| {
                WriteExecution::rollback_unconfirmed(
                    crate::persistence::run_create::RunCreateError::CommitOutcomeUnverified,
                )
            },
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert!(result.is_err());
        let events = read_events(&trace_dir.trace_dir().join("commit-unknown.jsonl"));
        assert_eq!(event_names(&events), vec!["intent"]);
    }

    #[test]
    fn integrity_failure_closes_with_intent_only_without_commit_or_rollback() {
        let (trace_dir, _writer, sink) = test_sink("commit-integrity");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let result: Result<(), crate::persistence::run_create::RunCreateError> = close_write(
            &trace,
            "run.create",
            MutationClass::RunCreate,
            |_| {
                WriteExecution::rollback_unconfirmed(
                    crate::persistence::run_create::RunCreateError::CommitIntegrityFailure,
                )
            },
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert!(result.is_err());
        let events = read_events(&trace_dir.trace_dir().join("commit-integrity.jsonl"));
        assert_eq!(event_names(&events), vec!["intent"]);
    }

    #[test]
    fn sink_failure_after_commit_remains_after_commit() {
        use super::test_support::exhaust_trace_reservation;

        let (_trace_dir, writer, sink) = test_sink("sink-after-commit");
        let session = sink.begin_write("run.annotate", MutationClass::RunMutation);
        exhaust_trace_reservation(&writer);
        session.finish_committed(SemanticOutcome::Completed);
        let failure = sink.take_failure().expect("trace failure");
        assert!(failure.after_commit);
    }

    #[test]
    fn sink_failure_before_commit_records_after_commit_false() {
        use super::test_support::exhaust_trace_reservation;

        let (_trace_dir, writer, sink) = test_sink("sink-before-commit");
        let session = sink.begin_write("run.create", MutationClass::RunCreate);
        exhaust_trace_reservation(&writer);
        session.finish_rolled_back(SemanticOutcome::Error);
        let failure = sink.take_failure().expect("trace failure");
        assert!(!failure.after_commit);
    }

    #[test]
    fn close_write_succeeds_when_commit_trace_emission_fails() {
        use super::test_support::{event_names, exhaust_trace_reservation, read_events};

        let (trace_dir, writer, sink) = test_sink("commit-truthful");
        let trace = OptionalTraceSink {
            inner: Some(sink.clone()),
        };
        let trace_path = trace_dir.trace_dir().join("commit-truthful.jsonl");

        let result: Result<&'static str, &str> = close_write(
            &trace,
            "run.annotate",
            MutationClass::RunMutation,
            |_trace| {
                exhaust_trace_reservation(&writer);
                WriteExecution::committed("persisted")
            },
            |_| SemanticOutcome::Completed,
            |_| SemanticOutcome::Error,
        );
        assert_eq!(result, Ok("persisted"));
        let failure = sink.take_failure().expect("trace failure");
        assert!(failure.after_commit);
        assert_eq!(event_names(&read_events(&trace_path)), vec!["intent"]);
    }

    #[test]
    fn open_traced_success_emits_no_persistence_read_closure() {
        let (trace_dir, _writer, sink) = test_sink("open-success");
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("state.db");
        let store = crate::persistence::sqlite::SqliteStore::open_traced(
            &db_path,
            OptionalTraceSink { inner: Some(sink) },
        )
        .expect("open");
        assert!(store.path().exists());
        let events = read_events(&trace_dir.trace_dir().join("open-success.jsonl"));
        assert!(event_names(&events).is_empty());
    }

    #[test]
    fn open_traced_future_schema_emits_invocation_error() {
        let (trace_dir, _writer, sink) = test_sink("open-future-schema");
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("state.db");
        open_at(&db_path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch("PRAGMA user_version = 2").unwrap();
        }

        let error = crate::persistence::sqlite::SqliteStore::open_traced(
            &db_path,
            OptionalTraceSink { inner: Some(sink) },
        )
        .expect_err("future schema");
        assert!(matches!(
            error,
            crate::persistence::error::PersistenceError::FutureSchema {
                supported: SUPPORTED_SCHEMA_VERSION,
                observed: 2,
            }
        ));

        let events = read_events(&trace_dir.trace_dir().join("open-future-schema.jsonl"));
        assert_eq!(
            super::test_support::invocation_event_names(&events),
            vec!["error"]
        );
        assert!(event_names(&events).is_empty());
        let error_event = events
            .iter()
            .find(|event| event["category"] == "invocation" && event["event"] == "error")
            .expect("invocation.error");
        assert_eq!(error_event["phase"], "persistence");
        assert!(error_event["message"].is_string());
    }

    #[test]
    fn open_traced_open_failure_emits_invocation_error() {
        let (trace_dir, _writer, sink) = test_sink("open-failure");
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path();

        let error = crate::persistence::sqlite::SqliteStore::open_traced(
            db_path,
            OptionalTraceSink { inner: Some(sink) },
        )
        .expect_err("directory open");
        assert!(matches!(
            error,
            crate::persistence::error::PersistenceError::Open { .. }
        ));

        let events = read_events(&trace_dir.trace_dir().join("open-failure.jsonl"));
        assert_eq!(
            super::test_support::invocation_event_names(&events),
            vec!["error"]
        );
        assert!(event_names(&events).is_empty());
        let error_event = events
            .iter()
            .find(|event| event["category"] == "invocation" && event["event"] == "error")
            .expect("invocation.error");
        assert_eq!(error_event["phase"], "persistence");
        assert!(error_event["source_chain"].is_array());
    }
}
