//! Transition compatibility and complete provider evaluation.
//!
//! This is the only module that knows transition routing.  Configuration,
//! artifact, schema, and evidence modules expose semantic projections; this
//! module composes them in a fixed order: schema, revision links, then evidence.

use crate::artifacts::{
    check_revision_link, extract_metadata, read_artifact, ArtifactReadDeny, ArtifactReadOutcome,
    LinkCheckOutcome, LinkViolation,
};
use crate::config::parse_initial_input;
use crate::evidence::{evaluate_evidence, AuthorIdentity as EvidenceAuthorIdentity};
use crate::protocol::{allow_response, deny_response, unsupported_response, EvaluateRequest};
use loop_core::{DurableEvaluationResult, Transition, TransitionKind};
use serde_json::{json, Value};
use std::fmt;

const SCHEMA_DENY_CODE: &str = "research-schema-invalid";
const EVIDENCE_DENY_CODE: &str = "research-review-incomplete";
const SCHEMA_DENY_MESSAGE: &str = "not judged: fix shape first";
const EVIDENCE_DENY_MESSAGE: &str = "review evidence incomplete";

/// Evaluation result consumed by `main.rs` for exit-code handling.
pub(crate) enum EvaluationOutcome {
    Response(Value),
    EvaluationError(String),
}

/// One exact transition entry from the research topology.
#[derive(Clone, Copy, Debug)]
struct Route {
    source: &'static str,
    event: &'static str,
    target: &'static str,
    kind: TransitionKind,
    subject: Option<&'static str>,
    gate: Option<&'static str>,
}

const ROUTES: &[Route] = &[
    Route {
        source: "scope",
        event: "scoped",
        target: "gather",
        kind: TransitionKind::Checked,
        subject: Some("brief.json"),
        gate: None,
    },
    Route {
        source: "gather",
        event: "gathered",
        target: "verify",
        kind: TransitionKind::Checked,
        subject: Some("sources.json"),
        gate: None,
    },
    Route {
        source: "gather",
        event: "revise",
        target: "scope",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
    Route {
        source: "verify",
        event: "verified",
        target: "synthesize",
        kind: TransitionKind::Checked,
        subject: Some("verification.json"),
        gate: Some("verify"),
    },
    Route {
        source: "verify",
        event: "revise",
        target: "gather",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
    Route {
        source: "verify",
        event: "revise-brief",
        target: "scope",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
    Route {
        source: "synthesize",
        event: "completed",
        target: "end",
        kind: TransitionKind::Checked,
        subject: Some("report.json"),
        gate: Some("synthesize"),
    },
    Route {
        source: "synthesize",
        event: "revise",
        target: "verify",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
    Route {
        source: "synthesize",
        event: "revise-sources",
        target: "gather",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
    Route {
        source: "synthesize",
        event: "revise-brief",
        target: "scope",
        kind: TransitionKind::CheckFree,
        subject: None,
        gate: None,
    },
];

/// Evaluate one already-parsed request.  Request parsing and protocol errors
/// remain owned by `main.rs`; this function only returns result JSON or an
/// evaluation-error diagnostic.
pub(crate) fn evaluate(request: &EvaluateRequest) -> EvaluationOutcome {
    // The workflow snapshot is part of the frozen wire envelope.  Routing is
    // provider-owned static topology, so this opaque snapshot is validated by
    // serde but does not alter the fixed reference route.
    let _ = &request.workflow;

    let Some(route) = route_for(&request.transition) else {
        return EvaluationOutcome::Response(unsupported_response());
    };

    let config = match parse_initial_input(&request.initial_input) {
        Ok(config) => config,
        Err(error) => return EvaluationOutcome::EvaluationError(error.to_string()),
    };

    let Some(subject) = route.subject else {
        return EvaluationOutcome::Response(allow_response());
    };

    let schema = config.schema(subject);
    let links = config.links_from(subject);
    let axes = route
        .gate
        .and_then(|gate| config.axes_for_gate(gate))
        .filter(|axes| !axes.is_empty());

    if schema.is_none() && links.is_empty() && axes.is_none() {
        return EvaluationOutcome::Response(allow_response());
    }

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

        if axes.is_some() {
            match extract_metadata(subject, document.value()) {
                Ok(value) => metadata = Some(value),
                Err(error) => return EvaluationOutcome::EvaluationError(error.to_string()),
            }
        }
    }

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

    if let (Some(gate), Some(axes), Some(metadata)) = (route.gate, axes, metadata.as_ref()) {
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

    EvaluationOutcome::Response(allow_response())
}

fn route_for(transition: &Transition) -> Option<Route> {
    ROUTES.iter().copied().find(|route| {
        route.source == transition.source.as_str()
            && route.event == transition.event.as_str()
            && route.target == transition.target.as_str()
            && route.kind == transition.kind
    })
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
    use loop_core::{EventId, StateId};

    fn transition(source: &str, event: &str, target: &str, kind: TransitionKind) -> Transition {
        Transition::new(
            StateId::new(source),
            EventId::new(event),
            StateId::new(target),
            kind,
        )
    }

    #[test]
    fn all_reference_routes_match_exactly() {
        for route in ROUTES {
            let matched = route_for(&transition(
                route.source,
                route.event,
                route.target,
                route.kind,
            ));
            assert_eq!(matched.map(|value| value.source), Some(route.source));
        }
    }

    #[test]
    fn single_tuple_field_mismatch_is_unsupported() {
        let route = ROUTES
            .iter()
            .find(|route| route.kind == TransitionKind::Checked)
            .expect("checked route");
        let variants = [
            transition("wrong", route.event, route.target, route.kind),
            transition(route.source, "wrong", route.target, route.kind),
            transition(route.source, route.event, "wrong", route.kind),
            transition(
                route.source,
                route.event,
                route.target,
                TransitionKind::CheckFree,
            ),
        ];
        for variant in variants {
            assert!(route_for(&variant).is_none());
        }
    }
}
