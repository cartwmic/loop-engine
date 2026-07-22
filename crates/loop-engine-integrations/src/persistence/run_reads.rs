//! Provider-free SQLite run read queries against migration `0001`.
//!
//! Authoritative state comes solely from `runs` columns and stored graph/inputs snapshots.
//! Journal rows are never replayed. Provider catalog rows are not consulted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::run_reader::{RunListFilter, RunListRow};
use loop_engine_core::capabilities::{Page, PageCursor, PageRequest};
use loop_engine_core::model::bounded::OPAQUE_INTEGRITY_WIRE_UTF8_BYTES;
use loop_engine_core::model::ids::{IdentifierError, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::run::Run;
use loop_engine_core::operations::paging::{self, PagingError, bounded_page};
use loop_engine_core::operations::run_graph::{StoredGraph, project as project_graph};
use loop_engine_core::operations::run_show::{RunShow, project as project_show};
use rusqlite::{Connection, Row, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::persistence::error::PersistenceError;
use crate::persistence::mapping::{self, MappingError as PersistMappingError};
use crate::persistence::records::RunRecord;
use crate::persistence::sqlite::connect_read_only_with_pragmas;
use crate::persistence::sqlite::{INTEGRATION_METADATA_TABLE, INTEGRITY_KEY_ROW_KEY};
use crate::persistence::traced::{
    MutationClass, OptionalTraceSink, ReadCompleteExtras, close_read, run_read_failure,
    run_read_rejected,
};

const RUN_CATALOG_COLLECTION: &str = "run.catalog";
const CURSOR_DOMAIN: &[u8] = b"loop-engine.integrations.cursor-v1";

/// SQLite-backed provider-free run reader. Each call opens a fresh pragma-configured connection.
#[derive(Debug, Clone)]
pub struct SqliteRunReads {
    path: PathBuf,
    trace: OptionalTraceSink,
}

impl SqliteRunReads {
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, run_id: &RunId) -> Result<Run, RunReadError> {
        close_read(
            &self.trace,
            "run.show",
            MutationClass::ReadOnly,
            || self.get_impl(run_id),
            |_| ReadCompleteExtras::default(),
            run_read_rejected,
            run_read_failure,
        )
    }

    pub fn show(&self, run_id: &RunId) -> Result<RunShow, RunReadError> {
        close_read(
            &self.trace,
            "run.show",
            MutationClass::ReadOnly,
            || self.show_impl(run_id),
            |_| ReadCompleteExtras::default(),
            run_read_rejected,
            run_read_failure,
        )
    }

    pub fn graph(&self, run_id: &RunId) -> Result<StoredGraph, RunReadError> {
        close_read(
            &self.trace,
            "run.graph",
            MutationClass::ReadOnly,
            || self.graph_impl(run_id),
            |graph| {
                let _ = graph;
                ReadCompleteExtras::default()
            },
            run_read_rejected,
            run_read_failure,
        )
    }

    pub fn list(
        &self,
        request: &PageRequest<RunListFilter>,
    ) -> Result<Page<RunListRow>, RunReadError> {
        close_read(
            &self.trace,
            "run.list",
            MutationClass::ReadOnly,
            || self.list_impl(request),
            |page| {
                let page_data_bytes =
                    page.rows.iter().map(list_row_encoded_bytes).sum::<usize>() as u64;
                ReadCompleteExtras::for_page(page, page_data_bytes)
            },
            run_read_rejected,
            run_read_failure,
        )
    }

    fn get_impl(&self, run_id: &RunId) -> Result<Run, RunReadError> {
        let conn = self.connect()?;
        let row = load_run_row(&conn, run_id)?;
        reconstruct_run(row)
    }

    fn show_impl(&self, run_id: &RunId) -> Result<RunShow, RunReadError> {
        Ok(project_show(&self.get_impl(run_id)?))
    }

    fn graph_impl(&self, run_id: &RunId) -> Result<StoredGraph, RunReadError> {
        Ok(project_graph(&self.get_impl(run_id)?))
    }

    fn list_impl(
        &self,
        request: &PageRequest<RunListFilter>,
    ) -> Result<Page<RunListRow>, RunReadError> {
        let conn = self.connect()?;
        let integrity_key = load_integrity_key(&conn)?;
        let filter = request.filter();
        let expected_fingerprint = run_catalog_filter_fingerprint(filter);
        let after = match request.cursor() {
            None => None,
            Some(cursor) => Some(decode_run_catalog_cursor(
                cursor,
                &integrity_key,
                filter,
                &expected_fingerprint,
            )?),
        };

        let fetch = usize::from(request.limit()).saturating_add(1);
        let sql = run_catalog_query_sql(filter, after.is_some(), fetch);
        let mut statement = conn.prepare(&sql).map_err(|source| RunReadError::Corrupt {
            message: source.to_string(),
        })?;
        let mut query = match &after {
            None => statement.query([]),
            Some(key) => statement.query(params![key.created_at, key.created_at, key.stable_id]),
        }
        .map_err(|source| RunReadError::Corrupt {
            message: source.to_string(),
        })?;

        let mut candidates = Vec::new();
        while let Some(row) = query.next().map_err(|source| RunReadError::Corrupt {
            message: source.to_string(),
        })? {
            let catalog_row = RunCatalogRow::try_from(row)?;
            let list_row = RunListRow {
                run_id: catalog_row.run_id.clone(),
                label: catalog_row.label.clone(),
                lifecycle: catalog_row.lifecycle,
                current_state: catalog_row.current_state.clone(),
            };
            let encoded = list_row_encoded_bytes(&list_row);
            candidates.push((catalog_row, list_row, encoded));
        }

        let count_limit = usize::from(request.limit());
        let byte_limit = request.byte_limit();
        let next_cursor = first_unreturned_cursor(
            &candidates,
            count_limit,
            byte_limit,
            filter,
            &expected_fingerprint,
            &integrity_key,
        )?;
        let page = bounded_page(
            candidates.into_iter().map(|(_, row, size)| (row, size)),
            count_limit,
            byte_limit,
            next_cursor,
        )?;
        Ok(page)
    }

    fn connect(&self) -> Result<Connection, RunReadError> {
        connect_read_only_with_pragmas(&self.path).map_err(RunReadError::from)
    }
}

#[derive(Debug, Error)]
pub enum RunReadError {
    #[error("run not found: {run_id}")]
    NotFound { run_id: RunId },
    #[error("stored run data is corrupt: {message}")]
    Corrupt { message: String },
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Page(#[from] PagingError),
}

/// Integration-owned raw run row before core mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRunRow(RunRecord);

impl RawRunRow {
    /// Returns the underlying persistence row record.
    pub fn into_record(self) -> RunRecord {
        self.0
    }
}

impl From<RunRecord> for RawRunRow {
    fn from(record: RunRecord) -> Self {
        Self(record)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunCatalogRow {
    created_at: String,
    run_id: RunId,
    label: Option<String>,
    lifecycle: Lifecycle,
    current_state: StateId,
}

impl TryFrom<&Row<'_>> for RunCatalogRow {
    type Error = RunReadError;

    fn try_from(row: &Row<'_>) -> Result<Self, Self::Error> {
        let lifecycle = parse_lifecycle(&sqlite_get::<String>(row, "lifecycle")?)?;
        Ok(Self {
            created_at: sqlite_get(row, "created_at")?,
            run_id: RunId::parse(sqlite_get::<String>(row, "run_id")?).map_err(corrupt_id)?,
            label: sqlite_get(row, "label")?,
            lifecycle,
            current_state: StateId::parse(sqlite_get::<String>(row, "current_state")?)
                .map_err(corrupt_id)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogKeyset {
    created_at: String,
    stable_id: String,
}

fn load_run_row(conn: &Connection, run_id: &RunId) -> Result<RunRecord, RunReadError> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                    workflow_state_version, lifecycle_version, label_version, label,
                    graph_revision, canonical_graph_version,
                    graph_canonical_projection_json, inputs_json, created_at
             FROM runs
             WHERE run_id = ?1",
        )
        .map_err(|source| RunReadError::Corrupt {
            message: source.to_string(),
        })?;
    let result = statement.query_row(params![run_id.as_str()], run_record_from_row);
    match result {
        Ok(row) => Ok(row),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(RunReadError::NotFound {
            run_id: run_id.clone(),
        }),
        Err(source) => Err(RunReadError::Corrupt {
            message: source.to_string(),
        }),
    }
}

fn reconstruct_run(record: RunRecord) -> Result<Run, RunReadError> {
    mapping::run_from_record(&record).map_err(map_mapping_error)
}

fn run_record_from_row(row: &Row<'_>) -> Result<RunRecord, rusqlite::Error> {
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
}

fn sqlite_get<T>(row: &Row<'_>, column: &str) -> Result<T, RunReadError>
where
    T: rusqlite::types::FromSql,
{
    row.get(column).map_err(|source| RunReadError::Corrupt {
        message: source.to_string(),
    })
}

fn map_mapping_error(error: PersistMappingError) -> RunReadError {
    RunReadError::Corrupt {
        message: error.to_string(),
    }
}

fn parse_lifecycle(value: &str) -> Result<Lifecycle, RunReadError> {
    match value {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        other => Err(RunReadError::Corrupt {
            message: format!("unknown lifecycle {other:?}"),
        }),
    }
}

fn corrupt_id(error: IdentifierError) -> RunReadError {
    RunReadError::Corrupt {
        message: error.to_string(),
    }
}

fn run_catalog_query_sql(filter: &RunListFilter, has_keyset: bool, fetch: usize) -> String {
    let lifecycle = match filter {
        RunListFilter::Active => "lifecycle = 'active'",
        RunListFilter::Terminal => "lifecycle IN ('final', 'terminated')",
        RunListFilter::All => "1 = 1",
    };
    let keyset = if has_keyset {
        "AND (created_at > ?1 OR (created_at = ?2 AND run_id > ?3))"
    } else {
        ""
    };
    format!(
        "SELECT run_id, label, lifecycle, current_state, created_at
         FROM runs
         WHERE {lifecycle} {keyset}
         ORDER BY created_at ASC, run_id ASC
         LIMIT {fetch}"
    )
}

fn run_catalog_filter_fingerprint(filter: &RunListFilter) -> String {
    let (all, terminal) = match filter {
        RunListFilter::Active => (false, false),
        RunListFilter::Terminal => (false, true),
        RunListFilter::All => (true, false),
    };
    digest_canonical_json(&json!({ "all": all, "terminal": terminal }))
}

fn run_list_row_item_json(row: &RunListRow) -> Value {
    let lifecycle = match row.lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    };
    json!({
        "run_id": row.run_id.as_str(),
        "label": row.label,
        "lifecycle": lifecycle,
        "current_state": row.current_state.as_str(),
    })
}

fn list_row_encoded_bytes(row: &RunListRow) -> usize {
    serde_json::to_vec(&run_list_row_item_json(row))
        .expect("run list row JSON is always serializable")
        .len()
}

fn first_unreturned_cursor(
    candidates: &[(RunCatalogRow, RunListRow, usize)],
    count_limit: usize,
    byte_limit: usize,
    filter: &RunListFilter,
    filter_fingerprint: &str,
    integrity_key: &[u8; 32],
) -> Result<Option<PageCursor>, RunReadError> {
    let mut bytes = 0usize;
    for (selected, (index, (_, _, size))) in candidates.iter().enumerate().enumerate() {
        if *size > byte_limit && selected == 0 {
            return Err(PagingError::RowTooLarge.into());
        }
        if selected == count_limit || bytes.saturating_add(*size) > byte_limit {
            // `last_key` is an exclusive start key, so continuation binds to
            // the final returned row rather than skipping the first unreturned row.
            let key = &candidates[index - 1].0;
            return mint_run_catalog_cursor(
                filter,
                filter_fingerprint,
                integrity_key,
                &key.created_at,
                key.run_id.as_str(),
            )
            .map(Some);
        }
        bytes = bytes.saturating_add(*size);
    }
    Ok(None)
}

fn decode_run_catalog_cursor(
    cursor: &PageCursor,
    integrity_key: &[u8; 32],
    filter: &RunListFilter,
    expected_fingerprint: &str,
) -> Result<CatalogKeyset, RunReadError> {
    let payload = decode_integrity_wire(integrity_key, cursor.as_str())?;
    if payload.get("cursor_version").and_then(Value::as_u64) != Some(1) {
        return Err(PagingError::CursorVersion.into());
    }
    if payload.get("collection").and_then(Value::as_str) != Some(RUN_CATALOG_COLLECTION) {
        return Err(PagingError::CursorBinding.into());
    }
    if payload.get("filter_fingerprint").and_then(Value::as_str) != Some(expected_fingerprint) {
        return Err(PagingError::CursorBinding.into());
    }
    paging::validate_binding(
        &paging::DecodedCursorBinding {
            schema_version: 1,
            operation: "run.list".into(),
            filter: run_list_filter_name(filter).into(),
        },
        "run.list",
        run_list_filter_name(filter),
    )?;
    let last_key = payload.get("last_key").ok_or(PagingError::CursorBinding)?;
    let Some(created_at) = last_key.get("created_at").and_then(Value::as_str) else {
        return Err(PagingError::CursorBinding.into());
    };
    let Some(stable_id) = last_key.get("stable_id").and_then(Value::as_str) else {
        return Err(PagingError::CursorBinding.into());
    };
    Ok(CatalogKeyset {
        created_at: created_at.to_owned(),
        stable_id: stable_id.to_owned(),
    })
}

fn mint_run_catalog_cursor(
    _filter: &RunListFilter,
    filter_fingerprint: &str,
    integrity_key: &[u8; 32],
    created_at: &str,
    stable_id: &str,
) -> Result<PageCursor, RunReadError> {
    let payload = json!({
        "collection": RUN_CATALOG_COLLECTION,
        "cursor_version": 1,
        "filter_fingerprint": filter_fingerprint,
        "last_key": {
            "created_at": created_at,
            "stable_id": stable_id,
        },
    });
    PageCursor::parse(mint_integrity_wire(integrity_key, payload)?)
        .map_err(PagingError::Bound)
        .map_err(Into::into)
}

fn run_list_filter_name(filter: &RunListFilter) -> &'static str {
    match filter {
        RunListFilter::Active => "active",
        RunListFilter::Terminal => "terminal",
        RunListFilter::All => "all",
    }
}

fn digest_canonical_json(value: &Value) -> String {
    sha256_hex(canonical_json(value).as_bytes())
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

fn mint_integrity_wire(integrity_key: &[u8; 32], payload: Value) -> Result<String, RunReadError> {
    let tag = hmac_sha256(integrity_key, &mac_input(CURSOR_DOMAIN, &payload));
    let wire = json!({
        "mac": base64url_no_pad(&tag),
        "payload": payload,
    });
    let encoded = base64url_no_pad(canonical_json(&wire).as_bytes());
    if encoded.len() > OPAQUE_INTEGRITY_WIRE_UTF8_BYTES {
        return Err(
            PagingError::Bound(loop_engine_core::model::bounded::BoundError::TooLong {
                field: "page_cursor",
                max: OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
                actual: encoded.len(),
            })
            .into(),
        );
    }
    Ok(encoded)
}

fn decode_integrity_wire(integrity_key: &[u8; 32], wire: &str) -> Result<Value, RunReadError> {
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

fn load_integrity_key(conn: &Connection) -> Result<[u8; 32], RunReadError> {
    let bytes: Vec<u8> = conn
        .query_row(
            &format!("SELECT value FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
            [INTEGRITY_KEY_ROW_KEY],
            |row| row.get(0),
        )
        .map_err(|source| RunReadError::Corrupt {
            message: source.to_string(),
        })?;
    if bytes.len() != 32 {
        return Err(RunReadError::Corrupt {
            message: "integrity_key must be 32 bytes".into(),
        });
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
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
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |accumulator, (left, right)| {
            accumulator | (left ^ right)
        })
        == 0
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    let mut index = 0usize;
    while index + 3 <= bytes.len() {
        let chunk = &bytes[index..index + 3];
        let block = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | u32::from(chunk[2]);
        output.push(TABLE[((block >> 18) & 63) as usize] as char);
        output.push(TABLE[((block >> 12) & 63) as usize] as char);
        output.push(TABLE[((block >> 6) & 63) as usize] as char);
        output.push(TABLE[(block & 63) as usize] as char);
        index += 3;
    }
    let remainder = bytes.len() - index;
    if remainder == 1 {
        let block = (bytes[index] as u32) << 16;
        output.push(TABLE[((block >> 18) & 63) as usize] as char);
        output.push(TABLE[((block >> 12) & 63) as usize] as char);
    } else if remainder == 2 {
        let block = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8);
        output.push(TABLE[((block >> 18) & 63) as usize] as char);
        output.push(TABLE[((block >> 12) & 63) as usize] as char);
        output.push(TABLE[((block >> 6) & 63) as usize] as char);
    }
    output
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
    let mut output = Vec::new();
    for ch in input.bytes() {
        let value = map[ch as usize];
        if value == 255 {
            return Err(());
        }
        bits = (bits << 6) | u32::from(value);
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            output.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    if base64url_no_pad(&output) != input {
        return Err(());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::PageRequest;
    use loop_engine_core::capabilities::run_reader::RunListFilter;
    use loop_engine_core::model::ids::RunId;
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::operations::run_graph::StoredGraph;
    use loop_engine_core::operations::run_show::RunShow;
    use tempfile::TempDir;

    use loop_engine_core::operations::paging::PagingError;
    use rusqlite::params;
    use serde_json::json;

    use super::{
        RunReadError, RunRecord, SqliteRunReads, decode_integrity_wire, list_row_encoded_bytes,
        load_integrity_key, mint_integrity_wire, reconstruct_run, run_catalog_filter_fingerprint,
        run_list_row_item_json,
    };
    use crate::persistence::error::PersistenceError;
    use crate::persistence::mapping::{self, MappingError as PersistMappingError};
    use crate::persistence::sqlite::SqliteStore;

    const MINIMAL_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"none"}}],"transitions":[]}"#;

    fn graph_revision_for_json(json: &str) -> String {
        let record = RunRecord {
            run_id: "019f0000-0000-7000-8000-000000000000".into(),
            registration_id: "019f0000-0000-7000-8000-000000000001".into(),
            config_revision_at_create: 1,
            current_state: "draft".into(),
            lifecycle: "active".into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: None,
            graph_revision:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            canonical_graph_version: 1,
            graph_canonical_projection_json: json.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:00.000Z".into(),
        };
        match mapping::run_from_record(&record) {
            Err(PersistMappingError::GraphDigestMismatch { computed, .. }) => computed,
            Ok(_) => panic!("placeholder graph revision must not match"),
            Err(error) => panic!("unexpected mapping error: {error:?}"),
        }
    }

    fn seed_registration(conn: &rusqlite::Connection, registration_id: &str, enabled: bool) {
        conn.execute(
            "INSERT INTO provider_registrations (
                registration_id, handle, enabled, config_revision, executable,
                argv_json, working_directory, timeout_seconds, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 1, '/bin/true', '[]', '/tmp', 60, '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
            params![
                registration_id,
                if enabled {
                    Some("provider")
                } else {
                    None
                },
                i32::from(enabled),
            ],
        )
        .unwrap();
    }

    fn seed_run(
        conn: &rusqlite::Connection,
        run_id: &str,
        registration_id: &str,
        lifecycle: &str,
        current_state: &str,
        created_at: &str,
        label: Option<&str>,
    ) {
        let graph_revision = graph_revision_for_json(MINIMAL_GRAPH_JSON);
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, label, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
             ) VALUES (?1, ?2, 1, ?3, ?4, 1, 1, 1, ?5, ?6, 1, ?7, '{}', ?8)",
            params![
                run_id,
                registration_id,
                current_state,
                lifecycle,
                label,
                graph_revision,
                MINIMAL_GRAPH_JSON,
                created_at,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
            [run_id],
        )
        .unwrap();
    }

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

    fn open_seeded_store(enabled_provider: bool) -> (TempDir, SqliteStore, String) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let store = SqliteStore::open(&path).unwrap();
        let registration_id = "019f0000-0000-7000-8000-000000000001";
        seed_registration(store.connection(), registration_id, enabled_provider);
        seed_run(
            store.connection(),
            "019f0000-0000-7000-8000-000000000101",
            registration_id,
            "active",
            "draft",
            "2026-07-17T12:00:01.000Z",
            Some("active-run"),
        );
        seed_run(
            store.connection(),
            "019f0000-0000-7000-8000-000000000102",
            registration_id,
            "final",
            "draft",
            "2026-07-17T12:00:02.000Z",
            None,
        );
        seed_run(
            store.connection(),
            "019f0000-0000-7000-8000-000000000103",
            registration_id,
            "terminated",
            "draft",
            "2026-07-17T12:00:03.000Z",
            None,
        );
        (directory, store, registration_id.to_owned())
    }

    #[test]
    fn absent_store_path_read_fails_without_creating_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        assert_no_store_files(&path);

        let reader = SqliteRunReads::new(&path);
        let error = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap_err();
        assert!(matches!(
            error,
            RunReadError::Persistence(PersistenceError::Open { .. })
        ));
        assert_no_store_files(&path);
    }

    #[test]
    fn migrated_store_reads_unchanged() {
        let (_dir, store, _) = open_seeded_store(true);
        let path = store.path().to_path_buf();
        drop(store);

        let reader = SqliteRunReads::new(path);
        let run = reader
            .get(&RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap())
            .unwrap();
        assert_eq!(run.lifecycle(), Lifecycle::Active);
        assert_eq!(run.label(), Some("active-run"));
    }

    #[test]
    fn fresh_connection_reads_authoritative_columns_after_restart() {
        let (_dir, store, _registration_id) = open_seeded_store(true);
        let path = store.path().to_path_buf();
        drop(store);

        let reader = SqliteRunReads::new(path);
        let run = reader
            .get(&RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap())
            .unwrap();
        assert_eq!(run.lifecycle(), Lifecycle::Active);
        assert_eq!(run.label(), Some("active-run"));
    }

    #[test]
    fn show_and_graph_project_reconstructed_run_without_provider_lookup() {
        let (_dir, store, _) = open_seeded_store(true);
        let reader = SqliteRunReads::new(store.path());
        let run_id = RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap();
        let show: RunShow = reader.show(&run_id).unwrap();
        let graph: StoredGraph = reader.graph(&run_id).unwrap();
        assert_eq!(show.run_id, run_id);
        assert_eq!(
            graph.revision.as_str(),
            graph_revision_for_json(MINIMAL_GRAPH_JSON).as_str()
        );
    }

    #[test]
    fn list_filters_active_terminal_and_all_with_stable_keyset() {
        let (_dir, store, _) = open_seeded_store(true);
        let reader = SqliteRunReads::new(store.path());

        let active = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::Active).unwrap())
            .unwrap();
        assert_eq!(active.rows.len(), 1);
        assert_eq!(
            active.rows[0].run_id.as_str(),
            "019f0000-0000-7000-8000-000000000101"
        );

        let terminal = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::Terminal).unwrap())
            .unwrap();
        assert_eq!(terminal.rows.len(), 2);

        let all = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap();
        assert_eq!(all.rows.len(), 3);

        let paged = reader
            .list(&PageRequest::new(1, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap();
        assert_eq!(paged.rows.len(), 1);
        assert!(paged.next_cursor.is_some());
        let page_two = reader
            .list(
                &PageRequest::new(10, 1_048_576, paged.next_cursor.clone(), RunListFilter::All)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(page_two.rows.len(), 2);
    }

    #[test]
    fn journal_payload_contradiction_does_not_override_authoritative_columns() {
        let (_dir, store, registration_id) = open_seeded_store(true);
        store
            .connection()
            .execute(
                "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
                 VALUES (?1, 1, 'completed', ?2)",
                params![
                    "019f0000-0000-7000-8000-000000000101",
                    r#"{"kind":"transition","target_state":"other","lifecycle":"final"}"#
                ],
            )
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE provider_registrations SET enabled = 0, handle = NULL WHERE registration_id = ?1",
                [registration_id],
            )
            .unwrap();

        let reader = SqliteRunReads::new(store.path());
        let run = reader
            .get(&RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap())
            .unwrap();
        assert_eq!(run.lifecycle(), Lifecycle::Active);
        assert_eq!(run.current_state().as_str(), "draft");
    }

    #[test]
    fn run_record_delegates_to_t106_mapping() {
        let row = RunRecord {
            run_id: "019f0000-0000-7000-8000-000000000101".into(),
            registration_id: "019f0000-0000-7000-8000-000000000001".into(),
            config_revision_at_create: 1,
            current_state: "draft".into(),
            lifecycle: "active".into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: Some("label".into()),
            graph_revision: graph_revision_for_json(MINIMAL_GRAPH_JSON),
            canonical_graph_version: 1,
            graph_canonical_projection_json: MINIMAL_GRAPH_JSON.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:01.000Z".into(),
        };
        let run = reconstruct_run(row).unwrap();
        assert_eq!(run.label(), Some("label"));
    }

    #[test]
    fn list_does_not_eagerly_load_tail_beyond_page_window() {
        let (_dir, store, registration_id) = open_seeded_store(true);
        let conn = store.connection();
        for index in 104..154 {
            let run_id = format!("019f0000-0000-7000-8000-{index:012}");
            seed_run(
                conn,
                &run_id,
                &registration_id,
                "active",
                "draft",
                &format!("2026-07-17T13:00:{index:02}.000Z"),
                None,
            );
        }
        conn.execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        conn.execute(
            "UPDATE runs SET lifecycle = 'unsupported-lifecycle' WHERE run_id = ?1",
            ["019f0000-0000-7000-8000-000000000153"],
        )
        .unwrap();
        conn.execute_batch("PRAGMA ignore_check_constraints = OFF")
            .unwrap();

        let reader = SqliteRunReads::new(store.path());
        let first = reader
            .list(&PageRequest::new(1, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert!(first.next_cursor.is_some());

        let active = reader
            .list(&PageRequest::new(1, 1_048_576, None, RunListFilter::Active).unwrap())
            .unwrap();
        assert_eq!(active.rows.len(), 1);
        assert!(active.next_cursor.is_some());

        assert!(matches!(
            reader.list(&PageRequest::new(100, 1_048_576, None, RunListFilter::All).unwrap(),),
            Err(RunReadError::Corrupt { .. })
        ));
    }

    #[test]
    fn oversized_first_list_row_errors_without_truncation() {
        let (_dir, store, _) = open_seeded_store(true);
        let reader = SqliteRunReads::new(store.path());
        let error = reader
            .list(&PageRequest::new(10, 1, None, RunListFilter::All).unwrap())
            .unwrap_err();
        assert!(matches!(
            error,
            RunReadError::Page(PagingError::RowTooLarge)
        ));
    }

    #[test]
    fn filter_fingerprints_are_stable_for_cursor_binding() {
        assert_eq!(
            run_catalog_filter_fingerprint(&RunListFilter::Active),
            run_catalog_filter_fingerprint(&RunListFilter::Active)
        );
        assert_ne!(
            run_catalog_filter_fingerprint(&RunListFilter::Active),
            run_catalog_filter_fingerprint(&RunListFilter::All)
        );
    }

    #[test]
    fn cursor_round_trip_uses_installation_integrity_key() {
        let (_dir, store, _) = open_seeded_store(true);
        let key = load_integrity_key(store.connection()).unwrap();
        let filter = RunListFilter::All;
        let fingerprint = run_catalog_filter_fingerprint(&filter);
        let payload = json!({
            "collection": super::RUN_CATALOG_COLLECTION,
            "cursor_version": 1,
            "filter_fingerprint": fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "019f0000-0000-7000-8000-000000000101",
            },
        });
        let wire = mint_integrity_wire(&key, payload.clone()).unwrap();
        let verified = decode_integrity_wire(&key, &wire).unwrap();
        assert_eq!(verified, payload);
        let cursor = super::mint_run_catalog_cursor(
            &filter,
            &fingerprint,
            &key,
            "2026-07-17T12:00:00.000Z",
            "019f0000-0000-7000-8000-000000000101",
        )
        .unwrap();
        let decoded =
            super::decode_run_catalog_cursor(&cursor, &key, &filter, &fingerprint).unwrap();
        assert_eq!(decoded.created_at, "2026-07-17T12:00:00.000Z");
        assert_eq!(decoded.stable_id, "019f0000-0000-7000-8000-000000000101");
    }

    #[test]
    fn list_row_encoded_bytes_matches_serde_json() {
        let (_dir, store, _) = open_seeded_store(true);
        let reader = SqliteRunReads::new(store.path());
        let page = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap();
        for row in &page.rows {
            let wire = run_list_row_item_json(row);
            let encoded = serde_json::to_vec(&wire).unwrap();
            assert_eq!(list_row_encoded_bytes(row), encoded.len());
        }
    }

    #[test]
    fn list_byte_stop_resumes_first_unreturned_row() {
        let (_dir, store, _) = open_seeded_store(true);
        let reader = SqliteRunReads::new(store.path());
        let sample = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap()
            .rows
            .into_iter()
            .next()
            .unwrap();
        let row_bytes = list_row_encoded_bytes(&sample);

        let exact_fit = reader
            .list(&PageRequest::new(10, row_bytes, None, RunListFilter::All).unwrap())
            .unwrap();
        assert_eq!(exact_fit.rows.len(), 1);
        assert!(exact_fit.next_cursor.is_some());

        let one_byte_under = reader.list(
            &PageRequest::new(10, row_bytes.saturating_sub(1), None, RunListFilter::All).unwrap(),
        );
        assert!(matches!(
            one_byte_under,
            Err(RunReadError::Page(PagingError::RowTooLarge))
        ));

        let traversal_budget = reader
            .list(&PageRequest::new(10, 1_048_576, None, RunListFilter::All).unwrap())
            .unwrap()
            .rows
            .iter()
            .map(list_row_encoded_bytes)
            .max()
            .unwrap();
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let page = reader
                .list(&PageRequest::new(10, traversal_budget, cursor, RunListFilter::All).unwrap())
                .unwrap();
            assert_eq!(page.rows.len(), 1);
            listed.push(page.rows[0].run_id.as_str().to_owned());
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(listed.len(), 3);
    }

    #[test]
    fn integrity_wire_rejects_edited_and_alias_base64url() {
        let (_dir, store, _) = open_seeded_store(true);
        let key = load_integrity_key(store.connection()).unwrap();
        let filter = RunListFilter::All;
        let fingerprint = run_catalog_filter_fingerprint(&filter);
        let payload = json!({
            "collection": super::RUN_CATALOG_COLLECTION,
            "cursor_version": 1,
            "filter_fingerprint": fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "019f0000-0000-7000-8000-000000000101",
            },
        });
        let wire = mint_integrity_wire(&key, payload).unwrap();

        let mut edited = wire.clone();
        let last = edited.pop().unwrap();
        edited.push(if last == 'A' { 'B' } else { 'A' });
        assert!(decode_integrity_wire(&key, &edited).is_err());

        let alias = format!("{wire}A");
        assert!(decode_integrity_wire(&key, &alias).is_err());
    }

    #[test]
    fn integrity_wire_rejects_noncanonical_wrapper() {
        let (_dir, store, _) = open_seeded_store(true);
        let key = load_integrity_key(store.connection()).unwrap();
        let filter = RunListFilter::All;
        let fingerprint = run_catalog_filter_fingerprint(&filter);
        let payload = json!({
            "collection": super::RUN_CATALOG_COLLECTION,
            "cursor_version": 1,
            "filter_fingerprint": fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "019f0000-0000-7000-8000-000000000101",
            },
        });
        let wire = mint_integrity_wire(&key, payload).unwrap();
        let canonical = String::from_utf8(super::base64url_decode(&wire).unwrap()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        let mac = parsed["mac"].as_str().unwrap();
        let payload_json = serde_json::to_string(&parsed["payload"]).unwrap();

        let reordered = format!(r#"{{"payload":{payload_json},"mac":"{mac}"}}"#);
        assert!(
            decode_integrity_wire(&key, &super::base64url_no_pad(reordered.as_bytes())).is_err()
        );

        let whitespace = canonical.replace(',', ", ");
        assert!(
            decode_integrity_wire(&key, &super::base64url_no_pad(whitespace.as_bytes())).is_err()
        );

        let duplicate = format!(r#"{{"mac":"{mac}","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(
            decode_integrity_wire(&key, &super::base64url_no_pad(duplicate.as_bytes())).is_err()
        );

        let extra = format!(r#"{{"extra":"x","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(decode_integrity_wire(&key, &super::base64url_no_pad(extra.as_bytes())).is_err());
    }
}
