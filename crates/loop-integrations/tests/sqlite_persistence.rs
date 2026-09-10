use loop_core::{
    operations::event::{execute as execute_event, Request as EventRequest},
    AppendContextRequest, CheckedEvaluationSnapshotRequest, CommitTransitionRequest,
    CompleteWorkSlotInvocationRequest, ContextAppendEffect, CreateRunRequest,
    CreateWorkSlotInvocationRequest, EvaluationFeedback, EvaluationRequest, EvaluationResult,
    HistoryAction, Lifecycle, Persistence, PersistenceConflict, PersistenceError,
    PersistenceRejection, ProviderAssociation, ProviderError, ProviderGateway, RecordDenialRequest,
    RunSummary, State, TerminateRequest, Timestamp, Transition, TransitionHistoryOutcome,
    WaiterWrittenStatus, WorkSlotBinding, WorkSlotId, Workflow,
};
use loop_integrations::SqlitePersistence;
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "test-workflow",
        "start",
        vec![
            State::new("start", "Start", "Begin", false),
            State::new("middle", "Middle", "Continue", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::checked("start", "approve", "middle"),
            Transition::checked("start", "retry", "start"),
            Transition::check_free("middle", "finish", "done"),
        ],
    )
}

fn create_request(id: &str) -> CreateRunRequest {
    CreateRunRequest::new(
        id,
        Some(format!("label-{id}")),
        workflow(),
        ProviderAssociation::new(json!({"command": "/bin/test", "args": []})),
        json!({"objective": "durable"}),
        "start",
        Lifecycle::Active,
        Timestamp::from_unix_millis(100),
        "test-provider",
        Some("/allocated/run-dir".to_owned()),
    )
}

fn create_observed(
    adapter: &SqlitePersistence,
    run_id: &str,
) -> Result<loop_core::CreateRunResult, Box<dyn std::error::Error>> {
    let created = adapter.create_run(create_request(run_id))?;
    adapter.load_show_data(&run_id.into())?;
    Ok(created)
}

fn append_request(run_id: &str, record_id: &str, created_at: i64) -> AppendContextRequest {
    AppendContextRequest::new(
        run_id,
        record_id,
        "note",
        json!({"record": record_id}),
        Timestamp::from_unix_millis(created_at),
    )
}

fn invocation_create_request(run_id: &str, invocation_id: &str) -> CreateWorkSlotInvocationRequest {
    CreateWorkSlotInvocationRequest::new(
        run_id,
        invocation_id,
        "slot-1",
        WorkSlotBinding::new("/bin/sh", vec!["-c".to_owned(), "exit 0".to_owned()]),
        "digest",
        "subject-a",
        1,
        Timestamp::from_unix_millis(500),
        1_000,
        String::new(),
    )
}

#[test]
fn show_refreshes_terminal_commit_between_snapshot_and_waiter_liveness(
) -> Result<(), Box<dyn std::error::Error>> {
    struct CompletingWaiter<'a>(&'a SqlitePersistence);
    impl loop_core::WorkSlotProcess for CompletingWaiter<'_> {
        type Handle = ();
        fn waiter_alive(&self, _pid: u32) -> bool {
            self.0
                .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
                    "race",
                    "race-invocation",
                    WaiterWrittenStatus::Succeeded,
                    0,
                    Timestamp::from_unix_millis(600),
                    vec![],
                ))
                .unwrap();
            false // exit observed after the commit, but after show's initial read
        }
        fn spawn_wait_invocation(
            &self,
            _: loop_core::WaiterSpawnArgs,
        ) -> Result<loop_core::StartedWaiter<()>, loop_core::ProcessError> {
            panic!("show spawned work")
        }
        fn send_envelope_and_detach(
            &self,
            _: loop_core::StartedWaiter<()>,
            _: &[u8],
        ) -> Result<(), loop_core::ProcessError> {
            panic!("show sent work")
        }
    }
    let directory = tempdir()?;
    let adapter = SqlitePersistence::open(directory.path().join("race.sqlite"))?;
    create_observed(&adapter, "race")?;
    adapter.create_work_slot_invocation(invocation_create_request("race", "race-invocation"))?;
    let result = loop_core::operations::show::execute(
        loop_core::operations::show::Request::new("race"),
        &adapter,
        &CompletingWaiter(&adapter),
        Timestamp::from_unix_millis(700),
    );
    let shown = result.value().expect("completed provider-free show");
    assert_eq!(
        shown.work_slot_invocations[0].status,
        loop_core::ProjectedInvocationStatus::Succeeded
    );
    assert_eq!(shown.work_slot_invocations[0].exit_code, Some(0));
    Ok(())
}

#[test]
fn historical_software_change_public_reads_preserve_absent_capabilities_and_refuse_new_evaluation(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("historical.sqlite");
    let adapter = SqlitePersistence::open(&path)?;
    let historical: Workflow = serde_json::from_str(include_str!(
        "../../../tests/fixtures/software-change-historical-reviewless.json"
    ))?;
    let input = json!({"config_version":"high-rigor-8", "review_policies":{
        "design-review":[{"id":"correctness","description":"original obligation","required_authors":2}]},
        "artifact_root":directory.path().to_str().unwrap()});
    adapter.create_run(CreateRunRequest::new(
        "historical",
        None,
        historical,
        ProviderAssociation::new(
            json!({"command":workspace_integration::binary("software-change"),"args":[]}),
        ),
        input.clone(),
        "implement",
        Lifecycle::Active,
        Timestamp::from_unix_millis(100),
        "software-change",
        Some(directory.path().to_string_lossy().into_owned()),
    ))?;
    adapter.load_show_data(&"historical".into())?;
    let original = json!({"config_version":"high-rigor-8", "subject_revision":"original-3",
        "result":"fail", "findings":"Original reviewer disagreement", "author":{"name":"original reviewer","kind":"agent"}});
    adapter.append_context(AppendContextRequest::new(
        "historical",
        "old-verdict",
        "review-evidence",
        original.clone(),
        Timestamp::from_unix_millis(200),
    ))?;
    let mut invocation = invocation_create_request("historical", "old-invocation");
    invocation.slot_id = "implement".into();
    adapter.create_work_slot_invocation(invocation)?;
    adapter.complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
        "historical",
        "old-invocation",
        WaiterWrittenStatus::Failed,
        7,
        Timestamp::from_unix_millis(600),
        Vec::new(),
    ))?;
    let call = |args: &[&str]| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let output = std::process::Command::new(workspace_integration::binary("loop-engine"))
            .arg("--database")
            .arg(&path)
            .arg("--json")
            .args(args)
            .output()?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        println!("HISTORICAL_ENVELOPE {} {}", args.join(" "), value);
        Ok(value)
    };
    let shown = call(&["show", "historical", "--view", "full"])?;
    assert_eq!(shown["status"], "completed");
    let result = &shown["result"];
    assert_eq!(result["initial_input"], input);
    assert_eq!(result["context"][0]["data"], original);
    assert!(result["initial_input"].get("contract_version").is_none());
    assert!(result["initial_input"].get("criterion_policy").is_none());
    let invocation = &result["work_slot_invocations"][0];
    assert_eq!(invocation["exit_code"], 7);
    assert_eq!(invocation["status"], "failed");
    assert!(invocation
        .get("ownership")
        .is_none_or(serde_json::Value::is_null));
    assert!(invocation
        .get("controls")
        .is_none_or(serde_json::Value::is_null));
    assert!(!result["requestable_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event"] == "revise-plan"));
    let before = call(&["history", "historical"])?;
    assert_eq!(before["status"], "completed");
    let refusal = call(&["event", "historical", "implementation-ready"])?;
    assert_eq!(refusal["status"], "error");
    assert!(refusal
        .to_string()
        .contains("unsupported software-change semantic contract"));
    assert!(refusal.to_string().contains("fixed original provider"));
    assert_eq!(call(&["history", "historical"])?, before);
    let after = call(&["show", "historical", "--view", "full"])?;
    assert_eq!(after["result"]["initial_input"], input);
    assert_eq!(after["result"]["context"], result["context"]);
    assert_eq!(after["result"]["state_visit"], result["state_visit"]);
    // A historical live-shaped record has no ownership protocol. The public
    // controller must refuse before treating its waiter PID as signal authority.
    let mut running = invocation_create_request("historical", "old-running");
    running.slot_id = "implement".into();
    running.waiter_pid = std::process::id();
    adapter.set_current_slot_subject(
        &"historical".into(),
        &"implement".into(),
        "subject-a".to_owned(),
    )?;
    adapter.create_work_slot_invocation(running)?;
    call(&["show", "historical"])?;
    let history = call(&["history", "historical"])?;
    let refused = call(&["cancel-invocation", "historical", "old-running"])?;
    assert_eq!(refused["status"], "rejected");
    assert_eq!(refused["code"], "ownership-unavailable");
    assert!(refused.to_string().contains("unsupported historical"));
    assert_eq!(call(&["history", "historical"])?, history);
    Ok(())
}

fn assert_waiter_written_status_has_no_overrun(status: WaiterWrittenStatus) {
    match status {
        WaiterWrittenStatus::Succeeded | WaiterWrittenStatus::Failed => {}
    }
    assert!(
        !format!("{status:?}").contains("Overrun"),
        "WaiterWrittenStatus must not include Overrun: {status:?}"
    );
}

fn checked_start_transition(event: &str, target: &str) -> Transition {
    Transition::checked("start", event, target)
}

struct StaticGateway {
    result: EvaluationResult,
}

impl StaticGateway {
    fn new(result: EvaluationResult) -> Self {
        Self { result }
    }
}

impl ProviderGateway for StaticGateway {
    fn describe(
        &self,
        _provider: &ProviderAssociation,
        _initial_input: Option<&serde_json::Value>,
    ) -> Result<Workflow, ProviderError> {
        Ok(workflow())
    }

    fn evaluate(
        &self,
        _provider: &ProviderAssociation,
        _request: EvaluationRequest,
    ) -> Result<EvaluationResult, ProviderError> {
        Ok(self.result.clone())
    }
}

#[test]
fn recovery_override_persistence_retains_denial_without_synthetic_allow(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("override.sqlite");
    let run_id = "exception";
    let denied_history;
    {
        let adapter = SqlitePersistence::open(&path)?;
        create_observed(&adapter, run_id)?;
        let denial = execute_event(
            EventRequest::new(run_id, "approve"),
            &StaticGateway::new(EvaluationResult::deny(EvaluationFeedback::new(
                "missing",
                "missing proof",
            ))),
            &adapter,
        );
        assert!(denial.is_rejected());
        denied_history = adapter.load_history(&run_id.into())?;
        let request =
            EventRequest::new(run_id, "approve").with_override(loop_core::StateVisitAttestation {
                state_visit: 0,
                owner: "owner".into(),
                reason: "accept exception".into(),
            });
        let outcome = execute_event(
            request,
            &StaticGateway::new(EvaluationResult::allow_with_context_append(
                ContextAppendEffect::new("MUST-NOT-APPEND", json!({})),
            )),
            &adapter,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        assert_eq!(
            outcome.value().unwrap().run.override_summary.override_count,
            1
        );
    }
    let adapter = SqlitePersistence::open(&path)?;
    let history = adapter.load_history(&run_id.into())?;
    assert_eq!(&history[..denied_history.len()], denied_history.as_slice());
    assert!(matches!(
        history.last().unwrap().action,
        HistoryAction::Transition {
            outcome: TransitionHistoryOutcome::Overridden { .. },
            ..
        }
    ));
    let evaluations = adapter.load_checked_evaluations(&run_id.into())?;
    assert_eq!(evaluations.len(), 1);
    assert!(evaluations[0].is_deny());
    assert!(adapter.load_context_records(&run_id.into())?.is_empty());
    adapter.load_show_data(&run_id.into())?;
    // A later normal check-free final edge retains the permanent exception.
    let terminal = execute_event(
        EventRequest::new(run_id, "finish"),
        &StaticGateway::new(EvaluationResult::Unsupported),
        &adapter,
    );
    assert!(terminal.is_completed());
    let summary = loop_core::OverrideSummary::new(1, Lifecycle::Final);
    assert_eq!(terminal.value().unwrap().run.override_summary, summary);
    assert_eq!(
        adapter.load_show_data(&run_id.into())?.run.override_summary,
        summary
    );
    assert_eq!(adapter.list_runs()?[0].override_summary, summary);
    Ok(())
}

#[test]
fn recovery_override_persistence_invalid_attestation_is_atomic(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "invalid")?;
    let before = adapter.load_history(&"invalid".into())?;
    let mut request = CommitTransitionRequest::new(
        "invalid",
        0.into(),
        "start",
        Transition::checked("start", "approve", "middle"),
        Lifecycle::Active,
    );
    request.exception = Some(loop_core::TransitionOverride {
        attestation: loop_core::StateVisitAttestation {
            state_visit: 1,
            owner: "owner".into(),
            reason: "reason".into(),
        },
        skipped_bound_checks: vec![],
        provider_evaluation: loop_core::SkippedProviderEvaluation::NotPerformed,
    });
    assert!(adapter.commit_transition(request.clone()).is_err());
    request.exception.as_mut().unwrap().attestation.state_visit = 0;
    request.context_append = Some(ContextAppendEffect::new("fake-allow", json!({})));
    assert!(adapter.commit_transition(request).is_err());
    assert_eq!(adapter.load_history(&"invalid".into())?, before);
    assert!(adapter.load_context_records(&"invalid".into())?.is_empty());
    Ok(())
}

#[test]
fn outer_checked_event_persists_provider_effect_allow_and_transition_together(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("event-effect.sqlite");
    let run_id = "run-event-effect";
    let effect = ContextAppendEffect::new(
        "accepted-intent-snapshot",
        json!({"schema_version": 1, "intent_revision": "r1"}),
    );

    {
        let adapter = SqlitePersistence::open(&path)?;
        create_observed(&adapter, run_id)?;
        let outcome = execute_event(
            EventRequest::new(run_id, "approve"),
            &StaticGateway::new(EvaluationResult::allow_with_context_append(effect.clone())),
            &adapter,
        );
        assert!(outcome.is_completed(), "checked event failed: {outcome:?}");
    }

    let reopened = SqlitePersistence::open(&path)?;
    let run = reopened.load_authoritative_run(&run_id.into())?;
    assert_eq!(run.current_state.as_str(), "middle");
    assert_eq!(run.lifecycle, Lifecycle::Active);
    assert_eq!(run.control_revision.as_u64(), 1);
    assert_eq!(run.last_sequence.as_u64(), 3);

    let context = reopened.load_context_records(&run_id.into())?;
    assert_eq!(context.len(), 1);
    assert_eq!(context[0].kind, effect.kind);
    assert_eq!(context[0].data, effect.data);
    assert_eq!(context[0].sequence.as_u64(), 2);
    assert!(!context[0].id.as_str().is_empty());

    let history = reopened.load_history(&run_id.into())?;
    assert_eq!(history.len(), 3);
    assert!(matches!(
        history[1].action,
        HistoryAction::ContextAppended { .. }
    ));
    assert!(matches!(
        history[2].action,
        HistoryAction::Transition {
            ref transition,
            outcome: TransitionHistoryOutcome::Committed,
        } if transition == &checked_start_transition("approve", "middle")
    ));

    let evaluations = reopened.load_checked_evaluations(&run_id.into())?;
    assert_eq!(evaluations.len(), 1);
    assert!(evaluations[0].is_allow());
    assert_eq!(evaluations[0].sequence.as_u64(), 3);
    assert_eq!(
        evaluations[0].transition,
        checked_start_transition("approve", "middle")
    );
    Ok(())
}

#[test]
fn outer_checked_event_deny_and_unsupported_do_not_persist_provider_effects_or_transitions(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let deny_path = directory.path().join("event-deny.sqlite");
    let deny_run_id = "run-event-deny";
    let feedback = EvaluationFeedback::new("needs-work", "Revise before approval");
    {
        let adapter = SqlitePersistence::open(&deny_path)?;
        create_observed(&adapter, deny_run_id)?;
        let outcome = execute_event(
            EventRequest::new(deny_run_id, "approve"),
            &StaticGateway::new(EvaluationResult::deny(feedback.clone())),
            &adapter,
        );
        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().expect("deny feedback").code, "needs-work");
    }
    let deny_reopened = SqlitePersistence::open(&deny_path)?;
    let deny_run = deny_reopened.load_authoritative_run(&deny_run_id.into())?;
    assert_eq!(deny_run.current_state.as_str(), "start");
    assert_eq!(deny_run.control_revision.as_u64(), 0);
    assert_eq!(deny_run.last_sequence.as_u64(), 2);
    assert!(deny_reopened
        .load_context_records(&deny_run_id.into())?
        .is_empty());
    let deny_evaluations = deny_reopened.load_checked_evaluations(&deny_run_id.into())?;
    assert_eq!(deny_evaluations.len(), 1);
    assert_eq!(deny_evaluations[0].feedback(), Some(&feedback));
    assert!(deny_evaluations[0].is_deny());
    assert!(matches!(
        deny_reopened.load_history(&deny_run_id.into())?[1].action,
        HistoryAction::Transition {
            outcome: TransitionHistoryOutcome::Denied { .. },
            ..
        }
    ));

    let unsupported_path = directory.path().join("event-unsupported.sqlite");
    let unsupported_run_id = "run-event-unsupported";
    {
        let adapter = SqlitePersistence::open(&unsupported_path)?;
        create_observed(&adapter, unsupported_run_id)?;
        let outcome = execute_event(
            EventRequest::new(unsupported_run_id, "approve"),
            &StaticGateway::new(EvaluationResult::Unsupported),
            &adapter,
        );
        assert!(outcome.is_error());
        assert_eq!(
            outcome.issue().expect("unsupported issue").code,
            "provider-unsupported"
        );
    }
    let unsupported_reopened = SqlitePersistence::open(&unsupported_path)?;
    let unsupported_run =
        unsupported_reopened.load_authoritative_run(&unsupported_run_id.into())?;
    assert_eq!(unsupported_run.current_state.as_str(), "start");
    assert_eq!(unsupported_run.control_revision.as_u64(), 0);
    assert_eq!(unsupported_run.last_sequence.as_u64(), 1);
    assert!(unsupported_reopened
        .load_context_records(&unsupported_run_id.into())?
        .is_empty());
    assert!(unsupported_reopened
        .load_checked_evaluations(&unsupported_run_id.into())?
        .is_empty());
    assert_eq!(
        unsupported_reopened
            .load_history(&unsupported_run_id.into())?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn context_effects_are_rejected_for_check_free_commits_without_writing(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    let run_id = "run-check-free-effect";
    create_observed(&adapter, run_id)?;

    let error = adapter
        .commit_transition(
            CommitTransitionRequest::new(
                run_id,
                0_u64.into(),
                "start",
                Transition::check_free("start", "finish", "middle"),
                Lifecycle::Active,
            )
            .with_context_append(ContextAppendEffect::new(
                "provider-context",
                json!({"value": "opaque"}),
            )),
        )
        .unwrap_err();
    assert!(matches!(error, PersistenceError::Failure(_)));
    assert_eq!(error.code(), "persistence-failure");
    assert_eq!(
        adapter
            .load_authoritative_run(&run_id.into())?
            .current_state,
        "start".into()
    );
    assert!(adapter.load_context_records(&run_id.into())?.is_empty());
    assert_eq!(adapter.load_history(&run_id.into())?.len(), 1);
    Ok(())
}

#[test]
fn unobserved_mutations_refuse_and_self_loop_requires_reobservation(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    adapter.create_run(create_request("run-observation"))?;

    assert!(!adapter.observation_is_current(&"run-observation".into(), 0_u64.into())?);
    let passive = adapter.load_status_data(&"run-observation".into())?;
    assert_eq!(passive.run.id.as_str(), "run-observation");
    assert!(!adapter.observation_is_current(&"run-observation".into(), 0_u64.into())?);
    for error in [
        adapter
            .append_context(append_request("run-observation", "ctx", 200))
            .unwrap_err(),
        adapter
            .commit_transition(CommitTransitionRequest::new(
                "run-observation",
                0_u64.into(),
                "start",
                checked_start_transition("retry", "start"),
                Lifecycle::Active,
            ))
            .unwrap_err(),
        adapter
            .terminate(TerminateRequest::new("run-observation"))
            .unwrap_err(),
        adapter
            .create_work_slot_invocation(invocation_create_request(
                "run-observation",
                "inv-unobserved",
            ))
            .unwrap_err(),
    ] {
        assert_eq!(error.code(), "run-not-observed");
    }
    assert_eq!(adapter.load_history(&"run-observation".into())?.len(), 1);
    assert_eq!(
        adapter
            .load_authoritative_run(&"run-observation".into())?
            .current_state
            .as_str(),
        "start"
    );

    adapter.load_show_data(&"run-observation".into())?;
    assert!(adapter.observation_is_current(&"run-observation".into(), 0_u64.into())?);
    adapter.create_work_slot_invocation(invocation_create_request(
        "run-observation",
        "inv-observed",
    ))?;
    let self_loop = adapter.commit_transition(CommitTransitionRequest::new(
        "run-observation",
        0_u64.into(),
        "start",
        checked_start_transition("retry", "start"),
        Lifecycle::Active,
    ))?;
    assert_eq!(self_loop.run.control_revision.as_u64(), 1);
    assert!(!adapter.observation_is_current(&"run-observation".into(), 1_u64.into())?);

    let stale_append = adapter
        .append_context(append_request("run-observation", "ctx-after-loop", 300))
        .unwrap_err();
    let stale_termination = adapter
        .terminate(TerminateRequest::new("run-observation"))
        .unwrap_err();
    let stale_invocation = adapter
        .create_work_slot_invocation(invocation_create_request(
            "run-observation",
            "inv-after-loop",
        ))
        .unwrap_err();
    assert_eq!(stale_append.code(), "run-not-observed");
    assert_eq!(stale_termination.code(), "run-not-observed");
    assert_eq!(stale_invocation.code(), "run-not-observed");
    assert_eq!(adapter.load_history(&"run-observation".into())?.len(), 3);
    Ok(())
}

#[test]
fn durable_round_trip_preserves_order_history_context_and_evaluations(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("loop.sqlite");
    {
        let adapter = SqlitePersistence::open(&path)?;
        let created = create_observed(&adapter, "run-1")?;
        assert_eq!(created.run.control_revision.as_u64(), 0);
        assert_eq!(created.run.last_sequence.as_u64(), 1);

        let appended = adapter.append_context(append_request("run-1", "ctx-1", 200))?;
        assert_eq!(appended.context.sequence.as_u64(), 2);
        assert_eq!(appended.run.control_revision.as_u64(), 0);

        let denied = adapter.record_denial(RecordDenialRequest::new(
            "run-1",
            appended.run.control_revision,
            "start",
            checked_start_transition("approve", "middle"),
            EvaluationFeedback::new("needs-work", "Revise first"),
        ))?;
        assert_eq!(denied.evaluation.sequence.as_u64(), 3);
        assert_eq!(denied.run.control_revision.as_u64(), 0);

        let appended_again = adapter.append_context(append_request("run-1", "ctx-2", 400))?;
        assert_eq!(appended_again.context.sequence.as_u64(), 4);

        adapter.load_show_data(&"run-1".into())?;
        let self_loop = adapter.commit_transition(CommitTransitionRequest::new(
            "run-1",
            appended_again.run.control_revision,
            "start",
            checked_start_transition("retry", "start"),
            Lifecycle::Active,
        ))?;
        assert_eq!(self_loop.run.current_state.as_str(), "start");
        assert_eq!(self_loop.run.control_revision.as_u64(), 1);
        assert_eq!(self_loop.run.last_sequence.as_u64(), 5);

        adapter.load_show_data(&"run-1".into())?;
        let committed = adapter.commit_transition(CommitTransitionRequest::new(
            "run-1",
            self_loop.run.control_revision,
            "start",
            checked_start_transition("approve", "middle"),
            Lifecycle::Active,
        ))?;
        assert_eq!(committed.run.current_state.as_str(), "middle");
        assert_eq!(committed.run.control_revision.as_u64(), 2);
        assert_eq!(committed.run.last_sequence.as_u64(), 6);

        adapter.load_show_data(&"run-1".into())?;
        let final_transition = adapter.commit_transition(CommitTransitionRequest::new(
            "run-1",
            committed.run.control_revision,
            "middle",
            Transition::check_free("middle", "finish", "done"),
            Lifecycle::Final,
        ))?;
        assert_eq!(final_transition.run.lifecycle, Lifecycle::Final);
        assert_eq!(final_transition.run.control_revision.as_u64(), 3);
        assert_eq!(final_transition.run.last_sequence.as_u64(), 7);

        let history = adapter.load_history(&"run-1".into())?;
        assert_eq!(
            history
                .iter()
                .map(|entry| entry.sequence.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7]
        );
        assert!(matches!(history[0].action, HistoryAction::RunCreated));
        assert!(matches!(
            history[2].action,
            HistoryAction::Transition {
                outcome: TransitionHistoryOutcome::Denied { .. },
                ..
            }
        ));

        let evaluations = adapter.load_checked_evaluations(&"run-1".into())?;
        assert_eq!(
            evaluations
                .iter()
                .map(|evaluation| evaluation.sequence.as_u64())
                .collect::<Vec<_>>(),
            vec![3, 5, 6]
        );
        assert!(evaluations[0].is_deny());
        assert!(evaluations[1].is_allow());
        assert!(evaluations[2].is_allow());

        let context = adapter.load_context_records(&"run-1".into())?;
        assert_eq!(
            context
                .iter()
                .map(|record| record.sequence.as_u64())
                .collect::<Vec<_>>(),
            vec![2, 4]
        );
        let show = adapter.load_show_data(&"run-1".into())?;
        assert_eq!(show.run, final_transition.run);
        assert_eq!(show.checked_evaluations, evaluations);
    }

    // A new connection sees all state after the original adapter is dropped.
    let reopened = SqlitePersistence::open(&path)?;
    let run = reopened.load_authoritative_run(&"run-1".into())?;
    assert_eq!(run.lifecycle, Lifecycle::Final);
    assert_eq!(run.current_state.as_str(), "done");
    assert_eq!(run.last_sequence.as_u64(), 7);
    assert_eq!(reopened.load_context_records(&"run-1".into())?.len(), 2);
    assert_eq!(reopened.load_history(&"run-1".into())?.len(), 7);
    assert_eq!(reopened.load_checked_evaluations(&"run-1".into())?.len(), 3);
    assert_eq!(reopened.load_show_data(&"run-1".into())?.run, run);
    assert_eq!(reopened.list_runs()?.len(), 1);
    Ok(())
}

#[test]
fn termination_is_atomic_and_advances_only_control_revision(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("termination.sqlite");
    let adapter = SqlitePersistence::open(&path)?;
    let created = create_observed(&adapter, "run-terminate")?;
    let terminated = adapter.terminate(TerminateRequest::new("run-terminate"))?;
    assert_eq!(terminated.run.lifecycle, Lifecycle::Terminated);
    assert_eq!(terminated.run.control_revision.as_u64(), 1);
    assert_eq!(terminated.run.last_sequence.as_u64(), 2);
    assert!(matches!(
        terminated.history.action,
        HistoryAction::Terminated
    ));

    let error = adapter
        .terminate(TerminateRequest::new("run-terminate"))
        .unwrap_err();
    assert_eq!(
        error,
        PersistenceError::Rejected(PersistenceRejection::RunNotActive {
            run_id: "run-terminate".into(),
            lifecycle: Lifecycle::Terminated,
        })
    );
    assert_eq!(adapter.load_history(&"run-terminate".into())?.len(), 2);
    assert_eq!(created.run.control_revision.as_u64(), 0);
    Ok(())
}

#[test]
fn stale_conditional_writes_after_termination_are_conflicts_and_noops(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-stale-after-termination")?;
    let snapshot =
        adapter.load_checked_evaluation_snapshot(CheckedEvaluationSnapshotRequest::new(
            "run-stale-after-termination",
            checked_start_transition("approve", "middle"),
        ))?;
    assert_eq!(snapshot.observed_control_revision.as_u64(), 0);

    let terminated = adapter.terminate(TerminateRequest::new("run-stale-after-termination"))?;
    assert_eq!(terminated.run.lifecycle, Lifecycle::Terminated);
    assert_eq!(terminated.run.control_revision.as_u64(), 1);

    let transition_error = adapter
        .commit_transition(CommitTransitionRequest::new(
            "run-stale-after-termination",
            snapshot.observed_control_revision,
            "start",
            snapshot.transition.clone(),
            Lifecycle::Active,
        ))
        .unwrap_err();
    assert!(matches!(
        transition_error,
        PersistenceError::Conflict(PersistenceConflict::LifecycleMismatch {
            expected: Lifecycle::Active,
            observed: Lifecycle::Terminated,
        })
    ));

    let denial_error = adapter
        .record_denial(RecordDenialRequest::new(
            "run-stale-after-termination",
            snapshot.observed_control_revision,
            "start",
            snapshot.transition,
            EvaluationFeedback::new("blocked", "run terminated"),
        ))
        .unwrap_err();
    assert!(matches!(
        denial_error,
        PersistenceError::Conflict(PersistenceConflict::LifecycleMismatch {
            expected: Lifecycle::Active,
            observed: Lifecycle::Terminated,
        })
    ));

    let run = adapter.load_authoritative_run(&"run-stale-after-termination".into())?;
    assert_eq!(run.lifecycle, Lifecycle::Terminated);
    assert_eq!(run.current_state, terminated.run.current_state);
    assert_eq!(run.control_revision, terminated.run.control_revision);
    assert_eq!(run.last_sequence, terminated.run.last_sequence);
    let history = adapter.load_history(&"run-stale-after-termination".into())?;
    assert_eq!(history.len(), 2);
    assert!(matches!(history[0].action, HistoryAction::RunCreated));
    assert!(matches!(history[1].action, HistoryAction::Terminated));
    assert!(adapter
        .load_checked_evaluations(&"run-stale-after-termination".into())?
        .is_empty());
    Ok(())
}

#[test]
fn snapshot_is_one_boundary_and_returns_all_checked_evaluations(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    let created = create_observed(&adapter, "run-snapshot")?;
    let appended = adapter.append_context(append_request("run-snapshot", "ctx", 200))?;
    let denied = adapter.record_denial(RecordDenialRequest::new(
        "run-snapshot",
        appended.run.control_revision,
        "start",
        checked_start_transition("approve", "middle"),
        EvaluationFeedback::new("blocked", "not yet"),
    ))?;
    let snapshot =
        adapter.load_checked_evaluation_snapshot(CheckedEvaluationSnapshotRequest::new(
            "run-snapshot",
            checked_start_transition("approve", "middle"),
        ))?;
    assert_eq!(snapshot.run, denied.run);
    assert_eq!(
        snapshot.observed_control_revision,
        denied.run.control_revision
    );
    assert_eq!(snapshot.context.len(), 1);
    assert_eq!(snapshot.checked_evaluations.len(), 1);
    assert_eq!(
        snapshot.checked_evaluations[0].sequence,
        denied.evaluation.sequence
    );

    let unavailable = adapter
        .load_checked_evaluation_snapshot(CheckedEvaluationSnapshotRequest::new(
            "run-snapshot",
            Transition::check_free("middle", "finish", "done"),
        ))
        .unwrap_err();
    assert!(matches!(
        unavailable,
        PersistenceError::Conflict(PersistenceConflict::ExactTransitionUnavailable { .. })
    ));
    assert_eq!(created.run.last_sequence.as_u64(), 1);
    Ok(())
}

#[test]
fn independent_instances_enforce_conditional_conflicts() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("concurrency.sqlite");
    let first = SqlitePersistence::open(&path)?;
    let second = SqlitePersistence::open(&path)?;
    create_observed(&first, "run-concurrent")?;
    let observed = first.load_authoritative_run(&"run-concurrent".into())?;

    let committed = second.commit_transition(CommitTransitionRequest::new(
        "run-concurrent",
        observed.control_revision,
        "start",
        checked_start_transition("retry", "start"),
        Lifecycle::Active,
    ))?;
    assert_eq!(committed.run.control_revision.as_u64(), 1);

    let stale = first
        .commit_transition(CommitTransitionRequest::new(
            "run-concurrent",
            observed.control_revision,
            "start",
            checked_start_transition("retry", "start"),
            Lifecycle::Active,
        ))
        .unwrap_err();
    assert!(matches!(
        stale,
        PersistenceError::Conflict(PersistenceConflict::ControlRevisionMismatch { .. })
    ));
    assert_eq!(second.load_history(&"run-concurrent".into())?.len(), 2);
    Ok(())
}

#[test]
fn required_history_failure_rolls_back_the_complete_append(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("rollback.sqlite");
    let adapter = SqlitePersistence::open(&path)?;
    create_observed(&adapter, "run-rollback")?;
    let raw = Connection::open(&path)?;
    raw.execute_batch(
        "CREATE TRIGGER fail_history_insert
         BEFORE INSERT ON history_entries
         BEGIN
             SELECT RAISE(ABORT, 'forced history failure');
         END;",
    )?;

    let error = adapter
        .append_context(append_request("run-rollback", "ctx-fails", 200))
        .unwrap_err();
    assert!(matches!(error, PersistenceError::Failure(_)));
    drop(raw);

    let run = adapter.load_authoritative_run(&"run-rollback".into())?;
    assert_eq!(run.last_sequence.as_u64(), 1);
    assert_eq!(run.control_revision.as_u64(), 0);
    assert!(adapter
        .load_context_records(&"run-rollback".into())?
        .is_empty());
    assert_eq!(adapter.load_history(&"run-rollback".into())?.len(), 1);
    Ok(())
}

#[test]
fn required_history_failure_rolls_back_complete_transition(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("transition-rollback.sqlite");
    let adapter = SqlitePersistence::open(&path)?;
    let created = create_observed(&adapter, "run-transition-rollback")?;
    let denied = adapter.record_denial(RecordDenialRequest::new(
        "run-transition-rollback",
        created.run.control_revision,
        "start",
        checked_start_transition("approve", "middle"),
        EvaluationFeedback::new("blocked", "preserve lineage"),
    ))?;
    let before_run = adapter.load_authoritative_run(&"run-transition-rollback".into())?;
    let before_history_count = adapter
        .load_history(&"run-transition-rollback".into())?
        .len();
    let before_evaluations = adapter.load_checked_evaluations(&"run-transition-rollback".into())?;
    assert_eq!(before_run, denied.run);
    assert_eq!(before_history_count, 2);
    assert_eq!(before_evaluations.len(), 1);

    let raw = Connection::open(&path)?;
    raw.execute_batch(
        "CREATE TRIGGER fail_history_insert
         BEFORE INSERT ON history_entries
         BEGIN
             SELECT RAISE(ABORT, 'forced history failure');
         END;",
    )?;
    drop(raw);

    let error = adapter
        .commit_transition(
            CommitTransitionRequest::new(
                "run-transition-rollback",
                before_run.control_revision,
                "start",
                checked_start_transition("approve", "middle"),
                Lifecycle::Active,
            )
            .with_context_append(ContextAppendEffect::new(
                "provider-effect",
                json!({"revision": "r1"}),
            )),
        )
        .unwrap_err();
    assert!(matches!(error, PersistenceError::Failure(_)));
    drop(adapter);

    let reopened = SqlitePersistence::open(&path)?;
    let after_run = reopened.load_authoritative_run(&"run-transition-rollback".into())?;
    assert_eq!(after_run.current_state, before_run.current_state);
    assert_eq!(after_run.lifecycle, before_run.lifecycle);
    assert_eq!(after_run.control_revision, before_run.control_revision);
    assert_eq!(after_run.last_sequence, before_run.last_sequence);
    assert_eq!(
        reopened
            .load_history(&"run-transition-rollback".into())?
            .len(),
        before_history_count
    );
    assert!(reopened
        .load_context_records(&"run-transition-rollback".into())?
        .is_empty());
    assert_eq!(
        reopened.load_checked_evaluations(&"run-transition-rollback".into())?,
        before_evaluations
    );
    Ok(())
}

#[test]
fn required_history_failure_rolls_back_run_creation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("create-rollback.sqlite");
    let setup = SqlitePersistence::open(&path)?;
    let raw = Connection::open(&path)?;
    raw.execute_batch(
        "CREATE TRIGGER fail_history_insert
         BEFORE INSERT ON history_entries
         BEGIN
             SELECT RAISE(ABORT, 'forced history failure');
         END;",
    )?;
    drop(raw);

    let error = setup
        .create_run(create_request("run-create-fails"))
        .unwrap_err();
    assert!(matches!(error, PersistenceError::Failure(_)));
    assert!(matches!(
        setup.load_authoritative_run(&"run-create-fails".into()),
        Err(PersistenceError::NotFound { .. })
    ));
    Ok(())
}

#[test]
fn create_run_persists_provider_and_artifact_root_for_list(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    let created = create_observed(&adapter, "run-list-catalog")?;
    let listed = adapter.list_runs()?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id.as_str(), "run-list-catalog");
    assert_eq!(listed[0].provider.as_deref(), Some("test-provider"));
    assert_eq!(
        listed[0].artifact_root.as_deref(),
        Some("/allocated/run-dir")
    );

    let from_run = RunSummary::from(&created.run);
    assert_eq!(from_run.provider, None);
    assert_eq!(from_run.artifact_root, None);
    assert_ne!(from_run.provider, listed[0].provider);
    assert_ne!(from_run.artifact_root, listed[0].artifact_root);
    Ok(())
}

#[test]
fn opening_legacy_catalog_adds_nullable_provider_and_artifact_root_columns(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("legacy.sqlite");
    {
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "CREATE TABLE runs (
                id                           TEXT PRIMARY KEY NOT NULL,
                label                        TEXT,
                workflow_id                  TEXT NOT NULL,
                workflow_json                TEXT NOT NULL,
                provider_association_json    TEXT NOT NULL,
                initial_input_json           TEXT NOT NULL,
                current_state                TEXT NOT NULL,
                lifecycle                    TEXT NOT NULL CHECK (lifecycle IN ('active', 'final', 'terminated')),
                control_revision             INTEGER NOT NULL CHECK (control_revision >= 0),
                last_sequence                INTEGER NOT NULL CHECK (last_sequence >= 1),
                created_at                   INTEGER NOT NULL
            );",
        )?;
        connection.execute(
            "INSERT INTO runs (
                id, label, workflow_id, workflow_json, provider_association_json,
                initial_input_json, current_state, lifecycle, control_revision,
                last_sequence, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                "legacy-run",
                "legacy",
                "test-workflow",
                "{}",
                "{}",
                "{}",
                "start",
                "active",
                0,
                1,
                100,
            ],
        )?;
    }

    let adapter = SqlitePersistence::open(&path)?;
    let listed = adapter.list_runs()?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id.as_str(), "legacy-run");
    assert_eq!(listed[0].provider, None);
    assert_eq!(listed[0].artifact_root, None);
    drop(adapter);

    let connection = Connection::open(&path)?;
    let mut statement = connection.prepare("PRAGMA table_info('runs')")?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    assert!(
        names.iter().any(|name| name == "provider"),
        "provider column missing after open: {names:?}"
    );
    assert!(
        names.iter().any(|name| name == "artifact_root"),
        "artifact_root column missing after open: {names:?}"
    );
    Ok(())
}

#[test]
fn create_running_work_slot_invocation_record() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-create")?;
    let created = adapter.create_work_slot_invocation(invocation_create_request(
        "run-invocation-create",
        "inv-running",
    ))?;
    assert!(created.invocation.status.is_none());
    assert!(created.invocation.exit_code.is_none());
    assert!(created.invocation.completed_at.is_none());
    assert!(matches!(
        created.history.action,
        HistoryAction::InvocationStarted { .. }
    ));

    let loaded = adapter.load_work_slot_invocations(&"run-invocation-create".into())?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].invocation_id.as_str(), "inv-running");
    assert!(loaded[0].status.is_none());
    assert!(matches!(
        created.history.action,
        HistoryAction::InvocationStarted {
            ref invocation_id
        } if invocation_id.as_str() == "inv-running"
    ));
    Ok(())
}

#[test]
fn waiter_terminal_write_succeeded_invocation() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-succeeded")?;
    adapter.create_work_slot_invocation(invocation_create_request(
        "run-invocation-succeeded",
        "inv-succeeded",
    ))?;
    let completed =
        adapter.complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "run-invocation-succeeded",
            "inv-succeeded",
            WaiterWrittenStatus::Succeeded,
            0,
            Timestamp::from_unix_millis(900),
            Vec::new(),
        ))?;
    assert_eq!(
        completed.invocation.status,
        Some(WaiterWrittenStatus::Succeeded)
    );
    assert_eq!(completed.invocation.exit_code, Some(0));
    assert_eq!(
        completed.invocation.completed_at,
        Some(Timestamp::from_unix_millis(900))
    );
    assert!(matches!(
        completed.history.action,
        HistoryAction::InvocationStatusChanged {
            status: WaiterWrittenStatus::Succeeded,
            ..
        }
    ));
    Ok(())
}

#[test]
fn waiter_terminal_write_failed_invocation() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-failed")?;
    adapter.create_work_slot_invocation(invocation_create_request(
        "run-invocation-failed",
        "inv-failed",
    ))?;
    let completed =
        adapter.complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "run-invocation-failed",
            "inv-failed",
            WaiterWrittenStatus::Failed,
            7,
            Timestamp::from_unix_millis(901),
            Vec::new(),
        ))?;
    assert_eq!(
        completed.invocation.status,
        Some(WaiterWrittenStatus::Failed)
    );
    assert_eq!(completed.invocation.exit_code, Some(7));
    assert!(matches!(
        completed.history.action,
        HistoryAction::InvocationStatusChanged {
            status: WaiterWrittenStatus::Failed,
            ..
        }
    ));
    Ok(())
}

#[test]
fn second_invocation_terminal_write_conflicts_and_waiter_cannot_write_overrun(
) -> Result<(), Box<dyn std::error::Error>> {
    assert_waiter_written_status_has_no_overrun(WaiterWrittenStatus::Succeeded);
    assert_waiter_written_status_has_no_overrun(WaiterWrittenStatus::Failed);

    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-conflict")?;
    adapter.create_work_slot_invocation(invocation_create_request(
        "run-invocation-conflict",
        "inv-conflict",
    ))?;
    adapter.complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
        "run-invocation-conflict",
        "inv-conflict",
        WaiterWrittenStatus::Succeeded,
        0,
        Timestamp::from_unix_millis(900),
        Vec::new(),
    ))?;
    let error = adapter
        .complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
            "run-invocation-conflict",
            "inv-conflict",
            WaiterWrittenStatus::Failed,
            1,
            Timestamp::from_unix_millis(901),
            Vec::new(),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        PersistenceError::Conflict(PersistenceConflict::InvocationAlreadyTerminal { .. })
    ));
    let loaded = adapter.load_work_slot_invocations(&"run-invocation-conflict".into())?;
    assert_eq!(loaded[0].status, Some(WaiterWrittenStatus::Succeeded));
    assert_eq!(loaded[0].exit_code, Some(0));
    Ok(())
}

#[test]
fn append_context_does_not_create_invocation_rows() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-append")?;
    adapter.append_context(append_request("run-invocation-append", "ctx-1", 200))?;
    let invocations = adapter.load_work_slot_invocations(&"run-invocation-append".into())?;
    assert!(invocations.is_empty());
    Ok(())
}

#[test]
fn get_and_set_current_slot_subject_replace_on_set_for_invocation_slot(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-subject")?;
    let slot_id = WorkSlotId::new("slot-1");
    let run_id = "run-invocation-subject".into();
    assert_eq!(adapter.get_current_slot_subject(&run_id, &slot_id)?, None);
    adapter.set_current_slot_subject(&run_id, &slot_id, "first".to_owned())?;
    assert_eq!(
        adapter.get_current_slot_subject(&run_id, &slot_id)?,
        Some("first".to_owned())
    );
    adapter.set_current_slot_subject(&run_id, &slot_id, "second".to_owned())?;
    assert_eq!(
        adapter.get_current_slot_subject(&run_id, &slot_id)?,
        Some("second".to_owned())
    );
    Ok(())
}

#[test]
fn create_run_and_commit_transition_persist_slot_subjects_atomically(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    let slot_id = WorkSlotId::new("slot-1");
    let created = adapter.create_run(
        create_request("run-slot-atomic")
            .with_slot_subjects(vec![(slot_id.clone(), "visit-start".to_owned())]),
    )?;
    adapter.load_show_data(&"run-slot-atomic".into())?;
    assert_eq!(created.run.id.as_str(), "run-slot-atomic");
    assert_eq!(
        adapter.get_current_slot_subject(&created.run.id, &slot_id)?,
        Some("visit-start".to_owned())
    );

    let committed = adapter.commit_transition(
        CommitTransitionRequest::new(
            created.run.id.clone(),
            created.run.control_revision,
            created.run.current_state.clone(),
            Transition::checked("start", "retry", "start"),
            Lifecycle::Active,
        )
        .with_slot_subjects(vec![(slot_id.clone(), "visit-next".to_owned())]),
    )?;
    assert_eq!(committed.run.current_state.as_str(), "start");
    assert_eq!(
        adapter.get_current_slot_subject(&created.run.id, &slot_id)?,
        Some("visit-next".to_owned())
    );
    Ok(())
}

#[test]
fn load_history_includes_invocation_actions_in_sequence_order(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-history")?;
    adapter.append_context(append_request("run-invocation-history", "ctx-1", 200))?;
    adapter.create_work_slot_invocation(invocation_create_request(
        "run-invocation-history",
        "inv-history",
    ))?;
    adapter.complete_work_slot_invocation(CompleteWorkSlotInvocationRequest::new(
        "run-invocation-history",
        "inv-history",
        WaiterWrittenStatus::Succeeded,
        0,
        Timestamp::from_unix_millis(900),
        Vec::new(),
    ))?;
    let history = adapter.load_history(&"run-invocation-history".into())?;
    assert_eq!(
        history
            .iter()
            .map(|entry| entry.sequence.as_u64())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(matches!(history[0].action, HistoryAction::RunCreated));
    assert!(matches!(
        history[1].action,
        HistoryAction::ContextAppended { .. }
    ));
    assert!(matches!(
        history[2].action,
        HistoryAction::InvocationStarted { .. }
    ));
    assert!(matches!(
        history[3].action,
        HistoryAction::InvocationStatusChanged {
            status: WaiterWrittenStatus::Succeeded,
            ..
        }
    ));
    Ok(())
}
