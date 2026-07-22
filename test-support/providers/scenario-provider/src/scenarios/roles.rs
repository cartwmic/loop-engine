use crate::protocol::{
    CompatibilityFindingDto, CompatibilityPayloadDto, CompatibilityResultDto,
    CompatibilityStatusDto, DiagnosticDto, EvidenceDto, GatePayloadDto, GateResultDto,
    GateVerdictDto, GuidancePayloadDto, GuidanceResultDto,
};
use crate::scenarios::Scenario;

pub fn evaluate_gates(scenario: Scenario, payload: GatePayloadDto) -> GateResultDto {
    match scenario {
        Scenario::GatePass => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                })
                .collect(),
            evidence: Some(vec![]),
        },
        Scenario::GateFail => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: false,
                })
                .collect(),
            evidence: Some(vec![]),
        },
        Scenario::GateMixed => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .enumerate()
                .map(|(index, gate_id)| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: index.is_multiple_of(2),
                })
                .collect(),
            evidence: Some(vec![]),
        },
        Scenario::GateExactSetViolation => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .take(payload.required_gate_ids.len().saturating_sub(1))
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                })
                .chain(std::iter::once(GateVerdictDto {
                    gate_id: "unexpected-gate".into(),
                    passed: true,
                }))
                .collect(),
            evidence: Some(vec![]),
        },
        Scenario::GateCallerEvidence => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: !payload.selected_evidence.is_empty()
                        || !payload.inline_evidence.is_empty(),
                })
                .collect(),
            evidence: Some(payload.selected_evidence),
        },
        Scenario::GateProviderEvidence => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                })
                .collect(),
            evidence: Some(vec![EvidenceDto {
                id: "provider-evidence-1".into(),
                kind: "test".into(),
                locator: "scenario://provider/evidence/1".into(),
                digest: None,
                media_type: None,
                metadata: None,
                observed_at: None,
            }]),
        },
        Scenario::GateProviderEvidenceDuplicate => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                })
                .collect(),
            evidence: Some(vec![
                EvidenceDto {
                    id: "duplicate-evidence-id".into(),
                    kind: "test".into(),
                    locator: "scenario://provider/evidence/dup-a".into(),
                    digest: None,
                    media_type: None,
                    metadata: None,
                    observed_at: None,
                },
                EvidenceDto {
                    id: "duplicate-evidence-id".into(),
                    kind: "test".into(),
                    locator: "scenario://provider/evidence/dup-b".into(),
                    digest: None,
                    media_type: None,
                    metadata: None,
                    observed_at: None,
                },
            ]),
        },
        Scenario::GateProviderEvidenceCollision => {
            let collision_id = payload
                .selected_evidence
                .first()
                .or_else(|| payload.inline_evidence.first())
                .map(|evidence| evidence.id.clone())
                .unwrap_or_else(|| "caller-evidence-1".into());
            GateResultDto::Verdicts {
                verdicts: payload
                    .required_gate_ids
                    .iter()
                    .map(|gate_id| GateVerdictDto {
                        gate_id: gate_id.clone(),
                        passed: true,
                    })
                    .collect(),
                evidence: Some(vec![EvidenceDto {
                    id: collision_id,
                    kind: "test".into(),
                    locator: "scenario://provider/evidence/collision".into(),
                    digest: None,
                    media_type: None,
                    metadata: None,
                    observed_at: None,
                }]),
            }
        }
        Scenario::GateIncompatible => GateResultDto::Incompatible {
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unsupported".into(),
                message: "Stored graph gate capability is unsupported.".into(),
                path: None,
            }],
        },
        Scenario::GateEvaluationError => GateResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation".into(),
                message: "Scenario-controlled gate evaluation error.".into(),
                path: None,
            }],
        },
        _ => GateResultDto::Verdicts {
            verdicts: payload
                .required_gate_ids
                .iter()
                .map(|gate_id| GateVerdictDto {
                    gate_id: gate_id.clone(),
                    passed: true,
                })
                .collect(),
            evidence: Some(vec![]),
        },
    }
}

pub fn live_guidance(scenario: Scenario, _payload: GuidancePayloadDto) -> GuidanceResultDto {
    match scenario {
        Scenario::GuidanceText => GuidanceResultDto::Guidance {
            text: "Scenario advisory guidance.".into(),
        },
        Scenario::GuidanceIncompatible => GuidanceResultDto::Incompatible {
            diagnostics: vec![DiagnosticDto {
                code: "compatibility.unsupported".into(),
                message: "Live guidance is unsupported for this run.".into(),
                path: None,
            }],
        },
        Scenario::GuidanceEvaluationError => GuidanceResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation".into(),
                message: "Scenario-controlled guidance evaluation error.".into(),
                path: None,
            }],
        },
        _ => GuidanceResultDto::Guidance {
            text: "Default scenario guidance.".into(),
        },
    }
}

pub fn check_compatibility(
    scenario: Scenario,
    payload: CompatibilityPayloadDto,
) -> CompatibilityResultDto {
    let capabilities = payload
        .capabilities
        .unwrap_or_else(|| vec!["gates".into(), "live_guidance".into()]);
    match scenario {
        Scenario::CompatibilityAllCompatible => CompatibilityResultDto::Findings {
            capabilities: capabilities
                .into_iter()
                .map(|capability| CompatibilityFindingDto {
                    capability,
                    status: CompatibilityStatusDto::Compatible,
                    diagnostics: vec![],
                })
                .collect(),
        },
        Scenario::CompatibilityIncompatible => CompatibilityResultDto::Findings {
            capabilities: capabilities
                .into_iter()
                .map(|capability| CompatibilityFindingDto {
                    capability,
                    status: CompatibilityStatusDto::Incompatible,
                    diagnostics: vec![DiagnosticDto {
                        code: "compatibility.unsupported".into(),
                        message: "Capability is incompatible with stored graph.".into(),
                        path: None,
                    }],
                })
                .collect(),
        },
        Scenario::CompatibilityMixed => CompatibilityResultDto::Findings {
            capabilities: capabilities
                .into_iter()
                .enumerate()
                .map(|(index, capability)| CompatibilityFindingDto {
                    capability,
                    status: if index.is_multiple_of(2) {
                        CompatibilityStatusDto::Compatible
                    } else {
                        CompatibilityStatusDto::Incompatible
                    },
                    diagnostics: if index.is_multiple_of(2) {
                        vec![]
                    } else {
                        vec![DiagnosticDto {
                            code: "compatibility.unsupported".into(),
                            message: "Mixed compatibility finding.".into(),
                            path: None,
                        }]
                    },
                })
                .collect(),
        },
        Scenario::CompatibilityEvaluationError => CompatibilityResultDto::EvaluationError {
            diagnostics: vec![DiagnosticDto {
                code: "provider.evaluation".into(),
                message: "Scenario-controlled compatibility evaluation error.".into(),
                path: None,
            }],
        },
        _ => CompatibilityResultDto::Findings {
            capabilities: capabilities
                .into_iter()
                .map(|capability| CompatibilityFindingDto {
                    capability,
                    status: CompatibilityStatusDto::Compatible,
                    diagnostics: vec![],
                })
                .collect(),
        },
    }
}
