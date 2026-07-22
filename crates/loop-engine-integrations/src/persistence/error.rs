use std::path::PathBuf;

use thiserror::Error;

/// Classifies deterministic bundled-schema verification mismatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaMismatchKind {
    /// Required table or index from bundled migration `0001` is absent.
    Missing,
    /// Object exists but its `sqlite_master.sql` does not match the authoritative shape.
    SqlDivergence,
}

/// Classifies commit I/O outcome errors for trace routing.
///
/// `traced.rs` should consult these methods so commit I/O failures with unknown
/// durable outcome do not emit rollback events. When
/// [`CommitOutcomeError::is_commit_outcome_unverified`] or
/// [`CommitOutcomeError::is_commit_integrity_failure`] is true, trace must report
/// `persistence.failed` without claiming rollback.
pub trait CommitOutcomeError {
    fn is_commit_outcome_unverified(&self) -> bool;
    fn is_commit_integrity_failure(&self) -> bool;
}

/// Whether a persistence write error should emit a trace rollback (vs unknown outcome).
pub fn commit_outcome_trace_is_rollback<E: CommitOutcomeError>(error: &E) -> bool {
    !error.is_commit_outcome_unverified() && !error.is_commit_integrity_failure()
}

#[cfg(test)]
impl CommitOutcomeError for &str {
    fn is_commit_outcome_unverified(&self) -> bool {
        false
    }

    fn is_commit_integrity_failure(&self) -> bool {
        false
    }
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to create persistence directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open persistence store at {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to read pragma {pragma}: {source}")]
    PragmaRead {
        pragma: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to apply persistence pragma {pragma}: {source}")]
    Pragma {
        pragma: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database schema version {observed} exceeds supported version {supported}")]
    FutureSchema { supported: u32, observed: u32 },
    #[error("bundled schema {object_type} {name} mismatch ({kind:?})")]
    SchemaMismatch {
        object_type: &'static str,
        name: &'static str,
        kind: SchemaMismatchKind,
    },
    #[error("failed to probe schema inventory: {source}")]
    SchemaInventoryProbe {
        #[source]
        source: rusqlite::Error,
    },
    #[error("schema migration failed: {message}")]
    Migration { message: String },
    #[error("integration metadata key {key} is missing")]
    MetadataKeyMissing { key: &'static str },
    #[error("integration metadata key {key} has length {actual}; expected {expected}")]
    MetadataKeyInvalidLength {
        key: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("failed to read integration metadata: {source}")]
    MetadataRead {
        #[source]
        source: rusqlite::Error,
    },
    #[error("invalid SQLite user_version {observed}")]
    InvalidUserVersion { observed: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_mismatch_error_names_object_and_kind() {
        let error = PersistenceError::SchemaMismatch {
            object_type: "table",
            name: "runs",
            kind: SchemaMismatchKind::Missing,
        };
        assert!(error.to_string().contains("table"));
        assert!(error.to_string().contains("runs"));
        assert!(error.to_string().contains("Missing"));
    }
}
