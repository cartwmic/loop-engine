use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("provider request exceeds {max} bytes (actual {actual})")]
    RequestOversized { max: usize, actual: usize },
    #[error("provider executable was not found: {0}")]
    ExecutableNotFound(String),
    #[error("provider spawn failed before launch: {0}")]
    PreLaunchSpawn(#[source] io::Error),
    #[error("provider supervision failed after launch: {0}")]
    Spawn(#[source] io::Error),
    #[error("provider stdin write failed: {0}")]
    Stdin(#[source] io::Error),
    #[error("provider stream reader failed: {0}")]
    Stream(#[source] io::Error),
    #[error("provider timeout is outside the platform instant range: {0} seconds")]
    TimeoutOutOfRange(u64),
    #[error("provider timed out")]
    Timeout,
    #[error("provider crashed from signal {0}")]
    Crash(i32),
    #[error("provider terminated by signal {0}")]
    Signal(i32),
    #[error("provider exited non-zero: {0:?}")]
    NonZero(Option<i32>),
    #[error("provider stdout exceeds {max} bytes (actual {actual})")]
    StdoutOversized { max: usize, actual: usize },
    #[error("provider stdout is invalid UTF-8")]
    InvalidUtf8,
    #[error("provider stdout does not contain exactly one JSON value: {0}")]
    Malformed(String),
    #[error("provider process-group termination failed: {0}")]
    Termination(String),
}
