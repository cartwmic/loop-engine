use std::io::{self, Write};

use crate::protocol::{AnyRequest, AnyResult, PROTOCOL_MAJOR_V1, result_envelope};
use crate::scenarios::Scenario;

pub const PROVIDER_RESULT_STDOUT_BYTES: usize = 1_048_576;
pub const PROVIDER_STDERR_TRACE_BYTES: usize = 1_048_576;

pub enum ProcessOutcome {
    Success,
    MalformedJson,
    ExtraStdout,
    MissingStdout,
    WrongMajor,
    NonZeroExit,
    Signal,
    Timeout,
    OversizedStdout,
    OversizedStderr,
    InvalidUtf8,
}

impl Scenario {
    pub fn process_outcome(self) -> Option<ProcessOutcome> {
        match self {
            Self::ProcessMalformedJson => Some(ProcessOutcome::MalformedJson),
            Self::ProcessExtraStdout => Some(ProcessOutcome::ExtraStdout),
            Self::ProcessMissingStdout => Some(ProcessOutcome::MissingStdout),
            Self::ProcessWrongMajor => Some(ProcessOutcome::WrongMajor),
            Self::ProcessNonzeroExit => Some(ProcessOutcome::NonZeroExit),
            Self::ProcessSignal => Some(ProcessOutcome::Signal),
            Self::ProcessTimeout => Some(ProcessOutcome::Timeout),
            Self::ProcessOversizedStdout => Some(ProcessOutcome::OversizedStdout),
            Self::ProcessOversizedStderr => Some(ProcessOutcome::OversizedStderr),
            Self::ProcessInvalidUtf8 => Some(ProcessOutcome::InvalidUtf8),
            _ => None,
        }
    }
}

pub fn emit_stdout(
    scenario: Scenario,
    request: &AnyRequest,
    result: AnyResult,
) -> io::Result<ProcessOutcome> {
    let Some(outcome) = scenario.process_outcome() else {
        let envelope = result_envelope(request.role, &request.invocation_id, result);
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &envelope)?;
        stdout.write_all(b"\n")?;
        return Ok(ProcessOutcome::Success);
    };

    match outcome {
        ProcessOutcome::MalformedJson => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(b"{not-json")?;
        }
        ProcessOutcome::ExtraStdout => {
            let envelope = result_envelope(request.role, &request.invocation_id, result);
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, &envelope)?;
            stdout.write_all(b"\nEXTRA")?;
        }
        ProcessOutcome::MissingStdout => {}
        ProcessOutcome::WrongMajor => {
            let mut envelope = result_envelope(request.role, &request.invocation_id, result);
            envelope["protocol_major"] = serde_json::json!(2);
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, &envelope)?;
            stdout.write_all(b"\n")?;
        }
        ProcessOutcome::OversizedStdout => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&[b'x'; PROVIDER_RESULT_STDOUT_BYTES + 1])?;
        }
        ProcessOutcome::InvalidUtf8 => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&[0xff, 0xfe, 0xfd])?;
        }
        ProcessOutcome::OversizedStderr => {
            let envelope = result_envelope(request.role, &request.invocation_id, result);
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, &envelope)?;
            stdout.write_all(b"\n")?;
            let mut stderr = io::stderr().lock();
            stderr.write_all(&[b'y'; PROVIDER_STDERR_TRACE_BYTES + 1])?;
            return Ok(ProcessOutcome::OversizedStderr);
        }
        ProcessOutcome::NonZeroExit | ProcessOutcome::Signal | ProcessOutcome::Timeout => {
            let envelope = result_envelope(request.role, &request.invocation_id, result);
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, &envelope)?;
            stdout.write_all(b"\n")?;
        }
        ProcessOutcome::Success => unreachable!(),
    }

    Ok(outcome)
}

pub fn finalize_process(outcome: ProcessOutcome) -> ! {
    match outcome {
        ProcessOutcome::Success | ProcessOutcome::OversizedStderr => std::process::exit(0),
        ProcessOutcome::NonZeroExit => std::process::exit(1),
        ProcessOutcome::Signal => std::process::abort(),
        ProcessOutcome::Timeout => loop {
            std::thread::park();
        },
        ProcessOutcome::MalformedJson
        | ProcessOutcome::ExtraStdout
        | ProcessOutcome::MissingStdout
        | ProcessOutcome::WrongMajor
        | ProcessOutcome::OversizedStdout
        | ProcessOutcome::InvalidUtf8 => std::process::exit(0),
    }
}

pub fn read_stdin_request() -> io::Result<String> {
    io::read_to_string(io::stdin())
}

pub fn parse_request(raw: &str) -> Result<AnyRequest, serde_json::Error> {
    serde_json::from_str(raw)
}

pub fn validate_request(request: &AnyRequest) -> Result<(), &'static str> {
    if request.protocol_major != PROTOCOL_MAJOR_V1 {
        return Err("unsupported protocol major");
    }
    Ok(())
}
