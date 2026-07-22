use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{InvocationError, InvocationFailure};
use loop_engine_core::model::attempt::{ProviderFact, ProviderRole as CoreProviderRole};
use loop_engine_core::model::bounded::BoundedText;
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::ids::RequestId;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::reason::{Reason, ReasonCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use super::dto::{
    PROTOCOL_MAJOR_V1, ProviderRole, RegistrationDto, RequestEnvelope, ResultEnvelope,
};
use super::validation::{
    PROVIDER_REQUEST_JSON_BYTES, PROVIDER_RESULT_STDOUT_BYTES, ProtocolValidationError,
    parse_provider_value, reject_topology_fields,
};
use crate::provider_process::{
    ProcessError, ProcessObservation, TracedProviderBoundary, process_failure_code,
};
use crate::sha256_digest::Sha256DigestComputer;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("provider request encoding failed: {0}")]
    RequestEncoding(String),
    #[error(transparent)]
    Protocol(#[from] ProtocolValidationError),
    #[error("unsupported provider protocol major {actual}; supported major is {supported}")]
    UnsupportedMajor { actual: u64, supported: u64 },
    #[error("provider result role mismatch: expected {expected:?}, got {actual:?}")]
    RoleMismatch {
        expected: ProviderRole,
        actual: ProviderRole,
    },
    #[error("provider result invocation ID mismatch")]
    InvocationMismatch,
    #[error("provider result mapping failed: {0}")]
    Mapping(String),
}

pub struct WireInvocation<R> {
    pub result: R,
    pub provider_version: Option<String>,
    pub protocol_major: u32,
    pub digest: DigestObservation,
    pub result_value: Value,
    pub graph_declaration_error: Option<String>,
    boundary: TracedProviderBoundary,
    observation: ProcessObservation,
    request_value: Value,
    invocation_id: String,
    role: ProviderRole,
}

impl<R> WireInvocation<R> {
    pub fn complete_trace(&self) -> Result<(), String> {
        self.boundary
            .finish(
                &self.invocation_id,
                self.role,
                self.request_value.clone(),
                &self.observation,
                true,
                None,
            )
            .map_err(|error| error.to_string())
    }

    fn fail_trace(&self, failure_code: &str) -> Option<String> {
        self.boundary
            .finish(
                &self.invocation_id,
                self.role,
                self.request_value.clone(),
                &self.observation,
                true,
                Some(failure_code),
            )
            .err()
            .map(|error| error.to_string())
    }
}

pub fn invoke<P, R>(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    payload: P,
    topology_forbidden: bool,
) -> Result<WireInvocation<R>, InvocationError<AdapterError>>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let digest = Sha256DigestComputer
        .executable_digest(config)
        .unwrap_or(DigestObservation::Unavailable);
    let request = RequestEnvelope {
        protocol_major: PROTOCOL_MAJOR_V1,
        role,
        invocation_id: request_id.as_str().to_owned(),
        registration: registration(config),
        payload,
    };
    let request_value = serde_json::to_value(&request).map_err(|error| {
        transport(
            AdapterError::RequestEncoding(error.to_string()),
            error_fact(config, request_id, role, digest.clone(), None, None),
            None,
        )
    })?;
    let bytes = serde_json::to_vec(&request_value).map_err(|error| {
        transport(
            AdapterError::RequestEncoding(error.to_string()),
            error_fact(config, request_id, role, digest.clone(), None, None),
            None,
        )
    })?;
    if bytes.len() > PROVIDER_REQUEST_JSON_BYTES {
        return Err(transport(
            AdapterError::Process(ProcessError::RequestOversized {
                max: PROVIDER_REQUEST_JSON_BYTES,
                actual: bytes.len(),
            }),
            error_fact(config, request_id, role, digest, None, None),
            None,
        ));
    }
    boundary
        .begin(config, request_id.as_str(), role)
        .map_err(|_| InvocationError::TraceBudgetUnavailable)?;
    let observation = boundary.execute(config, &bytes);
    if let Err(source) = &observation.result {
        let trace_finish = boundary.finish(
            request_id.as_str(),
            role,
            request_value,
            &observation,
            false,
            Some(process_failure_code(source)),
        );
        let source = observation.result.expect_err("checked process error");
        return Err(transport(
            AdapterError::Process(source),
            error_fact(config, request_id, role, digest, None, None),
            finish_failure(trace_finish),
        ));
    }
    let (raw, graph_declaration_error) = match parse_provider_value(
        &observation.stdout.retained,
        PROVIDER_RESULT_STDOUT_BYTES,
        role == ProviderRole::Describe,
    ) {
        Ok(parsed) => parsed,
        Err(source) => {
            let trace_finish = boundary.finish(
                request_id.as_str(),
                role,
                request_value,
                &observation,
                false,
                Some("provider.protocol.malformed"),
            );
            return Err(transport(
                AdapterError::Protocol(source),
                error_fact(config, request_id, role, digest, None, None),
                finish_failure(trace_finish),
            ));
        }
    };
    if let Some(actual) = raw.get("protocol_major").and_then(Value::as_u64)
        && actual != u64::from(PROTOCOL_MAJOR_V1)
    {
        let provider_version = valid_provider_version(raw.get("provider_version"))
            .ok()
            .flatten();
        let trace_finish = boundary.finish(
            request_id.as_str(),
            role,
            request_value,
            &observation,
            true,
            Some("provider.protocol.unsupported_major"),
        );
        return Err(transport(
            AdapterError::UnsupportedMajor {
                actual,
                supported: u64::from(PROTOCOL_MAJOR_V1),
            },
            error_fact(
                config,
                request_id,
                role,
                digest,
                provider_version,
                (actual > 0).then_some(actual),
            ),
            finish_failure(trace_finish),
        ));
    }
    let envelope = match serde_json::from_value::<ResultEnvelope<R>>(raw.clone()) {
        Ok(envelope) => envelope,
        Err(error) => {
            let source = ProtocolValidationError::Malformed(error.to_string());
            let trace_finish = boundary.finish(
                request_id.as_str(),
                role,
                request_value,
                &observation,
                true,
                Some("provider.protocol.malformed"),
            );
            return Err(transport(
                AdapterError::Protocol(source),
                error_fact(config, request_id, role, digest, None, None),
                finish_failure(trace_finish),
            ));
        }
    };
    let protocol_major = envelope.protocol_major;
    let provider_version = match valid_provider_version(raw.get("provider_version")) {
        Ok(value) => value,
        Err(source) => {
            let trace_finish = boundary.finish(
                request_id.as_str(),
                role,
                request_value,
                &observation,
                true,
                Some("provider.protocol.malformed"),
            );
            return Err(transport(
                AdapterError::Protocol(source),
                error_fact(
                    config,
                    request_id,
                    role,
                    digest,
                    None,
                    Some(u64::from(protocol_major)),
                ),
                finish_failure(trace_finish),
            ));
        }
    };
    if envelope.role != role {
        let actual = envelope.role;
        let trace_finish = boundary.finish(
            request_id.as_str(),
            role,
            request_value,
            &observation,
            true,
            Some("provider.protocol.malformed"),
        );
        return Err(transport(
            AdapterError::RoleMismatch {
                expected: role,
                actual,
            },
            error_fact(
                config,
                request_id,
                role,
                digest,
                provider_version,
                Some(u64::from(protocol_major)),
            ),
            finish_failure(trace_finish),
        ));
    }
    if envelope.invocation_id != request_id.as_str() {
        let trace_finish = boundary.finish(
            request_id.as_str(),
            role,
            request_value,
            &observation,
            true,
            Some("provider.protocol.malformed"),
        );
        return Err(transport(
            AdapterError::InvocationMismatch,
            error_fact(
                config,
                request_id,
                role,
                digest,
                provider_version,
                Some(u64::from(protocol_major)),
            ),
            finish_failure(trace_finish),
        ));
    }
    if topology_forbidden && let Err(source) = reject_topology_fields(&raw["result"]) {
        let trace_finish = boundary.finish(
            request_id.as_str(),
            role,
            request_value,
            &observation,
            true,
            Some("provider.protocol.authority_violation"),
        );
        return Err(transport(
            AdapterError::Protocol(source),
            error_fact(
                config,
                request_id,
                role,
                digest,
                provider_version,
                Some(u64::from(protocol_major)),
            ),
            finish_failure(trace_finish),
        ));
    }
    Ok(WireInvocation {
        result: envelope.result,
        provider_version,
        protocol_major,
        digest,
        result_value: raw["result"].clone(),
        graph_declaration_error,
        boundary: boundary.clone(),
        observation,
        request_value,
        invocation_id: request_id.as_str().to_owned(),
        role,
    })
}

pub fn mapping_failure<R>(
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    wire: &WireInvocation<R>,
    message: impl Into<String>,
) -> InvocationError<AdapterError> {
    mapping_failure_with_code(
        config,
        request_id,
        role,
        wire,
        "provider.protocol.mapping",
        message,
    )
}

pub fn mapping_failure_with_code<R>(
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    wire: &WireInvocation<R>,
    failure_code: &str,
    message: impl Into<String>,
) -> InvocationError<AdapterError> {
    let trace_failure = wire.fail_trace(failure_code);
    let message = message.into();
    let reason_code = if failure_code == "provider.evidence.malformed" {
        ReasonCode::ProviderEvidenceMalformed
    } else {
        ReasonCode::ProviderProtocolMalformed
    };
    let (reason, diagnostics) = invocation_failure(reason_code, failure_code, &message);
    InvocationError::Transport {
        source: AdapterError::Mapping(message),
        fact: Box::new(error_fact(
            config,
            request_id,
            role,
            wire.digest.clone(),
            wire.provider_version.clone(),
            Some(u64::from(wire.protocol_major)),
        )),
        failure: Box::new(InvocationFailure {
            reason,
            diagnostics,
        }),
        trace_failure,
    }
}

pub fn result_fact(
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    digest: DigestObservation,
    provider_version: Option<String>,
    protocol_major: u32,
    outcome: OutcomeClass,
) -> ProviderFact {
    make_fact(
        config,
        request_id,
        role,
        digest,
        provider_version,
        Some(u64::from(protocol_major)),
        outcome,
    )
}

fn finish_failure<E: std::fmt::Display>(result: Result<(), E>) -> Option<String> {
    result.err().map(|error| error.to_string())
}

fn transport(
    source: AdapterError,
    fact: ProviderFact,
    trace_failure: Option<String>,
) -> InvocationError<AdapterError> {
    let (reason_code, failure_code) = adapter_failure_class(&source);
    let detail = source.to_string();
    let (reason, diagnostics) = invocation_failure(reason_code, failure_code, &detail);
    InvocationError::Transport {
        source,
        fact: Box::new(fact),
        failure: Box::new(InvocationFailure {
            reason,
            diagnostics,
        }),
        trace_failure,
    }
}

fn adapter_failure_class(source: &AdapterError) -> (ReasonCode, &'static str) {
    match source {
        AdapterError::RequestEncoding(_) => (ReasonCode::ResourceExhausted, "resource.exhausted"),
        AdapterError::Process(error) => {
            let code = process_failure_code(error);
            (reason_code_for_failure_code(code), code)
        }
        AdapterError::UnsupportedMajor { .. } => (
            ReasonCode::ProviderProtocolUnsupportedMajor,
            "provider.protocol.unsupported_major",
        ),
        AdapterError::Protocol(ProtocolValidationError::Oversized { .. }) => (
            ReasonCode::ProviderProtocolOversized,
            "provider.protocol.oversized",
        ),
        AdapterError::Protocol(ProtocolValidationError::InvalidUtf8) => (
            ReasonCode::ProviderProtocolInvalidUtf8,
            "provider.protocol.invalid_utf8",
        ),
        AdapterError::Protocol(ProtocolValidationError::ForbiddenTopology(_)) => (
            ReasonCode::ProviderProtocolMalformed,
            "provider.protocol.authority_violation",
        ),
        AdapterError::Protocol(_)
        | AdapterError::RoleMismatch { .. }
        | AdapterError::InvocationMismatch
        | AdapterError::Mapping(_) => (
            ReasonCode::ProviderProtocolMalformed,
            "provider.protocol.malformed",
        ),
    }
}

fn reason_code_for_failure_code(code: &str) -> ReasonCode {
    match code {
        "resource.exhausted" => ReasonCode::ResourceExhausted,
        "provider.executable.not_found" => ReasonCode::ProviderExecutableNotFound,
        "provider.spawn.failed" => ReasonCode::ProviderSpawnFailed,
        "provider.timeout" => ReasonCode::ProviderTimeout,
        "provider.crash" => ReasonCode::ProviderCrash,
        "provider.signal" => ReasonCode::ProviderSignal,
        "provider.nonzero_exit" => ReasonCode::ProviderNonzeroExit,
        "provider.protocol.oversized" => ReasonCode::ProviderProtocolOversized,
        "provider.protocol.invalid_utf8" => ReasonCode::ProviderProtocolInvalidUtf8,
        _ => ReasonCode::ProviderProtocolMalformed,
    }
}

fn invocation_failure(
    reason_code: ReasonCode,
    diagnostic_code: &str,
    detail: &str,
) -> (Reason, Vec<Diagnostic>) {
    let reason = Reason::new(reason_code, detail).unwrap_or_else(|_| {
        Reason::new(reason_code, "provider failure detail exceeded bound")
            .expect("fixed provider failure reason is bounded")
    });
    let diagnostic = Diagnostic::new(diagnostic_code, detail, None).unwrap_or_else(|_| {
        Diagnostic::new(
            diagnostic_code,
            "provider failure detail exceeded diagnostic bound",
            None,
        )
        .expect("fixed provider failure diagnostic is bounded")
    });
    (reason, vec![diagnostic])
}

fn error_fact(
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    digest: DigestObservation,
    provider_version: Option<String>,
    protocol_major: Option<u64>,
) -> ProviderFact {
    make_fact(
        config,
        request_id,
        role,
        digest,
        provider_version,
        protocol_major,
        OutcomeClass::Error,
    )
}

fn make_fact(
    config: &ResolvedProviderConfig,
    request_id: &RequestId,
    role: ProviderRole,
    digest: DigestObservation,
    provider_version: Option<String>,
    protocol_major: Option<u64>,
    outcome: OutcomeClass,
) -> ProviderFact {
    ProviderFact::new(
        config.registration_id().clone(),
        config.config_revision(),
        core_role(role),
        request_id.clone(),
        config.config().executable(),
        outcome,
        digest,
        provider_version,
        protocol_major,
    )
    .expect("resolved provider facts satisfy core bounds")
}

fn valid_provider_version(
    value: Option<&Value>,
) -> Result<Option<String>, ProtocolValidationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(ProtocolValidationError::Malformed(
            "provider_version must be a string".into(),
        ));
    };
    BoundedText::<256>::opaque_non_empty("provider_version", value.to_owned())
        .map_err(|error| ProtocolValidationError::Malformed(error.to_string()))?;
    Ok(Some(value.to_owned()))
}

fn registration(config: &ResolvedProviderConfig) -> RegistrationDto {
    RegistrationDto {
        registration_id: config.registration_id().as_str().to_owned(),
        config_revision: config.config_revision(),
        executable: config.config().executable().to_owned(),
        argv: config
            .config()
            .argv()
            .iter()
            .map(|argument| argument.as_str().to_owned())
            .collect(),
        working_directory: config.config().working_directory().to_owned(),
        timeout_seconds: config.config().timeout_seconds(),
    }
}

fn core_role(role: ProviderRole) -> CoreProviderRole {
    match role {
        ProviderRole::Describe => CoreProviderRole::Describe,
        ProviderRole::ValidateInputs => CoreProviderRole::ValidateInputs,
        ProviderRole::EvaluateGates => CoreProviderRole::EvaluateGates,
        ProviderRole::LiveGuidance => CoreProviderRole::LiveGuidance,
        ProviderRole::CheckCompatibility => CoreProviderRole::CheckCompatibility,
    }
}
