use crate::capabilities::persistence_commands::{AppendEvidenceCommand, AttemptCommit};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::{AttemptFacts, EvidenceAssociations, JournalExtension};
use crate::model::evidence::EvidenceRecord;
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::lifecycle::Lifecycle;
use crate::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

pub fn supports_lifecycle(_lifecycle: Lifecycle) -> bool {
    true
}

pub fn execute<W: RunWriter>(
    writer: &W,
    command: AppendEvidenceCommand,
) -> Result<AttemptCommit, W::Error> {
    writer.append_evidence(command)
}

pub fn command(
    run: &Run,
    evidence: EvidenceRecord,
    completed_entry: JournalDraft,
    duplicate_rejection_entry: JournalDraft,
) -> Result<AppendEvidenceCommand, CommandError> {
    for entry in [&completed_entry, &duplicate_rejection_entry] {
        validate_journal(
            entry,
            run.id(),
            "run.evidence.add",
            JournalEntryKind::EvidenceAdded,
        )?;
    }
    let expected_attempt = AttemptFacts {
        evidence_associations: Some(EvidenceAssociations::default()),
        evidence_recorded: Some(EvidenceRecordedStatus::default()),
        ..AttemptFacts::default()
    };
    let completed_matches = completed_entry.outcome() == OutcomeClass::Completed
        && completed_entry.reason().is_none()
        && completed_entry.attempt() == Some(&expected_attempt)
        && matches!(
            completed_entry.extension(),
            JournalExtension::EvidenceAdded { added: Some(added) }
                if added.evidence_id == *evidence.id()
                    && added.kind == *evidence.kind()
                    && added.locator.as_str() == evidence.locator()
                    && added.digest.as_ref().map(|value| value.as_str()) == evidence.digest()
        );
    let duplicate_matches = duplicate_rejection_entry.outcome() == OutcomeClass::Rejected
        && duplicate_rejection_entry
            .reason()
            .map(|reason| reason.code())
            == Some(ReasonCode::EvidenceInvalid)
        && duplicate_rejection_entry.attempt() == Some(&expected_attempt)
        && matches!(
            duplicate_rejection_entry.extension(),
            JournalExtension::EvidenceAdded { added: None }
        );
    if !completed_matches || !duplicate_matches {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendEvidenceCommand::from_dual_disposition(
        run.id().clone(),
        evidence,
        completed_entry,
        duplicate_rejection_entry,
    ))
}

#[cfg(feature = "test-support")]
fn pre_resolved_rejection_command(
    run: &Run,
    journal_entry: JournalDraft,
) -> Result<AppendEvidenceCommand, CommandError> {
    validate_journal(
        &journal_entry,
        run.id(),
        "run.evidence.add",
        JournalEntryKind::EvidenceAdded,
    )?;
    if journal_entry.outcome() != OutcomeClass::Rejected
        || journal_entry.reason().map(|reason| reason.code()) != Some(ReasonCode::EvidenceInvalid)
        || !matches!(
            journal_entry.extension(),
            JournalExtension::EvidenceAdded { added: None }
        )
        || journal_entry.attempt()
            != Some(&AttemptFacts {
                evidence_associations: Some(EvidenceAssociations::default()),
                evidence_recorded: Some(EvidenceRecordedStatus::default()),
                ..AttemptFacts::default()
            })
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendEvidenceCommand::from_pre_resolved_rejection(
        run.id().clone(),
        journal_entry,
    ))
}

#[cfg(feature = "test-support")]
pub fn rejected_command(
    run: &Run,
    journal_entry: JournalDraft,
) -> Result<AppendEvidenceCommand, CommandError> {
    pre_resolved_rejection_command(run, journal_entry)
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::{
        AttemptFacts, EvidenceAddedFact, EvidenceAssociations, JournalExtension,
    };
    use crate::model::bounded::BoundedText;
    use crate::model::diagnostic::Diagnostic;
    use crate::model::outcome::{EvidenceRecordedStatus, OutcomeClass};
    use crate::model::reason::{Reason, ReasonCode};

    fn evidence_attempt() -> AttemptFacts {
        AttemptFacts {
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(EvidenceRecordedStatus::default()),
            ..AttemptFacts::default()
        }
    }

    fn duplicate_rejection_draft(
        run: &crate::model::run::Run,
    ) -> crate::model::journal::JournalDraft {
        crate::model::journal::JournalDraft::new(
            run.id().clone(),
            crate::model::time::ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            crate::model::ids::RequestId::parse("request-evidence-dup").unwrap(),
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::EvidenceInvalid, "duplicate evidence id").unwrap()),
            Some(evidence_attempt()),
            JournalExtension::EvidenceAdded { added: None },
        )
        .unwrap()
    }

    #[test]
    fn rejects_journal_from_different_operation() {
        let run = crate::operations::test_support::run();
        let draft = crate::operations::test_support::draft(
            "run.annotate",
            OutcomeClass::Completed,
            JournalExtension::Annotation,
            Some(AttemptFacts::default()),
        );
        assert!(
            super::command(
                &run,
                crate::operations::test_support::evidence(),
                draft.clone(),
                duplicate_rejection_draft(&run),
            )
            .is_err()
        );
    }

    #[test]
    fn command_carries_both_atomic_duplicate_dispositions() {
        let run = crate::operations::test_support::run();
        let evidence = crate::operations::test_support::evidence();
        let completed = crate::model::journal::JournalDraft::new(
            run.id().clone(),
            crate::model::time::ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            crate::model::ids::RequestId::parse("request-evidence").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(evidence_attempt()),
            JournalExtension::EvidenceAdded {
                added: Some(EvidenceAddedFact {
                    evidence_id: evidence.id().clone(),
                    kind: evidence.kind().clone(),
                    locator: BoundedText::opaque_non_empty("evidence_locator", evidence.locator())
                        .unwrap(),
                    digest: None,
                }),
            },
        )
        .unwrap();
        let rejected = duplicate_rejection_draft(&run);
        let command = super::command(&run, evidence.clone(), completed, rejected.clone()).unwrap();
        assert!(command.evidence().is_some());
        assert_eq!(command.completed_entry().outcome(), OutcomeClass::Completed);
        assert_eq!(
            command.duplicate_rejection_entry().outcome(),
            OutcomeClass::Rejected
        );
        assert!(matches!(
            command.duplicate_rejection_entry().extension(),
            JournalExtension::EvidenceAdded { added: None }
        ));
    }

    #[test]
    fn command_rejects_fabricated_attempt_facts() {
        let run = crate::operations::test_support::run();
        let evidence = crate::operations::test_support::evidence();
        let completed = crate::model::journal::JournalDraft::new(
            run.id().clone(),
            crate::model::time::ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            crate::model::ids::RequestId::parse("request-evidence").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(AttemptFacts {
                diagnostics: vec![Diagnostic::new("fabricated", "not observed", None).unwrap()],
                ..evidence_attempt()
            }),
            JournalExtension::EvidenceAdded {
                added: Some(EvidenceAddedFact {
                    evidence_id: evidence.id().clone(),
                    kind: evidence.kind().clone(),
                    locator: BoundedText::opaque_non_empty("evidence_locator", evidence.locator())
                        .unwrap(),
                    digest: None,
                }),
            },
        )
        .unwrap();
        assert!(
            super::command(&run, evidence, completed, duplicate_rejection_draft(&run),).is_err()
        );
    }

    #[test]
    fn command_rejects_mismatched_duplicate_disposition() {
        let run = crate::operations::test_support::run();
        let evidence = crate::operations::test_support::evidence();
        let completed = crate::model::journal::JournalDraft::new(
            run.id().clone(),
            crate::model::time::ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
            "run.evidence.add",
            crate::model::ids::RequestId::parse("request-evidence").unwrap(),
            OutcomeClass::Completed,
            None,
            Some(evidence_attempt()),
            JournalExtension::EvidenceAdded {
                added: Some(EvidenceAddedFact {
                    evidence_id: evidence.id().clone(),
                    kind: evidence.kind().clone(),
                    locator: BoundedText::opaque_non_empty("evidence_locator", evidence.locator())
                        .unwrap(),
                    digest: None,
                }),
            },
        )
        .unwrap();
        assert!(super::command(&run, evidence, completed.clone(), completed).is_err());
    }
}
