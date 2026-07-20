use crate::model::annotation::{ActorMetadata, Note};
use crate::model::attempt::{JournalExtension, LabelChangeFact};
use crate::model::bounded::{BoundedText, RUN_LABEL_UTF8_BYTES};
use crate::model::evidence::{EvidenceAssociation, EvidenceRecord};
use crate::model::ids::{RunId, StateId};
use crate::model::journal::{JournalDraft, JournalError};
use crate::model::lifecycle::Lifecycle;
use crate::model::run::Run;
use crate::model::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRunCommand {
    pub run: Run,
    /// Catalog revision resolved before provider invocation and rechecked in transaction.
    pub expected_config_revision: u64,
    pub creation_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEvidenceCommand {
    pub run_id: RunId,
    /// `None` for a rejected post-lookup attempt that appends journal only.
    pub evidence: Option<EvidenceRecord>,
    pub journal_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAnnotationCommand {
    pub run_id: RunId,
    pub note: Option<Note>,
    pub actor: Option<ActorMetadata>,
    pub corrects_sequence: Option<JournalSequence>,
    pub journal_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceLabelCommand {
    run_id: RunId,
    label: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
    /// Metadata template materialized with authoritative `label_before` in transaction.
    completed_entry_template: JournalDraft,
    /// Selected when lifecycle is terminal inside the write transaction.
    terminal_rejection_entry: JournalDraft,
}

impl ReplaceLabelCommand {
    pub(crate) fn from_parts(
        run_id: RunId,
        label: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
        completed_entry_template: JournalDraft,
        terminal_rejection_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            label,
            completed_entry_template,
            terminal_rejection_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_ref().map(BoundedText::as_str)
    }

    /// Persistence calls this only after its authoritative lifecycle/label re-read.
    pub fn into_transaction_parts(
        self,
        label_before: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
    ) -> Result<
        (
            RunId,
            Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
            JournalDraft,
            JournalDraft,
        ),
        JournalError,
    > {
        let completed =
            self.completed_entry_template
                .replacing_extension(JournalExtension::LabelChanged {
                    change: Some(LabelChangeFact {
                        label_before,
                        label_after: self.label.clone(),
                    }),
                })?;
        Ok((
            self.run_id,
            self.label,
            completed,
            self.terminal_rejection_entry,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminateRunCommand {
    pub run_id: RunId,
    pub expected_lifecycle_version: LifecycleVersion,
    pub note: Option<Note>,
    pub completed_entry: JournalDraft,
    pub terminal_or_stale_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendGuidanceAttemptCommand {
    pub run_id: RunId,
    pub expected_lifecycle_version: LifecycleVersion,
    pub journal_entry: JournalDraft,
    pub terminal_rejection_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendCompatibilityAttemptCommand {
    pub run_id: RunId,
    pub expected_lifecycle_version: LifecycleVersion,
    /// `None` means one side of digest observation was unavailable.
    pub observed_drift: Option<bool>,
    pub journal_entry: JournalDraft,
    pub terminal_rejection_entry: JournalDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEventAttemptCommand {
    parts: EventAttemptParts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAttemptParts {
    pub run_id: RunId,
    pub expected_workflow_version: WorkflowStateVersion,
    pub expected_lifecycle_version: LifecycleVersion,
    pub source_state: StateId,
    pub target_state: Option<StateId>,
    pub target_lifecycle: Option<Lifecycle>,
    pub inline_evidence: Vec<EvidenceRecord>,
    pub associations: Vec<EvidenceAssociation>,
    pub provider_evidence: Vec<EvidenceRecord>,
    pub journal_entry: JournalDraft,
    pub stale_journal_entry: JournalDraft,
}

impl CommitEventAttemptCommand {
    pub(crate) fn from_parts(parts: EventAttemptParts) -> Self {
        Self { parts }
    }

    pub fn into_parts(self) -> EventAttemptParts {
        self.parts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCommitBranch {
    ExpectedVersions,
    StaleVersions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCommitStatus {
    pub commit: CommitStatus,
    pub branch: EventCommitBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitStatus {
    pub committed: bool,
    pub state_changed: bool,
    pub workflow_state_version: WorkflowStateVersion,
    pub lifecycle_version: LifecycleVersion,
}
