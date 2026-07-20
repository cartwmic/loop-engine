use std::fs;
use std::path::{Path, PathBuf};

use loop_engine_core::model::bounded::{
    DIAGNOSTIC_ENCODED_BYTES, DIAGNOSTICS_PER_RESULT_COUNT, EVIDENCE_LOCATOR_UTF8_BYTES,
    EVIDENCE_RECORD_ENCODED_BYTES, GUIDANCE_TEXT_BYTES, IDENTIFIER_UTF8_BYTES,
    METADATA_NESTING_DEPTH, PROVIDER_SNAPSHOT_ENVELOPE_BYTES, RUN_INPUTS_ENCODED_TOTAL_BYTES,
};
use loop_engine_integrations::provider_protocol::dto::*;
use loop_engine_integrations::provider_protocol::validation::{
    GRAPH_PROJECTION_CANONICAL_BYTES, PROVIDER_REQUEST_JSON_BYTES, PROVIDER_RESULT_STDOUT_BYTES,
};
use schemars::{JsonSchema, schema_for};
use serde_json::json;

fn write<T: JsonSchema>(directory: &Path, name: &str, role: Option<ProviderRole>) {
    let mut schema = serde_json::to_value(schema_for!(T)).expect("schema serializes");
    if let Some(role) = role {
        schema["properties"]["protocol_major"] = json!({"type": "integer", "const": 1});
        schema["properties"]["role"] = json!({"type": "string", "const": role});
    }
    schema["x-loop-engine-bounds"] = json!({
        "provider_request_json_bytes": PROVIDER_REQUEST_JSON_BYTES,
        "provider_result_stdout_bytes": PROVIDER_RESULT_STDOUT_BYTES,
        "graph_projection_canonical_bytes": GRAPH_PROJECTION_CANONICAL_BYTES,
        "identifier_utf8_bytes": IDENTIFIER_UTF8_BYTES,
        "guidance_text_bytes": GUIDANCE_TEXT_BYTES,
        "diagnostic_encoded_bytes": DIAGNOSTIC_ENCODED_BYTES,
        "diagnostics_per_result_count": DIAGNOSTICS_PER_RESULT_COUNT,
        "metadata_nesting_depth": METADATA_NESTING_DEPTH,
        "run_inputs_encoded_total_bytes": RUN_INPUTS_ENCODED_TOTAL_BYTES,
        "provider_snapshot_envelope_bytes": PROVIDER_SNAPSHOT_ENVELOPE_BYTES,
        "evidence_record_encoded_bytes": EVIDENCE_RECORD_ENCODED_BYTES,
        "evidence_locator_utf8_bytes": EVIDENCE_LOCATOR_UTF8_BYTES
    });
    let bytes = serde_json::to_vec_pretty(&schema).expect("schema serializes");
    fs::write(directory.join(name), bytes).expect("schema writes");
}

fn main() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/provider/v1");
    fs::create_dir_all(&directory).expect("schema directory creates");
    write::<GraphDto>(&directory, "graph.json", None);
    write::<RequestEnvelope<EmptyPayload>>(
        &directory,
        "describe-request.json",
        Some(ProviderRole::Describe),
    );
    write::<ResultEnvelope<DescribeResultDto>>(
        &directory,
        "describe-result.json",
        Some(ProviderRole::Describe),
    );
    write::<RequestEnvelope<ValidateInputsPayloadDto>>(
        &directory,
        "validate-inputs-request.json",
        Some(ProviderRole::ValidateInputs),
    );
    write::<ResultEnvelope<ValidateInputsResultDto>>(
        &directory,
        "validate-inputs-result.json",
        Some(ProviderRole::ValidateInputs),
    );
    write::<RequestEnvelope<GatePayloadDto>>(
        &directory,
        "evaluate-gates-request.json",
        Some(ProviderRole::EvaluateGates),
    );
    write::<ResultEnvelope<GateResultDto>>(
        &directory,
        "evaluate-gates-result.json",
        Some(ProviderRole::EvaluateGates),
    );
    write::<RequestEnvelope<GuidancePayloadDto>>(
        &directory,
        "live-guidance-request.json",
        Some(ProviderRole::LiveGuidance),
    );
    write::<ResultEnvelope<GuidanceResultDto>>(
        &directory,
        "live-guidance-result.json",
        Some(ProviderRole::LiveGuidance),
    );
    write::<RequestEnvelope<CompatibilityPayloadDto>>(
        &directory,
        "check-compatibility-request.json",
        Some(ProviderRole::CheckCompatibility),
    );
    write::<ResultEnvelope<CompatibilityResultDto>>(
        &directory,
        "check-compatibility-result.json",
        Some(ProviderRole::CheckCompatibility),
    );
}
