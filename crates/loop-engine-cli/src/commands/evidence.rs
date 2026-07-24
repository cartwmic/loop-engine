//! Private evidence command adapters (WP1 T122/T131).
//!
//! Delivery-layer syntax-to-core mapping and one-operation-per-intent adapters.
//! Rendering, route registration, and concrete integration construction live elsewhere.

use loop_engine_core::capabilities::persistence_commands::{AttemptCommit, CommittedRunSnapshot};
use loop_engine_core::capabilities::run_reader::{EvidenceInventoryReader, EvidenceInventoryRow};
use loop_engine_core::capabilities::run_writer::RunWriter;
use loop_engine_core::capabilities::{Page, PageRequest};
use loop_engine_core::model::bounded::{BoundError, Metadata};
use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::ids::{EvidenceId, EvidenceKind, IdentifierError, RunId};
use loop_engine_core::model::journal::JournalDraft;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::reason::Reason;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::version::{LifecycleVersion, WorkflowStateVersion};
use loop_engine_core::operations::CommandError;
use loop_engine_core::operations::evidence_add;
use loop_engine_core::operations::evidence_list;
use loop_engine_core::operations::paging::PagingError;
use thiserror::Error;

use crate::args::{
    RunEvidenceAddParsed, RunEvidenceListParsed, SyntaxOpaqueWire, SyntaxPageLimit, SyntaxPath,
    SyntaxText,
};

/// Bounded conversion failures at the CLI delivery boundary (pre-dispatch, not domain rejection).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvidenceMapError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
    #[error(transparent)]
    Paging(#[from] PagingError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAddRequest {
    pub run_id: RunId,
    pub kind: EvidenceKind,
    pub locator: String,
    pub digest: Option<String>,
    pub media_type: Option<String>,
    pub metadata_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceListRequest {
    pub run_id: RunId,
    pub page_request: PageRequest<()>,
}

/// Composition-resolved append inputs after identifier generation, metadata delivery, and journal drafting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAddResolved {
    pub evidence: EvidenceRecord,
    pub completed_entry: JournalDraft,
    pub duplicate_rejection_entry: JournalDraft,
}

fn optional_opaque_wire(cursor: Option<&SyntaxOpaqueWire>) -> Option<String> {
    cursor.map(|value| value.as_str().to_string())
}

fn page_limit(limit: SyntaxPageLimit) -> u16 {
    limit.get()
}

fn opaque_locator(reference: &SyntaxText) -> String {
    reference.as_str().to_string()
}

fn optional_delivery_path(path: Option<&SyntaxPath>) -> Option<String> {
    path.map(|value| value.as_str().to_string())
}

pub fn map_add_request(
    parsed: &RunEvidenceAddParsed,
) -> Result<EvidenceAddRequest, EvidenceMapError> {
    Ok(EvidenceAddRequest {
        run_id: RunId::parse(parsed.run_id.as_str())?,
        kind: EvidenceKind::parse(parsed.kind.as_str())?,
        locator: opaque_locator(&parsed.reference),
        digest: parsed
            .digest
            .as_ref()
            .map(|value| value.as_str().to_string()),
        media_type: parsed
            .media_type
            .as_ref()
            .map(|value| value.as_str().to_string()),
        metadata_file: optional_delivery_path(parsed.metadata.as_ref()),
    })
}

pub fn map_list_request(
    parsed: &RunEvidenceListParsed,
) -> Result<EvidenceListRequest, EvidenceMapError> {
    Ok(EvidenceListRequest {
        run_id: RunId::parse(parsed.run_id.as_str())?,
        page_request: evidence_list::query(
            Some(page_limit(parsed.limit)),
            optional_opaque_wire(parsed.cursor.as_ref()),
        )?,
    })
}

pub fn map_evidence_record(
    request: &EvidenceAddRequest,
    evidence_id: EvidenceId,
    observed_at: ObservedAt,
    metadata: Option<Metadata>,
) -> Result<EvidenceRecord, BoundError> {
    EvidenceRecord::new(
        evidence_id,
        request.kind.clone(),
        request.locator.clone(),
        request.digest.clone(),
        request.media_type.clone(),
        metadata,
        EvidenceSource::Caller,
        observed_at,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAssociationView {
    pub evidence_id: String,
    pub event_id: Option<String>,
    pub gate_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecordView {
    pub id: String,
    pub kind: String,
    pub locator: String,
    pub digest: Option<String>,
    pub media_type: Option<String>,
    pub source: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceInventoryRowView {
    pub record: EvidenceRecordView,
    pub associations: Vec<EvidenceAssociationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceListOutcome {
    pub items: Vec<EvidenceInventoryRowView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAddOutcome {
    pub committed: bool,
    pub state_changed: bool,
    pub workflow_state_version: WorkflowStateVersion,
    pub lifecycle_version: LifecycleVersion,
    pub outcome: OutcomeClass,
    pub reason: Option<Reason>,
    pub run: CommittedRunSnapshot,
}

impl From<AttemptCommit> for EvidenceAddOutcome {
    fn from(attempt: AttemptCommit) -> Self {
        Self {
            committed: attempt.commit.committed,
            state_changed: attempt.commit.state_changed,
            workflow_state_version: attempt.commit.workflow_state_version,
            lifecycle_version: attempt.commit.lifecycle_version,
            outcome: attempt.outcome,
            reason: attempt.reason,
            run: attempt.run,
        }
    }
}

#[derive(Debug, Error)]
pub enum EvidenceAddError<E> {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Writer(E),
}

fn source_label(source: EvidenceSource) -> &'static str {
    match source {
        EvidenceSource::Caller => "caller",
        EvidenceSource::Provider => "provider",
    }
}

fn map_record_view(record: &EvidenceRecord) -> EvidenceRecordView {
    EvidenceRecordView {
        id: record.id().as_str().to_string(),
        kind: record.kind().as_str().to_string(),
        locator: record.locator().to_string(),
        digest: record.digest().map(str::to_string),
        media_type: record.media_type().map(str::to_string),
        source: source_label(record.source()).to_string(),
        observed_at: format!("{:?}", record.observed_at()),
    }
}

fn map_inventory_row(row: &EvidenceInventoryRow) -> EvidenceInventoryRowView {
    EvidenceInventoryRowView {
        record: map_record_view(&row.record),
        associations: row
            .associations
            .iter()
            .map(|association| EvidenceAssociationView {
                evidence_id: association.evidence_id().as_str().to_string(),
                event_id: association
                    .event_id()
                    .map(|event_id| event_id.as_str().to_string()),
                gate_id: association
                    .gate_id()
                    .map(|gate_id| gate_id.as_str().to_string()),
            })
            .collect(),
    }
}

fn map_evidence_page(page: Page<EvidenceInventoryRow>) -> EvidenceListOutcome {
    EvidenceListOutcome {
        items: page.rows.iter().map(map_inventory_row).collect(),
        next_cursor: page
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_string()),
    }
}

pub fn add<W: RunWriter>(
    writer: &W,
    run: &Run,
    resolved: EvidenceAddResolved,
) -> Result<EvidenceAddOutcome, EvidenceAddError<W::Error>> {
    let command = evidence_add::command(
        run,
        resolved.evidence,
        resolved.completed_entry,
        resolved.duplicate_rejection_entry,
    )?;
    evidence_add::execute(writer, command)
        .map(EvidenceAddOutcome::from)
        .map_err(EvidenceAddError::Writer)
}

pub fn list<R: EvidenceInventoryReader>(
    reader: &R,
    request: &EvidenceListRequest,
) -> Result<EvidenceListOutcome, R::Error> {
    evidence_list::execute(reader, &request.run_id, &request.page_request).map(map_evidence_page)
}
