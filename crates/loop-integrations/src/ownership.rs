//! Local invocation admission. The lock is held only for publication/admission
//! and completion arbitration, never while waiting for primary work.
use loop_core::OwnedExecution;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub const OWNERSHIP_ENV: &str = "LOOP_ENGINE_OWNERSHIP_DIRECTORY";

/// Dagu does not forward arbitrary parent environment to exec actions. Freeze
/// the invocation admission locator in its generated graph explicitly.
pub fn dagu_environment_yaml() -> String {
    std::env::var(OWNERSHIP_ENV)
        .ok()
        .map(|directory| {
            format!(
                "env:\n  {OWNERSHIP_ENV}: {}\n",
                serde_json::to_string(&directory).unwrap()
            )
        })
        .unwrap_or_default()
}

pub struct Admission {
    directory: PathBuf,
    _lock: File,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompletionReceipt {
    pub exit_code: i32,
    pub inner_workers: Vec<loop_core::InnerWorker>,
}

impl Admission {
    pub fn acquire(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("admission.lock"))?;
        lock.lock()?;
        Ok(Self {
            directory: directory.to_owned(),
            _lock: lock,
        })
    }

    /// Bounded acquisition for an active cancellation attempt. Other users keep
    /// the short blocking admission path; the controller has one total clock.
    pub fn acquire_until(directory: &Path, deadline: std::time::Instant) -> io::Result<Self> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("admission.lock"))?;
        loop {
            match lock.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        directory: directory.to_owned(),
                        _lock: lock,
                    })
                }
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }

    pub fn stopped(&self) -> bool {
        self.directory.join("stop.json").exists()
    }

    /// The cancellation controller calls this while holding admission, after
    /// validating the catalog target. Repeated acquisition retains the request.
    /// This is admission only: it neither signals nor acknowledges cleanup.
    pub fn admit_stop(&self, request: &serde_json::Value) -> io::Result<()> {
        if !self.stopped() {
            self.write("stop.json", request)?;
        }
        Ok(())
    }

    pub fn publish(&self, ownership: &OwnedExecution) -> io::Result<()> {
        // Allocate the controller lock before publishing a cancellable target.
        // A refused cancel (including a completion race) creates no files.
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("controller.lock"))?
            .sync_all()?;
        self.write("ownership.json", ownership)
    }

    pub fn record_completion(&self, receipt: &CompletionReceipt) -> io::Result<()> {
        self.write("waiter-completion.json", receipt)
    }

    pub fn write(&self, name: &str, value: &impl Serialize) -> io::Result<()> {
        let temporary = self.directory.join(format!("{name}.tmp"));
        let mut file = File::create(&temporary)?;
        serde_json::to_writer(&mut file, value)?;
        file.sync_all()?;
        fs::rename(temporary, self.directory.join(name))?;
        File::open(&self.directory)?.sync_all()
    }
}

pub fn directory(capture_dir: &str) -> PathBuf {
    Path::new(capture_dir).join("ownership")
}

/// Both facade helpers use this same admission path. Their own PID/group is
/// published before spawn, so primary work is already reachable through an
/// owned root even if it executes before spawn returns.
pub fn spawn_admitted(command: &mut Command) -> io::Result<Child> {
    let Some(directory) = std::env::var_os(OWNERSHIP_ENV) else {
        return command.spawn();
    };
    let admission = Admission::acquire(Path::new(&directory))?;
    if admission.stopped() {
        return Err(io::Error::other(
            "invocation cancellation admitted; primary worker launch inhibited",
        ));
    }
    let pid = std::process::id();
    #[cfg(unix)]
    let group = unsafe { libc::getpgrp() } as u32;
    #[cfg(not(unix))]
    let group = pid;
    admission.write(
        &format!("helper-{pid}.json"),
        &serde_json::json!({
            "root_pid": pid, "process_group_id": group
        }),
    )?;
    let mut child = command.spawn()?;
    if let Err(error) = admission.write(
        &format!("child-{}.json", child.id()),
        &serde_json::json!({
            "root_pid": child.id(), "parent_pid": pid, "process_group_id": group
        }),
    ) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

pub fn read_ownership(capture_dir: &str) -> io::Result<Option<OwnedExecution>> {
    match fs::read(directory(capture_dir).join("ownership.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A receipt is not cleanup acknowledgment. Waiter loss and elapsed allowance
/// cannot make an admitted, unacknowledged cancellation quiescent.
pub fn cleanup_pending(capture_dir: &str) -> bool {
    let directory = directory(capture_dir);
    directory.join("stop.json").exists() && !directory.join("cleanup-acknowledged.json").exists()
}

pub fn cancellation_state(capture_dir: &str) -> io::Result<Option<serde_json::Value>> {
    let directory = directory(capture_dir);
    if !directory.join("stop.json").is_file() {
        return Ok(None);
    }
    let read = |path: PathBuf| -> io::Result<serde_json::Value> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    };
    let mut attempts = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("attempt-") && name.ends_with(".json") {
            attempts.push(read(entry.path())?);
        }
    }
    attempts.sort_by_key(|value| value["attempt"].as_u64());
    Ok(Some(
        serde_json::json!({"request":read(directory.join("stop.json"))?, "attempts":attempts,
        "acknowledgment": if directory.join("cleanup-acknowledged.json").exists() {
            read(directory.join("cleanup-acknowledged.json"))?
        } else { serde_json::Value::Null }}),
    ))
}

pub fn live_owned_work(capture_dir: &str) -> io::Result<bool> {
    let directory = directory(capture_dir);
    if !directory.exists() || directory.join("cleanup-acknowledged.json").exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "ownership.json"
            || name.starts_with("helper-")
            || name.starts_with("child-")
            || name.starts_with("observed-")
        {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            #[cfg(unix)]
            for (key, group) in [("root_pid", false), ("process_group_id", true)] {
                if let Some(pid) = value[key].as_u64() {
                    let pid = i32::try_from(pid).map_err(io::Error::other)?;
                    if pid > 0 && unsafe { libc::kill(if group { -pid } else { pid }, 0) } == 0 {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}
