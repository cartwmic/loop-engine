//! Existing approval/admission fact shapes, shared by options and admission.
use crate::{narratives, workflow};
use loop_core::ContextRecord;
use serde_json::Value;

pub(crate) fn valid_narrative_fact(fact: &Value, phase: &str) -> bool {
    fact.as_object().is_some_and(|object| {
        object.len() == 3
            && object
                .keys()
                .all(|k| ["phase", "revision", "sha256"].contains(&k.as_str()))
    }) && fact["phase"] == phase
        && fact["revision"]
            .as_str()
            .is_some_and(|r| !r.trim().is_empty())
        && fact["sha256"].as_str().is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
pub(crate) fn valid_artifact_fact(
    fact: &Value,
    phase: &str,
    subject: &str,
    admitted: bool,
) -> bool {
    let Some(object) = fact.as_object() else {
        return false;
    };
    let allowed = if admitted {
        &["phase", "subject", "revision", "sha256"][..]
    } else {
        &["subject", "revision", "sha256"][..]
    };
    object
        .keys()
        .all(|k| allowed.contains(&k.as_str()) || (admitted && k == "approved_artifact"))
        && (!admitted || fact["phase"] == phase)
        && fact["subject"] == subject
        && fact["revision"]
            .as_str()
            .is_some_and(|r| !r.trim().is_empty())
        && fact["sha256"].as_str().is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
pub(crate) fn valid_binding_source(
    binding: &narratives::NarrativeBinding<'_>,
    record: &ContextRecord,
    approved: &Value,
    subject: &str,
    markdown: &str,
) -> bool {
    binding.artifact.subject == subject
        && binding.markdown.file == markdown
        && binding.approved_artifact_record_id == record.id.as_str()
        && binding.artifact.revision == approved["revision"].as_str().unwrap_or("")
        && binding.artifact.sha256 == approved["sha256"].as_str().unwrap_or("")
}

pub(crate) fn validate_artifact_record(
    record: &ContextRecord,
    fact: &Value,
    kind: &str,
    phase: &str,
    subject: &str,
) -> bool {
    if (record.kind != workflow::ARTIFACT_ADMITTED_KIND && fact.get("approved_artifact").is_some())
        || !valid_artifact_fact(
            fact,
            phase,
            subject,
            kind == workflow::ARTIFACT_ADMITTED_KIND,
        )
    {
        return false;
    }
    if let Some(admission) = narratives::phase_record_fact(record, workflow::ARTIFACT_ADMITTED_KIND)
    {
        if (record.kind != workflow::ARTIFACT_ADMITTED_KIND
            && admission.get("approved_artifact").is_some())
            || !valid_artifact_fact(admission, phase, subject, true)
        {
            return false;
        }
        if let Some(approval) =
            narratives::phase_record_fact(record, workflow::APPROVED_ARTIFACT_KIND)
        {
            if !valid_artifact_fact(approval, phase, subject, false)
                || admission["subject"] != approval["subject"]
                || admission["revision"] != approval["revision"]
                || admission["sha256"] != approval["sha256"]
            {
                return false;
            }
        }
    }
    true
}
