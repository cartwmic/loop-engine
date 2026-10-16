//! Read-only resolution of externally authorized author-count amendments.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct Resolution {
    pub base: Value,
    pub effective: Value,
    pub amendments: Vec<Value>,
    pub blocked: BTreeSet<String>,
    pending_ids: BTreeSet<String>,
}

fn targets(input: &Value) -> BTreeMap<String, u64> {
    let mut result = BTreeMap::new();
    if let Some(gates) = input["review_policies"].as_object() {
        for (gate, entries) in gates {
            for entry in entries.as_array().into_iter().flatten() {
                if let (Some(axis), Some(stage)) = (
                    entry["id"].as_str(),
                    entry
                        .get("review_stage")
                        .or_else(|| entry.get("stage"))
                        .and_then(Value::as_str),
                ) {
                    result.insert(
                        format!("review.{gate}.{stage}.{axis}"),
                        entry["required_authors"].as_u64().unwrap_or(1),
                    );
                }
            }
        }
    }
    for (key, field) in [
        ("criterion", "required_authors"),
        ("goal", "goal_required_authors"),
    ] {
        if let Some(count) = input["criterion_policy"][field].as_u64() {
            result.insert(key.into(), count);
        }
    }
    result
}

fn target_key(target: &Value) -> Result<String, String> {
    let object = target.as_object().ok_or("target must be an object")?;
    if object.len() == 1 && target["criterion"] == "all" {
        return Ok("criterion".into());
    }
    if object.len() == 1 && target["goal"] == true {
        return Ok("goal".into());
    }
    if object.len() == 3 {
        if let (Some(gate), Some(stage), Some(axis)) = (
            target["gate"].as_str(),
            target["stage"].as_str(),
            target["axis"].as_str(),
        ) {
            return Ok(format!("review.{gate}.{stage}.{axis}"));
        }
    }
    Err(format!("unknown target {target}"))
}

fn selected(data: &Value, counts: &BTreeMap<String, u64>) -> Result<Vec<String>, String> {
    let entries = data["targets"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("targets must be a nonempty array")?;
    let mut keys = BTreeSet::new();
    for entry in entries {
        let key = target_key(entry)?;
        if !counts.contains_key(&key) {
            return Err(format!("unknown target {entry}"));
        }
        if !keys.insert(key) {
            return Err("duplicate target".into());
        }
    }
    Ok(keys.into_iter().collect())
}

fn validate_data(data: &Value, proposal: bool) -> Result<(), String> {
    let object = data.as_object().ok_or("amendment must be an object")?;
    let allowed = [
        "targets",
        "before",
        "after",
        "actor",
        "authority",
        "reason",
        "corrects",
    ];
    if object
        .keys()
        .any(|key| !allowed.contains(&key.as_str()) || (proposal && key == "before"))
    {
        return Err("unknown or non-count field in amendment".into());
    }
    if data["after"].as_u64().is_none_or(|v| v < 1) {
        return Err("after must be an integer >= 1".into());
    }
    if !proposal && data["before"].as_u64().is_none_or(|v| v < 1) {
        return Err("before must be an integer >= 1".into());
    }
    for (object, field) in [("actor", "name"), ("authority", "reference")] {
        if data[object][field]
            .as_str()
            .is_none_or(|v| v.trim().is_empty())
        {
            return Err(format!("{object}.{field} must be nonempty"));
        }
    }
    if !matches!(
        data["actor"]["kind"].as_str(),
        Some("human" | "agent" | "script")
    ) {
        return Err("invalid actor kind".into());
    }
    if !matches!(
        data["authority"]["kind"].as_str(),
        Some("owner" | "standing-delegation")
    ) {
        return Err("invalid authority kind".into());
    }
    if data["reason"].as_str().is_none_or(|v| v.trim().is_empty()) {
        return Err("reason must be nonempty".into());
    }
    if data
        .get("corrects")
        .is_some_and(|v| v.as_str().is_none_or(|s| s.is_empty()))
    {
        return Err("corrects must be a record ID".into());
    }
    Ok(())
}

pub fn resolve(input: &Value, context: &Value) -> Resolution {
    let source = if crate::overlay::enabled(input) {
        crate::overlay::apply(input)
    } else {
        input.clone()
    };
    let input = &source;
    let base_counts = targets(input);
    let mut counts = base_counts.clone();
    let mut pending: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut amendments = Vec::new();
    for record in context
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["kind"] == "author-count-amendment")
    {
        let id = record["id"].as_str().unwrap_or("<missing-id>").to_owned();
        let data = &record["data"];
        let selection = selected(data, &counts);
        let affected: BTreeSet<String> = match &selection {
            Ok(keys) => keys.iter().cloned().collect(),
            Err(_) => {
                let named: Result<BTreeSet<_>, _> = data["targets"]
                    .as_array()
                    .ok_or("missing targets")
                    .and_then(|entries| {
                        entries
                            .iter()
                            .map(|entry| {
                                target_key(entry).and_then(|key| {
                                    if counts.contains_key(&key) {
                                        Ok(key)
                                    } else {
                                        Err("unknown target".into())
                                    }
                                })
                            })
                            .collect::<Result<BTreeSet<_>, String>>()
                            .map_err(|_| "unknown target")
                    });
                match named {
                    Ok(keys) if !keys.is_empty() => keys,
                    _ => counts.keys().cloned().collect(),
                }
            }
        };
        let validation = selection.and_then(|keys| {
            validate_data(data, false)?;
            if record["id"].as_str().is_none_or(|v| v.is_empty()) {
                return Err("missing record ID".into());
            }
            if let Some(corrects) = data["corrects"].as_str() {
                if !pending.contains_key(corrects) {
                    return Err(format!(
                        "corrects does not name an outstanding invalid amendment: {corrects}"
                    ));
                }
            }
            if keys
                .iter()
                .any(|key| counts[key] != data["before"].as_u64().unwrap())
            {
                return Err("stale before: does not match current effective count".into());
            }
            Ok(keys)
        });
        match validation {
            Ok(keys) => {
                for key in keys {
                    counts.insert(key, data["after"].as_u64().unwrap());
                }
                if let Some(corrects) = data["corrects"].as_str() {
                    pending.remove(corrects);
                }
                amendments.push(json!({"id":id,"valid":true}));
            }
            Err(diagnostic) => {
                pending.insert(id.clone(), affected);
                amendments.push(
                    json!({"id":id,"valid":false,"diagnostic":format!("{id}: {diagnostic}")}),
                );
            }
        }
    }
    let mut effective = input.clone();
    if let Some(gates) = effective
        .get_mut("review_policies")
        .and_then(Value::as_object_mut)
    {
        for (gate, entries) in gates {
            for entry in entries.as_array_mut().into_iter().flatten() {
                let key = format!(
                    "review.{gate}.{}.{}",
                    entry
                        .get("review_stage")
                        .or_else(|| entry.get("stage"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    entry["id"].as_str().unwrap_or("")
                );
                if let Some(count) = counts.get(&key) {
                    entry["required_authors"] = json!(count);
                }
            }
        }
    }
    for (key, field) in [
        ("criterion", "required_authors"),
        ("goal", "goal_required_authors"),
    ] {
        if let Some(count) = counts.get(key) {
            effective["criterion_policy"][field] = json!(count);
        }
    }
    Resolution {
        base: json!(base_counts),
        effective,
        amendments,
        pending_ids: pending.keys().cloned().collect(),
        blocked: pending.into_values().flatten().collect(),
    }
}

pub fn propose(resolution: &Resolution, mut data: Value) -> Result<Value, String> {
    validate_data(&data, true)?;
    let counts = targets(&resolution.effective);
    let keys = selected(&data, &counts)?;
    let before = counts[&keys[0]];
    if keys.iter().any(|key| counts[key] != before) {
        return Err("targets have different current counts; propose separate amendments".into());
    }
    if let Some(corrects) = data["corrects"].as_str() {
        if !resolution.pending_ids.contains(corrects) {
            return Err("corrects must name an invalid amendment".into());
        }
    }
    data["before"] = json!(before);
    Ok(json!({"kind":"author-count-amendment","data":data}))
}

pub fn run(args: &[String], show: &Value) -> Result<Value, String> {
    let packet = if show["operation"] == "show" && show["status"] == "completed" {
        &show["result"]
    } else {
        return Err("author-counts requires completed full show JSON".into());
    };
    let input = packet
        .get("initial_input")
        .ok_or("full show JSON must contain initial_input")?;
    let resolution = resolve(input, &packet["context"]);
    if args.is_empty() {
        return Ok(
            json!({"base":resolution.base,"effective":targets(&resolution.effective),"amendments":resolution.amendments}),
        );
    }
    if args.len() == 2 && args[0] == "--propose" {
        return propose(
            &resolution,
            serde_json::from_str(&args[1]).map_err(|e| e.to_string())?,
        );
    }
    Err("usage: software-change author-counts [--propose JSON] < full-show.json".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn amendment(id: &str, targets: Value, before: u64, after: u64) -> Value {
        json!({"id":id,"kind":"author-count-amendment","data":{"targets":targets,"before":before,"after":after,"actor":{"name":"owner","kind":"human"},"authority":{"kind":"owner","reference":"approval"},"reason":"adjust"}})
    }
    #[test]
    fn criterion_and_goal_counts_resolve_without_mutating_initial_input() {
        let input = json!({"criterion_policy":{"required_authors":2,"goal_required_authors":2},"review_policies":{}});
        let original = input.clone();
        let context = json!([
            amendment("lower", json!([{"criterion":"all"},{"goal":true}]), 2, 1),
            amendment("raise-goal", json!([{"goal":true}]), 1, 3)
        ]);
        let resolved = resolve(&input, &context);
        assert_eq!(
            resolved.effective["criterion_policy"]["required_authors"],
            1
        );
        assert_eq!(
            resolved.effective["criterion_policy"]["goal_required_authors"],
            3
        );
        assert!(resolved.blocked.is_empty());
        assert_eq!(input, original);
    }
    #[test]
    fn stale_before_blocks_only_named_target_and_requires_explicit_correction() {
        let input = json!({"criterion_policy":{"required_authors":2,"goal_required_authors":2}});
        let invalid = amendment("bad", json!([{"criterion":"all"}]), 1, 1);
        let valid = amendment("valid", json!([{"criterion":"all"}]), 2, 1);
        let resolved = resolve(&input, &json!([invalid.clone(), valid]));
        assert_eq!(resolved.blocked, BTreeSet::from(["criterion".into()]));
        let mut correction = amendment("correct", json!([{"criterion":"all"}]), 2, 1);
        correction["data"]["corrects"] = json!("bad");
        assert!(resolve(&input, &json!([invalid, correction]))
            .blocked
            .is_empty());
        assert!(input.get("review_policies").is_none());
        assert!(resolve(&input, &json!([]))
            .effective
            .get("review_policies")
            .is_none());
    }
    #[test]
    fn unknown_target_blocks_every_count_target_and_proposals_reject_non_count_changes() {
        let input = json!({"criterion_policy":{"required_authors":2,"goal_required_authors":2}});
        let resolved = resolve(
            &input,
            &json!([amendment(
                "unknown",
                json!([{"gate":"unknown","stage":"aggregate","axis":"axis"}]),
                2,
                1
            )]),
        );
        assert_eq!(
            resolved.blocked,
            BTreeSet::from(["criterion".into(), "goal".into()])
        );
        let mut proposal = amendment("proposal", json!([{"goal":true}]), 2, 1)["data"].clone();
        proposal.as_object_mut().unwrap().remove("before");
        assert_eq!(
            propose(&resolved, proposal.clone()).unwrap()["data"]["before"],
            2
        );
        proposal["self_review"] = json!(true);
        assert!(propose(&resolved, proposal).is_err());
    }

    #[test]
    fn materialized_narrative_counts_preserve_shipped_floors_and_exact_amendment_targets() {
        for (source, floor, stages) in [
            (include_str!("../data/configs/minimal.json"), 1, 1),
            (include_str!("../data/configs/standard.json"), 2, 1),
            (include_str!("../data/configs/high-rigor.json"), 2, 2),
        ] {
            let mut profile: Value = serde_json::from_str(source).unwrap();
            profile["extra"]["bookends"] = json!(true);
            for phase in ["design", "plan", "validation"] {
                for (pass, suffix) in [("ordinary", "review"), ("challenge", "adversarial-review")]
                {
                    let gate = format!("{phase}-narrative-{suffix}");
                    let mut axes = profile["extra"]["narrative_review"][pass].clone();
                    for axis in axes.as_array_mut().unwrap() {
                        axis["group"] =
                            json!(axis["group"].as_str().unwrap().replace("{phase}", phase));
                    }
                    profile["review_policies"][gate] = axes;
                }
            }
            let resolved = resolve(&profile, &json!([]));
            let counts = targets(&resolved.effective);
            let narrative_counts: Vec<_> = counts
                .iter()
                .filter(|(k, _)| k.contains("-narrative-"))
                .collect();
            assert_eq!(narrative_counts.len(), 6 * 3 * stages);
            assert!(narrative_counts.iter().all(|(_, count)| **count == floor));
            assert!(!narrative_counts
                .iter()
                .any(|(key, _)| key.ends_with("ids-grounded")));
            let context = json!([amendment(
                "owner-count",
                json!([{
                    "gate":"design-narrative-review","stage":"aggregate","axis":"narrative-faithful"
                }]),
                floor,
                3
            )]);
            let changed = resolve(&profile, &context);
            assert!(changed.blocked.is_empty());
            let key = "review.design-narrative-review.aggregate.narrative-faithful";
            assert_eq!(changed.base[key], floor);
            assert_eq!(targets(&changed.effective)[key], 3);
            for (target, count) in counts {
                if target != key {
                    assert_eq!(targets(&changed.effective)[&target], count);
                }
            }
        }
    }
}
