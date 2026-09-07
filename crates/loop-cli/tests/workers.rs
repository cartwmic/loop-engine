#[path = "../../../tests/bounded_process.rs"]
mod bounded_process;

#[path = "workers/fan_out.rs"]
mod fan_out;
#[path = "workers/recovery_backtracking.rs"]
mod recovery_backtracking;
#[path = "workers/recovery_cancellation.rs"]
mod recovery_cancellation;
#[path = "workers/recovery_override.rs"]
mod recovery_override;
#[path = "workers/recovery_ownership.rs"]
mod recovery_ownership;
#[path = "workers/stdin_exec.rs"]
mod stdin_exec;
#[path = "workers/wait_invocation.rs"]
mod wait_invocation;
