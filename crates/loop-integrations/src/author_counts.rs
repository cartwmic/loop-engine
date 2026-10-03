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
    serde_json::from_value(input.clone()).unwrap_or_default()
}

fn target_key(target: &Value) -> Result<String, String> {
    let object = target.as_object().ok_or("target must be an object")?;
    if object.len() == 2 {
        if let (Some(gate), Some(axis)) = (target["gate"].as_str(), target["axis"].as_str()) {
            return Ok(format!("{gate}.{axis}"));
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
    let effective = json!(counts);
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

pub fn inspect(resolution: &Resolution) -> Value {
    json!({"base":resolution.base,"effective":resolution.effective,"amendments":resolution.amendments,"blocked":resolution.blocked})
}
