use super::{decode_json, load_required_run, sqlite_failure, status_read_deadline};
use loop_core::{PersistenceError, RunId};
use rusqlite::{params, Connection, TransactionBehavior};

const STATUS_INVOCATION_LIMIT: i64 = 20;
const STATUS_ASSIGNMENT_LIMIT: i64 = 200;

pub(super) fn read_bounded_observation(
    connection: &mut Connection,
    run_id: &RunId,
    deadline: std::time::Instant,
    arm: bool,
) -> Result<serde_json::Value, PersistenceError> {
    use serde_json::json;
    let behavior = if arm {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(sqlite_failure)?;
    let result = (|| {
        let mut run = load_required_run(&transaction, run_id)?;
        if arm {
            transaction
                .execute(
                    "UPDATE runs SET observed_control_revision = control_revision WHERE id = ?1",
                    params![run_id.as_str()],
                )
                .map_err(sqlite_failure)?;
            run = load_required_run(&transaction, run_id)?;
        }
        let sampled_at_ms = now_ms();
        let state = run
            .workflow
            .states
            .iter()
            .find(|state| state.id == run.current_state);
        let slots = run
            .workflow
            .work_slots
            .iter()
            .filter(|slot| slot.state == run.current_state)
            .collect::<Vec<_>>();
        let events = if run.lifecycle.is_terminal() {
            Vec::new()
        } else {
            run.workflow
                .transitions
                .iter()
                .filter(|edge| edge.source == run.current_state)
                .map(|edge| json!({"event":edge.event,"target":edge.target,"kind":edge.kind}))
                .collect::<Vec<_>>()
        };
        let state_value = state
            .map(|state| {
                json!({
                    "id":state.id,"title":state.title,"instructions":state.instructions,
                    "action_guidance":state.action_guidance,"final":state.is_final
                })
            })
            .unwrap_or(serde_json::Value::Null);
        let mut output = json!({
            "schema_version":1,
            "sampled_at_ms":sampled_at_ms,
            "mutation_armed":arm,
            "delta_sequence":run.last_sequence.as_u64(),
            "run":{
                "run_id":run.id,"label":run.label,"workflow_id":run.workflow.id,
                "lifecycle":run.lifecycle,"current_state":run.current_state,
                "state_visit":run.control_revision.as_u64(),"state":state_value,
                "requestable_events":events,"work_slots":slots,

                "override_summary":run.override_summary
            },
            "invocations":{"state":"unavailable","reason":"not sampled","total":null,"limit":STATUS_INVOCATION_LIMIT,"returned":0,"truncated":false,"items":[]},
            "assignments":{"state":"unavailable","reason":"not sampled","total":null,"limit":STATUS_ASSIGNMENT_LIMIT,"returned":0,"truncated":false,"items":[]},
            "acceptance":{"state":"unknown","reason":"status does not infer provider or driver acceptance"},
            "completeness":{"run":"available","invocations":"unavailable","assignments":"unavailable"}
        });
        if std::time::Instant::now() < deadline {
            match load_advice_policy(&transaction, &run, deadline) {
                Ok(policy) => {
                    output["advice_policy"] = policy;
                    output["completeness"]["advice_policy"] = json!("available");
                }
                Err(error) => {
                    output["advice_policy"] = json!({"state":"unavailable","reason":error});
                    output["completeness"]["advice_policy"] = json!("unavailable");
                }
            }
        } else {
            output["advice_policy"] = json!({"state":"unavailable","reason":"observation deadline reached before advice policy sampling"});
            output["completeness"]["advice_policy"] = json!("unavailable");
        }
        let model_bindings = if std::time::Instant::now() < deadline {
            load_model_bindings(&transaction, &run, deadline)
        } else {
            Err("observation deadline reached before model binding sampling".to_owned())
        };
        match model_bindings {
            Ok(view) => output["model_bindings"] = view,
            Err(reason) => {
                output["model_bindings"] = json!({"state":"unavailable","reason":reason});
                output["completeness"]["model_bindings"] = json!("unavailable");
            }
        }
        if std::time::Instant::now() < deadline {
            match load_status_invocations(&transaction, run_id, sampled_at_ms, deadline) {
                Ok((total, items)) => {
                    let truncated = total > items.len() as u64;
                    output["invocations"] = json!({
                        "state":"available","total":total,"limit":STATUS_INVOCATION_LIMIT,
                        "returned":items.len(),"truncated":truncated,
                        "cursor":items.last().map(|item| item["cursor"].clone()),
                        "items":items
                    });
                    output["completeness"]["invocations"] =
                        json!(if truncated { "partial" } else { "available" });
                }
                Err(error) => {
                    output["invocations"] = json!({"state":"unavailable","reason":error.to_string(),"total":null,"limit":STATUS_INVOCATION_LIMIT,"returned":0,"truncated":false,"items":[]});
                }
            }
        } else {
            output["invocations"]["reason"] =
                json!("observation deadline reached before invocation sampling");
        }
        if std::time::Instant::now() < deadline {
            match load_status_assignments(&transaction, run_id) {
                Ok((total, items)) => {
                    let truncated = total > items.len() as u64;
                    let unindexed =
                        count_unindexed_current_invocations(&transaction, run_id).unwrap_or(1);
                    let complete = !truncated && unindexed == 0;
                    output["assignments"] = json!({
                        "state":if complete {"available"} else {"partial"},"total":total,"limit":STATUS_ASSIGNMENT_LIMIT,
                        "returned":items.len(),"truncated":truncated,"unindexed_invocations":unindexed,
                        "reason":if unindexed > 0 {Some("terminal invocations without indexed assignment facts remain in history")} else {None},
                        "cursor":items.last().map(|item| json!({"slot_id":item["slot_id"],"assignment_id":item["assignment_id"]})),
                        "items":items
                    });
                    output["completeness"]["assignments"] =
                        json!(if complete { "available" } else { "partial" });
                }
                Err(error) => {
                    output["assignments"] = json!({"state":"unavailable","reason":error.to_string(),"total":null,"limit":STATUS_ASSIGNMENT_LIMIT,"returned":0,"truncated":false,"items":[]});
                }
            }
        } else {
            output["assignments"]["reason"] =
                json!("observation deadline reached before assignment sampling");
        }
        Ok(output)
    })();
    match result {
        Ok(mut output) => {
            if std::time::Instant::now() >= deadline {
                output["completeness"]["deadline"] = json!("partial");
                if arm {
                    let _ = transaction.rollback();
                    return Err(status_read_deadline());
                }
                let _ = transaction.rollback();
                Ok(output)
            } else {
                transaction.commit().map_err(sqlite_failure)?;
                Ok(output)
            }
        }
        Err(error) => {
            let _ = transaction.rollback();
            Err(error)
        }
    }
}

// Read only records of one kind, never the full context/captures on the bounded lane.
fn load_records_of_kind(
    connection: &Connection,
    run_id: &RunId,
    kind: &str,
    what: &str,
    deadline: std::time::Instant,
) -> Result<Vec<loop_core::ContextRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT record_id, sequence, data_json, created_at FROM context_records
         WHERE run_id = ?1 AND kind = ?2 ORDER BY sequence ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id.as_str(), kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut context = Vec::new();
    for row in rows {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "observation deadline reached during {what} sampling"
            ));
        }
        let (id, sequence, data, created_at) = row.map_err(|error| error.to_string())?;
        context.push(loop_core::ContextRecord::new(
            id,
            kind,
            decode_json(&data, what).map_err(|error| error.to_string())?,
            super::from_sqlite_u64(sequence, "semantic sequence")
                .map_err(|error| error.to_string())?
                .into(),
            loop_core::Timestamp::from_unix_millis(created_at),
        ));
    }
    Ok(context)
}

// Read only policy changes, never the full context/captures on the bounded lane.
fn load_advice_policy(
    connection: &Connection,
    run: &loop_core::Run,
    deadline: std::time::Instant,
) -> Result<serde_json::Value, String> {
    let context = load_records_of_kind(
        connection,
        &run.id,
        loop_core::ADVICE_POLICY_CHANGE_RECORD_KIND,
        "advice policy change",
        deadline,
    )?;
    let policy = loop_core::effective_advice_policy(&run.initial_input, &run.workflow, &context)?;
    serde_json::to_value(policy).map_err(|error| error.to_string())
}

// Read only binding amendments; the bound original comes from the frozen input.
fn load_model_bindings(
    connection: &Connection,
    run: &loop_core::Run,
    deadline: std::time::Instant,
) -> Result<serde_json::Value, String> {
    let context = load_records_of_kind(
        connection,
        &run.id,
        loop_core::MODEL_BINDING_AMENDMENT_RECORD_KIND,
        "model binding amendment",
        deadline,
    )?;
    let view = loop_core::model_bindings_view(&run.initial_input, &context);
    match view["reason"].as_str() {
        Some(reason) if view["state"] == "unavailable" => Err(reason.to_owned()),
        _ => Ok(view),
    }
}

fn load_status_invocations(
    connection: &Connection,
    run_id: &RunId,
    sampled_at_ms: u64,
    deadline: std::time::Instant,
) -> Result<(u64, Vec<serde_json::Value>), PersistenceError> {
    use serde_json::json;
    let total = connection
        .query_row(
            "SELECT COUNT(*) FROM work_slot_invocations WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_failure)? as u64;
    let mut statement = connection
        .prepare(
            "SELECT invocation_id, slot_id, subject, waiter_pid, waiter_identity_json,
                started_at, allowed_time_ms, status, exit_code, completed_at,
                capture_dir, assignment_selection_json
         FROM work_slot_invocations WHERE run_id = ?1
         ORDER BY started_at DESC, invocation_id DESC LIMIT ?2",
        )
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str(), STATUS_INVOCATION_LIMIT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })
        .map_err(sqlite_failure)?;
    let mut items = Vec::new();
    for row in rows {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let (
            invocation_id,
            slot_id,
            subject,
            _waiter_pid,
            _waiter_identity_json,
            started_at,
            allowed_time_ms,
            stored_status,
            exit_code,
            completed_at,
            capture_dir,
            selection_json,
        ) = row.map_err(sqlite_failure)?;
        let elapsed_ms = i64::try_from(sampled_at_ms)
            .unwrap_or(i64::MAX)
            .saturating_sub(started_at)
            .max(0) as u64;
        let state = match stored_status.as_deref() {
            Some("succeeded") => "succeeded",
            Some("failed") => "failed",
            _ => "unknown",
        };
        let allowed_time_ms = u64::try_from(allowed_time_ms).unwrap_or(u64::MAX);
        let remaining_allowed_ms = if state == "running" {
            allowed_time_ms.saturating_sub(elapsed_ms)
        } else {
            0
        };
        let reason = if stored_status.is_none() {
            Some("no terminal waiter acknowledgement; quiet work remains unknown".to_owned())
        } else {
            None
        };
        let selection = selection_json
            .as_deref()
            .and_then(|raw| decode_json::<Vec<String>>(raw, "assignment selection").ok());
        items.push(json!({
            "invocation_id":invocation_id,"slot_id":slot_id,"subject_revision":subject,
            "execution":{"state":state,"exit_code":exit_code,"source":"historical invocation row"},
            "started_at_ms":started_at,"completed_at_ms":completed_at,
            "elapsed_ms":elapsed_ms,"allowed_time_ms":allowed_time_ms,
            "remaining_allowed_ms":remaining_allowed_ms,
            "overlay_meaning":"outer bound CLI process status only; not worker conformance or provider acceptance",
            "capture_dir":capture_dir,"assignment_selection":selection,
            "reason":reason,"cursor":{"started_at_ms":started_at,"invocation_id":invocation_id}
        }));
    }
    Ok((total, items))
}

fn count_unindexed_current_invocations(
    connection: &Connection,
    run_id: &RunId,
) -> Result<u64, PersistenceError> {
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM work_slot_invocations i
         JOIN slot_subjects s ON s.run_id=i.run_id AND s.slot_id=i.slot_id AND s.subject=i.subject
         WHERE i.run_id=?1 AND i.status IS NOT NULL AND i.assignment_facts_version=0",
            params![run_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_failure)?;
    Ok(count as u64)
}

fn load_status_assignments(
    connection: &Connection,
    run_id: &RunId,
) -> Result<(u64, Vec<serde_json::Value>), PersistenceError> {
    use serde_json::json;
    let total = connection.query_row(
        "SELECT COUNT(*) FROM (
            SELECT DISTINCT f.slot_id, f.assignment_id
            FROM work_slot_assignment_facts f
            JOIN slot_subjects s ON s.run_id = f.run_id AND s.slot_id = f.slot_id AND s.subject = f.subject
            WHERE f.run_id = ?1
        )",
        params![run_id.as_str()], |row| row.get::<_, i64>(0),
    ).map_err(sqlite_failure)? as u64;
    let mut statement = connection.prepare(
        "SELECT ids.slot_id, ids.subject, ids.assignment_id,
            (SELECT f.title FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.role FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.invocation_id FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.execution_state FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.conformance_state FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.conformance_error FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.conformance_error_truncated FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT f.facts_complete FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1),
            (SELECT COUNT(*) FROM work_slot_assignment_attempts a JOIN work_slot_assignment_facts f USING(run_id, invocation_id, assignment_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id),
            (SELECT COALESCE(SUM(a.failed),0) FROM work_slot_assignment_attempts a JOIN work_slot_assignment_facts f USING(run_id, invocation_id, assignment_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id),
            (SELECT f.attempts_path FROM work_slot_assignment_facts f JOIN work_slot_invocations i USING(run_id, invocation_id)
             WHERE f.run_id = ?1 AND f.slot_id = ids.slot_id AND f.subject = ids.subject AND f.assignment_id = ids.assignment_id
             ORDER BY i.started_at DESC, i.invocation_id DESC LIMIT 1)
         FROM (
            SELECT DISTINCT f.slot_id, f.subject, f.assignment_id
            FROM work_slot_assignment_facts f
            JOIN slot_subjects s ON s.run_id = f.run_id AND s.slot_id = f.slot_id AND s.subject = f.subject
            WHERE f.run_id = ?1
         ) ids
         ORDER BY ids.slot_id, ids.assignment_id LIMIT ?2"
    ).map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str(), STATUS_ASSIGNMENT_LIMIT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<String>>(13)?,
            ))
        })
        .map_err(sqlite_failure)?;
    let mut items = Vec::new();
    for row in rows {
        let (
            slot_id,
            subject,
            assignment_id,
            title,
            role,
            invocation_id,
            execution_state,
            conformance_state,
            error,
            error_truncated,
            facts_complete,
            attempts,
            failures,
            attempts_path,
        ) = row.map_err(sqlite_failure)?;
        let execution_state = execution_state.unwrap_or_else(|| "unknown".to_owned());
        let conformance_state = conformance_state.unwrap_or_else(|| "unknown".to_owned());
        let assignment_state = if facts_complete == 0 {
            "unknown"
        } else if execution_state == "not-started" {
            "not-started"
        } else if execution_state == "reused" {
            "reused"
        } else if execution_state == "failed" {
            "failed"
        } else if conformance_state == "failed" {
            if attempts >= 2 && failures >= 2 {
                "exhausted"
            } else {
                "invalid"
            }
        } else if execution_state == "succeeded" {
            "finished"
        } else {
            "unknown"
        };
        items.push(json!({
            "slot_id":slot_id,"subject_revision":subject,"assignment_id":assignment_id,
            "title":title,"role":role,"invocation_id":invocation_id,"state":assignment_state,
            "execution":{"state":execution_state,"source":if execution_state=="reused" {"attributed recovery source"} else {"captured worker process facts"}},
            "conformance":{"state":conformance_state,"error":error,"error_truncated":error_truncated!=0,
                "attempt_count":if facts_complete==0 {serde_json::Value::Null} else {json!(attempts)},
                "repeated_failure_count":if facts_complete==0 {serde_json::Value::Null} else {json!(failures)},"attempts_locator":attempts_path},
            "acceptance":{"state":"unknown","reason":"execution and conformance do not establish provider acceptance"}
        }));
    }
    Ok((total, items))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod advice_policy_tests {
    use loop_core::{
        AppendContextRequest, CreateRunRequest, Lifecycle, Persistence, ProviderAssociation, State,
        Timestamp, Transition, Workflow,
    };
    use serde_json::json;

    #[test]
    fn advice_policy_bounded_observation_resolves_only_policy_changes() {
        let persistence = super::super::SqlitePersistence::open_in_memory().unwrap();
        let workflow = Workflow::new(
            "advice",
            "work",
            vec![
                State::new("work", "Work", "", false),
                State::new("done", "Done", "", true),
            ],
            vec![Transition::check_free("work", "finish", "done")],
        );
        persistence
            .create_run(CreateRunRequest::new(
                "run",
                None,
                workflow,
                ProviderAssociation::new(json!({})),
                json!({}),
                "work",
                Lifecycle::Active,
                Timestamp::from_unix_millis(1),
                "test",
                None,
            ))
            .unwrap();
        let deadline = || std::time::Instant::now() + std::time::Duration::from_secs(10);
        let before = persistence
            .read_bounded_observation(&"run".into(), deadline(), true)
            .unwrap();
        assert_eq!(before["advice_policy"]["enabled"], false);
        assert_eq!(before["advice_policy"]["frozen"], serde_json::Value::Null);
        for (id, kind, data) in [
            ("large-note", "note", json!({"not_policy":"unrelated data"})),
            (
                "owner-on",
                loop_core::ADVICE_POLICY_CHANGE_RECORD_KIND,
                json!({
                    "actor":{"name":"owner","kind":"human"},"state_visit":0,"reason":"Advice now due",
                    "advice_departures":{"version":1,"occasions":[{"state":"work","event":"finish","occasion_id":"review"}]}
                }),
            ),
        ] {
            persistence
                .append_context(AppendContextRequest::new(
                    "run",
                    id,
                    kind,
                    data,
                    Timestamp::from_unix_millis(2),
                ))
                .unwrap();
        }
        let after = persistence
            .read_bounded_observation(&"run".into(), deadline(), true)
            .unwrap();
        assert_eq!(after["advice_policy"]["enabled"], true);
        assert_eq!(
            after["advice_policy"]["change_record_ids"],
            json!(["owner-on"])
        );
        assert_eq!(after["completeness"]["advice_policy"], "available");
        assert_eq!(
            after["advice_policy"]["effective"]["occasions"][0]["occasion_id"],
            "review"
        );
        persistence
            .append_context(AppendContextRequest::new(
                "run",
                "invalid-policy",
                loop_core::ADVICE_POLICY_CHANGE_RECORD_KIND,
                json!({}),
                Timestamp::from_unix_millis(3),
            ))
            .unwrap();
        let invalid = persistence
            .read_bounded_observation(&"run".into(), deadline(), false)
            .unwrap();
        assert_eq!(invalid["advice_policy"]["state"], "unavailable");
        assert_eq!(invalid["completeness"]["advice_policy"], "unavailable");
        assert!(
            invalid["advice_policy"].get("enabled").is_none(),
            "no stale fallback"
        );
    }

    fn bound_run(initial_input: serde_json::Value) -> super::super::SqlitePersistence {
        let persistence = super::super::SqlitePersistence::open_in_memory().unwrap();
        let workflow = Workflow::new(
            "bindings",
            "work",
            vec![
                State::new("work", "Work", "", false),
                State::new("done", "Done", "", true),
            ],
            vec![Transition::check_free("work", "finish", "done")],
        );
        persistence
            .create_run(CreateRunRequest::new(
                "run",
                None,
                workflow,
                ProviderAssociation::new(json!({})),
                initial_input,
                "work",
                Lifecycle::Active,
                Timestamp::from_unix_millis(1),
                "test",
                None,
            ))
            .unwrap();
        // Arming observation records that the run was observed, which append requires.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        persistence
            .read_bounded_observation(&"run".into(), deadline, true)
            .unwrap();
        persistence
    }

    fn observe(persistence: &super::super::SqlitePersistence) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        persistence
            .read_bounded_observation(&"run".into(), deadline, false)
            .unwrap()
    }

    #[test]
    fn bounded_observation_states_a_run_never_bound_and_leaves_completeness_alone() {
        let observed = observe(&bound_run(json!({})));
        assert_eq!(
            observed["model_bindings"],
            json!({"state":"never-bound","statement":loop_core::NEVER_BOUND_STATEMENT})
        );
        assert!(observed["completeness"].get("model_bindings").is_none());
    }

    #[test]
    fn bounded_observation_shows_original_and_amended_bindings_from_amendment_records_only() {
        let persistence = bound_run(
            json!({"model_bindings":{"roles":{"cold-reader":{"model":"p/m1","thinking":"high"}}}}),
        );
        let before = observe(&persistence);
        assert_eq!(before["model_bindings"]["state"], "bound");
        assert_eq!(before["model_bindings"]["history"], json!([]));
        for (id, kind, data) in [
            ("note", "note", json!({"unrelated": true})),
            (
                "amend-1",
                loop_core::MODEL_BINDING_AMENDMENT_RECORD_KIND,
                json!({
                    "actor":{"name":"owner","kind":"human"},
                    "authority":{"kind":"owner","reference":"chat"},
                    "words":"Use the cheaper reader.",
                    "changes":[{
                        "role":"cold-reader",
                        "old":{"model":"p/m1","thinking":"high"},
                        "new":{"model":"p/m2","thinking":"low"},
                        "mark_stale":true
                    }]
                }),
            ),
        ] {
            persistence
                .append_context(AppendContextRequest::new(
                    "run",
                    id,
                    kind,
                    data,
                    Timestamp::from_unix_millis(2),
                ))
                .unwrap();
        }
        let after = observe(&persistence);
        let view = &after["model_bindings"];
        let reader = view["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["role"] == "cold-reader")
            .unwrap();
        assert_eq!(reader["original"]["model"], "p/m1");
        assert_eq!(reader["in_force"]["model"], "p/m2");
        assert_eq!(view["history"][0]["record_id"], "amend-1");
        assert_eq!(view["history"][0]["words"], "Use the cheaper reader.");
        assert_eq!(view["history"][0]["changes"][0]["mark_stale"], true);
    }

    #[test]
    fn bounded_observation_reports_an_invalid_amendment_as_unavailable_not_stale() {
        let persistence = bound_run(json!({}));
        persistence
            .append_context(AppendContextRequest::new(
                "run",
                "broken",
                loop_core::MODEL_BINDING_AMENDMENT_RECORD_KIND,
                json!({}),
                Timestamp::from_unix_millis(2),
            ))
            .unwrap();
        let observed = observe(&persistence);
        assert_eq!(
            observed["model_bindings"]["state"], "unavailable",
            "{observed}"
        );
        assert_eq!(observed["completeness"]["model_bindings"], "unavailable");
    }
}
