use loop_core::{
    operations::event::{execute as execute_event, Request as EventRequest},
    AppendContextRequest, CheckedEvaluationSnapshotRequest, CommitTransitionRequest,
    ContextAppendEffect, CreateRunRequest, EvaluationFeedback, EvaluationRequest, EvaluationResult,
    HistoryAction, Lifecycle, Persistence, PersistenceConflict, PersistenceError,
    PersistenceRejection, ProviderAssociation, ProviderError, ProviderGateway, RecordDenialRequest,
    RunSummary, State, TerminateRequest, Timestamp, Transition, TransitionHistoryOutcome, Workflow,
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
fn append_context_does_not_create_invocation_rows() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    create_observed(&adapter, "run-invocation-append")?;
    adapter.append_context(append_request("run-invocation-append", "ctx-1", 200))?;
    let invocations = adapter.load_work_slot_invocations(&"run-invocation-append".into())?;
    assert!(invocations.is_empty());
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
    Connection::open(&path)?.execute(
        r#"INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json, instruction_digest, subject, waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at) VALUES ('historical', 'old-invocation', 'implement', '{"command":"old-worker","args":[]}', 'digest', 'subject-a', 0, 500, 1000, 'failed', 7, 600)"#,
        [],
    )?;
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
    Ok(())
}
