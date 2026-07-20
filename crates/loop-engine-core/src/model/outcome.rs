use thiserror::Error;

use super::bounded::BoundError;
use super::diagnostic::{Diagnostic, validate_diagnostics};
use super::ids::{RunId, StateId};
use super::lifecycle::Lifecycle;
use super::reason::Reason;
use super::requestable::RequestableEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeClass {
    Completed,
    Rejected,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvidenceRecordedStatus {
    pub inline: bool,
    pub selected_associations: bool,
    pub provider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run_id: RunId,
    pub label: Option<String>,
    pub lifecycle: Lifecycle,
    pub current_state: StateId,
    pub state_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeData {
    run: Option<RunSnapshot>,
    requestable_events: Option<Vec<RequestableEvent>>,
    evidence_recorded: Option<EvidenceRecordedStatus>,
}

impl OutcomeData {
    pub fn new(
        run: Option<RunSnapshot>,
        requestable_events: Option<Vec<RequestableEvent>>,
        evidence_recorded: Option<EvidenceRecordedStatus>,
    ) -> Result<Self, OutcomeError> {
        match (&run, &requestable_events) {
            (None, Some(_)) | (Some(_), None) => return Err(OutcomeError::RequestableShape),
            (Some(run), Some(events)) if run.lifecycle.is_terminal() && !events.is_empty() => {
                return Err(OutcomeError::TerminalRequestableEvents);
            }
            _ => {}
        }
        Ok(Self {
            run,
            requestable_events,
            evidence_recorded,
        })
    }

    pub fn run(&self) -> Option<&RunSnapshot> {
        self.run.as_ref()
    }

    pub fn requestable_events(&self) -> Option<&[RequestableEvent]> {
        self.requestable_events.as_deref()
    }

    pub fn evidence_recorded(&self) -> Option<EvidenceRecordedStatus> {
        self.evidence_recorded
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutcome {
    class: OutcomeClass,
    reason: Option<Reason>,
    data: OutcomeData,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OutcomeError {
    #[error("completed outcomes cannot carry a reason")]
    CompletedWithReason,
    #[error("rejected or error outcomes require a reason")]
    MissingReason,
    #[error("reason class does not match outcome class")]
    ReasonClassMismatch,
    #[error("requestable events must be present exactly when a run is resolved")]
    RequestableShape,
    #[error("terminal runs cannot have requestable events")]
    TerminalRequestableEvents,
    #[error(transparent)]
    Bound(#[from] BoundError),
}

impl PublicOutcome {
    pub fn new(
        class: OutcomeClass,
        reason: Option<Reason>,
        data: OutcomeData,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Self, OutcomeError> {
        match (class, &reason) {
            (OutcomeClass::Completed, Some(_)) => return Err(OutcomeError::CompletedWithReason),
            (OutcomeClass::Completed, None) => {}
            (_, None) => return Err(OutcomeError::MissingReason),
            (class, Some(reason)) if class != reason.code().outcome_class() => {
                return Err(OutcomeError::ReasonClassMismatch);
            }
            (_, Some(_)) => {}
        }
        validate_diagnostics(&diagnostics)?;
        Ok(Self {
            class,
            reason,
            data,
            diagnostics,
        })
    }

    pub fn class(&self) -> OutcomeClass {
        self.class
    }

    pub fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    pub fn data(&self) -> &OutcomeData {
        &self.data
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::{OutcomeData, OutcomeError, RunSnapshot};
    use crate::model::ids::{EventId, RunId, StateId};
    use crate::model::lifecycle::Lifecycle;
    use crate::model::requestable::RequestableEvent;

    #[test]
    fn requestable_shape_follows_resolution_and_lifecycle() {
        assert!(matches!(
            OutcomeData::new(None, Some(vec![]), None),
            Err(OutcomeError::RequestableShape)
        ));
        let terminal = RunSnapshot {
            run_id: RunId::parse("run").unwrap(),
            label: None,
            lifecycle: Lifecycle::Final,
            current_state: StateId::parse("done").unwrap(),
            state_changed: true,
        };
        assert!(matches!(
            OutcomeData::new(
                Some(terminal),
                Some(vec![RequestableEvent {
                    event: EventId::parse("again").unwrap(),
                    target: StateId::parse("done").unwrap(),
                    required_gates: vec![],
                }]),
                None,
            ),
            Err(OutcomeError::TerminalRequestableEvents)
        ));
    }
}
