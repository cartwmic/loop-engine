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

/// Prepared future binding; the stored legacy binding remains unchanged until
/// amendment/launch operations explicitly adopt this value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveBinding {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_filter: Option<ContextFilter>,
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

/// Resolve the creation snapshot plus ordered, engine-authored history.
/// The amendment projection on Run is loaded from history, never a policy registry.
pub fn effective_binding(
    run: &crate::Run,
    slot: &crate::WorkSlotId,
) -> Option<Result<crate::WorkSlotBinding, String>> {
    if let Some(amendment) = run
        .binding_amendments
        .iter()
        .rev()
        .find(|a| a.slot_id == *slot)
    {
        return Some(Ok(amendment.effective.clone()));
    }
    let value = run
        .initial_input
        .get("work_slot_bindings")?
        .get(slot.as_str())?;
    Some(serde_json::from_value(value.clone()).map_err(|error| format!(
        "work_slot_bindings[{slot}] must be an object with {{command, args, context_filter?}}: {error}"
    )))
}

/// The kernel identity of one process incarnation.
///
/// A PID is only a recyclable number. The engine records the boot identity
/// and kernel start identity alongside it so readers can distinguish the
/// recorded process from a later occupant of the same PID. The values are
/// native observations supplied by the integration layer; core does not
/// interpret their platform-specific clocks.
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

/// Elapsed allowance and waiter loss are never permission to overlap owned work.
/// A live waiter covers the short pre-publication startup window. Historical
/// numeric-only rows retain their legacy read behavior; they are not
/// cancellation authority.
pub fn invocation_owns_work(row: &crate::WorkSlotInvocation, waiter_alive: bool) -> bool {
    row.ownership
        .as_ref()
        .is_some_and(|o| o.live_owned_work || o.cleanup_pending)
        || (row.status.is_none() && waiter_alive)
}

/// Only verified cleanup may acknowledge a cancellation as terminal failure.
/// An interrupted or timed-out attempt has no acknowledgment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationAcknowledgment {
    pub invocation_id: crate::InvocationId,
    pub attempt: u64,
    pub elapsed_ms: u64,
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

/// Only references may cross the selector boundary; records cannot be rewritten.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFilterSelection {
    pub record_ids: Vec<String>,
}

/// Resolve references without permitting record mutation, duplication or reordering.
pub fn resolve_context_filter(
    eligible: &[crate::ContextRecord],
    selection: &ContextFilterSelection,
) -> Result<Vec<crate::ContextRecord>, String> {
    let mut cursor = 0;
    let mut selected = Vec::new();
    for id in &selection.record_ids {
        let Some(position) = eligible[cursor..]
            .iter()
            .position(|record| record.id.as_str() == id)
        else {
            return Err(format!(
                "context filter returned unknown, duplicate or reordered record `{id}`"
            ));
        };
        cursor += position;
        selected.push(eligible[cursor].clone());
        cursor += 1;
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recovery_steering_ordered_subset_is_reference_only() {
        let records = ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(n, id)| {
                crate::ContextRecord::new(
                    id,
                    "opaque",
                    json!({"kept":id}),
                    crate::SemanticSequence::new(n as u64 + 1),
                    crate::Timestamp::from_unix_millis(0),
                )
            })
            .collect::<Vec<_>>();
        let selection = |ids: &[&str]| ContextFilterSelection {
            record_ids: ids.iter().map(|s| (*s).into()).collect(),
        };
        assert_eq!(
            resolve_context_filter(&records, &selection(&["two"])).unwrap(),
            vec![records[1].clone()]
        );
        for ids in [vec!["missing"], vec!["one", "one"], vec!["two", "one"]] {
            assert!(resolve_context_filter(&records, &selection(&ids)).is_err());
        }
        assert!(serde_json::from_value::<ContextFilterSelection>(
            json!({"record_ids":[{"id":"one","data":{}}]})
        )
        .is_err());
    }

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
        let parsed: EffectiveBinding = serde_json::from_value(value.clone()).unwrap();
        assert!(parsed.context_filter.is_none());
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        let filtered = json!({"command":"worker","args":[],"context_filter":{
            "command":"software-change","args":["commission"]}});
        let parsed: EffectiveBinding = serde_json::from_value(filtered.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), filtered);
    }

    #[test]
    fn execution_contract_attestation_and_selector_are_closed() {
        assert!(serde_json::from_value::<StateVisitAttestation>(json!({
            "state_visit":-1,"owner":"owner","reason":"correction"
        }))
        .is_err());
        assert!(serde_json::from_value::<ContextFilterSelection>(json!({
            "record_ids":[],"command":"replacement"
        }))
        .is_err());
    }
}
