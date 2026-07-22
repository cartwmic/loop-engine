use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrdinalError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Returns the 1-based invocation ordinal, persisted atomically under scenario control.
pub fn next(path: &Path) -> Result<u64, OrdinalError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("lock");
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                std::thread::yield_now();
            }
            Err(error) => return Err(OrdinalError::Io(error)),
        }
    }

    let current = fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = current.saturating_add(1);
    fs::write(path, next.to_string())?;
    let _ = fs::remove_file(&lock_path);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn ordinal_increments_deterministically() {
        let temp = TempDir::new("ordinal");
        let path = temp.path().join("ordinal");
        assert_eq!(next(&path).unwrap(), 1);
        assert_eq!(next(&path).unwrap(), 2);
        assert_eq!(next(&path).unwrap(), 3);
    }
}
