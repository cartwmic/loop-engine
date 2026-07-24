use crate::capabilities::persistence_commands::{
    AppendAnnotationCommand, AppendCompatibilityAttemptCommand, AppendEvidenceCommand,
    AppendGuidanceAttemptCommand, AttemptCommit, CommitStatus, CreateRunCommand, LabelCommit,
    ReplaceLabelCommand, TerminateCommit, TerminateRunCommand,
};

/// Narrow journal-only writer used by `run.guidance`.
pub trait GuidanceAttemptWriter {
    type Error;

    fn append_guidance_attempt(
        &self,
        command: AppendGuidanceAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error>;
}

/// Narrow journal-only writer used by `run.compatibility`.
pub trait CompatibilityAttemptWriter {
    type Error;

    fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error>;
}

/// Atomic run/state/journal writes. No method exposes a partial save.
pub trait RunWriter {
    type Error;

    fn create(&self, command: CreateRunCommand) -> Result<CommitStatus, Self::Error>;
    fn append_evidence(&self, command: AppendEvidenceCommand)
    -> Result<AttemptCommit, Self::Error>;
    fn append_annotation(
        &self,
        command: AppendAnnotationCommand,
    ) -> Result<AttemptCommit, Self::Error>;
    fn replace_label(&self, command: ReplaceLabelCommand) -> Result<LabelCommit, Self::Error>;
    fn terminate(&self, command: TerminateRunCommand) -> Result<TerminateCommit, Self::Error>;
    fn append_guidance_attempt(
        &self,
        command: AppendGuidanceAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error>;
    fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error>;
}

impl<T: RunWriter> GuidanceAttemptWriter for T {
    type Error = T::Error;

    fn append_guidance_attempt(
        &self,
        command: AppendGuidanceAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error> {
        RunWriter::append_guidance_attempt(self, command)
    }
}

impl<T: RunWriter> CompatibilityAttemptWriter for T {
    type Error = T::Error;

    fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<AttemptCommit, Self::Error> {
        RunWriter::append_compatibility_attempt(self, command)
    }
}
