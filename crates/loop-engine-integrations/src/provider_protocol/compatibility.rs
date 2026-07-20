use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    CompatibilityRequest, CompatibilityResult, InvocationError,
};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::provider::ProviderObservation;

use super::canonical::graph_dto;
use super::describe::observed_now;
use super::dto::{CompatibilityPayloadDto, CompatibilityResultDto, ProviderRole};
use super::invoke::{AdapterError, invoke, mapping_failure, result_fact};
use super::mapping::compatibility as map_compatibility;
use crate::provider_process::TracedProviderBoundary;

pub fn check_compatibility(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request: CompatibilityRequest,
) -> Result<CompatibilityResult, InvocationError<AdapterError>> {
    let graph = ValidatedGraph::validate(request.run.run().graph().clone())
        .expect("stored run graph remains valid");
    let payload = CompatibilityPayloadDto {
        stored_graph: graph_dto(&SemanticGraphProjection::from_validated(&graph)),
        capabilities: None,
    };
    let wire = invoke::<_, CompatibilityResultDto>(
        boundary,
        config,
        &request.request_id,
        ProviderRole::CheckCompatibility,
        payload,
        true,
    )?;
    let report = map_compatibility(wire.result.clone()).map_err(|error| {
        mapping_failure(
            config,
            &request.request_id,
            ProviderRole::CheckCompatibility,
            &wire,
            error.to_string(),
        )
    })?;
    let observation = ProviderObservation::new(
        config.registration_id().clone(),
        config.config().executable(),
        wire.digest.clone(),
        wire.provider_version.clone(),
        observed_now(),
    )
    .expect("resolved provider observation satisfies core bounds");
    let outcome = match &report {
        loop_engine_core::model::compatibility::CompatibilityReport::Findings(_) => {
            OutcomeClass::Completed
        }
        loop_engine_core::model::compatibility::CompatibilityReport::EvaluationError(_) => {
            OutcomeClass::Error
        }
    };
    let trace_failure = wire.complete_trace().err();
    Ok(CompatibilityResult {
        report,
        observation,
        fact: result_fact(
            config,
            &request.request_id,
            ProviderRole::CheckCompatibility,
            wire.digest,
            wire.provider_version,
            wire.protocol_major,
            outcome,
        ),
        protocol_major: wire.protocol_major,
        trace_failure,
    })
}
