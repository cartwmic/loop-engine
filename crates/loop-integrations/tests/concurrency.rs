use loop_core::{
    operations::{execute_append, execute_event, execute_terminate, AppendRequest, EventRequest},
    AppendContextRequest, CheckedEvaluationSnapshotRequest, CommitTransitionRequest, ContextRecord,
    CreateRunRequest, EvaluationFeedback, EvaluationRequest, EvaluationResult, HistoryAction,
    Lifecycle, OperationOutcome, Persistence, ProviderAssociation, ProviderError, ProviderGateway,
    RecordDenialRequest, Run, State, TerminateRequest, Timestamp, Transition,
    TransitionHistoryOutcome, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::json;
use std::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Barrier, Mutex,
    },
    thread,
};
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "concurrency-fixture",
        "start",
        vec![
            State::new("start", "Start", "Begin work", false),
            State::new("left", "Left", "Continue on the left", false),
            State::new("right", "Right", "Continue on the right", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::check_free("start", "left", "left"),
            Transition::check_free("start", "right", "right"),
            Transition::check_free("start", "self", "start"),
            Transition::checked("start", "approve", "done"),
            Transition::checked("start", "alternate", "left"),
        ],
    )
}

fn create_run(
    adapter: &SqlitePersistence,
    run_id: &str,
) -> Result<Run, Box<dyn std::error::Error>> {
    let created = adapter.create_run(CreateRunRequest::new(
        run_id,
        Some("concurrency test".to_owned()),
        workflow(),
        ProviderAssociation::new(json!({"provider": "blocking-test-gateway"})),
        json!({"objective": "exercise one control point"}),
        "start",
        Lifecycle::Active,
        Timestamp::from_unix_millis(1),
    ))?;
    Ok(created.run)
}

fn append_request(run_id: &str, record_id: &str, value: i64) -> AppendContextRequest {
    AppendContextRequest::new(
        run_id,
        record_id,
        "observation",
        json!({"value": value}),
        Timestamp::from_unix_millis(value),
    )
}

fn check_free_transition(event: &str, target: &str) -> Transition {
    Transition::check_free("start", event, target)
}

fn checked_transition() -> Transition {
    Transition::checked("start", "approve", "done")
}

fn alternate_checked_transition() -> Transition {
    Transition::checked("start", "alternate", "left")
}

/// A deterministic provider fixture: it publishes the request after the
/// evaluation snapshot has been captured, then waits for the test to release
/// provider execution.  The same fixture can coordinate one or many calls.
#[derive(Clone)]
struct BlockingGateway {
    ready: Sender<EvaluationRequest>,
    release: Arc<Mutex<Receiver<()>>>,
    result: EvaluationResult,
}

impl BlockingGateway {
    fn new(result: EvaluationResult) -> (Self, Receiver<EvaluationRequest>, Sender<()>) {
        let (ready, ready_receiver) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        (
            Self {
                ready,
                release: Arc::new(Mutex::new(release_receiver)),
                result,
            },
            ready_receiver,
            release,
        )
    }
}

impl ProviderGateway for BlockingGateway {
    fn describe(&self, _provider: &ProviderAssociation) -> Result<Workflow, ProviderError> {
        Ok(workflow())
    }

    fn evaluate(
        &self,
        _provider: &ProviderAssociation,
        request: EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError> {
        self.ready
            .send(request)
            .expect("concurrency test receiver remains available");
        self.release
            .lock()
            .expect("concurrency test release mutex is not poisoned")
            .recv()
            .expect("concurrency test release remains available");
        Ok(self.result.clone())
    }
}

#[derive(Clone, Default)]
struct CountingGateway {
    evaluations: Arc<std::sync::atomic::AtomicUsize>,
}

impl CountingGateway {
    fn count(&self) -> usize {
        self.evaluations.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl ProviderGateway for CountingGateway {
    fn describe(&self, _provider: &ProviderAssociation) -> Result<Workflow, ProviderError> {
        Ok(workflow())
    }

    fn evaluate(
        &self,
        _provider: &ProviderAssociation,
        _request: EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError> {
        self.evaluations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(EvaluationResult::Allow)
    }
}

/// Gate the first authoritative read of each event attempt.  A three-party
/// barrier lets the test release both reads only after both attempts have
/// observed the same control point.
struct FirstReadBarrierPersistence {
    inner: SqlitePersistence,
    gate: Arc<Barrier>,
}

impl FirstReadBarrierPersistence {
    fn new(inner: SqlitePersistence, gate: Arc<Barrier>) -> Self {
        Self { inner, gate }
    }
}

impl Persistence for FirstReadBarrierPersistence {
    fn create_run(
        &self,
        request: CreateRunRequest,
    ) -> Result<loop_core::CreateRunResult, loop_core::PersistenceError> {
        self.inner.create_run(request)
    }

    fn append_context(
        &self,
        request: AppendContextRequest,
    ) -> Result<loop_core::AppendContextResult, loop_core::PersistenceError> {
        self.inner.append_context(request)
    }

    fn commit_transition(
        &self,
        request: CommitTransitionRequest,
    ) -> Result<loop_core::CommitTransitionResult, loop_core::PersistenceError> {
        self.inner.commit_transition(request)
    }

    fn record_denial(
        &self,
        request: RecordDenialRequest,
    ) -> Result<loop_core::RecordDenialResult, loop_core::PersistenceError> {
        self.inner.record_denial(request)
    }

    fn terminate(
        &self,
        request: TerminateRequest,
    ) -> Result<loop_core::TerminateResult, loop_core::PersistenceError> {
        self.inner.terminate(request)
    }

    fn load_authoritative_run(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Run, loop_core::PersistenceError> {
        let result = self.inner.load_authoritative_run(run_id);
        if result.is_ok() {
            self.gate.wait();
        }
        result
    }

    fn list_runs(&self) -> Result<Vec<loop_core::RunSummary>, loop_core::PersistenceError> {
        self.inner.list_runs()
    }

    fn load_context_records(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<ContextRecord>, loop_core::PersistenceError> {
        self.inner.load_context_records(run_id)
    }

    fn load_history(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<loop_core::HistoryEntry>, loop_core::PersistenceError> {
        self.inner.load_history(run_id)
    }

    fn load_checked_evaluations(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<loop_core::DurableEvaluation>, loop_core::PersistenceError> {
        self.inner.load_checked_evaluations(run_id)
    }

    fn load_checked_evaluation_snapshot(
        &self,
        request: CheckedEvaluationSnapshotRequest,
    ) -> Result<loop_core::CheckedEvaluationSnapshot, loop_core::PersistenceError> {
        self.inner.load_checked_evaluation_snapshot(request)
    }

    fn load_show_data(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<loop_core::ShowData, loop_core::PersistenceError> {
        self.inner.load_show_data(run_id)
    }
}

/// Two phase gate for the asymmetric snapshot-window test.  The event thread
/// has returned its authoritative read before the test commits termination;
/// only then is it allowed to request the checked snapshot.
struct AuthoritativeReadGatePersistence {
    inner: SqlitePersistence,
    ready: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl AuthoritativeReadGatePersistence {
    fn new(inner: SqlitePersistence, ready: Arc<Barrier>, release: Arc<Barrier>) -> Self {
        Self {
            inner,
            ready,
            release,
        }
    }
}

impl Persistence for AuthoritativeReadGatePersistence {
    fn create_run(
        &self,
        request: CreateRunRequest,
    ) -> Result<loop_core::CreateRunResult, loop_core::PersistenceError> {
        self.inner.create_run(request)
    }

    fn append_context(
        &self,
        request: AppendContextRequest,
    ) -> Result<loop_core::AppendContextResult, loop_core::PersistenceError> {
        self.inner.append_context(request)
    }

    fn commit_transition(
        &self,
        request: CommitTransitionRequest,
    ) -> Result<loop_core::CommitTransitionResult, loop_core::PersistenceError> {
        self.inner.commit_transition(request)
    }

    fn record_denial(
        &self,
        request: RecordDenialRequest,
    ) -> Result<loop_core::RecordDenialResult, loop_core::PersistenceError> {
        self.inner.record_denial(request)
    }

    fn terminate(
        &self,
        request: TerminateRequest,
    ) -> Result<loop_core::TerminateResult, loop_core::PersistenceError> {
        self.inner.terminate(request)
    }

    fn load_authoritative_run(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Run, loop_core::PersistenceError> {
        let result = self.inner.load_authoritative_run(run_id);
        if result.is_ok() {
            self.ready.wait();
            self.release.wait();
        }
        result
    }

    fn list_runs(&self) -> Result<Vec<loop_core::RunSummary>, loop_core::PersistenceError> {
        self.inner.list_runs()
    }

    fn load_context_records(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<ContextRecord>, loop_core::PersistenceError> {
        self.inner.load_context_records(run_id)
    }

    fn load_history(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<loop_core::HistoryEntry>, loop_core::PersistenceError> {
        self.inner.load_history(run_id)
    }

    fn load_checked_evaluations(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<Vec<loop_core::DurableEvaluation>, loop_core::PersistenceError> {
        self.inner.load_checked_evaluations(run_id)
    }

    fn load_checked_evaluation_snapshot(
        &self,
        request: CheckedEvaluationSnapshotRequest,
    ) -> Result<loop_core::CheckedEvaluationSnapshot, loop_core::PersistenceError> {
        self.inner.load_checked_evaluation_snapshot(request)
    }

    fn load_show_data(
        &self,
        run_id: &loop_core::RunId,
    ) -> Result<loop_core::ShowData, loop_core::PersistenceError> {
        self.inner.load_show_data(run_id)
    }
}

fn assert_only_creation_and_termination(
    adapter: &SqlitePersistence,
    run_id: &str,
) -> Result<Run, Box<dyn std::error::Error>> {
    let run = adapter.load_authoritative_run(&run_id.into())?;
    assert_eq!(run.lifecycle, Lifecycle::Terminated);
    assert_eq!(run.current_state.as_str(), "start");
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    assert!(adapter.load_checked_evaluations(&run_id.into())?.is_empty());
    let history = adapter.load_history(&run_id.into())?;
    assert_eq!(history.len(), 2);
    assert!(matches!(history[0].action, HistoryAction::RunCreated));
    assert!(matches!(history[1].action, HistoryAction::Terminated));
    Ok(run)
}

fn assert_stale_error<T>(outcome: &OperationOutcome<T>) {
    assert!(outcome.is_error(), "stale event must be an operation error");
    assert_eq!(
        outcome.issue().expect("error issue").code,
        "lifecycle-conflict"
    );
}

#[test]
fn competing_check_free_events_share_a_read_point_but_only_one_commits(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("competing-check-free.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-check-free")?;
    drop(setup);

    let first = SqlitePersistence::open(&path)?;
    let second = SqlitePersistence::open(&path)?;
    let gate = Arc::new(Barrier::new(3));
    let first = FirstReadBarrierPersistence::new(first, gate.clone());
    let second = FirstReadBarrierPersistence::new(second, gate.clone());
    let gateway = CountingGateway::default();
    let first_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-check-free", "left"),
            &gateway,
            &first,
        )
    });
    let second_gateway = CountingGateway::default();
    let second_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-check-free", "right"),
            &second_gateway,
            &second,
        )
    });

    // Both authoritative reads are complete before either commit is allowed.
    gate.wait();
    let first_outcome = first_handle.join().expect("first check-free thread");
    let second_outcome = second_handle.join().expect("second check-free thread");
    let completed = [first_outcome.is_completed(), second_outcome.is_completed()]
        .into_iter()
        .filter(|completed| *completed)
        .count();
    assert_eq!(completed, 1, "exactly one check-free event may commit");
    assert_eq!(
        [first_outcome.is_error(), second_outcome.is_error()]
            .into_iter()
            .filter(|error| *error)
            .count(),
        1,
        "the losing check-free event must be a conflict error"
    );

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-check-free".into())?;
    assert!(matches!(run.current_state.as_str(), "left" | "right"));
    assert_eq!(run.lifecycle, Lifecycle::Active);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    let history = adapter.load_history(&"run-check-free".into())?;
    assert_eq!(history.len(), 2);
    assert!(matches!(history[0].action, HistoryAction::RunCreated));
    assert!(matches!(
        history[1].action,
        HistoryAction::Transition {
            outcome: TransitionHistoryOutcome::Committed,
            ..
        }
    ));
    assert!(adapter
        .load_checked_evaluations(&"run-check-free".into())?
        .is_empty());
    Ok(())
}

#[test]
fn competing_checked_allows_share_one_snapshot_revision_but_only_one_control_mutation_commits(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("competing-checked.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-checked")?;
    drop(setup);

    let first = SqlitePersistence::open(&path)?;
    let second = SqlitePersistence::open(&path)?;
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::Allow);
    let gateway = Arc::new(gateway);
    let first_gateway = gateway.clone();
    let first_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-checked", "approve"),
            &*first_gateway,
            &first,
        )
    });
    let second_gateway = gateway.clone();
    let second_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-checked", "alternate"),
            &*second_gateway,
            &second,
        )
    });

    let first_request = ready.recv().expect("first checked snapshot");
    let second_request = ready.recv().expect("second checked snapshot");
    assert!(first_request.transition.kind.is_checked());
    assert!(second_request.transition.kind.is_checked());
    assert_eq!(first_request.transition.source.as_str(), "start");
    assert_eq!(second_request.transition.source.as_str(), "start");
    assert_ne!(
        first_request.transition.event,
        second_request.transition.event
    );
    assert!(
        (first_request.transition == checked_transition()
            && second_request.transition == alternate_checked_transition())
            || (first_request.transition == alternate_checked_transition()
                && second_request.transition == checked_transition())
    );
    assert!(first_request.prior_evaluations.is_empty());
    assert!(second_request.prior_evaluations.is_empty());
    release.send(()).expect("release first evaluation");
    release.send(()).expect("release second evaluation");

    let first_outcome = first_handle.join().expect("first checked thread");
    let second_outcome = second_handle.join().expect("second checked thread");
    assert_eq!(
        [first_outcome.is_completed(), second_outcome.is_completed()]
            .into_iter()
            .filter(|completed| *completed)
            .count(),
        1,
        "only one checked allow may commit from the shared revision"
    );
    assert_eq!(
        [first_outcome.is_error(), second_outcome.is_error()]
            .into_iter()
            .filter(|error| *error)
            .count(),
        1
    );

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-checked".into())?;
    assert!(matches!(run.current_state.as_str(), "done" | "left"));
    assert_eq!(
        run.lifecycle,
        if run.current_state.as_str() == "done" {
            Lifecycle::Final
        } else {
            Lifecycle::Active
        }
    );
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    let history = adapter.load_history(&"run-checked".into())?;
    assert_eq!(history.len(), 2);
    assert!(matches!(
        history[1].action,
        HistoryAction::Transition {
            outcome: TransitionHistoryOutcome::Committed,
            ..
        }
    ));
    let evaluations = adapter.load_checked_evaluations(&"run-checked".into())?;
    assert_eq!(evaluations.len(), 1);
    assert!(evaluations[0].is_allow());
    Ok(())
}

#[test]
fn committed_self_loop_advances_revision_and_stales_in_flight_allow(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("self-loop-stale.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-self-loop")?;
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let self_loop_adapter = SqlitePersistence::open(&path)?;
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::Allow);
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-self-loop", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    let request = ready.recv().expect("checked snapshot reaches provider");
    assert_eq!(request.transition, checked_transition());
    let self_loop = execute_event(
        EventRequest::new("run-self-loop", "self"),
        &CountingGateway::default(),
        &self_loop_adapter,
    );
    assert!(self_loop.is_completed());
    assert_eq!(
        self_loop
            .value()
            .expect("self-loop result")
            .run
            .control_revision
            .as_u64(),
        1
    );
    release.send(()).expect("release in-flight allow");
    let stale = event_handle.join().expect("in-flight event thread");
    assert!(stale.is_error());
    assert_eq!(
        stale.issue().expect("stale issue").code,
        "control-revision-conflict"
    );

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-self-loop".into())?;
    assert_eq!(run.current_state.as_str(), "start");
    assert_eq!(run.lifecycle, Lifecycle::Active);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    let history = adapter.load_history(&"run-self-loop".into())?;
    assert_eq!(history.len(), 2, "stale allow adds no history");
    assert!(matches!(
        history[1].action,
        HistoryAction::Transition {
            ref transition,
            outcome: TransitionHistoryOutcome::Committed,
        } if transition == &check_free_transition("self", "start")
    ));
    assert!(adapter
        .load_checked_evaluations(&"run-self-loop".into())?
        .is_empty());
    Ok(())
}

#[test]
fn termination_between_authoritative_read_and_snapshot_is_rejected(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("termination-before-snapshot.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-before-snapshot")?;
    drop(setup);

    let ready = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let event_persistence = AuthoritativeReadGatePersistence::new(
        SqlitePersistence::open(&path)?,
        ready.clone(),
        release.clone(),
    );
    let terminator = SqlitePersistence::open(&path)?;
    let gateway = CountingGateway::default();
    let gateway_for_assertion = gateway.clone();
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-before-snapshot", "approve"),
            &gateway,
            &event_persistence,
        )
    });

    ready.wait();
    let terminated = execute_terminate(TerminateRequest::new("run-before-snapshot"), &terminator);
    assert!(terminated.is_completed());
    release.wait();
    let outcome = event_handle.join().expect("event thread");
    assert!(outcome.is_rejected());
    assert_eq!(
        outcome.issue().expect("rejection issue").code,
        "run-not-active"
    );
    assert_eq!(
        gateway_for_assertion.count(),
        0,
        "snapshot rejection avoids provider"
    );

    let adapter = SqlitePersistence::open(&path)?;
    assert_only_creation_and_termination(&adapter, "run-before-snapshot")?;
    Ok(())
}

#[test]
fn termination_after_snapshot_during_allow_is_conflict_error_with_no_stale_effect(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("termination-during-allow.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-termination-allow")?;
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let terminator = SqlitePersistence::open(&path)?;
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::Allow);
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-termination-allow", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    let request = ready.recv().expect("snapshot completed before evaluation");
    assert_eq!(request.transition, checked_transition());
    let terminated = execute_terminate(TerminateRequest::new("run-termination-allow"), &terminator);
    assert!(terminated.is_completed());
    release.send(()).expect("release stale allow");
    let outcome = event_handle.join().expect("event thread");
    assert_stale_error(&outcome);

    let adapter = SqlitePersistence::open(&path)?;
    assert_only_creation_and_termination(&adapter, "run-termination-allow")?;
    Ok(())
}

#[test]
fn termination_after_snapshot_during_deny_is_conflict_error_without_denial_lineage(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("termination-during-deny.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-termination-deny")?;
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let terminator = SqlitePersistence::open(&path)?;
    let feedback = EvaluationFeedback::new("needs-work", "revise before approval")
        .with_details(json!({"finding": "test evidence"}));
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::deny(feedback));
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-termination-deny", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    let request = ready.recv().expect("snapshot completed before evaluation");
    assert_eq!(request.transition, checked_transition());
    let terminated = execute_terminate(TerminateRequest::new("run-termination-deny"), &terminator);
    assert!(terminated.is_completed());
    release.send(()).expect("release stale deny");
    let outcome = event_handle.join().expect("event thread");
    assert_stale_error(&outcome);

    let adapter = SqlitePersistence::open(&path)?;
    assert_only_creation_and_termination(&adapter, "run-termination-deny")?;
    Ok(())
}

#[test]
fn context_append_during_evaluation_does_not_invalidate_original_snapshot(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("context-during-evaluation.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-context")?;
    let initial = setup.append_context(append_request("run-context", "before", 2))?;
    assert_eq!(initial.run.control_revision.as_u64(), 0);
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let append_adapter = SqlitePersistence::open(&path)?;
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::Allow);
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-context", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    let request = ready.recv().expect("snapshot completed before evaluation");
    assert_eq!(
        request
            .context
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        vec!["before"]
    );
    let appended = execute_append(
        AppendRequest::new(
            "run-context",
            "during",
            "observation",
            json!({"value": 3}),
            Timestamp::from_unix_millis(3),
        ),
        &append_adapter,
    );
    assert!(appended.is_completed());
    assert_eq!(
        appended
            .value()
            .expect("append result")
            .run
            .control_revision
            .as_u64(),
        0,
        "context append does not advance control revision"
    );
    // The request was captured before the concurrent append and therefore
    // remains the original snapshot even though the append commits now.
    assert_eq!(request.context.len(), 1);
    release.send(()).expect("release allow");
    let outcome = event_handle.join().expect("event thread");
    assert!(outcome.is_completed());

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-context".into())?;
    assert_eq!(run.current_state.as_str(), "done");
    assert_eq!(run.lifecycle, Lifecycle::Final);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 4);
    let context = adapter.load_context_records(&"run-context".into())?;
    assert_eq!(
        context
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        vec!["before", "during"]
    );
    let history = adapter.load_history(&"run-context".into())?;
    assert_eq!(history.len(), 4);
    assert!(matches!(history[0].action, HistoryAction::RunCreated));
    assert!(matches!(
        history[1].action,
        HistoryAction::ContextAppended { .. }
    ));
    assert!(matches!(
        history[2].action,
        HistoryAction::ContextAppended { .. }
    ));
    assert!(matches!(
        history[3].action,
        HistoryAction::Transition {
            outcome: TransitionHistoryOutcome::Committed,
            ..
        }
    ));
    assert_eq!(
        adapter
            .load_checked_evaluations(&"run-context".into())?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn stale_checked_allow_has_no_transition_history_or_lineage(
) -> Result<(), Box<dyn std::error::Error>> {
    // The self-loop scenario above exercises the same stale-allow contract;
    // keep this focused assertion as a direct durable negative check.
    let directory = tempdir()?;
    let path = directory.path().join("stale-allow.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-stale-allow")?;
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let mutation_adapter = SqlitePersistence::open(&path)?;
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::Allow);
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-stale-allow", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    ready.recv().expect("snapshot completed");
    let mutation = execute_event(
        EventRequest::new("run-stale-allow", "self"),
        &CountingGateway::default(),
        &mutation_adapter,
    );
    assert!(mutation.is_completed());
    release.send(()).expect("release stale allow");
    let outcome = event_handle.join().expect("event thread");
    assert!(outcome.is_error());

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-stale-allow".into())?;
    assert_eq!(run.current_state.as_str(), "start");
    assert_eq!(run.lifecycle, Lifecycle::Active);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    let history = adapter.load_history(&"run-stale-allow".into())?;
    assert_eq!(history.len(), 2);
    assert!(adapter
        .load_checked_evaluations(&"run-stale-allow".into())?
        .is_empty());
    Ok(())
}

#[test]
fn stale_checked_deny_has_no_denial_history_or_lineage() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("stale-deny.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    create_run(&setup, "run-stale-deny")?;
    drop(setup);

    let event_adapter = SqlitePersistence::open(&path)?;
    let mutation_adapter = SqlitePersistence::open(&path)?;
    let feedback = EvaluationFeedback::new("blocked", "evidence is incomplete");
    let (gateway, ready, release) = BlockingGateway::new(EvaluationResult::deny(feedback));
    let event_handle = thread::spawn(move || {
        execute_event(
            EventRequest::new("run-stale-deny", "approve"),
            &gateway,
            &event_adapter,
        )
    });

    ready.recv().expect("snapshot completed");
    let mutation = execute_event(
        EventRequest::new("run-stale-deny", "self"),
        &CountingGateway::default(),
        &mutation_adapter,
    );
    assert!(mutation.is_completed());
    release.send(()).expect("release stale deny");
    let outcome = event_handle.join().expect("event thread");
    assert!(outcome.is_error());

    let adapter = SqlitePersistence::open(&path)?;
    let run = adapter.load_authoritative_run(&"run-stale-deny".into())?;
    assert_eq!(run.current_state.as_str(), "start");
    assert_eq!(run.lifecycle, Lifecycle::Active);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 2);
    let history = adapter.load_history(&"run-stale-deny".into())?;
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|entry| {
        !matches!(entry.action, HistoryAction::Transition { ref transition, outcome: TransitionHistoryOutcome::Denied { .. } } if transition.same_lineage(&checked_transition()))
    }));
    assert!(adapter
        .load_checked_evaluations(&"run-stale-deny".into())?
        .is_empty());
    Ok(())
}
