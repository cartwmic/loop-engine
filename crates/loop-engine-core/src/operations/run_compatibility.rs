use crate::capabilities::persistence_commands::{AppendCompatibilityAttemptCommand, CommitStatus};
use crate::capabilities::provider_catalog::{ProviderCatalog, ResolvedProviderConfig};
use crate::capabilities::provider_invoker::{
    CompatibilityRequest, CompatibilityResult, InvocationError, ProviderInvoker,
};
use crate::capabilities::run_reader::RunReader;
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::{JournalExtension, ProviderFact, ProviderRole};
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
    CatalogError(&'a C),
    RequestError(&'a Q),
    Provider(Result<&'a CompatibilityResult, &'a InvocationError<I>>),
}

#[derive(Debug)]
pub enum CompatibilityExecutionError<R, J, W> {
    Lookup(R),
    Command(J),
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
) -> Result<CommitStatus, CompatibilityExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: RunWriter,
    G: FnMut(&ResolvedProviderConfig, &Run) -> Result<CompatibilityRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        Option<&ProviderObservation>,
        CompatibilityResolution<'a, C::Error, R::Error, I::TransportError, Q>,
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
    let config = match catalog.resolve_enabled(run.registration_id()) {
        Ok(config) => config,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                None,
                Some(&creation),
                CompatibilityResolution::CatalogError(&error),
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
) -> Result<CommitStatus, CompatibilityExecutionError<R, J, W::Error>>
where
    W: RunWriter,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        Option<&ProviderObservation>,
        CompatibilityResolution<'a, C, R, I, Q>,
    ) -> Result<AppendCompatibilityAttemptCommand, J>,
{
    let command =
        command(run, config, creation, resolution).map_err(CompatibilityExecutionError::Command)?;
    writer
        .append_compatibility_attempt(command)
        .map_err(CompatibilityExecutionError::Writer)
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

pub fn command<E>(
    run: &Run,
    config: &ResolvedProviderConfig,
    creation_digest: &DigestObservation,
    result: Result<&CompatibilityResult, &InvocationError<E>>,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendCompatibilityAttemptCommand, CommandError> {
    validate_pair(run, &journal_entry, &terminal_rejection_entry)?;
    let observations = &journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?
        .provider_observations;
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
                && result.fact.outcome == OutcomeClass::Completed
                && one_bound_fact(observations, config, result)
        }
        (
            Ok(result),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => {
            matches!(
                result.report,
                crate::model::compatibility::CompatibilityReport::EvaluationError(_)
            ) && result.fact.outcome == OutcomeClass::Error
                && one_bound_fact(observations, config, result)
        }
        (
            Err(InvocationError::TraceBudgetUnavailable),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => observations.is_empty(),
        (
            Err(InvocationError::Transport { fact, .. }),
            OutcomeClass::Error,
            JournalExtension::CompatibilityAttempt { findings: None },
        ) => fact.outcome == OutcomeClass::Error && one_config_fact(observations, config, fact),
        _ => false,
    };
    if !report_matches {
        return Err(CommandError::JournalMismatch);
    }
    let observed_drift = result
        .ok()
        .and_then(|result| observed_drift(creation_digest, result.observation.digest()));
    Ok(AppendCompatibilityAttemptCommand {
        run_id: run.id().clone(),
        expected_lifecycle_version: run.lifecycle_version(),
        observed_drift,
        journal_entry,
        terminal_rejection_entry,
    })
}

pub fn local_rejection_command(
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
        && journal_entry
            .attempt()
            .is_some_and(|attempt| attempt.provider_observations.is_empty());
    if !valid {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendCompatibilityAttemptCommand {
        run_id: run.id().clone(),
        expected_lifecycle_version: run.lifecycle_version(),
        observed_drift: None,
        journal_entry,
        terminal_rejection_entry,
    })
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
        || terminal.provider_observations != ordinary.provider_observations
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
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
    use crate::model::provider::DigestObservation;

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
}
