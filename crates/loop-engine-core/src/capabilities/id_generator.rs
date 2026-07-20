use crate::model::ids::{EvidenceId, JournalId, RegistrationId, RequestId, RunId};

/// Stable identifier allocation boundary.
pub trait IdGenerator {
    type Error;

    fn registration_id(&self) -> Result<RegistrationId, Self::Error>;
    fn run_id(&self) -> Result<RunId, Self::Error>;
    fn request_id(&self) -> Result<RequestId, Self::Error>;
    fn evidence_id(&self) -> Result<EvidenceId, Self::Error>;
    fn journal_id(&self) -> Result<JournalId, Self::Error>;
}
