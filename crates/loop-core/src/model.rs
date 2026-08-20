//! Domain and provider-bound value types for Loop Engine.
//!
//! These types intentionally describe semantic workflow data only.  They do
//! not contain persistence, process, configuration, or CLI concerns.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// JSON data supplied to a run or attached as context.
///
/// The alias is intentionally just `serde_json::Value`: core stores and
/// transports this data but does not assign meaning to its shape.
pub type JsonValue = Value;

macro_rules! string_identifier {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Construct an identifier without imposing workflow-specific
            /// validation.  Structural validation belongs to the workflow
            /// validation layer.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_identifier!(
    /// Stable identity of a workflow definition.
    WorkflowId
);
string_identifier!(
    /// Stable identity of a state in a workflow.
    StateId
);
string_identifier!(
    /// Caller-requested event identity.
    EventId
);
string_identifier!(
    /// Stable identity of a work slot in a provider catalog.
    ///
    /// Slot identity is a string slot_id. Use the existing `string_identifier!` newtype pattern: `WorkSlotId`.
    WorkSlotId
);
string_identifier!(
    /// Stable identity of a durable workflow run.
    RunId
);
string_identifier!(
    /// Stable identity of an appended context record.
    ContextRecordId
);
string_identifier!(
    /// Stable identity of an engine-authored work-slot invocation.
    InvocationId
);

/// Monotonic ordering assigned to successful semantic actions in one run.
///
/// Ordering, rather than wall-clock time, is the authority for history and
/// evaluation lineage.  The persistence adapter allocates values; core does
/// not infer or increment them.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct SemanticSequence(u64);

impl SemanticSequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for SemanticSequence {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<SemanticSequence> for u64 {
    fn from(value: SemanticSequence) -> Self {
        value.as_u64()
    }
}

impl fmt::Display for SemanticSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Timestamp metadata attached to durable records.
///
/// The sequence is the semantic ordering authority.  This value is an
/// adapter-neutral Unix timestamp in milliseconds and carries no policy
/// meaning in core.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_unix_millis(value: i64) -> Self {
        Self(value)
    }

    pub const fn as_unix_millis(self) -> i64 {
        self.0
    }
}

impl From<i64> for Timestamp {
    fn from(value: i64) -> Self {
        Self::from_unix_millis(value)
    }
}

impl From<Timestamp> for i64 {
    fn from(value: Timestamp) -> Self {
        value.as_unix_millis()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The lifecycle of a run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Active,
    Final,
    Terminated,
}

impl Lifecycle {
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Final | Self::Terminated)
    }
}

/// A workflow state and the instructions exposed to its caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct State {
    pub id: StateId,
    pub title: String,
    pub instructions: String,
    #[serde(rename = "final")]
    pub is_final: bool,
}

impl State {
    pub fn new(
        id: impl Into<StateId>,
        title: impl Into<String>,
        instructions: impl Into<String>,
        is_final: bool,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            instructions: instructions.into(),
            is_final,
        }
    }
}

/// Whether an engine-selected transition requires provider evaluation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransitionKind {
    Checked,
    CheckFree,
}

impl TransitionKind {
    pub const fn is_checked(self) -> bool {
        matches!(self, Self::Checked)
    }

    pub const fn is_check_free(self) -> bool {
        matches!(self, Self::CheckFree)
    }
}

/// One engine-resolved edge in a workflow graph.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Transition {
    pub source: StateId,
    pub event: EventId,
    pub target: StateId,
    pub kind: TransitionKind,
}

impl Transition {
    pub fn new(
        source: impl Into<StateId>,
        event: impl Into<EventId>,
        target: impl Into<StateId>,
        kind: TransitionKind,
    ) -> Self {
        Self {
            source: source.into(),
            event: event.into(),
            target: target.into(),
            kind,
        }
    }

    pub fn checked(
        source: impl Into<StateId>,
        event: impl Into<EventId>,
        target: impl Into<StateId>,
    ) -> Self {
        Self::new(source, event, target, TransitionKind::Checked)
    }

    pub fn check_free(
        source: impl Into<StateId>,
        event: impl Into<EventId>,
        target: impl Into<StateId>,
    ) -> Self {
        Self::new(source, event, target, TransitionKind::CheckFree)
    }

    /// Exact lineage identity is source state plus event ID.  The target and
    /// kind remain part of the transition snapshot and are intentionally not
    /// ignored when serializing or displaying the selected action.
    pub fn same_lineage(&self, other: &Self) -> bool {
        self.source == other.source && self.event == other.event
    }
}

/// Catalog entry type `WorkSlot` with `id`, `state`, `event`, and optional
/// `stdin_context_kinds`. Omitted or empty kinds mean no extra stdin context.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkSlot {
    pub id: WorkSlotId,
    pub state: StateId,
    pub event: EventId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdin_context_kinds: Vec<String>,
}

impl WorkSlot {
    pub fn new(
        id: impl Into<WorkSlotId>,
        state: impl Into<StateId>,
        event: impl Into<EventId>,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.into(),
            event: event.into(),
            stdin_context_kinds: Vec::new(),
        }
    }

    pub fn with_stdin_context_kinds(mut self, kinds: Vec<String>) -> Self {
        self.stdin_context_kinds = kinds;
        self
    }
}

/// Binding value type with exactly `{command, args}`. `command: String`. `args: Vec<String>` — the same argv list type loop-integrations already uses for process argument lists (`ProviderDefinition.args` / `ProviderInvocation.args`). `#[serde(deny_unknown_fields)]`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkSlotBinding {
    pub command: String,
    pub args: Vec<String>,
}

impl WorkSlotBinding {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
        }
    }
}

/// Waiter-written terminal status. Stored values are ONLY `succeeded` and
/// `failed`. Overrun is projected by the reader overlay and is NOT stored.
/// The waiter does not write `overrun`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaiterWrittenStatus {
    Succeeded,
    Failed,
}

/// Reader-overlay projection of an invocation. Overrun is NOT stored.
///
/// Stored waiter-written statuses are ONLY `succeeded` and `failed`.
/// `running` means the waiter is still alive. Overlay-overrun is terminal
/// for retry: invoke MUST NOT reject as already-running. If waiter pid is
/// gone and no terminal status was written, project `failed` (crash residual).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectedInvocationStatus {
    Running,
    Succeeded,
    Failed,
    Overrun,
}

/// Inner worker identity and process exit copied from a helper `summary.json`.
///
/// Identity is `command` plus argv order. There is no label field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InnerWorker {
    pub command: String,
    pub args: Vec<String>,
    pub exit_code: i32,
}

impl InnerWorker {
    pub fn new(command: impl Into<String>, args: Vec<String>, exit_code: i32) -> Self {
        Self {
            command: command.into(),
            args,
            exit_code,
        }
    }
}

/// Engine-authored work-slot invocation record.
///
/// `status` is the stored waiter-written status: `None` means not yet written.
/// `waiter_pid` is internal (on the stored running record, not a user CLI flag).
/// `inner_workers` is empty until waiter complete copies a well-formed summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkSlotInvocation {
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub binding: WorkSlotBinding,
    pub instruction_digest: String,
    pub subject: String,
    pub waiter_pid: u32,
    pub started_at: Timestamp,
    pub allowed_time_ms: u64,
    pub status: Option<WaiterWrittenStatus>,
    pub exit_code: Option<i32>,
    pub completed_at: Option<Timestamp>,
    pub capture_dir: String,
    pub inner_workers: Vec<InnerWorker>,
}

impl WorkSlotInvocation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        invocation_id: impl Into<InvocationId>,
        slot_id: impl Into<WorkSlotId>,
        binding: WorkSlotBinding,
        instruction_digest: impl Into<String>,
        subject: impl Into<String>,
        waiter_pid: u32,
        started_at: Timestamp,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        exit_code: Option<i32>,
        completed_at: Option<Timestamp>,
        capture_dir: impl Into<String>,
        inner_workers: Vec<InnerWorker>,
    ) -> Self {
        Self {
            invocation_id: invocation_id.into(),
            slot_id: slot_id.into(),
            binding,
            instruction_digest: instruction_digest.into(),
            subject: subject.into(),
            waiter_pid,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
            capture_dir: capture_dir.into(),
            inner_workers,
        }
    }
}

/// A complete, provider-described workflow graph.
///
/// `work_slots` is declared only on describe as a field of the snapshotted
/// workflow. The catalog is not supplied in caller initial_input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Workflow {
    pub id: WorkflowId,
    pub initial_state: StateId,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_slots: Vec<WorkSlot>,
}

impl Workflow {
    pub fn new(
        id: impl Into<WorkflowId>,
        initial_state: impl Into<StateId>,
        states: Vec<State>,
        transitions: Vec<Transition>,
    ) -> Self {
        Self {
            id: id.into(),
            initial_state: initial_state.into(),
            states,
            transitions,
            work_slots: Vec::new(),
        }
    }

    pub fn with_work_slots(mut self, slots: Vec<WorkSlot>) -> Self {
        self.work_slots = slots;
        self
    }
}

/// An opaque caller/provider-selected provider name or alias.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProviderSelector(String);

impl ProviderSelector {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<String> for ProviderSelector {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ProviderSelector {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for ProviderSelector {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ProviderSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A resolved provider identity retained by a run.
///
/// Core deliberately does not know whether the value represents a command,
/// arguments, a remote endpoint, or another integration-specific identity.
/// Integrations may use any JSON shape that they can durably resolve and
/// later interpret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProviderAssociation(Value);

impl ProviderAssociation {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    pub fn as_json(&self) -> &Value {
        &self.0
    }

    pub fn into_json(self) -> Value {
        self.0
    }
}

impl From<Value> for ProviderAssociation {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

/// An immutable context record supplied by a caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextRecord {
    pub id: ContextRecordId,
    /// Opaque workflow-defined context kind.
    pub kind: String,
    /// Opaque JSON context payload.
    pub data: Value,
    pub sequence: SemanticSequence,
    pub created_at: Timestamp,
}

impl ContextRecord {
    pub fn new(
        id: impl Into<ContextRecordId>,
        kind: impl Into<String>,
        data: Value,
        sequence: SemanticSequence,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            data,
            sequence,
            created_at,
        }
    }
}

/// An immutable workflow run and its authoritative current control state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Run {
    pub id: RunId,
    pub label: Option<String>,
    /// Creation-time immutable workflow snapshot.
    pub workflow: Workflow,
    pub provider_association: ProviderAssociation,
    /// Immutable opaque JSON supplied at run creation.
    pub initial_input: Value,
    pub current_state: StateId,
    pub lifecycle: Lifecycle,
    /// Opaque to core semantics; persistence compares it conditionally.
    pub control_revision: ControlRevision,
    pub last_sequence: SemanticSequence,
    pub created_at: Timestamp,
}

impl Run {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<RunId>,
        label: Option<String>,
        workflow: Workflow,
        provider_association: ProviderAssociation,
        initial_input: Value,
        current_state: impl Into<StateId>,
        lifecycle: Lifecycle,
        control_revision: ControlRevision,
        last_sequence: SemanticSequence,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id: id.into(),
            label,
            workflow,
            provider_association,
            initial_input,
            current_state: current_state.into(),
            lifecycle,
            control_revision,
            last_sequence,
            created_at,
        }
    }
}

/// Internal control token used for conditional workflow mutations.
///
/// The newtype prevents accidental mixing with semantic sequence numbers.
/// Its numeric representation is an adapter detail; core only compares and
/// transports values.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct ControlRevision(u64);

impl ControlRevision {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for ControlRevision {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl From<ControlRevision> for u64 {
    fn from(value: ControlRevision) -> Self {
        value.as_u64()
    }
}

impl fmt::Display for ControlRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Actionable feedback returned when a checked transition is denied.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationFeedback {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl EvaluationFeedback {
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

/// The two evaluation results that are durable as checked-transition
/// lineage.  `Unsupported` is deliberately absent: it is an operation error,
/// not a semantic evaluation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum DurableEvaluationResult {
    Allow,
    Deny { feedback: EvaluationFeedback },
}

impl DurableEvaluationResult {
    pub const fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub const fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    pub fn feedback(&self) -> Option<&EvaluationFeedback> {
        match self {
            Self::Allow => None,
            Self::Deny { feedback } => Some(feedback),
        }
    }
}

/// One durable allow/deny evaluation for one exact checked transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurableEvaluation {
    pub transition: Transition,
    pub result: DurableEvaluationResult,
    pub sequence: SemanticSequence,
    pub occurred_at: Timestamp,
}

impl DurableEvaluation {
    pub fn allow(
        transition: Transition,
        sequence: SemanticSequence,
        occurred_at: Timestamp,
    ) -> Self {
        Self {
            transition,
            result: DurableEvaluationResult::Allow,
            sequence,
            occurred_at,
        }
    }

    pub fn deny(
        transition: Transition,
        feedback: EvaluationFeedback,
        sequence: SemanticSequence,
        occurred_at: Timestamp,
    ) -> Self {
        Self {
            transition,
            result: DurableEvaluationResult::Deny { feedback },
            sequence,
            occurred_at,
        }
    }

    pub fn is_allow(&self) -> bool {
        self.result.is_allow()
    }

    pub fn is_deny(&self) -> bool {
        self.result.is_deny()
    }

    pub fn feedback(&self) -> Option<&EvaluationFeedback> {
        self.result.feedback()
    }
}

/// Name emphasizing that these records are the prior lineage supplied to a
/// provider.  It is an alias rather than a second representation.
pub type PriorEvaluation = DurableEvaluation;

/// A provider-bound request to evaluate one engine-selected checked
/// transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationRequest {
    pub workflow: Workflow,
    pub initial_input: Value,
    pub context: Vec<ContextRecord>,
    pub transition: Transition,
    pub prior_evaluations: Vec<DurableEvaluation>,
}

impl EvaluationRequest {
    pub fn new(
        workflow: Workflow,
        initial_input: Value,
        context: Vec<ContextRecord>,
        transition: Transition,
        prior_evaluations: Vec<DurableEvaluation>,
    ) -> Self {
        Self {
            workflow,
            initial_input,
            context,
            transition,
            prior_evaluations,
        }
    }
}

/// Semantic result returned by a provider for the selected transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum EvaluationResult {
    Allow,
    Deny { feedback: EvaluationFeedback },
    Unsupported,
}

impl EvaluationResult {
    pub const fn allow() -> Self {
        Self::Allow
    }

    pub fn deny(feedback: EvaluationFeedback) -> Self {
        Self::Deny { feedback }
    }

    pub const fn unsupported() -> Self {
        Self::Unsupported
    }

    pub const fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub const fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported)
    }

    pub fn feedback(&self) -> Option<&EvaluationFeedback> {
        match self {
            Self::Deny { feedback } => Some(feedback),
            Self::Allow | Self::Unsupported => None,
        }
    }
}

/// Outcome stored in a transition history entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum TransitionHistoryOutcome {
    Committed,
    Denied { feedback: EvaluationFeedback },
}

impl TransitionHistoryOutcome {
    pub const fn is_committed(&self) -> bool {
        matches!(self, Self::Committed)
    }

    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    pub fn feedback(&self) -> Option<&EvaluationFeedback> {
        match self {
            Self::Committed => None,
            Self::Denied { feedback } => Some(feedback),
        }
    }
}

/// A semantic action that may appear in durable run history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryAction {
    RunCreated,
    ContextAppended {
        context_record_id: ContextRecordId,
    },
    Transition {
        transition: Transition,
        outcome: TransitionHistoryOutcome,
    },
    Terminated,
    InvocationStarted {
        invocation_id: InvocationId,
    },
    InvocationStatusChanged {
        invocation_id: InvocationId,
        status: WaiterWrittenStatus,
    },
}

/// One ordered semantic history entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryEntry {
    pub sequence: SemanticSequence,
    pub occurred_at: Timestamp,
    pub action: HistoryAction,
}

impl HistoryEntry {
    pub fn new(sequence: SemanticSequence, occurred_at: Timestamp, action: HistoryAction) -> Self {
        Self {
            sequence,
            occurred_at,
            action,
        }
    }

    pub fn run_created(sequence: SemanticSequence, occurred_at: Timestamp) -> Self {
        Self::new(sequence, occurred_at, HistoryAction::RunCreated)
    }

    pub fn context_appended(
        sequence: SemanticSequence,
        occurred_at: Timestamp,
        context_record_id: impl Into<ContextRecordId>,
    ) -> Self {
        Self::new(
            sequence,
            occurred_at,
            HistoryAction::ContextAppended {
                context_record_id: context_record_id.into(),
            },
        )
    }

    pub fn transition(
        sequence: SemanticSequence,
        occurred_at: Timestamp,
        transition: Transition,
        outcome: TransitionHistoryOutcome,
    ) -> Self {
        Self::new(
            sequence,
            occurred_at,
            HistoryAction::Transition {
                transition,
                outcome,
            },
        )
    }

    pub fn terminated(sequence: SemanticSequence, occurred_at: Timestamp) -> Self {
        Self::new(sequence, occurred_at, HistoryAction::Terminated)
    }

    pub fn invocation_started(
        sequence: SemanticSequence,
        occurred_at: Timestamp,
        invocation_id: impl Into<InvocationId>,
    ) -> Self {
        Self::new(
            sequence,
            occurred_at,
            HistoryAction::InvocationStarted {
                invocation_id: invocation_id.into(),
            },
        )
    }

    pub fn invocation_status_changed(
        sequence: SemanticSequence,
        occurred_at: Timestamp,
        invocation_id: impl Into<InvocationId>,
        status: WaiterWrittenStatus,
    ) -> Self {
        Self::new(
            sequence,
            occurred_at,
            HistoryAction::InvocationStatusChanged {
                invocation_id: invocation_id.into(),
                status,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_transition() -> Transition {
        Transition::checked("design", "approved", "review")
    }

    fn sample_workflow() -> Workflow {
        Workflow::new(
            "software-change",
            "design",
            vec![
                State::new("design", "Design", "Prepare a design", false),
                State::new("review", "Review", "Review the design", false),
                State::new("end", "Done", "Finished", true),
            ],
            vec![sample_transition()],
        )
    }

    #[test]
    fn domain_identifiers_are_distinct_and_round_trip_as_strings() {
        let workflow_id = WorkflowId::new("workflow");
        let state_id = StateId::from("workflow");

        assert_eq!(workflow_id.as_str(), "workflow");
        assert_eq!(state_id.to_string(), "workflow");
        assert_eq!(workflow_id.to_string(), state_id.to_string());

        let encoded = serde_json::to_string(&workflow_id).unwrap();
        assert_eq!(encoded, r#""workflow""#);
        assert_eq!(
            serde_json::from_str::<WorkflowId>(&encoded).unwrap(),
            workflow_id
        );
    }

    #[test]
    fn lifecycle_and_transition_kind_expose_semantic_predicates() {
        assert!(Lifecycle::Active.is_active());
        assert!(!Lifecycle::Active.is_terminal());
        assert!(Lifecycle::Final.is_terminal());
        assert!(TransitionKind::Checked.is_checked());
        assert!(TransitionKind::CheckFree.is_check_free());
    }

    #[test]
    fn workflow_and_opaque_run_data_serialize_round_trip() {
        let association = ProviderAssociation::new(json!({
            "command": "/providers/change",
            "args": ["--stable"]
        }));
        let run = Run::new(
            "run-1",
            Some("change".to_owned()),
            sample_workflow(),
            association,
            json!({"objective": "keep compatibility", "opaque": [1, true]}),
            "design",
            Lifecycle::Active,
            ControlRevision::from_u64(7),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(1234),
        );

        let encoded = serde_json::to_string(&run).unwrap();
        let decoded: Run = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, run);
        assert_eq!(decoded.initial_input["opaque"], json!([1, true]));
        assert_eq!(
            decoded.provider_association.as_json()["args"],
            json!(["--stable"])
        );
        let encoded_workflow = serde_json::to_value(&run.workflow).unwrap();
        assert!(encoded_workflow.get("work_slots").is_none());
    }

    #[test]
    fn evaluation_results_have_provider_contract_shapes_and_preserve_details() {
        assert_eq!(
            serde_json::to_value(EvaluationResult::Allow).unwrap(),
            json!({"result": "allow"})
        );
        assert_eq!(
            serde_json::to_value(EvaluationResult::Unsupported).unwrap(),
            json!({"result": "unsupported"})
        );

        let result = EvaluationResult::deny(
            EvaluationFeedback::new("missing-review", "A review is required")
                .with_details(json!({"policy": "security"})),
        );
        let encoded = serde_json::to_value(&result).unwrap();
        assert_eq!(
            encoded,
            json!({
                "result": "deny",
                "feedback": {
                    "code": "missing-review",
                    "message": "A review is required",
                    "details": {"policy": "security"}
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<EvaluationResult>(encoded).unwrap(),
            result
        );
    }

    #[test]
    fn durable_history_and_prior_evaluation_round_trip() {
        let transition = sample_transition();
        let feedback = EvaluationFeedback::new("needs-work", "Revise the design");
        let denial = DurableEvaluation::deny(
            transition.clone(),
            feedback.clone(),
            SemanticSequence::new(4),
            Timestamp::from_unix_millis(4000),
        );
        let history = HistoryEntry::transition(
            SemanticSequence::new(4),
            Timestamp::from_unix_millis(4000),
            transition,
            TransitionHistoryOutcome::Denied { feedback },
        );

        let encoded = serde_json::to_string(&(denial.clone(), history.clone())).unwrap();
        let decoded: (DurableEvaluation, HistoryEntry) = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, (denial, history));
    }

    #[test]
    fn lineage_identity_is_source_and_event() {
        let original = Transition::checked("design", "approved", "review");
        let same_lineage = Transition::checked("design", "approved", "another-target");
        let different_event = Transition::checked("design", "revise", "design");

        assert!(original.same_lineage(&same_lineage));
        assert!(!original.same_lineage(&different_event));
    }

    #[test]
    fn invocation_started_and_invocation_status_changed_history_action_serde_round_trip() {
        let started = HistoryAction::InvocationStarted {
            invocation_id: InvocationId::new("inv-1"),
        };
        let encoded_started = serde_json::to_value(&started).unwrap();
        assert_eq!(encoded_started["kind"], "invocation_started");
        assert_eq!(encoded_started["invocation_id"], "inv-1");
        assert_eq!(
            serde_json::from_value::<HistoryAction>(encoded_started).unwrap(),
            started
        );

        let changed = HistoryAction::InvocationStatusChanged {
            invocation_id: InvocationId::new("inv-1"),
            status: WaiterWrittenStatus::Succeeded,
        };
        let encoded_changed = serde_json::to_value(&changed).unwrap();
        assert_eq!(encoded_changed["kind"], "invocation_status_changed");
        assert_eq!(encoded_changed["invocation_id"], "inv-1");
        assert_eq!(encoded_changed["status"], "succeeded");
        assert_eq!(
            serde_json::from_value::<HistoryAction>(encoded_changed).unwrap(),
            changed
        );

        let failed = HistoryAction::InvocationStatusChanged {
            invocation_id: InvocationId::new("inv-2"),
            status: WaiterWrittenStatus::Failed,
        };
        let encoded_failed = serde_json::to_value(&failed).unwrap();
        assert_eq!(encoded_failed["status"], "failed");
        assert_eq!(
            serde_json::from_value::<HistoryAction>(encoded_failed).unwrap(),
            failed
        );
    }

    #[test]
    fn work_slot_omits_empty_stdin_context_kinds_and_deserializes_legacy_catalogs() {
        let omitted = json!({
            "id": "slot-1",
            "state": "start",
            "event": "finish"
        });
        let slot: WorkSlot = serde_json::from_value(omitted).unwrap();
        assert_eq!(slot, WorkSlot::new("slot-1", "start", "finish"));
        assert!(slot.stdin_context_kinds.is_empty());
        assert!(serde_json::to_value(&slot)
            .unwrap()
            .get("stdin_context_kinds")
            .is_none());

        let empty = WorkSlot::new("slot-1", "start", "finish").with_stdin_context_kinds(Vec::new());
        assert!(serde_json::to_value(&empty)
            .unwrap()
            .get("stdin_context_kinds")
            .is_none());

        let listed = WorkSlot::new("slot-1", "start", "finish")
            .with_stdin_context_kinds(vec!["kind-a".to_owned()]);
        let encoded = serde_json::to_value(&listed).unwrap();
        assert_eq!(encoded["stdin_context_kinds"], json!(["kind-a"]));
        assert_eq!(serde_json::from_value::<WorkSlot>(encoded).unwrap(), listed);
    }
}
