//! Driver-declared correction provenance. No prose/diff scope inference and no
//! owner approval. Exact previous bytes remain immutable in the context record.
use crate::{commission, config, finding_ledger};
use loop_core::ContextRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingReference {
    pub ledger_id: String,
    pub finding_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Driver {
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Correction {
    pub subject: String,
    pub subject_revision: String,
    pub prior_intent: commission::IntentBaseline,
    pub findings: Vec<FindingReference>,
    pub author: Driver,
    pub reason: String,
}

/// Latest actual earlier reviewed intent revision, not a numeric/wording guess.
pub fn prior_reviewed_revision(records: &[ContextRecord], current: &str) -> Option<String> {
    records
        .iter()
        .rev()
        .filter(|record| {
            record.kind == "review-evidence"
                && record.data["subject"] == "intent.json"
                && matches!(
                    record.data["gate"].as_str(),
                    Some("intent-review" | "intent-adversarial-review")
                )
        })
        .filter_map(|record| record.data["subject_revision"].as_str())
        .find(|revision| !revision.trim().is_empty() && *revision != current)
        .map(str::to_owned)
}

pub fn validate(
    data: &Value,
    initial_input: &Value,
    earlier: &[ContextRecord],
) -> Result<Correction, String> {
    let correction: Correction = serde_json::from_value(data.clone())
        .map_err(|error| format!("invalid intent-correction: {error}"))?;
    if correction.subject != "intent.json"
        || correction.subject_revision.trim().is_empty()
        || correction.author.name.trim().is_empty()
        || !["human", "agent", "script"].contains(&correction.author.kind.as_str())
        || correction.reason.trim().is_empty()
    {
        return Err(
            "intent-correction requires intent target, driver attribution and reason".into(),
        );
    }
    commission::validate_intent_baseline(&correction.prior_intent)?;
    if correction.prior_intent.revision == correction.subject_revision {
        return Err("intent-correction must bind a distinct prior revision".into());
    }
    let prior = prior_reviewed_revision(earlier, &correction.subject_revision)
        .ok_or("intent-correction has no earlier reviewed intent identity")?;
    if correction.prior_intent.revision != prior {
        return Err(format!(
            "intent-correction prior revision differs from actual latest reviewed `{prior}`"
        ));
    }
    if correction.findings.is_empty() {
        return Err("intent-correction requires accepted intent-owned findings".into());
    }
    let effective = crate::admission::effective_policy(initial_input, earlier).effective;
    let config = config::parse_initial_input(&effective).map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    for reference in &correction.findings {
        if reference.ledger_id.trim().is_empty()
            || reference.finding_id.trim().is_empty()
            || !seen.insert((&reference.ledger_id, &reference.finding_id))
        {
            return Err("intent-correction finding references must be nonempty and unique".into());
        }
        let found = earlier
            .iter()
            .filter(|record| record.id.as_str() == reference.ledger_id)
            .collect::<Vec<_>>();
        if found.len() != 1 {
            return Err("intent-correction ledger reference is missing/ambiguous".into());
        }
        let ledger = found[0];
        let gate = ledger.data["gate"]
            .as_str()
            .ok_or("correction ledger has no gate")?;
        if ledger.kind != "finding-ledger"
            || !["intent-review", "intent-adversarial-review"].contains(&gate)
            || ledger.data["subject"] != "intent.json"
        {
            return Err("correction must cite an intent-gate ledger".into());
        }
        let latest = earlier
            .iter()
            .rev()
            .find(|record| {
                record.kind == "finding-ledger"
                    && record.data["gate"] == gate
                    && record.data["subject"] == "intent.json"
            })
            .ok_or("missing latest intent ledger")?;
        if latest.id != ledger.id {
            return Err("correction must cite the latest matching intent-gate ledger".into());
        }
        let snapshot = finding_ledger::parse_snapshot(
            &ledger.data,
            gate,
            "intent.json",
            config.artifact_root(),
            Some(&config),
            earlier,
            ledger.sequence.as_u64(),
        )
        .map_err(|reasons| reasons.join("; "))?;
        let finding = snapshot
            .findings
            .iter()
            .find(|finding| finding.id == reference.finding_id)
            .ok_or("correction finding ID is not in its ledger")?;
        if finding.disposition != finding_ledger::FindingDisposition::Accepted
            || finding.owner_phase.as_deref() != Some("intent")
        {
            return Err("correction finding must be accepted and intent-owned".into());
        }
        let source = earlier
            .iter()
            .find(|record| record.id.as_str() == finding.source.record_id())
            .ok_or("correction finding source missing")?;
        if source.kind != "review-evidence"
            || source.data["gate"] != gate
            || source.data["subject"] != "intent.json"
            || source.data["subject_revision"] != correction.prior_intent.revision
        {
            return Err(
                "correction source is not a judgment of the exact prior intent revision".into(),
            );
        }
    }
    Ok(correction)
}
