use super::{decode_json, load_required_run, status_read_deadline};
use loop_core::{
    InnerWorker, PersistenceError, PersistenceFailure, RunId, WorkSlotInvocation, WorkerAttempt,
};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use std::path::Path;

const ERROR_BYTES: usize = 2048;
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
        let effective_bindings = slots
            .iter()
            .filter_map(|slot| {
                loop_core::effective_binding(&run, &slot.id)
                    .and_then(|binding| binding.ok())
                    .map(|binding| (slot.id.to_string(), json!(binding)))
            })
            .collect::<serde_json::Map<String, serde_json::Value>>();
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
                "effective_bindings":effective_bindings,
                "override_summary":run.override_summary
            },
            "invocations":{"state":"unavailable","reason":"not sampled","total":null,"limit":STATUS_INVOCATION_LIMIT,"returned":0,"truncated":false,"items":[]},
            "assignments":{"state":"unavailable","reason":"not sampled","total":null,"limit":STATUS_ASSIGNMENT_LIMIT,"returned":0,"truncated":false,"items":[]},
            "acceptance":{"state":"unknown","reason":"status does not infer provider or driver acceptance"},
            "completeness":{"run":"available","invocations":"unavailable","assignments":"unavailable"}
        });
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
            waiter_pid,
            waiter_identity_json,
            started_at,
            allowed_time_ms,
            stored_status,
            exit_code,
            completed_at,
            capture_dir,
            selection_json,
        ) = row.map_err(sqlite_failure)?;
        let identity = waiter_identity_json
            .as_deref()
            .and_then(|raw| decode_json::<loop_core::ProcessIdentity>(raw, "waiter identity").ok());
        let alive = u32::try_from(waiter_pid)
            .ok()
            .and_then(|pid| {
                identity.as_ref().map(|identity| {
                    crate::ownership::process_identity_matches(pid, Some(identity)).unwrap_or(false)
                })
            })
            .unwrap_or(false);
        let elapsed_ms = i64::try_from(sampled_at_ms)
            .unwrap_or(i64::MAX)
            .saturating_sub(started_at)
            .max(0) as u64;
        let state = match stored_status.as_deref() {
            Some("succeeded") => "succeeded",
            Some("failed")
                if Path::new(&capture_dir)
                    .join("ownership/cleanup-acknowledged.json")
                    .is_file() =>
            {
                "cancelled"
            }
            Some("failed") => "failed",
            Some(_) => "unknown",
            None if alive && elapsed_ms >= u64::try_from(allowed_time_ms).unwrap_or(u64::MAX) => {
                "overrun"
            }
            None if alive => "running",
            None => "unknown",
        };
        let allowed_time_ms = u64::try_from(allowed_time_ms).unwrap_or(u64::MAX);
        let remaining_allowed_ms = if state == "running" {
            allowed_time_ms.saturating_sub(elapsed_ms)
        } else {
            0
        };
        let reason = if stored_status.is_none() && !alive {
            Some("no terminal waiter acknowledgement; quiet work remains unknown".to_owned())
        } else {
            None
        };
        let selection = selection_json
            .as_deref()
            .and_then(|raw| decode_json::<Vec<String>>(raw, "assignment selection").ok());
        items.push(json!({
            "invocation_id":invocation_id,"slot_id":slot_id,"subject_revision":subject,
            "execution":{"state":state,"exit_code":exit_code,"source":"waiter record and sampled process identity"},
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

pub(super) fn insert_assignment_facts(
    transaction: &Transaction<'_>,
    run_id: &RunId,
    invocation: &WorkSlotInvocation,
    workers: &[InnerWorker],
) -> Result<(), PersistenceError> {
    for worker in workers {
        if worker.assignment_id.is_empty() {
            continue;
        }
        let label = invocation
            .assignment_labels
            .iter()
            .find(|label| label.assignment_id == worker.assignment_id);
        let conformance_state = if worker.started == Some(false) {
            "unknown"
        } else {
            match worker.conformance_status.as_deref() {
                Some("succeeded") => "conforming",
                Some("failed") => "failed",
                Some(_) => "unknown",
                None => "not-checked",
            }
        };
        transaction
            .execute(
                "INSERT INTO work_slot_assignment_facts (
                run_id, invocation_id, slot_id, subject, assignment_id, title, role,
                execution_state, conformance_state, conformance_error,
                conformance_error_truncated, facts_complete, attempts_path
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12)
             ON CONFLICT (run_id, invocation_id, assignment_id) DO UPDATE SET
                title=COALESCE(excluded.title, work_slot_assignment_facts.title),
                role=COALESCE(excluded.role, work_slot_assignment_facts.role),
                execution_state=excluded.execution_state,
                conformance_state=excluded.conformance_state,
                conformance_error=excluded.conformance_error,
                conformance_error_truncated=excluded.conformance_error_truncated,
                facts_complete=1,
                attempts_path=excluded.attempts_path",
                params![
                    run_id.as_str(),
                    invocation.invocation_id.as_str(),
                    invocation.slot_id.as_str(),
                    invocation.subject,
                    worker.assignment_id,
                    label.map(|label| label.title.as_str()),
                    label.map(|label| label.role.as_str()),
                    if worker.recovery_source.is_some() {
                        "reused"
                    } else if worker.started == Some(false) {
                        "not-started"
                    } else if worker.exit_code == 0 {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    conformance_state,
                    worker
                        .conformance_error
                        .as_deref()
                        .map(|error| bounded_text(error).0),
                    i64::from(
                        worker
                            .conformance_error
                            .as_deref()
                            .is_some_and(|error| error.len() > ERROR_BYTES)
                    ),
                    worker.attempts_path,
                ],
            )
            .map_err(sqlite_failure)?;

        transaction
            .execute(
                "DELETE FROM work_slot_assignment_attempts
             WHERE run_id = ?1 AND invocation_id = ?2 AND assignment_id = ?3",
                params![
                    run_id.as_str(),
                    invocation.invocation_id.as_str(),
                    worker.assignment_id
                ],
            )
            .map_err(sqlite_failure)?;
        for attempt in normalized_attempts(worker) {
            let normalized_errors = attempt
                .errors
                .iter()
                .map(|error| bounded_text(error))
                .collect::<Vec<_>>();
            let errors_truncated = normalized_errors.iter().any(|(_, truncated)| *truncated);
            let errors = normalized_errors
                .into_iter()
                .map(|(error, _)| error)
                .collect::<Vec<_>>();
            let errors_json = serde_json::to_string(&errors).map_err(|error| {
                PersistenceError::failure(PersistenceFailure::new(
                    "sqlite-encode",
                    error.to_string(),
                ))
            })?;
            transaction
                .execute(
                    "INSERT INTO work_slot_assignment_attempts (
                    run_id, invocation_id, assignment_id, attempt_number, failed,
                    errors_json, errors_truncated, stdout_path, stderr_path
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        run_id.as_str(),
                        invocation.invocation_id.as_str(),
                        worker.assignment_id,
                        i64::from(attempt.number.max(1)),
                        i64::from(attempt.failed || !errors.is_empty()),
                        errors_json,
                        i64::from(errors_truncated),
                        attempt
                            .stdout_path
                            .as_deref()
                            .or(worker.stdout_path.as_deref()),
                        attempt
                            .stderr_path
                            .as_deref()
                            .or(worker.stderr_path.as_deref()),
                    ],
                )
                .map_err(sqlite_failure)?;
        }
    }
    Ok(())
}

fn normalized_attempts(worker: &InnerWorker) -> Vec<WorkerAttempt> {
    if worker.started == Some(false) || worker.recovery_source.is_some() {
        return Vec::new();
    }
    if !worker.attempts.is_empty() {
        return worker.attempts.clone();
    }
    vec![WorkerAttempt {
        number: worker.selected_attempt.unwrap_or(1).max(1),
        failed: worker.exit_code != 0 || worker.conformance_status.as_deref() == Some("failed"),
        errors: worker.conformance_error.iter().cloned().collect(),
        stdout_path: worker.stdout_path.clone(),
        stderr_path: worker.stderr_path.clone(),
    }]
}

fn bounded_text(value: &str) -> (String, bool) {
    if value.len() <= ERROR_BYTES {
        return (value.to_owned(), false);
    }
    let mut end = ERROR_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

fn sqlite_failure(error: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new("sqlite", error.to_string()))
}
