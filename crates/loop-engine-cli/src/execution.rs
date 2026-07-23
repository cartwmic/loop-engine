use loop_engine_core::capabilities::id_generator::IdGenerator;
use loop_engine_core::capabilities::provider_invoker::{
    CompatibilityRequest, GateRequest, InvocationError,
};
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::attempt::{AttemptFacts, JournalExtension};
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::journal::JournalDraft;
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot};
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::operations::provider_add::ProviderAddExecutionError;
use loop_engine_core::operations::provider_check::{
    GraphConformance, ProviderCheckContinuation, ProviderCheckExecution,
    ProviderCheckExecutionError, TraceBudgetDisposition,
};
use loop_engine_core::operations::run_create::{RunCreateError, RunCreateExecutionError};
use loop_engine_core::operations::run_request::{self, RequestExecutionError};
use std::convert::Infallible;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use loop_engine_integrations::configuration::{ConfigurationError, normalize_registration_path};
use loop_engine_integrations::evidence_inputs::load_optional as load_inline_evidence;
use loop_engine_integrations::persistence::{
    CatalogPersistenceError, HistoryReadError, RunMutationError, RunReadError, RunRequestReadError,
    journal_entry_value as render_journal_entry,
};
use loop_engine_integrations::provider_protocol::AdapterError;
use loop_engine_integrations::provider_protocol::canonical::value_from_core;
use loop_engine_integrations::provider_protocol::{bounded_evidence_context, bounded_run_snapshot};
use loop_engine_integrations::run_inputs::{RunInputLoadError, load_optional as load_run_inputs};
use serde_json::{Value, json};
use thiserror::Error;

use crate::args::PlannedCommand;
use crate::commands::provider::{
    ProviderAddRequest, ProviderCatalogMutationOutcome, ProviderListError, ProviderListRequest,
    ProviderMapError, ProviderTargetRef, add, check, list_active_run_impact, list_registrations,
    map_add_request, map_check_request, map_list_request, map_target, resolve_target,
};
use crate::commands::run::{
    RunCreateDelivery, RunCreateRequest, RunHistoryError, RunHistoryRequest, RunListError,
    RunListRequest, RunMapError, RunRequestDelivery, RunTerminateDelivery, build_terminate_command,
    create, history, list, map_create_delivery, map_history_request,
    map_list_request as map_run_list_request, map_request_delivery, map_show_run_id,
    map_terminate_delivery, show, terminate,
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
    #[error(transparent)]
    RunMap(#[from] RunMapError),
    #[error(transparent)]
    RunInputs(#[from] RunInputLoadError),
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
    ProviderCheck {
        request: Value,
        target: ProviderTargetRef,
        active_runs: bool,
        cursor: Option<crate::args::SyntaxOpaqueWire>,
        limit: crate::args::SyntaxPageLimit,
    },
    RunCreate {
        request: Value,
        delivery: RunCreateDelivery,
        inputs: loop_engine_core::model::run_input::RunInputs,
    },
    RunList {
        request: Value,
        command: RunListRequest,
    },
    RunShow {
        request: Value,
        run_id: loop_engine_core::model::ids::RunId,
    },
    RunHistory {
        request: Value,
        command: RunHistoryRequest,
    },
    RunTerminate {
        request: Value,
        delivery: RunTerminateDelivery,
    },
    RunRequest {
        request: Value,
        delivery: RunRequestDelivery,
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
        PlannedCommand::ProviderCheck(parsed) => {
            let request = json!({
                "target": parsed.target.as_str(),
                "active_runs": parsed.active_runs,
                "cursor": parsed.cursor.as_ref().map(|value| value.as_str()),
                "limit": parsed.limit.get(),
            });
            Ok(PreparedApplicationCommand::ProviderCheck {
                request,
                target: map_target(&parsed.target)?,
                active_runs: parsed.active_runs,
                cursor: parsed.cursor,
                limit: parsed.limit,
            })
        }
        PlannedCommand::RunCreate(parsed) => {
            let delivery = map_create_delivery(&parsed)?;
            let input_path = delivery.inputs_path.as_ref().map(PathBuf::from);
            let inputs = load_run_inputs(input_path.as_deref())?;
            let request = json!({
                "target": parsed.target.as_str(),
                "label": parsed.label.as_ref().map(|value| value.as_str()),
                "inputs": parsed.inputs.as_ref().map(|value| value.as_str()),
            });
            Ok(PreparedApplicationCommand::RunCreate {
                request,
                delivery,
                inputs,
            })
        }
        PlannedCommand::RunList(parsed) => {
            let request = json!({
                "terminal": parsed.terminal,
                "all": parsed.all,
                "cursor": parsed.cursor.as_ref().map(|value| value.as_str()),
                "limit": parsed.limit.get(),
            });
            Ok(PreparedApplicationCommand::RunList {
                request,
                command: map_run_list_request(&parsed)?,
            })
        }
        PlannedCommand::RunShow(parsed) => {
            let request = json!({"run_id": parsed.run_id.as_str()});
            Ok(PreparedApplicationCommand::RunShow {
                request,
                run_id: map_show_run_id(&parsed)?,
            })
        }
        PlannedCommand::RunHistory(parsed) => {
            let request = json!({
                "run_id": parsed.run_id.as_str(),
                "cursor": parsed.cursor.as_ref().map(|value| value.as_str()),
                "limit": parsed.limit.get(),
            });
            Ok(PreparedApplicationCommand::RunHistory {
                request,
                command: map_history_request(&parsed)?,
            })
        }
        PlannedCommand::RunTerminate(parsed) => {
            let request = json!({
                "run_id": parsed.run_id.as_str(),
                "note": parsed.note.as_ref().map(|value| value.as_str()),
            });
            Ok(PreparedApplicationCommand::RunTerminate {
                request,
                delivery: map_terminate_delivery(&parsed)?,
            })
        }
        PlannedCommand::RunRequest(parsed) => {
            let request = json!({
                "run_id": parsed.run_id.as_str(),
                "event": parsed.event.as_str(),
                "evidence_ids": parsed.evidence_id.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
                "evidence": parsed.evidence.as_ref().map(|value| value.as_str()),
                "note": parsed.note.as_ref().map(|value| value.as_str()),
            });
            Ok(PreparedApplicationCommand::RunRequest {
                request,
                delivery: map_request_delivery(&parsed)?,
            })
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
        PreparedApplicationCommand::ProviderCheck {
            request,
            target,
            active_runs,
            cursor,
            limit,
        } => dispatch_traced_operation_with_data(
            &application.trace,
            TracedDispatchInput {
                operation: operation_id("provider.check"),
                request,
                operation_data: json!({}),
            },
            || {
                Ok(execute_check(
                    application,
                    target,
                    active_runs,
                    cursor.as_ref(),
                    limit,
                ))
            },
        ),
        PreparedApplicationCommand::RunCreate {
            request,
            delivery,
            inputs,
        } => dispatch_traced_operation_with_data(
            &application.trace,
            TracedDispatchInput {
                operation: operation_id("run.create"),
                request,
                operation_data: json!({}),
            },
            || Ok(execute_create(application, delivery, inputs)),
        ),
        PreparedApplicationCommand::RunList { request, command } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("run.list"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_run_list(application, command)),
            )
        }
        PreparedApplicationCommand::RunShow { request, run_id } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("run.show"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_show(application, &run_id)),
            )
        }
        PreparedApplicationCommand::RunHistory { request, command } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("run.history"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_history(application, command)),
            )
        }
        PreparedApplicationCommand::RunTerminate { request, delivery } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("run.terminate"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_terminate(application, delivery)),
            )
        }
        PreparedApplicationCommand::RunRequest { request, delivery } => {
            dispatch_traced_operation_with_data(
                &application.trace,
                TracedDispatchInput {
                    operation: operation_id("run.request"),
                    request,
                    operation_data: json!({}),
                },
                || Ok(execute_request(application, delivery)),
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

fn execute_check(
    application: &Application,
    target: ProviderTargetRef,
    active_runs: bool,
    cursor: Option<&crate::args::SyntaxOpaqueWire>,
    limit: crate::args::SyntaxPageLimit,
) -> (TracedOperationResult, Value) {
    let registration_id = match resolve_target(&application.catalog, "provider.check", &target) {
        Ok(row) => row.registration.id().clone(),
        Err(error) => {
            return delivered(
                failed(catalog_reason(&error), error.to_string()),
                json!({}),
                false,
            );
        }
    };
    let request = match map_check_request(
        registration_id.clone(),
        application
            .ids
            .request_id()
            .expect("UUID allocation is infallible"),
        active_runs,
        cursor,
        limit,
    ) {
        Ok(request) => request,
        Err(error) => {
            return delivered(
                failed(ReasonCode::CursorInvalid, error.to_string()),
                json!({}),
                false,
            );
        }
    };
    let result = check(
        &application.catalog,
        &application.run_reads,
        &application.invoker,
        &application.digests,
        request,
        |_, run| {
            let snapshot = bounded_run_snapshot(run).map_err(|error| error.to_string())?;
            Ok::<CompatibilityRequest, String>(CompatibilityRequest {
                request_id: application
                    .ids
                    .request_id()
                    .expect("UUID allocation is infallible"),
                run_id: run.id().clone(),
                run: snapshot,
            })
        },
        |impact, result| {
            Ok::<usize, Infallible>(checked_run_value(impact, &result.report).to_string().len())
        },
    );
    match result {
        Ok(ProviderCheckExecution::Completed(page)) => {
            provider_check_delivery(application, &registration_id, *page, false)
        }
        Ok(ProviderCheckExecution::TraceBudget(TraceBudgetDisposition::AfterProgress(page))) => {
            provider_check_delivery(application, &registration_id, *page, true)
        }
        Ok(ProviderCheckExecution::TraceBudget(TraceBudgetDisposition::BeforeFirstRow {
            ..
        })) => delivered(
            failed(
                ReasonCode::ResourceExhausted,
                "provider trace budget unavailable before first row".into(),
            ),
            json!({}),
            false,
        ),
        Err(error) => provider_check_failure(error),
    }
}

fn provider_check_delivery(
    application: &Application,
    registration_id: &loop_engine_core::model::ids::RegistrationId,
    page: loop_engine_core::operations::provider_check::ProviderCheckPage,
    trace_exhausted: bool,
) -> (TracedOperationResult, Value) {
    let next_cursor = match page.continuation {
        ProviderCheckContinuation::SourceCursor(cursor) => {
            cursor.map(|value| value.as_str().to_owned())
        }
        ProviderCheckContinuation::AfterRun(run_id) => match application
            .catalog
            .provider_check_cursor_after(registration_id, &run_id)
        {
            Ok(cursor) => Some(cursor.as_str().to_owned()),
            Err(error) => {
                return delivered(
                    failed(catalog_reason(&error), error.to_string()),
                    json!({}),
                    false,
                );
            }
        },
    };
    let (graph_status, graph_revision) = match &page.summary.graph_conformance {
        GraphConformance::Valid { revision } => ("valid", Some(revision.as_str())),
        GraphConformance::Invalid { .. } => ("invalid", None),
    };
    let items = page
        .rows
        .iter()
        .map(|row| checked_run_value(&row.impact, &row.report))
        .collect::<Vec<_>>();
    let mut data = page_value(items, next_cursor);
    let object = data.as_object_mut().expect("page is object");
    object.insert(
        "conformance".into(),
        json!({
            "config_revision": page.summary.config_revision,
            "protocol_major": page.summary.protocol_major,
            "graph_status": graph_status,
            "graph_revision": graph_revision,
        }),
    );
    object.insert("provider_calls".into(), json!(page.provider_calls));
    if trace_exhausted {
        object.insert("trace_budget_exhausted".into(), json!(true));
    }
    delivered(completed(), data, false)
}

fn execute_run_list(
    application: &Application,
    request: RunListRequest,
) -> (TracedOperationResult, Value) {
    match list(&application.run_reads, request) {
        Ok(outcome) => {
            let items = outcome
                .items
                .iter()
                .map(|row| {
                    json!({
                        "run_id": row.run_id,
                        "label": row.label,
                        "lifecycle": lifecycle_value(row.lifecycle),
                        "state": row.current_state,
                    })
                })
                .collect();
            delivered(completed(), page_value(items, outcome.next_cursor), false)
        }
        Err(error) => delivered(
            failed(run_list_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn execute_show(
    application: &Application,
    run_id: &loop_engine_core::model::ids::RunId,
) -> (TracedOperationResult, Value) {
    match show(&application.run_reads, run_id) {
        Ok(projection) => {
            let snapshot = RunSnapshot {
                run_id: projection.run_id.clone(),
                label: projection.label.clone(),
                lifecycle: projection.lifecycle,
                current_state: projection.current_state.clone(),
                state_changed: false,
            };
            let inputs = projection
                .inputs
                .values()
                .iter()
                .map(|(name, value)| (name.as_str().to_owned(), value_from_core(value)))
                .collect::<serde_json::Map<_, _>>();
            let static_guidance = match &projection.static_guidance {
                StaticGuidance::Text(text) => json!({
                    "kind": "text",
                    "text": text.as_str(),
                }),
                StaticGuidance::NoneRequired => json!({"kind": "none_required"}),
            };
            let live_guidance = match projection.live_guidance {
                LiveGuidanceCapability::Supported => "supported",
                LiveGuidanceCapability::Unsupported => "unsupported",
            };
            let event_details = projection
                .requestable_events
                .iter()
                .map(|event| {
                    json!({
                        "event": event.event.as_str(),
                        "target": event.target.as_str(),
                        "required_gates": event.required_gates.iter().map(|gate| gate.as_str()).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>();
            let selected_evidence = projection
                .selected_evidence
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>();
            delivered(
                completed_with_run_snapshot(snapshot, projection.requestable_events),
                json!({
                    "graph_revision": projection.graph_revision.as_str(),
                    "inputs": inputs,
                    "static_guidance": static_guidance,
                    "live_guidance": live_guidance,
                    "selected_evidence": selected_evidence,
                    "requestable_event_details": event_details,
                }),
                false,
            )
        }
        Err(error) => delivered(
            failed(run_read_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn execute_history(
    application: &Application,
    request: RunHistoryRequest,
) -> (TracedOperationResult, Value) {
    match history(&application.history, request) {
        Ok(outcome) => {
            let items = match outcome
                .items
                .iter()
                .map(render_journal_entry)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(items) => items,
                Err(error) => {
                    return delivered(
                        failed(ReasonCode::PersistenceFailed, error.to_string()),
                        json!({}),
                        false,
                    );
                }
            };
            delivered(completed(), page_value(items, outcome.next_cursor), false)
        }
        Err(error) => delivered(
            failed(run_history_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn execute_create(
    application: &Application,
    delivery: RunCreateDelivery,
    inputs: loop_engine_core::model::run_input::RunInputs,
) -> (TracedOperationResult, Value) {
    let registration = match resolve_target(&application.catalog, "run.create", &delivery.target) {
        Ok(row) => row.registration.id().clone(),
        Err(error) => {
            return delivered(
                failed(catalog_reason(&error), error.to_string()),
                json!({}),
                false,
            );
        }
    };
    let request = RunCreateRequest {
        registration_id: registration,
        label: delivery.label,
        inputs,
    };
    let result = create(
        &application.catalog,
        &application.invoker,
        &application.digests,
        &application.ids,
        &application.run_create,
        &request,
        |_, described, validated, run| {
            let mut observations = vec![described.fact.clone()];
            if let Some(validated) = validated {
                observations.push(validated.fact.clone());
            }
            JournalDraft::new(
                run.id().clone(),
                application.clock.now().expect("system clock is infallible"),
                "run.create",
                application
                    .ids
                    .request_id()
                    .expect("UUID allocation is infallible"),
                OutcomeClass::Completed,
                None,
                Some(AttemptFacts {
                    provider_observations: observations,
                    ..AttemptFacts::default()
                }),
                JournalExtension::RunCreated {
                    graph_revision: run.graph_revision().clone(),
                },
            )
        },
    );
    match result {
        Ok(execution) => delivered(
            completed_with_run_snapshot(
                run_snapshot(&execution.run, false),
                loop_engine_core::model::requestable::project(&execution.run),
            ),
            json!({}),
            execution.commit.committed,
        ),
        Err(error) => run_create_failure(error),
    }
}

fn execute_request(
    application: &Application,
    delivery: RunRequestDelivery,
) -> (TracedOperationResult, Value) {
    let observed_at = application.clock.now().expect("system clock is infallible");
    let inline_path = delivery.inline_evidence_path.as_deref().map(Path::new);
    let inline_evidence = load_inline_evidence(inline_path, observed_at);
    let event = delivery.event.clone();
    let command_request_id =
        loop_engine_core::model::ids::RequestId::parse(application.trace.request_id())
            .expect("startup trace request ID is a bounded identifier");
    let result = run_request::execute_with_inline_result(
        &application.run_request_reads,
        &application.catalog,
        &application.invoker,
        &application.event_attempts,
        &delivery.run_id,
        &delivery.event,
        &delivery.selected_evidence_ids,
        inline_evidence.as_deref(),
        delivery.note.as_ref(),
        |_, run, selected| {
            let inline = inline_evidence.as_deref().unwrap_or(&[]);
            Ok::<GateRequest, loop_engine_core::model::bounded::BoundError>(GateRequest {
                request_id: application
                    .ids
                    .request_id()
                    .expect("UUID allocation is infallible"),
                run: bounded_run_snapshot(run)?,
                event: event.clone(),
                selected_evidence: bounded_evidence_context("selected_evidence", selected)?,
                inline_evidence: bounded_evidence_context("inline_evidence", inline)?,
            })
        },
        |run, selected, note, resolution| {
            run_request::command(
                run,
                &delivery.event,
                selected,
                inline_evidence.as_deref().unwrap_or(&[]),
                note,
                resolution,
                observed_at,
                command_request_id.clone(),
            )
        },
    );
    match result {
        Ok(execution) => {
            let snapshot = RunSnapshot {
                run_id: delivery.run_id,
                label: execution.run.label,
                lifecycle: execution.run.lifecycle,
                current_state: execution.run.current_state,
                state_changed: execution.commit.state_changed,
            };
            let data = OutcomeData::new(
                Some(snapshot),
                Some(execution.requestable_events),
                Some(execution.evidence_recorded),
            )
            .expect("request execution has valid run outcome shape");
            let outcome = PublicOutcome::new(
                execution.outcome,
                execution.reason,
                data,
                execution.diagnostics,
            )
            .expect("committed request draft is a valid public outcome");
            delivered(outcome, json!({}), execution.commit.committed)
        }
        Err(RequestExecutionError::Lookup(error)) => delivered(
            failed(run_request_read_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
        Err(
            RequestExecutionError::Command(error) | RequestExecutionError::InvalidCommand(error),
        ) => delivered(
            failed(ReasonCode::PersistenceFailed, error.to_string()),
            json!({}),
            false,
        ),
        Err(RequestExecutionError::Writer(error)) => delivered(
            failed(ReasonCode::PersistenceFailed, error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn execute_terminate(
    application: &Application,
    delivery: RunTerminateDelivery,
) -> (TracedOperationResult, Value) {
    let run = match application
        .run_reads
        .get_for_operation("run.terminate", &delivery.run_id)
    {
        Ok(run) => run,
        Err(error) => {
            return delivered(
                failed(run_read_reason(&error), error.to_string()),
                json!({}),
                false,
            );
        }
    };
    let attempt = delivery.note.as_ref().map(|note| AttemptFacts {
        note: Some(note.clone()),
        ..AttemptFacts::default()
    });
    let draft = |outcome, reason| {
        JournalDraft::new(
            run.id().clone(),
            application.clock.now().expect("system clock is infallible"),
            "run.terminate",
            application
                .ids
                .request_id()
                .expect("UUID allocation is infallible"),
            outcome,
            reason,
            attempt.clone(),
            JournalExtension::RunTerminated,
        )
    };
    let completed = draft(OutcomeClass::Completed, None);
    let terminal_reason = Reason::new(
        ReasonCode::RunLifecycleTerminal,
        "run lifecycle is already terminal",
    )
    .expect("static reason is bounded");
    let rejected = draft(OutcomeClass::Rejected, Some(terminal_reason));
    let stale_reason = Reason::new(
        ReasonCode::StateStaleVersion,
        "run lifecycle changed before termination committed",
    )
    .expect("static reason is bounded");
    let stale = draft(OutcomeClass::Error, Some(stale_reason));
    let (completed, rejected, stale) = match completed.and_then(|completed| {
        rejected.and_then(|rejected| stale.map(|stale| (completed, rejected, stale)))
    }) {
        Ok(drafts) => drafts,
        Err(error) => {
            return delivered(
                failed(ReasonCode::PersistenceFailed, error.to_string()),
                json!({}),
                false,
            );
        }
    };
    let command = match build_terminate_command(&run, &delivery, completed, rejected, stale) {
        Ok(command) => command,
        Err(error) => {
            return delivered(
                failed(ReasonCode::PersistenceFailed, error.to_string()),
                json!({}),
                false,
            );
        }
    };
    match terminate(&application.run_mutations, command) {
        Ok(execution) => {
            let snapshot = RunSnapshot {
                run_id: run.id().clone(),
                label: execution.run.label,
                lifecycle: execution.run.lifecycle,
                current_state: execution.run.current_state,
                state_changed: false,
            };
            let outcome = match execution.outcome {
                OutcomeClass::Completed => completed_with_run_snapshot(snapshot, Vec::new()),
                OutcomeClass::Rejected => outcome_with_run(
                    OutcomeClass::Rejected,
                    ReasonCode::RunLifecycleTerminal,
                    "run lifecycle is already terminal",
                    snapshot,
                    Vec::new(),
                ),
                OutcomeClass::Error => outcome_with_run(
                    OutcomeClass::Error,
                    ReasonCode::StateStaleVersion,
                    "run lifecycle changed before termination committed",
                    snapshot,
                    Vec::new(),
                ),
            };
            delivered(outcome, json!({}), execution.commit.committed)
        }
        Err(error) => delivered(
            failed(run_mutation_reason(&error), error.to_string()),
            json!({}),
            false,
        ),
    }
}

fn checked_run_value(
    impact: &loop_engine_core::capabilities::provider_catalog::ActiveRunImpact,
    report: &loop_engine_core::model::compatibility::CompatibilityReport,
) -> Value {
    let report = match report {
        loop_engine_core::model::compatibility::CompatibilityReport::Findings(findings) => {
            let values = findings
                .as_slice()
                .iter()
                .map(|finding| {
                    json!({
                        "capability": finding.capability(),
                        "status": match finding.status() {
                            loop_engine_core::model::compatibility::CompatibilityStatus::Compatible => "compatible",
                            loop_engine_core::model::compatibility::CompatibilityStatus::Incompatible => "incompatible",
                            loop_engine_core::model::compatibility::CompatibilityStatus::Unknown => "unknown",
                        },
                        "diagnostics": diagnostics_value(finding.diagnostics()),
                    })
                })
                .collect::<Vec<_>>();
            json!({"findings": values})
        }
        loop_engine_core::model::compatibility::CompatibilityReport::EvaluationError(
            diagnostics,
        ) => {
            json!({"evaluation_error": diagnostics_value(diagnostics.as_slice())})
        }
    };
    json!({
        "run_id": impact.run_id.as_str(),
        "graph_revision": impact.graph_revision.as_str(),
        "report": report,
    })
}

fn diagnostics_value(values: &[loop_engine_core::model::diagnostic::Diagnostic]) -> Vec<Value> {
    values
        .iter()
        .map(|diagnostic| {
            json!({
                "code": diagnostic.code(),
                "message": diagnostic.message(),
                "path": diagnostic.path(),
            })
        })
        .collect()
}

fn run_snapshot(run: &loop_engine_core::model::run::Run, state_changed: bool) -> RunSnapshot {
    RunSnapshot {
        run_id: run.id().clone(),
        label: run.label().map(str::to_owned),
        lifecycle: run.lifecycle(),
        current_state: run.current_state().clone(),
        state_changed,
    }
}

fn completed_with_run_snapshot(
    run: RunSnapshot,
    requestable_events: Vec<loop_engine_core::model::requestable::RequestableEvent>,
) -> PublicOutcome {
    PublicOutcome::new(
        OutcomeClass::Completed,
        None,
        OutcomeData::new(Some(run), Some(requestable_events), None)
            .expect("run outcome shape is valid"),
        Vec::new(),
    )
    .expect("completed run outcome is valid")
}

fn outcome_with_run(
    class: OutcomeClass,
    reason_code: ReasonCode,
    message: &str,
    run: RunSnapshot,
    requestable_events: Vec<loop_engine_core::model::requestable::RequestableEvent>,
) -> PublicOutcome {
    let reason = Reason::new(reason_code, message).expect("static reason is bounded");
    PublicOutcome::new(
        class,
        Some(reason),
        OutcomeData::new(Some(run), Some(requestable_events), None)
            .expect("run outcome shape is valid"),
        Vec::new(),
    )
    .expect("run outcome class matches reason")
}

fn lifecycle_value(value: Lifecycle) -> &'static str {
    match value {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

fn run_list_reason(error: &RunListError<RunReadError>) -> ReasonCode {
    match error {
        RunListError::Reader(RunReadError::Page(_)) => ReasonCode::CursorInvalid,
        _ => ReasonCode::PersistenceFailed,
    }
}

fn run_history_reason(error: &RunHistoryError<HistoryReadError>) -> ReasonCode {
    match error {
        RunHistoryError::Reader(HistoryReadError::NotFound { .. }) => ReasonCode::RunNotFound,
        RunHistoryError::Reader(HistoryReadError::Page(_)) => ReasonCode::CursorInvalid,
        _ => ReasonCode::PersistenceFailed,
    }
}

fn run_read_reason(error: &RunReadError) -> ReasonCode {
    match error {
        RunReadError::NotFound { .. } => ReasonCode::RunNotFound,
        RunReadError::Page(_) => ReasonCode::CursorInvalid,
        _ => ReasonCode::PersistenceFailed,
    }
}

fn run_request_read_reason(error: &RunRequestReadError) -> ReasonCode {
    match error {
        RunRequestReadError::Run(error) => run_read_reason(error),
        RunRequestReadError::Evidence(_) => ReasonCode::PersistenceFailed,
    }
}

fn run_mutation_reason(error: &RunMutationError) -> ReasonCode {
    match error {
        RunMutationError::NotFound { .. } => ReasonCode::RunNotFound,
        _ => ReasonCode::PersistenceFailed,
    }
}

fn provider_check_failure(
    error: ProviderCheckExecutionError<
        CatalogPersistenceError,
        RunReadError,
        AdapterError,
        loop_engine_integrations::sha256_digest::DigestError,
        String,
        Infallible,
    >,
) -> (TracedOperationResult, Value) {
    let (reason, message) = match error {
        ProviderCheckExecutionError::Catalog(error) => (catalog_reason(&error), error.to_string()),
        ProviderCheckExecutionError::Reader(error) => (run_read_reason(&error), error.to_string()),
        ProviderCheckExecutionError::Describe(error)
        | ProviderCheckExecutionError::Compatibility(error) => invocation_error_reason(error),
        ProviderCheckExecutionError::Digest(error) => {
            (ReasonCode::ProviderSpawnFailed, error.to_string())
        }
        ProviderCheckExecutionError::Request(error) => (ReasonCode::ResourceExhausted, error),
        ProviderCheckExecutionError::RowSize(error) => match error {},
        ProviderCheckExecutionError::CompatibilityEvaluation(_) => (
            ReasonCode::ProviderEvaluationError,
            "provider compatibility evaluation failed".into(),
        ),
        ProviderCheckExecutionError::RowTooLarge => (
            ReasonCode::ResourceExhausted,
            "provider check row exceeds page budget".into(),
        ),
        ProviderCheckExecutionError::InvalidPlan => (
            ReasonCode::PersistenceFailed,
            "provider check traversal plan is invalid".into(),
        ),
    };
    delivered(failed(reason, message), json!({}), false)
}

fn invocation_error_reason(error: InvocationError<AdapterError>) -> (ReasonCode, String) {
    match error {
        InvocationError::TraceBudgetUnavailable => (
            ReasonCode::ResourceExhausted,
            "provider trace budget unavailable".into(),
        ),
        InvocationError::Transport {
            source, failure, ..
        } => (failure.reason.code(), source.to_string()),
    }
}

fn run_create_failure(
    error: RunCreateExecutionError<
        CatalogPersistenceError,
        AdapterError,
        loop_engine_integrations::sha256_digest::DigestError,
        std::convert::Infallible,
        loop_engine_integrations::persistence::RunCreateError,
        loop_engine_core::model::journal::JournalError,
    >,
) -> (TracedOperationResult, Value) {
    let (reason, message) = match error {
        RunCreateExecutionError::Catalog(error) => (catalog_reason(&error), error.to_string()),
        RunCreateExecutionError::Invocation(error) => invocation_error_reason(error),
        RunCreateExecutionError::Digest(error) => {
            (ReasonCode::ProviderSpawnFailed, error.to_string())
        }
        RunCreateExecutionError::Id(error) => match error {},
        RunCreateExecutionError::Journal(error) => {
            (ReasonCode::PersistenceFailed, error.to_string())
        }
        RunCreateExecutionError::Writer(error) => {
            (ReasonCode::PersistenceFailed, error.to_string())
        }
        RunCreateExecutionError::Operation(error) => match error {
            RunCreateError::Graph(_) => (ReasonCode::ProviderGraphInvalid, error.to_string()),
            RunCreateError::Bound(_) | RunCreateError::Command(_) => {
                (ReasonCode::ResourceExhausted, error.to_string())
            }
            RunCreateError::InputsRejected => (ReasonCode::InputRejected, error.to_string()),
            RunCreateError::InputEvaluationError => {
                (ReasonCode::ProviderEvaluationError, error.to_string())
            }
            RunCreateError::DigestDrift => (ReasonCode::ProviderDriftDetected, error.to_string()),
            RunCreateError::InvalidProviderConfigRevision => {
                (ReasonCode::PersistenceFailed, error.to_string())
            }
        },
    };
    delivered(failed(reason, message), json!({}), false)
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
