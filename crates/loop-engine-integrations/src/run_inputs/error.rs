use std::path::PathBuf;

use loop_engine_core::model::run_input::InputError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunInputLoadError {
    #[error("failed to read run inputs {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("run inputs {path} exceeds {max} bytes (actual {actual})")]
    TooLarge {
        path: PathBuf,
        max: usize,
        actual: usize,
    },
    #[error("run inputs {path} are malformed: {message}")]
    Malformed { path: PathBuf, message: String },
    #[error(transparent)]
    Input(#[from] InputError),
}
