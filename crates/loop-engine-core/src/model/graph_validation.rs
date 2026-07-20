use std::collections::BTreeSet;

use thiserror::Error;

use super::graph::WorkflowGraph;
use super::ids::{EventId, StateId};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("graph has no states")]
    Empty,
    #[error("duplicate state id: {0}")]
    DuplicateState(StateId),
    #[error("initial state does not exist: {0}")]
    MissingInitial(StateId),
    #[error("transition source does not exist: {0}")]
    MissingSource(StateId),
    #[error("transition target does not exist: {0}")]
    MissingTarget(StateId),
    #[error("ambiguous transition for state {state} and event {event}")]
    Ambiguous { state: StateId, event: EventId },
    #[error("final state declares an outgoing transition: {0}")]
    FinalHasOutgoing(StateId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGraph(WorkflowGraph);

impl ValidatedGraph {
    pub fn validate(graph: WorkflowGraph) -> Result<Self, GraphError> {
        if graph.states().next().is_none() {
            return Err(GraphError::Empty);
        }
        let mut states = BTreeSet::new();
        for state in graph.states() {
            if !states.insert(state.id().clone()) {
                return Err(GraphError::DuplicateState(state.id().clone()));
            }
        }
        if !states.contains(graph.initial_state()) {
            return Err(GraphError::MissingInitial(graph.initial_state().clone()));
        }
        let mut selectors = BTreeSet::new();
        for transition in graph.transitions() {
            if !states.contains(transition.source()) {
                return Err(GraphError::MissingSource(transition.source().clone()));
            }
            if !states.contains(transition.target()) {
                return Err(GraphError::MissingTarget(transition.target().clone()));
            }
            let selector = (transition.source().clone(), transition.event().clone());
            if !selectors.insert(selector.clone()) {
                return Err(GraphError::Ambiguous {
                    state: selector.0,
                    event: selector.1,
                });
            }
            if graph
                .state(transition.source())
                .is_some_and(|state| state.is_final())
            {
                return Err(GraphError::FinalHasOutgoing(transition.source().clone()));
            }
        }
        Ok(Self(graph))
    }

    pub fn graph(&self) -> &WorkflowGraph {
        &self.0
    }

    pub fn into_graph(self) -> WorkflowGraph {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{EventId, GraphError, StateId, ValidatedGraph, WorkflowGraph};
    use crate::model::graph::State;
    use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use crate::model::ids::{GateId, InputKind, InputName};
    use crate::model::run_input::{InputDeclaration, InputDeclarations};
    use crate::model::transition::Transition;

    fn state(id: &str, final_state: bool) -> State {
        State::new(
            StateId::parse(id).unwrap(),
            final_state,
            StaticGuidance::NoneRequired,
            None,
        )
    }

    fn transition(source: &str, event: &str, target: &str) -> Transition {
        Transition::new(
            StateId::parse(source).unwrap(),
            EventId::parse(event).unwrap(),
            StateId::parse(target).unwrap(),
            vec![GateId::parse("gate").unwrap()],
            None,
        )
        .unwrap()
    }

    fn graph(states: Vec<State>, transitions: Vec<Transition>, initial: &str) -> WorkflowGraph {
        WorkflowGraph::new_unvalidated(
            StateId::parse(initial).unwrap(),
            states,
            transitions,
            InputDeclarations::new(vec![InputDeclaration::new(
                InputName::parse("input").unwrap(),
                InputKind::parse("text").unwrap(),
                false,
                None,
            )])
            .unwrap(),
            LiveGuidanceCapability::Unsupported,
            None,
        )
    }

    #[test]
    fn valid_matrix_allows_cycles_zero_or_many_finals_and_sinks() {
        let cases = [
            graph(vec![state("a", false)], vec![], "a"),
            graph(vec![state("a", true)], vec![], "a"),
            graph(
                vec![state("a", false), state("b", false)],
                vec![transition("a", "go", "b"), transition("b", "back", "a")],
                "a",
            ),
            graph(vec![state("a", true), state("b", true)], vec![], "a"),
        ];
        for graph in cases {
            assert!(ValidatedGraph::validate(graph).is_ok());
        }
    }

    #[test]
    fn invalid_matrix_rejects_missing_duplicate_ambiguous_and_final_outgoing() {
        assert!(matches!(
            ValidatedGraph::validate(graph(vec![], vec![], "missing")),
            Err(GraphError::Empty)
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(
                vec![state("a", false), state("a", false)],
                vec![],
                "a"
            )),
            Err(GraphError::DuplicateState(_))
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(vec![state("a", false)], vec![], "missing")),
            Err(GraphError::MissingInitial(_))
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(
                vec![state("a", false)],
                vec![transition("missing", "go", "a")],
                "a"
            )),
            Err(GraphError::MissingSource(_))
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(
                vec![state("a", false)],
                vec![transition("a", "go", "missing")],
                "a"
            )),
            Err(GraphError::MissingTarget(_))
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(
                vec![state("a", false), state("b", false)],
                vec![transition("a", "go", "b"), transition("a", "go", "a")],
                "a"
            )),
            Err(GraphError::Ambiguous { .. })
        ));
        assert!(matches!(
            ValidatedGraph::validate(graph(
                vec![state("a", true)],
                vec![transition("a", "go", "a")],
                "a"
            )),
            Err(GraphError::FinalHasOutgoing(_))
        ));
    }
}
