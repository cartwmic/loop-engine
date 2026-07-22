use crate::protocol::{
    AnyRequest, AnyResult, CompatibilityPayloadDto, CompatibilityResultDto, DiagnosticDto,
    GatePayloadDto, GateResultDto, GuidancePayloadDto, GuidanceResultDto, ProviderRole,
    ValidateInputsPayloadDto, ValidateInputsResultDto,
};
use crate::scenarios::{Scenario, graphs, inputs, roles};

const MALFORMED_PAYLOAD: &str = "provider.protocol.malformed";

fn malformed_payload(path: &str, message: &str) -> DiagnosticDto {
    DiagnosticDto {
        code: MALFORMED_PAYLOAD.into(),
        message: message.into(),
        path: Some(path.into()),
    }
}

fn parse_validate_inputs(
    payload: &serde_json::Value,
) -> Result<ValidateInputsPayloadDto, DiagnosticDto> {
    serde_json::from_value(payload.clone()).map_err(|error| {
        malformed_payload(
            "/payload",
            &format!("malformed validate_inputs payload: {error}"),
        )
    })
}

fn parse_gate_payload(payload: &serde_json::Value) -> Result<GatePayloadDto, DiagnosticDto> {
    serde_json::from_value(payload.clone()).map_err(|error| {
        malformed_payload(
            "/payload",
            &format!("malformed evaluate_gates payload: {error}"),
        )
    })
}

fn parse_guidance_payload(
    payload: &serde_json::Value,
) -> Result<GuidancePayloadDto, DiagnosticDto> {
    serde_json::from_value(payload.clone()).map_err(|error| {
        malformed_payload(
            "/payload",
            &format!("malformed live_guidance payload: {error}"),
        )
    })
}

fn parse_compatibility_payload(
    payload: &serde_json::Value,
) -> Result<CompatibilityPayloadDto, DiagnosticDto> {
    serde_json::from_value(payload.clone()).map_err(|error| {
        malformed_payload(
            "/payload",
            &format!("malformed check_compatibility payload: {error}"),
        )
    })
}

pub fn handle_request(
    scenario: Scenario,
    request: &AnyRequest,
    invocation_ordinal: Option<u64>,
) -> AnyResult {
    match request.role {
        ProviderRole::Describe => {
            AnyResult::Describe(graphs::describe(scenario, invocation_ordinal))
        }
        ProviderRole::ValidateInputs => match parse_validate_inputs(&request.payload) {
            Ok(payload) => AnyResult::ValidateInputs(inputs::validate(scenario, payload)),
            Err(diagnostic) => {
                AnyResult::ValidateInputs(ValidateInputsResultDto::EvaluationError {
                    diagnostics: vec![diagnostic],
                })
            }
        },
        ProviderRole::EvaluateGates => match parse_gate_payload(&request.payload) {
            Ok(payload) => AnyResult::EvaluateGates(roles::evaluate_gates(scenario, payload)),
            Err(diagnostic) => AnyResult::EvaluateGates(GateResultDto::EvaluationError {
                diagnostics: vec![diagnostic],
            }),
        },
        ProviderRole::LiveGuidance => match parse_guidance_payload(&request.payload) {
            Ok(payload) => AnyResult::LiveGuidance(roles::live_guidance(scenario, payload)),
            Err(diagnostic) => AnyResult::LiveGuidance(GuidanceResultDto::EvaluationError {
                diagnostics: vec![diagnostic],
            }),
        },
        ProviderRole::CheckCompatibility => match parse_compatibility_payload(&request.payload) {
            Ok(payload) => {
                AnyResult::CheckCompatibility(roles::check_compatibility(scenario, payload))
            }
            Err(diagnostic) => {
                AnyResult::CheckCompatibility(CompatibilityResultDto::EvaluationError {
                    diagnostics: vec![diagnostic],
                })
            }
        },
    }
}
