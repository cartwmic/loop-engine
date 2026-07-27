//! Operational trace JSONL parsing and request correlation (T145).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::cli::StructuredDocument;
use super::strict_json::{StrictJsonError, parse_strict_json_value};

/// Path to a static JSONL fixture under `tests/support/fixtures/trace/`.
pub fn trace_fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support/fixtures/trace")
        .join(relative)
}

/// Copies one fixture trace file into a sandbox traces directory.
pub fn install_trace_fixture(
    fixture_relative: &str,
    traces_dir: &Path,
) -> Result<PathBuf, TraceParseError> {
    materialize_trace_fixture(&trace_fixture_path(fixture_relative), traces_dir)
}

fn materialize_trace_fixture(source: &Path, traces_dir: &Path) -> Result<PathBuf, TraceParseError> {
    let file_name = source
        .file_name()
        .ok_or_else(|| TraceParseError::Io("fixture path has no file name".into()))?;
    let destination = traces_dir.join(file_name);
    let bytes = fs::read(source).map_err(|error| TraceParseError::Io(error.to_string()))?;
    fs::write(&destination, bytes).map_err(|error| TraceParseError::Io(error.to_string()))?;
    Ok(destination)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTrace {
    pub path: PathBuf,
    pub request_id: String,
    pub events: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceParseError {
    MissingRequestId,
    MissingTracePath,
    TraceOutsideSandbox {
        trace: PathBuf,
        traces_dir: PathBuf,
    },
    TraceFilenameMismatch {
        request_id: String,
        path: PathBuf,
    },
    ReferencedArtifactMismatch {
        path: PathBuf,
        reason: String,
    },
    MissingFile(PathBuf),
    EmptyTrace,
    InvalidJsonLine {
        line: usize,
        message: String,
    },
    DuplicateKey {
        line: usize,
        path: String,
        key: String,
    },
    RequestIdMismatch {
        envelope: String,
        observed: String,
    },
    Io(String),
}

impl std::fmt::Display for TraceParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRequestId => formatter.write_str("structured document lacks request_id"),
            Self::MissingTracePath => formatter.write_str("structured document lacks trace path"),
            Self::TraceOutsideSandbox { trace, traces_dir } => write!(
                formatter,
                "trace path {} is outside sandbox traces directory {}",
                trace.display(),
                traces_dir.display()
            ),
            Self::TraceFilenameMismatch { request_id, path } => write!(
                formatter,
                "trace filename stem does not match request_id {request_id}: {}",
                path.display()
            ),
            Self::ReferencedArtifactMismatch { path, reason } => write!(
                formatter,
                "referenced trace artifact {}: {reason}",
                path.display()
            ),
            Self::MissingFile(path) => {
                write!(formatter, "trace file does not exist: {}", path.display())
            }
            Self::EmptyTrace => formatter.write_str("trace file contains no events"),
            Self::InvalidJsonLine { line, message } => {
                write!(formatter, "invalid JSONL at line {line}: {message}")
            }
            Self::DuplicateKey { line, path, key } => write!(
                formatter,
                "duplicate object key at line {line} path {path}: {key}"
            ),
            Self::RequestIdMismatch { envelope, observed } => write!(
                formatter,
                "trace request_id mismatch: envelope={envelope}, observed={observed}"
            ),
            Self::Io(message) => write!(formatter, "trace io error: {message}"),
        }
    }
}

impl std::error::Error for TraceParseError {}

pub fn read_trace_events(path: &Path) -> Result<Vec<Value>, TraceParseError> {
    let contents =
        fs::read_to_string(path).map_err(|error| TraceParseError::Io(error.to_string()))?;
    if contents.trim().is_empty() {
        return Err(TraceParseError::EmptyTrace);
    }
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
        .map(|(index, line)| parse_trace_line(index + 1, line))
        .collect()
}

pub fn parse_correlated_trace(
    document: &StructuredDocument,
    traces_dir: &Path,
) -> Result<ParsedTrace, TraceParseError> {
    parse_correlated_value(&document.value, traces_dir)
}

pub fn parse_correlated_value(
    value: &Value,
    traces_dir: &Path,
) -> Result<ParsedTrace, TraceParseError> {
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or(TraceParseError::MissingRequestId)?;
    let trace_path = value
        .get("trace")
        .and_then(Value::as_str)
        .ok_or(TraceParseError::MissingTracePath)?;
    let path = PathBuf::from(trace_path);
    validate_trace_path(&path, traces_dir, request_id)?;
    let events = read_trace_events(&path)?;
    validate_referenced_trace_artifact(request_id, &path, &events)?;
    Ok(ParsedTrace {
        path,
        request_id: request_id.to_owned(),
        events,
    })
}

fn validate_trace_path(
    path: &Path,
    traces_dir: &Path,
    request_id: &str,
) -> Result<(), TraceParseError> {
    let traces_dir = traces_dir
        .canonicalize()
        .unwrap_or_else(|_| traces_dir.to_path_buf());
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !normalized.starts_with(&traces_dir) {
        return Err(TraceParseError::TraceOutsideSandbox {
            trace: path.to_path_buf(),
            traces_dir: traces_dir.clone(),
        });
    }
    let expected = traces_dir.join(format!("{request_id}.jsonl"));
    let expected = expected.canonicalize().unwrap_or(expected);
    if normalized != expected {
        return Err(TraceParseError::TraceFilenameMismatch {
            request_id: request_id.to_owned(),
            path: path.to_path_buf(),
        });
    }
    if !path.exists() {
        return Err(TraceParseError::MissingFile(path.to_path_buf()));
    }
    Ok(())
}

fn validate_referenced_trace_artifact(
    request_id: &str,
    path: &Path,
    events: &[Value],
) -> Result<(), TraceParseError> {
    for event in events {
        let observed = event
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or_else(|| TraceParseError::ReferencedArtifactMismatch {
                path: path.to_path_buf(),
                reason: "trace line missing request_id".to_owned(),
            })?;
        if observed != request_id {
            return Err(TraceParseError::RequestIdMismatch {
                envelope: request_id.to_owned(),
                observed: observed.to_owned(),
            });
        }
    }
    Ok(())
}

fn parse_trace_line(line_number: usize, line: &str) -> Result<Value, TraceParseError> {
    parse_strict_json_value(line).map_err(|error| match error {
        StrictJsonError::DuplicateKey { path, key } => TraceParseError::DuplicateKey {
            line: line_number,
            path,
            key,
        },
        StrictJsonError::TrailingContent => TraceParseError::InvalidJsonLine {
            line: line_number,
            message: "trailing content after first JSON value".to_owned(),
        },
        StrictJsonError::Malformed(message) => TraceParseError::InvalidJsonLine {
            line: line_number,
            message,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_trace_fixture_keeps_read_only_source_and_creates_writable_destination() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let source_dir = temporary.path().join("source");
        let traces_dir = temporary.path().join("traces");
        fs::create_dir_all(&source_dir).expect("create source directory");
        fs::create_dir_all(&traces_dir).expect("create traces directory");
        let source = source_dir.join("fixture.jsonl");
        fs::write(&source, b"{\"event\":\"source\"}\n").expect("write source fixture");

        let original_permissions = fs::metadata(&source)
            .expect("source metadata")
            .permissions();
        let mut read_only_permissions = original_permissions.clone();
        read_only_permissions.set_readonly(true);
        fs::set_permissions(&source, read_only_permissions).expect("make source read-only");

        let destination =
            materialize_trace_fixture(&source, &traces_dir).expect("materialize fixture");

        assert_eq!(destination, traces_dir.join("fixture.jsonl"));
        assert_eq!(
            fs::read(&destination).expect("read destination fixture"),
            b"{\"event\":\"source\"}\n"
        );
        assert!(
            !fs::metadata(&destination)
                .expect("destination metadata")
                .permissions()
                .readonly()
        );
        fs::write(&destination, b"{\"event\":\"destination\"}\n")
            .expect("mutate destination fixture");
        assert_eq!(
            fs::read(&source).expect("read source after destination mutation"),
            b"{\"event\":\"source\"}\n"
        );
        assert!(
            fs::metadata(&source)
                .expect("source metadata after materialization")
                .permissions()
                .readonly()
        );

        fs::set_permissions(source, original_permissions).expect("restore source permissions");
    }
}
