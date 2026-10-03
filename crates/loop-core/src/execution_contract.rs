//! Provider-opaque execution values. Runtime admission and persistence are
//! implemented by the operation/process owners, not by these wire types.
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFilter {
    pub command: String,
    pub args: Vec<String>,
}

/// Immutable history payload for a future-only execution correction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BindingAmendment {
    pub slot_id: crate::WorkSlotId,
    pub state_visit: u64,
    pub owner: String,
    pub reason: String,
    pub original: Option<crate::WorkSlotBinding>,
    pub effective: crate::WorkSlotBinding,
}

/// The kernel identity of one process incarnation.
///
/// A PID is only a recyclable number. The engine records the boot-session
/// identity and native process-start identity alongside it so readers can
/// distinguish the recorded process from a later occupant of the same PID.
/// The values are supplied by the integration layer; core does not interpret
/// their platform-specific representations.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub boot_id: String,
    pub start_time: u64,
}

impl ProcessIdentity {
    pub fn new(pid: u32, boot_id: impl Into<String>, start_time: u64) -> Self {
        Self {
            pid,
            boot_id: boot_id.into(),
            start_time,
        }
    }
}

/// Engine-published local ownership, never caller-selected cancellation PIDs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedExecution {
    pub root_pid: u32,
    pub process_group_id: u32,
    /// Native incarnation identity for `root_pid`. It is optional only so
    /// pre-identity ownership files remain readable; new publication refuses
    /// to use a numeric-only root as control authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_identity: Option<ProcessIdentity>,
    pub admission_directory: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_locator: Option<std::path::PathBuf>,
}

/// Process facts are independent of the legacy waiter/time status overlay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionOwnershipState {
    pub execution: OwnedExecution,
    pub live_owned_work: bool,
    pub cleanup_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<serde_json::Value>,
}

/// Absent historical controls do not attest to execution of new capabilities.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationControls {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active: Option<NonZeroUsize>,
    #[serde(default)]
    pub force_fresh: bool,
}

/// Operation admission additionally checks nonempty owner/reason and the
/// observed current visit. Deserialization is not authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateVisitAttestation {
    pub state_visit: u64,
    pub owner: String,
    pub reason: String,
}

/// Small caller-authored facts for an explicitly permitted driver completion.
/// The engine adds the current slot, visit, binding, and instruction identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DriverActRequest {
    pub author: DriverActAuthor,
    pub reason: String,
    pub changed_artifacts: Vec<String>,
    pub unchanged_documents: DriverActDocuments,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DriverActAuthor {
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DriverActDocuments {
    pub intent_revision: String,
    pub design_revision: String,
    pub plan_revision: String,
}

/// Engine-attributed direct completion evidence carried into normal provider
/// evaluation and immutable transition history. It is not a worker result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DriverActEvidence {
    pub request: DriverActRequest,
    pub slot_id: crate::WorkSlotId,
    pub state_visit: u64,
    pub current_subject: String,
    pub instruction_digest: String,
    pub binding_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn execution_contract_defaults_and_closed_controls() {
        let controls: InvocationControls = serde_json::from_value(json!({})).unwrap();
        assert_eq!(controls, InvocationControls::default());
        for invalid in [
            json!({"max_active":0}),
            json!({"max_active":-1}),
            json!({"force_fresh":1}),
            json!({"policy":{}}),
        ] {
            assert!(serde_json::from_value::<InvocationControls>(invalid).is_err());
        }
        let value = json!({"max_active":2,"force_fresh":true});
        let parsed: InvocationControls = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }

    #[test]
    fn execution_contract_legacy_binding_has_no_filter_or_ownership_claim() {
        let value = json!({"command":"worker","args":[]});
        let parsed: crate::WorkSlotBinding = serde_json::from_value(value.clone()).unwrap();
        assert!(parsed.context_filter.is_none());
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        let filtered = json!({"command":"worker","args":[],"context_filter":{
            "command":"software-change","args":["commission"]}});
        let parsed: crate::WorkSlotBinding = serde_json::from_value(filtered.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), filtered);
    }

    #[test]
    fn execution_contract_attestation_and_selector_are_closed() {
        assert!(serde_json::from_value::<StateVisitAttestation>(json!({
            "state_visit":-1,"owner":"owner","reason":"correction"
        }))
        .is_err());
        assert!(serde_json::from_value::<StateVisitAttestation>(json!({
            "state_visit":0,"owner":"owner","reason":"correction","command":"replacement"
        }))
        .is_err());
    }
}
