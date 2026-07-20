use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::transition::Transition;
use thiserror::Error;

use super::dto::{GraphDto, StaticGuidanceDeclarationDto, StaticGuidanceDto};
use super::mapping::{
    MappingError, input_declarations, metadata, parse_event_id, parse_gate_id, parse_state_id,
};

#[derive(Debug, Error)]
pub enum GraphMappingError {
    #[error(transparent)]
    Mapping(#[from] MappingError),
    #[error("invalid graph semantics: {0}")]
    Semantic(String),
}

pub fn map_graph(dto: GraphDto) -> Result<ValidatedGraph, GraphMappingError> {
    let graph = map_graph_unvalidated(dto)?;
    ValidatedGraph::validate(graph).map_err(|error| GraphMappingError::Semantic(error.to_string()))
}

pub fn map_graph_unvalidated(dto: GraphDto) -> Result<WorkflowGraph, GraphMappingError> {
    let initial = parse_state_id(dto.initial_state, "/graph/initial_state")?;
    let states = dto
        .states
        .into_iter()
        .enumerate()
        .map(|(index, state)| {
            let path = format!("/graph/states/{index}");
            let guidance = match state.static_guidance {
                StaticGuidanceDto::Text(text)
                | StaticGuidanceDto::Declaration(StaticGuidanceDeclarationDto::Text { text }) => {
                    StaticGuidance::text(text).map_err(|error| {
                        MappingError::field(format!("{path}/static_guidance"), error)
                    })?
                }
                StaticGuidanceDto::Declaration(StaticGuidanceDeclarationDto::None) => {
                    StaticGuidance::NoneRequired
                }
            };
            Ok(State::new(
                parse_state_id(state.id, &format!("{path}/id"))?,
                state.final_state,
                guidance,
                metadata(state.metadata, &format!("{path}/metadata"))?,
            ))
        })
        .collect::<Result<Vec<_>, MappingError>>()?;
    let transitions = dto
        .transitions
        .into_iter()
        .enumerate()
        .map(|(index, transition)| {
            let path = format!("/graph/transitions/{index}");
            let gates = transition
                .gate_ids
                .into_iter()
                .enumerate()
                .map(|(gate_index, gate)| {
                    parse_gate_id(gate, &format!("{path}/gate_ids/{gate_index}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Transition::new_unvalidated(
                parse_state_id(transition.source_state, &format!("{path}/source_state"))?,
                parse_event_id(transition.event, &format!("{path}/event"))?,
                parse_state_id(transition.target_state, &format!("{path}/target_state"))?,
                gates,
                metadata(transition.metadata, &format!("{path}/metadata"))?,
            ))
        })
        .collect::<Result<Vec<_>, MappingError>>()?;
    let inputs = input_declarations(dto.input_declarations, "/graph/input_declarations")?;
    Ok(WorkflowGraph::new_unvalidated(
        initial,
        states,
        transitions,
        inputs,
        if dto.live_guidance_supported {
            LiveGuidanceCapability::Supported
        } else {
            LiveGuidanceCapability::Unsupported
        },
        metadata(dto.metadata, "/graph/metadata")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::map_graph;
    use crate::provider_protocol::dto::GraphDto;

    #[test]
    fn invalid_graph_reports_semantic_failure() {
        let dto: GraphDto = serde_json::from_str(
            r#"{"initial_state":"missing","states":[],"transitions":[],"input_declarations":[],"live_guidance_supported":false}"#,
        )
        .unwrap();
        assert!(map_graph(dto).is_err());
    }
}
