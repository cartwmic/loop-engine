use jiff::Timestamp;
use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    DescribeRequest, DescribeResult, DescribedGraph, InvocationError,
};
use loop_engine_core::model::graph_validation::GraphError;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::provider::ProviderObservation;
use loop_engine_core::model::time::ObservedAt;

use super::dto::{DescribeResultDto, EmptyPayload, ProviderRole};
use super::graph::map_graph_unvalidated;
use super::invoke::{AdapterError, invoke, mapping_failure, result_fact};
use crate::provider_process::TracedProviderBoundary;

pub fn describe(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request: DescribeRequest,
) -> Result<DescribeResult, InvocationError<AdapterError>> {
    let wire = invoke::<_, serde_json::Value>(
        boundary,
        config,
        &request.request_id,
        ProviderRole::Describe,
        EmptyPayload::default(),
        false,
    )?;
    if wire.result.get("kind").and_then(serde_json::Value::as_str) != Some("description") {
        return Err(mapping_failure(
            config,
            &request.request_id,
            ProviderRole::Describe,
            &wire,
            "describe result kind must be description".to_owned(),
        ));
    }
    let graph = if let Some(error) = &wire.graph_declaration_error {
        DescribedGraph::Invalid(GraphError::InvalidDeclaration(error.clone()))
    } else {
        match serde_json::from_value::<DescribeResultDto>(wire.result.clone()) {
            Ok(DescribeResultDto::Description { graph }) => match map_graph_unvalidated(graph) {
                Ok(graph) => DescribedGraph::Declared(graph),
                Err(error) => {
                    DescribedGraph::Invalid(GraphError::InvalidDeclaration(error.to_string()))
                }
            },
            Err(error) => {
                DescribedGraph::Invalid(GraphError::InvalidDeclaration(error.to_string()))
            }
        }
    };
    let observation = ProviderObservation::new(
        config.registration_id().clone(),
        config.config().executable(),
        wire.digest.clone(),
        wire.provider_version.clone(),
        observed_now(),
    )
    .expect("resolved provider observation satisfies core bounds");
    let trace_failure = wire.complete_trace().err();
    let fact = result_fact(
        config,
        &request.request_id,
        ProviderRole::Describe,
        wire.digest,
        wire.provider_version,
        wire.protocol_major,
        OutcomeClass::Completed,
    );
    Ok(DescribeResult {
        graph,
        observation,
        fact,
        protocol_major: wire.protocol_major,
        trace_failure,
    })
}

pub(crate) fn observed_now() -> ObservedAt {
    ObservedAt::parse(&Timestamp::now().to_string())
        .expect("system timestamp must satisfy core timestamp syntax")
}
