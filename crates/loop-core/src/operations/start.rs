//! `start` run operation.

use super::{persistence_error, provider_error, provider_resolution_error, workflow_error};
use crate::{
    CreateRunRequest, CreateRunResult, Lifecycle, OperationOutcome, ProviderGateway,
    ProviderResolver, ProviderSelector, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Caller-supplied values needed to create a run.
///
/// The run ID and timestamp are deliberately supplied by the caller or
/// composition root.  Core does not invent identifier or clock ports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub id: crate::RunId,
    pub provider: ProviderSelector,
    pub label: Option<String>,
    pub initial_input: Value,
    pub created_at: Timestamp,
}

impl Request {
    pub fn new(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: Option<String>,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            label,
            initial_input,
            created_at,
        }
    }

    pub fn with_label(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: impl Into<String>,
        created_at: Timestamp,
    ) -> Self {
        Self::new(id, provider, initial_input, Some(label.into()), created_at)
    }
}

/// Successful `start` data returned by the persistence boundary.
pub type Result = CreateRunResult;

/// Execute `start` through the provider and persistence ports.
///
/// Provider resolution and description happen before validation and before
/// persistence is called.  The persistence adapter owns the atomic run plus
/// creation-history write.
pub fn execute<R, G, P>(
    request: Request,
    resolver: &R,
    gateway: &G,
    persistence: &P,
) -> OperationOutcome<Result>
where
    R: ProviderResolver + ?Sized,
    G: ProviderGateway + ?Sized,
    P: crate::Persistence + ?Sized,
{
    let association = match resolver.resolve(&request.provider) {
        Ok(association) => association,
        Err(error) => return provider_resolution_error(error),
    };

    let workflow = match gateway.describe(&association) {
        Ok(workflow) => workflow,
        Err(error) => return provider_error(error),
    };

    if let Err(error) = workflow.validate() {
        return workflow_error(error);
    }

    let initial_state = workflow.initial_state.clone();
    let Some(initial_state_definition) = workflow
        .states
        .iter()
        .find(|state| state.id == initial_state)
    else {
        // `validate` guarantees this cannot happen for a well-formed
        // workflow.  Keep the operation total if a future validation change
        // ever weakens that guarantee.
        return OperationOutcome::error(
            "invalid-workflow",
            "validated workflow has no definition for its initial state",
        );
    };
    let lifecycle = if initial_state_definition.is_final {
        Lifecycle::Final
    } else {
        Lifecycle::Active
    };

    let create = CreateRunRequest::new(
        request.id,
        request.label,
        workflow,
        association,
        request.initial_input,
        initial_state,
        lifecycle,
        request.created_at,
    );

    match persistence.create_run(create) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

/// Execute `start` with ports first, which is convenient for composition
/// roots that keep their adapters together.
pub fn execute_with_ports<R, G, P>(
    resolver: &R,
    gateway: &G,
    persistence: &P,
    request: Request,
) -> OperationOutcome<Result>
where
    R: ProviderResolver + ?Sized,
    G: ProviderGateway + ?Sized,
    P: crate::Persistence + ?Sized,
{
    execute(request, resolver, gateway, persistence)
}
