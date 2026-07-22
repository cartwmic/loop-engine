mod compatibility;
mod config;
mod evidence;
mod gates;
mod graph;
mod guidance;
mod inputs;
mod protocol;

use std::io::{self, Read, Write};

use config::ProviderConfig;
use protocol::{
    CompatibilityPayloadDto, DescribeResultDto, EmptyPayload, GatePayloadDto, GuidancePayloadDto,
    PROTOCOL_MAJOR_V1, ProviderError, ProviderRole, RequestEnvelope, ResultEnvelope,
    ValidateInputsPayloadDto,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn main() {
    if let Err(err) = run() {
        eprintln!("reference-provider error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ProviderError> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;

    let raw: Value = serde_json::from_str(&stdin)
        .map_err(|err| ProviderError::MalformedRequest(format!("invalid request json: {err}")))?;

    let protocol_major = raw
        .get("protocol_major")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    if protocol_major != PROTOCOL_MAJOR_V1 {
        return Err(ProviderError::UnsupportedMajor(protocol_major));
    }

    let role = raw
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::MalformedRequest("missing role".to_string()))?
        .to_string();
    let invocation_id = raw
        .get("invocation_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::MalformedRequest("missing invocation_id".to_string()))?
        .to_string();

    let mut config = ProviderConfig::from_process_argv();
    merge_registration_argv(&mut config, &raw);

    let result_value = match role.as_str() {
        "describe" => {
            let request: RequestEnvelope<EmptyPayload> = parse_request(raw)?;
            ensure_role(&request, ProviderRole::Describe)?;
            serde_json::to_value(describe(&config))?
        }
        "validate_inputs" => {
            let request: RequestEnvelope<ValidateInputsPayloadDto> = parse_request(raw)?;
            ensure_role(&request, ProviderRole::ValidateInputs)?;
            serde_json::to_value(inputs::validate_inputs(
                &request.payload.declarations,
                &request.payload.candidate_values,
            ))?
        }
        "evaluate_gates" => {
            let request: RequestEnvelope<GatePayloadDto> = parse_request(raw)?;
            ensure_role(&request, ProviderRole::EvaluateGates)?;
            serde_json::to_value(gates::evaluate_gates(&request.payload, &config))?
        }
        "live_guidance" => {
            let request: RequestEnvelope<GuidancePayloadDto> = parse_request(raw)?;
            ensure_role(&request, ProviderRole::LiveGuidance)?;
            serde_json::to_value(guidance::live_guidance(&request.payload, &config))?
        }
        "check_compatibility" => {
            let request: RequestEnvelope<CompatibilityPayloadDto> = parse_request(raw)?;
            ensure_role(&request, ProviderRole::CheckCompatibility)?;
            serde_json::to_value(compatibility::check_compatibility(
                &request.payload,
                &config,
            ))?
        }
        other => {
            return Err(ProviderError::MalformedRequest(format!(
                "unsupported role {other}"
            )));
        }
    };

    let envelope = ResultEnvelope {
        protocol_major: PROTOCOL_MAJOR_V1,
        role: serde_json::from_value(json_role(&role)?)?,
        invocation_id,
        provider_version: config.provider_version.clone(),
        result: result_value,
    };

    let stdout = serde_json::to_string(&envelope)?;
    let mut out = io::stdout().lock();
    out.write_all(stdout.as_bytes())?;
    out.flush()?;
    Ok(())
}

fn describe(config: &ProviderConfig) -> DescribeResultDto {
    DescribeResultDto::Description {
        graph: graph::build_graph(config.describe_graph),
    }
}

fn merge_registration_argv(config: &mut ProviderConfig, raw: &Value) {
    let Some(argv) = raw
        .get("registration")
        .and_then(|registration| registration.get("argv"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let args = argv
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    config.merge_registration_argv(&args);
}

fn parse_request<T: DeserializeOwned>(raw: Value) -> Result<RequestEnvelope<T>, ProviderError> {
    serde_json::from_value(raw)
        .map_err(|err| ProviderError::MalformedRequest(format!("invalid request envelope: {err}")))
}

fn ensure_role<T>(
    request: &RequestEnvelope<T>,
    expected: ProviderRole,
) -> Result<(), ProviderError> {
    if request.role != expected {
        return Err(ProviderError::MalformedRequest(format!(
            "role mismatch: expected {expected:?}, got {:?}",
            request.role
        )));
    }
    Ok(())
}

fn json_role(role: &str) -> Result<Value, ProviderError> {
    Ok(match role {
        "describe" => Value::String("describe".to_string()),
        "validate_inputs" => Value::String("validate_inputs".to_string()),
        "evaluate_gates" => Value::String("evaluate_gates".to_string()),
        "live_guidance" => Value::String("live_guidance".to_string()),
        "check_compatibility" => Value::String("check_compatibility".to_string()),
        other => {
            return Err(ProviderError::MalformedRequest(format!(
                "unsupported role {other}"
            )));
        }
    })
}
