//! Operation-scoped aggregate reader for provider-backed run operations.
//!
//! Core's legacy `RunReader` capability spans several read concerns. This adapter keeps
//! concrete SQLite composition in integrations while preserving the calling operation ID in
//! every persistence trace.

use std::path::PathBuf;

use loop_engine_core::capabilities::run_reader::{
    EvidenceInventoryRow, RunListFilter, RunListRow, RunReader, SelectedEvidenceReadError,
};
use loop_engine_core::capabilities::{Page, PageRequest};
use loop_engine_core::model::attempt::ProviderRole;
use loop_engine_core::model::bounded::COLLECTION_PAGE_DATA_BUDGET_BYTES;
use loop_engine_core::model::evidence::EvidenceRecord;
use loop_engine_core::model::ids::{EvidenceId, RunId};
use loop_engine_core::model::journal::JournalEntry;
use loop_engine_core::model::provider::ProviderObservation;
use loop_engine_core::model::run::Run;
use thiserror::Error;

use super::{
    EvidenceReadError, HistoryReadError, OptionalTraceSink, RunReadError, SqliteEvidenceReads,
    SqliteHistoryReads, SqliteRunReads,
};

#[derive(Debug, Clone)]
pub struct SqliteOperationRunReader {
    operation_id: &'static str,
    runs: SqliteRunReads,
    evidence: SqliteEvidenceReads,
    history: SqliteHistoryReads,
}

#[derive(Debug, Error)]
pub enum OperationRunReadError {
    #[error(transparent)]
    Run(#[from] RunReadError),
    #[error(transparent)]
    Evidence(#[from] EvidenceReadError),
    #[error(transparent)]
    History(#[from] HistoryReadError),
    #[error("creation provider observation is missing or corrupt")]
    CreationObservation,
}

impl SqliteOperationRunReader {
    pub fn with_trace(
        path: impl Into<PathBuf>,
        trace: OptionalTraceSink,
        operation_id: &'static str,
    ) -> Self {
        let path = path.into();
        Self {
            operation_id,
            runs: SqliteRunReads::with_trace(path.clone(), trace.clone()),
            evidence: SqliteEvidenceReads::with_trace(path.clone(), trace.clone()),
            history: SqliteHistoryReads::with_trace(path, trace),
        }
    }
}

impl RunReader for SqliteOperationRunReader {
    type Error = OperationRunReadError;

    fn get(&self, run_id: &RunId) -> Result<Run, Self::Error> {
        self.runs
            .get_for_operation(self.operation_id, run_id)
            .map_err(Into::into)
    }

    fn creation_provider_observation(
        &self,
        run_id: &RunId,
    ) -> Result<ProviderObservation, Self::Error> {
        let request = PageRequest::new(1, COLLECTION_PAGE_DATA_BUDGET_BYTES, None, ())
            .expect("single-row history request is bounded");
        let page = self
            .history
            .history_for_operation(self.operation_id, run_id, &request)?;
        let entry = page
            .rows
            .first()
            .ok_or(OperationRunReadError::CreationObservation)?;
        let fact = entry
            .attempt()
            .and_then(|attempt| {
                attempt
                    .provider_observations
                    .iter()
                    .find(|fact| fact.role == ProviderRole::Describe)
            })
            .ok_or(OperationRunReadError::CreationObservation)?;
        ProviderObservation::new(
            fact.registration_id.clone(),
            fact.executable.as_str(),
            fact.digest.clone(),
            fact.provider_version
                .as_ref()
                .map(|version| version.as_str().to_owned()),
            entry.observed_at(),
        )
        .map_err(|_| OperationRunReadError::CreationObservation)
    }

    fn list(&self, request: &PageRequest<RunListFilter>) -> Result<Page<RunListRow>, Self::Error> {
        self.runs.list(request).map_err(Into::into)
    }

    fn evidence(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<EvidenceInventoryRow>, Self::Error> {
        self.evidence
            .inventory_for_operation(self.operation_id, run_id, request)
            .map_err(Into::into)
    }

    fn history(
        &self,
        run_id: &RunId,
        request: &PageRequest<()>,
    ) -> Result<Page<JournalEntry>, Self::Error> {
        self.history
            .history_for_operation(self.operation_id, run_id, request)
            .map_err(Into::into)
    }

    fn selected_evidence(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<Self::Error>> {
        self.evidence
            .selected_evidence_for_operation(self.operation_id, run_id, evidence_ids)
            .map_err(|error| match error {
                SelectedEvidenceReadError::Unavailable => SelectedEvidenceReadError::Unavailable,
                SelectedEvidenceReadError::Read(error) => {
                    SelectedEvidenceReadError::Read(OperationRunReadError::Evidence(error))
                }
            })
    }
}
