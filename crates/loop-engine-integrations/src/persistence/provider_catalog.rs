//! SQLite-backed provider catalog persistence (T107).

use std::collections::BTreeMap;
use std::path::PathBuf;

use loop_engine_core::capabilities::provider_catalog::{
    ActiveRunImpact, ActiveSetSnapshot, CatalogMutation, CatalogMutationResult,
    DisableAcknowledgement, ProviderCatalog, ProviderCatalogRow, ProviderConfig,
    ProviderListFilter, ProviderResolveFailure, ResolvedProviderConfig,
};
use loop_engine_core::capabilities::{Page, PageCursor, PageRequest};
use loop_engine_core::model::ids::{GraphRevision, ProviderHandle, RegistrationId, RunId};
use loop_engine_core::model::provider::ProviderRegistration;
use loop_engine_core::operations::paging::bounded_page;
use rusqlite::{Connection, Error as SqliteError, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::error::{CommitOutcomeError, PersistenceError};
use super::sqlite::commit::{CatalogRegistrationExpectation, finish_committed_transaction};
use super::sqlite::{
    INTEGRATION_METADATA_TABLE, INTEGRITY_KEY_BYTE_LENGTH, INTEGRITY_KEY_ROW_KEY,
    connect_read_only_with_pragmas, connect_with_pragmas,
};
use super::traced::{
    MutationClass, OptionalTraceSink, ReadCompleteExtras, SemanticOutcome, WriteExecution,
    WriteTraceSession, catalog_mutation_operation, catalog_read_failure, catalog_read_rejected,
    close_read, close_write, committed_or_unconfirmed, rollback_open_transaction,
};

const CURSOR_DOMAIN: &[u8] = b"loop-engine.integrations.cursor-v1";
const DISABLE_ACK_DOMAIN: &[u8] = b"loop-engine.integrations.disable-ack-v1";

const COLLECTION_REGISTRATIONS: &str = "provider.registrations";
const COLLECTION_REGISTRATION_ACTIVE_RUNS: &str = "provider.registration_active_runs";
const COLLECTION_DISABLE_WARNINGS: &str = "provider.disable_warnings";

const DISABLE_ACK_KIND: &str = "provider.disable_ack";

/// SQLite provider catalog store; each operation opens its own pragma-configured connection.
#[derive(Debug, Clone)]
pub struct SqliteProviderCatalog {
    pub path: PathBuf,
    trace: OptionalTraceSink,
}

/// Final or intermediate disable warning page with optional acknowledgement token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableWarningsPage {
    pub impacts: Page<ActiveRunImpact>,
    pub snapshot: ActiveSetSnapshot,
    pub acknowledgement: Option<DisableAcknowledgement>,
}

#[derive(Debug, Error)]
pub enum CatalogPersistenceError {
    #[error("registration not found")]
    NotFound,
    #[error("registration is disabled")]
    Disabled,
    #[error("provider handle is already enabled")]
    Duplicate,
    #[error("provider handle is occupied")]
    Occupied,
    #[error("catalog state is stale")]
    Stale,
    #[error("cursor is invalid")]
    InvalidCursor,
    #[error("disable acknowledgement is invalid")]
    InvalidAck,
    #[error("database constraint violation")]
    Constraint,
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("row mapping failed: {0}")]
    Mapping(String),
    #[error("commit I/O failed and durable outcome could not be verified")]
    CommitOutcomeUnverified,
    #[error("commit I/O failed and partial durable state indicates integrity failure")]
    CommitIntegrityFailure,
}

impl CommitOutcomeError for CatalogPersistenceError {
    fn is_commit_outcome_unverified(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnverified)
    }

    fn is_commit_integrity_failure(&self) -> bool {
        matches!(self, Self::CommitIntegrityFailure)
    }
}

impl SqliteProviderCatalog {
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

    fn connect_read(&self) -> Result<Connection, CatalogPersistenceError> {
        connect_read_only_with_pragmas(&self.path).map_err(CatalogPersistenceError::from)
    }

    fn connect_write(&self) -> Result<Connection, CatalogPersistenceError> {
        connect_with_pragmas(&self.path).map_err(CatalogPersistenceError::from)
    }

    /// Paginated disable warning traversal with cursor v1 integrity and final-page ack minting.
    pub fn disable_warnings_page(
        &self,
        registration_id: &RegistrationId,
        request: &PageRequest<()>,
    ) -> Result<DisableWarningsPage, CatalogPersistenceError> {
        close_read(
            &self.trace,
            "provider.disable",
            MutationClass::ReadOnly,
            || self.disable_warnings_page_impl(registration_id, request),
            |page| {
                let page_data_bytes = page
                    .impacts
                    .rows
                    .iter()
                    .map(encoded_active_run_bytes)
                    .sum::<usize>() as u64;
                ReadCompleteExtras::for_page(&page.impacts, page_data_bytes)
            },
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn disable_warnings_page_impl(
        &self,
        registration_id: &RegistrationId,
        request: &PageRequest<()>,
    ) -> Result<DisableWarningsPage, CatalogPersistenceError> {
        let conn = self.connect_read()?;
        let snapshot_transaction = CatalogReadTransaction::begin(&conn)?;
        let integrity_key = read_integrity_key(&conn)?;
        let row = load_registration_row(&conn, registration_id.as_str())?
            .ok_or(CatalogPersistenceError::NotFound)?;
        if row.enabled == 0 {
            return Err(CatalogPersistenceError::Disabled);
        }
        let snapshot = compute_active_set_snapshot(&conn, registration_id, row.config_revision)?;
        let filter_fingerprint =
            disable_warnings_filter_fingerprint(registration_id, row.config_revision, &snapshot)?;
        let page = page_active_runs(
            &conn,
            registration_id,
            request,
            COLLECTION_DISABLE_WARNINGS,
            &filter_fingerprint,
            &integrity_key,
        )?;
        let acknowledgement = if page.next_cursor.is_none() && !page.rows.is_empty() {
            Some(mint_disable_acknowledgement(
                &integrity_key,
                registration_id,
                row.config_revision,
                &snapshot,
                page.traversal_digest
                    .as_deref()
                    .ok_or(CatalogPersistenceError::InvalidCursor)?,
            )?)
        } else if page.next_cursor.is_none() && page.rows.is_empty() && snapshot.count() == 0 {
            Some(mint_disable_acknowledgement(
                &integrity_key,
                registration_id,
                row.config_revision,
                &snapshot,
                &RunIdsTraversalDigestHasher::new().finish(),
            )?)
        } else {
            None
        };
        let result = DisableWarningsPage {
            impacts: Page {
                rows: page.rows,
                next_cursor: page.next_cursor,
            },
            snapshot,
            acknowledgement,
        };
        snapshot_transaction.commit()?;
        Ok(result)
    }
}

impl ProviderCatalog for SqliteProviderCatalog {
    type Error = CatalogPersistenceError;

    fn classify_resolve_failure(error: &Self::Error) -> ProviderResolveFailure {
        match error {
            CatalogPersistenceError::NotFound => ProviderResolveFailure::Missing,
            CatalogPersistenceError::Disabled => ProviderResolveFailure::Tombstoned,
            CatalogPersistenceError::Stale => ProviderResolveFailure::Stale,
            _ => ProviderResolveFailure::Persistence,
        }
    }

    fn resolve_enabled(
        &self,
        registration_id: &RegistrationId,
    ) -> Result<ResolvedProviderConfig, Self::Error> {
        close_read(
            &self.trace,
            "provider.check",
            MutationClass::ReadOnly,
            || {
                let conn = self.connect_read()?;
                let row = load_registration_row(&conn, registration_id.as_str())?
                    .ok_or(CatalogPersistenceError::NotFound)?;
                if row.enabled == 0 {
                    return Err(CatalogPersistenceError::Disabled);
                }
                row_to_resolved(&row)
            },
            |_| ReadCompleteExtras::default(),
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn resolve_handle(&self, handle: &ProviderHandle) -> Result<ProviderCatalogRow, Self::Error> {
        close_read(
            &self.trace,
            "provider.check",
            MutationClass::ReadOnly,
            || {
                let conn = self.connect_read()?;
                let row = conn
                    .query_row(
                        "SELECT registration_id, handle, enabled, config_revision, executable, argv_json, \
                         working_directory, timeout_seconds, created_at, updated_at
                         FROM provider_registrations
                         WHERE handle = ?1 AND enabled = 1",
                        params![handle.as_str()],
                        map_registration_row,
                    )
                    .optional()
                    .map_err(sqlite_err)?
                    .ok_or(CatalogPersistenceError::NotFound)?;
                row_to_catalog_row(&row)
            },
            |_| ReadCompleteExtras::default(),
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn list(
        &self,
        request: &PageRequest<ProviderListFilter>,
    ) -> Result<Page<ProviderCatalogRow>, Self::Error> {
        close_read(
            &self.trace,
            "provider.list",
            MutationClass::ReadOnly,
            || self.list_impl(request),
            |page| {
                let page_data_bytes = page
                    .rows
                    .iter()
                    .map(encoded_catalog_row_bytes)
                    .sum::<usize>() as u64;
                ReadCompleteExtras::for_page(page, page_data_bytes)
            },
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn active_run_impact(
        &self,
        registration_id: &RegistrationId,
        request: &PageRequest<()>,
    ) -> Result<Page<ActiveRunImpact>, Self::Error> {
        close_read(
            &self.trace,
            "provider.check",
            MutationClass::ReadOnly,
            || self.active_run_impact_impl(registration_id, request),
            |page| {
                let page_data_bytes = page
                    .rows
                    .iter()
                    .map(encoded_active_run_bytes)
                    .sum::<usize>() as u64;
                ReadCompleteExtras::for_page(page, page_data_bytes)
            },
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn active_set_snapshot(
        &self,
        registration_id: &RegistrationId,
    ) -> Result<ActiveSetSnapshot, Self::Error> {
        close_read(
            &self.trace,
            "provider.check",
            MutationClass::ReadOnly,
            || {
                let conn = self.connect_read()?;
                let snapshot_transaction = CatalogReadTransaction::begin(&conn)?;
                let row = load_registration_row(&conn, registration_id.as_str())?
                    .ok_or(CatalogPersistenceError::NotFound)?;
                let snapshot =
                    compute_active_set_snapshot(&conn, registration_id, row.config_revision)?;
                snapshot_transaction.commit()?;
                Ok(snapshot)
            },
            |_| ReadCompleteExtras::default(),
            catalog_read_rejected,
            catalog_read_failure,
        )
    }

    fn mutate(&self, command: CatalogMutation) -> Result<CatalogMutationResult, Self::Error> {
        let operation = catalog_mutation_operation(&command);
        close_write(
            &self.trace,
            operation,
            MutationClass::Catalog,
            |trace| self.mutate_impl(command, trace),
            |_| SemanticOutcome::Completed,
            catalog_mutation_error_semantic,
        )
    }
}

fn catalog_mutation_error_semantic(error: &CatalogPersistenceError) -> SemanticOutcome {
    match error {
        CatalogPersistenceError::NotFound
        | CatalogPersistenceError::Disabled
        | CatalogPersistenceError::Duplicate
        | CatalogPersistenceError::Occupied
        | CatalogPersistenceError::Stale
        | CatalogPersistenceError::InvalidCursor
        | CatalogPersistenceError::InvalidAck => SemanticOutcome::Rejected,
        CatalogPersistenceError::Constraint
        | CatalogPersistenceError::Persistence(_)
        | CatalogPersistenceError::Mapping(_)
        | CatalogPersistenceError::CommitOutcomeUnverified
        | CatalogPersistenceError::CommitIntegrityFailure => SemanticOutcome::Error,
    }
}

impl SqliteProviderCatalog {
    fn list_impl(
        &self,
        request: &PageRequest<ProviderListFilter>,
    ) -> Result<Page<ProviderCatalogRow>, CatalogPersistenceError> {
        let conn = self.connect_read()?;
        let integrity_key = read_integrity_key(&conn)?;
        let filter_fingerprint = registrations_filter_fingerprint(request.filter())?;
        let after = match request.cursor() {
            Some(cursor) => {
                let decoded = decode_cursor(
                    &integrity_key,
                    cursor.as_str(),
                    COLLECTION_REGISTRATIONS,
                    &filter_fingerprint,
                )?;
                match decoded {
                    DecodedCursor::RegistrationKey {
                        created_at,
                        stable_id,
                    } => Some((created_at, stable_id)),
                    DecodedCursor::RunKey { .. } => {
                        return Err(CatalogPersistenceError::InvalidCursor);
                    }
                }
            }
            None => None,
        };
        let (where_clause, _) = list_filter_clause(request.filter());
        let mut sql = format!(
            "SELECT registration_id, handle, enabled, config_revision, executable, argv_json, \
             working_directory, timeout_seconds, created_at, updated_at
             FROM provider_registrations
             {where_clause}"
        );
        if after.is_some() {
            sql.push_str(" AND (created_at > ?1 OR (created_at = ?1 AND registration_id > ?2))");
        }
        sql.push_str(" ORDER BY created_at ASC, registration_id ASC");
        let limit = usize::from(request.limit()) + 1;
        sql.push_str(&format!(" LIMIT {limit}"));
        let mut rows = Vec::new();
        match after {
            Some((created_at, stable_id)) => {
                let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
                let mapped = stmt
                    .query_map(params![created_at, stable_id], map_registration_row)
                    .map_err(sqlite_err)?;
                for row in mapped {
                    rows.push(row.map_err(sqlite_err)?);
                }
            }
            None => {
                let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
                let mapped = stmt
                    .query_map([], map_registration_row)
                    .map_err(sqlite_err)?;
                for row in mapped {
                    rows.push(row.map_err(sqlite_err)?);
                }
            }
        }
        let count_limit = usize::from(request.limit());
        let byte_limit = request.byte_limit();
        let mut candidates = Vec::new();
        for row in rows {
            let catalog_row = row_to_catalog_row(&row)?;
            let bytes = encoded_catalog_row_bytes(&catalog_row);
            candidates.push((row.created_at, row.registration_id, catalog_row, bytes));
        }
        let next_cursor = first_unreturned_registrations_cursor(
            &candidates,
            count_limit,
            byte_limit,
            &filter_fingerprint,
            &integrity_key,
        )?;
        bounded_page(
            candidates.into_iter().map(|(_, _, row, size)| (row, size)),
            count_limit,
            byte_limit,
            next_cursor,
        )
        .map_err(|_| CatalogPersistenceError::InvalidCursor)
    }

    fn active_run_impact_impl(
        &self,
        registration_id: &RegistrationId,
        request: &PageRequest<()>,
    ) -> Result<Page<ActiveRunImpact>, CatalogPersistenceError> {
        let conn = self.connect_read()?;
        let snapshot_transaction = CatalogReadTransaction::begin(&conn)?;
        if load_registration_row(&conn, registration_id.as_str())?
            .ok_or(CatalogPersistenceError::NotFound)?
            .enabled
            == 0
        {
            return Err(CatalogPersistenceError::Disabled);
        }
        let integrity_key = read_integrity_key(&conn)?;
        let filter_fingerprint = registration_active_runs_filter_fingerprint(registration_id)?;
        let page = page_active_runs(
            &conn,
            registration_id,
            request,
            COLLECTION_REGISTRATION_ACTIVE_RUNS,
            &filter_fingerprint,
            &integrity_key,
        )?;
        let result = Page {
            rows: page.rows,
            next_cursor: page.next_cursor,
        };
        snapshot_transaction.commit()?;
        Ok(result)
    }

    fn mutate_impl(
        &self,
        command: CatalogMutation,
        trace: Option<&WriteTraceSession<'_>>,
    ) -> WriteExecution<CatalogMutationResult, CatalogPersistenceError> {
        let conn = match self.connect_write() {
            Ok(conn) => conn,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        let integrity_key = match read_integrity_key(&conn) {
            Ok(key) => key,
            Err(error) => return WriteExecution::no_transaction(error),
        };
        if let Err(error) = conn.execute("BEGIN IMMEDIATE", []).map_err(sqlite_err) {
            return WriteExecution::no_transaction(error);
        }
        let result = mutate_in_transaction(&conn, &integrity_key, command, trace);
        match result {
            Ok((value, expectation)) => committed_or_unconfirmed(finish_committed_transaction(
                &self.path,
                conn,
                value,
                |read| expectation.verify(read),
                sqlite_err,
                || CatalogPersistenceError::CommitOutcomeUnverified,
                || CatalogPersistenceError::CommitIntegrityFailure,
                CatalogPersistenceError::from,
            )),
            Err(error) => rollback_open_transaction(&conn, error),
        }
    }
}

struct RegistrationRow {
    registration_id: String,
    handle: Option<String>,
    enabled: i64,
    config_revision: i64,
    executable: String,
    argv_json: String,
    working_directory: String,
    timeout_seconds: i64,
    created_at: String,
    _updated_at: String,
}

struct ActiveRunPage {
    rows: Vec<ActiveRunImpact>,
    next_cursor: Option<PageCursor>,
    /// Digest of all active run IDs through the last row on this page.
    traversal_digest: Option<String>,
}

/// Incrementally hashes the canonical JSON array wire used by `run_ids_traversal_digest`.
#[derive(Clone, Debug, Default)]
struct RunIdsTraversalDigestHasher {
    body: Vec<u8>,
}

impl RunIdsTraversalDigestHasher {
    fn new() -> Self {
        Self { body: vec![b'['] }
    }

    fn is_empty(&self) -> bool {
        self.body.len() == 1
    }

    fn update_run_id(&mut self, run_id: &str) {
        if self.body.len() > 1 {
            self.body.push(b',');
        }
        let encoded = serde_json::to_string(run_id).unwrap_or_else(|_| "\"\"".to_string());
        self.body.extend_from_slice(encoded.as_bytes());
    }

    fn digest_hex(&self) -> String {
        let mut canonical = self.body.clone();
        canonical.push(b']');
        sha256_hex(&canonical)
    }

    fn finish(self) -> String {
        self.digest_hex()
    }
}

enum DecodedCursor {
    RegistrationKey {
        created_at: String,
        stable_id: String,
    },
    RunKey {
        run_id: String,
        warning_traversal_digest: Option<String>,
    },
}

struct DecodedDisableAck {
    registration_id: String,
    config_revision: u64,
    active_set_digest: String,
    warning_traversal_digest: String,
}

struct CatalogReadTransaction<'conn> {
    conn: &'conn Connection,
    active: bool,
}

impl<'conn> CatalogReadTransaction<'conn> {
    fn begin(conn: &'conn Connection) -> Result<Self, CatalogPersistenceError> {
        conn.execute_batch("BEGIN DEFERRED").map_err(sqlite_err)?;
        Ok(Self { conn, active: true })
    }

    fn commit(mut self) -> Result<(), CatalogPersistenceError> {
        self.conn.execute_batch("COMMIT").map_err(sqlite_err)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for CatalogReadTransaction<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

fn sqlite_err(source: SqliteError) -> CatalogPersistenceError {
    if source.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        CatalogPersistenceError::Constraint
    } else {
        CatalogPersistenceError::Mapping(source.to_string())
    }
}

fn u64_sql(value: u64) -> Result<i64, CatalogPersistenceError> {
    i64::try_from(value)
        .map_err(|_| CatalogPersistenceError::Mapping(format!("sqlite integer overflow: {value}")))
}

fn map_registration_row(row: &rusqlite::Row<'_>) -> Result<RegistrationRow, SqliteError> {
    Ok(RegistrationRow {
        registration_id: row.get(0)?,
        handle: row.get(1)?,
        enabled: row.get(2)?,
        config_revision: row.get(3)?,
        executable: row.get(4)?,
        argv_json: row.get(5)?,
        working_directory: row.get(6)?,
        timeout_seconds: row.get(7)?,
        created_at: row.get(8)?,
        _updated_at: row.get(9)?,
    })
}

fn load_registration_row(
    conn: &Connection,
    registration_id: &str,
) -> Result<Option<RegistrationRow>, CatalogPersistenceError> {
    conn.query_row(
        "SELECT registration_id, handle, enabled, config_revision, executable, argv_json, \
         working_directory, timeout_seconds, created_at, updated_at
         FROM provider_registrations WHERE registration_id = ?1",
        params![registration_id],
        map_registration_row,
    )
    .optional()
    .map_err(sqlite_err)
}

fn list_filter_clause(filter: &ProviderListFilter) -> (&'static str, ()) {
    match filter {
        ProviderListFilter::Enabled => ("WHERE enabled = 1", ()),
        ProviderListFilter::Tombstoned => ("WHERE enabled = 0", ()),
        ProviderListFilter::All => ("WHERE 1 = 1", ()),
    }
}

fn registrations_filter_fingerprint(
    filter: &ProviderListFilter,
) -> Result<String, CatalogPersistenceError> {
    let value = match filter {
        ProviderListFilter::Enabled => json!({"enabled": true, "tombstoned": false}),
        ProviderListFilter::Tombstoned => json!({"enabled": false, "tombstoned": true}),
        ProviderListFilter::All => json!({}),
    };
    Ok(digest_canonical_json(&value))
}

fn registration_active_runs_filter_fingerprint(
    registration_id: &RegistrationId,
) -> Result<String, CatalogPersistenceError> {
    let value = json!({"registration_id": registration_id.as_str()});
    Ok(digest_canonical_json(&value))
}

fn disable_warnings_filter_fingerprint(
    registration_id: &RegistrationId,
    config_revision: i64,
    snapshot: &ActiveSetSnapshot,
) -> Result<String, CatalogPersistenceError> {
    let value = json!({
        "active_set_digest": snapshot.digest(),
        "config_revision": config_revision,
        "registration_id": registration_id.as_str(),
    });
    Ok(digest_canonical_json(&value))
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
    serde_json::to_string(&canonical_value(value))
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))
        .unwrap_or_default()
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

fn read_integrity_key(
    conn: &Connection,
) -> Result<[u8; INTEGRITY_KEY_BYTE_LENGTH], CatalogPersistenceError> {
    let blob: Vec<u8> = conn
        .query_row(
            &format!("SELECT value FROM {INTEGRATION_METADATA_TABLE} WHERE key = ?1"),
            params![INTEGRITY_KEY_ROW_KEY],
            |row| -> Result<Vec<u8>, SqliteError> { row.get(0) },
        )
        .map_err(|source| match source {
            SqliteError::QueryReturnedNoRows => {
                CatalogPersistenceError::Persistence(PersistenceError::MetadataKeyMissing {
                    key: INTEGRITY_KEY_ROW_KEY,
                })
            }
            other => CatalogPersistenceError::Mapping(other.to_string()),
        })?;
    if blob.len() != INTEGRITY_KEY_BYTE_LENGTH {
        return Err(CatalogPersistenceError::Persistence(
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
    for (l, r) in left.iter().zip(right.iter()) {
        diff |= l ^ r;
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

fn base64url_decode(input: &str) -> Result<Vec<u8>, CatalogPersistenceError> {
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
            return Err(CatalogPersistenceError::InvalidCursor);
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
        return Err(CatalogPersistenceError::InvalidCursor);
    }
    Ok(out)
}

fn mac_input(domain: &[u8], payload: &Value) -> Vec<u8> {
    let mut data = domain.to_vec();
    data.push(0);
    data.extend_from_slice(canonical_json(payload).as_bytes());
    data
}

fn mint_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    domain: &[u8],
    payload: Value,
) -> Result<String, CatalogPersistenceError> {
    let tag = hmac_sha256(integrity_key, &mac_input(domain, &payload));
    let wire = json!({
        "mac": base64url_no_pad(&tag),
        "payload": payload,
    });
    let encoded = base64url_no_pad(canonical_json(&wire).as_bytes());
    PageCursor::parse(&encoded).map_err(|_| CatalogPersistenceError::InvalidCursor)?;
    Ok(encoded)
}

fn decode_integrity_wire(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    domain: &[u8],
    wire: &str,
    invalid: fn() -> CatalogPersistenceError,
) -> Result<Value, CatalogPersistenceError> {
    let decoded = base64url_decode(wire).map_err(|_| invalid())?;
    let parsed: Value = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    if canonical_json(&parsed).as_bytes() != decoded.as_slice() {
        return Err(invalid());
    }
    let Some(wrapper) = parsed.as_object() else {
        return Err(invalid());
    };
    if wrapper.len() != 2 || !wrapper.contains_key("mac") || !wrapper.contains_key("payload") {
        return Err(invalid());
    }
    let mac_b64 = wrapper
        .get("mac")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let payload = wrapper.get("payload").ok_or_else(invalid)?.clone();
    let tag = base64url_decode(mac_b64).map_err(|_| invalid())?;
    if tag.len() != 32 {
        return Err(invalid());
    }
    let expected = hmac_sha256(integrity_key, &mac_input(domain, &payload));
    if !constant_time_eq(&tag, &expected) {
        return Err(invalid());
    }
    Ok(payload)
}

fn decode_cursor(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    wire: &str,
    collection: &str,
    filter_fingerprint: &str,
) -> Result<DecodedCursor, CatalogPersistenceError> {
    let payload = decode_integrity_wire(integrity_key, CURSOR_DOMAIN, wire, || {
        CatalogPersistenceError::InvalidCursor
    })?;
    if payload.get("cursor_version").and_then(Value::as_u64) != Some(1) {
        return Err(CatalogPersistenceError::InvalidCursor);
    }
    if payload.get("collection").and_then(Value::as_str) != Some(collection) {
        return Err(CatalogPersistenceError::InvalidCursor);
    }
    if payload.get("filter_fingerprint").and_then(Value::as_str) != Some(filter_fingerprint) {
        return Err(CatalogPersistenceError::InvalidCursor);
    }
    let last_key = payload
        .get("last_key")
        .ok_or(CatalogPersistenceError::InvalidCursor)?;
    match collection {
        COLLECTION_REGISTRATIONS => {
            let created_at = last_key
                .get("created_at")
                .and_then(Value::as_str)
                .ok_or(CatalogPersistenceError::InvalidCursor)?
                .to_owned();
            let stable_id = last_key
                .get("stable_id")
                .and_then(Value::as_str)
                .ok_or(CatalogPersistenceError::InvalidCursor)?
                .to_owned();
            Ok(DecodedCursor::RegistrationKey {
                created_at,
                stable_id,
            })
        }
        COLLECTION_REGISTRATION_ACTIVE_RUNS | COLLECTION_DISABLE_WARNINGS => {
            let run_id = last_key
                .get("run_id")
                .and_then(Value::as_str)
                .ok_or(CatalogPersistenceError::InvalidCursor)?
                .to_owned();
            let warning_traversal_digest = if collection == COLLECTION_DISABLE_WARNINGS {
                payload
                    .get("warning_traversal_digest")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            } else {
                None
            };
            Ok(DecodedCursor::RunKey {
                run_id,
                warning_traversal_digest,
            })
        }
        _ => Err(CatalogPersistenceError::InvalidCursor),
    }
}

fn stream_active_run_ids_up_to(
    conn: &Connection,
    registration_id: &RegistrationId,
    run_id: &str,
    mut visit: impl FnMut(&str) -> Result<(), CatalogPersistenceError>,
) -> Result<(), CatalogPersistenceError> {
    let mut stmt = conn
        .prepare(
            "SELECT run_id FROM runs
             WHERE registration_id = ?1 AND lifecycle = 'active' AND run_id <= ?2
             ORDER BY run_id ASC",
        )
        .map_err(sqlite_err)?;
    let mut rows = stmt
        .query(params![registration_id.as_str(), run_id])
        .map_err(sqlite_err)?;
    while let Some(row) = rows.next().map_err(sqlite_err)? {
        visit(&row.get::<_, String>(0).map_err(sqlite_err)?)?;
    }
    Ok(())
}

fn verify_warning_traversal_position(
    conn: &Connection,
    registration_id: &RegistrationId,
    run_id: &str,
    warning_traversal_digest: Option<&str>,
) -> Result<RunIdsTraversalDigestHasher, CatalogPersistenceError> {
    let Some(expected) = warning_traversal_digest else {
        return Err(CatalogPersistenceError::InvalidCursor);
    };
    let mut hasher = RunIdsTraversalDigestHasher::new();
    stream_active_run_ids_up_to(conn, registration_id, run_id, |id| {
        hasher.update_run_id(id);
        Ok(())
    })?;
    if hasher.digest_hex() != expected {
        return Err(CatalogPersistenceError::InvalidCursor);
    }
    Ok(hasher)
}

fn mint_registrations_cursor(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    filter_fingerprint: &str,
    created_at: &str,
    stable_id: &str,
) -> Result<PageCursor, CatalogPersistenceError> {
    let payload = json!({
        "collection": COLLECTION_REGISTRATIONS,
        "cursor_version": 1,
        "filter_fingerprint": filter_fingerprint,
        "last_key": {
            "created_at": created_at,
            "stable_id": stable_id,
        },
    });
    PageCursor::parse(mint_integrity_wire(integrity_key, CURSOR_DOMAIN, payload)?)
        .map_err(|_| CatalogPersistenceError::InvalidCursor)
}

fn mint_active_runs_cursor(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    collection: &str,
    filter_fingerprint: &str,
    last_run_id: &str,
    warning_traversal_digest: Option<&str>,
) -> Result<PageCursor, CatalogPersistenceError> {
    let mut payload = json!({
        "collection": collection,
        "cursor_version": 1,
        "filter_fingerprint": filter_fingerprint,
        "last_key": {
            "run_id": last_run_id,
        },
    });
    if collection == COLLECTION_DISABLE_WARNINGS
        && let Some(digest) = warning_traversal_digest
    {
        payload.as_object_mut().unwrap().insert(
            "warning_traversal_digest".to_string(),
            Value::String(digest.to_string()),
        );
    }
    PageCursor::parse(mint_integrity_wire(integrity_key, CURSOR_DOMAIN, payload)?)
        .map_err(|_| CatalogPersistenceError::InvalidCursor)
}

fn first_unreturned_registrations_cursor(
    candidates: &[(String, String, ProviderCatalogRow, usize)],
    count_limit: usize,
    byte_limit: usize,
    filter_fingerprint: &str,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
) -> Result<Option<PageCursor>, CatalogPersistenceError> {
    let mut bytes = 0usize;
    for (selected, (index, (_, _, _, size))) in candidates.iter().enumerate().enumerate() {
        if *size > byte_limit && selected == 0 {
            return Err(CatalogPersistenceError::InvalidCursor);
        }
        if selected == count_limit || bytes.saturating_add(*size) > byte_limit {
            let (created_at, stable_id, _, _) = &candidates[index - 1];
            return mint_registrations_cursor(
                integrity_key,
                filter_fingerprint,
                created_at,
                stable_id,
            )
            .map(Some);
        }
        bytes = bytes.saturating_add(*size);
    }
    Ok(None)
}

fn first_unreturned_active_runs_cursor(
    candidates: &[(String, ActiveRunImpact, usize)],
    count_limit: usize,
    byte_limit: usize,
    collection: &str,
    filter_fingerprint: &str,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    mut traversal_hasher: RunIdsTraversalDigestHasher,
) -> Result<(Option<PageCursor>, RunIdsTraversalDigestHasher), CatalogPersistenceError> {
    let mut bytes = 0usize;
    for (selected, (index, (run_id, _, size))) in candidates.iter().enumerate().enumerate() {
        if *size > byte_limit && selected == 0 {
            return Err(CatalogPersistenceError::InvalidCursor);
        }
        if selected == count_limit || bytes.saturating_add(*size) > byte_limit {
            let last_run_id = &candidates[index - 1].0;
            let digest =
                if collection == COLLECTION_DISABLE_WARNINGS && !traversal_hasher.is_empty() {
                    Some(traversal_hasher.digest_hex())
                } else {
                    None
                };
            let cursor = mint_active_runs_cursor(
                integrity_key,
                collection,
                filter_fingerprint,
                last_run_id,
                digest.as_deref(),
            )?;
            return Ok((Some(cursor), traversal_hasher));
        }
        traversal_hasher.update_run_id(run_id);
        bytes = bytes.saturating_add(*size);
    }
    Ok((None, traversal_hasher))
}

fn page_active_runs(
    conn: &Connection,
    registration_id: &RegistrationId,
    request: &PageRequest<()>,
    collection: &str,
    filter_fingerprint: &str,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
) -> Result<ActiveRunPage, CatalogPersistenceError> {
    let mut traversal_hasher = RunIdsTraversalDigestHasher::new();
    let after = match request.cursor() {
        Some(cursor) => {
            let decoded = decode_cursor(
                integrity_key,
                cursor.as_str(),
                collection,
                filter_fingerprint,
            )?;
            match decoded {
                DecodedCursor::RunKey {
                    run_id,
                    warning_traversal_digest,
                } => {
                    if collection == COLLECTION_DISABLE_WARNINGS {
                        traversal_hasher = verify_warning_traversal_position(
                            conn,
                            registration_id,
                            &run_id,
                            warning_traversal_digest.as_deref(),
                        )?;
                    }
                    Some(run_id)
                }
                DecodedCursor::RegistrationKey { .. } => {
                    return Err(CatalogPersistenceError::InvalidCursor);
                }
            }
        }
        None => None,
    };
    let mut sql = String::from(
        "SELECT run_id, graph_revision FROM runs
         WHERE registration_id = ?1 AND lifecycle = 'active'",
    );
    if after.is_some() {
        sql.push_str(" AND run_id > ?2");
    }
    sql.push_str(" ORDER BY run_id ASC");
    let fetch = usize::from(request.limit()) + 1;
    sql.push_str(&format!(" LIMIT {fetch}"));
    let raw_rows: Vec<(String, String)> = match after {
        Some(run_id) => {
            let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
            stmt.query_map(
                params![registration_id.as_str(), run_id],
                |row| -> Result<(String, String), SqliteError> { Ok((row.get(0)?, row.get(1)?)) },
            )
            .map_err(sqlite_err)?
            .collect::<Result<Vec<(String, String)>, SqliteError>>()
            .map_err(sqlite_err)?
        }
        None => {
            let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
            stmt.query_map(
                params![registration_id.as_str()],
                |row| -> Result<(String, String), SqliteError> { Ok((row.get(0)?, row.get(1)?)) },
            )
            .map_err(sqlite_err)?
            .collect::<Result<Vec<(String, String)>, SqliteError>>()
            .map_err(sqlite_err)?
        }
    };
    let candidates: Vec<(String, ActiveRunImpact, usize)> = raw_rows
        .into_iter()
        .map(|(run_id, graph_revision)| {
            let impact = ActiveRunImpact {
                run_id: RunId::parse(&run_id)
                    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?,
                graph_revision: GraphRevision::parse(&graph_revision)
                    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?,
            };
            let bytes = encoded_active_run_bytes(&impact);
            Ok((run_id, impact, bytes))
        })
        .collect::<Result<Vec<(String, ActiveRunImpact, usize)>, CatalogPersistenceError>>()?;
    let count_limit = usize::from(request.limit());
    let byte_limit = request.byte_limit();
    let (next_cursor, traversal_hasher) = first_unreturned_active_runs_cursor(
        &candidates,
        count_limit,
        byte_limit,
        collection,
        filter_fingerprint,
        integrity_key,
        traversal_hasher,
    )?;
    let page = bounded_page(
        candidates
            .iter()
            .map(|(_, impact, size)| (impact.clone(), *size)),
        count_limit,
        byte_limit,
        next_cursor,
    )
    .map_err(|_| CatalogPersistenceError::InvalidCursor)?;
    let traversal_digest = if collection == COLLECTION_DISABLE_WARNINGS {
        Some(traversal_hasher.digest_hex())
    } else {
        None
    };
    Ok(ActiveRunPage {
        rows: page.rows,
        next_cursor: page.next_cursor,
        traversal_digest,
    })
}

#[cfg(test)]
fn run_ids_traversal_digest(run_ids: &[String]) -> String {
    let mut hasher = RunIdsTraversalDigestHasher::new();
    for run_id in run_ids {
        hasher.update_run_id(run_id);
    }
    hasher.finish()
}

fn sorted_active_run_digest(
    conn: &Connection,
    registration_id: &RegistrationId,
) -> Result<String, CatalogPersistenceError> {
    let mut hasher = RunIdsTraversalDigestHasher::new();
    let mut stmt = conn
        .prepare(
            "SELECT run_id FROM runs
             WHERE registration_id = ?1 AND lifecycle = 'active'
             ORDER BY run_id ASC",
        )
        .map_err(sqlite_err)?;
    let mut rows = stmt
        .query(params![registration_id.as_str()])
        .map_err(sqlite_err)?;
    while let Some(row) = rows.next().map_err(sqlite_err)? {
        hasher.update_run_id(&row.get::<_, String>(0).map_err(sqlite_err)?);
    }
    Ok(hasher.finish())
}

fn compute_active_set_snapshot(
    conn: &Connection,
    registration_id: &RegistrationId,
    config_revision: i64,
) -> Result<ActiveSetSnapshot, CatalogPersistenceError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE registration_id = ?1 AND lifecycle = 'active'",
            params![registration_id.as_str()],
            |row| -> Result<i64, SqliteError> { row.get(0) },
        )
        .map_err(sqlite_err)?;
    let digest = sorted_active_run_digest(conn, registration_id)?;
    ActiveSetSnapshot::new(count as u64, digest, config_revision as u64)
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))
}

fn mint_disable_acknowledgement(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    registration_id: &RegistrationId,
    config_revision: i64,
    snapshot: &ActiveSetSnapshot,
    warning_traversal_digest: &str,
) -> Result<DisableAcknowledgement, CatalogPersistenceError> {
    let payload = json!({
        "active_set_digest": snapshot.digest(),
        "config_revision": config_revision,
        "registration_id": registration_id.as_str(),
        "token_kind": DISABLE_ACK_KIND,
        "token_version": 1,
        "warning_traversal_digest": warning_traversal_digest,
    });
    DisableAcknowledgement::parse(mint_integrity_wire(
        integrity_key,
        DISABLE_ACK_DOMAIN,
        payload,
    )?)
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))
}

fn decode_disable_ack(
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    acknowledgement: &DisableAcknowledgement,
) -> Result<DecodedDisableAck, CatalogPersistenceError> {
    let payload = decode_integrity_wire(
        integrity_key,
        DISABLE_ACK_DOMAIN,
        acknowledgement.as_str(),
        || CatalogPersistenceError::InvalidAck,
    )?;
    if payload.get("token_version").and_then(Value::as_u64) != Some(1) {
        return Err(CatalogPersistenceError::InvalidAck);
    }
    if payload.get("token_kind").and_then(Value::as_str) != Some(DISABLE_ACK_KIND) {
        return Err(CatalogPersistenceError::InvalidAck);
    }
    Ok(DecodedDisableAck {
        registration_id: payload
            .get("registration_id")
            .and_then(Value::as_str)
            .ok_or(CatalogPersistenceError::InvalidAck)?
            .to_owned(),
        config_revision: payload
            .get("config_revision")
            .and_then(Value::as_u64)
            .ok_or(CatalogPersistenceError::InvalidAck)?,
        active_set_digest: payload
            .get("active_set_digest")
            .and_then(Value::as_str)
            .ok_or(CatalogPersistenceError::InvalidAck)?
            .to_owned(),
        warning_traversal_digest: payload
            .get("warning_traversal_digest")
            .and_then(Value::as_str)
            .ok_or(CatalogPersistenceError::InvalidAck)?
            .to_owned(),
    })
}

fn row_to_config(row: &RegistrationRow) -> Result<ProviderConfig, CatalogPersistenceError> {
    let argv: Vec<String> = serde_json::from_str(&row.argv_json)
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    ProviderConfig::new(
        row.executable.clone(),
        argv,
        row.working_directory.clone(),
        row.timeout_seconds as u64,
    )
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))
}

fn row_to_registration(
    row: &RegistrationRow,
) -> Result<ProviderRegistration, CatalogPersistenceError> {
    let id = RegistrationId::parse(&row.registration_id)
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    let handle = row
        .handle
        .as_ref()
        .map(ProviderHandle::parse)
        .transpose()
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    ProviderRegistration::restore(id, handle, row.config_revision as u64, row.enabled != 0)
        .ok_or_else(|| CatalogPersistenceError::Mapping("registration invariant".into()))
}

fn row_to_resolved(
    row: &RegistrationRow,
) -> Result<ResolvedProviderConfig, CatalogPersistenceError> {
    let registration_id = RegistrationId::parse(&row.registration_id)
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    let handle = ProviderHandle::parse(
        row.handle
            .as_deref()
            .ok_or(CatalogPersistenceError::Disabled)?,
    )
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    let config = row_to_config(row)?;
    ResolvedProviderConfig::new(registration_id, handle, row.config_revision as u64, config)
        .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))
}

fn row_to_catalog_row(
    row: &RegistrationRow,
) -> Result<ProviderCatalogRow, CatalogPersistenceError> {
    Ok(ProviderCatalogRow {
        registration: row_to_registration(row)?,
        config: if row.enabled != 0 {
            Some(row_to_config(row)?)
        } else {
            None
        },
    })
}

fn catalog_row_list_item_json(row: &ProviderCatalogRow) -> Value {
    json!({
        "registration_id": row.registration.id().as_str(),
        "handle": row.registration.handle().map(|handle| handle.as_str()),
        "enabled": row.registration.enabled(),
        "config_revision": row.registration.config_revision(),
        "config": row.config.as_ref().map(|config| {
            json!({
                "executable": config.executable(),
                "argv": config
                    .argv()
                    .iter()
                    .map(|arg| arg.as_str())
                    .collect::<Vec<_>>(),
                "working_directory": config.working_directory(),
                "timeout_seconds": config.timeout_seconds(),
            })
        }),
    })
}

fn active_run_list_item_json(row: &ActiveRunImpact) -> Value {
    json!({
        "run_id": row.run_id.as_str(),
        "graph_revision": row.graph_revision.as_str(),
    })
}

fn encoded_catalog_row_bytes(row: &ProviderCatalogRow) -> usize {
    serde_json::to_vec(&catalog_row_list_item_json(row))
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn encoded_active_run_bytes(row: &ActiveRunImpact) -> usize {
    serde_json::to_vec(&active_run_list_item_json(row))
        .expect("active run list item JSON is always serializable")
        .len()
}

fn now_rfc3339_ms() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn mutate_in_transaction(
    conn: &Connection,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    command: CatalogMutation,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    match command {
        CatalogMutation::Add {
            registration_id,
            handle,
            config,
        } => mutate_add(conn, registration_id, handle, config),
        CatalogMutation::Update {
            registration_id,
            expected_config_revision,
            config,
        } => mutate_update(
            conn,
            &registration_id,
            expected_config_revision,
            config,
            trace,
        ),
        CatalogMutation::Rename {
            registration_id,
            expected_config_revision,
            handle,
        } => mutate_rename(
            conn,
            &registration_id,
            expected_config_revision,
            handle,
            trace,
        ),
        CatalogMutation::Disable {
            registration_id,
            expected,
            acknowledgement,
        } => mutate_disable(
            conn,
            integrity_key,
            &registration_id,
            expected,
            acknowledgement,
            trace,
        ),
        CatalogMutation::Restore {
            registration_id,
            expected_config_revision,
            handle,
            config,
        } => mutate_restore(
            conn,
            registration_id,
            expected_config_revision,
            handle,
            config,
            trace,
        ),
    }
}

fn mutate_add(
    conn: &Connection,
    registration_id: RegistrationId,
    handle: ProviderHandle,
    config: ProviderConfig,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    if load_registration_row(conn, registration_id.as_str())?.is_some() {
        return Err(CatalogPersistenceError::Constraint);
    }
    let now = now_rfc3339_ms();
    let argv_json = serde_json::to_string(
        &config
            .argv()
            .iter()
            .map(|arg| arg.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    conn.execute(
        "INSERT INTO provider_registrations (
            registration_id, handle, enabled, config_revision, executable, argv_json,
            working_directory, timeout_seconds, created_at, updated_at
        ) VALUES (?1, ?2, 1, 1, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            registration_id.as_str(),
            handle.as_str(),
            config.executable(),
            argv_json,
            config.working_directory(),
            u64_sql(config.timeout_seconds())?,
            now,
        ],
    )
    .map_err(|error| map_handle_constraint(error, CatalogMutationKind::Add))?;
    let row = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    Ok((
        CatalogMutationResult {
            registration: row_to_registration(&row)?,
            affected_active_runs: 0,
            impact_cursor: None,
        },
        catalog_registration_expectation(&row),
    ))
}

fn catalog_registration_expectation(row: &RegistrationRow) -> CatalogRegistrationExpectation {
    CatalogRegistrationExpectation {
        registration_id: row.registration_id.clone(),
        handle: row.handle.clone(),
        enabled: row.enabled != 0,
        config_revision: row.config_revision as u64,
        executable: row.executable.clone(),
        argv_json: row.argv_json.clone(),
        working_directory: row.working_directory.clone(),
        timeout_seconds: row.timeout_seconds as u64,
        should_exist: true,
    }
}

enum CatalogMutationKind {
    Add,
    Rename,
    Restore,
}

fn map_handle_constraint(error: SqliteError, kind: CatalogMutationKind) -> CatalogPersistenceError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        match kind {
            CatalogMutationKind::Add | CatalogMutationKind::Rename => {
                CatalogPersistenceError::Duplicate
            }
            CatalogMutationKind::Restore => CatalogPersistenceError::Occupied,
        }
    } else {
        CatalogPersistenceError::Mapping(error.to_string())
    }
}

fn require_enabled_revision(
    row: &RegistrationRow,
    expected_config_revision: u64,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(), CatalogPersistenceError> {
    if let Some(session) = trace {
        session.version_check_catalog(expected_config_revision);
    }
    if row.enabled == 0 {
        return Err(CatalogPersistenceError::Disabled);
    }
    if row.config_revision as u64 != expected_config_revision {
        return Err(CatalogPersistenceError::Stale);
    }
    Ok(())
}

fn require_tombstoned_revision(
    row: &RegistrationRow,
    expected_config_revision: u64,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(), CatalogPersistenceError> {
    if let Some(session) = trace {
        session.version_check_catalog(expected_config_revision);
    }
    if row.enabled != 0 {
        return Err(CatalogPersistenceError::Stale);
    }
    if row.config_revision as u64 != expected_config_revision {
        return Err(CatalogPersistenceError::Stale);
    }
    Ok(())
}

fn mutate_update(
    conn: &Connection,
    registration_id: &RegistrationId,
    expected_config_revision: u64,
    config: ProviderConfig,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    if let Some(session) = trace {
        session.version_check_catalog(expected_config_revision);
    }
    let row = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    require_enabled_revision(&row, expected_config_revision, None)?;
    let affected = count_active_runs(conn, registration_id)?;
    let argv_json = serde_json::to_string(
        &config
            .argv()
            .iter()
            .map(|arg| arg.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    let next_revision = row.config_revision + 1;
    conn.execute(
        "UPDATE provider_registrations
         SET executable = ?1, argv_json = ?2, working_directory = ?3, timeout_seconds = ?4,
             config_revision = ?5, updated_at = ?6
         WHERE registration_id = ?7 AND enabled = 1 AND config_revision = ?8",
        params![
            config.executable(),
            argv_json,
            config.working_directory(),
            u64_sql(config.timeout_seconds())?,
            next_revision,
            now_rfc3339_ms(),
            registration_id.as_str(),
            u64_sql(expected_config_revision)?,
        ],
    )
    .map_err(sqlite_err)?;
    let updated = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    if updated.config_revision as u64 != expected_config_revision + 1 {
        return Err(CatalogPersistenceError::Stale);
    }
    Ok((
        CatalogMutationResult {
            registration: row_to_registration(&updated)?,
            affected_active_runs: affected,
            impact_cursor: impact_cursor_for(conn, registration_id, affected > 0)?,
        },
        catalog_registration_expectation(&updated),
    ))
}

fn mutate_rename(
    conn: &Connection,
    registration_id: &RegistrationId,
    expected_config_revision: u64,
    handle: ProviderHandle,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    let row = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    require_enabled_revision(&row, expected_config_revision, trace)?;
    conn.execute(
        "UPDATE provider_registrations
         SET handle = ?1, updated_at = ?2
         WHERE registration_id = ?3 AND enabled = 1 AND config_revision = ?4",
        params![
            handle.as_str(),
            now_rfc3339_ms(),
            registration_id.as_str(),
            u64_sql(expected_config_revision)?,
        ],
    )
    .map_err(|error| map_handle_constraint(error, CatalogMutationKind::Rename))?;
    let updated = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    Ok((
        CatalogMutationResult {
            registration: row_to_registration(&updated)?,
            affected_active_runs: 0,
            impact_cursor: None,
        },
        catalog_registration_expectation(&updated),
    ))
}

fn mutate_disable(
    conn: &Connection,
    integrity_key: &[u8; INTEGRITY_KEY_BYTE_LENGTH],
    registration_id: &RegistrationId,
    expected: ActiveSetSnapshot,
    acknowledgement: DisableAcknowledgement,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    let row = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    if row.enabled == 0 {
        return Err(CatalogPersistenceError::Disabled);
    }
    if let Some(session) = trace {
        session.version_check_catalog(expected.config_revision());
    }
    let decoded = decode_disable_ack(integrity_key, &acknowledgement)?;
    if decoded.registration_id != registration_id.as_str() {
        return Err(CatalogPersistenceError::InvalidAck);
    }
    let current = compute_active_set_snapshot(conn, registration_id, row.config_revision)?;
    if current.count() != expected.count()
        || current.digest() != expected.digest()
        || current.config_revision() != expected.config_revision()
    {
        return Err(CatalogPersistenceError::Stale);
    }
    if current.config_revision() != decoded.config_revision
        || current.digest() != decoded.active_set_digest.as_str()
    {
        return Err(CatalogPersistenceError::Stale);
    }
    let recomputed_traversal = sorted_active_run_digest(conn, registration_id)?;
    if recomputed_traversal != decoded.warning_traversal_digest {
        return Err(CatalogPersistenceError::Stale);
    }
    let affected = current.count();
    let next_revision = row.config_revision + 1;
    conn.execute(
        "UPDATE provider_registrations
         SET enabled = 0, handle = NULL, config_revision = ?1, updated_at = ?2
         WHERE registration_id = ?3 AND enabled = 1 AND config_revision = ?4",
        params![
            next_revision,
            now_rfc3339_ms(),
            registration_id.as_str(),
            row.config_revision,
        ],
    )
    .map_err(sqlite_err)?;
    let updated = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    Ok((
        CatalogMutationResult {
            registration: row_to_registration(&updated)?,
            affected_active_runs: affected,
            impact_cursor: impact_cursor_for(conn, registration_id, affected > 0)?,
        },
        catalog_registration_expectation(&updated),
    ))
}

fn mutate_restore(
    conn: &Connection,
    registration_id: RegistrationId,
    expected_config_revision: u64,
    handle: ProviderHandle,
    config: ProviderConfig,
    trace: Option<&WriteTraceSession<'_>>,
) -> Result<(CatalogMutationResult, CatalogRegistrationExpectation), CatalogPersistenceError> {
    let row = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    require_tombstoned_revision(&row, expected_config_revision, trace)?;
    let affected = count_active_runs(conn, &registration_id)?;
    let argv_json = serde_json::to_string(
        &config
            .argv()
            .iter()
            .map(|arg| arg.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| CatalogPersistenceError::Mapping(error.to_string()))?;
    let next_revision = row.config_revision + 1;
    conn.execute(
        "UPDATE provider_registrations
         SET handle = ?1, enabled = 1, executable = ?2, argv_json = ?3, working_directory = ?4,
             timeout_seconds = ?5, config_revision = ?6, updated_at = ?7
         WHERE registration_id = ?8 AND enabled = 0 AND config_revision = ?9",
        params![
            handle.as_str(),
            config.executable(),
            argv_json,
            config.working_directory(),
            u64_sql(config.timeout_seconds())?,
            next_revision,
            now_rfc3339_ms(),
            registration_id.as_str(),
            u64_sql(expected_config_revision)?,
        ],
    )
    .map_err(|error| map_handle_constraint(error, CatalogMutationKind::Restore))?;
    let updated = load_registration_row(conn, registration_id.as_str())?
        .ok_or(CatalogPersistenceError::NotFound)?;
    if updated.config_revision as u64 != expected_config_revision + 1 {
        return Err(CatalogPersistenceError::Stale);
    }
    Ok((
        CatalogMutationResult {
            registration: row_to_registration(&updated)?,
            affected_active_runs: affected,
            impact_cursor: impact_cursor_for(conn, &registration_id, affected > 0)?,
        },
        catalog_registration_expectation(&updated),
    ))
}

fn count_active_runs(
    conn: &Connection,
    registration_id: &RegistrationId,
) -> Result<u64, CatalogPersistenceError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE registration_id = ?1 AND lifecycle = 'active'",
            params![registration_id.as_str()],
            |row| -> Result<i64, SqliteError> { row.get(0) },
        )
        .map_err(sqlite_err)?;
    Ok(count as u64)
}

fn impact_cursor_for(
    _conn: &Connection,
    _registration_id: &RegistrationId,
    has_impact: bool,
) -> Result<Option<PageCursor>, CatalogPersistenceError> {
    // Cursorless start preserves first-unreturned semantics: an exclusive last_key
    // cursor minted from the first active run would skip that run on continuation.
    let _ = has_impact;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::sync::{Barrier, Mutex, MutexGuard, OnceLock};
    use std::thread;

    use loop_engine_core::capabilities::PageRequest;
    use loop_engine_core::capabilities::provider_catalog::ProviderCatalog;
    use loop_engine_core::model::bounded::{
        COLLECTION_PAGE_DATA_BUDGET_BYTES, COLLECTION_PAGE_DEFAULT_COUNT,
    };
    use loop_engine_core::model::ids::{GraphRevision, ProviderHandle, RegistrationId, RunId};
    use rusqlite_migration::{M, Migrations};
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::error::PersistenceError;
    use crate::persistence::sqlite::open_at;
    use crate::persistence::traced::OptionalTraceSink;
    use crate::persistence::traced::test_support::{event_names, read_events, test_sink};

    fn commit_io_test_lock() -> MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn test_catalog() -> (MutexGuard<'static, ()>, TempDir, SqliteProviderCatalog) {
        let guard = commit_io_test_lock();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        let migrations = Migrations::new(vec![M::up(include_str!(
            "../../migrations/0001_initial.sql"
        ))]);
        open_at(&path, &migrations, 1).unwrap();
        (guard, dir, SqliteProviderCatalog::new(path))
    }

    fn sample_config() -> ProviderConfig {
        ProviderConfig::new("/bin/provider", vec!["--flag".into()], "/work", 60).unwrap()
    }

    fn sample_graph_revision() -> &'static str {
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }

    fn collect_active_run_ids(
        catalog: &SqliteProviderCatalog,
        registration_id: &RegistrationId,
        count_limit: u16,
        byte_limit: usize,
    ) -> Vec<String> {
        let mut cursor: Option<PageCursor> = None;
        let mut ids = Vec::new();
        loop {
            let page = catalog
                .active_run_impact(
                    registration_id,
                    &PageRequest::new(count_limit, byte_limit, cursor, ()).unwrap(),
                )
                .unwrap();
            ids.extend(
                page.rows
                    .iter()
                    .map(|impact| impact.run_id.as_str().to_owned()),
            );
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        ids
    }

    fn collect_disable_warning_run_ids(
        catalog: &SqliteProviderCatalog,
        registration_id: &RegistrationId,
        count_limit: u16,
        byte_limit: usize,
    ) -> (Vec<String>, Option<DisableAcknowledgement>) {
        let mut cursor: Option<PageCursor> = None;
        let mut ids = Vec::new();
        let acknowledgement = loop {
            let page = catalog
                .disable_warnings_page(
                    registration_id,
                    &PageRequest::new(count_limit, byte_limit, cursor, ()).unwrap(),
                )
                .unwrap();
            if page.impacts.next_cursor.is_some() {
                assert!(page.acknowledgement.is_none());
            }
            ids.extend(
                page.impacts
                    .rows
                    .iter()
                    .map(|impact| impact.run_id.as_str().to_owned()),
            );
            if page.impacts.next_cursor.is_none() {
                break page.acknowledgement;
            }
            cursor = page.impacts.next_cursor;
        };
        (ids, acknowledgement)
    }

    #[test]
    fn catalog_byte_stop_resumes_first_unreturned_row() {
        let (_guard, _dir, catalog) = test_catalog();
        let first_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417a").unwrap();
        let second_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417b").unwrap();
        let third_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417c").unwrap();
        for (registration_id, handle) in [
            (first_id.clone(), "provider-a"),
            (second_id.clone(), "provider-b"),
            (third_id.clone(), "provider-c"),
        ] {
            catalog
                .mutate(CatalogMutation::Add {
                    registration_id,
                    handle: ProviderHandle::parse(handle).unwrap(),
                    config: sample_config(),
                })
                .unwrap();
        }
        let sample = catalog
            .list(
                &PageRequest::new(
                    100,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    None,
                    ProviderListFilter::Enabled,
                )
                .unwrap(),
            )
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.registration.id() == &first_id)
            .unwrap();
        let row_bytes = encoded_catalog_row_bytes(&sample);
        let mut cursor: Option<PageCursor> = None;
        let mut listed = Vec::new();
        loop {
            let page = catalog
                .list(
                    &PageRequest::new(10, row_bytes, cursor, ProviderListFilter::Enabled).unwrap(),
                )
                .unwrap();
            assert_eq!(page.rows.len(), 1);
            listed.push(page.rows[0].registration.id().clone());
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(listed.len(), 3);
        let mut sorted = listed.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
        assert!(listed.contains(&first_id));
        assert!(listed.contains(&second_id));
        assert!(listed.contains(&third_id));
    }

    #[test]
    fn disable_warning_union_traversal_no_early_ack() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4181").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("disable-byte-stop").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let conn = catalog.connect_write().unwrap();
        let run_ids = [
            "019f6e88-b403-73a6-89f9-ebfe668b4182",
            "019f6e88-b403-73a6-89f9-ebfe668b4183",
            "019f6e88-b403-73a6-89f9-ebfe668b4184",
        ];
        for run_id in run_ids {
            insert_active_run(
                &conn,
                registration_id.as_str(),
                run_id,
                sample_graph_revision(),
            );
        }
        let sample = ActiveRunImpact {
            run_id: RunId::parse(run_ids[0]).unwrap(),
            graph_revision: GraphRevision::parse(sample_graph_revision()).unwrap(),
        };
        let row_bytes = encoded_active_run_bytes(&sample);
        let (shown, acknowledgement) =
            collect_disable_warning_run_ids(&catalog, &registration_id, 2, row_bytes);
        assert_eq!(shown, run_ids.to_vec());
        let snapshot = catalog.active_set_snapshot(&registration_id).unwrap();
        let ack = acknowledgement.expect("final disable page must mint acknowledgement");
        catalog
            .mutate(CatalogMutation::Disable {
                registration_id,
                expected: snapshot,
                acknowledgement: ack,
            })
            .unwrap();
    }

    fn insert_active_run(
        conn: &Connection,
        registration_id: &str,
        run_id: &str,
        graph_revision: &str,
    ) {
        conn.execute(
            "INSERT INTO runs (
                run_id, registration_id, config_revision_at_create, current_state, lifecycle,
                workflow_state_version, lifecycle_version, label_version, graph_revision,
                canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
            ) VALUES (?1, ?2, 1, 'start', 'active', 1, 1, 1, ?3, 1, '{}', '{}', '2026-07-17T12:00:00.000Z')",
            params![run_id, registration_id, graph_revision],
        )
        .unwrap();
    }

    #[test]
    fn impact_cursor_update_includes_all_active_runs_one() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4191").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("impact-one").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let conn = catalog.connect_write().unwrap();
        let run_id = "019f6e88-b403-73a6-89f9-ebfe668b4192";
        insert_active_run(
            &conn,
            registration_id.as_str(),
            run_id,
            sample_graph_revision(),
        );
        let mutation = catalog
            .mutate(CatalogMutation::Update {
                registration_id: registration_id.clone(),
                expected_config_revision: 1,
                config: sample_config(),
            })
            .unwrap();
        assert_eq!(mutation.affected_active_runs, 1);
        assert!(mutation.impact_cursor.is_none());
        let listed = collect_active_run_ids(
            &catalog,
            &registration_id,
            COLLECTION_PAGE_DEFAULT_COUNT,
            COLLECTION_PAGE_DATA_BUDGET_BYTES,
        );
        assert_eq!(listed, vec![run_id.to_owned()]);
    }

    #[test]
    fn impact_cursor_update_includes_all_active_runs_many() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4193").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("impact-many").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let conn = catalog.connect_write().unwrap();
        let run_ids = [
            "019f6e88-b403-73a6-89f9-ebfe668b4194",
            "019f6e88-b403-73a6-89f9-ebfe668b4195",
            "019f6e88-b403-73a6-89f9-ebfe668b4196",
        ];
        for run_id in run_ids {
            insert_active_run(
                &conn,
                registration_id.as_str(),
                run_id,
                sample_graph_revision(),
            );
        }
        let mutation = catalog
            .mutate(CatalogMutation::Update {
                registration_id: registration_id.clone(),
                expected_config_revision: 1,
                config: sample_config(),
            })
            .unwrap();
        assert_eq!(mutation.affected_active_runs, 3);
        assert!(mutation.impact_cursor.is_none());
        let sample = ActiveRunImpact {
            run_id: RunId::parse(run_ids[0]).unwrap(),
            graph_revision: GraphRevision::parse(sample_graph_revision()).unwrap(),
        };
        let row_bytes = encoded_active_run_bytes(&sample);
        let listed = collect_active_run_ids(&catalog, &registration_id, 1, row_bytes);
        assert_eq!(listed, run_ids.to_vec());
    }

    #[test]
    fn add_resolve_update_rename_disable_restore_round_trip() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417e").unwrap();
        let handle = ProviderHandle::parse("provider-a").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: handle.clone(),
                config: sample_config(),
            })
            .unwrap();
        let resolved = catalog.resolve_enabled(&registration_id).unwrap();
        assert_eq!(resolved.config_revision(), 1);
        assert_eq!(resolved.handle().as_str(), "provider-a");
        let by_handle = catalog.resolve_handle(&handle).unwrap();
        assert!(by_handle.registration.enabled());
        let updated_config = ProviderConfig::new("/bin/provider2", vec![], "/work2", 120).unwrap();
        catalog
            .mutate(CatalogMutation::Update {
                registration_id: registration_id.clone(),
                expected_config_revision: 1,
                config: updated_config,
            })
            .unwrap();
        assert_eq!(
            catalog
                .resolve_enabled(&registration_id)
                .unwrap()
                .config_revision(),
            2
        );
        let renamed = ProviderHandle::parse("provider-b").unwrap();
        catalog
            .mutate(CatalogMutation::Rename {
                registration_id: registration_id.clone(),
                expected_config_revision: 2,
                handle: renamed.clone(),
            })
            .unwrap();
        assert_eq!(
            catalog
                .resolve_enabled(&registration_id)
                .unwrap()
                .config_revision(),
            2
        );
        assert_eq!(
            catalog
                .resolve_enabled(&registration_id)
                .unwrap()
                .handle()
                .as_str(),
            "provider-b"
        );
        let conn = catalog.connect_write().unwrap();
        insert_active_run(
            &conn,
            registration_id.as_str(),
            "019f6e88-b403-73a6-89f9-ebfe668b417f",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let request = PageRequest::new(
            COLLECTION_PAGE_DEFAULT_COUNT,
            COLLECTION_PAGE_DATA_BUDGET_BYTES,
            None,
            (),
        )
        .unwrap();
        let warning = catalog
            .disable_warnings_page(&registration_id, &request)
            .unwrap();
        assert!(warning.acknowledgement.is_some());
        catalog
            .mutate(CatalogMutation::Disable {
                registration_id: registration_id.clone(),
                expected: warning.snapshot,
                acknowledgement: warning.acknowledgement.unwrap(),
            })
            .unwrap();
        assert!(catalog.resolve_enabled(&registration_id).is_err());
        let restored_handle = ProviderHandle::parse("provider-restored").unwrap();
        let tombstone = catalog
            .list(
                &PageRequest::new(
                    100,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    None,
                    ProviderListFilter::Tombstoned,
                )
                .unwrap(),
            )
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.registration.id() == &registration_id)
            .unwrap();
        catalog
            .mutate(CatalogMutation::Restore {
                registration_id: registration_id.clone(),
                expected_config_revision: tombstone.registration.config_revision(),
                handle: restored_handle,
                config: sample_config(),
            })
            .unwrap();
        assert!(catalog.resolve_enabled(&registration_id).is_ok());
    }

    #[test]
    fn concurrent_handle_claims_are_exclusive() {
        use std::sync::Arc;

        let (_guard, _dir, catalog) = test_catalog();
        let path = catalog.path.clone();
        let barrier = Arc::new(Barrier::new(2));
        let handle = ProviderHandle::parse("contended").unwrap();
        let tombstone_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417b").unwrap();
        let claimant_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417c").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: tombstone_id.clone(),
                handle: handle.clone(),
                config: sample_config(),
            })
            .unwrap();
        let request = PageRequest::new(100, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let warning = catalog
            .disable_warnings_page(&tombstone_id, &request)
            .unwrap();
        catalog
            .mutate(CatalogMutation::Disable {
                registration_id: tombstone_id.clone(),
                expected: warning.snapshot,
                acknowledgement: warning.acknowledgement.unwrap(),
            })
            .unwrap();
        let tombstone = catalog
            .list(
                &PageRequest::new(
                    100,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    None,
                    ProviderListFilter::Tombstoned,
                )
                .unwrap(),
            )
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.registration.id() == &tombstone_id)
            .unwrap();
        let barrier_a = Arc::clone(&barrier);
        let path_a = path.clone();
        let handle_a = handle.clone();
        let restore_revision = tombstone.registration.config_revision();
        let restore = thread::spawn(move || {
            barrier_a.wait();
            let catalog = SqliteProviderCatalog::new(path_a);
            catalog.mutate(CatalogMutation::Restore {
                registration_id: tombstone_id,
                expected_config_revision: restore_revision,
                handle: handle_a,
                config: sample_config(),
            })
        });
        let barrier_b = Arc::clone(&barrier);
        let add = thread::spawn(move || {
            barrier_b.wait();
            let catalog = SqliteProviderCatalog::new(path);
            catalog.mutate(CatalogMutation::Add {
                registration_id: claimant_id,
                handle,
                config: sample_config(),
            })
        });
        let outcomes = [restore.join().unwrap(), add.join().unwrap()];
        assert!(
            outcomes
                .iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(CatalogPersistenceError::Duplicate | CatalogPersistenceError::Occupied)
                    )
                })
                .count()
                == 1
        );
        assert!(outcomes.iter().any(Result::is_ok));
    }

    #[test]
    fn tombstone_releases_handle_for_reuse() {
        let (_guard, _dir, catalog) = test_catalog();
        let first_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417c").unwrap();
        let second_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417d").unwrap();
        let handle = ProviderHandle::parse("shared-handle").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: first_id.clone(),
                handle: handle.clone(),
                config: sample_config(),
            })
            .unwrap();
        let request = PageRequest::new(100, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let warning = catalog.disable_warnings_page(&first_id, &request).unwrap();
        catalog
            .mutate(CatalogMutation::Disable {
                registration_id: first_id,
                expected: warning.snapshot,
                acknowledgement: warning.acknowledgement.unwrap(),
            })
            .unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: second_id.clone(),
                handle: handle.clone(),
                config: sample_config(),
            })
            .unwrap();
        assert!(catalog.resolve_handle(&handle).unwrap().registration.id() == &second_id);
    }

    #[test]
    fn stale_disable_acknowledgement_is_rejected() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4180").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("disable-me").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let request = PageRequest::new(100, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ()).unwrap();
        let warning = catalog
            .disable_warnings_page(&registration_id, &request)
            .unwrap();
        let ack = warning.acknowledgement.unwrap();
        catalog
            .mutate(CatalogMutation::Update {
                registration_id: registration_id.clone(),
                expected_config_revision: 1,
                config: sample_config(),
            })
            .unwrap();
        let err = catalog
            .mutate(CatalogMutation::Disable {
                registration_id,
                expected: warning.snapshot,
                acknowledgement: ack,
            })
            .unwrap_err();
        assert!(matches!(err, CatalogPersistenceError::Stale));
    }

    #[test]
    fn catalog_row_encoded_bytes_match_serde_json_with_special_argv() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b41a1").unwrap();
        let special_argv = vec!["\"quoted\"".into(), "back\\slash".into()];
        let config =
            ProviderConfig::new("/bin/echo", special_argv.clone(), "/work/dir\"\\n", 90).unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("special-argv").unwrap(),
                config,
            })
            .unwrap();
        let row = catalog
            .list(
                &PageRequest::new(
                    100,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    None,
                    ProviderListFilter::Enabled,
                )
                .unwrap(),
            )
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.registration.id() == &registration_id)
            .unwrap();
        let wire = catalog_row_list_item_json(&row);
        let encoded = serde_json::to_vec(&wire).unwrap();
        assert_eq!(encoded_catalog_row_bytes(&row), encoded.len());
        assert_eq!(
            encoded,
            serde_json::to_vec(&json!({
                "registration_id": registration_id.as_str(),
                "handle": "special-argv",
                "enabled": true,
                "config_revision": 1,
                "config": {
                    "executable": "/bin/echo",
                    "argv": special_argv,
                    "working_directory": "/work/dir\"\\n",
                    "timeout_seconds": 90,
                }
            }))
            .unwrap()
        );

        let row_bytes = encoded_catalog_row_bytes(&row);
        let mut cursor: Option<PageCursor> = None;
        let mut listed = Vec::new();
        loop {
            let page = catalog
                .list(
                    &PageRequest::new(10, row_bytes, cursor, ProviderListFilter::Enabled).unwrap(),
                )
                .unwrap();
            assert_eq!(page.rows.len(), 1);
            listed.push(page.rows[0].registration.id().clone());
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert!(listed.contains(&registration_id));
    }

    #[test]
    fn active_run_encoded_bytes_match_serde_json_and_page_budget() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b41a2").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("active-run-byte-budget").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let conn = catalog.connect_write().unwrap();
        let run_ids = [
            "019f6e88-b403-73a6-89f9-ebfe668b41a3",
            "019f6e88-b403-73a6-89f9-ebfe668b41a4",
            "019f6e88-b403-73a6-89f9-ebfe668b41a5",
        ];
        for run_id in run_ids {
            insert_active_run(
                &conn,
                registration_id.as_str(),
                run_id,
                sample_graph_revision(),
            );
        }
        let sample = ActiveRunImpact {
            run_id: RunId::parse(run_ids[0]).unwrap(),
            graph_revision: GraphRevision::parse(sample_graph_revision()).unwrap(),
        };
        let wire = active_run_list_item_json(&sample);
        let encoded = serde_json::to_vec(&wire).unwrap();
        assert_eq!(encoded_active_run_bytes(&sample), encoded.len());
        assert_eq!(
            encoded,
            serde_json::to_vec(&json!({
                "run_id": sample.run_id.as_str(),
                "graph_revision": sample.graph_revision.as_str(),
            }))
            .unwrap()
        );

        let row_bytes = encoded_active_run_bytes(&sample);
        let exact_fit = catalog
            .active_run_impact(
                &registration_id,
                &PageRequest::new(10, row_bytes, None, ()).unwrap(),
            )
            .unwrap();
        assert_eq!(exact_fit.rows.len(), 1);
        assert_eq!(exact_fit.rows[0].run_id.as_str(), run_ids[0]);
        assert!(exact_fit.next_cursor.is_some());

        let one_byte_under = catalog.active_run_impact(
            &registration_id,
            &PageRequest::new(10, row_bytes.saturating_sub(1), None, ()).unwrap(),
        );
        assert!(matches!(
            one_byte_under,
            Err(CatalogPersistenceError::InvalidCursor)
        ));

        let mut cursor: Option<PageCursor> = None;
        let mut listed = Vec::new();
        loop {
            let page = catalog
                .active_run_impact(
                    &registration_id,
                    &PageRequest::new(1, row_bytes, cursor, ()).unwrap(),
                )
                .unwrap();
            assert_eq!(page.rows.len(), 1);
            listed.push(page.rows[0].run_id.as_str().to_owned());
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(listed, run_ids.to_vec());
    }

    #[test]
    fn run_ids_traversal_digest_incremental_matches_canonical_json() {
        let run_ids: Vec<String> = [
            "019f6e88-b403-73a6-89f9-ebfe668b41d0",
            "019f6e88-b403-73a6-89f9-ebfe668b41d1",
            "019f6e88-b403-73a6-89f9-ebfe668b41d2",
            "019f6e88-b403-73a6-89f9-ebfe668b41d3",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let canonical = digest_canonical_json(&json!(
            run_ids.iter().map(String::as_str).collect::<Vec<_>>()
        ));
        assert_eq!(run_ids_traversal_digest(&run_ids), canonical);
        let mut hasher = RunIdsTraversalDigestHasher::new();
        for run_id in &run_ids {
            hasher.update_run_id(run_id);
        }
        assert_eq!(hasher.finish(), canonical);
    }

    #[test]
    fn disable_warning_many_page_traversal_resumes_without_skip() {
        let (_guard, _dir, catalog) = test_catalog();
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b41b1").unwrap();
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("many-page-warnings").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let conn = catalog.connect_write().unwrap();
        let run_ids = [
            "019f6e88-b403-73a6-89f9-ebfe668b41c0",
            "019f6e88-b403-73a6-89f9-ebfe668b41c1",
            "019f6e88-b403-73a6-89f9-ebfe668b41c2",
            "019f6e88-b403-73a6-89f9-ebfe668b41c3",
            "019f6e88-b403-73a6-89f9-ebfe668b41c4",
            "019f6e88-b403-73a6-89f9-ebfe668b41c5",
            "019f6e88-b403-73a6-89f9-ebfe668b41c6",
            "019f6e88-b403-73a6-89f9-ebfe668b41c7",
            "019f6e88-b403-73a6-89f9-ebfe668b41c8",
            "019f6e88-b403-73a6-89f9-ebfe668b41c9",
            "019f6e88-b403-73a6-89f9-ebfe668b41ca",
            "019f6e88-b403-73a6-89f9-ebfe668b41cb",
        ];
        for run_id in run_ids {
            insert_active_run(
                &conn,
                registration_id.as_str(),
                run_id,
                sample_graph_revision(),
            );
        }
        let sample = ActiveRunImpact {
            run_id: RunId::parse(run_ids[0]).unwrap(),
            graph_revision: GraphRevision::parse(sample_graph_revision()).unwrap(),
        };
        let row_bytes = encoded_active_run_bytes(&sample);
        let (shown, acknowledgement) =
            collect_disable_warning_run_ids(&catalog, &registration_id, 1, row_bytes);
        assert_eq!(shown, run_ids.to_vec());
        let snapshot = catalog.active_set_snapshot(&registration_id).unwrap();
        let ack = acknowledgement.expect("final disable page must mint acknowledgement");
        catalog
            .mutate(CatalogMutation::Disable {
                registration_id,
                expected: snapshot,
                acknowledgement: ack,
            })
            .unwrap();
    }

    #[test]
    fn traced_update_missing_registration_emits_intent_version_check_and_rollback() {
        let (_guard, dir, _catalog) = test_catalog();
        let path = dir.path().join("state.db");
        let (trace_dir, _trace_writer, sink) = test_sink("provider-update-missing-row");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let catalog = SqliteProviderCatalog::with_trace(path, trace);
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4199").unwrap();
        let err = catalog
            .mutate(CatalogMutation::Update {
                registration_id,
                expected_config_revision: 1,
                config: sample_config(),
            })
            .unwrap_err();
        assert!(matches!(err, CatalogPersistenceError::NotFound));
        let events = read_events(
            &trace_dir
                .trace_dir()
                .join("provider-update-missing-row.jsonl"),
        );
        assert_eq!(
            event_names(&events),
            vec!["intent", "version_check", "rollback"]
        );
        assert_eq!(events[1]["registration_config_revision"], 1);
        assert_eq!(events[2]["outcome"], "rejected");
    }

    #[test]
    fn traced_update_stale_revision_emits_intent_version_check_and_rollback() {
        let (_guard, dir, setup_catalog) = test_catalog();
        let path = dir.path().join("state.db");
        let registration_id =
            RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b4198").unwrap();
        setup_catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("stale-update").unwrap(),
                config: sample_config(),
            })
            .unwrap();
        let (trace_dir, _trace_writer, sink) = test_sink("provider-update-stale-race");
        let trace = OptionalTraceSink { inner: Some(sink) };
        let catalog = SqliteProviderCatalog::with_trace(path, trace);
        let err = catalog
            .mutate(CatalogMutation::Update {
                registration_id,
                expected_config_revision: 99,
                config: sample_config(),
            })
            .unwrap_err();
        assert!(matches!(err, CatalogPersistenceError::Stale));
        let events = read_events(
            &trace_dir
                .trace_dir()
                .join("provider-update-stale-race.jsonl"),
        );
        assert_eq!(
            event_names(&events),
            vec!["intent", "version_check", "rollback"]
        );
        assert_eq!(events[1]["registration_config_revision"], 99);
        assert_eq!(events[2]["outcome"], "rejected");
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

    #[test]
    fn absent_store_path_read_fails_without_creating_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        assert_no_store_files(&path);

        let catalog = SqliteProviderCatalog::new(&path);
        let error = catalog
            .list(
                &PageRequest::new(
                    10,
                    COLLECTION_PAGE_DATA_BUDGET_BYTES,
                    None,
                    ProviderListFilter::All,
                )
                .unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CatalogPersistenceError::Persistence(PersistenceError::Open { .. })
        ));
        assert_no_store_files(&path);
    }

    #[test]
    fn integrity_wire_rejects_edited_and_alias_base64url() {
        let (_guard, _dir, catalog) = test_catalog();
        let conn = catalog.connect_write().unwrap();
        let integrity_key = read_integrity_key(&conn).unwrap();
        let filter_fingerprint =
            registrations_filter_fingerprint(&ProviderListFilter::Enabled).unwrap();
        let payload = json!({
            "collection": super::COLLECTION_REGISTRATIONS,
            "cursor_version": 1,
            "filter_fingerprint": filter_fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "019f6e88-b403-73a6-89f9-ebfe668b41a1",
            },
        });
        let wire = mint_integrity_wire(&integrity_key, CURSOR_DOMAIN, payload).unwrap();

        let mut edited = wire.clone();
        let last = edited.pop().unwrap();
        edited.push(if last == 'A' { 'B' } else { 'A' });
        assert!(
            decode_integrity_wire(&integrity_key, CURSOR_DOMAIN, &edited, || {
                CatalogPersistenceError::InvalidCursor
            },)
            .is_err()
        );

        let alias = format!("{wire}A");
        assert!(
            decode_integrity_wire(&integrity_key, CURSOR_DOMAIN, &alias, || {
                CatalogPersistenceError::InvalidCursor
            },)
            .is_err()
        );
    }

    #[test]
    fn integrity_wire_rejects_noncanonical_wrapper() {
        let (_guard, _dir, catalog) = test_catalog();
        let conn = catalog.connect_write().unwrap();
        let integrity_key = read_integrity_key(&conn).unwrap();
        let filter_fingerprint =
            registrations_filter_fingerprint(&ProviderListFilter::Enabled).unwrap();
        let payload = json!({
            "collection": super::COLLECTION_REGISTRATIONS,
            "cursor_version": 1,
            "filter_fingerprint": filter_fingerprint,
            "last_key": {
                "created_at": "2026-07-17T12:00:00.000Z",
                "stable_id": "019f6e88-b403-73a6-89f9-ebfe668b41a1",
            },
        });
        let wire = mint_integrity_wire(&integrity_key, CURSOR_DOMAIN, payload.clone()).unwrap();
        let canonical = String::from_utf8(base64url_decode(&wire).unwrap()).unwrap();
        let parsed: Value = serde_json::from_str(&canonical).unwrap();
        let mac = parsed["mac"].as_str().unwrap();
        let payload_json = serde_json::to_string(&parsed["payload"]).unwrap();

        let reordered = format!(r#"{{"payload":{payload_json},"mac":"{mac}"}}"#);
        assert!(
            decode_integrity_wire(
                &integrity_key,
                CURSOR_DOMAIN,
                &base64url_no_pad(reordered.as_bytes()),
                || CatalogPersistenceError::InvalidCursor,
            )
            .is_err()
        );

        let whitespace = canonical.replace(',', ", ");
        assert!(
            decode_integrity_wire(
                &integrity_key,
                CURSOR_DOMAIN,
                &base64url_no_pad(whitespace.as_bytes()),
                || CatalogPersistenceError::InvalidCursor,
            )
            .is_err()
        );

        let duplicate = format!(r#"{{"mac":"{mac}","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(
            decode_integrity_wire(
                &integrity_key,
                CURSOR_DOMAIN,
                &base64url_no_pad(duplicate.as_bytes()),
                || CatalogPersistenceError::InvalidCursor,
            )
            .is_err()
        );

        let extra = format!(r#"{{"extra":"x","mac":"{mac}","payload":{payload_json}}}"#);
        assert!(
            decode_integrity_wire(
                &integrity_key,
                CURSOR_DOMAIN,
                &base64url_no_pad(extra.as_bytes()),
                || CatalogPersistenceError::InvalidCursor,
            )
            .is_err()
        );

        let ack_wire = mint_integrity_wire(&integrity_key, DISABLE_ACK_DOMAIN, payload).unwrap();
        let ack_canonical = String::from_utf8(base64url_decode(&ack_wire).unwrap()).unwrap();
        let ack_parsed: Value = serde_json::from_str(&ack_canonical).unwrap();
        let ack_mac = ack_parsed["mac"].as_str().unwrap();
        let ack_payload_json = serde_json::to_string(&ack_parsed["payload"]).unwrap();
        let ack_extra =
            format!(r#"{{"extra":"x","mac":"{ack_mac}","payload":{ack_payload_json}}}"#);
        assert!(
            decode_integrity_wire(
                &integrity_key,
                DISABLE_ACK_DOMAIN,
                &base64url_no_pad(ack_extra.as_bytes()),
                || CatalogPersistenceError::InvalidAck,
            )
            .is_err()
        );
    }
}
