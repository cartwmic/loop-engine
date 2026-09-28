//! `event` workflow-control operation.
//!
//! Event processing is the one core use case that combines authoritative
//! workflow resolution, conditional persistence, and (for checked edges)
//! provider evaluation.  Core chooses the edge and computes the resulting
//! lifecycle; persistence owns the atomic conditional mutation boundary.

use super::{persistence_error, provider_error, require_current_observation};
use crate::{
    instruction_digest, project_invocation_status, request_from_snapshot, resolve_transition,
    CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
    EvaluationResult, EventId, Lifecycle, OperationOutcome, OutcomeIssue, Persistence,
    ProjectedInvocationStatus, ProviderGateway, RecordDenialRequest, RunId, Timestamp, Transition,
    TransitionResolutionError, WorkSlotId,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

const BOUND_SLOT_INVOCATION_REQUIRED: &str = "bound-slot-invocation-required";
static VISIT_SUBJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Caller-supplied values for one event request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
    pub event: EventId,
    /// Clock used to project invocation overlay status. Defaults to now in
    /// [`Request::new`]; tests that need overrun control it with [`Request::with_now`].
    #[serde(default = "current_timestamp")]
    pub now: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_attestation: Option<crate::StateVisitAttestation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice_exception: Option<crate::AdviceExceptionAttestation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_act: Option<crate::DriverActRequest>,
}

impl Request {
    pub fn new(run_id: impl Into<RunId>, event: impl Into<EventId>) -> Self {
        Self {
            run_id: run_id.into(),
            event: event.into(),
            now: current_timestamp(),
            override_attestation: None,
            advice_exception: None,
            driver_act: None,
        }
    }

    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = now;
        self
    }

    pub fn with_override(mut self, attestation: crate::StateVisitAttestation) -> Self {
        self.override_attestation = Some(attestation);
        self
    }

    pub fn with_advice_exception(mut self, attestation: crate::AdviceExceptionAttestation) -> Self {
        self.advice_exception = Some(attestation);
        self
    }

    pub fn with_driver_act(mut self, act: crate::DriverActRequest) -> Self {
        self.driver_act = Some(act);
        self
    }

    pub fn event_id(&self) -> &EventId {
        &self.event
    }
}

/// Successful event data.  A checked denial is represented by the enclosing
/// [`OperationOutcome::Rejected`] value and therefore has no successful value.
pub type Result = CommitTransitionResult;

/// Execute one event request.
///
/// The first read is authoritative: it verifies existence/activity and
/// resolves the requested event from the run's stored workflow and current
/// state. Enabled advice closure is checked on both checked and check-free
/// normal departures. Its scoped exception is retained in transition history
/// but does not skip bound completion or provider evaluation. The broader
/// pre-existing event override remains separate. Ordinary check-free edges
/// commit atomically; a checked edge captures a durable snapshot, invokes the
/// provider outside persistence, and conditionally commits or records the
/// result against the snapshot's original control point.
///
/// No branch retries or re-resolves an event after a conflict. Persistence
/// conflicts are classified as operation errors by the core persistence-error
/// mapping.
pub fn execute<G, P>(request: Request, gateway: &G, persistence: &P) -> OperationOutcome<Result>
where
    G: ProviderGateway + ?Sized,
    P: Persistence + ?Sized,
{
    let run = match persistence.load_authoritative_run(&request.run_id) {
        Ok(run) => run,
        Err(error) => return persistence_error(error),
    };

    if !run.lifecycle.is_active() {
        return OperationOutcome::rejected(
            "run-not-active",
            format!("run `{}` is not active ({:?})", run.id, run.lifecycle),
        );
    }

    let transition = match resolve_requested_transition(&run.current_state, &run.workflow, &request)
    {
        Ok(Some(transition)) => transition.clone(),
        Ok(None) => {
            return OperationOutcome::rejected(
                "event-unavailable",
                format!(
                    "event `{}` is not available from state `{}`",
                    request.event, run.current_state
                ),
            )
        }
        Err(error) => return transition_resolution_error(error),
    };

    if (request.override_attestation.is_some() && request.advice_exception.is_some())
        || (request.driver_act.is_some()
            && (request.override_attestation.is_some() || request.advice_exception.is_some()))
    {
        return OperationOutcome::rejected(
            "incompatible-event-exceptions",
            "driver acts, advice exceptions, and general event overrides are separate and cannot be combined",
        );
    }

    if let Some(attestation) = &request.override_attestation {
        if attestation.owner.trim().is_empty() || attestation.reason.trim().is_empty() {
            return OperationOutcome::rejected(
                "invalid-override",
                "owner and reason must be nonempty",
            );
        }
        if attestation.state_visit != run.control_revision.as_u64() {
            return OperationOutcome::rejected(
                "stale-state-visit",
                "override must name the current observed state visit",
            );
        }
    }

    // Preserve the pre-existing bound-slot refusal before adding the
    // observation guard. A caller missing both prerequisites must still see
    // the actionable bound-slot reason that made the event invalid already.
    let driver_act = match request.driver_act.as_ref() {
        Some(act) => match prepare_driver_act(&run, &transition, act, persistence) {
            Ok(evidence) => Some(evidence),
            Err(outcome) => return outcome,
        },
        None => None,
    };

    if request.override_attestation.is_none()
        && driver_act.is_none()
        && transition.kind.is_checked()
    {
        if let Some(rejected) = enforce_bound_slot_gate(&run, &transition, persistence, request.now)
        {
            return rejected;
        }
    }

    if let Err(outcome) = super::require_quiescent_work(&run, persistence) {
        return outcome;
    }

    if let Err(outcome) =
        require_current_observation::<P, Result>(persistence, &run.id, run.control_revision)
    {
        return outcome;
    }

    let advice_exception = match enforce_advice_closure(&run, &transition, &request, persistence) {
        Ok(exception) => exception,
        Err(outcome) => return outcome,
    };

    if let Some(attestation) = request.override_attestation {
        return commit_override(&run, &transition, attestation, persistence);
    }

    if transition.kind.is_check_free() {
        return commit_check_free(&run, &transition, advice_exception, persistence);
    }

    evaluate_checked(
        run,
        &transition,
        advice_exception,
        driver_act,
        gateway,
        persistence,
    )
}

/// Execute `event` with ports first, which is convenient for composition
/// roots that keep their adapters together.
pub fn execute_with_ports<G, P>(
    gateway: &G,
    persistence: &P,
    request: Request,
) -> OperationOutcome<Result>
where
    G: ProviderGateway + ?Sized,
    P: Persistence + ?Sized,
{
    execute(request, gateway, persistence)
}

/// Execute `event` with persistence first.
pub fn execute_with_persistence<P, G>(
    persistence: &P,
    gateway: &G,
    request: Request,
) -> OperationOutcome<Result>
where
    G: ProviderGateway + ?Sized,
    P: Persistence + ?Sized,
{
    execute(request, gateway, persistence)
}

fn resolve_requested_transition<'a>(
    current_state: &'a crate::StateId,
    workflow: &'a crate::Workflow,
    request: &Request,
) -> std::result::Result<Option<&'a Transition>, TransitionResolutionError> {
    resolve_transition(workflow, current_state, &request.event)
}

fn transition_resolution_error<T>(error: TransitionResolutionError) -> OperationOutcome<T> {
    match error {
        TransitionResolutionError::MalformedWorkflow { error } => {
            OperationOutcome::error(error.code(), error.to_string())
        }
        TransitionResolutionError::UndefinedCurrentState { state } => OperationOutcome::error(
            "invalid-run",
            format!("authoritative current state `{state}` is undefined"),
        ),
    }
}

fn target_lifecycle(
    workflow: &crate::Workflow,
    transition: &Transition,
) -> std::result::Result<Lifecycle, OutcomeIssue> {
    let Some(target) = workflow
        .states
        .iter()
        .find(|state| state.id == transition.target)
    else {
        return Err(OutcomeIssue::new(
            "invalid-workflow",
            format!(
                "transition target state `{}` is absent from the stored workflow",
                transition.target
            ),
        ));
    };

    Ok(if target.is_final {
        Lifecycle::Final
    } else {
        Lifecycle::Active
    })
}

fn enforce_advice_closure<P>(
    run: &crate::Run,
    transition: &Transition,
    request: &Request,
    persistence: &P,
) -> std::result::Result<Option<crate::AdviceExceptionAttestation>, OperationOutcome<Result>>
where
    P: Persistence + ?Sized,
{
    let enabled = run
        .initial_input
        .as_object()
        .is_some_and(|input| input.contains_key(crate::ADVICE_DEPARTURES_INPUT_KEY));
    if !enabled {
        return if request.advice_exception.is_some() {
            Err(OperationOutcome::rejected(
                "advice-exception-disabled",
                "this run has no frozen advice departure map",
            ))
        } else {
            Ok(None)
        };
    }

    let context = persistence
        .load_context_records(&run.id)
        .map_err(persistence_error)?;
    let due = crate::unanswered_due_occasions(
        &run.initial_input,
        &run.id,
        &run.workflow,
        &run.current_state,
        transition,
        run.control_revision.as_u64(),
        &context,
    )
    .map_err(|message| OperationOutcome::rejected("advice-transition-incomplete", message))?
    .expect("enabled map returns advice closure status");

    match request.advice_exception.as_ref() {
        Some(exception) => {
            crate::validate_advice_exception(exception, run.control_revision.as_u64(), &due)
                .map_err(|message| {
                    OperationOutcome::rejected("invalid-advice-exception", message)
                })?;
            Ok(Some(exception.clone()))
        }
        None if due.is_empty() => Ok(None),
        None => Err(OperationOutcome::rejected(
            "advice-transition-incomplete",
            format!(
                "unanswered due advice occasions: {}; record valid advice and per-answer dispositions, or use an owner-attested --advice-exception naming exactly these occasions",
                due.join(", ")
            ),
        )),
    }
}

fn commit_check_free<P>(
    run: &crate::Run,
    transition: &Transition,
    advice_exception: Option<crate::AdviceExceptionAttestation>,
    persistence: &P,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    let resulting_lifecycle = match target_lifecycle(&run.workflow, transition) {
        Ok(lifecycle) => lifecycle,
        Err(issue) => return OperationOutcome::error_with_issue(issue),
    };

    let slot_subjects = slot_subjects_for_state(&run.workflow, &transition.target);
    let mut request = CommitTransitionRequest::new(
        run.id.clone(),
        run.control_revision,
        run.current_state.clone(),
        transition.clone(),
        resulting_lifecycle,
    )
    .with_slot_subjects(slot_subjects);
    if let Some(exception) = advice_exception {
        request = request.with_advice_exception(exception);
    }

    match persistence.commit_transition(request) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

fn commit_override<P: Persistence + ?Sized>(
    run: &crate::Run,
    transition: &Transition,
    attestation: crate::StateVisitAttestation,
    persistence: &P,
) -> OperationOutcome<Result> {
    let lifecycle = match target_lifecycle(&run.workflow, transition) {
        Ok(value) => value,
        Err(issue) => return OperationOutcome::error_with_issue(issue),
    };
    let mut skipped_bound_checks = Vec::new();
    if transition.kind.is_checked() {
        for slot in run.workflow.work_slots.iter().filter(|slot| {
            slot.state == run.current_state
                && slot.event == transition.event
                && crate::effective_binding(run, &slot.id).is_some()
        }) {
            let subject = match persistence.get_current_slot_subject(&run.id, &slot.id) {
                Ok(subject) => subject,
                Err(error) => return persistence_error(error),
            };
            let state = run
                .workflow
                .states
                .iter()
                .find(|state| state.id == slot.state)
                .expect("resolved source state");
            skipped_bound_checks.push(crate::SkippedBoundCheck {
                slot_id: slot.id.clone(),
                check: "succeeded-invocation-matching-slot-instruction-digest-and-current-visit-subject".into(),
                instruction_digest: instruction_digest(&state.instructions),
                subject,
            });
        }
    }
    let mut request = CommitTransitionRequest::new(
        run.id.clone(),
        run.control_revision,
        run.current_state.clone(),
        transition.clone(),
        lifecycle,
    )
    .with_slot_subjects(slot_subjects_for_state(&run.workflow, &transition.target));
    request.exception = Some(crate::TransitionOverride {
        attestation,
        skipped_bound_checks,
        provider_evaluation: if transition.kind.is_checked() {
            crate::SkippedProviderEvaluation::NotPerformed
        } else {
            crate::SkippedProviderEvaluation::NotApplicable
        },
    });
    match persistence.commit_transition(request) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

fn evaluate_checked<G, P>(
    run: crate::Run,
    transition: &Transition,
    advice_exception: Option<crate::AdviceExceptionAttestation>,
    driver_act: Option<crate::DriverActEvidence>,
    gateway: &G,
    persistence: &P,
) -> OperationOutcome<Result>
where
    G: ProviderGateway + ?Sized,
    P: Persistence + ?Sized,
{
    let snapshot_request =
        CheckedEvaluationSnapshotRequest::new(run.id.clone(), transition.clone());
    let snapshot = match persistence.load_checked_evaluation_snapshot(snapshot_request) {
        Ok(snapshot) => snapshot,
        Err(error) => return persistence_error(error),
    };

    // The persistence contract returns the exact edge requested at the
    // snapshot boundary.  Keep the provider-facing request and subsequent
    // mutation tied to the originally engine-selected edge rather than any
    // provider-supplied or independently selected target.
    let mut snapshot_for_request = snapshot.clone();
    snapshot_for_request.transition = transition.clone();
    let mut evaluation_request = request_from_snapshot(&snapshot_for_request);
    evaluation_request.driver_act = driver_act.clone();

    // `load_checked_evaluation_snapshot` is the complete persistence activity
    // for this phase.  The gateway call is deliberately made after it
    // returns, so provider execution is never inside a persistence boundary.
    let evaluation = match gateway.evaluate(&snapshot.run.provider_association, evaluation_request)
    {
        Ok(result) => result,
        Err(error) => return provider_error(error),
    };
    match evaluation {
        EvaluationResult::Allow { context_append } => commit_checked_allow(
            snapshot,
            transition,
            context_append,
            advice_exception,
            driver_act,
            persistence,
        ),
        EvaluationResult::Deny { feedback } => {
            record_checked_denial(snapshot, transition, feedback, persistence)
        }
        EvaluationResult::Unsupported => OperationOutcome::error(
            "provider-unsupported",
            format!(
                "provider does not support checked event `{}` from state `{}`",
                transition.event, transition.source
            ),
        ),
    }
}

fn commit_checked_allow<P>(
    snapshot: crate::CheckedEvaluationSnapshot,
    transition: &Transition,
    context_append: Option<crate::ContextAppendEffect>,
    advice_exception: Option<crate::AdviceExceptionAttestation>,
    driver_act: Option<crate::DriverActEvidence>,
    persistence: &P,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    let resulting_lifecycle = match target_lifecycle(&snapshot.run.workflow, transition) {
        Ok(lifecycle) => lifecycle,
        Err(issue) => return OperationOutcome::error_with_issue(issue),
    };

    let slot_subjects = slot_subjects_for_state(&snapshot.run.workflow, &transition.target);
    let mut request = CommitTransitionRequest::new(
        snapshot.run.id,
        snapshot.observed_control_revision,
        snapshot.run.current_state,
        transition.clone(),
        resulting_lifecycle,
    )
    .with_context_append(context_append)
    .with_slot_subjects(slot_subjects);
    if let Some(exception) = advice_exception {
        request = request.with_advice_exception(exception);
    }
    if let Some(act) = driver_act {
        request = request.with_driver_act(act);
    }

    match persistence.commit_transition(request) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

fn record_checked_denial<P>(
    snapshot: crate::CheckedEvaluationSnapshot,
    transition: &Transition,
    feedback: crate::EvaluationFeedback,
    persistence: &P,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    let request = RecordDenialRequest::new(
        snapshot.run.id,
        snapshot.observed_control_revision,
        snapshot.run.current_state,
        transition.clone(),
        feedback.clone(),
    );

    match persistence.record_denial(request) {
        Ok(_) => OperationOutcome::rejected_feedback(feedback),
        Err(error) => persistence_error(error),
    }
}

fn current_timestamp() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    Timestamp::from_unix_millis(millis)
}

fn overlay_status_label(status: ProjectedInvocationStatus) -> &'static str {
    match status {
        ProjectedInvocationStatus::Running => "running",
        ProjectedInvocationStatus::Succeeded => "succeeded",
        ProjectedInvocationStatus::Failed => "failed",
        ProjectedInvocationStatus::Overrun => "overrun",
    }
}

fn mint_visit_subject(slot_id: &WorkSlotId) -> String {
    let millis = current_timestamp().as_unix_millis();
    let suffix = VISIT_SUBJECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("visit-{slot_id}-{millis}-{suffix}")
}

fn slot_subjects_for_state(
    workflow: &crate::Workflow,
    state_id: &crate::StateId,
) -> Vec<(WorkSlotId, String)> {
    workflow
        .work_slots
        .iter()
        .filter(|slot| slot.state == *state_id)
        .map(|slot| (slot.id.clone(), mint_visit_subject(&slot.id)))
        .collect()
}

fn prepare_driver_act<P>(
    run: &crate::Run,
    transition: &Transition,
    request: &crate::DriverActRequest,
    persistence: &P,
) -> std::result::Result<crate::DriverActEvidence, OperationOutcome<Result>>
where
    P: Persistence + ?Sized,
{
    if !transition.kind.is_checked() {
        return Err(OperationOutcome::rejected(
            "driver-act-requires-checked-edge",
            "a driver act is only an alternative bound-completion input on a checked edge",
        ));
    }
    if request.author.name.trim().is_empty()
        || !matches!(request.author.kind.as_str(), "human" | "agent" | "script")
        || request.reason.trim().is_empty()
        || request.reason.len() > 4096
        || request.changed_artifacts.is_empty()
        || request.changed_artifacts.len() > 128
    {
        return Err(OperationOutcome::rejected(
            "invalid-driver-act",
            "driver act requires a known author, concise reason, and 1..=128 changed artifact paths",
        ));
    }
    let mut paths = BTreeSet::new();
    for path in &request.changed_artifacts {
        let mut components = Path::new(path).components();
        if path.trim().is_empty()
            || path.len() > 1024
            || Path::new(path).is_absolute()
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.any(|part| !matches!(part, Component::Normal(_)))
            || !paths.insert(path)
        {
            return Err(OperationOutcome::rejected(
                "invalid-driver-act",
                "driver-act artifact paths must be unique, relative, and contain no dot or parent components",
            ));
        }
    }
    if [
        &request.unchanged_documents.intent_revision,
        &request.unchanged_documents.design_revision,
        &request.unchanged_documents.plan_revision,
    ]
    .iter()
    .any(|revision| revision.trim().is_empty())
    {
        return Err(OperationOutcome::rejected(
            "invalid-driver-act",
            "driver act must name the unchanged accepted intent, design, and plan revisions",
        ));
    }
    let slot = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.state == run.current_state && slot.event == transition.event)
        .ok_or_else(|| {
            OperationOutcome::rejected(
                "driver-act-not-opted-in",
                "this checked edge has no opt-in bound work slot",
            )
        })?;
    if !slot.driver_act_allowed {
        return Err(OperationOutcome::rejected(
            "driver-act-not-opted-in",
            format!("bound slot `{}` does not permit driver acts", slot.id),
        ));
    }
    let binding = crate::effective_binding(run, &slot.id)
        .and_then(std::result::Result::ok)
        .ok_or_else(|| {
            OperationOutcome::rejected(
                "driver-act-requires-bound-slot",
                format!("opted-in slot `{}` has no valid frozen binding", slot.id),
            )
        })?;
    let state = run
        .workflow
        .states
        .iter()
        .find(|state| state.id == slot.state)
        .ok_or_else(|| {
            OperationOutcome::error(
                "invalid-run",
                format!("driver-act slot `{}` has no source state", slot.id),
            )
        })?;
    let current_subject = persistence
        .get_current_slot_subject(&run.id, &slot.id)
        .map_err(persistence_error)?
        .filter(|subject| !subject.trim().is_empty())
        .ok_or_else(|| {
            OperationOutcome::rejected(
                "no-current-visit-subject",
                format!("driver-act slot `{}` has no current visit subject", slot.id),
            )
        })?;
    let binding_bytes = serde_json::to_vec(&binding).map_err(|error| {
        OperationOutcome::error("driver-act-binding-invalid", error.to_string())
    })?;
    let binding_sha256 = format!("sha256:{:x}", Sha256::digest(binding_bytes));

    Ok(crate::DriverActEvidence {
        request: request.clone(),
        slot_id: slot.id.clone(),
        state_visit: run.control_revision.as_u64(),
        current_subject,
        instruction_digest: instruction_digest(&state.instructions),
        binding_sha256,
    })
}

fn enforce_bound_slot_gate<P>(
    run: &crate::Run,
    transition: &Transition,
    persistence: &P,
    now: Timestamp,
) -> Option<OperationOutcome<Result>>
where
    P: Persistence + ?Sized,
{
    let slot = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.state == run.current_state && slot.event == transition.event)?;
    let _binding = crate::effective_binding(run, &slot.id)?;

    let expected_digest = match run
        .workflow
        .states
        .iter()
        .find(|state| state.id == slot.state)
    {
        Some(state) => instruction_digest(&state.instructions),
        None => {
            return Some(OperationOutcome::error(
                "invalid-run",
                format!(
                    "work slot `{}` names state `{}` which is not in the workflow",
                    slot.id, slot.state
                ),
            ));
        }
    };

    let current_subject = match persistence.get_current_slot_subject(&run.id, &slot.id) {
        Ok(subject) => subject,
        Err(error) => return Some(persistence_error(error)),
    };
    let invocations = match persistence.load_work_slot_invocations(&run.id) {
        Ok(invocations) => invocations,
        Err(error) => return Some(persistence_error(error)),
    };

    let mut overlay_notes = Vec::new();
    for record in &invocations {
        let overlay =
            project_invocation_status(record, now, persistence.invocation_waiter_alive(record));
        if record.slot_id == slot.id {
            overlay_notes.push(overlay_status_label(overlay));
        }
        let subject_matches = current_subject
            .as_ref()
            .is_some_and(|subject| subject == &record.subject);
        if overlay == ProjectedInvocationStatus::Succeeded
            && record.slot_id == slot.id
            && record.instruction_digest == expected_digest
            && subject_matches
        {
            return None;
        }
    }

    let overlay_desc = if overlay_notes.is_empty() {
        "none".to_owned()
    } else {
        overlay_notes.join(", ")
    };
    let live_remedy = invocations.iter().find(|row|
        crate::invocation_owns_work(row, persistence.invocation_waiter_alive(row)))
        .map(|row| format!("; invocation `{}` owns live work or pending cleanup: wait or cancel-invocation {} {} before retry or state departure",
            row.invocation_id, run.id, row.invocation_id)).unwrap_or_default();
    Some(OperationOutcome::rejected(
        BOUND_SLOT_INVOCATION_REQUIRED,
        format!(
            "bound work slot `{}` requires a succeeded invocation matching slot id, instruction digest, and current visit subject; overlay was {overlay_desc}{live_remedy}",
            slot.id
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        instruction_digest, AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
        ContextRecord, ControlRevision, CreateRunRequest, CreateRunResult, DurableEvaluation,
        EvaluationFeedback, HistoryEntry, PersistenceError, PersistenceFailure,
        ProviderAssociation, ProviderError, Run, RunSummary, SemanticSequence, ShowData, State,
        StateId, TerminateRequest, TerminateResult, Timestamp, WaiterWrittenStatus, WorkSlot,
        WorkSlotBinding, WorkSlotInvocation, Workflow,
    };
    use serde_json::json;
    use std::cell::RefCell;

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do work", false),
                State::new("review", "Review", "Review work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![
                Transition::check_free("start", "finish", "done"),
                Transition::checked("start", "approve", "done"),
                Transition::checked("start", "review", "review"),
                Transition::checked("start", "self", "start"),
            ],
        )
    }

    fn run(lifecycle: Lifecycle, state: &str) -> Run {
        Run::new(
            "run-1",
            Some("test".to_owned()),
            workflow(),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "test"}),
            state,
            lifecycle,
            ControlRevision::from_u64(4),
            SemanticSequence::new(8),
            Timestamp::from_unix_millis(1),
        )
    }

    fn failure<T>() -> std::result::Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "fake",
            "fake persistence failure",
        )))
    }

    #[derive(Default)]
    struct FakeGateway {
        result: RefCell<Option<std::result::Result<crate::EvaluationResult, ProviderError>>>,
        requests: RefCell<Vec<crate::EvaluationRequest>>,
    }

    impl FakeGateway {
        fn with_result(
            result: std::result::Result<crate::EvaluationResult, ProviderError>,
        ) -> Self {
            Self {
                result: RefCell::new(Some(result)),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl ProviderGateway for FakeGateway {
        fn describe(
            &self,
            _provider: &ProviderAssociation,
            _initial_input: Option<&Value>,
        ) -> std::result::Result<Workflow, ProviderError> {
            Ok(workflow())
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            request: crate::EvaluationRequest,
        ) -> std::result::Result<crate::EvaluationResult, ProviderError> {
            self.requests.borrow_mut().push(request);
            self.result
                .borrow_mut()
                .take()
                .unwrap_or(Ok(crate::EvaluationResult::Unsupported))
        }
    }

    #[derive(Default)]
    struct FakePersistence {
        authoritative: RefCell<Option<std::result::Result<Run, PersistenceError>>>,
        snapshot: RefCell<Option<std::result::Result<CheckedEvaluationSnapshot, PersistenceError>>>,
        commit:
            RefCell<Option<std::result::Result<crate::CommitTransitionResult, PersistenceError>>>,
        denial: RefCell<Option<std::result::Result<crate::RecordDenialResult, PersistenceError>>>,
        snapshot_requests: RefCell<Vec<CheckedEvaluationSnapshotRequest>>,
        commit_requests: RefCell<Vec<CommitTransitionRequest>>,
        denial_requests: RefCell<Vec<RecordDenialRequest>>,
        invocations: RefCell<Vec<WorkSlotInvocation>>,
        subjects: RefCell<std::collections::BTreeMap<String, String>>,
        set_subject_calls: RefCell<Vec<(RunId, crate::WorkSlotId, String)>>,
        set_subject_error: RefCell<Option<PersistenceError>>,
    }

    impl FakePersistence {
        fn with_run(run: Run) -> Self {
            Self {
                authoritative: RefCell::new(Some(Ok(run))),
                ..Self::default()
            }
        }

        fn with_run_and_snapshot(run: Run, transition: Transition) -> Self {
            let snapshot = CheckedEvaluationSnapshot {
                observed_control_revision: run.control_revision,
                transition: transition.clone(),
                context: vec![
                    ContextRecord::new(
                        "second",
                        "note",
                        json!({"n": 2}),
                        SemanticSequence::new(7),
                        Timestamp::from_unix_millis(7),
                    ),
                    ContextRecord::new(
                        "first",
                        "note",
                        json!({"n": 1}),
                        SemanticSequence::new(3),
                        Timestamp::from_unix_millis(3),
                    ),
                ],
                checked_evaluations: vec![DurableEvaluation::deny(
                    transition,
                    EvaluationFeedback::new("prior", "Prior finding"),
                    SemanticSequence::new(5),
                    Timestamp::from_unix_millis(5),
                )],
                run,
            };
            Self {
                authoritative: RefCell::new(Some(Ok(snapshot.run.clone()))),
                snapshot: RefCell::new(Some(Ok(snapshot))),
                ..Self::default()
            }
        }

        fn conflict() -> PersistenceError {
            PersistenceError::conflict(crate::PersistenceConflict::ControlRevisionMismatch {
                expected: ControlRevision::from_u64(4),
                observed: ControlRevision::from_u64(5),
            })
        }
    }

    impl Persistence for FakePersistence {
        fn create_run(
            &self,
            _request: CreateRunRequest,
        ) -> std::result::Result<CreateRunResult, PersistenceError> {
            failure()
        }

        fn append_context(
            &self,
            _request: AppendContextRequest,
        ) -> std::result::Result<AppendContextResult, PersistenceError> {
            failure()
        }

        fn commit_transition(
            &self,
            request: CommitTransitionRequest,
        ) -> std::result::Result<crate::CommitTransitionResult, PersistenceError> {
            if !request.slot_subjects.is_empty() {
                if let Some(error) = self.set_subject_error.borrow_mut().take() {
                    for (slot_id, subject) in &request.slot_subjects {
                        self.set_subject_calls.borrow_mut().push((
                            request.run_id.clone(),
                            slot_id.clone(),
                            subject.clone(),
                        ));
                    }
                    return Err(error);
                }
                for (slot_id, subject) in &request.slot_subjects {
                    self.set_subject_calls.borrow_mut().push((
                        request.run_id.clone(),
                        slot_id.clone(),
                        subject.clone(),
                    ));
                    self.subjects
                        .borrow_mut()
                        .insert(slot_id.as_str().to_owned(), subject.clone());
                }
            }
            self.commit_requests.borrow_mut().push(request);
            self.commit.borrow_mut().take().unwrap_or_else(failure)
        }

        fn record_denial(
            &self,
            request: RecordDenialRequest,
        ) -> std::result::Result<crate::RecordDenialResult, PersistenceError> {
            self.denial_requests.borrow_mut().push(request);
            self.denial.borrow_mut().take().unwrap_or_else(failure)
        }

        fn terminate(
            &self,
            _request: TerminateRequest,
        ) -> std::result::Result<TerminateResult, PersistenceError> {
            failure()
        }

        fn load_authoritative_run(
            &self,
            run_id: &RunId,
        ) -> std::result::Result<Run, PersistenceError> {
            self.authoritative
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Err(PersistenceError::not_found(run_id.clone())))
        }

        fn list_runs(&self) -> std::result::Result<Vec<RunSummary>, PersistenceError> {
            failure()
        }

        fn load_context_records(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<ContextRecord>, PersistenceError> {
            failure()
        }

        fn load_history(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<HistoryEntry>, PersistenceError> {
            failure()
        }

        fn load_checked_evaluations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<DurableEvaluation>, PersistenceError> {
            failure()
        }

        fn load_checked_evaluation_snapshot(
            &self,
            request: CheckedEvaluationSnapshotRequest,
        ) -> std::result::Result<CheckedEvaluationSnapshot, PersistenceError> {
            self.snapshot_requests.borrow_mut().push(request);
            self.snapshot.borrow_mut().take().unwrap_or_else(failure)
        }

        fn load_show_data(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<ShowData, PersistenceError> {
            failure()
        }

        fn create_work_slot_invocation(
            &self,
            _request: crate::CreateWorkSlotInvocationRequest,
        ) -> std::result::Result<crate::CreateWorkSlotInvocationResult, PersistenceError> {
            failure()
        }

        fn complete_work_slot_invocation(
            &self,
            _request: crate::CompleteWorkSlotInvocationRequest,
        ) -> std::result::Result<crate::CompleteWorkSlotInvocationResult, PersistenceError>
        {
            failure()
        }

        fn get_current_slot_subject(
            &self,
            _run_id: &RunId,
            slot_id: &crate::WorkSlotId,
        ) -> std::result::Result<Option<String>, PersistenceError> {
            Ok(self.subjects.borrow().get(slot_id.as_str()).cloned())
        }

        fn set_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &crate::WorkSlotId,
            subject: String,
        ) -> std::result::Result<(), PersistenceError> {
            self.set_subject_calls.borrow_mut().push((
                run_id.clone(),
                slot_id.clone(),
                subject.clone(),
            ));
            if let Some(error) = self.set_subject_error.borrow_mut().take() {
                return Err(error);
            }
            self.subjects
                .borrow_mut()
                .insert(slot_id.as_str().to_owned(), subject);
            Ok(())
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<crate::WorkSlotInvocation>, PersistenceError> {
            Ok(self.invocations.borrow().clone())
        }

        fn invocation_waiter_alive(&self, invocation: &WorkSlotInvocation) -> bool {
            invocation.waiter_pid == std::process::id()
        }
    }

    fn successful_commit(run: Run, transition: Transition) -> crate::CommitTransitionResult {
        crate::CommitTransitionResult {
            history: HistoryEntry::transition(
                9_u64.into(),
                Timestamp::from_unix_millis(9),
                transition,
                crate::TransitionHistoryOutcome::Committed,
            ),
            run,
        }
    }

    fn successful_denial(
        run: Run,
        transition: Transition,
        feedback: EvaluationFeedback,
    ) -> crate::RecordDenialResult {
        crate::RecordDenialResult {
            evaluation: DurableEvaluation::deny(
                transition.clone(),
                feedback.clone(),
                9_u64.into(),
                Timestamp::from_unix_millis(9),
            ),
            history: HistoryEntry::transition(
                9_u64.into(),
                Timestamp::from_unix_millis(9),
                transition,
                crate::TransitionHistoryOutcome::Denied { feedback },
            ),
            run,
        }
    }

    fn attestation() -> crate::StateVisitAttestation {
        crate::StateVisitAttestation {
            state_visit: 4,
            owner: "owner".into(),
            reason: "explicit exception".into(),
        }
    }

    fn driver_act_request() -> crate::DriverActRequest {
        crate::DriverActRequest {
            author: crate::DriverActAuthor {
                name: "driver".into(),
                kind: "agent".into(),
            },
            reason: "narrow correction".into(),
            changed_artifacts: vec!["src/fix.rs".into()],
            unchanged_documents: crate::DriverActDocuments {
                intent_revision: "intent-r1".into(),
                design_revision: "design-r1".into(),
                plan_revision: "plan-r1".into(),
            },
        }
    }

    #[test]
    fn opted_in_driver_act_is_attributed_and_still_runs_provider_evaluation() {
        let transition = workflow()
            .transitions
            .into_iter()
            .find(|edge| edge.event.as_str() == "approve")
            .expect("checked edge");
        let mut run = run(Lifecycle::Active, "start");
        run.workflow.work_slots =
            vec![WorkSlot::new("implement", "start", "approve").with_driver_act_allowed(true)];
        run.initial_input =
            json!({"work_slot_bindings":{"implement":{"command":"worker","args":[]}}});
        let persistence = FakePersistence::with_run_and_snapshot(run.clone(), transition.clone());
        persistence
            .subjects
            .borrow_mut()
            .insert("implement".into(), "visit-current".into());
        *persistence.commit.borrow_mut() = Some(Ok(successful_commit(run, transition.clone())));
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));
        let outcome = execute(
            Request::new("run-1", "approve").with_driver_act(driver_act_request()),
            &gateway,
            &persistence,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        let requests = gateway.requests.borrow();
        let act = requests[0]
            .driver_act
            .as_ref()
            .expect("provider receives the act");
        assert_eq!(act.slot_id.as_str(), "implement");
        assert_eq!(act.state_visit, 4);
        assert_eq!(act.current_subject, "visit-current");
        assert!(!act.binding_sha256.is_empty());
        assert!(!act.instruction_digest.is_empty());
        let commits = persistence.commit_requests.borrow();
        assert!(commits[0].exception.is_none());
        assert_eq!(commits[0].driver_act.as_ref(), Some(act));
        assert_eq!(persistence.invocations.borrow().len(), 0);
    }

    #[test]
    fn driver_act_cannot_use_a_non_opted_in_review_slot() {
        let mut run = run(Lifecycle::Active, "start");
        run.workflow.work_slots = vec![WorkSlot::new("review-slot", "start", "approve")];
        run.initial_input =
            json!({"work_slot_bindings":{"review-slot":{"command":"worker","args":[]}}});
        let persistence = FakePersistence::with_run(run);
        let gateway = FakeGateway::default();
        let outcome = execute(
            Request::new("run-1", "approve").with_driver_act(driver_act_request()),
            &gateway,
            &persistence,
        );
        assert_eq!(outcome.issue().unwrap().code, "driver-act-not-opted-in");
        assert!(gateway.requests.borrow().is_empty());
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn recovery_override_skips_bound_completion_and_evaluation_not_obligation_history() {
        let persistence = bound_checked_persistence(Vec::new(), Some("visit-current"));
        let gateway = FakeGateway::default();
        let outcome = execute(
            Request::new("run-1", "approve").with_override(attestation()),
            &gateway,
            &persistence,
        );
        assert!(outcome.is_completed());
        assert!(gateway.requests.borrow().is_empty());
        assert!(persistence.snapshot_requests.borrow().is_empty());
        assert!(persistence.denial_requests.borrow().is_empty());
        let commits = persistence.commit_requests.borrow();
        let exception = commits[0].exception.as_ref().unwrap();
        assert_eq!(exception.attestation, attestation());
        assert_eq!(
            exception.provider_evaluation,
            crate::SkippedProviderEvaluation::NotPerformed
        );
        assert_eq!(exception.skipped_bound_checks.len(), 1);
        assert_eq!(exception.skipped_bound_checks[0].slot_id.as_str(), "slot-1");
        assert_eq!(
            exception.skipped_bound_checks[0].subject.as_deref(),
            Some("visit-current")
        );
        assert_eq!(commits[0].context_append, None);
    }

    #[test]
    fn recovery_override_refuses_stale_empty_unavailable_and_terminal_without_mutation() {
        for (visit, owner, reason, event, lifecycle, code) in [
            (
                3,
                "owner",
                "reason",
                "approve",
                Lifecycle::Active,
                "stale-state-visit",
            ),
            (
                4,
                " ",
                "reason",
                "approve",
                Lifecycle::Active,
                "invalid-override",
            ),
            (
                4,
                "owner",
                " ",
                "approve",
                Lifecycle::Active,
                "invalid-override",
            ),
            (
                4,
                "owner",
                "reason",
                "missing",
                Lifecycle::Active,
                "event-unavailable",
            ),
            (
                4,
                "owner",
                "reason",
                "approve",
                Lifecycle::Final,
                "run-not-active",
            ),
        ] {
            let persistence = FakePersistence::with_run(run(lifecycle, "start"));
            let gateway = FakeGateway::default();
            let request =
                Request::new("run-1", event).with_override(crate::StateVisitAttestation {
                    state_visit: visit,
                    owner: owner.into(),
                    reason: reason.into(),
                });
            let outcome = execute(request, &gateway, &persistence);
            assert_eq!(outcome.issue().unwrap().code, code);
            assert!(persistence.commit_requests.borrow().is_empty());
            assert!(persistence.denial_requests.borrow().is_empty());
            assert!(gateway.requests.borrow().is_empty());
        }
    }

    #[test]
    fn recovery_override_cannot_bypass_live_or_elapsed_work() {
        for now in [1500, 90000] {
            let persistence = bound_checked_persistence(
                vec![sample_invocation(
                    "slot-1",
                    start_instructions_digest(),
                    "visit-current",
                    None,
                    alive_pid(),
                    1000,
                    5000,
                )],
                Some("visit-current"),
            );
            let gateway = FakeGateway::default();
            let outcome = execute(
                Request::new("run-1", "approve")
                    .with_now(Timestamp::from_unix_millis(now))
                    .with_override(attestation()),
                &gateway,
                &persistence,
            );
            assert_eq!(outcome.issue().unwrap().code, "live-owned-work");
            assert!(persistence.commit_requests.borrow().is_empty());
            assert!(gateway.requests.borrow().is_empty());
        }
    }

    #[test]
    fn missing_run_is_error_without_provider_or_mutation() {
        let persistence = FakePersistence::default();
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("missing", "approve"), &gateway, &persistence);

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "run-not-found");
        assert!(gateway.requests.borrow().is_empty());
        assert!(persistence.snapshot_requests.borrow().is_empty());
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn unavailable_event_is_rejected_without_provider_or_history_write() {
        let persistence = FakePersistence::with_run(run(Lifecycle::Active, "start"));
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "missing"), &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "event-unavailable");
        assert!(gateway.requests.borrow().is_empty());
        assert!(persistence.snapshot_requests.borrow().is_empty());
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn terminal_event_is_rejected_without_provider_or_history_write() {
        let persistence = FakePersistence::with_run(run(Lifecycle::Final, "done"));
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "run-not-active");
        assert!(gateway.requests.borrow().is_empty());
        assert!(persistence.snapshot_requests.borrow().is_empty());
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn check_free_success_commits_authoritative_transition_without_provider() {
        let current = run(Lifecycle::Active, "start");
        let transition = Transition::check_free("start", "finish", "done");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition.clone(),
            )))),
            ..FakePersistence::with_run(current.clone())
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "finish"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(outcome.value().unwrap().run.lifecycle, Lifecycle::Final);
        assert!(gateway.requests.borrow().is_empty());
        let requests = persistence.commit_requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].expected_control_revision,
            current.control_revision
        );
        assert_eq!(requests[0].expected_source_state, StateId::from("start"));
        assert_eq!(requests[0].transition, transition);
        assert_eq!(requests[0].resulting_lifecycle, Lifecycle::Final);
    }

    #[test]
    fn check_free_conflict_is_error_without_retry_or_provider() {
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Err(FakePersistence::conflict()))),
            ..FakePersistence::with_run(run(Lifecycle::Active, "start"))
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "finish"), &gateway, &persistence);

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "control-revision-conflict");
        assert_eq!(persistence.commit_requests.borrow().len(), 1);
        assert!(gateway.requests.borrow().is_empty());
    }

    #[test]
    fn checked_allow_uses_snapshot_lineage_and_commits_original_control_point() {
        let current = run(Lifecycle::Active, "start");
        let transition = Transition::checked("start", "approve", "done");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition.clone(),
            )))),
            ..FakePersistence::with_run_and_snapshot(current.clone(), transition.clone())
        };
        let effect = crate::ContextAppendEffect::new("snapshot", json!({"revision": 1}));
        let gateway = FakeGateway::with_result(Ok(
            crate::EvaluationResult::allow_with_context_append(effect.clone()),
        ));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_completed());
        let requests = gateway.requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].transition, transition);
        assert_eq!(requests[0].context[0].id.as_str(), "first");
        assert_eq!(requests[0].context[1].id.as_str(), "second");
        assert_eq!(requests[0].prior_evaluations.len(), 1);
        assert_eq!(
            requests[0].prior_evaluations[0].sequence,
            SemanticSequence::new(5)
        );
        let commits = persistence.commit_requests.borrow();
        assert_eq!(commits.len(), 1);
        assert_eq!(
            commits[0].expected_control_revision,
            current.control_revision
        );
        assert_eq!(commits[0].expected_source_state, current.current_state);
        assert_eq!(commits[0].resulting_lifecycle, Lifecycle::Final);
        assert_eq!(commits[0].context_append, Some(effect));
        assert!(persistence.denial_requests.borrow().is_empty());
    }

    #[test]
    fn checked_deny_records_actionable_feedback_and_preserves_state() {
        let current = run(Lifecycle::Active, "start");
        let transition = Transition::checked("start", "review", "review");
        let feedback = EvaluationFeedback::new("needs-work", "Address the findings")
            .with_details(json!({"finding": "tests"}));
        let persistence = FakePersistence {
            denial: RefCell::new(Some(Ok(successful_denial(
                current.clone(),
                transition.clone(),
                feedback.clone(),
            )))),
            ..FakePersistence::with_run_and_snapshot(current.clone(), transition.clone())
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::deny(feedback.clone())));

        let outcome = execute(Request::new("run-1", "review"), &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "needs-work");
        assert_eq!(outcome.issue().unwrap().details, feedback.details);
        assert!(persistence.commit_requests.borrow().is_empty());
        let denials = persistence.denial_requests.borrow();
        assert_eq!(denials.len(), 1);
        assert_eq!(
            denials[0].expected_control_revision,
            current.control_revision
        );
        assert_eq!(denials[0].expected_source_state, current.current_state);
        assert_eq!(denials[0].transition, transition);
        assert_eq!(denials[0].feedback, feedback);
    }

    #[test]
    fn unsupported_and_provider_failure_are_errors_without_history_writes() {
        let transition = Transition::checked("start", "approve", "done");
        let make_persistence = || {
            FakePersistence::with_run_and_snapshot(
                run(Lifecycle::Active, "start"),
                transition.clone(),
            )
        };

        let unsupported_persistence = make_persistence();
        let unsupported_gateway =
            FakeGateway::with_result(Ok(crate::EvaluationResult::Unsupported));
        let unsupported = execute(
            Request::new("run-1", "approve"),
            &unsupported_gateway,
            &unsupported_persistence,
        );
        assert!(unsupported.is_error());
        assert_eq!(unsupported.issue().unwrap().code, "provider-unsupported");
        assert!(unsupported_persistence.commit_requests.borrow().is_empty());
        assert!(unsupported_persistence.denial_requests.borrow().is_empty());

        let failure_persistence = make_persistence();
        let failure_gateway =
            FakeGateway::with_result(Err(ProviderError::execution("crashed", "provider exited")));
        let failed = execute(
            Request::new("run-1", "approve"),
            &failure_gateway,
            &failure_persistence,
        );
        assert!(failed.is_error());
        assert_eq!(failed.issue().unwrap().code, "provider-execution-failed");
        assert!(failure_persistence.commit_requests.borrow().is_empty());
        assert!(failure_persistence.denial_requests.borrow().is_empty());
    }

    #[test]
    fn stale_allow_is_error_and_is_not_retried() {
        let transition = Transition::checked("start", "approve", "done");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Err(FakePersistence::conflict()))),
            ..FakePersistence::with_run_and_snapshot(run(Lifecycle::Active, "start"), transition)
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "control-revision-conflict");
        assert_eq!(persistence.commit_requests.borrow().len(), 1);
        assert_eq!(gateway.requests.borrow().len(), 1);
        assert!(persistence.denial_requests.borrow().is_empty());
    }

    #[test]
    fn stale_deny_is_error_and_is_not_recorded_or_retried() {
        let transition = Transition::checked("start", "review", "review");
        let feedback = EvaluationFeedback::new("needs-work", "Address the findings");
        let persistence = FakePersistence {
            denial: RefCell::new(Some(Err(FakePersistence::conflict()))),
            ..FakePersistence::with_run_and_snapshot(run(Lifecycle::Active, "start"), transition)
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::deny(feedback)));

        let outcome = execute(Request::new("run-1", "review"), &gateway, &persistence);

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "control-revision-conflict");
        assert_eq!(persistence.denial_requests.borrow().len(), 1);
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn self_loop_checked_attempt_uses_source_and_revision_for_staleness() {
        let transition = Transition::checked("start", "self", "start");
        let current = run(Lifecycle::Active, "start");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Err(FakePersistence::conflict()))),
            ..FakePersistence::with_run_and_snapshot(current.clone(), transition.clone())
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "self"), &gateway, &persistence);

        assert!(outcome.is_error());
        let commits = persistence.commit_requests.borrow();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].expected_source_state, StateId::from("start"));
        assert_eq!(commits[0].transition.target, StateId::from("start"));
        assert_eq!(
            commits[0].expected_control_revision,
            current.control_revision
        );
    }

    #[test]
    fn checked_allow_to_final_state_computes_final_lifecycle() {
        let transition = Transition::checked("start", "approve", "done");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition.clone(),
            )))),
            ..FakePersistence::with_run_and_snapshot(run(Lifecycle::Active, "start"), transition)
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(
            persistence.commit_requests.borrow()[0].resulting_lifecycle,
            Lifecycle::Final
        );
    }

    fn bound_input() -> serde_json::Value {
        json!({
            "objective": "test",
            "work_slot_bindings": {
                "slot-1": {"command": "echo", "args": ["ok"]}
            }
        })
    }

    fn slotted_workflow(slot_event: &str) -> Workflow {
        workflow().with_work_slots(vec![WorkSlot::new("slot-1", "start", slot_event)])
    }

    fn slotted_run(workflow: Workflow, input: serde_json::Value) -> Run {
        Run::new(
            "run-1",
            Some("test".to_owned()),
            workflow,
            ProviderAssociation::new(json!({"provider": "fake"})),
            input,
            "start",
            Lifecycle::Active,
            ControlRevision::from_u64(4),
            SemanticSequence::new(8),
            Timestamp::from_unix_millis(1),
        )
    }

    fn start_instructions_digest() -> String {
        instruction_digest("Do work")
    }

    fn alive_pid() -> u32 {
        std::process::id()
    }

    fn dead_pid() -> u32 {
        // A legal pid that is extremely unlikely to exist. Do not use
        // `u32::MAX`: that casts to `-1`, and `kill(-1, 0)` broadcasts.
        i32::MAX as u32
    }

    fn sample_invocation(
        slot_id: &str,
        digest: String,
        subject: &str,
        status: Option<WaiterWrittenStatus>,
        waiter_pid: u32,
        started_at: i64,
        allowed_time_ms: u64,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            "inv-1",
            slot_id,
            WorkSlotBinding::new("echo", vec!["ok".to_owned()]),
            digest,
            subject,
            waiter_pid,
            Timestamp::from_unix_millis(started_at),
            allowed_time_ms,
            status,
            None,
            None,
            String::new(),
            Vec::new(),
        )
    }

    fn bound_checked_persistence(
        invocations: Vec<WorkSlotInvocation>,
        subject: Option<&str>,
    ) -> FakePersistence {
        let transition = Transition::checked("start", "approve", "done");
        let current = slotted_run(slotted_workflow("approve"), bound_input());
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition.clone(),
            )))),
            invocations: RefCell::new(invocations),
            ..FakePersistence::with_run_and_snapshot(current, transition)
        };
        if let Some(subject) = subject {
            persistence
                .subjects
                .borrow_mut()
                .insert("slot-1".to_owned(), subject.to_owned());
        }
        persistence
    }

    fn assert_gate_rejected(outcome: &OperationOutcome<Result>, gateway: &FakeGateway) {
        assert!(outcome.is_rejected());
        assert_eq!(
            outcome.issue().unwrap().code,
            BOUND_SLOT_INVOCATION_REQUIRED
        );
        assert!(gateway.requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_overlay_succeeded_matching_id_digest_subject_allows_evaluate() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                start_instructions_digest(),
                subject,
                Some(WaiterWrittenStatus::Succeeded),
                dead_pid(),
                0,
                1_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(gateway.requests.borrow().len(), 1);
        assert_eq!(persistence.commit_requests.borrow().len(), 1);
    }

    #[test]
    fn work_slot_gate_overlay_running_rejects_without_evaluate() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                start_instructions_digest(),
                subject,
                None,
                alive_pid(),
                1_000,
                10_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(
            Request::new("run-1", "approve").with_now(Timestamp::from_unix_millis(1_500)),
            &gateway,
            &persistence,
        );

        assert_gate_rejected(&outcome, &gateway);
        assert!(outcome.issue().unwrap().message.contains("running"));
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_overlay_failed_rejects_without_evaluate() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                start_instructions_digest(),
                subject,
                Some(WaiterWrittenStatus::Failed),
                dead_pid(),
                0,
                1_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert_gate_rejected(&outcome, &gateway);
        assert!(outcome.issue().unwrap().message.contains("failed"));
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_projected_overrun_rejects_without_evaluate() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                start_instructions_digest(),
                subject,
                None,
                alive_pid(),
                1_000,
                5_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(
            Request::new("run-1", "approve").with_now(Timestamp::from_unix_millis(6_000)),
            &gateway,
            &persistence,
        );

        assert_gate_rejected(&outcome, &gateway);
        assert!(outcome.issue().unwrap().message.contains("overrun"));
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_mismatched_slot_id_rejects() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "other-slot",
                start_instructions_digest(),
                subject,
                Some(WaiterWrittenStatus::Succeeded),
                dead_pid(),
                0,
                1_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert_gate_rejected(&outcome, &gateway);
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_mismatched_digest_rejects() {
        let subject = "visit-slot-1-1";
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                instruction_digest("different instructions"),
                subject,
                Some(WaiterWrittenStatus::Succeeded),
                dead_pid(),
                0,
                1_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert_gate_rejected(&outcome, &gateway);
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_mismatched_subject_new_visit_rejects() {
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                start_instructions_digest(),
                "visit-old",
                Some(WaiterWrittenStatus::Succeeded),
                dead_pid(),
                0,
                1_000,
            )],
            Some("visit-new"),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert_gate_rejected(&outcome, &gateway);
        assert!(persistence.commit_requests.borrow().is_empty());
    }

    #[test]
    fn work_slot_gate_check_free_edge_proceeds_without_invocation() {
        let current = slotted_run(slotted_workflow("finish"), bound_input());
        let transition = Transition::check_free("start", "finish", "done");
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition,
            )))),
            ..FakePersistence::with_run(current)
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "finish"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert!(gateway.requests.borrow().is_empty());
        assert_eq!(persistence.commit_requests.borrow().len(), 1);
    }

    #[test]
    fn work_slot_gate_unbound_checked_slot_proceeds_without_invocation() {
        let transition = Transition::checked("start", "approve", "done");
        let current = slotted_run(slotted_workflow("approve"), json!({"objective": "test"}));
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                run(Lifecycle::Final, "done"),
                transition.clone(),
            )))),
            ..FakePersistence::with_run_and_snapshot(current, transition)
        };
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(gateway.requests.borrow().len(), 1);
        assert_eq!(persistence.commit_requests.borrow().len(), 1);
    }

    #[test]
    fn work_slot_gate_digest_match_uses_instruction_digest_helper() {
        let subject = "visit-slot-1-1";
        let expected = instruction_digest("Do work");
        assert_eq!(expected, start_instructions_digest());
        let persistence = bound_checked_persistence(
            vec![sample_invocation(
                "slot-1",
                expected,
                subject,
                Some(WaiterWrittenStatus::Succeeded),
                dead_pid(),
                0,
                1_000,
            )],
            Some(subject),
        );
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "approve"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(gateway.requests.borrow().len(), 1);
    }

    #[test]
    fn work_slot_gate_subject_reminted_on_later_entry_old_subject_not_current() {
        let transition = Transition::checked("start", "self", "start");
        let current = slotted_run(slotted_workflow("approve"), bound_input());
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                current.clone(),
                transition.clone(),
            )))),
            ..FakePersistence::with_run_and_snapshot(current, transition)
        };
        persistence
            .subjects
            .borrow_mut()
            .insert("slot-1".to_owned(), "visit-old".to_owned());
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "self"), &gateway, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(gateway.requests.borrow().len(), 1);
        let calls = persistence.set_subject_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.as_str(), "slot-1");
        assert_ne!(calls[0].2, "visit-old");
        assert!(calls[0].2.starts_with("visit-slot-1-"));
        assert_eq!(
            persistence
                .subjects
                .borrow()
                .get("slot-1")
                .map(String::as_str),
            Some(calls[0].2.as_str())
        );
        assert_ne!(
            persistence
                .subjects
                .borrow()
                .get("slot-1")
                .map(String::as_str),
            Some("visit-old")
        );
    }

    #[test]
    fn work_slot_subject_mint_failure_does_not_commit_transition() {
        let transition = Transition::checked("start", "self", "start");
        let current = slotted_run(slotted_workflow("approve"), bound_input());
        let persistence = FakePersistence {
            commit: RefCell::new(Some(Ok(successful_commit(
                current.clone(),
                transition.clone(),
            )))),
            set_subject_error: RefCell::new(Some(PersistenceError::failure(
                PersistenceFailure::new("fake", "could not store visit subject"),
            ))),
            ..FakePersistence::with_run_and_snapshot(current, transition)
        };
        persistence
            .subjects
            .borrow_mut()
            .insert("slot-1".to_owned(), "visit-old".to_owned());
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

        let outcome = execute(Request::new("run-1", "self"), &gateway, &persistence);

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "persistence-failure");
        assert!(persistence.commit_requests.borrow().is_empty());
        assert!(persistence.commit.borrow().is_some());
        assert_eq!(
            persistence
                .subjects
                .borrow()
                .get("slot-1")
                .map(String::as_str),
            Some("visit-old")
        );
    }
}
