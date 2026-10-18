//! Role of a stored record for model-binding checks.

use loop_core::ModelRole;
use serde_json::Value;

/// The model-binding role that produced a stored record, or `None` for a kind
/// that carries no role.
///
/// - `review-evidence` at an ordinary `*-review` gate is the ordinary
///   reviewer, except `policy_id` `intent-sufficient` at `intent-review`,
///   which is the cold reader.
/// - `review-evidence` at a `*-adversarial-review` gate (including
///   `intent-sufficient` at `intent-adversarial-review`, the informed reader)
///   is the challenge reviewer.
/// - `criterion-verdict` and `goal-verdict` are the criterion author.
///
/// A `review-evidence` record whose `gate` is missing or is not a review gate
/// has no role. `session-declaration` carries its own `role` and is not
/// inferred here.
pub fn role_of_record(kind: &str, data: &Value) -> Option<ModelRole> {
    match kind {
        "criterion-verdict" | "goal-verdict" => Some(ModelRole::CriterionAuthor),
        "review-evidence" => {
            let gate = data["gate"].as_str()?;
            if gate.ends_with("-adversarial-review") {
                Some(ModelRole::ChallengeReviewer)
            } else if gate == "intent-review" && data["policy_id"] == "intent-sufficient" {
                Some(ModelRole::ColdReader)
            } else if gate.ends_with("-review") {
                Some(ModelRole::OrdinaryReviewer)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn role_of_record_covers_every_return_kind() {
        let table = [
            (
                "review-evidence",
                json!({"gate": "intent-review", "policy_id": "ids-grounded"}),
                Some(ModelRole::OrdinaryReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "design-review"}),
                Some(ModelRole::OrdinaryReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "plan-narrative-review", "policy_id": "intent-sufficient"}),
                Some(ModelRole::OrdinaryReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "implementation-review", "policy_id": "x"}),
                Some(ModelRole::OrdinaryReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "validation-review"}),
                Some(ModelRole::OrdinaryReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "intent-review", "policy_id": "intent-sufficient"}),
                Some(ModelRole::ColdReader),
            ),
            (
                "review-evidence",
                json!({"gate": "intent-adversarial-review", "policy_id": "intent-sufficient"}),
                Some(ModelRole::ChallengeReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "intent-adversarial-review", "policy_id": "ids-grounded"}),
                Some(ModelRole::ChallengeReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "design-narrative-adversarial-review"}),
                Some(ModelRole::ChallengeReviewer),
            ),
            (
                "review-evidence",
                json!({"gate": "validation-adversarial-review"}),
                Some(ModelRole::ChallengeReviewer),
            ),
            (
                "criterion-verdict",
                json!({}),
                Some(ModelRole::CriterionAuthor),
            ),
            ("goal-verdict", json!({}), Some(ModelRole::CriterionAuthor)),
            ("review-evidence", json!({}), None),
            (
                "review-evidence",
                json!({"gate": "implementation-ready"}),
                None,
            ),
            ("session-declaration", json!({"role": "author"}), None),
            ("model-binding-amendment", json!({}), None),
            ("review-batch", json!({"gate": "intent-review"}), None),
        ];
        for (kind, data, expected) in table {
            assert_eq!(role_of_record(kind, &data), expected, "{kind} {data}");
        }
    }
}
