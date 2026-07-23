use std::collections::{BTreeMap, BTreeSet};

use crate::capabilities::event_attempt_writer::EventAttemptWriter;
use crate::capabilities::persistence_commands::{
    CommitEventAttemptCommand, CommitStatus, EventAttemptParts, EventCommitBranch,
};
use crate::capabilities::provider_catalog::{
    ProviderCatalog, ProviderResolveFailure, ResolvedProviderConfig,
};
use crate::capabilities::provider_invoker::{GateRequest, InvocationError, ProviderInvoker};
use crate::capabilities::run_reader::{RunRequestReader, SelectedEvidenceReadError};
use crate::model::annotation::Note;
use crate::model::attempt::{
    AttemptFacts, EvidenceAssociations, GateVerdictFact, GateVerdictFacts, GateVerdictResult,
    JournalExtension, ProviderFact, ProviderRole, TransitionFact,
};
use crate::model::decision::{DecisionError, TransitionDecision, resolve_gate_free, resolve_gated};
use crate::model::diagnostic::Diagnostic;
use crate::model::evidence::{EvidenceAssociation, EvidenceRecord};
use crate::model::gate::GateEvaluation;
use crate::model::ids::{EventId, EvidenceId, RequestId, RunId, StateId};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::reason::{Reason, ReasonCode};
use crate::model::run::Run;
use crate::model::time::ObservedAt;
use crate::operations::{CommandError, validate_journal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestExecutionDisposition {
    Completed,
    Rejected,
    Error,
}

#[derive(Debug)]
pub struct RequestExecution {
    pub disposition: RequestExecutionDisposition,
    pub commit: CommitStatus,
    pub outcome: OutcomeClass,
    pub reason: Option<Reason>,
    pub diagnostics: Vec<Diagnostic>,
    pub evidence_recorded: crate::model::outcome::EvidenceRecordedStatus,
    pub run: crate::capabilities::persistence_commands::CommittedRunSnapshot,
    pub requestable_events: Vec<crate::model::requestable::RequestableEvent>,
}

pub enum RequestResolution<'a, C, R, I, Q> {
    Completed {
        decision: &'a TransitionDecision,
        provider_binding: Option<(&'a ProviderFact, &'a ResolvedProviderConfig, OutcomeClass)>,
    },
    DecisionRejected {
        error: &'a DecisionError,
        provider_binding: Option<(&'a ProviderFact, &'a ResolvedProviderConfig, OutcomeClass)>,
    },
    ProviderFactInvalid {
        fact: &'a ProviderFact,
        config: &'a ResolvedProviderConfig,
    },
    ProviderEvidenceInvalid {
        fact: &'a ProviderFact,
        config: &'a ResolvedProviderConfig,
    },
    InputEvidenceInvalid,
    SelectionInvalid,
    SelectionUnavailable,
    SelectionReadError(&'a R),
    CatalogError(&'a C, ProviderResolveFailure),
    GateRequestError(&'a Q),
    InvocationError {
        error: &'a InvocationError<I>,
        config: &'a ResolvedProviderConfig,
    },
}

#[derive(Debug)]
pub enum RequestExecutionError<R, J, W> {
    Lookup(R),
    Command(J),
    InvalidCommand(CommandError),
    Writer(W),
}

/// Builds the complete authoritative attempt command for one resolved request.
/// Delivery supplies only time/request identity; core owns disposition, journal facts,
/// evidence associations, and stale-version semantics.
#[allow(clippy::too_many_arguments)]
pub fn command<C, R, I, Q>(
    run: &Run,
    event: &EventId,
    selected: &[EvidenceRecord],
    inline_evidence: &[EvidenceRecord],
    note: Option<&Note>,
    resolution: &RequestResolution<'_, C, R, I, Q>,
    observed_at: ObservedAt,
    request_id: RequestId,
) -> Result<CommitEventAttemptCommand, CommandError> {
    let provider_evidence = authoritative_provider_evidence(resolution);
    let provider_scopes = authoritative_provider_gate_scopes(resolution);
    let provider_observations = provider_observations_for_resolution(resolution);
    let gate_verdict_facts = gate_facts_for_resolution(run, event, resolution)?;
    let diagnostics = diagnostics_for_resolution(resolution);
    let (outcome, reason) = outcome_for_resolution(resolution)?;
    let (target_state, target_lifecycle, applied) = command_transition(run, event, resolution);
    let retain_input_evidence = !matches!(resolution, RequestResolution::InputEvidenceInvalid);
    let retained_inline = if retain_input_evidence {
        inline_evidence.to_vec()
    } else {
        Vec::new()
    };
    let retained_selected = if retain_input_evidence { selected } else { &[] };

    let evidence = EvidenceAssociations {
        inline: retained_inline.clone(),
        selected_ids: retained_selected
            .iter()
            .map(EvidenceRecord::id)
            .cloned()
            .collect(),
        provider_recorded_ids: provider_evidence
            .iter()
            .map(EvidenceRecord::id)
            .cloned()
            .collect(),
    };
    let associations = retained_inline
        .iter()
        .chain(retained_selected.iter())
        .map(|record| EvidenceAssociation::new(record.id().clone(), Some(event.clone()), None))
        .chain(provider_evidence.iter().map(|record| {
            EvidenceAssociation::new(
                record.id().clone(),
                Some(event.clone()),
                provider_scopes.get(record.id()).cloned(),
            )
        }))
        .collect::<Vec<_>>();
    let transition = TransitionFact::new(
        event.clone(),
        run.current_state().clone(),
        target_state.clone(),
        applied,
    )
    .map_err(|_| CommandError::JournalMismatch)?;
    let ordinary_attempt = AttemptFacts {
        transition: Some(transition.clone()),
        provider_observations: provider_observations.clone(),
        gate_verdict_facts: gate_verdict_facts.clone(),
        evidence_associations: Some(evidence.clone()),
        evidence_recorded: Some(evidence.recorded_status()),
        note: note.cloned(),
        diagnostics: diagnostics.clone(),
        ..AttemptFacts::default()
    };
    let ordinary = JournalDraft::new(
        run.id().clone(),
        observed_at,
        "run.request",
        request_id.clone(),
        outcome,
        reason,
        Some(ordinary_attempt),
        JournalExtension::TransitionAttempt,
    )
    .map_err(|_| CommandError::JournalMismatch)?;
    let stale_evidence = EvidenceAssociations::default();
    let stale_attempt = AttemptFacts {
        transition: Some(
            TransitionFact::new(
                event.clone(),
                run.current_state().clone(),
                target_state.clone(),
                false,
            )
            .map_err(|_| CommandError::JournalMismatch)?,
        ),
        provider_observations,
        gate_verdict_facts,
        evidence_associations: Some(stale_evidence.clone()),
        evidence_recorded: Some(stale_evidence.recorded_status()),
        note: note.cloned(),
        diagnostics,
        ..AttemptFacts::default()
    };
    let stale = JournalDraft::new(
        run.id().clone(),
        observed_at,
        "run.request",
        request_id,
        OutcomeClass::Error,
        Some(
            Reason::new(
                ReasonCode::StateStaleVersion,
                "run changed before event attempt committed",
            )
            .map_err(|_| CommandError::JournalMismatch)?,
        ),
        Some(stale_attempt),
        JournalExtension::TransitionAttempt,
    )
    .map_err(|_| CommandError::JournalMismatch)?;
    let candidate = CommitEventAttemptCommand::from_parts(EventAttemptParts {
        run_id: run.id().clone(),
        expected_workflow_version: run.workflow_state_version(),
        expected_lifecycle_version: run.lifecycle_version(),
        source_state: run.current_state().clone(),
        target_state,
        target_lifecycle,
        inline_evidence: retained_inline,
        associations,
        provider_evidence,
        journal_entry: ordinary,
        stale_journal_entry: stale,
    });
    revalidate_request_command(
        run,
        event,
        selected,
        inline_evidence,
        note,
        resolution,
        candidate,
    )
}

fn provider_observations_for_resolution<C, R, I, Q>(
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Vec<ProviderFact> {
    match resolution {
        RequestResolution::Completed {
            provider_binding, ..
        }
        | RequestResolution::DecisionRejected {
            provider_binding, ..
        } => provider_binding
            .map(|(fact, _, _)| vec![fact.clone()])
            .unwrap_or_default(),
        RequestResolution::ProviderFactInvalid { fact, .. }
        | RequestResolution::ProviderEvidenceInvalid { fact, .. } => vec![(*fact).clone()],
        RequestResolution::InvocationError {
            error: InvocationError::Transport { fact, .. },
            ..
        } => vec![(**fact).clone()],
        _ => Vec::new(),
    }
}

fn gate_facts_for_resolution<C, R, I, Q>(
    run: &Run,
    event: &EventId,
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Result<Option<GateVerdictFacts>, CommandError> {
    let required = required_gates_for_resolution(run, event, resolution);
    let result = match resolution {
        RequestResolution::Completed { decision, .. } if !decision.required_gates().is_empty() => {
            Some(GateVerdictResult::Verdicts(
                decision
                    .required_gates()
                    .iter()
                    .cloned()
                    .map(|gate_id| GateVerdictFact::new(gate_id, true, None))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| CommandError::JournalMismatch)?,
            ))
        }
        RequestResolution::DecisionRejected {
            error: DecisionError::GateFailed { verdicts },
            ..
        } => Some(GateVerdictResult::Verdicts(
            verdicts
                .iter()
                .map(|verdict| GateVerdictFact::new(verdict.gate().clone(), verdict.passed(), None))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| CommandError::JournalMismatch)?,
        )),
        RequestResolution::DecisionRejected {
            error: DecisionError::Incompatible { diagnostics },
            ..
        } => Some(GateVerdictResult::Incompatibility(
            diagnostics
                .first()
                .cloned()
                .ok_or(CommandError::JournalMismatch)?,
        )),
        RequestResolution::DecisionRejected {
            error: DecisionError::EvaluationError { diagnostics },
            ..
        } => Some(GateVerdictResult::EvaluationError(
            crate::model::diagnostic::Diagnostics::new(diagnostics.clone())
                .map_err(|_| CommandError::JournalMismatch)?,
        )),
        _ => None,
    };
    result
        .map(|result| {
            GateVerdictFacts::new(event.clone(), required, result)
                .map_err(|_| CommandError::JournalMismatch)
        })
        .transpose()
}

fn diagnostics_for_resolution<C, R, I, Q>(
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Vec<Diagnostic> {
    match resolution {
        RequestResolution::DecisionRejected {
            error: DecisionError::MalformedVerdicts { source, .. },
            ..
        } => malformed_verdict_diagnostics(source),
        RequestResolution::DecisionRejected {
            error:
                DecisionError::Incompatible { diagnostics }
                | DecisionError::EvaluationError { diagnostics },
            ..
        } => diagnostics.clone(),
        RequestResolution::ProviderFactInvalid { .. } => invalid_provider_fact_diagnostics(),
        RequestResolution::ProviderEvidenceInvalid { .. } => {
            invalid_provider_evidence_diagnostics()
        }
        RequestResolution::InvocationError {
            error: InvocationError::Transport { failure, .. },
            ..
        } => failure.diagnostics.clone(),
        _ => Vec::new(),
    }
}

fn outcome_for_resolution<C, R, I, Q>(
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Result<(OutcomeClass, Option<Reason>), CommandError> {
    let failure = match resolution {
        RequestResolution::Completed { .. } => return Ok((OutcomeClass::Completed, None)),
        RequestResolution::DecisionRejected { error, .. } => {
            (reason_code_for_decision_error(error), error.to_string())
        }
        RequestResolution::ProviderFactInvalid { .. } => (
            ReasonCode::ProviderProtocolMalformed,
            "provider fact does not match authoritative invocation".into(),
        ),
        RequestResolution::ProviderEvidenceInvalid { .. } => (
            ReasonCode::ProviderEvidenceMalformed,
            "provider evidence conflicts with request evidence".into(),
        ),
        RequestResolution::InputEvidenceInvalid => (
            ReasonCode::EvidenceInvalid,
            "inline evidence is invalid".into(),
        ),
        RequestResolution::SelectionInvalid | RequestResolution::SelectionUnavailable => (
            ReasonCode::EvidenceSelectionInvalid,
            "selected evidence is invalid or unavailable".into(),
        ),
        RequestResolution::SelectionReadError(_) => (
            ReasonCode::PersistenceFailed,
            "selected evidence could not be read".into(),
        ),
        RequestResolution::CatalogError(_, failure) => (
            failure.reason_code(),
            "run provider configuration is unavailable".into(),
        ),
        RequestResolution::GateRequestError(_) => (
            ReasonCode::ResourceExhausted,
            "provider gate request exceeds resource bounds".into(),
        ),
        RequestResolution::InvocationError {
            error: InvocationError::TraceBudgetUnavailable,
            ..
        } => (
            ReasonCode::ResourceExhausted,
            "provider trace budget unavailable".into(),
        ),
        RequestResolution::InvocationError {
            error: InvocationError::Transport { failure, .. },
            ..
        } => return Ok((OutcomeClass::Error, Some(failure.reason.clone()))),
    };
    let reason = Reason::new(failure.0, failure.1).map_err(|_| CommandError::JournalMismatch)?;
    Ok((failure.0.outcome_class(), Some(reason)))
}

fn command_transition<C, R, I, Q>(
    run: &Run,
    event: &EventId,
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> (
    Option<StateId>,
    Option<crate::model::lifecycle::Lifecycle>,
    bool,
) {
    match resolution {
        RequestResolution::Completed { decision, .. } => (
            Some(decision.target().clone()),
            Some(decision.lifecycle()),
            true,
        ),
        RequestResolution::DecisionRejected { error, .. } => {
            (rejected_target(run, event, error), None, false)
        }
        _ => (transition_target(run, event), None, false),
    }
}

/// Coordinates one lookup, optional one config resolution/provider call, and one CAS commit.
/// Every post-lookup disposition reaches the command factory and atomic writer exactly once.
#[allow(clippy::too_many_arguments)]
pub fn execute<R, C, I, W, G, F, Q, J>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    run_id: &RunId,
    event: &EventId,
    selected_ids: &[EvidenceId],
    inline_evidence: &[EvidenceRecord],
    note: Option<&Note>,
    gate_request: G,
    command: F,
) -> Result<RequestExecution, RequestExecutionError<R::Error, J, W::Error>>
where
    R: RunRequestReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: EventAttemptWriter,
    G: FnMut(
        &crate::capabilities::provider_catalog::ResolvedProviderConfig,
        &Run,
        &[EvidenceRecord],
    ) -> Result<GateRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        &[EvidenceRecord],
        Option<&Note>,
        &RequestResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<CommitEventAttemptCommand, J>,
{
    execute_with_inline_result::<R, C, I, W, G, F, Q, J, std::convert::Infallible>(
        reader,
        catalog,
        invoker,
        writer,
        run_id,
        event,
        selected_ids,
        Ok(inline_evidence),
        note,
        gate_request,
        command,
    )
}

/// Production variant preserving invalid inline-input disposition after successful run lookup.
#[allow(clippy::too_many_arguments)]
pub fn execute_with_inline_result<R, C, I, W, G, F, Q, J, E>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    run_id: &RunId,
    event: &EventId,
    selected_ids: &[EvidenceId],
    inline_evidence: Result<&[EvidenceRecord], &E>,
    note: Option<&Note>,
    mut gate_request: G,
    mut command: F,
) -> Result<RequestExecution, RequestExecutionError<R::Error, J, W::Error>>
where
    R: RunRequestReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: EventAttemptWriter,
    G: FnMut(
        &crate::capabilities::provider_catalog::ResolvedProviderConfig,
        &Run,
        &[EvidenceRecord],
    ) -> Result<GateRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        &[EvidenceRecord],
        Option<&Note>,
        &RequestResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<CommitEventAttemptCommand, J>,
{
    let run = reader.get(run_id).map_err(RequestExecutionError::Lookup)?;
    let inline_evidence = match inline_evidence {
        Ok(evidence) => evidence,
        Err(_) => {
            return commit_resolution(
                writer,
                &run,
                event,
                &[],
                &[],
                note,
                &RequestResolution::InputEvidenceInvalid,
                RequestExecutionDisposition::Rejected,
                &mut command,
            );
        }
    };
    if !evidence_ids_are_unique(inline_evidence) {
        return commit_resolution(
            writer,
            &run,
            event,
            &[],
            inline_evidence,
            note,
            &RequestResolution::InputEvidenceInvalid,
            RequestExecutionDisposition::Rejected,
            &mut command,
        );
    }
    if !validate_evidence_selection(selected_ids) {
        return commit_resolution(
            writer,
            &run,
            event,
            &[],
            inline_evidence,
            note,
            &RequestResolution::SelectionInvalid,
            RequestExecutionDisposition::Rejected,
            &mut command,
        );
    }
    let selected = if selected_ids.is_empty() {
        vec![]
    } else {
        match reader.selected_evidence(run_id, selected_ids) {
            Ok(selected) => selected,
            Err(SelectedEvidenceReadError::Unavailable) => {
                return commit_resolution(
                    writer,
                    &run,
                    event,
                    &[],
                    inline_evidence,
                    note,
                    &RequestResolution::SelectionUnavailable,
                    RequestExecutionDisposition::Rejected,
                    &mut command,
                );
            }
            Err(SelectedEvidenceReadError::Read(error)) => {
                return commit_resolution(
                    writer,
                    &run,
                    event,
                    &[],
                    inline_evidence,
                    note,
                    &RequestResolution::SelectionReadError(&error),
                    RequestExecutionDisposition::Error,
                    &mut command,
                );
            }
        }
    };
    if !request_evidence_is_valid(selected_ids, &selected, inline_evidence) {
        return commit_resolution(
            writer,
            &run,
            event,
            &[],
            inline_evidence,
            note,
            &RequestResolution::InputEvidenceInvalid,
            RequestExecutionDisposition::Rejected,
            &mut command,
        );
    }
    match resolve_gate_free(&run, event) {
        Ok(decision) => commit_resolution(
            writer,
            &run,
            event,
            &selected,
            inline_evidence,
            note,
            &RequestResolution::Completed {
                decision: &decision,
                provider_binding: None,
            },
            RequestExecutionDisposition::Completed,
            &mut command,
        ),
        Err(DecisionError::GatesRequired) => {
            let config = match catalog.resolve_enabled("run.request", run.registration_id()) {
                Ok(config) => config,
                Err(error) => {
                    return commit_resolution(
                        writer,
                        &run,
                        event,
                        &selected,
                        inline_evidence,
                        note,
                        &RequestResolution::CatalogError(
                            &error,
                            C::classify_resolve_failure(&error),
                        ),
                        RequestExecutionDisposition::Error,
                        &mut command,
                    );
                }
            };
            let request = match gate_request(&config, &run, &selected) {
                Ok(request) => request,
                Err(error) => {
                    return commit_resolution(
                        writer,
                        &run,
                        event,
                        &selected,
                        inline_evidence,
                        note,
                        &RequestResolution::GateRequestError(&error),
                        RequestExecutionDisposition::Error,
                        &mut command,
                    );
                }
            };
            match invoker.evaluate_gates(&config, request) {
                Ok(result)
                    if provider_fact_matches_config(
                        &config,
                        &result.fact,
                        ProviderRole::EvaluateGates,
                    ) && result.fact.outcome == gate_provider_outcome(&result.evaluation)
                        && provider_evidence_is_valid(
                            &result.evaluation,
                            &selected,
                            inline_evidence,
                        ) =>
                {
                    match resolve_gated(&run, event, &result.evaluation) {
                        Ok(decision) => commit_resolution(
                            writer,
                            &run,
                            event,
                            &selected,
                            inline_evidence,
                            note,
                            &RequestResolution::Completed {
                                decision: &decision,
                                provider_binding: Some((
                                    &result.fact,
                                    &config,
                                    gate_provider_outcome(&result.evaluation),
                                )),
                            },
                            RequestExecutionDisposition::Completed,
                            &mut command,
                        ),
                        Err(error) => commit_resolution(
                            writer,
                            &run,
                            event,
                            &selected,
                            inline_evidence,
                            note,
                            &RequestResolution::DecisionRejected {
                                error: &error,
                                provider_binding: Some((
                                    &result.fact,
                                    &config,
                                    gate_provider_outcome(&result.evaluation),
                                )),
                            },
                            decision_error_disposition(&error),
                            &mut command,
                        ),
                    }
                }
                Ok(result)
                    if provider_fact_matches_config(
                        &config,
                        &result.fact,
                        ProviderRole::EvaluateGates,
                    ) && result.fact.outcome == gate_provider_outcome(&result.evaluation) =>
                {
                    commit_resolution(
                        writer,
                        &run,
                        event,
                        &selected,
                        inline_evidence,
                        note,
                        &RequestResolution::ProviderEvidenceInvalid {
                            fact: &result.fact,
                            config: &config,
                        },
                        RequestExecutionDisposition::Error,
                        &mut command,
                    )
                }
                Ok(result) => commit_resolution(
                    writer,
                    &run,
                    event,
                    &selected,
                    inline_evidence,
                    note,
                    &RequestResolution::ProviderFactInvalid {
                        fact: &result.fact,
                        config: &config,
                    },
                    RequestExecutionDisposition::Error,
                    &mut command,
                ),
                Err(error) => commit_resolution(
                    writer,
                    &run,
                    event,
                    &selected,
                    inline_evidence,
                    note,
                    &RequestResolution::InvocationError {
                        error: &error,
                        config: &config,
                    },
                    RequestExecutionDisposition::Error,
                    &mut command,
                ),
            }
        }
        Err(error) => commit_resolution(
            writer,
            &run,
            event,
            &selected,
            inline_evidence,
            note,
            &RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            },
            decision_error_disposition(&error),
            &mut command,
        ),
    }
}

fn gate_provider_outcome(evaluation: &GateEvaluation) -> OutcomeClass {
    match evaluation {
        GateEvaluation::Verdicts(_) => OutcomeClass::Completed,
        GateEvaluation::Incompatible(_) => OutcomeClass::Rejected,
        GateEvaluation::EvaluationError(_) => OutcomeClass::Error,
    }
}

fn provider_fact_matches_config(
    config: &ResolvedProviderConfig,
    fact: &ProviderFact,
    role: ProviderRole,
) -> bool {
    fact.role == role
        && fact.registration_id == *config.registration_id()
        && fact.config_revision == config.config_revision()
        && fact.executable.as_str() == config.config().executable()
}

fn decision_error_disposition(error: &DecisionError) -> RequestExecutionDisposition {
    match error {
        DecisionError::EvaluationError { .. }
        | DecisionError::MalformedVerdicts { .. }
        | DecisionError::DecisionMismatch
        | DecisionError::Mutation(_) => RequestExecutionDisposition::Error,
        _ => RequestExecutionDisposition::Rejected,
    }
}

#[allow(clippy::too_many_arguments)]
fn commit_resolution<R, C, I, Q, W, F, J>(
    writer: &W,
    run: &Run,
    event: &EventId,
    selected: &[EvidenceRecord],
    inline_evidence: &[EvidenceRecord],
    note: Option<&Note>,
    resolution: &RequestResolution<'_, C, R, I, Q>,
    disposition: RequestExecutionDisposition,
    command: &mut F,
) -> Result<RequestExecution, RequestExecutionError<R, J, W::Error>>
where
    W: EventAttemptWriter,
    F: for<'a> FnMut(
        &Run,
        &[EvidenceRecord],
        Option<&Note>,
        &RequestResolution<'a, C, R, I, Q>,
    ) -> Result<CommitEventAttemptCommand, J>,
{
    let command =
        command(run, selected, note, resolution).map_err(RequestExecutionError::Command)?;
    let command = revalidate_request_command(
        run,
        event,
        selected,
        inline_evidence,
        note,
        resolution,
        command,
    )
    .map_err(RequestExecutionError::InvalidCommand)?;
    let status = writer
        .commit_event_attempt(command)
        .map_err(RequestExecutionError::Writer)?;
    let disposition = match status.branch {
        EventCommitBranch::ExpectedVersions => disposition,
        EventCommitBranch::StaleVersions | EventCommitBranch::ProviderEvidenceConflict => {
            RequestExecutionDisposition::Error
        }
        EventCommitBranch::InlineEvidenceConflict => RequestExecutionDisposition::Rejected,
    };
    let requestable_events = crate::model::requestable::project_state(
        run.graph(),
        status.run.lifecycle,
        &status.run.current_state,
    );
    Ok(RequestExecution {
        disposition,
        commit: status.commit,
        outcome: status.outcome,
        reason: status.reason,
        diagnostics: status.diagnostics,
        evidence_recorded: status.evidence_recorded,
        run: status.run,
        requestable_events,
    })
}

fn revalidate_request_command<C, R, I, Q>(
    run: &Run,
    event: &EventId,
    selected: &[EvidenceRecord],
    inline_evidence: &[EvidenceRecord],
    note: Option<&Note>,
    resolution: &RequestResolution<'_, C, R, I, Q>,
    command: CommitEventAttemptCommand,
) -> Result<CommitEventAttemptCommand, CommandError> {
    let parts = command.into_parts();
    let authoritative_provider = authoritative_provider_evidence(resolution);
    let (authoritative_selected, authoritative_inline) = match resolution {
        RequestResolution::InputEvidenceInvalid => (&[][..], &[][..]),
        _ => (selected, inline_evidence),
    };
    if parts
        .journal_entry
        .attempt()
        .and_then(|attempt| attempt.note.as_ref())
        != note
        || parts
            .stale_journal_entry
            .attempt()
            .and_then(|attempt| attempt.note.as_ref())
            != note
        || parts.inline_evidence.as_slice() != authoritative_inline
        || parts.provider_evidence.as_slice() != authoritative_provider.as_slice()
        || !provenance_ids_match(
            &parts.associations,
            authoritative_selected,
            authoritative_inline,
            &authoritative_provider,
        )
        || !association_context_valid(
            &parts.associations,
            event,
            &required_gates_for_resolution(run, event, resolution),
            &authoritative_provider_gate_scopes(resolution),
        )
    {
        return Err(CommandError::JournalMismatch);
    }

    match resolution {
        RequestResolution::Completed {
            decision,
            provider_binding,
        } => {
            if !decision_parts_match(&parts, decision) {
                return Err(CommandError::JournalMismatch);
            }
            completed_command(
                decision,
                *provider_binding,
                parts.inline_evidence,
                parts.associations,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::DecisionRejected {
            error,
            provider_binding,
        } => {
            if !rejected_parts_match(run, event, &parts, error)
                || !rejected_journal_matches_resolution(run, event, error, &parts.journal_entry)
            {
                return Err(CommandError::JournalMismatch);
            }
            rejected_command(
                run,
                event,
                *provider_binding,
                rejected_target(run, event, error),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::ProviderFactInvalid { fact, config: _ } => {
            if !pre_provider_parts_match(run, event, &parts) {
                return Err(CommandError::JournalMismatch);
            }
            invalid_provider_fact_command(
                run,
                event,
                fact,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::ProviderEvidenceInvalid { fact, config } => {
            if !pre_provider_parts_match(run, event, &parts) {
                return Err(CommandError::JournalMismatch);
            }
            invalid_provider_evidence_command(
                run,
                event,
                fact,
                config,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::InvocationError { error, config } => {
            if !pre_provider_parts_match(run, event, &parts)
                || !invocation_error_journal_matches(&parts.journal_entry, error)
            {
                return Err(CommandError::JournalMismatch);
            }
            let provider_binding = match error {
                InvocationError::Transport { fact, .. } => {
                    Some((fact.as_ref(), *config, OutcomeClass::Error))
                }
                InvocationError::TraceBudgetUnavailable => None,
            };
            rejected_command(
                run,
                event,
                provider_binding,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::InputEvidenceInvalid => {
            if !pre_provider_parts_match(run, event, &parts)
                || !pre_provider_journal_matches(
                    &parts.journal_entry,
                    OutcomeClass::Rejected,
                    ReasonCode::EvidenceInvalid,
                )
            {
                return Err(CommandError::JournalMismatch);
            }
            rejected_command(
                run,
                event,
                None,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::SelectionInvalid | RequestResolution::SelectionUnavailable => {
            revalidate_pre_provider_failure(
                run,
                event,
                &parts,
                OutcomeClass::Rejected,
                ReasonCode::EvidenceSelectionInvalid,
            )?;
            rejected_command(
                run,
                event,
                None,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::SelectionReadError(_) => {
            revalidate_pre_provider_failure(
                run,
                event,
                &parts,
                OutcomeClass::Error,
                ReasonCode::PersistenceFailed,
            )?;
            rejected_command(
                run,
                event,
                None,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::CatalogError(_, reason) => {
            revalidate_pre_provider_failure(
                run,
                event,
                &parts,
                OutcomeClass::Error,
                reason.reason_code(),
            )?;
            rejected_command(
                run,
                event,
                None,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
        RequestResolution::GateRequestError(_) => {
            revalidate_pre_provider_failure(
                run,
                event,
                &parts,
                OutcomeClass::Error,
                ReasonCode::ResourceExhausted,
            )?;
            rejected_command(
                run,
                event,
                None,
                transition_target(run, event),
                parts.inline_evidence,
                parts.associations,
                authoritative_provider,
                parts.journal_entry,
                parts.stale_journal_entry,
            )
        }
    }
}

fn revalidate_pre_provider_failure(
    run: &Run,
    event: &EventId,
    parts: &EventAttemptParts,
    outcome: OutcomeClass,
    reason: ReasonCode,
) -> Result<(), CommandError> {
    if !pre_provider_parts_match(run, event, parts)
        || !pre_provider_journal_matches(&parts.journal_entry, outcome, reason)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

fn authoritative_provider_evidence<C, R, I, Q>(
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Vec<EvidenceRecord> {
    match resolution {
        RequestResolution::Completed { decision, .. } => decision.provider_evidence().to_vec(),
        RequestResolution::DecisionRejected { error, .. } => provider_evidence_from_error(error),
        RequestResolution::ProviderFactInvalid { .. }
        | RequestResolution::ProviderEvidenceInvalid { .. }
        | RequestResolution::InvocationError { .. }
        | RequestResolution::InputEvidenceInvalid
        | RequestResolution::SelectionInvalid
        | RequestResolution::SelectionUnavailable
        | RequestResolution::SelectionReadError(_)
        | RequestResolution::CatalogError(_, _)
        | RequestResolution::GateRequestError(_) => Vec::new(),
    }
}

fn provider_evidence_from_error(error: &DecisionError) -> Vec<EvidenceRecord> {
    match error {
        DecisionError::GateFailed { verdicts } => verdicts
            .iter()
            .flat_map(|verdict| verdict.evidence().iter().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

fn required_gates_for_resolution<C, R, I, Q>(
    run: &Run,
    event: &EventId,
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> Vec<crate::model::ids::GateId> {
    match resolution {
        RequestResolution::Completed { decision, .. } => decision.required_gates().to_vec(),
        RequestResolution::DecisionRejected { .. }
        | RequestResolution::ProviderFactInvalid { .. }
        | RequestResolution::ProviderEvidenceInvalid { .. }
        | RequestResolution::InvocationError { .. } => required_gates_for_event(run, event),
        _ => Vec::new(),
    }
}

fn decision_parts_match(parts: &EventAttemptParts, decision: &TransitionDecision) -> bool {
    parts.run_id == *decision.run_id()
        && parts.expected_workflow_version == decision.expected_workflow_version()
        && parts.expected_lifecycle_version == decision.expected_lifecycle_version()
        && parts.source_state == *decision.source()
        && parts.target_state.as_ref() == Some(decision.target())
        && parts.target_lifecycle == Some(decision.lifecycle())
}

fn rejected_parts_match(
    run: &Run,
    event: &EventId,
    parts: &EventAttemptParts,
    error: &DecisionError,
) -> bool {
    parts.run_id == *run.id()
        && parts.expected_workflow_version == run.workflow_state_version()
        && parts.expected_lifecycle_version == run.lifecycle_version()
        && parts.source_state == *run.current_state()
        && parts.target_state == rejected_target(run, event, error)
        && parts.target_lifecycle.is_none()
}

fn rejected_journal_matches_resolution(
    run: &Run,
    event: &EventId,
    error: &DecisionError,
    journal_entry: &JournalDraft,
) -> bool {
    let Some(attempt) = journal_entry.attempt() else {
        return false;
    };
    let expected_outcome = match decision_error_disposition(error) {
        RequestExecutionDisposition::Rejected => OutcomeClass::Rejected,
        RequestExecutionDisposition::Error => OutcomeClass::Error,
        RequestExecutionDisposition::Completed => return false,
    };
    journal_entry.outcome() == expected_outcome
        && journal_entry.reason().map(|reason| reason.code())
            == Some(reason_code_for_decision_error(error))
        && rejected_gate_facts_match(run, event, error, attempt)
}

fn reason_code_for_decision_error(error: &DecisionError) -> ReasonCode {
    match error {
        DecisionError::Terminal => ReasonCode::RunLifecycleTerminal,
        DecisionError::UnknownEvent(_) | DecisionError::AmbiguousEvent(_) => {
            ReasonCode::EventUnknown
        }
        DecisionError::GatesRequired | DecisionError::EvaluationError { .. } => {
            ReasonCode::ProviderEvaluationError
        }
        DecisionError::MalformedVerdicts { .. } => ReasonCode::ProviderProtocolMalformed,
        DecisionError::GateFailed { .. } => ReasonCode::GateFailed,
        DecisionError::Incompatible { .. } => ReasonCode::CompatibilityUnsupported,
        DecisionError::DecisionMismatch | DecisionError::Mutation(_) => {
            ReasonCode::StateStaleVersion
        }
    }
}

fn rejected_gate_facts_match(
    run: &Run,
    event: &EventId,
    error: &DecisionError,
    attempt: &crate::model::attempt::AttemptFacts,
) -> bool {
    let required = required_gates_for_event(run, event);
    match error {
        DecisionError::GateFailed { verdicts } => {
            attempt.diagnostics.is_empty()
                && attempt.gate_verdict_facts.as_ref().is_some_and(|facts| {
                    facts.event == *event
                        && facts.gate_ids == required
                        && matches!(&facts.result, GateVerdictResult::Verdicts(recorded)
                        if recorded.len() == verdicts.len()
                            && recorded.iter().zip(verdicts).all(|(fact, verdict)| {
                                fact.gate_id == *verdict.gate()
                                    && fact.passed == verdict.passed()
                                    && fact.message.is_none()
                            }))
                })
        }
        DecisionError::MalformedVerdicts { source, .. } => {
            attempt.gate_verdict_facts.is_none()
                && attempt.diagnostics == malformed_verdict_diagnostics(source)
        }
        DecisionError::Incompatible { diagnostics } => {
            attempt.diagnostics == *diagnostics
                && attempt.gate_verdict_facts.as_ref().is_some_and(|facts| {
                    facts.event == *event
                        && facts.gate_ids == required
                        && matches!(&facts.result, GateVerdictResult::Incompatibility(recorded)
                            if diagnostics.first() == Some(recorded))
                })
        }
        DecisionError::EvaluationError { diagnostics } => {
            attempt.diagnostics == *diagnostics
                && attempt.gate_verdict_facts.as_ref().is_some_and(|facts| {
                    facts.event == *event
                        && facts.gate_ids == required
                        && matches!(&facts.result, GateVerdictResult::EvaluationError(recorded)
                            if recorded.as_slice() == diagnostics)
                })
        }
        DecisionError::Terminal
        | DecisionError::UnknownEvent(_)
        | DecisionError::AmbiguousEvent(_)
        | DecisionError::GatesRequired
        | DecisionError::DecisionMismatch
        | DecisionError::Mutation(_) => {
            attempt.gate_verdict_facts.is_none() && attempt.diagnostics.is_empty()
        }
    }
}

fn invocation_error_journal_matches<E>(
    journal_entry: &JournalDraft,
    error: &InvocationError<E>,
) -> bool {
    let Some(attempt) = journal_entry.attempt() else {
        return false;
    };
    match error {
        InvocationError::TraceBudgetUnavailable => {
            journal_entry.outcome() == OutcomeClass::Error
                && journal_entry.reason().map(|reason| reason.code())
                    == Some(ReasonCode::ResourceExhausted)
                && attempt.provider_observations.is_empty()
                && attempt.gate_verdict_facts.is_none()
                && attempt.diagnostics.is_empty()
        }
        InvocationError::Transport { failure, .. } => {
            journal_entry.outcome() == OutcomeClass::Error
                && journal_entry.reason() == Some(&failure.reason)
                && attempt.gate_verdict_facts.is_none()
                && attempt.diagnostics == failure.diagnostics
        }
    }
}

fn pre_provider_journal_matches(
    journal_entry: &JournalDraft,
    outcome: OutcomeClass,
    reason: ReasonCode,
) -> bool {
    journal_entry.outcome() == outcome
        && journal_entry.reason().map(|value| value.code()) == Some(reason)
        && journal_entry.attempt().is_some_and(|attempt| {
            attempt.provider_observations.is_empty()
                && attempt.gate_verdict_facts.is_none()
                && attempt.diagnostics.is_empty()
        })
}

fn pre_provider_parts_match(run: &Run, event: &EventId, parts: &EventAttemptParts) -> bool {
    parts.run_id == *run.id()
        && parts.expected_workflow_version == run.workflow_state_version()
        && parts.expected_lifecycle_version == run.lifecycle_version()
        && parts.source_state == *run.current_state()
        && parts.target_state == transition_target(run, event)
        && parts.target_lifecycle.is_none()
}

fn rejected_target(run: &Run, event: &EventId, error: &DecisionError) -> Option<StateId> {
    match error {
        DecisionError::Terminal
        | DecisionError::UnknownEvent(_)
        | DecisionError::AmbiguousEvent(_)
        | DecisionError::GatesRequired => None,
        _ => transition_target(run, event),
    }
}

fn transition_target(run: &Run, event: &EventId) -> Option<StateId> {
    run.graph()
        .transitions()
        .iter()
        .find(|transition| {
            transition.source() == run.current_state() && transition.event() == event
        })
        .map(|transition| transition.target().clone())
}

fn authoritative_provider_gate_scopes<C, R, I, Q>(
    resolution: &RequestResolution<'_, C, R, I, Q>,
) -> BTreeMap<EvidenceId, crate::model::ids::GateId> {
    match resolution {
        RequestResolution::Completed { decision, .. } => {
            decision.provider_evidence_scopes().clone()
        }
        RequestResolution::DecisionRejected {
            error: DecisionError::GateFailed { verdicts },
            ..
        } => verdicts
            .iter()
            .flat_map(|verdict| {
                verdict
                    .evidence()
                    .iter()
                    .map(|evidence| (evidence.id().clone(), verdict.gate().clone()))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn provenance_ids_match(
    associations: &[EvidenceAssociation],
    selected: &[EvidenceRecord],
    inline: &[EvidenceRecord],
    provider: &[EvidenceRecord],
) -> bool {
    let expected_count = inline
        .len()
        .saturating_add(selected.len())
        .saturating_add(provider.len());
    let expected = inline
        .iter()
        .chain(selected.iter())
        .chain(provider.iter())
        .map(EvidenceRecord::id)
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual = associations
        .iter()
        .map(EvidenceAssociation::evidence_id)
        .cloned()
        .collect::<BTreeSet<_>>();
    expected.len() == expected_count
        && actual.len() == associations.len()
        && associations.len() == expected_count
        && actual == expected
}

fn association_context_valid(
    associations: &[EvidenceAssociation],
    event: &EventId,
    required_gates: &[crate::model::ids::GateId],
    provider_scopes: &BTreeMap<EvidenceId, crate::model::ids::GateId>,
) -> bool {
    associations.iter().all(|association| {
        association.event_id() == Some(event)
            && match provider_scopes.get(association.evidence_id()) {
                Some(gate) => required_gates.contains(gate) && association.gate_id() == Some(gate),
                None => association.gate_id().is_none(),
            }
    })
}

pub fn validate_evidence_selection(ids: &[EvidenceId]) -> bool {
    ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

fn evidence_ids_are_unique(records: &[EvidenceRecord]) -> bool {
    records
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>()
        .len()
        == records.len()
}

fn request_evidence_is_valid(
    selected_ids: &[EvidenceId],
    selected: &[EvidenceRecord],
    inline: &[EvidenceRecord],
) -> bool {
    let requested = selected_ids.iter().collect::<BTreeSet<_>>();
    let returned = selected
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    let inline_ids = inline
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    evidence_ids_are_unique(selected)
        && evidence_ids_are_unique(inline)
        && requested == returned
        && returned.is_disjoint(&inline_ids)
}

fn provider_evidence_is_valid(
    evaluation: &GateEvaluation,
    selected: &[EvidenceRecord],
    inline: &[EvidenceRecord],
) -> bool {
    let GateEvaluation::Verdicts(verdicts) = evaluation else {
        return true;
    };
    let provider = verdicts
        .as_slice()
        .iter()
        .flat_map(|verdict| verdict.evidence());
    let request_ids = selected
        .iter()
        .chain(inline)
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    let mut provider_ids = BTreeSet::new();
    provider
        .into_iter()
        .all(|record| provider_ids.insert(record.id()) && !request_ids.contains(record.id()))
}

pub fn malformed_verdict_diagnostics(
    source: &crate::model::gate::VerdictSetError,
) -> Vec<Diagnostic> {
    vec![
        Diagnostic::new("provider.verdict_set.malformed", source.to_string(), None)
            .expect("fixed diagnostic code and bounded verdict-set message"),
    ]
}

pub fn invalid_provider_evidence_diagnostics() -> Vec<Diagnostic> {
    vec![
        Diagnostic::new(
            "provider.evidence.malformed",
            "provider evidence duplicates another provider record or request evidence",
            None,
        )
        .expect("fixed provider-evidence diagnostic is bounded"),
    ]
}

pub fn invalid_provider_fact_diagnostics() -> Vec<Diagnostic> {
    vec![
        Diagnostic::new(
            "provider.protocol.malformed",
            "provider fact does not match the authoritative invocation",
            None,
        )
        .expect("fixed provider-fact diagnostic is bounded"),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceConflictKind {
    Inline,
    Provider,
}

pub fn evidence_conflict_journal(
    ordinary: &JournalDraft,
    kind: EvidenceConflictKind,
) -> Result<JournalDraft, CommandError> {
    let mut attempt = ordinary
        .attempt()
        .cloned()
        .ok_or(CommandError::JournalMismatch)?;
    let transition = attempt
        .transition
        .as_mut()
        .ok_or(CommandError::JournalMismatch)?;
    transition.applied = false;
    let associations = crate::model::attempt::EvidenceAssociations::default();
    attempt.evidence_associations = Some(associations.clone());
    attempt.evidence_recorded = Some(associations.recorded_status());
    let (outcome, code, message) = match kind {
        EvidenceConflictKind::Inline => (
            OutcomeClass::Rejected,
            ReasonCode::EvidenceInvalid,
            "inline evidence id already exists for run",
        ),
        EvidenceConflictKind::Provider => (
            OutcomeClass::Error,
            ReasonCode::ProviderEvidenceMalformed,
            "provider evidence id already exists for run",
        ),
    };
    JournalDraft::new(
        ordinary.run_id().clone(),
        ordinary.observed_at(),
        ordinary.operation(),
        ordinary.request_id().clone(),
        outcome,
        Some(Reason::new(code, message).map_err(|_| CommandError::JournalMismatch)?),
        Some(attempt),
        ordinary.extension().clone(),
    )
    .map_err(|_| CommandError::JournalMismatch)
}

pub(crate) fn completed_command(
    decision: &TransitionDecision,
    provider_binding: Option<(&ProviderFact, &ResolvedProviderConfig, OutcomeClass)>,
    inline_evidence: Vec<EvidenceRecord>,
    associations: Vec<EvidenceAssociation>,
    journal_entry: JournalDraft,
    stale_journal_entry: JournalDraft,
) -> Result<CommitEventAttemptCommand, CommandError> {
    validate_attempt_journals(decision.run_id(), &journal_entry, &stale_journal_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let provider_facts_match = if decision.required_gates().is_empty() {
        provider_binding.is_none() && attempt.provider_observations.is_empty()
    } else {
        matches!((provider_binding, attempt.provider_observations.as_slice()),
            (Some((expected, config, expected_outcome)), [fact]) if fact == expected
                && provider_fact_matches_config(config, fact, ProviderRole::EvaluateGates)
                && fact.outcome == expected_outcome)
    };
    let transition_matches = attempt.transition.as_ref().is_some_and(|transition| {
        transition.event == *decision.event()
            && transition.source == *decision.source()
            && transition.target.as_ref() == Some(decision.target())
            && transition.applied
    });
    if !provider_facts_match
        || !attempt.diagnostics.is_empty()
        || !transition_matches
        || !completed_gate_facts_match(decision, attempt)
        || !evidence_matches(
            attempt,
            &inline_evidence,
            &associations,
            decision.provider_evidence(),
            decision.event(),
            decision.required_gates(),
        )
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(CommitEventAttemptCommand::from_parts(EventAttemptParts {
        run_id: decision.run_id().clone(),
        expected_workflow_version: decision.expected_workflow_version(),
        expected_lifecycle_version: decision.expected_lifecycle_version(),
        source_state: decision.source().clone(),
        target_state: Some(decision.target().clone()),
        target_lifecycle: Some(decision.lifecycle()),
        inline_evidence,
        associations,
        provider_evidence: decision.provider_evidence().to_vec(),
        journal_entry,
        stale_journal_entry,
    }))
}

#[cfg(feature = "test-support")]
pub fn completed_command_for_test(
    decision: &TransitionDecision,
    provider_binding: Option<(&ProviderFact, &ResolvedProviderConfig, OutcomeClass)>,
    inline_evidence: Vec<EvidenceRecord>,
    associations: Vec<EvidenceAssociation>,
    journal_entry: JournalDraft,
    stale_journal_entry: JournalDraft,
) -> Result<CommitEventAttemptCommand, CommandError> {
    completed_command(
        decision,
        provider_binding,
        inline_evidence,
        associations,
        journal_entry,
        stale_journal_entry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rejected_command(
    run: &Run,
    event: &EventId,
    provider_binding: Option<(&ProviderFact, &ResolvedProviderConfig, OutcomeClass)>,
    resolved_target: Option<StateId>,
    inline_evidence: Vec<EvidenceRecord>,
    associations: Vec<EvidenceAssociation>,
    provider_evidence: Vec<EvidenceRecord>,
    journal_entry: JournalDraft,
    stale_journal_entry: JournalDraft,
) -> Result<CommitEventAttemptCommand, CommandError> {
    validate_attempt_journals(run.id(), &journal_entry, &stale_journal_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let provider_facts_match = match provider_binding {
        None => attempt.provider_observations.is_empty(),
        Some((expected, config, expected_outcome)) => {
            matches!(attempt.provider_observations.as_slice(), [fact]
                if fact == expected
                    && provider_fact_matches_config(config, fact, ProviderRole::EvaluateGates)
                    && fact.outcome == expected_outcome)
        }
    };
    let transition_matches = attempt.transition.as_ref().is_some_and(|transition| {
        transition.event == *event
            && transition.source == *run.current_state()
            && transition.target == resolved_target
            && !transition.applied
    });
    if !provider_facts_match
        || !transition_matches
        || !evidence_matches(
            attempt,
            &inline_evidence,
            &associations,
            &provider_evidence,
            event,
            &required_gates_for_event(run, event),
        )
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(CommitEventAttemptCommand::from_parts(EventAttemptParts {
        run_id: run.id().clone(),
        expected_workflow_version: run.workflow_state_version(),
        expected_lifecycle_version: run.lifecycle_version(),
        source_state: run.current_state().clone(),
        target_state: resolved_target,
        target_lifecycle: None,
        inline_evidence,
        associations,
        provider_evidence,
        journal_entry,
        stale_journal_entry,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn invalid_provider_evidence_command(
    run: &Run,
    event: &EventId,
    fact: &ProviderFact,
    config: &ResolvedProviderConfig,
    resolved_target: Option<StateId>,
    inline_evidence: Vec<EvidenceRecord>,
    associations: Vec<EvidenceAssociation>,
    journal_entry: JournalDraft,
    stale_journal_entry: JournalDraft,
) -> Result<CommitEventAttemptCommand, CommandError> {
    validate_attempt_journals(run.id(), &journal_entry, &stale_journal_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let transition_matches = attempt.transition.as_ref().is_some_and(|transition| {
        transition.event == *event
            && transition.source == *run.current_state()
            && transition.target == resolved_target
            && !transition.applied
    });
    if journal_entry.outcome() != OutcomeClass::Error
        || journal_entry.reason().map(|reason| reason.code())
            != Some(ReasonCode::ProviderEvidenceMalformed)
        || !provider_fact_matches_config(config, fact, ProviderRole::EvaluateGates)
        || fact.outcome != OutcomeClass::Completed
        || !matches!(attempt.provider_observations.as_slice(), [recorded] if recorded == fact)
        || attempt.diagnostics != invalid_provider_evidence_diagnostics()
        || attempt.gate_verdict_facts.is_some()
        || !transition_matches
        || !evidence_matches(
            attempt,
            &inline_evidence,
            &associations,
            &[],
            event,
            &required_gates_for_event(run, event),
        )
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(CommitEventAttemptCommand::from_parts(EventAttemptParts {
        run_id: run.id().clone(),
        expected_workflow_version: run.workflow_state_version(),
        expected_lifecycle_version: run.lifecycle_version(),
        source_state: run.current_state().clone(),
        target_state: resolved_target,
        target_lifecycle: None,
        inline_evidence,
        associations,
        provider_evidence: Vec::new(),
        journal_entry,
        stale_journal_entry,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn invalid_provider_fact_command(
    run: &Run,
    event: &EventId,
    fact: &ProviderFact,
    resolved_target: Option<StateId>,
    inline_evidence: Vec<EvidenceRecord>,
    associations: Vec<EvidenceAssociation>,
    journal_entry: JournalDraft,
    stale_journal_entry: JournalDraft,
) -> Result<CommitEventAttemptCommand, CommandError> {
    validate_attempt_journals(run.id(), &journal_entry, &stale_journal_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let transition_matches = attempt.transition.as_ref().is_some_and(|transition| {
        transition.event == *event
            && transition.source == *run.current_state()
            && transition.target == resolved_target
            && !transition.applied
    });
    if journal_entry.outcome() != OutcomeClass::Error
        || journal_entry.reason().map(|reason| reason.code())
            != Some(ReasonCode::ProviderProtocolMalformed)
        || !matches!(attempt.provider_observations.as_slice(), [recorded] if recorded == fact)
        || attempt.diagnostics != invalid_provider_fact_diagnostics()
        || attempt.gate_verdict_facts.is_some()
        || !transition_matches
        || !evidence_matches(
            attempt,
            &inline_evidence,
            &associations,
            &[],
            event,
            &required_gates_for_event(run, event),
        )
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(CommitEventAttemptCommand::from_parts(EventAttemptParts {
        run_id: run.id().clone(),
        expected_workflow_version: run.workflow_state_version(),
        expected_lifecycle_version: run.lifecycle_version(),
        source_state: run.current_state().clone(),
        target_state: resolved_target,
        target_lifecycle: None,
        inline_evidence,
        associations,
        provider_evidence: Vec::new(),
        journal_entry,
        stale_journal_entry,
    }))
}

fn completed_gate_facts_match(
    decision: &TransitionDecision,
    attempt: &crate::model::attempt::AttemptFacts,
) -> bool {
    if decision.required_gates().is_empty() {
        return attempt.gate_verdict_facts.is_none();
    }
    let Some(facts) = &attempt.gate_verdict_facts else {
        return false;
    };
    let GateVerdictResult::Verdicts(verdicts) = &facts.result else {
        return false;
    };
    let verdict_ids = verdicts
        .iter()
        .map(|verdict| &verdict.gate_id)
        .collect::<BTreeSet<_>>();
    let required_ids = decision.required_gates().iter().collect::<BTreeSet<_>>();
    facts.event == *decision.event()
        && facts.gate_ids == decision.required_gates()
        && verdicts.len() == decision.required_gates().len()
        && verdict_ids == required_ids
        && verdicts
            .iter()
            .all(|verdict| verdict.passed && verdict.message.is_none())
}

fn required_gates_for_event(run: &Run, event: &EventId) -> Vec<crate::model::ids::GateId> {
    run.graph()
        .transitions()
        .iter()
        .find(|transition| {
            transition.source() == run.current_state() && transition.event() == event
        })
        .map_or_else(Vec::new, |transition| transition.required_gates().to_vec())
}

fn evidence_matches(
    attempt: &crate::model::attempt::AttemptFacts,
    inline: &[EvidenceRecord],
    associations: &[EvidenceAssociation],
    provider: &[EvidenceRecord],
    event: &EventId,
    required_gates: &[crate::model::ids::GateId],
) -> bool {
    let Some(recorded) = &attempt.evidence_associations else {
        return false;
    };
    let Some(recorded_status) = attempt.evidence_recorded else {
        return false;
    };
    if recorded.inline != inline || recorded.recorded_status() != recorded_status {
        return false;
    }

    let inline_ids = recorded
        .inline
        .iter()
        .map(EvidenceRecord::id)
        .cloned()
        .collect::<Vec<_>>();
    let provider_ids = provider
        .iter()
        .map(EvidenceRecord::id)
        .cloned()
        .collect::<Vec<_>>();
    if provider_ids != recorded.provider_recorded_ids {
        return false;
    }
    let expected_count = inline_ids
        .len()
        .saturating_add(recorded.selected_ids.len())
        .saturating_add(provider_ids.len());
    let expected = inline_ids
        .iter()
        .chain(recorded.selected_ids.iter())
        .chain(provider_ids.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual = associations
        .iter()
        .map(EvidenceAssociation::evidence_id)
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected.len() != expected_count
        || actual.len() != associations.len()
        || associations.len() != expected_count
        || actual != expected
    {
        return false;
    }

    let provider_ids = provider_ids.into_iter().collect::<BTreeSet<_>>();
    associations.iter().all(|association| {
        association.event_id() == Some(event)
            && association
                .gate_id()
                .is_none_or(|gate| required_gates.contains(gate))
            && (!provider_ids.contains(association.evidence_id())
                || association
                    .gate_id()
                    .is_some_and(|gate| required_gates.contains(gate)))
    })
}

fn validate_attempt_journals(
    run_id: &RunId,
    journal_entry: &JournalDraft,
    stale_journal_entry: &JournalDraft,
) -> Result<(), CommandError> {
    validate_journal(
        journal_entry,
        run_id,
        "run.request",
        JournalEntryKind::TransitionAttempt,
    )?;
    validate_journal(
        stale_journal_entry,
        run_id,
        "run.request",
        JournalEntryKind::TransitionAttempt,
    )?;
    let ordinary = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let stale = stale_journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    if stale_journal_entry.outcome() != OutcomeClass::Error
        || stale_journal_entry.reason().map(|reason| reason.code())
            != Some(ReasonCode::StateStaleVersion)
        || ordinary.actor.is_some()
        || ordinary.corrects_sequence.is_some()
        || stale.actor.is_some()
        || stale.corrects_sequence.is_some()
        || stale.provider_observations != ordinary.provider_observations
        || stale.gate_verdict_facts != ordinary.gate_verdict_facts
        || stale.note != ordinary.note
        || stale.diagnostics != ordinary.diagnostics
        || !stale_evidence_absent(stale)
        || !matching_stale_transition(ordinary, stale)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

fn stale_evidence_absent(attempt: &crate::model::attempt::AttemptFacts) -> bool {
    match (&attempt.evidence_associations, attempt.evidence_recorded) {
        (None, None) => true,
        (Some(associations), Some(recorded)) => {
            associations.inline.is_empty()
                && associations.selected_ids.is_empty()
                && associations.provider_recorded_ids.is_empty()
                && !recorded.inline
                && !recorded.selected_associations
                && !recorded.provider
        }
        _ => false,
    }
}

fn matching_stale_transition(
    ordinary: &crate::model::attempt::AttemptFacts,
    stale: &crate::model::attempt::AttemptFacts,
) -> bool {
    match (&ordinary.transition, &stale.transition) {
        (Some(ordinary), Some(stale)) => {
            stale.event == ordinary.event
                && stale.source == ordinary.source
                && stale.target == ordinary.target
                && !stale.applied
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::capabilities::persistence_commands::{CommitEventAttemptCommand, EventAttemptParts};
    use crate::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
    use crate::model::attempt::{
        AttemptFacts, EvidenceAssociations, GateVerdictFact, GateVerdictFacts, GateVerdictResult,
        JournalExtension, ProviderFact, ProviderRole,
    };
    use crate::model::diagnostic::Diagnostic;
    use crate::model::evidence::EvidenceAssociation;
    use crate::model::graph::{State, WorkflowGraph};
    use crate::model::graph_validation::ValidatedGraph;
    use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use crate::model::ids::{
        EventId, EvidenceId, GateId, GraphRevision, ProviderHandle, RegistrationId, RequestId,
        RunId, StateId,
    };
    use crate::model::journal::JournalDraft;
    use crate::model::lifecycle::Lifecycle;
    use crate::model::outcome::OutcomeClass;
    use crate::model::provider::DigestObservation;
    use crate::model::reason::{Reason, ReasonCode};
    use crate::model::run_input::{InputDeclarations, RunInputs};
    use crate::model::time::ObservedAt;
    use crate::model::transition::Transition;

    fn opaque_command(
        run: &crate::model::run::Run,
        event: &EventId,
        target_state: Option<StateId>,
        target_lifecycle: Option<Lifecycle>,
        provider_evidence: Vec<crate::model::evidence::EvidenceRecord>,
    ) -> CommitEventAttemptCommand {
        let evidence = crate::model::attempt::EvidenceAssociations::default();
        let attempt = crate::model::attempt::AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    target_state.clone(),
                    false,
                )
                .unwrap(),
            ),
            evidence_associations: Some(evidence.clone()),
            evidence_recorded: Some(evidence.recorded_status()),
            ..crate::model::attempt::AttemptFacts::default()
        };
        CommitEventAttemptCommand::from_parts(EventAttemptParts {
            run_id: run.id().clone(),
            expected_workflow_version: run.workflow_state_version(),
            expected_lifecycle_version: run.lifecycle_version(),
            source_state: run.current_state().clone(),
            target_state,
            target_lifecycle,
            inline_evidence: Vec::new(),
            associations: Vec::new(),
            provider_evidence,
            journal_entry: crate::operations::test_support::draft(
                "run.request",
                OutcomeClass::Rejected,
                JournalExtension::TransitionAttempt,
                Some(attempt.clone()),
            ),
            stale_journal_entry: crate::operations::test_support::draft(
                "run.request",
                OutcomeClass::Error,
                JournalExtension::TransitionAttempt,
                Some(attempt),
            ),
        })
    }

    fn request_draft(
        outcome: OutcomeClass,
        reason: ReasonCode,
        attempt: AttemptFacts,
    ) -> JournalDraft {
        JournalDraft::new(
            crate::model::ids::RunId::parse("run-1").unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.request",
            RequestId::parse("request-1").unwrap(),
            outcome,
            Some(Reason::new(reason, "test disposition").unwrap()),
            Some(attempt),
            JournalExtension::TransitionAttempt,
        )
        .unwrap()
    }

    fn gated_run() -> crate::model::run::Run {
        let ready = StateId::parse("ready").unwrap();
        let done = StateId::parse("done").unwrap();
        let graph = WorkflowGraph::new_unvalidated(
            ready.clone(),
            vec![
                State::new(ready.clone(), false, StaticGuidance::NoneRequired, None),
                State::new(done.clone(), true, StaticGuidance::NoneRequired, None),
            ],
            vec![
                Transition::new(
                    ready,
                    EventId::parse("advance").unwrap(),
                    done,
                    vec![
                        GateId::parse("first-gate").unwrap(),
                        GateId::parse("second-gate").unwrap(),
                    ],
                    None,
                )
                .unwrap(),
            ],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        crate::model::run::Run::create(
            RunId::parse("run-1").unwrap(),
            RegistrationId::parse("registration-1").unwrap(),
            ValidatedGraph::validate(graph).unwrap(),
            GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            RunInputs::default(),
            None,
        )
        .unwrap()
    }

    fn gate_free_run() -> crate::model::run::Run {
        let ready = StateId::parse("ready").unwrap();
        let done = StateId::parse("done").unwrap();
        let graph = WorkflowGraph::new_unvalidated(
            ready.clone(),
            vec![
                State::new(ready.clone(), false, StaticGuidance::NoneRequired, None),
                State::new(done.clone(), true, StaticGuidance::NoneRequired, None),
            ],
            vec![
                Transition::new(
                    ready,
                    EventId::parse("advance").unwrap(),
                    done,
                    vec![],
                    None,
                )
                .unwrap(),
            ],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        crate::model::run::Run::create(
            RunId::parse("run-1").unwrap(),
            RegistrationId::parse("registration-1").unwrap(),
            ValidatedGraph::validate(graph).unwrap(),
            GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            RunInputs::default(),
            None,
        )
        .unwrap()
    }

    fn provider_config(run: &crate::model::run::Run) -> ResolvedProviderConfig {
        ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap()
    }

    fn provider_fact(run: &crate::model::run::Run) -> ProviderFact {
        ProviderFact::new(
            run.registration_id().clone(),
            1,
            ProviderRole::EvaluateGates,
            RequestId::parse("provider-call-1").unwrap(),
            "/provider",
            OutcomeClass::Completed,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap()
    }

    #[test]
    fn authoritative_builder_accepts_gate_free_completion() {
        let run = gate_free_run();
        let event = EventId::parse("advance").unwrap();
        let decision = crate::model::decision::resolve_gate_free(&run, &event).unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::Completed {
                decision: &decision,
                provider_binding: None,
            };

        assert!(
            super::command(
                &run,
                &event,
                &[],
                &[],
                None,
                &resolution,
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                RequestId::parse("request-1").unwrap(),
            )
            .is_ok()
        );
    }

    #[test]
    fn selected_evidence_ids_must_be_unique() {
        let id = EvidenceId::parse("evidence-1").unwrap();
        assert!(super::validate_evidence_selection(std::slice::from_ref(
            &id
        )));
        assert!(!super::validate_evidence_selection(&[id.clone(), id]));
    }

    #[test]
    fn rejected_resolution_cannot_commit_fabricated_transition() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let error = crate::model::decision::DecisionError::UnknownEvent(event.clone());
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            };
        let command = opaque_command(
            &run,
            &event,
            Some(StateId::parse("fabricated").unwrap()),
            Some(Lifecycle::Final),
            Vec::new(),
        );

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn rejected_resolution_requires_authoritative_reason() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let error = crate::model::decision::DecisionError::UnknownEvent(event.clone());
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            };
        let command = opaque_command(&run, &event, None, None, Vec::new());

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn provider_evidence_must_bind_its_canonical_gate_scope() {
        let event = EventId::parse("advance").unwrap();
        let first = GateId::parse("first-gate").unwrap();
        let second = GateId::parse("second-gate").unwrap();
        let evidence = crate::operations::test_support::evidence();
        let provider_scopes =
            std::collections::BTreeMap::from([(evidence.id().clone(), first.clone())]);
        let wrong = EvidenceAssociation::new(
            evidence.id().clone(),
            Some(event.clone()),
            Some(second.clone()),
        );
        let right = EvidenceAssociation::new(
            evidence.id().clone(),
            Some(event.clone()),
            Some(first.clone()),
        );

        assert!(!super::association_context_valid(
            &[wrong],
            &event,
            &[first.clone(), second.clone()],
            &provider_scopes,
        ));
        assert!(super::association_context_valid(
            &[right],
            &event,
            &[first.clone(), second.clone()],
            &provider_scopes,
        ));

        let caller_scoped = EvidenceAssociation::new(
            EvidenceId::parse("caller-evidence").unwrap(),
            Some(event.clone()),
            Some(second.clone()),
        );
        let caller_unscoped = EvidenceAssociation::new(
            EvidenceId::parse("caller-evidence").unwrap(),
            Some(event.clone()),
            None,
        );
        assert!(!super::association_context_valid(
            &[caller_scoped],
            &event,
            &[first.clone(), second.clone()],
            &std::collections::BTreeMap::new(),
        ));
        assert!(super::association_context_valid(
            &[caller_unscoped],
            &event,
            &[first, second],
            &std::collections::BTreeMap::new(),
        ));
    }

    #[test]
    fn invalid_provider_fact_remains_journalable_exactly() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let fact = ProviderFact::new(
            RegistrationId::parse("registration-1").unwrap(),
            1,
            ProviderRole::Describe,
            RequestId::parse("provider-call-1").unwrap(),
            "/provider",
            OutcomeClass::Completed,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap();
        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    None,
                    false,
                )
                .unwrap(),
            ),
            provider_observations: vec![fact.clone()],
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            diagnostics: super::invalid_provider_fact_diagnostics(),
            ..AttemptFacts::default()
        };
        let mut fabricated = attempt.clone();
        fabricated.diagnostics =
            vec![Diagnostic::new("fabricated", "not produced by core validation", None).unwrap()];
        assert!(
            super::invalid_provider_fact_command(
                &run,
                &event,
                &fact,
                None,
                Vec::new(),
                Vec::new(),
                request_draft(
                    OutcomeClass::Error,
                    ReasonCode::ProviderProtocolMalformed,
                    fabricated.clone(),
                ),
                request_draft(
                    OutcomeClass::Error,
                    ReasonCode::StateStaleVersion,
                    fabricated,
                ),
            )
            .is_err()
        );
        let ordinary = request_draft(
            OutcomeClass::Error,
            ReasonCode::ProviderProtocolMalformed,
            attempt.clone(),
        );
        let stale = request_draft(OutcomeClass::Error, ReasonCode::StateStaleVersion, attempt);
        let command = super::invalid_provider_fact_command(
            &run,
            &event,
            &fact,
            None,
            Vec::new(),
            Vec::new(),
            ordinary,
            stale,
        )
        .expect("invalid observed fact must remain durable");
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::ProviderFactInvalid {
                fact: &fact,
                config: &config,
            };

        assert!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command,)
                .is_ok()
        );
    }

    #[test]
    fn stale_attempt_must_mirror_ordinary_note() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let command = opaque_command(&run, &event, None, None, Vec::new());
        let mut parts = command.into_parts();
        let mut stale_attempt = parts
            .stale_journal_entry
            .attempt()
            .expect("stale attempt")
            .clone();
        stale_attempt.note = Some(crate::model::annotation::Note::new("substituted note").unwrap());
        parts.stale_journal_entry = crate::operations::test_support::draft(
            "run.request",
            OutcomeClass::Error,
            JournalExtension::TransitionAttempt,
            Some(stale_attempt),
        );
        let command = CommitEventAttemptCommand::from_parts(parts);
        let error = crate::model::decision::DecisionError::Terminal;
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            };

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn request_note_must_match_authoritative_input() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let command = opaque_command(&run, &event, None, None, Vec::new());
        let mut parts = command.into_parts();
        let forged = crate::model::annotation::Note::new("forged note").unwrap();
        for draft in [&mut parts.journal_entry, &mut parts.stale_journal_entry] {
            let mut attempt = draft.attempt().expect("request attempt").clone();
            attempt.note = Some(forged.clone());
            *draft = crate::operations::test_support::draft(
                "run.request",
                draft.outcome(),
                JournalExtension::TransitionAttempt,
                Some(attempt),
            );
        }
        let command = CommitEventAttemptCommand::from_parts(parts);
        let error = crate::model::decision::DecisionError::Terminal;
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            };

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn rejected_resolution_cannot_persist_invented_provider_evidence() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let error = crate::model::decision::DecisionError::UnknownEvent(event.clone());
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: None,
            };
        let command = opaque_command(
            &run,
            &event,
            None,
            None,
            vec![crate::operations::test_support::evidence()],
        );

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn duplicate_or_colliding_request_evidence_is_rejected_without_claiming_it() {
        let run = crate::operations::test_support::run();
        let event = EventId::parse("advance").unwrap();
        let evidence = crate::operations::test_support::evidence();
        let duplicate = vec![evidence.clone(), evidence];
        assert!(!super::evidence_ids_are_unique(&duplicate));

        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    None,
                    false,
                )
                .unwrap(),
            ),
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            ..AttemptFacts::default()
        };
        let ordinary = request_draft(
            OutcomeClass::Rejected,
            ReasonCode::EvidenceInvalid,
            attempt.clone(),
        );
        let stale = request_draft(OutcomeClass::Error, ReasonCode::StateStaleVersion, attempt);
        let command = super::rejected_command(
            &run,
            &event,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            ordinary,
            stale,
        )
        .unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::InputEvidenceInvalid;

        assert!(
            super::revalidate_request_command(
                &run,
                &event,
                &[],
                &duplicate,
                None,
                &resolution,
                command,
            )
            .is_ok()
        );
    }

    #[test]
    fn invocation_error_requires_authoritative_reason_and_diagnostics() {
        let run = gated_run();
        let event = EventId::parse("advance").unwrap();
        let config = provider_config(&run);
        let fact = ProviderFact::new(
            run.registration_id().clone(),
            1,
            ProviderRole::EvaluateGates,
            RequestId::parse("provider-call-1").unwrap(),
            "/provider",
            OutcomeClass::Error,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap();
        let diagnostic = Diagnostic::new(
            "provider.evidence.malformed",
            "malformed provider evidence",
            None,
        )
        .unwrap();
        let error = crate::capabilities::provider_invoker::InvocationError::Transport {
            source: (),
            fact: Box::new(fact.clone()),
            failure: Box::new(crate::capabilities::provider_invoker::InvocationFailure {
                reason: Reason::new(
                    ReasonCode::ProviderEvidenceMalformed,
                    "malformed provider evidence",
                )
                .unwrap(),
                diagnostics: vec![diagnostic.clone()],
            }),
            trace_failure: None,
        };
        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(StateId::parse("done").unwrap()),
                    false,
                )
                .unwrap(),
            ),
            provider_observations: vec![fact.clone()],
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            diagnostics: vec![diagnostic],
            ..AttemptFacts::default()
        };
        let ordinary = request_draft(
            OutcomeClass::Error,
            ReasonCode::ProviderProtocolMalformed,
            attempt.clone(),
        );
        let stale = request_draft(OutcomeClass::Error, ReasonCode::StateStaleVersion, attempt);
        let command = super::rejected_command(
            &run,
            &event,
            Some((&fact, &config, OutcomeClass::Error)),
            Some(StateId::parse("done").unwrap()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            ordinary,
            stale,
        )
        .unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::InvocationError {
                error: &error,
                config: &config,
            };

        assert!(matches!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn completed_gate_facts_reject_duplicate_verdict_ids() {
        let run = gated_run();
        let event = EventId::parse("advance").unwrap();
        let first = GateId::parse("first-gate").unwrap();
        let second = GateId::parse("second-gate").unwrap();
        let evaluation = crate::model::gate::GateEvaluation::verdicts(vec![
            crate::model::gate::GateVerdict::new(first.clone(), true, Vec::new()),
            crate::model::gate::GateVerdict::new(second.clone(), true, Vec::new()),
        ]);
        let decision = crate::model::decision::resolve_gated(&run, &event, &evaluation).unwrap();
        let malformed = GateVerdictFacts {
            event,
            gate_ids: vec![first.clone(), second],
            result: GateVerdictResult::Verdicts(vec![
                GateVerdictFact::new(first.clone(), true, None).unwrap(),
                GateVerdictFact::new(first, true, None).unwrap(),
            ]),
        };

        let attempt = AttemptFacts {
            gate_verdict_facts: Some(malformed),
            ..AttemptFacts::default()
        };

        assert!(!super::completed_gate_facts_match(&decision, &attempt));
    }

    #[test]
    fn completed_request_cannot_add_fabricated_diagnostics() {
        let run = gated_run();
        let event = EventId::parse("advance").unwrap();
        let first = GateId::parse("first-gate").unwrap();
        let second = GateId::parse("second-gate").unwrap();
        let evaluation = crate::model::gate::GateEvaluation::verdicts(vec![
            crate::model::gate::GateVerdict::new(first.clone(), true, Vec::new()),
            crate::model::gate::GateVerdict::new(second.clone(), true, Vec::new()),
        ]);
        let decision = crate::model::decision::resolve_gated(&run, &event, &evaluation).unwrap();
        let fact = provider_fact(&run);
        let config = provider_config(&run);
        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(StateId::parse("done").unwrap()),
                    true,
                )
                .unwrap(),
            ),
            provider_observations: vec![fact.clone()],
            gate_verdict_facts: Some(GateVerdictFacts {
                event: event.clone(),
                gate_ids: vec![first.clone(), second.clone()],
                result: GateVerdictResult::Verdicts(vec![
                    GateVerdictFact::new(first, true, None).unwrap(),
                    GateVerdictFact::new(second, true, None).unwrap(),
                ]),
            }),
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            diagnostics: vec![
                Diagnostic::new("fabricated", "not produced by provider", None).unwrap(),
            ],
            ..AttemptFacts::default()
        };
        let ordinary = crate::operations::test_support::draft(
            "run.request",
            OutcomeClass::Completed,
            JournalExtension::TransitionAttempt,
            Some(attempt.clone()),
        );
        let mut stale_attempt = attempt;
        stale_attempt.transition.as_mut().unwrap().applied = false;
        let stale = request_draft(
            OutcomeClass::Error,
            ReasonCode::StateStaleVersion,
            stale_attempt,
        );

        assert!(
            super::completed_command(
                &decision,
                Some((&fact, &config, OutcomeClass::Completed)),
                Vec::new(),
                Vec::new(),
                ordinary,
                stale,
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_verdict_set_is_journalable_without_invalid_gate_facts() {
        let run = gated_run();
        let event = EventId::parse("advance").unwrap();
        let first = GateId::parse("first-gate").unwrap();
        let evaluation = crate::model::gate::GateEvaluation::verdicts(vec![
            crate::model::gate::GateVerdict::new(first.clone(), true, Vec::new()),
            crate::model::gate::GateVerdict::new(first, true, Vec::new()),
        ]);
        let error = crate::model::decision::resolve_gated(&run, &event, &evaluation).unwrap_err();
        let crate::model::decision::DecisionError::MalformedVerdicts { source, .. } = &error else {
            panic!("expected malformed verdict set");
        };
        let fact = provider_fact(&run);
        let config = provider_config(&run);
        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(StateId::parse("done").unwrap()),
                    false,
                )
                .unwrap(),
            ),
            provider_observations: vec![fact.clone()],
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            diagnostics: super::malformed_verdict_diagnostics(source),
            ..AttemptFacts::default()
        };
        let ordinary = request_draft(
            OutcomeClass::Error,
            ReasonCode::ProviderProtocolMalformed,
            attempt.clone(),
        );
        let stale = request_draft(OutcomeClass::Error, ReasonCode::StateStaleVersion, attempt);
        let command = super::rejected_command(
            &run,
            &event,
            Some((&fact, &config, OutcomeClass::Completed)),
            Some(StateId::parse("done").unwrap()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            ordinary,
            stale,
        )
        .unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::DecisionRejected {
                error: &error,
                provider_binding: Some((&fact, &config, OutcomeClass::Completed)),
            };

        assert!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command,)
                .is_ok()
        );
    }

    #[test]
    fn invalid_provider_evidence_is_journalable_without_persisting_it() {
        let run = gated_run();
        let event = EventId::parse("advance").unwrap();
        let fact = provider_fact(&run);
        let config = provider_config(&run);
        let evidence = crate::operations::test_support::evidence();
        let evaluation = crate::model::gate::GateEvaluation::verdicts(vec![
            crate::model::gate::GateVerdict::new(
                GateId::parse("first-gate").unwrap(),
                true,
                vec![evidence.clone()],
            ),
            crate::model::gate::GateVerdict::new(
                GateId::parse("second-gate").unwrap(),
                true,
                vec![evidence],
            ),
        ]);
        assert!(!super::provider_evidence_is_valid(&evaluation, &[], &[]));

        let associations = EvidenceAssociations::default();
        let attempt = AttemptFacts {
            transition: Some(
                crate::model::attempt::TransitionFact::new(
                    event.clone(),
                    run.current_state().clone(),
                    Some(StateId::parse("done").unwrap()),
                    false,
                )
                .unwrap(),
            ),
            provider_observations: vec![fact.clone()],
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            diagnostics: super::invalid_provider_evidence_diagnostics(),
            ..AttemptFacts::default()
        };
        let mut fabricated = attempt.clone();
        fabricated.diagnostics =
            vec![Diagnostic::new("fabricated", "not produced by core validation", None).unwrap()];
        assert!(
            super::invalid_provider_evidence_command(
                &run,
                &event,
                &fact,
                &config,
                Some(StateId::parse("done").unwrap()),
                Vec::new(),
                Vec::new(),
                request_draft(
                    OutcomeClass::Error,
                    ReasonCode::ProviderEvidenceMalformed,
                    fabricated.clone(),
                ),
                request_draft(
                    OutcomeClass::Error,
                    ReasonCode::StateStaleVersion,
                    fabricated,
                ),
            )
            .is_err()
        );
        let ordinary = request_draft(
            OutcomeClass::Error,
            ReasonCode::ProviderEvidenceMalformed,
            attempt.clone(),
        );
        let stale = request_draft(OutcomeClass::Error, ReasonCode::StateStaleVersion, attempt);
        let command = super::invalid_provider_evidence_command(
            &run,
            &event,
            &fact,
            &config,
            Some(StateId::parse("done").unwrap()),
            Vec::new(),
            Vec::new(),
            ordinary,
            stale,
        )
        .unwrap();
        let resolution: super::RequestResolution<'_, (), (), (), ()> =
            super::RequestResolution::ProviderEvidenceInvalid {
                fact: &fact,
                config: &config,
            };

        assert!(
            super::revalidate_request_command(&run, &event, &[], &[], None, &resolution, command,)
                .is_ok()
        );
    }
}
