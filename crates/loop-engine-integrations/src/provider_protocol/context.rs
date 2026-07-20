use std::collections::BTreeMap;

use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::ids::{EvidenceId, EvidenceKind};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::time::ObservedAt;

use super::canonical::{graph_dto, metadata_value, value_from_core};
use super::describe::observed_now;
use super::dto::{EvidenceDto, RunSnapshotDto};
use super::mapping::{MappingError, metadata};

pub fn run_snapshot(run: &Run) -> RunSnapshotDto {
    let validated = ValidatedGraph::validate(run.graph().clone())
        .expect("stored run graph remains semantically valid");
    let projection = SemanticGraphProjection::from_validated(&validated);
    RunSnapshotDto {
        run_id: run.id().as_str().to_owned(),
        registration_id: run.registration_id().as_str().to_owned(),
        graph_revision: run.graph_revision().as_str().to_owned(),
        lifecycle: match run.lifecycle() {
            Lifecycle::Active => "active",
            Lifecycle::Final => "final",
            Lifecycle::Terminated => "terminated",
        }
        .to_owned(),
        current_state: run.current_state().as_str().to_owned(),
        workflow_state_version: run.workflow_state_version().value(),
        lifecycle_version: run.lifecycle_version().value(),
        inputs: run
            .inputs()
            .values()
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value_from_core(value)))
            .collect::<BTreeMap<_, _>>(),
        stored_graph: graph_dto(&projection),
    }
}

pub fn evidence_dto(record: &EvidenceRecord) -> EvidenceDto {
    EvidenceDto {
        id: record.id().as_str().to_owned(),
        kind: record.kind().as_str().to_owned(),
        locator: record.locator().to_owned(),
        digest: record.digest().map(str::to_owned),
        media_type: record.media_type().map(str::to_owned),
        metadata: record.metadata().map(metadata_value),
        observed_at: Some(record.observed_at().as_timestamp().to_string()),
    }
}

pub fn provider_evidence(value: EvidenceDto, path: &str) -> Result<EvidenceRecord, MappingError> {
    let observed_at = match value.observed_at {
        Some(value) => ObservedAt::parse(&value)
            .map_err(|error| MappingError::field(format!("{path}/observed_at"), error))?,
        None => observed_now(),
    };
    EvidenceRecord::new(
        EvidenceId::parse(value.id)
            .map_err(|error| MappingError::field(format!("{path}/id"), error))?,
        EvidenceKind::parse(value.kind)
            .map_err(|error| MappingError::field(format!("{path}/kind"), error))?,
        value.locator,
        value.digest,
        value.media_type,
        metadata(value.metadata, &format!("{path}/metadata"))?,
        EvidenceSource::Provider,
        observed_at,
    )
    .map_err(|error| MappingError::field(path, error))
}
