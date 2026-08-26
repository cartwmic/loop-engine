use loop_core::{
    AppendContextRequest, CheckedEvaluationSnapshotRequest, CommitTransitionRequest,
    CompleteWorkSlotInvocationRequest, CreateRunRequest, CreateWorkSlotInvocationRequest,
    EvaluationFeedback, HistoryAction, Lifecycle, Persistence, PersistenceConflict,
    PersistenceError, PersistenceRejection, ProviderAssociation, RecordDenialRequest, RunSummary,
    State, TerminateRequest, Timestamp, Transition, TransitionHistoryOutcome, WaiterWrittenStatus,
    WorkSlotBinding, WorkSlotId, Workflow,
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

#[test]
fn unobserved_mutations_refuse_and_self_loop_requires_reobservation(
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = SqlitePersistence::open_in_memory()?;
    adapter.create_run(create_request("run-observation"))?;

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
        .commit_transition(CommitTransitionRequest::new(
            "run-transition-rollback",
            before_run.control_revision,
            "start",
            checked_start_transition("approve", "middle"),
            Lifecycle::Active,
        ))
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
