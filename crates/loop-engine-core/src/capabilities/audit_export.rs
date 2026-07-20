use crate::model::bounded::{BoundError, BoundedText, FILESYSTEM_PATH_UTF8_BYTES};
use crate::model::evidence::EvidenceRecord;
use crate::model::journal::JournalEntry;
use crate::model::provider::ProviderObservation;
use crate::model::run::Run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportTarget(BoundedText<FILESYSTEM_PATH_UTF8_BYTES>);

impl ExportTarget {
    pub fn parse(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self(BoundedText::opaque_non_empty("export_target", value)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSnapshot {
    pub run: Run,
    pub journal: Vec<JournalEntry>,
    pub evidence: Vec<EvidenceRecord>,
    pub provider_observations: Vec<ProviderObservation>,
}

pub trait AuditExporter {
    type Error;

    fn export_consistent(
        &self,
        run_id: &crate::model::ids::RunId,
        target: &ExportTarget,
    ) -> Result<AuditSnapshot, Self::Error>;
}
