//! Semantic ports between `loop-core` and integrations.
//!
//! These traits deliberately describe Loop Engine capabilities rather than
//! storage or process mechanics.  In particular, persistence implementations
//! own consistency boundaries and atomic writes; core never receives a
//! database transaction or a generic CRUD callback.

use crate::{
    ContextRecord, ContextRecordId, ControlRevision, DurableEvaluation, EvaluationFeedback,
    EvaluationRequest, EvaluationResult, HistoryEntry, InvocationId, Lifecycle,
    ProviderAssociation, ProviderSelector, Run, RunId, StateId, Timestamp, Transition,
    WaiterWrittenStatus, WorkSlotBinding, WorkSlotId, WorkSlotInvocation, Workflow,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Resolves a caller-facing selector once, at run creation time, into the
/// opaque identity that is retained by that run.
///
/// Implementations may consult aliases or configuration while resolving.  A
/// later operation on an existing run receives its stored
/// [`ProviderAssociation`] directly and must not resolve the selector again.
pub trait ProviderResolver {
    fn resolve(
        &self,
        selector: &ProviderSelector,
    ) -> Result<ProviderAssociation, ProviderResolutionError>;
}

/// Failure to turn a caller-facing provider selector into a durable
/// association.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderResolutionError {
    /// No configured provider matches the requested selector.
    UnknownSelector { selector: ProviderSelector },
    /// Configuration was present but could not describe a usable provider.
    InvalidConfiguration { code: String, message: String },
    /// The configured provider source could not be resolved at this time.
    Unavailable { code: String, message: String },
}

impl ProviderResolutionError {
    pub fn invalid_configuration(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidConfiguration {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Unavailable {
            code: code.into(),
            message: message.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownSelector { .. } => "unknown-provider",
            Self::InvalidConfiguration { .. } => "invalid-provider-configuration",
            Self::Unavailable { .. } => "provider-unavailable",
        }
    }
}

impl fmt::Display for ProviderResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSelector { selector } => {
                write!(
                    formatter,
                    "provider selector `{selector}` is not configured"
                )
            }
            Self::InvalidConfiguration { code, message } | Self::Unavailable { code, message } => {
                write!(formatter, "{code}: {message}")
            }
        }
    }
}

impl std::error::Error for ProviderResolutionError {}

/// Executes the two stateless semantic provider operations.
///
/// `evaluate` receives an engine-selected transition.  A provider can allow,
/// deny, or report unsupported; it cannot choose a target state or mutate a
/// run through this port.
pub trait ProviderGateway {
    /// Snapshot the provider workflow, including the `work_slots` catalog
    /// field (`id`, `state`, `event`) used by start to validate frozen
    /// `work_slot_bindings`.
    fn describe(&self, provider: &ProviderAssociation) -> Result<Workflow, ProviderError>;

    fn evaluate(
        &self,
        provider: &ProviderAssociation,
        request: EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError>;
}

/// Operational failure while invoking a provider or interpreting its
/// semantic response.
///
/// `Unsupported` is intentionally not an error variant: it is represented by
/// `Ok(EvaluationResult::Unsupported)` and is classified by core as an
/// operation error without durable semantic history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderError {
    /// The provider could not be reached or completed its operation.
    Execution { code: String, message: String },
    /// The provider did not finish within the configured integration limit.
    Timeout { message: String },
    /// The response could not be decoded as the provider response envelope.
    MalformedResponse { message: String },
    /// The response decoded but violated the semantic provider contract.
    InvalidResponse { message: String },
}

impl ProviderError {
    pub fn execution(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Execution {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout {
            message: message.into(),
        }
    }

    pub fn malformed_response(message: impl Into<String>) -> Self {
        Self::MalformedResponse {
            message: message.into(),
        }
    }

    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self::InvalidResponse {
            message: message.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Execution { .. } => "provider-execution-failed",
            Self::Timeout { .. } => "provider-timeout",
            Self::MalformedResponse { .. } => "provider-malformed-response",
            Self::InvalidResponse { .. } => "provider-invalid-response",
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Execution { code, message } => write!(formatter, "{code}: {message}"),
            Self::Timeout { message } => write!(formatter, "provider timeout: {message}"),
            Self::MalformedResponse { message } => {
                write!(formatter, "malformed provider response: {message}")
            }
            Self::InvalidResponse { message } => {
                write!(formatter, "invalid provider response: {message}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

/// Semantic input for atomic run creation.
///
/// Sequence and control-revision values are intentionally absent.  The
/// persistence adapter allocates the creation sequence and initial opaque
/// control revision while atomically creating the run and its creation
/// history entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateRunRequest {
    pub id: RunId,
    pub label: Option<String>,
    pub workflow: Workflow,
    pub provider_association: ProviderAssociation,
    pub initial_input: Value,
    pub initial_state: StateId,
    pub lifecycle: Lifecycle,
    pub created_at: Timestamp,
    /// Exact case-sensitive start alias (`ProviderSelector` string).
    pub provider: String,
    /// Recorded path from start composition. `None` only for migrated
    /// historical rows; new creates always pass `Some`.
    pub artifact_root: Option<String>,
    /// Slot-visit subjects persisted in the same transaction as run creation.
    /// Empty when the initial state is not a work slot.
    pub slot_subjects: Vec<(WorkSlotId, String)>,
}

impl CreateRunRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<RunId>,
        label: Option<String>,
        workflow: Workflow,
        provider_association: ProviderAssociation,
        initial_input: Value,
        initial_state: impl Into<StateId>,
        lifecycle: Lifecycle,
        created_at: Timestamp,
        provider: impl Into<String>,
        artifact_root: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label,
            workflow,
            provider_association,
            initial_input,
            initial_state: initial_state.into(),
            lifecycle,
            created_at,
            provider: provider.into(),
            artifact_root,
            slot_subjects: Vec::new(),
        }
    }

    pub fn with_slot_subjects(mut self, slot_subjects: Vec<(WorkSlotId, String)>) -> Self {
        self.slot_subjects = slot_subjects;
        self
    }
}

/// Result of an atomic run creation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateRunResult {
    pub run: Run,
    pub history: HistoryEntry,
}

/// Semantic input for an atomic context append.
///
/// The adapter allocates the context record's semantic sequence.  The
/// supplied record ID, kind, data, and timestamp remain caller-owned data.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppendContextRequest {
    pub run_id: RunId,
    pub record_id: ContextRecordId,
    pub kind: String,
    pub data: Value,
    pub created_at: Timestamp,
}

impl AppendContextRequest {
    pub fn new(
        run_id: impl Into<RunId>,
        record_id: impl Into<ContextRecordId>,
        kind: impl Into<String>,
        data: Value,
        created_at: Timestamp,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            record_id: record_id.into(),
            kind: kind.into(),
            data,
            created_at,
        }
    }
}

/// Result of an atomic context append.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppendContextResult {
    pub run: Run,
    pub context: ContextRecord,
    pub history: HistoryEntry,
}

/// Conditional input for a transition commit.
///
/// The persistence adapter must verify all of the following inside one
/// semantic mutation boundary: the run is active, its control revision still
/// equals `expected_control_revision`, and its authoritative current state
/// still equals `expected_source_state`.  It then commits the target state and
/// supplied lifecycle, advances revision, and appends one transition history
/// entry atomically.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitTransitionRequest {
    pub run_id: RunId,
    pub expected_control_revision: ControlRevision,
    pub expected_source_state: StateId,
    pub transition: Transition,
    /// Computed by loop-core from the target state's finality.  The adapter
    /// atomically applies this lifecycle together with the new current state
    /// after verifying the existing revision/active/source preconditions; it
    /// must not derive lifecycle itself.
    pub resulting_lifecycle: Lifecycle,
    /// Slot-visit subjects persisted in the same transaction as the committed
    /// target state. Empty when the target is not a work slot.
    pub slot_subjects: Vec<(WorkSlotId, String)>,
}

impl CommitTransitionRequest {
    pub fn new(
        run_id: impl Into<RunId>,
        expected_control_revision: ControlRevision,
        expected_source_state: impl Into<StateId>,
        transition: Transition,
        resulting_lifecycle: Lifecycle,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            expected_control_revision,
            expected_source_state: expected_source_state.into(),
            transition,
            resulting_lifecycle,
            slot_subjects: Vec::new(),
        }
    }

    pub fn with_slot_subjects(mut self, slot_subjects: Vec<(WorkSlotId, String)>) -> Self {
        self.slot_subjects = slot_subjects;
        self
    }
}

/// Result of a committed check-free transition or checked allow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitTransitionResult {
    pub run: Run,
    pub history: HistoryEntry,
}

/// Conditional input for an atomic checked-transition denial record.
///
/// A valid denial leaves the current state and control revision unchanged,
/// while allocating one semantic sequence for the durable denial history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordDenialRequest {
    pub run_id: RunId,
    pub expected_control_revision: ControlRevision,
    pub expected_source_state: StateId,
    pub transition: Transition,
    pub feedback: EvaluationFeedback,
}

impl RecordDenialRequest {
    pub fn new(
        run_id: impl Into<RunId>,
        expected_control_revision: ControlRevision,
        expected_source_state: impl Into<StateId>,
        transition: Transition,
        feedback: EvaluationFeedback,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            expected_control_revision,
            expected_source_state: expected_source_state.into(),
            transition,
            feedback,
        }
    }
}

/// Result of an atomically recorded checked denial.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordDenialResult {
    pub run: Run,
    pub evaluation: DurableEvaluation,
    pub history: HistoryEntry,
}

/// Input for an explicit terminal mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminateRequest {
    pub run_id: RunId,
}

impl TerminateRequest {
    pub fn new(run_id: impl Into<RunId>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

/// Result of an atomic termination.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminateResult {
    pub run: Run,
    pub history: HistoryEntry,
}

/// Semantic input for creating a running work-slot invocation record.
///
/// Inserts stored status `None`. Atomically appends
/// `HistoryAction::InvocationStarted` and increments `last_sequence` like
/// `append_context`. Reject if run missing/not active. Do not change
/// `control_revision`. `waiter_pid` is internal (on the stored running record,
/// not a user CLI flag).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateWorkSlotInvocationRequest {
    pub run_id: RunId,
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub binding: WorkSlotBinding,
    pub instruction_digest: String,
    pub subject: String,
    pub waiter_pid: u32,
    pub started_at: Timestamp,
    pub allowed_time_ms: u64,
}

impl CreateWorkSlotInvocationRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: impl Into<RunId>,
        invocation_id: impl Into<InvocationId>,
        slot_id: impl Into<WorkSlotId>,
        binding: WorkSlotBinding,
        instruction_digest: impl Into<String>,
        subject: impl Into<String>,
        waiter_pid: u32,
        started_at: Timestamp,
        allowed_time_ms: u64,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            invocation_id: invocation_id.into(),
            slot_id: slot_id.into(),
            binding,
            instruction_digest: instruction_digest.into(),
            subject: subject.into(),
            waiter_pid,
            started_at,
            allowed_time_ms,
        }
    }
}

/// Result of creating a running work-slot invocation record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateWorkSlotInvocationResult {
    pub invocation: WorkSlotInvocation,
    pub history: HistoryEntry,
}

/// Semantic input for a waiter terminal write.
///
/// CAS only if stored status is still `None`. Appends
/// `HistoryAction::InvocationStatusChanged` with waiter-written status
/// (`succeeded`/`failed` only). Conflict/reject if already terminal. Waiter
/// does not write `overrun`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompleteWorkSlotInvocationRequest {
    pub run_id: RunId,
    pub invocation_id: InvocationId,
    pub status: WaiterWrittenStatus,
    pub exit_code: i32,
    pub completed_at: Timestamp,
}

impl CompleteWorkSlotInvocationRequest {
    pub fn new(
        run_id: impl Into<RunId>,
        invocation_id: impl Into<InvocationId>,
        status: WaiterWrittenStatus,
        exit_code: i32,
        completed_at: Timestamp,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            invocation_id: invocation_id.into(),
            status,
            exit_code,
            completed_at,
        }
    }
}

/// Result of a waiter terminal write.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompleteWorkSlotInvocationResult {
    pub invocation: WorkSlotInvocation,
    pub history: HistoryEntry,
}

/// Input for a checked-evaluation snapshot.
///
/// The transition is the exact edge selected by core from the authoritative
/// workflow.  Persistence re-verifies, at one consistent read boundary, that
/// the run is active and that this exact edge is still available.  The
/// boundary ends before the provider gateway is called.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckedEvaluationSnapshotRequest {
    pub run_id: RunId,
    pub transition: Transition,
}

impl CheckedEvaluationSnapshotRequest {
    pub fn new(run_id: impl Into<RunId>, transition: Transition) -> Self {
        Self {
            run_id: run_id.into(),
            transition,
        }
    }
}

/// Consistent durable inputs captured for one checked provider evaluation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckedEvaluationSnapshot {
    pub run: Run,
    /// Explicitly names the revision observed at the snapshot boundary.  It
    /// is equal to `run.control_revision`; it is repeated to make the
    /// conditional commit token impossible to overlook at the call site.
    pub observed_control_revision: ControlRevision,
    pub transition: Transition,
    pub context: Vec<ContextRecord>,
    /// All ordered durable allow/deny records for checked transitions in the
    /// run. Core derives the exact-transition lineage for `transition` from
    /// this collection; persistence does not apply workflow lineage policy.
    pub checked_evaluations: Vec<DurableEvaluation>,
}

/// Durable data needed by core to construct the `show` projection.
///
/// The persistence adapter returns all ordered checked-transition records;
/// core derives exact-transition lineage and the latest-evaluation projection.
/// Records for transitions that are no longer requestable are intentionally
/// retained here so `show` can expose them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShowData {
    pub run: Run,
    pub context: Vec<ContextRecord>,
    pub checked_evaluations: Vec<DurableEvaluation>,
}

/// Stable list projection supplied by persistence.  Core does not need to
/// load history or invoke a provider to render this discovery view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunSummary {
    pub id: RunId,
    pub label: Option<String>,
    pub workflow_id: crate::WorkflowId,
    pub lifecycle: Lifecycle,
    pub current_state: StateId,
    pub provider: Option<String>,
    pub artifact_root: Option<String>,
}

impl From<&Run> for RunSummary {
    fn from(run: &Run) -> Self {
        Self {
            id: run.id.clone(),
            label: run.label.clone(),
            workflow_id: run.workflow.id.clone(),
            lifecycle: run.lifecycle,
            current_state: run.current_state.clone(),
            provider: None,
            artifact_root: None,
        }
    }
}

/// Semantic durable capabilities required by core operations.
///
/// Implementations must enforce the atomicity and conditional preconditions
/// documented on the request types.  No method exposes a database
/// transaction, generic insert/update/delete operation, or callback running
/// inside one.  Provider execution is performed outside persistence
/// boundaries; a checked-evaluation snapshot ends before the gateway call.
pub trait Persistence {
    /// Atomically create a run and its `run created` history entry.
    fn create_run(&self, request: CreateRunRequest) -> Result<CreateRunResult, PersistenceError>;

    /// Atomically verify activity, append one immutable context record, and
    /// append its matching history entry.  This does not advance control
    /// revision.
    fn append_context(
        &self,
        request: AppendContextRequest,
    ) -> Result<AppendContextResult, PersistenceError>;

    /// Atomically conditionally commit a transition and its history entry.
    /// The transition kind determines whether the committed entry represents
    /// a check-free edge or a checked allow.
    fn commit_transition(
        &self,
        request: CommitTransitionRequest,
    ) -> Result<CommitTransitionResult, PersistenceError>;

    /// Atomically conditionally record one checked denial without changing
    /// state or control revision.
    fn record_denial(
        &self,
        request: RecordDenialRequest,
    ) -> Result<RecordDenialResult, PersistenceError>;

    /// Atomically verify activity, mark the run terminated, advance control
    /// revision, and append termination history.
    fn terminate(&self, request: TerminateRequest) -> Result<TerminateResult, PersistenceError>;

    /// Load the authoritative current run state.  Missing runs are returned
    /// as `PersistenceError::NotFound`, not as an empty successful value.
    fn load_authoritative_run(&self, run_id: &RunId) -> Result<Run, PersistenceError>;

    /// Return stable discovery projections for all runs.
    fn list_runs(&self) -> Result<Vec<RunSummary>, PersistenceError>;

    /// Return immutable context records in semantic append-sequence order.
    fn load_context_records(&self, run_id: &RunId) -> Result<Vec<ContextRecord>, PersistenceError>;

    /// Return semantic history in sequence order.
    fn load_history(&self, run_id: &RunId) -> Result<Vec<HistoryEntry>, PersistenceError>;

    /// Return all ordered durable allow/deny records for checked transitions
    /// in a run. Unsupported, failed, stale, and uncommitted attempts are
    /// absent by construction. Core owns exact-transition lineage derivation,
    /// so this read deliberately does not filter by a requested transition.
    fn load_checked_evaluations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<DurableEvaluation>, PersistenceError>;

    /// Capture one consistent checked-evaluation boundary, including the
    /// authoritative run, observed revision, ordered context, and all ordered
    /// durable checked-transition records. Core derives exact-transition
    /// lineage from the returned records. The boundary must re-verify active
    /// lifecycle and exact transition availability before returning.
    fn load_checked_evaluation_snapshot(
        &self,
        request: CheckedEvaluationSnapshotRequest,
    ) -> Result<CheckedEvaluationSnapshot, PersistenceError>;

    /// Return the durable inputs needed for `show` without invoking a
    /// provider.  Checked evaluations include transitions no longer
    /// requestable so core can preserve latest results across revision edges.
    fn load_show_data(&self, run_id: &RunId) -> Result<ShowData, PersistenceError>;

    /// Create a running engine-authored invocation record (stored status
    /// `None`) and append `HistoryAction::InvocationStarted`.
    fn create_work_slot_invocation(
        &self,
        request: CreateWorkSlotInvocationRequest,
    ) -> Result<CreateWorkSlotInvocationResult, PersistenceError>;

    /// Waiter CAS write of terminal `succeeded`/`failed` plus `exit_code`.
    /// Only if no waiter-written status yet. Appends
    /// `HistoryAction::InvocationStatusChanged`.
    fn complete_work_slot_invocation(
        &self,
        request: CompleteWorkSlotInvocationRequest,
    ) -> Result<CompleteWorkSlotInvocationResult, PersistenceError>;

    /// Load the current engine-minted subject for a slot, if any.
    fn get_current_slot_subject(
        &self,
        run_id: &RunId,
        slot_id: &WorkSlotId,
    ) -> Result<Option<String>, PersistenceError>;

    /// Replace the current engine-minted subject for a slot.
    fn set_current_slot_subject(
        &self,
        run_id: &RunId,
        slot_id: &WorkSlotId,
        subject: String,
    ) -> Result<(), PersistenceError>;

    /// Load engine-authored invocation rows for a run. This is a read, not a
    /// general update.
    fn load_work_slot_invocations(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<WorkSlotInvocation>, PersistenceError>;
}

/// A semantic rejection produced while enforcing a persistence precondition.
///
/// Rejections are understood requests that do not satisfy lifecycle/workflow
/// rules.  Core maps them to the `rejected` operation outcome.  Staleness and
/// conditional-write failures use [`PersistenceConflict`] instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceRejection {
    RunNotActive { run_id: RunId, lifecycle: Lifecycle },
}

impl PersistenceRejection {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::RunNotActive { .. } => "run-not-active",
        }
    }
}

impl fmt::Display for PersistenceRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunNotActive { run_id, lifecycle } => {
                write!(formatter, "run `{run_id}` is not active ({lifecycle:?})")
            }
        }
    }
}

/// A conditional-control conflict.  Core classifies these as operation
/// errors/staleness, never as an ordinary workflow rejection, and does not
/// retry them automatically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceConflict {
    ControlRevisionMismatch {
        expected: ControlRevision,
        observed: ControlRevision,
    },
    SourceStateMismatch {
        expected: StateId,
        observed: StateId,
    },
    LifecycleMismatch {
        expected: Lifecycle,
        observed: Lifecycle,
    },
    ExactTransitionUnavailable {
        expected: Transition,
        observed_current_state: StateId,
    },
    InvocationAlreadyTerminal {
        invocation_id: InvocationId,
    },
}

impl PersistenceConflict {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ControlRevisionMismatch { .. } => "control-revision-conflict",
            Self::SourceStateMismatch { .. } => "source-state-conflict",
            Self::LifecycleMismatch { .. } => "lifecycle-conflict",
            Self::ExactTransitionUnavailable { .. } => "transition-stale",
            Self::InvocationAlreadyTerminal { .. } => "invocation-already-terminal",
        }
    }
}

impl fmt::Display for PersistenceConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlRevisionMismatch { expected, observed } => write!(
                formatter,
                "control revision changed (expected {expected}, observed {observed})"
            ),
            Self::SourceStateMismatch { expected, observed } => write!(
                formatter,
                "authoritative source state changed (expected `{expected}`, observed `{observed}`)"
            ),
            Self::LifecycleMismatch { expected, observed } => write!(
                formatter,
                "lifecycle changed (expected {expected:?}, observed {observed:?})"
            ),
            Self::ExactTransitionUnavailable {
                expected,
                observed_current_state,
            } => write!(
                formatter,
                "transition `{}` from `{}` is no longer available from `{observed_current_state}`",
                expected.event, expected.source
            ),
            Self::InvocationAlreadyTerminal { invocation_id } => write!(
                formatter,
                "invocation `{invocation_id}` already has a waiter-written terminal status"
            ),
        }
    }
}

/// Adapter failure that is neither a workflow rejection nor a conditional
/// staleness conflict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersistenceFailure {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl PersistenceFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl fmt::Display for PersistenceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

/// Error categories emitted by semantic persistence operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceError {
    NotFound { run_id: RunId },
    Rejected(PersistenceRejection),
    Conflict(PersistenceConflict),
    Failure(PersistenceFailure),
}

impl PersistenceError {
    pub fn not_found(run_id: impl Into<RunId>) -> Self {
        Self::NotFound {
            run_id: run_id.into(),
        }
    }

    pub fn rejected(reason: PersistenceRejection) -> Self {
        Self::Rejected(reason)
    }

    pub fn conflict(reason: PersistenceConflict) -> Self {
        Self::Conflict(reason)
    }

    pub fn failure(failure: PersistenceFailure) -> Self {
        Self::Failure(failure)
    }

    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    pub const fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }

    pub const fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict(_))
    }

    pub const fn is_staleness_conflict(&self) -> bool {
        self.is_conflict()
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "run-not-found",
            Self::Rejected(reason) => reason.code(),
            Self::Conflict(reason) => reason.code(),
            Self::Failure(_) => "persistence-failure",
        }
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { run_id } => write!(formatter, "run `{run_id}` was not found"),
            Self::Rejected(reason) => reason.fmt(formatter),
            Self::Conflict(reason) => reason.fmt(formatter),
            Self::Failure(failure) => failure.fmt(formatter),
        }
    }
}

impl std::error::Error for PersistenceError {}

/// Arguments needed to exec `loop-engine wait-invocation` without writing stdin yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaiterSpawnArgs {
    pub database: std::path::PathBuf,
    pub run_id: RunId,
    pub invocation_id: InvocationId,
}

impl WaiterSpawnArgs {
    pub fn new(
        database: impl Into<std::path::PathBuf>,
        run_id: impl Into<RunId>,
        invocation_id: impl Into<InvocationId>,
    ) -> Self {
        Self {
            database: database.into(),
            run_id: run_id.into(),
            invocation_id: invocation_id.into(),
        }
    }
}

/// A spawned waiter that has not yet received its envelope and has not been waited on.
pub struct StartedWaiter<H> {
    pub pid: u32,
    pub handle: H,
}

impl<H> StartedWaiter<H> {
    pub fn new(pid: u32, handle: H) -> Self {
        Self { pid, handle }
    }
}

/// Failure while spawning a waiter or writing its envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessError {
    pub code: String,
    pub message: String,
}

impl ProcessError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProcessError {}

/// Process port for work-slot waiter delegation.
///
/// Core must not call `std::process::Command` directly. Callers supply waiter
/// liveness; spawn must not waitpid or write stdin until `send_envelope_and_detach`.
pub trait WorkSlotProcess {
    /// Implementation-owned handle that can receive the waiter envelope.
    type Handle;

    fn waiter_alive(&self, pid: u32) -> bool;

    /// Spawn `loop-engine wait-invocation RUN_ID INVOCATION_ID` with piped stdin.
    /// Do not waitpid. Do not write stdin yet.
    fn spawn_wait_invocation(
        &self,
        args: WaiterSpawnArgs,
    ) -> std::result::Result<StartedWaiter<Self::Handle>, ProcessError>;

    /// Write envelope bytes to waiter stdin and detach (close stdin). Do not waitpid.
    fn send_envelope_and_detach(
        &self,
        waiter: StartedWaiter<Self::Handle>,
        envelope_json: &[u8],
    ) -> std::result::Result<(), ProcessError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SemanticSequence, State, TransitionKind};
    use serde_json::json;

    fn workflow() -> Workflow {
        Workflow::new(
            "stub-workflow",
            "start",
            vec![
                State::new("start", "Start", "Do work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::checked("start", "finish", "done")],
        )
    }

    fn association() -> ProviderAssociation {
        ProviderAssociation::new(json!({"opaque": true}))
    }

    struct StubResolver;

    impl ProviderResolver for StubResolver {
        fn resolve(
            &self,
            _selector: &ProviderSelector,
        ) -> Result<ProviderAssociation, ProviderResolutionError> {
            Ok(association())
        }
    }

    struct StubGateway;

    impl ProviderGateway for StubGateway {
        fn describe(&self, _provider: &ProviderAssociation) -> Result<Workflow, ProviderError> {
            Ok(workflow())
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            request: EvaluationRequest,
        ) -> Result<EvaluationResult, ProviderError> {
            assert_eq!(request.transition.kind, TransitionKind::Checked);
            Ok(EvaluationResult::Allow)
        }
    }

    /// A deliberately minimal persistence fake.  It proves every semantic
    /// capability can be implemented without exposing backend mechanics;
    /// operation behavior belongs to later core/integration tests.
    struct StubPersistence;

    fn unavailable<T>() -> Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "stub",
            "stub persistence has no storage",
        )))
    }

    impl Persistence for StubPersistence {
        fn create_run(
            &self,
            _request: CreateRunRequest,
        ) -> Result<CreateRunResult, PersistenceError> {
            unavailable()
        }

        fn append_context(
            &self,
            _request: AppendContextRequest,
        ) -> Result<AppendContextResult, PersistenceError> {
            unavailable()
        }

        fn commit_transition(
            &self,
            _request: CommitTransitionRequest,
        ) -> Result<CommitTransitionResult, PersistenceError> {
            unavailable()
        }

        fn record_denial(
            &self,
            _request: RecordDenialRequest,
        ) -> Result<RecordDenialResult, PersistenceError> {
            unavailable()
        }

        fn terminate(
            &self,
            _request: TerminateRequest,
        ) -> Result<TerminateResult, PersistenceError> {
            unavailable()
        }

        fn load_authoritative_run(&self, _run_id: &RunId) -> Result<Run, PersistenceError> {
            unavailable()
        }

        fn list_runs(&self) -> Result<Vec<RunSummary>, PersistenceError> {
            unavailable()
        }

        fn load_context_records(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<ContextRecord>, PersistenceError> {
            unavailable()
        }

        fn load_history(&self, _run_id: &RunId) -> Result<Vec<HistoryEntry>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluations(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<DurableEvaluation>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluation_snapshot(
            &self,
            _request: CheckedEvaluationSnapshotRequest,
        ) -> Result<CheckedEvaluationSnapshot, PersistenceError> {
            unavailable()
        }

        fn load_show_data(&self, _run_id: &RunId) -> Result<ShowData, PersistenceError> {
            unavailable()
        }

        fn create_work_slot_invocation(
            &self,
            _request: CreateWorkSlotInvocationRequest,
        ) -> Result<CreateWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn complete_work_slot_invocation(
            &self,
            _request: CompleteWorkSlotInvocationRequest,
        ) -> Result<CompleteWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn get_current_slot_subject(
            &self,
            _run_id: &RunId,
            _slot_id: &crate::WorkSlotId,
        ) -> Result<Option<String>, PersistenceError> {
            unavailable()
        }

        fn set_current_slot_subject(
            &self,
            _run_id: &RunId,
            _slot_id: &crate::WorkSlotId,
            _subject: String,
        ) -> Result<(), PersistenceError> {
            unavailable()
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<crate::WorkSlotInvocation>, PersistenceError> {
            unavailable()
        }
    }

    #[test]
    fn provider_ports_are_usable_with_minimal_fakes() {
        let resolver = StubResolver;
        let association = resolver.resolve(&ProviderSelector::new("stub")).unwrap();
        let gateway = StubGateway;
        let described = gateway.describe(&association).unwrap();
        let transition = described.transitions[0].clone();
        let result = gateway
            .evaluate(
                &association,
                EvaluationRequest::new(
                    described,
                    json!({"objective": "test"}),
                    vec![],
                    transition,
                    vec![],
                ),
            )
            .unwrap();

        assert!(matches!(result, EvaluationResult::Allow));
    }

    #[test]
    fn persistence_ports_are_usable_with_semantic_requests_and_classified_conflicts() {
        let persistence = StubPersistence;
        let request = CreateRunRequest::new(
            "run-1",
            Some("contract".to_owned()),
            workflow(),
            association(),
            json!({"input": true}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "stub",
            Some("/allocated/run-1".to_owned()),
        );
        let error = persistence.create_run(request).unwrap_err();
        assert_eq!(error.code(), "persistence-failure");

        let conflict = PersistenceError::conflict(PersistenceConflict::ControlRevisionMismatch {
            expected: ControlRevision::from_u64(1),
            observed: ControlRevision::from_u64(2),
        });
        assert!(conflict.is_conflict());
        assert!(conflict.is_staleness_conflict());
        assert_eq!(conflict.code(), "control-revision-conflict");

        let rejection = PersistenceError::rejected(PersistenceRejection::RunNotActive {
            run_id: RunId::new("run-1"),
            lifecycle: Lifecycle::Final,
        });
        assert!(rejection.is_rejected());
        assert!(!rejection.is_conflict());
    }

    #[test]
    fn commit_transition_contract_carries_final_lifecycle_for_final_target() {
        let workflow = workflow();
        let transition = workflow.transitions[0].clone();
        let target_is_final = workflow
            .states
            .iter()
            .find(|state| state.id.as_str() == transition.target.as_str())
            .expect("transition target is present in the workflow")
            .is_final;
        assert!(target_is_final);

        let request = CommitTransitionRequest::new(
            "run-1",
            ControlRevision::from_u64(1),
            "start",
            transition,
            if target_is_final {
                Lifecycle::Final
            } else {
                Lifecycle::Active
            },
        );

        assert_eq!(request.resulting_lifecycle, Lifecycle::Final);
    }

    #[test]
    fn snapshot_contract_keeps_observed_revision_and_durable_records_explicit() {
        let transition = Transition::checked("start", "finish", "done");
        let unrelated_transition = Transition::checked("start", "other", "done");
        let snapshot = CheckedEvaluationSnapshot {
            run: Run::new(
                "run-1",
                None,
                workflow(),
                association(),
                json!({}),
                "start",
                Lifecycle::Active,
                ControlRevision::from_u64(4),
                SemanticSequence::new(2),
                Timestamp::from_unix_millis(2),
            ),
            observed_control_revision: ControlRevision::from_u64(4),
            transition,
            context: vec![],
            checked_evaluations: vec![
                DurableEvaluation::deny(
                    unrelated_transition,
                    EvaluationFeedback::new("other", "Other transition"),
                    SemanticSequence::new(3),
                    Timestamp::from_unix_millis(3),
                ),
                DurableEvaluation::allow(
                    Transition::checked("start", "finish", "done"),
                    SemanticSequence::new(4),
                    Timestamp::from_unix_millis(4),
                ),
            ],
        };

        assert_eq!(
            snapshot.observed_control_revision,
            snapshot.run.control_revision
        );
        assert_eq!(snapshot.checked_evaluations.len(), 2);
        assert_eq!(
            snapshot.checked_evaluations[0].sequence,
            SemanticSequence::new(3)
        );
        assert_eq!(
            snapshot.checked_evaluations[1].sequence,
            SemanticSequence::new(4)
        );
    }
}
