use super::bounded::Metadata;
use super::guidance::{LiveGuidanceCapability, StaticGuidance};
use super::ids::StateId;
use super::run_input::InputDeclarations;
use super::transition::Transition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    id: StateId,
    final_state: bool,
    guidance: StaticGuidance,
    metadata: Option<Metadata>,
}

impl State {
    pub fn new(
        id: StateId,
        final_state: bool,
        guidance: StaticGuidance,
        metadata: Option<Metadata>,
    ) -> Self {
        Self {
            id,
            final_state,
            guidance,
            metadata,
        }
    }

    pub fn id(&self) -> &StateId {
        &self.id
    }

    pub fn is_final(&self) -> bool {
        self.final_state
    }

    pub fn guidance(&self) -> &StaticGuidance {
        &self.guidance
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowGraph {
    initial_state: StateId,
    states: Vec<State>,
    transitions: Vec<Transition>,
    inputs: InputDeclarations,
    live_guidance: LiveGuidanceCapability,
    metadata: Option<Metadata>,
}

impl WorkflowGraph {
    pub fn new_unvalidated(
        initial_state: StateId,
        states: Vec<State>,
        transitions: Vec<Transition>,
        inputs: InputDeclarations,
        live_guidance: LiveGuidanceCapability,
        metadata: Option<Metadata>,
    ) -> Self {
        Self {
            initial_state,
            states,
            transitions,
            inputs,
            live_guidance,
            metadata,
        }
    }

    pub fn initial_state(&self) -> &StateId {
        &self.initial_state
    }

    pub fn state(&self, id: &StateId) -> Option<&State> {
        self.states.iter().find(|state| state.id() == id)
    }

    pub fn states(&self) -> impl Iterator<Item = &State> {
        self.states.iter()
    }

    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    pub fn inputs(&self) -> &InputDeclarations {
        &self.inputs
    }

    pub fn live_guidance(&self) -> LiveGuidanceCapability {
        self.live_guidance
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InputDeclarations, LiveGuidanceCapability, State, StateId, StaticGuidance, WorkflowGraph,
    };

    fn state(id: &str, final_state: bool) -> State {
        State::new(
            StateId::parse(id).unwrap(),
            final_state,
            StaticGuidance::NoneRequired,
            None,
        )
    }

    #[test]
    fn zero_one_multiple_and_initial_final_are_representable() {
        for states in [
            vec![state("a", false)],
            vec![state("a", true)],
            vec![state("a", true), state("b", true)],
        ] {
            let graph = WorkflowGraph::new_unvalidated(
                StateId::parse("a").unwrap(),
                states,
                vec![],
                InputDeclarations::default(),
                LiveGuidanceCapability::Unsupported,
                None,
            );
            assert!(graph.state(graph.initial_state()).is_some());
        }
    }
}
