//! `invoke` work-slot delegation.

use super::persistence_error;
use crate::{
    instruction_digest, project_invocation_status, ContextRecord, CreateWorkSlotInvocationRequest,
    InvocationId, OperationOutcome, Persistence, ProcessError, ProjectedInvocationStatus, RunId,
    Timestamp, WaiterSpawnArgs, WorkSlotBinding, WorkSlotId, WorkSlotProcess,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

const WORK_SLOT_BINDINGS_KEY: &str = "work_slot_bindings";

/// Caller-supplied values needed to invoke a bound work slot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
    pub slot_id: WorkSlotId,
    pub invocation_id: InvocationId,
    pub database: PathBuf,
}

impl Request {
    pub fn new(
        run_id: impl Into<RunId>,
        slot_id: impl Into<WorkSlotId>,
        invocation_id: impl Into<InvocationId>,
        database: impl Into<PathBuf>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            slot_id: slot_id.into(),
            invocation_id: invocation_id.into(),
            database: database.into(),
        }
    }
}

/// Successful `invoke` data returned to the composition root.
///
/// `waiter_pid` is internal and is not part of this caller-facing result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Result {
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub started_at: Timestamp,
    pub allowed_time_ms: u64,
    pub capture_dir: String,
}

#[derive(Serialize)]
struct WorkerPacket {
    run_id: String,
    slot_id: String,
    artifact_root: String,
    instruction_body: String,
    capture_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Vec<ContextRecord>>,
}

#[derive(Serialize)]
struct WaiterEnvelope {
    command: String,
    args: Vec<String>,
    worker_packet: WorkerPacket,
}

/// Execute `invoke` through persistence and the work-slot process port.
///
/// Rejection checks run before spawn. On accept, create `capture_dir` after
/// admission and before waiter spawn. Launch order is then spawn waiter
/// (no waitpid, no stdin yet), read pid, create the running invocation, then
/// write the waiter envelope and detach. Invoke does not spawn the bound worker.
pub fn execute<P, Proc>(
    request: Request,
    persistence: &P,
    process: &Proc,
    now: Timestamp,
    allowed_time_ms: u64,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
    Proc: WorkSlotProcess + ?Sized,
{
    let run = match persistence.load_authoritative_run(&request.run_id) {
        Ok(run) => run,
        Err(error) => return persistence_error(error),
    };

    let Some(slot) = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.id == request.slot_id)
    else {
        return OperationOutcome::rejected(
            "unknown-work-slot",
            format!(
                "work slot `{}` is not in the workflow catalog for run `{}`",
                request.slot_id, request.run_id
            ),
        );
    };

    let Some(binding) = bound_worker(&run.initial_input, &request.slot_id) else {
        return OperationOutcome::rejected(
            "unbound-work-slot",
            format!(
                "work slot `{}` has no frozen work_slot_bindings entry",
                request.slot_id
            ),
        );
    };
    let binding = match binding {
        Ok(binding) => binding,
        Err(message) => {
            return OperationOutcome::rejected("invalid-work-slot-binding", message);
        }
    };

    let invocations = match persistence.load_work_slot_invocations(&request.run_id) {
        Ok(invocations) => invocations,
        Err(error) => return persistence_error(error),
    };
    for record in invocations
        .iter()
        .filter(|record| record.slot_id == request.slot_id)
    {
        let projected =
            project_invocation_status(record, now, process.waiter_alive(record.waiter_pid));
        if projected == ProjectedInvocationStatus::Running {
            return OperationOutcome::rejected(
                "work-slot-already-running",
                format!(
                    "work slot `{}` already has a running invocation",
                    request.slot_id
                ),
            );
        }
    }

    let Some(state) = run
        .workflow
        .states
        .iter()
        .find(|state| state.id == slot.state)
    else {
        return OperationOutcome::error(
            "invalid-run",
            format!(
                "work slot `{}` names state `{}` which is not in the workflow",
                request.slot_id, slot.state
            ),
        );
    };
    let instruction_body = state.instructions.clone();
    let digest = instruction_digest(&instruction_body);

    let subject = match persistence.get_current_slot_subject(&request.run_id, &request.slot_id) {
        Ok(Some(subject)) => subject,
        Ok(None) => {
            return OperationOutcome::rejected(
                "no-current-visit-subject",
                format!(
                    "work slot `{}` has no current visit subject",
                    request.slot_id
                ),
            );
        }
        Err(error) => return persistence_error(error),
    };

    let forwarded_context = if slot.stdin_context_kinds.is_empty() {
        None
    } else {
        match persistence.load_context_records(&request.run_id) {
            Ok(records) => Some(
                records
                    .into_iter()
                    .filter(|record| {
                        slot.stdin_context_kinds
                            .iter()
                            .any(|kind| kind == &record.kind)
                    })
                    .collect(),
            ),
            Err(error) => return persistence_error(error),
        }
    };

    let artifact_root = artifact_root_from_input(&run.initial_input);
    if artifact_root.is_empty() {
        return OperationOutcome::error(
            "capture-directory-failed",
            "cannot allocate capture_dir because artifact_root is empty",
        );
    }
    let capture_dir_path =
        capture_dir_path(&artifact_root, &request.slot_id, &request.invocation_id);
    let capture_dir = capture_dir_path.to_string_lossy().into_owned();
    if let Err(error) = std::fs::create_dir_all(&capture_dir_path) {
        return OperationOutcome::error(
            "capture-directory-failed",
            format!("could not create capture directory `{capture_dir}`: {error}"),
        );
    }

    let waiter = match process.spawn_wait_invocation(WaiterSpawnArgs::new(
        request.database.clone(),
        request.run_id.clone(),
        request.invocation_id.clone(),
    )) {
        Ok(waiter) => waiter,
        Err(error) => return process_error(error),
    };

    let create = CreateWorkSlotInvocationRequest::new(
        request.run_id.clone(),
        request.invocation_id.clone(),
        request.slot_id.clone(),
        binding.clone(),
        digest,
        subject,
        waiter.pid,
        now,
        allowed_time_ms,
        capture_dir.clone(),
    );
    if let Err(error) = persistence.create_work_slot_invocation(create) {
        return persistence_error(error);
    }

    let envelope = WaiterEnvelope {
        command: binding.command,
        args: binding.args,
        worker_packet: WorkerPacket {
            run_id: request.run_id.as_str().to_owned(),
            slot_id: request.slot_id.as_str().to_owned(),
            artifact_root,
            instruction_body,
            capture_dir: capture_dir.clone(),
            context: forwarded_context,
        },
    };
    let envelope_json = match serde_json::to_vec(&envelope) {
        Ok(bytes) => bytes,
        Err(error) => {
            return OperationOutcome::error(
                "waiter-envelope-serialization-failed",
                format!("could not serialize waiter envelope: {error}"),
            );
        }
    };
    if let Err(error) = process.send_envelope_and_detach(waiter, &envelope_json) {
        return process_error(error);
    }

    OperationOutcome::completed(Result {
        invocation_id: request.invocation_id,
        slot_id: request.slot_id,
        started_at: now,
        allowed_time_ms,
        capture_dir,
    })
}

fn bound_worker(
    initial_input: &Value,
    slot_id: &WorkSlotId,
) -> Option<std::result::Result<WorkSlotBinding, String>> {
    let Value::Object(map) = initial_input else {
        return None;
    };
    let bindings_value = map.get(WORK_SLOT_BINDINGS_KEY)?;
    let Value::Object(bindings) = bindings_value else {
        return None;
    };
    let binding = bindings.get(slot_id.as_str())?;
    Some(
        serde_json::from_value::<WorkSlotBinding>(binding.clone()).map_err(|error| {
            format!(
                "work_slot_bindings[{slot_id}] must be an object with exactly {{command, args}}: {error}"
            )
        }),
    )
}

fn artifact_root_from_input(initial_input: &Value) -> String {
    initial_input
        .get("artifact_root")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn capture_dir_path(
    artifact_root: &str,
    slot_id: &WorkSlotId,
    invocation_id: &InvocationId,
) -> PathBuf {
    PathBuf::from(artifact_root)
        .join("work-slot-captures")
        .join(slot_id.as_str())
        .join(invocation_id.as_str())
}

fn process_error<T>(error: ProcessError) -> OperationOutcome<T> {
    OperationOutcome::error_with_issue(crate::OutcomeIssue::new(error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
        CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
        CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult, ContextRecord,
        CreateRunRequest, CreateRunResult, CreateWorkSlotInvocationResult, HistoryEntry, Lifecycle,
        PersistenceError, PersistenceFailure, ProviderAssociation, RecordDenialRequest,
        RecordDenialResult, Run, RunSummary, ShowData, StartedWaiter, State, TerminateRequest,
        TerminateResult, Transition, WaiterWrittenStatus, WorkSlot, WorkSlotInvocation, Workflow,
    };
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeSet, HashMap};
    use std::rc::Rc;

    type CallLog = Rc<RefCell<Vec<&'static str>>>;

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do the slot work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        )
        .with_work_slots(vec![WorkSlot::new("slot-1", "start", "finish")])
    }

    fn bound_input(artifact_root: &str) -> Value {
        json!({
            "artifact_root": artifact_root,
            "work_slot_bindings": {
                "slot-1": {"command": "echo", "args": ["hello"]}
            }
        })
    }

    fn expected_capture_dir(artifact_root: &str, slot_id: &str, invocation_id: &str) -> String {
        PathBuf::from(artifact_root)
            .join("work-slot-captures")
            .join(slot_id)
            .join(invocation_id)
            .to_string_lossy()
            .into_owned()
    }

    fn sample_run(initial_input: Value) -> Run {
        Run::new(
            "run-1",
            None,
            workflow(),
            ProviderAssociation::new(json!({"provider": "fake"})),
            initial_input,
            "start",
            Lifecycle::Active,
            0_u64.into(),
            1_u64.into(),
            Timestamp::from_unix_millis(10),
        )
    }

    fn invoke_request() -> Request {
        Request::new("run-1", "slot-1", "inv-1", "/tmp/loop.db")
    }

    fn invocation(
        slot_id: &str,
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        waiter_pid: u32,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            "inv-existing",
            slot_id,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()]),
            "digest",
            "subject-existing",
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

    fn unavailable<T>() -> std::result::Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "fake-failure",
            "fake persistence failure",
        )))
    }

    struct FakePersistence {
        run: RefCell<std::result::Result<Run, PersistenceError>>,
        invocations: RefCell<Vec<WorkSlotInvocation>>,
        created: RefCell<Vec<CreateWorkSlotInvocationRequest>>,
        subjects: RefCell<HashMap<(String, String), String>>,
        set_subject_calls: RefCell<Vec<(String, String, String)>>,
        context_records: RefCell<Vec<ContextRecord>>,
        log: CallLog,
    }

    impl FakePersistence {
        fn new(run: Run, log: CallLog) -> Self {
            Self {
                run: RefCell::new(Ok(run)),
                invocations: RefCell::new(Vec::new()),
                created: RefCell::new(Vec::new()),
                subjects: RefCell::new(HashMap::new()),
                set_subject_calls: RefCell::new(Vec::new()),
                context_records: RefCell::new(Vec::new()),
                log,
            }
        }

        fn with_subject(self, slot_id: &str, subject: &str) -> Self {
            self.subjects
                .borrow_mut()
                .insert(("run-1".to_owned(), slot_id.to_owned()), subject.to_owned());
            self
        }

        fn with_invocations(self, invocations: Vec<WorkSlotInvocation>) -> Self {
            *self.invocations.borrow_mut() = invocations;
            self
        }

        fn with_context_records(self, records: Vec<ContextRecord>) -> Self {
            *self.context_records.borrow_mut() = records;
            self
        }
    }

    impl Persistence for FakePersistence {
        fn create_run(
            &self,
            _request: CreateRunRequest,
        ) -> std::result::Result<CreateRunResult, PersistenceError> {
            unavailable()
        }

        fn append_context(
            &self,
            _request: AppendContextRequest,
        ) -> std::result::Result<AppendContextResult, PersistenceError> {
            unavailable()
        }

        fn commit_transition(
            &self,
            _request: CommitTransitionRequest,
        ) -> std::result::Result<CommitTransitionResult, PersistenceError> {
            unavailable()
        }

        fn record_denial(
            &self,
            _request: RecordDenialRequest,
        ) -> std::result::Result<RecordDenialResult, PersistenceError> {
            unavailable()
        }

        fn terminate(
            &self,
            _request: TerminateRequest,
        ) -> std::result::Result<TerminateResult, PersistenceError> {
            unavailable()
        }

        fn load_authoritative_run(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Run, PersistenceError> {
            match &*self.run.borrow() {
                Ok(run) => Ok(run.clone()),
                Err(error) => Err(error.clone()),
            }
        }

        fn list_runs(&self) -> std::result::Result<Vec<RunSummary>, PersistenceError> {
            unavailable()
        }

        fn load_context_records(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<ContextRecord>, PersistenceError> {
            Ok(self.context_records.borrow().clone())
        }

        fn load_history(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<HistoryEntry>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<crate::DurableEvaluation>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluation_snapshot(
            &self,
            _request: CheckedEvaluationSnapshotRequest,
        ) -> std::result::Result<CheckedEvaluationSnapshot, PersistenceError> {
            unavailable()
        }

        fn load_show_data(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<ShowData, PersistenceError> {
            unavailable()
        }

        fn create_work_slot_invocation(
            &self,
            request: CreateWorkSlotInvocationRequest,
        ) -> std::result::Result<CreateWorkSlotInvocationResult, PersistenceError> {
            self.log.borrow_mut().push("create");
            self.created.borrow_mut().push(request.clone());
            Ok(CreateWorkSlotInvocationResult {
                invocation: WorkSlotInvocation::new(
                    request.invocation_id.clone(),
                    request.slot_id.clone(),
                    request.binding.clone(),
                    request.instruction_digest.clone(),
                    request.subject.clone(),
                    request.waiter_pid,
                    request.started_at,
                    request.allowed_time_ms,
                    None,
                    None,
                    None,
                    request.capture_dir.clone(),
                    Vec::new(),
                ),
                history: HistoryEntry::invocation_started(
                    2_u64.into(),
                    request.started_at,
                    request.invocation_id,
                ),
            })
        }

        fn complete_work_slot_invocation(
            &self,
            _request: CompleteWorkSlotInvocationRequest,
        ) -> std::result::Result<CompleteWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn get_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &WorkSlotId,
        ) -> std::result::Result<Option<String>, PersistenceError> {
            Ok(self
                .subjects
                .borrow()
                .get(&(run_id.as_str().to_owned(), slot_id.as_str().to_owned()))
                .cloned())
        }

        fn set_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &WorkSlotId,
            subject: String,
        ) -> std::result::Result<(), PersistenceError> {
            self.set_subject_calls.borrow_mut().push((
                run_id.as_str().to_owned(),
                slot_id.as_str().to_owned(),
                subject.clone(),
            ));
            self.subjects.borrow_mut().insert(
                (run_id.as_str().to_owned(), slot_id.as_str().to_owned()),
                subject,
            );
            Ok(())
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<WorkSlotInvocation>, PersistenceError> {
            Ok(self.invocations.borrow().clone())
        }
    }

    struct FakeProcess {
        log: CallLog,
        spawn_args: RefCell<Vec<WaiterSpawnArgs>>,
        envelopes: RefCell<Vec<Vec<u8>>>,
        next_pid: Cell<u32>,
        alive: RefCell<HashMap<u32, bool>>,
        default_alive: bool,
        waited: Cell<bool>,
    }

    impl FakeProcess {
        fn new(log: CallLog) -> Self {
            Self {
                log,
                spawn_args: RefCell::new(Vec::new()),
                envelopes: RefCell::new(Vec::new()),
                next_pid: Cell::new(4242),
                alive: RefCell::new(HashMap::new()),
                default_alive: true,
                waited: Cell::new(false),
            }
        }

        fn set_alive(&self, pid: u32, alive: bool) {
            self.alive.borrow_mut().insert(pid, alive);
        }
    }

    impl WorkSlotProcess for FakeProcess {
        type Handle = ();

        fn waiter_alive(&self, pid: u32) -> bool {
            self.alive
                .borrow()
                .get(&pid)
                .copied()
                .unwrap_or(self.default_alive)
        }

        fn spawn_wait_invocation(
            &self,
            args: WaiterSpawnArgs,
        ) -> std::result::Result<StartedWaiter<()>, ProcessError> {
            self.log.borrow_mut().push("spawn");
            self.spawn_args.borrow_mut().push(args);
            Ok(StartedWaiter::new(self.next_pid.get(), ()))
        }

        fn send_envelope_and_detach(
            &self,
            _waiter: StartedWaiter<()>,
            envelope_json: &[u8],
        ) -> std::result::Result<(), ProcessError> {
            self.log.borrow_mut().push("send");
            self.envelopes.borrow_mut().push(envelope_json.to_vec());
            Ok(())
        }
    }

    fn harness(run: Run) -> (FakePersistence, FakeProcess, CallLog) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let persistence = FakePersistence::new(run, log.clone());
        let process = FakeProcess::new(log.clone());
        (persistence, process, log)
    }

    #[test]
    fn unknown_slot_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let request = Request::new("run-1", "missing-slot", "inv-1", "/tmp/loop.db");

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "unknown-work-slot");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
        assert!(!process.waited.get());
    }

    #[test]
    fn unbound_missing_empty_or_omitted_slot_is_rejected_without_spawn() {
        let inputs = [
            json!({"artifact_root": "/tmp/artifacts"}),
            json!({"artifact_root": "/tmp/artifacts", "work_slot_bindings": {}}),
            json!({
                "artifact_root": "/tmp/artifacts",
                "work_slot_bindings": {
                    "other-slot": {"command": "echo", "args": []}
                }
            }),
        ];
        for input in inputs {
            let (persistence, process, log) = harness(sample_run(input));
            let persistence = persistence.with_subject("slot-1", "visit-1");

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );

            assert!(outcome.is_rejected(), "{outcome:?}");
            assert_eq!(outcome.issue().unwrap().code, "unbound-work-slot");
            assert!(process.spawn_args.borrow().is_empty());
            assert!(persistence.created.borrow().is_empty());
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn overlay_running_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_invocations(vec![invocation("slot-1", 1_000, 5_000, None, 42)]);
        process.set_alive(42, true);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(2_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "work-slot-already-running");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn overlay_overrun_is_not_already_running_and_invoke_is_accepted() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_invocations(vec![invocation("slot-1", 1_000, 5_000, None, 42)]);
        process.set_alive(42, true);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(6_000),
            30_000,
        );

        assert!(outcome.is_completed(), "{outcome:?}");
        assert_eq!(&*log.borrow(), &["spawn", "create", "send"]);
        assert!(!process.waited.get());
    }

    #[test]
    fn overlay_failed_and_succeeded_are_not_already_running() {
        for status in [
            Some(WaiterWrittenStatus::Failed),
            Some(WaiterWrittenStatus::Succeeded),
        ] {
            let artifacts = tempfile::tempdir().expect("temp artifact root");
            let artifact_root = artifacts.path().to_string_lossy().into_owned();
            let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
            let persistence = persistence
                .with_subject("slot-1", "visit-1")
                .with_invocations(vec![invocation("slot-1", 1_000, 5_000, status, 42)]);
            process.set_alive(42, true);

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(2_000),
                30_000,
            );

            assert!(outcome.is_completed(), "{status:?} {outcome:?}");
            assert_eq!(&*log.borrow(), &["spawn", "create", "send"]);
            assert!(!process.waited.get());
        }
    }

    #[test]
    fn happy_path_creates_invocation_then_sends_envelope_without_waitpid() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let expected_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-1");
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let now = Timestamp::from_unix_millis(1_000);
        let allowed_time_ms = 12_345;

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            now,
            allowed_time_ms,
        );

        assert!(outcome.is_completed(), "{outcome:?}");
        let result = outcome.value().expect("completed invoke");
        assert_eq!(result.invocation_id.as_str(), "inv-1");
        assert_eq!(result.slot_id.as_str(), "slot-1");
        assert_eq!(result.started_at, now);
        assert_eq!(result.allowed_time_ms, allowed_time_ms);
        assert_eq!(result.capture_dir, expected_dir);
        assert!(
            PathBuf::from(&result.capture_dir).is_dir(),
            "capture_dir should exist: {}",
            result.capture_dir
        );
        assert_eq!(&*log.borrow(), &["spawn", "create", "send"]);
        assert!(!process.waited.get());
        assert!(persistence.set_subject_calls.borrow().is_empty());

        let created = persistence.created.borrow();
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].instruction_digest,
            instruction_digest("Do the slot work")
        );
        assert_eq!(created[0].subject, "visit-1");
        assert_eq!(created[0].allowed_time_ms, allowed_time_ms);
        assert_eq!(created[0].waiter_pid, 4242);
        assert_eq!(created[0].started_at, now);
        assert_eq!(created[0].capture_dir, expected_dir);
        assert_eq!(
            created[0].binding,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()])
        );

        let spawn = &process.spawn_args.borrow()[0];
        assert_eq!(spawn.run_id.as_str(), "run-1");
        assert_eq!(spawn.invocation_id.as_str(), "inv-1");
        assert_eq!(spawn.database, PathBuf::from("/tmp/loop.db"));

        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope json");
        assert_eq!(envelope["command"], "echo");
        assert_eq!(envelope["args"], json!(["hello"]));
        let packet = envelope["worker_packet"]
            .as_object()
            .expect("worker_packet object");
        let keys = packet
            .keys()
            .map(|key| key.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "run_id",
                "slot_id",
                "artifact_root",
                "instruction_body",
                "capture_dir",
            ])
        );
        assert_eq!(packet["run_id"], "run-1");
        assert_eq!(packet["slot_id"], "slot-1");
        assert_eq!(packet["artifact_root"], artifact_root);
        assert_eq!(packet["instruction_body"], "Do the slot work");
        assert_eq!(packet["capture_dir"], expected_dir);
        assert!(packet.get("command").is_none());
    }

    #[test]
    fn empty_artifact_root_errors_before_spawn() {
        let inputs = [
            json!({
                "work_slot_bindings": {
                    "slot-1": {"command": "echo", "args": ["hello"]}
                }
            }),
            json!({
                "artifact_root": "",
                "work_slot_bindings": {
                    "slot-1": {"command": "echo", "args": ["hello"]}
                }
            }),
        ];
        for input in inputs {
            let (persistence, process, log) = harness(sample_run(input));
            let persistence = persistence.with_subject("slot-1", "visit-1");

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );

            assert!(outcome.is_error(), "{outcome:?}");
            assert_eq!(outcome.issue().unwrap().code, "capture-directory-failed");
            assert!(process.spawn_args.borrow().is_empty());
            assert!(persistence.created.borrow().is_empty());
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn capture_dir_create_failure_errors_without_spawn() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let blocker = PathBuf::from(&artifact_root)
            .join("work-slot-captures")
            .join("slot-1");
        std::fs::create_dir_all(blocker.parent().expect("parent")).expect("create parent");
        std::fs::write(&blocker, b"not a directory").expect("write blocker file");

        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_error(), "{outcome:?}");
        assert_eq!(outcome.issue().unwrap().code, "capture-directory-failed");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn second_invoke_uses_a_distinct_capture_dir_and_leaves_the_first() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let first_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-1");
        let second_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-2");
        let (persistence, process, _log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");

        let first = execute(
            Request::new("run-1", "slot-1", "inv-1", "/tmp/loop.db"),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(first.is_completed(), "{first:?}");
        let first_result = first.value().expect("first invoke");
        assert_eq!(first_result.capture_dir, first_dir);
        assert!(PathBuf::from(&first_dir).is_dir());
        std::fs::write(PathBuf::from(&first_dir).join("marker.txt"), b"keep").expect("marker");

        let second = execute(
            Request::new("run-1", "slot-1", "inv-2", "/tmp/loop.db"),
            &persistence,
            &process,
            Timestamp::from_unix_millis(2_000),
            30_000,
        );
        assert!(second.is_completed(), "{second:?}");
        let second_result = second.value().expect("second invoke");
        assert_eq!(second_result.capture_dir, second_dir);
        assert_ne!(second_result.capture_dir, first_result.capture_dir);
        assert!(PathBuf::from(&second_dir).is_dir());
        assert!(PathBuf::from(&first_dir).is_dir());
        assert_eq!(
            std::fs::read(PathBuf::from(&first_dir).join("marker.txt")).expect("read marker"),
            b"keep"
        );
        assert_eq!(persistence.created.borrow()[0].capture_dir, first_dir);
        assert_eq!(persistence.created.borrow()[1].capture_dir, second_dir);
    }

    #[test]
    fn missing_current_subject_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "no-current-visit-subject");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    fn run_with_slot(artifact_root: &str, slot: WorkSlot) -> Run {
        let mut run = sample_run(bound_input(artifact_root));
        run.workflow.work_slots = vec![slot];
        run
    }

    fn record(id: &str, kind: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            crate::SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn worker_packet(process: &FakeProcess) -> Value {
        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope json");
        envelope["worker_packet"].clone()
    }

    #[test]
    fn omitted_and_empty_stdin_context_kinds_keep_five_key_packet() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let slots = [
            WorkSlot::new("slot-1", "start", "finish"),
            WorkSlot::new("slot-1", "start", "finish").with_stdin_context_kinds(Vec::new()),
        ];
        for slot in slots {
            let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
            let persistence = persistence
                .with_subject("slot-1", "visit-1")
                .with_context_records(vec![record(
                    "ctx-1",
                    "kind-a",
                    1,
                    json!({"payload": "stored"}),
                )]);

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );
            assert!(outcome.is_completed(), "{outcome:?}");
            let packet = worker_packet(&process);
            let object = packet.as_object().expect("packet");
            assert!(object.get("context").is_none());
            let keys = object
                .keys()
                .map(|key| key.as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                keys,
                BTreeSet::from([
                    "run_id",
                    "slot_id",
                    "artifact_root",
                    "instruction_body",
                    "capture_dir",
                ])
            );
        }
    }

    #[test]
    fn work_slot_omitting_stdin_context_kinds_round_trips_without_the_key() {
        let omitted = json!({
            "id": "slot-1",
            "state": "start",
            "event": "finish"
        });
        let slot: WorkSlot = serde_json::from_value(omitted).unwrap();
        assert_eq!(slot, WorkSlot::new("slot-1", "start", "finish"));
        assert!(slot.stdin_context_kinds.is_empty());
        let encoded = serde_json::to_value(&slot).unwrap();
        assert!(encoded.get("stdin_context_kinds").is_none());
        assert_eq!(
            encoded,
            json!({
                "id": "slot-1",
                "state": "start",
                "event": "finish"
            })
        );

        let empty = WorkSlot::new("slot-1", "start", "finish").with_stdin_context_kinds(Vec::new());
        assert!(serde_json::to_value(&empty)
            .unwrap()
            .get("stdin_context_kinds")
            .is_none());
    }

    #[test]
    fn nonempty_stdin_context_kinds_forwards_matching_records_in_append_order() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let older = record(
            "ctx-old",
            "kind-a",
            1,
            json!({"rev": "1", "note": "historical"}),
        );
        let other_kind = record("ctx-other", "kind-b", 2, json!({"rev": "skip"}));
        let between = record("ctx-mid", "kind-c", 3, json!({"rev": "mid"}));
        let newer = record(
            "ctx-new",
            "kind-a",
            4,
            json!({"rev": "2", "note": "current"}),
        );
        let slot = WorkSlot::new("slot-1", "start", "finish")
            .with_stdin_context_kinds(vec!["kind-a".to_owned(), "kind-c".to_owned()]);
        let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_context_records(vec![
                older.clone(),
                other_kind,
                between.clone(),
                newer.clone(),
            ]);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        let packet = worker_packet(&process);
        let forwarded = packet["context"].as_array().expect("context array");
        assert_eq!(forwarded.len(), 3);
        assert_eq!(forwarded[0], serde_json::to_value(&older).unwrap());
        assert_eq!(forwarded[1], serde_json::to_value(&between).unwrap());
        assert_eq!(forwarded[2], serde_json::to_value(&newer).unwrap());
        assert_eq!(
            forwarded[0]["data"],
            json!({"rev": "1", "note": "historical"})
        );
        assert_eq!(forwarded[2]["data"], json!({"rev": "2", "note": "current"}));
    }

    #[test]
    fn nonempty_stdin_context_kinds_with_no_matches_still_emits_context_key() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let slot = WorkSlot::new("slot-1", "start", "finish")
            .with_stdin_context_kinds(vec!["kind-a".to_owned()]);
        let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_context_records(vec![record("ctx-other", "kind-b", 1, json!({}))]);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        let packet = worker_packet(&process);
        assert_eq!(packet["context"], json!([]));
    }

    #[test]
    fn review_slot_forwards_accepted_findings_and_draft_slot_omits_context() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let draft = WorkSlot::new("intent-draft", "explore", "intent-ready");
        let review = WorkSlot::new("intent-review", "intent-review", "approved")
            .with_stdin_context_kinds(vec!["accepted-findings".to_owned()]);
        let findings = record(
            "accepted-1",
            "accepted-findings",
            1,
            json!({
                "gate": "intent-review",
                "subject": "intent.json",
                "subject_revision": "1",
                "findings": []
            }),
        );
        let evidence = record(
            "evidence-1",
            "review-evidence",
            2,
            json!({"gate": "intent-review", "policy_id": "axis", "result": "pass"}),
        );
        let mut run = sample_run(json!({
            "artifact_root": artifact_root,
            "work_slot_bindings": {
                "intent-draft": {"command": "echo", "args": ["draft"]},
                "intent-review": {"command": "echo", "args": ["review"]}
            }
        }));
        run.workflow = Workflow::new(
            "workflow",
            "explore",
            vec![
                State::new("explore", "Explore", "Draft", false),
                State::new("intent-review", "Intent review", "Review", false),
                State::new("end", "End", "Done", true),
            ],
            vec![
                Transition::checked("explore", "intent-ready", "intent-review"),
                Transition::checked("intent-review", "approved", "end"),
            ],
        )
        .with_work_slots(vec![draft, review]);

        let (draft_persistence, draft_process, _draft_log) = harness(run.clone());
        let draft_persistence = draft_persistence
            .with_subject("intent-draft", "visit-draft")
            .with_context_records(vec![findings.clone(), evidence.clone()]);
        let draft_outcome = execute(
            Request::new("run-1", "intent-draft", "inv-draft", "/tmp/loop.db"),
            &draft_persistence,
            &draft_process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(draft_outcome.is_completed(), "{draft_outcome:?}");
        let draft_packet = worker_packet(&draft_process);
        assert_eq!(draft_packet["slot_id"], "intent-draft");
        assert!(draft_packet.get("context").is_none());
        let draft_keys = draft_packet
            .as_object()
            .expect("packet")
            .keys()
            .map(|key| key.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            draft_keys,
            BTreeSet::from([
                "run_id",
                "slot_id",
                "artifact_root",
                "instruction_body",
                "capture_dir",
            ])
        );

        let (review_persistence, review_process, _review_log) = harness(run);
        let review_persistence = review_persistence
            .with_subject("intent-review", "visit-review")
            .with_context_records(vec![findings.clone(), evidence]);
        let review_outcome = execute(
            Request::new("run-1", "intent-review", "inv-review", "/tmp/loop.db"),
            &review_persistence,
            &review_process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(review_outcome.is_completed(), "{review_outcome:?}");
        let review_packet = worker_packet(&review_process);
        assert_eq!(review_packet["slot_id"], "intent-review");
        let forwarded = review_packet["context"].as_array().expect("context");
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0], serde_json::to_value(&findings).unwrap());
    }
}
