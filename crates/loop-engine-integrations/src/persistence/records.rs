//! Integration-owned persistence row and JSON wire DTOs (T106).
//!
//! Core domain types remain free of Serde and Rusqlite annotations.

use serde::{Deserialize, Serialize};

pub const PROVIDER_REGISTRATION_ROW_VERSION: u32 = 1;
pub const RUN_ROW_VERSION: u32 = 1;
pub const EVIDENCE_ROW_VERSION: u32 = 1;
pub const JOURNAL_ROW_VERSION: u32 = 1;
pub const JOURNAL_PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// Frozen GV-01 canonical graph snapshot bytes ([graph-projection.md](graph-projection.md)).
pub const GV01_CANONICAL_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}}],"transitions":[]}"#;

/// Frozen `graph_revision` for [`GV01_CANONICAL_GRAPH_JSON`].
pub const GV01_GRAPH_REVISION: &str =
    "sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77";

/// SQLite `provider_registrations` row projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRegistrationRecord {
    pub registration_id: String,
    pub handle: Option<String>,
    pub enabled: bool,
    pub config_revision: u64,
    pub executable: String,
    pub argv_json: String,
    pub working_directory: String,
    pub timeout_seconds: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// SQLite `runs` row projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub registration_id: String,
    pub config_revision_at_create: u64,
    pub current_state: String,
    pub lifecycle: String,
    pub workflow_state_version: u64,
    pub lifecycle_version: u64,
    pub label_version: u64,
    pub label: Option<String>,
    pub graph_revision: String,
    pub canonical_graph_version: u64,
    pub graph_canonical_projection_json: String,
    pub inputs_json: String,
    pub created_at: String,
}

/// SQLite `evidence` row projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecordRow {
    pub run_id: String,
    pub evidence_id: String,
    pub kind: String,
    pub locator: String,
    pub digest: Option<String>,
    pub media_type: Option<String>,
    pub metadata_json: Option<String>,
    pub source: String,
    pub created_at: String,
}

/// SQLite `journal_entries` row projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub run_id: String,
    pub sequence: u64,
    pub outcome: String,
    pub encoded_payload_json: String,
}

/// Validated journal wire payload root (journal-contract v1).
///
/// Full `JournalEntry` reconstruction is not attempted here; adapters treat the
/// payload as opaque wire authority beyond the denormalized row columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalPayloadV1 {
    pub journal_schema_version: u32,
    pub sequence: u64,
    pub run_id: String,
    pub outcome: String,
    #[serde(default)]
    pub ts: Option<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub entry_kind: Option<String>,
}
