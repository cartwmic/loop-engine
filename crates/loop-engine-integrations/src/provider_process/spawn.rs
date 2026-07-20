use std::io::Write;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;

use super::error::ProcessError;
use super::process_group::{leader_exited, verify_group};
use super::streams::{CapturedStream, drain};
use super::timeout::{POLL_INTERVAL, terminate_group};
use crate::provider_protocol::validation::{
    PROVIDER_REQUEST_JSON_BYTES, PROVIDER_RESULT_STDOUT_BYTES,
};

pub const PROVIDER_STDERR_BYTES: usize = 1_048_576;

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessCapture {
    pub stdout: Vec<u8>,
    pub stderr: CapturedStream,
}

#[derive(Debug)]
pub struct ProcessObservation {
    pub result: Result<(), ProcessError>,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
    pub duration: Duration,
    pub exit_status: Option<i32>,
}

#[cfg(test)]
pub fn run(
    config: &ResolvedProviderConfig,
    request: &[u8],
) -> Result<ProcessCapture, ProcessError> {
    let observation = run_observed(config, request);
    observation.result?;
    Ok(ProcessCapture {
        stdout: observation.stdout.retained,
        stderr: observation.stderr,
    })
}

pub fn run_observed(config: &ResolvedProviderConfig, request: &[u8]) -> ProcessObservation {
    let started = Instant::now();
    if request.len() > PROVIDER_REQUEST_JSON_BYTES {
        return observation(
            Err(ProcessError::RequestOversized {
                max: PROVIDER_REQUEST_JSON_BYTES,
                actual: request.len(),
            }),
            started,
            None,
            empty_stream(),
            empty_stream(),
        );
    }

    let provider = config.config();
    let timeout_seconds = provider.timeout_seconds();
    let Some(deadline) = started.checked_add(Duration::from_secs(timeout_seconds)) else {
        return observation(
            Err(ProcessError::TimeoutOutOfRange(timeout_seconds)),
            started,
            None,
            empty_stream(),
            empty_stream(),
        );
    };
    let mut command = Command::new(provider.executable());
    command
        .args(provider.argv().iter().map(|argument| argument.as_str()))
        .current_dir(provider.working_directory())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let source = if error.kind() == std::io::ErrorKind::NotFound {
                ProcessError::ExecutableNotFound(provider.executable().to_owned())
            } else {
                ProcessError::Spawn(error)
            };
            return observation(Err(source), started, None, empty_stream(), empty_stream());
        }
    };
    let process_group_id = child.id();
    if let Err(source) = verify_group(child.id(), process_group_id) {
        let _ = child.kill();
        let _ = child.wait();
        return observation(Err(source), started, None, empty_stream(), empty_stream());
    }
    let mut stdin = child.stdin.take().expect("piped provider stdin");
    let stdout = child.stdout.take().expect("piped provider stdout");
    let stderr = child.stderr.take().expect("piped provider stderr");
    let request = request.to_vec();
    let stdin_thread = match thread::Builder::new()
        .name("provider-stdin".into())
        .spawn(move || {
            let result = stdin.write_all(&request).and_then(|()| stdin.flush());
            drop(stdin);
            result
        }) {
        Ok(thread) => thread,
        Err(error) => {
            let _ = terminate_group(process_group_id);
            let _ = child.wait();
            return observation(
                Err(ProcessError::Spawn(error)),
                started,
                None,
                empty_stream(),
                empty_stream(),
            );
        }
    };
    let stdout_thread = match thread::Builder::new()
        .name("provider-stdout".into())
        .spawn(move || drain(stdout, PROVIDER_RESULT_STDOUT_BYTES))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = terminate_group(process_group_id);
            let _ = child.wait();
            let _ = join_writer(stdin_thread);
            return observation(
                Err(ProcessError::Spawn(error)),
                started,
                None,
                empty_stream(),
                empty_stream(),
            );
        }
    };
    let stderr_thread = match thread::Builder::new()
        .name("provider-stderr".into())
        .spawn(move || drain(stderr, PROVIDER_STDERR_BYTES))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = terminate_group(process_group_id);
            let _ = child.wait();
            let _ = join_writer(stdin_thread);
            let _ = join_reader(stdout_thread);
            return observation(
                Err(ProcessError::Spawn(error)),
                started,
                None,
                empty_stream(),
                empty_stream(),
            );
        }
    };

    let status = loop {
        if Instant::now() >= deadline {
            if terminate_group(process_group_id).is_err() {
                drop(stdin_thread);
                drop(stdout_thread);
                drop(stderr_thread);
                return observation(
                    Err(ProcessError::Timeout),
                    started,
                    None,
                    empty_stream(),
                    empty_stream(),
                );
            }
            let _ = child.wait();
            let drain_deadline = Instant::now() + Duration::from_secs(1);
            while !(stdin_thread.is_finished()
                && stdout_thread.is_finished()
                && stderr_thread.is_finished())
                && Instant::now() < drain_deadline
            {
                thread::sleep(POLL_INTERVAL);
            }
            if !(stdin_thread.is_finished()
                && stdout_thread.is_finished()
                && stderr_thread.is_finished())
            {
                drop(stdin_thread);
                drop(stdout_thread);
                drop(stderr_thread);
                return observation(
                    Err(ProcessError::Timeout),
                    started,
                    None,
                    empty_stream(),
                    empty_stream(),
                );
            }
            let _ = join_writer(stdin_thread);
            let stdout = join_reader(stdout_thread).unwrap_or_else(|_| empty_stream());
            let stderr = join_reader(stderr_thread).unwrap_or_else(|_| empty_stream());
            return observation(Err(ProcessError::Timeout), started, None, stdout, stderr);
        }
        if stdin_thread.is_finished() && stdout_thread.is_finished() && stderr_thread.is_finished()
        {
            match leader_exited(child.id(), process_group_id) {
                Ok(true) => {
                    if let Err(source) = terminate_group(process_group_id) {
                        let _ = child.kill();
                        let _ = child.wait();
                        let stdout = join_reader(stdout_thread).unwrap_or_else(|_| empty_stream());
                        let stderr = join_reader(stderr_thread).unwrap_or_else(|_| empty_stream());
                        let _ = join_writer(stdin_thread);
                        return observation(Err(source), started, None, stdout, stderr);
                    }
                    match child.wait() {
                        Ok(observed) => break observed,
                        Err(error) => {
                            let stdout =
                                join_reader(stdout_thread).unwrap_or_else(|_| empty_stream());
                            let stderr =
                                join_reader(stderr_thread).unwrap_or_else(|_| empty_stream());
                            let _ = join_writer(stdin_thread);
                            return observation(
                                Err(ProcessError::Spawn(error)),
                                started,
                                None,
                                stdout,
                                stderr,
                            );
                        }
                    }
                }
                Ok(false) => {}
                Err(source) => {
                    let _ = terminate_group(process_group_id);
                    let _ = child.wait();
                    let stdout = join_reader(stdout_thread).unwrap_or_else(|_| empty_stream());
                    let stderr = join_reader(stderr_thread).unwrap_or_else(|_| empty_stream());
                    let _ = join_writer(stdin_thread);
                    return observation(Err(source), started, None, stdout, stderr);
                }
            }
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    };

    let stdin_result = join_writer(stdin_thread);
    let stdout = match join_reader(stdout_thread) {
        Ok(value) => value,
        Err(source) => {
            return observation(
                Err(source),
                started,
                Some(status.into_raw()),
                empty_stream(),
                join_reader(stderr_thread).unwrap_or_else(|_| empty_stream()),
            );
        }
    };
    let stderr = match join_reader(stderr_thread) {
        Ok(value) => value,
        Err(source) => {
            return observation(
                Err(source),
                started,
                Some(status.into_raw()),
                stdout,
                empty_stream(),
            );
        }
    };
    let raw_status = status.into_raw();
    let result = if let Some(signal) = status.signal() {
        if status.core_dumped() || is_crash_signal(signal) {
            Err(ProcessError::Crash(signal))
        } else {
            Err(ProcessError::Signal(signal))
        }
    } else if !status.success() {
        Err(ProcessError::NonZero(status.code()))
    } else if let Err(source) = stdin_result {
        Err(source)
    } else if stdout.original_length > PROVIDER_RESULT_STDOUT_BYTES {
        Err(ProcessError::StdoutOversized {
            max: PROVIDER_RESULT_STDOUT_BYTES,
            actual: stdout.original_length,
        })
    } else if std::str::from_utf8(&stdout.retained).is_err() {
        Err(ProcessError::InvalidUtf8)
    } else {
        Ok(())
    };
    observation(result, started, Some(raw_status), stdout, stderr)
}

#[cfg(target_os = "linux")]
fn is_crash_signal(signal: i32) -> bool {
    matches!(signal, 4 | 5 | 6 | 7 | 8 | 11 | 31)
}

#[cfg(target_os = "macos")]
fn is_crash_signal(signal: i32) -> bool {
    matches!(signal, 4 | 5 | 6 | 7 | 8 | 10 | 11 | 12)
}

fn observation(
    result: Result<(), ProcessError>,
    started: Instant,
    exit_status: Option<i32>,
    stdout: CapturedStream,
    stderr: CapturedStream,
) -> ProcessObservation {
    ProcessObservation {
        result,
        stdout,
        stderr,
        duration: started.elapsed(),
        exit_status,
    }
}

fn empty_stream() -> CapturedStream {
    CapturedStream {
        retained: Vec::new(),
        original_length: 0,
        truncated: false,
    }
}

fn join_writer(handle: thread::JoinHandle<std::io::Result<()>>) -> Result<(), ProcessError> {
    handle
        .join()
        .map_err(|_| ProcessError::Termination("provider stdin thread panicked".into()))?
        .map_err(ProcessError::Stdin)
}

fn join_reader(
    handle: thread::JoinHandle<std::io::Result<CapturedStream>>,
) -> Result<CapturedStream, ProcessError> {
    handle
        .join()
        .map_err(|_| ProcessError::Termination("provider stream thread panicked".into()))?
        .map_err(ProcessError::Stream)
}
