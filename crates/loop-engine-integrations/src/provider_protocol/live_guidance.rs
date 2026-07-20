use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    GuidanceInvocationResult, GuidanceRequest, InvocationError,
};
use loop_engine_core::model::live_guidance::{AdvisoryGuidance, LiveGuidanceResult};
use loop_engine_core::model::outcome::OutcomeClass;

use super::context::{evidence_dto, run_snapshot};
use super::dto::{GuidancePayloadDto, GuidanceResultDto, ProviderRole};
use super::invoke::{AdapterError, invoke, mapping_failure, result_fact};
use super::mapping::diagnostics;
use crate::provider_process::TracedProviderBoundary;

pub fn live_guidance(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request: GuidanceRequest,
) -> Result<GuidanceInvocationResult, InvocationError<AdapterError>> {
    let payload = GuidancePayloadDto {
        snapshot: run_snapshot(request.run.run()),
        selected_evidence: request
            .selected_evidence
            .records()
            .iter()
            .map(evidence_dto)
            .collect(),
    };
    let wire = invoke::<_, GuidanceResultDto>(
        boundary,
        config,
        &request.request_id,
        ProviderRole::LiveGuidance,
        payload,
        true,
    )?;
    if wire.result_value.get("evidence").is_some() {
        return Err(mapping_failure(
            config,
            &request.request_id,
            ProviderRole::LiveGuidance,
            &wire,
            "live-guidance result cannot carry evidence",
        ));
    }
    let result = match wire.result.clone() {
        GuidanceResultDto::Guidance { text } => {
            LiveGuidanceResult::Guidance(AdvisoryGuidance::new(text).map_err(|error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::LiveGuidance,
                    &wire,
                    error.to_string(),
                )
            })?)
        }
        GuidanceResultDto::Incompatible {
            diagnostics: values,
        } => LiveGuidanceResult::Incompatible(diagnostics(values, "/result/diagnostics").map_err(
            |error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::LiveGuidance,
                    &wire,
                    error.to_string(),
                )
            },
        )?),
        GuidanceResultDto::EvaluationError {
            diagnostics: values,
        } => LiveGuidanceResult::EvaluationError(
            diagnostics(values, "/result/diagnostics").map_err(|error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::LiveGuidance,
                    &wire,
                    error.to_string(),
                )
            })?,
        ),
    };
    let outcome = match &result {
        LiveGuidanceResult::Guidance(_) => OutcomeClass::Completed,
        LiveGuidanceResult::Incompatible(_) => OutcomeClass::Rejected,
        LiveGuidanceResult::EvaluationError(_) => OutcomeClass::Error,
    };
    let trace_failure = wire.complete_trace().err();
    Ok(GuidanceInvocationResult {
        result,
        fact: result_fact(
            config,
            &request.request_id,
            ProviderRole::LiveGuidance,
            wire.digest,
            wire.provider_version,
            wire.protocol_major,
            outcome,
        ),
        trace_failure,
    })
}
