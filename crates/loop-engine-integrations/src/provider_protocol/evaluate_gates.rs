use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    GateInvocationResult, GateRequest, InvocationError,
};
use loop_engine_core::model::gate::{GateEvaluation, GateVerdict, validate_verdict_set};
use loop_engine_core::model::outcome::OutcomeClass;

use super::context::{evidence_dto, provider_evidence, run_snapshot};
use super::dto::{GatePayloadDto, GateResultDto, ProviderRole};
use super::invoke::{
    AdapterError, invoke, mapping_failure, mapping_failure_with_code, result_fact,
};
use super::mapping::{diagnostics, parse_gate_id};
use crate::provider_process::TracedProviderBoundary;

pub fn evaluate_gates(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request: GateRequest,
) -> Result<GateInvocationResult, InvocationError<AdapterError>> {
    let transition = request
        .run
        .run()
        .graph()
        .transitions()
        .iter()
        .find(|transition| {
            transition.source() == request.run.run().current_state()
                && transition.event() == &request.event
        })
        .expect("core gate request selects a stored transition");
    let required = transition.required_gates().to_vec();
    let payload = GatePayloadDto {
        snapshot: run_snapshot(request.run.run()),
        event: request.event.as_str().to_owned(),
        required_gate_ids: required
            .iter()
            .map(|gate| gate.as_str().to_owned())
            .collect(),
        selected_evidence: request
            .selected_evidence
            .records()
            .iter()
            .map(evidence_dto)
            .collect(),
        inline_evidence: request
            .inline_evidence
            .records()
            .iter()
            .map(evidence_dto)
            .collect(),
    };
    let wire = invoke::<_, GateResultDto>(
        boundary,
        config,
        &request.request_id,
        ProviderRole::EvaluateGates,
        payload,
        true,
    )?;
    let evaluation = match wire.result.clone() {
        GateResultDto::Verdicts { verdicts, evidence } => {
            let mut evidence = evidence
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    provider_evidence(value, &format!("/result/evidence/{index}"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    mapping_failure_with_code(
                        config,
                        &request.request_id,
                        ProviderRole::EvaluateGates,
                        &wire,
                        "provider.evidence.malformed",
                        error.to_string(),
                    )
                })?;
            let mut mapped = verdicts
                .into_iter()
                .enumerate()
                .map(|(index, verdict)| {
                    Ok(GateVerdict::new(
                        parse_gate_id(
                            verdict.gate_id,
                            &format!("/result/verdicts/{index}/gate_id"),
                        )?,
                        verdict.passed,
                        Vec::new(),
                    ))
                })
                .collect::<Result<Vec<_>, super::mapping::MappingError>>()
                .map_err(|error| {
                    mapping_failure(
                        config,
                        &request.request_id,
                        ProviderRole::EvaluateGates,
                        &wire,
                        error.to_string(),
                    )
                })?;
            validate_verdict_set(&required, &mapped).map_err(|error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::EvaluateGates,
                    &wire,
                    error.to_string(),
                )
            })?;
            if let Some(first) = mapped.first_mut() {
                *first = GateVerdict::new(
                    first.gate().clone(),
                    first.passed(),
                    std::mem::take(&mut evidence),
                );
            } else if !evidence.is_empty() {
                return Err(mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::EvaluateGates,
                    &wire,
                    "provider evidence requires at least one gate verdict",
                ));
            }
            GateEvaluation::verdicts(mapped)
        }
        GateResultDto::Incompatible {
            diagnostics: values,
        } => {
            if wire.result_value.get("evidence").is_some() {
                return Err(mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::EvaluateGates,
                    &wire,
                    "incompatible result cannot carry evidence",
                ));
            }
            GateEvaluation::Incompatible(diagnostics(values, "/result/diagnostics").map_err(
                |error| {
                    mapping_failure(
                        config,
                        &request.request_id,
                        ProviderRole::EvaluateGates,
                        &wire,
                        error.to_string(),
                    )
                },
            )?)
        }
        GateResultDto::EvaluationError {
            diagnostics: values,
        } => {
            if wire.result_value.get("evidence").is_some() {
                return Err(mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::EvaluateGates,
                    &wire,
                    "evaluation-error result cannot carry evidence",
                ));
            }
            GateEvaluation::EvaluationError(diagnostics(values, "/result/diagnostics").map_err(
                |error| {
                    mapping_failure(
                        config,
                        &request.request_id,
                        ProviderRole::EvaluateGates,
                        &wire,
                        error.to_string(),
                    )
                },
            )?)
        }
    };
    let outcome = match &evaluation {
        GateEvaluation::Verdicts(_) => OutcomeClass::Completed,
        GateEvaluation::Incompatible(_) => OutcomeClass::Rejected,
        GateEvaluation::EvaluationError(_) => OutcomeClass::Error,
    };
    let trace_failure = wire.complete_trace().err();
    Ok(GateInvocationResult {
        evaluation,
        fact: result_fact(
            config,
            &request.request_id,
            ProviderRole::EvaluateGates,
            wire.digest,
            wire.provider_version,
            wire.protocol_major,
            outcome,
        ),
        trace_failure,
    })
}
