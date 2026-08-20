//! Transition compatibility and complete provider evaluation.
//!
//! Topology and source-state duties come from the union phase table in
//! `workflow`. This module composes configuration, artifact, schema, and
//! evidence projections in technical-design §9's fixed order. A static
//! combinatorial table of every rewired target is not the source of topology.

use crate::artifacts::{
    check_revision_link, extract_metadata, read_artifact, ArtifactReadDeny, ArtifactReadOutcome,
    LinkCheckOutcome, LinkViolation,
};
use crate::config::parse_initial_input;
use crate::evidence::{
    evaluate_accepted_findings, evaluate_evidence, AuthorIdentity as EvidenceAuthorIdentity,
};
use crate::protocol::{allow_response, deny_response, unsupported_response, EvaluateRequest};
use crate::workflow::{self, TransitionDuties};
use loop_core::{DurableEvaluationResult, TransitionKind};
use serde_json::{json, Value};
use std::fmt;

const SCHEMA_DENY_CODE: &str = "software-change-schema-invalid";
const EVIDENCE_DENY_CODE: &str = "software-change-review-incomplete";
const ACCEPTED_FINDINGS_DENY_CODE: &str = "software-change-accepted-findings-missing";
const SCHEMA_DENY_MESSAGE: &str = "not judged: fix shape first";
const EVIDENCE_DENY_MESSAGE: &str = "review evidence incomplete";
const ACCEPTED_FINDINGS_DENY_MESSAGE: &str = "accepted findings missing or malformed";

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
    // deliberately untouched by parse_initial_input.
    let config = match parse_initial_input(&request.initial_input) {
        Ok(config) => config,
        Err(error) => return EvaluationOutcome::EvaluationError(error.to_string()),
    };

    // Check-free edges are listed for complete compatibility but the provider
    // is not normally invoked for them.  They have no subject or obligations,
    // so they take the same zero-obligation result path.
    let TransitionDuties::Checked { subject, gate } = duties else {
        return EvaluationOutcome::Response(allow_response());
    };

    let schema = config.schema(subject);
    let links = config.links_from(subject);
    let axes = gate
        .and_then(|gate| config.axes_for_gate(gate))
        .filter(|axes| !axes.is_empty());

    // §9.4: no configured schema, link, or semantic axis means allow without
    // requiring artifact_root and without consulting context.
    if schema.is_none() && links.is_empty() && axes.is_none() {
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
        if !report.is_valid() {
            let violations = report
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
            return EvaluationOutcome::Response(schema_deny(request, violations));
        }

        // B2 guarantees these fields through the configured schema whenever
        // semantic evidence or accepted-findings is configured.  Schema-only
        // draft hops need no metadata extraction.
        if gate.is_some() {
            match extract_metadata(subject, document.value()) {
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

    // §9.6: evidence is consulted only after all deterministic checks pass.
    if let (Some(gate), Some(axes), Some(metadata)) = (gate, axes, metadata.as_ref()) {
        let subject_author =
            EvidenceAuthorIdentity::new(metadata.author().name(), metadata.author().kind());
        let evidence = evaluate_evidence(
            &request.context,
            gate,
            subject,
            metadata.revision(),
            &subject_author,
            config.config_version(),
            axes,
            config.axis_namespace(),
        );
        if !evidence.is_satisfied() {
            let details = evidence.details_value();
            let details = json!({
                "phase": "evidence",
                "diagnostics": details.get("diagnostics").cloned().unwrap_or(Value::Array(Vec::new())),
                "informational": details.get("informational").cloned().unwrap_or(Value::Array(Vec::new())),
                "inert_records": details.get("inert_records").cloned().unwrap_or(Value::Array(Vec::new())),
                "prior_denials": prior_denials(request),
            });
            return EvaluationOutcome::Response(deny_response(
                EVIDENCE_DENY_CODE,
                EVIDENCE_DENY_MESSAGE,
                Some(details),
            ));
        }
    }

    // Live review approved/passed also requires a well-formed current-revision
    // accepted-findings record. This is shape/presence only.
    if let (Some(gate), Some(metadata)) = (gate, metadata.as_ref()) {
        let findings =
            evaluate_accepted_findings(&request.context, gate, subject, metadata.revision());
        if !findings.is_satisfied() {
            let mut details = findings
                .details_value()
                .as_object()
                .cloned()
                .unwrap_or_default();
            details.insert("phase".to_owned(), json!("accepted-findings"));
            details.insert("prior_denials".to_owned(), prior_denials(request));
            return EvaluationOutcome::Response(deny_response(
                ACCEPTED_FINDINGS_DENY_CODE,
                ACCEPTED_FINDINGS_DENY_MESSAGE,
                Some(Value::Object(details)),
            ));
        }
    }

    EvaluationOutcome::Response(allow_response())
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
    let duties = workflow::duties_for(transition.source.as_str(), transition.event.as_str())?;
    match (duties, transition.kind) {
        (TransitionDuties::Checked { .. }, TransitionKind::Checked)
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
    fn union_transitions_match_phase_table_duties() {
        let workflow = workflow::software_change_workflow();
        for edge in workflow.transitions.clone() {
            let duties =
                duties_for_snapshotted_transition(&request(workflow.clone(), edge.clone()))
                    .unwrap_or_else(|| panic!("missing duties for {edge:?}"));
            match (duties, edge.kind) {
                (TransitionDuties::Checked { .. }, TransitionKind::Checked)
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
                (TransitionDuties::Checked { .. }, TransitionKind::Checked)
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
