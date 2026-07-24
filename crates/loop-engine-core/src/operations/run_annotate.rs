use crate::capabilities::persistence_commands::{AppendAnnotationCommand, AttemptCommit};
use crate::capabilities::run_writer::RunWriter;
use crate::model::annotation::{ActorMetadata, Note};
use crate::model::attempt::{AttemptFacts, JournalExtension};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::run::Run;
use crate::model::version::JournalSequence;
use crate::operations::{CommandError, validate_journal};

pub fn execute<W: RunWriter>(
    writer: &W,
    command: AppendAnnotationCommand,
) -> Result<AttemptCommit, W::Error> {
    writer.append_annotation(command)
}

pub fn command(
    run: &Run,
    note: Option<Note>,
    actor: Option<ActorMetadata>,
    corrects_sequence: Option<JournalSequence>,
    journal_entry: JournalDraft,
) -> Result<Option<AppendAnnotationCommand>, CommandError> {
    if note.is_none() && actor.is_none() && corrects_sequence.is_none() {
        return Ok(None);
    }
    validate_journal(
        &journal_entry,
        run.id(),
        "run.annotate",
        JournalEntryKind::Annotation,
    )?;
    let expected_attempt = AttemptFacts {
        note: note.clone(),
        actor: actor.clone(),
        corrects_sequence,
        ..AttemptFacts::default()
    };
    if journal_entry.outcome() != OutcomeClass::Completed
        || journal_entry.reason().is_some()
        || !matches!(journal_entry.extension(), JournalExtension::Annotation)
        || journal_entry.attempt() != Some(&expected_attempt)
    {
        return Err(CommandError::JournalMismatch);
    }
    Ok(Some(AppendAnnotationCommand::from_parts(
        run.id().clone(),
        note,
        actor,
        corrects_sequence,
        journal_entry,
    )))
}

#[cfg(test)]
mod tests {
    use crate::model::attempt::{AttemptFacts, JournalExtension};
    use crate::model::outcome::OutcomeClass;

    #[test]
    fn empty_annotation_is_noop() {
        let run = crate::operations::test_support::run();
        let draft = crate::operations::test_support::draft(
            "run.annotate",
            OutcomeClass::Completed,
            JournalExtension::Annotation,
            Some(AttemptFacts::default()),
        );
        assert!(
            super::command(&run, None, None, None, draft)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn annotation_cannot_fabricate_rejected_disposition() {
        let run = crate::operations::test_support::run();
        let note = crate::model::annotation::Note::new("audit note").unwrap();
        let draft = crate::operations::test_support::draft(
            "run.annotate",
            OutcomeClass::Rejected,
            JournalExtension::Annotation,
            Some(AttemptFacts {
                note: Some(note.clone()),
                ..AttemptFacts::default()
            }),
        );
        assert!(super::command(&run, Some(note), None, None, draft).is_err());
    }
}
