use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::operations::provider_add::ProviderAddExecutionError;
use std::ffi::OsStr;
use std::path::Path;

use loop_engine_integrations::configuration::{ConfigurationError, normalize_registration_path};
use loop_engine_integrations::persistence::CatalogPersistenceError;
use serde_json::{Value, json};
use thiserror::Error;

use crate::args::PlannedCommand;
use crate::commands::provider::{
    ProviderAddRequest, ProviderCatalogMutationOutcome, ProviderListError, ProviderListRequest,
    ProviderMapError, add, list_active_run_impact, list_registrations, map_add_request,
    map_list_request,
};
use crate::composition::Application;
use crate::dispatch::{
    DispatchDelivery, DispatchError, TracedDispatchInput, TracedOperationResult,
    dispatch_traced_operation_with_data,
};

#[derive(Debug, Error)]
pub enum PrepareApplicationError {
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    #[error(transparent)]
    ProviderMap(#[from] ProviderMapError),
}

pub enum PreparedApplicationCommand {
    ProviderAdd {
        request: Value,
        command: ProviderAddRequest,
    },
    ProviderList {
        request: Value,
        command: ProviderListRequest,
    },
}

pub fn prepare_application_command(
    command: PlannedCommand,
    caller_cwd: &Path,
    home: Option<&OsStr>,
) -> Result<PreparedApplicationCommand, PrepareApplicationError> {
    match command {
        PlannedCommand::ProviderAdd(parsed) => {
            let executable = normalize_registration_path(parsed.exec.as_str(), caller_cwd, home)?;
            let working_directory =
                normalize_registration_path(parsed.working_directory.as_str(), caller_cwd, home)?;
            let request = json!({
                "handle": parsed.handle.as_str(),
                "executable": executable,
                "argv": parsed.arg.elements.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
                "working_directory": working_directory,
                "timeout_seconds": parsed.timeout.as_ref().map(|value| value.get()),
            });
            let command = map_add_request(
                &parsed.handle,
                &executable,
                &working_directory,
                &parsed.arg,
                parsed.timeout.as_ref(),
            )?;
            Ok(PreparedApplicationCommand::ProviderAdd { request, command })
        }
        PlannedCommand::ProviderList(parsed) => {
            let request = json!({
                "enabled": parsed.enabled,
                "tombstoned": parsed.tombstoned,
                "active_runs_for": parsed.active_runs_for.as_ref().map(|value| value.as_str()),
                "cursor": parsed.cursor.as_ref().map(|value| value.as_str()),
                "limit": parsed.limit.get(),
            });
            let command = map_list_request(
                parsed.enabled,
                parsed.tombstoned,
                parsed.active_runs_for.as_ref(),
                parsed.cursor.as_ref(),
                parsed.limit,
            )?;
            Ok(PreparedApplicationCommand::ProviderList { request, command })
        }
        _ => unreachable!("startup prepares only exposed application routes"),
    }
}

pub fn execute_application_command(
    application: &Application,
    prepared: PreparedApplicationCommand,
) -> Result<DispatchDelivery, DispatchError> {
    match prepared {
        PreparedApplicationCommand::ProviderAdd { request, command } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("provider.add"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_add(application, command)),
            )
        }
        PreparedApplicationCommand::ProviderList { request, command } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("provider.list"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_list(application, command)),
            )
        }
    }
}

fn operation_id(value: &'static str) -> loop_engine_core::operations::catalog::OperationId {
    loop_engine_core::operations::catalog::OperationId::parse(value)
        .expect("exposed operation ID belongs to frozen catalog")
}

fn execute_add(
    application: &Application,
    request: crate::commands::provider::ProviderAddRequest,
) -> (TracedOperationResult, Value) {
    match add(&application.ids, &application.catalog, request) {
        Ok(outcome) => delivered(completed(), mutation_value(&outcome), true),
        Err(ProviderAddExecutionError::Id(error)) => delivered(
            failed(ReasonCode::PersistenceFailed, error.to_string()),
            json!({}),
            false,
        ),
        Err(ProviderAddExecutionError::Catalog(error)) => delivered(
            failed(catalog_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn execute_list(
    application: &Application,
    request: ProviderListRequest,
) -> (TracedOperationResult, Value) {
    let result = match request {
        ProviderListRequest::Registrations {
            filter,
            limit,
            cursor,
        } => list_registrations(&application.catalog, filter, limit, cursor).map(|outcome| {
            let items = outcome
                .items
                .iter()
                .map(registration_row_value)
                .collect::<Vec<_>>();
            page_value(items, outcome.next_cursor)
        }),
        ProviderListRequest::ActiveRunImpact {
            registration_id,
            limit,
            cursor,
        } => list_active_run_impact(&application.catalog, registration_id, limit, cursor).map(
            |outcome| {
                let items = outcome
                    .items
                    .iter()
                    .map(|item| {
                        json!({
                            "run_id": item.run_id,
                            "graph_revision": item.graph_revision,
                        })
                    })
                    .collect::<Vec<_>>();
                page_value(items, outcome.next_cursor)
            },
        ),
    };
    match result {
        Ok(data) => delivered(completed(), data, false),
        Err(error) => delivered(
            failed(provider_list_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn delivered(
    outcome: PublicOutcome,
    data: Value,
    after_commit: bool,
) -> (TracedOperationResult, Value) {
    (
        TracedOperationResult {
            outcome,
            after_commit,
        },
        data,
    )
}

fn page_value(items: Vec<Value>, next_cursor: Option<String>) -> Value {
    let mut data = serde_json::Map::new();
    data.insert("items".into(), Value::Array(items));
    if let Some(cursor) = next_cursor {
        data.insert("next_cursor".into(), Value::String(cursor));
    }
    Value::Object(data)
}

fn mutation_value(outcome: &ProviderCatalogMutationOutcome) -> Value {
    let mut data = serde_json::Map::new();
    data.insert(
        "registration".into(),
        registration_value(&outcome.registration),
    );
    data.insert(
        "affected_active_runs".into(),
        json!(outcome.affected_active_runs),
    );
    if let Some(cursor) = &outcome.impact_cursor {
        data.insert("impact_cursor".into(), Value::String(cursor.clone()));
    }
    Value::Object(data)
}

fn registration_row_value(row: &crate::commands::provider::ProviderCatalogRowView) -> Value {
    json!({
        "registration": registration_value(&row.registration),
        "config": row.config.as_ref().map(|config| json!({
            "executable": config.executable,
            "argv": config.argv,
            "working_directory": config.working_directory,
            "timeout_seconds": config.timeout_seconds,
        })),
    })
}

fn registration_value(registration: &crate::commands::provider::ProviderRegistrationView) -> Value {
    json!({
        "id": registration.id,
        "handle": registration.handle,
        "enabled": registration.enabled,
        "config_revision": registration.config_revision,
    })
}

fn completed() -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(None, None, None).expect("empty operation data is valid"),
        vec![],
    )
    .expect("completed outcome is valid")
}

fn failed(code: ReasonCode, message: String) -> PublicOutcome {
    let reason = Reason::new(code, message).expect("bounded persistence diagnostics fit");
    PublicOutcome::new(
        code.outcome_class(),
        Some(reason),
        OutcomeData::new(None, None, None).expect("empty operation data is valid"),
        vec![],
    )
    .expect("reason and class agree")
}

fn provider_list_reason(error: &ProviderListError<CatalogPersistenceError>) -> ReasonCode {
    match error {
        ProviderListError::Paging(_) => ReasonCode::CursorInvalid,
        ProviderListError::Catalog(error) => catalog_reason(error),
    }
}

fn catalog_reason(error: &CatalogPersistenceError) -> ReasonCode {
    match error {
        CatalogPersistenceError::NotFound => ReasonCode::CatalogRegistrationNotFound,
        CatalogPersistenceError::Disabled => ReasonCode::ProviderTombstoned,
        CatalogPersistenceError::Duplicate => ReasonCode::CatalogHandleDuplicate,
        CatalogPersistenceError::Occupied => ReasonCode::CatalogHandleOccupied,
        CatalogPersistenceError::Stale => ReasonCode::ProviderRegistrationStale,
        CatalogPersistenceError::InvalidCursor => ReasonCode::CursorInvalid,
        CatalogPersistenceError::InvalidAck => ReasonCode::CatalogAckTokenInvalid,
        CatalogPersistenceError::Constraint
        | CatalogPersistenceError::Persistence(_)
        | CatalogPersistenceError::Mapping(_)
        | CatalogPersistenceError::CommitOutcomeUnverified
        | CatalogPersistenceError::CommitIntegrityFailure => ReasonCode::PersistenceFailed,
    }
}
