use std::thread;
use std::time::{Duration, Instant};

use super::error::ProcessError;
use super::process_group::{group_exists, signal_group};

pub const POLL_INTERVAL: Duration = Duration::from_millis(10);
const TERMINATION_GRACE: Duration = Duration::from_secs(5);

pub fn terminate_group(process_group_id: u32) -> Result<(), ProcessError> {
    let _ = signal_group(process_group_id, "TERM");
    let deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < deadline {
        if !group_exists(process_group_id)? {
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    let _ = signal_group(process_group_id, "KILL");
    let deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < deadline {
        if !group_exists(process_group_id)? {
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(ProcessError::Termination(format!(
        "process group {process_group_id} remained after SIGKILL"
    )))
}
