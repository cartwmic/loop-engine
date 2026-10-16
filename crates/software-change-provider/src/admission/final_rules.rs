//! Remaining existing validators. No readiness/author-floor inference.
use super::AdmissionContext;
use crate::{commission, criterion, narratives, record_facts, validation, workflow};
use loop_core::{CandidateRecord, ContextRecord, SemanticSequence, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
fn text<'a>(data: &'a Value, key: &str) -> Result<&'a str, String> {
    data[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty `{key}`"))
}
fn root<'a>(ctx: &'a AdmissionContext<'_>) -> Result<&'a Path, String> {
    ctx.artifact_root
        .as_deref()
        .ok_or_else(|| "record requires artifact_root".into())
}
fn document(
    ctx: &AdmissionContext<'_>,
    subject: &str,
) -> Result<crate::artifacts::ArtifactDocument, String> {
    match crate::artifacts::read_artifact(ctx.initial_input.get("artifact_root"), subject) {
        crate::artifacts::ArtifactReadOutcome::Present(doc) => Ok(doc),
        crate::artifacts::ArtifactReadOutcome::Deny(error) => Err(error.to_string()),
        crate::artifacts::ArtifactReadOutcome::EvaluationError(error) => Err(error.to_string()),
    }
}
fn read(ctx: &AdmissionContext<'_>, subject: &str) -> Result<Value, String> {
    document(ctx, subject).map(|doc| doc.value().clone())
}
fn stored(record: &CandidateRecord) -> ContextRecord {
    ContextRecord::new(
        record.id.clone(),
        record.kind.clone(),
        record.data.clone(),
        SemanticSequence::new(0),
        Timestamp::from_unix_millis(0),
    )
}

pub(super) fn verdict(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    validation::historical_source(&record.data, &record.kind, ctx.context, root(ctx)?)?;
    if record.data.get("gate").is_some()
        && (record.data["gate"] != "validation-review"
            || record.data["review_stage"] != "aggregate"
            || record.data["config_version"] != ctx.initial_input["config_version"])
    {
        return Err("verdict has incompatible gate/stage/frozen config".into());
    }
    Ok(())
}
pub(super) fn verdict_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    validation::source_for_candidate(
        &record.data,
        &record.kind,
        ctx.context,
        root(ctx)?,
        "validation-review",
        text(ctx.initial_input, "config_version")?,
    )?;
    if validation::verify_verdict_origin(&record.data, &record.kind)?
        .is_some_and(|id| id != record.id.as_str())
    {
        return Err("verdict record ID differs from captured prechosen ID".into());
    }
    let report = read(ctx, "validation-report.json")?;
    if record.data["subject_revision"] != report["revision"] {
        return Err("stale verdict target revision".into());
    }
    crate::checkpoint::current_target("validation-report.json", root(ctx)?)?;
    if record.kind == "criterion-verdict"
        && !criterion::intent_ids(&read(ctx, "intent.json")?)
            .contains(text(&record.data, "criterion_id")?)
    {
        return Err("verdict names unknown current criterion".into());
    }
    let references: Vec<String> =
        serde_json::from_value(record.data["evidence_context_ids"].clone())
            .map_err(|e| e.to_string())?;
    let indexed = report["command_evidence_ids"]
        .as_array()
        .ok_or("report has no command index")?;
    if references
        .iter()
        .any(|id| !indexed.iter().any(|value| value.as_str() == Some(id)))
    {
        return Err("fresh verdict must reference current indexed command evidence".into());
    }
    let policy = validation::contract(&ctx.effective_policy)?;
    let allow_self = if record.kind == "criterion-verdict" {
        policy.criterion_policy.self_review
    } else {
        policy.criterion_policy.goal_self_review
    };
    if !allow_self
        && (record.data["author"] == report["author"]
            || record.data["author"] == read(ctx, "implementation-report.json")?["author"])
    {
        return Err("self-authorship cannot satisfy criterion policy".into());
    }
    Ok(())
}
pub(super) fn command(_: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    validation::command_shape(&record.data)
}
fn effective_commands(ctx: &AdmissionContext<'_>) -> Result<Vec<commission::ProofCommand>, String> {
    let plan = read(ctx, "plan.json")?;
    let selected = commission::select(
        ctx.context,
        &ctx.workflow.work_slots,
        "validation-draft",
        Some(&plan),
        None,
    )?;
    validation::commands(&json!(selected.proof_commands))
}
pub(super) fn command_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let specs = effective_commands(ctx)?;
    let id = text(&record.data, "proof_id")?;
    let spec = specs
        .iter()
        .find(|spec| spec.id == id)
        .ok_or("unknown named command proof ID")?;
    validation::command_evidence(&record.data, root(ctx)?, Some(spec), None, false, false)
        .map(|_| ())
}
pub(super) fn validation_command(
    _: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let spec = commission::validation_command(&record.data)?;
    validation::commands(&json!([spec])).map(|_| ())
}
pub(super) fn additional_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let spec = commission::validation_command(&record.data)?;
    let plan = read(ctx, "plan.json")?;
    let selected = commission::select(
        ctx.context,
        &ctx.workflow.work_slots,
        "validation-draft",
        Some(&plan),
        None,
    )?;
    if selected
        .proof_commands
        .iter()
        .any(|existing| existing["id"] == spec.id)
    {
        return Err("validation-command duplicates/replaces an existing proof ID".into());
    }
    Ok(())
}
pub(super) fn revalidation(
    _: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    validation::revalidation(&record.data, None).map(|_| ())
}
pub(super) fn revalidation_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let known = criterion::intent_ids(&read(ctx, "intent.json")?);
    validation::revalidation(&record.data, Some(&known))?;
    if record.data["subject_revision"] != read(ctx, "validation-report.json")?["revision"] {
        return Err("stale revalidation target".into());
    }
    Ok(())
}
pub(super) fn owner(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let decision =
        loop_integrations::owner_checkpoint::parse(&record.data, &["artifact", "narrative"])?;
    let options = workflow::PHASE_OPTIONS
        .iter()
        .find(|options| options.checkpoint == decision.checkpoint)
        .ok_or("unknown owner checkpoint")?;
    let narrative =
        workflow::option_enabled(Some(ctx.initial_input), "narratives", options.phase.name);
    if decision.subject != workflow::checkpoint_subject(options, narrative)
        || (!narrative && decision.target.as_deref() == Some("narrative"))
    {
        return Err("owner decision subject/target does not match frozen checkpoint".into());
    }
    Ok(())
}
fn phase(subject: &str) -> Result<&'static workflow::PhaseOptions, String> {
    workflow::PHASE_OPTIONS
        .iter()
        .find(|options| options.phase.subject == subject)
        .ok_or_else(|| "unknown artifact subject".into())
}
pub(super) fn artifact(_: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let subject = text(&record.data, "subject")?;
    let options = phase(subject)?;
    let raw = stored(record);
    if !record_facts::validate_artifact_record(
        &raw,
        &record.data,
        &record.kind,
        options.phase.name,
        subject,
    ) {
        return Err(
            "artifact approval/admission fact is malformed or enclosing identities disagree".into(),
        );
    }
    Ok(())
}
pub(super) fn narrative(_: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let phase = text(&record.data, "phase")?;
    if !narratives::NARRATIVE_PHASES.contains(&phase)
        || !record_facts::valid_narrative_fact(&record.data, phase)
    {
        return Err("narrative admission fact is malformed".into());
    }
    Ok(())
}
pub(super) fn reconciliation(
    _: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let data = &record.data;
    let fields = [
        "revision",
        "sha256",
        "documents",
        "artifact_admitted",
        "approved_artifact",
    ];
    if data
        .as_object()
        .is_none_or(|o| o.keys().any(|key| !fields.contains(&key.as_str())))
    {
        return Err("invalid reconciliation enclosing shape".into());
    }
    text(data, "revision")?;
    let hash = text(data, "sha256")?;
    if hash.len() != 71
        || !hash.starts_with("sha256:")
        || !hash[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid reconciliation digest".into());
    }
    if data["documents"].as_object().is_none_or(|o| {
        o.len() != 3
            || o.keys()
                .any(|key| !["intent", "design", "plan"].contains(&key.as_str()))
    }) {
        return Err("reconciliation requires exact document identities".into());
    }
    for key in ["intent", "design", "plan"] {
        text(&data["documents"], key)?;
    }
    let raw = stored(record);
    for kind in [
        workflow::ARTIFACT_ADMITTED_KIND,
        workflow::APPROVED_ARTIFACT_KIND,
    ] {
        if let Some(fact) = narratives::phase_record_fact(&raw, kind) {
            if !record_facts::validate_artifact_record(
                &raw,
                fact,
                kind,
                "implementation",
                "implementation-report.json",
            ) {
                return Err(
                    "reconciliation enclosing artifact facts are malformed or disagree".into(),
                );
            }
        }
    }
    Ok(())
}
pub(super) fn artifact_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let raw = stored(record);
    for kind in [
        workflow::ARTIFACT_ADMITTED_KIND,
        workflow::APPROVED_ARTIFACT_KIND,
    ] {
        if let Some(fact) = narratives::phase_record_fact(&raw, kind) {
            let subject = text(fact, "subject")?;
            let doc = document(ctx, subject)?;
            if fact["revision"] != doc.value()["revision"]
                || fact["sha256"] != format!("{:x}", Sha256::digest(doc.bytes()))
            {
                return Err("artifact fact differs from current exact bytes/revision".into());
            }
        }
    }
    Ok(())
}
pub(super) fn reconciliation_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    artifact_current(ctx, record)?;
    let doc = document(ctx, "reconciliation.json")?;
    if record.data["revision"] != doc.value()["revision"]
        || record.data["sha256"] != format!("sha256:{:x}", Sha256::digest(doc.bytes()))
    {
        return Err("reconciliation identity differs from current bytes/revision".into());
    }
    for name in ["intent", "design", "plan"] {
        if record.data["documents"][name] != read(ctx, &format!("{name}.json"))?["revision"] {
            return Err("reconciliation document identity changed".into());
        }
    }
    Ok(())
}

pub(super) fn narrative_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let options = workflow::PHASE_OPTIONS
        .iter()
        .find(|options| options.phase.name == record.data["phase"].as_str().unwrap_or(""))
        .ok_or("unknown narrative phase")?;
    let n = options.narrative.ok_or("phase has no narrative")?;
    let value = read(ctx, n.subject)?;
    let binding = narratives::validate_binding(&value)
        .map_err(|e| format!("invalid narrative binding: {e:?}"))?;
    let (_, approval) = narratives::latest_phase_record(
        ctx.context,
        workflow::APPROVED_ARTIFACT_KIND,
        options.phase.name,
        options.phase.subject,
    )
    .ok_or("binding has no earlier approval source")?;
    let (approval_record, _) = narratives::latest_phase_record(
        ctx.context,
        workflow::APPROVED_ARTIFACT_KIND,
        options.phase.name,
        options.phase.subject,
    )
    .ok_or("binding has no earlier approval source")?;
    if !record_facts::valid_binding_source(
        &binding,
        approval_record,
        approval,
        options.phase.subject,
        n.markdown,
    ) {
        return Err("binding does not name the exact latest approved artifact".into());
    }
    let artifact = document(ctx, options.phase.subject)?;
    if artifact.value()["revision"] != binding.artifact.revision
        || format!("{:x}", Sha256::digest(artifact.bytes())) != binding.artifact.sha256
    {
        return Err("binding artifact bytes/revision changed".into());
    }
    let bytes =
        crate::artifacts::read_markdown_bytes(ctx.initial_input.get("artifact_root"), n.markdown)
            .map_err(|error| format!("markdown unavailable: {error:?}"))?;
    if record.data["revision"] != binding.revision
        || record.data["sha256"] != format!("{:x}", Sha256::digest(bytes))
    {
        return Err("narrative fact differs from exact binding/markdown".into());
    }
    Ok(())
}
