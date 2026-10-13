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

fn parse_candidate(data: &Value, earlier: &[ContextRecord]) -> Result<Comparison, String> {
    let comparison: Comparison = serde_json::from_value(data.clone())
        .map_err(|e| format!("invalid intent-comparison: {e}"))?;
    if comparison.subject_revision.trim().is_empty() || comparison.baseline.is_null() {
        return Err("intent-comparison requires revision and baseline".into());
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
    if covered.is_empty() || covered.len() != comparison.reconstruction_ids.len() {
        return Err("intent-comparison must address every counted reconstruction exactly; retained IDs must be unique and nonempty".into());
    }
    for id in &covered {
        let sources: Vec<_> = earlier.iter().filter(|r| r.id.as_str() == id).collect();
        if sources.len() != 1
            || sources[0].kind != "review-evidence"
            || sources[0].data["gate"] != "intent-review"
            || sources[0].data["policy_id"] != "intent-sufficient"
            || sources[0].data["review_stage"] != "aggregate"
            || sources[0].data["baseline"] != comparison.baseline
        {
            return Err(format!("intent-comparison reconstruction `{id}` is missing or has incompatible identity/baseline"));
        }
        crate::evidence::parse_historical_evidence_record(&sources[0].data, "intent.json", true)?;
    }
    let mut dispositions = BTreeSet::new();
    for d in &comparison.dispositions {
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
        if !dispositions.insert(d.reconstruction_id.clone()) {
            return Err("duplicate intent-comparison disposition".into());
        }
    }
    if dispositions != covered {
        return Err(
            "intent-comparison dispositions must address its retained reconstructions".into(),
        );
    }
    Ok(comparison)
}

pub(crate) fn validate_candidate(data: &Value, earlier: &[ContextRecord]) -> Result<(), String> {
    parse_candidate(data, earlier).map(|_| ())
}

pub fn matching_baseline(record: &ContextRecord, intent: &Value) -> bool {
    record.data["policy_id"] != "intent-sufficient"
        || (intent.get("baseline").is_some_and(|v| !v.is_null())
            && record.data.get("baseline") == intent.get("baseline"))
}

/// Original reconstruction IDs behind the same counted aggregate evidence
/// selected at the gate, including eligible carries to this current target.
pub fn counted_reconstruction_ids(
    records: &[ContextRecord],
    intent: &Value,
    initial_input: &Value,
) -> Result<BTreeSet<String>, String> {
    let config = crate::config::validate_config(initial_input).map_err(|e| e.to_string())?;
    let axes = config
        .staged_axes_for_gate("intent-review")
        .and_then(|stages| stages.get("aggregate"))
        .ok_or("intent-sufficient aggregate policy is not configured")?;
    let policy = axes
        .get("intent-sufficient")
        .ok_or("intent-sufficient aggregate policy is not configured")?;
    let author = crate::evidence::AuthorIdentity::new(
        intent["author"]["name"]
            .as_str()
            .ok_or("intent author.name is missing")?,
        intent["author"]["kind"]
            .as_str()
            .ok_or("intent author.kind is missing")?,
    );
    let ledger = crate::finding_ledger::latest_snapshot_for_recovery(
        records,
        "intent-review",
        "intent.json",
        config.artifact_root(),
    )?;
    let sources = crate::evidence::counted_aggregate_sources(
        records,
        "intent-review",
        "intent.json",
        intent["revision"]
            .as_str()
            .ok_or("intent revision is missing")?,
        &author,
        config.config_version(),
        policy,
        axes,
        config.artifact_root(),
        ledger.as_ref(),
        config.contract_version() >= 3,
    )?;
    if sources
        .iter()
        .any(|record| !matching_baseline(record, intent))
    {
        return Err(
            "intent-sufficient reconstruction baseline changed; obtain a fresh cold exercise"
                .into(),
        );
    }
    Ok(sources.iter().map(|record| record.id.to_string()).collect())
}

pub fn validate(
    records: &[ContextRecord],
    intent: &Value,
    initial_input: &Value,
) -> Result<(), String> {
    let baseline = intent
        .get("baseline")
        .filter(|v| !v.is_null())
        .ok_or("intent-sufficient requires a declared intent.baseline")?;
    let required = counted_reconstruction_ids(records, intent, initial_input)?;
    if required.is_empty() {
        return Err("intent-sufficient comparison missing retained cold reconstructions".into());
    }
    let (index, record) = records
        .iter()
        .enumerate()
        .rev()
        .find(|(_, r)| {
            r.kind == "intent-comparison" && r.data["subject_revision"] == intent["revision"]
        })
        .ok_or("missing current intent-comparison")?;
    let comparison = parse_candidate(&record.data, &records[..index])?;
    if comparison.subject_revision != intent["revision"].as_str().unwrap_or_default()
        || comparison.baseline != *baseline
    {
        return Err(
            "intent-comparison baseline changed; obtain a fresh cold exercise and comparison"
                .into(),
        );
    }
    let covered: BTreeSet<_> = comparison.reconstruction_ids.iter().cloned().collect();
    if covered != required {
        return Err("intent-comparison must address every counted reconstruction exactly".into());
    }
    if comparison
        .dispositions
        .iter()
        .any(|d| d.disposition == "unwanted-permitted")
    {
        return Err("intent-comparison leaves an unwanted permitted alternative unresolved".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::test_support::*;
    use serde_json::json;

    fn comparison(intent: &Value, ids: &[&str], sequence: u64) -> ContextRecord {
        row(
            "comparison",
            "intent-comparison",
            sequence,
            json!({"subject_revision":intent["revision"],"baseline":intent["baseline"],
            "reconstruction_ids":ids,"actor":{"name":"owner","kind":"human"},"authority":"owner",
            "dispositions":ids.iter().map(|id|json!({"reconstruction_id":id,"alternative":"Encoding freedom",
                "disposition":"implementation-freedom","note":"Outcome unchanged"})).collect::<Vec<_>>()}),
        )
    }

    #[test]
    fn recovery_comparison_requires_exact_counted_fresh_and_carried_original_ids() {
        let root = Root::new();
        let mut intent = intent();
        intent["revision"] = json!("r2");
        std::fs::write(
            root.path.join("intent.json"),
            serde_json::to_vec(&intent).unwrap(),
        )
        .unwrap();
        let old = review("old", 1, "old-reader", "pass");
        let mut fresh = review("fresh", 3, "fresh-reader", "pass");
        fresh.data["subject_revision"] = json!("r2");
        let mut records = vec![old, carry("carry", "old", 2, "r2"), fresh];
        assert_eq!(
            counted_reconstruction_ids(&records, &intent, &root.input(2)).unwrap(),
            BTreeSet::from(["old".into(), "fresh".into()])
        );
        records.push(comparison(&intent, &["fresh"], 4));
        assert!(validate(&records, &intent, &root.input(2))
            .unwrap_err()
            .contains("every counted"));
        records.pop();
        records.push(comparison(&intent, &["old", "fresh"], 4));
        assert!(validate(&records, &intent, &root.input(2)).is_ok());
        let original = records[0].clone();
        assert_eq!(
            original.data["subject_revision"], "r1",
            "source revision is never rewritten"
        );
        let mut baseline = intent.clone();
        baseline["baseline"]["commit"] = json!("changed");
        assert!(validate(&records, &baseline, &root.input(2))
            .unwrap_err()
            .contains("baseline changed"));
    }

    #[test]
    fn recovery_comparison_excludes_retired_withdrawn_malformed_ineligible_and_superseded_returns()
    {
        let root = Root::new();
        let intent = intent();
        let retired = review("retired", 2, "old", "pass");
        let mut malformed = review("malformed", 5, "bad", "pass");
        malformed.data["result"] = json!(4);
        let self_review = review("self", 6, "drafter", "pass");
        let mut stale = review("stale", 7, "stale", "pass");
        stale.data["config_version"] = json!("other");
        let mut records = vec![
            roster("before", 1, &["old"]),
            retired.clone(),
            roster("after", 3, &["fresh"]),
            ledger(&retired, 4, "retired-author", "recorded"),
            malformed,
            self_review,
            stale,
            review("earlier", 8, "fresh", "pass"),
            review("latest", 9, "fresh", "pass"),
            review("withdrawn", 10, "gone", "pass"),
            withdrawal("withdraw", "withdrawn", 11),
            comparison(&intent, &["latest"], 12),
        ];
        assert_eq!(
            counted_reconstruction_ids(&records, &intent, &root.input(1)).unwrap(),
            BTreeSet::from(["latest".into()])
        );
        assert!(validate(&records, &intent, &root.input(1)).is_ok());
        let mut bad = records[8].clone();
        bad.id = "wrong-config-newer".into();
        bad.sequence = 13_u64.into();
        bad.data["config_version"] = json!("other");
        records.push(bad);
        assert!(
            counted_reconstruction_ids(&records, &intent, &root.input(1))
                .unwrap()
                .is_empty(),
            "latest stale-config verdict cannot revive an older return"
        );
    }

    #[test]
    fn recovery_comparison_checks_owner_shape_full_coverage_and_duplicate_dispositions() {
        let root = Root::new();
        let intent = intent();
        let cold = review("cold", 1, "reader", "pass");
        let valid = comparison(&intent, &["cold"], 2);
        assert!(
            validate(std::slice::from_ref(&cold), &intent, &root.input(1))
                .unwrap_err()
                .contains("missing current")
        );
        for (field, value) in [
            ("reconstruction_ids", json!([])),
            ("reconstruction_ids", json!(["cold", "cold"])),
            ("actor", json!({"name":"driver","kind":"agent"})),
            ("extra", json!(true)),
        ] {
            let mut bad = valid.clone();
            bad.data[field] = value;
            assert!(validate(&[cold.clone(), bad], &intent, &root.input(1)).is_err());
        }
        let mut duplicate = valid;
        let disposition = duplicate.data["dispositions"][0].clone();
        duplicate.data["dispositions"]
            .as_array_mut()
            .unwrap()
            .push(disposition);
        assert!(validate(&[cold, duplicate], &intent, &root.input(1))
            .unwrap_err()
            .contains("duplicate"));
    }
    #[test]
    fn recovery_counted_reconstructions_match_current_group_and_assessment_conformity() {
        let root = Root::new();
        let intent = intent();
        let mut input: Value =
            serde_json::from_str(include_str!("../data/configs/standard.json")).unwrap();
        input["artifact_root"] = root.value();
        // A two-axis aggregate group exercises the existing group completeness
        // rule, without weakening any judging/floor obligation.
        input["review_policies"]["intent-review"] = json!([
            {"id":"intent-sufficient","description":"Cold exercise","review_stage":"aggregate","required_authors":2,"group":"cold-group"},
            {"id":"peer","description":"Peer axis","review_stage":"aggregate","required_authors":2,"group":"cold-group"}
        ]);
        let mut cold = review("cold", 1, "reader", "pass");
        cold.data["config_version"] = input["config_version"].clone();
        cold.data["group"] = json!("wrong");
        cold.data["assessment_id"] = json!("batch");
        let mut peer = cold.clone();
        peer.id = "peer".into();
        peer.sequence = 2_u64.into();
        peer.data["policy_id"] = json!("peer");
        peer.data["group"] = json!("cold-group");
        assert!(
            counted_reconstruction_ids(&[cold.clone(), peer.clone()], &intent, &input)
                .unwrap()
                .is_empty()
        );
        cold.data["group"] = json!("cold-group");
        peer.data["assessment_id"] = json!("other-batch");
        assert!(
            counted_reconstruction_ids(&[cold.clone(), peer.clone()], &intent, &input)
                .unwrap()
                .is_empty()
        );
        peer.data["assessment_id"] = json!("batch");
        assert_eq!(
            counted_reconstruction_ids(&[cold, peer], &intent, &input).unwrap(),
            BTreeSet::from(["cold".into()])
        );
    }

    #[test]
    fn comparison_requires_full_coverage_and_exact_baseline() {
        let root = Root::new();
        let intent = intent();
        let cold = review("cold", 1, "cold-reader", "pass");
        assert!(
            validate(std::slice::from_ref(&cold), &intent, &root.input(1))
                .unwrap_err()
                .contains("missing current")
        );
        let comparison = comparison(&intent, &["cold"], 2);
        assert!(validate(&[cold.clone(), comparison.clone()], &intent, &root.input(1)).is_ok());
        let mut incomplete = comparison.clone();
        incomplete.data["reconstruction_ids"] = json!([]);
        assert!(
            validate(&[cold.clone(), incomplete], &intent, &root.input(1))
                .unwrap_err()
                .contains("every counted")
        );
        let mut changed = intent.clone();
        changed["baseline"]["commit"] = json!("after");
        assert!(validate(
            &[cold.clone(), comparison.clone()],
            &changed,
            &root.input(1)
        )
        .unwrap_err()
        .contains("baseline changed"));
        let mut invalid = comparison;
        invalid.data["driver_inference"] = json!(true);
        assert!(validate(&[cold, invalid], &intent, &root.input(1))
            .unwrap_err()
            .contains("invalid intent-comparison"));
    }
}
