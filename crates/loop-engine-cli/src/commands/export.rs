//! Private export command adapter (WP1 T122/T132).
//!
//! Delivery-layer syntax-to-core mapping and one-operation-per-intent adapter.
//! Rendering, route registration, export publication, and concrete integration
//! construction live elsewhere.

use loop_engine_core::capabilities::audit_export::{AuditExporter, AuditSnapshot, ExportTarget};
use loop_engine_core::model::bounded::BoundError;
use loop_engine_core::model::ids::{IdentifierError, RunId};
use loop_engine_core::operations::run_export::{self, ExportRequest};
use thiserror::Error;

use crate::args::RunExportParsed;

/// Bounded conversion failures at the CLI delivery boundary (pre-dispatch, not domain rejection).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportMapError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDeliveryOutcome {
    pub snapshot: AuditSnapshot,
    pub output: String,
}

pub fn map_request(parsed: &RunExportParsed) -> Result<ExportRequest, ExportMapError> {
    Ok(run_export::request(
        RunId::parse(parsed.run_id.as_str())?,
        ExportTarget::parse(parsed.output.as_str())?,
    ))
}

pub fn execute<E: AuditExporter>(
    exporter: &E,
    request: &ExportRequest,
) -> Result<AuditSnapshot, E::Error> {
    run_export::execute(exporter, request)
}
