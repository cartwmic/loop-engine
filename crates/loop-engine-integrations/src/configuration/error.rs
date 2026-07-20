use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigurationError {
    #[error("HOME is unavailable")]
    HomeUnavailable,
    #[error("current working directory is unavailable: {0}")]
    CurrentDirectory(#[source] std::io::Error),
    #[error("path must be absolute: {0}")]
    RelativePath(String),
    #[error("path is not valid UTF-8")]
    PathNotUtf8,
    #[error("path exceeds {max} UTF-8 bytes (actual {actual})")]
    PathTooLong { max: usize, actual: usize },
    #[error("failed to inspect configuration path {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read configuration {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("configuration {path} exceeds {max} bytes (actual {actual})")]
    TooLarge {
        path: PathBuf,
        max: usize,
        actual: usize,
    },
    #[error("configuration {path} is malformed: {message}")]
    Malformed { path: PathBuf, message: String },
    #[error("configuration {path} uses schema version {actual}; supported version is 1")]
    UnsupportedVersion { path: PathBuf, actual: u64 },
    #[error("timeout_seconds must be positive")]
    NonPositiveTimeout,
}
