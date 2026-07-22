use crate::capabilities::persistence_commands::{AppendGuidanceAttemptCommand, CommitStatus};
use crate::capabilities::provider_catalog::{
    ProviderCatalog, ProviderResolveFailure, ResolvedProviderConfig,
};
use crate::capabilities::provider_invoker::{
    GuidanceInvocationResult, GuidanceRequest, InvocationError, ProviderInvoker,
};
use crate::capabilities::run_reader::{RunReader, SelectedEvidenceReadError};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::{AttemptFacts, JournalExtension, ProviderRole};
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
    CatalogError(&'a C, ProviderResolveFailure),
    RequestError(&'a Q),
    Provider(Result<&'a GuidanceInvocationResult, &'a InvocationError<I>>),
}

#[derive(Debug)]
pub enum GuidanceExecutionError<R, J, W> {
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
        &GuidanceResolution<'a, C::Error, R::Error, I::TransportError, Q>,
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
            &GuidanceResolution::SelectionInvalid,
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
                    &GuidanceResolution::SelectionUnavailable,
                    &mut command,
                );
            }
            Err(SelectedEvidenceReadError::Read(error)) => {
                return persist_resolution(
                    writer,
                    &run,
                    None,
                    &[],
                    &GuidanceResolution::SelectionReadError(&error),
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
            &GuidanceResolution::Local(local),
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
                &GuidanceResolution::CatalogError(&error, C::classify_resolve_failure(&error)),
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
                &GuidanceResolution::RequestError(&error),
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
        &GuidanceResolution::Provider(result.as_ref()),
        &mut command,
    )
}

fn persist_resolution<R, C, I, Q, W, F, J>(
    writer: &W,
    run: &Run,
    config: Option<&ResolvedProviderConfig>,
    selected: &[EvidenceRecord],
    resolution: &GuidanceResolution<'_, C, R, I, Q>,
    command: &mut F,
) -> Result<CommitStatus, GuidanceExecutionError<R, J, W::Error>>
where
    W: RunWriter,
    F: for<'a> FnMut(
        &Run,
        Option<&ResolvedProviderConfig>,
        &[EvidenceRecord],
        &GuidanceResolution<'a, C, R, I, Q>,
    ) -> Result<AppendGuidanceAttemptCommand, J>,
{
    let command =
        command(run, config, selected, resolution).map_err(GuidanceExecutionError::Command)?;
    revalidate_guidance_command(run, config, selected, resolution, &command)
        .map_err(GuidanceExecutionError::InvalidCommand)?;
    writer
        .append_guidance_attempt(command)
        .map_err(GuidanceExecutionError::Writer)
}

fn revalidate_guidance_command<C, R, I, Q>(
    run: &Run,
    config: Option<&ResolvedProviderConfig>,
    selected: &[EvidenceRecord],
    resolution: &GuidanceResolution<'_, C, R, I, Q>,
    attempt_command: &AppendGuidanceAttemptCommand,
) -> Result<(), CommandError> {
    validate_pair(
        run,
        selected,
        attempt_command.journal_entry(),
        attempt_command.terminal_rejection_entry(),
    )?;
    match resolution {
        GuidanceResolution::Provider(result) => {
            let config = config.ok_or(CommandError::JournalMismatch)?;
            crate::operations::run_guidance::command(
                run,
                config,
                selected,
                *result,
                attempt_command.journal_entry().clone(),
                attempt_command.terminal_rejection_entry().clone(),
            )
            .map(|_| ())
        }
        GuidanceResolution::Local(disposition) => {
            let expected_reason = match disposition {
                GuidanceDisposition::Terminal => ReasonCode::RunLifecycleTerminal,
                GuidanceDisposition::StoredUnsupported => ReasonCode::GuidanceUnsupported,
                GuidanceDisposition::Invoke => return Err(CommandError::JournalMismatch),
            };
            if attempt_command
                .journal_entry()
                .reason()
                .map(|reason| reason.code())
                != Some(expected_reason)
            {
                return Err(CommandError::JournalMismatch);
            }
            local_rejection_command(
                run,
                selected,
                attempt_command.journal_entry().clone(),
                attempt_command.terminal_rejection_entry().clone(),
            )
            .map(|_| ())
        }
        GuidanceResolution::SelectionInvalid | GuidanceResolution::SelectionUnavailable => {
            revalidate_guidance_pre_invocation(
                run,
                selected,
                OutcomeClass::Rejected,
                ReasonCode::EvidenceSelectionInvalid,
                attempt_command,
            )
        }
        GuidanceResolution::SelectionReadError(_) => revalidate_guidance_pre_invocation(
            run,
            selected,
            OutcomeClass::Error,
            ReasonCode::PersistenceFailed,
            attempt_command,
        ),
        GuidanceResolution::CatalogError(_, reason) => revalidate_guidance_pre_invocation(
            run,
            selected,
            OutcomeClass::Error,
            reason.reason_code(),
            attempt_command,
        ),
        GuidanceResolution::RequestError(_) => revalidate_guidance_pre_invocation(
            run,
            selected,
            OutcomeClass::Error,
            ReasonCode::ResourceExhausted,
            attempt_command,
        ),
    }
}

fn revalidate_guidance_pre_invocation(
    run: &Run,
    selected: &[EvidenceRecord],
    expected_outcome: OutcomeClass,
    expected_reason: ReasonCode,
    attempt_command: &AppendGuidanceAttemptCommand,
) -> Result<(), CommandError> {
    validate_pair(
        run,
        selected,
        attempt_command.journal_entry(),
        attempt_command.terminal_rejection_entry(),
    )?;
    let valid = attempt_command.journal_entry().outcome() == expected_outcome
        && attempt_command
            .journal_entry()
            .reason()
            .map(|reason| reason.code())
            == Some(expected_reason)
        && matches!(
            attempt_command.journal_entry().extension(),
            JournalExtension::GuidanceAttempt {
                guidance_text: None
            }
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

pub fn disposition(run: &Run) -> GuidanceDisposition {
    if run.lifecycle() != Lifecycle::Active {
        GuidanceDisposition::Terminal
    } else if run.graph().live_guidance() == LiveGuidanceCapability::Unsupported {
        GuidanceDisposition::StoredUnsupported
    } else {
        GuidanceDisposition::Invoke
    }
}

pub(crate) fn command<E>(
    run: &Run,
    config: &ResolvedProviderConfig,
    selected: &[EvidenceRecord],
    result: Result<&GuidanceInvocationResult, &InvocationError<E>>,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendGuidanceAttemptCommand, CommandError> {
    validate_pair(run, selected, &journal_entry, &terminal_rejection_entry)?;
    let attempt = journal_entry
        .attempt()
        .ok_or(CommandError::JournalMismatch)?;
    let observations = &attempt.provider_observations;
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
                ) => guidance.text() == text.as_str() && attempt.diagnostics.is_empty(),
                (
                    LiveGuidanceResult::Incompatible(diagnostics),
                    OutcomeClass::Rejected,
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None,
                    },
                ) => attempt.diagnostics == diagnostics.as_slice(),
                (
                    LiveGuidanceResult::EvaluationError(diagnostics),
                    OutcomeClass::Error,
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None,
                    },
                ) => attempt.diagnostics == diagnostics.as_slice(),
                _ => false,
            };
            let reason_matches = match &invocation.result {
                LiveGuidanceResult::Guidance(_) => journal_entry.reason().is_none(),
                LiveGuidanceResult::Incompatible(_) => {
                    journal_entry.reason().map(|reason| reason.code())
                        == Some(ReasonCode::CompatibilityUnsupported)
                }
                LiveGuidanceResult::EvaluationError(_) => {
                    journal_entry.reason().map(|reason| reason.code())
                        == Some(ReasonCode::ProviderEvaluationError)
                }
            };
            shape_matches
                && reason_matches
                && invocation.fact.outcome == journal_entry.outcome()
                && one_guidance_fact(observations, config, &invocation.fact)
        }
        Err(InvocationError::TraceBudgetUnavailable) => {
            journal_entry.outcome() == OutcomeClass::Error
                && journal_entry.reason().map(|reason| reason.code())
                    == Some(ReasonCode::ResourceExhausted)
                && attempt.diagnostics.is_empty()
                && matches!(
                    journal_entry.extension(),
                    JournalExtension::GuidanceAttempt {
                        guidance_text: None
                    }
                )
                && observations.is_empty()
        }
        Err(InvocationError::Transport { fact, failure, .. }) => {
            journal_entry.outcome() == OutcomeClass::Error
                && journal_entry.reason() == Some(&failure.reason)
                && attempt.diagnostics == failure.diagnostics
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
    Ok(AppendGuidanceAttemptCommand::from_parts(
        run.id().clone(),
        run.lifecycle_version(),
        journal_entry,
        terminal_rejection_entry,
    ))
}

pub(crate) fn local_rejection_command(
    run: &Run,
    selected: &[EvidenceRecord],
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<AppendGuidanceAttemptCommand, CommandError> {
    validate_pair(run, selected, &journal_entry, &terminal_rejection_entry)?;
    let valid = journal_entry.outcome() == OutcomeClass::Rejected
        && matches!(
            journal_entry.extension(),
            JournalExtension::GuidanceAttempt {
                guidance_text: None
            }
        )
        && journal_entry.attempt().is_some_and(|attempt| {
            attempt.provider_observations.is_empty() && attempt.diagnostics.is_empty()
        });
    if !valid {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendGuidanceAttemptCommand::from_parts(
        run.id().clone(),
        run.lifecycle_version(),
        journal_entry,
        terminal_rejection_entry,
    ))
}

fn validate_pair(
    run: &Run,
    selected: &[EvidenceRecord],
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
        || terminal != ordinary
        || !matches!(
            terminal_rejection_entry.extension(),
            JournalExtension::GuidanceAttempt {
                guidance_text: None
            }
        )
        || !guidance_context_is_exact(ordinary)
        || !guidance_context_is_exact(terminal)
        || !evidence_facts_match(ordinary, selected)
        || !evidence_facts_match(terminal, selected)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

fn guidance_context_is_exact(attempt: &AttemptFacts) -> bool {
    attempt.transition.is_none()
        && attempt.gate_verdict_facts.is_none()
        && attempt.note.is_none()
        && attempt.actor.is_none()
        && attempt.corrects_sequence.is_none()
}

fn evidence_facts_match(attempt: &AttemptFacts, selected: &[EvidenceRecord]) -> bool {
    let expected_ids = selected
        .iter()
        .map(EvidenceRecord::id)
        .cloned()
        .collect::<Vec<_>>();
    match (&attempt.evidence_associations, attempt.evidence_recorded) {
        (None, None) if selected.is_empty() => true,
        (Some(associations), Some(recorded)) => {
            !selected.is_empty()
                && associations.inline.is_empty()
                && associations.provider_recorded_ids.is_empty()
                && associations.selected_ids == expected_ids
                && associations.recorded_status() == recorded
                && !recorded.inline
                && !recorded.provider
                && recorded.selected_associations
        }
        _ => false,
    }
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
    use crate::capabilities::persistence_commands::AppendGuidanceAttemptCommand;
    use crate::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
    use crate::capabilities::provider_invoker::GuidanceInvocationResult;
    use crate::model::attempt::{AttemptFacts, EvidenceAssociations, ProviderFact, ProviderRole};
    use crate::model::evidence::{EvidenceRecord, EvidenceSource};
    use crate::model::ids::{EvidenceId, EvidenceKind, ProviderHandle, RequestId};
    use crate::model::journal::JournalDraft;
    use crate::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
    use crate::model::provider::DigestObservation;
    use crate::model::reason::{Reason, ReasonCode};
    use crate::model::time::ObservedAt;

    fn guidance_attempt(selected_ids: Vec<EvidenceId>) -> AttemptFacts {
        let associations = EvidenceAssociations {
            selected_ids,
            ..EvidenceAssociations::default()
        };
        AttemptFacts {
            evidence_associations: Some(associations.clone()),
            evidence_recorded: Some(associations.recorded_status()),
            ..AttemptFacts::default()
        }
    }

    fn guidance_draft(
        outcome: OutcomeClass,
        reason_code: ReasonCode,
        attempt: AttemptFacts,
    ) -> JournalDraft {
        JournalDraft::new(
            crate::model::ids::RunId::parse("run-1").unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.guidance",
            RequestId::parse("request-1").unwrap(),
            outcome,
            Some(Reason::new(reason_code, "test disposition").unwrap()),
            Some(attempt),
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
        )
        .unwrap()
    }

    fn terminal_rejection_draft(attempt: AttemptFacts) -> JournalDraft {
        crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(attempt),
        )
    }

    fn evidence(id: &str) -> EvidenceRecord {
        EvidenceRecord::new(
            EvidenceId::parse(id).unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            "opaque:locator",
            None,
            None,
            None,
            EvidenceSource::Caller,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn stored_unsupported_is_rejected_without_invocation() {
        let run = crate::operations::test_support::run();
        assert_eq!(
            super::disposition(&run),
            super::GuidanceDisposition::StoredUnsupported
        );
    }

    #[test]
    fn empty_selection_requires_no_evidence_claims() {
        let run = crate::operations::test_support::run();
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(AttemptFacts::default()),
        );
        let terminal = terminal_rejection_draft(AttemptFacts::default());
        assert!(
            super::local_rejection_command(&run, &[], ordinary.clone(), terminal.clone()).is_ok()
        );
        assert!(
            super::local_rejection_command(
                &run,
                &[],
                ordinary,
                terminal_rejection_draft(guidance_attempt(vec![
                    EvidenceId::parse("evidence-1").unwrap()
                ]))
            )
            .is_err()
        );
    }

    #[test]
    fn selected_evidence_requires_exact_ids_without_inline_or_provider() {
        let run = crate::operations::test_support::run();
        let evidence = crate::operations::test_support::evidence();
        let attempt = guidance_attempt(vec![evidence.id().clone()]);
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(attempt.clone()),
        );
        let terminal = terminal_rejection_draft(attempt);
        assert!(
            super::local_rejection_command(
                &run,
                std::slice::from_ref(&evidence),
                ordinary,
                terminal
            )
            .is_ok()
        );

        let mut inline_attempt = guidance_attempt(vec![evidence.id().clone()]);
        inline_attempt
            .evidence_associations
            .as_mut()
            .expect("associations")
            .inline
            .push(evidence.clone());
        inline_attempt.evidence_recorded = Some(EvidenceRecordedStatus {
            inline: true,
            selected_associations: true,
            provider: false,
        });
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(inline_attempt.clone()),
        );
        assert!(
            super::local_rejection_command(
                &run,
                std::slice::from_ref(&evidence),
                ordinary,
                terminal_rejection_draft(inline_attempt)
            )
            .is_err()
        );
    }

    #[test]
    fn terminal_race_draft_must_mirror_evidence_facts() {
        let run = crate::operations::test_support::run();
        let evidence = crate::operations::test_support::evidence();
        let attempt = guidance_attempt(vec![evidence.id().clone()]);
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(attempt),
        );
        let terminal = terminal_rejection_draft(AttemptFacts::default());
        assert!(
            super::local_rejection_command(
                &run,
                std::slice::from_ref(&evidence),
                ordinary,
                terminal
            )
            .is_err()
        );
    }

    #[test]
    fn provider_evaluation_error_requires_authoritative_diagnostics() {
        let run = crate::operations::test_support::run();
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let authoritative = crate::model::diagnostic::Diagnostic::new(
            "authoritative",
            "actual provider diagnostic",
            None,
        )
        .unwrap();
        let fabricated =
            crate::model::diagnostic::Diagnostic::new("fabricated", "substituted diagnostic", None)
                .unwrap();
        let fact = ProviderFact::new(
            run.registration_id().clone(),
            1,
            ProviderRole::LiveGuidance,
            RequestId::parse("provider-call-1").unwrap(),
            "/provider",
            OutcomeClass::Error,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap();
        let invocation = GuidanceInvocationResult {
            result: crate::model::live_guidance::LiveGuidanceResult::evaluation_error(vec![
                authoritative,
            ])
            .unwrap(),
            fact: fact.clone(),
            trace_failure: None,
        };
        let mut attempt = guidance_attempt(Vec::new());
        attempt.provider_observations = vec![fact];
        attempt.diagnostics = vec![fabricated];
        let ordinary = guidance_draft(
            OutcomeClass::Error,
            ReasonCode::ProviderEvaluationError,
            attempt.clone(),
        );
        let terminal = guidance_draft(
            OutcomeClass::Rejected,
            ReasonCode::RunLifecycleTerminal,
            attempt,
        );
        let command = AppendGuidanceAttemptCommand::from_parts(
            run.id().clone(),
            run.lifecycle_version(),
            ordinary,
            terminal,
        );
        let resolution: super::GuidanceResolution<'_, (), (), (), ()> =
            super::GuidanceResolution::Provider(Ok(&invocation));

        assert!(matches!(
            super::revalidate_guidance_command(&run, Some(&config), &[], &resolution, &command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn completed_guidance_cannot_add_fabricated_diagnostics() {
        let run = crate::operations::test_support::run();
        let config = ResolvedProviderConfig::new(
            run.registration_id().clone(),
            ProviderHandle::parse("provider").unwrap(),
            1,
            ProviderConfig::new("/provider", vec![], "/", 30).unwrap(),
        )
        .unwrap();
        let fact = ProviderFact::new(
            run.registration_id().clone(),
            1,
            ProviderRole::LiveGuidance,
            RequestId::parse("provider-call-1").unwrap(),
            "/provider",
            OutcomeClass::Completed,
            DigestObservation::Unavailable,
            None,
            Some(1),
        )
        .unwrap();
        let invocation = GuidanceInvocationResult {
            result: crate::model::live_guidance::LiveGuidanceResult::Guidance(
                crate::model::live_guidance::AdvisoryGuidance::new("next").unwrap(),
            ),
            fact: fact.clone(),
            trace_failure: None,
        };
        let attempt = AttemptFacts {
            provider_observations: vec![fact],
            diagnostics: vec![
                crate::model::diagnostic::Diagnostic::new(
                    "fabricated",
                    "not supplied by provider",
                    None,
                )
                .unwrap(),
            ],
            ..AttemptFacts::default()
        };
        let ordinary = JournalDraft::new(
            run.id().clone(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.guidance",
            RequestId::parse("request-1").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(attempt.clone()),
            super::JournalExtension::GuidanceAttempt {
                guidance_text: Some(
                    crate::model::bounded::BoundedText::<
                        { crate::model::bounded::GUIDANCE_TEXT_BYTES },
                    >::non_empty("guidance_text", "next")
                    .unwrap(),
                ),
            },
        )
        .unwrap();

        assert!(
            super::command::<()>(
                &run,
                &config,
                &[],
                Ok(&invocation),
                ordinary,
                guidance_draft(
                    OutcomeClass::Rejected,
                    ReasonCode::RunLifecycleTerminal,
                    attempt,
                ),
            )
            .is_err()
        );
    }

    #[test]
    fn stored_unsupported_cannot_claim_terminal_reason() {
        let run = crate::operations::test_support::run();
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(AttemptFacts::default()),
        );
        let terminal = terminal_rejection_draft(AttemptFacts::default());
        let command = super::local_rejection_command(&run, &[], ordinary, terminal)
            .expect("self-consistent terminal rejection");
        let resolution: super::GuidanceResolution<'_, (), (), (), ()> =
            super::GuidanceResolution::Local(super::GuidanceDisposition::StoredUnsupported);

        assert!(matches!(
            super::revalidate_guidance_command(&run, None, &[], &resolution, &command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn invalid_selection_cannot_claim_guidance_unsupported() {
        let run = crate::operations::test_support::run();
        let attempt = AttemptFacts::default();
        let command = AppendGuidanceAttemptCommand::from_parts(
            run.id().clone(),
            run.lifecycle_version(),
            guidance_draft(
                OutcomeClass::Rejected,
                ReasonCode::GuidanceUnsupported,
                attempt.clone(),
            ),
            guidance_draft(
                OutcomeClass::Rejected,
                ReasonCode::RunLifecycleTerminal,
                attempt,
            ),
        );
        let resolution: super::GuidanceResolution<'_, (), (), (), ()> =
            super::GuidanceResolution::SelectionInvalid;

        assert!(matches!(
            super::revalidate_guidance_command(&run, None, &[], &resolution, &command),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }

    #[test]
    fn opaque_command_cannot_substitute_unselected_evidence() {
        let run = crate::operations::test_support::run();
        let selected = evidence("selected-evidence");
        let substituted = evidence("substituted-evidence");
        let attempt = guidance_attempt(vec![substituted.id().clone()]);
        let ordinary = crate::operations::test_support::draft(
            "run.guidance",
            OutcomeClass::Rejected,
            super::JournalExtension::GuidanceAttempt {
                guidance_text: None,
            },
            Some(attempt.clone()),
        );
        let malicious = super::local_rejection_command(
            &run,
            std::slice::from_ref(&substituted),
            ordinary,
            terminal_rejection_draft(attempt),
        )
        .expect("self-consistent substituted command");
        let resolution: super::GuidanceResolution<'_, (), (), (), ()> =
            super::GuidanceResolution::Local(super::GuidanceDisposition::StoredUnsupported);

        assert!(matches!(
            super::revalidate_guidance_command(
                &run,
                None,
                std::slice::from_ref(&selected),
                &resolution,
                &malicious,
            ),
            Err(crate::operations::CommandError::JournalMismatch)
        ));
    }
}
