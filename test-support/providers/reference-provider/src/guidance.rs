//! Live guidance generation for active runs.

use crate::config::{GuidanceMode, ProviderConfig};
use crate::graph::{
    STATE_DESIGN, STATE_DESIGN_REVIEW, STATE_END, STATE_EXPLORE, STATE_IMPLEMENT,
    STATE_IMPLEMENTATION_REVIEW, STATE_PLAN, STATE_PLAN_REVIEW, STATE_VALIDATION,
};
use crate::protocol::{DiagnosticDto, GuidancePayloadDto, GuidanceResultDto};

pub fn live_guidance(payload: &GuidancePayloadDto, config: &ProviderConfig) -> GuidanceResultDto {
    match config.guidance_mode {
        GuidanceMode::Incompatible => {
            return GuidanceResultDto::Incompatible {
                diagnostics: vec![DiagnosticDto {
                    code: "compatibility.unsupported".to_string(),
                    message: "live guidance disabled by fixture mode".to_string(),
                    path: None,
                }],
            };
        }
        GuidanceMode::EvaluationError => {
            return GuidanceResultDto::EvaluationError {
                diagnostics: vec![DiagnosticDto {
                    code: "provider.evaluation_error".to_string(),
                    message: "live guidance forced to error by fixture mode".to_string(),
                    path: None,
                }],
            };
        }
        GuidanceMode::Default | GuidanceMode::RecommendEvidence => {}
    }

    let state = payload.snapshot.current_state.as_str();
    let mut text = static_guidance_for_state(state);

    if config.guidance_mode == GuidanceMode::RecommendEvidence {
        if payload.selected_evidence.is_empty() {
            text.push_str(
                "\nRecommended evidence: produce the next required artifact under artifact_root.",
            );
        } else {
            let ids = payload
                .selected_evidence
                .iter()
                .map(|evidence| evidence.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str("\nRecommended evidence IDs for context: ");
            text.push_str(&ids);
            text.push_str(" (advisory only; does not authorize transition).");
        }
    }

    GuidanceResultDto::Guidance { text }
}

fn static_guidance_for_state(state: &str) -> String {
    match state {
        STATE_EXPLORE => "Capture intent under artifact_root/intent.json, then request intent-ready."
            .to_string(),
        STATE_DESIGN => {
            "Author design.json linked to the accepted intent revision, then request design-ready."
                .to_string()
        }
        STATE_DESIGN_REVIEW => "Record design-review.json for the current design revision with an approving or revision verdict."
            .to_string(),
        STATE_PLAN => {
            "Author plan.json linked to the approved design revision, then request plan-ready."
                .to_string()
        }
        STATE_PLAN_REVIEW => "Record plan-review.json for the current plan revision with an approving or revision verdict."
            .to_string(),
        STATE_IMPLEMENT => "Persist implementation.json linked to the approved plan revision, then request implementation-ready."
            .to_string(),
        STATE_IMPLEMENTATION_REVIEW => "Record implementation-review.json for the current implementation revision."
            .to_string(),
        STATE_VALIDATION => {
            "Record validation.json with passed or failed verdict matching the requested event."
                .to_string()
        }
        STATE_END => "No further work remains.".to_string(),
        other => format!("Continue work for state {other} using stored static guidance."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::REGISTRATION_ID;
    use crate::protocol::{CanonicalGraphDto, EvidenceDto, RunSnapshotDto};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn payload(state: &str, selected: Vec<EvidenceDto>) -> GuidancePayloadDto {
        GuidancePayloadDto {
            snapshot: RunSnapshotDto {
                run_id: "run-1".to_string(),
                registration_id: REGISTRATION_ID.to_string(),
                graph_revision: "sha256:test".to_string(),
                lifecycle: "active".to_string(),
                current_state: state.to_string(),
                workflow_state_version: 1,
                lifecycle_version: 1,
                inputs: BTreeMap::from([("artifact_root".to_string(), json!("/tmp/artifacts"))]),
                stored_graph: CanonicalGraphDto {
                    canonical_graph_version: 1,
                    initial_state_id: STATE_EXPLORE.to_string(),
                    input_declarations: vec![],
                    live_guidance_supported: true,
                    metadata: None,
                    states: vec![],
                    transitions: vec![],
                },
            },
            selected_evidence: selected,
        }
    }

    #[test]
    fn default_guidance_is_state_specific() {
        let result = live_guidance(&payload(STATE_PLAN, vec![]), &ProviderConfig::default());
        match result {
            GuidanceResultDto::Guidance { text } => assert!(text.contains("plan.json")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn recommend_mode_mentions_selected_evidence() {
        let config = ProviderConfig {
            guidance_mode: GuidanceMode::RecommendEvidence,
            ..ProviderConfig::default()
        };
        let result = live_guidance(
            &payload(
                STATE_DESIGN,
                vec![EvidenceDto {
                    id: "intent-document-1".to_string(),
                    kind: "intent-document".to_string(),
                    locator: "file:///tmp/intent.json".to_string(),
                    digest: None,
                    media_type: None,
                    metadata: None,
                    observed_at: None,
                }],
            ),
            &config,
        );
        match result {
            GuidanceResultDto::Guidance { text } => {
                assert!(text.contains("intent-document-1"));
                assert!(text.contains("advisory only"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn incompatible_mode_returns_incompatible() {
        let config = ProviderConfig {
            guidance_mode: GuidanceMode::Incompatible,
            ..ProviderConfig::default()
        };
        let result = live_guidance(&payload(STATE_EXPLORE, vec![]), &config);
        assert!(matches!(result, GuidanceResultDto::Incompatible { .. }));
    }
}
