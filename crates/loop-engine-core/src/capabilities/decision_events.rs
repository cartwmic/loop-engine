use crate::model::bounded::{BoundedText, IDENTIFIER_UTF8_BYTES};
use crate::model::compatibility::CompatibilityStatus;
use crate::model::ids::{EventId, GateId, RegistrationId, RunId, StateId};
use crate::model::lifecycle::Lifecycle;
use crate::model::outcome::OutcomeClass;
use crate::model::version::{LifecycleVersion, WorkflowStateVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Passed,
    Failed,
    Incompatible,
    EvaluationError,
}

/// Consequential facts emitted to the operational trace adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionEvent {
    TransitionResolved {
        run_id: RunId,
        event: EventId,
        source: StateId,
        target: Option<StateId>,
        outcome: OutcomeClass,
    },
    GateObserved {
        run_id: RunId,
        gate: Option<GateId>,
        decision: GateDecision,
    },
    LifecycleChanged {
        run_id: RunId,
        before: Lifecycle,
        after: Lifecycle,
        version: LifecycleVersion,
    },
    CompatibilityObserved {
        run_id: RunId,
        registration_id: RegistrationId,
        capability: BoundedText<IDENTIFIER_UTF8_BYTES>,
        status: CompatibilityStatus,
    },
    StaleAttempt {
        run_id: RunId,
        expected_workflow: WorkflowStateVersion,
        actual_workflow: WorkflowStateVersion,
        expected_lifecycle: LifecycleVersion,
        actual_lifecycle: LifecycleVersion,
    },
    StaleRegistration {
        registration_id: RegistrationId,
        expected_config_revision: u64,
        actual_config_revision: u64,
    },
}

pub trait DecisionEventSink {
    type Error;

    fn record(&self, event: DecisionEvent) -> Result<(), Self::Error>;
}
