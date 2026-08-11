//! `event` workflow-control operation.
//!
//! Event processing is the one core use case that combines authoritative
//! workflow resolution, conditional persistence, and (for checked edges)
//! provider evaluation.  Core chooses the edge and computes the resulting
//! lifecycle; persistence owns the atomic conditional mutation boundary.

use super::{persistence_error, provider_error};
use crate::{
    request_from_snapshot, resolve_transition, CheckedEvaluationSnapshotRequest,
    CommitTransitionRequest, CommitTransitionResult, EvaluationResult, EventId, Lifecycle,
    OperationOutcome, OutcomeIssue, Persistence, ProviderGateway, RecordDenialRequest, RunId,
    Transition, TransitionResolutionError,
};
use serde::{Deserialize, Serialize};

/// Caller-supplied values for one event request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
    pub event: EventId,
}

impl Request {
    pub fn new(run_id: impl Into<RunId>, event: impl Into<EventId>) -> Self {
        Self {
            run_id: run_id.into(),
            event: event.into(),
        }
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
/// state.  Check-free edges go directly to a conditional atomic commit.  A
/// checked edge captures one durable snapshot, ends persistence activity,
/// invokes the provider, and conditionally commits or records the result
/// against the snapshot's original control point.
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

    if transition.kind.is_check_free() {
        return commit_check_free(&run, &transition, persistence);
    }

    evaluate_checked(run, &transition, gateway, persistence)
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

fn commit_check_free<P>(
    run: &crate::Run,
    transition: &Transition,
    persistence: &P,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    let resulting_lifecycle = match target_lifecycle(&run.workflow, transition) {
        Ok(lifecycle) => lifecycle,
        Err(issue) => return OperationOutcome::error_with_issue(issue),
    };

    let request = CommitTransitionRequest::new(
        run.id.clone(),
        run.control_revision,
        run.current_state.clone(),
        transition.clone(),
        resulting_lifecycle,
    );

    match persistence.commit_transition(request) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

fn evaluate_checked<G, P>(
    run: crate::Run,
    transition: &Transition,
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
    let evaluation_request = request_from_snapshot(&snapshot_for_request);

    // `load_checked_evaluation_snapshot` is the complete persistence activity
    // for this phase.  The gateway call is deliberately made after it
    // returns, so provider execution is never inside a persistence boundary.
    let evaluation = match gateway.evaluate(&snapshot.run.provider_association, evaluation_request)
    {
        Ok(result) => result,
        Err(error) => return provider_error(error),
    };

    match evaluation {
        EvaluationResult::Allow => commit_checked_allow(snapshot, transition, persistence),
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
    persistence: &P,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    let resulting_lifecycle = match target_lifecycle(&snapshot.run.workflow, transition) {
        Ok(lifecycle) => lifecycle,
        Err(issue) => return OperationOutcome::error_with_issue(issue),
    };

    let request = CommitTransitionRequest::new(
        snapshot.run.id,
        snapshot.observed_control_revision,
        snapshot.run.current_state,
        transition.clone(),
        resulting_lifecycle,
    );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot, ContextRecord,
        ControlRevision, CreateRunRequest, CreateRunResult, DurableEvaluation, EvaluationFeedback,
        HistoryEntry, PersistenceError, PersistenceFailure, ProviderAssociation, ProviderError,
        Run, RunSummary, SemanticSequence, ShowData, State, StateId, TerminateRequest,
        TerminateResult, Timestamp, Workflow,
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
        let gateway = FakeGateway::with_result(Ok(crate::EvaluationResult::Allow));

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
}
