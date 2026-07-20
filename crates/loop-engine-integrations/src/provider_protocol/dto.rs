use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_MAJOR_V1: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRole {
    Describe,
    ValidateInputs,
    EvaluateGates,
    LiveGuidance,
    CheckCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistrationDto {
    pub registration_id: String,
    #[schemars(range(min = 1))]
    pub config_revision: u64,
    pub executable: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    #[schemars(range(min = 1))]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestEnvelope<P> {
    pub protocol_major: u32,
    pub role: ProviderRole,
    pub invocation_id: String,
    pub registration: RegistrationDto,
    pub payload: P,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResultEnvelope<R> {
    pub protocol_major: u32,
    pub role: ProviderRole,
    pub invocation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    pub result: R,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmptyPayload {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StaticGuidanceDto {
    Text(String),
    Declaration(StaticGuidanceDeclarationDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StaticGuidanceDeclarationDto {
    Text { text: String },
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateDto {
    pub id: String,
    #[serde(rename = "final")]
    pub final_state: bool,
    pub static_guidance: StaticGuidanceDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TransitionDto {
    pub source_state: String,
    pub event: String,
    pub target_state: String,
    pub gate_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InputDeclarationDto {
    pub id: String,
    pub kind: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphDto {
    pub initial_state: String,
    pub states: Vec<StateDto>,
    pub transitions: Vec<TransitionDto>,
    pub input_declarations: Vec<InputDeclarationDto>,
    pub live_guidance_supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DescribeResultDto {
    Description { graph: GraphDto },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValidateInputsPayloadDto {
    pub declarations: Vec<InputDeclarationDto>,
    pub candidate_values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DiagnosticDto {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidateInputsResultDto {
    Accepted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        values: Option<BTreeMap<String, Value>>,
    },
    Rejected {
        diagnostics: Vec<DiagnosticDto>,
    },
    EvaluationError {
        diagnostics: Vec<DiagnosticDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceDto {
    pub id: String,
    pub kind: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunSnapshotDto {
    pub run_id: String,
    pub registration_id: String,
    pub graph_revision: String,
    pub lifecycle: String,
    pub current_state: String,
    pub workflow_state_version: u64,
    pub lifecycle_version: u64,
    pub inputs: BTreeMap<String, Value>,
    pub stored_graph: CanonicalGraphDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GatePayloadDto {
    pub snapshot: RunSnapshotDto,
    pub event: String,
    pub required_gate_ids: Vec<String>,
    pub selected_evidence: Vec<EvidenceDto>,
    pub inline_evidence: Vec<EvidenceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateVerdictDto {
    pub gate_id: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateResultDto {
    Verdicts {
        verdicts: Vec<GateVerdictDto>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<Vec<EvidenceDto>>,
    },
    Incompatible {
        diagnostics: Vec<DiagnosticDto>,
    },
    EvaluationError {
        diagnostics: Vec<DiagnosticDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GuidancePayloadDto {
    pub snapshot: RunSnapshotDto,
    pub selected_evidence: Vec<EvidenceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuidanceResultDto {
    Guidance { text: String },
    Incompatible { diagnostics: Vec<DiagnosticDto> },
    EvaluationError { diagnostics: Vec<DiagnosticDto> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompatibilityPayloadDto {
    pub stored_graph: CanonicalGraphDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatusDto {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompatibilityFindingDto {
    pub capability: String,
    pub status: CompatibilityStatusDto,
    pub diagnostics: Vec<DiagnosticDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompatibilityResultDto {
    Findings {
        capabilities: Vec<CompatibilityFindingDto>,
    },
    EvaluationError {
        diagnostics: Vec<DiagnosticDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalGraphDto {
    pub canonical_graph_version: u64,
    pub initial_state_id: String,
    pub input_declarations: Vec<CanonicalInputDto>,
    pub live_guidance_supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
    pub states: Vec<CanonicalStateDto>,
    pub transitions: Vec<CanonicalTransitionDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalStateDto {
    #[serde(rename = "final")]
    pub final_state: bool,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
    pub static_guidance: CanonicalGuidanceDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalGuidanceDto {
    Text { text: String },
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalTransitionDto {
    pub event_id: String,
    pub gate_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
    pub source_state_id: String,
    pub target_state_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalInputDto {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
    pub required: bool,
}
