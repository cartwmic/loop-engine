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

#[doc(hidden)]
pub fn validation_command(data: &Value) -> Result<ProofCommand, String> {
    if data.get("spec").is_some() {
        serde_json::from_value::<ValidationCommandEnvelope>(data.clone())
            .map(|envelope| envelope.spec)
            .map_err(|error| error.to_string())
    } else {
        serde_json::from_value::<ProofCommand>(data.clone()).map_err(|error| error.to_string())
    }
}

/// Shared driver/reviewer guidance for artifact and narrative challenge slots.
pub const CHALLENGE_GUIDANCE: &str = "Independently test whether the ordinary review's passing or sufficiency conclusion holds: assume it did not pass; why might it not have? Use ordinary findings and their dispositions as context, not as individual findings to rebut. Normal entry into the challenge state proves ordinary review passed; only its current-revision finding ledger is checked, not ordinary evidence again. The challenge outcome is independent of the append order of ordinary individual, aggregate and carried records. Separately, a challenge assessment counts only if first recorded after the latest relevant ordinary-review record; an earlier assessment, including a raw re-append, requires explicit evidence-applicability appended after that cutoff. Ordinary-review writes are refused during the challenge; use the existing check-free revise route for correction and renewed ordinary approval. Owner override remains exceptional and does not prove ordinary approval.";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

fn ids_valid(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| !id.trim().is_empty())
        && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

/// Intrinsic steering parser shared by commission selection and the rule table.
/// A stale task-targeted record stays historical/inert as before.
pub(crate) fn parse_steering(
    data: &Value,
    earlier: &[ContextRecord],
    slots: &[WorkSlot],
    plan: Option<&Value>,
) -> Result<Steering, String> {
    let steering: Steering = serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
    if steering.instruction.trim().is_empty() {
        return Err("steering requires instruction".into());
    }
    if let Some(baseline) = &steering.intent_baseline {
        validate_intent_baseline(baseline)?;
    }
    if !steering.supersedes.is_empty() && !ids_valid(&steering.supersedes) {
        return Err("steering has invalid supersedes IDs".into());
    }
    for id in &steering.supersedes {
        if !earlier
            .iter()
            .any(|record| record.kind == STEERING_KIND && record.id.as_str() == id)
        {
            return Err(format!(
                "steering supersedes unknown or non-earlier steering `{id}`"
            ));
        }
    }
    let revision = plan.and_then(|p| p["revision"].as_str());
    match &steering.target {
        Target::All => {}
        Target::Slots { ids } => {
            if !ids_valid(ids)
                || ids
                    .iter()
                    .any(|id| !slots.iter().any(|slot| slot.id.as_str() == id))
            {
                return Err("steering has invalid slot targets".into());
            }
        }
        Target::Tasks { plan_revision, ids } => {
            if !ids_valid(ids) || plan_revision.trim().is_empty() {
                return Err("steering has invalid task target".into());
            }
            if revision == Some(plan_revision.as_str()) {
                let tasks: BTreeSet<_> = plan
                    .and_then(|p| p["tasks"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|task| task["id"].as_str())
                    .collect();
                if ids.iter().any(|id| !tasks.contains(id.as_str())) {
                    return Err("steering has unknown task target".into());
                }
            }
        }
    }
    let stale_task = matches!(&steering.target, Target::Tasks { plan_revision, .. } if revision != Some(plan_revision.as_str()));
    if !stale_task {
        for update in &steering.proof_updates {
            validate_update(update, plan)?;
        }
    }
    Ok(steering)
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
        if slot_id != "implement" || !tasks.contains(task) {
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
    for (index, record) in records.iter().enumerate() {
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
        let steering = parse_steering(&record.data, &records[..index], slots, plan)
            .map_err(|e| format!("steering `{}`: {e}", record.id))?;
        superseded.extend(steering.supersedes.iter().cloned());
        let applies = match &steering.target {
            Target::All => true,
            Target::Slots { ids } => ids.iter().any(|id| id == slot_id),
            Target::Tasks { plan_revision, ids } => {
                if revision != Some(plan_revision.as_str()) {
                    diagnostics.push(format!("steering `{}` stale task revision `{plan_revision}` (current {revision:?}); not applied", record.id));
                    false
                } else {
                    slot_id == "implement"
                        && task_id.is_none_or(|task| ids.iter().any(|id| id == task))
                }
            }
        };
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
    let withdrawn = crate::recovery::withdrawn_record_ids(records, None)?;
    for record in records
        .iter()
        .filter(|r| r.kind == "validation-command" && !withdrawn.contains(r.id.as_str()))
    {
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
        guidance: slot_id
            .ends_with("-adversarial-review")
            .then(|| CHALLENGE_GUIDANCE.to_owned()),
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
    // Apply recovery while the original history is still available, before
    // frozen stdin-kind/stage selection can discard withdrawal or roster facts.
    let artifact_root = packet
        .get("artifact_root")
        .or_else(|| packet["initial_input"].get("artifact_root"));
    let exclusions = crate::recovery::excluded_record_ids(
        &records,
        slot_id,
        review_subject(slot_id).unwrap_or("validation-report.json"),
        artifact_root,
    )?;
    let stale_exclusions = crate::recovery::stale_exclusions(&records);
    records.retain(|record| !exclusions.contains(record.id.as_str()));
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
    selected.diagnostics.extend(
        stale_exclusions
            .values()
            .filter(|cause| slot.stdin_context_kinds.contains(&cause.kind))
            .map(crate::recovery::StaleExclusion::message),
    );
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

fn narrative_options(slot: &str) -> Option<&'static crate::workflow::PhaseOptions> {
    crate::workflow::PHASE_OPTIONS.iter().find(|options| {
        options
            .narrative
            .is_some_and(|n| [n.review, n.challenge].contains(&slot))
    })
}

fn checkpoint_options(slot: &str) -> Option<&'static crate::workflow::PhaseOptions> {
    crate::workflow::PHASE_OPTIONS
        .iter()
        .find(|options| options.checkpoint == slot)
}

// Read only fixed provider filenames, and refuse links escaping the artifact root.
// Markdown is a real UTF-8 source, not a JSON subject or a fabricated record.
fn read_option_source(root: &Path, locator: &str, is_json: bool) -> Result<ReviewSource, String> {
    let path = root.join(locator);
    let unavailable = |error: &dyn std::fmt::Display| {
        format!(
            "mandatory review source `{locator}` is unavailable at {}: {error}",
            path.display()
        )
    };
    let canonical_root = root.canonicalize().map_err(|e| unavailable(&e))?;
    let canonical_path = path.canonicalize().map_err(|e| unavailable(&e))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "mandatory review source `{locator}` escapes artifact_root"
        ));
    }
    let bytes = std::fs::read(&canonical_path).map_err(|e| unavailable(&e))?;
    let value = if is_json {
        serde_json::from_slice(&bytes)
            .map_err(|e| format!("mandatory review source `{locator}` is not valid JSON: {e}"))?
    } else {
        std::str::from_utf8(&bytes).map_err(|e| {
            format!("mandatory review source `{locator}` is not readable UTF-8: {e}")
        })?;
        Value::Null
    };
    Ok(ReviewSource {
        locator: locator.to_owned(),
        sha256: format!("sha256:{:x}", Sha256::digest(&bytes)),
        bytes,
        value,
    })
}

fn source_projection(root: &Path, source: &ReviewSource) -> Value {
    json!({"locator":source.locator,"path":root.join(&source.locator),
        "sha256":source.sha256,"bytes":source.bytes.len()})
}

fn phase_identity(context: &[ContextRecord], kind: &str, phase: &str, subject: &str) -> Value {
    crate::narratives::latest_phase_record(context, kind, phase, subject)
        .map(|(record, fact)| {
            json!({"record_id":record.id.as_str(),"record_kind":record.kind,
            "sequence":record.sequence,"record":record,"fact_kind":kind,"fact":fact})
        })
        .unwrap_or(Value::Null)
}

struct PhaseCommission {
    phase: &'static str,
    subject: &'static str,
    identity: Value,
    approval: Value,
    sources: Vec<ReviewSource>,
    missing: Vec<String>,
}

fn phase_commission(
    packet: &Value,
    slot: &str,
    root: &Path,
    options: &'static crate::workflow::PhaseOptions,
) -> Result<PhaseCommission, String> {
    use crate::workflow::{
        self, APPROVED_ARTIFACT_KIND, ARTIFACT_ADMITTED_KIND, NARRATIVE_ADMITTED_KIND,
    };
    let phase = options.phase.name;
    let narrative_enabled =
        workflow::option_enabled(Some(&packet["initial_input"]), "narratives", phase);
    let narrative = options.narrative.filter(|_| narrative_enabled);
    let subject = workflow::checkpoint_subject(options, narrative_enabled);
    let context: Vec<ContextRecord> =
        serde_json::from_value(packet["context"].clone()).map_err(|e| e.to_string())?;
    let approval = phase_identity(
        &context,
        APPROVED_ARTIFACT_KIND,
        phase,
        options.phase.subject,
    );
    let identity = if narrative_enabled {
        phase_identity(&context, NARRATIVE_ADMITTED_KIND, phase, subject)
    } else {
        approval.clone()
    };
    let mut missing = Vec::new();
    if narrative_options(slot).is_some() && !narrative_enabled {
        missing.push(format!(
            "narrative phase `{phase}` is not enabled in frozen input"
        ));
    }
    if checkpoint_options(slot).is_some()
        && !workflow::option_enabled(Some(&packet["initial_input"]), "checkpoints", phase)
    {
        missing.push(format!(
            "checkpoint phase `{phase}` is not enabled in frozen input"
        ));
    }
    let mut locators = vec!["intent.json", options.phase.subject];
    if let Some(n) = narrative {
        locators.extend([n.subject, n.markdown]);
    }
    locators.dedup();
    let mut sources = Vec::new();
    for locator in locators {
        match read_option_source(root, locator, locator.ends_with(".json")) {
            Ok(source) => sources.push(source),
            Err(error) => missing.push(error),
        }
    }
    for (kind, projection) in [
        (
            ARTIFACT_ADMITTED_KIND,
            phase_identity(
                &context,
                ARTIFACT_ADMITTED_KIND,
                phase,
                options.phase.subject,
            ),
        ),
        (APPROVED_ARTIFACT_KIND, approval.clone()),
    ] {
        if projection.is_null() {
            missing.push(format!("missing latest {kind} for `{phase}`"));
        } else if let Some(artifact) = sources.iter().find(|s| s.locator == options.phase.subject) {
            let fact = &projection["fact"];
            if fact["revision"]
                .as_str()
                .is_none_or(|r| r.trim().is_empty())
                || fact["revision"] != artifact.value["revision"]
                || fact["sha256"].as_str() != artifact.sha256.strip_prefix("sha256:")
            {
                missing.push(format!(
                    "current artifact must match latest {kind} revision and sha256"
                ));
            }
        }
    }
    // Nested boundary facts share one real identity and must agree with each other.
    if let Some((record, fact)) = crate::narratives::latest_phase_record(
        &context,
        APPROVED_ARTIFACT_KIND,
        phase,
        options.phase.subject,
    ) {
        if let Some(admission) =
            crate::narratives::phase_record_fact(record, ARTIFACT_ADMITTED_KIND)
        {
            if admission["phase"] != phase
                || ["subject", "revision", "sha256"]
                    .iter()
                    .any(|key| admission[key] != fact[key])
            {
                missing.push("nested artifact admission and approval facts disagree".to_owned());
            }
        }
    }
    if let Some(n) = narrative {
        if identity.is_null() {
            missing.push(format!("missing latest narrative admission for `{phase}`"));
        }
        if let Some(source) = sources.iter().find(|s| s.locator == n.subject) {
            match crate::narratives::validate_binding(&source.value) {
                Err(violations) => missing.extend(
                    violations
                        .iter()
                        .map(|v| format!("narrative binding {}: {}", v.path, v.message)),
                ),
                Ok(binding) => {
                    let fact = &approval["fact"];
                    if binding.artifact.subject != options.phase.subject
                        || binding.markdown.file != n.markdown
                        || Some(binding.approved_artifact_record_id)
                            != approval["record_id"].as_str()
                        || Some(binding.artifact.revision) != fact["revision"].as_str()
                        || Some(binding.artifact.sha256) != fact["sha256"].as_str()
                    {
                        missing.push("binding must cite latest approved artifact identity and exact phase files".to_owned());
                    }
                    if Some(binding.revision) != identity["fact"]["revision"].as_str() {
                        missing.push(
                            "binding must match latest admitted narrative revision".to_owned(),
                        );
                    }
                    if let Some(markdown) = sources.iter().find(|s| s.locator == n.markdown) {
                        let digest = markdown.sha256.strip_prefix("sha256:");
                        if Some(binding.markdown.sha256) != digest
                            || identity["fact"]["sha256"].as_str() != digest
                        {
                            missing.push(
                                "markdown must match binding and latest narrative admission sha256"
                                    .to_owned(),
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(PhaseCommission {
        phase,
        subject,
        identity,
        approval,
        sources,
        missing,
    })
}

/// Optional phase packets use the same review selector; the additional view is
/// descriptive only. It never records or attests to an owner's approval.
pub fn inspect_with_stage(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
) -> Result<Value, String> {
    inspect_with_stage_excluding(
        input,
        requested_slot,
        task,
        requested_stage,
        &BTreeSet::new(),
        &[],
    )
}

/// Shared classification after caller-supplied exclusions and before durable
/// selection is exposed. Computing valid withdrawals/retirements is A-owned.
pub fn inspect_with_stage_excluding(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
    excluded_ids: &BTreeSet<String>,
    extensions: &[crate::admission::Rule],
) -> Result<Value, String> {
    let original = input.get("result").unwrap_or(input);
    let classify = original["initial_input"]["contract_version"]
        .as_u64()
        .is_some_and(|v| v >= 3);
    let records: Vec<ContextRecord> = if classify {
        serde_json::from_value(original["context"].clone()).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let workflow: Option<loop_core::Workflow> = if classify {
        Some(if let Some(graph) = original.get("workflow_graph") {
            serde_json::from_value(graph.clone()).map_err(|e| e.to_string())?
        } else {
            crate::workflow::describe_workflow(Some(&original["initial_input"]))?
        })
    } else {
        None
    };
    let ctx = workflow
        .as_ref()
        .map(|workflow| {
            crate::admission::AdmissionContext::new(
                &original["initial_input"],
                &records,
                original["current_state"].as_str().unwrap_or(""),
                original["state_visit"].as_u64().unwrap_or(0),
                workflow,
            )
        })
        .transpose()?;
    let admission = ctx.as_ref().map(|ctx| (ctx, extensions));
    // Never erase originals before immutable ledger/correction/source lookup.
    let mut output = inspect_with_stage_inner(
        input,
        requested_slot,
        task,
        requested_stage,
        excluded_ids,
        admission,
    )?;
    if let Some(ctx) = &ctx {
        let selected: BTreeSet<String> = output["context"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| record["id"].as_str().map(str::to_owned))
            .collect();
        let mut diagnostics = output["record_diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (record, verdict) in
            crate::admission::classify_selected(ctx, excluded_ids, Some(&selected), extensions)
        {
            if let Err(refusal) = verdict {
                if !diagnostics.iter().any(|diagnostic| {
                    diagnostic["record_id"] == record.id.as_str()
                        && diagnostic["rule"] == refusal.rule
                }) {
                    diagnostics.push(json!({
                        "record_id":record.id,"rule":refusal.rule,"reason":refusal.message
                    }));
                }
            }
        }
        if !diagnostics.is_empty() {
            output["record_diagnostics"] = json!(diagnostics);
        }
    }
    Ok(output)
}

fn inspect_with_stage_inner(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
    excluded_ids: &BTreeSet<String>,
    admission: Option<(
        &crate::admission::AdmissionContext<'_>,
        &[crate::admission::Rule],
    )>,
) -> Result<Value, String> {
    let mut effective_input = input.clone();
    if let Some(packet) = effective_input
        .get_mut("result")
        .filter(|packet| packet.get("initial_input").is_some())
    {
        let context: Vec<ContextRecord> = serde_json::from_value(packet["context"].clone())
            .map_err(|e| format!("invalid full context: {e}"))?;
        let resolution = crate::admission::effective_policy(&packet["initial_input"], &context);
        let gate = requested_slot
            .or_else(|| packet["slot_id"].as_str())
            .unwrap_or("");
        if resolution.blocked.iter().any(|key| {
            key.starts_with(&format!("review.{gate}."))
                || (gate.starts_with("validation") && matches!(key.as_str(), "criterion" | "goal"))
        }) {
            return Err("invalid author-count amendment blocks commission".into());
        }
        packet["initial_input"] = resolution.effective;
    }
    let input = &effective_input;
    let packet = input.get("result").unwrap_or(input);
    let slot = requested_slot.or_else(|| packet["slot_id"].as_str());
    let options = slot.and_then(|s| narrative_options(s).or_else(|| checkpoint_options(s)));
    let Some(options) = options.filter(|_| input.get("result").is_some()) else {
        return inspect_review_with_stage_excluding(
            input,
            requested_slot,
            task,
            requested_stage,
            excluded_ids,
            admission,
        );
    };
    if requested_stage.is_some_and(|stage| !matches!(stage, "individual" | "aggregate")) {
        return Err("commission review stage must be `individual` or `aggregate`".to_owned());
    }
    let slot = slot.expect("option slot");
    let root = packet
        .get("artifact_root")
        .or_else(|| packet["initial_input"].get("artifact_root"))
        .and_then(Value::as_str)
        .ok_or("missing artifact_root")?;
    let root = Path::new(root);
    let phase = phase_commission(packet, slot, root, options)?;
    let mut output = if phase.missing.is_empty() {
        inspect_review_with_stage_excluding(
            input,
            Some(slot),
            task,
            requested_stage,
            excluded_ids,
            admission,
        )?
    } else {
        inspect_with_stage_raw(input, Some(slot), task, requested_stage)?
    };
    let mut missing = output["missing_inputs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    missing.extend(phase.missing.iter().map(|s| json!(s)));
    output["missing_inputs"] = json!(missing);
    output["artifact_root"] = json!(root);
    output["commission"]["review_stage"] = json!(requested_stage);
    output["phase"] = json!(phase.phase);
    output["approved_artifact"] = phase.approval;
    let sources = json!(phase
        .sources
        .iter()
        .map(|s| source_projection(root, s))
        .collect::<Vec<_>>());
    if checkpoint_options(slot).is_some() {
        let subject_source = phase
            .sources
            .iter()
            .find(|s| s.locator == phase.subject)
            .map(|s| source_projection(root, s));
        let narrative_source = options
            .narrative
            .and_then(|n| phase.sources.iter().find(|s| s.locator == n.markdown))
            .map(|s| source_projection(root, s));
        let bindings_context: Vec<ContextRecord> =
            serde_json::from_value(packet["context"].clone()).map_err(|error| error.to_string())?;
        output["owner_approval_view"] = json!({
            "model_bindings":loop_core::model_bindings_view(&packet["initial_input"], &bindings_context),
            "checkpoint":slot,"phase":phase.phase,"subject":phase.subject,
            "subject_revision":phase.identity["fact"]["revision"],
            "subject_record_id":phase.identity["record_id"],"subject_identity":phase.identity,
            "subject_source":subject_source,"narrative_source":narrative_source,"sources":sources,
            "owner_approval":{"status":"not-performed"},
            "decision_guidance":"Ask the owner to approve or reject this exact subject revision. Append an owner-decision context record with checkpoint, subject, subject_revision, decision (approve or reject), actor and the owner's nonempty verbatim text. A rejection must name target artifact or, when a narrative is enabled, narrative, and a nonempty reason. Artifact rejection uses revise; narrative rejection uses revise-narrative. This view is not an owner approval; do not paraphrase the owner's text or invent an approval command."
        });
        if phase.phase == "intent" {
            if let Some(intent_source) = phase
                .sources
                .iter()
                .find(|source| source.locator == "intent.json")
            {
                let context: Vec<ContextRecord> = serde_json::from_value(packet["context"].clone())
                    .map_err(|error| error.to_string())?;
                let references: Vec<_> = context.iter().collect();
                output["owner_approval_view"]["intent_revision_view"] = owner_approval_view(
                    &intent_source.value,
                    &intent_source.sha256,
                    &references,
                    &context,
                    &packet["initial_input"],
                );
            }
        }
    } else {
        output["review_contract_version"] = json!(3);
        output["review_target"] = json!({"phase":phase.phase,"subject":phase.subject,
            "revision":phase.sources.iter().find(|s| s.locator == phase.subject)
                .and_then(|s| s.value["revision"].as_str()),"admitted_identity":phase.identity});
        output["review_sources"] = sources;
    }
    Ok(output)
}

#[cfg(test)]
fn inspect_review_with_stage(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
) -> Result<Value, String> {
    inspect_review_with_stage_excluding(
        input,
        requested_slot,
        task,
        requested_stage,
        &BTreeSet::new(),
        None,
    )
}

fn inspect_review_with_stage_excluding(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
    excluded_ids: &BTreeSet<String>,
    admission: Option<(
        &crate::admission::AdmissionContext<'_>,
        &[crate::admission::Rule],
    )>,
) -> Result<Value, String> {
    match inspect_with_budgets(
        input,
        requested_slot,
        task,
        requested_stage,
        excluded_ids,
        admission,
    ) {
        Ok(mut output) => {
            let mut missing = output["missing_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            missing.extend(
                missing_parent_claims(input, requested_slot)?
                    .into_iter()
                    .map(Value::String),
            );
            output["missing_inputs"] = json!(missing);
            Ok(output)
        }
        Err(error)
            if requested_slot.is_some()
                && (error.starts_with("mandatory review source")
                    && error.contains("is unavailable")
                    || error.contains("missing current parent aggregate grounds")) =>
        {
            let mut output = inspect_with_stage_raw(input, requested_slot, task, requested_stage)?;
            let mut missing = missing_parent_claims(input, requested_slot).unwrap_or_default();
            if missing.is_empty() || !error.contains("missing current parent aggregate grounds") {
                missing.push(error);
            }
            output["missing_inputs"] = json!(missing);
            Ok(output)
        }
        Err(error) => Err(error),
    }
}

fn missing_parent_claims(
    input: &Value,
    requested_slot: Option<&str>,
) -> Result<Vec<String>, String> {
    let packet = input.get("result").unwrap_or(input);
    let gate = requested_slot
        .or_else(|| packet.get("slot_id").and_then(Value::as_str))
        .ok_or("commission requires --slot SLOT for show inspection")?;
    let Some(parent) = parent_gate(gate) else {
        return Ok(Vec::new());
    };
    let Some(axes) = packet["initial_input"]["review_policies"][gate].as_array() else {
        return Ok(Vec::new());
    };
    let subject = review_subject(gate).ok_or("challenge has no review subject")?;
    let root = packet
        .get("artifact_root")
        .or_else(|| packet["initial_input"].get("artifact_root"))
        .and_then(Value::as_str)
        .ok_or("missing artifact_root")?;
    let (revision, sources) = read_review_sources(Path::new(root), subject)?;
    let records: Vec<ContextRecord> =
        serde_json::from_value(packet["context"].clone()).map_err(|error| error.to_string())?;
    let root_value = Value::String(root.to_owned());
    let excluded =
        crate::recovery::excluded_record_ids(&records, parent, subject, Some(&root_value))?;
    let available_records: Vec<_> = records
        .iter()
        .filter(|row| {
            !excluded.contains(row.id.as_str())
                && crate::evidence::parse_evidence_record_for_stage_with_context(
                    &row.data,
                    subject,
                    Some(&root_value),
                    false,
                    &records[..records
                        .iter()
                        .position(|source| source.id == row.id)
                        .expect("parent source")],
                )
                .is_ok()
        })
        .cloned()
        .collect();
    let available =
        latest_aggregate_by_axis_author(&available_records, parent, subject, &revision)?;
    let missing: Vec<&str> = axes
        .iter()
        .filter_map(|axis| axis["id"].as_str())
        .filter(|axis| {
            !available
                .keys()
                .any(|(available_axis, _, _)| available_axis == axis)
        })
        .collect();
    let legacy = |missing: &[&str]| -> Vec<String> {
        missing
            .iter()
            .map(|axis| format!("challenge `{gate}` is missing current parent aggregate claim `{axis}` for `{subject}`"))
            .collect()
    };
    // The parent gate is judged by the resolver the checked event uses, fresh or
    // carried evidence alike, so commission and the gate name the same cause.
    let Some(resolution) =
        parent_resolution(packet, &records, parent, subject, &revision, &sources)
    else {
        return Ok(legacy(&missing));
    };
    let causes = resolution.causes();
    if causes.is_empty() {
        return Ok(Vec::new());
    }
    // Speak up only where the plain claim check cannot see the problem: a claim is
    // missing, carried evidence is in play, or a review group excluded a pass.
    let group_cause = causes.iter().any(|(category, text)| {
        matches!(
            category.as_str(),
            "assessment_missing"
                | "assessment_inconsistent"
                | "mixed_authors"
                | "unresolved_finding"
        ) || text.contains("excluded (rule")
    });
    if missing.is_empty() && !group_cause && !applicability_in_play(&records, parent, subject) {
        return Ok(Vec::new());
    }
    let mut lines: Vec<String> = causes
        .iter()
        .map(|(category, text)| {
            format!("challenge `{gate}` cannot use parent review `{parent}` for `{subject}` revision {revision} ({category}): {text}")
        })
        .collect();
    // An axis the resolver says nothing about keeps the plain statement.
    let unexplained: Vec<&str> = missing
        .iter()
        .copied()
        .filter(|axis| {
            !causes
                .iter()
                .any(|(_, text)| text.contains(&format!("axis `{axis}`")))
        })
        .filter(|axis| {
            !resolution
                .evidence
                .rows
                .iter()
                .any(|row| row.stage == "aggregate" && row.axis == *axis)
        })
        .collect();
    lines.extend(legacy(&unexplained));
    Ok(lines)
}

/// Whether each existing challenge row counts, using the checked gate's own
/// chronology function (the same cutoff, record kinds and group-local parent
/// axes): an assessment first recorded at or before the latest relevant
/// ordinary-review record is early, and a raw re-append keeps the assessment's
/// first position. Absent when the challenge gate has no rows yet or the
/// policy cannot be read.
fn challenge_standing(
    input: &Value,
    records: &[ContextRecord],
    gate: &str,
    parent: &str,
    subject: &str,
) -> Option<Value> {
    let config = crate::config::parse_initial_input(
        &crate::admission::effective_policy(input, records).effective,
    )
    .ok()?;
    let challenge_axes = config.staged_axes_for_gate(gate)?;
    let parent_axes = config.staged_axes_for_gate(parent)?;
    let early: Vec<Value> = crate::evidence::early_challenge_rows(
        records,
        gate,
        parent,
        subject,
        challenge_axes,
        parent_axes,
    );
    let early_ids: BTreeSet<&str> = early
        .iter()
        .filter_map(|row| row["record_id"].as_str())
        .collect();
    let rows: Vec<Value> = records
        .iter()
        .filter(|record| {
            matches!(record.kind.as_str(), "review-evidence" | "evidence-applicability") && {
                let data = crate::evidence::intake_data(records, record);
                data["gate"] == gate && data["subject"] == subject
            }
        })
        .map(|record| {
            let data = crate::evidence::intake_data(records, record);
            let is_early = early_ids.contains(record.id.as_str());
            json!({
                "record_id": record.id,
                "kind": record.kind,
                "axis": data.get("policy_id"),
                "assessment_identity": crate::evidence::challenge_assessment_identity(data),
                "standing": if is_early { "early" } else { "counts" },
                "reason": if is_early {
                    json!("challenge-before-ordinary-clearance: the assessment was first recorded at or before the latest relevant ordinary-review record; obtain a fresh assessment or append evidence-applicability after that record")
                } else { Value::Null },
            })
        })
        .collect();
    (!rows.is_empty()).then(|| json!(rows))
}

/// Any applicability aimed at this subject for a parent review record, whatever
/// revision it names: carried evidence is in play even when its target went stale.
fn applicability_in_play(records: &[ContextRecord], gate: &str, subject: &str) -> bool {
    records.iter().any(|record| {
        record.kind == "evidence-applicability"
            && record
                .data
                .pointer("/target/subject")
                .and_then(Value::as_str)
                == Some(subject)
            && record
                .data
                .pointer("/origin/id")
                .and_then(Value::as_str)
                .and_then(|id| records.iter().find(|source| source.id.as_str() == id))
                .is_some_and(|source| matches_review(source, gate, subject))
    })
}

fn parent_resolution(
    packet: &Value,
    records: &[ContextRecord],
    parent: &str,
    subject: &str,
    revision: &str,
    sources: &[ReviewSource],
) -> Option<crate::evidence::GateResolution> {
    let author = sources
        .iter()
        .find(|source| source.locator == subject)?
        .value
        .get("author")?;
    let author = crate::evidence::AuthorIdentity::new(
        author.get("name")?.as_str()?,
        author.get("kind")?.as_str()?,
    );
    // The frozen input names the artifact root; fall back to the packet's own.
    let mut initial_input = packet["initial_input"].clone();
    if initial_input.get("artifact_root").is_none() {
        initial_input["artifact_root"] = packet.get("artifact_root")?.clone();
    }
    crate::evidence::resolve_gate(&initial_input, records, parent, subject, revision, &author).ok()
}

/// The parent review as the checked event sees it, with every part labeled:
/// the original judgment, the current applicability attestation that carried
/// it, or a fresh review. Absent when nothing was carried, so fresh-only
/// packets are unchanged.
fn parent_review_block(
    resolution: &crate::evidence::GateResolution,
    records: &[ContextRecord],
    parent: &str,
    subject: &str,
    revision: &str,
) -> Option<Value> {
    if !resolution
        .evidence
        .rows
        .iter()
        .any(|row| row.evidence.carried)
    {
        return None;
    }
    let author_of = |ev: &crate::evidence::ConformingEvidence| json!({"name": ev.author.name(), "kind": ev.author.kind()});
    let mut parts = Vec::new();
    for row in &resolution.evidence.rows {
        let ev = &row.evidence;
        let stored = records.iter().find(|r| r.id.as_str() == ev.source_id);
        let originally_for = stored.and_then(|r| r.data.get("subject_revision")).cloned();
        let earlier_assessment = !ev.carried
            && ev.assessment_id.as_ref().is_some_and(|id| {
                records.iter().any(|other| {
                    other.kind == "review-evidence"
                        && other.id.as_str() != ev.source_id
                        && other.sequence < stored.map_or(other.sequence, |s| s.sequence)
                        && other.data.get("assessment_id").and_then(Value::as_str) == Some(id)
                        && other.data.get("policy_id").and_then(Value::as_str)
                            == Some(ev.policy_id.as_str())
                        && other.data.get("review_stage")
                            == stored.and_then(|s| s.data.get("review_stage"))
                        && other.data.get("author") == stored.and_then(|s| s.data.get("author"))
                })
            });
        parts.push(json!({
            "label": if ev.carried { "original judgment" }
                else if earlier_assessment { "re-appended earlier assessment (not a fresh review)" }
                else { "fresh review" },
            "record_id": ev.source_id, "gate": ev.gate, "axis": ev.policy_id,
            "review_stage": ev.review_stage, "result": ev.result.as_str(),
            "author": author_of(ev), "group": ev.group, "assessment_id": ev.assessment_id,
            "originally_for_revision": originally_for, "grounds": ev.grounds,
            "currency_note": ev.currency_note,
        }));
        if let Some(carry) = ev
            .applicability_id
            .as_ref()
            .and_then(|id| records.iter().find(|r| r.id.as_str() == id))
        {
            parts.push(json!({
                "label": "current applicability attestation",
                "record_id": carry.id, "origin_id": ev.source_id, "axis": ev.policy_id,
                "target": carry.data.get("target"),
                "attesting_driver": carry.data.get("attesting_driver"),
                "reason": carry.data.get("reason"),
            }));
        }
    }
    Some(json!({
        "gate": parent, "subject": subject, "revision": revision,
        "eligible": resolution.is_eligible(),
        "causes": resolution.causes().iter().map(|(category, text)| json!({"cause": category, "detail": text})).collect::<Vec<_>>(),
        "parts": parts,
    }))
}

/// Accepted citation forms, the root each relative form resolves against, and
/// the real structure of every JSON review source, so a reviewer cites pointers
/// that exist.
fn locator_guidance(artifact_root: &Path, sources: &[ReviewSource]) -> Value {
    fn arrays(value: &Value, pointer: &str, depth: usize, out: &mut Vec<(String, usize)>) {
        if depth > 4 || out.len() >= 400 {
            return;
        }
        match value {
            Value::Array(items) => {
                out.push((pointer.to_owned(), items.len()));
                for (index, item) in items.iter().enumerate() {
                    arrays(item, &format!("{pointer}/{index}"), depth + 1, out);
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    arrays(item, &format!("{pointer}/{key}"), depth + 1, out);
                }
            }
            _ => {}
        }
    }
    let roots = crate::source_scopes::locator_roots(artifact_root);
    let resolution = match &roots {
        Ok(map) => json!({
            "artifact_directory": artifact_root,
            "primary_source_root": map.get("primary"),
            "scopes": map,
            "rule": "A path without a scope is tried beneath the artifact directory, then beneath the primary source root. scope:path uses only that scope's root. Resolution never depends on the directory the command runs from.",
            "note": if map.is_empty() { Some(crate::source_scopes::NO_LOCATOR_ROOT) } else { None },
        }),
        Err(error) => json!({"artifact_directory": artifact_root, "error": error}),
    };
    let json_sources: Vec<_> = sources
        .iter()
        .filter(|source| source.locator.ends_with(".json"))
        .map(|source| {
            let mut lengths = Vec::new();
            arrays(&source.value, "", 0, &mut lengths);
            json!({
                "locator": source.locator,
                "top_level_keys": source.value.as_object().map(|map| map.keys().cloned().collect::<Vec<_>>()),
                "array_lengths": lengths.into_iter().collect::<BTreeMap<_, _>>(),
            })
        })
        .collect();
    json!({
        "accepted_forms": [
            "file#/json/pointer  (a JSON Pointer that exists in that file; array indices run 0 to length-1)",
            "file#L<start>-L<end>  (a line range of that file)",
            "scope:path#selector  (a path beneath a declared source_scopes root, with either selector above)",
            "record:ID#/pointer  (a pointer into one retained context record; review contract v3)"
        ],
        "resolution": resolution,
        "json_sources": json_sources,
    })
}

/// The command-evidence IDs a criterion or goal verdict may cite in
/// evidence_context_ids, read from the fixed report's command index. The index
/// may list an evidence-applicability wrapper for carried evidence; a verdict
/// cannot cite that wrapper, so the packet names the original command evidence
/// to cite instead (#48). An owner instruction delivered to this commission
/// (an unsuperseded `user-steering` record) is also citable (LE-201).
fn citable_command_evidence(
    root: &Path,
    context: &[ContextRecord],
    steering_ids: &[String],
) -> Value {
    let report = std::fs::read(root.join("validation-report.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let find = |id: &str| context.iter().find(|record| record.id.as_str() == id);
    let mut citable: Vec<&str> = Vec::new();
    let mut wrappers = Vec::new();
    for id in report["command_evidence_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let Some(record) = find(id) else { continue };
        let target = match record.kind.as_str() {
            "command-evidence" => Some(id),
            "evidence-applicability" => record
                .data
                .pointer("/origin/id")
                .and_then(Value::as_str)
                .filter(|origin| find(origin).is_some_and(|r| r.kind == "command-evidence")),
            _ => None,
        };
        let Some(target) = target else { continue };
        if target != id {
            wrappers.push(json!({"indexed_id": id, "cite_instead": target}));
        }
        if !citable.contains(&target) {
            citable.push(target);
        }
    }
    json!({
        "command_evidence_ids": citable,
        "indexed_but_not_citable": wrappers,
        "user_steering_ids": steering_ids,
        "rule": "A criterion or goal verdict's evidence_context_ids may name only the command_evidence_ids and user_steering_ids listed here. An evidence-applicability record is never citable, even when the report's command index lists it; cite the original command evidence named in cite_instead. A user_steering_ids entry is the owner's words; cite it when they establish the verdict's result, for example when they withdraw, relax or defer the requirement, and judge the requirement as those words state it.",
    })
}

#[derive(Clone)]
struct ReviewSource {
    locator: String,
    sha256: String,
    bytes: Vec<u8>,
    value: Value,
}

#[doc(hidden)]
pub fn review_subject(gate: &str) -> Option<&'static str> {
    match gate {
        "intent-review" | "intent-adversarial-review" => Some("intent.json"),
        "design-review" | "design-adversarial-review" => Some("design.json"),
        "plan-review" | "plan-adversarial-review" => Some("plan.json"),
        "implementation-review" | "implementation-adversarial-review" => {
            Some("implementation-report.json")
        }
        "validation-review" | "validation-adversarial-review" => Some("validation-report.json"),
        _ => narrative_options(gate).and_then(|options| options.narrative.map(|n| n.subject)),
    }
}

fn parent_gate(gate: &str) -> Option<&'static str> {
    match gate {
        "intent-adversarial-review" => Some("intent-review"),
        "design-adversarial-review" => Some("design-review"),
        "plan-adversarial-review" => Some("plan-review"),
        "implementation-adversarial-review" => Some("implementation-review"),
        "validation-adversarial-review" => Some("validation-review"),
        _ => narrative_options(gate)
            .and_then(|options| options.narrative)
            .filter(|n| n.challenge == gate)
            .map(|n| n.review),
    }
}

fn read_review_sources(root: &Path, subject: &str) -> Result<(String, Vec<ReviewSource>), String> {
    let narrative_subject = crate::workflow::PHASE_OPTIONS
        .iter()
        .any(|options| options.narrative.is_some_and(|n| n.subject == subject));
    let mut paths = vec!["intent.json"];
    if subject != "intent.json" {
        paths.push(subject);
    }
    let mut sources = Vec::new();
    let mut subject_value = None;
    for locator in paths {
        if narrative_subject {
            let source = read_option_source(root, locator, true)?;
            if locator == subject {
                subject_value = Some(source.value.clone());
            }
            sources.push(source);
            continue;
        }
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
    if let Some(narrative) = crate::workflow::PHASE_OPTIONS
        .iter()
        .filter_map(|options| options.narrative.map(|n| (options, n)))
        .find(|(_, n)| n.subject == subject)
    {
        for locator in [narrative.1.markdown, narrative.0.phase.subject] {
            sources.push(read_option_source(
                root,
                locator,
                locator.ends_with(".json"),
            )?);
        }
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

// Validation findings may originate in the fixed criterion/goal index, not
// only in axis review evidence. Keep all other gates review-evidence-only.
fn matches_finding_source(record: &ContextRecord, gate: &str, subject: &str) -> bool {
    matches_review(record, gate, subject)
        || (gate == "validation-review"
            && subject == "validation-report.json"
            && (record.kind == "criterion-verdict"
                && record
                    .data
                    .get("criterion_id")
                    .and_then(Value::as_str)
                    .is_some_and(crate::criterion::is_criterion_id)
                || record.kind == "goal-verdict" && record.data.get("criterion_id").is_none())
            && record.data.get("subject").and_then(Value::as_str) == Some(subject)
            && record.data.get("checkpoint").and_then(Value::as_str)
                == Some("validation-checkpoint.json"))
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
            matches_finding_source(source, gate, subject)
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
                    matches_finding_source(source, gate, subject)
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
        (!superseded.contains(record.id.as_str()) && owner_source_targets_intent(record))
            .then_some(steering.intent_baseline)
            .flatten()
            .map(|baseline| (record.id.as_str().to_owned(), baseline))
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct IntentRevisionBaseline {
    pub record_id: String,
    pub source_kind: &'static str,
    pub baseline: IntentBaseline,
    pub correction: Option<crate::intent_correction::Correction>,
}

/// One qualified prior-byte selector for admission, commission and owner view.
/// No numeric revision guess, prose scope inference or invented owner statement.
pub fn intent_revision_baseline(
    records: &[ContextRecord],
    initial: &Value,
    intent: &Value,
) -> Result<Option<IntentRevisionBaseline>, String> {
    let current = intent["revision"]
        .as_str()
        .filter(|revision| !revision.trim().is_empty())
        .ok_or("intent has no revision")?;
    let prior = crate::intent_correction::prior_reviewed_revision(records, current);
    let owner_records: Vec<_> = records.iter().collect();
    if let Some((record_id, baseline)) = active_intent_baseline(&owner_records) {
        validate_intent_baseline(&baseline)?;
        if prior
            .as_deref()
            .is_none_or(|revision| revision == baseline.revision)
        {
            return Ok(Some(IntentRevisionBaseline {
                record_id,
                source_kind: "user-steering",
                baseline,
                correction: None,
            }));
        }
    }
    for (index, record) in records.iter().enumerate().rev().filter(|(_, record)| {
        record.kind == "intent-correction" && record.data["subject_revision"] == current
    }) {
        let correction =
            crate::intent_correction::validate(&record.data, initial, &records[..index])?;
        if prior.as_deref() == Some(correction.prior_intent.revision.as_str()) {
            return Ok(Some(IntentRevisionBaseline {
                record_id: record.id.as_str().into(),
                source_kind: "intent-correction",
                baseline: correction.prior_intent.clone(),
                correction: Some(correction),
            }));
        }
    }
    if let Some(prior) = prior {
        return Err(format!("later intent revision `{current}` has no exact qualified prior baseline for `{prior}`; supply owner user-steering intent_baseline for a scope change, or a ledger-cited intent-correction for an accepted intent finding"));
    }
    Ok(None)
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
    let mut ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
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
    // A carried position names the current applicability record in the fixed
    // index. Its origin names an earlier verdict, never the current index ID.
    let applicable: Vec<_> = context
        .iter()
        .filter(|record| {
            record.kind == "evidence-applicability" && indexed.contains(record.id.as_str())
        })
        .collect();
    for carry in &applicable {
        if let Some(source) = carry
            .data
            .pointer("/origin/id")
            .and_then(Value::as_str)
            .and_then(|id| context.iter().find(|record| record.id.as_str() == id))
        {
            if matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict") {
                ids.entry(source.kind.clone())
                    .or_default()
                    .insert(source.id.as_str().to_owned());
            }
        }
    }
    ids.insert(
        "evidence-applicability".to_owned(),
        applicable
            .iter()
            .map(|record| record.id.as_str().to_owned())
            .collect(),
    );
    ids
}

#[cfg(test)]
fn project_review_context(
    records: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    requested_stage: Option<&str>,
    root: &Path,
) -> Result<Vec<ContextRecord>, String> {
    project_review_context_with_diagnostics(
        records,
        records,
        gate,
        subject,
        revision,
        requested_stage,
        root,
    )
    .map(|(selected, _)| selected)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn project_review_context_with_diagnostics(
    records: &[ContextRecord],
    recovery_context: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    requested_stage: Option<&str>,
    root: &Path,
) -> Result<(Vec<ContextRecord>, Vec<Value>), String> {
    project_review_context_with_diagnostics_excluding(
        records,
        recovery_context,
        gate,
        subject,
        revision,
        requested_stage,
        root,
        &BTreeSet::new(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn project_review_context_with_diagnostics_excluding(
    records: &[ContextRecord],
    recovery_context: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    requested_stage: Option<&str>,
    root: &Path,
    requested_exclusions: &BTreeSet<String>,
    admission: Option<(
        &crate::admission::AdmissionContext<'_>,
        &[crate::admission::Rule],
    )>,
) -> Result<(Vec<ContextRecord>, Vec<Value>), String> {
    let root_value = Value::String(root.to_string_lossy().into_owned());
    let mut diagnostics = Vec::new();
    let mut excluded = requested_exclusions.clone();
    for candidate_gate in std::iter::once(gate).chain(parent_gate(gate)) {
        match crate::recovery::excluded_record_ids(
            recovery_context,
            candidate_gate,
            subject,
            Some(&root_value),
        ) {
            Ok(ids) => excluded.extend(ids),
            Err(reason) => {
                let record_id = recovery_context
                    .iter()
                    .find(|row| {
                        row.kind == crate::recovery::RECORD_WITHDRAWAL_KIND
                            && reason.contains(&format!("record `{}`", row.id))
                    })
                    .map(|row| row.id.to_string());
                diagnostics.push(json!({"record_id":record_id,"reason":reason}));
            }
        }
    }
    diagnostics.extend(
        crate::recovery::stale_exclusions(recovery_context)
            .into_values()
            .filter(|cause| {
                cause.gate.as_deref() == Some(gate)
                    || cause
                        .gate
                        .as_deref()
                        .is_some_and(|cause_gate| parent_gate(gate) == Some(cause_gate))
            })
            .map(|cause| json!({"record_id":cause.record_id,"reason":cause.message()})),
    );
    let mut admission_refusals = BTreeMap::new();
    if let Some((ctx, extensions)) = admission {
        let selected: BTreeSet<String> = records
            .iter()
            .filter(|record| {
                let relevant_gate = record.data["gate"] == gate
                    || parent_gate(gate).is_some_and(|parent| record.data["gate"] == parent);
                (matches!(
                    record.kind.as_str(),
                    "review-evidence" | "finding-ledger" | "reviewer-manifest"
                ) && relevant_gate)
                    || (record.kind == "evidence-applicability"
                        && record.data["target"]["subject"] == subject
                        && record.data["target"]["revision"] == revision)
                    || (matches!(
                        record.kind.as_str(),
                        "intent-comparison" | "intent-correction"
                    ) && subject == "intent.json"
                        && record.data["subject_revision"] == revision)
            })
            .map(|record| record.id.to_string())
            .collect();
        for (record, result) in
            crate::admission::classify_selected(ctx, &excluded, Some(&selected), extensions)
        {
            if let Err(refusal) = result {
                admission_refusals.insert(record.id.to_string(), refusal);
            }
        }
    }
    let mut usable = Vec::new();
    for record in records
        .iter()
        .filter(|row| !excluded.contains(row.id.as_str()))
    {
        if let Some(refusal) = admission_refusals.get(record.id.as_str()) {
            diagnostics.push(json!({
                "record_id":record.id,"rule":refusal.rule,"reason":refusal.message
            }));
            if record.kind == "finding-ledger" {
                usable.push(record.clone());
            }
            continue;
        }
        let relevant_gate = record.data["gate"] == gate
            || parent_gate(gate).is_some_and(|parent| record.data["gate"] == parent);
        let check = if record.kind == "review-evidence" && relevant_gate {
            crate::evidence::parse_evidence_record_for_stage_with_context(
                &record.data,
                subject,
                Some(&root_value),
                false,
                &recovery_context[..recovery_context
                    .iter()
                    .position(|source| source.id == record.id)
                    .expect("packet source")],
            )
            .map(|_| ())
        } else if record.kind == "evidence-applicability"
            && record.data["target"]["subject"] == subject
            && record.data["target"]["revision"] == revision
        {
            let source = record.data["origin"]["id"]
                .as_str()
                .and_then(|id| recovery_context.iter().find(|row| row.id.as_str() == id));
            if source.is_none_or(|source| source.kind == "review-evidence") {
                crate::evidence::parse_applicable_evidence_for_stage(
                    recovery_context,
                    record,
                    subject,
                    revision,
                    Some(&root_value),
                    false,
                )
                .map(|_| ())
                .map_err(crate::evidence::EvidenceParseError::into_message)
            } else {
                Ok(())
            }
        } else if record.kind == "evidence-applicability"
            && record.data["target"]["subject"] == "validation-report.json"
        {
            let source_id = record.data["origin"]["id"].as_str().ok_or_else(|| {
                format!(
                    "validation applicability `{}` has no original verdict source",
                    record.id
                )
            });
            source_id.and_then(|id| {
                let source = recovery_context.iter().find(|row| row.id.as_str() == id)
                    .ok_or_else(|| format!("validation applicability `{}` is missing its original verdict `{id}`", record.id))?;
                if !matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                    || source.data["subject"] != subject || source.data["checkpoint"] != "validation-checkpoint.json"
                    || record.data["target"]["subject_revision"] != revision
                    || record.data["target"]["checkpoint"] != json!({"phase":"validation","report_revision":revision}) {
                    return Err(format!("validation applicability `{}` does not reference a validation verdict for `{subject}`", record.id));
                }
                Ok(())
            })
        } else if record.kind == "finding-ledger" && relevant_gate {
            let gate = record.data["gate"].as_str().unwrap_or_default();
            let prefix: Vec<_> = recovery_context
                .iter()
                .filter(|row| row.sequence <= record.sequence)
                .cloned()
                .collect();
            crate::finding_ledger::latest_snapshot_for_recovery(
                &prefix,
                gate,
                subject,
                Some(&root_value),
            )
            .map(|_| ())
        } else {
            Ok(())
        };
        match check {
            Ok(()) => usable.push(record.clone()),
            Err(reason) => {
                diagnostics.push(json!({"record_id":record.id,"reason":reason}));
                // Keep the immutable ledger's recorded duties visible in the
                // packet even when it is malformed; it is never gate clearance.
                if record.kind == "finding-ledger" {
                    usable.push(record.clone());
                }
            }
        }
    }
    let records = usable.as_slice();
    let parent = parent_gate(gate);
    let parent_aggregate_ids = if let Some(parent) = parent {
        let parent_aggregates =
            latest_aggregate_by_axis_author(records, parent, subject, revision)?;
        let mut parent_aggregates = parent_aggregates;
        parent_aggregates.retain(|_, record| {
            if record.data.get("review_contract_version").and_then(Value::as_u64).is_none_or(|v| !matches!(v, 2 | 3)) {
                diagnostics.push(json!({"record_id":record.id,"reason":"challenge parent has no supported versioned grounds"}));
                false
            } else { true }
        });
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
        let Some(findings) = ledger.data.get("findings").and_then(Value::as_array) else {
            diagnostics.push(
                json!({"record_id":ledger.id,"reason":"confirmation ledger has no findings array"}),
            );
            continue;
        };
        for finding in findings {
            let check = || -> Result<(), String> {
                let source_id = finding
                    .pointer("/source/id")
                    .and_then(Value::as_str)
                    .ok_or("confirmation finding has no exact source locator")?;
                let source = recovery_context.iter().find(|row| row.id.as_str() == source_id)
                    .ok_or_else(|| format!("mandatory confirmation finding source `{source_id}` is missing; inspect retained source for {ledger_gate}/{subject}"))?;
                if !matches_finding_source(source, ledger_gate, subject)
                    || (matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                        && finding.get("policy_id").and_then(Value::as_str)
                            != Some(
                                source
                                    .data
                                    .get("criterion_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("goal"),
                            ))
                {
                    return Err(format!("mandatory confirmation finding source `{source_id}` has the wrong gate or subject"));
                }
                Ok(())
            };
            if let Err(reason) = check() {
                diagnostics.push(json!({"record_id":ledger.id,"reason":reason}));
            }
        }
    }
    let validation_ids = if matches!(gate, "validation-review" | "validation-adversarial-review") {
        validation_record_ids(root, records)
    } else {
        BTreeMap::new()
    };
    let mut selected_review_ids = BTreeSet::new();
    for record in records {
        let own = matches_review(record, gate, subject);
        let own_verdict_source =
            matches_finding_source(record, gate, subject) && record.kind != "review-evidence";
        let parent_source = parent.is_some_and(|parent| matches_review(record, parent, subject));
        if excluded_ids.contains(record.id.as_str()) {
            continue;
        }
        if (own
            && (ledger_refs.contains(record.id.as_str())
                || applicability_refs.contains(record.id.as_str())
                || (requested_stage.is_some_and(|stage| evidence_stage(record) == stage)
                    && record.data.get("subject_revision").and_then(Value::as_str)
                        == Some(revision))))
            || (own_verdict_source && ledger_refs.contains(record.id.as_str()))
            || (parent_source
                && (parent_aggregate_ids.contains(record.id.as_str())
                    || ledger_refs.contains(record.id.as_str())
                    || applicability_refs.contains(record.id.as_str())))
        {
            selected_review_ids.insert(record.id.as_str().to_owned());
        }
    }
    let mut selected = Vec::new();
    for record in records {
        if record.kind == "review-evidence" {
            if selected_review_ids.contains(record.id.as_str()) {
                selected.push(compact_review_evidence(record, revision));
            }
            continue;
        }
        if matches_finding_source(record, gate, subject)
            && record.kind != "review-evidence"
            && selected_review_ids.contains(record.id.as_str())
        {
            selected.push(record.clone());
            continue;
        }
        if record.kind == "evidence-applicability" {
            if matches!(gate, "validation-review" | "validation-adversarial-review")
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
                let target = crate::evidence::canonical_target(
                    &record.data["target"],
                    matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict"),
                )?;
                if !matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                    || source.data.get("subject").and_then(Value::as_str) != Some(subject)
                    || source.data.get("checkpoint").and_then(Value::as_str)
                        != Some("validation-checkpoint.json")
                    || record
                        .data
                        .pointer("/target/subject")
                        .and_then(Value::as_str)
                        != Some(subject)
                    || target["revision"].as_str() != Some(revision)
                    || record.data.pointer("/target/checkpoint")
                        != Some(&json!({"phase":"validation","report_revision":revision}))
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
                        (!excluded_ids.contains(id) && selected_review_ids.contains(id))
                            || (finding["disposition"] == "accepted"
                                && finding["status"] == "unresolved")
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
        if record.kind == "user-steering" {
            selected.push(record.clone());
            continue;
        }
        if record.kind == "intent-correction"
            && subject == "intent.json"
            && record.data["subject_revision"] == revision
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
    Ok((selected, diagnostics))
}

/// Deliver a retained review exchange by reference. The stored record keeps the
/// full request and the reviewer's original response; a commission carries the
/// verdict, findings and grounds, and replaces each exchange body with its size,
/// digest and `record:ID#/pointer` locator. Evidence of the target revision is
/// delivered whole; only other revisions' evidence (carried or ledger-cited) is
/// compacted.
fn compact_review_evidence(record: &ContextRecord, revision: &str) -> ContextRecord {
    if record.data.get("subject_revision").and_then(Value::as_str) == Some(revision) {
        return record.clone();
    }
    let mut compact = record.clone();
    if let Some(exchange) = compact
        .data
        .get_mut("external_review")
        .and_then(Value::as_object_mut)
    {
        for part in ["request", "original"] {
            if let Some(body) = exchange.get(part) {
                let bytes = serde_json::to_vec(body).unwrap_or_default();
                let stub = json!({"omitted_from_commission":true,"bytes":bytes.len(),
                    "sha256":format!("sha256:{:x}",Sha256::digest(&bytes)),
                    "locator":format!("record:{}#/data/external_review/{part}",record.id)});
                exchange.insert(part.to_owned(), stub);
            }
        }
    }
    compact
}

fn commission_size_info(
    records: &[ContextRecord],
    sources: &[ReviewSource],
    artifact_root: &str,
) -> Result<Value, String> {
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
    let context_bytes =
        serde_json::to_vec(&json!({"artifact_root":artifact_root,"context":delivered}))
            .map_err(|error| error.to_string())?
            .len();
    let original_bytes = sources
        .iter()
        .map(|source| source.bytes.len())
        .sum::<usize>();
    let mut context_bytes_by_kind = BTreeMap::<String, usize>::new();
    for record in &delivered {
        *context_bytes_by_kind
            .entry(record.kind.clone())
            .or_default() += serde_json::to_vec(record)
            .map_err(|error| error.to_string())?
            .len();
    }
    Ok(
        json!({"status":"informational only; driver selects relevant context; no content quota",
        "context_bytes":context_bytes,"context_bytes_by_kind":context_bytes_by_kind,
        "retrieved_original_bytes":original_bytes,"total_evidence_bytes":context_bytes.saturating_add(original_bytes)}),
    )
}

fn inspect_with_budgets(
    input: &Value,
    requested_slot: Option<&str>,
    task: Option<&str>,
    requested_stage: Option<&str>,
    excluded_ids: &BTreeSet<String>,
    admission: Option<(
        &crate::admission::AdmissionContext<'_>,
        &[crate::admission::Rule],
    )>,
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
    let full_context_records: Vec<ContextRecord> =
        serde_json::from_value(packet["context"].clone()).map_err(|e| e.to_string())?;
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
    let revision_baseline = if subject == Some("intent.json") {
        let intent = &sources
            .iter()
            .find(|source| source.locator == "intent.json")
            .expect("intent source")
            .value;
        intent_revision_baseline(&full_context_records, &packet["initial_input"], intent)?
    } else {
        None
    };
    let (selected, recovery_diagnostics) = project_review_context_with_diagnostics_excluding(
        &records,
        &full_context_records,
        slot_id,
        subject.unwrap(),
        &revision,
        requested_stage,
        Path::new(root),
        excluded_ids,
        admission,
    )?;
    records = selected;
    let alignment_required = slot_id == "intent-adversarial-review"
        && packet["initial_input"]["review_policies"][slot_id]
            .as_array()
            .is_some_and(|axes| axes.iter().any(|axis| axis["id"] == "intent-sufficient"));
    let alignment_diagnostic = if alignment_required {
        let intent = &sources
            .iter()
            .find(|source| source.locator == "intent.json")
            .expect("intent source")
            .value;
        records.extend(
            full_context_records
                .iter()
                .filter(|r| {
                    r.kind == "intent-comparison"
                        && r.data["subject_revision"] == intent["revision"]
                })
                .cloned(),
        );
        crate::intent_comparison::validate(&full_context_records, intent, &packet["initial_input"])
            .err()
    } else {
        None
    };
    let mut projected = input.clone();
    let context_value = serde_json::to_value(&records).map_err(|error| error.to_string())?;
    if show.is_some() {
        projected["result"]["context"] = context_value;
    } else {
        projected["context"] = context_value;
    }
    let mut output = inspect_with_stage_raw(&projected, Some(slot_id), task, None)?;
    output["record_diagnostics"] = json!(recovery_diagnostics);
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
                .filter(|record| {
                    record.kind == "user-steering" && owner_source_targets_intent(record)
                })
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
    let size_info = commission_size_info(&selected_records, &sources, root)?;
    if output.get("record_ids").is_some() {
        return Ok(output);
    }
    if let Some(diagnostic) = alignment_diagnostic {
        if !output["missing_inputs"].is_array() {
            output["missing_inputs"] = json!([]);
        }
        output["missing_inputs"]
            .as_array_mut()
            .expect("array")
            .push(json!({"kind":"intent-comparison","diagnostic":diagnostic}));
    }
    if alignment_required {
        let comparisons: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "intent-comparison")
            .collect();
        output["intent_comparisons"] = json!(comparisons);
    }
    output["review_contract_version"] = json!(3);
    output["review_target"] = json!({"subject":subject,"revision":revision});
    output["locator_guidance"] = locator_guidance(Path::new(root), &sources);
    if matches!(
        slot_id,
        "validation-review" | "validation-adversarial-review"
    ) {
        let steering_ids: Vec<String> = output["commission"]["steering_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        output["citable_evidence"] =
            citable_command_evidence(Path::new(root), &full_context_records, &steering_ids);
    }
    if let Some(parent) = parent_gate(slot_id) {
        if let Some(block) = parent_resolution(
            packet,
            &full_context_records,
            parent,
            subject.unwrap(),
            &revision,
            &sources,
        )
        .and_then(|resolution| {
            parent_review_block(
                &resolution,
                &full_context_records,
                parent,
                subject.unwrap(),
                &revision,
            )
        }) {
            output["parent_review"] = block;
        }
        if let Some(standing) = challenge_standing(
            &packet["initial_input"],
            &full_context_records,
            slot_id,
            parent,
            subject.unwrap(),
        ) {
            output["challenge_standing"] = standing;
        }
    }
    output["review_sources"] = json!(sources.iter().map(|source|json!({"locator":source.locator,"sha256":source.sha256,"bytes":source.bytes.len()})).collect::<Vec<_>>());
    output["review_budget"] = size_info;
    output["review_record_sources"] = json!(selected_records
        .iter()
        .map(|record| json!({"record_id":record.id,"kind":record.kind}))
        .collect::<Vec<_>>());
    if let Some(baseline) = revision_baseline {
        let record = full_context_records
            .iter()
            .find(|record| record.id.as_str() == baseline.record_id)
            .expect("selected baseline record");
        output["intent_revision_baseline"] = json!({"source":baseline,"record":record,"delivered_in_context":selected_records.iter().any(|selected| selected.id==record.id)});
    }
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
            &packet["initial_input"],
        );
    }
    Ok(output)
}

fn owner_approval_view(
    intent: &Value,
    current_sha256: &str,
    context: &[&ContextRecord],
    actor_context: &[ContextRecord],
    initial: &Value,
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
    let exact_diff = match intent_revision_baseline(actor_context, initial, intent) {
        Ok(Some(source)) => match validate_intent_baseline(&source.baseline) {
            Ok(previous) => {
                json!({"status":"available","source_record_id":source.record_id,"source_kind":source.source_kind,
                "source_locator":format!("record:{}#/data/{}/json_bytes",source.record_id,if source.source_kind=="intent-correction" {"prior_intent"} else {"intent_baseline"}),
                "baseline_source":{"locator":source.baseline.locator,"sha256":source.baseline.sha256,"bytes":source.baseline.json_bytes.len()},
                "correction":source.correction,"from_revision":source.baseline.revision,"to_revision":intent.get("revision"),"changes":json_diff(&previous,intent,"")})
            }
            Err(error) => {
                json!({"status":"invalid-source","source_record_id":source.record_id,"diagnostic":error})
            }
        },
        Err(error) => {
            json!({"status":"missing-prior-intent-bytes","diagnostic":error,"locator":"intent.json"})
        }
        Ok(None) => {
            json!({"status":"initial-draft-no-prior-intent","diagnostic":"No earlier reviewed intent exists; compare the full draft against retained qualified owner-source statements.","owner_source_locator":"user-steering context records"})
        }
    };
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
        "model_bindings":loop_core::model_bindings_view(initial, actor_context),
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

pub(crate) fn validate_intent_baseline(baseline: &IntentBaseline) -> Result<Value, String> {
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

    fn typed_record(id: &str, kind: &str, n: u64, mut data: Value) -> ContextRecord {
        if kind == "review-evidence" {
            data.as_object_mut()
                .unwrap()
                .entry("config_version")
                .or_insert(json!("fixture-1"));
        }
        if kind == "evidence-applicability" {
            data.as_object_mut()
                .unwrap()
                .entry("attesting_driver")
                .or_insert(json!({"name":"driver","kind":"agent"}));
        }
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

    fn bulky_exchange(tag: &str) -> Value {
        json!({"request":{"text":format!("request {tag} {}", "r".repeat(20_000))},
            "original":{"text":format!("original {tag} {}", "o".repeat(20_000))}})
    }

    #[test]
    fn delivered_review_evidence_size_tracks_applicable_records_not_history() {
        let evidence = |id: &str, n: u64, revision: &str, stage: &str| {
            typed_record(
                id,
                "review-evidence",
                n,
                json!({
                    "gate":"design-review","subject":"design.json","subject_revision":revision,
                    "review_stage":stage,"policy_id":"axis","author":{"name":"A","kind":"agent"},
                    "result":"fail","findings":format!("finding of {id}"),
                    "external_review":bulky_exchange(id)
                }),
            )
        };
        let project = |old_reviews: u64| {
            let mut records = vec![evidence("carried", 1, "1", "individual")];
            // Unrelated history: other revisions' reviews that nothing carries or cites.
            records.extend(
                (0..old_reviews).map(|n| evidence(&format!("old-{n}"), 10 + n, "1", "aggregate")),
            );
            let base = 100 + old_reviews;
            records.push(evidence("current", base, "2", "aggregate"));
            records.push(typed_record(
                "ledger",
                "finding-ledger",
                base + 1,
                json!({"gate":"design-review","subject":"design.json","subject_revision":"2",
                    "findings":[{"source":{"kind":"context-record","id":"carried"},"statement":"x","disposition":"accepted","status":"unresolved"}]}),
            ));
            records.push(typed_record(
                "carry",
                "evidence-applicability",
                base + 2,
                json!({"origin":{"kind":"context-record","id":"carried"},
                    "target":{"subject":"design.json","revision":"2"},"reason":"unaffected"}),
            ));
            project_review_context(
                &records,
                "design-review",
                "design.json",
                "2",
                Some("aggregate"),
                Path::new("/unused-for-design"),
            )
            .unwrap()
        };
        let few = project(1);
        let many = project(12);
        let size = |records: &[ContextRecord]| serde_json::to_vec(records).unwrap().len();
        assert_eq!(
            size(&few),
            size(&many),
            "unrelated history must not be delivered"
        );
        let carried = many.iter().find(|r| r.id.as_str() == "carried").unwrap();
        assert!(carried.data["findings"]
            .as_str()
            .unwrap()
            .contains("finding of carried"));
        for part in ["request", "original"] {
            let stub = &carried.data["external_review"][part];
            assert_eq!(stub["omitted_from_commission"], true);
            assert_eq!(
                stub["locator"],
                format!("record:carried#/data/external_review/{part}")
            );
            assert!(stub["bytes"].as_u64().unwrap() > 20_000);
        }
        // Target-revision evidence is delivered whole.
        let current = many.iter().find(|r| r.id.as_str() == "current").unwrap();
        assert!(current.data["external_review"]["request"]["text"].is_string());
        assert!(current.data["external_review"]["original"]["text"].is_string());
        // One whole record (~40 KB); uncompacted, the carried one would add another ~40 KB.
        assert!(size(&many) < 45_000, "{}", size(&many));
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
    fn recovery_confirmation_diagnoses_missing_mandatory_finding_source() {
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
        let records = vec![prior_aggregate, ledger];
        let (_, diagnostics) = project_review_context_with_diagnostics(
            &records,
            &records,
            "design-review",
            "design.json",
            "2",
            None,
            Path::new("/unused-for-design"),
        )
        .unwrap();
        assert!(json!(diagnostics)
            .to_string()
            .contains("mandatory confirmation finding source `missing-source` is missing"));
    }

    #[test]
    fn validation_confirmation_projects_original_verdicts_and_only_indexed_carries() {
        let root = std::env::temp_dir().join(format!(
            "software-change-commission-validation-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("validation-report.json"),
            serde_json::to_vec(&json!({
                "revision":"v2", "criteria":[
                    {"criterion_id":"AC-1","verdict_ids":["fresh-ac1"]},
                    {"criterion_id":"AC-2","verdict_ids":["carry-ac2"]}],
                "goal_verdict_ids":["carry-goal"]
            }))
            .unwrap(),
        )
        .unwrap();
        let source = |id, kind, criterion: Option<&str>| {
            let mut data = json!({"subject":"validation-report.json","subject_revision":"v1",
                "checkpoint":"validation-checkpoint.json","author":{"name":"independent","kind":"agent"},
                "result":"fail","findings":["original failure"]});
            if let Some(criterion) = criterion {
                data["criterion_id"] = json!(criterion);
            }
            typed_record(id, kind, 1, data)
        };
        let original_fail = source("old-ac1", "criterion-verdict", Some("AC-1"));
        let original_ac2 = source("old-ac2", "criterion-verdict", Some("AC-2"));
        let original_goal = source("old-goal", "goal-verdict", None);
        let carry = |id, origin, n| {
            typed_record(
                id,
                "evidence-applicability",
                n,
                json!({
                    "origin":{"kind":"context-record","id":origin},
                    "target":{"subject":"validation-report.json","subject_revision":"v2",
                              "checkpoint":{"phase":"validation","report_revision":"v2"}},
                    "attesting_driver":{"name":"driver","kind":"agent"},"reason":"unaffected original"
                }),
            )
        };
        let ac2 = carry("carry-ac2", "old-ac2", 2);
        let goal = carry("carry-goal", "old-goal", 3);
        let unindexed = carry("unindexed", "old-ac2", 4);
        let ledger = typed_record(
            "current-ledger",
            "finding-ledger",
            5,
            json!({
                "gate":"validation-review","subject":"validation-report.json","subject_revision":"v2",
                "findings":[{"source":{"kind":"context-record","id":"old-ac1"},
                    "policy_id":"AC-1","statement":"original failure","disposition":"accepted","status":"resolved"}]
            }),
        );
        let records = vec![
            original_fail.clone(),
            original_ac2.clone(),
            original_goal.clone(),
            ac2.clone(),
            goal.clone(),
            unindexed,
            ledger.clone(),
        ];
        let projected = project_review_context(
            &records,
            "validation-review",
            "validation-report.json",
            "v2",
            Some("aggregate"),
            &root,
        )
        .unwrap();
        let ids: BTreeSet<_> = projected.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(
            ids,
            BTreeSet::from([
                "old-ac1",
                "old-ac2",
                "old-goal",
                "carry-ac2",
                "carry-goal",
                "current-ledger"
            ])
        );
        assert_eq!(
            projected
                .iter()
                .find(|record| record.id.as_str() == "old-ac1")
                .unwrap()
                .data["result"],
            "fail"
        );
        assert_eq!(
            projected
                .iter()
                .find(|record| record.id.as_str() == "old-ac1")
                .unwrap()
                .data["author"]["name"],
            "independent"
        );
        assert_eq!(
            projected
                .iter()
                .find(|record| record.id.as_str() == "current-ledger")
                .unwrap()
                .data["findings"][0]["source"]["id"],
            "old-ac1"
        );

        for (invalid, expected) in [
            (
                typed_record("old-ac1", "goal-verdict", 1, original_fail.data.clone()),
                "wrong gate or subject",
            ),
            (
                source("old-ac1", "criterion-verdict", Some("AC-2")),
                "wrong gate or subject",
            ),
            (
                typed_record(
                    "old-ac1",
                    "criterion-verdict",
                    1,
                    json!({"subject":"design.json","subject_revision":"v1","checkpoint":"validation-checkpoint.json","criterion_id":"AC-1"}),
                ),
                "wrong gate or subject",
            ),
            (
                typed_record(
                    "old-ac1",
                    "criterion-verdict",
                    1,
                    json!({"subject":"validation-report.json","subject_revision":"v1","checkpoint":"wrong-checkpoint","criterion_id":"AC-1"}),
                ),
                "wrong gate or subject",
            ),
        ] {
            let mut bad = records.clone();
            bad[0] = invalid;
            assert!(project_review_context_with_diagnostics(
                &bad,
                &bad,
                "validation-review",
                "validation-report.json",
                "v2",
                Some("aggregate"),
                &root
            )
            .unwrap()
            .1
            .iter()
            .any(|diagnostic| diagnostic["reason"].as_str().unwrap().contains(expected)));
        }
        let mut bad = records;
        bad[3].data["target"]["subject_revision"] = json!("v1");
        assert!(project_review_context_with_diagnostics(
            &bad,
            &bad,
            "validation-review",
            "validation-report.json",
            "v2",
            Some("aggregate"),
            &root
        )
        .unwrap()
        .1
        .iter()
        .any(|diagnostic| diagnostic["reason"]
            .as_str()
            .unwrap()
            .contains("does not reference a validation verdict")));
        std::fs::remove_dir_all(root).unwrap();
    }

    // bookends:LE-149 bookends:LE-171 bookends:LE-183 bookends:LE-201
    #[test]
    fn validation_commission_lists_citable_command_evidence_and_names_originals_for_wrappers() {
        let root = std::env::temp_dir().join(format!(
            "commission-citable-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for name in [
            "intent",
            "design",
            "plan",
            "implementation-report",
            "validation-report",
        ] {
            std::fs::write(
                root.join(format!("{name}.json")),
                std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join(format!("data/calibration/fixtures/{name}-good.json")),
                )
                .unwrap(),
            )
            .unwrap();
        }
        let mut report: Value =
            serde_json::from_slice(&std::fs::read(root.join("validation-report.json")).unwrap())
                .unwrap();
        // One direct command, one carried command (wrapper), one wrapper around a
        // verdict, and one ID that is not in the context at all.
        report["command_evidence_ids"] =
            json!(["cmd-direct", "carry-cmd", "carry-verdict", "unknown"]);
        std::fs::write(
            root.join("validation-report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        let mut initial: Value =
            serde_json::from_str(include_str!("../data/configs/minimal.json")).unwrap();
        initial["artifact_root"] = json!(root);
        let graph = crate::workflow::describe_workflow(Some(&initial)).unwrap();
        let wrapper = |id: &str, origin: &str, n| {
            typed_record(
                id,
                "evidence-applicability",
                n,
                json!({
                    "origin":{"kind":"context-record","id":origin},
                    "target":{"subject":"validation-report.json","revision":report["revision"],
                              "checkpoint":{"phase":"validation","report_revision":report["revision"]}},
                    "reason":"unchanged command obligation"
                }),
            )
        };
        let records = vec![
            typed_record(
                "cmd-direct",
                "command-evidence",
                1,
                json!({"proof_id":"p1"}),
            ),
            typed_record("cmd-old", "command-evidence", 2, json!({"proof_id":"p2"})),
            wrapper("carry-cmd", "cmd-old", 3),
            typed_record(
                "old-verdict",
                "criterion-verdict",
                4,
                json!({"result":"pass"}),
            ),
            wrapper("carry-verdict", "old-verdict", 5),
            typed_record(
                "old-note",
                "user-steering",
                6,
                json!({"target":{"kind":"all"},"instruction":"old owner words"}),
            ),
            typed_record(
                "note",
                "user-steering",
                7,
                json!({"target":{"kind":"all"},"instruction":"withdraw the requirement","supersedes":["old-note"]}),
            ),
        ];
        let packet = json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change","artifact_root":root,"initial_input":initial,"context":records,"workflow_graph":graph,"work_slots":graph.work_slots}});
        for gate in ["validation-review", "validation-adversarial-review"] {
            let output = inspect_with_stage(&packet, Some(gate), None, Some("aggregate")).unwrap();
            let citable = &output["citable_evidence"];
            assert_eq!(
                citable["command_evidence_ids"],
                json!(["cmd-direct", "cmd-old"]),
                "{gate}: {citable}"
            );
            assert_eq!(
                citable["indexed_but_not_citable"],
                json!([{"indexed_id":"carry-cmd","cite_instead":"cmd-old"}]),
                "{gate}: {citable}"
            );
            // LE-201: the delivered, unsuperseded owner steering is citable too.
            assert_eq!(citable["user_steering_ids"], json!(["note"]), "{gate}");
            assert!(citable["rule"]
                .as_str()
                .unwrap()
                .contains("evidence-applicability record is never citable"));
        }
        // Reviews of other subjects cite no command evidence, so carry no such list.
        let design =
            inspect_with_stage(&packet, Some("design-review"), None, Some("aggregate")).unwrap();
        assert!(design.get("citable_evidence").is_none());
        std::fs::remove_dir_all(root).unwrap();
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
    fn challenge_preparation_discloses_partial_and_complete_parent_claims() {
        let root = std::env::temp_dir().join(format!(
            "software-change-partial-parent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("intent.json"), br#"{"revision":"1"}"#).unwrap();
        let bytes = br#"{"revision":"4"}"#;
        std::fs::write(root.join("design.json"), bytes).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let parent = typed_record(
            "parent-a",
            "review-evidence",
            1,
            json!({
                "gate":"design-review","subject":"design.json","subject_revision":"4",
                "review_stage":"aggregate","policy_id":"a","author":{"name":"A","kind":"agent"},
                "result":"pass","findings":"","review_contract_version":2,
                "grounds":{"reason":"inspected design","evidence":[{"locator":"design.json#/revision","sha256":digest}]}
            }),
        );
        let slot = WorkSlot::new(
            "design-adversarial-review",
            "design-adversarial-review",
            "approved",
        )
        .with_stdin_context_kinds(vec!["review-evidence".into()]);
        let mut input = json!({"status":"completed","result":{
            "artifact_root":root,"work_slots":[slot],"context":[parent],
            "initial_input":{"review_policies":{"design-adversarial-review":[
                {"id":"a","group":"design"},{"id":"b","group":"design"},{"id":"c","group":"design"}
            ]}}
        }});
        let before = input.clone();
        let output =
            inspect_with_stage(&input, Some("design-adversarial-review"), None, None).unwrap();
        assert_eq!(input, before);
        assert_eq!(output["missing_inputs"].as_array().unwrap().len(), 2);
        assert!(output["missing_inputs"][0]
            .as_str()
            .unwrap()
            .contains("`b`"));
        assert!(output["missing_inputs"][1]
            .as_str()
            .unwrap()
            .contains("`c`"));
        assert_eq!(output["context"][0]["id"], "parent-a");
        for axis in ["b", "c"] {
            let mut record = input["result"]["context"][0].clone();
            record["id"] = json!(format!("parent-{axis}"));
            record["data"]["policy_id"] = json!(axis);
            record["sequence"] = json!(if axis == "b" { 2 } else { 3 });
            input["result"]["context"]
                .as_array_mut()
                .unwrap()
                .push(record);
        }
        let output =
            inspect_with_stage(&input, Some("design-adversarial-review"), None, None).unwrap();
        assert_eq!(output["missing_inputs"], json!([]));
        assert_eq!(output["context"].as_array().unwrap().len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    // ---- AC-1 / AC-2 / AC-8: carried parent aggregates, named causes, locator guidance ----

    struct CarriedParent {
        root: std::path::PathBuf,
        input: Value,
    }

    impl CarriedParent {
        /// A design-review aggregate for revision 4 (axes a, b, c, one reviewer, one
        /// assessment) carried by explicit applicability to the unchanged design revision 5.
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "software-change-carried-parent-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("intent.json"), br#"{"revision":"1"}"#).unwrap();
            std::fs::write(
                root.join("design.json"),
                br#"{"revision":"5","author":{"name":"driver","kind":"agent"},"elements":[{"name":"x"},{"name":"y"}],"decisions":[]}"#,
            )
            .unwrap();
            let mut context = Vec::new();
            for (n, axis) in ["a", "b", "c"].iter().enumerate() {
                context.push(typed_record(
                    &format!("parent-{axis}"),
                    "review-evidence",
                    n as u64 + 1,
                    json!({
                        "gate":"design-review","subject":"design.json","subject_revision":"4",
                        "review_stage":"aggregate","policy_id":axis,"group":"design",
                        "assessment_id":"original-return","config_version":"carry-16",
                        "author":{"name":"A","kind":"agent"},"result":"pass","findings":"",
                        "review_contract_version":3,
                        "grounds":{"reason":"inspected design","evidence":[{"locator":"design.json#/elements"}]}
                    }),
                ));
            }
            for (n, axis) in ["a", "b", "c"].iter().enumerate() {
                context.push(typed_record(
                    &format!("carry-{axis}"),
                    "evidence-applicability",
                    n as u64 + 4,
                    json!({
                        "origin":{"kind":"context-record","id":format!("parent-{axis}")},
                        "target":{"subject":"design.json","revision":"5"},
                        "attesting_driver":{"name":"driver","kind":"agent"},
                        "reason":"design revision 5 changed only the revision field"
                    }),
                ));
            }
            context.push(typed_record(
                "ledger",
                "finding-ledger",
                7,
                json!({"schema_version":"1","gate":"design-review","subject":"design.json",
                       "subject_revision":"5","author":{"name":"driver","kind":"agent"},"findings":[]}),
            ));
            let axes = |_challenge: bool| -> Vec<Value> {
                ["a", "b", "c"]
                    .iter()
                    .map(|axis| {
                        json!({"id":axis,"description":axis,"group":"design","review_stage":"aggregate"})
                    })
                    .collect()
            };
            let slot = WorkSlot::new(
                "design-adversarial-review",
                "design-adversarial-review",
                "approved",
            )
            .with_stdin_context_kinds(vec![
                "review-evidence".into(),
                "evidence-applicability".into(),
                "finding-ledger".into(),
            ]);
            let input = json!({"status":"completed","result":{
                "artifact_root":root,"work_slots":[slot],"context":context,
                "initial_input":{
                    "contract_version":3,
                    "criterion_policy":{"required_authors":1,"goal_required_authors":1},
                    "config_version":"carry-16",
                    "artifact_schemas":{"design.json":{
                        "type":"object",
                        "properties":{
                            "revision":{"type":"string"},
                            "author":{"type":"object",
                                "properties":{"name":{"type":"string"},
                                    "kind":{"type":"string","enum":["human","agent","script"]}},
                                "required":["name","kind"],"additionalProperties":false}
                        },
                        "required":["revision","author"],"additionalProperties":false
                    }},
                    "review_policies":{
                        "design-review":axes(false),
                        "design-adversarial-review":axes(true)
                    }
                }
            }});
            Self { root, input }
        }

        fn commission(&self) -> Value {
            inspect_with_stage(&self.input, Some("design-adversarial-review"), None, None).unwrap()
        }

        fn context_mut(&mut self) -> &mut Vec<Value> {
            self.input["result"]["context"].as_array_mut().unwrap()
        }
    }

    impl Drop for CarriedParent {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn carried_complete_parent_aggregate_yields_a_labeled_packet() {
        let fixture = CarriedParent::new("labeled");
        let output = fixture.commission();
        assert_eq!(output["missing_inputs"], json!([]), "{output}");
        let parent = &output["parent_review"];
        assert_eq!(parent["eligible"], true, "{parent}");
        let parts = parent["parts"].as_array().unwrap();
        let labels = |label: &str| parts.iter().filter(|p| p["label"] == label).count();
        // Three original judgments and the three attestations that carried them;
        // nothing carried is presented as a fresh review.
        assert_eq!(labels("original judgment"), 3);
        assert_eq!(labels("current applicability attestation"), 3);
        assert_eq!(labels("fresh review"), 0);
        let original = parts.iter().find(|p| p["record_id"] == "parent-a").unwrap();
        assert_eq!(original["author"], json!({"name":"A","kind":"agent"}));
        assert_eq!(original["group"], "design");
        assert_eq!(original["assessment_id"], "original-return");
        assert_eq!(original["originally_for_revision"], "4");
        assert_eq!(original["grounds"]["reason"], "inspected design");
        let attestation = parts.iter().find(|p| p["record_id"] == "carry-a").unwrap();
        assert_eq!(attestation["target"]["revision"], "5");
        assert_eq!(attestation["origin_id"], "parent-a");
        assert_eq!(attestation["attesting_driver"]["name"], "driver");
    }

    #[test]
    fn carried_parent_that_cannot_supply_a_packet_names_the_unmet_prerequisite() {
        // One refusing history per cause, each checked for the cause named.
        type Mutation = fn(&mut CarriedParent);
        let cases: [(&str, Mutation, &str, &str); 4] = [
            (
                "missing-member",
                |f| f.context_mut().retain(|r| r["id"] != "carry-c"),
                "missing",
                "axis @c@",
            ),
            (
                "wrong-target",
                |f| {
                    let carry = f
                        .context_mut()
                        .iter_mut()
                        .find(|r| r["id"] == "carry-b")
                        .unwrap();
                    carry["data"]["target"]["revision"] = json!("9");
                },
                "unverified",
                "axis @b@",
            ),
            (
                "invalid-source",
                |f| {
                    let carry = f
                        .context_mut()
                        .iter_mut()
                        .find(|r| r["id"] == "carry-a")
                        .unwrap();
                    carry["data"]["origin"]["id"] = json!("no-such-record");
                },
                "unverified",
                "axis @a@",
            ),
            (
                "assessment-mismatch",
                |f| {
                    // Axis a stays carried; b and c are re-judged freshly by one
                    // reviewer under two different assessments.
                    f.context_mut()
                        .retain(|r| r["id"] != "carry-b" && r["id"] != "carry-c");
                    for (n, (axis, assessment)) in
                        [("b", "return-1"), ("c", "return-2")].iter().enumerate()
                    {
                        let mut fresh = f
                            .context_mut()
                            .iter()
                            .find(|r| r["id"] == "parent-a")
                            .unwrap()
                            .clone();
                        fresh["id"] = json!(format!("fresh-{axis}"));
                        fresh["sequence"] = json!(20 + n);
                        fresh["data"]["subject_revision"] = json!("5");
                        fresh["data"]["policy_id"] = json!(axis);
                        fresh["data"]["assessment_id"] = json!(assessment);
                        f.context_mut().push(fresh);
                    }
                },
                "assessment_inconsistent",
                "different assessments",
            ),
        ];
        for (name, mutate, category, text) in cases {
            let mut fixture = CarriedParent::new(name);
            mutate(&mut fixture);
            let output = fixture.commission();
            let missing = output["missing_inputs"].to_string();
            let text = text.replace('@', "\x60");
            assert!(
                missing.contains(&format!("({category})")) && missing.contains(&text),
                "{name}: {missing}"
            );
            assert!(
                !missing.contains(
                    "is missing current parent aggregate claim @a@"
                        .replace('@', "\x60")
                        .as_str()
                ),
                "{name}: must not imply a fresh review is always needed: {missing}"
            );
        }
        // An accepted finding that is still unresolved.
        let mut fixture = CarriedParent::new("unresolved");
        let mut failing = fixture
            .context_mut()
            .iter()
            .find(|r| r["id"] == "parent-a")
            .unwrap()
            .clone();
        failing["id"] = json!("fail-a");
        failing["sequence"] = json!(30);
        failing["data"]["subject_revision"] = json!("5");
        failing["data"]["result"] = json!("fail");
        failing["data"]["findings"] = json!("a defect the driver accepted");
        failing["data"]["author"] = json!({"name":"B","kind":"agent"});
        failing["data"]["assessment_id"] = json!("second-opinion");
        fixture.context_mut().push(failing);
        let ledger = fixture
            .context_mut()
            .iter_mut()
            .find(|r| r["id"] == "ledger")
            .unwrap();
        ledger["sequence"] = json!(40);
        ledger["data"]["findings"] = json!([{
            "id":"F-1","source":{"kind":"context-record","id":"fail-a"},"policy_id":"a",
            "summary":"open","disposition":"accepted","reason":"valid","owner_phase":"design",
            "task_ids":[],"review_axes":["a"],"status":"unresolved"
        }]);
        let missing = fixture.commission()["missing_inputs"].to_string();
        assert!(
            missing.contains("(unresolved_finding)") && missing.contains("F-1"),
            "{missing}"
        );
    }

    #[test]
    fn review_packets_state_locator_forms_roots_and_the_real_structure() {
        let fixture = CarriedParent::new("guidance");
        let output = fixture.commission();
        let guidance = &output["locator_guidance"];
        assert_eq!(guidance["accepted_forms"].as_array().unwrap().len(), 4);
        assert_eq!(
            guidance["resolution"]["artifact_directory"],
            json!(fixture.root)
        );
        assert!(guidance["resolution"]["rule"]
            .as_str()
            .unwrap()
            .contains("never depends"));
        let design = guidance["json_sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["locator"] == "design.json")
            .unwrap();
        // Matches the actual file: keys revision, author, elements (2), decisions (0).
        let keys: Vec<_> = design["top_level_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), 4);
        for key in ["revision", "author", "elements", "decisions"] {
            assert!(keys.contains(&key), "{key}");
        }
        assert_eq!(design["array_lengths"]["/elements"], 2);
        assert_eq!(design["array_lengths"]["/decisions"], 0);
    }

    #[test]
    fn recovery_challenge_diagnoses_absent_parent_aggregate_ground() {
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
        let records = vec![parent];
        let (selected, diagnostics) = project_review_context_with_diagnostics(
            &records,
            &records,
            "design-adversarial-review",
            "design.json",
            "4",
            None,
            Path::new("/unused-for-design"),
        )
        .unwrap();
        assert!(selected.is_empty());
        assert!(json!(diagnostics)
            .to_string()
            .contains("supported versioned grounds"));
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
        let view = owner_approval_view(
            &intent,
            &sha,
            &[&owner],
            &[owner.clone(), reviewer, driver],
            &json!({}),
        );
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
    fn successor_size_information_never_refuses_large_sources() {
        let sources = vec![ReviewSource {
            locator: "intent.json".to_owned(),
            sha256: "sha256:fixture".to_owned(),
            bytes: vec![b'x'; 300_000],
            value: json!({}),
        }];
        let info = commission_size_info(&[], &sources, "/fixture/root").unwrap();
        assert_eq!(info["retrieved_original_bytes"], 300_000);
        assert!(info["status"]
            .as_str()
            .unwrap()
            .contains("no content quota"));
        assert!(info.get("initial_limit_bytes").is_none());
        assert!(info.get("calls").is_none());
    }

    fn narrative_fixture(phase: &str, nested: bool) -> (Value, std::path::PathBuf) {
        // Wall-clock timestamps can repeat within this fixture's fast loop.
        // Allocate atomically; never reuse or clear a directory from another call.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = loop {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = std::env::temp_dir().join(format!(
                "commission-narrative-{}-{serial}",
                std::process::id(),
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("allocate narrative fixture: {error}"),
            }
        };
        let mut profile: Value =
            serde_json::from_str(include_str!("../data/configs/high-rigor.json")).unwrap();
        profile["artifact_root"] = json!(root);
        profile["extra"]["narratives"] = json!([phase]);
        profile["extra"]["checkpoints"] =
            json!(["intent", "design", "plan", "implementation", "validation"]);
        for (pass, suffix) in [("ordinary", "review"), ("challenge", "adversarial-review")] {
            let mut axes = profile["extra"]["narrative_review"][pass].clone();
            for axis in axes.as_array_mut().unwrap() {
                axis["group"] = json!(axis["group"].as_str().unwrap().replace("{phase}", phase));
            }
            profile["review_policies"][format!("{phase}-narrative-{suffix}")] = axes;
        }
        let options = crate::workflow::PHASE_OPTIONS
            .iter()
            .find(|o| o.phase.name == phase)
            .unwrap();
        let n = options.narrative.unwrap();
        std::fs::write(
            root.join("intent.json"),
            br#"{"revision":"1","outcome":"frozen outcome"}"#,
        )
        .unwrap();
        let artifact = br#"{"revision":"3","author":{"name":"writer","kind":"agent"}}"#;
        std::fs::write(root.join(options.phase.subject), artifact).unwrap();
        let markdown = b"# Proposed outcome\nThe approved artifact describes a bounded change.\n";
        std::fs::write(root.join(n.markdown), markdown).unwrap();
        let artifact_hash = format!("{:x}", Sha256::digest(artifact));
        let markdown_hash = format!("{:x}", Sha256::digest(markdown));
        let binding = json!({"revision":"3/1","author":{"name":"writer","kind":"agent"},
            "artifact":{"subject":options.phase.subject,"revision":"3","sha256":artifact_hash},
            "approved_artifact_record_id":if nested {"admission"} else {"approval"},
            "markdown":{"file":n.markdown,"sha256":markdown_hash}});
        std::fs::write(root.join(n.subject), serde_json::to_vec(&binding).unwrap()).unwrap();
        let approval = binding["artifact"].clone();
        let mut admission = approval.clone();
        admission["phase"] = json!(phase);
        if nested {
            admission["approved_artifact"] = approval.clone();
        }
        let mut context = vec![typed_record("admission", "artifact-admitted", 1, admission)];
        if !nested {
            context.push(typed_record("approval", "approved-artifact", 2, approval));
        }
        context.push(typed_record(
            "narrative",
            "narrative-admitted",
            3,
            json!({"phase":phase,"revision":"3/1","sha256":markdown_hash}),
        ));
        let graph =
            serde_json::to_value(crate::workflow::describe_workflow(Some(&profile)).unwrap())
                .unwrap();
        (
            json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change",
            "artifact_root":root,"initial_input":profile,"context":context,"work_slots":graph["work_slots"]}}),
            root,
        )
    }

    #[test]
    fn non_intent_reviews_deliver_only_effective_targeted_steering_and_real_ids() {
        let root = std::env::temp_dir().join(format!(
            "commission-steering-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for name in [
            "intent",
            "design",
            "plan",
            "implementation-report",
            "validation-report",
        ] {
            std::fs::write(
                root.join(format!("{name}.json")),
                std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join(format!("data/calibration/fixtures/{name}-good.json")),
                )
                .unwrap(),
            )
            .unwrap();
        }
        let mut initial: Value =
            serde_json::from_str(include_str!("../data/configs/minimal.json")).unwrap();
        initial["artifact_root"] = json!(root);
        let graph = crate::workflow::describe_workflow(Some(&initial)).unwrap();
        let records = vec![
            typed_record(
                "old",
                "user-steering",
                1,
                json!({"target":{"kind":"all"},"instruction":"old"}),
            ),
            typed_record(
                "all-current",
                "user-steering",
                2,
                json!({"target":{"kind":"all"},"instruction":"current","supersedes":["old"]}),
            ),
            typed_record(
                "design-only",
                "user-steering",
                3,
                json!({"target":{"kind":"slots","ids":["design-review"]},"instruction":"design"}),
            ),
            typed_record(
                "unrelated",
                "user-steering",
                4,
                json!({"target":{"kind":"slots","ids":["intent-review"]},"instruction":"intent"}),
            ),
        ];
        let packet = json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change","artifact_root":root,"initial_input":initial,"context":records,"workflow_graph":graph,"work_slots":graph.work_slots}});
        for gate in [
            "design-review",
            "plan-review",
            "implementation-review",
            "validation-review",
        ] {
            let output = inspect_with_stage(&packet, Some(gate), None, Some("aggregate")).unwrap();
            let ids = output["context"]
                .as_array()
                .unwrap()
                .iter()
                .map(|record| record["id"].as_str().unwrap())
                .collect::<BTreeSet<_>>();
            assert!(ids.contains("all-current"));
            assert!(!ids.contains("old"));
            assert!(!ids.contains("unrelated"));
            assert_eq!(ids.contains("design-only"), gate == "design-review");
            assert!(output["review_record_sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["record_id"] == "all-current"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn narrative_commissions_expose_four_accessible_sources_and_composite_identity() {
        for phase in ["design", "plan", "validation"] {
            for nested in [false, true] {
                let (input, root) = narrative_fixture(phase, nested);
                for stage in ["individual", "aggregate"] {
                    let gate = format!("{phase}-narrative-review");
                    let before = input.clone();
                    let output =
                        inspect_with_stage(&input, Some(&gate), None, Some(stage)).unwrap();
                    assert_eq!(input, before);
                    assert_eq!(output["missing_inputs"], json!([]), "{output}");
                    assert_eq!(output["commission"]["review_stage"], stage);
                    assert_eq!(output["review_target"]["revision"], "3/1");
                    assert_eq!(output["review_target"]["phase"], phase);
                    assert_eq!(output["review_sources"].as_array().unwrap().len(), 4);
                    for source in output["review_sources"].as_array().unwrap() {
                        let bytes = std::fs::read(source["path"].as_str().unwrap()).unwrap();
                        assert_eq!(
                            source["sha256"],
                            format!("sha256:{:x}", Sha256::digest(&bytes))
                        );
                    }
                    assert_eq!(
                        output["approved_artifact"]["record_id"],
                        if nested { "admission" } else { "approval" }
                    );
                    assert_eq!(
                        output["approved_artifact"]["record"]["kind"],
                        if nested {
                            "artifact-admitted"
                        } else {
                            "approved-artifact"
                        }
                    );
                }
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn narrative_challenge_uses_only_current_parent_aggregate_grounds() {
        let (mut input, root) = narrative_fixture("design", false);
        let parent_gate = "design-narrative-review";
        for (i, axis) in [
            "narrative-faithful",
            "narrative-intent-aligned",
            "narrative-understandable",
        ]
        .into_iter()
        .enumerate()
        {
            let policy = input["result"]["initial_input"]["review_policies"][parent_gate]
                .as_array()
                .unwrap()
                .iter()
                .find(|policy| policy["id"] == axis && policy["review_stage"] == "aggregate")
                .unwrap();
            let data = json!({"gate":parent_gate,"subject":"design-narrative.json","subject_revision":"3/1",
                "config_version":input["result"]["initial_input"]["config_version"],"group":policy["group"],"assessment_id":"scripted-parent-assessment",
                "review_stage":"aggregate","policy_id":axis,"author":{"name":"reviewer","kind":"agent"},
                "result":"pass","findings":"","review_contract_version":3,
                "grounds":{"reason":"inspected exact sources","evidence":[{"locator":"design-narrative.md#L1-L2"}]}});
            input["result"]["context"]
                .as_array_mut()
                .unwrap()
                .push(json!(typed_record(
                    &format!("parent-{i}"),
                    "review-evidence",
                    4 + i as u64,
                    data
                )));
        }
        let gate = "design-narrative-adversarial-review";
        for stage in ["individual", "aggregate"] {
            let output = inspect_with_stage(&input, Some(gate), None, Some(stage)).unwrap();
            assert_eq!(output["missing_inputs"], json!([]));
            assert_eq!(output["context"].as_array().unwrap().len(), 3);
            assert_eq!(output["context"][0]["id"], "parent-0");
        }
        input["result"]["context"].as_array_mut().unwrap().pop();
        let output = inspect_with_stage(&input, Some(gate), None, Some("aggregate")).unwrap();
        assert!(output["missing_inputs"]
            .to_string()
            .contains("narrative-understandable"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkpoint_view_uses_admission_identity_not_mutated_binding_revision() {
        let (input, root) = narrative_fixture("design", true);
        let output = inspect(&input, Some("design-checkpoint"), None).unwrap();
        let view = &output["owner_approval_view"];
        assert_eq!(view["subject"], "design-narrative.json");
        assert_eq!(view["subject_revision"], "3/1");
        assert_eq!(view["subject_record_id"], "narrative");
        assert_eq!(view["narrative_source"]["locator"], "design-narrative.md");
        assert_eq!(view["owner_approval"]["status"], "not-performed");
        assert!(view["decision_guidance"]
            .as_str()
            .unwrap()
            .contains("verbatim"));
        let mut binding: Value =
            serde_json::from_slice(&std::fs::read(root.join("design-narrative.json")).unwrap())
                .unwrap();
        binding["revision"] = json!("3/2");
        std::fs::write(
            root.join("design-narrative.json"),
            serde_json::to_vec(&binding).unwrap(),
        )
        .unwrap();
        let output = inspect(&input, Some("design-checkpoint"), None).unwrap();
        assert_eq!(output["owner_approval_view"]["subject_revision"], "3/1");
        assert!(output["missing_inputs"]
            .to_string()
            .contains("latest admitted narrative revision"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_views_project_the_model_bindings_fold() {
        let binding = |model: &str| json!({"model": model, "thinking": "high"});
        let amendment = typed_record(
            "amend-1",
            "model-binding-amendment",
            90,
            json!({
                "actor": {"name": "owner", "kind": "human"},
                "authority": {"kind": "owner"},
                "words": "Use the new model.",
                "changes": [{"role": "ordinary-reviewer", "old": binding("p/old"), "new": binding("p/new"), "mark_stale": true}],
            }),
        );
        let in_force_of = |view: &Value, role: &str| {
            view["model_bindings"]["roles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["role"] == role)
                .unwrap()["in_force"]
                .clone()
        };
        // Checkpoint owner view: never-bound, then frozen input plus amendment.
        let (mut input, root) = narrative_fixture("design", true);
        let output = inspect(&input, Some("design-checkpoint"), None).unwrap();
        let view = &output["owner_approval_view"];
        assert_eq!(view["model_bindings"]["state"], "never-bound");
        assert!(view["model_bindings"]["statement"].is_string());
        input["result"]["initial_input"]["model_bindings"] =
            json!({"roles": {"ordinary-reviewer": binding("p/old")}});
        input["result"]["context"]
            .as_array_mut()
            .unwrap()
            .push(json!(amendment));
        let output = inspect(&input, Some("design-checkpoint"), None).unwrap();
        let view = &output["owner_approval_view"];
        assert_eq!(view["model_bindings"]["state"], "bound");
        assert_eq!(in_force_of(view, "ordinary-reviewer"), binding("p/new"));
        assert_eq!(view["model_bindings"]["history"][0]["record_id"], "amend-1");
        assert_eq!(
            view["model_bindings"]["history"][0]["words"],
            "Use the new model."
        );
        std::fs::remove_dir_all(root).unwrap();
        // Intent owner view.
        let intent = json!({"revision": "r1", "author": {"name": "drafter", "kind": "agent"}});
        let initial = json!({"model_bindings": {"roles": {"ordinary-reviewer": binding("p/old")}}});
        let view = owner_approval_view(
            &intent,
            "sha256:x",
            &[],
            std::slice::from_ref(&amendment),
            &initial,
        );
        assert_eq!(view["model_bindings"]["state"], "bound");
        assert_eq!(in_force_of(&view, "ordinary-reviewer"), binding("p/new"));
        let view = owner_approval_view(&intent, "sha256:x", &[], &[], &json!({}));
        assert_eq!(view["model_bindings"]["state"], "never-bound");
    }

    #[test]
    fn stale_marked_review_evidence_is_dropped_from_commission_with_its_cause() {
        use crate::recovery::test_support::*;
        let root = Root::new();
        let binding = |model: &str| json!({"model": model, "thinking": "high"});
        let amendment = |mark_stale: bool| {
            row(
                "amend-1",
                "model-binding-amendment",
                2,
                json!({
                    "actor": {"name": "owner", "kind": "human"},
                    "authority": {"kind": "owner"},
                    "words": "Use the new cold reader.",
                    "changes": [{"role": "cold-reader", "old": binding("p/old"), "new": binding("p/new"), "mark_stale": mark_stale}],
                }),
            )
        };
        let project = |mark_stale: bool| {
            let records = vec![
                review("old", 1, "reader", "pass"),
                amendment(mark_stale),
                review("replacement", 3, "new", "pass"),
            ];
            let filtered: Vec<_> = records
                .iter()
                .filter(|row| row.kind == "review-evidence")
                .cloned()
                .collect();
            let (projected, diagnostics) = project_review_context_with_diagnostics(
                &filtered,
                &records,
                "intent-review",
                "intent.json",
                "r1",
                Some("aggregate"),
                &root.path,
            )
            .unwrap();
            let ids: Vec<_> = projected.iter().map(|row| row.id.to_string()).collect();
            let gate_side = crate::recovery::excluded_record_ids(
                &records,
                "intent-review",
                "intent.json",
                Some(&root.value()),
            )
            .unwrap();
            (ids, diagnostics, gate_side)
        };
        let (ids, diagnostics, gate_side) = project(false);
        assert_eq!(ids, ["old", "replacement"]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(gate_side.is_empty());
        let (ids, diagnostics, gate_side) = project(true);
        assert_eq!(ids, ["replacement"]);
        assert_eq!(gate_side, BTreeSet::from(["old".to_owned()]));
        let text = json!(diagnostics).to_string();
        for needle in [
            "amend-1",
            "role `cold-reader`",
            "reader (agent)",
            "axis `intent-sufficient`",
        ] {
            assert!(text.contains(needle), "{needle} in {text}");
        }
        assert_eq!(diagnostics[0]["record_id"], "old");
    }

    #[test]
    fn artifact_checkpoint_views_keep_actual_nested_reconciliation_identity() {
        let (mut input, root) = narrative_fixture("design", false);
        input["result"]["initial_input"]["extra"]["narratives"] = json!([]);
        for phase in ["intent", "design", "plan", "implementation", "validation"] {
            let options = checkpoint_options(&format!("{phase}-checkpoint")).unwrap();
            let artifact = br#"{"revision":"4"}"#;
            std::fs::write(root.join(options.phase.subject), artifact).unwrap();
            let hash = format!("{:x}", Sha256::digest(artifact));
            let admission =
                json!({"phase":phase,"subject":options.phase.subject,"revision":"4","sha256":hash});
            let approval = json!({"subject":options.phase.subject,"revision":"4","sha256":hash});
            input["result"]["context"] = json!([typed_record(
                "real-reconciliation",
                "reconciliation-decision",
                1,
                json!({"revision":"reconciliation-r2","documents":{"intent":"1"},
                    "artifact_admitted":admission,"approved_artifact":approval})
            )]);
            let output = inspect(&input, Some(options.checkpoint), None).unwrap();
            assert_eq!(output["missing_inputs"], json!([]), "{output}");
            let view = &output["owner_approval_view"];
            assert_eq!(view["subject"], options.phase.subject);
            assert_eq!(view["subject_revision"], "4");
            assert_eq!(view["subject_record_id"], "real-reconciliation");
            assert_eq!(
                view["subject_identity"]["record"]["data"]["revision"],
                "reconciliation-r2"
            );
            assert_eq!(
                view["subject_identity"]["record"]["data"]["documents"]["intent"],
                "1"
            );
            assert!(view["narrative_source"].is_null());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn narrative_missing_and_stale_inputs_are_visible_without_claiming_approval() {
        for (file, replacement, expected) in [
            ("design-narrative.md", None, "unavailable"),
            ("design-narrative.json", None, "unavailable"),
            ("intent.json", None, "unavailable"),
            ("design.json", None, "unavailable"),
            (
                "design-narrative.md",
                Some("changed prose"),
                "markdown must match",
            ),
            (
                "design.json",
                Some(r#"{"revision":"3","changed":true}"#),
                "current artifact must match",
            ),
            ("design-narrative.json", Some("{}"), "narrative binding"),
        ] {
            let (input, root) = narrative_fixture("design", false);
            if let Some(bytes) = replacement {
                std::fs::write(root.join(file), bytes).unwrap();
            } else {
                std::fs::remove_file(root.join(file)).unwrap();
            }
            for slot in [
                "design-narrative-review",
                "design-narrative-adversarial-review",
                "design-checkpoint",
            ] {
                let output = inspect(&input, Some(slot), None).unwrap();
                assert!(
                    output["missing_inputs"].to_string().contains(expected),
                    "{file}: {output}"
                );
                if slot.ends_with("checkpoint") {
                    assert_eq!(
                        output["owner_approval_view"]["owner_approval"]["status"],
                        "not-performed"
                    );
                    assert_eq!(output["owner_approval_view"]["subject_revision"], "3/1");
                }
            }
            std::fs::remove_dir_all(root).unwrap();
        }
        let (mut input, root) = narrative_fixture("design", false);
        input["result"]["context"] = json!([]);
        let output = inspect(&input, Some("design-checkpoint"), None).unwrap();
        assert!(output["owner_approval_view"]["subject_revision"].is_null());
        assert!(output["missing_inputs"]
            .to_string()
            .contains("missing latest"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn options_off_and_extension_slots_preserve_existing_projection() {
        let (mut input, root) = narrative_fixture("design", false);
        input["result"]["work_slots"]
            .as_array_mut()
            .unwrap()
            .push(json!(WorkSlot::new(
                "extension-review",
                "extension-review",
                "approved"
            )
            .with_stdin_context_kinds(vec!["review-evidence".into()])));
        for slot in [
            "design-review",
            "intent-review",
            "design-draft",
            "extension-review",
        ] {
            let current = inspect_with_stage(&input, Some(slot), None, Some("aggregate")).unwrap();
            let prior =
                inspect_review_with_stage(&input, Some(slot), None, Some("aggregate")).unwrap();
            assert_eq!(current, prior);
            assert!(current.get("phase").is_none());
        }
        let mut filter = input["result"].clone();
        filter["slot_id"] = json!("design-narrative-review");
        let output = inspect(&filter, None, None).unwrap();
        assert_eq!(
            output,
            inspect_review_with_stage(&filter, None, None, None).unwrap()
        );
        assert!(output.get("record_ids").is_some());
        assert!(output.get("review_sources").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_challenge_packet_diagnoses_citation_drift_and_requires_replacement_floor() {
        use crate::recovery::test_support::{review, Root};
        let root = Root::new();
        let mut source = review("parent-drifted", 1, "parent-reader", "pass");
        source.data["gate"] = json!("design-review");
        source.data["subject"] = json!("design.json");
        source.data["policy_id"] = json!("axis");
        let slot = WorkSlot::new(
            "design-adversarial-review",
            "design-adversarial-review",
            "approved",
        )
        .with_stdin_context_kinds(vec![
            "review-evidence".into(),
            "finding-ledger".into(),
            "evidence-applicability".into(),
        ]);
        let mut input = json!({"status":"completed","result":{"work_slots":[slot],"initial_input":root.input(1),"context":[source]}});
        std::fs::write(root.path.join("source.txt"), "first\n").unwrap();
        let output = inspect_with_stage(
            &input,
            Some("design-adversarial-review"),
            None,
            Some("aggregate"),
        )
        .unwrap();
        assert_eq!(
            output["record_diagnostics"][0]["record_id"],
            "parent-drifted"
        );
        assert!(!output["record_diagnostics"][0]["reason"]
            .as_str()
            .unwrap()
            .is_empty());
        assert!(!output["missing_inputs"].as_array().unwrap().is_empty());
        let records: Vec<ContextRecord> =
            serde_json::from_value(input["result"]["context"].clone()).unwrap();
        let config = crate::config::validate_config(&root.input(1)).unwrap();
        let evaluate = |records: &[ContextRecord]| {
            crate::evidence::evaluate_staged_evidence_with_dispositions(
                records,
                "design-review",
                "design.json",
                "r1",
                &crate::evidence::AuthorIdentity::new("drafter", "agent"),
                "recovery-1",
                config.staged_axes_for_gate("design-review").unwrap(),
                config.axis_namespace(),
                Some(&root.value()),
                None,
                false,
            )
        };
        assert!(!evaluate(&records).is_satisfied());
        let mut corrected = records[0].clone();
        corrected.id = "parent-replacement".into();
        corrected.sequence = 2_u64.into();
        corrected.data["grounds"]["evidence"][0]["locator"] = json!("design.json#/revision");
        input["result"]["context"]
            .as_array_mut()
            .unwrap()
            .push(json!(corrected));
        let records: Vec<ContextRecord> =
            serde_json::from_value(input["result"]["context"].clone()).unwrap();
        assert!(
            evaluate(&records).is_satisfied(),
            "later conforming evidence keeps existing correction behavior"
        );
        let corrected_packet = inspect_with_stage(
            &input,
            Some("design-adversarial-review"),
            None,
            Some("aggregate"),
        )
        .unwrap();
        assert_eq!(corrected_packet["missing_inputs"], json!([]));
        assert!(corrected_packet["context"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "parent-replacement"));
        assert!(corrected_packet["record_diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["record_id"] == "parent-drifted"));
    }

    #[test]
    fn recovery_commission_excludes_retired_and_withdrawn_records_with_full_history_available() {
        use crate::recovery::test_support::*;
        let root = Root::new();
        let source = review("old", 2, "reader", "pass");
        let mut malformed = review("malformed", 5, "reader", "pass");
        malformed.data["result"] = json!(9);
        let records = vec![
            roster("before", 1, &["reader"]),
            source.clone(),
            roster("after", 3, &["new"]),
            ledger(&source, 4, "retired-author", "recorded"),
            malformed,
            carry("carry", "malformed", 6, "r1"),
            review("replacement", 7, "new", "pass"),
        ];
        let filtered: Vec<_> = records
            .iter()
            .filter(|row| {
                matches!(
                    row.kind.as_str(),
                    "review-evidence" | "finding-ledger" | "evidence-applicability"
                )
            })
            .cloned()
            .collect();
        let (projected, diagnostics) = project_review_context_with_diagnostics(
            &filtered,
            &records,
            "intent-review",
            "intent.json",
            "r1",
            Some("aggregate"),
            &root.path,
        )
        .unwrap();
        assert!(diagnostics.is_empty(), "{:?}", diagnostics);
        assert_eq!(
            projected
                .iter()
                .filter(|row| row.kind == "review-evidence")
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["replacement"]
        );
        let history = records.clone();
        let mut withdrawn = vec![
            review("withdrawn", 1, "reader", "pass"),
            withdrawal("withdraw", "withdrawn", 2),
            review("replacement", 3, "new", "pass"),
        ];
        let (projected, _) = project_review_context_with_diagnostics(
            &withdrawn,
            &withdrawn,
            "intent-review",
            "intent.json",
            "r1",
            Some("aggregate"),
            &root.path,
        )
        .unwrap();
        assert!(!projected.iter().any(|row| row.id.as_str() == "withdrawn"));
        assert_eq!(records, history, "recovery never mutates original history");
        withdrawn[0].data["result"] = json!("fail");
        withdrawn[0].data["findings"] = json!("Finding");
        let pending = ledger(&withdrawn[0], 2, "accepted", "unresolved");
        withdrawn[1].sequence = 3_u64.into();
        withdrawn[2].sequence = 4_u64.into();
        withdrawn.insert(1, pending);
        let (projected, _) = project_review_context_with_diagnostics(
            &withdrawn,
            &withdrawn,
            "intent-review",
            "intent.json",
            "r1",
            Some("aggregate"),
            &root.path,
        )
        .unwrap();
        assert!(
            projected.iter().any(|row| row.kind == "finding-ledger"
                && row.data["findings"][0]["source"]["id"] == "withdrawn"),
            "unresolved accepted duty remains in packet independently of coverage exclusion"
        );
    }

    #[test]
    fn recovery_withdrawn_supplemental_command_does_not_remove_frozen_proof() {
        use crate::recovery::test_support::{row, withdrawal};
        let frozen = json!({"id":"frozen","command":"checker","args":[],"owner":"driver","obligation":"Frozen proof"});
        let plan = json!({"revision":"1","tasks":[],"proof_commands":[frozen.clone()]});
        let extra = row(
            "extra-command",
            "validation-command",
            1,
            json!({"spec":{"id":"extra","command":"other","args":[],"owner":"driver","obligation":"Supplemental proof"}}),
        );
        let slots = vec![WorkSlot::new(
            "validation-draft",
            "validation",
            "validation-ready",
        )];
        let before = select(
            std::slice::from_ref(&extra),
            &slots,
            "validation-draft",
            Some(&plan),
            None,
        )
        .unwrap();
        assert_eq!(before.proof_commands.len(), 2);
        let after = select(
            &[extra, withdrawal("withdraw", "extra-command", 2)],
            &slots,
            "validation-draft",
            Some(&plan),
            None,
        )
        .unwrap();
        assert_eq!(after.proof_commands, vec![frozen.clone()]);
        let root = crate::recovery::test_support::Root::new();
        std::fs::write(
            root.path.join("plan.json"),
            serde_json::to_vec(&plan).unwrap(),
        )
        .unwrap();
        let slot = WorkSlot::new("validation-draft", "validation", "validation-ready")
            .with_stdin_context_kinds(vec!["validation-command".into()]);
        let show = json!({"status":"completed","result":{"work_slots":[slot],"initial_input":root.input(1),
            "context":[row("extra-command","validation-command",1,json!({"spec":{"id":"extra","command":"other","args":[],"owner":"driver","obligation":"Supplemental proof"}})),withdrawal("withdraw","extra-command",2)]}});
        let inspected = inspect(&show, Some("validation-draft"), None).unwrap();
        assert_eq!(inspected["commission"]["proof_commands"], json!([frozen]));
        assert!(
            inspected["context"].as_array().unwrap().is_empty(),
            "removed command stays excluded even though withdrawal is not in frozen stdin kinds"
        );
        assert!(crate::recovery::record_withdrawal_guardrails("bad",&json!({"record_ids":["frozen"],"reason":"No longer needed","attesting_driver":{"name":"driver","kind":"agent"}}),&[],None).is_err());
    }
}
