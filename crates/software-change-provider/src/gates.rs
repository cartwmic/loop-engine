//! Transition compatibility and complete provider evaluation.
//!
//! Topology and source-state duties come from the union phase table in
//! `workflow`. This module composes configuration, artifact, schema, and
//! evidence projections in technical-design §9's fixed order. A static
//! combinatorial table of every rewired target is not the source of topology.

use crate::artifacts::{
    check_revision_link, extract_metadata, read_artifact, ArtifactDocument, ArtifactReadDeny,
    ArtifactReadError, ArtifactReadOutcome, LinkCheckOutcome, LinkViolation,
};
use crate::checkpoint;
use crate::config::parse_initial_input;
use crate::criterion::{self, CriterionViolation};
use crate::evidence::AuthorIdentity as EvidenceAuthorIdentity;
use crate::finding_ledger::evaluate_finding_ledger;
use crate::overlay;
use crate::protocol::{allow_response, deny_response, unsupported_response, EvaluateRequest};
use crate::workflow::{self, TransitionDuties};
use bookends_check::CheckStatus;
use loop_core::{DurableEvaluationResult, TransitionKind};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fmt;

const SCHEMA_DENY_CODE: &str = "software-change-schema-invalid";
const EVIDENCE_DENY_CODE: &str = "software-change-review-incomplete";
const FINDING_LEDGER_DENY_CODE: &str = "software-change-finding-ledger-invalid";
const CHECKPOINT_DENY_CODE: &str = "software-change-checkpoint-invalid";
const RECONCILIATION_DENY_CODE: &str = "software-change-reconciliation-blocked";
const BOOKENDS_RED_DENY_CODE: &str = "software-change-bookends-red";
const BOOKENDS_CANDIDATE_DENY_CODE: &str = "software-change-bookends-candidate";
const SCHEMA_DENY_MESSAGE: &str = "not judged: fix shape first";
const EVIDENCE_DENY_MESSAGE: &str = "review evidence incomplete";
const FINDING_LEDGER_DENY_MESSAGE: &str = "finding ledger missing or malformed";
const CHECKPOINT_DENY_MESSAGE: &str = "repository checkpoint missing or stale";
const RECONCILIATION_DENY_MESSAGE: &str = "reconciliation result blocks progression";
const BOOKENDS_RED_DENY_MESSAGE: &str = "in-process bookends check is red";
const BOOKENDS_CANDIDATE_DENY_MESSAGE: &str =
    "provisional Bookends candidate blocks Bookends-enabled final completion";

/// Evaluation result consumed by `main.rs` for exit-code handling.
pub(crate) enum EvaluationOutcome {
    Response(Value),
    EvaluationError(String),
}

/// Evaluate one already-parsed request.  Request parsing and protocol errors
/// remain owned by `main.rs`; this function only returns result JSON or an
/// evaluation-error diagnostic.
pub(crate) fn evaluate(request: &EvaluateRequest) -> EvaluationOutcome {
    // Trust the snapshotted (source, event, target, kind) tuple for routing.
    // Duties come from the source state's phase-table row, not from a static
    // combinatorial ROUTES table of every rewired target.
    let Some(duties) = duties_for_snapshotted_transition(request) else {
        return EvaluationOutcome::Response(unsupported_response());
    };

    // §9.3: config validation is obligation-independent and artifact_root is
    // deliberately untouched by parse_initial_input. Overlay injection is
    // evaluate-time only and does not mutate the frozen initial_input.
    let overlay_on = overlay::enabled(&request.initial_input);
    let semantic_coverage = overlay::semantic_coverage_enabled(&request.initial_input);
    let initial_input = if overlay_on {
        overlay::apply(&request.initial_input)
    } else {
        request.initial_input.clone()
    };
    // Contract v2 remains inspectable by the engine and by historical helper
    // code, but this candidate provider must not reinterpret its semantic
    // obligations as v3. Return the protocol-level unsupported result for a
    // checked v2 evaluation before reading artifacts or context.
    if request.transition.kind == TransitionKind::Checked
        && initial_input["contract_version"].as_u64() == Some(2)
    {
        return EvaluationOutcome::Response(unsupported_response());
    }

    let config = match parse_initial_input(&initial_input) {
        Ok(config) => config,
        Err(error) => return EvaluationOutcome::EvaluationError(error.to_string()),
    };

    // Check-free edges are listed for complete compatibility but the provider
    // is not normally invoked for them. The v3 implementation handoff is a
    // checked engine boundary with no artifact prerequisite; reconciliation
    // owns the first post-edit result.
    let (subject, gate) = match duties {
        TransitionDuties::Checked { subject, gate } => (subject, gate),
        TransitionDuties::CheckedNoArtifact | TransitionDuties::CheckFree => {
            return EvaluationOutcome::Response(allow_response())
        }
    };

    let bookends_report = if overlay_on {
        match load_bookends_report() {
            Ok(report) => Some(report),
            Err(error) => return EvaluationOutcome::EvaluationError(error),
        }
    } else {
        None
    };

    let schema = config.schema(subject);
    let links = config.links_from(subject);
    let staged_axes = gate
        .and_then(|gate| config.staged_axes_for_gate(gate))
        .filter(|axes| !axes.is_empty());
    let checkpoint_phase = checkpoint::phase_for_subject(subject);
    let checkpoint_required = checkpoint_phase.is_some();

    // §9.4: no configured schema, link, or semantic axis means allow without
    // requiring artifact_root and without consulting context, except that
    // implementation and validation phases still require their independent
    // repository checkpoint.
    if schema.is_none() && links.is_empty() && staged_axes.is_none() && !checkpoint_required {
        if overlay_on && is_passed(request) && !bookends_is_green(bookends_report.as_ref()) {
            return EvaluationOutcome::Response(bookends_red_deny(
                request,
                bookends_report
                    .as_ref()
                    .expect("overlay-on loads the checker"),
            ));
        }
        return EvaluationOutcome::Response(allow_response());
    }

    // §9.5: only an actual schema/link read requires artifact_root.  The
    // artifact reader owns canonical containment and the §6 I/O taxonomy.
    let document = if schema.is_some() || !links.is_empty() {
        match read_artifact(config.artifact_root(), subject) {
            ArtifactReadOutcome::Present(document) => Some(document),
            ArtifactReadOutcome::Deny(deny) => {
                return EvaluationOutcome::Response(schema_deny_for_read(request, deny))
            }
            ArtifactReadOutcome::EvaluationError(error) => {
                return EvaluationOutcome::EvaluationError(error.to_string())
            }
        }
    } else {
        None
    };

    let mut metadata = None;
    if let (Some(schema), Some(document)) = (schema, document.as_ref()) {
        let report = schema.evaluate(document.value());
        let violations: Vec<Value> = report
            .violations()
            .iter()
            .map(|violation| {
                json!({
                    "path": violation.path,
                    "rule": violation.rule,
                    "message": violation.message,
                })
            })
            .collect();
        if !violations.is_empty() {
            return EvaluationOutcome::Response(schema_deny(request, violations));
        }

        // B2 guarantees these fields through the configured schema whenever
        // semantic evidence or a finding ledger is configured. Schema-only
        // draft hops need no metadata extraction.
        if gate.is_some() {
            let extracted = if subject == "validation-report.json"
                && matches!(initial_input["contract_version"].as_u64(), Some(2 | 3))
            {
                crate::artifacts::extract_index_metadata(document.value())
            } else {
                extract_metadata(subject, document.value())
            };
            match extracted {
                Ok(value) => metadata = Some(value),
                Err(error) => return EvaluationOutcome::EvaluationError(error.to_string()),
            }
        }
    }

    // Revision links are checked after the source subject has passed its
    // schema check.  T04 preserves target read-deny/error classes for this
    // layer to map into schema denial vs evaluation error.
    for link in links {
        let document = document
            .as_ref()
            .expect("configured links require a source schema and document");
        match check_revision_link(config.artifact_root(), subject, document.value(), link) {
            LinkCheckOutcome::Holds => {}
            LinkCheckOutcome::Violation(violation) => {
                return EvaluationOutcome::Response(schema_deny_for_link(request, violation))
            }
            LinkCheckOutcome::ReadDenied(deny) => {
                return EvaluationOutcome::Response(schema_deny_for_read(request, deny))
            }
            LinkCheckOutcome::EvaluationError(error) => {
                return EvaluationOutcome::EvaluationError(error.to_string())
            }
        }
    }

    if subject == crate::workflow::RECONCILIATION_SUBJECT {
        if let Some(document) = document.as_ref() {
            if let Err(response) = evaluate_reconciliation_result(
                request,
                document.value(),
                bookends_report
                    .as_ref()
                    .map(|report| report.live_ids.as_slice()),
                Some(if overlay_on {
                    "bookends-enabled"
                } else {
                    "bookends-disabled"
                }),
            ) {
                return EvaluationOutcome::Response(response);
            }
        }
    }

    if let Some(document) = document.as_ref() {
        match check_criterion_rules(&config, subject, document, config.contract_version() == 3) {
            Ok(violations) if !violations.is_empty() => {
                return EvaluationOutcome::Response(schema_deny_for_criteria(request, violations))
            }
            Ok(_) => {}
            Err(CriterionResolutionError::ReadDenied(deny)) => {
                return EvaluationOutcome::Response(schema_deny_for_read(request, *deny))
            }
            Err(CriterionResolutionError::LinkViolation(violation)) => {
                return EvaluationOutcome::Response(schema_deny_for_link(request, *violation))
            }
            Err(CriterionResolutionError::EvaluationError(error)) => {
                return EvaluationOutcome::EvaluationError(error.to_string())
            }
        }
    }

    let mut overlay_candidate_blocks = false;
    if overlay_on {
        let needs_current_intent = (subject == "validation-report.json" && is_passed(request))
            || document.as_ref().is_some_and(|document| {
                criterion::has_references(config.schema(subject), subject, document.value())
            });
        let current_intent = if subject == "intent.json" {
            document.clone()
        } else if needs_current_intent {
            let Some(document) = document.as_ref() else {
                return EvaluationOutcome::EvaluationError(
                    "overlay subject requires an artifact document".to_owned(),
                );
            };
            match resolve_current_intent(&config, document, &mut BTreeSet::new()) {
                Ok(intent) => intent,
                Err(CriterionResolutionError::ReadDenied(deny)) => {
                    return EvaluationOutcome::Response(schema_deny_for_read(request, *deny))
                }
                Err(CriterionResolutionError::LinkViolation(violation)) => {
                    return EvaluationOutcome::Response(schema_deny_for_link(request, *violation))
                }
                Err(CriterionResolutionError::EvaluationError(error)) => {
                    return EvaluationOutcome::EvaluationError(error.to_string())
                }
            }
        } else {
            None
        };
        let mut overlay_violations = Vec::new();
        if let Some(intent) = current_intent.as_ref() {
            overlay_violations.extend(overlay::intent_overlay_violations_for_profile(
                intent.value(),
                &bookends_report
                    .as_ref()
                    .expect("overlay-on loads the checker")
                    .live_ids,
                semantic_coverage,
            ));
        }
        if !overlay_violations.is_empty() {
            return EvaluationOutcome::Response(schema_deny_for_overlay(
                request,
                overlay_violations,
            ));
        }
        overlay_candidate_blocks = is_passed(request)
            && subject == "validation-report.json"
            && current_intent
                .as_ref()
                .is_some_and(|intent| overlay::has_candidate(intent.value()));
    }

    if checkpoint_required {
        if let Err(error) = verify_checkpoint(
            request,
            checkpoint_phase.expect("checkpoint phase"),
            &config,
        ) {
            return EvaluationOutcome::Response(error);
        }
    }

    // The fixed criterion/goal index applies to both supported semantic
    // contract generations when evaluating an already-readable artifact.
    if subject == "validation-report.json"
        && matches!(initial_input["contract_version"].as_u64(), Some(2 | 3))
    {
        let result = (|| {
            let root = std::path::Path::new(
                config
                    .artifact_root()
                    .and_then(Value::as_str)
                    .ok_or("missing artifact_root")?,
            );
            let bytes = std::fs::read(root.join(subject)).map_err(|e| e.to_string())?;
            let report: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            crate::validation::evaluate(
                &initial_input,
                &request.context,
                root,
                &report,
                request.transition.source.as_str() != "validation"
                    || request.transition.target.as_str() == "end",
            )
        })();
        if let Err(error) = result {
            return EvaluationOutcome::Response(deny_response(
                "software-change-criterion-incomplete",
                "criterion validation incomplete",
                Some(json!({"diagnostic":error})),
            ));
        }
    }

    if overlay_on && is_passed(request) && !bookends_is_green(bookends_report.as_ref()) {
        return EvaluationOutcome::Response(bookends_red_deny(
            request,
            bookends_report
                .as_ref()
                .expect("overlay-on loads the checker"),
        ));
    }
    if subject == crate::workflow::RECONCILIATION_SUBJECT
        && request.transition.event.as_str() == crate::workflow::RECONCILIATION_READY_EVENT
        && request.transition.target.as_str() == "validation"
    {
        if let Err(error) = verify_post_reconciliation_checkpoint(&config, request) {
            return EvaluationOutcome::Response(checkpoint_deny(request, error));
        }
    }
    if overlay_candidate_blocks {
        return EvaluationOutcome::Response(bookends_candidate_deny(request));
    }

    // §9.6: review evidence and the driver ledger are consulted only after
    // all deterministic artifact and revision-link checks pass. The ledger is
    // checked first so a missing, stale, or mechanically inconsistent driver
    // snapshot can never be mistaken for a reviewer verdict.
    if let (Some(gate), Some(metadata)) = (gate, metadata.as_ref()) {
        let subject_author =
            EvidenceAuthorIdentity::new(metadata.author().name(), metadata.author().kind());
        let ledger = evaluate_finding_ledger(
            &request.context,
            gate,
            subject,
            metadata.revision(),
            &config,
        );
        if !ledger.is_satisfied() {
            let mut details = ledger
                .details_value()
                .as_object()
                .cloned()
                .unwrap_or_default();
            details.insert("phase".to_owned(), json!("finding-ledger"));
            details.insert("prior_denials".to_owned(), prior_denials(request));
            return EvaluationOutcome::Response(deny_response(
                FINDING_LEDGER_DENY_CODE,
                FINDING_LEDGER_DENY_MESSAGE,
                Some(Value::Object(details)),
            ));
        }
        let evidence = staged_axes.map(|axes| {
            crate::evidence::evaluate_staged_evidence_with_dispositions(
                &request.context,
                gate,
                subject,
                metadata.revision(),
                &subject_author,
                config.config_version(),
                axes,
                config.axis_namespace(),
                config.artifact_root(),
                ledger.current_snapshot(),
                config.contract_version() == 3,
            )
        });
        if let Some(evidence) = evidence {
            if !evidence.is_satisfied() {
                let details = evidence.details_value();
                let details = json!({
                    "phase": "evidence",
                    "diagnostics": details.get("diagnostics").cloned().unwrap_or(Value::Array(Vec::new())),
                    "informational": details.get("informational").cloned().unwrap_or(Value::Array(Vec::new())),
                    "inert_records": details.get("inert_records").cloned().unwrap_or(Value::Array(Vec::new())),
                    "satisfied_by_disposition": details.get("satisfied_by_disposition").cloned().unwrap_or(Value::Array(Vec::new())),
                    "prior_denials": prior_denials(request),
                });
                return EvaluationOutcome::Response(deny_response(
                    EVIDENCE_DENY_CODE,
                    EVIDENCE_DENY_MESSAGE,
                    Some(details),
                ));
            }
        }
    }

    if subject == "implementation-report.json" && request.transition.target.as_str() == "validation"
    {
        let Some(root) = config.artifact_root().and_then(Value::as_str) else {
            return EvaluationOutcome::Response(checkpoint_deny(
                request,
                "artifact_root is required to preserve accepted implementation proof".to_owned(),
            ));
        };
        if let Err(error) =
            checkpoint::record_accepted_implementation_from_cwd(std::path::Path::new(root))
        {
            return EvaluationOutcome::EvaluationError(error);
        }
    }

    EvaluationOutcome::Response(allow_response())
}

enum CriterionResolutionError {
    ReadDenied(Box<ArtifactReadDeny>),
    LinkViolation(Box<LinkViolation>),
    EvaluationError(Box<ArtifactReadError>),
}

fn check_criterion_rules(
    config: &crate::config::ValidatedConfig,
    subject: &str,
    document: &ArtifactDocument,
    require_plan_task_criteria: bool,
) -> Result<Vec<CriterionViolation>, CriterionResolutionError> {
    let has_references =
        criterion::has_references(config.schema(subject), subject, document.value())
            || (require_plan_task_criteria
                && subject == "plan.json"
                && document
                    .value()
                    .get("tasks")
                    .and_then(Value::as_array)
                    .is_some_and(|tasks| !tasks.is_empty()));
    if subject == "intent.json" {
        return Ok(criterion::validate_intent(
            config.schema(subject),
            document.value(),
        ));
    }
    // Downstream references are optional.  Do not resolve an intent chain or
    // impose a new read obligation when the artifact contains no reference;
    // the existing configured schema and revision-link checks remain the only
    // checks for an unreferenced downstream artifact.
    if !has_references {
        return Ok(Vec::new());
    }

    let mut visited = BTreeSet::new();
    let Some(current_intent) = resolve_current_intent(config, document, &mut visited)? else {
        return Ok(vec![CriterionViolation {
            path: format!("/{subject}"),
            rule: "criterion-reference".to_owned(),
            message: "downstream criterion references require a currently linked intent".to_owned(),
        }]);
    };

    let identity_violations =
        criterion::validate_intent(config.schema("intent.json"), current_intent.value());
    if !identity_violations.is_empty() {
        return Ok(identity_violations);
    }
    let known_ids = criterion::intent_ids(current_intent.value());
    Ok(criterion::validate_references_with_task_requirement(
        subject,
        document.value(),
        &known_ids,
        require_plan_task_criteria,
    ))
}

fn resolve_current_intent(
    config: &crate::config::ValidatedConfig,
    current: &ArtifactDocument,
    visited: &mut BTreeSet<String>,
) -> Result<Option<ArtifactDocument>, CriterionResolutionError> {
    if current.subject() == "intent.json" {
        return Ok(Some(current.clone()));
    }
    if !visited.insert(current.subject().to_owned()) {
        return Ok(None);
    }

    for link in config.links_from(current.subject()) {
        let target = match read_artifact(config.artifact_root(), link.to()) {
            ArtifactReadOutcome::Present(document) => document,
            ArtifactReadOutcome::Deny(deny) => {
                return Err(CriterionResolutionError::ReadDenied(Box::new(deny)))
            }
            ArtifactReadOutcome::EvaluationError(error) => {
                return Err(CriterionResolutionError::EvaluationError(Box::new(error)))
            }
        };
        match check_revision_link(
            config.artifact_root(),
            current.subject(),
            current.value(),
            link,
        ) {
            LinkCheckOutcome::Holds => {}
            LinkCheckOutcome::Violation(violation) => {
                return Err(CriterionResolutionError::LinkViolation(Box::new(violation)))
            }
            LinkCheckOutcome::ReadDenied(deny) => {
                return Err(CriterionResolutionError::ReadDenied(Box::new(deny)))
            }
            LinkCheckOutcome::EvaluationError(error) => {
                return Err(CriterionResolutionError::EvaluationError(Box::new(error)))
            }
        }

        if target.subject() == "intent.json" {
            return Ok(Some(target));
        }
        if let Some(intent) = resolve_current_intent(config, &target, visited)? {
            return Ok(Some(intent));
        }
    }
    Ok(None)
}

fn verify_checkpoint(
    request: &EvaluateRequest,
    phase: checkpoint::CheckpointPhase,
    config: &crate::config::ValidatedConfig,
) -> Result<(), Value> {
    let Some(root) = config.artifact_root().and_then(Value::as_str) else {
        return Err(checkpoint_deny(
            request,
            "artifact_root is required for implementation and validation checkpoints".to_owned(),
        ));
    };
    checkpoint::verify_from_cwd(phase, std::path::Path::new(root))
        .map_err(|error| checkpoint_deny(request, error))?;
    if phase == checkpoint::CheckpointPhase::Validation {
        checkpoint::verify_accepted_implementation_from_cwd(std::path::Path::new(root))
            .map_err(|error| checkpoint_deny(request, error))?;
    }
    Ok(())
}

fn checkpoint_deny(request: &EvaluateRequest, diagnostic: String) -> Value {
    deny_response(
        CHECKPOINT_DENY_CODE,
        CHECKPOINT_DENY_MESSAGE,
        Some(json!({
            "phase": "checkpoint",
            "diagnostic": diagnostic,
            "prior_denials": prior_denials(request),
        })),
    )
}

fn evaluate_reconciliation_result(
    request: &EvaluateRequest,
    value: &Value,
    live_ids: Option<&[String]>,
    expected_mode: Option<&str>,
) -> Result<(), Value> {
    let decision = value
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let blockers = value
        .get("blockers")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();

    if decision == "blocked" {
        if blockers == 0 {
            return Err(reconciliation_deny(
                request,
                "a blocked reconciliation must name at least one concrete blocker".to_owned(),
            ));
        }
        return Err(reconciliation_deny(
            request,
            "reconciliation decision is blocked".to_owned(),
        ));
    }
    if decision != "complete" {
        return Err(reconciliation_deny(
            request,
            "reconciliation decision must be complete or blocked".to_owned(),
        ));
    }
    if blockers != 0 {
        return Err(reconciliation_deny(
            request,
            "a complete reconciliation cannot retain blockers".to_owned(),
        ));
    }

    let branch = value
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mode = value.get("mode").and_then(Value::as_str);
    if !matches!(mode, Some("bookends-enabled") | Some("bookends-disabled")) {
        return Err(reconciliation_deny(
            request,
            "reconciliation mode must be bookends-enabled or bookends-disabled".to_owned(),
        ));
    }
    if let Some(expected_mode) = expected_mode {
        if mode != Some(expected_mode) {
            return Err(reconciliation_deny(
                request,
                format!(
                    "reconciliation mode `{}` does not match the active `{expected_mode}` path",
                    mode.unwrap_or("missing")
                ),
            ));
        }
    }
    let authorization = value.get("authorization").and_then(Value::as_str);
    let application = value.get("application").and_then(Value::as_str);
    let commit = value.get("commit").and_then(Value::as_str);
    let action = value.get("action").and_then(Value::as_str);
    let traceability = value.get("traceability");
    let traceability_status = traceability
        .and_then(|traceability| traceability.get("status"))
        .and_then(Value::as_str);
    let traceability_references = traceability
        .and_then(|traceability| traceability.get("references"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let proof_references = value
        .get("proof_references")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let bookends_enabled = mode == Some("bookends-enabled");

    if !bookends_enabled
        && (traceability_status != Some("not-applicable")
            || !traceability_references.is_empty()
            || proof_references
                .iter()
                .filter_map(Value::as_str)
                .any(is_bookends_reference))
    {
        return Err(reconciliation_deny(
            request,
            "Bookends-disabled reconciliation must not carry PRD traceability or Bookends citations"
                .to_owned(),
        ));
    }
    if bookends_enabled
        && proof_references
            .iter()
            .filter_map(Value::as_str)
            .any(is_bookends_reference)
    {
        validate_live_references(request, proof_references, live_ids)?;
    }

    let has_document_update = value
        .get("document_observations")
        .and_then(Value::as_array)
        .is_some_and(|observations| {
            observations.iter().any(|observation| {
                matches!(
                    observation.get("status").and_then(Value::as_str),
                    Some("updated")
                )
            })
        });
    let has_corrected_behavior = value
        .get("behavior_observations")
        .and_then(Value::as_array)
        .is_some_and(|observations| {
            observations.iter().any(|observation| {
                observation.get("status").and_then(Value::as_str) == Some("corrected")
            })
        });

    match branch {
        "sufficient-existing-wording" => {
            let implementation_correction = action == Some("implementation-correction");
            if (!matches!(action, Some("none") | Some("no-document-change"))
                && !implementation_correction)
                || has_document_update
                || authorization != Some("not-required")
                || application != Some("not-required")
                || commit != Some("not-required")
                || (implementation_correction && !has_corrected_behavior)
            {
                return Err(reconciliation_deny(
                    request,
                    "sufficient wording requires no document edit and no requirement-amendment statuses; an implementation correction must record corrected behavior".to_owned(),
                ));
            }
            if bookends_enabled {
                require_live_traceability(
                    request,
                    traceability_status,
                    traceability_references,
                    live_ids,
                )?;
            }
        }
        "change-specific-proof" => {
            if !matches!(action, Some("none") | Some("no-document-change"))
                || has_document_update
                || authorization != Some("not-required")
                || application != Some("not-required")
                || commit != Some("not-required")
            {
                return Err(reconciliation_deny(
                    request,
                    "change-specific proof requires no document edit and no requirement-amendment statuses".to_owned(),
                ));
            }
            if bookends_enabled {
                allow_change_specific_traceability(
                    request,
                    traceability_status,
                    traceability_references,
                    live_ids,
                )?;
            }
        }
        "missing-or-changed-enduring-meaning" => {
            if authorization != Some("accepted")
                || application != Some("applied")
                || commit != Some("committed")
                || !has_document_update
            {
                return Err(reconciliation_deny(
                    request,
                    "missing or changed enduring meaning requires exact owner acceptance, an applied document edit, a committed change, and an updated document observation".to_owned(),
                ));
            }
            if bookends_enabled {
                if action != Some("amendment-application") {
                    return Err(reconciliation_deny(
                        request,
                        "Bookends-enabled requirement changes must use the amendment-application action".to_owned(),
                    ));
                }
                require_live_traceability(
                    request,
                    traceability_status,
                    traceability_references,
                    live_ids,
                )?;
            } else if action != Some("document-edit")
                || traceability_status != Some("not-applicable")
                || !traceability_references.is_empty()
            {
                return Err(reconciliation_deny(
                    request,
                    "Bookends-disabled document changes must use document-edit and carry no PRD traceability".to_owned(),
                ));
            }
        }
        _ => {
            return Err(reconciliation_deny(
                request,
                "reconciliation branch is not recognized".to_owned(),
            ));
        }
    }
    Ok(())
}

fn is_bookends_reference(reference: &str) -> bool {
    reference.starts_with("LE-") || reference.starts_with("bookends:")
}

fn validate_live_references(
    request: &EvaluateRequest,
    references: &[Value],
    live_ids: Option<&[String]>,
) -> Result<(), Value> {
    let mut has_live_requirement = false;
    for reference in references.iter().filter_map(Value::as_str) {
        if !is_bookends_reference(reference) {
            continue;
        }
        let Some(id) = normalized_requirement_id(reference) else {
            return Err(reconciliation_deny(
                request,
                format!("traceability reference `{reference}` is not a valid PRD citation"),
            ));
        };
        if let Some(live_ids) = live_ids {
            if !live_ids.iter().any(|live_id| live_id == id) {
                return Err(reconciliation_deny(
                    request,
                    format!("traceability reference `{reference}` is provisional or not live"),
                ));
            }
        }
        has_live_requirement = true;
    }
    if !has_live_requirement {
        return Err(reconciliation_deny(
            request,
            "Bookends-enabled completion must retain at least one live PRD ID in traceability"
                .to_owned(),
        ));
    }
    Ok(())
}

fn normalized_requirement_id(reference: &str) -> Option<&str> {
    if let Some(id) = reference.strip_prefix("bookends:") {
        return id
            .strip_prefix("LE-")
            .filter(|suffix| valid_requirement_number(suffix))
            .map(|_| &reference["bookends:".len()..]);
    }
    if let Some(id) = reference.strip_prefix("LE-") {
        return valid_requirement_number(id).then_some(reference);
    }
    None
}

fn valid_requirement_number(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_digit() && first != '0')
        && characters.all(|character| character.is_ascii_digit())
}

fn require_live_traceability(
    request: &EvaluateRequest,
    status: Option<&str>,
    references: &[Value],
    live_ids: Option<&[String]>,
) -> Result<(), Value> {
    if !matches!(status, Some("retained") | Some("updated")) {
        return Err(reconciliation_deny(
            request,
            "Bookends-enabled completion must retain or update live traceability".to_owned(),
        ));
    }
    validate_live_references(request, references, live_ids)
}

fn allow_change_specific_traceability(
    request: &EvaluateRequest,
    status: Option<&str>,
    references: &[Value],
    live_ids: Option<&[String]>,
) -> Result<(), Value> {
    if status == Some("not-applicable") && references.is_empty() {
        return Ok(());
    }
    if matches!(status, Some("retained") | Some("updated")) {
        return validate_live_references(request, references, live_ids);
    }
    Err(reconciliation_deny(
        request,
        "change-specific proof must have no PRD traceability or retain only live traceability"
            .to_owned(),
    ))
}

fn verify_post_reconciliation_checkpoint(
    config: &crate::config::ValidatedConfig,
    _request: &EvaluateRequest,
) -> Result<(), String> {
    let Some(root) = config.artifact_root().and_then(Value::as_str) else {
        return Err(
            "artifact_root is required before reconciliation can enter validation".to_owned(),
        );
    };
    let root = std::path::Path::new(root);
    checkpoint::verify_from_cwd(checkpoint::CheckpointPhase::Implementation, root)?;
    // Reviewless v3 graphs skip the implementation-report transition that used
    // to record accepted implementation proof. Preserve that proof boundary
    // without making reconciliation write or replace the checkpoint itself.
    checkpoint::record_accepted_implementation_from_cwd(root)
}

fn reconciliation_deny(request: &EvaluateRequest, diagnostic: String) -> Value {
    deny_response(
        RECONCILIATION_DENY_CODE,
        RECONCILIATION_DENY_MESSAGE,
        Some(json!({
            "phase": "reconciliation",
            "diagnostic": diagnostic,
            "prior_denials": prior_denials(request),
        })),
    )
}

fn load_bookends_report() -> Result<bookends_check::CheckReport, String> {
    let cwd = std::env::current_dir()
        .map_err(|error| format!("cannot read evaluate process cwd: {error}"))?;
    let bypass = match std::env::var("BOOKENDS_BYPASS") {
        Ok(value) if !value.is_empty() => {
            let Some((class, reason)) = value.split_once(':') else {
                return Err(
                    "BOOKENDS_BYPASS must have the form <class>:<reason> when non-empty".into(),
                );
            };
            if class.is_empty() || reason.is_empty() {
                return Err("BOOKENDS_BYPASS must have non-empty <class> and <reason>".into());
            }
            Some((class.to_owned(), reason.to_owned()))
        }
        _ => None,
    };
    let bypass = bypass
        .as_ref()
        .map(|(class, reason)| (class.as_str(), reason.as_str()));
    bookends_check::check_repo(&cwd, bypass).map_err(|error| error.to_string())
}

fn is_passed(request: &EvaluateRequest) -> bool {
    request.transition.event.as_str() == "passed"
}

fn bookends_is_green(report: Option<&bookends_check::CheckReport>) -> bool {
    report.is_some_and(|report| matches!(report.status, CheckStatus::Green))
}

fn schema_deny_for_overlay(
    request: &EvaluateRequest,
    violations: Vec<overlay::OverlayViolation>,
) -> Value {
    schema_deny(
        request,
        violations
            .into_iter()
            .map(|violation| {
                json!({
                    "path": violation.path,
                    "rule": violation.rule,
                    "message": violation.message,
                })
            })
            .collect(),
    )
}

fn bookends_candidate_deny(request: &EvaluateRequest) -> Value {
    deny_response(
        BOOKENDS_CANDIDATE_DENY_CODE,
        BOOKENDS_CANDIDATE_DENY_MESSAGE,
        Some(json!({
            "phase": "bookends",
            "diagnostic": "resolve every current candidate through explicit owner acceptance and committed PRD integration or honest reclassification before final validation passed",
            "prior_denials": prior_denials(request),
        })),
    )
}

fn bookends_red_deny(request: &EvaluateRequest, report: &bookends_check::CheckReport) -> Value {
    let mut details = json!({
        "phase": "bookends",
        "status": match &report.status {
            CheckStatus::Green => "green",
            CheckStatus::Red => "red",
            CheckStatus::Bypass { .. } => "bypass",
        },
        "findings": report.findings,
        "prior_denials": prior_denials(request),
    });
    if let CheckStatus::Bypass { class, reason } = &report.status {
        let object = details
            .as_object_mut()
            .expect("bookends denial details are an object");
        object.insert("bypass_class".to_owned(), json!(class));
        object.insert("bypass_reason".to_owned(), json!(reason));
    }
    deny_response(
        BOOKENDS_RED_DENY_CODE,
        BOOKENDS_RED_DENY_MESSAGE,
        Some(details),
    )
}

fn duties_for_snapshotted_transition(request: &EvaluateRequest) -> Option<TransitionDuties> {
    let transition = &request.transition;
    let in_snapshot = request.workflow.transitions.iter().any(|edge| {
        edge.source == transition.source
            && edge.event == transition.event
            && edge.target == transition.target
            && edge.kind == transition.kind
    });
    if !in_snapshot {
        return None;
    }
    let duties = workflow::duties_for_transition(
        transition.source.as_str(),
        transition.event.as_str(),
        transition.target.as_str(),
    )?;
    match (duties, transition.kind) {
        (
            TransitionDuties::Checked { .. } | TransitionDuties::CheckedNoArtifact,
            TransitionKind::Checked,
        )
        | (TransitionDuties::CheckFree, TransitionKind::CheckFree) => Some(duties),
        _ => None,
    }
}

fn schema_deny(request: &EvaluateRequest, violations: Vec<Value>) -> Value {
    deny_response(
        SCHEMA_DENY_CODE,
        SCHEMA_DENY_MESSAGE,
        Some(json!({
            "phase": "schema",
            "violations": violations,
            "prior_denials": prior_denials(request),
        })),
    )
}

fn schema_deny_for_read(request: &EvaluateRequest, deny: ArtifactReadDeny) -> Value {
    schema_deny(
        request,
        vec![json!({
            "path": format!("/{}", deny.subject()),
            "rule": "artifact-read",
            "message": deny.to_string(),
        })],
    )
}

fn schema_deny_for_link(request: &EvaluateRequest, violation: LinkViolation) -> Value {
    schema_deny(
        request,
        vec![json!({
            "path": "/revision-links",
            "rule": "revision-link",
            "message": violation.to_string(),
        })],
    )
}

fn schema_deny_for_criteria(
    request: &EvaluateRequest,
    violations: Vec<CriterionViolation>,
) -> Value {
    schema_deny(
        request,
        violations
            .into_iter()
            .map(|violation| {
                json!({
                    "path": violation.path,
                    "rule": violation.rule,
                    "message": violation.message,
                })
            })
            .collect(),
    )
}

fn prior_denials(request: &EvaluateRequest) -> Value {
    Value::Array(
        request
            .prior_evaluations
            .iter()
            .filter_map(|evaluation| {
                let DurableEvaluationResult::Deny { feedback } = &evaluation.result else {
                    return None;
                };
                Some(json!({
                    "sequence": evaluation.sequence.as_u64(),
                    "code": feedback.code,
                    "message": feedback.message,
                }))
            })
            .collect(),
    )
}

impl fmt::Display for EvaluationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Response(value) => write!(formatter, "{value}"),
            Self::EvaluationError(message) => formatter.write_str(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{EventId, StateId, Transition, Workflow};
    use serde_json::json;

    fn transition(source: &str, event: &str, target: &str, kind: TransitionKind) -> Transition {
        Transition::new(
            StateId::new(source),
            EventId::new(event),
            StateId::new(target),
            kind,
        )
    }

    fn request(workflow: Workflow, edge: Transition) -> EvaluateRequest {
        EvaluateRequest {
            operation: "evaluate".to_owned(),
            workflow,
            initial_input: json!({"config_version": "none", "review_policies": {}}),
            context: Vec::new(),
            transition: edge,
            prior_evaluations: Vec::new(),
        }
    }

    #[test]
    fn reconciliation_result_branches_require_honest_completion_status() {
        let workflow = workflow::describe_workflow(Some(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "test-v3",
            "review_policies": {}
        })))
        .expect("v3 workflow");
        let edge = workflow
            .transitions
            .iter()
            .find(|edge| {
                edge.source.as_str() == workflow::RECONCILIATION_STATE
                    && edge.event.as_str() == workflow::RECONCILIATION_READY_EVENT
            })
            .expect("reconciliation edge")
            .clone();
        let request = request(workflow, edge);

        let mut blocked = json!({
            "decision": "blocked",
            "blockers": ["owner authorization is missing"]
        });
        let response = evaluate_reconciliation_result(&request, &blocked, None, None)
            .expect_err("blocked result must deny progression");
        assert_eq!(response["feedback"]["code"], RECONCILIATION_DENY_CODE);

        blocked["decision"] = json!("complete");
        assert!(evaluate_reconciliation_result(&request, &blocked, None, None).is_err());

        let unchanged = json!({
            "mode": "bookends-disabled",
            "decision": "complete",
            "blockers": [],
            "branch": "sufficient-existing-wording",
            "document_observations": [{"path": "README.md", "status": "sufficient", "observation": "already states the behavior"}],
            "behavior_observations": [{"status": "matches-intent", "observation": "public result matches"}],
            "action": "no-document-change",
            "authorization": "not-required",
            "application": "not-required",
            "commit": "not-required",
            "traceability": {"status": "not-applicable", "references": []},
            "proof_references": ["journey:reconciliation"],
        });
        assert!(evaluate_reconciliation_result(&request, &unchanged, None, None).is_ok());

        let mut off_corrected = unchanged.clone();
        off_corrected["behavior_observations"][0]["status"] = json!("corrected");
        off_corrected["action"] = json!("implementation-correction");
        assert!(evaluate_reconciliation_result(&request, &off_corrected, None, None).is_ok());
        off_corrected["traceability"]["status"] = json!("retained");
        assert!(evaluate_reconciliation_result(&request, &off_corrected, None, None).is_err());

        let mut change_specific = unchanged.clone();
        change_specific["mode"] = json!("bookends-enabled");
        change_specific["branch"] = json!("change-specific-proof");
        assert!(evaluate_reconciliation_result(&request, &change_specific, None, None).is_ok());

        let missing = json!({
            "mode": "bookends-enabled",
            "decision": "complete",
            "blockers": [],
            "branch": "missing-or-changed-enduring-meaning",
            "document_observations": [{"path": "docs/PRD.md", "status": "updated", "observation": "accepted wording is now committed"}],
            "behavior_observations": [{"status": "matches-intent", "observation": "public proof matches"}],
            "action": "amendment-application",
            "authorization": "accepted",
            "application": "applied",
            "commit": "pending",
            "traceability": {"status": "updated", "references": ["bookends:LE-142"]},
            "proof_references": ["journey:reconciliation"],
        });
        assert!(evaluate_reconciliation_result(&request, &missing, None, None).is_err());

        let mut corrected = json!({
            "mode": "bookends-enabled",
            "decision": "complete",
            "blockers": [],
            "branch": "sufficient-existing-wording",
            "document_observations": [{"path": "docs/PRD.md", "status": "sufficient", "observation": "existing wording is enough"}],
            "behavior_observations": [{"status": "corrected", "observation": "completed output now reports the accepted result"}],
            "action": "implementation-correction",
            "authorization": "not-required",
            "application": "not-required",
            "commit": "not-required",
            "traceability": {"status": "retained", "references": ["bookends:LE-1"]},
            "proof_references": ["journey:reconciliation"],
        });
        assert!(evaluate_reconciliation_result(&request, &corrected, None, None).is_ok());
        corrected["traceability"]["references"] = json!([]);
        assert!(evaluate_reconciliation_result(&request, &corrected, None, None).is_err());

        let amended = json!({
            "mode": "bookends-enabled",
            "decision": "complete",
            "blockers": [],
            "branch": "missing-or-changed-enduring-meaning",
            "document_observations": [{"path": "docs/PRD.md", "status": "updated", "observation": "accepted wording is now committed"}],
            "behavior_observations": [{"status": "matches-intent", "observation": "public proof matches"}],
            "action": "amendment-application",
            "authorization": "accepted",
            "application": "applied",
            "commit": "committed",
            "traceability": {"status": "updated", "references": ["bookends:LE-142"]},
            "proof_references": ["journey:reconciliation"],
        });
        assert!(evaluate_reconciliation_result(&request, &amended, None, None).is_ok());
        assert!(evaluate_reconciliation_result(
            &request,
            &amended,
            Some(&["LE-1".to_owned()]),
            None
        )
        .is_err());

        let off_with_prd_reference = json!({
            "mode": "bookends-disabled",
            "decision": "complete",
            "blockers": [],
            "branch": "change-specific-proof",
            "document_observations": [{"path": "README.md", "status": "sufficient", "observation": "already states the behavior"}],
            "behavior_observations": [{"status": "change-specific", "observation": "only this change needs the proof"}],
            "action": "no-document-change",
            "authorization": "not-required",
            "application": "not-required",
            "commit": "not-required",
            "traceability": {"status": "retained", "references": []},
            "proof_references": ["bookends:LE-142"],
        });
        assert!(
            evaluate_reconciliation_result(&request, &off_with_prd_reference, None, None).is_err()
        );
    }

    #[test]
    fn union_transitions_match_phase_table_duties() {
        let workflow = workflow::software_change_workflow();
        for edge in workflow.transitions.clone() {
            let duties =
                duties_for_snapshotted_transition(&request(workflow.clone(), edge.clone()))
                    .unwrap_or_else(|| panic!("missing duties for {edge:?}"));
            match (duties, edge.kind) {
                (
                    TransitionDuties::Checked { .. } | TransitionDuties::CheckedNoArtifact,
                    TransitionKind::Checked,
                )
                | (TransitionDuties::CheckFree, TransitionKind::CheckFree) => {}
                _ => panic!("kind mismatch for {edge:?}"),
            }
        }
    }

    #[test]
    fn snapshot_tuple_mismatch_is_unsupported() {
        let workflow = workflow::software_change_workflow();
        let variants = [
            transition(
                "wrong",
                "intent-ready",
                "intent-review",
                TransitionKind::Checked,
            ),
            transition("explore", "wrong", "intent-review", TransitionKind::Checked),
            transition("explore", "intent-ready", "design", TransitionKind::Checked),
            transition(
                "explore",
                "intent-ready",
                "intent-review",
                TransitionKind::CheckFree,
            ),
            transition(
                "intent-review",
                "revise",
                "explore",
                TransitionKind::Checked,
            ),
        ];
        for variant in variants {
            assert!(
                duties_for_snapshotted_transition(&request(workflow.clone(), variant.clone()))
                    .is_none(),
                "{variant:?} should be unsupported against the union snapshot"
            );
        }
    }

    #[test]
    fn live_rewired_target_is_trusted_from_the_snapshot() {
        let live = workflow::describe_workflow(Some(&json!({"review_policies": {}})))
            .expect("empty policies stitch");
        let rewired = transition("explore", "intent-ready", "design", TransitionKind::Checked);
        let duties = duties_for_snapshotted_transition(&request(live, rewired))
            .expect("rewired draft hop is in the live snapshot");
        assert_eq!(
            duties,
            TransitionDuties::Checked {
                subject: "intent.json",
                gate: None,
            }
        );

        let union = workflow::software_change_workflow();
        assert!(
            duties_for_snapshotted_transition(&request(
                union,
                transition("explore", "intent-ready", "design", TransitionKind::Checked),
            ))
            .is_none(),
            "union snapshot does not include the review-less rewired target"
        );
    }

    fn assert_tuples_match_phase_table(workflow: &Workflow) {
        for edge in &workflow.transitions {
            let duties =
                duties_for_snapshotted_transition(&request(workflow.clone(), edge.clone()))
                    .unwrap_or_else(|| panic!("missing duties for {edge:?}"));
            let expected = workflow::duties_for(edge.source.as_str(), edge.event.as_str())
                .unwrap_or_else(|| panic!("phase table missing {edge:?}"));
            assert_eq!(duties, expected, "{edge:?}");
            match (duties, edge.kind) {
                (
                    TransitionDuties::Checked { .. } | TransitionDuties::CheckedNoArtifact,
                    TransitionKind::Checked,
                )
                | (TransitionDuties::CheckFree, TransitionKind::CheckFree) => {}
                _ => panic!("kind mismatch for {edge:?}"),
            }
        }
    }

    fn passed_hops(workflow: &Workflow) -> Vec<(&str, &str)> {
        workflow
            .transitions
            .iter()
            .filter(|edge| edge.event.as_str() == "passed")
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect()
    }

    #[test]
    fn stitched_graphs_and_evaluate_tuples_match_phase_table() {
        let union = workflow::software_change_workflow();
        assert_eq!(union.states.len(), 16);
        assert_eq!(
            passed_hops(&union),
            vec![("validation-adversarial-review", "end")]
        );
        assert_tuples_match_phase_table(&union);

        let omitted_key = workflow::describe_workflow(Some(&json!({"objective": "a2"})))
            .expect("omitted review_policies is union");
        assert_eq!(omitted_key, union);
        assert_tuples_match_phase_table(&omitted_key);

        let omit_all = workflow::describe_workflow(Some(&json!({"review_policies": {}})))
            .expect("empty lists omit reviews");
        assert_eq!(
            omit_all
                .states
                .iter()
                .map(|state| state.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "explore",
                "design",
                "plan",
                "implement",
                "validation",
                "end",
            ]
        );
        assert_eq!(passed_hops(&omit_all), vec![("validation", "end")]);
        assert_tuples_match_phase_table(&omit_all);
        assert_eq!(
            duties_for_snapshotted_transition(&request(
                omit_all.clone(),
                transition("validation", "passed", "end", TransitionKind::Checked),
            )),
            Some(TransitionDuties::Checked {
                subject: "validation-report.json",
                gate: None,
            })
        );

        let validation_review_only = workflow::describe_workflow(Some(&json!({
            "review_policies": {
                "validation-review": [{"id": "delivery", "description": "d"}]
            }
        })))
        .expect("validation-review only");
        assert_eq!(
            passed_hops(&validation_review_only),
            vec![("validation-review", "end")]
        );
        assert_tuples_match_phase_table(&validation_review_only);
        assert_eq!(
            duties_for_snapshotted_transition(&request(
                validation_review_only.clone(),
                transition(
                    "validation-review",
                    "passed",
                    "end",
                    TransitionKind::Checked,
                ),
            )),
            Some(TransitionDuties::Checked {
                subject: "validation-report.json",
                gate: Some("validation-review"),
            })
        );

        let adversarial_last = workflow::describe_workflow(Some(&json!({
            "review_policies": {
                "validation-review": [{"id": "delivery", "description": "d"}],
                "validation-adversarial-review": [{"id": "delivery", "description": "d"}]
            }
        })))
        .expect("adversarial last hop");
        assert_eq!(
            passed_hops(&adversarial_last),
            vec![("validation-adversarial-review", "end")]
        );
        assert_tuples_match_phase_table(&adversarial_last);
        assert_eq!(
            duties_for_snapshotted_transition(&request(
                adversarial_last.clone(),
                transition(
                    "validation-review",
                    "approved",
                    "validation-adversarial-review",
                    TransitionKind::Checked,
                ),
            )),
            Some(TransitionDuties::Checked {
                subject: "validation-report.json",
                gate: Some("validation-review"),
            })
        );
    }
}
