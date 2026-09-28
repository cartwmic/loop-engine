//! Deterministic recipient and review-context selection. Core transports references; this module owns steering and review packet projection.
use loop_core::{ContextRecord, WorkSlot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
    #[serde(default)]
    pub intent_baseline: Option<IntentBaseline>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentBaseline {
    pub revision: String,
    pub locator: String,
    pub sha256: String,
    /// Exact UTF-8 JSON source bytes, retained verbatim as a string.
    pub json_bytes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CallBudget {
    author: String,
    model_id: String,
    context_window_tokens: u64,
    system_tokens: u64,
    framing_tokens: u64,
    output_reserve_tokens: u64,
    reasoning_reserve_tokens: u64,
}

impl CallBudget {
    fn reserve_tokens(&self) -> u64 {
        self.system_tokens
            .saturating_add(self.framing_tokens)
            .saturating_add(self.output_reserve_tokens)
            .saturating_add(self.reasoning_reserve_tokens)
    }

    fn input_capacity(&self) -> u64 {
        self.context_window_tokens
            .saturating_sub(self.reserve_tokens())
    }
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
        if let Some(baseline) = steering.intent_baseline.as_ref() {
            validate_intent_baseline(baseline).map_err(|error| {
                format!(
                    "steering `{}` has invalid intent baseline: {error}",
                    record.id
                )
            })?;
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
            let mut projected = record.clone();
            if let Some(data) = projected.data.as_object_mut() {
                data.remove("intent_baseline");
            }
            selected.push(projected);
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

/// Build a read-only review packet projection for an optional frozen stage.
/// Selection never mutates durable context or changes frozen review policy.
fn inspect_with_stage_raw(
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

pub fn inspect_with_stage(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
) -> Result<Value, String> {
    inspect_with_budgets(input, requested_slot, task, requested_stage, None)
}

const INITIAL_CONTEXT_BYTES: usize = 32 * 1024;
const TOTAL_COMMISSION_BYTES: usize = 256 * 1024;

#[derive(Clone)]
struct ReviewSource {
    locator: String,
    sha256: String,
    bytes: Vec<u8>,
    value: Value,
}

fn review_subject(gate: &str) -> Option<&'static str> {
    match gate {
        "intent-review" | "intent-adversarial-review" => Some("intent.json"),
        "design-review" | "design-adversarial-review" => Some("design.json"),
        "plan-review" | "plan-adversarial-review" => Some("plan.json"),
        "implementation-review" | "implementation-adversarial-review" => {
            Some("implementation-report.json")
        }
        "validation-review" | "validation-adversarial-review" => Some("validation-report.json"),
        _ => None,
    }
}

fn parent_gate(gate: &str) -> Option<&'static str> {
    match gate {
        "intent-adversarial-review" => Some("intent-review"),
        "design-adversarial-review" => Some("design-review"),
        "plan-adversarial-review" => Some("plan-review"),
        "implementation-adversarial-review" => Some("implementation-review"),
        "validation-adversarial-review" => Some("validation-review"),
        _ => None,
    }
}

fn read_review_sources(root: &Path, subject: &str) -> Result<(String, Vec<ReviewSource>), String> {
    let mut paths = vec!["intent.json"];
    if subject != "intent.json" {
        paths.push(subject);
    }
    let mut sources = Vec::new();
    let mut subject_value = None;
    for locator in paths {
        let path = root.join(locator);
        let bytes = std::fs::read(&path).map_err(|error| {
            format!(
                "mandatory review source `{locator}` is unavailable at {}: {error}",
                path.display()
            )
        })?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!("mandatory review source `{locator}` is not valid JSON: {error}")
        })?;
        if locator == subject {
            subject_value = Some(value.clone());
        }
        sources.push(ReviewSource {
            locator: locator.to_owned(),
            sha256: format!("sha256:{:x}", Sha256::digest(&bytes)),
            bytes,
            value,
        });
    }
    let revision = subject_value
        .as_ref()
        .and_then(|value| value.get("revision"))
        .and_then(Value::as_str)
        .filter(|revision| !revision.trim().is_empty())
        .ok_or_else(|| format!("mandatory review source `{subject}` has no non-empty revision"))?
        .to_owned();
    Ok((revision, sources))
}

fn evidence_stage(record: &ContextRecord) -> &str {
    record
        .data
        .get("review_stage")
        .or_else(|| record.data.get("stage"))
        .and_then(Value::as_str)
        .unwrap_or("aggregate")
}

fn matches_review(record: &ContextRecord, gate: &str, subject: &str) -> bool {
    record.kind == "review-evidence"
        && record.data.get("gate").and_then(Value::as_str) == Some(gate)
        && record.data.get("subject").and_then(Value::as_str) == Some(subject)
}

fn latest_aggregate_by_axis_author<'a>(
    records: &'a [ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
) -> Result<BTreeMap<(String, String, String), &'a ContextRecord>, String> {
    let mut latest = BTreeMap::new();
    for record in records.iter().filter(|record| {
        matches_review(record, gate, subject)
            && evidence_stage(record) == "aggregate"
            && record.data.get("subject_revision").and_then(Value::as_str) == Some(revision)
    }) {
        let axis = record
            .data
            .get("policy_id")
            .and_then(Value::as_str)
            .filter(|axis| !axis.is_empty())
            .ok_or_else(|| format!("parent aggregate `{}` has no axis identity", record.id))?;
        let name = record
            .data
            .pointer("/author/name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("parent aggregate `{}` has no author identity", record.id))?;
        let kind = record
            .data
            .pointer("/author/kind")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| format!("parent aggregate `{}` has no author kind", record.id))?;
        latest.insert((axis.to_owned(), name.to_owned(), kind.to_owned()), record);
    }
    Ok(latest)
}

fn has_current_aggregate(
    records: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
) -> bool {
    records.iter().any(|record| {
        matches_review(record, gate, subject)
            && evidence_stage(record) == "aggregate"
            && record.data.get("subject_revision").and_then(Value::as_str) == Some(revision)
    })
}

fn has_material_confirmation_sources(
    records: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
) -> bool {
    let ledger = latest_ledger(records, gate, subject);
    let old_finding_source = ledger
        .and_then(|ledger| ledger.data.get("findings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|finding| finding.get("disposition").and_then(Value::as_str) == Some("accepted"))
        .filter_map(|finding| finding.pointer("/source/id").and_then(Value::as_str))
        .filter_map(|id| records.iter().find(|record| record.id.as_str() == id))
        .any(|source| {
            matches_review(source, gate, subject)
                && source.data.get("subject_revision").and_then(Value::as_str) != Some(revision)
        });
    if old_finding_source {
        return true;
    }
    records.iter().any(|record| {
        record.kind == "evidence-applicability"
            && record
                .data
                .pointer("/target/subject")
                .and_then(Value::as_str)
                == Some(subject)
            && record
                .data
                .pointer("/target/revision")
                .and_then(Value::as_str)
                == Some(revision)
            && record
                .data
                .pointer("/origin/id")
                .and_then(Value::as_str)
                .and_then(|id| records.iter().find(|source| source.id.as_str() == id))
                .is_some_and(|source| {
                    matches_review(source, gate, subject)
                        && source.data.get("subject_revision").and_then(Value::as_str)
                            != Some(revision)
                })
    })
}

fn latest_ledger<'a>(
    records: &'a [ContextRecord],
    gate: &str,
    subject: &str,
) -> Option<&'a ContextRecord> {
    records.iter().rev().find(|record| {
        record.kind == "finding-ledger"
            && record.data.get("gate").and_then(Value::as_str) == Some(gate)
            && record.data.get("subject").and_then(Value::as_str) == Some(subject)
    })
}

fn owner_source_targets_intent(record: &ContextRecord) -> bool {
    let Ok(steering) = serde_json::from_value::<Steering>(record.data.clone()) else {
        return true;
    };
    match steering.target {
        Target::All => true,
        Target::Slots { ids } => ids.iter().any(|id| {
            matches!(
                id.as_str(),
                "intent-draft" | "intent-review" | "intent-adversarial-review"
            )
        }),
        Target::Tasks { .. } => false,
    }
}

fn active_intent_baseline(records: &[&ContextRecord]) -> Option<(String, IntentBaseline)> {
    let mut parsed = Vec::new();
    let mut superseded = BTreeSet::new();
    for record in records
        .iter()
        .filter(|record| record.kind == "user-steering")
    {
        let Ok(steering) = serde_json::from_value::<Steering>(record.data.clone()) else {
            continue;
        };
        superseded.extend(steering.supersedes.iter().cloned());
        parsed.push((*record, steering));
    }
    parsed.into_iter().rev().find_map(|(record, steering)| {
        (!superseded.contains(record.id.as_str()))
            .then_some(steering.intent_baseline)
            .flatten()
            .map(|baseline| (record.id.as_str().to_owned(), baseline))
    })
}

fn ledger_sources(record: Option<&ContextRecord>) -> BTreeSet<String> {
    record
        .and_then(|record| record.data.get("findings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|finding| finding.pointer("/source/id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn validation_record_ids(
    root: &Path,
    context: &[ContextRecord],
) -> BTreeMap<String, BTreeSet<String>> {
    let Ok(bytes) = std::fs::read(root.join("validation-report.json")) else {
        return BTreeMap::new();
    };
    let Ok(report) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    let mut ids = BTreeMap::new();
    for (field, kind) in [
        ("command_evidence_ids", "command-evidence"),
        ("goal_verdict_ids", "goal-verdict"),
    ] {
        ids.insert(
            kind.to_owned(),
            report[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        );
    }
    ids.insert(
        "criterion-verdict".to_owned(),
        report["criteria"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|row| row["verdict_ids"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    );
    let indexed: BTreeSet<_> = ids
        .get("criterion-verdict")
        .into_iter()
        .flatten()
        .chain(ids.get("goal-verdict").into_iter().flatten())
        .cloned()
        .collect();
    let applicable: BTreeSet<_> = context
        .iter()
        .filter(|record| record.kind == "evidence-applicability")
        .filter(|record| {
            record
                .data
                .pointer("/origin/id")
                .and_then(Value::as_str)
                .is_some_and(|id| indexed.contains(id))
        })
        .map(|record| record.id.as_str().to_owned())
        .collect();
    ids.insert("evidence-applicability".to_owned(), applicable);
    ids
}

fn project_review_context(
    records: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    requested_stage: Option<&str>,
    root: &Path,
) -> Result<Vec<ContextRecord>, String> {
    let parent = parent_gate(gate);
    let parent_aggregate_ids = if let Some(parent) = parent {
        let parent_aggregates =
            latest_aggregate_by_axis_author(records, parent, subject, revision)?;
        if parent_aggregates.is_empty() {
            return Err(format!("challenge `{gate}` is missing current parent aggregate grounds for `{subject}`; inspect the ordinary parent review commission"));
        }
        for record in parent_aggregates.values() {
            if record
                .data
                .get("review_contract_version")
                .and_then(Value::as_u64)
                != Some(2)
            {
                return Err(format!(
                    "challenge parent `{}` has no version-2 grounds; inspect its selected output",
                    record.id
                ));
            }
            crate::criterion::validate_grounding(
                record.data.get("grounds").ok_or_else(|| {
                    format!("challenge parent `{}` is missing grounds", record.id)
                })?,
                root,
            )
            .map_err(|error| {
                format!(
                    "challenge parent `{}` grounds are unavailable: {error}",
                    record.id
                )
            })?;
        }
        parent_aggregates
            .values()
            .map(|record| record.id.as_str().to_owned())
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    let initial_current_gate = !has_current_aggregate(records, gate, subject, revision)
        && !has_material_confirmation_sources(records, gate, subject, revision);
    let own_ledger = latest_ledger(records, gate, subject);
    let mut excluded_ids = BTreeSet::new();
    if initial_current_gate {
        excluded_ids.extend(
            records
                .iter()
                .filter(|record| {
                    matches_review(record, gate, subject) && evidence_stage(record) == "individual"
                })
                .map(|record| record.id.as_str().to_owned()),
        );
    }
    let ledger_refs = ledger_sources(own_ledger);
    let mut applicability_refs = BTreeSet::new();
    for record in records
        .iter()
        .filter(|record| record.kind == "evidence-applicability")
    {
        if record
            .data
            .pointer("/target/subject")
            .and_then(Value::as_str)
            != Some(subject)
            || record
                .data
                .pointer("/target/revision")
                .and_then(Value::as_str)
                != Some(revision)
        {
            continue;
        }
        let source_id = record
            .data
            .pointer("/origin/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!(
                    "current applicability `{}` has no original source locator",
                    record.id
                )
            })?;
        let source = records
            .iter()
            .find(|candidate| candidate.id.as_str() == source_id)
            .ok_or_else(|| {
                format!(
                    "current applicability `{}` is missing source `{source_id}`",
                    record.id
                )
            })?;
        if (matches_review(source, gate, subject) && !excluded_ids.contains(source_id))
            || parent.is_some_and(|parent| {
                matches_review(source, parent, subject) && evidence_stage(source) == "aggregate"
            })
        {
            applicability_refs.insert(source_id.to_owned());
        }
    }
    for (ledger_gate, ledger) in [(gate, own_ledger)] {
        let Some(ledger) = ledger else { continue };
        let findings = ledger
            .data
            .get("findings")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("confirmation ledger `{}` has no findings array", ledger.id))?;
        for finding in findings {
            let source_id = finding
                .pointer("/source/id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "confirmation ledger `{}` has a finding without an exact source locator",
                        ledger.id
                    )
                })?;
            let source = records
                .iter()
                .find(|record| record.id.as_str() == source_id)
                .ok_or_else(|| format!("mandatory confirmation finding source `{source_id}` is missing; inspect retained source for {ledger_gate}/{subject}"))?;
            if !matches_review(source, ledger_gate, subject) {
                return Err(format!("mandatory confirmation finding source `{source_id}` has the wrong gate or subject"));
            }
        }
    }
    let validation_ids = if gate.starts_with("validation-") {
        validation_record_ids(root, records)
    } else {
        BTreeMap::new()
    };
    let mut selected_review_ids = BTreeSet::new();
    for record in records {
        let own = matches_review(record, gate, subject);
        let parent_source = parent.is_some_and(|parent| matches_review(record, parent, subject));
        if excluded_ids.contains(record.id.as_str()) {
            continue;
        }
        if own
            && (ledger_refs.contains(record.id.as_str())
                || applicability_refs.contains(record.id.as_str())
                || (requested_stage.is_some_and(|stage| evidence_stage(record) == stage)
                    && record.data.get("subject_revision").and_then(Value::as_str)
                        == Some(revision)))
        {
            selected_review_ids.insert(record.id.as_str().to_owned());
        } else if parent_source
            && (parent_aggregate_ids.contains(record.id.as_str())
                || ledger_refs.contains(record.id.as_str())
                || applicability_refs.contains(record.id.as_str()))
        {
            selected_review_ids.insert(record.id.as_str().to_owned());
        }
    }
    let mut selected = Vec::new();
    for record in records {
        if record.kind == "review-evidence" {
            if selected_review_ids.contains(record.id.as_str()) {
                selected.push(record.clone());
            }
            continue;
        }
        if record.kind == "evidence-applicability" {
            if gate.starts_with("validation-")
                && validation_ids
                    .get("evidence-applicability")
                    .is_some_and(|ids| ids.contains(record.id.as_str()))
            {
                let source_id = record
                    .data
                    .pointer("/origin/id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        format!(
                            "validation applicability `{}` has no original verdict source",
                            record.id
                        )
                    })?;
                let source = records
                    .iter()
                    .find(|candidate| candidate.id.as_str() == source_id)
                    .ok_or_else(|| format!("validation applicability `{}` is missing its original verdict `{source_id}`", record.id))?;
                if !matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                    || record
                        .data
                        .pointer("/target/subject")
                        .and_then(Value::as_str)
                        != Some(subject)
                {
                    return Err(format!("validation applicability `{}` does not reference a validation verdict for `{subject}`", record.id));
                }
                selected.push(record.clone());
                continue;
            }
            let source = record.data.pointer("/origin/id").and_then(Value::as_str);
            let source_record =
                source.and_then(|id| records.iter().find(|candidate| candidate.id.as_str() == id));
            let source_relevant = source_record.is_some_and(|source_record| {
                (matches_review(source_record, gate, subject)
                    || parent.is_some_and(|parent| {
                        matches_review(source_record, parent, subject)
                            && evidence_stage(source_record) == "aggregate"
                    }))
                    && !(initial_current_gate
                        && matches_review(source_record, gate, subject)
                        && evidence_stage(source_record) == "individual")
            });
            let target_subject = record
                .data
                .pointer("/target/subject")
                .and_then(Value::as_str);
            let target_revision = record
                .data
                .pointer("/target/revision")
                .and_then(Value::as_str);
            if source_relevant
                && source.is_some_and(|id| !excluded_ids.contains(id))
                && target_subject == Some(subject)
                && target_revision == Some(revision)
            {
                selected.push(record.clone());
            }
            continue;
        }
        if record.kind == "finding-ledger" {
            let ledger_gate = record.data.get("gate").and_then(Value::as_str);
            let latest = if ledger_gate == Some(gate) {
                latest_ledger(records, gate, subject)
            } else {
                None
            };
            if !latest.is_some_and(|latest| latest.id == record.id)
                || record.data.get("subject").and_then(Value::as_str) != Some(subject)
            {
                continue;
            }
            let mut projected = record.clone();
            if let Some(findings) = projected
                .data
                .get_mut("findings")
                .and_then(Value::as_array_mut)
            {
                findings.retain(|finding| {
                    let source = finding.pointer("/source/id").and_then(Value::as_str);
                    source.is_some_and(|id| {
                        !excluded_ids.contains(id) && selected_review_ids.contains(id)
                    })
                });
            }
            if projected.data["findings"]
                .as_array()
                .is_some_and(|findings| !findings.is_empty())
            {
                selected.push(projected);
            }
            continue;
        }
        if record.kind == "user-steering"
            && subject == "intent.json"
            && owner_source_targets_intent(record)
        {
            selected.push(record.clone());
            continue;
        }
        if record.kind == "steering-incorporation" {
            // Driver incorporation is not an owner-authored instruction.
            continue;
        }
        if validation_ids
            .get(&record.kind)
            .is_some_and(|ids| ids.contains(record.id.as_str()))
        {
            selected.push(record.clone());
        }
    }
    Ok(selected)
}

fn validate_call_budgets(budgets: &[CallBudget]) -> Result<(), String> {
    if budgets.is_empty() {
        return Err(
            "review commission requires per-call model window and reserve metadata".to_owned(),
        );
    }
    let mut authors = BTreeSet::new();
    for budget in budgets {
        if budget.author.trim().is_empty()
            || budget.model_id.trim().is_empty()
            || !authors.insert(budget.author.as_str())
            || budget.context_window_tokens == 0
            || budget.system_tokens == 0
            || budget.framing_tokens == 0
            || budget.output_reserve_tokens == 0
            || budget.reasoning_reserve_tokens == 0
            || budget.input_capacity() == 0
        {
            return Err("per-call review budget has missing/duplicate author, model, window, or reserve values".to_owned());
        }
    }
    Ok(())
}

fn check_commission_budget(
    records: &[ContextRecord],
    sources: &[ReviewSource],
    budgets: &[CallBudget],
    initial: bool,
    artifact_root: &str,
) -> Result<Value, String> {
    validate_call_budgets(budgets)?;
    // Bound-worker stdin drops only the engine-owned top-level origin field;
    // byte accounting must use that exact delivery projection, while the full
    // routed snapshot remains retained for independent provider verification.
    let delivered: Vec<_> = records
        .iter()
        .cloned()
        .map(|mut record| {
            if let Some(data) = record.data.as_object_mut() {
                data.remove(loop_core::ENGINE_ORIGIN_KEY);
            }
            record
        })
        .collect();
    let location_context = json!({"artifact_root":artifact_root,"context":delivered});
    let context_bytes = serde_json::to_vec(&location_context)
        .map_err(|error| format!("could not measure delivered review context: {error}"))?
        .len();
    let original_bytes = sources
        .iter()
        .map(|source| source.bytes.len())
        .sum::<usize>();
    let total_bytes = context_bytes.saturating_add(original_bytes);
    let initial_limit = if initial {
        INITIAL_CONTEXT_BYTES
    } else {
        TOTAL_COMMISSION_BYTES
    };
    let locators = sources
        .iter()
        .map(|source| source.locator.as_str())
        .collect::<Vec<_>>();
    if (initial && total_bytes > INITIAL_CONTEXT_BYTES) || total_bytes > TOTAL_COMMISSION_BYTES {
        return Err(format!(
            "mandatory review sources do not fit the starting byte bounds (context={context_bytes}, originals={original_bytes}, supplied_total={total_bytes}, initial_limit={initial_limit}, total_limit={TOTAL_COMMISSION_BYTES}); no source was truncated; inspect: {}",
            locators.join(", ")
        ));
    }
    for budget in budgets {
        // UTF-8 byte length is used as a conservative token upper bound. The
        // driver still measures actual per-call model usage on the real path.
        if total_bytes as u64 > budget.input_capacity() {
            return Err(format!(
                "mandatory sources exceed `{}` call capacity for `{}`: conservative_input_upper_bound={total_bytes}, window={}, reserved(system={}, framing={}, output={}, reasoning={}); inspect: {}",
                budget.model_id,
                budget.author,
                budget.context_window_tokens,
                budget.system_tokens,
                budget.framing_tokens,
                budget.output_reserve_tokens,
                budget.reasoning_reserve_tokens,
                locators.join(", ")
            ));
        }
    }
    Ok(json!({
        "status":"fits-conservative-upper-bound",
        "context_bytes":context_bytes,
        "retrieved_original_bytes":original_bytes,
        "initial_evidence_bytes":if initial {json!(total_bytes)} else {Value::Null},
        "total_evidence_bytes":total_bytes,
        "initial_limit_bytes":INITIAL_CONTEXT_BYTES,
        "total_limit_bytes":TOTAL_COMMISSION_BYTES,
        "token_count_method":"UTF-8 byte upper bound; model windows and system/framing/output/reasoning reserves are caller-verified per call",
        "calls":budgets.iter().map(|budget|json!({
            "author":budget.author,
            "model_id":budget.model_id,
            "context_window_tokens":budget.context_window_tokens,
            "system_tokens":budget.system_tokens,
            "framing_tokens":budget.framing_tokens,
            "output_reserve_tokens":budget.output_reserve_tokens,
            "reasoning_reserve_tokens":budget.reasoning_reserve_tokens,
            "reserved_tokens":budget.reserve_tokens(),
            "available_input_tokens":budget.input_capacity()
        })).collect::<Vec<_>>(),
        "sources":sources.iter().map(|source|json!({"locator":source.locator,"sha256":source.sha256,"bytes":source.bytes.len()})).collect::<Vec<_>>()
    }))
}

fn inspect_with_budgets(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
    budgets: Option<&[CallBudget]>,
) -> Result<Value, String> {
    let show = input.get("result");
    let packet = show.unwrap_or(input);
    let slot_id = requested_slot
        .or_else(|| packet.get("slot_id").and_then(Value::as_str))
        .ok_or("commission requires --slot SLOT for show inspection")?;
    let subject = review_subject(slot_id);
    if subject.is_none() {
        return inspect_with_stage_raw(input, requested_slot, task, requested_stage);
    }
    if show.is_some() && input.get("status").and_then(Value::as_str) != Some("completed") {
        return Err("commission requires a completed show envelope".into());
    }
    let slots: Vec<WorkSlot> = serde_json::from_value(
        packet
            .get("work_slots")
            .cloned()
            .ok_or("missing work_slots")?,
    )
    .map_err(|error| error.to_string())?;
    let slot = slots
        .iter()
        .find(|slot| slot.id.as_str() == slot_id)
        .ok_or_else(|| format!("unknown commission slot `{slot_id}`"))?;
    let mut records: Vec<ContextRecord> =
        serde_json::from_value(packet.get("context").cloned().ok_or("missing context")?)
            .map_err(|error| error.to_string())?;
    records.retain(|record| slot.stdin_context_kinds.contains(&record.kind));
    let full_context_records = records.clone();
    let root = packet
        .get("artifact_root")
        .or_else(|| {
            packet
                .get("initial_input")
                .and_then(|value| value.get("artifact_root"))
        })
        .and_then(Value::as_str)
        .ok_or("missing artifact_root")?;
    let (revision, sources) = read_review_sources(Path::new(root), subject.unwrap())?;
    if subject == Some("intent.json") {
        let material_later_revision = records.iter().any(|record| {
            matches!(record.kind.as_str(), "review-evidence" | "finding-ledger")
                && matches!(
                    record.data.get("gate").and_then(Value::as_str),
                    Some("intent-review" | "intent-adversarial-review")
                )
                && record.data.get("subject").and_then(Value::as_str) == Some("intent.json")
                && record.data.get("subject_revision").and_then(Value::as_str)
                    != Some(revision.as_str())
        });
        let owner_records: Vec<_> = records.iter().collect();
        let active_baseline = active_intent_baseline(&owner_records);
        let prior_revision = records
            .iter()
            .rev()
            .find(|record| {
                (matches_review(record, "intent-review", "intent.json")
                    || matches_review(record, "intent-adversarial-review", "intent.json"))
                    && record.data.get("subject_revision").and_then(Value::as_str)
                        != Some(revision.as_str())
            })
            .and_then(|record| record.data.get("subject_revision"))
            .and_then(Value::as_str);
        let has_matching_baseline = active_baseline
            .as_ref()
            .is_some_and(|(_, baseline)| Some(baseline.revision.as_str()) == prior_revision);
        if material_later_revision && !has_matching_baseline {
            return Err(format!(
                "material intent revision has no exact owner baseline for prior revision {prior_revision:?}; supply a qualified owner user-steering intent_baseline with exact bytes/digest and original locator before commissioning (current: intent.json)"
            ));
        }
    }
    let initial = parent_gate(slot_id).is_none()
        && !has_current_aggregate(&records, slot_id, subject.unwrap(), &revision)
        && !has_material_confirmation_sources(&records, slot_id, subject.unwrap(), &revision);
    records = project_review_context(
        &records,
        slot_id,
        subject.unwrap(),
        &revision,
        requested_stage,
        Path::new(root),
    )?;
    let mut projected = input.clone();
    let context_value = serde_json::to_value(&records).map_err(|error| error.to_string())?;
    if show.is_some() {
        projected["result"]["context"] = context_value;
    } else {
        projected["context"] = context_value;
    }
    let mut output = inspect_with_stage_raw(&projected, Some(slot_id), task, None)?;
    let selected_ids = if output.get("record_ids").is_some() {
        output["record_ids"].as_array().cloned().unwrap_or_default()
    } else {
        output["commission"]["record_ids"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    if subject == Some("intent.json") {
        let mut selected: BTreeSet<String> = selected_ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        selected.extend(
            records
                .iter()
                .filter(|record| record.kind == "user-steering")
                .map(|record| record.id.as_str().to_owned()),
        );
        let ordered_ids: Vec<_> = records
            .iter()
            .filter(|record| selected.contains(record.id.as_str()))
            .map(|record| Value::String(record.id.as_str().to_owned()))
            .collect();
        if output.get("record_ids").is_some() {
            output["record_ids"] = Value::Array(ordered_ids);
        } else {
            output["commission"]["record_ids"] = Value::Array(ordered_ids.clone());
            output["context"] = json!(records
                .iter()
                .filter(|record| ordered_ids
                    .iter()
                    .any(|id| id.as_str() == Some(record.id.as_str())))
                .collect::<Vec<_>>());
        }
    }
    let final_ids = if output.get("record_ids").is_some() {
        output["record_ids"].as_array().cloned().unwrap_or_default()
    } else {
        output["commission"]["record_ids"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let selected_records: Vec<_> = records
        .iter()
        .filter(|record| {
            final_ids
                .iter()
                .any(|id| id.as_str() == Some(record.id.as_str()))
        })
        .cloned()
        .collect();
    let is_filter_packet =
        show.is_none() && packet.get("slot_id").is_some() && packet.get("work_slots").is_some();
    let budget_summary = match budgets {
        Some(budgets) => Some(check_commission_budget(
            &selected_records,
            &sources,
            budgets,
            initial,
            root,
        )?),
        None if is_filter_packet => {
            return Err("review context filter lacks required per-call token budgets".to_owned())
        }
        None => None,
    };
    if output.get("record_ids").is_some() {
        return Ok(output);
    }
    output["review_contract_version"] = json!(2);
    output["review_target"] = json!({"subject":subject,"revision":revision});
    output["review_sources"] = json!(sources.iter().map(|source|json!({"locator":source.locator,"sha256":source.sha256,"bytes":source.bytes.len()})).collect::<Vec<_>>());
    output["review_budget"] = budget_summary
        .unwrap_or_else(|| json!({"status":"inspection-only; token budget not supplied"}));
    if subject == Some("intent.json") {
        let intent = sources
            .iter()
            .find(|source| source.locator == "intent.json")
            .expect("intent source was loaded");
        let final_context: Vec<_> = selected_records.iter().collect();
        output["owner_approval_view"] = owner_approval_view(
            &intent.value,
            &intent.sha256,
            &final_context,
            &full_context_records,
        );
    }
    Ok(output)
}

fn owner_approval_view(
    intent: &Value,
    current_sha256: &str,
    context: &[&ContextRecord],
    actor_context: &[ContextRecord],
) -> Value {
    let owner_sources: Vec<_> = context
        .iter()
        .filter(|record| record.kind == "user-steering" && owner_source_targets_intent(record))
        .map(|record| {
            json!({
                "record_id":record.id.as_str(),
                "sequence":record.sequence,
                "instruction":record.data.get("instruction"),
                "supersedes":record.data.get("supersedes").cloned().unwrap_or_else(||json!([])),
                "target":record.data.get("target")
            })
        })
        .collect();
    let baseline = active_intent_baseline(context);
    let prior_revision_exists = actor_context.iter().any(|record| {
        (matches_review(record, "intent-review", "intent.json")
            || matches_review(record, "intent-adversarial-review", "intent.json"))
            && record.data.get("subject_revision").and_then(Value::as_str)
                != intent.get("revision").and_then(Value::as_str)
    });
    let exact_diff = baseline.map(|(record_id, baseline)| match validate_intent_baseline(&baseline) {
        Ok(previous) => json!({
            "status":"available",
            "source_record_id":record_id,
            "baseline_source":{"locator":baseline.locator,"sha256":baseline.sha256,"bytes":baseline.json_bytes.len()},
            "from_revision":baseline.revision,
            "to_revision":intent.get("revision"),
            "changes":json_diff(&previous, intent, "")
        }),
        Err(error) => json!({"status":"invalid-source","source_record_id":record_id,"diagnostic":error})
    }).unwrap_or_else(|| if prior_revision_exists {
        json!({
            "status":"missing-prior-intent-bytes",
            "diagnostic":"Exact wording delta is unavailable because no qualified retained prior-intent bytes were supplied; do not substitute a driver paraphrase.",
            "locator":"intent.json"
        })
    } else {
        json!({
            "status":"initial-draft-no-prior-intent",
            "diagnostic":"No earlier reviewed intent exists; compare the full draft against the retained qualified owner-source statements.",
            "owner_source_locator":"user-steering context records"
        })
    });
    let mut reviewers = BTreeSet::new();
    let mut drivers = BTreeSet::new();
    for record in actor_context {
        if matches_review(record, "intent-review", "intent.json")
            || matches_review(record, "intent-adversarial-review", "intent.json")
        {
            if let Some(author) = record.data.pointer("/author/name").and_then(Value::as_str) {
                reviewers.insert(author.to_owned());
            }
        } else if record.kind == "finding-ledger"
            && matches!(
                record.data.get("gate").and_then(Value::as_str),
                Some("intent-review" | "intent-adversarial-review")
            )
            && record.data.get("subject").and_then(Value::as_str) == Some("intent.json")
        {
            if let Some(author) = record.data.pointer("/author/name").and_then(Value::as_str) {
                drivers.insert(author.to_owned());
            }
        }
    }
    json!({
        "current_intent":{"locator":"intent.json","sha256":current_sha256,"value":intent},
        "owner_sources":owner_sources,
        "exact_substantive_wording_delta":exact_diff,
        "choice_surface":{"constraints":intent.get("constraints"),"non_goals":intent.get("non_goals"),"acceptance":intent.get("acceptance")},
        "actors":{
            "draft_author":intent.get("author"),
            "reviewers":reviewers,
            "driver":drivers,
            "owner_approval":{"kind":"human","status":"not-performed"}
        }
    })
}

fn validate_intent_baseline(baseline: &IntentBaseline) -> Result<Value, String> {
    if baseline.revision.trim().is_empty()
        || baseline.locator.trim().is_empty()
        || baseline.sha256
            != format!(
                "sha256:{:x}",
                Sha256::digest(baseline.json_bytes.as_bytes())
            )
    {
        return Err(
            "retained intent baseline locator, revision, or exact-byte digest is invalid"
                .to_owned(),
        );
    }
    let value: Value = serde_json::from_str(&baseline.json_bytes)
        .map_err(|error| format!("retained intent baseline JSON is invalid: {error}"))?;
    if value.get("revision").and_then(Value::as_str) != Some(baseline.revision.as_str()) {
        return Err("retained intent baseline revision disagrees with its exact bytes".to_owned());
    }
    Ok(value)
}

fn json_diff(old: &Value, new: &Value, pointer: &str) -> Vec<Value> {
    if old == new {
        return Vec::new();
    }
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
            let mut changes = Vec::new();
            for key in keys {
                let path = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (old.get(key), new.get(key)) {
                    (Some(before), Some(after)) => changes.extend(json_diff(before, after, &path)),
                    (Some(before), None) => {
                        changes.push(json!({"pointer":path,"change":"remove","before":before}))
                    }
                    (None, Some(after)) => {
                        changes.push(json!({"pointer":path,"change":"add","after":after}))
                    }
                    (None, None) => unreachable!(),
                }
            }
            changes
        }
        (Value::Array(old), Value::Array(new)) => {
            let mut changes = Vec::new();
            for index in 0..old.len().max(new.len()) {
                let path = format!("{pointer}/{index}");
                match (old.get(index), new.get(index)) {
                    (Some(before), Some(after)) => changes.extend(json_diff(before, after, &path)),
                    (Some(before), None) => {
                        changes.push(json!({"pointer":path,"change":"remove","before":before}))
                    }
                    (None, Some(after)) => {
                        changes.push(json!({"pointer":path,"change":"add","after":after}))
                    }
                    (None, None) => unreachable!(),
                }
            }
            changes
        }
        _ => vec![json!({"pointer":pointer,"change":"replace","before":old,"after":new})],
    }
}

pub fn run_from_stdin(args: &[String]) -> i32 {
    let mut slot = None;
    let mut task = None;
    let mut stage = None;
    let mut budgets: Option<Vec<CallBudget>> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--call-budgets" {
            if budgets.is_some() {
                eprintln!("duplicate {arg}");
                return 2;
            }
            let Some(value) = iter.next() else {
                eprintln!("missing value for {arg}");
                return 2;
            };
            budgets = match serde_json::from_str(value) {
                Ok(budgets) => Some(budgets),
                Err(error) => {
                    eprintln!("invalid per-call review budgets: {error}");
                    return 2;
                }
            };
            continue;
        }
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
        .and_then(|input| inspect_with_budgets(&input, slot, task, stage, budgets.as_deref()));
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

    fn typed_record(id: &str, kind: &str, n: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
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
    fn implementation_task_steering_does_not_repeat_exact_intent_baseline_bytes() {
        let owner = record(
            "owner",
            1,
            json!({
                "target":{"kind":"all"},"instruction":"owner source",
                "intent_baseline":{"revision":"r1","locator":"intent.json@r1","sha256":"sha256:old","json_bytes":"large exact baseline bytes"}
            }),
        );
        let snapshot = vec![owner.clone()];
        let delivered = task_steering(&snapshot, &json!({"revision":"r1"}), "task-a").unwrap();
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].data.get("intent_baseline").is_none());
        assert!(snapshot[0].data.get("intent_baseline").is_some());
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

    #[test]
    fn initial_high_aggregate_removes_both_individuals_and_linked_ledger_sources() {
        let individual_a = typed_record(
            "individual-a",
            "review-evidence",
            3,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "review_stage":"individual","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"fail","findings":"A private first-stage finding"
            }),
        );
        let individual_b = typed_record(
            "individual-b",
            "review-evidence",
            4,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "review_stage":"individual","policy_id":"axis","author":{"name":"B","kind":"agent"},
                "result":"fail","findings":"B private first-stage finding"
            }),
        );
        let applicability = typed_record(
            "carry-a",
            "evidence-applicability",
            5,
            json!({
                "origin":{"kind":"context-record","id":"individual-a"},
                "target":{"subject":"design.json","revision":"2"},
                "reason":"unaffected"
            }),
        );
        let ledger = typed_record(
            "ledger",
            "finding-ledger",
            6,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "findings":[
                    {"source":{"kind":"context-record","id":"individual-a"},"statement":"A private first-stage finding"},
                    {"source":{"kind":"context-record","id":"individual-b"},"statement":"B private first-stage finding"}
                ]
            }),
        );
        let unrelated = typed_record(
            "unrelated",
            "review-evidence",
            7,
            json!({
                "gate":"plan-review","subject":"plan.json","subject_revision":"1",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"C","kind":"agent"},
                "result":"fail","findings":"unrelated old history"
            }),
        );
        let incorporation = typed_record(
            "driver-note",
            "steering-incorporation",
            8,
            json!({"applied":"driver paraphrase"}),
        );
        let projected = project_review_context(
            &[
                individual_a,
                individual_b,
                applicability,
                ledger,
                unrelated,
                incorporation,
            ],
            "design-review",
            "design.json",
            "2",
            Some("aggregate"),
            Path::new("/unused-for-design"),
        )
        .unwrap();
        let ids: BTreeSet<_> = projected.iter().map(|record| record.id.as_str()).collect();
        for hidden in [
            "owner-old",
            "owner-new",
            "individual-a",
            "individual-b",
            "carry-a",
            "ledger",
            "unrelated",
            "driver-note",
        ] {
            assert!(!ids.contains(hidden), "leaked {hidden}: {projected:?}");
        }
        let delivered = serde_json::to_string(&projected).unwrap();
        assert!(!delivered.contains("private first-stage finding"));
    }

    #[test]
    fn confirmation_keeps_current_accepted_finding_and_delivered_applicability() {
        let prior_aggregate = typed_record(
            "aggregate-old",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"1",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":""
            }),
        );
        let source = typed_record(
            "individual-fail",
            "review-evidence",
            2,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"1",
                "review_stage":"individual","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"fail","findings":"retain this accepted failure"
            }),
        );
        let ledger = typed_record(
            "current-ledger",
            "finding-ledger",
            3,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "findings":[{"source":{"kind":"context-record","id":"individual-fail"},"statement":"retain this accepted failure","disposition":"accepted","status":"unresolved"}]
            }),
        );
        let applicability = typed_record(
            "current-carry",
            "evidence-applicability",
            4,
            json!({
                "origin":{"kind":"context-record","id":"individual-fail"},
                "target":{"subject":"design.json","revision":"2"},"reason":"driver inspected current design"
            }),
        );
        let projected = project_review_context(
            &[prior_aggregate, source, ledger, applicability],
            "design-review",
            "design.json",
            "2",
            Some("aggregate"),
            Path::new("/unused-for-design"),
        )
        .unwrap();
        assert!(projected
            .iter()
            .any(|record| record.id.as_str() == "current-ledger"));
        assert!(projected
            .iter()
            .any(|record| record.id.as_str() == "individual-fail"));
        assert!(projected
            .iter()
            .any(|record| record.id.as_str() == "current-carry"));
        let ledger = projected
            .iter()
            .find(|record| record.id.as_str() == "current-ledger")
            .unwrap();
        assert_eq!(
            ledger.data["findings"][0]["statement"],
            "retain this accepted failure"
        );
    }

    #[test]
    fn same_revision_confirmation_keeps_findings_after_an_earlier_aggregate() {
        let earlier_aggregate = typed_record(
            "aggregate-current",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":""
            }),
        );
        let individual = typed_record(
            "individual-current",
            "review-evidence",
            2,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "review_stage":"individual","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"fail","findings":"current accepted finding"
            }),
        );
        let ledger = typed_record(
            "ledger-current",
            "finding-ledger",
            3,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "findings":[{"source":{"kind":"context-record","id":"individual-current"},"statement":"current accepted finding","disposition":"accepted","status":"unresolved"}]
            }),
        );
        let applicability = typed_record(
            "applicability-current",
            "evidence-applicability",
            4,
            json!({
                "origin":{"kind":"context-record","id":"individual-current"},
                "target":{"subject":"design.json","revision":"2"},"reason":"confirmed current"
            }),
        );
        let projected = project_review_context(
            &[earlier_aggregate, individual, ledger, applicability],
            "design-review",
            "design.json",
            "2",
            None,
            Path::new("/unused-for-design"),
        )
        .unwrap();
        let ids: BTreeSet<_> = projected.iter().map(|record| record.id.as_str()).collect();
        assert!(ids.contains("individual-current"));
        assert!(ids.contains("ledger-current"));
        assert!(ids.contains("applicability-current"));
    }

    #[test]
    fn confirmation_refuses_missing_mandatory_finding_source() {
        let prior_aggregate = typed_record(
            "aggregate-old",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"1",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":""
            }),
        );
        let ledger = typed_record(
            "current-ledger",
            "finding-ledger",
            2,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"2",
                "findings":[{"source":{"kind":"context-record","id":"missing-source"},"statement":"mandatory confirmation finding","disposition":"accepted","status":"unresolved"}]
            }),
        );
        let error = project_review_context(
            &[prior_aggregate, ledger],
            "design-review",
            "design.json",
            "2",
            None,
            Path::new("/unused-for-design"),
        )
        .unwrap_err();
        assert!(error.contains("mandatory confirmation finding source `missing-source` is missing"));
    }

    #[test]
    fn challenge_projection_contains_actual_parent_aggregate_grounds() {
        let root = std::env::temp_dir().join(format!(
            "software-change-challenge-ground-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("intent.json"), br#"{"revision":"1"}"#).unwrap();
        let parent_source = br#"{"revision":"4"}"#;
        std::fs::write(root.join("design.json"), parent_source).unwrap();
        let parent_digest = format!("sha256:{:x}", Sha256::digest(parent_source));
        let parent = typed_record(
            "parent-grounded",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"4",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":"","review_contract_version":2,
                "grounds":{"reason":"parent inspected the accepted constraint","evidence":[{"locator":"design.json#/revision","sha256":parent_digest}]}
            }),
        );
        let old_parent_individual = typed_record(
            "parent-individual",
            "review-evidence",
            2,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"4",
                "review_stage":"individual","policy_id":"axis","author":{"name":"B","kind":"agent"},
                "result":"fail","findings":"must not replace parent aggregate grounds"
            }),
        );
        let own_individual = typed_record(
            "challenge-individual",
            "review-evidence",
            3,
            json!({
                "gate":"design-adversarial-review","subject":"design.json","subject_revision":"4",
                "review_stage":"individual","policy_id":"axis","author":{"name":"B","kind":"agent"},
                "result":"fail","findings":"must not leak to first challenge aggregate"
            }),
        );
        let parent_ledger = typed_record(
            "parent-ledger",
            "finding-ledger",
            4,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"4",
                "findings":[{"source":{"kind":"context-record","id":"parent-individual"},"statement":"must not leak through ledger"}]
            }),
        );
        let projected = project_review_context(
            &[parent, old_parent_individual, own_individual, parent_ledger],
            "design-adversarial-review",
            "design.json",
            "4",
            None,
            &root,
        )
        .unwrap();
        let delivered: Vec<_> = projected.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(delivered, ["parent-grounded"]);
        assert!(!serde_json::to_string(&projected)
            .unwrap()
            .contains("must not leak through ledger"));
        assert_eq!(
            projected[0].data["grounds"]["reason"],
            "parent inspected the accepted constraint"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn challenge_refuses_absent_parent_aggregate_ground() {
        let parent = typed_record(
            "parent-without-ground",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"4",
                "review_stage":"aggregate","policy_id":"axis","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":""
            }),
        );
        let error = project_review_context(
            &[parent],
            "design-adversarial-review",
            "design.json",
            "4",
            None,
            Path::new("/unused-for-design"),
        )
        .unwrap_err();
        assert!(error.contains("version-2 grounds"));
    }

    #[test]
    fn intent_approval_delta_uses_exact_qualified_baseline_and_preserves_actor_split() {
        let baseline = json!({"revision":"r1","author":{"name":"drafter","kind":"agent"},"outcome":"retain qualification"});
        let baseline_bytes = serde_json::to_string(&baseline).unwrap();
        let owner = record(
            "owner-source",
            1,
            json!({
                "target":{"kind":"all"},"instruction":"retain the precise owner qualification",
                "intent_baseline":{"revision":"r1","locator":"intent.json at accepted r1","sha256":format!("sha256:{:x}",Sha256::digest(baseline_bytes.as_bytes())),"json_bytes":baseline_bytes}
            }),
        );
        let intent = json!({"revision":"r2","author":{"name":"draft-author","kind":"agent"},"outcome":"qualification removed"});
        let sha = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&intent).unwrap())
        );
        let reviewer = typed_record(
            "reviewer-source",
            "review-evidence",
            2,
            json!({
                "gate":"intent-review","subject":"intent.json","subject_revision":"r1",
                "author":{"name":"independent-reviewer","kind":"agent"}
            }),
        );
        let driver = typed_record(
            "driver-ledger",
            "finding-ledger",
            3,
            json!({
                "gate":"intent-review","subject":"intent.json","subject_revision":"r1",
                "author":{"name":"coordinating-driver","kind":"human"}
            }),
        );
        let view =
            owner_approval_view(&intent, &sha, &[&owner], &[owner.clone(), reviewer, driver]);
        assert_eq!(view["current_intent"]["value"], intent);
        assert_eq!(
            view["exact_substantive_wording_delta"]["status"],
            "available"
        );
        assert!(view["exact_substantive_wording_delta"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["pointer"] == "/outcome"));
        assert_eq!(view["actors"]["draft_author"]["name"], "draft-author");
        assert!(view["actors"]["reviewers"]
            .as_array()
            .unwrap()
            .contains(&json!("independent-reviewer")));
        assert!(view["actors"]["driver"]
            .as_array()
            .unwrap()
            .contains(&json!("coordinating-driver")));
        assert_eq!(view["actors"]["owner_approval"]["status"], "not-performed");
        assert_eq!(
            view["owner_sources"][0]["instruction"],
            "retain the precise owner qualification"
        );
    }

    #[test]
    fn budget_refuses_missing_sources_instead_of_truncating_or_exceeding_model_window() {
        let source_bytes = vec![b'x'; 9000];
        let sources = vec![ReviewSource {
            locator: "design.json".to_owned(),
            sha256: "sha256:fixture".to_owned(),
            bytes: source_bytes,
            value: json!({}),
        }];
        let budget = CallBudget {
            author: "reviewer".to_owned(),
            model_id: "small/model".to_owned(),
            context_window_tokens: 10000,
            system_tokens: 1000,
            framing_tokens: 1000,
            output_reserve_tokens: 1000,
            reasoning_reserve_tokens: 1000,
        };
        let error =
            check_commission_budget(&[], &sources, &[budget], true, "/fixture/root").unwrap_err();
        assert!(error.contains("small/model"));
        assert!(error.contains("design.json"));
        assert!(error.contains("inspect: design.json"));
    }

    #[test]
    fn initial_byte_bound_counts_mandatory_originals_but_later_stage_can_use_total_bound() {
        let source_bytes = vec![b'x'; INITIAL_CONTEXT_BYTES + 1];
        let sources = vec![ReviewSource {
            locator: "intent.json".to_owned(),
            sha256: "sha256:fixture".to_owned(),
            bytes: source_bytes,
            value: json!({}),
        }];
        let budget = CallBudget {
            author: "reviewer".to_owned(),
            model_id: "measured/model".to_owned(),
            context_window_tokens: 128 * 1024,
            system_tokens: 4000,
            framing_tokens: 4000,
            output_reserve_tokens: 4000,
            reasoning_reserve_tokens: 16000,
        };
        let initial_error = check_commission_budget(
            &[],
            &sources,
            std::slice::from_ref(&budget),
            true,
            "/fixture/root",
        )
        .unwrap_err();
        assert!(initial_error.contains("initial_limit=32768"));
        assert!(initial_error.contains("intent.json"));

        let later =
            check_commission_budget(&[], &sources, &[budget], false, "/fixture/root").unwrap();
        assert_eq!(later["initial_evidence_bytes"], Value::Null);
        let location_bytes =
            serde_json::to_vec(&json!({"artifact_root":"/fixture/root","context":[]}))
                .unwrap()
                .len();
        assert_eq!(
            later["total_evidence_bytes"],
            INITIAL_CONTEXT_BYTES + 1 + location_bytes
        );
        assert_eq!(later["calls"][0]["available_input_tokens"], 103_072);
    }
}
