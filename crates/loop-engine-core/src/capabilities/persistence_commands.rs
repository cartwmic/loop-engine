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
    run: Run,
    /// Catalog revision resolved before provider invocation and rechecked in transaction.
    expected_config_revision: u64,
    creation_entry: JournalDraft,
}

impl CreateRunCommand {
    pub(crate) fn from_parts(
        run: Run,
        expected_config_revision: u64,
        creation_entry: JournalDraft,
    ) -> Self {
        Self {
            run,
            expected_config_revision,
            creation_entry,
        }
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn expected_config_revision(&self) -> u64 {
        self.expected_config_revision
    }

    pub fn creation_entry(&self) -> &JournalDraft {
        &self.creation_entry
    }

    pub fn into_parts(self) -> (Run, u64, JournalDraft) {
        (self.run, self.expected_config_revision, self.creation_entry)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(run: Run, expected_config_revision: u64, creation_entry: JournalDraft) -> Self {
        Self::from_parts(run, expected_config_revision, creation_entry)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_expected_config_revision(mut self, expected_config_revision: u64) -> Self {
        self.expected_config_revision = expected_config_revision;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEvidenceCommand {
    run_id: RunId,
    /// `None` for a pre-resolved rejection that appends journal only.
    evidence: Option<EvidenceRecord>,
    /// Selected when the evidence id is fresh at authoritative read.
    completed_entry: JournalDraft,
    /// Selected when the evidence id already exists at authoritative read.
    duplicate_rejection_entry: JournalDraft,
}

impl AppendEvidenceCommand {
    pub(crate) fn from_dual_disposition(
        run_id: RunId,
        evidence: EvidenceRecord,
        completed_entry: JournalDraft,
        duplicate_rejection_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            evidence: Some(evidence),
            completed_entry,
            duplicate_rejection_entry,
        }
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn from_pre_resolved_rejection(run_id: RunId, journal_entry: JournalDraft) -> Self {
        Self {
            run_id,
            evidence: None,
            completed_entry: journal_entry.clone(),
            duplicate_rejection_entry: journal_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn evidence(&self) -> Option<&EvidenceRecord> {
        self.evidence.as_ref()
    }

    pub fn completed_entry(&self) -> &JournalDraft {
        &self.completed_entry
    }

    pub fn duplicate_rejection_entry(&self) -> &JournalDraft {
        &self.duplicate_rejection_entry
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAnnotationCommand {
    run_id: RunId,
    note: Option<Note>,
    actor: Option<ActorMetadata>,
    corrects_sequence: Option<JournalSequence>,
    journal_entry: JournalDraft,
}

impl AppendAnnotationCommand {
    pub(crate) fn from_parts(
        run_id: RunId,
        note: Option<Note>,
        actor: Option<ActorMetadata>,
        corrects_sequence: Option<JournalSequence>,
        journal_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            note,
            actor,
            corrects_sequence,
            journal_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn journal_entry(&self) -> &JournalDraft {
        &self.journal_entry
    }

    pub fn into_parts(self) -> (RunId, Option<JournalSequence>, JournalDraft) {
        (self.run_id, self.corrects_sequence, self.journal_entry)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        run_id: RunId,
        note: Option<Note>,
        actor: Option<ActorMetadata>,
        corrects_sequence: Option<JournalSequence>,
        journal_entry: JournalDraft,
    ) -> Self {
        Self::from_parts(run_id, note, actor, corrects_sequence, journal_entry)
    }
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
    run_id: RunId,
    expected_lifecycle_version: LifecycleVersion,
    note: Option<Note>,
    completed_entry: JournalDraft,
    terminal_or_stale_entry: JournalDraft,
}

impl TerminateRunCommand {
    pub(crate) fn from_parts(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        note: Option<Note>,
        completed_entry: JournalDraft,
        terminal_or_stale_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            expected_lifecycle_version,
            note,
            completed_entry,
            terminal_or_stale_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn expected_lifecycle_version(&self) -> LifecycleVersion {
        self.expected_lifecycle_version
    }

    pub fn into_parts(self) -> (RunId, LifecycleVersion, JournalDraft, JournalDraft) {
        (
            self.run_id,
            self.expected_lifecycle_version,
            self.completed_entry,
            self.terminal_or_stale_entry,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        note: Option<Note>,
        completed_entry: JournalDraft,
        terminal_or_stale_entry: JournalDraft,
    ) -> Self {
        Self::from_parts(
            run_id,
            expected_lifecycle_version,
            note,
            completed_entry,
            terminal_or_stale_entry,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_terminal_or_stale_entry(mut self, terminal_or_stale_entry: JournalDraft) -> Self {
        self.terminal_or_stale_entry = terminal_or_stale_entry;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendGuidanceAttemptCommand {
    run_id: RunId,
    expected_lifecycle_version: LifecycleVersion,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
}

impl AppendGuidanceAttemptCommand {
    pub(crate) fn from_parts(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        journal_entry: JournalDraft,
        terminal_rejection_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            expected_lifecycle_version,
            journal_entry,
            terminal_rejection_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn expected_lifecycle_version(&self) -> LifecycleVersion {
        self.expected_lifecycle_version
    }

    pub fn journal_entry(&self) -> &JournalDraft {
        &self.journal_entry
    }

    pub fn terminal_rejection_entry(&self) -> &JournalDraft {
        &self.terminal_rejection_entry
    }

    pub fn into_parts(self) -> (RunId, LifecycleVersion, JournalDraft, JournalDraft) {
        (
            self.run_id,
            self.expected_lifecycle_version,
            self.journal_entry,
            self.terminal_rejection_entry,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        journal_entry: JournalDraft,
        terminal_rejection_entry: JournalDraft,
    ) -> Self {
        Self::from_parts(
            run_id,
            expected_lifecycle_version,
            journal_entry,
            terminal_rejection_entry,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendCompatibilityAttemptCommand {
    run_id: RunId,
    expected_lifecycle_version: LifecycleVersion,
    /// `None` means one side of digest observation was unavailable.
    observed_drift: Option<bool>,
    journal_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
}

impl AppendCompatibilityAttemptCommand {
    pub(crate) fn from_parts(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        observed_drift: Option<bool>,
        journal_entry: JournalDraft,
        terminal_rejection_entry: JournalDraft,
    ) -> Self {
        Self {
            run_id,
            expected_lifecycle_version,
            observed_drift,
            journal_entry,
            terminal_rejection_entry,
        }
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn expected_lifecycle_version(&self) -> LifecycleVersion {
        self.expected_lifecycle_version
    }

    pub fn observed_drift(&self) -> Option<bool> {
        self.observed_drift
    }

    pub fn journal_entry(&self) -> &JournalDraft {
        &self.journal_entry
    }

    pub fn terminal_rejection_entry(&self) -> &JournalDraft {
        &self.terminal_rejection_entry
    }

    pub fn into_parts(
        self,
    ) -> (
        RunId,
        LifecycleVersion,
        Option<bool>,
        JournalDraft,
        JournalDraft,
    ) {
        (
            self.run_id,
            self.expected_lifecycle_version,
            self.observed_drift,
            self.journal_entry,
            self.terminal_rejection_entry,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        run_id: RunId,
        expected_lifecycle_version: LifecycleVersion,
        observed_drift: Option<bool>,
        journal_entry: JournalDraft,
        terminal_rejection_entry: JournalDraft,
    ) -> Self {
        Self::from_parts(
            run_id,
            expected_lifecycle_version,
            observed_drift,
            journal_entry,
            terminal_rejection_entry,
        )
    }
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
    InlineEvidenceConflict,
    ProviderEvidenceConflict,
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

#[cfg(test)]
mod encapsulation_contract {
    use super::{
        AppendAnnotationCommand, AppendCompatibilityAttemptCommand, AppendGuidanceAttemptCommand,
        CreateRunCommand, TerminateRunCommand,
    };
    use crate::model::annotation::{ActorMetadata, Note};
    use crate::model::ids::RunId;
    use crate::model::journal::JournalDraft;
    use crate::model::run::Run;
    use crate::model::version::{JournalSequence, LifecycleVersion};

    #[test]
    #[allow(clippy::type_complexity)]
    fn core_builders_can_name_private_constructors() {
        let _: fn(Run, u64, JournalDraft) -> CreateRunCommand = CreateRunCommand::from_parts;
        let _: fn(
            RunId,
            Option<Note>,
            Option<ActorMetadata>,
            Option<JournalSequence>,
            JournalDraft,
        ) -> AppendAnnotationCommand = AppendAnnotationCommand::from_parts;
        let _: fn(
            RunId,
            LifecycleVersion,
            Option<Note>,
            JournalDraft,
            JournalDraft,
        ) -> TerminateRunCommand = TerminateRunCommand::from_parts;
        let _: fn(
            RunId,
            LifecycleVersion,
            JournalDraft,
            JournalDraft,
        ) -> AppendGuidanceAttemptCommand = AppendGuidanceAttemptCommand::from_parts;
        let _: fn(
            RunId,
            LifecycleVersion,
            Option<bool>,
            JournalDraft,
            JournalDraft,
        ) -> AppendCompatibilityAttemptCommand = AppendCompatibilityAttemptCommand::from_parts;
    }

    #[test]
    fn production_surface_exposes_read_and_into_parts_only() {
        fn inspect_create(command: &CreateRunCommand) {
            let _ = command.run();
            let _ = command.expected_config_revision();
            let _ = command.creation_entry();
        }
        fn consume_create(command: CreateRunCommand) {
            let _ = command.into_parts();
        }
        fn inspect_annotation(command: &AppendAnnotationCommand) {
            let _ = command.run_id();
            let _ = command.journal_entry();
        }
        fn consume_annotation(command: AppendAnnotationCommand) {
            let _ = command.into_parts();
        }
        fn inspect_terminate(command: &TerminateRunCommand) {
            let _ = command.run_id();
            let _ = command.expected_lifecycle_version();
        }
        fn consume_terminate(command: TerminateRunCommand) {
            let _ = command.into_parts();
        }
        fn inspect_guidance(command: &AppendGuidanceAttemptCommand) {
            let _ = command.run_id();
            let _ = command.journal_entry();
            let _ = command.terminal_rejection_entry();
        }
        fn consume_guidance(command: AppendGuidanceAttemptCommand) {
            let _ = command.into_parts();
        }
        fn inspect_compatibility(command: &AppendCompatibilityAttemptCommand) {
            let _ = command.run_id();
            let _ = command.observed_drift();
            let _ = command.journal_entry();
        }
        fn consume_compatibility(command: AppendCompatibilityAttemptCommand) {
            let _ = command.into_parts();
        }
        let _ = (
            inspect_create,
            consume_create,
            inspect_annotation,
            consume_annotation,
            inspect_terminate,
            consume_terminate,
            inspect_guidance,
            consume_guidance,
            inspect_compatibility,
            consume_compatibility,
        );
    }
}
