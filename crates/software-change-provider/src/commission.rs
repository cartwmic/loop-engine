//! Pure recipient selection. Core transports references; this module owns steering.
use loop_core::{ContextRecord, WorkSlot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// An additive validation obligation, never a replacement or a verdict.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProofCommand {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub owner: String,
    pub obligation: String,
}

/// Provider-owned validation-command data is namespaced so the command
/// specification cannot collide with engine-owned append fields. Keep
/// accepting the historical flat shape for records written before the
/// namespace was introduced.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidationCommandEnvelope {
    spec: ProofCommand,
}

fn validation_command(data: &Value) -> Result<ProofCommand, String> {
    if data.get("spec").is_some() {
        serde_json::from_value::<ValidationCommandEnvelope>(data.clone())
            .map(|envelope| envelope.spec)
            .map_err(|error| error.to_string())
    } else {
        serde_json::from_value::<ProofCommand>(data.clone()).map_err(|error| error.to_string())
    }
}

pub const STEERING_KIND: &str = "user-steering";
pub const INCORPORATION_KIND: &str = "steering-incorporation";

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Target {
    All,
    Slots {
        ids: Vec<String>,
    },
    Tasks {
        plan_revision: String,
        ids: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Steering {
    pub target: Target,
    pub instruction: String,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub proof_updates: Vec<ProofUpdate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofUpdate {
    pub proof_id: String,
    pub reason: String,
    pub owner: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct Commission {
    pub record_ids: Vec<String>,
    pub steering_ids: Vec<String>,
    pub diagnostics: Vec<String>,
    pub proof_commands: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_stage: Option<String>,
}

fn ids_valid(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| !id.trim().is_empty())
        && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

/// No file writes, launches, semantic classification, or plan revision changes.
/// `task_id=None` at implement retains task steering for later exact-task narrowing.
pub fn select(
    records: &[ContextRecord],
    slots: &[WorkSlot],
    slot_id: &str,
    plan: Option<&Value>,
    task_id: Option<&str>,
) -> Result<Commission, String> {
    let slot = slots
        .iter()
        .find(|slot| slot.id.as_str() == slot_id)
        .ok_or_else(|| format!("unknown commission slot `{slot_id}`"))?;
    let revision = plan.and_then(|p| p.get("revision")).and_then(Value::as_str);
    let tasks: BTreeSet<&str> = plan
        .and_then(|p| p.get("tasks"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("id").and_then(Value::as_str))
        .collect();
    if let Some(task) = task_id {
        if slot_id != "implement" || (task != "summarizer" && !tasks.contains(task)) {
            return Err(format!(
                "unknown commission task `{task}` for slot `{slot_id}`"
            ));
        }
    }
    let mut previous = None;
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::new();
    let mut superseded = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for record in records {
        if previous.is_some_and(|sequence| sequence >= record.sequence)
            || !seen.insert(record.id.as_str())
        {
            return Err("commission context must have unique IDs in sequence order".into());
        }
        previous = Some(record.sequence);
        if record.kind == INCORPORATION_KIND {
            validate_incorporation(&record.data, records)
                .map_err(|e| format!("incorporation `{}`: {e}", record.id))?;
        }
        if record.kind != STEERING_KIND {
            continue;
        }
        let steering: Steering = serde_json::from_value(record.data.clone())
            .map_err(|e| format!("steering `{}`: {e}", record.id))?;
        if steering.instruction.trim().is_empty() {
            return Err(format!("steering `{}` requires instruction", record.id));
        }
        if !steering.supersedes.is_empty() && !ids_valid(&steering.supersedes) {
            return Err(format!(
                "steering `{}` has invalid supersedes IDs",
                record.id
            ));
        }
        for id in &steering.supersedes {
            if !parsed
                .iter()
                .any(|(prior, _, _): &(&ContextRecord, Steering, bool)| prior.id.as_str() == id)
            {
                return Err(format!(
                    "steering `{}` supersedes unknown or non-earlier steering `{id}`",
                    record.id
                ));
            }
            superseded.insert(id.clone());
        }
        let applies = match &steering.target {
            Target::All => true,
            Target::Slots { ids } => {
                if !ids_valid(ids)
                    || ids
                        .iter()
                        .any(|id| !slots.iter().any(|s| s.id.as_str() == id))
                {
                    return Err(format!("steering `{}` has invalid slot targets", record.id));
                }
                ids.iter().any(|id| id == slot_id)
            }
            Target::Tasks { plan_revision, ids } => {
                if !ids_valid(ids) || plan_revision.trim().is_empty() {
                    return Err(format!("steering `{}` has invalid task target", record.id));
                }
                if revision != Some(plan_revision.as_str()) {
                    diagnostics.push(format!("steering `{}` stale task revision `{plan_revision}` (current {revision:?}); not applied", record.id));
                    false
                } else {
                    if ids.iter().any(|id| !tasks.contains(id.as_str())) {
                        return Err(format!("steering `{}` has unknown task target", record.id));
                    }
                    slot_id == "implement"
                        && task_id.is_none_or(|task| ids.iter().any(|id| id == task))
                }
            }
        };
        // Validate execution updates even for another recipient: malformed durable
        // instructions must not silently become executable on a later launch.
        let stale_task = matches!(&steering.target, Target::Tasks { plan_revision, .. } if revision != Some(plan_revision.as_str()));
        if !stale_task {
            for update in &steering.proof_updates {
                validate_update(update, plan)?;
            }
        }
        parsed.push((record, steering, applies));
    }
    let selected: BTreeSet<&str> = parsed
        .iter()
        .filter(|(r, _, applies)| *applies && !superseded.contains(r.id.as_str()))
        .map(|(r, _, _)| r.id.as_str())
        .collect();
    let mut proof_commands = plan
        .and_then(|p| p.get("proof_commands"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (record, steering, _) in &parsed {
        if !selected.contains(record.id.as_str()) {
            continue;
        }
        for update in &steering.proof_updates {
            let proof = proof_commands
                .iter_mut()
                .find(|p| p["id"] == update.proof_id)
                .expect("validated proof");
            if let Some(owner) = &update.owner {
                proof["owner"] = json!(owner);
            }
            if let Some(command) = &update.command {
                proof["command"] = json!(command);
                proof["args"] = json!(update.args.as_ref().expect("validated argv"));
            }
        }
    }
    let mut proof_ids: BTreeSet<String> = proof_commands
        .iter()
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect();
    for record in records.iter().filter(|r| r.kind == "validation-command") {
        let command = validation_command(&record.data)
            .map_err(|e| format!("invalid validation-command `{}`: {e}", record.id))?;
        if [
            &command.id,
            &command.command,
            &command.owner,
            &command.obligation,
        ]
        .iter()
        .any(|s| s.trim().is_empty())
            || !proof_ids.insert(command.id.clone())
        {
            return Err(format!("validation-command `{}` has blank fields or duplicates/replaces an existing proof ID", record.id));
        }
        proof_commands.push(serde_json::to_value(command).map_err(|e| e.to_string())?);
    }
    let record_ids = records
        .iter()
        .filter(|r| {
            if r.kind == STEERING_KIND {
                selected.contains(r.id.as_str())
            } else {
                slot.stdin_context_kinds.contains(&r.kind)
            }
        })
        .map(|r| r.id.as_str().to_owned())
        .collect();
    Ok(Commission {
        record_ids,
        steering_ids: records
            .iter()
            .filter(|r| selected.contains(r.id.as_str()))
            .map(|r| r.id.as_str().to_owned())
            .collect(),
        diagnostics,
        proof_commands,
        review_stage: None,
    })
}

fn record_belongs_to_stage(record: &ContextRecord, records: &[ContextRecord], stage: &str) -> bool {
    fn visit(
        record: &ContextRecord,
        records: &[ContextRecord],
        stage: &str,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        // Applicability references are durable input, not trusted topology.
        // Reject a cycle rather than recursing forever on malformed context.
        if !visited.insert(record.id.as_str().to_owned()) {
            return false;
        }
        if record.kind == "review-evidence" {
            return record
                .data
                .get("review_stage")
                .or_else(|| record.data.get("stage"))
                .and_then(Value::as_str)
                .unwrap_or("aggregate")
                == stage;
        }
        if record.kind == "evidence-applicability" {
            let Some(origin) = record.data.get("origin").and_then(Value::as_object) else {
                return false;
            };
            let Some(source_id) = origin.get("id").and_then(Value::as_str) else {
                return false;
            };
            return records
                .iter()
                .find(|candidate| candidate.id.as_str() == source_id)
                .is_some_and(|source| visit(source, records, stage, visited));
        }
        true
    }

    visit(record, records, stage, &mut BTreeSet::new())
}

fn validate_update(update: &ProofUpdate, plan: Option<&Value>) -> Result<(), String> {
    let exists = plan
        .and_then(|p| p.get("proof_commands"))
        .and_then(Value::as_array)
        .is_some_and(|proofs| proofs.iter().any(|p| p["id"] == update.proof_id));
    if !exists
        || update.reason.trim().is_empty()
        || update.owner.as_ref().is_some_and(|s| s.trim().is_empty())
        || update.command.as_ref().is_some_and(|s| s.trim().is_empty())
        || update.command.is_some() != update.args.is_some()
        || (update.owner.is_none() && update.command.is_none())
    {
        return Err(format!("invalid execution update for proof `{}`; name an existing proof, reason and owner and/or command with args", update.proof_id));
    }
    Ok(())
}

/// Narrow an already-selected invocation snapshot without resolving supersedes
/// again: their source records may correctly have been removed by the filter.
pub fn task_steering(
    records: &[ContextRecord],
    plan: &Value,
    task_id: &str,
) -> Result<Vec<ContextRecord>, String> {
    let mut selected = Vec::new();
    for record in records.iter().filter(|r| r.kind == STEERING_KIND) {
        let steering: Steering =
            serde_json::from_value(record.data.clone()).map_err(|e| e.to_string())?;
        let applies = match steering.target {
            Target::All => true,
            Target::Slots { ids } => ids.iter().any(|id| id == "implement"),
            Target::Tasks { plan_revision, ids } => {
                plan["revision"] == plan_revision && ids.iter().any(|id| id == task_id)
            }
        };
        if applies {
            selected.push(record.clone());
        }
    }
    Ok(selected)
}

/// An unbound receipt is an attestation, never inferred proof or progression.
pub fn validate_incorporation(data: &Value, records: &[ContextRecord]) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        steering_ids: Vec<String>,
        applied: String,
    }
    let receipt: Receipt = serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
    if !ids_valid(&receipt.steering_ids)
        || receipt.applied.trim().is_empty()
        || receipt.steering_ids.iter().any(|id| {
            !records
                .iter()
                .any(|r| r.id.as_str() == id && r.kind == STEERING_KIND)
        })
    {
        return Err("incorporation requires existing steering IDs and what was applied".into());
    }
    Ok(())
}

pub fn load_plan(root: &Path) -> Result<Option<Value>, String> {
    match std::fs::read(root.join("plan.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("plan.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("plan.json: {e}")),
    }
}

/// Accept either a filter packet or the ordinary completed show envelope.
/// Filter stdout stays closed `{record_ids}`; inspection also returns receipts.
pub fn inspect(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
) -> Result<Value, String> {
    inspect_with_stage(input, requested_slot, task, None)
}

/// Inspect a commission for one frozen review stage. Stage filtering is only
/// a delivery projection; it does not alter the run's context or policy.
pub fn inspect_with_stage(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
) -> Result<Value, String> {
    let show = input.get("result");
    if show.is_some() && input.get("status").and_then(Value::as_str) != Some("completed") {
        return Err("commission requires a completed show envelope".into());
    }
    let packet = show.unwrap_or(input);
    let slot_id = requested_slot
        .or_else(|| packet.get("slot_id").and_then(Value::as_str))
        .ok_or("commission requires --slot SLOT for show inspection")?;
    let slots: Vec<WorkSlot> = serde_json::from_value(
        packet
            .get("work_slots")
            .cloned()
            .ok_or("missing work_slots")?,
    )
    .map_err(|e| e.to_string())?;
    let mut records: Vec<ContextRecord> =
        serde_json::from_value(packet.get("context").cloned().ok_or("missing context")?)
            .map_err(|e| e.to_string())?;
    if let Some(stage) = requested_stage {
        if !matches!(stage, "individual" | "aggregate") {
            return Err("commission review stage must be `individual` or `aggregate`".to_owned());
        }
        let all_records = records.clone();
        records.retain(|record| record_belongs_to_stage(record, &all_records, stage));
    }
    let slot = slots
        .iter()
        .find(|s| s.id.as_str() == slot_id)
        .ok_or_else(|| format!("unknown commission slot `{slot_id}`"))?;
    // Ordinary show contains all kinds; bound packets already have this exact
    // eligibility restriction. Historical catalogs do not gain new kinds.
    records.retain(|record| slot.stdin_context_kinds.contains(&record.kind));
    let root = packet
        .get("artifact_root")
        .or_else(|| {
            packet
                .get("initial_input")
                .and_then(|v| v.get("artifact_root"))
        })
        .and_then(Value::as_str)
        .ok_or("missing artifact_root")?;
    let plan = load_plan(Path::new(root))?;
    let mut selected = select(&records, &slots, slot_id, plan.as_ref(), task)?;
    selected.review_stage = requested_stage.map(str::to_owned);
    for diagnostic in &selected.diagnostics {
        eprintln!("{diagnostic}");
    }
    if show.is_none() {
        Ok(json!({"record_ids": selected.record_ids}))
    } else {
        let context: Vec<_> = records
            .iter()
            .filter(|r| selected.record_ids.iter().any(|id| id == r.id.as_str()))
            .collect();
        let mut output =
            json!({"slot_id":slot_id,"task_id":task,"commission":selected,"context":context});
        if slot_id.starts_with("validation") {
            if let Ok(bytes) = std::fs::read(Path::new(root).join("validation-report.json")) {
                let report: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                let mut ids: Vec<&Value> = report["criteria"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|r| r["verdict_ids"].as_array().into_iter().flatten())
                    .collect();
                ids.extend(report["goal_verdict_ids"].as_array().into_iter().flatten());
                output["validation_collection"] = json!(ids.into_iter().map(|id| {
                    let selected = records.iter().find(|r|Some(r.id.as_str())==id.as_str());
                    let carried = selected.is_some_and(|r|r.kind=="evidence-applicability");
                    let source = if carried { selected.and_then(|r|records.iter().find(|s|s.id.as_str()==r.data["origin"]["id"].as_str().unwrap_or(""))) } else { selected };
                    json!({"selected_id":id,"mode":if carried{"carried"}else if source.is_some(){"fresh"}else{"pending"},"source":source})
                }).collect::<Vec<_>>());
            }
        }
        Ok(output)
    }
}

pub fn run_from_stdin(args: &[String]) -> i32 {
    let mut slot = None;
    let mut task = None;
    let mut stage = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let target = match arg.as_str() {
            "--slot" => &mut slot,
            "--task" => &mut task,
            "--stage" => &mut stage,
            _ => {
                eprintln!("unknown commission argument `{arg}`");
                return 2;
            }
        };
        if target.is_some() {
            eprintln!("duplicate {arg}");
            return 2;
        }
        *target = iter.next().map(String::as_str);
        if target.is_none() {
            eprintln!("missing value for {arg}");
            return 2;
        }
    }
    let result = serde_json::from_reader(std::io::stdin())
        .map_err(|e| e.to_string())
        .and_then(|input| inspect_with_stage(&input, slot, task, stage));
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(e) => {
            eprintln!("commission: {e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    fn record(id: &str, n: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            STEERING_KIND,
            data,
            SemanticSequence::new(n),
            Timestamp::from_unix_millis(0),
        )
    }
    #[test]
    fn recovery_steering_selection_supersession_tasks_and_proofs() {
        let slots = vec![
            WorkSlot::new("implement", "implement", "ready")
                .with_stdin_context_kinds(vec![STEERING_KIND.into()]),
            WorkSlot::new("design-review", "design-review", "approved"),
        ];
        let plan = json!({"revision":"1","tasks":[{"id":"A"},{"id":"B"}],"proof_commands":[{"id":"p","owner":"driver","command":"old","args":[],"obligation":"unchanged"}]});
        let records = vec![
            record(
                "old",
                1,
                json!({"target":{"kind":"all"},"instruction":"old"}),
            ),
            record(
                "new",
                2,
                json!({"target":{"kind":"slots","ids":["implement"]},"instruction":"new","supersedes":["old"],"proof_updates":[{"proof_id":"p","owner":"proof-owner","reason":"one final matrix"}]}),
            ),
            record(
                "task",
                3,
                json!({"target":{"kind":"tasks","plan_revision":"1","ids":["A"]},"instruction":"task"}),
            ),
            record(
                "stale",
                4,
                json!({"target":{"kind":"tasks","plan_revision":"0","ids":["removed"]},"instruction":"stale"}),
            ),
        ];
        let selected = select(&records, &slots, "implement", Some(&plan), None).unwrap();
        assert_eq!(selected.steering_ids, ["new", "task"]);
        assert_eq!(selected.diagnostics.len(), 1);
        assert_eq!(selected.proof_commands[0]["owner"], "proof-owner");
        assert_eq!(selected.proof_commands[0]["obligation"], "unchanged");
        assert_eq!(plan["revision"], "1");
        assert!(select(&records, &slots, "design-review", Some(&plan), None)
            .unwrap()
            .steering_ids
            .is_empty());
        let snapshot = vec![records[1].clone(), records[2].clone()];
        assert_eq!(task_steering(&snapshot, &plan, "A").unwrap().len(), 2);
        assert_eq!(task_steering(&snapshot, &plan, "B").unwrap().len(), 1);
        assert_eq!(
            task_steering(&snapshot, &plan, "summarizer").unwrap().len(),
            1
        );
        assert!(validate_incorporation(
            &json!({"steering_ids":["new"],"applied":"owner only"}),
            &records
        )
        .is_ok());
        assert!(validate_incorporation(
            &json!({"steering_ids":["missing"],"applied":"x"}),
            &records
        )
        .is_err());
    }
    #[test]
    fn recovery_steering_invalid_targets_and_updates_refuse() {
        let slots = vec![WorkSlot::new("implement", "implement", "ready")];
        let plan = json!({"revision":"1","tasks":[{"id":"A"}]});
        for data in [
            json!({"target":{"kind":"slots","ids":["missing"]},"instruction":"x"}),
            json!({"target":{"kind":"tasks","plan_revision":"1","ids":["missing"]},"instruction":"x"}),
            json!({"target":{"kind":"all"},"instruction":"x","supersedes":["missing"]}),
            json!({"target":{"kind":"all"},"instruction":"x","proof_updates":[{"proof_id":"missing","owner":"driver","reason":"x"}]}),
        ] {
            assert!(select(
                &[record("bad", 1, data)],
                &slots,
                "implement",
                Some(&plan),
                None
            )
            .is_err());
        }
    }
}
