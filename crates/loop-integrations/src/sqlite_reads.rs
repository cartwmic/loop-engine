use super::{load_required_run, sqlite_failure, TargetedReadRequest};
use loop_core::{AssignmentLabel, PersistenceError, PersistenceFailure, RunId};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

const MAX_PAGE_LIMIT: u32 = 500;
const MAX_STREAM_BYTES: u32 = 65_536;

pub(super) fn read_targeted(
    connection: &mut Connection,
    run_id: &RunId,
    request: &TargetedReadRequest,
    deadline: std::time::Instant,
) -> Result<Value, PersistenceError> {
    if !matches!(request.kind.as_str(), "stdout" | "stderr")
        && (request.limit == 0 || request.limit > MAX_PAGE_LIMIT)
    {
        return Err(issue(
            "invalid-read-limit",
            format!("page limit must be between 1 and {MAX_PAGE_LIMIT}"),
        ));
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(sqlite_failure)?;
    let run = load_required_run(&transaction, run_id)?;
    let delta_sequence = run.last_sequence.as_u64();
    match request.kind.as_str() {
        "history" | "delta" => {
            let after = parse_cursor(request.cursor.as_deref(), 0)?;
            let total = transaction
                .query_row(
                    "SELECT COUNT(*) FROM history_entries WHERE run_id = ?1",
                    params![run_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(sqlite_failure)? as u64;
            let remaining = transaction
                .query_row(
                    "SELECT COUNT(*) FROM history_entries WHERE run_id = ?1 AND sequence > ?2",
                    params![run_id.as_str(), to_sql_i64(after)?],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(sqlite_failure)? as u64;
            let mut statement = transaction
                .prepare(
                    "SELECT sequence, occurred_at, action_json FROM history_entries
                 WHERE run_id = ?1 AND sequence > ?2 ORDER BY sequence ASC LIMIT ?3",
                )
                .map_err(sqlite_failure)?;
            let rows = statement
                .query_map(
                    params![
                        run_id.as_str(),
                        to_sql_i64(after)?,
                        i64::from(request.limit)
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(sqlite_failure)?;
            let mut items = Vec::new();
            for row in rows {
                let (sequence, occurred_at, action) = row.map_err(sqlite_failure)?;
                let action: Value = serde_json::from_str(&action)
                    .map_err(|error| issue("history-decode-failed", error.to_string()))?;
                items.push(
                    json!({"sequence":sequence,"occurred_at_ms":occurred_at,"action":action}),
                );
            }
            drop(statement);
            let changed_invocations = items
                .iter()
                .filter_map(|item| {
                    let action = &item["action"];
                    let kind = action["kind"].as_str()?;
                    matches!(kind, "invocation_started" | "invocation_status_changed")
                        .then(|| action["invocation_id"].as_str().map(str::to_owned))
                        .flatten()
                })
                .collect::<std::collections::BTreeSet<_>>();
            transaction.commit().map_err(sqlite_failure)?;
            let next = items.last().and_then(|item| item["sequence"].as_u64());
            let truncated = remaining > items.len() as u64;
            let mut output = json!({
                "schema_version":1,"kind":request.kind,"state":"available","total":total,
                "remaining":remaining,"limit":request.limit,"cursor":after,
                "next_cursor":if truncated {next} else {None},
                "truncated":truncated,"delta_sequence":delta_sequence,"items":items
            });
            if request.kind == "delta" {
                output["sampled_execution"] =
                    match super::sqlite_observation::read_bounded_observation(
                        connection, run_id, deadline, false,
                    ) {
                        Ok(sample) => {
                            let invocations = sample["invocations"]["items"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            let assignments = sample["assignments"]["items"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            let changed = invocations
                                .iter()
                                .filter(|row| {
                                    changed_invocations
                                        .contains(row["invocation_id"].as_str().unwrap_or(""))
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            let active_ids = invocations
                                .iter()
                                .filter(|row| {
                                    matches!(
                                        row.pointer("/execution/state").and_then(Value::as_str),
                                        Some("running" | "overrun")
                                    )
                                })
                                .filter_map(|row| row["invocation_id"].as_str().map(str::to_owned))
                                .collect::<std::collections::BTreeSet<_>>();
                            let active = assignments
                                .into_iter()
                                .filter(|row| {
                                    active_ids.contains(row["invocation_id"].as_str().unwrap_or(""))
                                })
                                .collect::<Vec<_>>();
                            let complete =
                                sample["completeness"].as_object().is_some_and(|lanes| {
                                    lanes.values().all(|value| value == "available")
                                });
                            json!({"state":if complete {"available"} else {"partial"},"sampled_at_ms":sample["sampled_at_ms"],
                            "delta_sequence":sample["delta_sequence"],"changed_invocations":changed,"active_assignments":active,
                            "missed_sample":if complete {"unknown"} else {"true"},
                            "meaning":"invocation history rows are durable changes; live assignment values are current samples, not invented events"})
                        }
                        Err(error) => {
                            json!({"state":"unavailable","reason":error.to_string(),"missed_sample":"true"})
                        }
                    };
            }
            Ok(output)
        }
        "assignment" => {
            let unindexed = count_unindexed_assignment_invocations(
                &transaction,
                run_id,
                request.assignment_id.as_deref(),
                None,
            )?;
            read_assignment_page(transaction, run_id, request, delta_sequence, unindexed)
        }
        "attempt" | "error" => {
            let unindexed = count_unindexed_assignment_invocations(
                &transaction,
                run_id,
                request.assignment_id.as_deref(),
                request.invocation_id.as_deref(),
            )?;
            read_attempt_page(transaction, run_id, request, delta_sequence, unindexed)
        }
        "stdout" | "stderr" => read_stream(transaction, run_id, request, deadline, delta_sequence),
        other => Err(issue(
            "invalid-read-kind",
            format!("unsupported read kind `{other}`"),
        )),
    }
}

fn read_assignment_page(
    transaction: rusqlite::Transaction<'_>,
    run_id: &RunId,
    request: &TargetedReadRequest,
    delta_sequence: u64,
    unindexed_invocations: u64,
) -> Result<Value, PersistenceError> {
    let assignment = request
        .assignment_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            issue(
                "missing-assignment-id",
                "assignment reads require --assignment",
            )
        })?;
    let offset = parse_cursor(request.cursor.as_deref(), 0)?;
    let invocation_filter = request.invocation_id.as_deref();
    let total: i64 = if let Some(invocation_id) = invocation_filter {
        transaction.query_row(
            "SELECT COUNT(*) FROM work_slot_assignment_facts WHERE run_id=?1 AND assignment_id=?2 AND invocation_id=?3",
            params![run_id.as_str(), assignment, invocation_id], |row| row.get(0),
        ).map_err(sqlite_failure)?
    } else {
        transaction.query_row(
            "SELECT COUNT(*) FROM work_slot_assignment_facts WHERE run_id=?1 AND assignment_id=?2",
            params![run_id.as_str(), assignment], |row| row.get(0),
        ).map_err(sqlite_failure)?
    };
    let mut statement = transaction.prepare(
        "SELECT f.invocation_id, f.slot_id, f.subject, f.title, f.role,
                f.execution_state, f.conformance_state, f.conformance_error,
                f.conformance_error_truncated, f.facts_complete, f.attempts_path, i.started_at,
                (SELECT COUNT(*) FROM work_slot_assignment_attempts a
                 WHERE a.run_id=f.run_id AND a.invocation_id=f.invocation_id AND a.assignment_id=f.assignment_id),
                (SELECT COALESCE(SUM(a.failed),0) FROM work_slot_assignment_attempts a
                 WHERE a.run_id=f.run_id AND a.invocation_id=f.invocation_id AND a.assignment_id=f.assignment_id)
         FROM work_slot_assignment_facts f JOIN work_slot_invocations i
           ON i.run_id=f.run_id AND i.invocation_id=f.invocation_id
         WHERE f.run_id=?1 AND f.assignment_id=?2 AND (?3 IS NULL OR f.invocation_id=?3)
         ORDER BY i.started_at ASC, f.invocation_id ASC LIMIT ?4 OFFSET ?5",
    ).map_err(sqlite_failure)?;
    let rows = statement
        .query_map(
            params![
                run_id.as_str(),
                assignment,
                invocation_filter,
                i64::from(request.limit),
                to_sql_i64(offset)?
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                ))
            },
        )
        .map_err(sqlite_failure)?;
    let mut items = Vec::new();
    for row in rows {
        let (
            invocation_id,
            slot_id,
            subject,
            title,
            role,
            execution,
            conformance,
            error,
            error_truncated,
            facts_complete,
            attempts_path,
            started_at,
            attempts,
            failures,
        ) = row.map_err(sqlite_failure)?;
        items.push(json!({
            "invocation_id":invocation_id,"slot_id":slot_id,"subject_revision":subject,
            "assignment_id":assignment,"title":title,"role":role,"execution":{"state":execution},
            "conformance":{"state":conformance,"error":error,"error_truncated":error_truncated!=0,
                "attempt_count":if facts_complete==0 {Value::Null} else {json!(attempts)},
                "repeated_failure_count":if facts_complete==0 {Value::Null} else {json!(failures)},
                "facts_complete":facts_complete!=0,"attempts_locator":attempts_path},
            "acceptance":{"state":"unknown"},"started_at_ms":started_at
        }));
    }
    drop(statement);
    transaction.commit().map_err(sqlite_failure)?;
    page_envelope(
        "assignment",
        total as u64,
        offset,
        request.limit,
        delta_sequence,
        unindexed_invocations,
        items,
    )
}

fn read_attempt_page(
    transaction: rusqlite::Transaction<'_>,
    run_id: &RunId,
    request: &TargetedReadRequest,
    delta_sequence: u64,
    unindexed_invocations: u64,
) -> Result<Value, PersistenceError> {
    let assignment = request
        .assignment_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            issue(
                "missing-assignment-id",
                "attempt/error reads require --assignment",
            )
        })?;
    let offset = parse_cursor(request.cursor.as_deref(), 0)?;
    let errors_only = request.kind == "error";
    let invocation_filter = request.invocation_id.as_deref();
    let predicate = if errors_only {
        " AND (a.failed=1 OR a.errors_json != '[]')"
    } else {
        ""
    };
    let base = format!(
        "SELECT COUNT(*) FROM work_slot_assignment_attempts a
         WHERE a.run_id=?1 AND a.assignment_id=?2 AND (?3 IS NULL OR a.invocation_id=?3){predicate}"
    );
    let total: i64 = transaction
        .query_row(
            &base,
            params![run_id.as_str(), assignment, invocation_filter],
            |row| row.get(0),
        )
        .map_err(sqlite_failure)?;
    let sql = format!(
        "SELECT a.invocation_id, f.slot_id, f.subject, a.attempt_number, a.failed,
                a.errors_json, a.errors_truncated, a.stdout_path, a.stderr_path, i.started_at
         FROM work_slot_assignment_attempts a
         JOIN work_slot_assignment_facts f USING(run_id, invocation_id, assignment_id)
         JOIN work_slot_invocations i USING(run_id, invocation_id)
         WHERE a.run_id=?1 AND a.assignment_id=?2 AND (?3 IS NULL OR a.invocation_id=?3){predicate}
         ORDER BY i.started_at ASC, a.invocation_id ASC, a.attempt_number ASC
         LIMIT ?4 OFFSET ?5"
    );
    let mut statement = transaction.prepare(&sql).map_err(sqlite_failure)?;
    let rows = statement
        .query_map(
            params![
                run_id.as_str(),
                assignment,
                invocation_filter,
                i64::from(request.limit),
                to_sql_i64(offset)?
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .map_err(sqlite_failure)?;
    let mut items = Vec::new();
    for row in rows {
        let (
            invocation_id,
            slot_id,
            subject,
            number,
            failed,
            errors,
            errors_truncated,
            stdout,
            stderr,
            started_at,
        ) = row.map_err(sqlite_failure)?;
        let errors: Value = serde_json::from_str(&errors)
            .map_err(|error| issue("attempt-decode-failed", error.to_string()))?;
        items.push(json!({"invocation_id":invocation_id,"slot_id":slot_id,
            "subject_revision":subject,"attempt":number,"failed":failed!=0,
            "errors":errors,"errors_truncated":errors_truncated!=0,"stdout_path":stdout,"stderr_path":stderr,"started_at_ms":started_at}));
    }
    drop(statement);
    transaction.commit().map_err(sqlite_failure)?;
    page_envelope(
        &request.kind,
        total as u64,
        offset,
        request.limit,
        delta_sequence,
        unindexed_invocations,
        items,
    )
}

fn read_stream(
    transaction: rusqlite::Transaction<'_>,
    run_id: &RunId,
    request: &TargetedReadRequest,
    deadline: std::time::Instant,
    delta_sequence: u64,
) -> Result<Value, PersistenceError> {
    let assignment = request
        .assignment_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            issue(
                "missing-assignment-id",
                "stdout/stderr reads require --assignment",
            )
        })?;
    let invocation_id = request
        .invocation_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            issue(
                "missing-invocation-id",
                "stdout/stderr reads require --invocation",
            )
        })?;
    let attempt = request.attempt.unwrap_or(1);
    if attempt == 0 {
        return Err(issue(
            "invalid-attempt",
            "attempt number must be at least 1",
        ));
    }
    if request.limit == 0 || request.limit > MAX_STREAM_BYTES {
        return Err(issue(
            "invalid-read-limit",
            format!("stream limit must be between 1 and {MAX_STREAM_BYTES} bytes"),
        ));
    }
    let path_column = match request.kind.as_str() {
        "stdout" => "stdout_path",
        "stderr" => "stderr_path",
        _ => unreachable!(),
    };
    let sql = format!(
        "SELECT a.{path_column}, i.capture_dir, i.assignment_facts_version FROM work_slot_assignment_attempts a
         JOIN work_slot_invocations i USING(run_id, invocation_id)
         WHERE a.run_id=?1 AND a.invocation_id=?2 AND a.assignment_id=?3 AND a.attempt_number=?4"
    );
    let selected: Option<(Option<String>, String, i64)> = transaction
        .query_row(
            &sql,
            params![
                run_id.as_str(),
                invocation_id,
                assignment,
                i64::from(attempt)
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(sqlite_failure)?;
    let (path, capture_dir, facts_version) = selected.ok_or_else(|| {
        issue(
            "captured-attempt-not-found",
            "no indexed retained attempt matches the requested identity; use full show for legacy records",
        )
    })?;
    if facts_version == 0 {
        return Err(issue(
            "assignment-index-unavailable",
            "this invocation predates indexed attempt facts; use full show and its capture locator",
        ));
    }
    let path = path.filter(|path| !path.is_empty()).ok_or_else(|| {
        issue(
            "captured-stream-unavailable",
            "the original stream path was not retained for this attempt",
        )
    })?;
    transaction.commit().map_err(sqlite_failure)?;
    if std::time::Instant::now() >= deadline {
        return Err(issue(
            "status-read-deadline",
            "output locator lookup reached the read deadline",
        ));
    }
    let root = std::fs::canonicalize(&capture_dir)
        .map_err(|error| issue("capture-unavailable", error.to_string()))?;
    let candidate = PathBuf::from(&path);
    let actual_path = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let actual_path = std::fs::canonicalize(&actual_path)
        .map_err(|error| issue("captured-stream-unavailable", error.to_string()))?;
    if !actual_path.starts_with(&root) {
        return Err(issue(
            "captured-stream-outside-origin",
            "resolved stream path is outside its invocation capture",
        ));
    }
    let before = std::fs::metadata(&actual_path)
        .map_err(|error| issue("captured-stream-unavailable", error.to_string()))?;
    if request.offset > before.len() {
        return Err(issue(
            "invalid-offset",
            format!(
                "offset {} exceeds exact stream total {}",
                request.offset,
                before.len()
            ),
        ));
    }
    if std::time::Instant::now() >= deadline {
        return Err(issue(
            "status-read-deadline",
            "output metadata read reached the deadline",
        ));
    }
    let mut file = std::fs::File::open(&actual_path)
        .map_err(|error| issue("captured-stream-unavailable", error.to_string()))?;
    file.seek(SeekFrom::Start(request.offset))
        .map_err(|error| issue("captured-stream-read-failed", error.to_string()))?;
    let remaining = before
        .len()
        .saturating_sub(request.offset)
        .min(u64::from(request.limit));
    let mut bytes = vec![0u8; remaining as usize];
    let mut read = 0usize;
    while read < bytes.len() {
        let count = file
            .read(&mut bytes[read..])
            .map_err(|error| issue("captured-stream-read-failed", error.to_string()))?;
        if count == 0 {
            break;
        }
        read += count;
    }
    bytes.truncate(read);
    let after = std::fs::metadata(&actual_path)
        .map_err(|error| issue("captured-stream-unavailable", error.to_string()))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(issue(
            "captured-stream-changed",
            "original stream changed while the byte range was read",
        ));
    }
    let total = before.len();
    let next_offset = request.offset.saturating_add(bytes.len() as u64);
    let truncated = next_offset < total;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(
        json!({"schema_version":1,"kind":request.kind,"state":"available","delta_sequence":delta_sequence,"invocation_id":invocation_id,
        "assignment_id":assignment,"attempt":attempt,"stream":request.kind,
        "encoding":"hex","exact_bytes":true,"data":hex(&bytes),"sha256":format!("sha256:{digest}"),
        "total_bytes":total,"offset":request.offset,"limit":request.limit,
        "next_offset":if truncated {Some(next_offset)} else {None},"cursor":request.offset,
        "truncated":truncated,"path":path}),
    )
}

fn page_envelope(
    kind: &str,
    total: u64,
    offset: u64,
    limit: u32,
    delta_sequence: u64,
    unindexed_invocations: u64,
    items: Vec<Value>,
) -> Result<Value, PersistenceError> {
    let next_offset = offset.saturating_add(items.len() as u64);
    let page_more = next_offset < total;
    let truncated = page_more || unindexed_invocations > 0;
    let complete_total = (unindexed_invocations == 0).then_some(total);
    Ok(json!({
        "schema_version":1,"kind":kind,
        "state":if unindexed_invocations==0 {"available"} else {"partial"},
        "delta_sequence":delta_sequence,"total":complete_total,"indexed_total":total,
        "unindexed_invocations":unindexed_invocations,"limit":limit,"cursor":offset,
        "next_cursor":if page_more {Some(next_offset)} else {None},
        "truncated":truncated,"items":items
    }))
}

fn count_unindexed_assignment_invocations(
    connection: &Connection,
    run_id: &RunId,
    assignment_id: Option<&str>,
    invocation_id: Option<&str>,
) -> Result<u64, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT assignment_labels_json FROM work_slot_invocations
         WHERE run_id=?1 AND assignment_facts_version=0 AND (?2 IS NULL OR invocation_id=?2)
         ORDER BY started_at DESC, invocation_id DESC LIMIT 21",
        )
        .map_err(sqlite_failure)?;
    let rows = statement
        .query_map(params![run_id.as_str(), invocation_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite_failure)?;
    let mut incomplete = 0u64;
    let mut scanned = 0usize;
    for row in rows {
        scanned += 1;
        if scanned > 20 {
            return Ok(incomplete.max(1));
        }
        let raw = row.map_err(sqlite_failure)?;
        if raw.len() > 262_144 {
            incomplete += 1;
            continue;
        }
        let labels = serde_json::from_str::<Vec<AssignmentLabel>>(&raw).unwrap_or_default();
        if labels.is_empty()
            || assignment_id.is_none_or(|id| labels.iter().any(|label| label.assignment_id == id))
        {
            incomplete += 1;
        }
    }
    Ok(incomplete)
}

fn parse_cursor(cursor: Option<&str>, default: u64) -> Result<u64, PersistenceError> {
    match cursor {
        None => Ok(default),
        Some(cursor) => cursor
            .parse::<u64>()
            .map_err(|_| issue("invalid-cursor", "cursor must be a non-negative integer")),
    }
}

fn to_sql_i64(value: u64) -> Result<i64, PersistenceError> {
    i64::try_from(value)
        .map_err(|_| issue("invalid-cursor", "cursor exceeds SQLite's integer range"))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn issue(code: &str, message: impl Into<String>) -> PersistenceError {
    PersistenceError::failure(PersistenceFailure::new(code, message))
}
