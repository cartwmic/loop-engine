use crate::capabilities::persistence_commands::{AppendGuidanceAttemptCommand, CommitStatus};
use crate::capabilities::provider_catalog::{ProviderCatalog, ResolvedProviderConfig};
use crate::capabilities::provider_invoker::{
    GuidanceInvocationResult, GuidanceRequest, InvocationError, ProviderInvoker,
};
use crate::capabilities::run_reader::{RunReader, SelectedEvidenceReadError};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::{JournalExtension, ProviderRole};
use crate::model::evidence::EvidenceRecord;
use crate::model::guidance::LiveGuidanceCapability;
use crate::model::ids::{EvidenceId, RunId};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::lifecycle::Lifecycle;
use crate::model::live_guidance::LiveGuidanceResult;
use crate::model::outcome::OutcomeClass;
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceDisposition {
    Invoke,
    StoredUnsupported,
    Terminal,
}

pub enum GuidanceResolution<'a, C, R, I, Q> {
    Local(GuidanceDisposition),
    SelectionInvalid,
    SelectionUnavailable,
    SelectionReadError(&'a R),
    CatalogError(&'a C),
    RequestError(&'a Q),
    Provider(Result<&'a GuidanceInvocationResult, &'a InvocationError<I>>),
}

#[derive(Debug)]
pub enum GuidanceExecutionError<R, J, W> {
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
    selected_ids: &[EvidenceId],
    mut request: G,
    mut command: F,
) -> Result<CommitStatus, GuidanceExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: RunWriter,
    G: FnMut(&ResolvedProviderConfig, &Run, &[EvidenceRecord]) -> Result<GuidanceRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        &[EvidenceRecord],
        GuidanceResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<AppendGuidanceAttemptCommand, J>,
{
    let run = reader.get(run_id).map_err(GuidanceExecutionError::Lookup)?;
    if selected_ids
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != selected_ids.len()
    {
        return persist_resolution(
            writer,
            &run,
            None,
            &[],
            GuidanceResolution::SelectionInvalid,
            &mut command,
        );
    }
    let selected = if selected_ids.is_empty() {
        vec![]
    } else {
        match reader.selected_evidence(run_id, selected_ids) {
            Ok(selected) => selected,
            Err(SelectedEvidenceReadError::Unavailable) => {
                return persist_resolution(
                    writer,
                    &run,
                    None,
                    &[],
                    GuidanceResolution::SelectionUnavailable,
                    &mut command,
                );
            }
            Err(SelectedEvidenceReadError::Read(error)) => {
                return persist_resolution(
                    writer,
                    &run,
                    None,
                    &[],
                    GuidanceResolution::SelectionReadError(&error),
                    &mut command,
                );
            }
        }
    };
    let local = disposition(&run);
    if local != GuidanceDisposition::Invoke {
        return persist_resolution(
            writer,
            &run,
            None,
            &selected,
            GuidanceResolution::Local(local),
            &mut command,
        );
    }
    let config = match catalog.resolve_enabled(run.registration_id()) {
        Ok(config) => config,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                None,
                &selected,
                GuidanceResolution::CatalogError(&error),
                &mut command,
            );
        }
    };
    let provider_request = match request(&config, &run, &selected) {
        Ok(request) => request,
        Err(error) => {
            return persist_resolution(
                writer,
                &run,
                Some(&config),
                &selected,
                GuidanceResolution::RequestError(&error),
                &mut command,
            );
        }
    };
    let result = invoker.live_guidance(&config, provider_request);
    persist_resolution(
        writer,
        &run,
        Some(&config),
        &selected,
        GuidanceResolution::Provider(result.as_ref()),
        &mut command,
    )
}

fn persist_resolution<R, C, I, Q, W, F, J>(
    writer: &W,
    run: &Run,
    config: Option<&ResolvedProviderConfig>,
    selected: &[EvidenceRecord],
    resolution: GuidanceResolution<'_, C, R, I, Q>,
    command: &mut F,
) -> Result<CommitStatus, GuidanceExecutionError<R, J, W::Error>>
where
    W: RunWriter,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        &[EvidenceRecord],
        GuidanceResolution<'a, C, R, I, Q>,
    ) -> Result<AppendGuidanceAttemptCommand, J>,
{
    let command =
        command(run, config, selected, resolution).map_err(GuidanceExecutionError::Command)?;
    writer
        .append_guidance_attempt(command)
        .map_err(GuidanceExecutionError::Writer)
}

pub fn disposition(run: &Run) -> GuidanceDisposition {
    if run.lifecycle() != Lifecycle::Active {
        GuidanceDisposition::Terminal
    } else if run.graph().live_guidance() == LiveGuidanceCapability::Unsupported {
        GuidanceDisposition::StoredUnsupported
    } else {
        GuidanceDisposition::Invoke
    }
}

pub fn command<E>(
    run: &Run,
    config: &ResolvedProviderConfig,
    result: Result<&GuidanceInvocationResult, &InvocationError<E>>,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendGuidanceAttemptCommand, CommandError> {
    validate_pair(run, &journal_entry, &terminal_rejection_entry)?;
    let observations = &journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?
        .provider_observations;
    let result_matches = match result {
        Ok(invocation) => {
            let shape_matches = match (
                &invocation.result,
                journal_entry.outcome(),
                journal_entry.extension(),
            ) {
                (
                    LiveGuidanceResult::Guidance(guidance),
                    OutcomeClass::Completed,
                    JournalExtension::GuidanceAttempt {
                        guidance_text: Some(text),
                    },
                ) => guidance.text() == text.as_str(),
                (
                    LiveGuidanceResult::Incompatible(_),
                    OutcomeClass::Rejected,
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None,
                    },
                )
                | (
                    LiveGuidanceResult::EvaluationError(_),
                    OutcomeClass::Error,
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None,
                    },
                ) => true,
                _ => false,
            };
            shape_matches
                && invocation.fact.outcome == journal_entry.outcome()
                && one_guidance_fact(observations, config, &invocation.fact)
        }
        Err(InvocationError::TraceBudgetUnavailable) => {
            journal_entry.outcome() == OutcomeClass::Error
                && matches!(
                    journal_entry.extension(),
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None
                    }
                )
                && observations.is_empty()
        }
        Err(InvocationError::Transport { fact, .. }) => {
            journal_entry.outcome() == OutcomeClass::Error
                && matches!(
                    journal_entry.extension(),
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None
                    }
                )
                && fact.outcome == OutcomeClass::Error
                && one_guidance_fact(observations, config, fact)
        }
    };
    if !result_matches {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendGuidanceAttemptCommand {
        run_id: run.id().clone(),
        expected_lifecycle_version: run.lifecycle_version(),
        journal_entry,
        terminal_rejection_entry,
    })
}

pub fn local_rejection_command(
    run: &Run,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendGuidanceAttemptCommand, CommandError> {
    validate_pair(run, &journal_entry, &terminal_rejection_entry)?;
    let valid = journal_entry.outcome() == OutcomeClass::Rejected
        && matches!(
            journal_entry.extension(),
            JournalExtension::GuidanceAttempt {
                guidance_text: None
            }
        )
        && journal_entry
            .attempt()
            .is_some_and(|attempt| attempt.provider_observations.is_empty());
    if !valid {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendGuidanceAttemptCommand {
        run_id: run.id().clone(),
        expected_lifecycle_version: run.lifecycle_version(),
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
            "run.guidance",
            JournalEntryKind::GuidanceAttempt,
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
        || terminal.evidence_associations != ordinary.evidence_associations
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

fn one_guidance_fact(
    facts: &[crate::model::attempt::ProviderFact],
    config: &ResolvedProviderConfig,
    expected: &crate::model::attempt::ProviderFact,
) -> bool {
    matches!(facts, [fact]
        if fact == expected
            && fact.role == ProviderRole::LiveGuidance
            && fact.registration_id == *config.registration_id()
            && fact.config_revision == config.config_revision()
            && fact.executable.as_str() == config.config().executable())
}

#[cfg(test)]
mod tests {
    #[test]
    fn stored_unsupported_is_rejected_without_invocation() {
        let run = crate::operations::test_support::run();
        assert_eq!(
            super::disposition(&run),
            super::GuidanceDisposition::StoredUnsupported
        );
    }
}
