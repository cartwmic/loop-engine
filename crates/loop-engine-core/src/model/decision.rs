use thiserror::Error;

use super::diagnostic::Diagnostic;
use super::evidence::EvidenceRecord;
use super::gate::{GateEvaluation, GateVerdict, VerdictSetError, validate_verdict_set};
use super::ids::{EventId, GateId, RunId, StateId};
use super::lifecycle::Lifecycle;
use super::run::{Run, RunMutationError};
use super::version::{LifecycleVersion, WorkflowStateVersion};

#[derive(Debug, PartialEq, Eq)]
pub struct TransitionDecision {
    run_id: RunId,
    source: StateId,
    event: EventId,
    target: StateId,
    required_gates: Vec<GateId>,
    expected_workflow_version: WorkflowStateVersion,
    expected_lifecycle_version: LifecycleVersion,
    lifecycle: Lifecycle,
    state_changed: bool,
    provider_evidence: Vec<EvidenceRecord>,
}

impl TransitionDecision {
    pub fn target(&self) -> &StateId {
        &self.target
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn state_changed(&self) -> bool {
        self.state_changed
    }

    pub fn provider_evidence(&self) -> &[EvidenceRecord] {
        &self.provider_evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecisionError {
    #[error("run lifecycle is terminal")]
    Terminal,
    #[error("event is unknown: {0}")]
    UnknownEvent(EventId),
    #[error("event is ambiguous: {0}")]
    AmbiguousEvent(EventId),
    #[error("transition requires provider gate evaluation")]
    GatesRequired,
    #[error("one or more gates failed")]
    GateFailed { verdicts: Vec<GateVerdict> },
    #[error("stored declarations are incompatible with provider")]
    Incompatible { diagnostics: Vec<Diagnostic> },
    #[error("provider evaluation failed")]
    EvaluationError { diagnostics: Vec<Diagnostic> },
    #[error("malformed gate verdict set: {source}")]
    MalformedVerdicts {
        source: VerdictSetError,
        verdicts: Vec<GateVerdict>,
    },
    #[error("decision does not belong to current run or stored transition")]
    DecisionMismatch,
    #[error(transparent)]
    Mutation(#[from] RunMutationError),
}

fn selected_transition<'a>(
    run: &'a Run,
    event: &EventId,
) -> Result<&'a super::transition::Transition, DecisionError> {
    if run.lifecycle() != Lifecycle::Active {
        return Err(DecisionError::Terminal);
    }
    let matches = run
        .graph()
        .transitions()
        .iter()
        .filter(|transition| {
            transition.source() == run.current_state() && transition.event() == event
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(DecisionError::UnknownEvent(event.clone())),
        [transition] => Ok(*transition),
        _ => Err(DecisionError::AmbiguousEvent(event.clone())),
    }
}

fn decision(
    run: &Run,
    event: EventId,
    target: StateId,
    required_gates: Vec<GateId>,
    provider_evidence: Vec<EvidenceRecord>,
) -> TransitionDecision {
    let lifecycle = if run
        .graph()
        .state(&target)
        .is_some_and(|state| state.is_final())
    {
        Lifecycle::Final
    } else {
        Lifecycle::Active
    };
    TransitionDecision {
        run_id: run.id().clone(),
        source: run.current_state().clone(),
        event,
        state_changed: target != *run.current_state(),
        target,
        required_gates,
        expected_workflow_version: run.workflow_state_version(),
        expected_lifecycle_version: run.lifecycle_version(),
        lifecycle,
        provider_evidence,
    }
}

pub fn resolve_gate_free(run: &Run, event: &EventId) -> Result<TransitionDecision, DecisionError> {
    let transition = selected_transition(run, event)?;
    if !transition.required_gates().is_empty() {
        return Err(DecisionError::GatesRequired);
    }
    Ok(decision(
        run,
        event.clone(),
        transition.target().clone(),
        vec![],
        vec![],
    ))
}

pub fn resolve_gated(
    run: &Run,
    event: &EventId,
    evaluation: &GateEvaluation,
) -> Result<TransitionDecision, DecisionError> {
    let transition = selected_transition(run, event)?;
    if transition.required_gates().is_empty() {
        return resolve_gate_free(run, event);
    }
    match evaluation {
        GateEvaluation::Incompatible(diagnostics) => Err(DecisionError::Incompatible {
            diagnostics: diagnostics.as_slice().to_vec(),
        }),
        GateEvaluation::EvaluationError(diagnostics) => Err(DecisionError::EvaluationError {
            diagnostics: diagnostics.as_slice().to_vec(),
        }),
        GateEvaluation::Verdicts(verdicts) => {
            let exact = validate_verdict_set(transition.required_gates(), verdicts.as_slice())
                .map_err(|source| DecisionError::MalformedVerdicts {
                    source,
                    verdicts: verdicts.as_slice().to_vec(),
                })?;
            if exact.values().any(|verdict| !verdict.passed()) {
                return Err(DecisionError::GateFailed {
                    verdicts: verdicts.as_slice().to_vec(),
                });
            }
            let evidence = exact
                .values()
                .flat_map(|verdict| verdict.evidence().iter().cloned())
                .collect();
            Ok(decision(
                run,
                event.clone(),
                transition.target().clone(),
                transition.required_gates().to_vec(),
                evidence,
            ))
        }
    }
}

pub fn apply(run: &mut Run, decision: TransitionDecision) -> Result<(), DecisionError> {
    let stored_match = run.id() == &decision.run_id
        && run.current_state() == &decision.source
        && run.graph().transitions().iter().any(|transition| {
            transition.source() == &decision.source
                && transition.event() == &decision.event
                && transition.target() == &decision.target
                && transition.required_gates() == decision.required_gates
        });
    if !stored_match {
        return Err(DecisionError::DecisionMismatch);
    }
    run.apply_state(
        decision.target,
        decision.lifecycle,
        decision.expected_workflow_version,
        decision.expected_lifecycle_version,
    )?;
    Ok(())
}
