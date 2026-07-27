//! Project-neutral direct process execution for candidate validation.
//!
//! Every child starts in its own Unix process group. Output readers enforce
//! independent byte limits and request group termination as soon as either
//! limit is crossed. Callers may clone a cancellation handle before awaiting
//! completion so concurrent schedulers can cancel sibling groups through the
//! same supervision path.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::Serialize;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("xtask process execution supports only macOS and Linux");

use std::os::unix::process::{CommandExt, ExitStatusExt};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const TERMINATION_GRACE: Duration = Duration::from_millis(500);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(2);

/// Environment mutations applied after inheriting caller environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnvironmentChanges {
    set: BTreeMap<String, String>,
    unset: BTreeSet<String>,
}

impl EnvironmentChanges {
    pub fn new(set: BTreeMap<String, String>, unset: BTreeSet<String>) -> Self {
        Self { set, unset }
    }

    pub fn set(&self) -> &BTreeMap<String, String> {
        &self.set
    }

    pub fn unset(&self) -> &BTreeSet<String> {
        &self.unset
    }
}

/// Complete direct-execution request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    program: String,
    args: Vec<String>,
    candidate_root: PathBuf,
    cwd: PathBuf,
    environment: EnvironmentChanges,
    stdin: Vec<u8>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessSpec {
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        candidate_root: impl Into<PathBuf>,
        cwd: impl Into<PathBuf>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            program: program.into(),
            args,
            candidate_root: candidate_root.into(),
            cwd: cwd.into(),
            environment: EnvironmentChanges::default(),
            stdin: Vec::new(),
            timeout,
            max_output_bytes,
        }
    }

    pub fn with_environment(mut self, environment: EnvironmentChanges) -> Self {
        self.environment = environment;
        self
    }

    pub fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = stdin;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn candidate_root(&self) -> &Path {
        &self.candidate_root
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn environment(&self) -> &EnvironmentChanges {
        &self.environment
    }

    pub fn stdin(&self) -> &[u8] {
        &self.stdin
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }
}

/// Encoding used to retain exact stream bytes in JSON-compatible evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StreamEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "base64")]
    Base64,
}

/// Child stream whose independent bound was crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Stdout,
    Stderr,
}

/// Lossless bounded evidence for one child stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEvidence {
    encoding: StreamEncoding,
    data: String,
    complete: bool,
    #[serde(skip)]
    exact_bytes: Vec<u8>,
}

impl StreamEvidence {
    fn from_bytes(exact_bytes: Vec<u8>, complete: bool) -> Self {
        match String::from_utf8(exact_bytes.clone()) {
            Ok(data) => Self {
                encoding: StreamEncoding::Utf8,
                data,
                complete,
                exact_bytes,
            },
            Err(_) => Self {
                encoding: StreamEncoding::Base64,
                data: encode_base64(&exact_bytes),
                complete,
                exact_bytes,
            },
        }
    }

    fn empty() -> Self {
        Self::from_bytes(Vec::new(), true)
    }

    pub fn encoding(&self) -> StreamEncoding {
        self.encoding
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn complete(&self) -> bool {
        self.complete
    }

    pub fn exact_bytes(&self) -> &[u8] {
        &self.exact_bytes
    }
}

/// Typed category for failures occurring before a child is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnFailureKind {
    InvalidCandidateRoot,
    InvalidCwd,
    CwdOutsideCandidate,
    InvalidConfiguration,
    ExecutableNotFound,
    Spawn,
}

/// Authoritative reason one execution attempt ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessTermination {
    Exit {
        code: i32,
    },
    Signal {
        signal: i32,
    },
    Timeout,
    OutputLimit {
        streams: Vec<StreamKind>,
    },
    Cancelled,
    SpawnFailure {
        failure_kind: SpawnFailureKind,
        message: String,
    },
    SupervisionFailure {
        message: String,
    },
}

impl ProcessTermination {
    /// Return spawn category without requiring callers to inspect its message.
    pub fn spawn_failure_kind(&self) -> Option<SpawnFailureKind> {
        match self {
            Self::SpawnFailure { failure_kind, .. } => Some(*failure_kind),
            _ => None,
        }
    }
}

/// Evidence that no process from the launched group was left running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CleanupOutcome {
    NotRequired,
    Completed {
        term_sent: bool,
        kill_sent: bool,
    },
    Failed {
        term_sent: bool,
        kill_sent: bool,
        message: String,
    },
}

/// Complete evidence from one process attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessOutcome {
    pub termination: ProcessTermination,
    pub stdout: StreamEvidence,
    pub stderr: StreamEvidence,
    pub duration_millis: u64,
    pub cleanup: CleanupOutcome,
}

impl ProcessOutcome {
    pub fn success(&self) -> bool {
        self.termination == (ProcessTermination::Exit { code: 0 })
            && !matches!(self.cleanup, CleanupOutcome::Failed { .. })
    }
}

/// Result of requesting cancellation through a cloned external handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationRequest {
    Requested,
    AlreadyRequested,
    AlreadyFinished,
}

/// Cloneable authority to terminate exactly one launched process group.
#[derive(Debug, Clone)]
pub struct CancellationHandle {
    control: Arc<Control>,
}

impl CancellationHandle {
    /// Request process-group termination. First request sends SIGTERM
    /// synchronously; later calls never duplicate that signal.
    pub fn cancel(&self) -> CancellationRequest {
        self.control.request(CancellationCause::External)
    }
}

/// Owned launched-process attempt. Completion must be awaited or dropping the
/// owner performs best-effort SIGKILL cleanup.
pub struct RunningProcess {
    state: RunningState,
    control: Arc<Control>,
    started: Instant,
}

enum RunningState {
    Child {
        child: Child,
        stdout: Arc<CaptureState>,
        stderr: Arc<CaptureState>,
        stdin: Arc<WriterState>,
        deadline: Instant,
    },
    Completed(Option<ProcessOutcome>),
}

impl RunningProcess {
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            control: Arc::clone(&self.control),
        }
    }

    /// Await child, stream, timeout, and group-cleanup completion.
    pub fn await_completion(mut self) -> ProcessOutcome {
        let state = std::mem::replace(&mut self.state, RunningState::Completed(None));
        match state {
            RunningState::Completed(mut outcome) => outcome
                .take()
                .expect("completed process outcome is available exactly once"),
            RunningState::Child {
                mut child,
                stdout,
                stderr,
                stdin,
                deadline,
            } => self.await_child(
                &mut child,
                Arc::clone(&stdout),
                Arc::clone(&stderr),
                Arc::clone(&stdin),
                deadline,
            ),
        }
    }

    fn await_child(
        &mut self,
        child: &mut Child,
        stdout: Arc<CaptureState>,
        stderr: Arc<CaptureState>,
        stdin: Arc<WriterState>,
        deadline: Instant,
    ) -> ProcessOutcome {
        let mut supervision_error = None;
        let mut cancellation_cleanup_started = None;
        let mut cleanup_probe = CleanupProbeState::default();
        let mut cleanup_failure = None;

        loop {
            let now = Instant::now();
            match self
                .control
                .observe_leader(now + Duration::from_millis(100))
            {
                Ok(_) => {}
                Err(error) => {
                    supervision_error = Some(error);
                    self.control.request(CancellationCause::Supervision);
                }
            }

            // Zombie-aware probing observes completion without reaping: timeout
            // never replaces an observed exit, and leader still anchors PGID.
            if now >= deadline && !self.control.leader_completed() {
                self.control.request(CancellationCause::Timeout);
            }

            if let Some(requested) = self.control.requested_at() {
                cancellation_cleanup_started.get_or_insert(requested);
            }
            if let Some(started) = cancellation_cleanup_started {
                if now.saturating_duration_since(started) >= TERMINATION_GRACE {
                    self.control.send_kill_once();
                }
                if now.saturating_duration_since(started) >= CLEANUP_DEADLINE
                    && !self.control.leader_completed()
                {
                    cleanup_failure.get_or_insert_with(|| {
                        format!(
                            "process leader {} remained after cancellation cleanup deadline",
                            self.control.process_group_id
                        )
                    });
                    self.control.send_kill_once();
                }
            }

            if self.control.leader_completed() {
                let cleanup_deadline = cleanup_probe
                    .started
                    .and_then(|started| started.checked_add(CLEANUP_DEADLINE))
                    .unwrap_or_else(|| now + CLEANUP_DEADLINE);
                let probe_deadline = cleanup_deadline.min(now + Duration::from_millis(100));
                let streams_drained = stdout.done.load(Ordering::Acquire)
                    && stderr.done.load(Ordering::Acquire)
                    && stdin.done.load(Ordering::Acquire);
                let decision = cleanup_probe.observe(
                    now,
                    group_has_live_members(self.control.process_group_id, probe_deadline),
                    streams_drained,
                );
                match decision {
                    CleanupProbeDecision::Complete => break,
                    CleanupProbeDecision::Continue => {
                        if cleanup_probe.last_group_live || cleanup_probe.latest_error.is_some() {
                            self.control.send_term_once();
                            if cleanup_probe.started.is_some_and(|started| {
                                now.saturating_duration_since(started) >= TERMINATION_GRACE
                            }) {
                                self.control.send_kill_once();
                            }
                        }
                    }
                    CleanupProbeDecision::DeadlineExceeded => {
                        cleanup_failure =
                            Some(cleanup_probe.deadline_failure(self.control.process_group_id));
                        self.control.send_kill_once();
                        break;
                    }
                }
            }

            thread::sleep(POLL_INTERVAL);
        }

        // Reap and release group identity under same lifecycle lock used by
        // cancellation and every group signal. No handle can signal afterward.
        let status = match self.control.reap_leader(child) {
            Ok(status) => Some(status),
            Err(error) => {
                supervision_error.get_or_insert(error);
                None
            }
        };

        let stdout_evidence = stdout.snapshot();
        let stderr_evidence = stderr.snapshot();
        if supervision_error.is_none() {
            supervision_error = stdout
                .error()
                .or_else(|| stderr.error())
                .or_else(|| stdin.error());
        }
        let termination = derive_termination(
            self.control.cause(),
            self.control.output_limited_streams(),
            supervision_error,
            status,
        );
        let cleanup = derive_cleanup(
            &self.control,
            cleanup_failure,
            cleanup_probe.last_group_live,
        );
        ProcessOutcome {
            termination,
            stdout: stdout_evidence,
            stderr: stderr_evidence,
            duration_millis: elapsed_millis(self.started),
            cleanup,
        }
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        if let RunningState::Child { child, .. } = &mut self.state {
            self.control.drop_and_reap(child);
        }
    }
}

/// Spawn one direct child. Validation and OS spawn failures are retained inside
/// returned owner and become typed evidence from [`RunningProcess::await_completion`].
pub fn spawn(spec: ProcessSpec) -> RunningProcess {
    let started = Instant::now();
    let inert_control = || Arc::new(Control::inactive());

    let cwd = match contained_cwd(&spec.candidate_root, &spec.cwd) {
        Ok(cwd) => cwd,
        Err((kind, message)) => {
            return completed_spawn_failure(started, inert_control(), kind, message);
        }
    };
    if spec.program.is_empty() {
        return completed_spawn_failure(
            started,
            inert_control(),
            SpawnFailureKind::InvalidConfiguration,
            "program must be non-empty".to_owned(),
        );
    }
    if spec.timeout.is_zero() {
        return completed_spawn_failure(
            started,
            inert_control(),
            SpawnFailureKind::InvalidConfiguration,
            "timeout must be positive".to_owned(),
        );
    }
    if spec.max_output_bytes == 0 {
        return completed_spawn_failure(
            started,
            inert_control(),
            SpawnFailureKind::InvalidConfiguration,
            "max_output_bytes must be positive".to_owned(),
        );
    }
    let Some(deadline) = started.checked_add(spec.timeout) else {
        return completed_spawn_failure(
            started,
            inert_control(),
            SpawnFailureKind::InvalidConfiguration,
            "timeout is outside platform instant range".to_owned(),
        );
    };

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(spec.environment.set())
        .process_group(0);
    // Removal deliberately follows additions so unset wins duplicate keys.
    for name in spec.environment.unset() {
        command.env_remove(name);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let kind = if error.kind() == io::ErrorKind::NotFound {
                SpawnFailureKind::ExecutableNotFound
            } else {
                SpawnFailureKind::Spawn
            };
            return completed_spawn_failure(
                started,
                inert_control(),
                kind,
                format!("failed to spawn `{}`: {error}", spec.program),
            );
        }
    };

    let control = Arc::new(Control::new(child.id()));
    let stdout = Arc::new(CaptureState::new());
    let stderr = Arc::new(CaptureState::new());
    let stdin = Arc::new(WriterState::new());

    let stdout_pipe = child.stdout.take().expect("configured piped stdout");
    let stderr_pipe = child.stderr.take().expect("configured piped stderr");
    spawn_reader(
        "validation-stdout",
        stdout_pipe,
        Arc::clone(&stdout),
        spec.max_output_bytes,
        StreamKind::Stdout,
        Arc::clone(&control),
    );
    spawn_reader(
        "validation-stderr",
        stderr_pipe,
        Arc::clone(&stderr),
        spec.max_output_bytes,
        StreamKind::Stderr,
        Arc::clone(&control),
    );

    let stdin_pipe = child.stdin.take().expect("configured piped stdin");
    spawn_writer(
        stdin_pipe,
        spec.stdin,
        Arc::clone(&stdin),
        Arc::clone(&control),
    );

    RunningProcess {
        state: RunningState::Child {
            child,
            stdout,
            stderr,
            stdin,
            deadline,
        },
        control,
        started,
    }
}

/// Spawn and synchronously await one process attempt.
pub fn execute(spec: ProcessSpec) -> ProcessOutcome {
    spawn(spec).await_completion()
}

/// Validate existing canonical cwd containment without spawning a child.
///
/// Empty-input schedulers use this before reporting skipped success. Failures
/// retain the same typed process evidence as normal spawn preflight.
pub fn preflight_cwd(spec: &ProcessSpec) -> Option<ProcessOutcome> {
    let started = Instant::now();
    contained_cwd(&spec.candidate_root, &spec.cwd)
        .err()
        .map(|(kind, message)| {
            completed_spawn_failure(started, Arc::new(Control::inactive()), kind, message)
                .await_completion()
        })
}

fn contained_cwd(candidate_root: &Path, cwd: &Path) -> Result<PathBuf, (SpawnFailureKind, String)> {
    let root = candidate_root.canonicalize().map_err(|error| {
        (
            SpawnFailureKind::InvalidCandidateRoot,
            format!(
                "failed to resolve candidate root {}: {error}",
                candidate_root.display()
            ),
        )
    })?;
    if !root.is_dir() {
        return Err((
            SpawnFailureKind::InvalidCandidateRoot,
            format!("candidate root is not a directory: {}", root.display()),
        ));
    }
    let requested = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        candidate_root.join(cwd)
    };
    let resolved = requested.canonicalize().map_err(|error| {
        (
            SpawnFailureKind::InvalidCwd,
            format!("failed to resolve cwd {}: {error}", requested.display()),
        )
    })?;
    if !resolved.is_dir() {
        return Err((
            SpawnFailureKind::InvalidCwd,
            format!("cwd is not a directory: {}", resolved.display()),
        ));
    }
    if !resolved.starts_with(&root) {
        return Err((
            SpawnFailureKind::CwdOutsideCandidate,
            format!(
                "cwd {} resolves outside candidate root {}",
                requested.display(),
                root.display()
            ),
        ));
    }
    Ok(resolved)
}

fn completed_spawn_failure(
    started: Instant,
    control: Arc<Control>,
    kind: SpawnFailureKind,
    message: String,
) -> RunningProcess {
    RunningProcess {
        state: RunningState::Completed(Some(ProcessOutcome {
            termination: ProcessTermination::SpawnFailure {
                failure_kind: kind,
                message,
            },
            stdout: StreamEvidence::empty(),
            stderr: StreamEvidence::empty(),
            duration_millis: elapsed_millis(started),
            cleanup: CleanupOutcome::NotRequired,
        })),
        control,
        started,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancellationCause {
    Timeout,
    OutputLimit,
    External,
    Supervision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaderPhase {
    Active,
    Completed,
    Released,
}

#[derive(Debug)]
struct Lifecycle {
    phase: LeaderPhase,
    cause: Option<(CancellationCause, Instant)>,
    term_sent: bool,
    kill_sent: bool,
    signal_errors: Vec<String>,
}

#[derive(Debug)]
struct Control {
    process_group_id: u32,
    lifecycle: Mutex<Lifecycle>,
    output_limits: Mutex<BTreeSet<StreamKind>>,
}

impl Control {
    fn new(process_group_id: u32) -> Self {
        Self {
            process_group_id,
            lifecycle: Mutex::new(Lifecycle {
                phase: LeaderPhase::Active,
                cause: None,
                term_sent: false,
                kill_sent: false,
                signal_errors: Vec::new(),
            }),
            output_limits: Mutex::new(BTreeSet::new()),
        }
    }

    fn inactive() -> Self {
        Self {
            process_group_id: 0,
            lifecycle: Mutex::new(Lifecycle {
                phase: LeaderPhase::Released,
                cause: None,
                term_sent: false,
                kill_sent: false,
                signal_errors: Vec::new(),
            }),
            output_limits: Mutex::new(BTreeSet::new()),
        }
    }

    fn request(&self, cause: CancellationCause) -> CancellationRequest {
        let mut lifecycle = lock(&self.lifecycle);
        if lifecycle.phase != LeaderPhase::Active {
            return CancellationRequest::AlreadyFinished;
        }
        if lifecycle.cause.is_some() {
            return CancellationRequest::AlreadyRequested;
        }
        lifecycle.cause = Some((cause, Instant::now()));
        self.signal_group_locked(&mut lifecycle, Signal::SIGTERM);
        CancellationRequest::Requested
    }

    fn record_output_limit(&self, stream: StreamKind) {
        lock(&self.output_limits).insert(stream);
        self.request(CancellationCause::OutputLimit);
    }

    fn cause(&self) -> Option<CancellationCause> {
        lock(&self.lifecycle).cause.map(|(cause, _)| cause)
    }

    fn requested_at(&self) -> Option<Instant> {
        lock(&self.lifecycle).cause.map(|(_, requested)| requested)
    }

    fn output_limited_streams(&self) -> Vec<StreamKind> {
        lock(&self.output_limits).iter().copied().collect()
    }

    fn leader_completed(&self) -> bool {
        lock(&self.lifecycle).phase != LeaderPhase::Active
    }

    fn observe_leader(&self, deadline: Instant) -> Result<bool, String> {
        let mut lifecycle = lock(&self.lifecycle);
        if lifecycle.phase != LeaderPhase::Active {
            return Ok(true);
        }
        let pid = self.process_group_id.to_string();
        let (status, output) = bounded_probe("/bin/ps", &["-o", "stat=", "-p", &pid], deadline)?;
        if !status.success() {
            return Err(format!(
                "targeted leader probe for {} exited {status}",
                self.process_group_id
            ));
        }
        let state = String::from_utf8_lossy(&output);
        let completed = state
            .split_whitespace()
            .next()
            .is_some_and(|value| value.starts_with('Z'));
        if completed {
            lifecycle.phase = LeaderPhase::Completed;
        }
        Ok(completed)
    }

    fn send_term_once(&self) {
        let mut lifecycle = lock(&self.lifecycle);
        self.signal_group_locked(&mut lifecycle, Signal::SIGTERM);
    }

    fn send_kill_once(&self) {
        let mut lifecycle = lock(&self.lifecycle);
        self.signal_group_locked(&mut lifecycle, Signal::SIGKILL);
    }

    fn signal_group_locked(&self, lifecycle: &mut Lifecycle, signal: Signal) {
        if lifecycle.phase == LeaderPhase::Released || self.process_group_id == 0 {
            return;
        }
        let already_sent = match signal {
            Signal::SIGTERM => &mut lifecycle.term_sent,
            Signal::SIGKILL => &mut lifecycle.kill_sent,
            _ => unreachable!("only termination signals are supported"),
        };
        if *already_sent {
            return;
        }
        *already_sent = true;
        let Ok(group) = i32::try_from(self.process_group_id) else {
            lifecycle.signal_errors.push(format!(
                "process group {} does not fit pid_t",
                self.process_group_id
            ));
            return;
        };
        if let Err(error) = killpg(Pid::from_raw(group), signal)
            && error != Errno::ESRCH
        {
            lifecycle.signal_errors.push(format!(
                "failed signaling process group {} with {signal}: {error}",
                self.process_group_id
            ));
        }
    }

    fn reap_leader(&self, child: &mut Child) -> Result<ExitStatus, String> {
        let mut lifecycle = lock(&self.lifecycle);
        let status = child.wait().map_err(|error| {
            format!(
                "failed reaping process leader {}: {error}",
                self.process_group_id
            )
        })?;
        lifecycle.phase = LeaderPhase::Released;
        Ok(status)
    }

    fn drop_and_reap(&self, child: &mut Child) {
        let mut lifecycle = lock(&self.lifecycle);
        self.signal_group_locked(&mut lifecycle, Signal::SIGKILL);
        let _ = child.wait();
        lifecycle.phase = LeaderPhase::Released;
    }

    fn cleanup_snapshot(&self) -> (bool, bool, Vec<String>) {
        let lifecycle = lock(&self.lifecycle);
        (
            lifecycle.term_sent,
            lifecycle.kill_sent,
            lifecycle.signal_errors.clone(),
        )
    }
}

struct CaptureState {
    bytes: Mutex<Vec<u8>>,
    done: AtomicBool,
    complete: AtomicBool,
    error: Mutex<Option<String>>,
}

impl CaptureState {
    fn new() -> Self {
        Self {
            bytes: Mutex::new(Vec::new()),
            done: AtomicBool::new(false),
            complete: AtomicBool::new(true),
            error: Mutex::new(None),
        }
    }

    fn snapshot(&self) -> StreamEvidence {
        StreamEvidence::from_bytes(
            lock(&self.bytes).clone(),
            self.complete.load(Ordering::Acquire) && self.done.load(Ordering::Acquire),
        )
    }

    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

struct WriterState {
    done: AtomicBool,
    error: Mutex<Option<String>>,
}

impl WriterState {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    name: &str,
    mut reader: R,
    capture: Arc<CaptureState>,
    limit: usize,
    stream: StreamKind,
    control: Arc<Control>,
) {
    let builder = thread::Builder::new().name(name.to_owned());
    let thread_capture = Arc::clone(&capture);
    let thread_control = Arc::clone(&control);
    let result = builder.spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let mut retained = lock(&thread_capture.bytes);
                    let remaining = limit.saturating_sub(retained.len());
                    retained.extend_from_slice(&buffer[..count.min(remaining)]);
                    let crossed = count > remaining;
                    drop(retained);
                    if crossed {
                        thread_capture.complete.store(false, Ordering::Release);
                        thread_control.record_output_limit(stream);
                    }
                }
                Err(error) => {
                    thread_capture.complete.store(false, Ordering::Release);
                    *lock(&thread_capture.error) =
                        Some(format!("failed reading {stream:?}: {error}"));
                    thread_control.request(CancellationCause::Supervision);
                    break;
                }
            }
        }
        thread_capture.done.store(true, Ordering::Release);
    });
    if let Err(error) = result {
        capture.complete.store(false, Ordering::Release);
        *lock(&capture.error) = Some(format!("failed spawning {name} reader: {error}"));
        capture.done.store(true, Ordering::Release);
        control.request(CancellationCause::Supervision);
    }
}

fn spawn_writer(
    mut writer: impl Write + Send + 'static,
    input: Vec<u8>,
    state: Arc<WriterState>,
    control: Arc<Control>,
) {
    let thread_state = Arc::clone(&state);
    let thread_control = Arc::clone(&control);
    let result = thread::Builder::new()
        .name("validation-stdin".to_owned())
        .spawn(move || {
            if let Err(error) = writer.write_all(&input).and_then(|()| writer.flush()) {
                *lock(&thread_state.error) = Some(format!("failed writing stdin: {error}"));
                thread_control.request(CancellationCause::Supervision);
            }
            drop(writer);
            thread_state.done.store(true, Ordering::Release);
        });
    if let Err(error) = result {
        *lock(&state.error) = Some(format!("failed spawning stdin writer: {error}"));
        state.done.store(true, Ordering::Release);
        control.request(CancellationCause::Supervision);
    }
}

const PROBE_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupProbeDecision {
    Continue,
    Complete,
    DeadlineExceeded,
}

#[derive(Debug, Default)]
struct CleanupProbeState {
    started: Option<Instant>,
    latest_error: Option<String>,
    last_group_live: bool,
}

impl CleanupProbeState {
    fn observe(
        &mut self,
        now: Instant,
        probe: Result<bool, String>,
        streams_drained: bool,
    ) -> CleanupProbeDecision {
        match probe {
            Ok(false) => {
                self.latest_error = None;
                self.last_group_live = false;
                return if streams_drained {
                    CleanupProbeDecision::Complete
                } else {
                    CleanupProbeDecision::Continue
                };
            }
            Ok(true) => {
                self.latest_error = None;
                self.last_group_live = true;
                self.started.get_or_insert(now);
            }
            Err(error) => {
                self.latest_error = Some(error);
                self.started.get_or_insert(now);
            }
        }

        if self
            .started
            .is_some_and(|started| now.saturating_duration_since(started) >= CLEANUP_DEADLINE)
        {
            CleanupProbeDecision::DeadlineExceeded
        } else {
            CleanupProbeDecision::Continue
        }
    }

    fn deadline_failure(&self, process_group_id: u32) -> String {
        self.latest_error.clone().unwrap_or_else(|| {
            format!("process group {process_group_id} retained live members after cleanup deadline")
        })
    }
}

fn group_has_live_members(process_group_id: u32, deadline: Instant) -> Result<bool, String> {
    let group = process_group_id.to_string();
    let (status, output) = bounded_probe("/usr/bin/pgrep", &["-g", &group], deadline)?;
    if !status.success() {
        if status.code() == Some(1) {
            return Ok(false);
        }
        return Err(format!("targeted process-group probe exited {status}"));
    }
    let pids = String::from_utf8(output)
        .map_err(|_| "targeted process-group probe returned non-UTF-8 output".to_owned())?;
    for pid in pids.split_whitespace() {
        pid.parse::<u32>()
            .map_err(|_| format!("targeted process-group probe returned invalid PID `{pid}`"))?;
        let (status, state) = bounded_probe("/bin/ps", &["-o", "stat=", "-p", pid], deadline)?;
        if !status.success() {
            continue;
        }
        let state = String::from_utf8_lossy(&state);
        if state
            .split_whitespace()
            .any(|value| !value.starts_with('Z'))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn bounded_probe(
    program: &str,
    args: &[&str],
    deadline: Instant,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed spawning targeted process probe `{program}`: {error}"))?;
    let mut stdout = child.stdout.take().expect("probe stdout is piped");
    let reader = thread::Builder::new()
        .name("validation-process-probe".to_owned())
        .spawn(move || {
            let mut retained = Vec::new();
            let mut buffer = [0u8; 4096];
            let mut overflowed = false;
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => return Ok((retained, overflowed)),
                    Ok(count) => {
                        let remaining = PROBE_OUTPUT_LIMIT.saturating_sub(retained.len());
                        retained.extend_from_slice(&buffer[..count.min(remaining)]);
                        overflowed |= count > remaining;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .map_err(|error| format!("failed spawning targeted process-probe reader: {error}"))?;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(format!(
                    "targeted process probe `{program}` exceeded deadline"
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(format!(
                    "failed polling targeted process probe `{program}`: {error}"
                ));
            }
        }
    };
    let (output, overflowed) = reader
        .join()
        .map_err(|_| "targeted process-probe reader panicked".to_owned())?
        .map_err(|error| format!("failed reading targeted process-probe output: {error}"))?;
    if overflowed {
        return Err(format!(
            "targeted process probe `{program}` exceeded {PROBE_OUTPUT_LIMIT}-byte output bound"
        ));
    }
    Ok((status, output))
}

fn derive_termination(
    cause: Option<CancellationCause>,
    output_limits: Vec<StreamKind>,
    supervision_error: Option<String>,
    status: Option<ExitStatus>,
) -> ProcessTermination {
    // Reader evidence is authoritative even when the leader became a zombie
    // before the reader reported crossing its bound. In that race the
    // lifecycle deliberately rejects a new cancellation request so no signal
    // can escape after reap, but the completed attempt still failed its bound.
    if !output_limits.is_empty() {
        return ProcessTermination::OutputLimit {
            streams: output_limits,
        };
    }

    match cause {
        Some(CancellationCause::Timeout) => ProcessTermination::Timeout,
        Some(CancellationCause::OutputLimit) => ProcessTermination::OutputLimit {
            streams: output_limits,
        },
        Some(CancellationCause::External) => ProcessTermination::Cancelled,
        Some(CancellationCause::Supervision) => ProcessTermination::SupervisionFailure {
            message: supervision_error.unwrap_or_else(|| "process supervision failed".to_owned()),
        },
        None => {
            if let Some(message) = supervision_error {
                return ProcessTermination::SupervisionFailure { message };
            }
            match status {
                Some(status) => status
                    .code()
                    .map(|code| ProcessTermination::Exit { code })
                    .unwrap_or_else(|| ProcessTermination::Signal {
                        signal: status.signal().unwrap_or_default(),
                    }),
                None => ProcessTermination::SupervisionFailure {
                    message: "child ended without an exit status".to_owned(),
                },
            }
        }
    }
}

fn derive_cleanup(
    control: &Control,
    prior_failure: Option<String>,
    live_members_at_release: bool,
) -> CleanupOutcome {
    let (term_sent, kill_sent, signal_errors) = control.cleanup_snapshot();
    let failure = prior_failure.or_else(|| {
        live_members_at_release.then(|| {
            format!(
                "process group {} retained live members at identity release",
                control.process_group_id
            )
        })
    });
    if let Some(message) = failure {
        return CleanupOutcome::Failed {
            term_sent,
            kill_sent,
            message,
        };
    }
    if !signal_errors.is_empty() {
        return CleanupOutcome::Failed {
            term_sent,
            kill_sent,
            message: signal_errors.join("; "),
        };
    }
    if term_sent || kill_sent {
        CleanupOutcome::Completed {
            term_sent,
            kill_sent,
        }
    } else {
        CleanupOutcome::NotRequired
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(TABLE[usize::from(first >> 2)]));
        encoded.push(char::from(
            TABLE[usize::from(((first & 0b0000_0011) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                TABLE[usize::from(((second & 0b0000_1111) << 2) | (third >> 6))],
            ));
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(char::from(TABLE[usize::from(third & 0b0011_1111)]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_probe_error_then_empty_recovers() {
        let started = Instant::now();
        let mut state = CleanupProbeState::default();

        assert_eq!(
            state.observe(
                started,
                Err("targeted process probe timed out".to_owned()),
                false,
            ),
            CleanupProbeDecision::Continue
        );
        assert_eq!(
            state.observe(started + CLEANUP_DEADLINE - POLL_INTERVAL, Ok(false), true,),
            CleanupProbeDecision::Complete
        );
        assert_eq!(state.latest_error, None);
        assert!(!state.last_group_live);
    }

    #[test]
    fn persistent_cleanup_probe_error_fails_at_deadline_with_latest_error() {
        let started = Instant::now();
        let mut state = CleanupProbeState::default();

        assert_eq!(
            state.observe(started, Err("first probe error".to_owned()), true),
            CleanupProbeDecision::Continue
        );
        assert_eq!(
            state.observe(
                started + CLEANUP_DEADLINE,
                Err("latest probe error".to_owned()),
                true,
            ),
            CleanupProbeDecision::DeadlineExceeded
        );
        assert_eq!(state.deadline_failure(42), "latest probe error".to_owned());
    }
}
