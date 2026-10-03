//! Mechanical coverage for the externally judged cold reconstruction comparison.
use loop_core::ContextRecord;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Actor {
    name: String,
    kind: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Disposition {
    reconstruction_id: String,
    alternative: String,
    disposition: String,
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comparison {
    subject_revision: String,
    baseline: Value,
    reconstruction_ids: Vec<String>,
    actor: Actor,
    authority: String,
    dispositions: Vec<Disposition>,
}

pub fn matching_baseline(record: &ContextRecord, intent: &Value) -> bool {
    record.data["policy_id"] != "intent-sufficient"
        || (intent.get("baseline").is_some_and(|v| !v.is_null())
            && record.data.get("baseline") == intent.get("baseline"))
}

pub fn validate(records: &[ContextRecord], intent: &Value) -> Result<(), String> {
    let baseline = intent
        .get("baseline")
        .filter(|v| !v.is_null())
        .ok_or("intent-sufficient requires a declared intent.baseline")?;
    let mut latest = std::collections::BTreeMap::new();
    for record in records.iter().filter(|record| {
        record.kind == "review-evidence"
            && record.data["gate"] == "intent-review"
            && record.data["policy_id"] == "intent-sufficient"
            && record.data["review_stage"] == "aggregate"
            && record.data["subject_revision"] == intent["revision"]
    }) {
        latest.insert(record.data["author"].to_string(), record);
    }
    for carry in records.iter().filter(|r| {
        r.kind == "evidence-applicability"
            && r.data["target"]["revision"] == intent["revision"]
            && r.data["target"]["subject"] == "intent.json"
    }) {
        if let Some(source) = records
            .iter()
            .find(|r| Some(r.id.as_str()) == carry.data["origin"]["id"].as_str())
        {
            if source.kind == "review-evidence"
                && source.data["gate"] == "intent-review"
                && source.data["policy_id"] == "intent-sufficient"
            {
                latest
                    .entry(source.data["author"].to_string())
                    .or_insert(source);
            }
        }
    }
    if latest.is_empty() {
        return Err("intent-sufficient comparison missing retained cold reconstructions".into());
    }
    if latest
        .values()
        .any(|record| !matching_baseline(record, intent))
    {
        return Err(
            "intent-sufficient reconstruction baseline changed; obtain a fresh cold exercise"
                .into(),
        );
    }
    let required: BTreeSet<_> = latest.values().map(|r| r.id.as_str().to_owned()).collect();
    let record = records
        .iter()
        .rev()
        .find(|r| r.kind == "intent-comparison" && r.data["subject_revision"] == intent["revision"])
        .ok_or("missing current intent-comparison")?;
    let comparison: Comparison = serde_json::from_value(record.data.clone())
        .map_err(|e| format!("invalid intent-comparison: {e}"))?;
    if comparison.subject_revision != intent["revision"].as_str().unwrap_or_default()
        || comparison.baseline != *baseline
    {
        return Err(
            "intent-comparison baseline changed; obtain a fresh cold exercise and comparison"
                .into(),
        );
    }
    if comparison.actor.name.trim().is_empty()
        || !["human", "agent"].contains(&comparison.actor.kind.as_str())
        || comparison.authority.trim().is_empty()
        || (comparison.authority == "owner" && comparison.actor.kind != "human")
    {
        return Err(
            "intent-comparison requires owner or explicit owner-authorization reference".into(),
        );
    }
    let covered: BTreeSet<_> = comparison.reconstruction_ids.iter().cloned().collect();
    if covered != required {
        return Err("intent-comparison must address every counted reconstruction exactly".into());
    }
    let mut dispositions = BTreeSet::new();
    for d in comparison.dispositions {
        if !covered.contains(&d.reconstruction_id)
            || d.alternative.trim().is_empty()
            || d.note.trim().is_empty()
            || ![
                "implementation-freedom",
                "unwanted-permitted",
                "unsupported-assumption",
            ]
            .contains(&d.disposition.as_str())
        {
            return Err("invalid intent-comparison disposition".into());
        }
        if d.disposition == "unwanted-permitted" {
            return Err(
                "intent-comparison leaves an unwanted permitted alternative unresolved".into(),
            );
        }
        dispositions.insert(d.reconstruction_id);
    }
    if dispositions != required {
        return Err(
            "intent-comparison dispositions must address every counted reconstruction".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;
    fn row(id: &str, kind: &str, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        )
    }
    #[test]
    fn comparison_requires_full_coverage_and_exact_baseline() {
        let intent = json!({"revision":"r1","baseline":{"kind":"git","commit":"before"}});
        let cold = row(
            "cold",
            "review-evidence",
            json!({"gate":"intent-review","policy_id":"intent-sufficient",
            "review_stage":"aggregate","subject_revision":"r1","author":{"name":"cold-reader"},"baseline":intent["baseline"]}),
        );
        assert!(validate(std::slice::from_ref(&cold), &intent)
            .unwrap_err()
            .contains("missing current"));
        let comparison = row(
            "comparison",
            "intent-comparison",
            json!({"subject_revision":"r1","baseline":intent["baseline"],
            "reconstruction_ids":["cold"],"actor":{"name":"owner","kind":"human"},"authority":"owner",
            "dispositions":[{"reconstruction_id":"cold","alternative":"encoding freedom","disposition":"implementation-freedom","note":"outcome unchanged"}]}),
        );
        assert!(validate(&[cold.clone(), comparison.clone()], &intent).is_ok());
        let mut incomplete = comparison.clone();
        incomplete.data["reconstruction_ids"] = json!([]);
        assert!(validate(&[cold.clone(), incomplete], &intent)
            .unwrap_err()
            .contains("every counted"));
        let mut changed = intent.clone();
        changed["baseline"]["commit"] = json!("after");
        assert!(validate(&[cold.clone(), comparison.clone()], &changed)
            .unwrap_err()
            .contains("baseline changed"));
        let mut invalid = comparison;
        invalid.data["driver_inference"] = json!(true);
        assert!(validate(&[cold, invalid], &intent)
            .unwrap_err()
            .contains("invalid intent-comparison"));
    }
}
