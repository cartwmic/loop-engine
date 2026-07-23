use crate::capabilities::persistence_commands::{TerminateCommit, TerminateRunCommand};
use crate::capabilities::run_writer::RunWriter;
use crate::model::annotation::Note;
use crate::model::attempt::{AttemptFacts, JournalExtension};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

pub fn execute<W: RunWriter>(
    writer: &W,
    command: TerminateRunCommand,
) -> Result<TerminateCommit, W::Error> {
    writer.terminate(command)
}

pub fn command(
    run: &Run,
    note: Option<Note>,
    completed_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
    stale_error_entry: JournalDraft,
) -> Result<TerminateRunCommand, CommandError> {
    for entry in [
        &completed_entry,
        &terminal_rejection_entry,
        &stale_error_entry,
    ] {
        validate_journal(
            entry,
            run.id(),
            "run.terminate",
            JournalEntryKind::RunTerminated,
        )?;
    }
    let expected_attempt = note.as_ref().map(|note| AttemptFacts {
        note: Some(note.clone()),
        ..AttemptFacts::default()
    });
    let attempt_matches = |entry: &JournalDraft| entry.attempt() == expected_attempt.as_ref();
    if completed_entry.outcome() != OutcomeClass::Completed
        || completed_entry.reason().is_some()
        || !matches!(completed_entry.extension(), JournalExtension::RunTerminated)
        || !attempt_matches(&completed_entry)
        || terminal_rejection_entry.outcome() != OutcomeClass::Rejected
        || terminal_rejection_entry
            .reason()
            .map(|reason| reason.code())
            != Some(ReasonCode::RunLifecycleTerminal)
        || !matches!(
            terminal_rejection_entry.extension(),
            JournalExtension::RunTerminated
        )
        || !attempt_matches(&terminal_rejection_entry)
        || stale_error_entry.outcome() != OutcomeClass::Error
        || stale_error_entry.reason().map(|reason| reason.code())
            != Some(ReasonCode::StateStaleVersion)
        || !matches!(
            stale_error_entry.extension(),
            JournalExtension::RunTerminated
        )
        || !attempt_matches(&stale_error_entry)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(TerminateRunCommand::from_parts(
        run.id().clone(),
        run.lifecycle_version(),
        note,
        completed_entry,
        terminal_rejection_entry,
        stale_error_entry,
    ))
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::{AttemptFacts, JournalExtension};
    use crate::model::outcome::OutcomeClass;

    #[test]
    fn command_carries_completed_and_concurrent_terminal_facts() {
        let run = crate::operations::test_support::run();
        let completed = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Completed,
            JournalExtension::RunTerminated,
            None,
        );
        let rejected = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Rejected,
            JournalExtension::RunTerminated,
            None,
        );
        let stale = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Error,
            JournalExtension::RunTerminated,
            None,
        );
        assert!(super::command(&run, None, completed, rejected, stale).is_ok());
    }

    #[test]
    fn termination_cannot_add_unowned_attempt_facts() {
        let run = crate::operations::test_support::run();
        let completed = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Completed,
            JournalExtension::RunTerminated,
            Some(AttemptFacts::default()),
        );
        let rejected = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Rejected,
            JournalExtension::RunTerminated,
            None,
        );
        let stale = crate::operations::test_support::draft(
            "run.terminate",
            OutcomeClass::Error,
            JournalExtension::RunTerminated,
            None,
        );
        assert!(super::command(&run, None, completed, rejected, stale).is_err());
    }
}
