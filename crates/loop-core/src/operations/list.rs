//! `list` run operation.

use super::persistence_error;
use crate::{OperationOutcome, Persistence, RunSummary};
use serde::{Deserialize, Serialize};

/// `list` has no semantic input.  The unit request keeps the operation API
/// uniform for callers that model every invocation as a request value.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request;

impl Request {
    pub const fn new() -> Self {
        Self
    }
}

/// Successful `list` data.
pub type Result = Vec<RunSummary>;

/// Execute a stable run-discovery read.
pub fn execute<P>(persistence: &P) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    match persistence.list_runs() {
        Ok(runs) => OperationOutcome::completed(runs),
        Err(error) => persistence_error(error),
    }
}

/// Execute `list` while accepting the unit request used by generic dispatch.
pub fn execute_with_request<P>(_request: Request, persistence: &P) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    execute(persistence)
}
