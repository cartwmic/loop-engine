//! `history` run operation.

use super::persistence_error;
use crate::{HistoryEntry, OperationOutcome, Persistence, RunId};
use serde::{Deserialize, Serialize};

/// Caller-supplied run identity for a history read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
}

impl Request {
    pub fn new(run_id: impl Into<RunId>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

/// Successful `history` data.
pub type Result = Vec<HistoryEntry>;

/// Execute a durable history read.
///
/// The persistence contract returns semantic records in sequence order.  The
/// defensive sort also keeps the public operation deterministic for a simple
/// fake adapter and does not alter the durable records themselves.
pub fn execute<P>(request: Request, persistence: &P) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    match persistence.load_history(&request.run_id) {
        Ok(mut history) => {
            history.sort_by_key(|entry| entry.sequence);
            OperationOutcome::completed(history)
        }
        Err(error) => persistence_error(error),
    }
}

/// Execute `history` with persistence first.
pub fn execute_with_persistence<P>(persistence: &P, request: Request) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    execute(request, persistence)
}
