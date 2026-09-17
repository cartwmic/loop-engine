//! Local invocation admission and native process-incarnation ownership.
//!
//! The lock is held only for publication/admission and completion arbitration,
//! never while waiting for primary work.  Numeric PIDs and process-group
//! numbers are recyclable, so all control decisions use the native process
//! reader below together with the engine-authored boot/start identity.
use loop_core::{OwnedExecution, ProcessIdentity};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
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
        let Some(identity) = ownership.root_identity.as_ref() else {
            return Err(io::Error::other(
                "cannot publish ownership without a native root identity",
            ));
        };
        if identity.pid != ownership.root_pid {
            return Err(io::Error::other(
                "native root identity PID does not match recorded root PID",
            ));
        }
        if identity.pid <= 1 || ownership.process_group_id == 0 {
            return Err(io::Error::other("invalid native ownership identity"));
        }
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

/// A native process table row. The numeric fields are topology facts; the
/// identity is the authority for deciding whether this row is the recorded
/// incarnation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRecord {
    pub identity: ProcessIdentity,
    pub parent_pid: u32,
    pub process_group_id: u32,
    /// Platform-normalized state. `T` means stopped and `Z` means zombie;
    /// other values are diagnostic only.
    pub state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedProcess {
    pub identity: ProcessIdentity,
    pub process_group_id: u32,
}

/// Native ownership resolution for one process-table snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedProcessSnapshot {
    pub records: Vec<RecordedProcess>,
    pub live: Vec<ProcessRecord>,
    pub anchored_groups: BTreeSet<u32>,
    pub identity_usable: bool,
}

/// Read one process through the platform-native process interface.
pub fn read_process(pid: u32) -> io::Result<Option<ProcessRecord>> {
    native::read_process(pid)
}

/// Read the native incarnation identity for `pid`.
pub fn read_process_identity(pid: u32) -> io::Result<Option<ProcessIdentity>> {
    Ok(read_process(pid)?.map(|process| process.identity))
}

/// Read the current process identity. New ownership publication uses this
/// rather than treating the current numeric PID as sufficient evidence.
pub fn current_process_identity() -> io::Result<ProcessIdentity> {
    read_process_identity(std::process::id())?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "current process has no readable native identity",
        )
    })
}

/// Read one complete native process snapshot. This is the sole process-table
/// reader used by ownership and cancellation; no shell-formatted `ps` output
/// is parsed for identity or topology.
pub fn read_processes() -> io::Result<Vec<ProcessRecord>> {
    native::read_processes()
}

/// Compare an expected incarnation with the process currently occupying its
/// PID. A missing or different incarnation is simply not the owned process.
pub fn process_identity_matches(pid: u32, expected: Option<&ProcessIdentity>) -> io::Result<bool> {
    let Some(expected) = expected else {
        return Ok(false);
    };
    if expected.pid != pid {
        return Ok(false);
    }
    Ok(read_process_identity(pid)?.is_some_and(|actual| actual == *expected))
}

/// Recheck an identity immediately before signaling it. A PID that vanished or
/// changed incarnation is not an error and is never signaled.
pub fn signal_process(identity: &ProcessIdentity, signal: i32) -> io::Result<bool> {
    if identity.pid <= 1 || identity.pid == std::process::id() {
        return Err(io::Error::other("invalid owned process identity"));
    }
    if !process_identity_matches(identity.pid, Some(identity))? {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: libc::pid_t, signal: libc::c_int) -> libc::c_int;
        }
        if unsafe { kill(identity.pid as libc::pid_t, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(false);
            }
            return Err(error);
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        Err(io::Error::other(
            "local cancellation unsupported on this platform",
        ))
    }
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
    let helper = read_process(pid)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not read native identity for admitting helper",
        )
    })?;
    admission.write(
        &format!("helper-{pid}.json"),
        &serde_json::json!({
            "root_pid": pid,
            "process_group_id": helper.process_group_id,
            "identity": helper.identity,
        }),
    )?;
    let mut child = command.spawn()?;
    match read_process_identity(child.id()) {
        Ok(Some(identity)) => {
            if let Err(error) = admission.write(
                &format!("child-{}.json", child.id()),
                &serde_json::json!({
                    "root_pid": child.id(),
                    "parent_pid": pid,
                    "process_group_id": helper.process_group_id,
                    "identity": identity,
                }),
            ) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        Ok(None) => {
            // A very short child can exit before its marker is written. The
            // admitting helper remains the durable anchor and owns the wait.
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
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
        serde_json::json!({"request":read(directory.join("stop.json"))?,"attempts":attempts,
        "acknowledgment": if directory.join("cleanup-acknowledged.json").exists() {
            read(directory.join("cleanup-acknowledged.json"))?
        } else { serde_json::Value::Null }}),
    ))
}

/// Whether the recorded ownership has enough identity material for control.
/// Numeric-only or partially written ownership is deliberately unavailable,
/// not permission to adopt a live PID or process group.
pub fn identity_available(capture_dir: &str) -> io::Result<bool> {
    Ok(read_recorded_processes(&directory(capture_dir))?.1)
}

/// Resolve recorded process incarnations against one native snapshot.
///
/// A group number is used only after at least one recorded identity is live in
/// that same group. Recorded identities are also matched independently, so a
/// surviving child remains owned after its root disappears.
pub fn discover_owned_processes(
    directory: &Path,
    current: &[ProcessRecord],
) -> io::Result<OwnedProcessSnapshot> {
    let (records, mut identity_usable) = read_recorded_processes(directory)?;
    if !identity_usable {
        return Ok(OwnedProcessSnapshot {
            records,
            live: Vec::new(),
            anchored_groups: BTreeSet::new(),
            identity_usable: false,
        });
    }

    // Some macOS processes are hidden from an unprivileged process-table
    // listing. Re-read every recorded PID directly so an inaccessible genuine
    // owner is conservatively unavailable rather than silently considered gone.
    let mut processes = current.to_vec();
    for recorded in &records {
        if processes
            .iter()
            .any(|process| process.identity == recorded.identity)
            || processes
                .iter()
                .any(|process| process.identity.pid == recorded.identity.pid)
        {
            continue;
        }
        match read_process(recorded.identity.pid) {
            Ok(Some(process)) => processes.push(process),
            Ok(None) => {}
            Err(_) => identity_usable = false,
        }
    }
    if !identity_usable {
        return Ok(OwnedProcessSnapshot {
            records,
            live: Vec::new(),
            anchored_groups: BTreeSet::new(),
            identity_usable: false,
        });
    }

    let mut owned = BTreeSet::new();
    let mut anchored_groups = BTreeSet::new();
    for recorded in &records {
        if let Some(process) = processes
            .iter()
            .find(|process| process.identity == recorded.identity)
        {
            owned.insert(process.identity.clone());
            if recorded.process_group_id != 0
                && process.process_group_id == recorded.process_group_id
            {
                anchored_groups.insert(recorded.process_group_id);
            }
        }
    }

    loop {
        let mut changed = false;
        for process in &processes {
            let parent_owned = processes
                .iter()
                .find(|parent| parent.identity.pid == process.parent_pid)
                .is_some_and(|parent| owned.contains(&parent.identity));
            if owned.contains(&process.identity)
                || parent_owned
                || anchored_groups.contains(&process.process_group_id)
            {
                changed |= owned.insert(process.identity.clone());
            }
        }
        if !changed {
            break;
        }
    }

    let mut live = processes
        .iter()
        .filter(|process| owned.contains(&process.identity))
        .cloned()
        .collect::<Vec<_>>();
    live.sort_by_key(|process| process.identity.clone());
    Ok(OwnedProcessSnapshot {
        records,
        live,
        anchored_groups,
        identity_usable: true,
    })
}

fn read_recorded_processes(directory: &Path) -> io::Result<(Vec<RecordedProcess>, bool)> {
    if !directory.is_dir() {
        return Ok((Vec::new(), false));
    }
    let mut records = Vec::new();
    let mut identity_usable = true;
    let mut saw_ownership = false;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "ownership.json" {
            saw_ownership = true;
            let owned: OwnedExecution = serde_json::from_slice(&fs::read(entry.path())?)?;
            let Some(identity) = owned.root_identity else {
                identity_usable = false;
                continue;
            };
            if identity.pid != owned.root_pid {
                // The identity is authoritative; retaining it still lets a
                // controlled stale-number case prove that the numeric field is
                // not used for signaling.
            }
            if identity.pid <= 1 {
                identity_usable = false;
            }
            records.push(RecordedProcess {
                identity,
                process_group_id: owned.process_group_id,
            });
            if owned.process_group_id == 0 {
                identity_usable = false;
            }
            continue;
        }
        if name.starts_with("observed-stop-")
            || !name.ends_with(".json")
            || !(name.starts_with("helper-")
                || name.starts_with("child-")
                || name.starts_with("observed-"))
        {
            // Only atomically published marker names are ownership input.
            // Admission writes `<name>.tmp` before the final rename; an
            // interrupted publication must not turn into an EOF/invalid
            // ownership decision.
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(entry.path())?)?;
        let identity = value
            .get("identity")
            .or_else(|| value.get("observation").and_then(|v| v.get("identity")))
            .cloned();
        let Some(identity) = identity else {
            identity_usable = false;
            continue;
        };
        let identity: ProcessIdentity = serde_json::from_value(identity)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let process_group_id = value
            .get("process_group_id")
            .or_else(|| {
                value
                    .get("observation")
                    .and_then(|v| v.get("process_group_id"))
            })
            .or_else(|| value.get("observation").and_then(|v| v.get("group")))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        if identity.pid <= 1 || process_group_id == 0 {
            identity_usable = false;
        }
        records.push(RecordedProcess {
            identity,
            process_group_id,
        });
    }
    records.sort_by_key(|record| record.identity.clone());
    records.dedup_by(|left, right| left.identity == right.identity);
    identity_usable &= saw_ownership && !records.is_empty();
    Ok((records, identity_usable))
}

pub fn live_owned_work(capture_dir: &str) -> io::Result<bool> {
    let directory = directory(capture_dir);
    if !directory.exists() || directory.join("cleanup-acknowledged.json").exists() {
        return Ok(false);
    }
    let current = read_processes()?;
    let snapshot = discover_owned_processes(&directory, &current)?;
    if !snapshot.identity_usable {
        // Unknown ownership is conservatively live. Callers that need to
        // control it separately reject it as unavailable rather than signaling.
        return Ok(true);
    }
    Ok(!snapshot.live.is_empty())
}

#[cfg(target_os = "linux")]
mod native {
    use super::{ProcessIdentity, ProcessRecord};
    use std::fs;
    use std::io;
    use std::path::Path;

    fn boot_id() -> io::Result<String> {
        let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let value = value.trim().to_owned();
        if value.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Linux boot identity is empty",
            ));
        }
        Ok(value)
    }

    fn read_process_with_boot(pid: u32, boot_id: &str) -> io::Result<Option<ProcessRecord>> {
        let path = Path::new("/proc").join(pid.to_string()).join("stat");
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let close = raw.rfind(')').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "malformed Linux process stat")
        })?;
        let fields = raw
            .get(close + 1..)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed process stat"))?
            .split_whitespace()
            .collect::<Vec<_>>();
        // After the executable name, fields[0] is state, fields[1] ppid,
        // fields[2] pgrp, and fields[19] is field 22: starttime.
        if fields.len() <= 19 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short Linux process stat",
            ));
        }
        let parse = |index: usize, name: &str| {
            fields[index].parse::<u64>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Linux process {name}: {error}"),
                )
            })
        };
        let parent_pid = u32::try_from(parse(1, "parent")?).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("parent PID overflow: {error}"),
            )
        })?;
        let process_group_id = u32::try_from(parse(2, "group")?).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("process group overflow: {error}"),
            )
        })?;
        let start_time = parse(19, "start time")?;
        Ok(Some(ProcessRecord {
            identity: ProcessIdentity::new(pid, boot_id.to_owned(), start_time),
            parent_pid,
            process_group_id,
            state: fields[0].to_owned(),
        }))
    }

    pub(super) fn read_process(pid: u32) -> io::Result<Option<ProcessRecord>> {
        read_process_with_boot(pid, &boot_id()?)
    }

    pub(super) fn read_processes() -> io::Result<Vec<ProcessRecord>> {
        let boot_id = boot_id()?;
        let mut processes = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            match read_process_with_boot(pid, &boot_id) {
                Ok(Some(process)) => processes.push(process),
                Ok(None) => {}
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES)) => {}
                Err(error) => return Err(error),
            }
        }
        processes.sort_by_key(|process| process.identity.pid);
        Ok(processes)
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{ProcessIdentity, ProcessRecord};
    use std::io;
    use std::mem::MaybeUninit;
    use std::ptr;

    fn boot_id() -> io::Result<String> {
        let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
        let mut boot = MaybeUninit::<libc::timeval>::zeroed();
        let mut length = std::mem::size_of::<libc::timeval>();
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                boot.as_mut_ptr().cast(),
                &mut length,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let boot = unsafe { boot.assume_init() };
        Ok(format!("{}:{}", boot.tv_sec, boot.tv_usec))
    }

    fn normalized_state(status: u32) -> String {
        match status {
            libc::SZOMB => "Z".to_owned(),
            libc::SSTOP => "T".to_owned(),
            libc::SRUN => "R".to_owned(),
            libc::SSLEEP => "S".to_owned(),
            libc::SIDL => "I".to_owned(),
            other => format!("?{other}"),
        }
    }

    fn bsd_process_record(info: libc::proc_bsdinfo, boot_id: &str) -> ProcessRecord {
        ProcessRecord {
            identity: ProcessIdentity::new(
                info.pbi_pid,
                boot_id.to_owned(),
                info.pbi_start_tvsec
                    .saturating_mul(1_000_000)
                    .saturating_add(info.pbi_start_tvusec),
            ),
            parent_pid: info.pbi_ppid,
            process_group_id: info.pbi_pgid,
            state: normalized_state(info.pbi_status),
        }
    }

    // `kinfo_proc` is not exposed by the libc crate on Apple targets. Keep the
    // ABI layout local for the fields needed by the KERN_PROC_PID fallback.
    // The fallback is used only for the ordinary case where libproc denies a
    // row; if this layout ever stops matching the kernel, the short read
    // below fails closed instead of making a numeric identity claim.
    #[repr(C)]
    #[allow(dead_code)]
    struct MacExternProc {
        p_un: libc::timeval,
        p_vmspace: *mut libc::c_void,
        p_sigacts: *mut libc::c_void,
        p_flag: libc::c_int,
        p_stat: libc::c_char,
        p_pid: libc::pid_t,
        p_oppid: libc::pid_t,
        p_dupfd: libc::c_int,
        user_stack: *mut libc::c_void,
        exit_thread: *mut libc::c_void,
        p_debugger: libc::c_int,
        sigwait: libc::boolean_t,
        p_estcpu: libc::c_uint,
        p_cpticks: libc::c_int,
        p_pctcpu: libc::c_uint,
        p_wchan: *mut libc::c_void,
        p_wmesg: *mut libc::c_char,
        p_swtime: libc::c_uint,
        p_slptime: libc::c_uint,
        p_realtimer: libc::itimerval,
        p_rtime: libc::timeval,
        p_uticks: libc::u_quad_t,
        p_sticks: libc::u_quad_t,
        p_iticks: libc::u_quad_t,
        p_traceflag: libc::c_int,
        p_tracep: *mut libc::c_void,
        p_siglist: libc::c_int,
        p_textvp: *mut libc::c_void,
        p_holdcnt: libc::c_int,
        p_sigmask: libc::sigset_t,
        p_sigignore: libc::sigset_t,
        p_sigcatch: libc::sigset_t,
        p_priority: libc::c_uchar,
        p_usrpri: libc::c_uchar,
        p_nice: libc::c_char,
        p_comm: [libc::c_char; libc::MAXCOMLEN + 1],
        p_pgrp: *mut libc::c_void,
        p_addr: *mut libc::c_void,
        p_xstat: libc::c_ushort,
        p_acflag: libc::c_ushort,
        p_ru: *mut libc::c_void,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct MacPCred {
        pc_lock: [libc::c_char; 72],
        pc_ucred: *mut libc::c_void,
        p_ruid: libc::uid_t,
        p_svuid: libc::uid_t,
        p_rgid: libc::gid_t,
        p_svgid: libc::gid_t,
        p_refcnt: libc::c_int,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct MacUcred {
        cr_ref: libc::c_int,
        cr_uid: libc::uid_t,
        cr_ngroups: libc::c_short,
        cr_groups: [libc::gid_t; 16],
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct MacVmspace {
        dummy: i32,
        dummy2: *mut libc::c_void,
        dummy3: [i32; 5],
        dummy4: [*mut libc::c_void; 3],
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct MacEproc {
        e_paddr: *mut libc::c_void,
        e_sess: *mut libc::c_void,
        e_pcred: MacPCred,
        e_ucred: MacUcred,
        e_vm: MacVmspace,
        e_ppid: libc::pid_t,
        e_pgid: libc::pid_t,
        e_jobc: libc::c_short,
        e_tdev: libc::dev_t,
        e_tpgid: libc::pid_t,
        e_tsess: *mut libc::c_void,
        e_wmesg: [libc::c_char; 8],
        e_xsize: i32,
        e_xrssize: libc::c_short,
        e_xccount: libc::c_short,
        e_xswrss: libc::c_short,
        e_flag: i32,
        e_login: [libc::c_char; 12],
        e_spare: [i32; 4],
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct MacKinfoProc {
        kp_proc: MacExternProc,
        kp_eproc: MacEproc,
    }

    fn read_process_from_sysctl(
        pid: libc::pid_t,
        boot_id: &str,
    ) -> io::Result<Option<ProcessRecord>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        // Ask the kernel for its complete record rather than passing the
        // Rust layout size as the buffer length. An unexpected record size
        // below is rejected, so a changed Darwin ABI fails closed.
        let mut buffer = [0_u64; 512];
        let mut length = std::mem::size_of_val(&buffer);
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                buffer.as_mut_ptr().cast(),
                &mut length,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                None | Some(0) | Some(libc::ESRCH) | Some(libc::ENOENT)
            ) {
                return Ok(None);
            }
            return Err(error);
        }
        if length != std::mem::size_of::<MacKinfoProc>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected macOS KERN_PROC process info size: got {length}, expected {}",
                    std::mem::size_of::<MacKinfoProc>()
                ),
            ));
        }
        let info = unsafe { &*buffer.as_ptr().cast::<MacKinfoProc>() };
        if info.kp_proc.p_pid != pid {
            return Ok(None);
        }
        let start_time = u64::try_from(info.kp_proc.p_un.tv_sec)
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000_000))
            .and_then(|seconds| {
                u64::try_from(info.kp_proc.p_un.tv_usec)
                    .ok()
                    .and_then(|micros| seconds.checked_add(micros))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid macOS KERN_PROC process start time",
                )
            })?;
        let parent_pid = u32::try_from(info.kp_eproc.e_ppid).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid macOS KERN_PROC parent PID: {error}"),
            )
        })?;
        let process_group_id = u32::try_from(info.kp_eproc.e_pgid).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid macOS KERN_PROC process group: {error}"),
            )
        })?;
        Ok(Some(ProcessRecord {
            identity: ProcessIdentity::new(pid as u32, boot_id.to_owned(), start_time),
            parent_pid,
            process_group_id,
            state: normalized_state(info.kp_proc.p_stat as u8 as u32),
        }))
    }

    fn read_process_with_boot(pid: u32, boot_id: &str) -> io::Result<Option<ProcessRecord>> {
        let pid = libc::pid_t::try_from(pid).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("PID overflow: {error}"),
            )
        })?;
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let result = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
            )
        };
        if result <= 0 {
            let error = io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES)) {
                return read_process_from_sysctl(pid, boot_id);
            }
            if matches!(
                error.raw_os_error(),
                None | Some(0) | Some(libc::ESRCH) | Some(libc::ENOENT)
            ) {
                return Ok(None);
            }
            return Err(error);
        }
        if result < std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short macOS process info",
            ));
        }
        Ok(Some(bsd_process_record(
            unsafe { info.assume_init() },
            boot_id,
        )))
    }

    pub(super) fn read_process(pid: u32) -> io::Result<Option<ProcessRecord>> {
        read_process_with_boot(pid, &boot_id()?)
    }

    pub(super) fn read_processes() -> io::Result<Vec<ProcessRecord>> {
        let boot_id = boot_id()?;
        let count = unsafe { libc::proc_listallpids(ptr::null_mut(), 0) };
        if count <= 0 {
            return Ok(Vec::new());
        }
        let mut pids = vec![0 as libc::pid_t; count as usize + 16];
        let result = loop {
            let result = unsafe {
                libc::proc_listallpids(
                    pids.as_mut_ptr().cast(),
                    (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
                )
            };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            if result as usize >= pids.len() {
                pids.resize(result as usize + 16, 0);
                continue;
            }
            break result as usize;
        };
        let mut processes = Vec::new();
        for pid in pids.into_iter().take(result) {
            if pid <= 0 {
                continue;
            }
            match read_process_with_boot(pid as u32, &boot_id) {
                Ok(Some(process)) => processes.push(process),
                Ok(None) => {}
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES)) => {}
                Err(error) => return Err(error),
            }
        }
        processes.sort_by_key(|process| process.identity.pid);
        Ok(processes)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod native {
    use super::{ProcessIdentity, ProcessRecord};
    use std::io;

    pub(super) fn read_process(_pid: u32) -> io::Result<Option<ProcessRecord>> {
        Err(io::Error::other(
            "native process identity is unsupported on this platform",
        ))
    }

    pub(super) fn read_processes() -> io::Result<Vec<ProcessRecord>> {
        Err(io::Error::other(
            "native process identity is unsupported on this platform",
        ))
    }

    #[allow(dead_code)]
    fn _keep_identity_type(_: ProcessIdentity) {}
}
