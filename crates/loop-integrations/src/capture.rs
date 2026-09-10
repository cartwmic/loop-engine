//! Provider-free serial command capture. Receipts are execution facts, not verdicts.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::{
    ffi::OsStrExt,
    fs::PermissionsExt,
    process::{CommandExt, ExitStatusExt},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}
unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
    fn flock(fd: i32, operation: i32) -> i32;
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub inherit_environment: Vec<String>,
    pub timeout_ms: u64,
    pub obligations: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub rows: Vec<Row>,
}
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_json(path: &Path) -> io::Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn immutable(path: &Path, value: &Value) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    f.write_all(b"\n")?;
    f.sync_all()
}
fn atomic(path: &Path, value: &Value) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = File::create(&tmp)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(path.parent().unwrap())?.sync_all()
}
fn git(repo: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let result = Command::new("git").args(args).current_dir(repo).output()?;
    if !result.status.success() {
        return Err(error(String::from_utf8_lossy(&result.stderr)));
    }
    Ok(result.stdout)
}
/// Byte-for-byte counterpart of scripts/test_contract.py; public fixture checks parity.
pub fn repository_proof_identity(repo: &Path) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut add = |bytes: &[u8]| {
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    };
    add(&git(repo, &["rev-parse", "HEAD"])?);
    add(&git(repo, &["ls-files", "--stage", "-z"])?);
    add(&git(
        repo,
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?);
    let names = git(
        repo,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    for name in names
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>()
    {
        add(name);
        let path = repo.join(std::ffi::OsStr::from_bytes(name));
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                add(b"symlink");
                add(fs::read_link(path)?.as_os_str().as_bytes());
            }
            Ok(meta) if meta.is_file() => {
                add((meta.permissions().mode() & 0o777).to_string().as_bytes());
                add(&fs::read(path)?);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => add(b"deleted"),
            Err(e) => return Err(e),
            _ => {
                return Err(error(format!(
                    "unsupported repository proof entry: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

// Hold a kernel-released lock, not a stale PID lock. A lost controller leaves
// started state unresolved; abort never guesses that its work disappeared.
struct Lock(File);
impl Lock {
    fn acquire(root: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("controller.lock"))?;
        if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
            return Err(error(
                "capture controller active; use capture-abort or wait",
            ));
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            flock(self.0.as_raw_fd(), 8);
        }
    }
}

fn settings(row: &Row) -> Value {
    let inherited: BTreeMap<_, _> = row
        .inherit_environment
        .iter()
        .map(|key| (key, std::env::var(key).ok()))
        .collect();
    json!({"environment":row.environment,"inherited_environment":inherited,"timeout_ms":row.timeout_ms})
}
fn resolve(row: &Row, cwd: &Path) -> Option<PathBuf> {
    let executable = &row.argv[0];
    let candidate = |p: PathBuf| {
        if p.is_file() && fs::metadata(&p).ok()?.permissions().mode() & 0o111 != 0 {
            fs::canonicalize(p).ok()
        } else {
            None
        }
    };
    if executable.contains('/') {
        return candidate(cwd.join(executable));
    }
    let path = row
        .environment
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())?;
    std::env::split_paths(&path).find_map(|p| candidate(cwd.join(p).join(executable)))
}
fn validate(matrix: &Matrix) -> io::Result<()> {
    let mut ids = BTreeSet::new();
    if matrix.rows.is_empty() {
        return Err(error("matrix requires rows"));
    }
    for r in &matrix.rows {
        if r.id.is_empty()
            || !ids.insert(&r.id)
            || r.argv.is_empty()
            || r.argv[0].is_empty()
            || r.timeout_ms == 0
            || r.obligations.iter().any(String::is_empty)
        {
            return Err(error("invalid or duplicate matrix row"));
        }
        let mut keys = BTreeSet::new();
        for key in r.environment.keys().chain(&r.inherit_environment) {
            if key.is_empty() || key.contains(['=', '\0']) || key == "LOOP_CAPTURE_OWNER" {
                return Err(error("invalid/reserved environment name"));
            }
        }
        if r.inherit_environment.iter().any(|k| !keys.insert(k))
            || r.argv
                .iter()
                .chain(r.environment.values())
                .any(|s| s.contains('\0'))
        {
            return Err(error("invalid argv/environment"));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Process {
    pid: i32,
    parent: i32,
    zombie: bool,
    started: String,
}
fn inventory() -> io::Result<Vec<Process>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,stat=,lstart="])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()?;
    if !output.status.success() {
        return Err(error("process inventory unavailable"));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            if f.len() != 8 {
                return Err(error("malformed process inventory"));
            }
            Ok(Process {
                pid: f[0].parse().map_err(|_| error("pid"))?,
                parent: f[1].parse().map_err(|_| error("parent"))?,
                zombie: f[2].starts_with('Z'),
                started: f[3..].join(" "),
            })
        })
        .collect()
}
// Inherited marker locates ordinary setsid/double-fork descendants even after
// reparenting. Inspect only in memory: never persist ps environment or secrets.
fn marked(token: &str, snapshot: &[Process]) -> io::Result<BTreeSet<i32>> {
    let marker = format!("LOOP_CAPTURE_OWNER={token}");
    let mut found = BTreeSet::new();
    #[cfg(target_os = "linux")]
    for p in snapshot {
        if let Ok(bytes) = fs::read(format!("/proc/{}/environ", p.pid)) {
            if bytes.split(|b| *b == 0).any(|v| v == marker.as_bytes()) {
                found.insert(p.pid);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = snapshot;
        let output = Command::new("ps")
            .args(["eww", "-axo", "pid=,command="])
            .output()?;
        if !output.status.success() {
            return Err(error("owned descendant inventory unavailable"));
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.split_whitespace().any(|s| s == marker) {
                if let Some(pid) = line.split_whitespace().next().and_then(|s| s.parse().ok()) {
                    found.insert(pid);
                }
            }
        }
    }
    Ok(found)
}
// Start stamps fence stale numeric history during this controller's lifetime.
// ps has second resolution: this is not atomic signaling or protection against
// same-second PID reuse. Interrupted captures are never adopted from these files.
fn owned(
    token: &str,
    root_pid: i32,
    root_unreaped: bool,
    known: &mut BTreeMap<i32, String>,
) -> io::Result<Vec<Process>> {
    let snapshot = inventory()?;
    let mut current: BTreeSet<_> = snapshot
        .iter()
        .filter(|p| known.get(&p.pid) == Some(&p.started))
        .map(|p| p.pid)
        .collect();
    // Establish the root while Child still owns its unreaped process identity,
    // never by looking up a number for the first time after try_wait reaps it.
    if root_unreaped && !known.contains_key(&root_pid) {
        if !snapshot.iter().any(|p| p.pid == root_pid) {
            return Err(error(format!(
                "tracked child {root_pid} missing from inventory"
            )));
        }
        current.insert(root_pid);
    }
    current.extend(marked(token, &snapshot)?);
    loop {
        let before = current.len();
        for p in &snapshot {
            if current.contains(&p.parent) {
                current.insert(p.pid);
            }
        }
        if before == current.len() {
            break;
        }
    }
    let live: Vec<_> = snapshot
        .into_iter()
        .filter(|p| current.contains(&p.pid))
        .collect();
    for p in &live {
        known.insert(p.pid, p.started.clone());
    }
    Ok(live)
}
fn send(process: &Process, sig: i32) -> io::Result<()> {
    let pid = process.pid;
    if pid <= 1 || pid == std::process::id() as i32 {
        return Err(error("invalid owned pid"));
    }
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()?;
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(()); // This process disappeared before signaling.
        }
        return Err(error(format!(
            "cannot verify owned process {pid} before signal"
        )));
    }
    let started = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if started != process.started {
        return Ok(()); // The current occupant is not the observed process.
    }
    if unsafe { kill(pid, sig) } != 0 && io::Error::last_os_error().raw_os_error() != Some(3) {
        return Err(error(format!(
            "signal {sig} to owned process {pid}: {}",
            io::Error::last_os_error()
        )));
    }
    Ok(())
}
fn pump(
    mut input: impl Read + std::os::fd::AsRawFd + Send + 'static,
    path: PathBuf,
    stderr: bool,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let result = (|| {
            #[cfg(target_os = "macos")]
            let nonblock = 4;
            #[cfg(not(target_os = "macos"))]
            let nonblock = 2048;
            let flags = unsafe { fcntl(input.as_raw_fd(), 3) };
            if flags < 0 || unsafe { fcntl(input.as_raw_fd(), 4, flags | nonblock) } < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut file = OpenOptions::new().append(true).open(path)?;
            let mut bytes = [0; 8192];
            loop {
                if stop.load(Ordering::SeqCst) {
                    file.sync_all()?;
                    return Err(error("capture stopped with unresolved cleanup"));
                }
                let n = match input.read(&mut bytes) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                if n == 0 {
                    return file.sync_all();
                }
                file.write_all(&bytes[..n])?;
                file.flush()?;
                if stderr {
                    let mut out = io::stderr().lock();
                    out.write_all(&bytes[..n])?;
                    out.flush()?;
                } else {
                    let mut out = io::stdout().lock();
                    out.write_all(&bytes[..n])?;
                    out.flush()?;
                }
            }
        })();
        if result.is_err() {
            failed.store(true, Ordering::SeqCst);
        }
        result
    })
}

/// Shared by single/matrix callers and provider validation integration. Streams
/// are live; the returned value is the actual immutable child receipt.
pub fn execute_row(row: &Row, cwd: &Path, attempt: &Path, root: &Path) -> io::Result<Value> {
    execute_row_with_forwarding(row, cwd, attempt, root, false)
}

fn execute_row_with_forwarding(
    row: &Row,
    cwd: &Path,
    attempt: &Path,
    root: &Path,
    stdout_to_stderr: bool,
) -> io::Result<Value> {
    fs::create_dir(attempt)?;
    let before = repository_proof_identity(cwd)?;
    let executable = resolve(row, cwd);
    let token = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut receipt = json!({"id":row.id,"argv":row.argv,"resolved_executable":executable,"cwd":cwd,"settings":settings(row),"obligations":row.obligations,"repository_before":before,"started_at":now()});
    immutable(&attempt.join("started.json"), &receipt)?;
    File::create(attempt.join("stdout"))?.sync_all()?;
    File::create(attempt.join("stderr"))?.sync_all()?;
    atomic(
        &root.join("state.json"),
        &json!({"status":"running","attempt":attempt,"owner_token":token,"cleanup":"pending"}),
    )?;
    let started = Instant::now();
    let mut command = Command::new(executable.as_deref().unwrap_or(Path::new(&row.argv[0])));
    command
        .arg0(&row.argv[0])
        .args(&row.argv[1..])
        .current_dir(cwd)
        .envs(&row.environment)
        .env("LOOP_CAPTURE_OWNER", &token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let interrupted_before_spawn =
        INTERRUPTED.load(Ordering::SeqCst) || root.join("abort-request.json").exists();
    let child = if interrupted_before_spawn {
        Err(error("capture aborted before spawn"))
    } else if executable.is_some() {
        command.spawn()
    } else {
        Err(error("executable not found or not executable"))
    };
    let mut timed_out = false;
    let mut aborted = interrupted_before_spawn;
    let mut cleanup = "complete";
    let mut capture_error = None;
    let (status, spawn_error) = match child {
        Err(e) => (None, Some(e.to_string())),
        Ok(mut child) => {
            let pid = child.id() as i32;
            atomic(
                &root.join("state.json"),
                &json!({"status":"running","attempt":attempt,"owner_token":token,"root_pid":pid,"cleanup":"pending"}),
            )?;
            let stop_streams = Arc::new(AtomicBool::new(false));
            let stream_failure = Arc::new(AtomicBool::new(false));
            let stdout = pump(
                child.stdout.take().unwrap(),
                attempt.join("stdout"),
                stdout_to_stderr,
                stop_streams.clone(),
                stream_failure.clone(),
            );
            let stderr = pump(
                child.stderr.take().unwrap(),
                attempt.join("stderr"),
                true,
                stop_streams.clone(),
                stream_failure.clone(),
            );
            let mut known = BTreeMap::new();
            let mut result = None;
            let mut stopping = None;
            loop {
                let live = match owned(&token, pid, result.is_none(), &mut known) {
                    Ok(live) => live,
                    Err(e) => {
                        capture_error = Some(e.to_string());
                        cleanup = "pending";
                        if result.is_none() {
                            let _ = child.kill();
                            result = child.wait().ok();
                        }
                        break;
                    }
                };
                if result.is_none() {
                    result = child.try_wait()?;
                }
                timed_out |=
                    started.elapsed() >= Duration::from_millis(row.timeout_ms) && result.is_none();
                aborted |=
                    INTERRUPTED.load(Ordering::SeqCst) || root.join("abort-request.json").exists();
                atomic(
                    &root.join("owned.json"),
                    &json!({"ownership_format":"pid-start-v1","owner_token":token,"root_pid":pid,"observed_pids":known.keys().collect::<Vec<_>>(),"process_starts":known}),
                )?;
                if live.is_empty() && result.is_some() {
                    break;
                }
                // Terminal child with still-owned descendants is not quiescence.
                if timed_out || aborted || result.is_some() || stream_failure.load(Ordering::SeqCst)
                {
                    stopping.get_or_insert_with(Instant::now);
                }
                if let Some(stop) = stopping {
                    if stop.elapsed() >= Duration::from_secs(10) {
                        cleanup = "pending";
                        break;
                    }
                    for p in &live {
                        if !p.zombie && !live.iter().any(|c| c.parent == p.pid && !c.zombie) {
                            if let Err(e) = send(
                                p,
                                if stop.elapsed() < Duration::from_secs(3) {
                                    15
                                } else {
                                    9
                                },
                            ) {
                                capture_error = Some(e.to_string());
                                cleanup = "pending";
                                break;
                            }
                        }
                    }
                    if cleanup == "pending" {
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(25));
            }
            if cleanup != "complete" {
                stop_streams.store(true, Ordering::SeqCst);
            }
            for handle in [stdout, stderr] {
                if let Err(e) = handle
                    .join()
                    .unwrap_or_else(|_| Err(error("stream capture thread panicked")))
                {
                    capture_error.get_or_insert_with(|| e.to_string());
                }
            }
            (result, None)
        }
    };
    let after = repository_proof_identity(cwd)?;
    let object = receipt.as_object_mut().unwrap();
    object.extend(json!({"repository_after":after,"finished_at":now(),"wall_seconds":started.elapsed().as_secs_f64(),"exit_code":status.and_then(|s|s.code()),"signal":status.and_then(|s|s.signal()),"timed_out":timed_out,"aborted":aborted,"spawn_error":spawn_error,"capture_error":capture_error,"cleanup":cleanup,"stdout":"stdout","stderr":"stderr","stdout_sha256":hash(&fs::read(attempt.join("stdout"))?),"stderr_sha256":hash(&fs::read(attempt.join("stderr"))?)}).as_object().unwrap().clone());
    immutable(&attempt.join("receipt.json"), &receipt)?;
    Ok(receipt)
}
/// Read a selected immutable receipt without launching, resuming or changing it.
pub fn verify_selected(index_path: &Path, row: &Row, cwd: &Path) -> io::Result<(PathBuf, Value)> {
    let index = read_json(index_path)?;
    let matches: Vec<_> = index["receipts"]
        .as_array()
        .ok_or_else(|| error("missing capture selection"))?
        .iter()
        .filter(|s| s["id"] == row.id)
        .collect();
    if matches.len() != 1 {
        return Err(error("missing or duplicate selected receipt"));
    }
    let root = index_path
        .parent()
        .ok_or_else(|| error("missing index parent"))?;
    let path = root
        .join(
            matches[0]["receipt"]
                .as_str()
                .ok_or_else(|| error("missing receipt path"))?,
        )
        .canonicalize()?;
    if !path.starts_with(root.canonicalize()?.join("attempts")) {
        return Err(error("receipt escapes capture attempts"));
    }
    let receipt = read_json(&path)?;
    let schema: Value = serde_json::from_str(include_str!(
        "../../../docs/operational-ux-capture.schema.json"
    ))?;
    if !jsonschema::validator_for(&schema["$defs"]["receipt"])
        .map_err(|e| error(e.to_string()))?
        .is_valid(&receipt)
    {
        return Err(error("malformed capture receipt"));
    }
    let identity = repository_proof_identity(cwd)?;
    if receipt["id"] != row.id
        || receipt["argv"] != json!(row.argv)
        || receipt["cwd"] != json!(cwd)
        || receipt["settings"] != settings(row)
        || receipt["obligations"] != json!(row.obligations)
        || receipt["resolved_executable"] != json!(resolve(row, cwd))
        || receipt["repository_before"] != identity
        || receipt["repository_after"] != identity
        || receipt["finished_at"].as_f64() < receipt["started_at"].as_f64()
        || !success(&receipt)
    {
        return Err(error("stale, mismatched or failed selected capture"));
    }
    let attempt = path.parent().unwrap();
    let started = read_json(&attempt.join("started.json"))?;
    for key in [
        "id",
        "argv",
        "resolved_executable",
        "cwd",
        "settings",
        "obligations",
        "repository_before",
        "started_at",
    ] {
        if started.get(key).is_none() || started[key] != receipt[key] {
            return Err(error("started/finished mismatch"));
        }
    }
    for stream in ["stdout", "stderr"] {
        let stream_path = attempt
            .join(
                receipt[stream]
                    .as_str()
                    .ok_or_else(|| error("missing stream"))?,
            )
            .canonicalize()?;
        if !stream_path.starts_with(attempt)
            || receipt[format!("{stream}_sha256")] != hash(&fs::read(stream_path)?)
        {
            return Err(error("capture stream path/digest mismatch"));
        }
    }
    Ok((path, receipt))
}

fn success(r: &Value) -> bool {
    r["exit_code"] == 0
        && r["signal"].is_null()
        && r["timed_out"] == false
        && r["aborted"] != true
        && r["spawn_error"].is_null()
        && r["capture_error"].is_null()
        && r["cleanup"] == "complete"
}

pub fn run_matrix(matrix: &Matrix, cwd: &Path, root: &Path, resume: bool) -> io::Result<i32> {
    run_matrix_with_forwarding(matrix, cwd, root, resume, false)
}

/// Preserve separate retained streams while keeping a structured worker's stdout clean.
pub fn run_matrix_with_forwarding(
    matrix: &Matrix,
    cwd: &Path,
    root: &Path,
    resume: bool,
    stdout_to_stderr: bool,
) -> io::Result<i32> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err(error(
            "capture cleanup is supported on macOS and Linux only",
        ));
    }
    validate(matrix)?;
    let _lock = Lock::acquire(root)?;
    let identity = repository_proof_identity(cwd)?;
    let settings: BTreeMap<_, _> = matrix.rows.iter().map(|r| (&r.id, settings(r))).collect();
    let signature = hash(&serde_json::to_vec(
        &json!({"matrix":matrix,"cwd":cwd,"settings":settings,"executables":matrix.rows.iter().map(|r|resolve(r,cwd)).collect::<Vec<_>>()}),
    )?);
    let mut index = json!({"matrix_identity":signature,"cwd":cwd,"settings":settings,"repository_identity":identity,"receipts":[]});
    let mut skip = 0;
    if resume {
        for entry in fs::read_dir(root.join("attempts"))? {
            let attempt = entry?.path();
            if !attempt.join("started.json").is_file() || !attempt.join("receipt.json").is_file() {
                return Err(error(
                    "incomplete started attempt; resume refused before spawn",
                ));
            }
            if read_json(&attempt.join("receipt.json"))?["cleanup"] != "complete" {
                return Err(error(
                    "retained attempt cleanup pending; resume refused before spawn",
                ));
            }
        }
        let state = read_json(&root.join("state.json"))?;
        if state["cleanup"] != "complete" || state["status"] == "running" {
            return Err(error(
                "cleanup pending or incomplete attempt; resume refused",
            ));
        }
        let old = read_json(&root.join("index.json"))?;
        for key in ["matrix_identity", "cwd", "settings", "repository_identity"] {
            if old[key] != index[key] {
                return Err(error(format!("changed {key}; resume refused before spawn")));
            }
        }
        let selected = old["receipts"]
            .as_array()
            .ok_or_else(|| error("invalid selected index"))?;
        if selected.len() > matrix.rows.len() {
            return Err(error("extra selected rows"));
        }
        let mut failed = false;
        for (i, selection) in selected.iter().enumerate() {
            if failed || selection["id"] != matrix.rows[i].id {
                return Err(error("noncontiguous selection"));
            }
            let path = root.join(
                selection["receipt"]
                    .as_str()
                    .ok_or_else(|| error("missing receipt"))?,
            );
            let r = read_json(&path)?;
            let schema: Value = serde_json::from_str(include_str!(
                "../../../docs/operational-ux-capture.schema.json"
            ))?;
            let validator = jsonschema::validator_for(&schema["$defs"]["receipt"])
                .map_err(|e| error(e.to_string()))?;
            if !validator.is_valid(&r) {
                return Err(error("partial or malformed finished receipt"));
            }
            if r["id"] != matrix.rows[i].id
                || r["argv"] != json!(matrix.rows[i].argv)
                || r["cwd"] != json!(cwd)
                || r["resolved_executable"] != json!(resolve(&matrix.rows[i], cwd))
                || r["obligations"] != json!(matrix.rows[i].obligations)
                || r["finished_at"].as_f64() < r["started_at"].as_f64()
                || r["settings"] != settings[&matrix.rows[i].id]
                || r["repository_before"] != identity
                || r["repository_after"] != identity
                || r["cleanup"] != "complete"
                || !r["capture_error"].is_null()
            {
                return Err(error("stale/incomplete selected receipt"));
            }
            let started = read_json(&path.parent().unwrap().join("started.json"))?;
            for key in [
                "id",
                "argv",
                "resolved_executable",
                "cwd",
                "settings",
                "obligations",
                "repository_before",
                "started_at",
            ] {
                if started.get(key).is_none() {
                    return Err(error("partial started record"));
                }
            }
            for (key, value) in started
                .as_object()
                .ok_or_else(|| error("invalid started record"))?
            {
                if r[key] != *value {
                    return Err(error("started/finished receipt mismatch"));
                }
            }
            for stream in ["stdout", "stderr"] {
                let bytes = fs::read(
                    path.parent()
                        .unwrap()
                        .join(r[stream].as_str().ok_or_else(|| error("missing stream"))?),
                )?;
                if r[format!("{stream}_sha256")] != hash(&bytes) {
                    return Err(error("stream digest mismatch"));
                }
            }
            if success(&r) {
                skip += 1;
            } else {
                failed = true;
            }
        }
        index = old;
        index["receipts"].as_array_mut().unwrap().truncate(skip);
        if root.join("abort-request.json").exists() {
            fs::remove_file(root.join("abort-request.json"))?;
        }
    } else if root.join("index.json").exists()
        || root.join("state.json").exists()
        || root.join("attempts").exists()
    {
        return Err(error(
            "capture already exists; inspect then explicitly --resume",
        ));
    }
    fs::create_dir_all(root.join("attempts"))?;
    if !resume {
        atomic(&root.join("index.json"), &index)?;
    }
    INTERRUPTED.store(false, Ordering::SeqCst);
    let previous = unsafe { signal(2, interrupt as *const () as usize) };
    let result = (|| {
        for (i, row) in matrix.rows.iter().enumerate().skip(skip) {
            if INTERRUPTED.load(Ordering::SeqCst) || root.join("abort-request.json").exists() {
                return Err(error("capture aborted before admission"));
            }
            if repository_proof_identity(cwd)? != identity {
                return Err(error("repository changed before admission"));
            }
            let attempt = root.join("attempts").join(format!(
                "{i}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let receipt = execute_row_with_forwarding(row, cwd, &attempt, root, stdout_to_stderr)?;
            index["receipts"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":row.id,"receipt":attempt.join("receipt.json")}));
            atomic(&root.join("index.json"), &index)?;
            let ok = success(&receipt) && receipt["repository_after"] == identity;
            atomic(
                &root.join("state.json"),
                &json!({"status":if !ok {"failed"} else if i+1 == matrix.rows.len() {"completed"} else {"running"},"cleanup":receipt["cleanup"],"attempt":attempt,"selected_count":i+1,"row_count":matrix.rows.len()}),
            )?;
            if !ok {
                return Ok(receipt["exit_code"]
                    .as_i64()
                    .filter(|v| *v > 0 && *v < 256)
                    .unwrap_or(1) as i32);
            }
        }
        Ok(0)
    })();
    unsafe {
        signal(2, previous);
    }
    result
}

fn abort(root: &Path) -> io::Result<i32> {
    let state = read_json(&root.join("state.json"))?;
    if state["cleanup"] == "complete" && state["status"] != "running" {
        return Ok(0);
    }
    atomic(
        &root.join("abort-request.json"),
        &json!({"requested_at":now()}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let state = read_json(&root.join("state.json"))?;
        if state["cleanup"] == "complete" && state["status"] != "running" {
            return Ok(0);
        }
        if Instant::now() >= deadline {
            return Err(error(
                "cleanup pending; controller has not verified owned process disappearance",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Main dispatch happens before the buffered workflow CLI: child bytes must be
/// flushed while they are running, not packed into the final JSON envelope.
pub fn cli(args: &[String]) -> Option<i32> {
    let command = args.first()?;
    if !["capture-command", "capture-matrix", "capture-abort"].contains(&command.as_str()) {
        return None;
    }
    if args
        .iter()
        .skip(1)
        .take_while(|s| s.as_str() != "--")
        .any(|s| s == "--help" || s == "-h")
    {
        println!("Usage: loop-engine capture-matrix --matrix FILE --working-directory ABS --output-dir ABS [--resume]\n       loop-engine capture-command --working-directory ABS --output-dir ABS [--timeout-ms N] [--inherit-environment NAME] [--resume] -- EXECUTABLE ARG...\n       loop-engine capture-abort --output-dir ABS\nStreams are raw child bytes, not a JSON envelope. Inspect index.json and immutable attempts. Default single-command timeout: 3600000ms. Declare non-secret inherited execution settings explicitly. Abort success means verified owned cleanup, never semantic approval.");
        return Some(0);
    }
    let run = || -> io::Result<i32> {
        let mut options = BTreeMap::new();
        let mut inherited = vec![];
        let mut argv = vec![];
        let mut resume = false;
        let mut i = 1;
        while i < args.len() {
            let key = &args[i];
            if key == "--" {
                argv = args[i + 1..].to_vec();
                break;
            }
            if key == "--resume" {
                if resume {
                    return Err(error("duplicate --resume"));
                }
                resume = true;
                i += 1;
                continue;
            }
            if ![
                "--matrix",
                "--working-directory",
                "--output-dir",
                "--timeout-ms",
                "--inherit-environment",
            ]
            .contains(&key.as_str())
            {
                return Err(error(format!("unsupported option {key}")));
            }
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| error("missing option value"))?
                .clone();
            if key == "--inherit-environment" {
                inherited.push(value);
            } else if options.insert(key.as_str(), value).is_some() {
                return Err(error("duplicate option"));
            }
            i += 1;
        }
        let root = PathBuf::from(
            options
                .get("--output-dir")
                .ok_or_else(|| error("requires --output-dir"))?,
        );
        if !root.is_absolute() {
            return Err(error("output directory must be absolute"));
        }
        if command == "capture-abort" {
            if options.len() != 1 || resume || !argv.is_empty() || !inherited.is_empty() {
                return Err(error("unsupported abort arguments"));
            }
            return abort(&root);
        }
        let cwd = PathBuf::from(
            options
                .get("--working-directory")
                .ok_or_else(|| error("requires --working-directory"))?,
        );
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(error("working directory must exist and be absolute"));
        }
        let cwd = cwd.canonicalize()?;
        // Check the actual Git root, including cwd subdirectories and symlinks.
        let repo = PathBuf::from(
            String::from_utf8(git(&cwd, &["rev-parse", "--show-toplevel"])?)
                .map_err(|_| error("non-UTF8 Git root"))?
                .trim(),
        );
        let mut ancestor = root.as_path();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| error("invalid output path"))?;
        }
        if ancestor.canonicalize()?.starts_with(repo.canonicalize()?) {
            return Err(error("capture root must be outside the checkout"));
        }
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        if root.starts_with(repo.canonicalize()?) {
            return Err(error("capture root must be outside the checkout"));
        }
        let matrix = if command == "capture-matrix" {
            if !argv.is_empty() || options.contains_key("--timeout-ms") || !inherited.is_empty() {
                return Err(error("matrix settings belong in rows"));
            }
            serde_json::from_slice(&fs::read(
                options
                    .get("--matrix")
                    .ok_or_else(|| error("requires --matrix"))?,
            )?)?
        } else {
            if options.contains_key("--matrix") {
                return Err(error("single command cannot take --matrix"));
            }
            Matrix {
                rows: vec![Row {
                    id: "command".into(),
                    argv,
                    environment: BTreeMap::new(),
                    inherit_environment: inherited,
                    timeout_ms: options
                        .get("--timeout-ms")
                        .map(|v| v.parse())
                        .transpose()
                        .map_err(|_| error("invalid timeout"))?
                        .unwrap_or(3_600_000),
                    obligations: vec![],
                }],
            }
        };
        run_matrix(&matrix, &cwd, &root, resume)
    };
    Some(match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("capture: {e}");
            20
        }
    })
}
