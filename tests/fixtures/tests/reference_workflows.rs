//! End-to-end acceptance suites for the three reference workflows.
//!
//! These tests intentionally compose the core operations with the real SQLite
//! adapter, the real one-shot subprocess gateway, and the fixture provider
//! executables.  `Composition` opens a new SQLite connection for every
//! operation, which keeps actor handoff tests honest: no operation depends on
//! an in-memory persistence object left by the previous actor.

use loop_core::{
    self as core, AppendContextResult, ContextRecord, EvaluationResult, EventRequest, EventResult,
    HistoryEntry, Lifecycle, OperationOutcome, Persistence, ProviderError, ProviderGateway,
    ProviderSelector, Run, RunId, ShowProjection, StartRequest, StateId, Timestamp, Transition,
    Workflow,
};
use loop_integrations::{
    ConfiguredProviderResolver, ProviderConfiguration, ProviderDefinition, ProviderInvocation,
    SqlitePersistence, SubprocessProviderGateway,
};
use loop_reference_fixtures::{
    agents_policy_input, document_policy, fixture_binary, policy_document_initial_input,
    policy_document_workflow, readme_policy_input, research_artifact_schemas, research_brief,
    research_initial_input, research_policy_set_a, research_report, research_review_context,
    research_revision_links, research_sources, research_verification, research_workflow,
    software_change_initial_input, software_change_policy_set_a, software_change_policy_set_b,
    software_change_review_context, software_change_workflow, DESIGN_REVIEW_GATE,
    IMPLEMENTATION_REVIEW_GATE, PLAN_REVIEW_GATE, RESEARCH_SYNTHESIZE_GATE, RESEARCH_VERIFY_GATE,
    VALIDATION_GATE,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::tempdir;

const PROVIDER_ALIAS: &str = "reference";
const SOFTWARE_ALIAS: &str = "software";
const DOCUMENT_ALIAS: &str = "document";
const RESEARCH_ALIAS: &str = "research";

/// A composition root used by the acceptance suites.
///
/// It deliberately does not retain a `SqlitePersistence` instance.  Each
/// operation opens the durable file afresh, just as a new actor/process would.
struct Composition {
    database: PathBuf,
    resolver: ConfiguredProviderResolver,
    gateway: SubprocessProviderGateway,
}

impl Composition {
    fn new(database: impl Into<PathBuf>, resolver: ConfiguredProviderResolver) -> Self {
        Self {
            database: database.into(),
            resolver,
            gateway: SubprocessProviderGateway::new(Duration::from_secs(2)),
        }
    }

    fn persistence(&self) -> SqlitePersistence {
        SqlitePersistence::open(&self.database).expect("open durable acceptance database")
    }

    fn catalog_root(&self) -> PathBuf {
        self.database
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn start(&self, run_id: &str, provider: &str, initial_input: Value) -> Run {
        let persistence = self.persistence();
        let outcome = core::execute_start(
            StartRequest::new(
                RunId::from(run_id),
                ProviderSelector::from(provider),
                initial_input,
                None,
                Timestamp::from_unix_millis(1),
                self.catalog_root(),
            ),
            &self.resolver,
            &self.gateway,
            &persistence,
        );
        require_completed(outcome).run
    }

    fn show(&self, run_id: &str) -> ShowProjection {
        struct ShowProcess;

        impl core::WorkSlotProcess for ShowProcess {
            type Handle = ();

            fn waiter_alive(&self, _pid: u32) -> bool {
                false
            }

            fn spawn_wait_invocation(
                &self,
                _args: core::WaiterSpawnArgs,
            ) -> std::result::Result<core::StartedWaiter<()>, core::ProcessError> {
                Err(core::ProcessError::new(
                    "unsupported",
                    "show helper does not spawn waiters",
                ))
            }

            fn send_envelope_and_detach(
                &self,
                _waiter: core::StartedWaiter<()>,
                _envelope_json: &[u8],
            ) -> std::result::Result<(), core::ProcessError> {
                Err(core::ProcessError::new(
                    "unsupported",
                    "show helper does not spawn waiters",
                ))
            }
        }

        let persistence = self.persistence();
        require_completed(core::execute_show(
            core::ShowRequest::new(RunId::from(run_id)),
            &persistence,
            &ShowProcess,
            Timestamp::from_unix_millis(1),
        ))
    }

    fn authoritative(&self, run_id: &str) -> Run {
        self.persistence()
            .load_authoritative_run(&RunId::from(run_id))
            .expect("load authoritative run")
    }

    fn append(&self, run_id: &str, record_id: &str, kind: &str, data: Value) -> ContextRecord {
        let persistence = self.persistence();
        let result: AppendContextResult = require_completed(core::execute_append(
            core::AppendRequest::new(
                RunId::from(run_id),
                record_id,
                kind,
                data,
                Timestamp::from_unix_millis(1),
            ),
            &persistence,
        ));
        result.context
    }

    fn append_record(&self, run_id: &str, record: ContextRecord) -> ContextRecord {
        self.append(
            run_id,
            record.id.as_str(),
            &record.kind,
            record.data.clone(),
        )
    }

    fn event(&self, run_id: &str, event: &str) -> OperationOutcome<EventResult> {
        self.event_with_gateway(run_id, event, &self.gateway)
    }

    fn event_with_gateway<G>(
        &self,
        run_id: &str,
        event: &str,
        gateway: &G,
    ) -> OperationOutcome<EventResult>
    where
        G: ProviderGateway + ?Sized,
    {
        let persistence = self.persistence();
        core::execute_event(
            EventRequest::new(RunId::from(run_id), event),
            gateway,
            &persistence,
        )
    }

    fn history(&self, run_id: &str) -> Vec<HistoryEntry> {
        self.persistence()
            .load_history(&RunId::from(run_id))
            .expect("load durable history")
    }

    fn evaluation_request(&self, run_id: &str, transition: Transition) -> core::EvaluationRequest {
        let snapshot = self
            .persistence()
            .load_checked_evaluation_snapshot(core::CheckedEvaluationSnapshotRequest::new(
                RunId::from(run_id),
                transition,
            ))
            .expect("capture durable evaluation snapshot");
        core::request_from_snapshot(&snapshot)
    }
}

fn resolver(
    entries: impl IntoIterator<Item = (&'static str, ProviderInvocation)>,
) -> ConfiguredProviderResolver {
    let providers = entries
        .into_iter()
        .map(|(alias, invocation)| {
            (
                alias.to_owned(),
                ProviderDefinition::new(invocation.command, invocation.args),
            )
        })
        .collect::<BTreeMap<_, _>>();
    ConfiguredProviderResolver::new(ProviderConfiguration { providers })
}

fn reference_resolver() -> ConfiguredProviderResolver {
    resolver([
        (
            SOFTWARE_ALIAS,
            ProviderInvocation::new(
                fixture_binary("software-change-provider")
                    .to_string_lossy()
                    .into_owned(),
                Vec::<String>::new(),
            ),
        ),
        (
            DOCUMENT_ALIAS,
            ProviderInvocation::new(
                fixture_binary("policy-document-provider")
                    .to_string_lossy()
                    .into_owned(),
                Vec::<String>::new(),
            ),
        ),
        (
            RESEARCH_ALIAS,
            ProviderInvocation::new(
                fixture_binary("research-provider")
                    .to_string_lossy()
                    .into_owned(),
                Vec::<String>::new(),
            ),
        ),
        (
            PROVIDER_ALIAS,
            ProviderInvocation::new(
                fixture_binary("software-change-provider")
                    .to_string_lossy()
                    .into_owned(),
                Vec::<String>::new(),
            ),
        ),
    ])
}

fn require_completed<T>(outcome: OperationOutcome<T>) -> T {
    match outcome {
        OperationOutcome::Completed(value) => value,
        OperationOutcome::Rejected(issue) => {
            panic!("expected completed outcome, got rejected: {issue:?}")
        }
        OperationOutcome::Error(issue) => {
            panic!("expected completed outcome, got error: {issue:?}")
        }
    }
}

fn require_rejected<T>(outcome: OperationOutcome<T>) -> core::OutcomeIssue {
    match outcome {
        OperationOutcome::Rejected(issue) => issue,
        OperationOutcome::Completed(_) => panic!("expected rejected outcome, got completed"),
        OperationOutcome::Error(issue) => panic!("expected rejected outcome, got error: {issue:?}"),
    }
}

fn require_error<T>(outcome: OperationOutcome<T>) -> core::OutcomeIssue {
    match outcome {
        OperationOutcome::Error(issue) => issue,
        OperationOutcome::Completed(_) => panic!("expected error outcome, got completed"),
        OperationOutcome::Rejected(issue) => {
            panic!("expected error outcome, got rejected: {issue:?}")
        }
    }
}

fn assert_requestable(show: &ShowProjection, event: &str, kind: core::TransitionKind) {
    assert!(show
        .requestable_events
        .iter()
        .any(|candidate| candidate.event.as_str() == event && candidate.kind == kind));
}

fn latest_for<'a>(
    show: &'a ShowProjection,
    transition: &Transition,
) -> &'a core::DurableEvaluation {
    show.latest_evaluations
        .iter()
        .find(|evaluation| evaluation.transition.same_lineage(transition))
        .unwrap_or_else(|| panic!("no latest evaluation for {transition:?}"))
}

fn append_software_evidence(
    engine: &Composition,
    run_id: &str,
    record_id: &str,
    gate: &str,
    policy_id: &str,
    passed: bool,
    findings: Value,
) {
    let record = software_change_review_context(record_id, gate, policy_id, passed, findings, 0);
    engine.append_record(run_id, record);
}

/// A gateway that is guaranteed to be unavailable if a check-free edge were
/// to invoke it.  The count proves that revision/backtracking is provider-free.
#[derive(Default)]
struct UnavailableGateway {
    calls: AtomicUsize,
}

impl UnavailableGateway {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ProviderGateway for UnavailableGateway {
    fn describe(
        &self,
        _provider: &core::ProviderAssociation,
        _initial_input: Option<&serde_json::Value>,
    ) -> Result<Workflow, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::execution(
            "provider-unavailable",
            "acceptance fixture provider is unavailable",
        ))
    }

    fn evaluate(
        &self,
        _provider: &core::ProviderAssociation,
        _request: core::EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::execution(
            "provider-unavailable",
            "acceptance fixture provider is unavailable",
        ))
    }
}

fn with_allocated_artifact_root(mut input: Value, database: &Path, run_id: &str) -> Value {
    let catalog_root = database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let allocated = catalog_root
        .join("runs")
        .join(run_id)
        .canonicalize()
        .expect("allocated run directory")
        .to_string_lossy()
        .into_owned();
    input
        .as_object_mut()
        .expect("object initial_input")
        .insert("artifact_root".to_owned(), json!(allocated));
    input
}

fn policy_axes(input: &Value, gate: &str) -> Vec<String> {
    input["review_policies"][gate]
        .as_array()
        .expect("configured policy axes")
        .iter()
        .map(|policy| policy["id"].as_str().expect("policy ID").to_owned())
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Write a tiny provider wrapper used only for the stored-association
/// evolution acceptance scenario.  It delegates normal requests to the real
/// fixture, can expose a changed `describe`, and can return unsupported for a
/// checked action while recording the exact evaluation request it received.
fn write_provider_wrapper(
    path: &Path,
    fixture: &Path,
    request_log: &Path,
    describe_workflow: &Workflow,
    unsupported_evaluate: bool,
) {
    let workflow_json = serde_json::to_string(describe_workflow).expect("serialize wrapper graph");
    let fixture = shell_quote(&fixture.to_string_lossy());
    let request_log = shell_quote(&request_log.to_string_lossy());
    let workflow_json = shell_quote(&workflow_json);
    let unsupported = if unsupported_evaluate { "1" } else { "0" };
    let script = format!(
        "#!/bin/sh\ninput=$(cat)\nprintf '%s' \"$input\" > {request_log}\nif printf '%s' \"$input\" | grep -q '\"operation\":\"describe\"'; then\n  printf '%s' {workflow_json}\n  exit 0\nfi\nif [ {unsupported} = 1 ]; then\n  printf '%s' '{{\"result\":\"unsupported\"}}'\n  exit 0\nfi\nprintf '%s' \"$input\" | {fixture}\n",
    request_log = request_log,
    workflow_json = workflow_json,
    unsupported = unsupported,
    fixture = fixture,
    );
    fs::write(path, script).expect("write provider wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("make provider wrapper executable");
    }
}

#[test]
fn software_change_reference_workflow_end_to_end_from_clean_durable_state(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = tempdir()?;
    let workspace = tempdir()?;
    let workspace_reference = workspace.path().join("workspace.txt");
    fs::write(&workspace_reference, "external software workspace\n")?;
    let database = state.path().join("software-change.sqlite");
    let engine = Composition::new(&database, reference_resolver());

    // Minimal initial idea, with the external-work identity carried durably in
    // opaque initial input.  `show` is persistence-only, so this assertion
    // proves policies are exposed without a provider call or discovery API.
    let minimal_input = software_change_initial_input(
        software_change_policy_set_a(),
        Some(json!({
            "path": workspace_reference,
            "identity": "software-workspace-A"
        })),
    );
    let minimal = engine.start("software-minimal", SOFTWARE_ALIAS, minimal_input.clone());
    assert_eq!(minimal.workflow, software_change_workflow());
    let initial = engine.show("software-minimal");
    assert_eq!(
        initial.initial_input,
        with_allocated_artifact_root(minimal_input, &database, "software-minimal")
    );
    assert_eq!(initial.current_state, StateId::from("explore"));
    assert_eq!(initial.workflow_id.as_str(), "software-change");
    assert_eq!(
        policy_axes(&initial.initial_input, DESIGN_REVIEW_GATE),
        ["architecture", "compatibility"]
    );
    assert_requestable(&initial, "intent-ready", core::TransitionKind::Checked);

    // Substantial already-known work uses the same immutable graph and core
    // operation.  Its prior context is ordinary opaque context, not a second
    // workflow model.
    let mut substantial_input = software_change_initial_input(
        software_change_policy_set_b(),
        Some(json!({
            "path": workspace_reference,
            "identity": "software-workspace-A"
        })),
    );
    substantial_input["objective"] = Value::String("already discussed migration".to_owned());
    let _substantial = engine.start(
        "software-substantial",
        SOFTWARE_ALIAS,
        substantial_input.clone(),
    );
    engine.append(
        "software-substantial",
        "known-intent",
        "prior-work",
        json!({
            "intent": "already clarified",
            "design": "already discussed",
            "plan": "already outlined"
        }),
    );
    let substantial_run = engine.authoritative("software-substantial");
    assert_eq!(substantial_run.workflow, minimal.workflow);
    assert_eq!(
        substantial_run.provider_association,
        minimal.provider_association
    );
    assert_eq!(
        substantial_run.initial_input,
        with_allocated_artifact_root(substantial_input, &database, "software-substantial")
    );
    let substantial_show = engine.show("software-substantial");
    assert_eq!(substantial_show.context.len(), 1);
    assert_eq!(
        policy_axes(&substantial_show.initial_input, DESIGN_REVIEW_GATE),
        ["security-boundary"]
    );

    // User steering is appended before the first review and remains visible
    // to every later actor/evaluation request through the durable snapshot.
    engine.append(
        "software-minimal",
        "steering-1",
        "user-steering",
        json!({"instruction": "preserve the existing public API"}),
    );
    require_completed(engine.event("software-minimal", "intent-ready"));
    require_completed(engine.event("software-minimal", "design-ready"));

    let design_approval = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let missing = require_rejected(engine.event("software-minimal", "approved"));
    assert_eq!(missing.code, "software-change-review-incomplete");
    assert!(missing.message.contains("architecture"));
    assert!(missing.message.contains("compatibility"));
    assert_eq!(
        missing.details.as_ref().unwrap()["missing"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // The fresh actor reads the policy axes from show, performs each axis
    // externally, and appends results as ordinary context.  One failed axis
    // still denies with policy-specific actionable feedback.
    let after_missing = engine.show("software-minimal");
    assert_eq!(
        after_missing.current_state,
        StateId::from(DESIGN_REVIEW_GATE)
    );
    let axes = policy_axes(&after_missing.initial_input, DESIGN_REVIEW_GATE);
    assert_eq!(axes, ["architecture", "compatibility"]);
    append_software_evidence(
        &engine,
        "software-minimal",
        "design-architecture-pass",
        DESIGN_REVIEW_GATE,
        &axes[0],
        true,
        json!([]),
    );
    append_software_evidence(
        &engine,
        "software-minimal",
        "design-compatibility-fail",
        DESIGN_REVIEW_GATE,
        &axes[1],
        false,
        json!(["Document the compatibility boundary"]),
    );
    let failed = require_rejected(engine.event("software-minimal", "approved"));
    assert_eq!(failed.code, "software-change-review-incomplete");
    assert!(failed.message.contains("compatibility"));
    assert!(failed.details.as_ref().unwrap()["failed"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "Document the compatibility boundary"));
    assert_eq!(
        failed.details.as_ref().unwrap()["prior_denials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A fresh actor/process reopens SQLite and obtains all handoff data from
    // show only: current work/instructions, policies, evidence, events, and
    // latest feedback.  The external reference is then used to locate work.
    let handoff = engine.show("software-minimal");
    assert_eq!(handoff.current_state, StateId::from(DESIGN_REVIEW_GATE));
    assert!(handoff
        .current_state_instructions
        .contains("external review"));
    assert_eq!(
        policy_axes(&handoff.initial_input, DESIGN_REVIEW_GATE),
        axes
    );
    assert!(handoff
        .context
        .iter()
        .any(|record| record.kind == "review-evidence"));
    assert_requestable(&handoff, "approved", core::TransitionKind::Checked);
    assert_requestable(&handoff, "revise", core::TransitionKind::CheckFree);
    assert_eq!(
        latest_for(&handoff, &design_approval)
            .feedback()
            .unwrap()
            .code,
        "software-change-review-incomplete"
    );
    let recovered_workspace = PathBuf::from(
        handoff.initial_input["external_reference"]["path"]
            .as_str()
            .expect("durable workspace reference"),
    );
    assert_eq!(
        fs::read_to_string(recovered_workspace)?,
        "external software workspace\n"
    );

    // Revision is check-free and remains available even with a provider that
    // cannot be reached.  It must commit without invoking that provider.
    let unavailable = UnavailableGateway::default();
    require_completed(engine.event_with_gateway("software-minimal", "revise", &unavailable));
    assert_eq!(unavailable.call_count(), 0);
    let after_revision = engine.show("software-minimal");
    assert_eq!(after_revision.current_state, StateId::from("design"));
    assert!(after_revision.current_state_instructions.contains("design"));
    assert_eq!(
        policy_axes(&after_revision.initial_input, DESIGN_REVIEW_GATE),
        axes
    );
    assert!(after_revision
        .context
        .iter()
        .any(|record| record.kind == "review-evidence"));
    assert_requestable(
        &after_revision,
        "design-ready",
        core::TransitionKind::Checked,
    );
    assert_eq!(
        latest_for(&after_revision, &design_approval)
            .feedback()
            .unwrap()
            .code,
        "software-change-review-incomplete"
    );
    let recovered_after_revision = PathBuf::from(
        after_revision.initial_input["external_reference"]["path"]
            .as_str()
            .expect("workspace reference after revision handoff"),
    );
    assert_eq!(
        fs::read_to_string(recovered_after_revision)?,
        "external software workspace\n"
    );

    // The actor follows the stored design instructions, adds steering and the
    // corrected evidence, then repeats the checked transition.  The second
    // denial above already demonstrated that the provider received prior
    // durable lineage; the later allow supersedes all earlier denials.
    engine.append(
        "software-minimal",
        "steering-2",
        "user-steering",
        json!({"instruction": "include compatibility test coverage"}),
    );
    append_software_evidence(
        &engine,
        "software-minimal",
        "design-compatibility-pass",
        DESIGN_REVIEW_GATE,
        "compatibility",
        true,
        json!([]),
    );
    let steering_request = engine.evaluation_request(
        "software-minimal",
        Transition::checked("design", "design-ready", DESIGN_REVIEW_GATE),
    );
    assert!(steering_request.context.iter().any(|record| {
        record.kind == "user-steering"
            && record.data["instruction"] == "include compatibility test coverage"
    }));
    require_completed(engine.event("software-minimal", "design-ready"));
    require_completed(engine.event("software-minimal", "approved"));
    let after_allow = engine.show("software-minimal");
    assert_eq!(after_allow.current_state, StateId::from("plan"));
    assert!(
        after_allow
            .context
            .iter()
            .filter(|record| record.kind == "user-steering")
            .count()
            >= 2
    );
    assert!(latest_for(&after_allow, &design_approval).is_allow());
    assert!(
        after_allow
            .latest_evaluations
            .iter()
            .filter(|evaluation| evaluation.transition.same_lineage(&design_approval))
            .count()
            == 1
    );

    // Complete every remaining software-change gate with externally produced
    // per-axis evidence.  The provider only validates the durable evidence;
    // it does not perform semantic review or generate a prompt.
    require_completed(engine.event("software-minimal", "plan-ready"));
    append_software_evidence(
        &engine,
        "software-minimal",
        "plan-coverage-pass",
        PLAN_REVIEW_GATE,
        "coverage",
        true,
        json!([]),
    );
    require_completed(engine.event("software-minimal", "approved"));
    require_completed(engine.event("software-minimal", "implementation-ready"));
    append_software_evidence(
        &engine,
        "software-minimal",
        "implementation-correctness-pass",
        IMPLEMENTATION_REVIEW_GATE,
        "correctness",
        true,
        json!([]),
    );
    require_completed(engine.event("software-minimal", "approved"));
    append_software_evidence(
        &engine,
        "software-minimal",
        "validation-regression-pass",
        VALIDATION_GATE,
        "regression",
        true,
        json!([]),
    );
    require_completed(engine.event("software-minimal", "passed"));
    let final_show = engine.show("software-minimal");
    assert_eq!(final_show.current_state, StateId::from("end"));
    assert_eq!(final_show.lifecycle, Lifecycle::Final);
    assert!(final_show.requestable_events.is_empty());
    Ok(())
}

#[test]
fn software_change_stored_association_and_workflow_snapshot_evolution_is_durable(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = tempdir()?;
    let wrapper_dir = tempdir()?;
    let wrapper = wrapper_dir.path().join("stored-provider.sh");
    let request_log = wrapper_dir.path().join("requests.json");
    let software_fixture = fixture_binary("software-change-provider");
    let policy_fixture = fixture_binary("policy-document-provider");
    let original_workflow = software_change_workflow();
    write_provider_wrapper(
        &wrapper,
        &software_fixture,
        &request_log,
        &original_workflow,
        false,
    );
    let database = state.path().join("evolution.sqlite");
    let first_resolver = resolver([(
        "A",
        ProviderInvocation::new("/bin/sh", [wrapper.to_string_lossy().into_owned()]),
    )]);
    let first_engine = Composition::new(&database, first_resolver);
    let initial_input = software_change_initial_input(software_change_policy_set_a(), None);
    let run = first_engine.start("association-stable", "A", initial_input);
    assert_eq!(run.workflow, original_workflow);
    assert_eq!(run.provider_association.as_json()["command"], "/bin/sh");

    // Alias A is changed to a different provider.  A new run follows the new
    // alias, while the existing run retains its durable command/args pair.
    let switched_resolver = resolver([
        (
            "A",
            ProviderInvocation::new(
                policy_fixture.to_string_lossy().into_owned(),
                Vec::<String>::new(),
            ),
        ),
        (
            "B",
            ProviderInvocation::new(
                software_fixture.to_string_lossy().into_owned(),
                Vec::<String>::new(),
            ),
        ),
    ]);
    let switched_engine = Composition::new(&database, switched_resolver);
    let switched = switched_engine.start(
        "association-new-run",
        "A",
        policy_document_initial_input(
            "audit",
            "/tmp/existing-document.md",
            json!([]),
            json!([]),
            "independent",
        ),
    );
    assert_eq!(switched.workflow, policy_document_workflow());
    assert_eq!(
        switched_engine
            .authoritative("association-stable")
            .provider_association
            .as_json()["command"],
        "/bin/sh"
    );
    assert_eq!(
        switched_engine
            .show("association-stable")
            .workflow_id
            .as_str(),
        "software-change"
    );

    // The implementation currently reached by the stored command changes its
    // describe output, but show must retain the creation snapshot.  The same
    // changed implementation then reports unsupported for the stored checked
    // action; that error must not create state/history/lineage effects.
    let mut changed_workflow = original_workflow.clone();
    changed_workflow.id = core::WorkflowId::from("changed-describe");
    write_provider_wrapper(
        &wrapper,
        &software_fixture,
        &request_log,
        &changed_workflow,
        true,
    );
    let changed_description = switched_engine
        .gateway
        .describe(&run.provider_association, None)
        .expect("changed provider describe response");
    assert_eq!(changed_description.id.as_str(), "changed-describe");

    let before_show = switched_engine.show("association-stable");
    let before_run = switched_engine.authoritative("association-stable");
    let before_history = switched_engine.history("association-stable");
    let unsupported = require_error(switched_engine.event("association-stable", "intent-ready"));
    assert_eq!(unsupported.code, "provider-unsupported");
    let after_show = switched_engine.show("association-stable");
    let after_run = switched_engine.authoritative("association-stable");
    let after_history = switched_engine.history("association-stable");
    assert_eq!(after_show, before_show);
    assert_eq!(after_run, before_run);
    assert_eq!(after_history, before_history);
    assert!(after_show.latest_evaluations.is_empty());

    let request: Value = serde_json::from_str(&fs::read_to_string(&request_log)?)?;
    assert_eq!(request["operation"], "evaluate");
    assert_eq!(request["workflow"]["id"], "software-change");
    assert_ne!(request["workflow"]["id"], "changed-describe");
    assert_eq!(request["transition"]["event"], "intent-ready");
    Ok(())
}

#[test]
fn policy_document_reference_workflow_draft_audit_and_current_conformance_end_to_end(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = tempdir()?;
    let documents = tempdir()?;
    let database = state.path().join("policy-document.sqlite");
    let engine = Composition::new(&database, reference_resolver());

    // Draft mode: deterministic failure, a fresh-actor handoff, semantic
    // failure/revision cycles, prior findings, deterministic regression, and
    // successful finalization all use one provider/topology.
    let readme = documents.path().join("README.md");
    fs::write(&readme, "# Loop Engine\n")?;
    let readme_input = readme_policy_input(readme.to_string_lossy(), "draft");
    let draft = engine.start("document-draft", DOCUMENT_ALIAS, readme_input.clone());
    assert_eq!(draft.workflow, policy_document_workflow());
    let draft_show = engine.show("document-draft");
    assert_eq!(
        draft_show.initial_input,
        with_allocated_artifact_root(readme_input, &database, "document-draft")
    );
    assert_eq!(draft_show.initial_input["mode"], "draft");
    let recovered_readme = PathBuf::from(
        draft_show.initial_input["document"]["path"]
            .as_str()
            .expect("durable README reference"),
    );
    assert!(recovered_readme.ends_with("README.md"));

    require_completed(engine.event("document-draft", "ready"));
    let deterministic_transition =
        Transition::checked("deterministic-review", "passed", "semantic-review");
    let deterministic_failure = require_rejected(engine.event("document-draft", "passed"));
    assert_eq!(
        deterministic_failure.code,
        "policy-document-deterministic-failed"
    );
    assert!(deterministic_failure
        .message
        .contains("readme-start-command"));
    assert!(deterministic_failure.details.as_ref().unwrap()["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["id"] == "readme-start-command"));
    let denied_show = engine.show("document-draft");
    assert_eq!(
        denied_show.current_state,
        StateId::from("deterministic-review")
    );
    assert_eq!(
        latest_for(&denied_show, &deterministic_transition)
            .feedback()
            .unwrap()
            .code,
        "policy-document-deterministic-failed"
    );

    // Reopen with a fresh actor/process and recover the external document
    // solely from show; no retained test-harness path is used for this edit.
    require_completed(engine.event("document-draft", "revise"));
    fs::write(
        PathBuf::from(
            engine.show("document-draft").initial_input["document"]["path"]
                .as_str()
                .expect("README path after revision"),
        ),
        "# Loop Engine\ncargo test\n",
    )?;
    require_completed(engine.event("document-draft", "ready"));
    require_completed(engine.event("document-draft", "passed"));
    assert_eq!(
        engine.show("document-draft").current_state,
        StateId::from("semantic-review")
    );

    let semantic_failure = require_rejected(engine.event("document-draft", "passed"));
    assert_eq!(semantic_failure.code, "policy-document-semantic-failed");
    assert!(semantic_failure.message.contains("readme-purpose"));
    assert!(semantic_failure.details.as_ref().unwrap()["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["id"] == "readme-purpose"));

    // A second failed semantic review receives the first durable finding.
    require_completed(engine.event("document-draft", "revise"));
    require_completed(engine.event("document-draft", "ready"));
    require_completed(engine.event("document-draft", "passed"));
    let repeated_semantic_failure = require_rejected(engine.event("document-draft", "passed"));
    assert_eq!(
        repeated_semantic_failure.code,
        "policy-document-semantic-failed"
    );
    assert!(
        repeated_semantic_failure.details.as_ref().unwrap()["prior_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "policy-document-semantic-failed")
    );

    // Make semantic conformance pass, then regress a deterministic rule just
    // before finalization.  The final semantic review must re-run deterministic
    // policy checks against the current external file.
    require_completed(engine.event("document-draft", "revise"));
    let final_document_path = PathBuf::from(
        engine.show("document-draft").initial_input["document"]["path"]
            .as_str()
            .expect("README path before final cycle"),
    );
    fs::write(
        &final_document_path,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    require_completed(engine.event("document-draft", "ready"));
    require_completed(engine.event("document-draft", "passed"));
    assert_eq!(
        engine.show("document-draft").current_state,
        StateId::from("semantic-review")
    );
    fs::write(
        &final_document_path,
        "# Loop Engine\nworkflow coordination\n",
    )?;
    let regression = require_rejected(engine.event("document-draft", "passed"));
    assert_eq!(regression.code, "policy-document-deterministic-failed");
    assert!(regression.message.contains("readme-start-command"));
    assert_eq!(regression.details.as_ref().unwrap()["mode"], "draft");

    require_completed(engine.event("document-draft", "revise"));
    let restored_path = PathBuf::from(
        engine.show("document-draft").initial_input["document"]["path"]
            .as_str()
            .expect("README path before restoration"),
    );
    fs::write(
        restored_path,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    require_completed(engine.event("document-draft", "ready"));
    require_completed(engine.event("document-draft", "passed"));
    require_completed(engine.event("document-draft", "passed"));
    let draft_final = engine.show("document-draft");
    assert_eq!(draft_final.current_state, StateId::from("end"));
    assert_eq!(draft_final.lifecycle, Lifecycle::Final);
    assert!(draft_final.requestable_events.is_empty());
    assert!(draft_final.initial_input["document"]["path"]
        .as_str()
        .unwrap()
        .ends_with("README.md"));

    // Audit mode and AGENTS.md-like policy shape use the exact same core
    // operations and provider protocol.  First keep the document deterministic
    // but semantically invalid to prove independent review ignores lineage.
    let agents = documents.path().join("AGENTS.md");
    fs::write(&agents, "Repository scope\nValidation\n")?;
    let audit_input = agents_policy_input(agents.to_string_lossy(), "audit");
    let audit = engine.start("document-audit", DOCUMENT_ALIAS, audit_input.clone());
    assert_eq!(audit.workflow, draft.workflow);
    assert_eq!(
        engine.show("document-audit").initial_input,
        with_allocated_artifact_root(audit_input, &database, "document-audit")
    );
    assert_eq!(engine.show("document-audit").initial_input["mode"], "audit");
    require_completed(engine.event("document-audit", "ready"));
    require_completed(engine.event("document-audit", "passed"));
    let audit_semantic_failure = require_rejected(engine.event("document-audit", "passed"));
    assert_eq!(
        audit_semantic_failure.code,
        "policy-document-semantic-failed"
    );
    assert!(audit_semantic_failure.message.contains("agents-handoff"));

    // Independent mode performs the same semantic review after a revision but
    // deliberately omits prior lineage from its feedback, with no engine mode.
    require_completed(engine.event("document-audit", "revise"));
    require_completed(engine.event("document-audit", "ready"));
    require_completed(engine.event("document-audit", "passed"));
    let independent_failure = require_rejected(engine.event("document-audit", "passed"));
    assert_eq!(independent_failure.code, "policy-document-semantic-failed");
    assert_eq!(
        independent_failure.details.as_ref().unwrap()["review_mode"],
        "independent"
    );
    assert!(independent_failure.details.as_ref().unwrap()["prior_findings"].is_null());

    require_completed(engine.event("document-audit", "revise"));
    let recovered_agents = PathBuf::from(
        engine.show("document-audit").initial_input["document"]["path"]
            .as_str()
            .expect("durable AGENTS.md reference"),
    );
    fs::write(
        recovered_agents,
        "Repository scope\nValidation\ndurable handoff\n",
    )?;
    require_completed(engine.event("document-audit", "ready"));
    require_completed(engine.event("document-audit", "passed"));
    require_completed(engine.event("document-audit", "passed"));
    let audit_final = engine.show("document-audit");
    assert_eq!(audit_final.current_state, StateId::from("end"));
    assert_eq!(audit_final.lifecycle, Lifecycle::Final);
    assert!(audit_final.requestable_events.is_empty());
    Ok(())
}

#[test]
fn policy_document_provider_policy_shape_neutrality_uses_same_composed_mechanism(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = tempdir()?;
    let documents = tempdir()?;
    let database = state.path().join("shape-neutral.sqlite");
    let engine = Composition::new(&database, reference_resolver());
    let readme = documents.path().join("README.md");
    let agents = documents.path().join("AGENTS.md");
    fs::write(
        &readme,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    fs::write(&agents, "Repository scope\nValidation\ndurable handoff\n")?;

    for (run_id, input, expected_name) in [
        (
            "shape-readme",
            readme_policy_input(readme.to_string_lossy(), "draft"),
            "README.md",
        ),
        (
            "shape-agents",
            agents_policy_input(agents.to_string_lossy(), "audit"),
            "AGENTS.md",
        ),
    ] {
        let run = engine.start(run_id, DOCUMENT_ALIAS, input.clone());
        assert_eq!(run.workflow, policy_document_workflow());
        let show = engine.show(run_id);
        assert_eq!(
            show.initial_input,
            with_allocated_artifact_root(input, &database, run_id)
        );
        assert!(show.initial_input["document"]["path"]
            .as_str()
            .unwrap()
            .ends_with(expected_name));
        require_completed(engine.event(run_id, "ready"));
        require_completed(engine.event(run_id, "passed"));
        require_completed(engine.event(run_id, "passed"));
        assert_eq!(engine.show(run_id).lifecycle, Lifecycle::Final);
    }
    Ok(())
}

fn write_research_artifact(root: &Path, name: &str, value: &Value) {
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize artifact"),
    )
    .expect("write research artifact");
}

#[test]
fn research_reference_workflow_end_to_end_from_clean_durable_state(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = tempdir()?;
    let artifacts = tempdir()?;
    let database = state.path().join("research.sqlite");
    let engine = Composition::new(&database, reference_resolver());
    let input = research_initial_input(
        artifacts.path().to_string_lossy(),
        research_policy_set_a(),
        research_artifact_schemas(),
        research_revision_links(),
        "fixture-1",
    );
    let started = engine.start("research-e2e", RESEARCH_ALIAS, input.clone());
    assert_eq!(started.workflow, research_workflow());
    let initial = engine.show("research-e2e");
    assert_eq!(initial.initial_input, input);
    assert_eq!(initial.current_state, StateId::from("scope"));
    assert_eq!(initial.workflow_id.as_str(), "research");
    assert_requestable(&initial, "scoped", core::TransitionKind::Checked);

    let missing_brief = require_rejected(engine.event("research-e2e", "scoped"));
    assert_eq!(missing_brief.code, "research-schema-invalid");
    assert_eq!(missing_brief.details.as_ref().unwrap()["phase"], "schema");
    assert_eq!(
        engine.show("research-e2e").current_state,
        StateId::from("scope")
    );

    write_research_artifact(
        artifacts.path(),
        "brief.json",
        &research_brief("1", "owner"),
    );
    require_completed(engine.event("research-e2e", "scoped"));
    assert_eq!(
        engine.show("research-e2e").current_state,
        StateId::from("gather")
    );

    write_research_artifact(
        artifacts.path(),
        "sources.json",
        &research_sources("1", "1", "owner"),
    );
    require_completed(engine.event("research-e2e", "gathered"));
    assert_eq!(
        engine.show("research-e2e").current_state,
        StateId::from("verify")
    );

    write_research_artifact(
        artifacts.path(),
        "verification.json",
        &research_verification("1", "1", "owner"),
    );
    let missing_verify = require_rejected(engine.event("research-e2e", "verified"));
    assert_eq!(missing_verify.code, "research-review-incomplete");
    assert_eq!(
        missing_verify.details.as_ref().unwrap()["phase"],
        "evidence"
    );
    assert_eq!(
        engine.show("research-e2e").current_state,
        StateId::from("verify")
    );

    for (index, axis) in ["claim-grounded", "adversarial"].into_iter().enumerate() {
        engine.append_record(
            "research-e2e",
            research_review_context(
                &format!("verify-{axis}"),
                RESEARCH_VERIFY_GATE,
                axis,
                true,
                "",
                "reviewer",
                "agent",
                "verification.json",
                "1",
                "fixture-1",
                (index as u64) + 1,
            ),
        );
    }
    require_completed(engine.event("research-e2e", "verified"));
    assert_eq!(
        engine.show("research-e2e").current_state,
        StateId::from("synthesize")
    );

    write_research_artifact(
        artifacts.path(),
        "report.json",
        &research_report("1", "1", "owner"),
    );
    let missing_synthesize = require_rejected(engine.event("research-e2e", "completed"));
    assert_eq!(missing_synthesize.code, "research-review-incomplete");

    for (index, axis) in ["cited-conclusion", "scope-faithful"]
        .into_iter()
        .enumerate()
    {
        engine.append_record(
            "research-e2e",
            research_review_context(
                &format!("synthesize-{axis}"),
                RESEARCH_SYNTHESIZE_GATE,
                axis,
                true,
                "",
                "reviewer",
                "agent",
                "report.json",
                "1",
                "fixture-1",
                (index as u64) + 3,
            ),
        );
    }
    require_completed(engine.event("research-e2e", "completed"));
    let terminal = engine.show("research-e2e");
    assert_eq!(terminal.current_state, StateId::from("end"));
    assert_eq!(terminal.lifecycle, Lifecycle::Final);
    assert!(terminal.requestable_events.is_empty());
    Ok(())
}

#[cfg(test)]
mod compile_guards {
    use super::*;

    #[test]
    fn resolver_helper_uses_the_real_configuration_boundary() {
        let resolver = resolver([("alias", ProviderInvocation::new("/bin/echo", ["provider"]))]);
        let association = resolver
            .resolve(&ProviderSelector::from("alias"))
            .expect("configured alias");
        let invocation = ProviderInvocation::from_association(&association).expect("association");
        assert_eq!(invocation.command, "/bin/echo");
        assert_eq!(invocation.args, ["provider"]);
    }

    #[test]
    fn document_policy_constructor_remains_opaque_to_core() {
        let input = policy_document_initial_input(
            "draft",
            "/tmp/README.md",
            json!([document_policy("a", "A", "required_text", "A")]),
            json!([]),
            "lineage-aware",
        );
        assert_eq!(input["deterministic_policies"][0]["id"], "a");
    }
}
