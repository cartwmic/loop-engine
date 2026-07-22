use std::collections::BTreeMap;
use std::sync::OnceLock;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rusqlite_migration::{M, MigrationDefinitionError, Migrations};
use tracing::{debug, info};

use super::error::{PersistenceError, SchemaMismatchKind};

/// Upper bound on re-synchronization attempts when concurrent opens race on DDL.
const MIGRATION_SERIALIZATION_RETRIES: u32 = 16;

pub const INITIAL_MIGRATION_SQL: &str = include_str!("../../migrations/0001_initial.sql");

/// Bundled schema generation shipped with this binary (`0001` through T105).
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Authoritative v1 tables from `0001_initial.sql`.
pub const V1_SCHEMA_TABLES: &[&str] = &[
    "integration_metadata",
    "provider_registrations",
    "runs",
    "run_journal_sequences",
    "evidence",
    "journal_entries",
    "evidence_associations",
];

/// Authoritative v1 indexes from `0001_initial.sql`.
pub const V1_SCHEMA_INDEXES: &[&str] = &[
    "idx_provider_registrations_handle_enabled",
    "idx_provider_registrations_created_id",
    "idx_provider_registrations_enabled_created_id",
    "idx_runs_registration_lifecycle_id",
    "idx_runs_registration_active_id",
    "idx_runs_created_id",
    "idx_evidence_run_created_id",
    "idx_evidence_associations_run_journal",
    "idx_evidence_associations_run_evidence",
];

type SchemaObjectKey = (&'static str, &'static str);

pub fn bundled_migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(INITIAL_MIGRATION_SQL)])
}

pub(crate) fn read_user_version(conn: &Connection) -> Result<u32, PersistenceError> {
    let version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .map_err(|source| PersistenceError::PragmaRead {
            pragma: "user_version",
            source,
        })?;
    if version < 0 {
        return Err(PersistenceError::InvalidUserVersion { observed: version });
    }
    Ok(version as u32)
}

fn ensure_schema_compatible_from_version(
    observed: u32,
    supported_version: u32,
) -> Result<(), PersistenceError> {
    if observed > supported_version {
        return Err(PersistenceError::FutureSchema {
            supported: supported_version,
            observed,
        });
    }
    Ok(())
}

/// Read `user_version` and reject newer schemas before any write-affecting pragma.
#[cfg(test)]
pub(crate) fn preflight_schema_compatibility(
    conn: &Connection,
    supported_version: u32,
) -> Result<(), PersistenceError> {
    let observed = read_user_version(conn)?;
    ensure_schema_compatible_from_version(observed, supported_version)
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_sqlite_master_sql(
    conn: &Connection,
    object_type: &str,
    name: &str,
) -> Result<Option<String>, PersistenceError> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
        params![object_type, name],
        |row| row.get(0),
    )
    .optional()
    .map_err(|source| PersistenceError::SchemaInventoryProbe { source })
}

fn build_expected_v1_schema_objects() -> BTreeMap<SchemaObjectKey, String> {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(INITIAL_MIGRATION_SQL)
        .expect("bundled initial migration must apply to in-memory database");

    let mut expected = BTreeMap::new();
    for &name in V1_SCHEMA_TABLES {
        let sql = read_sqlite_master_sql(&conn, "table", name)
            .expect("probe bundled migration schema")
            .unwrap_or_else(|| panic!("bundled migration missing table {name}"));
        expected.insert(("table", name), normalize_schema_sql(&sql));
    }
    for &name in V1_SCHEMA_INDEXES {
        let sql = read_sqlite_master_sql(&conn, "index", name)
            .expect("probe bundled migration schema")
            .unwrap_or_else(|| panic!("bundled migration missing index {name}"));
        expected.insert(("index", name), normalize_schema_sql(&sql));
    }
    expected
}

fn expected_v1_schema_objects() -> &'static BTreeMap<SchemaObjectKey, String> {
    static EXPECTED: OnceLock<BTreeMap<SchemaObjectKey, String>> = OnceLock::new();
    EXPECTED.get_or_init(build_expected_v1_schema_objects)
}

fn verify_schema_object_shape(
    conn: &Connection,
    object_type: &'static str,
    name: &'static str,
    expected_sql: &str,
) -> Result<(), PersistenceError> {
    match read_sqlite_master_sql(conn, object_type, name)? {
        None => Err(PersistenceError::SchemaMismatch {
            object_type,
            name,
            kind: SchemaMismatchKind::Missing,
        }),
        Some(actual) if normalize_schema_sql(&actual) != expected_sql => {
            Err(PersistenceError::SchemaMismatch {
                object_type,
                name,
                kind: SchemaMismatchKind::SqlDivergence,
            })
        }
        Some(_) => Ok(()),
    }
}

/// Compare each bundled v1 object's `sqlite_master.sql` to the authoritative migration shape.
pub(crate) fn verify_v1_schema_shape(conn: &Connection) -> Result<(), PersistenceError> {
    let expected = expected_v1_schema_objects();
    for &name in V1_SCHEMA_TABLES {
        verify_schema_object_shape(
            conn,
            "table",
            name,
            expected
                .get(&("table", name))
                .expect("bundled expected schema catalog"),
        )?;
    }
    for &name in V1_SCHEMA_INDEXES {
        verify_schema_object_shape(
            conn,
            "index",
            name,
            expected
                .get(&("index", name))
                .expect("bundled expected schema catalog"),
        )?;
    }
    Ok(())
}

fn verify_schema_shape_at_supported_version(
    conn: &Connection,
    supported_version: u32,
) -> Result<(), PersistenceError> {
    if supported_version == SUPPORTED_SCHEMA_VERSION {
        verify_v1_schema_shape(conn)
    } else {
        Ok(())
    }
}

fn map_to_latest_error(
    error: rusqlite_migration::Error,
    conn: &Connection,
    supported_version: u32,
) -> PersistenceError {
    if matches!(
        error,
        rusqlite_migration::Error::MigrationDefinition(
            MigrationDefinitionError::DatabaseTooFarAhead
        )
    ) {
        let observed = read_user_version(conn).unwrap_or(supported_version.saturating_add(1));
        return PersistenceError::FutureSchema {
            supported: supported_version,
            observed,
        };
    }
    PersistenceError::Migration {
        message: error.to_string(),
    }
}

fn acquire_migration_writer_lock(
    conn: &mut Connection,
) -> Result<rusqlite::Transaction<'_>, PersistenceError> {
    conn.transaction()
        .map_err(|source| PersistenceError::Migration {
            message: format!("failed to acquire migration writer lock: {source}"),
        })
}

/// Serialize preflight, migration, and postcheck against schema writers.
///
/// Rejects future-version databases and verifies bundled schema shape while holding
/// an immediate writer lock. Does not nest inside `rusqlite_migration` transactions.
pub(crate) fn run_startup_schema_pipeline(
    conn: &mut Connection,
    migrations: &Migrations<'_>,
    supported_version: u32,
) -> Result<(), PersistenceError> {
    conn.set_transaction_behavior(TransactionBehavior::Immediate);
    let result =
        run_startup_schema_pipeline_under_immediate_behavior(conn, migrations, supported_version);
    conn.set_transaction_behavior(TransactionBehavior::Deferred);
    result
}

fn run_startup_schema_pipeline_under_immediate_behavior(
    conn: &mut Connection,
    migrations: &Migrations<'_>,
    supported_version: u32,
) -> Result<(), PersistenceError> {
    for attempt in 0..MIGRATION_SERIALIZATION_RETRIES {
        let tx = acquire_migration_writer_lock(conn)?;
        let observed = read_user_version(&tx)?;
        ensure_schema_compatible_from_version(observed, supported_version)?;

        let pending =
            migrations
                .pending_migrations(&tx)
                .map_err(|error| PersistenceError::Migration {
                    message: error.to_string(),
                })?;
        if pending < 0 {
            return Err(PersistenceError::FutureSchema {
                supported: supported_version,
                observed,
            });
        }
        if pending == 0 {
            verify_schema_shape_at_supported_version(&tx, supported_version)?;
            tx.commit().map_err(|source| PersistenceError::Migration {
                message: format!("failed to release migration writer lock: {source}"),
            })?;
            debug!(
                current_schema_version = observed,
                supported_schema_version = supported_version,
                "persistence schema already at latest version"
            );
            return Ok(());
        }

        tx.commit().map_err(|source| PersistenceError::Migration {
            message: format!("failed to release migration writer lock: {source}"),
        })?;

        debug!(
            current_schema_version = observed,
            supported_schema_version = supported_version,
            migration_attempt = attempt,
            "evaluating persistence migrations"
        );

        match migrations.to_latest(conn) {
            Ok(()) => {
                let applied = read_user_version(conn)?;
                info!(
                    applied_schema_version = applied,
                    supported_schema_version = supported_version,
                    "persistence migrations complete"
                );
            }
            Err(error) => {
                // Another opener may have completed the same migration after this
                // connection released its preflight lock. Treat only an observed,
                // shape-valid supported schema as a successful concurrent winner.
                let observed = read_user_version(conn)?;
                if observed == supported_version {
                    verify_schema_shape_at_supported_version(conn, supported_version)?;
                    continue;
                }
                return Err(map_to_latest_error(error, conn, supported_version));
            }
        }
    }

    Err(PersistenceError::Migration {
        message: "concurrent migration serialization exhausted retries".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn preflight_rejects_future_schema_from_user_version() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 2").unwrap();

        let error = preflight_schema_compatibility(&conn, SUPPORTED_SCHEMA_VERSION).unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::FutureSchema {
                supported: 1,
                observed: 2,
            }
        ));
    }

    #[test]
    fn verify_v1_schema_shape_requires_every_bundled_object() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 1;
             CREATE TABLE integration_metadata (
                 key TEXT NOT NULL PRIMARY KEY CHECK (key = 'integrity_key'),
                 value BLOB NOT NULL CHECK (length(value) = 32)
             );
             INSERT INTO integration_metadata (key, value) VALUES ('integrity_key', randomblob(32));",
        )
        .unwrap();

        let error = verify_v1_schema_shape(&conn).unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::SchemaMismatch {
                object_type: "table",
                name: "provider_registrations",
                kind: SchemaMismatchKind::Missing,
            }
        ));
    }

    #[test]
    fn verify_v1_schema_shape_rejects_altered_column_definition() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let mut conn = Connection::open(&path).unwrap();
        run_startup_schema_pipeline(&mut conn, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION)
            .unwrap();

        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TABLE runs;
             CREATE TABLE runs (
                 run_id TEXT NOT NULL PRIMARY KEY,
                 registration_id TEXT NOT NULL,
                 config_revision_at_create INTEGER NOT NULL,
                 current_state TEXT NOT NULL,
                 lifecycle TEXT NOT NULL,
                 workflow_state_version INTEGER NOT NULL,
                 lifecycle_version INTEGER NOT NULL,
                 label_version INTEGER NOT NULL,
                 label TEXT,
                 graph_revision TEXT NOT NULL,
                 canonical_graph_version INTEGER NOT NULL,
                 graph_canonical_projection_json TEXT NOT NULL,
                 inputs_json TEXT NOT NULL DEFAULT '{}',
                 created_at TEXT NOT NULL
             );
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();

        let error = verify_v1_schema_shape(&conn).unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::SchemaMismatch {
                object_type: "table",
                name: "runs",
                kind: SchemaMismatchKind::SqlDivergence,
            }
        ));
    }

    #[test]
    fn startup_pipeline_rejects_future_schema_before_pending_migration() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.db");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 2").unwrap();

        let error =
            run_startup_schema_pipeline(&mut conn, &bundled_migrations(), SUPPORTED_SCHEMA_VERSION)
                .unwrap_err();
        assert!(matches!(
            error,
            PersistenceError::FutureSchema {
                supported: 1,
                observed: 2,
            }
        ));
    }
}
