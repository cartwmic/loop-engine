use crate::capabilities::persistence_commands::{CommitEventAttemptCommand, EventCommitStatus};

/// One atomic compare-and-commit boundary for every event attempt disposition.
pub trait EventAttemptWriter {
    type Error;

    /// Stale commands may append their error attempt but must not apply target state.
    fn commit_event_attempt(
        &self,
        command: CommitEventAttemptCommand,
    ) -> Result<EventCommitStatus, Self::Error>;
}
