//! Core domain vocabulary for Loop Engine.
//!
//! The core crate is deliberately independent of persistence, subprocess,
//! configuration, and CLI concerns.  It contains durable/provider-bound value
//! types and semantic operation outcomes used by later core layers.

mod invocation;
mod model;
pub mod operations;
mod outcome;
mod ports;
mod workflow;

pub use invocation::{instruction_digest, project_invocation_status};
pub use model::{
    ContextRecord, ContextRecordId, ControlRevision, DurableEvaluation, DurableEvaluationResult,
    EvaluationFeedback, EvaluationRequest, EvaluationResult, EventId, HistoryAction, HistoryEntry,
    InnerWorker, InvocationId, JsonValue, Lifecycle, PriorEvaluation, ProjectedInvocationStatus,
    ProviderAssociation, ProviderSelector, Run, RunId, SemanticSequence, State, StateId, Timestamp,
    Transition, TransitionHistoryOutcome, TransitionKind, WaiterWrittenStatus, WorkSlot,
    WorkSlotBinding, WorkSlotId, WorkSlotInvocation, Workflow, WorkflowId,
};
pub use operations::{
    execute_append, execute_event, execute_history, execute_invoke, execute_list, execute_show,
    execute_start, execute_terminate, lineage_for_transition, project_show, request_from_snapshot,
    AppendRequest, EventRequest, EventResult, HistoryRequest, InvokeRequest, InvokeResult,
    ListRequest, ProjectionError, RequestableEvent, ShowProjection, ShowRequest, StartRequest,
    TerminateRunRequest,
};
pub use outcome::{OperationOutcome, OperationStatus, OutcomeIssue};
pub use ports::{
    AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
    CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
    CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult, CreateRunRequest,
    CreateRunResult, CreateWorkSlotInvocationRequest, CreateWorkSlotInvocationResult, Persistence,
    PersistenceConflict, PersistenceError, PersistenceFailure, PersistenceRejection, ProcessError,
    ProviderError, ProviderGateway, ProviderResolutionError, ProviderResolver, RecordDenialRequest,
    RecordDenialResult, RunSummary, ShowData, StartedWaiter, TerminateRequest, TerminateResult,
    WaiterSpawnArgs, WorkSlotProcess,
};
pub use workflow::{
    resolve_transition, validate_workflow, workflow_validation_errors, TransitionResolutionError,
    WorkflowValidationError,
};
