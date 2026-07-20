use std::collections::BTreeSet;

use crate::capabilities::event_attempt_writer::EventAttemptWriter;
use crate::capabilities::persistence_commands::{
    CommitEventAttemptCommand, CommitStatus, EventAttemptParts, EventCommitBranch,
};
use crate::capabilities::provider_catalog::{ProviderCatalog, ResolvedProviderConfig};
use crate::capabilities::provider_invoker::{GateRequest, InvocationError, ProviderInvoker};
use crate::capabilities::run_reader::{RunReader, SelectedEvidenceReadError};
use crate::model::attempt::{ProviderFact, ProviderRole};
use crate::model::decision::{DecisionError, TransitionDecision, resolve_gate_free, resolve_gated};
use crate::model::evidence::{EvidenceAssociation, EvidenceRecord};
use crate::model::gate::GateEvaluation;
use crate::model::ids::{EventId, EvidenceId, RunId, StateId};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
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
    ProviderFactInvalid(&'a ProviderFact),
    SelectionInvalid,
    SelectionUnavailable,
    SelectionReadError(&'a R),
    CatalogError(&'a C),
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
    Writer(W),
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
    mut gate_request: G,
    mut command: F,
) -> Result<RequestExecution, RequestExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
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
        RequestResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<CommitEventAttemptCommand, J>,
{
    let run = reader.get(run_id).map_err(RequestExecutionError::Lookup)?;
    if !validate_evidence_selection(selected_ids) {
        return commit_resolution(
            writer,
            &run,
            &[],
            RequestResolution::SelectionInvalid,
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
                    &[],
                    RequestResolution::SelectionUnavailable,
                    RequestExecutionDisposition::Rejected,
                    &mut command,
                );
            }
            Err(SelectedEvidenceReadError::Read(error)) => {
                return commit_resolution(
                    writer,
                    &run,
                    &[],
                    RequestResolution::SelectionReadError(&error),
                    RequestExecutionDisposition::Error,
                    &mut command,
                );
            }
        }
    };
    match resolve_gate_free(&run, event) {
        Ok(decision) => commit_resolution(
            writer,
            &run,
            &selected,
            RequestResolution::Completed {
                decision: &decision,
                provider_binding: None,
            },
            RequestExecutionDisposition::Completed,
            &mut command,
        ),
        Err(DecisionError::GatesRequired) => {
            let config = match catalog.resolve_enabled(run.registration_id()) {
                Ok(config) => config,
                Err(error) => {
                    return commit_resolution(
                        writer,
                        &run,
                        &selected,
                        RequestResolution::CatalogError(&error),
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
                        &selected,
                        RequestResolution::GateRequestError(&error),
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
                    ) && result.fact.outcome == gate_provider_outcome(&result.evaluation) =>
                {
                    match resolve_gated(&run, event, &result.evaluation) {
                        Ok(decision) => commit_resolution(
                            writer,
                            &run,
                            &selected,
                            RequestResolution::Completed {
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
                            &selected,
                            RequestResolution::DecisionRejected {
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
                Ok(result) => commit_resolution(
                    writer,
                    &run,
                    &selected,
                    RequestResolution::ProviderFactInvalid(&result.fact),
                    RequestExecutionDisposition::Error,
                    &mut command,
                ),
                Err(error) => commit_resolution(
                    writer,
                    &run,
                    &selected,
                    RequestResolution::InvocationError {
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
            &selected,
            RequestResolution::DecisionRejected {
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
        | DecisionError::Mutation(_) => RequestExecutionDisposition::Error,
        _ => RequestExecutionDisposition::Rejected,
    }
}

fn commit_resolution<R, C, I, Q, W, F, J>(
    writer: &W,
    run: &Run,
    selected: &[EvidenceRecord],
    resolution: RequestResolution<'_, C, R, I, Q>,
    disposition: RequestExecutionDisposition,
    command: &mut F,
) -> Result<RequestExecution, RequestExecutionError<R, J, W::Error>>
where
    W: EventAttemptWriter,
    F: for<'a> FnMut(
        &Run,
        &[EvidenceRecord],
        RequestResolution<'a, C, R, I, Q>,
    ) -> Result<CommitEventAttemptCommand, J>,
{
    let command = command(run, selected, resolution).map_err(RequestExecutionError::Command)?;
    let status = writer
        .commit_event_attempt(command)
        .map_err(RequestExecutionError::Writer)?;
    let disposition = match status.branch {
        EventCommitBranch::ExpectedVersions => disposition,
        EventCommitBranch::StaleVersions => RequestExecutionDisposition::Error,
    };
    Ok(RequestExecution {
        disposition,
        commit: status.commit,
    })
}

pub fn validate_evidence_selection(ids: &[EvidenceId]) -> bool {
    ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

pub fn resolve(
    run: &Run,
    event: &EventId,
    gate_evaluation: Option<&GateEvaluation>,
) -> Result<TransitionDecision, DecisionError> {
    match gate_evaluation {
        Some(evaluation) => resolve_gated(run, event, evaluation),
        None => resolve_gate_free(run, event),
    }
}

pub fn completed_command(
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
        || !transition_matches
        || !evidence_matches(
            attempt,
            &inline_evidence,
            &associations,
            decision.provider_evidence(),
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

#[allow(clippy::too_many_arguments)]
pub fn rejected_command(
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
        || !evidence_matches(attempt, &inline_evidence, &associations, &provider_evidence)
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

fn evidence_matches(
    attempt: &crate::model::attempt::AttemptFacts,
    inline: &[EvidenceRecord],
    associations: &[EvidenceAssociation],
    provider: &[EvidenceRecord],
) -> bool {
    let Some(recorded) = &attempt.evidence_associations else {
        return false;
    };
    if recorded.inline != inline {
        return false;
    }
    let expected = recorded
        .inline
        .iter()
        .map(EvidenceRecord::id)
        .chain(recorded.selected_ids.iter())
        .chain(recorded.provider_recorded_ids.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual = associations
        .iter()
        .map(EvidenceAssociation::evidence_id)
        .cloned()
        .collect::<BTreeSet<_>>();
    let provider_ids = provider
        .iter()
        .map(EvidenceRecord::id)
        .collect::<BTreeSet<_>>();
    expected == actual
        && provider_ids
            == recorded
                .provider_recorded_ids
                .iter()
                .collect::<BTreeSet<_>>()
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
        || stale.provider_observations != ordinary.provider_observations
        || stale.gate_verdict_facts != ordinary.gate_verdict_facts
        || stale.evidence_associations != ordinary.evidence_associations
        || stale.evidence_recorded != ordinary.evidence_recorded
        || !matching_stale_transition(ordinary, stale)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
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
    use crate::model::ids::EvidenceId;

    #[test]
    fn selected_evidence_ids_must_be_unique() {
        let id = EvidenceId::parse("evidence-1").unwrap();
        assert!(super::validate_evidence_selection(std::slice::from_ref(
            &id
        )));
        assert!(!super::validate_evidence_selection(&[id.clone(), id]));
    }
}
