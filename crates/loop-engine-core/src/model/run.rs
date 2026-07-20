use thiserror::Error;

use super::bounded::{BoundError, BoundedText, RUN_LABEL_UTF8_BYTES};
use super::graph::WorkflowGraph;
use super::graph_validation::ValidatedGraph;
use super::ids::{GraphRevision, RegistrationId, RunId, StateId};
use super::lifecycle::Lifecycle;
use super::run_input::RunInputs;
use super::version::{LifecycleVersion, WorkflowStateVersion};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunMutationError {
    #[error("run lifecycle is terminal")]
    Terminal,
    #[error("run changed after decision resolution")]
    Stale,
    #[error("target state is absent from stored graph: {0}")]
    UnknownTarget(StateId),
    #[error("resolved lifecycle is inconsistent with target state")]
    InvalidLifecycle,
    #[error("state version exhausted")]
    VersionExhausted,
    #[error(transparent)]
    Bound(#[from] BoundError),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunRestoreError {
    #[error("stored current state is absent from stored graph: {0}")]
    UnknownCurrentState(StateId),
    #[error("stored lifecycle is inconsistent with stored current state")]
    InvalidLifecycle,
    #[error(transparent)]
    Bound(#[from] BoundError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    id: RunId,
    registration_id: RegistrationId,
    graph: WorkflowGraph,
    graph_revision: GraphRevision,
    inputs: RunInputs,
    current_state: StateId,
    lifecycle: Lifecycle,
    workflow_state_version: WorkflowStateVersion,
    lifecycle_version: LifecycleVersion,
    label: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
}

impl Run {
    pub fn create(
        id: RunId,
        registration_id: RegistrationId,
        graph: ValidatedGraph,
        graph_revision: GraphRevision,
        inputs: RunInputs,
        label: Option<String>,
    ) -> Result<Self, BoundError> {
        let current_state = graph.graph().initial_state().clone();
        let lifecycle = if graph
            .graph()
            .state(&current_state)
            .is_some_and(|state| state.is_final())
        {
            Lifecycle::Final
        } else {
            Lifecycle::Active
        };
        Ok(Self {
            id,
            registration_id,
            graph: graph.into_graph(),
            graph_revision,
            inputs,
            current_state,
            lifecycle,
            workflow_state_version: WorkflowStateVersion::initial(),
            lifecycle_version: LifecycleVersion::initial(),
            label: parse_label(label)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: RunId,
        registration_id: RegistrationId,
        graph: ValidatedGraph,
        graph_revision: GraphRevision,
        inputs: RunInputs,
        current_state: StateId,
        lifecycle: Lifecycle,
        workflow_state_version: WorkflowStateVersion,
        lifecycle_version: LifecycleVersion,
        label: Option<String>,
    ) -> Result<Self, RunRestoreError> {
        let state = graph
            .graph()
            .state(&current_state)
            .ok_or_else(|| RunRestoreError::UnknownCurrentState(current_state.clone()))?;
        let lifecycle_valid = match lifecycle {
            Lifecycle::Active => !state.is_final(),
            Lifecycle::Final => state.is_final(),
            Lifecycle::Terminated => true,
        };
        if !lifecycle_valid {
            return Err(RunRestoreError::InvalidLifecycle);
        }
        Ok(Self {
            id,
            registration_id,
            graph: graph.into_graph(),
            graph_revision,
            inputs,
            current_state,
            lifecycle,
            workflow_state_version,
            lifecycle_version,
            label: parse_label(label)?,
        })
    }

    pub fn id(&self) -> &RunId {
        &self.id
    }

    pub fn registration_id(&self) -> &RegistrationId {
        &self.registration_id
    }

    pub fn graph(&self) -> &WorkflowGraph {
        &self.graph
    }

    pub fn graph_revision(&self) -> &GraphRevision {
        &self.graph_revision
    }

    pub fn inputs(&self) -> &RunInputs {
        &self.inputs
    }

    pub fn current_state(&self) -> &StateId {
        &self.current_state
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn workflow_state_version(&self) -> WorkflowStateVersion {
        self.workflow_state_version
    }

    pub fn lifecycle_version(&self) -> LifecycleVersion {
        self.lifecycle_version
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_ref().map(BoundedText::as_str)
    }

    pub(crate) fn apply_state(
        &mut self,
        target: StateId,
        lifecycle: Lifecycle,
        expected_workflow_version: WorkflowStateVersion,
        expected_lifecycle_version: LifecycleVersion,
    ) -> Result<(), RunMutationError> {
        if self.lifecycle.is_terminal() {
            return Err(RunMutationError::Terminal);
        }
        if self.workflow_state_version != expected_workflow_version
            || self.lifecycle_version != expected_lifecycle_version
        {
            return Err(RunMutationError::Stale);
        }
        let target_state = self
            .graph
            .state(&target)
            .ok_or_else(|| RunMutationError::UnknownTarget(target.clone()))?;
        let expected_lifecycle = if target_state.is_final() {
            Lifecycle::Final
        } else {
            Lifecycle::Active
        };
        if lifecycle != expected_lifecycle {
            return Err(RunMutationError::InvalidLifecycle);
        }
        let state_changed = self.current_state != target;
        let lifecycle_changed = self.lifecycle != lifecycle;
        let next_workflow = if state_changed {
            Some(
                self.workflow_state_version
                    .next()
                    .ok_or(RunMutationError::VersionExhausted)?,
            )
        } else {
            None
        };
        let next_lifecycle = if lifecycle_changed {
            Some(
                self.lifecycle_version
                    .next()
                    .ok_or(RunMutationError::VersionExhausted)?,
            )
        } else {
            None
        };
        self.current_state = target;
        self.lifecycle = lifecycle;
        if let Some(version) = next_workflow {
            self.workflow_state_version = version;
        }
        if let Some(version) = next_lifecycle {
            self.lifecycle_version = version;
        }
        Ok(())
    }

    pub fn set_label(&mut self, label: Option<String>) -> Result<(), RunMutationError> {
        if self.lifecycle.is_terminal() {
            return Err(RunMutationError::Terminal);
        }
        self.label = parse_label(label)?;
        Ok(())
    }

    pub fn terminate(&mut self) -> Result<(), RunMutationError> {
        if self.lifecycle.is_terminal() {
            return Err(RunMutationError::Terminal);
        }
        let next = self
            .lifecycle_version
            .next()
            .ok_or(RunMutationError::VersionExhausted)?;
        self.lifecycle = Lifecycle::Terminated;
        self.lifecycle_version = next;
        Ok(())
    }
}

fn parse_label(
    label: Option<String>,
) -> Result<Option<BoundedText<RUN_LABEL_UTF8_BYTES>>, BoundError> {
    label
        .map(|value| BoundedText::non_empty("run_label", value))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::{
        GraphRevision, Lifecycle, LifecycleVersion, RegistrationId, Run, RunId, RunInputs,
        RunMutationError, StateId, ValidatedGraph, WorkflowGraph, WorkflowStateVersion,
    };
    use crate::model::graph::State;
    use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use crate::model::run_input::InputDeclarations;

    fn graph(initial_final: bool) -> ValidatedGraph {
        let state = State::new(
            StateId::parse("initial").unwrap(),
            initial_final,
            StaticGuidance::NoneRequired,
            None,
        );
        ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
            StateId::parse("initial").unwrap(),
            vec![state],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        ))
        .unwrap()
    }

    fn revision() -> GraphRevision {
        GraphRevision::parse(format!("sha256:{}", "0".repeat(64))).unwrap()
    }

    fn run(initial_final: bool) -> Run {
        Run::create(
            RunId::parse("run").unwrap(),
            RegistrationId::parse("registration").unwrap(),
            graph(initial_final),
            revision(),
            RunInputs::default(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn initial_final_and_terminal_no_reopen() {
        assert_eq!(run(true).lifecycle(), Lifecycle::Final);
        let mut active = run(false);
        active.terminate().unwrap();
        assert!(matches!(
            active.terminate(),
            Err(RunMutationError::Terminal)
        ));
        assert!(matches!(
            active.set_label(Some("new".into())),
            Err(RunMutationError::Terminal)
        ));
    }

    #[test]
    fn persisted_active_run_reconstitutes_with_versions() {
        let restored = Run::restore(
            RunId::parse("run").unwrap(),
            RegistrationId::parse("registration").unwrap(),
            graph(false),
            revision(),
            RunInputs::default(),
            StateId::parse("initial").unwrap(),
            Lifecycle::Active,
            WorkflowStateVersion::try_from(41).unwrap(),
            LifecycleVersion::try_from(2).unwrap(),
            Some("label".into()),
        )
        .unwrap();
        assert_eq!(restored.workflow_state_version().value(), 41);
        assert_eq!(restored.lifecycle_version().value(), 2);
        assert_eq!(restored.label(), Some("label"));
    }

    #[test]
    fn label_is_optional_and_non_unique() {
        let mut first = run(false);
        let mut second = run(false);
        first.set_label(Some("same".into())).unwrap();
        second.set_label(Some("same".into())).unwrap();
        assert_eq!(first.label(), second.label());
    }
}
