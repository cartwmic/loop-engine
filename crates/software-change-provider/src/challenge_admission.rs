//! Write-only rule for C's shared admission table. Never run on stored history.
use loop_core::ContextRecord;
use serde_json::Value;

pub const RULE_ID: &str = "b-ordinary-during-challenge";

fn ordinary_gate(state: &str) -> Option<&'static str> {
    match state {
        "intent-adversarial-review" => Some("intent-review"),
        "design-adversarial-review" => Some("design-review"),
        "plan-adversarial-review" => Some("plan-review"),
        "implementation-adversarial-review" => Some("implementation-review"),
        "validation-adversarial-review" => Some("validation-review"),
        "design-narrative-adversarial-review" => Some("design-narrative-review"),
        "plan-narrative-adversarial-review" => Some("plan-narrative-review"),
        "validation-narrative-adversarial-review" => Some("validation-narrative-review"),
        _ => None,
    }
}

/// Returns `(rule_id, message)` for a new candidate only. C's adapter maps this
/// into its Refusal and registers it as WriteOnly for all five kinds. Intrinsic
/// shape/origin validation and fail-closed transport belong to C, not this rule.
/// Frozen workflow/policy is unnecessary: the eight supported state IDs fully
/// determine the ordinary gate. Context supplies immutable origin lookup.
pub fn check_new_write(
    current_state: &str,
    record_id: &str,
    kind: &str,
    data: &Value,
    context: &[ContextRecord],
) -> Option<(&'static str, String)> {
    let parent = ordinary_gate(current_state)?;
    let mut kind = kind;
    let mut data = data;
    let mut visited = std::collections::BTreeSet::new();
    while kind == "evidence-applicability" {
        let origin = data["origin"]["id"].as_str()?;
        if !visited.insert(origin) {
            return None; // Invalid origins are refused by the shared shape rule.
        }
        let record = context.iter().find(|r| r.id.as_str() == origin)?;
        kind = &record.kind;
        data = &record.data;
    }
    let belongs = match kind {
        "review-evidence" | "finding-ledger" => data["gate"] == parent,
        "author-count-amendment" => data["targets"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["gate"] == parent)),
        "intent-comparison" => parent == "intent-review",
        _ => false,
    };
    belongs.then(|| (RULE_ID, format!(
        "Record `{record_id}` belongs to ordinary gate `{parent}` and cannot be written during `{current_state}`; use the existing check-free `revise` route for correction and renewed ordinary approval."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;

    fn origin(gate: &str) -> ContextRecord {
        ContextRecord::new(
            "origin",
            "review-evidence",
            json!({"gate":gate}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        )
    }

    #[test]
    fn all_challenges_refuse_ordinary_kinds_and_accept_other_writes() {
        for state in [
            "intent-adversarial-review",
            "design-adversarial-review",
            "plan-adversarial-review",
            "implementation-adversarial-review",
            "validation-adversarial-review",
            "design-narrative-adversarial-review",
            "plan-narrative-adversarial-review",
            "validation-narrative-adversarial-review",
        ] {
            let parent = ordinary_gate(state).unwrap();
            for (kind, data) in [
                ("review-evidence", json!({"gate":parent})),
                ("finding-ledger", json!({"gate":parent})),
                (
                    "author-count-amendment",
                    json!({"targets":[{"gate":"unrelated"},{"gate":parent}]}),
                ),
                ("evidence-applicability", json!({"origin":{"id":"origin"}})),
            ] {
                let context = vec![origin(parent)];
                let (rule, message) =
                    check_new_write(state, "candidate", kind, &data, &context).unwrap();
                assert_eq!(rule, RULE_ID);
                for part in ["candidate", parent, "revise", "check-free"] {
                    assert!(message.contains(part));
                }
                assert!(check_new_write(parent, "candidate", kind, &data, &context).is_none());
            }
            for kind in ["review-evidence", "finding-ledger"] {
                assert!(
                    check_new_write(state, "challenge", kind, &json!({"gate":state}), &[])
                        .is_none()
                );
                assert!(check_new_write(
                    state,
                    "other",
                    kind,
                    &json!({"gate":"other-review"}),
                    &[]
                )
                .is_none());
            }
            assert!(check_new_write(
                state,
                "carry",
                "evidence-applicability",
                &json!({"origin":{"id":"origin"}}),
                &[origin(state)]
            )
            .is_none());
            assert!(check_new_write(
                state,
                "amend",
                "author-count-amendment",
                &json!({"targets":[{"gate":state}]}),
                &[]
            )
            .is_none());
            assert_eq!(
                check_new_write(state, "comparison", "intent-comparison", &json!({}), &[])
                    .is_some(),
                parent == "intent-review"
            );
        }
    }

    #[test]
    fn early_preparation_and_history_are_not_reclassified() {
        let context = vec![origin("intent-review")];
        let original = context.clone();
        for state in ["explore", "design", "implement", "validation", "end"] {
            assert!(check_new_write(
                state,
                "new",
                "review-evidence",
                &json!({"gate":"intent-review"}),
                &context
            )
            .is_none());
        }
        assert!(check_new_write(
            "intent-adversarial-review",
            "opaque",
            "note",
            &json!({"gate":"intent-review"}),
            &context
        )
        .is_none());
        assert_eq!(context, original);
    }
}
