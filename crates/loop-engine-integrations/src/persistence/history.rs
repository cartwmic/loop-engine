//! Immutable per-run journal history reads ordered by positive `sequence` (T115).
//!
//! History rows are decoded from stored journal payloads only; authoritative run columns
//! are never read or replayed to infer current workflow state.

use std::collections::BTreeMap;
use std::path::PathBuf;

use loop_engine_core::capabilities::run_reader::RunHistoryReader;
use loop_engine_core::capabilities::{Page, PageCursor, PageRequest};
use loop_engine_core::model::annotation::{ActorMetadata, Note};
use loop_engine_core::model::attempt::{
    AttemptFacts, EvidenceAddedFact, EvidenceAssociations, GateVerdictFact, GateVerdictFacts,
    GateVerdictResult, JournalExtension, LabelChangeFact, ProviderFact, ProviderRole,
    TransitionFact,
};
use loop_engine_core::model::bounded::{
    BoundError, COLLECTION_PAGE_DATA_BUDGET_BYTES, JOURNAL_ENTRY_ENCODED_BYTES,
    OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
};
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
use loop_engine_core::operations::paging::{self, PagingError, bounded_page};
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, Row, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::persistence::error::PersistenceError;
use crate::persistence::mapping::{self, MappingError};
use crate::persistence::records::JournalRecord;
use crate::persistence::sqlite::connect_read_only_with_pragmas;
use crate::persistence::sqlite::{
    INTEGRATION_METADATA_TABLE, INTEGRITY_KEY_BYTE_LENGTH, INTEGRITY_KEY_ROW_KEY,
};
use crate::persistence::traced::{
    MutationClass, OptionalTraceSink, ReadCompleteExtras, close_read, history_read_failure,
    history_read_rejected,
};
use crate::provider_protocol::mapping as protocol_mapping;

const COLLECTION_HISTORY: &str = "run.history";
const CURSOR_DOMAIN: &[u8] = b"loop-engine.integrations.cursor-v1";
const STRUCTURED_CLI_ENVELOPE_BYTES: usize = 4_194_304;
const ENVELOPE_FRAMING_HEADROOM_BYTES: usize = 1_048_576;

#[allow(dead_code)]
const _ENVELOPE_BOUNDS_CHECK: () = {
    assert!(
        JOURNAL_ENTRY_ENCODED_BYTES + ENVELOPE_FRAMING_HEADROOM_BYTES
            <= STRUCTURED_CLI_ENVELOPE_BYTES
    );
    assert!(JOURNAL_ENTRY_ENCODED_BYTES <= COLLECTION_PAGE_DATA_BUDGET_BYTES);
};

/// SQLite-backed immutable journal history reader.
#[derive(Debug, Clone)]
pub struct SqliteHistoryReads {
    path: PathBuf,
    trace: OptionalTraceSink,
}

#[derive(Debug, Error)]
pub enum HistoryReadError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("stored journal data is corrupt: {message}")]
    Corrupt { message: String },
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Page(#[from] PagingError),
    #[error(transparent)]
    Bound(#[from] BoundError),
}

impl SqliteHistoryReads {
    /// Untraced bootstrap constructor (tests and internal wiring without an operational trace).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_trace(path, OptionalTraceSink::none())
    }

    pub fn with_trace(path: impl Into<PathBuf>, trace: OptionalTraceSink) -> Self {
        Self {
            path: path.into(),
            trace,
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Returns immutable journal rows for `run_id` in ascending per-run sequence order.
    pub fn history(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, HistoryReadError> {
        self.history_for_operation("run.history", run_id, request)
    }

    pub fn history_for_operation(
        &self,
        operation_id: &'static str,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, HistoryReadError> {
        close_read(
            &self.trace,
            operation_id,
            MutationClass::ReadOnly,
            || self.history_impl(run_id, request),
            |page| {
                let page_data_bytes = page
                    .rows
                    .iter()
                    .map(JournalEntry::encoded_size)
                    .sum::<usize>() as u64;
                ReadCompleteExtras::for_page(page, page_data_bytes)
            },
            history_read_rejected,
            history_read_failure,
        )
    }

    fn history_impl(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, HistoryReadError> {
        let conn = connect_read_only_with_pragmas(&self.path)?;
        let _snapshot = ReadOnlySnapshot::begin(&conn)?;
        ensure_run_exists(&conn, run_id)?;
        let allocator_next = load_run_journal_allocator(&conn, run_id)?;
        let integrity_key = load_integrity_key(&conn)?;
        let filter_fingerprint = history_filter_fingerprint(run_id);
        let after_sequence = match request.cursor() {
            None => 0u64,
            Some(cursor) => {
                decode_history_cursor(cursor, &integrity_key, run_id, &filter_fingerprint)?
            }
        };

        let fetch = usize::from(request.limit()).saturating_add(1);
        let mut statement = conn
            .prepare(
                "SELECT run_id, sequence, outcome, encoded_payload_json
                 FROM journal_entries
                 WHERE run_id = ?1 AND sequence > ?2
                 ORDER BY sequence ASC
                 LIMIT ?3",
            )
            .map_err(sqlite_corrupt)?;
        let mut query = statement
            .query(params![
                run_id.as_str(),
                i64::try_from(after_sequence).map_err(|_| HistoryReadError::Corrupt {
                    message: "cursor sequence out of range".into(),
                })?,
                i64::try_from(fetch).map_err(|_| HistoryReadError::Corrupt {
                    message: "page fetch limit out of range".into(),
                })?
            ])
            .map_err(sqlite_corrupt)?;

        let mut candidates = Vec::new();
        let mut expected_sequence = after_sequence.saturating_add(1);
        if after_sequence == 0 {
            expected_sequence = 1;
        }

        while let Some(row) = query.next().map_err(sqlite_corrupt)? {
            let record = journal_record_from_row(row)?;
            if record.run_id != run_id.as_str() {
                return Err(HistoryReadError::Corrupt {
                    message: "journal row run_id does not match query".into(),
                });
            }
            if record.sequence != expected_sequence {
                return Err(HistoryReadError::Corrupt {
                    message: format!(
                        "journal sequence discontinuity: expected {expected_sequence}, found {}",
                        record.sequence
                    ),
                });
            }
            expected_sequence = expected_sequence.saturating_add(1);
            let encoded_bytes = record.encoded_payload_json.len();
            if encoded_bytes > JOURNAL_ENTRY_ENCODED_BYTES {
                return Err(HistoryReadError::Corrupt {
                    message: format!(
                        "journal entry exceeds encoded bound: {encoded_bytes} > {JOURNAL_ENTRY_ENCODED_BYTES}"
                    ),
                });
            }
            let entry = journal_entry_from_record(&conn, &record)?;
            candidates.push((entry, encoded_bytes, record.sequence));
        }

        require_first_page_journal_invariants(after_sequence, &candidates)?;

        let count_limit = usize::from(request.limit());
        let byte_limit = request.byte_limit();
        let next_cursor = first_unreturned_cursor(
            &candidates,
            count_limit,
            byte_limit,
            run_id,
            &filter_fingerprint,
            &integrity_key,
        )?;
        let page = bounded_page(
            candidates.into_iter().map(|(entry, size, _)| (entry, size)),
            count_limit,
            byte_limit,
            next_cursor,
        )?;
        if page.next_cursor.is_none()
            && let Some(last) = page.rows.last()
        {
            verify_journal_allocator_tail(allocator_next, last.sequence().value())?;
        }
        Ok(page)
    }
}

impl RunHistoryReader for SqliteHistoryReads {
    type Error = HistoryReadError;

    fn history(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, Self::Error> {
        SqliteHistoryReads::history(self, run_id, request)
    }
}

struct ReadOnlySnapshot<'conn> {
    conn: &'conn Connection,
}

impl<'conn> ReadOnlySnapshot<'conn> {
    fn begin(conn: &'conn Connection) -> Result<Self, HistoryReadError> {
        conn.execute("BEGIN DEFERRED", []).map_err(sqlite_corrupt)?;
        Ok(Self { conn })
    }
}

impl Drop for ReadOnlySnapshot<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("ROLLBACK");
    }
}

fn decode_journal_sequence(value: i64) -> Result<u64, HistoryReadError> {
    if value <= 0 {
        return Err(HistoryReadError::Corrupt {
            message: format!("sequence must be positive, found {value}"),
        });
    }
    u64::try_from(value).map_err(|_| HistoryReadError::Corrupt {
        message: "sequence out of range".into(),
    })
}

fn journal_record_from_row(row: &Row<'_>) -> Result<JournalRecord, HistoryReadError> {
    Ok(JournalRecord {
        run_id: row.get("run_id").map_err(sqlite_corrupt)?,
        sequence: decode_journal_sequence(row.get("sequence").map_err(sqlite_corrupt)?)?,
        outcome: row.get("outcome").map_err(sqlite_corrupt)?,
        encoded_payload_json: row.get("encoded_payload_json").map_err(sqlite_corrupt)?,
    })
}

fn journal_entry_from_record(
    conn: &Connection,
    record: &JournalRecord,
) -> Result<JournalEntry, HistoryReadError> {
    mapping::validate_journal_record(record).map_err(map_mapping_error)?;
    let root: Value = serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
        HistoryReadError::Corrupt {
            message: format!("encoded_payload_json: {error}"),
        }
    })?;
    let entry_bytes = record.encoded_payload_json.len();
    let encoded_sizes = encoded_sizes_from_payload(&root, entry_bytes);
    let sequence =
        JournalSequence::try_from(record.sequence).map_err(|_| HistoryReadError::Corrupt {
            message: "sequence must be positive".into(),
        })?;
    let run_id =
        RunId::parse(record.run_id.clone()).map_err(|error| HistoryReadError::Corrupt {
            message: error.to_string(),
        })?;
    let observed_at = ObservedAt::parse(parse_required_string(&root, "ts")?).map_err(|error| {
        HistoryReadError::Corrupt {
            message: error.to_string(),
        }
    })?;
    let operation = parse_required_string(&root, "operation")?.to_owned();
    let request_id = RequestId::parse(parse_required_string(&root, "request_id")?.to_owned())
        .map_err(|error| HistoryReadError::Corrupt {
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

/// Validates one stored journal row's wire payload and semantic facts on `conn`.
pub fn validate_journal_record_semantics(
    conn: &Connection,
    record: &JournalRecord,
) -> Result<(), HistoryReadError> {
    journal_entry_from_record(conn, record).map(|_| ())
}

fn ensure_correction_target_exists(
    conn: &Connection,
    run_id: &RunId,
    corrects_sequence: u64,
) -> Result<(), HistoryReadError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM journal_entries WHERE run_id = ?1 AND sequence = ?2",
            params![
                run_id.as_str(),
                i64::try_from(corrects_sequence).map_err(|_| HistoryReadError::Corrupt {
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
        Err(HistoryReadError::Corrupt {
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
) -> Result<&'a str, HistoryReadError> {
    root.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| HistoryReadError::Corrupt {
            message: format!("missing or invalid {field}"),
        })
}

fn parse_outcome(value: &str) -> Result<OutcomeClass, HistoryReadError> {
    match value {
        "completed" => Ok(OutcomeClass::Completed),
        "rejected" => Ok(OutcomeClass::Rejected),
        "error" => Ok(OutcomeClass::Error),
        other => Err(HistoryReadError::Corrupt {
            message: format!("unsupported outcome {other:?}"),
        }),
    }
}

fn parse_reason(root: &Value, outcome: OutcomeClass) -> Result<Option<Reason>, HistoryReadError> {
    match root.get("reason") {
        None | Some(Value::Null) => {
            if outcome == OutcomeClass::Completed {
                Ok(None)
            } else {
                Err(HistoryReadError::Corrupt {
                    message: "non-completed journal entry requires reason".into(),
                })
            }
        }
        Some(value) => {
            let code = value.get("code").and_then(Value::as_str).ok_or_else(|| {
                HistoryReadError::Corrupt {
                    message: "reason.code missing".into(),
                }
            })?;
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| HistoryReadError::Corrupt {
                    message: "reason.message missing".into(),
                })?;
            let reason_code =
                reason_code_from_wire(code).ok_or_else(|| HistoryReadError::Corrupt {
                    message: format!("unknown reason code {code:?}"),
                })?;
            Reason::new(reason_code, message)
                .map(Some)
                .map_err(HistoryReadError::Bound)
        }
    }
}

fn reason_code_from_wire(code: &str) -> Option<ReasonCode> {
    ReasonCode::ALL
        .into_iter()
        .find(|candidate| candidate.code() == code)
}

fn parse_state_fact(root: &Value, field: &'static str) -> Result<StateFact, HistoryReadError> {
    let object = root.get(field).ok_or_else(|| HistoryReadError::Corrupt {
        message: format!("missing {field}"),
    })?;
    let state =
        StateId::parse(parse_required_string(object, "state")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
                message: error.to_string(),
            }
        })?;
    let lifecycle = match parse_required_string(object, "lifecycle")? {
        "active" => Lifecycle::Active,
        "final" => Lifecycle::Final,
        "terminated" => Lifecycle::Terminated,
        other => {
            return Err(HistoryReadError::Corrupt {
                message: format!("unknown lifecycle {other:?}"),
            });
        }
    };
    let workflow_state_version = WorkflowStateVersion::try_from(
        object
            .get("workflow_state_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| HistoryReadError::Corrupt {
                message: format!("{field}.workflow_state_version missing"),
            })?,
    )
    .map_err(|error| HistoryReadError::Corrupt {
        message: error.to_string(),
    })?;
    let lifecycle_version = LifecycleVersion::try_from(
        object
            .get("lifecycle_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| HistoryReadError::Corrupt {
                message: format!("{field}.lifecycle_version missing"),
            })?,
    )
    .map_err(|error| HistoryReadError::Corrupt {
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
) -> Result<Option<AttemptFacts>, HistoryReadError> {
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
        .map_err(HistoryReadError::Bound)?;
    let actor = root.get("actor").map(parse_actor_metadata).transpose()?;
    let corrects_sequence = root
        .get("corrects_sequence")
        .and_then(Value::as_u64)
        .map(JournalSequence::try_from)
        .transpose()
        .map_err(|error| HistoryReadError::Corrupt {
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

fn parse_entry_kind(value: &str) -> Result<JournalEntryKind, HistoryReadError> {
    match value {
        "run.created" => Ok(JournalEntryKind::RunCreated),
        "evidence.added" => Ok(JournalEntryKind::EvidenceAdded),
        "annotation" => Ok(JournalEntryKind::Annotation),
        "label.changed" => Ok(JournalEntryKind::LabelChanged),
        "transition.attempt" => Ok(JournalEntryKind::TransitionAttempt),
        "guidance.attempt" => Ok(JournalEntryKind::GuidanceAttempt),
        "compatibility.attempt" => Ok(JournalEntryKind::CompatibilityAttempt),
        "run.terminated" => Ok(JournalEntryKind::RunTerminated),
        other => Err(HistoryReadError::Corrupt {
            message: format!("unsupported entry_kind {other:?}"),
        }),
    }
}

fn parse_transition(value: &Value) -> Result<TransitionFact, HistoryReadError> {
    let event =
        EventId::parse(parse_required_string(value, "event")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
                message: error.to_string(),
            }
        })?;
    let source = StateId::parse(parse_required_string(value, "source_state")?.to_owned()).map_err(
        |error| HistoryReadError::Corrupt {
            message: error.to_string(),
        },
    )?;
    let applied = value
        .get("applied")
        .and_then(Value::as_bool)
        .ok_or_else(|| HistoryReadError::Corrupt {
            message: "transition.applied missing".into(),
        })?;
    let target = value
        .get("target_state")
        .and_then(Value::as_str)
        .map(|value| StateId::parse(value.to_owned()))
        .transpose()
        .map_err(|error| HistoryReadError::Corrupt {
            message: error.to_string(),
        })?;
    TransitionFact::new(event, source, target, applied).map_err(map_attempt_error)
}

fn parse_provider_observations(value: &Value) -> Result<Vec<ProviderFact>, HistoryReadError> {
    let Value::Array(items) = value else {
        return Err(HistoryReadError::Corrupt {
            message: "provider_observations must be an array".into(),
        });
    };
    items
        .iter()
        .map(parse_provider_observation)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_provider_observation(value: &Value) -> Result<ProviderFact, HistoryReadError> {
    let registration_id =
        RegistrationId::parse(parse_required_string(value, "registration_id")?.to_owned())
            .map_err(|error| HistoryReadError::Corrupt {
                message: error.to_string(),
            })?;
    let config_revision = value
        .get("config_revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| HistoryReadError::Corrupt {
            message: "provider_observation.config_revision missing".into(),
        })?;
    let role = match parse_required_string(value, "role")? {
        "describe" => ProviderRole::Describe,
        "validate_inputs" => ProviderRole::ValidateInputs,
        "evaluate_gates" => ProviderRole::EvaluateGates,
        "live_guidance" => ProviderRole::LiveGuidance,
        "check_compatibility" => ProviderRole::CheckCompatibility,
        other => {
            return Err(HistoryReadError::Corrupt {
                message: format!("unsupported provider role {other:?}"),
            });
        }
    };
    let invocation_id = RequestId::parse(parse_required_string(value, "invocation_id")?.to_owned())
        .map_err(|error| HistoryReadError::Corrupt {
            message: error.to_string(),
        })?;
    let executable = parse_required_string(value, "executable")?.to_owned();
    let outcome = parse_outcome(parse_required_string(value, "outcome")?)?;
    let digest = match value.get("executable_digest").and_then(Value::as_str) {
        Some(value) => {
            DigestObservation::observed(value.to_owned()).map_err(HistoryReadError::Bound)?
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
    .map_err(HistoryReadError::Bound)
}

fn parse_gate_verdict_facts(value: &Value) -> Result<GateVerdictFacts, HistoryReadError> {
    let event =
        EventId::parse(parse_required_string(value, "event")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
                message: error.to_string(),
            }
        })?;
    let gate_ids = value
        .get("gate_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| HistoryReadError::Corrupt {
            message: "gate_verdict_facts.gate_ids missing".into(),
        })?
        .iter()
        .map(|item| {
            GateId::parse(
                item.as_str()
                    .ok_or_else(|| HistoryReadError::Corrupt {
                        message: "gate_ids entries must be strings".into(),
                    })?
                    .to_owned(),
            )
            .map_err(|error| HistoryReadError::Corrupt {
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
        return Err(HistoryReadError::Corrupt {
            message: "gate_verdict_facts requires exactly one result variant".into(),
        });
    }
    let result = if has_verdicts {
        let verdicts = value
            .get("verdicts")
            .and_then(Value::as_array)
            .ok_or_else(|| HistoryReadError::Corrupt {
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
            .map_err(HistoryReadError::Bound)?,
        )
    };
    GateVerdictFacts::new(event, gate_ids, result).map_err(map_attempt_error)
}

fn parse_gate_verdict(value: &Value) -> Result<GateVerdictFact, HistoryReadError> {
    let gate_id =
        GateId::parse(parse_required_string(value, "gate_id")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
                message: error.to_string(),
            }
        })?;
    let passed = match parse_required_string(value, "status")? {
        "pass" => true,
        "fail" => false,
        other => {
            return Err(HistoryReadError::Corrupt {
                message: format!("unsupported gate verdict status {other:?}"),
            });
        }
    };
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);
    GateVerdictFact::new(gate_id, passed, message).map_err(HistoryReadError::Bound)
}

fn parse_evidence_associations(
    value: &Value,
    observed_at: &ObservedAt,
) -> Result<EvidenceAssociations, HistoryReadError> {
    let inline = match value.get("inline") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| parse_inline_evidence(item, observed_at))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(HistoryReadError::Corrupt {
                message: "evidence_associations.inline must be an array".into(),
            });
        }
        None => Vec::new(),
    };
    let selected_ids = parse_string_id_list(value.get("selected_ids"), "selected_ids")?
        .into_iter()
        .map(EvidenceId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HistoryReadError::Corrupt {
            message: error.to_string(),
        })?;
    let provider_recorded_ids =
        parse_string_id_list(value.get("provider_recorded_ids"), "provider_recorded_ids")?
            .into_iter()
            .map(EvidenceId::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| HistoryReadError::Corrupt {
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
) -> Result<EvidenceRecord, HistoryReadError> {
    let id = EvidenceId::parse(parse_required_string(value, "evidence_id")?.to_owned()).map_err(
        |error| HistoryReadError::Corrupt {
            message: error.to_string(),
        },
    )?;
    let kind =
        EvidenceKind::parse(parse_required_string(value, "kind")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
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
            return Err(HistoryReadError::Corrupt {
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
    .map_err(HistoryReadError::Bound)
}

fn parse_string_id_list(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Vec<String>, HistoryReadError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| HistoryReadError::Corrupt {
                        message: format!("{field} entries must be strings"),
                    })
            })
            .collect(),
        Some(_) => Err(HistoryReadError::Corrupt {
            message: format!("{field} must be an array"),
        }),
    }
}

fn parse_evidence_recorded(value: &Value) -> Result<EvidenceRecordedStatus, HistoryReadError> {
    Ok(EvidenceRecordedStatus {
        inline: value
            .get("inline")
            .and_then(Value::as_bool)
            .ok_or_else(|| HistoryReadError::Corrupt {
                message: "evidence_recorded.inline missing".into(),
            })?,
        selected_associations: value
            .get("selected_associations")
            .and_then(Value::as_bool)
            .ok_or_else(|| HistoryReadError::Corrupt {
                message: "evidence_recorded.selected_associations missing".into(),
            })?,
        provider: value
            .get("provider")
            .and_then(Value::as_bool)
            .ok_or_else(|| HistoryReadError::Corrupt {
                message: "evidence_recorded.provider missing".into(),
            })?,
    })
}

fn parse_actor_metadata(value: &Value) -> Result<ActorMetadata, HistoryReadError> {
    let mapped = protocol_mapping::core_value(value.clone(), "actor").map_err(|error| {
        HistoryReadError::Corrupt {
            message: error.to_string(),
        }
    })?;
    ActorMetadata::new(mapped).map_err(HistoryReadError::Bound)
}

fn parse_diagnostics(value: &Value) -> Result<Vec<Diagnostic>, HistoryReadError> {
    let Value::Array(items) = value else {
        return Err(HistoryReadError::Corrupt {
            message: "diagnostics must be an array".into(),
        });
    };
    items
        .iter()
        .map(parse_diagnostic_object)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_diagnostic_object(value: &Value) -> Result<Diagnostic, HistoryReadError> {
    let code = parse_required_string(value, "code")?.to_owned();
    let message = parse_required_string(value, "message")?.to_owned();
    let path = value.get("path").and_then(Value::as_str).map(str::to_owned);
    Diagnostic::new(code, message, path).map_err(HistoryReadError::Bound)
}

fn parse_extension(
    root: &Value,
    entry_kind: &str,
    outcome: OutcomeClass,
) -> Result<JournalExtension, HistoryReadError> {
    match entry_kind {
        "run.created" => {
            let graph_revision =
                GraphRevision::parse(parse_required_string(root, "graph_revision")?.to_owned())
                    .map_err(|error| HistoryReadError::Corrupt {
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
                    return Err(HistoryReadError::Corrupt {
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
                    return Err(HistoryReadError::Corrupt {
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
                .map_err(HistoryReadError::Bound)?;
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
        other => Err(HistoryReadError::Corrupt {
            message: format!("unsupported entry_kind {other:?}"),
        }),
    }
}

fn parse_evidence_added_fact(root: &Value) -> Result<EvidenceAddedFact, HistoryReadError> {
    use loop_engine_core::model::bounded::BoundedText;
    let evidence_id = EvidenceId::parse(parse_required_string(root, "evidence_id")?.to_owned())
        .map_err(|error| HistoryReadError::Corrupt {
            message: error.to_string(),
        })?;
    let kind =
        EvidenceKind::parse(parse_required_string(root, "kind")?.to_owned()).map_err(|error| {
            HistoryReadError::Corrupt {
                message: error.to_string(),
            }
        })?;
    let locator =
        BoundedText::opaque_non_empty("evidence_locator", parse_required_string(root, "locator")?)
            .map_err(HistoryReadError::Bound)?;
    let digest = root
        .get("digest")
        .and_then(Value::as_str)
        .map(|value| BoundedText::opaque_non_empty("evidence_digest", value))
        .transpose()
        .map_err(HistoryReadError::Bound)?;
    Ok(EvidenceAddedFact {
        evidence_id,
        kind,
        locator,
        digest,
    })
}

fn parse_label_change(root: &Value) -> Result<LabelChangeFact, HistoryReadError> {
    let label_before = parse_optional_label(root.get("label_before"))?;
    let label_after = parse_optional_label(root.get("label_after"))?;
    Ok(LabelChangeFact {
        label_before,
        label_after,
    })
}

fn parse_optional_label(
    value: Option<&Value>,
) -> Result<Option<loop_engine_core::model::bounded::BoundedText<256>>, HistoryReadError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            loop_engine_core::model::bounded::BoundedText::non_empty("label", text.clone())
                .map(Some)
                .map_err(HistoryReadError::Bound)
        }
        Some(_) => Err(HistoryReadError::Corrupt {
            message: "label fields must be strings or null".into(),
        }),
    }
}

fn parse_compatibility_findings(value: &Value) -> Result<CompatibilityFindings, HistoryReadError> {
    let Value::Array(items) = value else {
        return Err(HistoryReadError::Corrupt {
            message: "findings must be an array".into(),
        });
    };
    let findings = items
        .iter()
        .map(parse_compatibility_finding)
        .collect::<Result<Vec<_>, _>>()?;
    CompatibilityFindings::new(findings).map_err(|error| HistoryReadError::Corrupt {
        message: error.to_string(),
    })
}

fn parse_compatibility_finding(value: &Value) -> Result<CompatibilityFinding, HistoryReadError> {
    use loop_engine_core::model::compatibility::CompatibilityStatus;
    let capability = parse_required_string(value, "capability")?.to_owned();
    let status = match parse_required_string(value, "status")? {
        "compatible" => CompatibilityStatus::Compatible,
        "incompatible" => CompatibilityStatus::Incompatible,
        "unknown" => CompatibilityStatus::Unknown,
        other => {
            return Err(HistoryReadError::Corrupt {
                message: format!("unsupported compatibility status {other:?}"),
            });
        }
    };
    let diagnostics = match value.get("message").and_then(Value::as_str) {
        None => Vec::new(),
        Some(message) => {
            vec![
                Diagnostic::new("compatibility.message", message, None)
                    .map_err(HistoryReadError::Bound)?,
            ]
        }
    };
    CompatibilityFinding::new(capability, status, diagnostics).map_err(HistoryReadError::Bound)
}

fn first_unreturned_cursor(
    candidates: &[(JournalEntry, usize, u64)],
    count_limit: usize,
    byte_limit: usize,
    run_id: &RunId,
    filter_fingerprint: &str,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
) -> Result<Option<PageCursor>, HistoryReadError> {
    let mut bytes = 0usize;
    for (selected, (index, (_, size, _))) in candidates.iter().enumerate().enumerate() {
        if *size > byte_limit && selected == 0 {
            return Err(PagingError::RowTooLarge.into());
        }
        if selected == count_limit || bytes.saturating_add(*size) > byte_limit {
            let sequence = candidates[index - 1].2;
            return mint_history_cursor(run_id, filter_fingerprint, integrity_key, sequence)
                .map(Some);
        }
        bytes = bytes.saturating_add(*size);
    }
    Ok(None)
}

fn decode_history_cursor(
    cursor: &PageCursor,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    run_id: &RunId,
    expected_fingerprint: &str,
) -> Result<u64, HistoryReadError> {
    let payload = decode_integrity_wire(integrity_key, cursor.as_str())?;
    if payload.get("cursor_version").and_then(Value::as_u64) != Some(1) {
        return Err(PagingError::CursorVersion.into());
    }
    if payload.get("collection").and_then(Value::as_str) != Some(COLLECTION_HISTORY) {
        return Err(PagingError::CursorBinding.into());
    }
    if payload.get("filter_fingerprint").and_then(Value::as_str) != Some(expected_fingerprint) {
        return Err(PagingError::CursorBinding.into());
    }
    paging::validate_binding(
        &paging::DecodedCursorBinding {
            schema_version: 1,
            operation: "run.history".into(),
            filter: run_id.as_str().into(),
        },
        "run.history",
        run_id.as_str(),
    )?;
    let last_key = payload.get("last_key").ok_or(PagingError::CursorBinding)?;
    let sequence = last_key
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or(PagingError::CursorBinding)?;
    if sequence == 0 {
        return Err(PagingError::CursorBinding.into());
    }
    Ok(sequence)
}

fn mint_history_cursor(
    _run_id: &RunId,
    filter_fingerprint: &str,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    sequence: u64,
) -> Result<PageCursor, HistoryReadError> {
    let payload = json!({
        "collection": COLLECTION_HISTORY,
        "cursor_version": 1,
        "filter_fingerprint": filter_fingerprint,
        "last_key": {
            "sequence": sequence,
        },
    });
    PageCursor::parse(mint_integrity_wire(integrity_key, payload)?)
        .map_err(PagingError::Bound)
        .map_err(Into::into)
}

fn history_filter_fingerprint(run_id: &RunId) -> String {
    digest_canonical_json(&json!({ "run_id": run_id.as_str() }))
}

fn load_run_journal_allocator(conn: &Connection, run_id: &RunId) -> Result<u64, HistoryReadError> {
    let next_sequence: Option<i64> = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_corrupt)?;
    let Some(next_sequence) = next_sequence else {
        return Err(HistoryReadError::Corrupt {
            message: format!(
                "run_journal_sequences row missing for run {}",
                run_id.as_str()
            ),
        });
    };
    if next_sequence <= 0 {
        return Err(HistoryReadError::Corrupt {
            message: format!(
                "run_journal_sequences.next_sequence must be positive, found {next_sequence}"
            ),
        });
    }
    u64::try_from(next_sequence).map_err(|_| HistoryReadError::Corrupt {
        message: "run_journal_sequences.next_sequence out of range".into(),
    })
}

fn require_first_page_journal_invariants(
    after_sequence: u64,
    candidates: &[(JournalEntry, usize, u64)],
) -> Result<(), HistoryReadError> {
    if after_sequence != 0 {
        return Ok(());
    }
    let Some((first_entry, _, sequence)) = candidates.first() else {
        return Err(HistoryReadError::Corrupt {
            message: "journal is empty for persisted run".into(),
        });
    };
    if *sequence != 1 {
        return Err(HistoryReadError::Corrupt {
            message: format!("journal must begin at sequence 1, found {sequence}"),
        });
    }
    if first_entry.kind() != JournalEntryKind::RunCreated {
        return Err(HistoryReadError::Corrupt {
            message: "journal sequence 1 must be run.created entry kind".into(),
        });
    }
    Ok(())
}

fn verify_journal_allocator_tail(
    allocator_next: u64,
    tail_sequence: u64,
) -> Result<(), HistoryReadError> {
    let expected = tail_sequence.saturating_add(1);
    if allocator_next != expected {
        return Err(HistoryReadError::Corrupt {
            message: format!(
                "run_journal_sequences.next_sequence {allocator_next} does not match journal tail {expected}"
            ),
        });
    }
    Ok(())
}

fn ensure_run_exists(conn: &Connection, run_id: &RunId) -> Result<(), HistoryReadError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM runs WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_corrupt)?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(HistoryReadError::NotFound {
            run_id: run_id.clone(),
        })
    }
}

fn load_integrity_key(
    conn: &Connection,
) -> Result<[u8; INTEGRITY_KEY_BYTE_LENGTH], HistoryReadError> {
    let bytes: Vec<u8> = conn
        .query_row(
            &format!("SELECT value FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
            [INTEGRITY_KEY_ROW_KEY],
            |row| row.get(0),
        )
        .map_err(|source| match source {
            SqliteError::QueryReturnedNoRows => {
                HistoryReadError::Persistence(PersistenceError::MetadataKeyMissing {
                    key: INTEGRITY_KEY_ROW_KEY,
                })
            }
            other => {
                HistoryReadError::Persistence(PersistenceError::MetadataRead { source: other })
            }
        })?;
    if bytes.len() != INTEGRITY_KEY_BYTE_LENGTH {
        return Err(HistoryReadError::Persistence(
            PersistenceError::MetadataKeyInvalidLength {
                key: INTEGRITY_KEY_ROW_KEY,
                expected: INTEGRITY_KEY_BYTE_LENGTH,
                actual: bytes.len(),
            },
        ));
    }
    let mut key = [0u8; INTEGRITY_KEY_BYTE_LENGTH];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn sqlite_corrupt(error: SqliteError) -> HistoryReadError {
    HistoryReadError::Corrupt {
        message: error.to_string(),
    }
}

fn map_mapping_error(error: MappingError) -> HistoryReadError {
    HistoryReadError::Corrupt {
        message: error.to_string(),
    }
}

fn map_journal_error(error: JournalError) -> HistoryReadError {
    HistoryReadError::Corrupt {
        message: error.to_string(),
    }
}

fn map_attempt_error(error: loop_engine_core::model::attempt::AttemptError) -> HistoryReadError {
    HistoryReadError::Corrupt {
        message: error.to_string(),
    }
}

fn digest_canonical_json(value: &Value) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&canonical_value(value)).expect("canonical json")
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .iter()
                .map(|(key, value)| (key.as_str(), canonical_value(value)))
                .collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

fn mac_input(domain: &[u8], payload: &Value) -> Vec<u8> {
    let mut data = domain.to_vec();
    data.push(0);
    data.extend_from_slice(canonical_json(payload).as_bytes());
    data
}

fn mint_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    payload: Value,
) -> Result<String, HistoryReadError> {
    let tag = hmac_sha256(integrity_key, &mac_input(CURSOR_DOMAIN, &payload));
    let wire = json!({
        "mac": base64url_no_pad(&tag),
        "payload": payload,
    });
    let encoded = base64url_no_pad(canonical_json(&wire).as_bytes());
    if encoded.len() > OPAQUE_INTEGRITY_WIRE_UTF8_BYTES {
        return Err(PagingError::Bound(BoundError::TooLong {
            field: "page_cursor",
            max: OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
            actual: encoded.len(),
        })
        .into());
    }
    Ok(encoded)
}

fn decode_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    wire: &str,
) -> Result<Value, HistoryReadError> {
    if wire.len() > OPAQUE_INTEGRITY_WIRE_UTF8_BYTES {
        return Err(PagingError::CursorBinding.into());
    }
    let decoded = base64url_decode(wire).map_err(|_| PagingError::CursorBinding)?;
    let parsed: Value = serde_json::from_slice(&decoded).map_err(|_| PagingError::CursorBinding)?;
    if canonical_json(&parsed).as_bytes() != decoded.as_slice() {
        return Err(PagingError::CursorBinding.into());
    }
    let Some(wrapper) = parsed.as_object() else {
        return Err(PagingError::CursorBinding.into());
    };
    if wrapper.len() != 2 || !wrapper.contains_key("mac") || !wrapper.contains_key("payload") {
        return Err(PagingError::CursorBinding.into());
    }
    let mac_b64 = wrapper
        .get("mac")
        .and_then(Value::as_str)
        .ok_or(PagingError::CursorBinding)?;
    let payload = wrapper
        .get("payload")
        .ok_or(PagingError::CursorBinding)?
        .clone();
    let tag = base64url_decode(mac_b64).map_err(|_| PagingError::CursorBinding)?;
    if tag.len() != 32 {
        return Err(PagingError::CursorBinding.into());
    }
    let expected = hmac_sha256(integrity_key, &mac_input(CURSOR_DOMAIN, &payload));
    if !constant_time_eq(&tag, &expected) {
        return Err(PagingError::CursorBinding.into());
    }
    Ok(payload)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for index in 0..BLOCK {
        ipad[index] ^= key_block[index];
        opad[index] ^= key_block[index];
    }
    let inner = Sha256::digest([ipad.as_ref(), data].concat());
    let outer = Sha256::digest([opad.as_ref(), inner.as_slice()].concat());
    let mut tag = [0u8; 32];
    tag.copy_from_slice(&outer);
    tag
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut index = 0usize;
    while index + 3 <= bytes.len() {
        let chunk = &bytes[index..index + 3];
        let block = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        out.push(TABLE[((block >> 18) & 63) as usize] as char);
        out.push(TABLE[((block >> 12) & 63) as usize] as char);
        out.push(TABLE[((block >> 6) & 63) as usize] as char);
        out.push(TABLE[(block & 63) as usize] as char);
        index += 3;
    }
    let remainder = bytes.len() - index;
    if remainder == 1 {
        let block = (bytes[index] as u32) << 16;
        out.push(TABLE[((block >> 18) & 63) as usize] as char);
        out.push(TABLE[((block >> 12) & 63) as usize] as char);
    } else if remainder == 2 {
        let block = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8);
        out.push(TABLE[((block >> 18) & 63) as usize] as char);
        out.push(TABLE[((block >> 12) & 63) as usize] as char);
        out.push(TABLE[((block >> 6) & 63) as usize] as char);
    }
    out
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, ()> {
    let mut map = [255u8; 256];
    for (index, byte) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
        .iter()
        .enumerate()
    {
        map[*byte as usize] = index as u8;
    }
    let mut bits = 0u32;
    let mut bit_count = 0u32;
    let mut out = Vec::new();
    for ch in input.bytes() {
        let value = map[ch as usize];
        if value == 255 {
            return Err(());
        }
        bits = (bits << 6) | u32::from(value);
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    if base64url_no_pad(&out) != input {
        return Err(());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::PageRequest;
    use loop_engine_core::model::bounded::{
        COLLECTION_PAGE_DATA_BUDGET_BYTES, JOURNAL_ENTRY_ENCODED_BYTES, NOTE_TEXT_UTF8_BYTES,
    };
    use loop_engine_core::model::version::JournalSequence;
    use loop_engine_core::operations::paging::PagingError;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::error::PersistenceError;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
    use crate::persistence::records::GV01_GRAPH_REVISION;
    use crate::persistence::sqlite::open_at;

    fn assert_no_store_files(db_path: &std::path::Path) {
        assert!(!db_path.exists(), "database file must not be created");
        let wal = format!("{}-wal", db_path.display());
        let shm = format!("{}-shm", db_path.display());
        assert!(
            !std::path::Path::new(&wal).exists(),
            "WAL sidecar must not be created"
        );
        assert!(
            !std::path::Path::new(&shm).exists(),
            "SHM sidecar must not be created"
        );
    }

    fn test_reads() -> (TempDir, SqliteHistoryReads) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        (dir, SqliteHistoryReads::new(path))
    }

    fn insert_registration(conn: &Connection, registration_id: &str) {
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable, argv_json,
                working_directory, timeout_seconds, created_at, updated_at
            ) VALUES (?1, 'provider-a', 1, 1, '/bin/provider', '[]', '/work', 60,
                      '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            params![registration_id],
        )
        .unwrap();
    }

    fn insert_run(conn: &Connection, run_id: &str, registration_id: &str) {
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES (?1, ?2, 1, 'draft', 'active', 1, 1, 1,
                      'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                      1, '{}', '{}', '2026-07-17T12:00:00.000Z')",
            params![run_id, registration_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 1)",
            params![run_id],
        )
        .unwrap();
    }

    fn state_json(state: &str) -> Value {
        json!({
            "state": state,
            "lifecycle": "active",
            "workflow_state_version": 1,
            "lifecycle_version": 1
        })
    }

    fn creation_payload(run_id: &str, sequence: u64) -> String {
        json!({
            "journal_schema_version": 1,
            "sequence": sequence,
            "run_id": run_id,
            "ts": "2026-07-17T14:00:00.123Z",
            "operation": "run.create",
            "request_id": "req-create-001",
            "entry_kind": "run.created",
            "outcome": "completed",
            "reason": null,
            "state_before": state_json("draft"),
            "state_after": state_json("draft"),
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

    fn annotation_payload(
        run_id: &str,
        sequence: u64,
        note: &str,
        corrects: Option<u64>,
    ) -> String {
        let mut value = json!({
            "journal_schema_version": 1,
            "sequence": sequence,
            "run_id": run_id,
            "ts": "2026-07-17T15:00:00.123Z",
            "operation": "run.annotate",
            "request_id": format!("req-annotate-{sequence:03}"),
            "entry_kind": "annotation",
            "outcome": "completed",
            "reason": null,
            "state_before": state_json("draft"),
            "state_after": state_json("draft"),
            "note": note
        });
        if let Some(sequence) = corrects {
            value
                .as_object_mut()
                .expect("object")
                .insert("corrects_sequence".into(), json!(sequence));
        }
        value.to_string()
    }

    fn insert_journal(
        conn: &Connection,
        run_id: &str,
        sequence: u64,
        outcome: &str,
        payload: &str,
    ) {
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, i64::try_from(sequence).unwrap(), outcome, payload],
        )
        .unwrap();
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = ?1 WHERE run_id = ?2",
            params![i64::try_from(sequence.saturating_add(1)).unwrap(), run_id],
        )
        .unwrap();
    }

    fn seed_history(conn: &Connection, run_id: &str) {
        insert_journal(conn, run_id, 1, "completed", &creation_payload(run_id, 1));
        insert_journal(
            conn,
            run_id,
            2,
            "completed",
            &annotation_payload(run_id, 2, "first note", None),
        );
        insert_journal(
            conn,
            run_id,
            3,
            "completed",
            &annotation_payload(run_id, 3, "clarifies prior rejection", Some(2)),
        );
    }

    fn build_max_annotation_payload(run_id: &str, sequence: u64) -> String {
        let mut low = 1usize;
        let mut high = NOTE_TEXT_UTF8_BYTES;
        let mut best = annotation_payload(run_id, sequence, "x", None);
        while low <= high {
            let mid = (low + high) / 2;
            let candidate = annotation_payload(run_id, sequence, &"n".repeat(mid), None);
            let len = candidate.len();
            if len <= JOURNAL_ENTRY_ENCODED_BYTES {
                best = candidate;
                low = mid + 1;
            } else {
                high = mid.saturating_sub(1);
            }
        }
        assert!(best.len() <= JOURNAL_ENTRY_ENCODED_BYTES);
        best
    }

    #[test]
    fn absent_store_path_read_fails_without_creating_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        assert_no_store_files(&path);

        let reads = SqliteHistoryReads::new(path.clone());
        let run_id = RunId::parse("run-1").unwrap();
        let error = reads
            .history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            HistoryReadError::Persistence(PersistenceError::Open { .. })
        ));
        assert_no_store_files(&path);
    }

    #[test]
    fn migrated_store_reads_unchanged() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        seed_history(&conn, "run-1");
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let page = reads.history(&run_id, &request).unwrap();
        assert_eq!(page.rows.len(), 3);
        assert_eq!(
            page.rows
                .iter()
                .map(|entry| entry.sequence().value())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn history_orders_across_fresh_connections() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        seed_history(&conn, "run-1");
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let first = reads.history(&run_id, &request).unwrap();
        assert_eq!(
            first
                .rows
                .iter()
                .map(|entry| entry.sequence().value())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let reopened = SqliteHistoryReads::new(reads.path().clone());
        let second = reopened.history(&run_id, &request).unwrap();
        assert_eq!(second.rows.len(), 3);
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn rollback_leaves_no_sequence_gap() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &creation_payload("run-1", 1),
        );
        conn.execute("BEGIN IMMEDIATE", []).unwrap();
        insert_journal(
            &conn,
            "run-1",
            2,
            "completed",
            &annotation_payload("run-1", 2, "rolled back", None),
        );
        conn.execute("ROLLBACK", []).unwrap();
        insert_journal(
            &conn,
            "run-1",
            2,
            "completed",
            &annotation_payload("run-1", 2, "committed", None),
        );

        let run_id = RunId::parse("run-1").unwrap();
        let page = reads
            .history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap();
        assert_eq!(
            page.rows
                .iter()
                .map(|entry| entry.sequence().value())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn detects_sequence_gap_and_payload_corruption() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &creation_payload("run-1", 1),
        );
        insert_journal(
            &conn,
            "run-1",
            3,
            "completed",
            &annotation_payload("run-1", 3, "gap", None),
        );
        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));

        conn.execute("DELETE FROM journal_entries WHERE sequence = 3", [])
            .unwrap();
        insert_journal(
            &conn,
            "run-1",
            2,
            "completed",
            &json!({"journal_schema_version":1,"sequence":99,"run_id":"run-1","outcome":"completed"}).to_string(),
        );
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn base64url_decoder_rejects_noncanonical_residual_bits_and_lengths() {
        assert_eq!(super::base64url_decode("AA").unwrap(), vec![0]);
        assert!(super::base64url_decode("AB").is_err());
        assert!(super::base64url_decode("A").is_err());
    }

    #[test]
    fn integrity_wire_rejects_noncanonical_wrapper() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        let integrity_key = load_integrity_key(&conn).unwrap();
        let run_id = RunId::parse("run-1").unwrap();
        let filter_fingerprint = history_filter_fingerprint(&run_id);
        let payload = json!({
            "collection": super::COLLECTION_HISTORY,
            "cursor_version": 1,
            "filter_fingerprint": filter_fingerprint,
            "last_key": {
                "sequence": 1,
            },
        });
        let wire = mint_integrity_wire(&integrity_key, payload).unwrap();
        let canonical = String::from_utf8(base64url_decode(&wire).unwrap()).unwrap();
        let parsed: Value = serde_json::from_str(&canonical).unwrap();
        let mac = parsed["mac"].as_str().unwrap();
        let payload_json = serde_json::to_string(&parsed["payload"]).unwrap();

        let reordered = format!(r#"{{"payload":{payload_json},"mac":"{mac}"}}"#);
        assert!(
            decode_integrity_wire(&integrity_key, &base64url_no_pad(reordered.as_bytes())).is_err()
        );

        let whitespace = canonical.replace(',', ", ");
        assert!(
            decode_integrity_wire(&integrity_key, &base64url_no_pad(whitespace.as_bytes()))
                .is_err()
        );

        let duplicate = format!(r#"{{"mac":"{mac}","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(
            decode_integrity_wire(&integrity_key, &base64url_no_pad(duplicate.as_bytes())).is_err()
        );

        let extra = format!(r#"{{"extra":"x","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(
            decode_integrity_wire(&integrity_key, &base64url_no_pad(extra.as_bytes())).is_err()
        );
    }

    #[test]
    fn cursor_tamper_and_filter_binding_reject_before_progress() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_run(&conn, "run-2", "reg-1");
        seed_history(&conn, "run-1");
        insert_journal(
            &conn,
            "run-2",
            1,
            "completed",
            &creation_payload("run-2", 1),
        );

        let run_id = RunId::parse("run-1").unwrap();
        let other = RunId::parse("run-2").unwrap();
        let first = reads
            .history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap();
        let cursor = first.next_cursor.clone().expect("cursor");

        let key = load_integrity_key(&conn).unwrap();
        let mut tampered = cursor.as_str().to_owned();
        tampered.replace_range(tampered.len().saturating_sub(1).., "A");
        let tampered_cursor = PageCursor::parse(tampered).unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(
                    1,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    Some(tampered_cursor),
                    ()
                )
                .unwrap(),
            ),
            Err(HistoryReadError::Page(PagingError::CursorBinding))
        ));

        assert!(matches!(
            reads.history(
                &other,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, Some(cursor), ()).unwrap(),
            ),
            Err(HistoryReadError::Page(PagingError::CursorBinding))
        ));

        let _fingerprint = history_filter_fingerprint(&run_id);
        let foreign =
            mint_history_cursor(&run_id, &history_filter_fingerprint(&other), &key, 1).unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, Some(foreign), ()).unwrap(),
            ),
            Err(HistoryReadError::Page(PagingError::CursorBinding))
        ));
    }

    #[test]
    fn max_entry_golden_fits_page_and_cli_envelope_without_truncation() {
        const {
            assert!(
                JOURNAL_ENTRY_ENCODED_BYTES + ENVELOPE_FRAMING_HEADROOM_BYTES
                    <= STRUCTURED_CLI_ENVELOPE_BYTES
            );
        }
        const {
            assert!(JOURNAL_ENTRY_ENCODED_BYTES <= COLLECTION_PAGE_DATA_BUDGET_BYTES);
        }

        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &creation_payload("run-1", 1),
        );
        let max_payload = build_max_annotation_payload("run-1", 2);
        let encoded_len = max_payload.len();
        assert!(encoded_len <= JOURNAL_ENTRY_ENCODED_BYTES);
        insert_journal(&conn, "run-1", 2, "completed", &max_payload);
        insert_journal(
            &conn,
            "run-1",
            3,
            "completed",
            &annotation_payload("run-1", 3, "tail", None),
        );

        let run_id = RunId::parse("run-1").unwrap();
        let first = reads
            .history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].sequence(), JournalSequence::first());

        let page = reads
            .history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, first.next_cursor, ())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].sequence().value(), 2);
        assert!(page.next_cursor.is_some());
        assert!(encoded_len <= COLLECTION_PAGE_DATA_BUDGET_BYTES);
        assert!(encoded_len + ENVELOPE_FRAMING_HEADROOM_BYTES <= STRUCTURED_CLI_ENVELOPE_BYTES);

        let tail = reads
            .history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, page.next_cursor, ())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(tail.rows.len(), 1);
        assert_eq!(tail.rows[0].sequence().value(), 3);
    }

    #[test]
    fn history_does_not_eagerly_load_tail_beyond_page_window() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &creation_payload("run-1", 1),
        );
        for sequence in 2..50 {
            insert_journal(
                &conn,
                "run-1",
                sequence,
                "completed",
                &annotation_payload("run-1", sequence, "ok", None),
            );
        }
        insert_journal(
            &conn,
            "run-1",
            50,
            "completed",
            &json!({
                "journal_schema_version": 1,
                "sequence": 999,
                "run_id": "run-1",
                "ts": "2026-07-17T15:00:00.123Z",
                "operation": "run.annotate",
                "request_id": "req-annotate-050",
                "entry_kind": "annotation",
                "outcome": "completed",
                "reason": null,
                "state_before": state_json("draft"),
                "state_after": state_json("draft"),
                "note": "tail corruption"
            })
            .to_string(),
        );

        let run_id = RunId::parse("run-1").unwrap();
        let first = reads
            .history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].sequence().value(), 1);
        assert!(first.next_cursor.is_some());

        let second = reads
            .history(
                &run_id,
                &PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, first.next_cursor, ())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].sequence().value(), 2);

        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(100, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn oversized_first_row_errors_without_truncation() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let payload = creation_payload("run-1", 1);
        assert!(payload.len() > 256);
        insert_journal(&conn, "run-1", 1, "completed", &payload);
        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(&run_id, &PageRequest::new(10, 256, None, ()).unwrap(),),
            Err(HistoryReadError::Page(PagingError::RowTooLarge))
        ));
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
        assert_eq!(verdicts.event.as_str(), "go");
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
            Err(HistoryReadError::Corrupt { .. })
        ));
        assert!(matches!(
            parse_gate_verdict_facts(&json!({
                "event": "go",
                "gate_ids": [gate_ids.clone()],
                "verdicts": [{"gate_id": "gate-1", "status": "pass"}],
                "incompatibility": {"code": "x", "message": "y"},
            })),
            Err(HistoryReadError::Corrupt { .. })
        ));
        assert!(matches!(
            parse_gate_verdict_facts(&json!({
                "event": "go",
                "gate_ids": [gate_ids],
                "verdicts": [{"gate_id": "gate-1", "status": "pass"}],
                "evaluation_error": [{"code": "x", "message": "y"}],
            })),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn history_rejects_run_without_journal() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn history_rejects_missing_journal_allocator() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &creation_payload("run-1", 1),
        );
        conn.execute(
            "DELETE FROM run_journal_sequences WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn history_rejects_wrong_journal_allocator_on_final_page() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        seed_history(&conn, "run-1");
        conn.execute(
            "UPDATE run_journal_sequences SET next_sequence = 99 WHERE run_id = 'run-1'",
            [],
        )
        .unwrap();
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn history_rejects_sequence_one_non_creation() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_journal(
            &conn,
            "run-1",
            1,
            "completed",
            &annotation_payload("run-1", 1, "not creation", None),
        );
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.history(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            ),
            Err(HistoryReadError::Corrupt { .. })
        ));
    }
}
