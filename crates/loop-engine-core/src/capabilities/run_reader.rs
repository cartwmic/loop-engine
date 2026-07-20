use crate::capabilities::{Page, PageRequest};
use crate::model::evidence::{EvidenceAssociation, EvidenceRecord};
use crate::model::ids::{EvidenceId, RunId};
use crate::model::journal::JournalEntry;
use crate::model::lifecycle::Lifecycle;
use crate::model::provider::ProviderObservation;
use crate::model::run::Run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunListFilter {
    Active,
    /// Includes both final and terminated lifecycle rows.
    Terminal,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListRow {
    pub run_id: RunId,
    pub label: Option<String>,
    pub lifecycle: Lifecycle,
    pub current_state: crate::model::ids::StateId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedEvidenceReadError<E> {
    /// One or more selected IDs are absent from this run.
    Unavailable,
    /// Persistence/read infrastructure failed.
    Read(E),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceInventoryRow {
    pub record: EvidenceRecord,
    pub associations: Vec<EvidenceAssociation>,
}

pub trait RunReader {
    type Error;

    fn get(&self, run_id: &RunId) -> Result<Run, Self::Error>;
    fn creation_provider_observation(
        &self,
        run_id: &RunId,
    ) -> Result<ProviderObservation, Self::Error>;
    fn list(&self, request: &PageRequest<RunListFilter>) -> Result<Page<RunListRow>, Self::Error>;
    fn evidence(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<EvidenceInventoryRow>, Self::Error>;
    /// Returns immutable journal rows in ascending per-run sequence order.
    fn history(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, Self::Error>;
    fn selected_evidence(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<Self::Error>>;
}
