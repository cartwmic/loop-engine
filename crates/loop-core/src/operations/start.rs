//! `start` run operation.

use super::{persistence_error, provider_error, provider_resolution_error, workflow_error};
use crate::{
    CreateRunRequest, CreateRunResult, Lifecycle, OperationOutcome, ProviderGateway,
    ProviderResolver, ProviderSelector, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Caller-supplied values needed to create a run.
///
/// The run ID and timestamp are deliberately supplied by the caller or
/// composition root.  Core does not invent identifier or clock ports.
/// `catalog_root` is the parent of the resolved catalog database; start
/// allocates `<catalog_root>/runs/<run_id>/` before `create_run`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub id: crate::RunId,
    pub provider: ProviderSelector,
    pub label: Option<String>,
    pub initial_input: Value,
    pub created_at: Timestamp,
    pub catalog_root: PathBuf,
}

impl Request {
    pub fn new(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: Option<String>,
        created_at: Timestamp,
        catalog_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            label,
            initial_input,
            created_at,
            catalog_root: catalog_root.into(),
        }
    }

    pub fn with_label(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: impl Into<String>,
        created_at: Timestamp,
        catalog_root: impl Into<PathBuf>,
    ) -> Self {
        Self::new(
            id,
            provider,
            initial_input,
            Some(label.into()),
            created_at,
            catalog_root,
        )
    }
}

/// Successful `start` data returned by the persistence boundary.
pub type Result = CreateRunResult;

/// Execute `start` through deterministic provider and atomic persistence ports.
/// Execution bindings are retired; omitted or empty bindings remain harmless.
pub fn execute<R, G, P>(
    request: Request,
    resolver: &R,
    gateway: &G,
    persistence: &P,
) -> OperationOutcome<Result>
where
    R: ProviderResolver + ?Sized,
    G: ProviderGateway + ?Sized,
    P: crate::Persistence + ?Sized,
{
    let association = match resolver.resolve(&request.provider) {
        Ok(association) => association,
        Err(error) => return provider_resolution_error(error),
    };

    let workflow = match gateway.describe(&association, Some(&request.initial_input)) {
        Ok(workflow) => workflow,
        Err(error) => return provider_error(error),
    };

    if let Err(error) = workflow.validate() {
        return workflow_error(error);
    }

    if let Some(rejected) = work_slot_bindings_rejection(&request.initial_input) {
        return rejected;
    }

    let initial_state = workflow.initial_state.clone();
    let Some(initial_state_definition) = workflow
        .states
        .iter()
        .find(|state| state.id == initial_state)
    else {
        // `validate` guarantees this cannot happen for a well-formed
        // workflow.  Keep the operation total if a future validation change
        // ever weakens that guarantee.
        return OperationOutcome::error(
            "invalid-workflow",
            "validated workflow has no definition for its initial state",
        );
    };
    let lifecycle = if initial_state_definition.is_final {
        Lifecycle::Final
    } else {
        Lifecycle::Active
    };

    let allocated = match allocate_run_directory(&request.catalog_root, &request.id) {
        Ok(path) => path,
        Err((code, message)) => return OperationOutcome::error(code, message),
    };
    let (composed_input, recorded_artifact_root) =
        compose_stored_input(request.initial_input, &allocated);

    let create = CreateRunRequest::new(
        request.id,
        request.label,
        workflow,
        association,
        composed_input,
        initial_state,
        lifecycle,
        request.created_at,
        request.provider.as_str().to_owned(),
        Some(recorded_artifact_root),
    );

    match persistence.create_run(create) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

/// Execute `start` with ports first, which is convenient for composition
/// roots that keep their adapters together.
pub fn execute_with_ports<R, G, P>(
    resolver: &R,
    gateway: &G,
    persistence: &P,
    request: Request,
) -> OperationOutcome<Result>
where
    R: ProviderResolver + ?Sized,
    G: ProviderGateway + ?Sized,
    P: crate::Persistence + ?Sized,
{
    execute(request, resolver, gateway, persistence)
}

fn work_slot_bindings_rejection(initial_input: &Value) -> Option<OperationOutcome<Result>> {
    let bindings = initial_input.as_object()?.get("work_slot_bindings")?;
    if bindings
        .as_object()
        .is_some_and(|bindings| bindings.is_empty())
    {
        return None;
    }
    Some(OperationOutcome::rejected(
        "work-slot-bindings-retired",
        "work_slot_bindings execution is retired; perform work externally and record supplied results",
    ))
}

fn allocate_run_directory(
    catalog_root: &Path,
    run_id: &crate::RunId,
) -> std::result::Result<PathBuf, (String, String)> {
    let mut components = Path::new(run_id.as_str()).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => {
            return Err((
                "run-directory-failed".to_owned(),
                format!(
                    "run id `{}` is not a single path component",
                    run_id.as_str()
                ),
            ));
        }
    }

    let catalog_root = if catalog_root.is_relative() {
        let current_dir = std::env::current_dir().map_err(|error| {
            (
                "run-directory-failed".to_owned(),
                format!("could not resolve current directory for catalog root: {error}"),
            )
        })?;
        current_dir.join(catalog_root)
    } else {
        catalog_root.to_path_buf()
    };
    let allocated = catalog_root.join("runs").join(run_id.as_str());
    std::fs::create_dir_all(&allocated).map_err(|error| {
        (
            "run-directory-failed".to_owned(),
            format!(
                "could not create run directory `{}`: {error}",
                allocated.display()
            ),
        )
    })?;
    std::fs::canonicalize(&allocated).map_err(|error| {
        (
            "run-directory-failed".to_owned(),
            format!(
                "could not canonicalize run directory `{}`: {error}",
                allocated.display()
            ),
        )
    })
}

/// Compose stored `initial_input` and the catalog `artifact_root` string.
///
/// A JSON object that already has a non-empty string `artifact_root` keeps
/// that string (including a relative path).  Other objects receive the
/// allocated canonical path.  Non-objects are left unchanged; the catalog
/// column still records the allocated path.
fn compose_stored_input(initial_input: Value, allocated_canonical: &Path) -> (Value, String) {
    let allocated = allocated_canonical.to_string_lossy().into_owned();
    match initial_input {
        Value::Object(mut map) => {
            let caller_path = match map.get("artifact_root") {
                Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                _ => None,
            };
            if let Some(recorded) = caller_path {
                (Value::Object(map), recorded)
            } else {
                map.insert("artifact_root".to_owned(), Value::String(allocated.clone()));
                (Value::Object(map), allocated)
            }
        }
        other => (other, allocated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkSlotId;
    use crate::{
        AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
        CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
        ContextRecord, CreateRunRequest, DurableEvaluation, HistoryEntry, Persistence,
        PersistenceError, PersistenceFailure, ProviderAssociation, ProviderError,
        ProviderResolutionError, Run, RunId, RunSummary, ShowData, State, TerminateRequest,
        TerminateResult, Transition, WorkSlotInvocation, Workflow,
    };
    use serde_json::json;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
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

    fn failure<T>() -> std::result::Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "fake",
            "fake persistence failure",
        )))
    }

    #[derive(Default)]
    struct FakeResolver;

    impl ProviderResolver for FakeResolver {
        fn resolve(
            &self,
            _selector: &ProviderSelector,
        ) -> std::result::Result<ProviderAssociation, ProviderResolutionError> {
            Ok(ProviderAssociation::new(json!({"provider": "fake"})))
        }
    }

    struct FakeGateway {
        workflow: Workflow,
    }

    impl FakeGateway {
        fn new(workflow: Workflow) -> Self {
            Self { workflow }
        }
    }

    impl ProviderGateway for FakeGateway {
        fn describe(
            &self,
            _provider: &ProviderAssociation,
            _initial_input: Option<&Value>,
        ) -> std::result::Result<Workflow, ProviderError> {
            Ok(self.workflow.clone())
        }

        fn evaluate(
            &self,
            _provider: &ProviderAssociation,
            _request: crate::EvaluationRequest,
        ) -> std::result::Result<crate::EvaluationResult, ProviderError> {
            Ok(crate::EvaluationResult::Unsupported)
        }
    }

    #[derive(Default)]
    struct RecordingPersistence {
        created: RefCell<Vec<CreateRunRequest>>,
        set_subject_calls: RefCell<Vec<(RunId, WorkSlotId, String)>>,
        subjects: RefCell<std::collections::BTreeMap<String, String>>,
    }

    impl Persistence for RecordingPersistence {
        fn create_run(
            &self,
            request: CreateRunRequest,
        ) -> std::result::Result<CreateRunResult, PersistenceError> {
            self.created.borrow_mut().push(request.clone());
            Ok(CreateRunResult {
                run: run_from(&request),
                history: HistoryEntry::run_created(1_u64.into(), request.created_at),
            })
        }

        fn append_context(
            &self,
            _request: AppendContextRequest,
        ) -> std::result::Result<AppendContextResult, PersistenceError> {
            failure()
        }

        fn commit_transition(
            &self,
            _request: CommitTransitionRequest,
        ) -> std::result::Result<CommitTransitionResult, PersistenceError> {
            failure()
        }

        fn record_denial(
            &self,
            _request: crate::RecordDenialRequest,
        ) -> std::result::Result<crate::RecordDenialResult, PersistenceError> {
            failure()
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
            Err(PersistenceError::not_found(run_id.clone()))
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
            _request: CheckedEvaluationSnapshotRequest,
        ) -> std::result::Result<CheckedEvaluationSnapshot, PersistenceError> {
            failure()
        }

        fn load_show_data(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<ShowData, PersistenceError> {
            failure()
        }

        fn get_current_slot_subject(
            &self,
            _run_id: &RunId,
            slot_id: &WorkSlotId,
        ) -> std::result::Result<Option<String>, PersistenceError> {
            Ok(self.subjects.borrow().get(slot_id.as_str()).cloned())
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<WorkSlotInvocation>, PersistenceError> {
            Ok(Vec::new())
        }
    }

    fn temp_catalog() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp catalog root")
    }

    fn start_request(catalog_root: impl Into<PathBuf>) -> Request {
        Request::new(
            "run-1",
            "fake",
            json!({"objective": "test"}),
            Some("example".to_owned()),
            Timestamp::from_unix_millis(10),
            catalog_root,
        )
    }

    #[test]
    fn start_does_not_mint_when_initial_state_is_not_a_slot() {
        let catalog = temp_catalog();
        let persistence = RecordingPersistence::default();
        let outcome = execute(
            start_request(catalog.path()),
            &FakeResolver,
            &FakeGateway::new(workflow()),
            &persistence,
        );

        assert!(outcome.is_completed());
        assert!(persistence.set_subject_calls.borrow().is_empty());
    }
}
