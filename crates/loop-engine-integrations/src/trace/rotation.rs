use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::error::TraceError;

pub const TRACE_INIT_RESERVATION_BYTES: u64 = 16_777_216;
pub const TRACE_PROVIDER_CALL_RESERVATION_BYTES: u64 = 10_485_760;
pub const TRACE_FILE_MAX_BYTES: u64 = 125_829_120;
pub const TRACE_RETAINED_FILES_MAX: usize = 100;
pub const TRACE_DIRECTORY_BUDGET_BYTES: u64 = 134_217_728;
const SIDECAR_SLOT_BYTES: usize = 160;
const SIDECAR_BYTES: usize = SIDECAR_SLOT_BYTES * 2;
const LOCK_RETRY_LIMIT: Duration = Duration::from_secs(1);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    generation: u64,
    generation_complement: u64,
    unused_reservation_bytes: u64,
    unused_reservation_complement: u64,
}

pub(crate) struct EvictedTrace {
    pub path: PathBuf,
    pub encoded_bytes: u64,
}

pub(crate) struct RotationFiles {
    pub lock: File,
    pub sidecar: File,
    pub sidecar_path: PathBuf,
    pub evicted: Vec<EvictedTrace>,
}

pub(crate) fn initialize(directory: &Path, request_id: &str) -> Result<RotationFiles, TraceError> {
    create_private_directory(directory)?;
    let reserve_directory = directory.join(".reserve");
    create_private_directory(&reserve_directory)?;
    let lock_path = directory.join(".rotation.lock");
    let lock = private_open(&lock_path, false)?;
    lock_bounded(&lock, &lock_path)?;
    reconcile_stale(&reserve_directory)?;
    let evicted = evict_closed(directory, TRACE_INIT_RESERVATION_BYTES, true)?;

    let sidecar_path = reserve_directory.join(format!("{request_id}.json"));
    let mut sidecar = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&sidecar_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                TraceError::Collision(sidecar_path.clone())
            } else {
                TraceError::io(&sidecar_path, error)
            }
        })?;
    lock_bounded(&sidecar, &sidecar_path)?;
    write_sidecar(&mut sidecar, &sidecar_path, TRACE_INIT_RESERVATION_BYTES)?;
    lock.unlock()
        .map_err(|error| TraceError::io(&lock_path, error))?;
    Ok(RotationFiles {
        lock,
        sidecar,
        sidecar_path,
        evicted,
    })
}

pub(crate) fn with_rotation<T>(
    directory: &Path,
    lock: &File,
    operation: impl FnOnce() -> Result<T, TraceError>,
) -> Result<T, TraceError> {
    let lock_path = directory.join(".rotation.lock");
    lock_bounded(lock, &lock_path)?;
    let result = operation();
    let unlock = lock
        .unlock()
        .map_err(|error| TraceError::io(&lock_path, error));
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

pub(crate) fn ensure_additional_capacity(
    directory: &Path,
    required: u64,
) -> Result<Vec<EvictedTrace>, TraceError> {
    reconcile_stale(&directory.join(".reserve"))?;
    evict_closed(directory, required, false)
}

pub(crate) fn write_reservation(
    sidecar: &mut File,
    path: &Path,
    value: u64,
) -> Result<(), TraceError> {
    write_sidecar(sidecar, path, value)
}

pub(crate) fn directory_usage(directory: &Path) -> Result<u64, TraceError> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(directory).map_err(|error| TraceError::io(directory, error))? {
        let entry = entry.map_err(|error| TraceError::io(directory, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            total = total.saturating_add(
                entry
                    .metadata()
                    .map_err(|error| TraceError::io(&path, error))?
                    .len(),
            );
        }
    }
    let reserve = directory.join(".reserve");
    if reserve.exists() {
        for entry in std::fs::read_dir(&reserve).map_err(|error| TraceError::io(&reserve, error))? {
            let entry = entry.map_err(|error| TraceError::io(&reserve, error))?;
            total = total.saturating_add(read_sidecar(&entry.path())?);
        }
    }
    Ok(total)
}

fn lock_bounded(file: &File, path: &Path) -> Result<(), TraceError> {
    let deadline = Instant::now() + LOCK_RETRY_LIMIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(TraceError::io(
                    path,
                    std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "trace coordination lock retry limit exceeded",
                    ),
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(TraceError::io(path, error));
            }
        }
    }
}

fn reconcile_stale(reserve_directory: &Path) -> Result<(), TraceError> {
    for entry in std::fs::read_dir(reserve_directory)
        .map_err(|error| TraceError::io(reserve_directory, error))?
    {
        let entry = entry.map_err(|error| TraceError::io(reserve_directory, error))?;
        let path = entry.path();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| TraceError::io(&path, error))?;
        match file.try_lock() {
            Ok(()) => {
                file.unlock()
                    .map_err(|error| TraceError::io(&path, error))?;
                std::fs::remove_file(&path).map_err(|error| TraceError::io(&path, error))?;
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(TraceError::io(&path, error));
            }
        }
    }
    Ok(())
}

fn evict_closed(
    directory: &Path,
    required: u64,
    reserve_file_slot: bool,
) -> Result<Vec<EvictedTrace>, TraceError> {
    let reserve = directory.join(".reserve");
    let mut closed = Vec::new();
    let mut retained_slots = 0_usize;
    for entry in std::fs::read_dir(directory).map_err(|error| TraceError::io(directory, error))? {
        let entry = entry.map_err(|error| TraceError::io(directory, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        retained_slots = retained_slots.saturating_add(1);
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !reserve.join(format!("{stem}.json")).exists() {
            let metadata = entry
                .metadata()
                .map_err(|error| TraceError::io(&path, error))?;
            let modified = metadata.modified().ok();
            closed.push((modified, path));
        }
    }
    for entry in std::fs::read_dir(&reserve).map_err(|error| TraceError::io(&reserve, error))? {
        let entry = entry.map_err(|error| TraceError::io(&reserve, error))?;
        let stem = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_owned();
        if !directory.join(format!("{stem}.jsonl")).exists() {
            retained_slots = retained_slots.saturating_add(1);
        }
    }
    closed.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut evicted = Vec::new();
    loop {
        let usage = directory_usage(directory)?;
        let within_count = if reserve_file_slot {
            retained_slots < TRACE_RETAINED_FILES_MAX
        } else {
            retained_slots <= TRACE_RETAINED_FILES_MAX
        };
        if within_count && usage.saturating_add(required) <= TRACE_DIRECTORY_BUDGET_BYTES {
            return Ok(evicted);
        }
        if closed.is_empty() {
            return Err(TraceError::BudgetExhausted {
                required,
                available: TRACE_DIRECTORY_BUDGET_BYTES.saturating_sub(usage),
            });
        }
        let (_, victim) = closed.remove(0);
        let encoded_bytes = std::fs::metadata(&victim)
            .map_err(|error| TraceError::io(&victim, error))?
            .len();
        std::fs::remove_file(&victim).map_err(|error| TraceError::io(&victim, error))?;
        retained_slots = retained_slots.saturating_sub(1);
        evicted.push(EvictedTrace {
            path: victim,
            encoded_bytes,
        });
    }
}

fn create_private_directory(path: &Path) -> Result<(), TraceError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        missing.push(cursor.to_owned());
        cursor = cursor.parent().ok_or_else(|| {
            TraceError::io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "directory has no parent"),
            )
        })?;
    }
    for directory in missing.into_iter().rev() {
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(TraceError::io(&directory, error)),
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| TraceError::io(&directory, error))?;
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| TraceError::io(path, error))
}

fn private_open(path: &Path, create_new: bool) -> Result<File, TraceError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(!create_new)
        .create_new(create_new)
        .mode(0o600)
        .open(path)
        .map_err(|error| TraceError::io(path, error))
}

fn write_sidecar(file: &mut File, path: &Path, value: u64) -> Result<(), TraceError> {
    let existing_bytes = file
        .metadata()
        .map_err(|error| TraceError::io(path, error))?
        .len();
    let previous = read_sidecar_file(file).map_err(|error| TraceError::io(path, error))?;
    if existing_bytes != 0 && previous.is_none() {
        return Err(TraceError::MalformedSidecar(path.to_owned()));
    }
    let generation = previous.map_or(1, |sidecar| sidecar.generation.saturating_add(1));
    let sidecar = Sidecar {
        generation,
        generation_complement: !generation,
        unused_reservation_bytes: value,
        unused_reservation_complement: !value,
    };
    let json = serde_json::to_string(&sidecar)?;
    if json.len() > SIDECAR_SLOT_BYTES {
        return Err(TraceError::MalformedSidecar(path.to_owned()));
    }
    let mut bytes = json.into_bytes();
    bytes.resize(SIDECAR_SLOT_BYTES, b' ');
    let slot = generation.saturating_sub(1) % 2;
    file.set_len(SIDECAR_BYTES as u64)
        .and_then(|()| file.seek(SeekFrom::Start(slot * SIDECAR_SLOT_BYTES as u64)))
        .and_then(|_| file.write_all(&bytes))
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_data())
        .map_err(|error| TraceError::io(path, error))
}

fn read_sidecar(path: &Path) -> Result<u64, TraceError> {
    let mut file = File::open(path).map_err(|error| TraceError::io(path, error))?;
    read_sidecar_file(&mut file)
        .map_err(|error| TraceError::io(path, error))?
        .map(|sidecar| sidecar.unused_reservation_bytes)
        .ok_or_else(|| TraceError::MalformedSidecar(path.to_owned()))
}

fn read_sidecar_file(file: &mut File) -> Result<Option<Sidecar>, std::io::Error> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut latest: Option<Sidecar> = None;
    for slot in bytes.chunks(SIDECAR_SLOT_BYTES).take(2) {
        let Ok(sidecar) = serde_json::from_slice::<Sidecar>(slot) else {
            continue;
        };
        if sidecar.generation_complement != !sidecar.generation
            || sidecar.unused_reservation_complement != !sidecar.unused_reservation_bytes
        {
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|current| sidecar.generation > current.generation)
        {
            latest = Some(sidecar);
        }
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use super::{SIDECAR_SLOT_BYTES, read_sidecar, write_sidecar};

    #[test]
    fn torn_new_sidecar_slot_falls_back_to_last_complete_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reservation.json");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        write_sidecar(&mut file, &path, 10).unwrap();
        write_sidecar(&mut file, &path, 20).unwrap();
        file.seek(SeekFrom::Start(SIDECAR_SLOT_BYTES as u64))
            .unwrap();
        file.write_all(b"torn").unwrap();
        file.sync_data().unwrap();
        assert_eq!(read_sidecar(&path).unwrap(), 10);
        write_sidecar(&mut file, &path, 30).unwrap();
        assert_eq!(read_sidecar(&path).unwrap(), 30);
    }
}
