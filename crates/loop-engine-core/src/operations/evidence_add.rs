use crate::capabilities::persistence_commands::{AppendEvidenceCommand, CommitStatus};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::JournalExtension;
use crate::model::evidence::EvidenceRecord;
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::lifecycle::Lifecycle;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

pub fn supports_lifecycle(_lifecycle: Lifecycle) -> bool {
    true
}

pub fn execute<W: RunWriter>(
    writer: &W,
    command: AppendEvidenceCommand,
) -> Result<CommitStatus, W::Error> {
    writer.append_evidence(command)
}

pub fn command(
    run: &Run,
    evidence: EvidenceRecord,
    journal_entry: JournalDraft,
) -> Result<AppendEvidenceCommand, CommandError> {
    validate_journal(
        &journal_entry,
        run.id(),
        "run.evidence.add",
        JournalEntryKind::EvidenceAdded,
    )?;
    let matches_evidence = matches!(
        journal_entry.extension(),
        JournalExtension::EvidenceAdded { added: Some(added) }
            if added.evidence_id == *evidence.id()
                && added.kind == *evidence.kind()
                && added.locator.as_str() == evidence.locator()
                && added.digest.as_ref().map(|value| value.as_str()) == evidence.digest()
    );
    if !matches_evidence {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendEvidenceCommand {
        run_id: run.id().clone(),
        evidence: Some(evidence),
        journal_entry,
    })
}

pub fn rejected_command(
    run: &Run,
    journal_entry: JournalDraft,
) -> Result<AppendEvidenceCommand, CommandError> {
    validate_journal(
        &journal_entry,
        run.id(),
        "run.evidence.add",
        JournalEntryKind::EvidenceAdded,
    )?;
    if !matches!(
        journal_entry.extension(),
        JournalExtension::EvidenceAdded { added: None }
    ) {
        return Err(CommandError::JournalMismatch);
    }
    Ok(AppendEvidenceCommand {
        run_id: run.id().clone(),
        evidence: None,
        journal_entry,
    })
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::{AttemptFacts, JournalExtension};
    use crate::model::outcome::OutcomeClass;

    #[test]
    fn rejects_journal_from_different_operation() {
        let run = crate::operations::test_support::run();
        let draft = crate::operations::test_support::draft(
            "run.annotate",
            OutcomeClass::Completed,
            JournalExtension::Annotation,
            Some(AttemptFacts::default()),
        );
        assert!(super::command(&run, crate::operations::test_support::evidence(), draft).is_err());
    }
}
