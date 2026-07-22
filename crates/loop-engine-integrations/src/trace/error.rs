use std::path::PathBuf;

use thiserror::Error;

/// Trace file I/O phase preserved for truthful `trace.sink_failure.phase` mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceIoPhase {
    Write,
    Flush,
    Fsync,
}

impl TraceIoPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Flush => "flush",
            Self::Fsync => "fsync",
        }
    }
}

impl std::fmt::Display for TraceIoPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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
    #[error("trace I/O failed during {phase} at {path}: {source}")]
    Io {
        path: PathBuf,
        phase: TraceIoPhase,
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
        Self::io_at(path, TraceIoPhase::Write, source)
    }

    pub(crate) fn io_at(
        path: impl Into<PathBuf>,
        phase: TraceIoPhase,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            path: path.into(),
            phase,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn io_phase_labels_match_contract() {
        assert_eq!(TraceIoPhase::Write.as_str(), "write");
        assert_eq!(TraceIoPhase::Flush.as_str(), "flush");
        assert_eq!(TraceIoPhase::Fsync.as_str(), "fsync");
    }

    #[test]
    fn io_defaults_to_write_phase() {
        let error = TraceError::io("/tmp/trace.jsonl", io::ErrorKind::NotFound.into());
        assert!(matches!(
            error,
            TraceError::Io {
                phase: TraceIoPhase::Write,
                ..
            }
        ));
    }

    #[test]
    fn io_at_preserves_flush_and_fsync_phases() {
        let flush = TraceError::io_at(
            "/tmp/trace.jsonl",
            TraceIoPhase::Flush,
            io::ErrorKind::Interrupted.into(),
        );
        assert!(matches!(
            flush,
            TraceError::Io {
                phase: TraceIoPhase::Flush,
                ..
            }
        ));

        let fsync = TraceError::io_at(
            "/tmp/trace.jsonl",
            TraceIoPhase::Fsync,
            io::ErrorKind::Other.into(),
        );
        assert!(matches!(
            fsync,
            TraceError::Io {
                phase: TraceIoPhase::Fsync,
                ..
            }
        ));
    }
}
