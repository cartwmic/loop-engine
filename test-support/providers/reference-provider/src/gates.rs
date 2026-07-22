//! Gate evaluation and artifact validation policy.

use std::collections::BTreeSet;
use std::path::Path;

use crate::config::ProviderConfig;
use crate::evidence::{
    ArtifactError, ReviewVerdict, ValidationVerdict, artifact_root_from_inputs, artifacts,
    evidence_for_artifact, malformed_evidence, read_revision_doc, require_linkage,
    require_review_verdict, require_revision, require_validation_verdict,
};
use crate::graph::{
    GATE_DESIGN_READY, GATE_DESIGN_REVIEW_APPROVED, GATE_DESIGN_REVIEW_CHANGES,
    GATE_IMPLEMENTATION_READY, GATE_IMPLEMENTATION_REVIEW_APPROVED,
    GATE_IMPLEMENTATION_REVIEW_CHANGES, GATE_INTENT_READY, GATE_PLAN_READY,
    GATE_PLAN_REVIEW_APPROVED, GATE_PLAN_REVIEW_CHANGES, GATE_VALIDATION_FAILED,
    GATE_VALIDATION_PASSED, SUPPORTED_GATES,
};
use crate::protocol::{DiagnosticDto, EvidenceDto, GatePayloadDto, GateResultDto, GateVerdictDto};

pub fn evaluate_gates(payload: &GatePayloadDto, config: &ProviderConfig) -> GateResultDto {
    if config.gate_incompatible {
        return GateResultDto::Incompatible {
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unsupported".to_string(),
                message: "gate evaluation disabled by fixture mode".to_string(),
                path: None,
            }],
        };
    }
    if config.gate_evaluation_error {
        return GateResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation_error".to_string(),
                message: "gate evaluation forced to error by fixture mode".to_string(),
                path: None,
            }],
        };
    }

    if let Some(unsupported) = unsupported_required_gates(&payload.required_gate_ids) {
        let gates = unsupported.join(", ");
        return GateResultDto::Incompatible {
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unsupported".to_string(),
                message: format!("stored graph requires unsupported gates: {gates}"),
                path: None,
            }],
        };
    }

    let root = match artifact_root_from_inputs(&payload.snapshot.inputs) {
        Ok(path) => path,
        Err(diagnostic) => {
            return fail_all(payload, diagnostic);
        }
    };

    let existing_ids = collect_existing_ids(payload);
    let mut verdicts = Vec::with_capacity(payload.required_gate_ids.len());
    let mut evidence = Vec::new();
    let mut failure: Option<DiagnosticDto> = None;

    for gate_id in &payload.required_gate_ids {
        let evaluation = evaluate_gate(gate_id, &root, &payload.event, &existing_ids, &evidence);
        match evaluation {
            GateEvaluation::Pass(record) => {
                verdicts.push(GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                });
                evidence.push(record);
            }
            GateEvaluation::Fail(diagnostic) => {
                verdicts.push(GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: false,
                });
                failure.get_or_insert(diagnostic);
            }
        }
    }

    if let Some(_diagnostic) = failure {
        return GateResultDto::Verdicts {
            verdicts,
            evidence: None,
        };
    }

    if config.malformed_evidence {
        return GateResultDto::Verdicts {
            verdicts,
            evidence: Some(vec![malformed_evidence()]),
        };
    }

    GateResultDto::Verdicts {
        verdicts,
        evidence: if evidence.is_empty() {
            None
        } else {
            Some(evidence)
        },
    }
}

enum GateEvaluation {
    Pass(EvidenceDto),
    Fail(DiagnosticDto),
}

fn evaluate_gate(
    gate_id: &str,
    root: &Path,
    event: &str,
    existing_ids: &[String],
    pending_evidence: &[EvidenceDto],
) -> GateEvaluation {
    let pending_ids: Vec<String> = existing_ids
        .iter()
        .chain(pending_evidence.iter().map(|e| &e.id))
        .cloned()
        .collect();

    match gate_id {
        GATE_INTENT_READY => check_simple(root, artifacts::INTENT, &pending_ids),
        GATE_DESIGN_READY => check_design(root, &pending_ids),
        GATE_DESIGN_REVIEW_APPROVED => check_review(
            root,
            artifacts::DESIGN_REVIEW,
            ReviewVerdict::Approved,
            event,
        ),
        GATE_DESIGN_REVIEW_CHANGES => check_review(
            root,
            artifacts::DESIGN_REVIEW,
            ReviewVerdict::ChangesRequested,
            event,
        ),
        GATE_PLAN_READY => check_plan(root, &pending_ids),
        GATE_PLAN_REVIEW_APPROVED => {
            check_review(root, artifacts::PLAN_REVIEW, ReviewVerdict::Approved, event)
        }
        GATE_PLAN_REVIEW_CHANGES => check_review(
            root,
            artifacts::PLAN_REVIEW,
            ReviewVerdict::ChangesRequested,
            event,
        ),
        GATE_IMPLEMENTATION_READY => check_implementation(root, &pending_ids),
        GATE_IMPLEMENTATION_REVIEW_APPROVED => check_review(
            root,
            artifacts::IMPLEMENTATION_REVIEW,
            ReviewVerdict::Approved,
            event,
        ),
        GATE_IMPLEMENTATION_REVIEW_CHANGES => check_review(
            root,
            artifacts::IMPLEMENTATION_REVIEW,
            ReviewVerdict::ChangesRequested,
            event,
        ),
        GATE_VALIDATION_PASSED => {
            check_validation(root, ValidationVerdict::Passed, event, &pending_ids)
        }
        GATE_VALIDATION_FAILED => {
            check_validation(root, ValidationVerdict::Failed, event, &pending_ids)
        }
        other => GateEvaluation::Fail(DiagnosticDto {
            code: "gate.internal".to_string(),
            message: format!("unexpected gate {other} after compatibility preflight"),
            path: None,
        }),
    }
}

fn unsupported_required_gates(gate_ids: &[String]) -> Option<Vec<String>> {
    let supported: BTreeSet<&str> = SUPPORTED_GATES.iter().copied().collect();
    let unsupported: Vec<String> = gate_ids
        .iter()
        .filter(|gate_id| !supported.contains(gate_id.as_str()))
        .cloned()
        .collect();
    if unsupported.is_empty() {
        None
    } else {
        Some(unsupported)
    }
}

fn check_simple(
    root: &Path,
    artifact: crate::evidence::ArtifactRef,
    existing_ids: &[String],
) -> GateEvaluation {
    match read_revision_doc(root, &artifact) {
        Ok(doc) => match require_revision(&doc, artifact.relative_path) {
            Ok(revision) => GateEvaluation::Pass(evidence_for_artifact(
                artifact,
                root,
                &revision,
                existing_ids,
            )),
            Err(err) => GateEvaluation::Fail(err.diagnostic()),
        },
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn check_design(root: &Path, existing_ids: &[String]) -> GateEvaluation {
    let artifact = artifacts::DESIGN;
    match read_revision_doc(root, &artifact) {
        Ok(doc) => {
            let revision = match require_revision(&doc, artifact.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let intent = match read_revision_doc(root, &artifacts::INTENT) {
                Ok(intent_doc) => intent_doc,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let intent_revision = match require_revision(&intent, artifacts::INTENT.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            if let Err(err) = require_linkage(
                &doc,
                artifact.relative_path,
                "intent_revision",
                &intent_revision,
            ) {
                return GateEvaluation::Fail(err.diagnostic());
            }
            GateEvaluation::Pass(evidence_for_artifact(
                artifact,
                root,
                &revision,
                existing_ids,
            ))
        }
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn check_plan(root: &Path, existing_ids: &[String]) -> GateEvaluation {
    let artifact = artifacts::PLAN;
    match read_revision_doc(root, &artifact) {
        Ok(doc) => {
            let revision = match require_revision(&doc, artifact.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let design = match read_revision_doc(root, &artifacts::DESIGN) {
                Ok(design_doc) => design_doc,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let design_revision = match require_revision(&design, artifacts::DESIGN.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            if doc.subject_revision.as_deref() != Some(design_revision.as_str()) {
                return GateEvaluation::Fail(
                    ArtifactError::RevisionMismatch {
                        path: artifact.relative_path.to_string(),
                        expected: design_revision,
                        actual: doc.subject_revision.unwrap_or_default(),
                    }
                    .diagnostic(),
                );
            }
            GateEvaluation::Pass(evidence_for_artifact(
                artifact,
                root,
                &revision,
                existing_ids,
            ))
        }
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn check_implementation(root: &Path, existing_ids: &[String]) -> GateEvaluation {
    let artifact = artifacts::IMPLEMENTATION;
    match read_revision_doc(root, &artifact) {
        Ok(doc) => {
            let revision = match require_revision(&doc, artifact.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let plan = match read_revision_doc(root, &artifacts::PLAN) {
                Ok(plan_doc) => plan_doc,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let plan_revision = match require_revision(&plan, artifacts::PLAN.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            if doc.plan_revision.as_deref() != Some(plan_revision.as_str()) {
                return GateEvaluation::Fail(
                    ArtifactError::RevisionMismatch {
                        path: artifact.relative_path.to_string(),
                        expected: plan_revision,
                        actual: doc.plan_revision.unwrap_or_default(),
                    }
                    .diagnostic(),
                );
            }
            GateEvaluation::Pass(evidence_for_artifact(
                artifact,
                root,
                &revision,
                existing_ids,
            ))
        }
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn check_review(
    root: &Path,
    artifact: crate::evidence::ArtifactRef,
    expected_verdict: ReviewVerdict,
    event: &str,
) -> GateEvaluation {
    if !event_matches_review(event, expected_verdict) {
        return GateEvaluation::Fail(DiagnosticDto {
            code: "event.verdict_mismatch".to_string(),
            message: format!("event {event} does not match expected review gate"),
            path: None,
        });
    }

    let subject = subject_artifact_for_review(artifact.relative_path);
    match read_revision_doc(root, &artifact) {
        Ok(doc) => {
            let revision = match require_revision(&doc, artifact.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let subject_doc = match read_revision_doc(root, &subject) {
                Ok(subject_doc) => subject_doc,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            let subject_revision = match require_revision(&subject_doc, subject.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            if doc.subject_revision.as_deref() != Some(subject_revision.as_str()) {
                return GateEvaluation::Fail(
                    ArtifactError::RevisionMismatch {
                        path: artifact.relative_path.to_string(),
                        expected: subject_revision,
                        actual: doc.subject_revision.unwrap_or_default(),
                    }
                    .diagnostic(),
                );
            }
            if let Err(err) = require_review_verdict(&doc, artifact.relative_path, expected_verdict)
            {
                return GateEvaluation::Fail(err.diagnostic());
            }
            GateEvaluation::Pass(evidence_for_artifact(artifact, root, &revision, &[]))
        }
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn check_validation(
    root: &Path,
    expected_verdict: ValidationVerdict,
    event: &str,
    existing_ids: &[String],
) -> GateEvaluation {
    if !event_matches_validation(event, expected_verdict) {
        return GateEvaluation::Fail(DiagnosticDto {
            code: "event.verdict_mismatch".to_string(),
            message: format!("event {event} does not match expected validation gate"),
            path: None,
        });
    }

    let artifact = artifacts::VALIDATION;
    match read_revision_doc(root, &artifact) {
        Ok(doc) => {
            let revision = match require_revision(&doc, artifact.relative_path) {
                Ok(revision) => revision,
                Err(err) => return GateEvaluation::Fail(err.diagnostic()),
            };
            if let Err(err) =
                require_validation_verdict(&doc, artifact.relative_path, expected_verdict)
            {
                return GateEvaluation::Fail(err.diagnostic());
            }
            GateEvaluation::Pass(evidence_for_artifact(
                artifact,
                root,
                &revision,
                existing_ids,
            ))
        }
        Err(err) => GateEvaluation::Fail(err.diagnostic()),
    }
}

fn subject_artifact_for_review(relative_path: &str) -> crate::evidence::ArtifactRef {
    match relative_path {
        "reviews/design-review.json" => artifacts::DESIGN,
        "reviews/plan-review.json" => artifacts::PLAN,
        _ => artifacts::IMPLEMENTATION,
    }
}

fn event_matches_review(event: &str, expected: ReviewVerdict) -> bool {
    matches!(
        (event, expected),
        ("approved", ReviewVerdict::Approved)
            | ("changes-requested", ReviewVerdict::ChangesRequested)
    )
}

fn event_matches_validation(event: &str, expected: ValidationVerdict) -> bool {
    matches!(
        (event, expected),
        ("passed", ValidationVerdict::Passed) | ("failed", ValidationVerdict::Failed)
    )
}

fn collect_existing_ids(payload: &GatePayloadDto) -> Vec<String> {
    payload
        .selected_evidence
        .iter()
        .chain(payload.inline_evidence.iter())
        .map(|evidence| evidence.id.clone())
        .collect()
}

fn fail_all(payload: &GatePayloadDto, _diagnostic: DiagnosticDto) -> GateResultDto {
    GateResultDto::Verdicts {
        verdicts: payload
            .required_gate_ids
            .iter()
            .map(|gate_id| GateVerdictDto {
                gate_id: gate_id.clone(),
                passed: false,
            })
            .collect(),
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GATE_LEGACY_STORED_ONLY, STATE_DESIGN, STATE_DESIGN_REVIEW, STATE_EXPLORE};
    use crate::protocol::{CanonicalGraphDto, RunSnapshotDto};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_artifact_root() -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("reference-provider-gates-{id}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_json(path: &Path, value: serde_json::Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    fn snapshot(root: &Path, state: &str) -> RunSnapshotDto {
        RunSnapshotDto {
            run_id: "run-1".to_string(),
            registration_id: crate::graph::REGISTRATION_ID.to_string(),
            graph_revision: "sha256:test".to_string(),
            lifecycle: "active".to_string(),
            current_state: state.to_string(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            inputs: BTreeMap::from([(
                "artifact_root".to_string(),
                json!(root.display().to_string()),
            )]),
            stored_graph: CanonicalGraphDto {
                canonical_graph_version: 1,
                initial_state_id: STATE_EXPLORE.to_string(),
                input_declarations: vec![],
                live_guidance_supported: true,
                metadata: None,
                states: vec![],
                transitions: vec![],
            },
        }
    }

    #[test]
    fn intent_ready_passes_with_valid_artifact() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_INTENT_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, evidence } => {
                assert!(verdicts.iter().all(|v| v.passed));
                assert!(evidence.is_some());
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn missing_artifact_fails_gate() {
        let temp = temp_artifact_root();
        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_INTENT_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, .. } => {
                assert!(verdicts.iter().all(|v| !v.passed));
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn verdict_mismatch_fails_review_gate() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("design.json"),
            json!({"revision":"1","intent_revision":"1"}),
        );
        write_json(
            &temp.join("reviews/design-review.json"),
            json!({"revision":"1","subject_revision":"1","verdict":"changes_requested"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_DESIGN_REVIEW),
            event: "approved".to_string(),
            required_gate_ids: vec![GATE_DESIGN_REVIEW_APPROVED.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, .. } => {
                assert!(!verdicts[0].passed);
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn append_only_evidence_id_for_same_locator_new_revision() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"2","summary":"intent"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_INTENT_READY.to_string()],
            selected_evidence: vec![EvidenceDto {
                id: "intent-document-1".to_string(),
                kind: "intent-document".to_string(),
                locator: format!("file://{}/intent.json", temp.display()),
                digest: None,
                media_type: None,
                metadata: None,
                observed_at: None,
            }],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { evidence, .. } => {
                let evidence = evidence.expect("expected evidence");
                assert_eq!(evidence[0].id, "intent-document-2");
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn unsupported_stored_gate_returns_incompatible_not_failed_verdict() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_LEGACY_STORED_ONLY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Incompatible { diagnostics } => {
                assert_eq!(diagnostics[0].code, "compatibility.unsupported");
                assert!(diagnostics[0].message.contains(GATE_LEGACY_STORED_ONLY));
            }
            GateResultDto::Verdicts { verdicts, .. } => {
                panic!("expected incompatible, got failed verdicts: {verdicts:?}");
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn design_ready_requires_intent_revision_field() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );
        write_json(
            &temp.join("design.json"),
            json!({"revision":"1","subject_revision":"1"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_DESIGN),
            event: "design-ready".to_string(),
            required_gate_ids: vec![GATE_DESIGN_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, .. } => assert!(!verdicts[0].passed),
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn design_ready_rejects_wrong_intent_revision_link() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );
        write_json(
            &temp.join("design.json"),
            json!({"revision":"1","intent_revision":"2"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_DESIGN),
            event: "design-ready".to_string(),
            required_gate_ids: vec![GATE_DESIGN_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, .. } => assert!(!verdicts[0].passed),
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn invalid_artifact_json_fails_gate() {
        let temp = temp_artifact_root();
        fs::write(temp.join("intent.json"), "not-json").unwrap();

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_INTENT_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };

        let result = evaluate_gates(&payload, &ProviderConfig::default());
        match result {
            GateResultDto::Verdicts { verdicts, .. } => assert!(!verdicts[0].passed),
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn malformed_evidence_mode_emits_invalid_provider_evidence() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );

        let payload = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_EXPLORE),
            event: "intent-ready".to_string(),
            required_gate_ids: vec![GATE_INTENT_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };
        let config = ProviderConfig {
            malformed_evidence: true,
            ..ProviderConfig::default()
        };

        let result = evaluate_gates(&payload, &config);
        match result {
            GateResultDto::Verdicts { verdicts, evidence } => {
                assert!(verdicts.iter().all(|v| v.passed));
                let evidence = evidence.expect("expected malformed evidence");
                assert_eq!(evidence[0].id, "");
                assert_eq!(evidence[0].kind, "invalid");
            }
            other => panic!("unexpected result {other:?}"),
        }
    }

    #[test]
    fn design_revision_cycle_produces_new_evidence_id() {
        let temp = temp_artifact_root();
        write_json(
            &temp.join("intent.json"),
            json!({"revision":"1","summary":"intent"}),
        );
        write_json(
            &temp.join("design.json"),
            json!({"revision":"1","intent_revision":"1"}),
        );

        let first = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_DESIGN),
            event: "design-ready".to_string(),
            required_gate_ids: vec![GATE_DESIGN_READY.to_string()],
            selected_evidence: vec![],
            inline_evidence: vec![],
        };
        let first_result = evaluate_gates(&first, &ProviderConfig::default());
        let first_id = match first_result {
            GateResultDto::Verdicts { evidence, .. } => {
                evidence.expect("first evidence")[0].id.clone()
            }
            other => panic!("unexpected first result {other:?}"),
        };

        write_json(
            &temp.join("design.json"),
            json!({"revision":"2","intent_revision":"1"}),
        );
        let second = GatePayloadDto {
            snapshot: snapshot(&temp, STATE_DESIGN),
            event: "design-ready".to_string(),
            required_gate_ids: vec![GATE_DESIGN_READY.to_string()],
            selected_evidence: vec![EvidenceDto {
                id: first_id.clone(),
                kind: "design-document".to_string(),
                locator: format!("file://{}/design.json", temp.display()),
                digest: None,
                media_type: None,
                metadata: None,
                observed_at: None,
            }],
            inline_evidence: vec![],
        };
        let second_result = evaluate_gates(&second, &ProviderConfig::default());
        match second_result {
            GateResultDto::Verdicts { evidence, .. } => {
                let evidence = evidence.expect("second evidence");
                assert_eq!(evidence[0].id, "design-document-2");
                assert_ne!(evidence[0].id, first_id);
            }
            other => panic!("unexpected second result {other:?}"),
        }
    }
}
