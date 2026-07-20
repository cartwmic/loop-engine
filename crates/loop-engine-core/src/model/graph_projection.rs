use super::bounded::Metadata;
use super::graph_validation::ValidatedGraph;
use super::guidance::{LiveGuidanceCapability, StaticGuidance};
use super::ids::{EventId, GateId, InputKind, InputName, StateId};

pub const CANONICAL_GRAPH_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateProjection {
    pub id: StateId,
    pub final_state: bool,
    pub guidance: StaticGuidance,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransitionProjection {
    pub source: StateId,
    pub event: EventId,
    pub target: StateId,
    pub required_gates: Vec<GateId>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputProjection {
    pub name: InputName,
    pub kind: InputKind,
    pub required: bool,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticGraphProjection {
    pub canonical_graph_version: u64,
    pub initial_state: StateId,
    pub states: Vec<StateProjection>,
    pub transitions: Vec<TransitionProjection>,
    pub inputs: Vec<InputProjection>,
    pub live_guidance: LiveGuidanceCapability,
    pub metadata: Option<Metadata>,
}

impl SemanticGraphProjection {
    pub fn from_validated(graph: &ValidatedGraph) -> Self {
        let graph = graph.graph();
        let mut states = graph
            .states()
            .map(|state| StateProjection {
                id: state.id().clone(),
                final_state: state.is_final(),
                guidance: state.guidance().clone(),
                metadata: state.metadata().cloned(),
            })
            .collect::<Vec<_>>();
        states.sort_by(|left, right| left.id.cmp(&right.id));
        let mut transitions = graph
            .transitions()
            .iter()
            .map(|transition| {
                let mut required_gates = transition.required_gates().to_vec();
                required_gates.sort();
                TransitionProjection {
                    source: transition.source().clone(),
                    event: transition.event().clone(),
                    target: transition.target().clone(),
                    required_gates,
                    metadata: transition.metadata().cloned(),
                }
            })
            .collect::<Vec<_>>();
        transitions
            .sort_by(|left, right| (&left.source, &left.event).cmp(&(&right.source, &right.event)));
        let mut inputs = graph
            .inputs()
            .values()
            .map(|input| InputProjection {
                name: input.name().clone(),
                kind: input.kind().clone(),
                required: input.required(),
                metadata: input.metadata().cloned(),
            })
            .collect::<Vec<_>>();
        inputs.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            canonical_graph_version: CANONICAL_GRAPH_VERSION,
            initial_state: graph.initial_state().clone(),
            states,
            transitions,
            inputs,
            live_guidance: graph.live_guidance(),
            metadata: graph.metadata().cloned(),
        }
    }
}
