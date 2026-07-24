//! Private application operations coordinating model decisions through capabilities.
//!
//! Phase 3 operations remain absent from [`catalog::exposed_operations`] until
//! production integrations, CLI routes, traces, facets, and E2Es close together.

use thiserror::Error;

use crate::model::ids::RunId;
use crate::model::journal::{JournalDraft, JournalEntryKind};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("journal entry does not match operation command authority")]
    JournalMismatch,
}

impl From<crate::model::bounded::BoundError> for CommandError {
    fn from(_: crate::model::bounded::BoundError) -> Self {
        Self::JournalMismatch
    }
}

impl From<crate::model::journal::JournalError> for CommandError {
    fn from(_: crate::model::journal::JournalError) -> Self {
        Self::JournalMismatch
    }
}

pub(crate) fn validate_journal(
    entry: &JournalDraft,
    run_id: &RunId,
    operation: &str,
    kind: JournalEntryKind,
) -> Result<(), CommandError> {
    if entry.run_id() != run_id || entry.operation() != operation || entry.kind() != kind {
        return Err(CommandError::JournalMismatch);
    }
    Ok(())
}

pub mod catalog;
pub mod evidence_add;
pub mod evidence_list;
pub mod paging;
pub mod provider_add;
pub mod provider_check;
pub mod provider_disable;
pub mod provider_list;
pub mod provider_rename;
pub mod provider_restore;
pub mod provider_update;
pub mod run_annotate;
pub mod run_compatibility;
pub mod run_create;
pub mod run_export;
pub mod run_graph;
pub mod run_guidance;
pub mod run_history;
pub mod run_label;
pub mod run_list;
pub mod run_request;
pub mod run_show;
pub mod run_terminate;

#[cfg(test)]
pub(crate) mod test_support;
