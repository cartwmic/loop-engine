//! Private run command adapters (WP1 T122/T130–T132).
//!
//! Delivery-layer syntax-to-core mapping and one-operation-per-intent adapters.
//! Rendering, route registration, journal fabrication, and concrete integration
//! construction live elsewhere.

use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::event_attempt_writer::EventAttemptWriter;
use loop_engine_core::capabilities::id_generator::IdGenerator;
use loop_engine_core::capabilities::persistence_commands::{
    AppendAnnotationCommand, CommitStatus, ReplaceLabelCommand, TerminateCommit,
    TerminateRunCommand,
};
use loop_engine_core::capabilities::provider_catalog::ProviderCatalog;
use loop_engine_core::capabilities::provider_invoker::ProviderInvoker;
use loop_engine_core::capabilities::run_reader::{
    RunCatalogReader, RunHistoryReader, RunListFilter, RunListRow, RunReader,
};
use loop_engine_core::capabilities::run_writer::RunWriter;
use loop_engine_core::capabilities::{Page, PageRequest};
use loop_engine_core::model::annotation::{ActorMetadata, Note};
use loop_engine_core::model::bounded::BoundError;
use loop_engine_core::model::evidence::EvidenceRecord;
use loop_engine_core::model::ids::{EventId, EvidenceId, IdentifierError, RegistrationId, RunId};
use loop_engine_core::model::journal::{JournalDraft, JournalEntry};
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::RunInputs;
use loop_engine_core::model::version::JournalSequence;
use loop_engine_core::operations::CommandError;
use loop_engine_core::operations::paging::PagingError;
use loop_engine_core::operations::run_annotate;
use loop_engine_core::operations::run_compatibility::{
    self, CompatibilityExecutionError, CompatibilityResolution,
};
use loop_engine_core::operations::run_create::{self, RunCreateExecution, RunCreateExecutionError};
use loop_engine_core::operations::run_graph::{self, StoredGraph};
use loop_engine_core::operations::run_guidance::{
    self, GuidanceExecutionError, GuidanceResolution,
};
use loop_engine_core::operations::run_history;
use loop_engine_core::operations::run_label::{self, LabelError};
use loop_engine_core::operations::run_list;
use loop_engine_core::operations::run_request::{
    self, RequestExecution, RequestExecutionError, RequestResolution,
};
use loop_engine_core::operations::run_show::{self, RunShow};
use loop_engine_core::operations::run_terminate;
use thiserror::Error;

use crate::args::{
    RunAnnotateParsed, RunCompatibilityParsed, RunCreateParsed, RunGraphParsed, RunGuidanceParsed,
    RunHistoryParsed, RunLabelMode, RunLabelParsed, RunListParsed, RunRequestParsed, RunShowParsed,
    RunTerminateParsed, SyntaxIdentifier, SyntaxOpaqueWire, SyntaxPageLimit, SyntaxPositiveU64,
    SyntaxText,
};
use crate::commands::provider::{ProviderMapError, ProviderTargetRef, map_target};

/// Bounded conversion failures at the CLI delivery boundary (pre-dispatch, not domain rejection).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunMapError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
    #[error(transparent)]
    Paging(#[from] PagingError),
    #[error(transparent)]
    Label(#[from] LabelError),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    ProviderTarget(#[from] ProviderMapError),
    #[error("corrects sequence must be positive")]
    InvalidCorrectsSequence,
    #[error("evidence-id values must be unique")]
    DuplicateEvidenceIds,
}

fn opaque_wire(cursor: &SyntaxOpaqueWire) -> String {
    cursor.as_str().to_string()
}

fn optional_opaque_wire(cursor: Option<&SyntaxOpaqueWire>) -> Option<String> {
    cursor.map(opaque_wire)
}

fn page_limit(limit: SyntaxPageLimit) -> u16 {
    limit.get()
}

fn map_run_id(id: &SyntaxIdentifier) -> Result<RunId, RunMapError> {
    Ok(RunId::parse(id.as_str())?)
}

fn map_event_id(id: &SyntaxIdentifier) -> Result<EventId, RunMapError> {
    Ok(EventId::parse(id.as_str())?)
}

fn map_evidence_ids(ids: &[SyntaxIdentifier]) -> Result<Vec<EvidenceId>, RunMapError> {
    let parsed = ids
        .iter()
        .map(|id| EvidenceId::parse(id.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    if !run_request::validate_evidence_selection(&parsed) {
        return Err(RunMapError::DuplicateEvidenceIds);
    }
    Ok(parsed)
}

fn map_optional_note(note: &Option<SyntaxText>) -> Result<Option<Note>, RunMapError> {
    note.as_ref()
        .map(|value| Note::new(value.as_str()))
        .transpose()
        .map_err(RunMapError::Bound)
}

fn map_journal_sequence(
    value: Option<&SyntaxPositiveU64>,
) -> Result<Option<JournalSequence>, RunMapError> {
    match value {
        None => Ok(None),
        Some(raw) => JournalSequence::try_from(raw.get())
            .map(Some)
            .map_err(|_| RunMapError::InvalidCorrectsSequence),
    }
}

pub fn list_filter(terminal: bool, all: bool) -> RunListFilter {
    if all {
        RunListFilter::All
    } else if terminal {
        RunListFilter::Terminal
    } else {
        RunListFilter::Active
    }
}

// --- Delivery / request DTOs (core-aligned) ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCreateDelivery {
    pub target: ProviderTargetRef,
    pub label: Option<String>,
    /// Optional run-inputs document path; composition loads [`RunInputs`] from this delivery input.
    pub inputs_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCreateRequest {
    pub registration_id: RegistrationId,
    pub label: Option<String>,
    pub inputs: RunInputs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListRequest {
    pub filter: RunListFilter,
    pub limit: u16,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHistoryRequest {
    pub run_id: RunId,
    pub limit: u16,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAnnotateDelivery {
    pub run_id: RunId,
    pub note: Option<Note>,
    /// Optional actor-metadata document path; composition loads [`ActorMetadata`] from this delivery input.
    pub actor_path: Option<String>,
    pub corrects_sequence: Option<JournalSequence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLabelDelivery {
    pub run_id: RunId,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequestDelivery {
    pub run_id: RunId,
    pub event: EventId,
    pub selected_evidence_ids: Vec<EvidenceId>,
    /// Optional inline-evidence document path; composition loads records from this delivery input.
    pub inline_evidence_path: Option<String>,
    pub note: Option<Note>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunGuidanceDelivery {
    pub run_id: RunId,
    pub selected_evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTerminateDelivery {
    pub run_id: RunId,
    pub note: Option<Note>,
}

pub fn map_show_run_id(parsed: &RunShowParsed) -> Result<RunId, RunMapError> {
    map_run_id(&parsed.run_id)
}

pub fn map_graph_run_id(parsed: &RunGraphParsed) -> Result<RunId, RunMapError> {
    map_run_id(&parsed.run_id)
}

pub fn map_compatibility_run_id(parsed: &RunCompatibilityParsed) -> Result<RunId, RunMapError> {
    map_run_id(&parsed.run_id)
}

pub fn map_create_delivery(parsed: &RunCreateParsed) -> Result<RunCreateDelivery, RunMapError> {
    Ok(RunCreateDelivery {
        target: map_target(&parsed.target)?,
        label: parsed
            .label
            .as_ref()
            .map(|value| value.as_str().to_string()),
        inputs_path: parsed.inputs.as_ref().map(|path| path.as_str().to_string()),
    })
}

pub fn map_list_request(parsed: &RunListParsed) -> Result<RunListRequest, RunMapError> {
    Ok(RunListRequest {
        filter: list_filter(parsed.terminal, parsed.all),
        limit: page_limit(parsed.limit),
        cursor: optional_opaque_wire(parsed.cursor.as_ref()),
    })
}

pub fn map_history_request(parsed: &RunHistoryParsed) -> Result<RunHistoryRequest, RunMapError> {
    Ok(RunHistoryRequest {
        run_id: map_run_id(&parsed.run_id)?,
        limit: page_limit(parsed.limit),
        cursor: optional_opaque_wire(parsed.cursor.as_ref()),
    })
}

pub fn map_annotate_delivery(
    parsed: &RunAnnotateParsed,
) -> Result<RunAnnotateDelivery, RunMapError> {
    Ok(RunAnnotateDelivery {
        run_id: map_run_id(&parsed.run_id)?,
        note: map_optional_note(&parsed.note)?,
        actor_path: parsed.actor.as_ref().map(|path| path.as_str().to_string()),
        corrects_sequence: map_journal_sequence(parsed.corrects.as_ref())?,
    })
}

pub fn map_label_delivery(parsed: &RunLabelParsed) -> Result<RunLabelDelivery, RunMapError> {
    let label = match &parsed.mode {
        RunLabelMode::Set(text) => Some(text.as_str().to_string()),
        RunLabelMode::Clear => None,
    };
    Ok(RunLabelDelivery {
        run_id: map_run_id(&parsed.run_id)?,
        label,
    })
}

pub fn map_request_delivery(parsed: &RunRequestParsed) -> Result<RunRequestDelivery, RunMapError> {
    Ok(RunRequestDelivery {
        run_id: map_run_id(&parsed.run_id)?,
        event: map_event_id(&parsed.event)?,
        selected_evidence_ids: map_evidence_ids(&parsed.evidence_id)?,
        inline_evidence_path: parsed
            .evidence
            .as_ref()
            .map(|path| path.as_str().to_string()),
        note: map_optional_note(&parsed.note)?,
    })
}

pub fn map_guidance_delivery(
    parsed: &RunGuidanceParsed,
) -> Result<RunGuidanceDelivery, RunMapError> {
    Ok(RunGuidanceDelivery {
        run_id: map_run_id(&parsed.run_id)?,
        selected_evidence_ids: map_evidence_ids(&parsed.evidence_id)?,
    })
}

pub fn map_terminate_delivery(
    parsed: &RunTerminateParsed,
) -> Result<RunTerminateDelivery, RunMapError> {
    Ok(RunTerminateDelivery {
        run_id: map_run_id(&parsed.run_id)?,
        note: map_optional_note(&parsed.note)?,
    })
}

// --- Renderable outcome DTOs ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListRowView {
    pub run_id: String,
    pub label: Option<String>,
    pub lifecycle: loop_engine_core::model::lifecycle::Lifecycle,
    pub current_state: String,
}

impl From<&RunListRow> for RunListRowView {
    fn from(row: &RunListRow) -> Self {
        Self {
            run_id: row.run_id.as_str().to_string(),
            label: row.label.clone(),
            lifecycle: row.lifecycle,
            current_state: row.current_state.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListOutcome {
    pub items: Vec<RunListRowView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHistoryOutcome {
    pub items: Vec<JournalEntry>,
    pub next_cursor: Option<String>,
}

fn map_list_page(page: Page<RunListRow>) -> RunListOutcome {
    RunListOutcome {
        items: page.rows.iter().map(RunListRowView::from).collect(),
        next_cursor: page
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_string()),
    }
}

fn map_history_page(page: Page<JournalEntry>) -> RunHistoryOutcome {
    RunHistoryOutcome {
        items: page.rows,
        next_cursor: page
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_string()),
    }
}

#[derive(Debug, Error)]
pub enum RunListError<E> {
    #[error(transparent)]
    Paging(#[from] PagingError),
    #[error(transparent)]
    Reader(E),
}

#[derive(Debug, Error)]
pub enum RunHistoryError<E> {
    #[error(transparent)]
    Paging(#[from] PagingError),
    #[error(transparent)]
    Reader(E),
}

// --- Command construction (public core `command` entry points only) ---

pub fn build_annotate_command(
    run: &Run,
    delivery: &RunAnnotateDelivery,
    actor: Option<ActorMetadata>,
    journal_entry: JournalDraft,
) -> Result<Option<AppendAnnotationCommand>, RunMapError> {
    run_annotate::command(
        run,
        delivery.note.clone(),
        actor,
        delivery.corrects_sequence,
        journal_entry,
    )
    .map_err(RunMapError::Command)
}

pub fn build_label_command(
    run: &Run,
    delivery: &RunLabelDelivery,
    completed_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
) -> Result<ReplaceLabelCommand, RunMapError> {
    run_label::command(
        run,
        delivery.label.clone(),
        completed_entry,
        terminal_rejection_entry,
    )
    .map_err(RunMapError::Label)
}

pub fn build_terminate_command(
    run: &Run,
    delivery: &RunTerminateDelivery,
    completed_entry: JournalDraft,
    terminal_rejection_entry: JournalDraft,
    stale_error_entry: JournalDraft,
) -> Result<TerminateRunCommand, RunMapError> {
    run_terminate::command(
        run,
        delivery.note.clone(),
        completed_entry,
        terminal_rejection_entry,
        stale_error_entry,
    )
    .map_err(RunMapError::Command)
}

// --- Adapters: exactly one core operation each ---

#[allow(
    clippy::type_complexity,
    reason = "preserves exact core capability errors"
)]
pub fn create<C, I, D, G, W, F, J>(
    catalog: &C,
    invoker: &I,
    digests: &D,
    ids: &G,
    writer: &W,
    request: &RunCreateRequest,
    journal: F,
) -> Result<
    RunCreateExecution,
    RunCreateExecutionError<C::Error, I::TransportError, D::Error, G::Error, W::Error, J>,
>
where
    C: ProviderCatalog,
    I: ProviderInvoker,
    D: DigestComputer,
    D::Error: std::fmt::Display,
    G: IdGenerator,
    W: RunWriter,
    F: FnOnce(
        &loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig,
        &loop_engine_core::capabilities::provider_invoker::DescribeResult,
        Option<&loop_engine_core::capabilities::provider_invoker::InputValidationInvocationResult>,
        &Run,
    ) -> Result<JournalDraft, J>,
{
    run_create::execute(
        catalog,
        invoker,
        digests,
        ids,
        writer,
        &request.registration_id,
        request.inputs.clone(),
        request.label.clone(),
        journal,
    )
}

pub fn list<R: RunCatalogReader>(
    reader: &R,
    request: RunListRequest,
) -> Result<RunListOutcome, RunListError<R::Error>> {
    let page_request: PageRequest<RunListFilter> =
        run_list::query(request.filter, Some(request.limit), request.cursor)?;
    let page = run_list::execute(reader, &page_request).map_err(RunListError::Reader)?;
    Ok(map_list_page(page))
}

pub fn show<R: RunReader>(reader: &R, run_id: &RunId) -> Result<RunShow, R::Error> {
    run_show::execute(reader, run_id)
}

pub fn graph<R: RunReader>(reader: &R, run_id: &RunId) -> Result<StoredGraph, R::Error> {
    run_graph::execute(reader, run_id)
}

pub fn history<R: RunHistoryReader>(
    reader: &R,
    request: RunHistoryRequest,
) -> Result<RunHistoryOutcome, RunHistoryError<R::Error>> {
    let page_request: PageRequest<()> = run_history::query(Some(request.limit), request.cursor)?;
    let page = run_history::execute(reader, &request.run_id, &page_request)
        .map_err(RunHistoryError::Reader)?;
    Ok(map_history_page(page))
}

pub fn annotate<W: RunWriter>(
    writer: &W,
    command: AppendAnnotationCommand,
) -> Result<CommitStatus, W::Error> {
    run_annotate::execute(writer, command)
}

pub fn label<W: RunWriter>(
    writer: &W,
    command: ReplaceLabelCommand,
) -> Result<CommitStatus, W::Error> {
    run_label::execute(writer, command)
}

#[allow(clippy::too_many_arguments)]
pub fn request<R, C, I, W, G, F, Q, J>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    delivery: &RunRequestDelivery,
    inline_evidence: &[EvidenceRecord],
    gate_request: G,
    command: F,
) -> Result<RequestExecution, RequestExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: EventAttemptWriter,
    G: FnMut(
        &loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig,
        &Run,
        &[EvidenceRecord],
    ) -> Result<loop_engine_core::capabilities::provider_invoker::GateRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        &[EvidenceRecord],
        Option<&Note>,
        &RequestResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<
        loop_engine_core::capabilities::persistence_commands::CommitEventAttemptCommand,
        J,
    >,
{
    run_request::execute(
        reader,
        catalog,
        invoker,
        writer,
        &delivery.run_id,
        &delivery.event,
        &delivery.selected_evidence_ids,
        inline_evidence,
        delivery.note.as_ref(),
        gate_request,
        command,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn guidance<R, C, I, W, G, F, Q, J>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    delivery: &RunGuidanceDelivery,
    request: G,
    command: F,
) -> Result<CommitStatus, GuidanceExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: RunWriter,
    G: FnMut(
        &loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig,
        &Run,
        &[EvidenceRecord],
    ) -> Result<loop_engine_core::capabilities::provider_invoker::GuidanceRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        Option<&loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig>,
        &[EvidenceRecord],
        &GuidanceResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<
        loop_engine_core::capabilities::persistence_commands::AppendGuidanceAttemptCommand,
        J,
    >,
{
    run_guidance::execute(
        reader,
        catalog,
        invoker,
        writer,
        &delivery.run_id,
        &delivery.selected_evidence_ids,
        request,
        command,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compatibility<R, C, I, W, G, F, Q, J>(
    reader: &R,
    catalog: &C,
    invoker: &I,
    writer: &W,
    run_id: &RunId,
    request: G,
    command: F,
) -> Result<CommitStatus, CompatibilityExecutionError<R::Error, J, W::Error>>
where
    R: RunReader,
    C: ProviderCatalog,
    I: ProviderInvoker,
    W: RunWriter,
    G: FnMut(
        &loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig,
        &Run,
    )
        -> Result<loop_engine_core::capabilities::provider_invoker::CompatibilityRequest, Q>,
    F: for<'a> FnMut(
        &Run,
        Option<&loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig>,
        Option<&loop_engine_core::model::provider::ProviderObservation>,
        &CompatibilityResolution<'a, C::Error, R::Error, I::TransportError, Q>,
    ) -> Result<
        loop_engine_core::capabilities::persistence_commands::AppendCompatibilityAttemptCommand,
        J,
    >,
{
    run_compatibility::execute(reader, catalog, invoker, writer, run_id, request, command)
}

pub fn terminate<W: RunWriter>(
    writer: &W,
    command: TerminateRunCommand,
) -> Result<TerminateCommit, W::Error> {
    run_terminate::execute(writer, command)
}
