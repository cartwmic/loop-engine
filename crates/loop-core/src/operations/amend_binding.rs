//! Future-only binding correction; never changes initial input or a started attempt.
use crate::{OperationOutcome, Persistence, RunId, Timestamp, WorkSlotBinding, WorkSlotId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub state_visit: u64,
    pub owner: String,
    pub reason: String,
    pub binding: WorkSlotBinding,
}

pub fn execute<P: Persistence + ?Sized>(
    run_id: &RunId,
    slot_id: &WorkSlotId,
    request: Request,
    persistence: &P,
    now: Timestamp,
) -> OperationOutcome<crate::HistoryEntry> {
    if request.owner.trim().is_empty()
        || request.reason.trim().is_empty()
        || request.binding.command.trim().is_empty()
        || request
            .binding
            .context_filter
            .as_ref()
            .is_some_and(|f| f.command.trim().is_empty())
    {
        return OperationOutcome::rejected(
            "invalid-binding-amendment",
            "owner, reason and commands must be nonempty",
        );
    }
    let run = match persistence.load_authoritative_run(run_id) {
        Ok(run) => run,
        Err(error) => return super::persistence_error(error),
    };
    if !run.lifecycle.is_active() {
        return OperationOutcome::rejected(
            "run-not-active",
            "binding amendments require an active run",
        );
    }
    if !run
        .workflow
        .work_slots
        .iter()
        .any(|slot| slot.id == *slot_id)
    {
        return OperationOutcome::rejected(
            "unknown-work-slot",
            format!("unknown work slot `{slot_id}`"),
        );
    }
    match persistence.amend_binding(run_id, slot_id, request, now) {
        Ok(history) => OperationOutcome::completed(history),
        Err(error) => super::persistence_error(error),
    }
}
