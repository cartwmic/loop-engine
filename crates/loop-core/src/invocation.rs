//! Inert projection of historical invocation rows.
use crate::{ProjectedInvocationStatus, Timestamp, WaiterWrittenStatus, WorkSlotInvocation};

pub fn project_invocation_status(
    record: &WorkSlotInvocation,
    _now: Timestamp,
    _waiter_alive: bool,
) -> ProjectedInvocationStatus {
    match record.status {
        Some(WaiterWrittenStatus::Succeeded) => ProjectedInvocationStatus::Succeeded,
        Some(WaiterWrittenStatus::Failed) => ProjectedInvocationStatus::Failed,
        None => ProjectedInvocationStatus::Unknown,
    }
}
