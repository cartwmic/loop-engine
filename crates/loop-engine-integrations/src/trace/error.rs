use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraceError {
    #[error("trace directory budget is exhausted (required {required}, available {available})")]
    BudgetExhausted { required: u64, available: u64 },
    #[error("trace file would exceed {max} bytes")]
    FileLimit { max: u64 },
    #[error("trace reservation is exhausted")]
    ReservationExhausted,
    #[error("trace sink is unavailable after a prior write failure")]
    SinkFailed,
    #[error("trace path collision: {0}")]
    Collision(PathBuf),
    #[error("trace payload cannot replace envelope field {0}")]
    ReservedPayloadField(String),
    #[error("trace I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("trace event serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("trace reservation sidecar is malformed: {0}")]
    MalformedSidecar(PathBuf),
    #[error("no provider trace reservation is active")]
    NoProviderReservation,
}

impl TraceError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
