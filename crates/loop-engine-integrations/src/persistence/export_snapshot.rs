//! Consistent export snapshot loading and journal decoding (T116).
//!
//! Owns the single deferred read transaction, SQLite row extraction, and
//! journal entry reconstruction used by audit export.

use std::path::Path;

use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::annotation::{ActorMetadata, Note};
use loop_engine_core::model::attempt::{
    AttemptFacts, EvidenceAddedFact, EvidenceAssociations, GateVerdictFact, GateVerdictFacts,
    GateVerdictResult, JournalExtension, LabelChangeFact, ProviderFact, ProviderRole,
    TransitionFact,
};
use loop_engine_core::model::bounded::BoundError;
use loop_engine_core::model::compatibility::{CompatibilityFinding, CompatibilityFindings};
use loop_engine_core::model::diagnostic::{Diagnostic, Diagnostics};
use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::ids::{
    EventId, EvidenceId, EvidenceKind, GateId, GraphRevision, RegistrationId, RequestId, RunId,
    StateId,
};
use loop_engine_core::model::journal::{
    JournalEncodedSizes, JournalEntry, JournalEntryKind, JournalError, StateFact,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, params};
use serde_json::Value;
use thiserror::Error;

use crate::persistence::PersistenceError;
use crate::persistence::mapping::{self, MappingError};
use crate::persistence::records::{EvidenceRecordRow, JournalRecord, RunRecord};
use crate::persistence::sqlite::connect_read_only_with_pragmas;
use crate::provider_protocol::mapping as protocol_mapping;

#[derive(Debug, Error)]
pub enum ExportSnapshotError {
    #[error("run not found: {run_id}")]
    RunNotFound { run_id: RunId },
    #[error("persistence read failed: {message}")]
    Failed { message: String },
    #[error(transparent)]
    Bound(#[from] BoundError),
}

#[derive(Debug, Clone)]
pub struct ExportSnapshot {
    pub run_record: RunRecord,
    pub evidence_rows: Vec<EvidenceRecordRow>,
    pub journal_records: Vec<JournalRecord>,
    pub journal_entries: Vec<JournalEntry>,
    pub exported_at: String,
}

pub fn load_consistent_snapshot<C: TimeSource>(
    db_path: &Path,
    run_id: &RunId,
    clock: &C,
) -> Result<ExportSnapshot, ExportSnapshotError> {
    let conn = connect_read_only_with_pragmas(db_path).map_err(map_persistence_error)?;
    conn.execute("BEGIN DEFERRED", [])
        .map_err(map_sqlite_read_error)?;
    let exported_at = export_timestamp(clock)?;
    let result = build_consistent_snapshot(&conn, run_id, exported_at);
    match &result {
        Ok(_) => {
            conn.execute("COMMIT", []).map_err(map_sqlite_read_error)?;
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    result
}

#[cfg(test)]
fn load_consistent_snapshot_with_after_run<C: TimeSource>(
    db_path: &Path,
    run_id: &RunId,
    clock: &C,
    after_run: impl FnOnce(),
) -> Result<ExportSnapshot, ExportSnapshotError> {
    let conn = connect_read_only_with_pragmas(db_path).map_err(map_persistence_error)?;
    conn.execute("BEGIN DEFERRED", [])
        .map_err(map_sqlite_read_error)?;
    let exported_at = export_timestamp(clock)?;
    let result = (|| {
        let run_record = load_run_record(&conn, run_id)?;
        after_run();
        let evidence_rows = load_evidence_rows(&conn, run_id)?;
        let journal_records = load_journal_records(&conn, run_id)?;
        let journal_entries = journal_entries_from_records(&conn, &journal_records)?;
        Ok(ExportSnapshot {
            run_record,
            evidence_rows,
            journal_records,
            journal_entries,
            exported_at,
        })
    })();
    match &result {
        Ok(_) => {
            conn.execute("COMMIT", []).map_err(map_sqlite_read_error)?;
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
    result
}

fn build_consistent_snapshot(
    conn: &Connection,
    run_id: &RunId,
    exported_at: String,
) -> Result<ExportSnapshot, ExportSnapshotError> {
    let run_record = load_run_record(conn, run_id)?;
    let evidence_rows = load_evidence_rows(conn, run_id)?;
    let journal_records = load_journal_records(conn, run_id)?;
    let journal_entries = journal_entries_from_records(conn, &journal_records)?;
    Ok(ExportSnapshot {
        run_record,
        evidence_rows,
        journal_records,
        journal_entries,
        exported_at,
    })
}

fn export_timestamp<C: TimeSource>(clock: &C) -> Result<String, ExportSnapshotError> {
    match clock.now() {
        Ok(observed) => Ok(mapping::format_observed_at(&observed)),
        Err(_) => Err(ExportSnapshotError::Failed {
            message: "clock read failed".into(),
        }),
    }
}

fn load_run_record(conn: &Connection, run_id: &RunId) -> Result<RunRecord, ExportSnapshotError> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                    workflow_state_version, lifecycle_version, label_version, label,
                    graph_revision, canonical_graph_version,
                    graph_canonical_projection_json, inputs_json, created_at
             FROM runs
             WHERE run_id = ?1",
        )
        .map_err(map_sqlite_read_error)?;
    statement
        .query_row(params![run_id.as_str()], |row| {
            Ok(RunRecord {
                run_id: row.get("run_id")?,
                registration_id: row.get("registration_id")?,
                config_revision_at_create: row.get::<_, i64>("config_revision_at_create")? as u64,
                current_state: row.get("current_state")?,
                lifecycle: row.get("lifecycle")?,
                workflow_state_version: row.get::<_, i64>("workflow_state_version")? as u64,
                lifecycle_version: row.get::<_, i64>("lifecycle_version")? as u64,
                label_version: row.get::<_, i64>("label_version")? as u64,
                label: row.get("label")?,
                graph_revision: row.get("graph_revision")?,
                canonical_graph_version: row.get::<_, i64>("canonical_graph_version")? as u64,
                graph_canonical_projection_json: row.get("graph_canonical_projection_json")?,
                inputs_json: row.get("inputs_json")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|error| match error {
            SqliteError::QueryReturnedNoRows => ExportSnapshotError::RunNotFound {
                run_id: run_id.clone(),
            },
            other => map_sqlite_read_error(other),
        })
}

fn load_evidence_rows(
    conn: &Connection,
    run_id: &RunId,
) -> Result<Vec<EvidenceRecordRow>, ExportSnapshotError> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
             FROM evidence
             WHERE run_id = ?1
             ORDER BY created_at ASC, evidence_id ASC",
        )
        .map_err(map_sqlite_read_error)?;
    let rows = statement
        .query_map(params![run_id.as_str()], |row| {
            Ok(EvidenceRecordRow {
                run_id: row.get(0)?,
                evidence_id: row.get(1)?,
                kind: row.get(2)?,
                locator: row.get(3)?,
                digest: row.get(4)?,
                media_type: row.get(5)?,
                metadata_json: row.get(6)?,
                source: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(map_sqlite_read_error)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite_read_error)
}

fn load_journal_records(
    conn: &Connection,
    run_id: &RunId,
) -> Result<Vec<JournalRecord>, ExportSnapshotError> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, sequence, outcome, encoded_payload_json
             FROM journal_entries
             WHERE run_id = ?1
             ORDER BY sequence ASC",
        )
        .map_err(map_sqlite_read_error)?;
    let rows = statement
        .query_map(params![run_id.as_str()], |row| {
            Ok(JournalRecord {
                run_id: row.get(0)?,
                sequence: row.get::<_, i64>(1)? as u64,
                outcome: row.get(2)?,
                encoded_payload_json: row.get(3)?,
            })
        })
        .map_err(map_sqlite_read_error)?;
    let records = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite_read_error)?;
    if records.is_empty() {
        return Err(ExportSnapshotError::Failed {
            message: "journal is empty for persisted run".into(),
        });
    }
    let mut expected = 1u64;
    for record in &records {
        mapping::validate_journal_record(record).map_err(map_mapping_error)?;
        if record.sequence != expected {
            return Err(ExportSnapshotError::Failed {
                message: format!(
                    "journal sequence discontinuity: expected {expected}, found {}",
                    record.sequence
                ),
            });
        }
        expected = expected.saturating_add(1);
    }
    require_sequence_one_creation(&records[0])?;
    let tail_sequence = records
        .last()
        .expect("journal validated as nonempty")
        .sequence;
    let allocator_next = load_run_journal_allocator(conn, run_id)?;
    verify_journal_allocator_tail(allocator_next, tail_sequence)?;
    Ok(records)
}

fn load_run_journal_allocator(
    conn: &Connection,
    run_id: &RunId,
) -> Result<u64, ExportSnapshotError> {
    let next_sequence: Option<i64> = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_read_error)?;
    let Some(next_sequence) = next_sequence else {
        return Err(ExportSnapshotError::Failed {
            message: format!(
                "run_journal_sequences row missing for run {}",
                run_id.as_str()
            ),
        });
    };
    if next_sequence <= 0 {
        return Err(ExportSnapshotError::Failed {
            message: format!(
                "run_journal_sequences.next_sequence must be positive, found {next_sequence}"
            ),
        });
    }
    u64::try_from(next_sequence).map_err(|_| ExportSnapshotError::Failed {
        message: "run_journal_sequences.next_sequence out of range".into(),
    })
}

fn require_sequence_one_creation(record: &JournalRecord) -> Result<(), ExportSnapshotError> {
    if record.sequence != 1 {
        return Err(ExportSnapshotError::Failed {
            message: format!(
                "journal must begin at sequence 1, found {}",
                record.sequence
            ),
        });
    }
    let root: Value = serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
        ExportSnapshotError::Failed {
            message: format!("encoded_payload_json: {error}"),
        }
    })?;
    let entry_kind = parse_required_string(&root, "entry_kind")?;
    if parse_entry_kind(entry_kind)? != JournalEntryKind::RunCreated {
        return Err(ExportSnapshotError::Failed {
            message: "journal sequence 1 must be run.created entry kind".into(),
        });
    }
    Ok(())
}

fn verify_journal_allocator_tail(
    allocator_next: u64,
    tail_sequence: u64,
) -> Result<(), ExportSnapshotError> {
    let expected = tail_sequence.saturating_add(1);
    if allocator_next != expected {
        return Err(ExportSnapshotError::Failed {
            message: format!(
                "run_journal_sequences.next_sequence {allocator_next} does not match journal tail {expected}"
            ),
        });
    }
    Ok(())
}

fn map_persistence_error(error: PersistenceError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn map_sqlite_read_error(error: SqliteError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn map_mapping_error(error: MappingError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn sqlite_corrupt(error: SqliteError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn journal_entry_from_record(
    conn: &Connection,
    record: &JournalRecord,
) -> Result<JournalEntry, ExportSnapshotError> {
    mapping::validate_journal_record(record).map_err(map_mapping_error)?;
    let root: Value = serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
        ExportSnapshotError::Failed {
            message: format!("encoded_payload_json: {error}"),
        }
    })?;
    let entry_bytes = record.encoded_payload_json.len();
    let encoded_sizes = encoded_sizes_from_payload(&root, entry_bytes);
    let sequence =
        JournalSequence::try_from(record.sequence).map_err(|_| ExportSnapshotError::Failed {
            message: "sequence must be positive".into(),
        })?;
    let run_id =
        RunId::parse(record.run_id.clone()).map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let observed_at = ObservedAt::parse(parse_required_string(&root, "ts")?).map_err(|error| {
        ExportSnapshotError::Failed {
            message: error.to_string(),
        }
    })?;
    let operation = parse_required_string(&root, "operation")?.to_owned();
    let request_id = RequestId::parse(parse_required_string(&root, "request_id")?.to_owned())
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let outcome = parse_outcome(record.outcome.as_str())?;
    let reason = parse_reason(&root, outcome)?;
    let state_before = parse_state_fact(&root, "state_before")?;
    let state_after = parse_state_fact(&root, "state_after")?;
    let entry_kind = parse_required_string(&root, "entry_kind")?;
    let attempt = parse_attempt(&root, entry_kind, outcome, &observed_at)?;
    if let Some(corrects) = attempt.as_ref().and_then(|facts| facts.corrects_sequence) {
        ensure_correction_target_exists(conn, &run_id, corrects.value())?;
    }
    let extension = parse_extension(&root, entry_kind, outcome)?;
    JournalEntry::new(
        sequence,
        run_id,
        observed_at,
        operation,
        request_id,
        outcome,
        reason,
        state_before,
        state_after,
        attempt,
        extension,
        encoded_sizes,
    )
    .map_err(map_journal_error)
}

fn ensure_correction_target_exists(
    conn: &Connection,
    run_id: &RunId,
    corrects_sequence: u64,
) -> Result<(), ExportSnapshotError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM journal_entries WHERE run_id = ?1 AND sequence = ?2",
            params![
                run_id.as_str(),
                i64::try_from(corrects_sequence).map_err(|_| ExportSnapshotError::Failed {
                    message: "corrects_sequence out of range".into(),
                })?
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_corrupt)?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(ExportSnapshotError::Failed {
            message: format!(
                "corrects_sequence {corrects_sequence} does not reference an earlier journal row"
            ),
        })
    }
}

fn encoded_sizes_from_payload(root: &Value, entry_bytes: usize) -> JournalEncodedSizes {
    JournalEncodedSizes {
        entry: entry_bytes,
        evidence_associations: encoded_field_bytes(root.get("evidence_associations")),
        provider_observations: encoded_field_bytes(root.get("provider_observations")),
        gate_verdict_facts: encoded_field_bytes(root.get("gate_verdict_facts")),
        diagnostics: diagnostics_aggregate_bytes(root.get("diagnostics")),
        note: root
            .get("note")
            .and_then(Value::as_str)
            .map(str::len)
            .unwrap_or(0),
        actor: encoded_field_bytes(root.get("actor")),
    }
}

fn encoded_field_bytes(value: Option<&Value>) -> usize {
    value
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn diagnostics_aggregate_bytes(value: Option<&Value>) -> usize {
    let Some(Value::Array(items)) = value else {
        return 0;
    };
    items
        .iter()
        .filter_map(|item| serde_json::to_vec(item).ok())
        .map(|bytes| bytes.len())
        .sum()
}

fn parse_required_string<'a>(
    root: &'a Value,
    field: &'static str,
) -> Result<&'a str, ExportSnapshotError> {
    root.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ExportSnapshotError::Failed {
            message: format!("missing or invalid {field}"),
        })
}

fn parse_outcome(value: &str) -> Result<OutcomeClass, ExportSnapshotError> {
    match value {
        "completed" => Ok(OutcomeClass::Completed),
        "rejected" => Ok(OutcomeClass::Rejected),
        "error" => Ok(OutcomeClass::Error),
        other => Err(ExportSnapshotError::Failed {
            message: format!("unsupported outcome {other:?}"),
        }),
    }
}

fn parse_reason(
    root: &Value,
    outcome: OutcomeClass,
) -> Result<Option<Reason>, ExportSnapshotError> {
    match root.get("reason") {
        None | Some(Value::Null) => {
            if outcome == OutcomeClass::Completed {
                Ok(None)
            } else {
                Err(ExportSnapshotError::Failed {
                    message: "non-completed journal entry requires reason".into(),
                })
            }
        }
        Some(value) => {
            let code = value.get("code").and_then(Value::as_str).ok_or_else(|| {
                ExportSnapshotError::Failed {
                    message: "reason.code missing".into(),
                }
            })?;
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| ExportSnapshotError::Failed {
                    message: "reason.message missing".into(),
                })?;
            let reason_code =
                reason_code_from_wire(code).ok_or_else(|| ExportSnapshotError::Failed {
                    message: format!("unknown reason code {code:?}"),
                })?;
            Reason::new(reason_code, message)
                .map(Some)
                .map_err(ExportSnapshotError::Bound)
        }
    }
}

fn reason_code_from_wire(code: &str) -> Option<ReasonCode> {
    ReasonCode::ALL
        .into_iter()
        .find(|candidate| candidate.code() == code)
}

fn parse_state_fact(root: &Value, field: &'static str) -> Result<StateFact, ExportSnapshotError> {
    let object = root.get(field).ok_or_else(|| ExportSnapshotError::Failed {
        message: format!("missing {field}"),
    })?;
    let state =
        StateId::parse(parse_required_string(object, "state")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let lifecycle = match parse_required_string(object, "lifecycle")? {
        "active" => Lifecycle::Active,
        "final" => Lifecycle::Final,
        "terminated" => Lifecycle::Terminated,
        other => {
            return Err(ExportSnapshotError::Failed {
                message: format!("unknown lifecycle {other:?}"),
            });
        }
    };
    let workflow_state_version = WorkflowStateVersion::try_from(
        object
            .get("workflow_state_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: format!("{field}.workflow_state_version missing"),
            })?,
    )
    .map_err(|error| ExportSnapshotError::Failed {
        message: error.to_string(),
    })?;
    let lifecycle_version = LifecycleVersion::try_from(
        object
            .get("lifecycle_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: format!("{field}.lifecycle_version missing"),
            })?,
    )
    .map_err(|error| ExportSnapshotError::Failed {
        message: error.to_string(),
    })?;
    Ok(StateFact {
        state,
        lifecycle,
        workflow_state_version,
        lifecycle_version,
    })
}

fn parse_attempt(
    root: &Value,
    entry_kind: &str,
    _outcome: OutcomeClass,
    observed_at: &ObservedAt,
) -> Result<Option<AttemptFacts>, ExportSnapshotError> {
    let kind = parse_entry_kind(entry_kind)?;
    let required = matches!(
        kind,
        JournalEntryKind::RunCreated
            | JournalEntryKind::EvidenceAdded
            | JournalEntryKind::Annotation
            | JournalEntryKind::TransitionAttempt
            | JournalEntryKind::GuidanceAttempt
            | JournalEntryKind::CompatibilityAttempt
    );
    let has_attempt_fields = root.get("transition").is_some()
        || root.get("provider_observations").is_some()
        || root.get("gate_verdict_facts").is_some()
        || root.get("evidence_associations").is_some()
        || root.get("evidence_recorded").is_some()
        || root.get("note").is_some()
        || root.get("actor").is_some()
        || root.get("corrects_sequence").is_some()
        || root.get("diagnostics").is_some();
    if !required && !has_attempt_fields {
        return Ok(None);
    }
    let transition = root.get("transition").map(parse_transition).transpose()?;
    let provider_observations = root
        .get("provider_observations")
        .map(parse_provider_observations)
        .transpose()?
        .unwrap_or_default();
    let gate_verdict_facts = root
        .get("gate_verdict_facts")
        .map(parse_gate_verdict_facts)
        .transpose()?;
    let evidence_associations = root
        .get("evidence_associations")
        .map(|value| parse_evidence_associations(value, observed_at))
        .transpose()?;
    let evidence_recorded = root
        .get("evidence_recorded")
        .map(parse_evidence_recorded)
        .transpose()?;
    let note = root
        .get("note")
        .and_then(Value::as_str)
        .map(Note::new)
        .transpose()
        .map_err(ExportSnapshotError::Bound)?;
    let actor = root.get("actor").map(parse_actor_metadata).transpose()?;
    let corrects_sequence = root
        .get("corrects_sequence")
        .and_then(Value::as_u64)
        .map(JournalSequence::try_from)
        .transpose()
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let diagnostics = root
        .get("diagnostics")
        .map(parse_diagnostics)
        .transpose()?
        .unwrap_or_default();
    let attempt = AttemptFacts {
        transition,
        provider_observations,
        gate_verdict_facts,
        evidence_associations,
        evidence_recorded,
        note,
        actor,
        corrects_sequence,
        diagnostics,
    };
    if required || has_attempt_fields {
        let attempt = attempt.validate().map_err(map_attempt_error)?;
        Ok(Some(attempt))
    } else {
        Ok(None)
    }
}

fn parse_entry_kind(value: &str) -> Result<JournalEntryKind, ExportSnapshotError> {
    match value {
        "run.created" => Ok(JournalEntryKind::RunCreated),
        "evidence.added" => Ok(JournalEntryKind::EvidenceAdded),
        "annotation" => Ok(JournalEntryKind::Annotation),
        "label.changed" => Ok(JournalEntryKind::LabelChanged),
        "transition.attempt" => Ok(JournalEntryKind::TransitionAttempt),
        "guidance.attempt" => Ok(JournalEntryKind::GuidanceAttempt),
        "compatibility.attempt" => Ok(JournalEntryKind::CompatibilityAttempt),
        "run.terminated" => Ok(JournalEntryKind::RunTerminated),
        other => Err(ExportSnapshotError::Failed {
            message: format!("unsupported entry_kind {other:?}"),
        }),
    }
}

fn parse_transition(value: &Value) -> Result<TransitionFact, ExportSnapshotError> {
    let event =
        EventId::parse(parse_required_string(value, "event")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let source = StateId::parse(parse_required_string(value, "source_state")?.to_owned()).map_err(
        |error| ExportSnapshotError::Failed {
            message: error.to_string(),
        },
    )?;
    let applied = value
        .get("applied")
        .and_then(Value::as_bool)
        .ok_or_else(|| ExportSnapshotError::Failed {
            message: "transition.applied missing".into(),
        })?;
    let target = value
        .get("target_state")
        .and_then(Value::as_str)
        .map(|value| StateId::parse(value.to_owned()))
        .transpose()
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    TransitionFact::new(event, source, target, applied).map_err(map_attempt_error)
}

fn parse_provider_observations(value: &Value) -> Result<Vec<ProviderFact>, ExportSnapshotError> {
    let Value::Array(items) = value else {
        return Err(ExportSnapshotError::Failed {
            message: "provider_observations must be an array".into(),
        });
    };
    items
        .iter()
        .map(parse_provider_observation)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_provider_observation(value: &Value) -> Result<ProviderFact, ExportSnapshotError> {
    let registration_id =
        RegistrationId::parse(parse_required_string(value, "registration_id")?.to_owned())
            .map_err(|error| ExportSnapshotError::Failed {
                message: error.to_string(),
            })?;
    let config_revision = value
        .get("config_revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| ExportSnapshotError::Failed {
            message: "provider_observation.config_revision missing".into(),
        })?;
    let role = match parse_required_string(value, "role")? {
        "describe" => ProviderRole::Describe,
        "validate_inputs" => ProviderRole::ValidateInputs,
        "evaluate_gates" => ProviderRole::EvaluateGates,
        "live_guidance" => ProviderRole::LiveGuidance,
        "check_compatibility" => ProviderRole::CheckCompatibility,
        other => {
            return Err(ExportSnapshotError::Failed {
                message: format!("unsupported provider role {other:?}"),
            });
        }
    };
    let invocation_id = RequestId::parse(parse_required_string(value, "invocation_id")?.to_owned())
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let executable = parse_required_string(value, "executable")?.to_owned();
    let outcome = parse_outcome(parse_required_string(value, "outcome")?)?;
    let digest = match value.get("executable_digest").and_then(Value::as_str) {
        Some(value) => {
            DigestObservation::observed(value.to_owned()).map_err(ExportSnapshotError::Bound)?
        }
        None => DigestObservation::Unavailable,
    };
    let provider_version = value
        .get("provider_version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let protocol_major = value.get("protocol_major").and_then(Value::as_u64);
    ProviderFact::new(
        registration_id,
        config_revision,
        role,
        invocation_id,
        executable,
        outcome,
        digest,
        provider_version,
        protocol_major,
    )
    .map_err(ExportSnapshotError::Bound)
}

fn parse_gate_verdict_facts(value: &Value) -> Result<GateVerdictFacts, ExportSnapshotError> {
    let event =
        EventId::parse(parse_required_string(value, "event")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let gate_ids = value
        .get("gate_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ExportSnapshotError::Failed {
            message: "gate_verdict_facts.gate_ids missing".into(),
        })?
        .iter()
        .map(|item| {
            GateId::parse(
                item.as_str()
                    .ok_or_else(|| ExportSnapshotError::Failed {
                        message: "gate_ids entries must be strings".into(),
                    })?
                    .to_owned(),
            )
            .map_err(|error| ExportSnapshotError::Failed {
                message: error.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let has_verdicts = value.get("verdicts").is_some();
    let has_incompatibility = value.get("incompatibility").is_some();
    let has_evaluation_error = value.get("evaluation_error").is_some();
    let variant_count = usize::from(has_verdicts)
        + usize::from(has_incompatibility)
        + usize::from(has_evaluation_error);
    if variant_count != 1 {
        return Err(ExportSnapshotError::Failed {
            message: "gate_verdict_facts requires exactly one result variant".into(),
        });
    }
    let result = if has_verdicts {
        let verdicts = value
            .get("verdicts")
            .and_then(Value::as_array)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: "gate_verdict_facts.verdicts must be an array".into(),
            })?
            .iter()
            .map(parse_gate_verdict)
            .collect::<Result<Vec<_>, _>>()?;
        GateVerdictResult::Verdicts(verdicts)
    } else if has_incompatibility {
        GateVerdictResult::Incompatibility(parse_diagnostic_object(
            value.get("incompatibility").expect("variant key checked"),
        )?)
    } else {
        GateVerdictResult::EvaluationError(
            Diagnostics::new(parse_diagnostics(
                value.get("evaluation_error").expect("variant key checked"),
            )?)
            .map_err(ExportSnapshotError::Bound)?,
        )
    };
    GateVerdictFacts::new(event, gate_ids, result).map_err(map_attempt_error)
}

fn parse_gate_verdict(value: &Value) -> Result<GateVerdictFact, ExportSnapshotError> {
    let gate_id =
        GateId::parse(parse_required_string(value, "gate_id")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let passed = match parse_required_string(value, "status")? {
        "pass" => true,
        "fail" => false,
        other => {
            return Err(ExportSnapshotError::Failed {
                message: format!("unsupported gate verdict status {other:?}"),
            });
        }
    };
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);
    GateVerdictFact::new(gate_id, passed, message).map_err(ExportSnapshotError::Bound)
}

fn parse_evidence_associations(
    value: &Value,
    observed_at: &ObservedAt,
) -> Result<EvidenceAssociations, ExportSnapshotError> {
    let inline = match value.get("inline") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| parse_inline_evidence(item, observed_at))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(ExportSnapshotError::Failed {
                message: "evidence_associations.inline must be an array".into(),
            });
        }
        None => Vec::new(),
    };
    let selected_ids = parse_string_id_list(value.get("selected_ids"), "selected_ids")?
        .into_iter()
        .map(EvidenceId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let provider_recorded_ids =
        parse_string_id_list(value.get("provider_recorded_ids"), "provider_recorded_ids")?
            .into_iter()
            .map(EvidenceId::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ExportSnapshotError::Failed {
                message: error.to_string(),
            })?;
    Ok(EvidenceAssociations {
        inline,
        selected_ids,
        provider_recorded_ids,
    })
}

fn parse_inline_evidence(
    value: &Value,
    observed_at: &ObservedAt,
) -> Result<EvidenceRecord, ExportSnapshotError> {
    let id = EvidenceId::parse(parse_required_string(value, "evidence_id")?.to_owned()).map_err(
        |error| ExportSnapshotError::Failed {
            message: error.to_string(),
        },
    )?;
    let kind =
        EvidenceKind::parse(parse_required_string(value, "kind")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let locator = parse_required_string(value, "locator")?.to_owned();
    let digest = value
        .get("digest")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let media_type = value
        .get("media_type")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let source = match value
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("caller")
    {
        "caller" => EvidenceSource::Caller,
        "provider" => EvidenceSource::Provider,
        other => {
            return Err(ExportSnapshotError::Failed {
                message: format!("unsupported inline evidence source {other:?}"),
            });
        }
    };
    EvidenceRecord::new(
        id,
        kind,
        locator,
        digest,
        media_type,
        None,
        source,
        *observed_at,
    )
    .map_err(ExportSnapshotError::Bound)
}

fn parse_string_id_list(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Vec<String>, ExportSnapshotError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ExportSnapshotError::Failed {
                        message: format!("{field} entries must be strings"),
                    })
            })
            .collect(),
        Some(_) => Err(ExportSnapshotError::Failed {
            message: format!("{field} must be an array"),
        }),
    }
}

fn parse_evidence_recorded(value: &Value) -> Result<EvidenceRecordedStatus, ExportSnapshotError> {
    Ok(EvidenceRecordedStatus {
        inline: value
            .get("inline")
            .and_then(Value::as_bool)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: "evidence_recorded.inline missing".into(),
            })?,
        selected_associations: value
            .get("selected_associations")
            .and_then(Value::as_bool)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: "evidence_recorded.selected_associations missing".into(),
            })?,
        provider: value
            .get("provider")
            .and_then(Value::as_bool)
            .ok_or_else(|| ExportSnapshotError::Failed {
                message: "evidence_recorded.provider missing".into(),
            })?,
    })
}

fn parse_actor_metadata(value: &Value) -> Result<ActorMetadata, ExportSnapshotError> {
    let mapped = protocol_mapping::core_value(value.clone(), "actor").map_err(|error| {
        ExportSnapshotError::Failed {
            message: error.to_string(),
        }
    })?;
    ActorMetadata::new(mapped).map_err(ExportSnapshotError::Bound)
}

fn parse_diagnostics(value: &Value) -> Result<Vec<Diagnostic>, ExportSnapshotError> {
    let Value::Array(items) = value else {
        return Err(ExportSnapshotError::Failed {
            message: "diagnostics must be an array".into(),
        });
    };
    items
        .iter()
        .map(parse_diagnostic_object)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_diagnostic_object(value: &Value) -> Result<Diagnostic, ExportSnapshotError> {
    let code = parse_required_string(value, "code")?.to_owned();
    let message = parse_required_string(value, "message")?.to_owned();
    let path = value.get("path").and_then(Value::as_str).map(str::to_owned);
    Diagnostic::new(code, message, path).map_err(ExportSnapshotError::Bound)
}

fn parse_extension(
    root: &Value,
    entry_kind: &str,
    outcome: OutcomeClass,
) -> Result<JournalExtension, ExportSnapshotError> {
    match entry_kind {
        "run.created" => {
            let graph_revision =
                GraphRevision::parse(parse_required_string(root, "graph_revision")?.to_owned())
                    .map_err(|error| ExportSnapshotError::Failed {
                        message: error.to_string(),
                    })?;
            Ok(JournalExtension::RunCreated { graph_revision })
        }
        "evidence.added" => {
            let added = match (
                root.get("evidence_id"),
                root.get("kind"),
                root.get("locator"),
            ) {
                (Some(_), Some(_), Some(_)) => Some(parse_evidence_added_fact(root)?),
                (None, None, None) if outcome != OutcomeClass::Completed => None,
                _ => {
                    return Err(ExportSnapshotError::Failed {
                        message: "evidence.added extension fields incomplete".into(),
                    });
                }
            };
            Ok(JournalExtension::EvidenceAdded { added })
        }
        "annotation" => Ok(JournalExtension::Annotation),
        "label.changed" => {
            let change = match (root.get("label_before"), root.get("label_after")) {
                (Some(_), Some(_)) => Some(parse_label_change(root)?),
                (None, None) if outcome != OutcomeClass::Completed => None,
                _ => {
                    return Err(ExportSnapshotError::Failed {
                        message: "label.changed extension fields incomplete".into(),
                    });
                }
            };
            Ok(JournalExtension::LabelChanged { change })
        }
        "transition.attempt" => Ok(JournalExtension::TransitionAttempt),
        "guidance.attempt" => {
            use loop_engine_core::model::bounded::BoundedText;
            let guidance_text = root
                .get("guidance_text")
                .and_then(Value::as_str)
                .map(|text| BoundedText::non_empty("guidance_text", text.to_owned()))
                .transpose()
                .map_err(ExportSnapshotError::Bound)?;
            Ok(JournalExtension::GuidanceAttempt { guidance_text })
        }
        "compatibility.attempt" => {
            let findings = root
                .get("findings")
                .map(parse_compatibility_findings)
                .transpose()?;
            Ok(JournalExtension::CompatibilityAttempt { findings })
        }
        "run.terminated" => Ok(JournalExtension::RunTerminated),
        other => Err(ExportSnapshotError::Failed {
            message: format!("unsupported entry_kind {other:?}"),
        }),
    }
}

fn parse_evidence_added_fact(root: &Value) -> Result<EvidenceAddedFact, ExportSnapshotError> {
    use loop_engine_core::model::bounded::BoundedText;
    let evidence_id = EvidenceId::parse(parse_required_string(root, "evidence_id")?.to_owned())
        .map_err(|error| ExportSnapshotError::Failed {
            message: error.to_string(),
        })?;
    let kind =
        EvidenceKind::parse(parse_required_string(root, "kind")?.to_owned()).map_err(|error| {
            ExportSnapshotError::Failed {
                message: error.to_string(),
            }
        })?;
    let locator =
        BoundedText::opaque_non_empty("evidence_locator", parse_required_string(root, "locator")?)
            .map_err(ExportSnapshotError::Bound)?;
    let digest = root
        .get("digest")
        .and_then(Value::as_str)
        .map(|value| BoundedText::opaque_non_empty("evidence_digest", value))
        .transpose()
        .map_err(ExportSnapshotError::Bound)?;
    Ok(EvidenceAddedFact {
        evidence_id,
        kind,
        locator,
        digest,
    })
}

fn parse_label_change(root: &Value) -> Result<LabelChangeFact, ExportSnapshotError> {
    let label_before = parse_optional_label(root.get("label_before"))?;
    let label_after = parse_optional_label(root.get("label_after"))?;
    Ok(LabelChangeFact {
        label_before,
        label_after,
    })
}

fn parse_optional_label(
    value: Option<&Value>,
) -> Result<Option<loop_engine_core::model::bounded::BoundedText<256>>, ExportSnapshotError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            loop_engine_core::model::bounded::BoundedText::non_empty("label", text.clone())
                .map(Some)
                .map_err(ExportSnapshotError::Bound)
        }
        Some(_) => Err(ExportSnapshotError::Failed {
            message: "label fields must be strings or null".into(),
        }),
    }
}

fn parse_compatibility_findings(
    value: &Value,
) -> Result<CompatibilityFindings, ExportSnapshotError> {
    let Value::Array(items) = value else {
        return Err(ExportSnapshotError::Failed {
            message: "findings must be an array".into(),
        });
    };
    let findings = items
        .iter()
        .map(parse_compatibility_finding)
        .collect::<Result<Vec<_>, _>>()?;
    CompatibilityFindings::new(findings).map_err(|error| ExportSnapshotError::Failed {
        message: error.to_string(),
    })
}

fn parse_compatibility_finding(value: &Value) -> Result<CompatibilityFinding, ExportSnapshotError> {
    use loop_engine_core::model::compatibility::CompatibilityStatus;
    let capability = parse_required_string(value, "capability")?.to_owned();
    let status = match parse_required_string(value, "status")? {
        "compatible" => CompatibilityStatus::Compatible,
        "incompatible" => CompatibilityStatus::Incompatible,
        "unknown" => CompatibilityStatus::Unknown,
        other => {
            return Err(ExportSnapshotError::Failed {
                message: format!("unsupported compatibility status {other:?}"),
            });
        }
    };
    let diagnostics = match value.get("message").and_then(Value::as_str) {
        None => Vec::new(),
        Some(message) => {
            vec![
                Diagnostic::new("compatibility.message", message, None)
                    .map_err(ExportSnapshotError::Bound)?,
            ]
        }
    };
    CompatibilityFinding::new(capability, status, diagnostics).map_err(ExportSnapshotError::Bound)
}

fn map_journal_error(error: JournalError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn map_attempt_error(error: loop_engine_core::model::attempt::AttemptError) -> ExportSnapshotError {
    ExportSnapshotError::Failed {
        message: error.to_string(),
    }
}

fn journal_entries_from_records(
    conn: &Connection,
    records: &[JournalRecord],
) -> Result<Vec<JournalEntry>, ExportSnapshotError> {
    let mut entries = Vec::with_capacity(records.len());
    for record in records {
        entries.push(journal_entry_from_record(conn, record)?);
    }
    Ok(entries)
}

#[cfg(test)]
pub mod test_support {
    use rusqlite::{Connection, params};
    use serde_json::json;

    use crate::persistence::records::{GV01_CANONICAL_GRAPH_JSON, GV01_GRAPH_REVISION};

    fn creation_payload(run_id: &str) -> String {
        json!({
            "journal_schema_version": 1,
            "sequence": 1,
            "run_id": run_id,
            "ts": "2026-07-17T14:00:00.123Z",
            "operation": "run.create",
            "request_id": "req-create-001",
            "entry_kind": "run.created",
            "outcome": "completed",
            "reason": null,
            "state_before": {
                "state": "draft",
                "lifecycle": "active",
                "workflow_state_version": 1,
                "lifecycle_version": 1
            },
            "state_after": {
                "state": "draft",
                "lifecycle": "active",
                "workflow_state_version": 1,
                "lifecycle_version": 1
            },
            "provider_observations": [{
                "registration_id": "reg-1",
                "config_revision": 1,
                "role": "describe",
                "invocation_id": "pv-describe-001",
                "executable": "/bin/provider",
                "outcome": "completed"
            }],
            "graph_revision": GV01_GRAPH_REVISION
        })
        .to_string()
    }

    pub fn seed_minimal_run(conn: &Connection, run_id: &str) {
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
            ) VALUES ('reg-1', 'provider-a', 1, 1, '/bin/provider', '[]', '/work', 60,
                      '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES (?1, 'reg-1', 1, 'draft', 'active', 1, 1, 1, NULL, ?2, 1, ?3, '{}', '2026-07-17T12:00:00.000Z')",
            params![run_id, GV01_GRAPH_REVISION, GV01_CANONICAL_GRAPH_JSON],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 1)",
            params![run_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
             VALUES (?1, 1, 'completed', ?2)",
            params![run_id, creation_payload(run_id)],
        )
        .unwrap();
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = 2 WHERE run_id = ?1",
            params![run_id],
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use loop_engine_core::capabilities::time::TimeSource;
    use loop_engine_core::model::attempt::GateVerdictResult;
    use loop_engine_core::model::ids::{EventId, GateId, RunId};
    use loop_engine_core::model::outcome::OutcomeClass;
    use loop_engine_core::model::time::ObservedAt;
    use rusqlite::{Connection, params};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{
        ExportSnapshotError, load_consistent_snapshot_with_after_run, parse_attempt,
        parse_gate_verdict_facts, test_support,
    };
    use crate::persistence::{SqliteStore, connect_with_pragmas};

    #[derive(Debug, Clone, Copy)]
    struct FixedClock {
        observed_at: ObservedAt,
    }

    impl TimeSource for FixedClock {
        type Error = Infallible;

        fn now(&self) -> Result<ObservedAt, Self::Error> {
            Ok(self.observed_at)
        }
    }

    #[test]
    fn load_consistent_snapshot_excludes_rows_committed_during_read_transaction() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        let run_id = "run-snapshot-isolation";
        test_support::seed_minimal_run(store.connection(), run_id);
        let db_path = db_path.clone();
        let run_id = run_id.to_owned();
        let loaded = Arc::new(Barrier::new(2));
        let writer_done = Arc::new(Barrier::new(2));
        let loaded_for_callback = loaded.clone();
        let writer_done_for_callback = writer_done.clone();

        let write_conn = connect_with_pragmas(&db_path).unwrap();
        let db_path_for_reader = db_path.clone();
        let run_id_for_reader = run_id.clone();
        let reader = thread::spawn(move || {
            load_consistent_snapshot_with_after_run(
                &db_path_for_reader,
                &RunId::parse(&run_id_for_reader).unwrap(),
                &FixedClock {
                    observed_at: ObservedAt::parse("2026-07-17T15:00:00.000Z").unwrap(),
                },
                || {
                    loaded_for_callback.wait();
                    writer_done_for_callback.wait();
                },
            )
        });

        loaded.wait();
        let late_payload = serde_json::json!({
            "journal_schema_version": 1,
            "sequence": 2,
            "run_id": run_id,
            "ts": "2026-07-17T16:00:00.000Z",
            "operation": "run.annotate",
            "request_id": "req-annotate-002",
            "entry_kind": "annotation",
            "outcome": "completed",
            "reason": null,
            "state_before": {"state": "draft", "lifecycle": "active", "workflow_state_version": 1, "lifecycle_version": 1},
            "state_after": {"state": "draft", "lifecycle": "active", "workflow_state_version": 1, "lifecycle_version": 1},
            "note": "late writer"
        })
        .to_string();
        let write_result = (|| {
            write_conn.execute(
                "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
                 VALUES (?1, 2, 'completed', ?2)",
                params![run_id, late_payload],
            )?;
            write_conn.execute(
                "UPDATE run_journal_sequences SET next_sequence = 3 WHERE run_id = ?1",
                params![run_id],
            )?;
            Ok::<_, rusqlite::Error>(())
        })();
        writer_done.wait();
        write_result.unwrap();

        let snapshot = reader.join().unwrap().expect("snapshot load");
        assert_eq!(snapshot.journal_records.len(), 1);
        assert_eq!(snapshot.journal_records[0].sequence, 1);

        let live_count: i64 = Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_count, 2);
    }

    fn producer_gate_verdict_wire(result: &str, gate_ids: &[&str]) -> Value {
        match result {
            "verdicts" => json!({
                "event": "go",
                "gate_ids": gate_ids,
                "verdicts": gate_ids.iter().map(|gate_id| json!({
                    "gate_id": gate_id,
                    "status": "pass",
                })).collect::<Vec<_>>(),
            }),
            "incompatibility" => json!({
                "event": "go",
                "gate_ids": gate_ids,
                "incompatibility": {
                    "code": "protocol.incompatible",
                    "message": "unsupported protocol major",
                    "path": "/protocol_major",
                },
            }),
            "evaluation_error" => json!({
                "event": "go",
                "gate_ids": gate_ids,
                "evaluation_error": [{
                    "code": "provider.evaluation_error",
                    "message": "gate evaluation failed",
                    "path": "/result",
                }],
            }),
            other => panic!("unsupported gate verdict result variant {other:?}"),
        }
    }

    #[test]
    fn gate_verdict_facts_roundtrip_all_producer_variants() {
        let gate_ids = ["gate-alpha", "gate-beta"];
        let expected_gate_ids = gate_ids
            .iter()
            .map(|id| GateId::parse(*id).unwrap())
            .collect::<Vec<_>>();
        let evidence_associations = json!({
            "inline": [{
                "evidence_id": "evidence-inline-1",
                "kind": "document",
                "locator": "file:///evidence.pdf",
            }],
            "selected_ids": [],
            "provider_recorded_ids": [],
        });

        let verdicts =
            parse_gate_verdict_facts(&producer_gate_verdict_wire("verdicts", &gate_ids)).unwrap();
        assert_eq!(verdicts.event, EventId::parse("go").unwrap());
        assert_eq!(verdicts.gate_ids, expected_gate_ids);
        match verdicts.result {
            GateVerdictResult::Verdicts(parsed) => {
                assert_eq!(parsed.len(), gate_ids.len());
                for (gate_id, verdict) in gate_ids.iter().zip(parsed) {
                    assert_eq!(verdict.gate_id.as_str(), *gate_id);
                    assert!(verdict.passed);
                }
            }
            other => panic!("expected verdicts variant, got {other:?}"),
        }

        let incompatibility =
            parse_gate_verdict_facts(&producer_gate_verdict_wire("incompatibility", &gate_ids))
                .unwrap();
        assert_eq!(incompatibility.gate_ids, expected_gate_ids);
        match incompatibility.result {
            GateVerdictResult::Incompatibility(diagnostic) => {
                assert_eq!(diagnostic.code(), "protocol.incompatible");
                assert_eq!(diagnostic.message(), "unsupported protocol major");
                assert_eq!(diagnostic.path(), Some("/protocol_major"));
            }
            other => panic!("expected incompatibility variant, got {other:?}"),
        }

        let evaluation_error =
            parse_gate_verdict_facts(&producer_gate_verdict_wire("evaluation_error", &gate_ids))
                .unwrap();
        assert_eq!(evaluation_error.gate_ids, expected_gate_ids);
        match evaluation_error.result {
            GateVerdictResult::EvaluationError(diagnostics) => {
                assert_eq!(diagnostics.as_slice().len(), 1);
                assert_eq!(
                    diagnostics.as_slice()[0].code(),
                    "provider.evaluation_error"
                );
            }
            other => panic!("expected evaluation_error variant, got {other:?}"),
        }

        let observed_at = ObservedAt::parse("2026-07-17T14:00:00.123Z").unwrap();
        let attempt_root = json!({
            "transition": {
                "event": "go",
                "source_state": "draft",
                "target_state": "review",
                "applied": false,
            },
            "gate_verdict_facts": producer_gate_verdict_wire("verdicts", &gate_ids),
            "evidence_associations": evidence_associations,
            "evidence_recorded": {
                "inline": true,
                "selected_associations": false,
                "provider": false,
            },
        });
        let attempt = parse_attempt(
            &attempt_root,
            "transition.attempt",
            OutcomeClass::Completed,
            &observed_at,
        )
        .unwrap()
        .expect("attempt facts");
        let parsed_gate = attempt
            .gate_verdict_facts
            .expect("gate verdict facts in attempt");
        assert_eq!(parsed_gate.gate_ids, expected_gate_ids);
        assert!(matches!(parsed_gate.result, GateVerdictResult::Verdicts(_)));
        let associations = attempt
            .evidence_associations
            .expect("evidence associations in attempt");
        assert_eq!(associations.inline.len(), 1);
        assert_eq!(associations.inline[0].id().as_str(), "evidence-inline-1");
    }

    #[test]
    fn gate_verdict_facts_rejects_missing_or_multiple_result_variants() {
        let gate_ids = json!("gate-1");
        assert!(matches!(
            parse_gate_verdict_facts(&json!({
                "event": "go",
                "gate_ids": [gate_ids.clone()],
            })),
            Err(ExportSnapshotError::Failed { .. })
        ));
        assert!(matches!(
            parse_gate_verdict_facts(&json!({
                "event": "go",
                "gate_ids": [gate_ids.clone()],
                "verdicts": [{"gate_id": "gate-1", "status": "pass"}],
                "incompatibility": {"code": "x", "message": "y"},
            })),
            Err(ExportSnapshotError::Failed { .. })
        ));
        assert!(matches!(
            parse_gate_verdict_facts(&json!({
                "event": "go",
                "gate_ids": [gate_ids],
                "verdicts": [{"gate_id": "gate-1", "status": "pass"}],
                "evaluation_error": [{"code": "x", "message": "y"}],
            })),
            Err(ExportSnapshotError::Failed { .. })
        ));
    }

    fn state_json() -> Value {
        json!({
            "state": "draft",
            "lifecycle": "active",
            "workflow_state_version": 1,
            "lifecycle_version": 1
        })
    }

    fn annotation_payload(run_id: &str, sequence: u64) -> String {
        json!({
            "journal_schema_version": 1,
            "sequence": sequence,
            "run_id": run_id,
            "ts": "2026-07-17T15:00:00.000Z",
            "operation": "run.annotate",
            "request_id": format!("req-annotate-{sequence:03}"),
            "entry_kind": "annotation",
            "outcome": "completed",
            "reason": null,
            "state_before": state_json(),
            "state_after": state_json(),
            "note": "note"
        })
        .to_string()
    }

    fn insert_journal(conn: &Connection, run_id: &str, sequence: u64, payload: &str) {
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
             VALUES (?1, ?2, 'completed', ?3)",
            params![run_id, i64::try_from(sequence).unwrap(), payload],
        )
        .unwrap();
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = ?1 WHERE run_id = ?2",
            params![i64::try_from(sequence.saturating_add(1)).unwrap(), run_id],
        )
        .unwrap();
    }

    fn load_snapshot(
        db_path: &std::path::Path,
        run_id: &str,
    ) -> Result<super::ExportSnapshot, ExportSnapshotError> {
        super::load_consistent_snapshot(
            db_path,
            &RunId::parse(run_id).unwrap(),
            &FixedClock {
                observed_at: ObservedAt::parse("2026-07-17T15:00:00.000Z").unwrap(),
            },
        )
    }

    #[test]
    fn export_rejects_run_without_journal() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        let conn = store.connection();
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
            ) VALUES ('reg-1', 'provider-a', 1, 1, '/bin/provider', '[]', '/work', 60,
                      '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES ('run-1', 'reg-1', 1, 'draft', 'active', 1, 1, 1, NULL,
                      'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                      1, '{}', '{}', '2026-07-17T12:00:00.000Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES ('run-1', 1)",
            [],
        )
        .unwrap();

        assert!(matches!(
            load_snapshot(&db_path, "run-1"),
            Err(ExportSnapshotError::Failed { .. })
        ));
    }

    #[test]
    fn export_rejects_missing_journal_allocator() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        test_support::seed_minimal_run(store.connection(), "run-1");
        store
            .connection()
            .execute(
                "DELETE FROM run_journal_sequences WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();

        assert!(matches!(
            load_snapshot(&db_path, "run-1"),
            Err(ExportSnapshotError::Failed { .. })
        ));
    }

    #[test]
    fn export_rejects_wrong_journal_allocator() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        let conn = store.connection();
        test_support::seed_minimal_run(conn, "run-1");
        insert_journal(conn, "run-1", 2, &annotation_payload("run-1", 2));
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = 99 WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();

        assert!(matches!(
            load_snapshot(&db_path, "run-1"),
            Err(ExportSnapshotError::Failed { .. })
        ));
    }

    #[test]
    fn export_rejects_sequence_one_non_creation() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        let conn = store.connection();
        test_support::seed_minimal_run(conn, "run-1");
        conn.execute("DELETE FROM journal_entries WHERE run_id = 'run-1'", [])
            .unwrap();
        insert_journal(conn, "run-1", 1, &annotation_payload("run-1", 1));

        assert!(matches!(
            load_snapshot(&db_path, "run-1"),
            Err(ExportSnapshotError::Failed { .. })
        ));
    }
}
