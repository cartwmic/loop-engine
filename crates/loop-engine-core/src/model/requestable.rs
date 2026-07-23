use super::graph::WorkflowGraph;
use super::ids::{EventId, GateId, StateId};
use super::lifecycle::Lifecycle;
use super::run::Run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestableEvent {
    pub event: EventId,
    pub target: StateId,
    pub required_gates: Vec<GateId>,
}

pub fn project(run: &Run) -> Vec<RequestableEvent> {
    project_state(run.graph(), run.lifecycle(), run.current_state())
}

pub fn project_state(
    graph: &WorkflowGraph,
    lifecycle: Lifecycle,
    current_state: &StateId,
) -> Vec<RequestableEvent> {
    if lifecycle != Lifecycle::Active {
        return vec![];
    }
    let mut events = graph
        .transitions()
        .iter()
        .filter(|transition| transition.source() == current_state)
        .map(|transition| RequestableEvent {
            event: transition.event().clone(),
            target: transition.target().clone(),
            required_gates: transition.required_gates().to_vec(),
        })
        .collect::<Vec<_>>();
    events.sort_by(|left, right| left.event.cmp(&right.event));
    events
}

#[cfg(test)]
mod tests {
    use super::{EventId, GateId, StateId, project};
    use crate::model::graph::{State, WorkflowGraph};
    use crate::model::graph_validation::ValidatedGraph;
    use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use crate::model::ids::{GraphRevision, RegistrationId, RunId};
    use crate::model::run::Run;
    use crate::model::run_input::{InputDeclarations, RunInputs};
    use crate::model::transition::Transition;

    fn run(final_initial: bool, transitions: Vec<Transition>) -> Run {
        let graph = WorkflowGraph::new_unvalidated(
            StateId::parse("a").unwrap(),
            vec![
                State::new(
                    StateId::parse("a").unwrap(),
                    final_initial,
                    StaticGuidance::NoneRequired,
                    None,
                ),
                State::new(
                    StateId::parse("b").unwrap(),
                    false,
                    StaticGuidance::NoneRequired,
                    None,
                ),
            ],
            transitions,
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        Run::create(
            RunId::parse("run").unwrap(),
            RegistrationId::parse("registration").unwrap(),
            ValidatedGraph::validate(graph).unwrap(),
            GraphRevision::parse(format!("sha256:{}", "0".repeat(64))).unwrap(),
            RunInputs::default(),
            None,
        )
        .unwrap()
    }

    fn transition(event: &str, target: &str, gate: Option<&str>) -> Transition {
        Transition::new(
            StateId::parse("a").unwrap(),
            EventId::parse(event).unwrap(),
            StateId::parse(target).unwrap(),
            gate.into_iter()
                .map(|value| GateId::parse(value).unwrap())
                .collect(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn sink_multi_event_self_loop_and_terminal_projection() {
        assert!(project(&run(false, vec![])).is_empty());
        let active = run(
            false,
            vec![
                transition("z", "b", Some("gate")),
                transition("a", "a", None),
            ],
        );
        let events = project(&active);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_str(), "a");
        assert_eq!(events[0].target.as_str(), "a");
        assert_eq!(events[1].required_gates[0].as_str(), "gate");
        assert!(project(&run(true, vec![])).is_empty());
    }
}
