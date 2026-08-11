//! `terminate` run operation.

use super::persistence_error;
use crate::{OperationOutcome, Persistence, TerminateRequest, TerminateResult};

/// The terminate operation uses the semantic port request directly.
pub type Request = TerminateRequest;

/// Successful `terminate` data returned by the persistence boundary.
pub type Result = TerminateResult;

/// Execute an atomic active-run termination.
///
/// Persistence verifies activity, advances control revision, and appends one
/// termination history entry in one semantic mutation.  Terminal runs are
/// therefore rejected by the adapter without a new history entry.
pub fn execute<P>(request: Request, persistence: &P) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    match persistence.terminate(request) {
        Ok(result) => OperationOutcome::completed(result),
        Err(error) => persistence_error(error),
    }
}

/// Execute `terminate` with persistence first.
pub fn execute_with_persistence<P>(persistence: &P, request: Request) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
{
    execute(request, persistence)
}
