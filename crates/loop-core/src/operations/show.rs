//! The provider-free `show` projection.
//!
//! Persistence supplies authoritative run state, ordered context, and all
//! durable checked evaluations.  Core derives requestable events and the
//! latest evaluation for each exact checked transition without invoking a
//! provider or loading raw history.

use super::persistence_error;
use crate::{
    project_invocation_status, DurableEvaluation, EventId, InnerWorker, InvocationId, Lifecycle,
    OperationOutcome, Persistence, ProjectedInvocationStatus, Run, RunId, ShowData, StateId,
    Timestamp, Transition, TransitionKind, WorkSlot, WorkSlotBinding, WorkSlotId,
    WorkSlotInvocation, WorkSlotProcess, WorkflowId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

const WORK_SLOT_BINDINGS_KEY: &str = "work_slot_bindings";

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

/// Overlay view of one work-slot invocation.  `waiter_pid` is never included.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkSlotInvocationView {
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub binding: WorkSlotBinding,
    pub instruction_digest: String,
    pub subject: String,
    pub status: ProjectedInvocationStatus,
    pub started_at: Timestamp,
    pub allowed_time_ms: u64,
    pub exit_code: Option<i32>,
    pub completed_at: Option<Timestamp>,
    pub overlay_meaning: String,
    pub elapsed_ms: u64,
    pub remaining_allowed_ms: u64,
    pub capture_dir: String,
    pub inner_workers: Vec<InnerWorker>,
}

const OVERLAY_MEANING_SUCCEEDED: &str =
    "Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work.";
const OVERLAY_MEANING_FAILED: &str =
    "Overlay failed means the bound CLI exited nonzero or the waiter vanished.";
const OVERLAY_MEANING_RUNNING: &str =
    "Overlay running means the waiter is alive and allowed time has not elapsed.";
const OVERLAY_MEANING_OVERRUN: &str =
    "Overlay overrun means allowed time elapsed while the waiter is alive; invoke the same slot again.";

fn overlay_meaning(status: ProjectedInvocationStatus) -> &'static str {
    match status {
        ProjectedInvocationStatus::Succeeded => OVERLAY_MEANING_SUCCEEDED,
        ProjectedInvocationStatus::Failed => OVERLAY_MEANING_FAILED,
        ProjectedInvocationStatus::Running => OVERLAY_MEANING_RUNNING,
        ProjectedInvocationStatus::Overrun => OVERLAY_MEANING_OVERRUN,
    }
}

fn invocation_elapsed_ms(record: &WorkSlotInvocation, now: Timestamp) -> u64 {
    let end = record.completed_at.unwrap_or(now).as_unix_millis();
    end.saturating_sub(record.started_at.as_unix_millis())
        .max(0) as u64
}

fn remaining_allowed_ms(
    status: ProjectedInvocationStatus,
    allowed_time_ms: u64,
    elapsed_ms: u64,
) -> u64 {
    if status == ProjectedInvocationStatus::Running {
        allowed_time_ms.saturating_sub(elapsed_ms)
    } else {
        0
    }
}

impl WorkSlotInvocationView {
    fn from_record(record: &WorkSlotInvocation, now: Timestamp, waiter_alive: bool) -> Self {
        let status = project_invocation_status(record, now, waiter_alive);
        let elapsed_ms = invocation_elapsed_ms(record, now);
        let remaining_allowed_ms = remaining_allowed_ms(status, record.allowed_time_ms, elapsed_ms);
        let inner_workers = if status == ProjectedInvocationStatus::Running {
            Vec::new()
        } else {
            record.inner_workers.clone()
        };
        Self {
            invocation_id: record.invocation_id.clone(),
            slot_id: record.slot_id.clone(),
            binding: record.binding.clone(),
            instruction_digest: record.instruction_digest.clone(),
            subject: record.subject.clone(),
            status,
            started_at: record.started_at,
            allowed_time_ms: record.allowed_time_ms,
            exit_code: record.exit_code,
            completed_at: record.completed_at,
            overlay_meaning: overlay_meaning(status).to_owned(),
            elapsed_ms,
            remaining_allowed_ms,
            capture_dir: record.capture_dir.clone(),
            inner_workers,
        }
    }
}

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
    /// Catalog snapshot from `run.workflow.work_slots` (`id`, `state`, `event`).
    pub work_slots: Vec<WorkSlot>,
    /// Overlay-projected invocations.  Never includes `waiter_pid`.
    pub work_slot_invocations: Vec<WorkSlotInvocationView>,
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

fn bound_slot_for_current_state(run: &Run) -> Option<(&WorkSlot, WorkSlotBinding)> {
    let Value::Object(map) = &run.initial_input else {
        return None;
    };
    let Some(Value::Object(bindings)) = map.get(WORK_SLOT_BINDINGS_KEY) else {
        return None;
    };
    for slot in &run.workflow.work_slots {
        if slot.state != run.current_state {
            continue;
        }
        let Some(value) = bindings.get(slot.id.as_str()) else {
            continue;
        };
        let binding = serde_json::from_value::<WorkSlotBinding>(value.clone())
            .unwrap_or_else(|_| WorkSlotBinding::new(String::new(), Vec::new()));
        return Some((slot, binding));
    }
    None
}

fn current_state_instructions_for(run: &Run, stored: &str) -> String {
    match bound_slot_for_current_state(run) {
        Some((slot, binding)) => {
            let args = serde_json::to_string(&binding.args).unwrap_or_else(|_| "[]".to_owned());
            format!(
                "Bound work slot `{slot_id}` is configured. Frozen worker CLI: command={command} args={args}. Legal start: loop-engine invoke {run_id} {slot_id}. Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. Captures are at the named capture directory on the invocation view and invoke result. The driver triages worker output, appends provider-shaped records, then requests the shown event. On overrun invoke the same slot again. On failed inspect stderr.",
                slot_id = slot.id,
                command = binding.command,
                run_id = run.id,
            )
        }
        None => stored.to_owned(),
    }
}

/// Project durable show data into the complete continuation view.
pub fn project(data: ShowData) -> std::result::Result<ShowProjection, ProjectionError> {
    project_with_invocations(data, &[], Timestamp::from_unix_millis(0), |_| false)
}

/// Project show data with work-slot invocation overlay.
pub fn project_with_invocations(
    data: ShowData,
    invocations: &[WorkSlotInvocation],
    now: Timestamp,
    waiter_alive: impl Fn(u32) -> bool,
) -> std::result::Result<ShowProjection, ProjectionError> {
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

    let current_state_instructions =
        current_state_instructions_for(&data.run, &current_state.instructions);
    let work_slots = data.run.workflow.work_slots.clone();
    let work_slot_invocations = invocations
        .iter()
        .map(|record| {
            WorkSlotInvocationView::from_record(record, now, waiter_alive(record.waiter_pid))
        })
        .collect();

    Ok(ShowProjection {
        run_id: data.run.id,
        label: data.run.label,
        workflow_id: data.run.workflow.id,
        lifecycle: data.run.lifecycle,
        current_state: data.run.current_state,
        current_state_title: current_state.title.clone(),
        current_state_instructions,
        initial_input: data.run.initial_input,
        context,
        requestable_events,
        latest_evaluations: latest_evaluations(&data.checked_evaluations),
        work_slots,
        work_slot_invocations,
    })
}

/// Execute a provider-free `show` read, overlaying work-slot invocations.
pub fn execute<P, Proc>(
    request: Request,
    persistence: &P,
    process: &Proc,
    now: Timestamp,
) -> OperationOutcome<ShowProjection>
where
    P: Persistence + ?Sized,
    Proc: WorkSlotProcess + ?Sized,
{
    let data = match persistence.load_show_data(&request.run_id) {
        Ok(data) => data,
        Err(error) => return persistence_error(error),
    };
    let invocations = match persistence.load_work_slot_invocations(&request.run_id) {
        Ok(invocations) => invocations,
        Err(error) => return persistence_error(error),
    };
    match project_with_invocations(data, &invocations, now, |pid| process.waiter_alive(pid)) {
        Ok(projection) => OperationOutcome::completed(projection),
        Err(error) => OperationOutcome::error(error.code(), error.to_string()),
    }
}

/// Execute `show` with persistence first.
pub fn execute_with_persistence<P, Proc>(
    persistence: &P,
    request: Request,
    process: &Proc,
    now: Timestamp,
) -> OperationOutcome<ShowProjection>
where
    P: Persistence + ?Sized,
    Proc: WorkSlotProcess + ?Sized,
{
    execute(request, persistence, process, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextRecord, ControlRevision, EvaluationFeedback, InnerWorker, ProviderAssociation,
        SemanticSequence, State, Timestamp, TransitionKind, WaiterWrittenStatus, Workflow,
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
        run_custom(
            current_state,
            lifecycle,
            workflow(),
            json!({"objective": "ship safely"}),
        )
    }

    fn run_custom(
        current_state: &str,
        lifecycle: Lifecycle,
        workflow: Workflow,
        initial_input: Value,
    ) -> Run {
        Run::new(
            "run-1",
            Some("show me".to_owned()),
            workflow,
            ProviderAssociation::new(json!({"provider": "fake"})),
            initial_input,
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

    fn slot_workflow(instructions: &str) -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", instructions, false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::checked("start", "submit", "done")],
        )
        .with_work_slots(vec![WorkSlot::new("slot-1", "start", "submit")])
    }

    fn bound_input() -> Value {
        json!({
            "objective": "ship safely",
            "work_slot_bindings": {
                "slot-1": {"command": "worker", "args": ["--flag", "value"]}
            }
        })
    }

    fn sample_invocation(
        invocation_id: &str,
        waiter_pid: u32,
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        exit_code: Option<i32>,
        completed_at: Option<i64>,
    ) -> WorkSlotInvocation {
        sample_invocation_with_capture(
            invocation_id,
            waiter_pid,
            started_at,
            allowed_time_ms,
            status,
            exit_code,
            completed_at,
            String::new(),
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn sample_invocation_with_capture(
        invocation_id: &str,
        waiter_pid: u32,
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        exit_code: Option<i32>,
        completed_at: Option<i64>,
        capture_dir: impl Into<String>,
        inner_workers: Vec<InnerWorker>,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            invocation_id,
            "slot-1",
            WorkSlotBinding::new("worker", vec!["--flag".to_owned(), "value".to_owned()]),
            "abc123digest",
            "subject-1",
            waiter_pid,
            Timestamp::from_unix_millis(started_at),
            allowed_time_ms,
            status,
            exit_code,
            completed_at.map(Timestamp::from_unix_millis),
            capture_dir,
            inner_workers,
        )
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
        assert!(projection.work_slots.is_empty());
        assert!(projection.work_slot_invocations.is_empty());
    }

    #[test]
    fn work_slots_catalog_snapshot_has_id_state_event_only() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("Do the work"),
                json!({"objective": "ship safely"}),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let projection = project(data).unwrap();

        assert_eq!(
            projection.work_slots,
            vec![WorkSlot::new("slot-1", "start", "submit")]
        );
        let json = serde_json::to_value(&projection.work_slots[0]).unwrap();
        let object = json.as_object().expect("work slot object");
        assert_eq!(object.len(), 3);
        assert_eq!(object.get("id"), Some(&json!("slot-1")));
        assert_eq!(object.get("state"), Some(&json!("start")));
        assert_eq!(object.get("event"), Some(&json!("submit")));
        assert!(!object.contains_key("instructions"));
        assert!(!object.contains_key("instruction_body"));
        assert!(!object.contains_key("body"));
        assert!(!object.contains_key("instruction"));
    }

    #[test]
    fn work_slot_invocations_field_set_omits_waiter_pid() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("Do the work"),
                bound_input(),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let record = sample_invocation(
            "inv-1",
            4242,
            1_000,
            5_000,
            Some(WaiterWrittenStatus::Succeeded),
            Some(0),
            Some(2_000),
        );
        let projection =
            project_with_invocations(data, &[record], Timestamp::from_unix_millis(3_000), |_| {
                true
            })
            .unwrap();

        assert_eq!(projection.work_slot_invocations.len(), 1);
        let view = &projection.work_slot_invocations[0];
        assert_eq!(view.invocation_id.as_str(), "inv-1");
        assert_eq!(view.slot_id.as_str(), "slot-1");
        assert_eq!(
            view.binding,
            WorkSlotBinding::new("worker", vec!["--flag".to_owned(), "value".to_owned()])
        );
        assert_eq!(view.instruction_digest, "abc123digest");
        assert_eq!(view.subject, "subject-1");
        assert_eq!(view.status, ProjectedInvocationStatus::Succeeded);
        assert_eq!(view.started_at, Timestamp::from_unix_millis(1_000));
        assert_eq!(view.allowed_time_ms, 5_000);
        assert_eq!(view.exit_code, Some(0));
        assert_eq!(view.completed_at, Some(Timestamp::from_unix_millis(2_000)));
        assert_eq!(view.overlay_meaning, OVERLAY_MEANING_SUCCEEDED);
        assert_eq!(view.elapsed_ms, 1_000);
        assert_eq!(view.remaining_allowed_ms, 0);
        assert_eq!(view.capture_dir, "");
        assert!(view.inner_workers.is_empty());

        let json = serde_json::to_value(&projection).unwrap();
        let serialized = json.to_string();
        assert!(
            !serialized.contains("waiter_pid"),
            "show projection JSON must not contain waiter_pid: {serialized}"
        );
        let invocation = &json["work_slot_invocations"][0];
        let object = invocation.as_object().expect("invocation object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = vec![
            "allowed_time_ms",
            "binding",
            "capture_dir",
            "completed_at",
            "elapsed_ms",
            "exit_code",
            "inner_workers",
            "instruction_digest",
            "invocation_id",
            "overlay_meaning",
            "remaining_allowed_ms",
            "slot_id",
            "started_at",
            "status",
            "subject",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert!(!object.contains_key("waiter_pid"));
        assert_eq!(object.get("invocation_id"), Some(&json!("inv-1")));
        assert_eq!(object.get("slot_id"), Some(&json!("slot-1")));
        assert_eq!(object.get("status"), Some(&json!("succeeded")));
    }

    #[test]
    fn work_slot_invocations_status_is_overlay() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("Do the work"),
                bound_input(),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let invocations = vec![
            sample_invocation(
                "inv-succeeded",
                1,
                0,
                1_000,
                Some(WaiterWrittenStatus::Succeeded),
                Some(0),
                Some(10),
            ),
            sample_invocation("inv-vanished", 2, 0, 10_000, None, None, None),
            sample_invocation("inv-overrun", 3, 0, 1_000, None, None, None),
            sample_invocation("inv-running", 4, 0, 10_000, None, None, None),
        ];
        let now = Timestamp::from_unix_millis(5_000);
        let projection = project_with_invocations(data, &invocations, now, |pid| pid != 2).unwrap();

        assert_eq!(
            projection.work_slot_invocations[0].status,
            ProjectedInvocationStatus::Succeeded
        );
        assert_eq!(
            projection.work_slot_invocations[1].status,
            ProjectedInvocationStatus::Failed
        );
        assert_eq!(
            projection.work_slot_invocations[2].status,
            ProjectedInvocationStatus::Overrun
        );
        assert_eq!(
            projection.work_slot_invocations[3].status,
            ProjectedInvocationStatus::Running
        );
        assert_eq!(
            projection.work_slot_invocations[3].overlay_meaning,
            OVERLAY_MEANING_RUNNING
        );
        assert_eq!(projection.work_slot_invocations[3].elapsed_ms, 5_000);
        assert_eq!(
            projection.work_slot_invocations[3].remaining_allowed_ms,
            5_000
        );
        assert!(projection.work_slot_invocations[3].inner_workers.is_empty());
        assert_eq!(projection.work_slot_invocations[0].remaining_allowed_ms, 0);
        assert_eq!(projection.work_slot_invocations[1].remaining_allowed_ms, 0);
        assert_eq!(projection.work_slot_invocations[2].remaining_allowed_ms, 0);
    }

    #[test]
    fn bound_work_slot_current_state_redacts_body_and_names_invoke_cli() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("SECRET BODY"),
                bound_input(),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let projection = project(data).unwrap();
        let instructions = &projection.current_state_instructions;

        assert!(
            !instructions.contains("SECRET BODY"),
            "bound slot must omit stored work body, got: {instructions}"
        );
        assert_eq!(
            instructions,
            "Bound work slot `slot-1` is configured. Frozen worker CLI: command=worker args=[\"--flag\",\"value\"]. Legal start: loop-engine invoke run-1 slot-1. Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work. Captures are at the named capture directory on the invocation view and invoke result. The driver triages worker output, appends provider-shaped records, then requests the shown event. On overrun invoke the same slot again. On failed inspect stderr."
        );
        let triage = [
            "Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work.",
            "Captures are at the named capture directory on the invocation view and invoke result.",
            "The driver triages worker output, appends provider-shaped records, then requests the shown event.",
            "On overrun invoke the same slot again.",
            "On failed inspect stderr.",
        ];
        let mut cursor = 0;
        for sentence in triage {
            let found = instructions[cursor..].find(sentence).unwrap_or_else(|| {
                panic!("missing triage sentence `{sentence}` in `{instructions}`")
            });
            cursor += found + sentence.len();
        }
        assert!(instructions.contains("slot-1"));
        assert!(instructions.contains("worker"));
        assert!(instructions.contains("[\"--flag\",\"value\"]"));
        assert!(instructions.contains("loop-engine invoke run-1 slot-1"));
    }

    #[test]
    fn work_slot_unbound_current_state_keeps_stored_instructions_without_invoke_cli() {
        let projection = project(show_data("start", Lifecycle::Active, vec![], vec![])).unwrap();

        assert_eq!(projection.current_state_instructions, "Do the work");
        assert!(!projection
            .current_state_instructions
            .contains("loop-engine invoke"));
        assert!(!projection
            .current_state_instructions
            .contains("Bound work slot"));
    }

    #[test]
    fn work_slot_current_state_without_binding_is_unbound_and_not_redacted() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("SECRET BODY"),
                json!({"objective": "ship safely"}),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let projection = project(data).unwrap();

        assert_eq!(projection.current_state_instructions, "SECRET BODY");
        assert!(!projection
            .current_state_instructions
            .contains("loop-engine invoke"));
        assert!(!projection
            .current_state_instructions
            .contains("Bound work slot"));

        let empty_bindings = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("SECRET BODY"),
                json!({"objective": "ship safely", "work_slot_bindings": {}}),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let empty_projection = project(empty_bindings).unwrap();
        assert_eq!(empty_projection.current_state_instructions, "SECRET BODY");
        assert!(!empty_projection
            .current_state_instructions
            .contains("loop-engine invoke"));
    }

    #[test]
    fn succeeded_invocation_reports_heartbeat_and_inner_nonzero() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("Do the work"),
                bound_input(),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let record = sample_invocation_with_capture(
            "inv-1",
            4242,
            1_000,
            5_000,
            Some(WaiterWrittenStatus::Succeeded),
            Some(0),
            Some(2_500),
            "/tmp/artifacts/work-slot-captures/slot-1/inv-1",
            vec![InnerWorker::new("python3", vec!["worker.py".to_owned()], 7)],
        );
        let projection =
            project_with_invocations(data, &[record], Timestamp::from_unix_millis(9_000), |_| {
                false
            })
            .unwrap();
        let view = &projection.work_slot_invocations[0];
        assert_eq!(view.status, ProjectedInvocationStatus::Succeeded);
        assert_eq!(view.overlay_meaning, OVERLAY_MEANING_SUCCEEDED);
        assert_eq!(view.elapsed_ms, 1_500);
        assert_eq!(view.remaining_allowed_ms, 0);
        assert_eq!(
            view.capture_dir,
            "/tmp/artifacts/work-slot-captures/slot-1/inv-1"
        );
        assert_eq!(view.inner_workers.len(), 1);
        assert_eq!(view.inner_workers[0].exit_code, 7);
        assert_eq!(view.inner_workers[0].command, "python3");
    }

    #[test]
    fn running_invocation_hides_stored_inner_workers_and_counts_remaining_time() {
        let data = ShowData {
            run: run_custom(
                "start",
                Lifecycle::Active,
                slot_workflow("Do the work"),
                bound_input(),
            ),
            context: vec![],
            checked_evaluations: vec![],
        };
        let record = sample_invocation_with_capture(
            "inv-running",
            9,
            1_000,
            4_000,
            None,
            None,
            None,
            "/tmp/captures/inv-running",
            vec![InnerWorker::new("python3", vec!["stale.py".to_owned()], 7)],
        );
        let projection =
            project_with_invocations(data, &[record], Timestamp::from_unix_millis(3_500), |_| {
                true
            })
            .unwrap();
        let view = &projection.work_slot_invocations[0];
        assert_eq!(view.status, ProjectedInvocationStatus::Running);
        assert_eq!(view.overlay_meaning, OVERLAY_MEANING_RUNNING);
        assert_eq!(view.elapsed_ms, 2_500);
        assert_eq!(view.remaining_allowed_ms, 1_500);
        assert_eq!(view.capture_dir, "/tmp/captures/inv-running");
        assert!(
            view.inner_workers.is_empty(),
            "running overlay must not project stored inner_workers"
        );
    }
}
