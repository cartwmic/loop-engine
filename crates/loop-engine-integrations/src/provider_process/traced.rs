use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CapturedStream, ProcessError, ProcessObservation, run_observed};
use crate::provider_protocol::dto::ProviderRole;
use crate::trace::{TraceCategory, TraceError, TraceEvent, TraceWriter};

#[derive(Debug, thiserror::Error)]
pub(crate) enum TraceBoundaryError {
    #[error("trace writer lock unavailable")]
    LockUnavailable,
    #[error(transparent)]
    Trace(#[from] TraceError),
}

#[derive(Clone)]
pub struct TracedProviderBoundary {
    writer: Arc<Mutex<TraceWriter>>,
}

impl TracedProviderBoundary {
    pub fn new(writer: Arc<Mutex<TraceWriter>>) -> Self {
        Self { writer }
    }

    pub(crate) fn begin(
        &self,
        config: &ResolvedProviderConfig,
        invocation_id: &str,
        role: ProviderRole,
    ) -> Result<(), TraceBoundaryError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| TraceBoundaryError::LockUnavailable)?;
        writer.reserve_provider_call()?;
        let mut payload = BTreeMap::new();
        payload.insert("invocation_id".into(), json!(invocation_id));
        payload.insert("role".into(), json!(role));
        payload.insert(
            "registration_id".into(),
            json!(config.registration_id().as_str()),
        );
        payload.insert("executable".into(), json!(config.config().executable()));
        payload.insert(
            "argv".into(),
            json!(
                config
                    .config()
                    .argv()
                    .iter()
                    .map(|value| value.as_str())
                    .collect::<Vec<_>>()
            ),
        );
        payload.insert(
            "working_directory".into(),
            json!(config.config().working_directory()),
        );
        payload.insert(
            "timeout_seconds".into(),
            json!(config.config().timeout_seconds()),
        );
        let event = TraceEvent::new(
            writer.request_id(),
            TraceCategory::Provider,
            "start",
            payload,
        );
        if let Err(error) = writer.write(&event) {
            let _ = writer.release_provider_call();
            return Err(error.into());
        }
        Ok(())
    }

    pub(crate) fn execute(
        &self,
        config: &ResolvedProviderConfig,
        request: &[u8],
    ) -> ProcessObservation {
        run_observed(config, request)
    }

    pub(crate) fn finish(
        &self,
        invocation_id: &str,
        role: ProviderRole,
        request: Value,
        observation: &ProcessObservation,
        protocol_parsed: bool,
        failure_code: Option<&str>,
    ) -> Result<(), TraceBoundaryError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| TraceBoundaryError::LockUnavailable)?;
        let event_name = if failure_code.is_some() {
            "failure"
        } else {
            "finish"
        };
        let mut payload = BTreeMap::new();
        payload.insert("invocation_id".into(), json!(invocation_id));
        payload.insert("role".into(), json!(role));
        payload.insert("request".into(), request);
        stream_payload(&mut payload, "stdout", &observation.stdout);
        payload.insert(
            "stderr_b64".into(),
            Value::String(base64(&observation.stderr.retained)),
        );
        payload.insert(
            "stderr_byte_length".into(),
            json!(observation.stderr.original_length),
        );
        payload.insert(
            "stderr_truncated".into(),
            json!(observation.stderr.truncated),
        );
        payload.insert(
            "result_digest".into(),
            if protocol_parsed {
                Value::String(sha256(&observation.stdout.retained))
            } else {
                Value::Null
            },
        );
        payload.insert(
            "result_byte_length".into(),
            if protocol_parsed {
                json!(observation.stdout.original_length)
            } else {
                Value::Null
            },
        );
        payload.insert(
            "duration_ms".into(),
            json!(observation.duration.as_millis()),
        );
        payload.insert("exit_status".into(), json!(observation.exit_status));
        if let Some(code) = failure_code {
            payload.insert("failure_code".into(), json!(code));
        }
        let event = TraceEvent::new(
            writer.request_id(),
            TraceCategory::Provider,
            event_name,
            payload,
        );
        let write_result = writer.write(&event).map(|_| ());
        let release_result = writer.release_provider_call().map(|_| ());
        write_result.and(release_result).map_err(Into::into)
    }
}

pub fn process_failure_code(error: &ProcessError) -> &'static str {
    match error {
        ProcessError::RequestOversized { .. } => "resource.exhausted",
        ProcessError::ExecutableNotFound(_) => "provider.executable.not_found",
        ProcessError::PreLaunchSpawn(_)
        | ProcessError::Spawn(_)
        | ProcessError::TimeoutOutOfRange(_)
        | ProcessError::Stdin(_)
        | ProcessError::Stream(_)
        | ProcessError::Termination(_) => "provider.spawn.failed",
        ProcessError::Timeout => "provider.timeout",
        ProcessError::Crash(_) => "provider.crash",
        ProcessError::Signal(_) => "provider.signal",
        ProcessError::NonZero(_) => "provider.nonzero_exit",
        ProcessError::StdoutOversized { .. } => "provider.protocol.oversized",
        ProcessError::InvalidUtf8 => "provider.protocol.invalid_utf8",
        ProcessError::Malformed(_) => "provider.protocol.malformed",
    }
}

fn stream_payload(payload: &mut BTreeMap<String, Value>, prefix: &str, stream: &CapturedStream) {
    payload.insert(
        format!("{prefix}_b64"),
        Value::String(base64(&stream.retained)),
    );
    payload.insert(
        format!("{prefix}_original_length"),
        json!(stream.original_length),
    );
    payload.insert(format!("{prefix}_truncated"), json!(stream.truncated));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}
