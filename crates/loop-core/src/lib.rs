//! Core domain vocabulary for Loop Engine.
//!
//! The core crate is deliberately independent of persistence, subprocess,
//! configuration, and CLI concerns.  It contains durable/provider-bound value
//! types and semantic operation outcomes used by later core layers.

mod advice;
mod advice_transition;
mod execution_contract;
mod invocation;
mod model;
pub mod operations;
mod outcome;
mod ports;
mod workflow;

pub use advice::{
    parse_json_rejecting_duplicate_keys, AdviceAdmissibility, AdviceAnswer, AdviceCommandConfig,
    AdviceQuestion, AdviceRequest, AdviceResponse, AdviceScoreLevel, ADVICE_CAPTURE_KIND,
    ADVICE_COMMAND_INPUT_KEY, ADVICE_PROTOCOL_VERSION,
};
pub(crate) use advice_transition::{unanswered_due_occasions, validate_advice_exception};
pub use advice_transition::{
    AdviceDeparture, AdviceDepartureMap, AdviceExceptionAttestation,
    ADVICE_APPLICABILITY_RECORD_KIND, ADVICE_DEPARTURES_INPUT_KEY, ADVICE_DISPOSITION_RECORD_KIND,
    ADVICE_OCCASION_RECORD_KIND,
};
pub use execution_contract::{
    effective_binding, invocation_owns_work, resolve_context_filter, BindingAmendment,
    CancellationAcknowledgment, ContextFilter, ContextFilterSelection, DriverActAuthor,
    DriverActDocuments, DriverActEvidence, DriverActRequest, EffectiveBinding,
    ExecutionOwnershipState, InvocationControls, OwnedExecution, ProcessIdentity,
    StateVisitAttestation,
};
pub use invocation::{instruction_digest, project_invocation_status, work_slot_binding_digest};
pub use model::{
    AllowResponse, AssignmentLabel, CompletionMode, ContextAppendEffect, ContextRecord,
    ContextRecordId, ControlRevision, DurableEvaluation, DurableEvaluationResult,
    EvaluationFeedback, EvaluationRequest, EvaluationResult, EventId, FanOutRecoveryAdapterUsage,
    FanOutRecoveryApproval, FanOutRecoveryDerivation, FanOutRecoveryInput, FanOutRecoverySource,
    HistoryAction, HistoryEntry, InnerWorker, InvocationId, JsonValue, Lifecycle, OverrideSummary,
    PriorEvaluation, ProjectedInvocationStatus, ProviderAssociation, ProviderSelector, Run, RunId,
    SemanticSequence, SkippedBoundCheck, SkippedProviderEvaluation, State, StateId, Timestamp,
    Transition, TransitionHistoryOutcome, TransitionKind, TransitionOverride, WaiterWrittenStatus,
    WorkSlot, WorkSlotBinding, WorkSlotId, WorkSlotInvocation, WorkerAttempt, Workflow, WorkflowId,
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
    AppendAdviceAttemptRequest, AppendContextRequest, AppendContextResult, CarryAct, CarryRequest,
    CheckedEvaluationSnapshot, CheckedEvaluationSnapshotRequest, CommitTransitionRequest,
    CommitTransitionResult, CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult,
    CreateRunRequest, CreateRunResult, CreateWorkSlotInvocationRequest,
    CreateWorkSlotInvocationResult, EngineOrigin, EvidenceApplicability, OriginReference,
    Persistence, PersistenceConflict, PersistenceError, PersistenceFailure, PersistenceRejection,
    ProcessError, ProviderError, ProviderGateway, ProviderResolutionError, ProviderResolver,
    RecordDenialRequest, RecordDenialResult, RunSummary, ShowData, StartedWaiter, TerminateRequest,
    TerminateResult, WaiterSpawnArgs, WorkSlotProcess, ENGINE_ORIGIN_KEY,
    EVIDENCE_APPLICABILITY_KIND, ORIGIN_KEY,
};
pub use workflow::{
    resolve_transition, validate_workflow, workflow_validation_errors, TransitionResolutionError,
    WorkflowValidationError,
};
