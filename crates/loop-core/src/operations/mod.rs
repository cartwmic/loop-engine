//! Semantic application operations for durable workflow runs.
//!
//! These use cases coordinate provider ports and semantic persistence ports,
//! while keeping outcome classification in `loop-core`.  Integrations own
//! atomicity and backend mechanics; operations only construct the semantic
//! requests described by the ports.

pub mod append;
pub mod evaluation;
pub mod event;
pub mod history;
pub mod invoke;
pub mod list;
pub mod show;
pub mod start;
pub mod terminate;

pub use crate::invocation::{instruction_digest, project_invocation_status};
/// Start-facing catalog and frozen-binding contract types.
pub use crate::{WorkSlot, WorkSlotBinding, WorkSlotId};
pub use append::{execute as execute_append, Request as AppendRequest};
pub use evaluation::{lineage_for_transition, request_from_snapshot};
pub use event::{execute as execute_event, Request as EventRequest, Result as EventResult};
pub use history::{execute as execute_history, Request as HistoryRequest};
pub use invoke::{execute as execute_invoke, Request as InvokeRequest, Result as InvokeResult};
pub use list::{execute as execute_list, Request as ListRequest};
pub use show::{
    execute as execute_show, latest_evaluations, project as project_show, ProjectionError,
    Request as ShowRequest, RequestableEvent, RunChangeReport, ShowProjection,
    WorkSlotInvocationView,
};
pub use start::{execute as execute_start, Request as StartRequest};
pub use terminate::{execute as execute_terminate, Request as TerminateRunRequest};

use crate::{
    ControlRevision, OperationOutcome, OutcomeIssue, Persistence, PersistenceError, ProviderError,
    ProviderResolutionError, RunId, WorkflowValidationError,
};

/// Classify a provider-resolution failure as an operation error.
pub(crate) fn provider_resolution_error<T>(error: ProviderResolutionError) -> OperationOutcome<T> {
    OperationOutcome::error_with_issue(OutcomeIssue::new(error.code(), error.to_string()))
}

/// Classify a provider invocation failure as an operation error.
pub(crate) fn provider_error<T>(error: ProviderError) -> OperationOutcome<T> {
    OperationOutcome::error_with_issue(OutcomeIssue::new(error.code(), error.to_string()))
}

/// Classify a malformed provider-described workflow as an operation error.
pub(crate) fn workflow_error<T>(error: WorkflowValidationError) -> OperationOutcome<T> {
    OperationOutcome::error_with_issue(OutcomeIssue::new(error.code(), error.to_string()))
}

/// Classify persistence failures without losing the distinction between a
/// semantic rejection and an operation error.
///
/// Persistence owns lifecycle and conditional-write checks.  A rejected
/// request is therefore surfaced as `Rejected`; missing runs, conflicts, and
/// adapter failures are all operation `Error`s.
pub(crate) fn require_current_observation<P, T>(
    persistence: &P,
    run_id: &RunId,
    control_revision: ControlRevision,
) -> Result<(), OperationOutcome<T>>
where
    P: Persistence + ?Sized,
{
    match persistence.observation_is_current(run_id, control_revision) {
        Ok(true) => Ok(()),
        Ok(false) => Err(OperationOutcome::rejected(
            "run-not-observed",
            format!("run `{run_id}` must be observed with `show` before it can be mutated"),
        )),
        Err(error) => Err(persistence_error(error)),
    }
}

pub(crate) fn persistence_error<T>(error: PersistenceError) -> OperationOutcome<T> {
    let rejected = error.is_rejected();
    let code = error.code().to_owned();
    let message = error.to_string();
    let mut issue = OutcomeIssue::new(code, message);

    if let PersistenceError::Failure(failure) = &error {
        if let Some(details) = &failure.details {
            issue = issue.with_details(details.clone());
        }
    }

    if rejected {
        OperationOutcome::rejected_with_issue(issue)
    } else {
        OperationOutcome::error_with_issue(issue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppendContextRequest, AppendContextResult, ContextRecord, ContextRecordId,
        CreateRunRequest, CreateRunResult, HistoryAction, HistoryEntry, Lifecycle, Persistence,
        PersistenceFailure, PersistenceRejection, ProviderAssociation, ProviderGateway,
        ProviderResolver, Run, RunId, RunSummary, SemanticSequence, State, StateId,
        TerminateRequest, TerminateResult, Timestamp, Transition, WorkSlot, Workflow,
    };
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    fn workflow(initial_final: bool) -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do work", initial_final),
                State::new("done", "Done", "Finished", true),
            ],
            if initial_final {
                vec![]
            } else {
                vec![Transition::check_free("start", "finish", "done")]
            },
        )
    }

    fn run_from(request: &CreateRunRequest) -> Run {
        Run::new(
            request.id.clone(),
            request.label.clone(),
            request.workflow.clone(),
            request.provider_association.clone(),
            request.initial_input.clone(),
            request.initial_state.clone(),
            request.lifecycle,
            0_u64.into(),
            1_u64.into(),
            request.created_at,
        )
    }

    #[derive(Default)]
    struct FakeResolver {
        result: Option<Result<ProviderAssociation, ProviderResolutionError>>,
        calls: RefCell<usize>,
    }

    impl ProviderResolver for FakeResolver {
        fn resolve(
            &self,
            _selector: &crate::ProviderSelector,
        ) -> Result<ProviderAssociation, ProviderResolutionError> {
            *self.calls.borrow_mut() += 1;
            self.result
                .clone()
                .unwrap_or_else(|| Ok(ProviderAssociation::new(json!({"provider": "fake"}))))
        }
    }

    #[derive(Default)]
    struct FakeGateway {
        described: Option<Result<Workflow, ProviderError>>,
        describe_calls: RefCell<usize>,
        describe_inputs: RefCell<Vec<Option<Value>>>,
        describe_from_input: Option<fn(Option<&Value>) -> Workflow>,
    }

    impl ProviderGateway for FakeGateway {
        fn describe(
            &self,
            _provider: &ProviderAssociation,
            initial_input: Option<&Value>,
        ) -> Result<Workflow, ProviderError> {
            *self.describe_calls.borrow_mut() += 1;
            self.describe_inputs
                .borrow_mut()
                .push(initial_input.cloned());
            if let Some(describe_from_input) = self.describe_from_input {
                return Ok(describe_from_input(initial_input));
            }
            self.described
                .clone()
                .unwrap_or_else(|| Ok(workflow(false)))
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            _request: crate::EvaluationRequest,
        ) -> Result<crate::EvaluationResult, ProviderError> {
            Ok(crate::EvaluationResult::Unsupported)
        }
    }

    #[derive(Default)]
    struct FakePersistence {
        created: RefCell<Vec<CreateRunRequest>>,
        appended: RefCell<Vec<AppendContextRequest>>,
        terminated: RefCell<Vec<TerminateRequest>>,
        create_result: RefCell<Option<Result<CreateRunResult, PersistenceError>>>,
        append_result: RefCell<Option<Result<AppendContextResult, PersistenceError>>>,
        list_result: RefCell<Option<Result<Vec<RunSummary>, PersistenceError>>>,
        history_result: RefCell<Option<Result<Vec<HistoryEntry>, PersistenceError>>>,
        terminate_result: RefCell<Option<Result<TerminateResult, PersistenceError>>>,
    }

    fn unavailable<T>() -> Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "fake-failure",
            "fake persistence failure",
        )))
    }

    impl Persistence for FakePersistence {
        fn create_run(
            &self,
            request: CreateRunRequest,
        ) -> Result<CreateRunResult, PersistenceError> {
            self.created.borrow_mut().push(request.clone());
            self.create_result.borrow_mut().take().unwrap_or_else(|| {
                Ok(CreateRunResult {
                    run: run_from(&request),
                    history: HistoryEntry::run_created(1_u64.into(), request.created_at),
                })
            })
        }

        fn append_context(
            &self,
            request: AppendContextRequest,
        ) -> Result<AppendContextResult, PersistenceError> {
            self.appended.borrow_mut().push(request.clone());
            self.append_result
                .borrow_mut()
                .take()
                .unwrap_or_else(unavailable)
        }

        fn commit_transition(
            &self,
            _request: crate::CommitTransitionRequest,
        ) -> Result<crate::CommitTransitionResult, PersistenceError> {
            unavailable()
        }

        fn record_denial(
            &self,
            _request: crate::RecordDenialRequest,
        ) -> Result<crate::RecordDenialResult, PersistenceError> {
            unavailable()
        }

        fn terminate(
            &self,
            request: TerminateRequest,
        ) -> Result<TerminateResult, PersistenceError> {
            self.terminated.borrow_mut().push(request.clone());
            self.terminate_result
                .borrow_mut()
                .take()
                .unwrap_or_else(unavailable)
        }

        fn load_authoritative_run(&self, _run_id: &RunId) -> Result<Run, PersistenceError> {
            unavailable()
        }

        fn list_runs(&self) -> Result<Vec<RunSummary>, PersistenceError> {
            self.list_result
                .borrow_mut()
                .take()
                .unwrap_or_else(unavailable)
        }

        fn load_context_records(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<ContextRecord>, PersistenceError> {
            unavailable()
        }

        fn load_history(&self, _run_id: &RunId) -> Result<Vec<HistoryEntry>, PersistenceError> {
            self.history_result
                .borrow_mut()
                .take()
                .unwrap_or_else(unavailable)
        }

        fn load_checked_evaluations(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<crate::DurableEvaluation>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluation_snapshot(
            &self,
            _request: crate::CheckedEvaluationSnapshotRequest,
        ) -> Result<crate::CheckedEvaluationSnapshot, PersistenceError> {
            unavailable()
        }

        fn load_show_data(&self, _run_id: &RunId) -> Result<crate::ShowData, PersistenceError> {
            unavailable()
        }

        fn create_work_slot_invocation(
            &self,
            _request: crate::CreateWorkSlotInvocationRequest,
        ) -> Result<crate::CreateWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn complete_work_slot_invocation(
            &self,
            _request: crate::CompleteWorkSlotInvocationRequest,
        ) -> Result<crate::CompleteWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn get_current_slot_subject(
            &self,
            _run_id: &RunId,
            _slot_id: &crate::WorkSlotId,
        ) -> Result<Option<String>, PersistenceError> {
            Ok(None)
        }

        fn set_current_slot_subject(
            &self,
            _run_id: &RunId,
            _slot_id: &crate::WorkSlotId,
            _subject: String,
        ) -> Result<(), PersistenceError> {
            Ok(())
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> Result<Vec<crate::WorkSlotInvocation>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    fn start_request(catalog_root: impl Into<PathBuf>) -> start::Request {
        start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test"}),
            Some("example".to_owned()),
            Timestamp::from_unix_millis(10),
            catalog_root,
        )
    }

    fn temp_catalog() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp catalog root")
    }

    fn allocated_canonical(catalog_root: &Path, run_id: &str) -> PathBuf {
        catalog_root
            .join("runs")
            .join(run_id)
            .canonicalize()
            .expect("engine-owned run directory")
    }

    fn allocated_canonical_string(catalog_root: &Path, run_id: &str) -> String {
        allocated_canonical(catalog_root, run_id)
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn start_resolves_describes_validates_and_creates_atomically() {
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = FakeGateway::default();
        let persistence = FakePersistence::default();

        let outcome = start::execute(
            start_request(catalog.path()),
            &resolver,
            &gateway,
            &persistence,
        );

        assert!(outcome.is_completed());
        assert_eq!(*resolver.calls.borrow(), 1);
        assert_eq!(*gateway.describe_calls.borrow(), 1);
        assert_eq!(
            gateway.describe_inputs.borrow().as_slice(),
            [Some(json!({"objective": "test"}))]
        );
        assert_eq!(persistence.created.borrow().len(), 1);
        let request = &persistence.created.borrow()[0];
        assert_eq!(request.workflow, workflow(false));
        let allocated = allocated_canonical_string(catalog.path(), "run-1");
        assert_eq!(request.lifecycle, Lifecycle::Active);
        assert_eq!(request.initial_state, StateId::from("start"));
        assert_eq!(
            request.initial_input,
            json!({"objective": "test", "artifact_root": allocated})
        );
        assert_eq!(request.provider, "fake");
        assert_eq!(request.artifact_root.as_deref(), Some(allocated.as_str()));
        assert!(allocated_canonical(catalog.path(), "run-1").is_dir());
    }

    fn workflow_named(id: &str) -> Workflow {
        Workflow::new(
            id,
            "start",
            vec![
                State::new("start", "Start", "Do work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        )
    }

    fn workflow_from_topology(initial_input: Option<&Value>) -> Workflow {
        let marker = initial_input
            .and_then(Value::as_object)
            .and_then(|map| map.get("topology"))
            .and_then(Value::as_str)
            .unwrap_or("union");
        workflow_named(&format!("workflow-{marker}"))
    }

    #[test]
    fn start_stores_the_workflow_describe_returned() {
        let catalog = temp_catalog();
        let described = workflow_named("described-snapshot");
        let resolver = FakeResolver::default();
        let gateway = FakeGateway {
            described: Some(Ok(described.clone())),
            ..FakeGateway::default()
        };
        let persistence = FakePersistence::default();
        let caller = json!({"objective": "test", "review_policies": {"design-review": ["axis"]}});

        let outcome = start::execute(
            start::Request::new(
                "run-1",
                "fake",
                caller.clone(),
                None,
                Timestamp::from_unix_millis(10),
                catalog.path(),
            ),
            &resolver,
            &gateway,
            &persistence,
        );

        assert!(outcome.is_completed());
        assert_eq!(gateway.describe_inputs.borrow().as_slice(), [Some(caller)]);
        assert_eq!(persistence.created.borrow()[0].workflow, described);
        assert!(persistence.created.borrow()[0]
            .initial_input
            .get("work_slot_bindings")
            .is_none());
    }

    #[test]
    fn start_stores_workflow_when_describe_varies_with_initial_input() {
        let resolver = FakeResolver::default();
        let gateway = FakeGateway {
            describe_from_input: Some(workflow_from_topology),
            ..FakeGateway::default()
        };
        let cases = [
            ("run-union", json!({"objective": "a"}), "workflow-union"),
            (
                "run-live",
                json!({"objective": "a", "topology": "live"}),
                "workflow-live",
            ),
        ];
        for (run_id, caller, expected_id) in cases {
            let catalog = temp_catalog();
            let persistence = FakePersistence::default();
            let outcome = start::execute(
                start::Request::new(
                    run_id,
                    "fake",
                    caller.clone(),
                    None,
                    Timestamp::from_unix_millis(10),
                    catalog.path(),
                ),
                &resolver,
                &gateway,
                &persistence,
            );

            assert!(outcome.is_completed(), "{run_id}");
            assert_eq!(
                persistence.created.borrow()[0].workflow.id.as_str(),
                expected_id,
                "{run_id}"
            );
            assert_eq!(
                persistence.created.borrow()[0].workflow,
                workflow_named(expected_id)
            );
            assert!(persistence.created.borrow()[0]
                .initial_input
                .get("work_slot_bindings")
                .is_none());
        }
        assert_eq!(*gateway.describe_calls.borrow(), 2);
        assert_eq!(
            gateway.describe_inputs.borrow().as_slice(),
            [
                Some(json!({"objective": "a"})),
                Some(json!({"objective": "a", "topology": "live"})),
            ]
        );
    }

    #[test]
    fn start_keeps_nonempty_caller_artifact_root_and_still_creates_allocated_dir() {
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = FakeGateway::default();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test", "artifact_root": "relative/caller-path"}),
            Some("example".to_owned()),
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        assert_eq!(
            created.initial_input,
            json!({"objective": "test", "artifact_root": "relative/caller-path"})
        );
        assert_eq!(
            created.artifact_root.as_deref(),
            Some("relative/caller-path")
        );
        assert_eq!(created.provider, "fake");
        assert!(allocated_canonical(catalog.path(), "run-1").is_dir());
    }

    #[test]
    fn start_treats_empty_null_missing_or_non_string_artifact_root_as_absent() {
        let cases = [
            json!({"objective": "test", "artifact_root": ""}),
            json!({"objective": "test", "artifact_root": null}),
            json!({"objective": "test"}),
            json!({"objective": "test", "artifact_root": 1}),
            json!({"objective": "test", "artifact_root": true}),
            json!({"objective": "test", "artifact_root": []}),
            json!({"objective": "test", "artifact_root": {}}),
        ];
        for (index, initial_input) in cases.into_iter().enumerate() {
            let catalog = temp_catalog();
            let run_id = format!("run-{index}");
            let resolver = FakeResolver::default();
            let gateway = FakeGateway::default();
            let persistence = FakePersistence::default();
            let request = start::Request::new(
                run_id.clone(),
                "fake",
                initial_input,
                None,
                Timestamp::from_unix_millis(10),
                catalog.path(),
            );

            let outcome = start::execute(request, &resolver, &gateway, &persistence);

            assert!(outcome.is_completed(), "case {index}");
            let created = &persistence.created.borrow()[0];
            let allocated = allocated_canonical_string(catalog.path(), &run_id);
            assert_eq!(
                created.initial_input.get("artifact_root"),
                Some(&json!(allocated)),
                "case {index}"
            );
            assert_eq!(
                created.artifact_root.as_deref(),
                Some(allocated.as_str()),
                "case {index}"
            );
            assert_eq!(created.provider, "fake", "case {index}");
            assert!(allocated_canonical(catalog.path(), &run_id).is_dir());
        }
    }

    #[test]
    fn start_does_not_rewrite_non_object_initial_input() {
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = FakeGateway::default();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!("just a string"),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        assert_eq!(created.initial_input, json!("just a string"));
        let allocated = allocated_canonical_string(catalog.path(), "run-1");
        assert_eq!(created.artifact_root.as_deref(), Some(allocated.as_str()));
        assert_eq!(created.provider, "fake");
        assert!(allocated_canonical(catalog.path(), "run-1").is_dir());
    }

    fn gateway_with_explore_intent_slot() -> FakeGateway {
        FakeGateway {
            described: Some(Ok(workflow(false).with_work_slots(vec![WorkSlot::new(
                "explore-intent",
                "explore",
                "intent-ready",
            )]))),
            ..FakeGateway::default()
        }
    }

    // Frozen initial_input key is `work_slot_bindings`: object map slot_id → {command, args}.
    // Binding value type with exactly `{command, args}`. `command: String`. `args: Vec<String>` — the same argv list type loop-integrations already uses for process argument lists (`ProviderDefinition.args` / `ProviderInvocation.args`). `#[serde(deny_unknown_fields)]`.
    // Omit the key OR `{}` both mean no bindings. Start must succeed in both cases (existing start tests omit the key and MUST keep passing).
    // Start rejects: unknown slot id (not in the provider catalog snapshot for this workflow), unknown fields on a binding object, non-object values (the map itself or a binding value).

    #[test]
    fn work_slot_bind_omitted_key_succeeds() {
        // Omit the key OR `{}` both mean no bindings. Start must succeed in both cases (existing start tests omit the key and MUST keep passing).
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test"}),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        assert!(created.initial_input.get("work_slot_bindings").is_none());
    }

    #[test]
    fn work_slot_bind_empty_object_succeeds_and_keeps_empty_map() {
        // Omit the key OR `{}` both mean no bindings. Start must succeed in both cases (existing start tests omit the key and MUST keep passing).
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test", "work_slot_bindings": {}}),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        let allocated = allocated_canonical_string(catalog.path(), "run-1");
        assert_eq!(
            created.initial_input.get("work_slot_bindings"),
            Some(&json!({}))
        );
        assert_eq!(
            created.initial_input.get("artifact_root"),
            Some(&json!(allocated))
        );
    }

    #[test]
    fn work_slot_bind_valid_command_args_for_known_slot_is_frozen() {
        // Binding value type with exactly `{command, args}`. `command: String`. `args: Vec<String>` — the same argv list type loop-integrations already uses for process argument lists (`ProviderDefinition.args` / `ProviderInvocation.args`). `#[serde(deny_unknown_fields)]`.
        // Frozen initial_input key is `work_slot_bindings`: object map slot_id → {command, args}.
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let bindings = json!({
            "explore-intent": {
                "command": "/usr/bin/explore",
                "args": ["--intent", "ready"]
            }
        });
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test", "work_slot_bindings": bindings}),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        assert_eq!(created.initial_input["work_slot_bindings"], bindings);
        assert_eq!(
            created.initial_input["work_slot_bindings"]["explore-intent"]["command"],
            json!("/usr/bin/explore")
        );
        assert_eq!(
            created.initial_input["work_slot_bindings"]["explore-intent"]["args"],
            json!(["--intent", "ready"])
        );
    }

    #[test]
    fn work_slot_bind_zero_worker_fan_out_argv_is_still_accepted() {
        // Start does not parse fan-out argv. Zero --worker is a preview-bindings
        // error, not a start rejection.
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let bindings = json!({
            "explore-intent": {
                "command": "loop-engine",
                "args": ["fan-out"]
            }
        });
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"objective": "test", "work_slot_bindings": bindings}),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_completed());
        let created = &persistence.created.borrow()[0];
        assert_eq!(created.initial_input["work_slot_bindings"], bindings);
    }

    #[test]
    fn work_slot_bind_unknown_slot_id_is_rejected_without_creating_a_run() {
        // Start rejects: unknown slot id (not in the provider catalog snapshot for this workflow)
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({
                "work_slot_bindings": {
                    "not-a-slot": {"command": "echo", "args": []}
                }
            }),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "unknown-work-slot");
        assert!(persistence.created.borrow().is_empty());
    }

    #[test]
    fn work_slot_bind_extra_fields_on_binding_object_are_rejected() {
        // Start rejects: unknown fields on a binding object
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({
                "work_slot_bindings": {
                    "explore-intent": {
                        "command": "echo",
                        "args": [],
                        "extra": true
                    }
                }
            }),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "invalid-work-slot-binding");
        assert!(persistence.created.borrow().is_empty());
    }

    #[test]
    fn work_slot_bind_non_object_map_or_value_is_rejected() {
        // Start rejects: non-object values (the map itself or a binding value).
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({"work_slot_bindings": "not-an-object"}),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(
            outcome.issue().unwrap().code,
            "work-slot-bindings-not-object"
        );
        assert!(persistence.created.borrow().is_empty());

        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-2",
            "fake",
            json!({
                "work_slot_bindings": {
                    "explore-intent": "not-an-object"
                }
            }),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "invalid-work-slot-binding");
        assert!(persistence.created.borrow().is_empty());
    }

    #[test]
    fn work_slot_bind_string_args_value_is_rejected() {
        // args must be a JSON array of strings (the Vec<String> argv type); a string args value is rejected.
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = gateway_with_explore_intent_slot();
        let persistence = FakePersistence::default();
        let request = start::Request::new(
            "run-1",
            "fake",
            json!({
                "work_slot_bindings": {
                    "explore-intent": {
                        "command": "echo",
                        "args": "--oops"
                    }
                }
            }),
            None,
            Timestamp::from_unix_millis(10),
            catalog.path(),
        );

        let outcome = start::execute(request, &resolver, &gateway, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "invalid-work-slot-binding");
        assert!(persistence.created.borrow().is_empty());
    }

    #[test]
    fn start_rejects_run_id_that_is_not_a_single_path_component() {
        let catalog = temp_catalog();
        let outside = temp_catalog();
        let absolute_escaped = outside.path().join("should-not-be-created");
        let absolute_id = absolute_escaped.to_string_lossy().into_owned();
        let escaped_parent = catalog.path().join("escaped");
        let escaped_separator = catalog.path().join("runs").join("foo");

        let cases = [
            ("..", escaped_parent.as_path()),
            ("../escaped", escaped_parent.as_path()),
            (absolute_id.as_str(), absolute_escaped.as_path()),
            ("foo/bar", escaped_separator.as_path()),
        ];

        for (run_id, escaped) in cases {
            let resolver = FakeResolver::default();
            let gateway = FakeGateway::default();
            let persistence = FakePersistence::default();
            let request = start::Request::new(
                run_id,
                "fake",
                json!({"objective": "test"}),
                None,
                Timestamp::from_unix_millis(10),
                catalog.path(),
            );

            let outcome = start::execute(request, &resolver, &gateway, &persistence);

            assert!(outcome.is_error(), "run_id `{run_id}`");
            assert_eq!(
                outcome.issue().unwrap().code,
                "run-directory-failed",
                "run_id `{run_id}`"
            );
            assert!(
                persistence.created.borrow().is_empty(),
                "run_id `{run_id}` must not persist"
            );
            assert!(
                !escaped.exists(),
                "run_id `{run_id}` must not create `{}`",
                escaped.display()
            );
        }
    }

    #[test]
    fn from_run_for_run_summary_sets_provider_and_artifact_root_none() {
        let run = Run::new(
            "run-1",
            Some("example".to_owned()),
            workflow(false),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "test", "artifact_root": "/allocated/run-1"}),
            "start",
            Lifecycle::Active,
            0_u64.into(),
            1_u64.into(),
            Timestamp::from_unix_millis(10),
        );
        let summary = RunSummary::from(&run);
        assert_eq!(summary.provider, None);
        assert_eq!(summary.artifact_root, None);
        assert_eq!(summary.id, run.id);
        assert_eq!(summary.label, run.label);
        assert_eq!(summary.workflow_id, run.workflow.id);
        assert_eq!(summary.lifecycle, run.lifecycle);
        assert_eq!(summary.current_state, run.current_state);
    }

    #[test]
    fn invalid_described_workflow_is_error_and_does_not_create_a_run() {
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = FakeGateway {
            described: Some(Ok(Workflow::new(
                "invalid",
                "missing",
                vec![State::new("start", "Start", "", false)],
                vec![],
            ))),
            ..FakeGateway::default()
        };
        let persistence = FakePersistence::default();

        let outcome = start::execute(
            start_request(catalog.path()),
            &resolver,
            &gateway,
            &persistence,
        );

        assert!(outcome.is_error());
        assert_eq!(persistence.created.borrow().len(), 0);
    }

    #[test]
    fn final_initial_state_creates_a_final_run() {
        let catalog = temp_catalog();
        let resolver = FakeResolver::default();
        let gateway = FakeGateway {
            described: Some(Ok(workflow(true))),
            ..FakeGateway::default()
        };
        let persistence = FakePersistence::default();

        let outcome = start::execute(
            start_request(catalog.path()),
            &resolver,
            &gateway,
            &persistence,
        );

        assert!(outcome.is_completed());
        assert_eq!(persistence.created.borrow()[0].lifecycle, Lifecycle::Final);
        assert_eq!(outcome.value().unwrap().run.lifecycle, Lifecycle::Final);
    }

    #[test]
    fn append_success_returns_the_atomic_context_and_history_result() {
        let request = AppendContextRequest::new(
            "run-1",
            "context-1",
            "user-steering",
            json!({"text": "keep compatibility"}),
            Timestamp::from_unix_millis(11),
        );
        let context = ContextRecord::new(
            request.record_id.clone(),
            request.kind.clone(),
            request.data.clone(),
            2_u64.into(),
            request.created_at,
        );
        let history = HistoryEntry::context_appended(
            2_u64.into(),
            request.created_at,
            request.record_id.clone(),
        );
        let run = Run::new(
            "run-1",
            None,
            workflow(false),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "test"}),
            "start",
            Lifecycle::Active,
            0_u64.into(),
            2_u64.into(),
            Timestamp::from_unix_millis(10),
        );
        let persistence = FakePersistence {
            append_result: RefCell::new(Some(Ok(AppendContextResult {
                run,
                context: context.clone(),
                history: history.clone(),
            }))),
            ..FakePersistence::default()
        };

        let outcome = append::execute(request, &persistence);

        assert!(outcome.is_completed());
        assert_eq!(outcome.value().unwrap().context, context);
        assert_eq!(outcome.value().unwrap().history, history);
    }

    #[test]
    fn append_maps_terminal_rejection_without_attempting_any_other_write() {
        let persistence = FakePersistence {
            append_result: RefCell::new(Some(Err(PersistenceError::rejected(
                PersistenceRejection::RunNotActive {
                    run_id: RunId::new("run-1"),
                    lifecycle: Lifecycle::Final,
                },
            )))),
            ..FakePersistence::default()
        };
        let request = AppendContextRequest::new(
            "run-1",
            "context-1",
            "user-steering",
            json!({"text": "keep compatibility"}),
            Timestamp::from_unix_millis(11),
        );

        let outcome = append::execute(request, &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "run-not-active");
        assert_eq!(persistence.appended.borrow().len(), 1);
    }

    #[test]
    fn list_returns_persistence_projection() {
        let summary = RunSummary {
            id: RunId::new("run-1"),
            label: Some("example".to_owned()),
            workflow_id: "workflow".into(),
            lifecycle: Lifecycle::Active,
            current_state: "start".into(),
            provider: None,
            artifact_root: None,
        };
        let persistence = FakePersistence {
            list_result: RefCell::new(Some(Ok(vec![summary.clone()]))),
            ..FakePersistence::default()
        };

        let outcome = list::execute(&persistence);

        assert_eq!(outcome.value().unwrap(), &vec![summary]);
    }

    #[test]
    fn history_is_returned_in_semantic_sequence_order() {
        let later = HistoryEntry::new(
            3_u64.into(),
            Timestamp::from_unix_millis(3),
            HistoryAction::Terminated,
        );
        let earlier = HistoryEntry::run_created(1_u64.into(), Timestamp::from_unix_millis(1));
        let persistence = FakePersistence {
            history_result: RefCell::new(Some(Ok(vec![later, earlier]))),
            ..FakePersistence::default()
        };

        let outcome = history::execute(history::Request::new("run-1"), &persistence);
        let entries = outcome.value().unwrap();

        assert_eq!(entries[0].sequence, SemanticSequence::new(1));
        assert_eq!(entries[1].sequence, SemanticSequence::new(3));
    }

    #[test]
    fn terminate_success_returns_the_terminated_run_and_history() {
        let history = HistoryEntry::terminated(2_u64.into(), Timestamp::from_unix_millis(12));
        let run = Run::new(
            "run-1",
            None,
            workflow(false),
            ProviderAssociation::new(json!({"provider": "fake"})),
            json!({"objective": "test"}),
            "start",
            Lifecycle::Terminated,
            1_u64.into(),
            2_u64.into(),
            Timestamp::from_unix_millis(10),
        );
        let persistence = FakePersistence {
            terminate_result: RefCell::new(Some(Ok(TerminateResult {
                run,
                history: history.clone(),
            }))),
            ..FakePersistence::default()
        };

        let outcome = terminate::execute(terminate::Request::new("run-1"), &persistence);

        assert!(outcome.is_completed());
        assert_eq!(
            outcome.value().unwrap().run.lifecycle,
            Lifecycle::Terminated
        );
        assert_eq!(outcome.value().unwrap().history, history);
    }

    #[test]
    fn terminate_maps_terminal_rejection() {
        let persistence = FakePersistence {
            terminate_result: RefCell::new(Some(Err(PersistenceError::rejected(
                PersistenceRejection::RunNotActive {
                    run_id: RunId::new("run-1"),
                    lifecycle: Lifecycle::Terminated,
                },
            )))),
            ..FakePersistence::default()
        };

        let outcome = terminate::execute(terminate::Request::new("run-1"), &persistence);

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "run-not-active");
        assert_eq!(persistence.terminated.borrow().len(), 1);
    }

    #[test]
    fn missing_run_is_an_error_for_append_history_and_terminate() {
        let missing = || PersistenceError::not_found("missing");
        let append_persistence = FakePersistence {
            append_result: RefCell::new(Some(Err(missing()))),
            ..FakePersistence::default()
        };
        let history_persistence = FakePersistence {
            history_result: RefCell::new(Some(Err(missing()))),
            ..FakePersistence::default()
        };
        let terminate_persistence = FakePersistence {
            terminate_result: RefCell::new(Some(Err(missing()))),
            ..FakePersistence::default()
        };

        let append_outcome = append::execute(
            AppendContextRequest::new(
                "missing",
                "context-1",
                "kind",
                Value::Null,
                Timestamp::from_unix_millis(1),
            ),
            &append_persistence,
        );
        let history_outcome =
            history::execute(history::Request::new("missing"), &history_persistence);
        let terminate_outcome =
            terminate::execute(terminate::Request::new("missing"), &terminate_persistence);

        for outcome in [
            append_outcome.status(),
            history_outcome.status(),
            terminate_outcome.status(),
        ] {
            assert_eq!(outcome, crate::OperationStatus::Error);
        }
    }

    #[test]
    fn provider_and_persistence_failures_are_errors_with_stable_codes() {
        let catalog = temp_catalog();
        let resolver = FakeResolver {
            result: Some(Err(ProviderResolutionError::unavailable(
                "config",
                "cannot load provider",
            ))),
            ..FakeResolver::default()
        };
        let gateway = FakeGateway::default();
        let persistence = FakePersistence::default();
        let resolution = start::execute(
            start_request(catalog.path()),
            &resolver,
            &gateway,
            &persistence,
        );
        assert_eq!(resolution.issue().unwrap().code, "provider-unavailable");
        assert_eq!(persistence.created.borrow().len(), 0);

        let failure = PersistenceError::failure(
            PersistenceFailure::new("disk", "write failed").with_details(json!({"errno": 5})),
        );
        let persistence = FakePersistence {
            append_result: RefCell::new(Some(Err(failure))),
            ..FakePersistence::default()
        };
        let append = append::execute(
            AppendContextRequest::new(
                "run-1",
                ContextRecordId::new("context-1"),
                "kind",
                Value::Null,
                Timestamp::from_unix_millis(1),
            ),
            &persistence,
        );
        assert!(append.is_error());
        assert_eq!(append.issue().unwrap().code, "persistence-failure");
        assert_eq!(append.issue().unwrap().details, Some(json!({"errno": 5})));
    }
}
