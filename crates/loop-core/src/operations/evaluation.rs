//! Checked-transition evaluation lineage and provider-request construction.
//!
//! Persistence supplies the complete ordered durable checked-evaluation set in
//! a [`CheckedEvaluationSnapshot`].  Core applies the workflow identity rule
//! here: a lineage is the records for the exact source-state + event pair.
//! This keeps provider requests independent of raw run history and of any
//! persistence representation.

use crate::{CheckedEvaluationSnapshot, DurableEvaluation, EvaluationRequest, Transition};

/// Derive durable evaluation lineage for one exact checked transition.
///
/// The persistence contract supplies only durable allow/deny records, but the
/// checked-kind guard keeps this projection correct for defensive fakes and
/// makes the boundary explicit.  A transition's lineage identity is its
/// source state plus event ID; target and kind are not part of the identity.
/// Results are returned in semantic-sequence order, regardless of the order
/// used by a simple adapter or test double.
pub fn lineage_for_transition(
    transition: &Transition,
    evaluations: &[DurableEvaluation],
) -> Vec<DurableEvaluation> {
    if !transition.kind.is_checked() {
        return Vec::new();
    }

    let mut lineage = evaluations
        .iter()
        .filter(|evaluation| {
            evaluation.transition.kind.is_checked()
                && evaluation.transition.same_lineage(transition)
        })
        .cloned()
        .collect::<Vec<_>>();
    lineage.sort_by_key(|evaluation| evaluation.sequence);
    lineage
}

/// Construct the provider request represented by one consistent durable
/// checked-evaluation snapshot.
///
/// The snapshot is consumed only by cloning its immutable run data and read
/// records.  The observed control revision remains on the snapshot for the
/// caller's later conditional commit; it is deliberately not sent as
/// provider input.
pub fn request_from_snapshot(snapshot: &CheckedEvaluationSnapshot) -> EvaluationRequest {
    let mut context = snapshot.context.clone();
    context.sort_by_key(|record| record.sequence);

    EvaluationRequest::new(
        snapshot.run.workflow.clone(),
        snapshot.run.initial_input.clone(),
        context,
        snapshot.transition.clone(),
        lineage_for_transition(&snapshot.transition, &snapshot.checked_evaluations),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextRecord, ControlRevision, EvaluationFeedback, Lifecycle, ProviderAssociation, Run,
        SemanticSequence, State, Timestamp, TransitionKind, Workflow,
    };
    use serde_json::json;

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do the work", false),
                State::new("review", "Review", "Review the work", false),
                State::new("other", "Other", "Other work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![
                Transition::checked("start", "submit", "review"),
                Transition::checked("start", "other", "other"),
                Transition::check_free("review", "revise", "start"),
            ],
        )
    }

    fn run() -> Run {
        Run::new(
            "run-1",
            Some("lineage".to_owned()),
            workflow(),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "preserve context"}),
            "start",
            Lifecycle::Active,
            ControlRevision::from_u64(9),
            SemanticSequence::new(8),
            Timestamp::from_unix_millis(1),
        )
    }

    #[test]
    fn lineage_contains_only_exact_checked_transition_in_semantic_order() {
        let selected = Transition::checked("start", "submit", "review");
        let same_lineage_deny = DurableEvaluation::deny(
            selected.clone(),
            EvaluationFeedback::new("needs-work", "Fix the review findings"),
            SemanticSequence::new(6),
            Timestamp::from_unix_millis(6),
        );
        let unrelated_event = DurableEvaluation::allow(
            Transition::checked("start", "other", "other"),
            SemanticSequence::new(4),
            Timestamp::from_unix_millis(4),
        );
        let same_lineage_allow = DurableEvaluation::allow(
            Transition::checked("start", "submit", "another-target"),
            SemanticSequence::new(8),
            Timestamp::from_unix_millis(8),
        );
        let check_free_same_pair = DurableEvaluation::allow(
            Transition::new("start", "submit", "review", TransitionKind::CheckFree),
            SemanticSequence::new(9),
            Timestamp::from_unix_millis(9),
        );

        let lineage = lineage_for_transition(
            &selected,
            &[
                same_lineage_allow.clone(),
                check_free_same_pair,
                unrelated_event,
                same_lineage_deny.clone(),
            ],
        );

        assert_eq!(lineage, vec![same_lineage_deny, same_lineage_allow]);
    }

    #[test]
    fn request_contains_stored_workflow_input_ordered_context_and_lineage() {
        let transition = Transition::checked("start", "submit", "review");
        let first_context = ContextRecord::new(
            "first",
            "observation",
            json!({"value": 1}),
            SemanticSequence::new(3),
            Timestamp::from_unix_millis(3),
        );
        let second_context = ContextRecord::new(
            "second",
            "review",
            json!({"value": 2}),
            SemanticSequence::new(7),
            Timestamp::from_unix_millis(7),
        );
        let prior = DurableEvaluation::deny(
            transition.clone(),
            EvaluationFeedback::new("missing-evidence", "Add evidence"),
            SemanticSequence::new(5),
            Timestamp::from_unix_millis(5),
        );
        let snapshot = CheckedEvaluationSnapshot {
            run: run(),
            observed_control_revision: ControlRevision::from_u64(9),
            transition: transition.clone(),
            context: vec![second_context.clone(), first_context.clone()],
            checked_evaluations: vec![prior.clone()],
        };

        let request = request_from_snapshot(&snapshot);

        assert_eq!(request.workflow, snapshot.run.workflow);
        assert_eq!(request.initial_input, snapshot.run.initial_input);
        assert_eq!(request.transition, transition);
        assert_eq!(request.context, vec![first_context, second_context]);
        assert_eq!(request.prior_evaluations, vec![prior]);
    }

    #[test]
    fn check_free_transition_has_no_evaluation_lineage() {
        let transition = Transition::check_free("review", "revise", "start");
        let durable = DurableEvaluation::allow(
            Transition::checked("review", "revise", "start"),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(1),
        );

        assert!(lineage_for_transition(&transition, &[durable]).is_empty());
    }
}
