//! The provider-free `show` projection.
//!
//! Persistence supplies authoritative run state, ordered context, and all
//! durable checked evaluations.  Core derives requestable events and the
//! latest evaluation for each exact checked transition without invoking a
//! provider or loading raw history.

use super::persistence_error;
use crate::{
    DurableEvaluation, EventId, Lifecycle, OperationOutcome, Persistence, RunId, ShowData, StateId,
    Transition, TransitionKind, WorkflowId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Caller-supplied run identity for a `show` read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
}

impl Request {
    pub fn new(run_id: impl Into<RunId>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

/// One event currently requestable from the authoritative run state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestableEvent {
    pub event: EventId,
    pub target: StateId,
    pub kind: TransitionKind,
}

impl RequestableEvent {
    pub fn from_transition(transition: &Transition) -> Self {
        Self {
            event: transition.event.clone(),
            target: transition.target.clone(),
            kind: transition.kind,
        }
    }

    /// The event identity, named explicitly for callers that use the PRD
    /// terminology rather than the transition model's `event` field.
    pub fn event_id(&self) -> &EventId {
        &self.event
    }

    pub const fn is_checked(&self) -> bool {
        self.kind.is_checked()
    }

    pub const fn is_check_free(&self) -> bool {
        self.kind.is_check_free()
    }
}

/// A malformed authoritative run state that prevents a complete `show`
/// projection.  Normal persistence implementations cannot produce this for a
/// run created through `start`, but returning an error is safer than panicking
/// if a fake or corrupted adapter does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    UndefinedCurrentState { state: StateId },
}

impl ProjectionError {
    pub const fn code(&self) -> &'static str {
        "invalid-run"
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedCurrentState { state } => write!(
                formatter,
                "authoritative current state `{state}` is absent from the stored workflow"
            ),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// Complete provider-free continuation projection returned by `show`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShowProjection {
    pub run_id: RunId,
    pub label: Option<String>,
    pub workflow_id: WorkflowId,
    pub lifecycle: Lifecycle,
    pub current_state: StateId,
    pub current_state_title: String,
    pub current_state_instructions: String,
    pub initial_input: Value,
    pub context: Vec<crate::ContextRecord>,
    pub requestable_events: Vec<RequestableEvent>,
    /// One latest durable allow/deny record for every exact checked
    /// transition that has been evaluated, ordered by that record's semantic
    /// sequence.  This intentionally includes transitions that are no longer
    /// requestable from the current state.
    pub latest_evaluations: Vec<DurableEvaluation>,
}

impl ShowProjection {
    pub fn current_state_id(&self) -> &StateId {
        &self.current_state
    }

    pub fn state_title(&self) -> &str {
        &self.current_state_title
    }

    pub fn state_instructions(&self) -> &str {
        &self.current_state_instructions
    }

    pub fn evaluations(&self) -> &[DurableEvaluation] {
        &self.latest_evaluations
    }
}

/// Derive one latest durable evaluation per exact checked transition.
///
/// The input is the complete ordered durable set returned by persistence.  A
/// later allow replaces an earlier deny and a later deny replaces an earlier
/// allow.  Semantic sequence, rather than timestamps or workflow position,
/// determines both replacement and output order.
pub fn latest_evaluations(evaluations: &[DurableEvaluation]) -> Vec<DurableEvaluation> {
    let mut ordered = evaluations.to_vec();
    ordered.sort_by_key(|evaluation| evaluation.sequence);

    let mut latest: Vec<DurableEvaluation> = Vec::new();
    for evaluation in ordered {
        if !evaluation.transition.kind.is_checked() {
            continue;
        }

        if let Some(existing) = latest
            .iter_mut()
            .find(|existing| existing.transition.same_lineage(&evaluation.transition))
        {
            *existing = evaluation;
        } else {
            latest.push(evaluation);
        }
    }

    latest.sort_by_key(|evaluation| evaluation.sequence);
    latest
}

/// Project durable show data into the complete continuation view.
pub fn project(data: ShowData) -> std::result::Result<ShowProjection, ProjectionError> {
    let current_state = data
        .run
        .workflow
        .states
        .iter()
        .find(|state| state.id == data.run.current_state)
        .ok_or_else(|| ProjectionError::UndefinedCurrentState {
            state: data.run.current_state.clone(),
        })?;

    let mut context = data.context;
    context.sort_by_key(|record| record.sequence);

    let requestable_events = if data.run.lifecycle.is_terminal() {
        Vec::new()
    } else {
        data.run
            .workflow
            .transitions
            .iter()
            .filter(|transition| transition.source == data.run.current_state)
            .map(RequestableEvent::from_transition)
            .collect()
    };

    Ok(ShowProjection {
        run_id: data.run.id,
        label: data.run.label,
        workflow_id: data.run.workflow.id,
        lifecycle: data.run.lifecycle,
        current_state: data.run.current_state,
        current_state_title: current_state.title.clone(),
        current_state_instructions: current_state.instructions.clone(),
        initial_input: data.run.initial_input,
        context,
        requestable_events,
        latest_evaluations: latest_evaluations(&data.checked_evaluations),
    })
}

/// Execute a provider-free `show` read.
pub fn execute<P>(request: Request, persistence: &P) -> OperationOutcome<ShowProjection>
where
    P: Persistence + ?Sized,
{
    match persistence.load_show_data(&request.run_id) {
        Ok(data) => match project(data) {
            Ok(projection) => OperationOutcome::completed(projection),
            Err(error) => OperationOutcome::error(error.code(), error.to_string()),
        },
        Err(error) => persistence_error(error),
    }
}

/// Execute `show` with persistence first.
pub fn execute_with_persistence<P>(
    persistence: &P,
    request: Request,
) -> OperationOutcome<ShowProjection>
where
    P: Persistence + ?Sized,
{
    execute(request, persistence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextRecord, ControlRevision, EvaluationFeedback, ProviderAssociation, Run,
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
                Transition::check_free("start", "skip", "other"),
                Transition::checked("review", "approve", "done"),
                Transition::checked("review", "revise", "start"),
                Transition::checked("other", "return", "start"),
            ],
        )
    }

    fn run(current_state: &str, lifecycle: Lifecycle) -> Run {
        Run::new(
            "run-1",
            Some("show me".to_owned()),
            workflow(),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "ship safely"}),
            current_state,
            lifecycle,
            ControlRevision::from_u64(3),
            SemanticSequence::new(10),
            Timestamp::from_unix_millis(1),
        )
    }

    fn context(id: &str, sequence: u64) -> ContextRecord {
        ContextRecord::new(
            id,
            "observation",
            json!({"id": id}),
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn show_data(
        current_state: &str,
        lifecycle: Lifecycle,
        context: Vec<ContextRecord>,
        checked_evaluations: Vec<DurableEvaluation>,
    ) -> ShowData {
        ShowData {
            run: run(current_state, lifecycle),
            context,
            checked_evaluations,
        }
    }

    #[test]
    fn context_is_preserved_in_durable_append_order() {
        let projection = project(show_data(
            "start",
            Lifecycle::Active,
            vec![context("second", 8), context("first", 4)],
            vec![],
        ))
        .unwrap();

        assert_eq!(
            projection
                .context
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[test]
    fn requestable_events_include_target_and_kind() {
        let projection = project(show_data("start", Lifecycle::Active, vec![], vec![])).unwrap();

        assert_eq!(projection.requestable_events.len(), 2);
        assert_eq!(
            projection.requestable_events[0].event,
            EventId::from("submit")
        );
        assert_eq!(
            projection.requestable_events[0].target,
            StateId::from("review")
        );
        assert_eq!(
            projection.requestable_events[0].kind,
            TransitionKind::Checked
        );
        assert_eq!(
            projection.requestable_events[1].event,
            EventId::from("skip")
        );
        assert_eq!(
            projection.requestable_events[1].target,
            StateId::from("other")
        );
        assert_eq!(
            projection.requestable_events[1].kind,
            TransitionKind::CheckFree
        );
    }

    #[test]
    fn latest_projection_handles_multiple_denials_and_deny_to_allow() {
        let transition = Transition::checked("start", "submit", "review");
        let first_deny = DurableEvaluation::deny(
            transition.clone(),
            EvaluationFeedback::new("missing-a", "Add A"),
            SemanticSequence::new(3),
            Timestamp::from_unix_millis(3),
        );
        let second_deny = DurableEvaluation::deny(
            transition.clone(),
            EvaluationFeedback::new("missing-b", "Add B"),
            SemanticSequence::new(5),
            Timestamp::from_unix_millis(5),
        );
        let allow = DurableEvaluation::allow(
            transition,
            SemanticSequence::new(7),
            Timestamp::from_unix_millis(7),
        );

        let latest = latest_evaluations(&[allow.clone(), first_deny, second_deny]);
        assert_eq!(latest, vec![allow]);
        assert!(latest[0].is_allow());
    }

    #[test]
    fn latest_projection_handles_allow_to_deny_and_exposes_feedback() {
        let transition = Transition::checked("start", "submit", "review");
        let allow = DurableEvaluation::allow(
            transition.clone(),
            SemanticSequence::new(3),
            Timestamp::from_unix_millis(3),
        );
        let feedback = EvaluationFeedback::new("new-finding", "Address the new finding")
            .with_details(json!({"line": 42}));
        let deny = DurableEvaluation::deny(
            transition,
            feedback.clone(),
            SemanticSequence::new(6),
            Timestamp::from_unix_millis(6),
        );

        let latest = latest_evaluations(&[deny, allow]);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].feedback(), Some(&feedback));
    }

    #[test]
    fn evaluations_survive_leaving_and_returning_to_a_state() {
        let old_edge = Transition::checked("review", "approve", "done");
        let evaluation = DurableEvaluation::deny(
            old_edge.clone(),
            EvaluationFeedback::new("revise", "Revise before approval"),
            SemanticSequence::new(12),
            Timestamp::from_unix_millis(12),
        );

        let projection = project(show_data(
            "start",
            Lifecycle::Active,
            vec![],
            vec![evaluation.clone()],
        ))
        .unwrap();

        assert_eq!(projection.latest_evaluations, vec![evaluation]);
        assert!(projection.requestable_events.iter().all(|event| {
            event.event != EventId::from("approve") && event.event != EventId::from("revise")
        }));
    }

    #[test]
    fn unrelated_transition_lineage_is_not_collapsed_into_selected_transition() {
        let selected = Transition::checked("start", "submit", "review");
        let unrelated = DurableEvaluation::deny(
            Transition::checked("start", "other", "other"),
            EvaluationFeedback::new("other", "Other feedback"),
            SemanticSequence::new(4),
            Timestamp::from_unix_millis(4),
        );
        let selected_deny = DurableEvaluation::deny(
            selected,
            EvaluationFeedback::new("selected", "Selected feedback"),
            SemanticSequence::new(6),
            Timestamp::from_unix_millis(6),
        );
        let latest = latest_evaluations(&[selected_deny.clone(), unrelated.clone()]);

        assert_eq!(latest, vec![unrelated, selected_deny]);
    }

    #[test]
    fn terminal_show_exposes_no_requestable_events() {
        let projection = project(show_data("done", Lifecycle::Final, vec![], vec![])).unwrap();
        assert!(projection.requestable_events.is_empty());

        let terminated =
            project(show_data("start", Lifecycle::Terminated, vec![], vec![])).unwrap();
        assert!(terminated.requestable_events.is_empty());
    }

    #[test]
    fn projection_includes_run_identity_state_text_and_initial_input() {
        let projection = project(show_data("start", Lifecycle::Active, vec![], vec![])).unwrap();

        assert_eq!(projection.run_id, RunId::from("run-1"));
        assert_eq!(projection.label.as_deref(), Some("show me"));
        assert_eq!(projection.workflow_id, WorkflowId::from("workflow"));
        assert_eq!(projection.lifecycle, Lifecycle::Active);
        assert_eq!(projection.current_state, StateId::from("start"));
        assert_eq!(projection.current_state_title, "Start");
        assert_eq!(projection.current_state_instructions, "Do the work");
        assert_eq!(
            projection.initial_input,
            json!({"objective": "ship safely"})
        );
    }
}
