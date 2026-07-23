//! Provider-free SQLite evidence inventory and selected-context reads (T110).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use loop_engine_core::capabilities::provider_invoker::EvidenceContext;
use loop_engine_core::capabilities::run_reader::{EvidenceInventoryRow, SelectedEvidenceReadError};
use loop_engine_core::capabilities::{Page, PageCursor, PageRequest};
use loop_engine_core::model::bounded::{
    BoundError, EVIDENCE_RECORD_ENCODED_BYTES, Metadata, OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
    Value as CoreValue,
};
use loop_engine_core::model::evidence::{EvidenceAssociation, EvidenceRecord, EvidenceSource};
use loop_engine_core::model::ids::{EventId, EvidenceId, EvidenceKind, GateId, RunId};
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::operations::paging::bounded_page;
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::error::PersistenceError;
use super::mapping::format_observed_at;
use super::sqlite::{
    INTEGRATION_METADATA_TABLE, INTEGRITY_KEY_BYTE_LENGTH, INTEGRITY_KEY_ROW_KEY,
    connect_read_only_with_pragmas,
};
use super::traced::{
    MutationClass, OptionalTraceSink, ReadCompleteExtras, close_read, evidence_read_failure,
    evidence_read_rejected,
};

const COLLECTION_EVIDENCE: &str = "run.evidence";
const CURSOR_DOMAIN: &[u8] = b"loop-engine.integrations.cursor-v1";

/// SQLite-backed evidence inventory and selected-context reader.
#[derive(Debug, Clone)]
pub struct SqliteEvidenceReads {
    pub path: PathBuf,
    trace: OptionalTraceSink,
}

#[derive(Debug, Error)]
pub enum EvidenceReadError {
    #[error("run not found")]
    NotFound,
    #[error("evidence unavailable")]
    Unavailable,
    #[error("corrupt persistence row: {detail}")]
    Corrupt { detail: String },
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Page(#[from] loop_engine_core::operations::paging::PagingError),
    #[error(transparent)]
    Bound(#[from] BoundError),
}

impl SqliteEvidenceReads {
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

    pub fn inventory(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<EvidenceInventoryRow>, EvidenceReadError> {
        close_read(
            &self.trace,
            "run.evidence.list",
            MutationClass::ReadOnly,
            || self.inventory_impl(run_id, request),
            |page| {
                let page_data_bytes = page
                    .rows
                    .iter()
                    .map(|row| inventory_row_encoded_bytes(row).unwrap_or(0))
                    .sum::<usize>() as u64;
                ReadCompleteExtras::for_page(page, page_data_bytes)
            },
            evidence_read_rejected,
            evidence_read_failure,
        )
    }

    pub fn selected_evidence(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<EvidenceReadError>> {
        self.selected_evidence_for_operation("run.evidence.list", run_id, evidence_ids)
    }

    pub fn selected_evidence_for_operation(
        &self,
        operation_id: &'static str,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<EvidenceReadError>> {
        close_read(
            &self.trace,
            operation_id,
            MutationClass::ReadOnly,
            || self.selected_evidence_impl(run_id, evidence_ids),
            |records| ReadCompleteExtras {
                item_count: Some(records.len() as u64),
                ..ReadCompleteExtras::default()
            },
            |error| matches!(error, SelectedEvidenceReadError::Unavailable),
            |error| match error {
                SelectedEvidenceReadError::Read(inner) => evidence_read_failure(inner),
                SelectedEvidenceReadError::Unavailable => {
                    ("persistence.failed", Some("evidence unavailable".into()))
                }
            },
        )
    }

    pub fn selected_context(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<EvidenceContext, SelectedEvidenceReadError<EvidenceReadError>> {
        close_read(
            &self.trace,
            "run.evidence.list",
            MutationClass::ReadOnly,
            || self.selected_context_impl(run_id, evidence_ids),
            |context| ReadCompleteExtras {
                item_count: Some(context.records().len() as u64),
                ..ReadCompleteExtras::default()
            },
            |error| matches!(error, SelectedEvidenceReadError::Unavailable),
            |error| match error {
                SelectedEvidenceReadError::Read(inner) => evidence_read_failure(inner),
                SelectedEvidenceReadError::Unavailable => {
                    ("persistence.failed", Some("evidence unavailable".into()))
                }
            },
        )
    }

    fn inventory_impl(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<EvidenceInventoryRow>, EvidenceReadError> {
        let conn = connect_read_only_with_pragmas(&self.path)?;
        let snapshot_transaction = EvidenceReadTransaction::begin(&conn)?;
        ensure_run_exists(&conn, run_id)?;
        let integrity_key = read_integrity_key(&conn)?;
        let filter_fingerprint = evidence_filter_fingerprint(run_id);
        let after = match request.cursor() {
            Some(cursor) => Some(decode_evidence_cursor(
                &integrity_key,
                cursor.as_str(),
                &filter_fingerprint,
            )?),
            None => None,
        };
        let mut sql = String::from(
            "SELECT evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
             FROM evidence
             WHERE run_id = ?1",
        );
        if after.is_some() {
            sql.push_str(" AND (created_at > ?2 OR (created_at = ?2 AND evidence_id > ?3))");
        }
        sql.push_str(" ORDER BY created_at ASC, evidence_id ASC");
        let fetch = usize::from(request.limit()).saturating_add(1);
        sql.push_str(&format!(" LIMIT {fetch}"));
        let raw_rows = match after {
            Some(key) => {
                let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
                stmt.query_map(
                    params![run_id.as_str(), key.created_at, key.evidence_id],
                    map_evidence_row,
                )
                .map_err(sqlite_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sqlite_err)?
            }
            None => {
                let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
                stmt.query_map(params![run_id.as_str()], map_evidence_row)
                    .map_err(sqlite_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sqlite_err)?
            }
        };
        let associations = load_associations_for_evidence(
            &conn,
            run_id,
            raw_rows.iter().map(|row| row.evidence_id.as_str()),
        )?;
        let mut candidates = Vec::with_capacity(raw_rows.len());
        for row in &raw_rows {
            let record = row.to_record()?;
            let row_associations = associations
                .get(record.id().as_str())
                .cloned()
                .unwrap_or_default();
            let inventory_row = EvidenceInventoryRow {
                record,
                associations: row_associations,
            };
            let encoded_bytes = inventory_row_encoded_bytes(&inventory_row)?;
            candidates.push((
                inventory_row,
                encoded_bytes,
                EvidenceCursorKey {
                    created_at: row.created_at.clone(),
                    evidence_id: row.evidence_id.clone(),
                },
            ));
        }
        let count_limit = usize::from(request.limit());
        let byte_limit = request.byte_limit();
        let next_cursor = first_unreturned_evidence_cursor(
            &candidates,
            count_limit,
            byte_limit,
            &integrity_key,
            &filter_fingerprint,
        )?;
        let page = bounded_page(
            candidates.into_iter().map(|(row, size, _)| (row, size)),
            count_limit,
            byte_limit,
            next_cursor,
        )
        .map_err(EvidenceReadError::from)?;
        snapshot_transaction.commit()?;
        Ok(page)
    }

    fn selected_evidence_impl(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<EvidenceReadError>> {
        if evidence_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = connect_read_only_with_pragmas(&self.path)
            .map_err(|error| SelectedEvidenceReadError::Read(EvidenceReadError::from(error)))?;
        ensure_run_exists(&conn, run_id).map_err(SelectedEvidenceReadError::Read)?;
        let loaded = load_selected_records(&conn, run_id, evidence_ids)
            .map_err(SelectedEvidenceReadError::Read)?;
        for id in evidence_ids {
            if !loaded.contains_key(id.as_str()) {
                return Err(SelectedEvidenceReadError::Unavailable);
            }
        }
        Ok(evidence_ids
            .iter()
            .map(|id| {
                loaded
                    .get(id.as_str())
                    .expect("availability checked")
                    .clone()
            })
            .collect())
    }

    fn selected_context_impl(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<EvidenceContext, SelectedEvidenceReadError<EvidenceReadError>> {
        let records = self.selected_evidence_impl(run_id, evidence_ids)?;
        let encoded_bytes =
            selected_context_encoded_bytes(&records).map_err(SelectedEvidenceReadError::Read)?;
        EvidenceContext::new("selected_evidence", records, encoded_bytes)
            .map_err(|error| SelectedEvidenceReadError::Read(EvidenceReadError::Bound(error)))
    }
}

// Local row mapping until T106 `records`/`mapping` lands; keep isolated for concurrent merge.
#[derive(Debug, Clone)]
struct EvidenceRecordRow {
    evidence_id: String,
    kind: String,
    locator: String,
    digest: Option<String>,
    media_type: Option<String>,
    metadata_json: Option<String>,
    source: String,
    created_at: String,
}

impl EvidenceRecordRow {
    fn to_record(&self) -> Result<EvidenceRecord, EvidenceReadError> {
        let observed_at =
            ObservedAt::parse(&self.created_at).map_err(|error| EvidenceReadError::Corrupt {
                detail: format!("evidence.created_at: {error}"),
            })?;
        let source = match self.source.as_str() {
            "caller" => EvidenceSource::Caller,
            "provider" => EvidenceSource::Provider,
            other => {
                return Err(EvidenceReadError::Corrupt {
                    detail: format!("evidence.source: unexpected value {other:?}"),
                });
            }
        };
        let metadata = parse_metadata_json(self.metadata_json.as_deref())?;
        EvidenceRecord::new(
            EvidenceId::parse(self.evidence_id.clone()).map_err(corrupt_id)?,
            EvidenceKind::parse(self.kind.clone()).map_err(corrupt_kind)?,
            self.locator.clone(),
            self.digest.clone(),
            self.media_type.clone(),
            metadata,
            source,
            observed_at,
        )
        .map_err(|error| EvidenceReadError::Corrupt {
            detail: format!("evidence record: {error}"),
        })
    }
}

fn map_evidence_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceRecordRow> {
    Ok(EvidenceRecordRow {
        evidence_id: row.get(0)?,
        kind: row.get(1)?,
        locator: row.get(2)?,
        digest: row.get(3)?,
        media_type: row.get(4)?,
        metadata_json: row.get(5)?,
        source: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn corrupt_id(error: loop_engine_core::model::ids::IdentifierError) -> EvidenceReadError {
    EvidenceReadError::Corrupt {
        detail: format!("evidence_id: {error}"),
    }
}

fn corrupt_kind(error: loop_engine_core::model::ids::IdentifierError) -> EvidenceReadError {
    EvidenceReadError::Corrupt {
        detail: format!("evidence.kind: {error}"),
    }
}

fn parse_metadata_json(raw: Option<&str>) -> Result<Option<Metadata>, EvidenceReadError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let parsed: Value = serde_json::from_str(raw).map_err(|error| EvidenceReadError::Corrupt {
        detail: format!("evidence.metadata_json: {error}"),
    })?;
    let Value::Object(values) = parsed else {
        return Err(EvidenceReadError::Corrupt {
            detail: "evidence.metadata_json: expected object".into(),
        });
    };
    let mapped = values
        .into_iter()
        .map(|(key, value)| core_value(value, "evidence.metadata").map(|value| (key, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Metadata::new("evidence.metadata", mapped, EVIDENCE_RECORD_ENCODED_BYTES).map_err(|error| {
        EvidenceReadError::Corrupt {
            detail: format!("evidence.metadata: {error}"),
        }
    })
}

fn core_value(value: Value, path: &str) -> Result<CoreValue, EvidenceReadError> {
    use loop_engine_core::model::bounded::FiniteNumber;
    match value {
        Value::Null => Ok(CoreValue::Null),
        Value::Bool(value) => Ok(CoreValue::Bool(value)),
        Value::Number(value) => {
            let number = value.as_f64().ok_or_else(|| EvidenceReadError::Corrupt {
                detail: format!("{path}: number is outside binary64 domain"),
            })?;
            Ok(CoreValue::Number(
                FiniteNumber::new("evidence.metadata", number).map_err(|error| {
                    EvidenceReadError::Corrupt {
                        detail: format!("{path}: {error}"),
                    }
                })?,
            ))
        }
        Value::String(value) => Ok(CoreValue::String(value)),
        Value::Array(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| core_value(value, &format!("{path}/{index}")))
            .collect::<Result<Vec<_>, _>>()
            .map(CoreValue::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| {
                core_value(value, &format!("{path}/{key}")).map(|value| (key, value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(CoreValue::Object),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvidenceCursorKey {
    created_at: String,
    evidence_id: String,
}

fn evidence_filter_fingerprint(run_id: &RunId) -> String {
    digest_canonical_json(&json!({"run_id": run_id.as_str()}))
}

fn ensure_run_exists(conn: &Connection, run_id: &RunId) -> Result<(), EvidenceReadError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM runs WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_err)?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(EvidenceReadError::NotFound)
    }
}

fn load_selected_records(
    conn: &Connection,
    run_id: &RunId,
    evidence_ids: &[EvidenceId],
) -> Result<HashMap<String, EvidenceRecord>, EvidenceReadError> {
    let placeholders = evidence_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
         FROM evidence
         WHERE run_id = ?1 AND evidence_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
    let run_id_str = run_id.as_str().to_owned();
    let evidence_id_strs: Vec<String> = evidence_ids
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    let mut query_params: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(1 + evidence_id_strs.len());
    query_params.push(&run_id_str);
    for id in &evidence_id_strs {
        query_params.push(id);
    }
    let rows = stmt
        .query_map(query_params.as_slice(), map_evidence_row)
        .map_err(sqlite_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_err)?;
    rows.into_iter()
        .map(|row| {
            row.to_record()
                .map(|record| (record.id().as_str().to_owned(), record))
        })
        .collect()
}

fn load_associations_for_evidence<'a>(
    conn: &Connection,
    run_id: &RunId,
    evidence_ids: impl Iterator<Item = &'a str>,
) -> Result<HashMap<String, Vec<EvidenceAssociation>>, EvidenceReadError> {
    let ids = evidence_ids.collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT evidence_id, event_id, gate_id
         FROM evidence_associations
         WHERE run_id = ?1 AND evidence_id IN ({placeholders})
         ORDER BY journal_sequence ASC, evidence_id ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
    let run_id_str = run_id.as_str().to_owned();
    let mut query_params: Vec<&dyn rusqlite::ToSql> = vec![&run_id_str];
    for id in &ids {
        query_params.push(id);
    }
    let mut grouped: HashMap<String, Vec<EvidenceAssociation>> = HashMap::new();
    let mut rows = stmt.query(query_params.as_slice()).map_err(sqlite_err)?;
    while let Some(row) = rows.next().map_err(sqlite_err)? {
        let evidence_id: String = row.get(0).map_err(sqlite_err)?;
        let event_id: Option<String> = row.get(1).map_err(sqlite_err)?;
        let gate_id: Option<String> = row.get(2).map_err(sqlite_err)?;
        let parsed_event = match event_id {
            None => None,
            Some(value) => {
                Some(
                    EventId::parse(value).map_err(|error| EvidenceReadError::Corrupt {
                        detail: format!("evidence_associations.event_id: {error}"),
                    })?,
                )
            }
        };
        let parsed_gate = match gate_id {
            None => None,
            Some(value) => {
                Some(
                    GateId::parse(value).map_err(|error| EvidenceReadError::Corrupt {
                        detail: format!("evidence_associations.gate_id: {error}"),
                    })?,
                )
            }
        };
        grouped
            .entry(evidence_id.clone())
            .or_default()
            .push(EvidenceAssociation::new(
                EvidenceId::parse(evidence_id).map_err(corrupt_id)?,
                parsed_event,
                parsed_gate,
            ));
    }
    Ok(grouped)
}

#[derive(Serialize)]
struct InventoryAssociationWire<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gate_id: Option<&'a str>,
}

#[derive(Serialize)]
struct InventoryItemWire<'a> {
    evidence_id: &'a str,
    kind: &'a str,
    locator: &'a str,
    digest: Option<&'a str>,
    media_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BTreeMap<String, Value>>,
    source: &'a str,
    created_at: String,
    associations: Vec<InventoryAssociationWire<'a>>,
}

#[derive(Serialize)]
struct SelectedEvidenceDto {
    id: String,
    kind: String,
    locator: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
}

fn metadata_wire_map(metadata: &Metadata) -> BTreeMap<String, Value> {
    metadata
        .values()
        .iter()
        .map(|(key, value)| (key.clone(), core_value_to_json(value)))
        .collect()
}

fn core_value_to_json(value: &CoreValue) -> Value {
    match value {
        CoreValue::Null => Value::Null,
        CoreValue::Bool(value) => Value::Bool(*value),
        CoreValue::Number(value) => serde_json::Number::from_f64(value.value())
            .map(Value::Number)
            .expect("core finite number must be JSON representable"),
        CoreValue::String(value) => Value::String(value.clone()),
        CoreValue::Array(values) => Value::Array(values.iter().map(core_value_to_json).collect()),
        CoreValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), core_value_to_json(value)))
                .collect(),
        ),
    }
}

fn selected_evidence_dto(record: &EvidenceRecord) -> SelectedEvidenceDto {
    SelectedEvidenceDto {
        id: record.id().as_str().to_owned(),
        kind: record.kind().as_str().to_owned(),
        locator: record.locator().to_owned(),
        digest: record.digest().map(str::to_owned),
        media_type: record.media_type().map(str::to_owned),
        metadata: record.metadata().map(metadata_wire_map),
        observed_at: Some(format_observed_at(&record.observed_at())),
    }
}

fn inventory_row_encoded_bytes(row: &EvidenceInventoryRow) -> Result<usize, EvidenceReadError> {
    let record = &row.record;
    let created_at = format_observed_at(&record.observed_at());
    let source = match record.source() {
        EvidenceSource::Caller => "caller",
        EvidenceSource::Provider => "provider",
    };
    let associations = row
        .associations
        .iter()
        .map(|association| InventoryAssociationWire {
            event_id: association.event_id().map(|id| id.as_str()),
            gate_id: association.gate_id().map(|id| id.as_str()),
        })
        .collect();
    let wire = InventoryItemWire {
        evidence_id: record.id().as_str(),
        kind: record.kind().as_str(),
        locator: record.locator(),
        digest: record.digest(),
        media_type: record.media_type(),
        metadata: record.metadata().map(metadata_wire_map),
        source,
        created_at,
        associations,
    };
    serde_json::to_vec(&wire)
        .map(|bytes| bytes.len())
        .map_err(|error| EvidenceReadError::Corrupt {
            detail: format!("inventory row encoding: {error}"),
        })
}

fn selected_context_encoded_bytes(records: &[EvidenceRecord]) -> Result<usize, EvidenceReadError> {
    let dtos = records
        .iter()
        .map(selected_evidence_dto)
        .collect::<Vec<_>>();
    serde_json::to_vec(&dtos)
        .map(|bytes| bytes.len())
        .map_err(|error| EvidenceReadError::Corrupt {
            detail: format!("selected evidence encoding: {error}"),
        })
}

fn decode_evidence_cursor(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    wire: &str,
    filter_fingerprint: &str,
) -> Result<EvidenceCursorKey, loop_engine_core::operations::paging::PagingError> {
    if wire.len() > OPAQUE_INTEGRITY_WIRE_UTF8_BYTES {
        return Err(loop_engine_core::operations::paging::PagingError::CursorBinding);
    }
    let payload = decode_integrity_wire(integrity_key, wire)
        .map_err(|_| loop_engine_core::operations::paging::PagingError::CursorBinding)?;
    if payload.get("cursor_version").and_then(Value::as_u64) != Some(1) {
        return Err(loop_engine_core::operations::paging::PagingError::CursorVersion);
    }
    if payload.get("collection").and_then(Value::as_str) != Some(COLLECTION_EVIDENCE) {
        return Err(loop_engine_core::operations::paging::PagingError::CursorBinding);
    }
    if payload.get("filter_fingerprint").and_then(Value::as_str) != Some(filter_fingerprint) {
        return Err(loop_engine_core::operations::paging::PagingError::CursorBinding);
    }
    let last_key = payload
        .get("last_key")
        .ok_or(loop_engine_core::operations::paging::PagingError::CursorBinding)?;
    Ok(EvidenceCursorKey {
        created_at: last_key
            .get("created_at")
            .and_then(Value::as_str)
            .ok_or(loop_engine_core::operations::paging::PagingError::CursorBinding)?
            .to_owned(),
        evidence_id: last_key
            .get("evidence_id")
            .and_then(Value::as_str)
            .ok_or(loop_engine_core::operations::paging::PagingError::CursorBinding)?
            .to_owned(),
    })
}

fn first_unreturned_evidence_cursor(
    candidates: &[(EvidenceInventoryRow, usize, EvidenceCursorKey)],
    count_limit: usize,
    byte_limit: usize,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    filter_fingerprint: &str,
) -> Result<Option<PageCursor>, EvidenceReadError> {
    let mut bytes = 0usize;
    for (selected, (index, (_, size, _))) in candidates.iter().enumerate().enumerate() {
        if *size > byte_limit && selected == 0 {
            return Err(loop_engine_core::operations::paging::PagingError::RowTooLarge.into());
        }
        if selected == count_limit || bytes.saturating_add(*size) > byte_limit {
            // `last_key` is an exclusive start key, so continuation binds to
            // the final returned row rather than skipping the first unreturned row.
            let key = &candidates[index - 1].2;
            return mint_evidence_cursor(
                integrity_key,
                filter_fingerprint,
                &key.created_at,
                &key.evidence_id,
            )
            .map(Some)
            .map_err(EvidenceReadError::from);
        }
        bytes = bytes.saturating_add(*size);
    }
    Ok(None)
}

fn mint_evidence_cursor(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    filter_fingerprint: &str,
    created_at: &str,
    evidence_id: &str,
) -> Result<PageCursor, loop_engine_core::operations::paging::PagingError> {
    let payload = json!({
        "collection": COLLECTION_EVIDENCE,
        "cursor_version": 1,
        "filter_fingerprint": filter_fingerprint,
        "last_key": {
            "created_at": created_at,
            "evidence_id": evidence_id,
        },
    });
    PageCursor::parse(
        mint_integrity_wire(integrity_key, payload)
            .map_err(|_| loop_engine_core::operations::paging::PagingError::CursorBinding)?,
    )
    .map_err(|_| loop_engine_core::operations::paging::PagingError::CursorBinding)
}

fn read_integrity_key(
    conn: &Connection,
) -> Result<[u8; INTEGRITY_KEY_BYTE_LENGTH], EvidenceReadError> {
    let blob: Vec<u8> = conn
        .query_row(
            &format!("SELECT value FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
            params![INTEGRITY_KEY_ROW_KEY],
            |row| row.get(0),
        )
        .map_err(|source| match source {
            SqliteError::QueryReturnedNoRows => {
                EvidenceReadError::Persistence(PersistenceError::MetadataKeyMissing {
                    key: INTEGRITY_KEY_ROW_KEY,
                })
            }
            other => {
                EvidenceReadError::Persistence(PersistenceError::MetadataRead { source: other })
            }
        })?;
    if blob.len() != INTEGRITY_KEY_BYTE_LENGTH {
        return Err(EvidenceReadError::Persistence(
            PersistenceError::MetadataKeyInvalidLength {
                key: INTEGRITY_KEY_ROW_KEY,
                expected: INTEGRITY_KEY_BYTE_LENGTH,
                actual: blob.len(),
            },
        ));
    }
    let mut key = [0u8; INTEGRITY_KEY_BYTE_LENGTH];
    key.copy_from_slice(&blob);
    Ok(key)
}

struct EvidenceReadTransaction<'conn> {
    conn: &'conn Connection,
    active: bool,
}

impl<'conn> EvidenceReadTransaction<'conn> {
    fn begin(conn: &'conn Connection) -> Result<Self, EvidenceReadError> {
        conn.execute_batch("BEGIN DEFERRED").map_err(sqlite_err)?;
        Ok(Self { conn, active: true })
    }

    fn commit(mut self) -> Result<(), EvidenceReadError> {
        self.conn.execute_batch("COMMIT").map_err(sqlite_err)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for EvidenceReadTransaction<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

fn sqlite_err(error: SqliteError) -> EvidenceReadError {
    EvidenceReadError::Persistence(PersistenceError::Open {
        path: PathBuf::new(),
        source: error,
    })
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
    canonical_value(value).to_string()
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
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

fn decode_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    wire: &str,
) -> Result<Value, ()> {
    let decoded = base64url_decode(wire).map_err(|_| ())?;
    let parsed: Value = serde_json::from_slice(&decoded).map_err(|_| ())?;
    if canonical_json(&parsed).as_bytes() != decoded.as_slice() {
        return Err(());
    }
    let Some(wrapper) = parsed.as_object() else {
        return Err(());
    };
    if wrapper.len() != 2 || !wrapper.contains_key("mac") || !wrapper.contains_key("payload") {
        return Err(());
    }
    let mac_b64 = wrapper.get("mac").and_then(Value::as_str).ok_or(())?;
    let payload = wrapper.get("payload").ok_or(())?.clone();
    let tag = base64url_decode(mac_b64).map_err(|_| ())?;
    if tag.len() != 32 {
        return Err(());
    }
    let expected = hmac_sha256(integrity_key, &mac_input(CURSOR_DOMAIN, &payload));
    if !constant_time_eq(&tag, &expected) {
        return Err(());
    }
    Ok(payload)
}

fn mint_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    payload: Value,
) -> Result<String, ()> {
    let tag = hmac_sha256(integrity_key, &mac_input(CURSOR_DOMAIN, &payload));
    let wire = json!({
        "mac": base64url_no_pad(&tag),
        "payload": payload,
    });
    let encoded = base64url_no_pad(canonical_json(&wire).as_bytes());
    if encoded.len() > OPAQUE_INTEGRITY_WIRE_UTF8_BYTES {
        return Err(());
    }
    Ok(encoded)
}

fn mac_input(domain: &[u8], payload: &Value) -> Vec<u8> {
    let mut data = domain.to_vec();
    data.push(0);
    data.extend_from_slice(canonical_json(payload).as_bytes());
    data
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
        COLLECTION_PAGE_DATA_BUDGET_BYTES, EVIDENCE_LOCATOR_UTF8_BYTES,
        SELECTED_EVIDENCE_CONTEXT_TOTAL_BYTES,
    };
    use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
    use loop_engine_core::model::ids::EvidenceKind;
    use loop_engine_core::model::time::ObservedAt;
    use loop_engine_core::operations::paging::PagingError;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::error::PersistenceError;
    use crate::persistence::migrations::{SUPPORTED_SCHEMA_VERSION, bundled_migrations};
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

    fn test_reads() -> (TempDir, SqliteEvidenceReads) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        open_at(&path, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION).unwrap();
        (dir, SqliteEvidenceReads::new(path))
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
            ) VALUES (?1, ?2, 1, 'ready', 'active', 1, 1, 1,
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

    fn insert_evidence(
        conn: &Connection,
        run_id: &str,
        evidence_id: &str,
        created_at: &str,
        locator: &str,
    ) {
        conn.execute(
            "INSERT INTO evidence (
                run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
            ) VALUES (?1, ?2, 'artifact', ?3, NULL, NULL, NULL, 'caller', ?4)",
            params![run_id, evidence_id, locator, created_at],
        )
        .unwrap();
    }

    fn insert_journal_and_association(
        conn: &Connection,
        run_id: &str,
        sequence: i64,
        evidence_id: &str,
        event_id: Option<&str>,
        gate_id: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
             VALUES (?1, ?2, 'completed', '{}')",
            params![run_id, sequence],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO evidence_associations (run_id, journal_sequence, evidence_id, event_id, gate_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, sequence, evidence_id, event_id, gate_id],
        )
        .unwrap();
    }

    fn selected_context_boundary_fixtures() -> Vec<(String, String)> {
        let locator = "x".repeat(EVIDENCE_LOCATOR_UTF8_BYTES);
        let observed_at = ObservedAt::parse("2026-07-17T12:00:00.000Z").unwrap();
        let mut fixtures = Vec::new();
        let mut records = Vec::new();
        loop {
            let evidence_id = format!("ev-{}", fixtures.len());
            let record = EvidenceRecord::new(
                EvidenceId::parse(&evidence_id).unwrap(),
                EvidenceKind::parse("artifact").unwrap(),
                locator.clone(),
                None,
                None,
                None,
                EvidenceSource::Caller,
                observed_at,
            )
            .unwrap();
            records.push(record);
            fixtures.push((evidence_id, locator.clone()));
            let encoded = selected_context_encoded_bytes(&records).unwrap();
            if encoded > SELECTED_EVIDENCE_CONTEXT_TOTAL_BYTES {
                return fixtures;
            }
        }
    }

    #[test]
    fn absent_store_path_read_fails_without_creating_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        assert_no_store_files(&path);

        let reads = SqliteEvidenceReads::new(path.clone());
        let run_id = RunId::parse("run-1").unwrap();
        let error = reads
            .inventory(
                &run_id,
                &PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            EvidenceReadError::Persistence(PersistenceError::Open { .. })
        ));
        assert_no_store_files(&path);
    }

    #[test]
    fn migrated_store_reads_unchanged() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-1",
            "2026-07-17T12:00:00.000Z",
            "opaque:one",
        );
        drop(conn);

        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let page = reads.inventory(&run_id, &request).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].record.id().as_str(), "ev-1");
        assert_eq!(page.rows[0].record.locator(), "opaque:one");
    }

    #[test]
    fn selected_evidence_rejects_missing_id_atomically() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-1",
            "2026-07-17T12:00:00.000Z",
            "opaque:one",
        );
        let run_id = RunId::parse("run-1").unwrap();
        let present = EvidenceId::parse("ev-1").unwrap();
        let missing = EvidenceId::parse("ev-missing").unwrap();
        assert!(matches!(
            reads.selected_evidence(&run_id, &[present.clone(), missing.clone()],),
            Err(SelectedEvidenceReadError::Unavailable)
        ));
    }

    #[test]
    fn selected_evidence_preserves_caller_order_and_empty_default() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-a",
            "2026-07-17T12:00:01.000Z",
            "opaque:a",
        );
        insert_evidence(
            &conn,
            "run-1",
            "ev-b",
            "2026-07-17T12:00:02.000Z",
            "opaque:b",
        );
        let run_id = RunId::parse("run-1").unwrap();
        assert!(reads.selected_evidence(&run_id, &[]).unwrap().is_empty());
        let selected = reads
            .selected_evidence(
                &run_id,
                &[
                    EvidenceId::parse("ev-b").unwrap(),
                    EvidenceId::parse("ev-a").unwrap(),
                ],
            )
            .unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|record| record.id().as_str())
                .collect::<Vec<_>>(),
            vec!["ev-b", "ev-a"]
        );
    }

    #[test]
    fn selected_evidence_preserves_duplicate_requested_ids() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-a",
            "2026-07-17T12:00:01.000Z",
            "opaque:a",
        );
        let run_id = RunId::parse("run-1").unwrap();
        let ev_a = EvidenceId::parse("ev-a").unwrap();
        let selected = reads
            .selected_evidence(&run_id, &[ev_a.clone(), ev_a])
            .unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].id().as_str(), "ev-a");
        assert_eq!(selected[1].id().as_str(), "ev-a");
    }

    #[test]
    fn inventory_excludes_unrelated_run_and_history_associations() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_run(&conn, "run-2", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-target",
            "2026-07-17T12:00:01.000Z",
            "opaque:target",
        );
        insert_evidence(
            &conn,
            "run-2",
            "ev-other-run",
            "2026-07-17T12:00:01.000Z",
            "opaque:other-run",
        );
        insert_journal_and_association(
            &conn,
            "run-1",
            1,
            "ev-target",
            Some("approved"),
            Some("review"),
        );
        insert_evidence(
            &conn,
            "run-1",
            "ev-peer",
            "2026-07-17T12:00:02.000Z",
            "opaque:peer",
        );
        insert_journal_and_association(&conn, "run-1", 2, "ev-peer", Some("rejected"), None);
        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let page = reads.inventory(&run_id, &request).unwrap();
        assert_eq!(page.rows.len(), 2);
        assert!(
            page.rows
                .iter()
                .all(|row| row.record.id().as_str() != "ev-other-run")
        );
        let target = page
            .rows
            .iter()
            .find(|row| row.record.id().as_str() == "ev-target")
            .expect("target row");
        assert_eq!(target.associations.len(), 1);
        assert_eq!(
            target.associations[0].event_id().map(|id| id.as_str()),
            Some("approved")
        );
        assert_eq!(
            target.associations[0].gate_id().map(|id| id.as_str()),
            Some("review")
        );
        let peer = page
            .rows
            .iter()
            .find(|row| row.record.id().as_str() == "ev-peer")
            .expect("peer row");
        assert_eq!(peer.associations.len(), 1);
        assert_eq!(
            peer.associations[0].event_id().map(|id| id.as_str()),
            Some("rejected")
        );
        assert!(peer.associations[0].gate_id().is_none());
    }

    #[test]
    fn inventory_orders_by_created_at_then_evidence_id() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-b",
            "2026-07-17T12:00:00.000Z",
            "opaque:b",
        );
        insert_evidence(
            &conn,
            "run-1",
            "ev-a",
            "2026-07-17T12:00:00.000Z",
            "opaque:a",
        );
        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let page = reads.inventory(&run_id, &request).unwrap();
        assert_eq!(
            page.rows
                .iter()
                .map(|row| row.record.id().as_str())
                .collect::<Vec<_>>(),
            vec!["ev-a", "ev-b"]
        );
    }

    #[test]
    fn inventory_rejects_oversized_first_row_without_truncation() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-large",
            "2026-07-17T12:00:00.000Z",
            &"x".repeat(8_000),
        );
        let run_id = RunId::parse("run-1").unwrap();
        let request = PageRequest::new(10, 256, None, ()).unwrap();
        assert!(matches!(
            reads.inventory(&run_id, &request),
            Err(EvidenceReadError::Page(PagingError::RowTooLarge))
        ));
    }

    #[test]
    fn selected_context_enforces_aggregate_bound_without_truncation() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let fixtures = selected_context_boundary_fixtures();
        let created_at = "2026-07-17T12:00:00.000Z";
        let mut ids = Vec::with_capacity(fixtures.len());
        for (evidence_id, locator) in &fixtures {
            insert_evidence(&conn, "run-1", evidence_id, created_at, locator);
            ids.push(EvidenceId::parse(evidence_id).unwrap());
        }
        let run_id = RunId::parse("run-1").unwrap();
        assert!(matches!(
            reads.selected_context(&run_id, &ids),
            Err(SelectedEvidenceReadError::Read(EvidenceReadError::Bound(_)))
        ));
    }

    #[test]
    fn inventory_mints_cursor_for_next_page() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        insert_evidence(
            &conn,
            "run-1",
            "ev-1",
            "2026-07-17T12:00:00.000Z",
            "opaque:1",
        );
        insert_evidence(
            &conn,
            "run-1",
            "ev-2",
            "2026-07-17T12:00:01.000Z",
            "opaque:2",
        );
        let run_id = RunId::parse("run-1").unwrap();
        let first = PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let page = reads.inventory(&run_id, &first).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert!(page.next_cursor.is_some());
        let second =
            PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, page.next_cursor, ()).unwrap();
        let tail = reads.inventory(&run_id, &second).unwrap();
        assert_eq!(tail.rows.len(), 1);
        assert_eq!(tail.rows[0].record.id().as_str(), "ev-2");
        assert!(tail.next_cursor.is_none());
    }

    fn inventory_row_for_locator(evidence_id: &str, locator: &str) -> EvidenceInventoryRow {
        let observed_at = ObservedAt::parse("2026-07-17T12:00:00.000Z").unwrap();
        let record = EvidenceRecord::new(
            EvidenceId::parse(evidence_id).unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            locator.to_owned(),
            None,
            None,
            None,
            EvidenceSource::Caller,
            observed_at,
        )
        .unwrap();
        EvidenceInventoryRow {
            record,
            associations: Vec::new(),
        }
    }

    fn inventory_locator_for_half_budget(byte_limit: usize) -> String {
        let mut locator = "x".to_owned();
        while locator.len() <= EVIDENCE_LOCATOR_UTF8_BYTES {
            let encoded =
                inventory_row_encoded_bytes(&inventory_row_for_locator("ev-probe", &locator))
                    .unwrap();
            if encoded > byte_limit / 2 && encoded <= byte_limit {
                return locator;
            }
            locator.push('x');
        }
        panic!("unable to derive inventory locator for byte budget {byte_limit}");
    }

    #[test]
    fn inventory_pages_byte_budget_without_skips_or_duplicates() {
        const BYTE_LIMIT: usize = 2_048;
        const COUNT_LIMIT: u16 = 10;
        let locator = inventory_locator_for_half_budget(BYTE_LIMIT);
        let per_row =
            inventory_row_encoded_bytes(&inventory_row_for_locator("ev-probe", &locator)).unwrap();
        assert!(per_row <= BYTE_LIMIT);
        assert!(per_row.saturating_mul(2) > BYTE_LIMIT);

        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        let expected_ids = ["ev-1", "ev-2", "ev-3", "ev-4"];
        for (index, evidence_id) in expected_ids.iter().enumerate() {
            insert_evidence(
                &conn,
                "run-1",
                evidence_id,
                &format!("2026-07-17T12:00:{index:02}.000Z"),
                &locator,
            );
        }

        let run_id = RunId::parse("run-1").unwrap();
        let mut cursor = None;
        let mut collected = Vec::new();
        loop {
            let request = PageRequest::new(COUNT_LIMIT, BYTE_LIMIT, cursor, ()).unwrap();
            let page = reads.inventory(&run_id, &request).unwrap();
            assert!(
                !page.rows.is_empty(),
                "inventory paging must emit at least one row per page"
            );
            for row in &page.rows {
                let id = row.record.id().as_str().to_owned();
                assert!(
                    !collected.contains(&id),
                    "duplicate evidence id {id} across pages"
                );
                assert_eq!(
                    row.record.locator(),
                    locator,
                    "inventory row must not be truncated"
                );
                collected.push(id);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            collected,
            expected_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn integrity_wire_rejects_edited_and_alias_base64url() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        let integrity_key = read_integrity_key(&conn).unwrap();
        let filter_fingerprint = evidence_filter_fingerprint(&RunId::parse("run-1").unwrap());
        let payload = json!({
            "collection": super::COLLECTION_EVIDENCE,
            "cursor_version": 1,
            "filter_fingerprint": filter_fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "ev-a",
            },
        });
        let wire = mint_integrity_wire(&integrity_key, payload).unwrap();

        let mut edited = wire.clone();
        let last = edited.pop().unwrap();
        edited.push(if last == 'A' { 'B' } else { 'A' });
        assert!(decode_integrity_wire(&integrity_key, &edited).is_err());

        let alias = format!("{wire}A");
        assert!(decode_integrity_wire(&integrity_key, &alias).is_err());
    }

    #[test]
    fn integrity_wire_rejects_noncanonical_wrapper() {
        let (_dir, reads) = test_reads();
        let conn = Connection::open(reads.path.clone()).unwrap();
        let integrity_key = read_integrity_key(&conn).unwrap();
        let filter_fingerprint = evidence_filter_fingerprint(&RunId::parse("run-1").unwrap());
        let payload = json!({
            "collection": super::COLLECTION_EVIDENCE,
            "cursor_version": 1,
            "filter_fingerprint": filter_fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "ev-a",
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
}
