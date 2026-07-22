//! Stable exit codes and stdout/stderr contract finalization (T128).
//!
//! Maps authoritative pre-dispatch failures, driver metadata, and dispatched
//! [`PublicOutcome`] values into frozen exit codes and byte destinations.
//! Presentation only — no policy decision or outcome reclassification.

use loop_engine_core::model::outcome::OutcomeClass;
use serde_json::{Value, json};
use thiserror::Error;

use crate::diagnostics::{
    DiagnosticRenderError, PreDispatchFailureDto, render_pre_dispatch_human,
    render_pre_dispatch_json,
};
use crate::render::dto::{OutcomeRenderError, OutcomeRenderRequest};
use crate::render::human::render_human_outcome;
use crate::render::json::render_structured_outcome;

/// Exit code for a successfully completed application operation.
pub const EXIT_COMPLETED: i32 = 0;
/// Exit code for an internal/runtime error during application dispatch.
pub const EXIT_ERROR: i32 = 1;
/// Exit code for a domain-rejected application operation.
pub const EXIT_REJECTED: i32 = 2;
/// Exit code for usage, configuration, platform, persistence-open, or parse failure before dispatch.
pub const EXIT_PRE_DISPATCH: i32 = 64;

/// Output rendering mode frozen in [cli-contract.md].
///
/// [cli-contract.md]: ../../../docs/cli-contract.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Human,
    Json,
}

/// Where contract-permitted bytes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteDestination {
    Stdout,
    Stderr,
}

/// Final process streams and exit code for one CLI invocation path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ProcessOutput {
    pub fn stdout_destination(&self) -> Option<ByteDestination> {
        if self.stdout.is_empty() {
            None
        } else {
            Some(ByteDestination::Stdout)
        }
    }

    pub fn stderr_destination(&self) -> Option<ByteDestination> {
        if self.stderr.is_empty() {
            None
        } else {
            Some(ByteDestination::Stderr)
        }
    }
}

/// Authoritative invocation phase for stdout/stderr routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationPhase {
    PreDispatch,
    DriverMetadata,
    ApplicationDispatch,
}

impl InvocationPhase {
    pub fn stdout_destination(self, has_payload: bool) -> Option<ByteDestination> {
        match self {
            Self::PreDispatch => None,
            Self::DriverMetadata | Self::ApplicationDispatch if has_payload => {
                Some(ByteDestination::Stdout)
            }
            Self::DriverMetadata | Self::ApplicationDispatch => None,
        }
    }

    pub fn stderr_destination(self, has_payload: bool) -> Option<ByteDestination> {
        match self {
            Self::PreDispatch if has_payload => Some(ByteDestination::Stderr),
            Self::PreDispatch => None,
            Self::DriverMetadata => None,
            Self::ApplicationDispatch if has_payload => Some(ByteDestination::Stderr),
            Self::ApplicationDispatch => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExitRenderError {
    #[error(transparent)]
    Diagnostic(#[from] DiagnosticRenderError),
    #[error(transparent)]
    Outcome(#[from] OutcomeRenderError),
}

/// Maps a dispatched outcome class to the frozen exit code table.
pub fn exit_code_for_outcome(class: OutcomeClass) -> i32 {
    match class {
        OutcomeClass::Completed => EXIT_COMPLETED,
        OutcomeClass::Rejected => EXIT_REJECTED,
        OutcomeClass::Error => EXIT_ERROR,
    }
}

/// Pre-dispatch failures always exit `64` with empty stdout and rich stderr.
pub fn finalize_pre_dispatch(
    format: RenderFormat,
    failure: &PreDispatchFailureDto,
) -> Result<ProcessOutput, ExitRenderError> {
    let stderr = match format {
        RenderFormat::Human => append_single_newline(render_pre_dispatch_human(failure)),
        RenderFormat::Json => append_single_newline(render_pre_dispatch_json(failure)?),
    };
    Ok(ProcessOutput {
        exit_code: EXIT_PRE_DISPATCH,
        stdout: Vec::new(),
        stderr,
    })
}

/// Driver `--help` metadata: stdout only, exit `0`.
pub fn finalize_driver_help(
    format: RenderFormat,
    usage: &str,
    request_id: &str,
    trace_path: &str,
) -> ProcessOutput {
    let stdout = match format {
        RenderFormat::Human => usage.as_bytes().to_vec(),
        RenderFormat::Json => json_line(&json!({
            "schema_version": 1,
            "kind": "help",
            "usage": usage,
            "request_id": request_id,
            "trace": trace_path,
        })),
    };
    ProcessOutput {
        exit_code: EXIT_COMPLETED,
        stdout,
        stderr: Vec::new(),
    }
}

/// Driver `--version` metadata: stdout only, exit `0`.
pub fn finalize_driver_version(
    format: RenderFormat,
    version: &str,
    request_id: &str,
    trace_path: &str,
) -> ProcessOutput {
    let stdout = match format {
        RenderFormat::Human => format!("loop-engine {version}\n").into_bytes(),
        RenderFormat::Json => json_line(&json!({
            "schema_version": 1,
            "kind": "version",
            "name": "loop-engine",
            "version": version,
            "request_id": request_id,
            "trace": trace_path,
        })),
    };
    ProcessOutput {
        exit_code: EXIT_COMPLETED,
        stdout,
        stderr: Vec::new(),
    }
}

/// Driver `--list-operations` metadata: stdout only, exit `0`.
pub fn finalize_driver_list_operations(
    format: RenderFormat,
    operations: &[(&str, &str)],
    request_id: &str,
    trace_path: &str,
) -> ProcessOutput {
    let stdout = match format {
        RenderFormat::Human => operations
            .iter()
            .map(|(id, argv)| format!("{id}\t{argv}\n"))
            .collect::<String>()
            .into_bytes(),
        RenderFormat::Json => {
            let rows = operations
                .iter()
                .map(|(id, argv)| json!({ "id": id, "argv": argv }))
                .collect::<Vec<_>>();
            json_line(&json!({
                "schema_version": 1,
                "kind": "operation_list",
                "operations": rows,
                "request_id": request_id,
                "trace": trace_path,
            }))
        }
    };
    ProcessOutput {
        exit_code: EXIT_COMPLETED,
        stdout,
        stderr: Vec::new(),
    }
}

/// Dispatched application outcomes: one stdout payload, empty stderr, exit `0`/`2`/`1`.
pub fn finalize_dispatched_outcome(
    format: RenderFormat,
    request: &OutcomeRenderRequest<'_>,
) -> Result<ProcessOutput, ExitRenderError> {
    let exit_code = exit_code_for_outcome(request.outcome.class());
    let stdout = match format {
        RenderFormat::Json => {
            let rendered = render_structured_outcome(request)?;
            append_single_newline(rendered)
        }
        RenderFormat::Human => append_single_newline(render_human_outcome(request)?),
    };
    Ok(ProcessOutput {
        exit_code,
        stdout,
        stderr: Vec::new(),
    })
}

/// Late post-dispatch envelope construction failure: stderr only, exit `1`.
pub fn finalize_dispatched_render_failure(format: RenderFormat, message: &str) -> ProcessOutput {
    let stderr = match format {
        RenderFormat::Human => format!("Error: {message}\n").into_bytes(),
        RenderFormat::Json => json_line(&json!({
            "schema_version": 1,
            "message": message,
        })),
    };
    ProcessOutput {
        exit_code: EXIT_ERROR,
        stdout: Vec::new(),
        stderr,
    }
}

fn append_single_newline(payload: String) -> Vec<u8> {
    let mut bytes = payload.into_bytes();
    if bytes.last() != Some(&b'\n') {
        bytes.push(b'\n');
    }
    bytes
}

fn json_line(value: &Value) -> Vec<u8> {
    append_single_newline(serde_json::to_string(value).expect("driver metadata serializes to json"))
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome};
    use loop_engine_core::model::reason::{Reason, ReasonCode};

    #[test]
    fn exit_codes_match_frozen_contract_table() {
        assert_eq!(
            exit_code_for_outcome(OutcomeClass::Completed),
            EXIT_COMPLETED
        );
        assert_eq!(exit_code_for_outcome(OutcomeClass::Rejected), EXIT_REJECTED);
        assert_eq!(exit_code_for_outcome(OutcomeClass::Error), EXIT_ERROR);
        assert_eq!(EXIT_PRE_DISPATCH, 64);
    }

    #[test]
    fn pre_dispatch_routes_empty_stdout_and_stderr_only() {
        assert_eq!(InvocationPhase::PreDispatch.stdout_destination(false), None);
        assert_eq!(
            InvocationPhase::PreDispatch.stderr_destination(true),
            Some(ByteDestination::Stderr)
        );
    }

    #[test]
    fn dispatched_render_failure_exits_error_with_stderr_only() {
        let output = finalize_dispatched_render_failure(
            RenderFormat::Json,
            "structured CLI envelope exceeds bound",
        );
        assert_eq!(output.exit_code, EXIT_ERROR);
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }

    #[test]
    fn envelope_render_error_preserves_outcome_class_exit_mapping_helper() {
        let outcome = PublicOutcome::new(
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::RunNotFound, "run not found").expect("reason")),
            OutcomeData::new(None, None, None).expect("data"),
            vec![],
        )
        .expect("outcome");
        assert_eq!(exit_code_for_outcome(outcome.class()), EXIT_REJECTED);
    }
}
