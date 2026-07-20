use crate::capabilities::persistence_commands::{
    AppendAnnotationCommand, AppendCompatibilityAttemptCommand, AppendEvidenceCommand,
    AppendGuidanceAttemptCommand, CommitStatus, CreateRunCommand, ReplaceLabelCommand,
    TerminateRunCommand,
};

/// Atomic run/state/journal writes. No method exposes a partial save.
pub trait RunWriter {
    type Error;

    fn create(&self, command: CreateRunCommand) -> Result<CommitStatus, Self::Error>;
    fn append_evidence(&self, command: AppendEvidenceCommand) -> Result<CommitStatus, Self::Error>;
    fn append_annotation(
        &self,
        command: AppendAnnotationCommand,
    ) -> Result<CommitStatus, Self::Error>;
    fn replace_label(&self, command: ReplaceLabelCommand) -> Result<CommitStatus, Self::Error>;
    fn terminate(&self, command: TerminateRunCommand) -> Result<CommitStatus, Self::Error>;
    fn append_guidance_attempt(
        &self,
        command: AppendGuidanceAttemptCommand,
    ) -> Result<CommitStatus, Self::Error>;
    fn append_compatibility_attempt(
        &self,
        command: AppendCompatibilityAttemptCommand,
    ) -> Result<CommitStatus, Self::Error>;
}
