use thiserror::Error;

use crate::capabilities::persistence_commands::{CommitStatus, ReplaceLabelCommand};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::JournalExtension;
use crate::model::bounded::{BoundError, BoundedText};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LabelError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Command(#[from] CommandError),
}

pub fn execute<W: RunWriter>(
    writer: &W,
    command: ReplaceLabelCommand,
) -> Result<CommitStatus, W::Error> {
    writer.replace_label(command)
}

pub fn command(
    run: &Run,
    label: Option<String>,
    completed_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<ReplaceLabelCommand, LabelError> {
    let label = label
        .map(|value| BoundedText::non_empty("run_label", value))
        .transpose()?;
    for entry in [&completed_entry, &terminal_rejection_entry] {
        validate_journal(entry, run.id(), "run.label", JournalEntryKind::LabelChanged)?;
    }
    let completed_matches = completed_entry.outcome() == OutcomeClass::Completed
        && matches!(
            completed_entry.extension(),
            JournalExtension::LabelChanged { change: Some(change) }
                if change.label_after == label
        );
    let terminal_matches = terminal_rejection_entry.outcome() == OutcomeClass::Rejected
        && terminal_rejection_entry
            .reason()
            .map(|reason| reason.code())
            == Some(ReasonCode::RunLifecycleTerminal)
        && matches!(
            terminal_rejection_entry.extension(),
            JournalExtension::LabelChanged { change: None }
        );
    if !completed_matches || !terminal_matches {
        return Err(CommandError::JournalMismatch.into());
    }
    Ok(ReplaceLabelCommand::from_parts(
        run.id().clone(),
        label,
        completed_entry,
        terminal_rejection_entry,
    ))
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::{JournalExtension, LabelChangeFact};
    use crate::model::bounded::BoundedText;
    use crate::model::outcome::OutcomeClass;

    #[test]
    fn command_carries_both_atomic_lifecycle_dispositions() {
        let run = crate::operations::test_support::run();
        let completed = crate::operations::test_support::draft(
            "run.label",
            OutcomeClass::Completed,
            JournalExtension::LabelChanged {
                change: Some(LabelChangeFact {
                    label_before: None,
                    label_after: Some(BoundedText::non_empty("run_label", "next").unwrap()),
                }),
            },
            None,
        );
        let rejected = crate::operations::test_support::draft(
            "run.label",
            OutcomeClass::Rejected,
            JournalExtension::LabelChanged { change: None },
            None,
        );
        let command = super::command(&run, Some("next".into()), completed, rejected).unwrap();
        assert_eq!(command.label(), Some("next"));
        let authoritative_before = BoundedText::non_empty("run_label", "concurrent").unwrap();
        let (_, _, completed, _) = command
            .into_transaction_parts(Some(authoritative_before))
            .unwrap();
        assert!(matches!(
            completed.extension(),
            JournalExtension::LabelChanged { change: Some(change) }
                if change.label_before.as_ref().map(BoundedText::as_str) == Some("concurrent")
                    && change.label_after.as_ref().map(BoundedText::as_str) == Some("next")
        ));
    }
}
