use crate::capabilities::persistence_commands::{AppendCompatibilityAttemptCommand, AttemptCommit};
use crate::capabilities::provider_catalog::{
    ProviderCatalog, ProviderResolveFailure, ResolvedProviderConfig,
};
use crate::capabilities::provider_invoker::{
    CompatibilityRequest, CompatibilityResult, InvocationError, ProviderInvoker,
};
use crate::capabilities::run_reader::RunReader;
use crate::capabilities::run_writer::CompatibilityAttemptWriter;
use crate::model::attempt::{AttemptFacts, JournalExtension, ProviderFact, ProviderRole};
use crate::model::ids::RunId;
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::lifecycle::Lifecycle;
use crate::model::outcome::OutcomeClass;
use crate::model::provider::{DigestObservation, ProviderObservation};
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

pub enum CompatibilityResolution<'a, C, R, I, Q> {
    Terminal,
    CreationObservationError(&'a R),
    CatalogError(&'a C, ProviderResolveFailure),
    RequestError(&'a Q),
    Provider(Result<&'a CompatibilityResult, &'a InvocationError<I>>),
}

#[derive(Debug)]
pub enum CompatibilityExecutionError<R, J, W> {
    Lookup(R),
    Command(J),
    InvalidCommand(CommandError),
    Writer(W),
}

#[allow(clippy::too_many_arguments)]
pub fn execute<R, C, I, W, G, F, Q, J>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    run_id: &RunId,
    mut request: G,
    mut command: F,
) -> Result<AttemptCommit, CompatibilityExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: CompatibilityAttemptWriter,
    G: FnMut(&ResolvedProviderConfig, &Run) -> Result<CompatibilityRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        Option<&ProviderObservation>,
        &CompatibilityResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<AppendCompatibilityAttemptCommand, J>,
{
    let run = reader
        .get(run_id)
        .map_err(CompatibilityExecutionError::Lookup)?;
    if !can_invoke(&run) {
        return persist_resolution(
            writer,
            &run,
            None,
            None,
            CompatibilityResolution::Terminal,
            &mut command,
        );
    }
    let creation = match reader.creation_provider_observation(run_id) {
        Ok(observation) => observation,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                None,
                None,
                CompatibilityResolution::CreationObservationError(&error),
                &mut command,
            );
        }
    };
    let config = match catalog.resolve_enabled("run.compatibility", run.registration_id()) {
        Ok(config) => config,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                None,
                Some(&creation),
                CompatibilityResolution::CatalogError(&error, C::classify_resolve_failure(&error)),
                &mut command,
            );
        }
    };
    let provider_request = match request(&config, &run) {
        Ok(request) => request,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                Some(&config),
                Some(&creation),
                CompatibilityResolution::RequestError(&error),
                &mut command,
            );
        }
    };
    let result = invoker.check_compatibility(&config, provider_request);
    persist_resolution(
        writer,
        &run,
        Some(&config),
        Some(&creation),
        CompatibilityResolution::Provider(result.as_ref()),
        &mut command,
    )
}

fn persist_resolution<R, C, I, Q, W, F, J>(
    writer: &W,
    run: &Run,
    config: Option<&ResolvedProviderConfig>,
    creation: Option<&ProviderObservation>,
    resolution: CompatibilityResolution<'_, C, R, I, Q>,
    command: &mut F,
) -> Result<AttemptCommit, CompatibilityExecutionError<R, J, W::Error>>
where
    W: CompatibilityAttemptWriter,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        Option<&ProviderObservation>,
        &CompatibilityResolution<'a, C, R, I, Q>,
    ) -> Result<AppendCompatibilityAttemptCommand, J>,
{
    let command = command(run, config, creation, &resolution)
        .map_err(CompatibilityExecutionError::Command)?;
    revalidate_compatibility_command(run, config, creation, &resolution, &command)
        .map_err(CompatibilityExecutionError::InvalidCommand)?;
    writer
        .append_compatibility_attempt(command)
        .map_err(CompatibilityExecutionError::Writer)
}

fn revalidate_compatibility_command<C, R, I, Q>(
    run: &Run,
    config: Option<&ResolvedProviderConfig>,
    creation: Option<&ProviderObservation>,
    resolution: &CompatibilityResolution<'_, C, R, I, Q>,
    attempt_command: &AppendCompatibilityAttemptCommand,
) -> Result<(), CommandError> {
    let expected = command_for_resolution(
        run,
        creation,
        resolution,
        attempt_command.journal_entry().observed_at(),
        attempt_command.journal_entry().request_id().clone(),
    )?;
    if expected != *attempt_command {
        return Err(CommandError::JournalMismatch);
    }
    match resolution {
        CompatibilityResolution::Provider(result) => {
            let config = config.ok_or(CommandError::JournalMismatch)?;
            let creation = creation.ok_or(CommandError::JournalMismatch)?;
            let expected = command(
                run,
                config,
                creation.digest(),
                *result,
                attempt_command.journal_entry().clone(),
                attempt_command.terminal_rejection_entry().clone(),
            )?;
            if expected != *attempt_command {
                return Err(CommandError::JournalMismatch);
            }
            Ok(())
        }
        CompatibilityResolution::Terminal => {
            let expected = local_rejection_command(
                run,
                attempt_command.journal_entry().clone(),
                attempt_command.terminal_rejection_entry().clone(),
            )?;
            if expected != *attempt_command {
                return Err(CommandError::JournalMismatch);
            }
            Ok(())
        }
        CompatibilityResolution::CreationObservationError(_) => {
            revalidate_compatibility_pre_invocation(
                run,
                ReasonCode::PersistenceFailed,
                attempt_command,
            )
        }
        CompatibilityResolution::CatalogError(_, reason) => {
            revalidate_compatibility_pre_invocation(run, reason.reason_code(), attempt_command)
        }
        CompatibilityResolution::RequestError(_) => revalidate_compatibility_pre_invocation(
            run,
            ReasonCode::ResourceExhausted,
            attempt_command,
        ),
    }
}

fn revalidate_compatibility_pre_invocation(
    run: &Run,
    expected_reason: ReasonCode,
    attempt_command: &AppendCompatibilityAttemptCommand,
) -> Result<(), CommandError> {
    validate_pair(
        run,
        attempt_command.journal_entry(),
        attempt_command.terminal_rejection_entry(),
    )?;
    let expected_stale = stale_attempt_entry(
        attempt_command.terminal_rejection_entry(),
        "run state changed before compatibility committed",
    )?;
    let valid = attempt_command.run_id() == run.id()
        && attempt_command.expected_workflow_version() == run.workflow_state_version()
        && attempt_command.expected_lifecycle_version() == run.lifecycle_version()
        && attempt_command.stale_error_entry() == &expected_stale
        && attempt_command.observed_drift().is_none()
        && attempt_command.journal_entry().outcome() == OutcomeClass::Error
        && attempt_command
            .journal_entry()
            .reason()
            .map(|reason| reason.code())
            == Some(expected_reason)
        && matches!(
            attempt_command.journal_entry().extension(),
            JournalExtension::CompatibilityAttempt { findings: None }
        )
        && attempt_command
            .journal_entry()
            .attempt()
            .is_some_and(|attempt| {
                attempt.provider_observations.is_empty() && attempt.diagnostics.is_empty()
            });
    if !valid {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

/// Builds the authoritative persistence command for an observed compatibility resolution.
/// Delivery supplies correlation/time only; outcome policy and journal facts remain core-owned.
pub fn command_for_resolution<C, R, I, Q>(
    run: &Run,
    creation: Option<&ProviderObservation>,
    resolution: &CompatibilityResolution<'_, C, R, I, Q>,
    observed_at: crate::model::time::ObservedAt,
    request_id: crate::model::ids::RequestId,
) -> Result<AppendCompatibilityAttemptCommand, CommandError> {
    use crate::model::compatibility::CompatibilityReport;
    use crate::model::reason::Reason;

    let mut attempt = AttemptFacts::default();
    let mut drift = None;
    let (outcome, reason, findings) = match resolution {
        CompatibilityResolution::Terminal => (
            OutcomeClass::Rejected,
            Some(Reason::new(
                ReasonCode::RunLifecycleTerminal,
                "run lifecycle is terminal",
            )?),
            None,
        ),
        CompatibilityResolution::CreationObservationError(_) => (
            OutcomeClass::Error,
            Some(Reason::new(
                ReasonCode::PersistenceFailed,
                "creation provider observation is unavailable",
            )?),
            None,
        ),
        CompatibilityResolution::CatalogError(_, failure) => (
            OutcomeClass::Error,
            Some(Reason::new(
                failure.reason_code(),
                "provider registration is unavailable",
            )?),
            None,
        ),
        CompatibilityResolution::RequestError(_) => (
            OutcomeClass::Error,
            Some(Reason::new(
                ReasonCode::ResourceExhausted,
                "compatibility request exceeds bounds",
            )?),
            None,
        ),
        CompatibilityResolution::Provider(Ok(result)) => {
            attempt.provider_observations.push(result.fact.clone());
            drift = creation.and_then(|creation| {
                observed_drift(creation.digest(), result.observation.digest())
            });
            match &result.report {
                CompatibilityReport::Findings(findings) => {
                    (OutcomeClass::Completed, None, Some(findings.clone()))
                }
                CompatibilityReport::EvaluationError(diagnostics) => {
                    attempt.diagnostics = diagnostics.as_slice().to_vec();
                    (
                        OutcomeClass::Error,
                        Some(Reason::new(
                            ReasonCode::ProviderEvaluationError,
                            "provider compatibility evaluation failed",
                        )?),
                        None,
                    )
                }
            }
        }
        CompatibilityResolution::Provider(Err(InvocationError::TraceBudgetUnavailable)) => (
            OutcomeClass::Error,
            Some(Reason::new(
                ReasonCode::ResourceExhausted,
                "provider trace budget unavailable",
            )?),
            None,
        ),
        CompatibilityResolution::Provider(Err(InvocationError::Transport {
            fact,
            failure,
            ..
        })) => {
            attempt.provider_observations.push(*fact.clone());
            attempt.diagnostics = failure.diagnostics.clone();
            (OutcomeClass::Error, Some(failure.reason.clone()), None)
        }
    };
    let ordinary = JournalDraft::new(
        run.id().clone(),
        observed_at,
        "run.compatibility",
        request_id.clone(),
        outcome,
        reason,
        Some(attempt.clone()),
        JournalExtension::CompatibilityAttempt { findings },
    )?;
    let terminal = JournalDraft::new(
        run.id().clone(),
        observed_at,
        "run.compatibility",
        request_id.clone(),
        OutcomeClass::Rejected,
        Some(Reason::new(
            ReasonCode::RunLifecycleTerminal,
            "run lifecycle changed before compatibility committed",
        )?),
        Some(attempt.clone()),
        JournalExtension::CompatibilityAttempt { findings: None },
    )?;
    let stale = JournalDraft::new(
        run.id().clone(),
        observed_at,
        "run.compatibility",
        request_id,
        OutcomeClass::Error,
        Some(Reason::new(
            ReasonCode::StateStaleVersion,
            "run state changed before compatibility committed",
        )?),
        Some(attempt),
        JournalExtension::CompatibilityAttempt { findings: None },
    )?;
    Ok(AppendCompatibilityAttemptCommand::from_parts(
        run.id().clone(),
        run.workflow_state_version(),
        run.lifecycle_version(),
        drift,
        ordinary,
        terminal,
        stale,
    ))
}

pub fn can_invoke(run: &Run) -> bool {
    run.lifecycle() == Lifecycle::Active
}

pub fn observed_drift(creation: &DigestObservation, current: &DigestObservation) -> Option<bool> {
    match (creation, current) {
        (DigestObservation::Observed(before), DigestObservation::Observed(now)) => {
            Some(before != now)
        }
        _ => None,
    }
}

pub(crate) fn command<E>(
    run: &Run,
    config: &ResolvedProviderConfig,
    creation_digest: &DigestObservation,
    result: Result<&CompatibilityResult, &InvocationError<E>>,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendCompatibilityAttemptCommand, CommandError> {
    validate_pair(run, &journal_entry, &terminal_rejection_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let observations = &attempt.provider_observations;
    let report_matches = match (result, journal_entry.outcome(), journal_entry.extension()) {
        (
            Ok(result),
            OutcomeClass::Completed,
            JournalExtension::CompatibilityAttempt {
                findings: Some(recorded),
            },
        ) => {
            result.report
                == crate::model::compatibility::CompatibilityReport::Findings(recorded.clone())
                && journal_entry.reason().is_none()
                && attempt.diagnostics.is_empty()
                && result.fact.outcome == OutcomeClass::Completed
                && one_bound_fact(observations, config, result)
        }
        (
            Ok(result),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => {
            matches!(
                &result.report,
                crate::model::compatibility::CompatibilityReport::EvaluationError(diagnostics)
                    if attempt.diagnostics == diagnostics.as_slice()
            ) && journal_entry.reason().map(|reason| reason.code())
                == Some(ReasonCode::ProviderEvaluationError)
                && result.fact.outcome == OutcomeClass::Error
                && one_bound_fact(observations, config, result)
        }
        (
            Err(InvocationError::TraceBudgetUnavailable),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => {
            journal_entry.reason().map(|reason| reason.code())
                == Some(ReasonCode::ResourceExhausted)
                && attempt.diagnostics.is_empty()
                && observations.is_empty()
        }
        (
            Err(InvocationError::Transport { fact, failure, .. }),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => {
            journal_entry.reason() == Some(&failure.reason)
                && attempt.diagnostics == failure.diagnostics
                && fact.outcome == OutcomeClass::Error
                && one_config_fact(observations, config, fact)
        }
        _ => false,
    };
    if !report_matches {
        return Err(CommandError::JournalMismatch);
    }
    let observed_drift = result
        .ok()
        .and_then(|result| observed_drift(creation_digest, result.observation.digest()));
    let stale_error_entry = stale_attempt_entry(
        &terminal_rejection_entry,
        "run state changed before compatibility committed",
    )?;
    Ok(AppendCompatibilityAttemptCommand::from_parts(
        run.id().clone(),
        run.workflow_state_version(),
        run.lifecycle_version(),
        observed_drift,
        journal_entry,
        terminal_rejection_entry,
        stale_error_entry,
    ))
}

pub(crate) fn local_rejection_command(
    run: &Run,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendCompatibilityAttemptCommand, CommandError> {
    validate_pair(run, &journal_entry, &terminal_rejection_entry)?;
    let valid = journal_entry.outcome() == OutcomeClass::Rejected
        && matches!(
            journal_entry.extension(),
            JournalExtension::CompatibilityAttempt { findings: None }
        )
        && journal_entry.attempt().is_some_and(|attempt| {
            attempt.provider_observations.is_empty() && attempt.diagnostics.is_empty()
        });
    if !valid {
        return Err(CommandError::JournalMismatch);
    }
    let stale_error_entry = stale_attempt_entry(
        &terminal_rejection_entry,
        "run state changed before compatibility committed",
    )?;
    Ok(AppendCompatibilityAttemptCommand::from_parts(
        run.id().clone(),
        run.workflow_state_version(),
        run.lifecycle_version(),
        None,
        journal_entry,
        terminal_rejection_entry,
        stale_error_entry,
    ))
}

fn stale_attempt_entry(
    terminal_entry: &JournalDraft,
    detail: &'static str,
) -> Result<JournalDraft, CommandError> {
    Ok(JournalDraft::new(
        terminal_entry.run_id().clone(),
        terminal_entry.observed_at(),
        "run.compatibility",
        terminal_entry.request_id().clone(),
        OutcomeClass::Error,
        Some(crate::model::reason::Reason::new(
            ReasonCode::StateStaleVersion,
            detail,
        )?),
        terminal_entry.attempt().cloned(),
        terminal_entry.extension().clone(),
    )?)
}

fn validate_pair(
    run: &Run,
    journal_entry: &JournalDraft,
    terminal_rejection_entry: &JournalDraft,
) -> Result<(), CommandError> {
    for entry in [journal_entry, terminal_rejection_entry] {
        validate_journal(
            entry,
            run.id(),
            "run.compatibility",
            JournalEntryKind::CompatibilityAttempt,
        )?;
    }
    let ordinary = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let terminal = terminal_rejection_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    if terminal_rejection_entry.outcome() != OutcomeClass::Rejected
        || terminal_rejection_entry
            .reason()
            .map(|reason| reason.code())
            != Some(ReasonCode::RunLifecycleTerminal)
        || terminal != ordinary
        || !matches!(
            terminal_rejection_entry.extension(),
            JournalExtension::CompatibilityAttempt { findings: None }
        )
        || !compatibility_context_is_exact(ordinary)
        || !compatibility_context_is_exact(terminal)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

fn compatibility_context_is_exact(attempt: &AttemptFacts) -> bool {
    attempt.transition.is_none()
        && attempt.gate_verdict_facts.is_none()
        && attempt.evidence_associations.is_none()
        && attempt.evidence_recorded.is_none()
        && attempt.note.is_none()
        && attempt.actor.is_none()
        && attempt.corrects_sequence.is_none()
}

fn one_config_fact(
    facts: &[ProviderFact],
    config: &ResolvedProviderConfig,
    expected: &ProviderFact,
) -> bool {
    matches!(facts, [fact]
        if fact == expected
            && fact.role == ProviderRole::CheckCompatibility
            && fact.registration_id == *config.registration_id()
            && fact.config_revision == config.config_revision()
            && fact.executable.as_str() == config.config().executable())
}

fn one_bound_fact(
    facts: &[ProviderFact],
    config: &ResolvedProviderConfig,
    result: &CompatibilityResult,
) -> bool {
    matches!(facts, [fact]
        if fact == &result.fact
            && fact.role == ProviderRole::CheckCompatibility
            && fact.registration_id == *config.registration_id()
            && fact.config_revision == config.config_revision()
            && fact.executable.as_str() == config.config().executable()
            && fact.executable.as_str() == result.observation.locator()
            && fact.digest == *result.observation.digest()
            && fact.provider_version.as_ref().map(|value| value.as_str()) == result.observation.version()
            && fact.protocol_major == Some(u64::from(result.protocol_major)))
}

#[cfg(test)]
mod tests {
    use crate::capabilities::persistence_commands::AppendCompatibilityAttemptCommand;
    use crate::capabilities::provider_catalog::{
        ProviderConfig, ProviderResolveFailure, ResolvedProviderConfig,
    };
    use crate::capabilities::provider_invoker::{CompatibilityResult, InvocationError};
    use crate::model::attempt::{AttemptFacts, EvidenceAssociations, ProviderFact, ProviderRole};
    use crate::model::compatibility::CompatibilityReport;
    use crate::model::diagnostic::Diagnostic;
    use crate::model::ids::{ProviderHandle, RequestId};
    use crate::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
    use crate::model::provider::{DigestObservation, ProviderObservation};
    use crate::model::reason::{Reason, ReasonCode};
    use crate::model::time::ObservedAt;

    #[test]
    fn drift_is_explicit_and_unknown_when_digest_unavailable() {
        let first = DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap();
        let second = DigestObservation::observed(format!("sha256:{}", "b".repeat(64))).unwrap();
        assert_eq!(super::observed_drift(&first, &second), Some(true));
        assert_eq!(
            super::observed_drift(&first, &DigestObservation::Unavailable),
            None
        );
    }

    #[test]
    fn compatibility_attempts_must_not_claim_evidence() {
        let run = crate::operations::test_support::run();
        let ordinary = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts::default()),
        );
        let terminal = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts::default()),
        );
        assert!(super::local_rejection_command(&run, ordinary.clone(), terminal.clone()).is_ok());

        let claimed = AttemptFacts {
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        };
        let ordinary = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(claimed.clone()),
        );
        assert!(
            super::local_rejection_command(&run, ordinary, terminal_rejection_draft(claimed))
                .is_err()
        );
    }

    #[test]
    fn terminal_race_draft_must_mirror_attempt_facts() {
        let run = crate::operations::test_support::run();
        let ordinary = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts::default()),
        );
        let terminal = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(AttemptFacts {
                diagnostics: vec![
                    Diagnostic::new("fabricated", "terminal-only diagnostic", None).unwrap(),
                ],
                ..AttemptFacts::default()
            }),
        );

        assert!(super::local_rejection_command(&run, ordinary, terminal).is_err());
    }

    #[test]
    fn opaque_command_cannot_substitute_fabricated_provider_result() {
        let run = crate::operations::test_support::run();
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let digest = DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap();
        let observation = ProviderObservation::new(
            run.registration_id().clone(),
            "/provider",
            digest.clone(),
            None,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        )
        .unwrap();
        let result = compatibility_result(&config, &observation, "actual-invocation");
        let fabricated = compatibility_result(&config, &observation, "fabricated-invocation");
        let findings = match &fabricated.report {
            CompatibilityReport::Findings(findings) => findings.clone(),
            CompatibilityReport::EvaluationError(_) => unreachable!(),
        };
        let attempt = AttemptFacts {
            provider_observations: vec![fabricated.fact.clone()],
            ..AttemptFacts::default()
        };
        let ordinary = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Completed,
            super::JournalExtension::CompatibilityAttempt {
                findings: Some(findings),
            },
            Some(attempt.clone()),
        );
        let terminal = terminal_rejection_draft(attempt);
        let fabricated_command = super::command::<()>(
            &run,
            &config,
            observation.digest(),
            Ok(&fabricated),
            ordinary,
            terminal,
        )
        .unwrap();
        let provider_result: Result<&CompatibilityResult, &InvocationError<()>> = Ok(&result);
        let resolution: super::CompatibilityResolution<'_, (), (), (), ()> =
            super::CompatibilityResolution::Provider(provider_result);

        assert!(
            super::revalidate_compatibility_command(
                &run,
                Some(&config),
                Some(&observation),
                &resolution,
                &fabricated_command,
            )
            .is_err()
        );
    }

    #[test]
    fn completed_compatibility_cannot_add_fabricated_diagnostics() {
        let run = crate::operations::test_support::run();
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let digest = DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap();
        let observation = ProviderObservation::new(
            run.registration_id().clone(),
            "/provider",
            digest,
            None,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        )
        .unwrap();
        let result = compatibility_result(&config, &observation, "actual-invocation");
        let findings = match &result.report {
            CompatibilityReport::Findings(findings) => findings.clone(),
            CompatibilityReport::EvaluationError(_) => unreachable!(),
        };
        let attempt = AttemptFacts {
            provider_observations: vec![result.fact.clone()],
            diagnostics: vec![
                Diagnostic::new("fabricated", "not supplied by provider", None).unwrap(),
            ],
            ..AttemptFacts::default()
        };
        let ordinary = crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Completed,
            super::JournalExtension::CompatibilityAttempt {
                findings: Some(findings),
            },
            Some(attempt.clone()),
        );

        assert!(
            super::command::<()>(
                &run,
                &config,
                observation.digest(),
                Ok(&result),
                ordinary,
                terminal_rejection_draft(attempt),
            )
            .is_err()
        );
    }

    #[test]
    fn catalog_failure_cannot_claim_provider_evaluation_error() {
        let run = crate::operations::test_support::run();
        let attempt = AttemptFacts::default();
        let ordinary = crate::model::journal::JournalDraft::new(
            run.id().clone(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.compatibility",
            RequestId::parse("request-1").unwrap(),
            OutcomeClass::Error,
            Some(
                Reason::new(
                    ReasonCode::ProviderEvaluationError,
                    "forged provider evaluation failure",
                )
                .unwrap(),
            ),
            Some(attempt.clone()),
            super::JournalExtension::CompatibilityAttempt { findings: None },
        )
        .unwrap();
        let terminal = terminal_rejection_draft(attempt);
        let command = AppendCompatibilityAttemptCommand::from_parts(
            run.id().clone(),
            run.workflow_state_version(),
            run.lifecycle_version(),
            None,
            ordinary,
            terminal.clone(),
            terminal,
        );
        let source = ();
        let resolution: super::CompatibilityResolution<'_, (), (), (), ()> =
            super::CompatibilityResolution::CatalogError(&source, ProviderResolveFailure::Missing);

        assert!(matches!(
            super::revalidate_compatibility_command(&run, None, None, &resolution, &command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn pre_invocation_command_cannot_forge_expected_workflow_version() {
        let run = crate::operations::test_support::run();
        let source = ();
        let resolution: super::CompatibilityResolution<'_, (), (), (), ()> =
            super::CompatibilityResolution::CatalogError(&source, ProviderResolveFailure::Missing);
        let valid = super::command_for_resolution(
            &run,
            None,
            &resolution,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            RequestId::parse("request-1").unwrap(),
        )
        .unwrap();
        let forged = AppendCompatibilityAttemptCommand::from_parts(
            run.id().clone(),
            crate::model::version::WorkflowStateVersion::try_from(
                run.workflow_state_version().value() + 1,
            )
            .unwrap(),
            valid.expected_lifecycle_version(),
            valid.observed_drift(),
            valid.journal_entry().clone(),
            valid.terminal_rejection_entry().clone(),
            valid.stale_error_entry().clone(),
        );

        assert!(matches!(
            super::revalidate_compatibility_command(&run, None, None, &resolution, &forged),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn provider_evaluation_error_requires_authoritative_reason() {
        let run = crate::operations::test_support::run();
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let digest = DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap();
        let observation = ProviderObservation::new(
            run.registration_id().clone(),
            "/provider",
            digest.clone(),
            None,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        )
        .unwrap();
        let diagnostic = Diagnostic::new(
            "provider.evaluation_error",
            "provider could not evaluate compatibility",
            None,
        )
        .unwrap();
        let fact = ProviderFact::new(
            config.registration_id().clone(),
            config.config_revision(),
            ProviderRole::CheckCompatibility,
            RequestId::parse("actual-invocation").unwrap(),
            config.config().executable(),
            OutcomeClass::Error,
            digest,
            None,
            Some(1),
        )
        .unwrap();
        let result = CompatibilityResult {
            report: CompatibilityReport::evaluation_error(vec![diagnostic.clone()]).unwrap(),
            observation,
            fact: fact.clone(),
            protocol_major: 1,
            trace_failure: None,
        };
        let attempt = AttemptFacts {
            provider_observations: vec![fact],
            diagnostics: vec![diagnostic],
            ..AttemptFacts::default()
        };
        let ordinary = crate::model::journal::JournalDraft::new(
            run.id().clone(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.compatibility",
            RequestId::parse("request-1").unwrap(),
            OutcomeClass::Error,
            Some(
                Reason::new(
                    ReasonCode::ProviderProtocolMalformed,
                    "wrong provider failure classification",
                )
                .unwrap(),
            ),
            Some(attempt.clone()),
            super::JournalExtension::CompatibilityAttempt { findings: None },
        )
        .unwrap();

        assert!(
            super::command::<()>(
                &run,
                &config,
                result.observation.digest(),
                Ok(&result),
                ordinary,
                terminal_rejection_draft(attempt),
            )
            .is_err()
        );
    }

    fn compatibility_result(
        config: &ResolvedProviderConfig,
        observation: &ProviderObservation,
        invocation_id: &str,
    ) -> CompatibilityResult {
        CompatibilityResult {
            report: CompatibilityReport::findings(vec![]).unwrap(),
            observation: observation.clone(),
            fact: ProviderFact::new(
                config.registration_id().clone(),
                config.config_revision(),
                ProviderRole::CheckCompatibility,
                RequestId::parse(invocation_id).unwrap(),
                config.config().executable(),
                OutcomeClass::Completed,
                observation.digest().clone(),
                None,
                Some(1),
            )
            .unwrap(),
            protocol_major: 1,
            trace_failure: None,
        }
    }

    fn terminal_rejection_draft(attempt: AttemptFacts) -> crate::model::journal::JournalDraft {
        crate::operations::test_support::draft(
            "run.compatibility",
            OutcomeClass::Rejected,
            super::JournalExtension::CompatibilityAttempt { findings: None },
            Some(attempt),
        )
    }
}
