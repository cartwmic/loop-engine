//! Intrinsic review/steering rules. Aggregation readiness is deliberately not
//! a record refusal, and these parsers never call the rule dispatcher.
use super::AdmissionContext;
use crate::{commission, config, evidence, finding_ledger};
use loop_core::{CandidateRecord, ContextRecord, SemanticSequence, Timestamp};
use serde_json::Value;

fn text<'a>(data: &'a Value, field: &str) -> Result<&'a str, String> {
    data[field]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty `{field}`"))
}
fn configuration(ctx: &AdmissionContext<'_>) -> Result<config::ValidatedConfig, String> {
    config::parse_initial_input(&ctx.effective_policy).map_err(|e| e.to_string())
}
fn subject(ctx: &AdmissionContext<'_>, gate: &str) -> Result<&'static str, String> {
    if ctx.effective_policy["review_policies"].get(gate).is_none() {
        return Err(format!("gate `{gate}` has no frozen review policy"));
    }
    commission::review_subject(gate).ok_or_else(|| format!("unknown review gate `{gate}`"))
}
fn root(ctx: &AdmissionContext<'_>) -> Option<Value> {
    ctx.artifact_root
        .as_ref()
        .map(|root| Value::String(root.to_string_lossy().into_owned()))
}
fn candidate(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<ContextRecord, String> {
    let sequence = ctx.context.last().map_or(Ok(1), |r| {
        r.sequence
            .as_u64()
            .checked_add(1)
            .ok_or("context sequence overflow")
    })?;
    Ok(ContextRecord::new(
        record.id.clone(),
        record.kind.clone(),
        record.data.clone(),
        SemanticSequence::new(sequence),
        Timestamp::from_unix_millis(0),
    ))
}

pub(super) fn steering(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let plan = ctx
        .artifact_root
        .as_ref()
        .map(|root| commission::load_plan(root))
        .transpose()?
        .flatten();
    commission::parse_steering(
        &record.data,
        ctx.context,
        &ctx.workflow.work_slots,
        plan.as_ref(),
    )
    .map(|_| ())
}
pub(super) fn incorporation(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    commission::validate_incorporation(&record.data, ctx.context)
}

fn policy_check(
    ctx: &AdmissionContext<'_>,
    parsed: &evidence::ConformingEvidence,
) -> Result<(), String> {
    let config = configuration(ctx)?;
    if parsed.config_version != config.config_version() {
        return Err("review config_version does not match frozen input".into());
    }
    let stages = config
        .staged_axes_for_gate(&parsed.gate)
        .ok_or("review gate has no configured axes")?;
    let axes = stages
        .iter()
        .find(|(stage, _)| stage.as_str() == parsed.review_stage)
        .map(|(_, axes)| axes)
        .ok_or("review stage is not configured on gate")?;
    let axis = axes
        .get(&parsed.policy_id)
        .ok_or("review axis is not configured on gate/stage")?;
    if let Some(group) = axis.group() {
        let members = axes
            .values()
            .filter(|axis| axis.group() == Some(group))
            .count();
        let grouped_profile = config
            .config_version()
            .rsplit_once('-')
            .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
            .is_some_and(|v| v >= 14);
        if (members > 1 || grouped_profile) && parsed.group.as_deref() != Some(group) {
            return Err("review group does not match frozen axis group".into());
        }
    }
    Ok(())
}
pub(super) fn assessment(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let config = configuration(ctx)?;
    let gate = text(&record.data, "gate")?;
    let stage = record
        .data
        .get("review_stage")
        .or_else(|| record.data.get("stage"))
        .and_then(Value::as_str)
        .unwrap_or("aggregate");
    let axis_id = text(&record.data, "policy_id")?;
    let axes = config
        .staged_axes_for_gate(gate)
        .and_then(|stages| {
            stages
                .iter()
                .find(|(key, _)| key.as_str() == stage)
                .map(|(_, axes)| axes)
        })
        .ok_or("review stage not configured")?;
    let axis = axes.get(axis_id).ok_or("review axis not configured")?;
    if let Some(group) = axis.group() {
        if axes
            .values()
            .filter(|axis| axis.group() == Some(group))
            .count()
            > 1
            && record.data["assessment_id"]
                .as_str()
                .is_none_or(|id| id.trim().is_empty())
        {
            return Err("multi-axis review group requires assessment_id".into());
        }
    }
    Ok(())
}

pub(super) fn review(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let gate = text(&record.data, "gate")?;
    let expected = subject(ctx, gate)?;
    let config = configuration(ctx)?;
    let artifact_root = root(ctx);
    let parsed = evidence::parse_conforming_with_context(
        record
            .data
            .as_object()
            .ok_or("review data must be an object")?,
        expected,
        artifact_root.as_ref(),
        config.contract_version() >= 3,
        ctx.context,
        false,
    )
    .map_err(evidence::EvidenceParseError::into_message)?;
    policy_check(ctx, &parsed)?;
    if let Some(root) = &ctx.artifact_root {
        if let Some(current) = std::fs::read(root.join(expected))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        {
            if current["revision"] == parsed.subject_revision {
                let direct = current["author"]
                    == serde_json::json!({"name":parsed.author.name(),"kind":parsed.author.kind()});
                let config = configuration(ctx)?;
                let axis = config
                    .staged_axes_for_gate(gate)
                    .and_then(|stages| {
                        stages
                            .iter()
                            .find(|(stage, _)| stage.as_str() == parsed.review_stage)
                            .and_then(|(_, axes)| axes.get(&parsed.policy_id))
                    })
                    .ok_or("review policy unavailable")?;
                evidence::validate_direct_review(
                    gate,
                    &parsed.policy_id,
                    axis.self_review(),
                    direct,
                )?;
            }
        }
    }
    Ok(())
}
/// New writes must resolve citations even if they claim a historical revision;
/// Classify retains the existing parser's historical shape-only behavior.
pub(super) fn write_citations(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let version = record.data["review_contract_version"]
        .as_u64()
        .map(|v| v as u32);
    if configuration(ctx)?.contract_version() >= 3 && version.is_none() {
        return Err(
            "new grounded review evidence requires review_contract_version 2 or 3 and grounds"
                .into(),
        );
    }
    if matches!(version, Some(2 | 3)) {
        super::validate_citations(
            &record.data["grounds"],
            ctx.artifact_root
                .as_deref()
                .ok_or("grounded review requires artifact_root")?,
            version,
            ctx.context,
        )?;
    }
    Ok(())
}
pub(super) fn applicability(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let shape = evidence::parse_applicability_shape(&record.data)
        .map_err(evidence::EvidenceParseError::into_message)?;
    let origin = shape.origin.id.as_str();
    let matches: Vec<_> = ctx
        .context
        .iter()
        .filter(|r| r.id.as_str() == origin)
        .collect();
    if matches.len() != 1 {
        return Err("applicability origin is missing or ambiguous".into());
    }
    let source = matches[0];
    let canonical = evidence::canonical_target(&shape.target, false)?;
    if matches!(
        source.kind.as_str(),
        "criterion-verdict" | "goal-verdict" | "command-evidence"
    ) {
        let artifact_root = root(ctx);
        if source.kind == "command-evidence" {
            crate::validation::command_shape(&source.data)?;
            if source.data["external_result"].is_null()
                || source.data["external_result"]["exit_code"] != 0
            {
                return Err(
                    "command applicability requires an earlier successful external result".into(),
                );
            }
        } else {
            crate::validation::historical_source_record(
                source,
                ctx.context,
                ctx.artifact_root
                    .as_deref()
                    .ok_or("applicability requires artifact_root")?,
            )?;
            if let Some(root) = ctx.artifact_root.as_deref() {
                crate::validation::carried_criterion_is_current(
                    source,
                    source.data.get("criterion_id").and_then(Value::as_str),
                    ctx.context,
                    std::path::Path::new(root),
                )?;
            }
        }
        evidence::validate_applicability_target(
            &canonical,
            "validation-report.json",
            text(&canonical, "revision")?,
            artifact_root.as_ref(),
            true,
        )?;
        return Ok(());
    }
    if source.kind != "review-evidence" {
        return Err("applicability origin is not a supported judgment".into());
    }
    let gate = text(&source.data, "gate")?;
    let expected = subject(ctx, gate)?;
    let revision = text(&record.data["target"], "revision")?;
    let config = configuration(ctx)?;
    let artifact_root = root(ctx);
    let stored = candidate(ctx, record)?;
    let parsed = evidence::parse_applicable_evidence_for_stage(
        ctx.context,
        &stored,
        expected,
        revision,
        artifact_root.as_ref(),
        config.contract_version() >= 3,
    )
    .map_err(evidence::EvidenceParseError::into_message)?;
    policy_check(ctx, &parsed)?;
    assessment(
        ctx,
        &CandidateRecord {
            id: source.id.clone(),
            kind: source.kind.clone(),
            data: source.data.clone(),
        },
    )
}
/// Currency is a new-write check, not a retroactive state/visit rule.
pub(super) fn write_target(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let shape = evidence::parse_applicability_shape(&record.data)
        .map_err(evidence::EvidenceParseError::into_message)?;
    let source = ctx
        .context
        .iter()
        .find(|source| source.id.as_str() == shape.origin.id)
        .ok_or("applicability origin missing")?;
    let expected = if source.kind == "review-evidence" {
        subject(ctx, text(&source.data, "gate")?)?
    } else {
        "validation-report.json"
    };
    let path = ctx
        .artifact_root
        .as_ref()
        .ok_or("applicability requires artifact_root")?
        .join(expected);
    let current: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let artifact_root = root(ctx);
    let canonical = evidence::canonical_target(&shape.target, false)?;
    evidence::validate_applicability_target(
        &canonical,
        expected,
        text(&current, "revision")?,
        artifact_root.as_ref(),
        false,
    )?;
    if source.kind == "review-evidence" {
        let config = configuration(ctx)?;
        let stored = candidate(ctx, record)?;
        let parsed = evidence::parse_applicable_evidence_for_stage(
            ctx.context,
            &stored,
            expected,
            text(&current, "revision")?,
            artifact_root.as_ref(),
            config.contract_version() >= 3,
        )
        .map_err(evidence::EvidenceParseError::into_message)?;
        let gate = text(&source.data, "gate")?;
        let stages = config
            .staged_axes_for_gate(gate)
            .ok_or("review policy unavailable")?;
        let axis = stages
            .iter()
            .find(|(stage, _)| stage.as_str() == parsed.review_stage)
            .and_then(|(_, axes)| axes.get(&parsed.policy_id))
            .ok_or("review policy unavailable")?;
        let direct = current["author"]
            == serde_json::json!({"name":parsed.author.name(),"kind":parsed.author.kind()});
        evidence::validate_direct_review(gate, &parsed.policy_id, axis.self_review(), direct)?;
    }
    if source.kind == "command-evidence" {
        crate::validation::command_evidence(
            &source.data,
            ctx.artifact_root
                .as_deref()
                .ok_or("artifact_root required")?,
            None,
            None,
            true,
            true,
        )?;
    }
    if source.kind == "criterion-verdict"
        && ctx
            .context
            .iter()
            .rev()
            .find(|r| {
                r.kind == "criterion-revalidation"
                    && r.data["subject_revision"] == current["revision"]
            })
            .is_some_and(|r| {
                r.data["affected_criteria"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&source.data["criterion_id"]))
            })
    {
        return Err("affected criteria require fresh verdicts".into());
    }
    Ok(())
}

pub(super) fn ledger(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let gate = text(&record.data, "gate")?;
    let expected = subject(ctx, gate)?;
    if text(&record.data, "subject")? != expected {
        return Err("ledger subject does not match gate".into());
    }
    let config = configuration(ctx)?;
    finding_ledger::validate_candidate(&candidate(ctx, record)?, ctx.context, &config)
}
pub(super) fn manifest(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    subject(ctx, text(&record.data, "gate")?)?;
    finding_ledger::parse_manifest(&record.data).map(|_| ())
}
pub(super) fn comparison(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    crate::intent_comparison::validate_candidate(&record.data, ctx.context)
}
pub(super) fn advisory(_: &AdmissionContext<'_>, _: &CandidateRecord) -> Result<(), String> {
    // No normative closed schema or validating consumer exists. Suggestions
    // are inert and are never selected as review/ledger/gate authority.
    Ok(())
}
pub(super) fn correction(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    crate::intent_correction::validate(&record.data, ctx.initial_input, ctx.context).map(|_| ())
}
pub(super) fn correction_current(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    let document = match crate::artifacts::read_artifact(
        ctx.initial_input.get("artifact_root"),
        "intent.json",
    ) {
        crate::artifacts::ArtifactReadOutcome::Present(document) => document,
        crate::artifacts::ArtifactReadOutcome::Deny(error) => return Err(error.to_string()),
        crate::artifacts::ArtifactReadOutcome::EvaluationError(error) => {
            return Err(error.to_string())
        }
    };
    if document.value()["revision"] != record.data["subject_revision"] {
        return Err("intent-correction target is not current".into());
    }
    Ok(())
}
