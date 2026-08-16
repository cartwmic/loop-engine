//! Shared invocation helpers: instruction digest and the reader overlay.
//!
//! LOCKED (do not reopen):
//! - Reader overlay: ONE loop-core function used later by show, history, event,
//!   invoke, and bound-slot-gate. Stored waiter-written statuses are ONLY
//!   `succeeded` and `failed`. `running` means the waiter is still alive. If
//!   stored status is unwritten AND waiter alive AND elapsed from started_at >=
//!   allowed_time_ms, project `overrun`. Overlay-overrun is terminal for
//!   retry: invoke MUST NOT reject as already-running. If waiter pid is gone
//!   and no terminal status was written, project `failed` (crash residual).
//!   Waiter does not write `overrun`. Later CLIs do not waitpid the original
//!   worker.
//! - Overlay is a PURE loop-core function: it does not probe OS processes.
//!   Signature: stored record + `now: Timestamp` + `waiter_alive: bool` →
//!   projected status `running|succeeded|failed|overrun`. Callers supply
//!   `waiter_alive`.
//! - instruction_digest: SHA-256 of stored instruction body UTF-8 bytes,
//!   lowercase hex. One loop-core helper. Hex-encode without a new crate.

use crate::{ProjectedInvocationStatus, Timestamp, WaiterWrittenStatus, WorkSlotInvocation};
use sha2::{Digest, Sha256};

/// SHA-256 of `body` UTF-8 bytes, encoded as lowercase hexadecimal.
pub fn instruction_digest(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    to_hex_lowercase(&digest)
}

/// Project stored waiter-written status (and liveness/time) to a reader status.
///
/// This function does not probe OS processes. Callers supply `waiter_alive`.
pub fn project_invocation_status(
    record: &WorkSlotInvocation,
    now: Timestamp,
    waiter_alive: bool,
) -> ProjectedInvocationStatus {
    match record.status {
        Some(WaiterWrittenStatus::Succeeded) => ProjectedInvocationStatus::Succeeded,
        Some(WaiterWrittenStatus::Failed) => ProjectedInvocationStatus::Failed,
        None if !waiter_alive => ProjectedInvocationStatus::Failed,
        None => {
            let elapsed_ms = elapsed_millis(record.started_at, now);
            if elapsed_ms >= record.allowed_time_ms {
                ProjectedInvocationStatus::Overrun
            } else {
                ProjectedInvocationStatus::Running
            }
        }
    }
}

fn elapsed_millis(started_at: Timestamp, now: Timestamp) -> u64 {
    // elapsed = now - started_at (saturating, treat negative as 0)
    let elapsed = now
        .as_unix_millis()
        .saturating_sub(started_at.as_unix_millis());
    if elapsed <= 0 {
        0
    } else {
        u64::try_from(elapsed).unwrap_or(0)
    }
}

fn to_hex_lowercase(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Timestamp, WaiterWrittenStatus, WorkSlotBinding, WorkSlotInvocation};

    fn record(
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            "inv-1",
            "slot-1",
            WorkSlotBinding::new("echo", vec!["ok".to_owned()]),
            instruction_digest(""),
            "subject-1",
            42,
            Timestamp::from_unix_millis(started_at),
            allowed_time_ms,
            status,
            None,
            None,
            String::new(),
            Vec::new(),
        )
    }

    #[test]
    fn instruction_digest_is_sha256_utf8_lowercase_hex() {
        // Known vector: sha256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            instruction_digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            instruction_digest(""),
            instruction_digest("").to_lowercase()
        );
        assert_eq!(instruction_digest("").len(), 64);
    }

    #[test]
    fn overlay_stored_succeeded_stays_succeeded_even_if_waiter_alive_false() {
        let status = project_invocation_status(
            &record(0, 1_000, Some(WaiterWrittenStatus::Succeeded)),
            Timestamp::from_unix_millis(10_000),
            false,
        );
        assert_eq!(status, ProjectedInvocationStatus::Succeeded);
    }

    #[test]
    fn overlay_stored_failed_stays_failed() {
        let status = project_invocation_status(
            &record(0, 1_000, Some(WaiterWrittenStatus::Failed)),
            Timestamp::from_unix_millis(10_000),
            true,
        );
        assert_eq!(status, ProjectedInvocationStatus::Failed);
    }

    #[test]
    fn overlay_unwritten_waiter_alive_elapsed_less_than_allowed_is_running() {
        let status = project_invocation_status(
            &record(1_000, 5_000, None),
            Timestamp::from_unix_millis(2_000),
            true,
        );
        assert_eq!(status, ProjectedInvocationStatus::Running);
    }

    #[test]
    fn overlay_unwritten_waiter_alive_elapsed_at_least_allowed_is_overrun() {
        let status = project_invocation_status(
            &record(1_000, 5_000, None),
            Timestamp::from_unix_millis(6_000),
            true,
        );
        assert_eq!(status, ProjectedInvocationStatus::Overrun);
        let equal = project_invocation_status(
            &record(1_000, 5_000, None),
            Timestamp::from_unix_millis(6_000),
            true,
        );
        assert_eq!(equal, ProjectedInvocationStatus::Overrun);
    }

    #[test]
    fn overlay_unwritten_waiter_not_alive_is_failed_vanished() {
        // Vanished waiter without terminal status is failed.
        let status = project_invocation_status(
            &record(1_000, 5_000, None),
            Timestamp::from_unix_millis(1_100),
            false,
        );
        assert_eq!(status, ProjectedInvocationStatus::Failed);
    }

    #[test]
    fn overlay_overrun_is_distinct_from_running() {
        assert_ne!(
            ProjectedInvocationStatus::Overrun,
            ProjectedInvocationStatus::Running
        );
        let overrun =
            project_invocation_status(&record(0, 1, None), Timestamp::from_unix_millis(1), true);
        let running =
            project_invocation_status(&record(0, 10, None), Timestamp::from_unix_millis(1), true);
        assert_eq!(overrun, ProjectedInvocationStatus::Overrun);
        assert_eq!(running, ProjectedInvocationStatus::Running);
        assert_ne!(overrun, running);
    }

    #[test]
    fn overlay_treats_negative_elapsed_as_zero() {
        let status = project_invocation_status(
            &record(5_000, 1_000, None),
            Timestamp::from_unix_millis(1_000),
            true,
        );
        assert_eq!(status, ProjectedInvocationStatus::Running);
    }
}
