//! Stored-graph compatibility checks without mutating supplied graph snapshots.

use std::collections::BTreeSet;

use crate::config::{CompatMode, ProviderConfig};
use crate::graph::{GATE_LEGACY_STORED_ONLY, SUPPORTED_GATES};
use crate::protocol::{
    CanonicalGraphDto, CompatibilityFindingDto, CompatibilityPayloadDto, CompatibilityResultDto,
    CompatibilityStatusDto, DiagnosticDto,
};

const CAPABILITY_EVALUATE_GATES: &str = "evaluate_gates";
const CAPABILITY_LIVE_GUIDANCE: &str = "live_guidance";

pub fn check_compatibility(
    payload: &CompatibilityPayloadDto,
    config: &ProviderConfig,
) -> CompatibilityResultDto {
    if config.compat_mode == CompatMode::EvaluationError {
        return CompatibilityResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation_error".to_string(),
                message: "compatibility check forced to error by fixture mode".to_string(),
                path: None,
            }],
        };
    }

    let requested = payload
        .capabilities
        .clone()
        .unwrap_or_else(default_capabilities);

    let findings = requested
        .into_iter()
        .map(|capability| finding_for_capability(&capability, &payload.stored_graph, config))
        .collect();

    CompatibilityResultDto::Findings {
        capabilities: findings,
    }
}

fn default_capabilities() -> Vec<String> {
    vec![
        CAPABILITY_EVALUATE_GATES.to_string(),
        CAPABILITY_LIVE_GUIDANCE.to_string(),
    ]
}

fn finding_for_capability(
    capability: &str,
    stored_graph: &CanonicalGraphDto,
    config: &ProviderConfig,
) -> CompatibilityFindingDto {
    match capability {
        CAPABILITY_EVALUATE_GATES => evaluate_gates_capability(stored_graph, config),
        CAPABILITY_LIVE_GUIDANCE => CompatibilityFindingDto {
            capability: capability.to_string(),
            status: if stored_graph.live_guidance_supported {
                CompatibilityStatusDto::Compatible
            } else {
                CompatibilityStatusDto::Incompatible
            },
            diagnostics: if stored_graph.live_guidance_supported {
                vec![]
            } else {
                vec![DiagnosticDto {
                    code: "compatibility.unsupported".to_string(),
                    message: "stored graph declares live guidance unsupported".to_string(),
                    path: None,
                }]
            },
        },
        other => CompatibilityFindingDto {
            capability: other.to_string(),
            status: CompatibilityStatusDto::Unknown,
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unknown".to_string(),
                message: format!("capability {other} is not evaluated by this provider"),
                path: None,
            }],
        },
    }
}

fn evaluate_gates_capability(
    stored_graph: &CanonicalGraphDto,
    config: &ProviderConfig,
) -> CompatibilityFindingDto {
    let supported: BTreeSet<&str> = SUPPORTED_GATES.iter().copied().collect();
    let mut unsupported = BTreeSet::new();

    for transition in &stored_graph.transitions {
        for gate_id in &transition.gate_ids {
            if !supported.contains(gate_id.as_str()) {
                unsupported.insert(gate_id.clone());
            }
        }
    }

    if config.compat_mode == CompatMode::Incompatible {
        unsupported.insert(GATE_LEGACY_STORED_ONLY.to_string());
    }

    if unsupported.is_empty() {
        CompatibilityFindingDto {
            capability: CAPABILITY_EVALUATE_GATES.to_string(),
            status: CompatibilityStatusDto::Compatible,
            diagnostics: vec![],
        }
    } else {
        let gates = unsupported.into_iter().collect::<Vec<_>>().join(", ");
        CompatibilityFindingDto {
            capability: CAPABILITY_EVALUATE_GATES.to_string(),
            status: CompatibilityStatusDto::Incompatible,
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unsupported".to_string(),
                message: format!("stored graph requires unsupported gates: {gates}"),
                path: None,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GATE_INTENT_READY, STATE_DESIGN, STATE_EXPLORE};
    use crate::protocol::CanonicalTransitionDto;

    fn stored_graph(gate_ids: Vec<&str>) -> CanonicalGraphDto {
        CanonicalGraphDto {
            canonical_graph_version: 1,
            initial_state_id: STATE_EXPLORE.to_string(),
            input_declarations: vec![],
            live_guidance_supported: true,
            metadata: None,
            states: vec![],
            transitions: vec![CanonicalTransitionDto {
                event_id: "intent-ready".to_string(),
                gate_ids: gate_ids.into_iter().map(str::to_string).collect(),
                metadata: None,
                source_state_id: STATE_EXPLORE.to_string(),
                target_state_id: STATE_DESIGN.to_string(),
            }],
        }
    }

    #[test]
    fn compatible_when_all_gates_supported() {
        let payload = CompatibilityPayloadDto {
            stored_graph: stored_graph(vec![GATE_INTENT_READY]),
            capabilities: None,
        };
        let result = check_compatibility(&payload, &ProviderConfig::default());
        match result {
            CompatibilityResultDto::Findings { capabilities } => {
                assert!(capabilities.iter().any(|finding| {
                    finding.capability == CAPABILITY_EVALUATE_GATES
                        && finding.status == CompatibilityStatusDto::Compatible
                }));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn incompatible_mode_flags_legacy_gate_even_when_absent_from_graph() {
        let payload = CompatibilityPayloadDto {
            stored_graph: stored_graph(vec![GATE_INTENT_READY]),
            capabilities: Some(vec![CAPABILITY_EVALUATE_GATES.to_string()]),
        };
        let config = ProviderConfig {
            compat_mode: CompatMode::Incompatible,
            ..ProviderConfig::default()
        };
        let result = check_compatibility(&payload, &config);
        match result {
            CompatibilityResultDto::Findings { capabilities } => {
                assert!(
                    capabilities
                        .iter()
                        .any(|finding| { finding.status == CompatibilityStatusDto::Incompatible })
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn stored_graph_with_unknown_gate_is_incompatible() {
        let payload = CompatibilityPayloadDto {
            stored_graph: stored_graph(vec!["legacy-intent-ready"]),
            capabilities: Some(vec![CAPABILITY_EVALUATE_GATES.to_string()]),
        };
        let result = check_compatibility(&payload, &ProviderConfig::default());
        match result {
            CompatibilityResultDto::Findings { capabilities } => {
                assert!(
                    capabilities
                        .iter()
                        .any(|finding| { finding.status == CompatibilityStatusDto::Incompatible })
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
