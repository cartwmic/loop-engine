#![allow(dead_code)]

//! Test-only bounded subprocess execution for the affected integration suites.
//!
//! Every child is placed in its own process group. Output is drained from both
//! pipes while the child runs so a descendant cannot wedge the test harness by
//! retaining a capture pipe after the direct child exits.

use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt as UnixCommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub const ORDINARY_DEADLINE: Duration = Duration::from_secs(60);

const TERMINATION_GRACE: Duration = Duration::from_millis(250);
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// The completed output and any error encountered while writing the supplied
/// stdin. A broken pipe remains observable to tests that previously asserted
/// that behavior.
pub struct Completed {
    pub output: Output,
    pub stdin_error: Option<io::Error>,
}

pub trait CommandExt {
    fn bounded_output(&mut self, phase: &str) -> io::Result<Output>;
}

impl CommandExt for Command {
    fn bounded_output(&mut self, phase: &str) -> io::Result<Output> {
        run(self, phase)
    }
}

pub fn run(command: &mut Command, phase: &str) -> io::Result<Output> {
    run_with_deadline(command, phase, ORDINARY_DEADLINE)
}

pub fn run_with_deadline(
    command: &mut Command,
    phase: &str,
    deadline: Duration,
) -> io::Result<Output> {
    run_with_stdin_deadline(command, phase, &[], deadline).map(|completed| completed.output)
}

pub fn run_with_stdin(command: &mut Command, phase: &str, stdin: &[u8]) -> io::Result<Completed> {
    run_with_stdin_deadline(command, phase, stdin, ORDINARY_DEADLINE)
}

pub fn prepare_process_group(command: &mut Command) {
    // SAFETY: this closure runs in the freshly forked child before exec. It
    // only places that child in a process group whose id is its own pid.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

pub fn wait_existing(child: Child, phase: &str) -> io::Result<Output> {
    wait_existing_with_deadline(child, phase, ORDINARY_DEADLINE)
}

pub fn wait_existing_with_deadline(
    mut child: Child,
    phase: &str,
    deadline: Duration,
) -> io::Result<Output> {
    let pgid = child.id() as i32;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("bounded subprocess stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("bounded subprocess stderr was not piped"))?;
    let (stdout_rx, stdout_thread) = drain(stdout);
    let (stderr_rx, stderr_thread) = drain(stderr);
    let deadline_at = Instant::now() + deadline;
    let mut status = None;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let mut failure = None;

    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(next) => status = next,
                Err(error) => {
                    failure = Some(format!("could not poll direct child: {error}"));
                    break;
                }
            }
        }
        collect(&stdout_rx, &mut stdout_result);
        collect(&stderr_rx, &mut stderr_result);
        if let Some(Err(error)) = stdout_result.as_ref() {
            failure = Some(format!("stdout drain failed: {error}"));
            break;
        }
        if let Some(Err(error)) = stderr_result.as_ref() {
            failure = Some(format!("stderr drain failed: {error}"));
            break;
        }
        if status.is_some() && stdout_result.is_some() && stderr_result.is_some() {
            break;
        }
        if Instant::now() >= deadline_at {
            failure = Some(if status.is_some() {
                "direct child exited but an output pipe did not reach EOF".to_owned()
            } else {
                "direct child exceeded subprocess deadline".to_owned()
            });
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    if let Some(reason) = failure {
        terminate_group(&mut child, pgid);
        let stdout_result = finish_stream(stdout_result, stdout_rx, phase, pgid, "stdout")?;
        let stderr_result = finish_stream(stderr_result, stderr_rx, phase, pgid, "stderr")?;
        join_finished(stdout_thread);
        join_finished(stderr_thread);
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "{phase}: {reason} (pgid {pgid}); stdout={} bytes, stderr={} bytes",
                stdout_result.len(),
                stderr_result.len()
            ),
        ));
    }

    let status = status.ok_or_else(|| {
        io::Error::other(format!(
            "{phase}: bounded subprocess ended without an exit status (pgid {pgid})"
        ))
    })?;
    let stdout = match stdout_result {
        Some(Ok(bytes)) => bytes,
        Some(Err(error)) => return Err(with_diagnostic(phase, pgid, error)),
        None => return Err(diagnostic(phase, pgid, "stdout drain did not complete")),
    };
    let stderr = match stderr_result {
        Some(Ok(bytes)) => bytes,
        Some(Err(error)) => return Err(with_diagnostic(phase, pgid, error)),
        None => return Err(diagnostic(phase, pgid, "stderr drain did not complete")),
    };
    join_finished(stdout_thread);
    join_finished(stderr_thread);
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub fn run_with_stdin_deadline(
    command: &mut Command,
    phase: &str,
    stdin: &[u8],
    deadline: Duration,
) -> io::Result<Completed> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    prepare_process_group(command);

    let mut child = command.spawn()?;
    let pgid = child.id() as i32;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("bounded subprocess stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("bounded subprocess stderr was not piped"))?;
    let stdin_pipe = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("bounded subprocess stdin was not piped"))?;

    let (stdout_rx, stdout_thread) = drain(stdout);
    let (stderr_rx, stderr_thread) = drain(stderr);
    let (stdin_rx, stdin_thread) = write_stdin(stdin_pipe, stdin.to_owned());
    let deadline_at = Instant::now() + deadline;

    let mut status = None;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let mut stdin_result = None;
    let mut failure = None;

    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(next) => status = next,
                Err(error) => {
                    failure = Some(format!("could not poll direct child: {error}"));
                    break;
                }
            }
        }
        collect(&stdout_rx, &mut stdout_result);
        collect(&stderr_rx, &mut stderr_result);
        collect(&stdin_rx, &mut stdin_result);

        if let Some(Err(error)) = stdout_result.as_ref() {
            failure = Some(format!("stdout drain failed: {error}"));
            break;
        }
        if let Some(Err(error)) = stderr_result.as_ref() {
            failure = Some(format!("stderr drain failed: {error}"));
            break;
        }
        if status.is_some()
            && stdout_result.is_some()
            && stderr_result.is_some()
            && stdin_result.is_some()
        {
            break;
        }
        if Instant::now() >= deadline_at {
            failure = Some(if status.is_some() {
                "direct child exited but an output pipe did not reach EOF".to_owned()
            } else {
                "direct child exceeded subprocess deadline".to_owned()
            });
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    if let Some(reason) = failure {
        terminate_group(&mut child, pgid);
        let stdout_result = finish_stream(stdout_result, stdout_rx, phase, pgid, "stdout")?;
        let stderr_result = finish_stream(stderr_result, stderr_rx, phase, pgid, "stderr")?;
        let _ = finish_stdin(stdin_result, stdin_rx);
        join_finished(stdout_thread);
        join_finished(stderr_thread);
        join_finished(stdin_thread);
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "{phase}: {reason} (pgid {pgid}); stdout={} bytes, stderr={} bytes",
                stdout_result.len(),
                stderr_result.len()
            ),
        ));
    }

    let status = status.ok_or_else(|| {
        io::Error::other(format!(
            "{phase}: bounded subprocess ended without an exit status (pgid {pgid})"
        ))
    })?;
    let stdout = match stdout_result {
        Some(Ok(bytes)) => bytes,
        Some(Err(error)) => return Err(with_diagnostic(phase, pgid, error)),
        None => return Err(diagnostic(phase, pgid, "stdout drain did not complete")),
    };
    let stderr = match stderr_result {
        Some(Ok(bytes)) => bytes,
        Some(Err(error)) => return Err(with_diagnostic(phase, pgid, error)),
        None => return Err(diagnostic(phase, pgid, "stderr drain did not complete")),
    };
    let stdin_error = match stdin_result {
        Some(Ok(())) => None,
        Some(Err(error)) => Some(error),
        None => return Err(diagnostic(phase, pgid, "stdin writer did not complete")),
    };

    join_finished(stdout_thread);
    join_finished(stderr_thread);
    join_finished(stdin_thread);
    Ok(Completed {
        output: Output {
            status,
            stdout,
            stderr,
        },
        stdin_error,
    })
}

fn drain<R>(mut reader: R) -> (Receiver<io::Result<Vec<u8>>>, thread::JoinHandle<()>)
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    (receiver, handle)
}

fn write_stdin(
    mut stdin: impl Write + Send + 'static,
    bytes: Vec<u8>,
) -> (Receiver<io::Result<()>>, thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let result = stdin.write_all(&bytes);
        drop(stdin);
        let _ = sender.send(result);
    });
    (receiver, handle)
}

fn collect<T>(receiver: &Receiver<T>, slot: &mut Option<T>) {
    if slot.is_none() {
        if let Ok(value) = receiver.try_recv() {
            *slot = Some(value);
        }
    }
}

fn terminate_group(child: &mut Child, pgid: i32) {
    // SAFETY: pgid was assigned by setpgid(0, 0) in this child, so the
    // negative pid targets only the subprocess group created for this call.
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGTERM);
    }
    let grace_deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < grace_deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // SIGKILL also handles children that deliberately ignore SIGTERM.
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.wait();
}

fn finish_stream(
    result: Option<io::Result<Vec<u8>>>,
    receiver: Receiver<io::Result<Vec<u8>>>,
    phase: &str,
    pgid: i32,
    stream: &str,
) -> io::Result<Vec<u8>> {
    match result {
        Some(result) => result.map_err(|error| with_diagnostic(phase, pgid, error)),
        None => match receiver.recv_timeout(DRAIN_GRACE) {
            Ok(result) => result.map_err(|error| with_diagnostic(phase, pgid, error)),
            Err(RecvTimeoutError::Timeout) => Err(diagnostic(
                phase,
                pgid,
                &format!("{stream} pipe did not drain after process-group cleanup"),
            )),
            Err(RecvTimeoutError::Disconnected) => Err(diagnostic(
                phase,
                pgid,
                &format!("{stream} drain thread disconnected"),
            )),
        },
    }
}

fn finish_stdin(
    result: Option<io::Result<()>>,
    receiver: Receiver<io::Result<()>>,
) -> Option<io::Error> {
    match result {
        Some(result) => result.err(),
        None => receiver
            .recv_timeout(DRAIN_GRACE)
            .ok()
            .and_then(Result::err),
    }
}

fn with_diagnostic(phase: &str, pgid: i32, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{phase}: {error} (pgid {pgid})"))
}

fn diagnostic(phase: &str, pgid: i32, message: &str) -> io::Error {
    io::Error::other(format!("{phase}: {message} (pgid {pgid})"))
}

fn join_finished(handle: thread::JoinHandle<()>) {
    let _ = handle.join();
}
