use std::collections::BTreeMap;

use loop_engine_core::model::bounded::{FiniteNumber, Metadata, Value};
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_projection::{CANONICAL_GRAPH_VERSION, SemanticGraphProjection};
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{EventId, GateId, InputKind, InputName, StateId};
use loop_engine_core::model::run_input::{InputDeclaration, InputDeclarations};
use loop_engine_core::model::transition::Transition;

fn metadata(key: &str, value: Value) -> Option<Metadata> {
    Metadata::new(
        "metadata",
        BTreeMap::from([(key.to_owned(), value)]),
        65_536,
    )
    .unwrap()
}

fn state(id: &str) -> State {
    State::new(
        StateId::parse(id).unwrap(),
        id == "done",
        StaticGuidance::text("guidance").unwrap(),
        metadata("state", Value::String(id.into())),
    )
}

fn projection(reverse: bool, marker: &str) -> SemanticGraphProjection {
    let mut states = vec![state("work"), state("done")];
    let mut gates = vec![
        GateId::parse("review").unwrap(),
        GateId::parse("test").unwrap(),
    ];
    let mut inputs = vec![
        InputDeclaration::new(
            InputName::parse("scope").unwrap(),
            InputKind::parse("text").unwrap(),
            true,
            metadata("input", Value::Bool(true)),
        ),
        InputDeclaration::new(
            InputName::parse("mode").unwrap(),
            InputKind::parse("token").unwrap(),
            false,
            None,
        ),
    ];
    if reverse {
        states.reverse();
        gates.reverse();
        inputs.reverse();
    }
    let transitions = vec![
        Transition::new(
            StateId::parse("work").unwrap(),
            EventId::parse("finish").unwrap(),
            StateId::parse("done").unwrap(),
            gates,
            metadata(
                "weight",
                Value::Number(FiniteNumber::new("number", 1.0).unwrap()),
            ),
        )
        .unwrap(),
    ];
    let graph = WorkflowGraph::new_unvalidated(
        StateId::parse("work").unwrap(),
        states,
        transitions,
        InputDeclarations::new(inputs).unwrap(),
        LiveGuidanceCapability::Supported,
        metadata("marker", Value::String(marker.into())),
    );
    SemanticGraphProjection::from_validated(&ValidatedGraph::validate(graph).unwrap())
}

#[test]
fn reorder_equivalent_graphs_have_equal_semantic_projection() {
    let actual = projection(false, "same");
    assert_eq!(actual, projection(true, "same"));
    assert_eq!(actual.canonical_graph_version, CANONICAL_GRAPH_VERSION);
    assert_eq!(actual.transitions[0].required_gates[0].as_str(), "review");
    assert!(actual.transitions[0].metadata.is_some());
    assert!(actual.states.iter().all(|state| state.metadata.is_some()));
    assert_eq!(actual.inputs[0].name.as_str(), "mode");
    assert_eq!(actual.inputs[0].kind.as_str(), "token");
    assert!(actual.inputs[1].metadata.is_some());
    assert!(actual.metadata.is_some());
}

#[test]
fn every_projected_metadata_layer_changes_semantic_projection() {
    assert_ne!(projection(false, "one"), projection(false, "two"));
    assert_eq!(
        FiniteNumber::new("number", 1.0).unwrap(),
        FiniteNumber::new("number", 1e0).unwrap()
    );
    assert_eq!(
        Metadata::new("metadata", BTreeMap::new(), 100).unwrap(),
        None
    );
}
