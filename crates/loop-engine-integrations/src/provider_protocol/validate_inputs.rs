use std::collections::BTreeMap;

use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    InputValidationInvocationResult, InputValidationResult, InvocationError, ValidateInputsRequest,
};
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::run_input::InputDeclaration;

use super::canonical::value_from_core;
use super::dto::{
    InputDeclarationDto, ProviderRole, ValidateInputsPayloadDto, ValidateInputsResultDto,
};
use super::invoke::{AdapterError, invoke, mapping_failure, result_fact};
use super::mapping::diagnostics;
use crate::provider_process::TracedProviderBoundary;

pub fn validate_inputs(
    boundary: &TracedProviderBoundary,
    config: &ResolvedProviderConfig,
    request: ValidateInputsRequest,
) -> Result<InputValidationInvocationResult, InvocationError<AdapterError>> {
    let payload = ValidateInputsPayloadDto {
        declarations: request
            .input_declarations
            .values()
            .map(input_declaration)
            .collect(),
        candidate_values: request
            .inputs
            .values()
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value_from_core(value)))
            .collect::<BTreeMap<_, _>>(),
    };
    let wire = invoke::<_, ValidateInputsResultDto>(
        boundary,
        config,
        &request.request_id,
        ProviderRole::ValidateInputs,
        payload,
        true,
    )?;
    let result = match wire.result.clone() {
        ValidateInputsResultDto::Accepted { .. } => InputValidationResult::Accepted,
        ValidateInputsResultDto::Rejected {
            diagnostics: values,
        } => InputValidationResult::Rejected(diagnostics(values, "/result/diagnostics").map_err(
            |error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::ValidateInputs,
                    &wire,
                    error.to_string(),
                )
            },
        )?),
        ValidateInputsResultDto::EvaluationError {
            diagnostics: values,
        } => InputValidationResult::EvaluationError(
            diagnostics(values, "/result/diagnostics").map_err(|error| {
                mapping_failure(
                    config,
                    &request.request_id,
                    ProviderRole::ValidateInputs,
                    &wire,
                    error.to_string(),
                )
            })?,
        ),
    };
    let outcome = match &result {
        InputValidationResult::Accepted => OutcomeClass::Completed,
        InputValidationResult::Rejected(_) => OutcomeClass::Rejected,
        InputValidationResult::EvaluationError(_) => OutcomeClass::Error,
    };
    let trace_failure = wire.complete_trace().err();
    Ok(InputValidationInvocationResult {
        result,
        fact: result_fact(
            config,
            &request.request_id,
            ProviderRole::ValidateInputs,
            wire.digest,
            wire.provider_version,
            wire.protocol_major,
            outcome,
        ),
        trace_failure,
    })
}

pub(crate) fn input_declaration(value: &InputDeclaration) -> InputDeclarationDto {
    InputDeclarationDto {
        id: value.name().as_str().to_owned(),
        kind: value.kind().as_str().to_owned(),
        required: value.required(),
        metadata: value.metadata().map(super::canonical::metadata_value),
    }
}
