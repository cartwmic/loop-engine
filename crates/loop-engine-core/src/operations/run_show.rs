use crate::capabilities::run_reader::RunLookup;
use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use crate::model::ids::{EvidenceId, GraphRevision, RunId, StateId};
use crate::model::lifecycle::Lifecycle;
use crate::model::requestable::{RequestableEvent, project as requestable_events};
use crate::model::run::Run;
use crate::model::run_input::RunInputs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunShow {
    pub run_id: RunId,
    pub label: Option<String>,
    pub graph_revision: GraphRevision,
    pub lifecycle: Lifecycle,
    pub current_state: StateId,
    pub inputs: RunInputs,
    pub static_guidance: StaticGuidance,
    pub live_guidance: LiveGuidanceCapability,
    pub selected_evidence: Vec<EvidenceId>,
    pub requestable_events: Vec<RequestableEvent>,
}

pub fn execute<R: RunLookup>(reader: &R, run_id: &RunId) -> Result<RunShow, R::Error> {
    reader
        .get_for_operation("run.show", run_id)
        .map(|run| project(&run))
}

pub fn project(run: &Run) -> RunShow {
    let state = run
        .graph()
        .state(run.current_state())
        .expect("run current state is validated");
    RunShow {
        run_id: run.id().clone(),
        label: run.label().map(str::to_owned),
        graph_revision: run.graph_revision().clone(),
        lifecycle: run.lifecycle(),
        current_state: run.current_state().clone(),
        inputs: run.inputs().clone(),
        static_guidance: state.guidance().clone(),
        live_guidance: run.graph().live_guidance(),
        selected_evidence: vec![],
        requestable_events: requestable_events(run),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn projection_is_provider_free_and_preserves_authoritative_state() {
        let run = crate::operations::test_support::run();
        let show = super::project(&run);
        assert_eq!(show.run_id, *run.id());
        assert_eq!(show.current_state, *run.current_state());
        assert_eq!(show.label.as_deref(), run.label());
        assert!(show.selected_evidence.is_empty());
    }
}
