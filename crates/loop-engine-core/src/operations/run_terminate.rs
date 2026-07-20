use crate::capabilities::persistence_commands::{CommitStatus, TerminateRunCommand};
use crate::capabilities::run_writer::RunWriter;
use crate::model::annotation::Note;
use crate::model::attempt::JournalExtension;
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::reason::ReasonCode;
use crate::model::run::Run;
use crate::operations::{CommandError, validate_journal};

pub fn execute<W: RunWriter>(
    writer: &W,
    command: TerminateRunCommand,
) -> Result<CommitStatus, W::Error> {
    writer.terminate(command)
}

pub fn command(
    run: &Run,
    note: Option<Note>,
    completed_entry: JournalDraft,
    terminal_or_stale_entry: JournalDraft,
) -> Result<TerminateRunCommand, CommandError> {
    for entry in [&completed_entry, &terminal_or_stale_entry] {
        validate_journal(
            entry,
            run.id(),
            "run.terminate",
            JournalEntryKind::RunTerminated,
        )?;
    }
    let note_matches = |entry: &JournalDraft| match (&note, entry.attempt()) {
        (None, None) => true,
        (expected, Some(attempt)) => attempt.note.as_ref() == expected.as_ref(),
        (Some(_), None) => false,
    };
    if completed_entry.outcome() != OutcomeClass::Completed
        || !matches!(completed_entry.extension(), JournalExtension::RunTerminated)
        || !note_matches(&completed_entry)
        || terminal_or_stale_entry.outcome() != OutcomeClass::Rejected
        || terminal_or_stale_entry.reason().map(|reason| reason.code())
            != Some(ReasonCode::RunLifecycleTerminal)
        || !matches!(
            terminal_or_stale_entry.extension(),
            JournalExtension::RunTerminated
        )
        || !note_matches(&terminal_or_stale_entry)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(TerminateRunCommand {
        run_id: run.id().clone(),
        expected_lifecycle_version: run.lifecycle_version(),
        note,
        completed_entry,
        terminal_or_stale_entry,
    })
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::JournalExtension;
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
        assert!(super::command(&run, None, completed, rejected).is_ok());
    }
}
