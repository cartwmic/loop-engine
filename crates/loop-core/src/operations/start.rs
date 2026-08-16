//! `start` run operation.

use super::{persistence_error, provider_error, provider_resolution_error, workflow_error};
use crate::{
    CreateRunRequest, CreateRunResult, Lifecycle, OperationOutcome, ProviderGateway,
    ProviderResolver, ProviderSelector, Timestamp, WorkSlot, WorkSlotBinding, WorkSlotId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static VISIT_SUBJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Execute `start` through the provider and persistence ports.
///
/// Provider resolution and description happen before validation and before
/// persistence is called.  After a valid workflow, start validates reserved
/// `work_slot_bindings` on object initial_input against the described
/// `work_slots` catalog, then allocates the engine-owned per-run directory
/// and composes reserved `artifact_root` into stored object input.  The
/// persistence adapter owns the atomic run plus creation-history write.
///
/// Frozen initial_input key is `work_slot_bindings`: object map slot_id → {command, args}.
/// Slot identity is a string slot_id. Use the existing `string_identifier!` newtype pattern: `WorkSlotId`.
/// Catalog entry type `WorkSlot` with exactly: `id` (WorkSlotId), `state` (StateId), `event` (EventId). No instruction body.
/// Binding value type with exactly `{command, args}`. `command: String`. `args: Vec<String>` — the same argv list type loop-integrations already uses for process argument lists (`ProviderDefinition.args` / `ProviderInvocation.args`). `#[serde(deny_unknown_fields)]`.
/// Omit the key OR `{}` both mean no bindings. Start must succeed in both cases (existing start tests omit the key and MUST keep passing).
/// Start rejects: unknown slot id (not in the provider catalog snapshot for this workflow), unknown fields on a binding object, non-object values (the map itself or a binding value).
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

    let workflow = match gateway.describe(&association) {
        Ok(workflow) => workflow,
        Err(error) => return provider_error(error),
    };

    if let Err(error) = workflow.validate() {
        return workflow_error(error);
    }

    if let Some(rejected) =
        work_slot_bindings_rejection(&request.initial_input, &workflow.work_slots)
    {
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

    let slot_subjects = slot_subjects_for_state(&workflow, &initial_state);
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
    )
    .with_slot_subjects(slot_subjects);

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

/// Frozen initial_input key is `work_slot_bindings`: object map slot_id → {command, args}.
///
/// Slot identity is a string slot_id. Use the existing `string_identifier!` newtype pattern: `WorkSlotId`.
/// Catalog entry type `WorkSlot` with exactly: `id` (WorkSlotId), `state` (StateId), `event` (EventId). No instruction body.
/// Binding value type with exactly `{command, args}`. `command: String`. `args: Vec<String>` — the same argv list type loop-integrations already uses for process argument lists (`ProviderDefinition.args` / `ProviderInvocation.args`). `#[serde(deny_unknown_fields)]`.
/// Omit the key OR `{}` both mean no bindings. Start must succeed in both cases (existing start tests omit the key and MUST keep passing).
/// Start rejects: unknown slot id (not in the provider catalog snapshot for this workflow), unknown fields on a binding object, non-object values (the map itself or a binding value).
///
/// Missing key or `{}` → no bindings; continue; leave the JSON as the caller
/// sent it (do not inject `{}`). Persist by leaving `work_slot_bindings` in
/// the frozen `initial_input` already stored on CreateRunRequest.
const WORK_SLOT_BINDINGS_KEY: &str = "work_slot_bindings";

fn work_slot_bindings_rejection(
    initial_input: &Value,
    work_slots: &[WorkSlot],
) -> Option<OperationOutcome<Result>> {
    let Value::Object(map) = initial_input else {
        return None;
    };
    let bindings_value = map.get(WORK_SLOT_BINDINGS_KEY)?;
    let Value::Object(bindings) = bindings_value else {
        return Some(OperationOutcome::rejected(
            "work-slot-bindings-not-object",
            "work_slot_bindings must be an object map of slot_id to {command, args}",
        ));
    };
    if bindings.is_empty() {
        return None;
    }

    let known: std::collections::BTreeSet<&str> =
        work_slots.iter().map(|slot| slot.id.as_str()).collect();

    for (slot_id, binding) in bindings {
        if let Err(error) = serde_json::from_value::<WorkSlotBinding>(binding.clone()) {
            return Some(OperationOutcome::rejected(
                "invalid-work-slot-binding",
                format!(
                    "work_slot_bindings[{slot_id}] must be an object with exactly {{command, args}}: {error}"
                ),
            ));
        }
        if !known.contains(slot_id.as_str()) {
            return Some(OperationOutcome::rejected(
                "unknown-work-slot",
                format!(
                    "work_slot_bindings slot id `{slot_id}` is not in the provider catalog snapshot for this workflow"
                ),
            ));
        }
    }

    None
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

fn current_timestamp() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    Timestamp::from_unix_millis(millis)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
        CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
        CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult, ContextRecord,
        CreateRunRequest, DurableEvaluation, HistoryEntry, Persistence, PersistenceError,
        PersistenceFailure, ProviderAssociation, ProviderError, ProviderResolutionError, Run,
        RunId, RunSummary, ShowData, State, TerminateRequest, TerminateResult, Transition,
        WorkSlotInvocation, Workflow,
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

    fn slotted_workflow() -> Workflow {
        workflow().with_work_slots(vec![WorkSlot::new("slot-1", "start", "approve")])
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
        set_subject_error: RefCell<Option<PersistenceError>>,
    }

    impl Persistence for RecordingPersistence {
        fn create_run(
            &self,
            request: CreateRunRequest,
        ) -> std::result::Result<CreateRunResult, PersistenceError> {
            if let Some(error) = self.set_subject_error.borrow_mut().take() {
                if !request.slot_subjects.is_empty() {
                    for (slot_id, subject) in &request.slot_subjects {
                        self.set_subject_calls.borrow_mut().push((
                            request.id.clone(),
                            slot_id.clone(),
                            subject.clone(),
                        ));
                    }
                    return Err(error);
                }
            }
            for (slot_id, subject) in &request.slot_subjects {
                self.set_subject_calls.borrow_mut().push((
                    request.id.clone(),
                    slot_id.clone(),
                    subject.clone(),
                ));
                self.subjects
                    .borrow_mut()
                    .insert(slot_id.as_str().to_owned(), subject.clone());
            }
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

        fn create_work_slot_invocation(
            &self,
            _request: crate::CreateWorkSlotInvocationRequest,
        ) -> std::result::Result<crate::CreateWorkSlotInvocationResult, PersistenceError> {
            failure()
        }

        fn complete_work_slot_invocation(
            &self,
            _request: CompleteWorkSlotInvocationRequest,
        ) -> std::result::Result<CompleteWorkSlotInvocationResult, PersistenceError> {
            failure()
        }

        fn get_current_slot_subject(
            &self,
            _run_id: &RunId,
            slot_id: &WorkSlotId,
        ) -> std::result::Result<Option<String>, PersistenceError> {
            Ok(self.subjects.borrow().get(slot_id.as_str()).cloned())
        }

        fn set_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &WorkSlotId,
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
    fn start_mints_subject_when_initial_state_is_a_slot() {
        let catalog = temp_catalog();
        let persistence = RecordingPersistence::default();
        let outcome = execute(
            start_request(catalog.path()),
            &FakeResolver,
            &FakeGateway::new(slotted_workflow()),
            &persistence,
        );

        assert!(outcome.is_completed());
        let calls = persistence.set_subject_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.as_str(), "run-1");
        assert_eq!(calls[0].1.as_str(), "slot-1");
        assert!(calls[0].2.starts_with("visit-slot-1-"));
        assert_eq!(
            persistence
                .subjects
                .borrow()
                .get("slot-1")
                .map(String::as_str),
            Some(calls[0].2.as_str())
        );
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

    #[test]
    fn start_subject_mint_failure_is_persistence_error() {
        let catalog = temp_catalog();
        let persistence = RecordingPersistence {
            set_subject_error: RefCell::new(Some(PersistenceError::failure(
                PersistenceFailure::new("fake", "could not store visit subject"),
            ))),
            ..RecordingPersistence::default()
        };
        let outcome = execute(
            start_request(catalog.path()),
            &FakeResolver,
            &FakeGateway::new(slotted_workflow()),
            &persistence,
        );

        assert!(outcome.is_error());
        assert_eq!(outcome.issue().unwrap().code, "persistence-failure");
        assert!(persistence.created.borrow().is_empty());
        assert_eq!(persistence.set_subject_calls.borrow().len(), 1);
    }
}
