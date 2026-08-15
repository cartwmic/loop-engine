//! `start` run operation.

use super::{persistence_error, provider_error, provider_resolution_error, workflow_error};
use crate::{
    CreateRunRequest, CreateRunResult, Lifecycle, OperationOutcome, ProviderGateway,
    ProviderResolver, ProviderSelector, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Caller-supplied values needed to create a run.
///
/// The run ID and timestamp are deliberately supplied by the caller or
/// composition root.  Core does not invent identifier or clock ports.
/// `catalog_root` is the parent of the resolved catalog database; start
/// allocates `<catalog_root>/runs/<run_id>/` before `create_run`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub id: crate::RunId,
    pub provider: ProviderSelector,
    pub label: Option<String>,
    pub initial_input: Value,
    pub created_at: Timestamp,
    pub catalog_root: PathBuf,
}

impl Request {
    pub fn new(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: Option<String>,
        created_at: Timestamp,
        catalog_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            label,
            initial_input,
            created_at,
            catalog_root: catalog_root.into(),
        }
    }

    pub fn with_label(
        id: impl Into<crate::RunId>,
        provider: impl Into<ProviderSelector>,
        initial_input: Value,
        label: impl Into<String>,
        created_at: Timestamp,
        catalog_root: impl Into<PathBuf>,
    ) -> Self {
        Self::new(
            id,
            provider,
            initial_input,
            Some(label.into()),
            created_at,
            catalog_root,
        )
    }
}

/// Successful `start` data returned by the persistence boundary.
pub type Result = CreateRunResult;

/// Execute `start` through the provider and persistence ports.
///
/// Provider resolution and description happen before validation and before
/// persistence is called.  After a valid workflow, start allocates the
/// engine-owned per-run directory and composes reserved `artifact_root` into
/// stored object input.  The persistence adapter owns the atomic run plus
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

    let allocated = match allocate_run_directory(&request.catalog_root, &request.id) {
        Ok(path) => path,
        Err((code, message)) => return OperationOutcome::error(code, message),
    };
    let (composed_input, recorded_artifact_root) =
        compose_stored_input(request.initial_input, &allocated);

    let create = CreateRunRequest::new(
        request.id,
        request.label,
        workflow,
        association,
        composed_input,
        initial_state,
        lifecycle,
        request.created_at,
        request.provider.as_str().to_owned(),
        Some(recorded_artifact_root),
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

fn allocate_run_directory(
    catalog_root: &Path,
    run_id: &crate::RunId,
) -> std::result::Result<PathBuf, (String, String)> {
    let catalog_root = if catalog_root.is_relative() {
        let current_dir = std::env::current_dir().map_err(|error| {
            (
                "run-directory-failed".to_owned(),
                format!("could not resolve current directory for catalog root: {error}"),
            )
        })?;
        current_dir.join(catalog_root)
    } else {
        catalog_root.to_path_buf()
    };
    let allocated = catalog_root.join("runs").join(run_id.as_str());
    std::fs::create_dir_all(&allocated).map_err(|error| {
        (
            "run-directory-failed".to_owned(),
            format!(
                "could not create run directory `{}`: {error}",
                allocated.display()
            ),
        )
    })?;
    std::fs::canonicalize(&allocated).map_err(|error| {
        (
            "run-directory-failed".to_owned(),
            format!(
                "could not canonicalize run directory `{}`: {error}",
                allocated.display()
            ),
        )
    })
}

/// Compose stored `initial_input` and the catalog `artifact_root` string.
///
/// A JSON object that already has a non-empty string `artifact_root` keeps
/// that string (including a relative path).  Other objects receive the
/// allocated canonical path.  Non-objects are left unchanged; the catalog
/// column still records the allocated path.
fn compose_stored_input(initial_input: Value, allocated_canonical: &Path) -> (Value, String) {
    let allocated = allocated_canonical.to_string_lossy().into_owned();
    match initial_input {
        Value::Object(mut map) => {
            let caller_path = match map.get("artifact_root") {
                Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                _ => None,
            };
            if let Some(recorded) = caller_path {
                (Value::Object(map), recorded)
            } else {
                map.insert("artifact_root".to_owned(), Value::String(allocated.clone()));
                (Value::Object(map), allocated)
            }
        }
        other => (other, allocated),
    }
}
