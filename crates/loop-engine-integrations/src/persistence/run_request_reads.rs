use loop_engine_core::capabilities::run_reader::{RunRequestReader, SelectedEvidenceReadError};
use loop_engine_core::model::evidence::EvidenceRecord;
use loop_engine_core::model::ids::{EvidenceId, RunId};
use loop_engine_core::model::run::Run;
use thiserror::Error;

use super::{EvidenceReadError, RunReadError, SqliteEvidenceReads, SqliteRunReads};

/// Private one-operation adapter joining authoritative run and evidence reads for `run.request`.
pub struct SqliteRunRequestReader {
    runs: SqliteRunReads,
    evidence: SqliteEvidenceReads,
}

#[derive(Debug, Error)]
pub enum RunRequestReadError {
    #[error(transparent)]
    Run(#[from] RunReadError),
    #[error(transparent)]
    Evidence(#[from] EvidenceReadError),
}

impl SqliteRunRequestReader {
    pub fn new(runs: SqliteRunReads, evidence: SqliteEvidenceReads) -> Self {
        Self { runs, evidence }
    }
}

impl RunRequestReader for SqliteRunRequestReader {
    type Error = RunRequestReadError;

    fn get(&self, run_id: &RunId) -> Result<Run, Self::Error> {
        self.runs
            .get_for_operation("run.request", run_id)
            .map_err(RunRequestReadError::Run)
    }

    fn selected_evidence(
        &self,
        run_id: &RunId,
        evidence_ids: &[EvidenceId],
    ) -> Result<Vec<EvidenceRecord>, SelectedEvidenceReadError<Self::Error>> {
        self.evidence
            .selected_evidence_for_operation("run.request", run_id, evidence_ids)
            .map_err(|error| match error {
                SelectedEvidenceReadError::Unavailable => SelectedEvidenceReadError::Unavailable,
                SelectedEvidenceReadError::Read(error) => {
                    SelectedEvidenceReadError::Read(RunRequestReadError::Evidence(error))
                }
            })
    }
}
