//! Provider-neutral evaluation of trusted `owner-decision` context records.
//!
//! The closed JSON data shape is `{checkpoint, subject, subject_revision,
//! decision: "approve"|"reject", target, reason, text, actor: {name, kind}}`.
//! Actor kinds are `human`, `agent`, and `script`; they are claims, not authentication.
//! All identity strings and the verbatim decision `text` must be nonempty after
//! whitespace checking. Approval permits absent or null `target` and `reason`;
//! rejection requires nonempty strings for both and a caller-allowed target.
//! Unknown fields, including unknown actor fields, are refused. No words are trimmed.
//!
//! `evaluate(records: &[ContextRecord], checkpoint: &str, subject: &str,
//! subject_revision: &str, allowed_targets: &[&str]) -> Evaluation` selects the
//! latest well-formed decision for the checkpoint by semantic sequence, **before**
//! comparing subject and revision. A later decision about another subject or
//! revision therefore leaves the current subject pending, rather than reviving an
//! older approval. Malformed decisions are ignored and reported by record ID;
//! unrelated kinds and named checkpoints are ignored. Records without a string
//! checkpoint (or with a blank one) are reported because they cannot be assigned.
//! Providers own checkpoint/subject/target names and all routing and freshness checks.

use loop_core::ContextRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Claimed identity of the actor recording the owner's verbatim words.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub name: String,
    pub kind: ActorKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Agent,
    Script,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approve,
    Reject,
}

/// Closed `owner-decision` data. Use [`parse`] to enforce semantic constraints too.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerDecision {
    pub checkpoint: String,
    pub subject: String,
    pub subject_revision: String,
    pub decision: Decision,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub text: String,
    pub actor: Actor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Approved,
    Rejected { target: String, reason: String },
    Pending,
}

/// An ignored malformed record, identified without changing its retained data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub record_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub status: Status,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse one closed decision payload and validate against caller-owned target names.
/// The returned strings preserve the supplied bytes, including whitespace.
pub fn parse(data: &Value, allowed_targets: &[&str]) -> Result<OwnerDecision, String> {
    let decision: OwnerDecision =
        serde_json::from_value(data.clone()).map_err(|error| error.to_string())?;
    for (field, value) in [
        ("checkpoint", decision.checkpoint.as_str()),
        ("subject", decision.subject.as_str()),
        ("subject_revision", decision.subject_revision.as_str()),
        ("text", decision.text.as_str()),
        ("actor.name", decision.actor.name.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("{field} must be nonempty"));
        }
    }
    if decision.decision == Decision::Reject {
        let target = decision
            .target
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or("reject requires nonempty target")?;
        if !allowed_targets.contains(&target) {
            return Err(format!("unknown rejection target {target}"));
        }
        if decision
            .reason
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("reject requires nonempty reason".into());
        }
    }
    Ok(decision)
}

/// Evaluate the latest well-formed decision across all subjects and revisions.
/// Chronology comes from `ContextRecord.sequence`, not input slice order.
pub fn evaluate(
    records: &[ContextRecord],
    checkpoint: &str,
    subject: &str,
    subject_revision: &str,
    allowed_targets: &[&str],
) -> Evaluation {
    let mut diagnostics = Vec::new();
    let mut latest = None;
    for record in records.iter().filter(|record| {
        record.kind == "owner-decision"
            && record.data["checkpoint"]
                .as_str()
                .is_none_or(|name| name.trim().is_empty() || name == checkpoint)
    }) {
        match parse(&record.data, allowed_targets) {
            Ok(decision) => {
                if latest
                    .as_ref()
                    .is_none_or(|(sequence, _)| record.sequence >= *sequence)
                {
                    latest = Some((record.sequence, decision));
                }
            }
            Err(message) => diagnostics.push(Diagnostic {
                record_id: record.id.as_str().to_owned(),
                message,
            }),
        }
    }
    let status = match latest.map(|(_, decision)| decision) {
        Some(decision)
            if decision.subject == subject && decision.subject_revision == subject_revision =>
        {
            match decision.decision {
                Decision::Approve => Status::Approved,
                Decision::Reject => Status::Rejected {
                    target: decision.target.unwrap(),
                    reason: decision.reason.unwrap(),
                },
            }
        }
        _ => Status::Pending,
    };
    Evaluation {
        status,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;

    const TARGETS: &[&str] = &["document", "summary"];

    fn approve() -> Value {
        json!({
            "checkpoint": "handoff", "subject": "document.json", "subject_revision": "1",
            "decision": "approve", "text": "  Yes, ship this.\n",
            "actor": {"name": "owner", "kind": "human"}
        })
    }

    fn reject() -> Value {
        let mut data = approve();
        data["decision"] = json!("reject");
        data["target"] = json!("summary");
        data["reason"] = json!("  Missing the caveat.\n");
        data["text"] = json!("No — keep my wording.\n");
        data
    }

    fn row(id: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            "owner-decision",
            data,
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(0),
        )
    }

    fn current(records: &[ContextRecord]) -> Evaluation {
        evaluate(records, "handoff", "document.json", "1", TARGETS)
    }

    #[test]
    fn approve_current_revision_preserves_verbatim_words() {
        let data = approve();
        let parsed = parse(&data, TARGETS).unwrap();
        assert_eq!(parsed.text, data["text"].as_str().unwrap());
        assert_eq!(serde_json::to_value(parsed).unwrap()["text"], data["text"]);
        assert_eq!(
            current(&[row("approval", 1, data)]).status,
            Status::Approved
        );
        let mut null_fields = approve();
        null_fields["target"] = Value::Null;
        null_fields["reason"] = Value::Null;
        assert!(parse(&null_fields, TARGETS).is_ok());
    }

    #[test]
    fn reject_current_revision_returns_target_and_verbatim_reason() {
        let data = reject();
        assert_eq!(
            parse(&data, TARGETS).unwrap().text,
            "No — keep my wording.\n"
        );
        assert_eq!(
            current(&[row("rejection", 1, data)]).status,
            Status::Rejected {
                target: "summary".into(),
                reason: "  Missing the caveat.\n".into()
            }
        );
    }

    #[test]
    fn rejection_requires_reason_and_allowed_target() {
        for field in ["target", "reason"] {
            for value in [None, Some(Value::Null), Some(json!("")), Some(json!(" \n"))] {
                let mut data = reject();
                if let Some(value) = value {
                    data[field] = value;
                } else {
                    data.as_object_mut().unwrap().remove(field);
                }
                let result = current(&[row("invalid", 1, data)]);
                assert_eq!(result.status, Status::Pending);
                assert_eq!(result.diagnostics.len(), 1);
                assert!(result.diagnostics[0].message.contains(field));
            }
        }
        let mut data = reject();
        data["target"] = json!("not-allowed");
        assert!(parse(&data, TARGETS).unwrap_err().contains("target"));
        assert!(parse(&reject(), &[]).is_err());
    }

    #[test]
    fn empty_text_and_identity_strings_are_refused() {
        for field in ["checkpoint", "subject", "subject_revision", "text"] {
            for value in ["", " \n"] {
                let mut data = approve();
                data[field] = json!(value);
                assert!(parse(&data, TARGETS).unwrap_err().contains(field));
            }
        }
        let mut data = approve();
        data["actor"]["name"] = json!(" \n");
        assert!(parse(&data, TARGETS).unwrap_err().contains("actor.name"));
    }

    #[test]
    fn decision_for_another_revision_or_subject_leaves_pending() {
        assert_eq!(current(&[]).status, Status::Pending);
        for mut data in [approve(), reject()] {
            data["subject_revision"] = json!("2");
            assert_eq!(current(&[row("other", 1, data)]).status, Status::Pending);
        }
        let mut other = approve();
        other["subject"] = json!("other.json");
        assert_eq!(
            current(&[row("old", 1, approve()), row("other", 2, other)]).status,
            Status::Pending
        );
    }

    #[test]
    fn later_rejection_of_another_revision_outranks_earlier_approval() {
        let mut rejection = reject();
        rejection["subject_revision"] = json!("2");
        // Slice order is deliberately not chronological.
        let mut records = vec![row("newer", 2, rejection), row("older", 1, approve())];
        assert_eq!(current(&records).status, Status::Pending);
        records.push(row("fresh", 3, approve()));
        assert_eq!(current(&records).status, Status::Approved);
    }

    #[test]
    fn latest_matching_rejection_overrides_approval() {
        assert!(matches!(
            current(&[row("approval", 1, approve()), row("rejection", 2, reject())]).status,
            Status::Rejected { .. }
        ));
    }

    #[test]
    fn malformed_records_are_ignored_and_reported() {
        let mut records = vec![row("valid", 1, approve())];
        let mut unknown = approve();
        unknown["extra"] = json!(true);
        let mut unknown_actor = approve();
        unknown_actor["actor"]["extra"] = json!(true);
        let mut bad_actor = approve();
        bad_actor["actor"]["kind"] = json!("owner");
        let mut bad_decision = approve();
        bad_decision["decision"] = json!("abstain");
        let mut bad_text = approve();
        bad_text["text"] = json!(7);
        let mut blank_checkpoint = approve();
        blank_checkpoint["checkpoint"] = json!(" \n");
        let malformed = [
            unknown,
            unknown_actor,
            bad_actor,
            bad_decision,
            bad_text,
            blank_checkpoint,
            json!({}),
            json!([]),
        ];
        for (index, data) in malformed.into_iter().enumerate() {
            records.push(row(&format!("invalid-{index}"), index as u64 + 2, data));
        }
        let result = current(&records);
        assert_eq!(result.status, Status::Approved);
        assert_eq!(result.diagnostics.len(), 8);
        for (index, diagnostic) in result.diagnostics.iter().enumerate() {
            assert_eq!(diagnostic.record_id, format!("invalid-{index}"));
            assert!(!diagnostic.message.is_empty());
        }
    }

    #[test]
    fn unrelated_kinds_and_checkpoints_do_not_affect_evaluation() {
        let mut other = reject();
        other["checkpoint"] = json!("elsewhere");
        other["target"] = json!("another-checkpoints-target");
        let mut unrelated = row("comparison", 3, reject());
        unrelated.kind = "intent-comparison".into();
        let result = current(&[
            row("valid", 1, approve()),
            row("other", 2, other),
            unrelated,
        ]);
        assert_eq!(result.status, Status::Approved);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn all_conventional_actor_kinds_are_claims_not_authentication() {
        for kind in ["human", "agent", "script"] {
            let mut data = approve();
            data["actor"]["kind"] = json!(kind);
            assert_eq!(current(&[row("claim", 1, data)]).status, Status::Approved);
        }
    }
}
