use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BarrierError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

pub struct Barrier {
    root: PathBuf,
    id: String,
}

impl Barrier {
    pub fn new(root: PathBuf, id: String) -> Self {
        Self { root, id }
    }

    fn base(&self) -> PathBuf {
        self.root.join(&self.id)
    }

    fn reached_dir(&self) -> PathBuf {
        self.base().join("reached")
    }

    fn release_marker(&self) -> PathBuf {
        self.base().join("release")
    }

    pub fn reached(&self, invocation_id: &str) -> Result<(), BarrierError> {
        fs::create_dir_all(self.reached_dir())?;
        let marker = self.reached_dir().join(invocation_id);
        fs::write(&marker, b"reached")?;
        wait_for_release(&self.release_marker())?;
        Ok(())
    }

    pub fn release(&self) -> Result<(), BarrierError> {
        fs::create_dir_all(self.base())?;
        fs::write(self.release_marker(), b"release")?;
        Ok(())
    }

    #[cfg(test)]
    pub fn cleanup(&self) -> Result<(), BarrierError> {
        let _ = fs::remove_dir_all(self.base());
        Ok(())
    }
}

fn wait_for_release(release_marker: &Path) -> Result<(), BarrierError> {
    loop {
        if release_marker.is_file() {
            return Ok(());
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn reached_waits_for_release_without_timing_only_synchronization() {
        let temp = TempDir::new("barrier-unit");
        let reached = thread::spawn({
            let fs_barrier = Barrier::new(temp.path().to_path_buf(), "overlap".to_string());
            move || fs_barrier.reached("inv-1").unwrap()
        });
        let reached_marker = temp.path().join("overlap/reached/inv-1");
        while !reached_marker.is_file() {
            thread::yield_now();
        }
        let fs_barrier = Barrier::new(temp.path().to_path_buf(), "overlap".to_string());
        fs_barrier.release().unwrap();
        reached.join().unwrap();
        assert!(temp.path().join("overlap/reached/inv-1").is_file());
        assert!(temp.path().join("overlap/release").is_file());
        fs_barrier.cleanup().unwrap();
    }

    #[test]
    fn cleanup_removes_stale_markers_after_aborted_waiter() {
        let temp = TempDir::new("barrier-cleanup");
        let fs_barrier = Barrier::new(temp.path().to_path_buf(), "stale".to_string());
        fs::create_dir_all(fs_barrier.reached_dir()).unwrap();
        fs::write(fs_barrier.reached_dir().join("inv-dead"), b"reached").unwrap();
        fs_barrier.cleanup().unwrap();
        assert!(!temp.path().join("stale").exists());
    }
}
