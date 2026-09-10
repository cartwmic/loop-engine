//! Provider-owned historical context selection; core only transports originals.
use crate::{config::InitialInput, document::Snapshot};
use loop_core::{ContextRecord, WorkSlot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    target_id: String,
    slot_id: String,
    record_ids: Vec<String>,
    #[serde(default)]
    supersedes: Option<String>,
    #[serde(default)]
    receipt: Option<Value>,
}

pub fn prepare(input: &Value, profile: &Value) -> Result<Value, String> {
    let config = InitialInput::parse(profile)?;
    let snapshot = Snapshot::read(&config.target)?;
    let show = input.get("result");
    if show.is_some() && input["status"] != "completed" {
        return Err("commission requires completed show".into());
    }
    let packet = show.unwrap_or(input);
    let slot_id = packet["slot_id"].as_str().unwrap_or("semantic-review");
    if slot_id != "semantic-review" {
        return Err("commission supports semantic-review only".into());
    }
    if let Some(frozen) = packet.get("initial_input") {
        let actual = InitialInput::parse(frozen)?;
        if actual.target.id != config.target.id
            || actual.target.path != config.target.path
            || actual.profile_version != config.profile_version
        {
            return Err("commission profile does not match frozen target/profile".into());
        }
    }
    let slots: Vec<WorkSlot> = serde_json::from_value(packet["work_slots"].clone())
        .map_err(|e| format!("work_slots: {e}"))?;
    let slot = slots
        .iter()
        .find(|s| s.id.as_str() == slot_id)
        .ok_or("missing semantic-review slot")?;
    let mut records: Vec<ContextRecord> =
        serde_json::from_value(packet["context"].clone()).map_err(|e| format!("context: {e}"))?;
    records.retain(|r| slot.stdin_context_kinds.contains(&r.kind));
    let mut selections = Vec::new();
    for record in records
        .iter()
        .filter(|r| r.kind == "review-context-selection")
    {
        if record.data["target_id"] != config.target.id || record.data["slot_id"] != slot_id {
            continue;
        }
        let selection: Selection = serde_json::from_value(record.data.clone())
            .map_err(|e| format!("selection {}: {e}", record.id))?;
        selections.push((record, selection));
    }
    selections.sort_by_key(|(r, _)| r.sequence);
    let mut diagnostics = Vec::new();
    // Validate supersession identities even when an earlier selection is no longer active.
    for (record, selection) in &selections {
        if let Some(id) = &selection.supersedes {
            if !selections.iter().any(|(old, s)| {
                old.id.as_str() == id
                    && old.sequence < record.sequence
                    && s.target_id == selection.target_id
                    && s.slot_id == selection.slot_id
            }) {
                return Err(format!(
                    "selection {} has unknown/inapplicable supersedes {id}",
                    record.id
                ));
            }
            diagnostics.push(json!({"selection_id":record.id,"supersedes":id}));
        }
    }
    // Preparation accepts data only: the engine will assign the actual record identity.
    let proposed = input.get("selection_data");
    if proposed.is_some() && show.is_none() {
        return Err("selection_data requires completed full show".into());
    }
    let proposal: Option<Selection> = proposed
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("selection_data: {e}")))
        .transpose()?;
    let active = proposal
        .as_ref()
        .or_else(|| selections.last().map(|(_, s)| s));
    let mut wanted = HashSet::new();
    diagnostics.clear();
    if let Some(selection) = active {
        if selection.target_id != config.target.id || selection.slot_id != slot_id {
            return Err("selection target/slot mismatch".into());
        }
        if let Some(id) = &selection.supersedes {
            if !selections.iter().any(|(r, _)| r.id.as_str() == id) {
                return Err(format!("unknown/inapplicable supersedes {id}"));
            }
            diagnostics.push(json!({"supersedes":id}));
        }
        let sequence = if proposal.is_some() {
            None
        } else {
            let record = selections.last().unwrap().0;
            wanted.insert(record.id.as_str());
            Some(record.sequence)
        };
        for id in &selection.record_ids {
            let original = records
                .iter()
                .find(|r| r.id.as_str() == id)
                .ok_or_else(|| format!("selection references unknown record {id}"))?;
            if original.kind != "review-evidence"
                || sequence.is_some_and(|seq| original.sequence >= seq)
            {
                return Err(format!(
                    "selected record {id} is not original prior review-evidence"
                ));
            }
            if !wanted.insert(original.id.as_str()) {
                return Err(format!("duplicate selected record {id}"));
            }
            let stale = original.data["target_id"] != snapshot.target_id
                || original.data["target_sha256"] != snapshot.sha256
                || original.data["profile_version"] != config.profile_version;
            diagnostics
                .push(json!({"record_id":id,"role":"historical-context-not-proof","stale":stale}));
        }
    }
    let selected: Vec<_> = records
        .iter()
        .filter(|r| wanted.contains(r.id.as_str()))
        .collect();
    let ids: Vec<_> = selected.iter().map(|r| &r.id).collect();
    let identity = json!({"target_id":snapshot.target_id,"target_path":snapshot.path,
        "target_sha256":snapshot.sha256,"profile_version":config.profile_version});
    let receipt = json!({"current_target":identity,"diagnostics":diagnostics});
    if let Some(selection) = active {
        if selection
            .receipt
            .as_ref()
            .is_some_and(|supplied| supplied != &receipt)
        {
            return Err("selection receipt is stale or mismatched; prepare and append a refreshed selection".into());
        }
    }
    if let Some(data) = proposed {
        let mut data = data.clone();
        data["receipt"] = receipt;
        return Ok(data);
    }
    // Filter stdout stays reference-only. Only a genuine appended selection
    // carries a receipt into bound stdin; successful stderr is not delivery.
    eprintln!(
        "{}",
        json!({"current_target":identity,"diagnostics":diagnostics})
    );
    if show.is_none() {
        Ok(json!({"record_ids":ids}))
    } else {
        Ok(
            json!({"record_ids":ids,"context":selected,"current_target":identity,"diagnostics":diagnostics}),
        )
    }
}

pub fn run(args: &[String]) -> i32 {
    use std::io::Read;
    let result = (|| {
        if args.len() != 1 {
            return Err(
                "usage: policy-document commission FROZEN_PROFILE_JSON < packet.json".into(),
            );
        }
        let profile: Value = serde_json::from_str(&args[0]).map_err(|e| format!("profile: {e}"))?;
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        let input = serde_json::from_str(&input).map_err(|e| format!("packet: {e}"))?;
        prepare(&input, &profile)
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
