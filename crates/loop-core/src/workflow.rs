//! Structural workflow validation and authoritative transition resolution.
//!
//! Workflow validation intentionally checks only whether a workflow can be
//! interpreted as a deterministic state/event graph.  It does not assess
//! reachability, termination, or any workflow-specific quality property.

use crate::{EventId, StateId, Transition, Workflow};
use std::collections::BTreeSet;
use std::fmt;

/// A structural defect that prevents a workflow from being interpreted
/// deterministically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowValidationError {
    /// More than one state declares the same identifier.
    DuplicateStateId { state: StateId },
    /// The workflow's initial state does not name one of its states.
    UndefinedInitialState { state: StateId },
    /// A transition source does not name one of the workflow's states.
    UndefinedTransitionSource {
        source: StateId,
        event: EventId,
        target: StateId,
    },
    /// A transition target does not name one of the workflow's states.
    UndefinedTransitionTarget {
        source: StateId,
        event: EventId,
        target: StateId,
    },
    /// A source state and event identify more than one transition.
    DuplicateTransition { source: StateId, event: EventId },
    /// Final states are terminal and therefore cannot have outgoing edges.
    TransitionFromFinalState {
        source: StateId,
        event: EventId,
        target: StateId,
    },
}

impl WorkflowValidationError {
    /// Stable machine-readable classification for callers that do not need
    /// to inspect the structured fields of the error.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DuplicateStateId { .. } => "duplicate-state-id",
            Self::UndefinedInitialState { .. } => "undefined-initial-state",
            Self::UndefinedTransitionSource { .. } => "undefined-transition-source",
            Self::UndefinedTransitionTarget { .. } => "undefined-transition-target",
            Self::DuplicateTransition { .. } => "duplicate-source-event",
            Self::TransitionFromFinalState { .. } => "transition-from-final-state",
        }
    }
}

impl fmt::Display for WorkflowValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStateId { state } => {
                write!(formatter, "workflow contains duplicate state ID `{state}`")
            }
            Self::UndefinedInitialState { state } => {
                write!(formatter, "workflow initial state `{state}` is undefined")
            }
            Self::UndefinedTransitionSource {
                source,
                event,
                target,
            } => write!(
                formatter,
                "transition `{source}` + `{event}` -> `{target}` has an undefined source state"
            ),
            Self::UndefinedTransitionTarget {
                source,
                event,
                target,
            } => write!(
                formatter,
                "transition `{source}` + `{event}` -> `{target}` has an undefined target state"
            ),
            Self::DuplicateTransition { source, event } => write!(
                formatter,
                "workflow has duplicate transition for source state `{source}` and event `{event}`"
            ),
            Self::TransitionFromFinalState {
                source,
                event,
                target,
            } => write!(
                formatter,
                "final state `{source}` has outgoing transition `{event}` -> `{target}`"
            ),
        }
    }
}

impl std::error::Error for WorkflowValidationError {}

/// Why a requested transition could not be resolved.
///
/// `Ok(None)` is deliberately reserved for a well-formed workflow in which
/// the requested event is not available from the authoritative current state.
/// A malformed workflow is an error instead, so callers cannot accidentally
/// treat an invalid graph as an ordinary unavailable event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionResolutionError {
    /// The stored workflow definition is structurally invalid.
    MalformedWorkflow { error: WorkflowValidationError },
    /// The supplied authoritative current state is not part of the stored
    /// workflow.  This cannot occur for a run created through the normal
    /// validated path, but is reported rather than being mistaken for an
    /// unavailable event when resolving a raw definition.
    UndefinedCurrentState { state: StateId },
}

impl TransitionResolutionError {
    pub fn malformed_workflow(error: WorkflowValidationError) -> Self {
        Self::MalformedWorkflow { error }
    }

    pub fn validation_error(&self) -> Option<&WorkflowValidationError> {
        match self {
            Self::MalformedWorkflow { error } => Some(error),
            Self::UndefinedCurrentState { .. } => None,
        }
    }

    pub const fn is_malformed_workflow(&self) -> bool {
        matches!(self, Self::MalformedWorkflow { .. })
    }
}

impl From<WorkflowValidationError> for TransitionResolutionError {
    fn from(error: WorkflowValidationError) -> Self {
        Self::malformed_workflow(error)
    }
}

impl fmt::Display for TransitionResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedWorkflow { error } => write!(formatter, "malformed workflow: {error}"),
            Self::UndefinedCurrentState { state } => {
                write!(
                    formatter,
                    "authoritative current state `{state}` is undefined"
                )
            }
        }
    }
}

impl std::error::Error for TransitionResolutionError {}

impl Workflow {
    /// Return every structural validation error in deterministic definition
    /// order.
    ///
    /// The checks intentionally do not perform graph traversal.  In
    /// particular, cycles, unreachable states, non-final sinks, and workflows
    /// without a final state produce no error here.
    pub fn validation_errors(&self) -> Vec<WorkflowValidationError> {
        let mut errors = Vec::new();
        let mut state_ids = BTreeSet::new();

        for state in &self.states {
            if !state_ids.insert(state.id.clone()) {
                errors.push(WorkflowValidationError::DuplicateStateId {
                    state: state.id.clone(),
                });
            }
        }

        if !state_ids.contains(&self.initial_state) {
            errors.push(WorkflowValidationError::UndefinedInitialState {
                state: self.initial_state.clone(),
            });
        }

        let final_states: BTreeSet<StateId> = self
            .states
            .iter()
            .filter(|state| state.is_final)
            .map(|state| state.id.clone())
            .collect();
        let mut source_events = BTreeSet::new();

        for transition in &self.transitions {
            let source_defined = state_ids.contains(&transition.source);
            let target_defined = state_ids.contains(&transition.target);

            if !source_defined {
                errors.push(WorkflowValidationError::UndefinedTransitionSource {
                    source: transition.source.clone(),
                    event: transition.event.clone(),
                    target: transition.target.clone(),
                });
            }
            if !target_defined {
                errors.push(WorkflowValidationError::UndefinedTransitionTarget {
                    source: transition.source.clone(),
                    event: transition.event.clone(),
                    target: transition.target.clone(),
                });
            }

            if !source_events.insert((transition.source.clone(), transition.event.clone())) {
                errors.push(WorkflowValidationError::DuplicateTransition {
                    source: transition.source.clone(),
                    event: transition.event.clone(),
                });
            }

            if source_defined && final_states.contains(&transition.source) {
                errors.push(WorkflowValidationError::TransitionFromFinalState {
                    source: transition.source.clone(),
                    event: transition.event.clone(),
                    target: transition.target.clone(),
                });
            }
        }

        errors
    }

    /// Validate the workflow, returning the first deterministic structural
    /// error when it is malformed.
    pub fn validate(&self) -> Result<(), WorkflowValidationError> {
        match self.validation_errors().into_iter().next() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Resolve one event against the authoritative current state in this
    /// stored workflow.
    ///
    /// The result is `Ok(Some(transition))` for the one matching edge,
    /// `Ok(None)` when no edge is available from the current state, and an
    /// error when the workflow or authoritative state is malformed.
    pub fn resolve_transition<S, E>(
        &self,
        current_state: S,
        event: E,
    ) -> Result<Option<&Transition>, TransitionResolutionError>
    where
        S: AsRef<str>,
        E: AsRef<str>,
    {
        self.validate()
            .map_err(TransitionResolutionError::malformed_workflow)?;

        let current_state = StateId::new(current_state.as_ref());
        let event = EventId::new(event.as_ref());

        if !self.states.iter().any(|state| state.id == current_state) {
            return Err(TransitionResolutionError::UndefinedCurrentState {
                state: current_state,
            });
        }

        Ok(self
            .transitions
            .iter()
            .find(|transition| transition.source == current_state && transition.event == event))
    }
}

/// Validate a workflow definition using the core structural rules.
pub fn validate_workflow(workflow: &Workflow) -> Result<(), WorkflowValidationError> {
    workflow.validate()
}

/// Return all structural validation errors for a workflow definition.
pub fn workflow_validation_errors(workflow: &Workflow) -> Vec<WorkflowValidationError> {
    workflow.validation_errors()
}

/// Resolve an event against an authoritative current state in a stored
/// workflow definition.
pub fn resolve_transition<S, E>(
    workflow: &Workflow,
    current_state: S,
    event: E,
) -> Result<Option<&Transition>, TransitionResolutionError>
where
    S: AsRef<str>,
    E: AsRef<str>,
{
    workflow.resolve_transition(current_state, event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: &str, is_final: bool) -> crate::State {
        crate::State::new(id, id, format!("instructions for {id}"), is_final)
    }

    fn workflow(
        initial_state: &str,
        states: Vec<crate::State>,
        transitions: Vec<Transition>,
    ) -> Workflow {
        Workflow::new("test-workflow", initial_state, states, transitions)
    }

    #[test]
    fn rejects_duplicate_state_ids() {
        let definition = workflow(
            "start",
            vec![state("start", false), state("start", false)],
            vec![],
        );

        assert!(matches!(
            definition.validate(),
            Err(WorkflowValidationError::DuplicateStateId { state }) if state == StateId::from("start")
        ));
    }

    #[test]
    fn rejects_undefined_initial_state() {
        let definition = workflow("missing", vec![state("start", false)], vec![]);

        assert!(matches!(
            definition.validate(),
            Err(WorkflowValidationError::UndefinedInitialState { state }) if state == StateId::from("missing")
        ));
    }

    #[test]
    fn rejects_undefined_transition_endpoints() {
        let definition = workflow(
            "start",
            vec![state("start", false)],
            vec![Transition::check_free(
                "missing-source",
                "go",
                "missing-target",
            )],
        );

        let errors = definition.validation_errors();
        assert!(errors.iter().any(|error| matches!(
            error,
            WorkflowValidationError::UndefinedTransitionSource { source, .. }
                if source == &StateId::from("missing-source")
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            WorkflowValidationError::UndefinedTransitionTarget { target, .. }
                if target == &StateId::from("missing-target")
        )));
        assert!(definition.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_source_event_pairs() {
        let definition = workflow(
            "start",
            vec![
                state("start", false),
                state("one", false),
                state("two", false),
            ],
            vec![
                Transition::check_free("start", "go", "one"),
                Transition::checked("start", "go", "two"),
            ],
        );

        assert!(matches!(
            definition.validate(),
            Err(WorkflowValidationError::DuplicateTransition { source, event })
                if source == StateId::from("start") && event == EventId::from("go")
        ));
    }

    #[test]
    fn rejects_outgoing_transitions_from_final_states() {
        let definition = workflow(
            "start",
            vec![state("start", false), state("done", true)],
            vec![Transition::check_free("done", "restart", "start")],
        );

        assert!(matches!(
            definition.validate(),
            Err(WorkflowValidationError::TransitionFromFinalState { source, event, target })
                if source == StateId::from("done")
                    && event == EventId::from("restart")
                    && target == StateId::from("start")
        ));
    }

    #[test]
    fn permits_cycles() {
        let definition = workflow(
            "a",
            vec![state("a", false), state("b", false)],
            vec![
                Transition::check_free("a", "next", "b"),
                Transition::check_free("b", "back", "a"),
            ],
        );

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn permits_unreachable_states() {
        let definition = workflow(
            "start",
            vec![state("start", false), state("unreachable", true)],
            vec![],
        );

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn permits_non_final_sink_states() {
        let definition = workflow("sink", vec![state("sink", false)], vec![]);

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn permits_workflows_without_a_final_state() {
        let definition = workflow(
            "start",
            vec![state("start", false), state("next", false)],
            vec![Transition::check_free("start", "next", "next")],
        );

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn resolution_is_scoped_to_authoritative_current_state() {
        let expected = Transition::check_free("other", "go", "done");
        let definition = workflow(
            "start",
            vec![
                state("start", false),
                state("other", false),
                state("done", true),
            ],
            vec![expected.clone()],
        );

        assert_eq!(definition.resolve_transition("start", "go").unwrap(), None);
        assert_eq!(
            definition.resolve_transition("other", "go").unwrap(),
            Some(&expected)
        );
    }

    #[test]
    fn unavailable_event_is_distinct_from_malformed_workflow() {
        let definition = workflow(
            "start",
            vec![state("start", false), state("done", true)],
            vec![],
        );

        assert_eq!(
            definition.resolve_transition("start", "missing").unwrap(),
            None
        );

        let malformed = workflow(
            "start",
            vec![state("start", false)],
            vec![Transition::check_free("start", "go", "missing")],
        );
        assert!(matches!(
            malformed.resolve_transition("start", "go"),
            Err(TransitionResolutionError::MalformedWorkflow { .. })
        ));
    }

    #[test]
    fn final_state_has_no_resolvable_outgoing_event() {
        let definition = workflow("done", vec![state("done", true)], vec![]);

        assert_eq!(
            definition.resolve_transition("done", "anything").unwrap(),
            None
        );
    }
}
