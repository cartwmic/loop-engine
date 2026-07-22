//! Provider-owned JSON Schema generation from local protocol DTOs (schemars only).

use schemars::{JsonSchema, schema_for};
use serde_json::{Value, json};

use crate::protocol::{
    CompatibilityPayloadDto, CompatibilityResultDto, DescribeResultDto, EmptyPayload,
    GatePayloadDto, GateResultDto, GraphDto, GuidancePayloadDto, GuidanceResultDto, ProviderRole,
    RequestEnvelope, ResultEnvelope, ValidateInputsPayloadDto, ValidateInputsResultDto,
};

pub struct SchemaEntry {
    pub name: &'static str,
    pub schema: Value,
}

pub fn role_snake(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::Describe => "describe",
        ProviderRole::ValidateInputs => "validate_inputs",
        ProviderRole::EvaluateGates => "evaluate_gates",
        ProviderRole::LiveGuidance => "live_guidance",
        ProviderRole::CheckCompatibility => "check_compatibility",
    }
}

pub fn envelope_schema<T: JsonSchema>(role: Option<ProviderRole>) -> Value {
    let mut schema = serde_json::to_value(schema_for!(T)).expect("schema serializes");
    if let Some(role) = role {
        schema["properties"]["protocol_major"] = json!({"type": "integer", "const": 1});
        schema["properties"]["role"] = json!({"type": "string", "const": role_snake(role)});
    }
    schema
}

pub fn graph_schema() -> Value {
    serde_json::to_value(schema_for!(GraphDto)).expect("schema serializes")
}

pub fn all_local_schemas() -> Vec<SchemaEntry> {
    vec![
        SchemaEntry {
            name: "graph.json",
            schema: graph_schema(),
        },
        SchemaEntry {
            name: "describe-request.json",
            schema: envelope_schema::<RequestEnvelope<EmptyPayload>>(Some(ProviderRole::Describe)),
        },
        SchemaEntry {
            name: "describe-result.json",
            schema: envelope_schema::<ResultEnvelope<DescribeResultDto>>(Some(
                ProviderRole::Describe,
            )),
        },
        SchemaEntry {
            name: "validate-inputs-request.json",
            schema: envelope_schema::<RequestEnvelope<ValidateInputsPayloadDto>>(Some(
                ProviderRole::ValidateInputs,
            )),
        },
        SchemaEntry {
            name: "validate-inputs-result.json",
            schema: envelope_schema::<ResultEnvelope<ValidateInputsResultDto>>(Some(
                ProviderRole::ValidateInputs,
            )),
        },
        SchemaEntry {
            name: "evaluate-gates-request.json",
            schema: envelope_schema::<RequestEnvelope<GatePayloadDto>>(Some(
                ProviderRole::EvaluateGates,
            )),
        },
        SchemaEntry {
            name: "evaluate-gates-result.json",
            schema: envelope_schema::<ResultEnvelope<GateResultDto>>(Some(
                ProviderRole::EvaluateGates,
            )),
        },
        SchemaEntry {
            name: "live-guidance-request.json",
            schema: envelope_schema::<RequestEnvelope<GuidancePayloadDto>>(Some(
                ProviderRole::LiveGuidance,
            )),
        },
        SchemaEntry {
            name: "live-guidance-result.json",
            schema: envelope_schema::<ResultEnvelope<GuidanceResultDto>>(Some(
                ProviderRole::LiveGuidance,
            )),
        },
        SchemaEntry {
            name: "check-compatibility-request.json",
            schema: envelope_schema::<RequestEnvelope<CompatibilityPayloadDto>>(Some(
                ProviderRole::CheckCompatibility,
            )),
        },
        SchemaEntry {
            name: "check-compatibility-result.json",
            schema: envelope_schema::<ResultEnvelope<CompatibilityResultDto>>(Some(
                ProviderRole::CheckCompatibility,
            )),
        },
    ]
}
