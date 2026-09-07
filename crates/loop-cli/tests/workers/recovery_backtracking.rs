//! The central target uses the same completed public proof as the journey.
use std::process::Command;
use std::time::Duration;

fn prove(case: &str) {
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::Builder::new()
        .prefix("recovery-backtracking-nextest-")
        .tempdir()
        .unwrap()
        .keep();
    let mut command = Command::new("python3");
    command
        .current_dir(&checkout)
        .arg(checkout.join("scripts/recovery_backtracking.py"))
        .arg(workspace_integration::binary("loop-engine"))
        .arg(workspace_integration::binary("software-change"))
        .arg(&root)
        .arg(case);
    let output = super::bounded_process::run_with_deadline(
        &mut command,
        "public recovery backtracking",
        Duration::from_secs(90),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "captures: {}\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"status\": \"passed\""));
}

#[test]
fn recovery_backtracking_completed_work_without_report() {
    prove("complete");
}

#[test]
fn recovery_backtracking_cancelled_work_without_report() {
    prove("cancel");
}

#[test]
fn recovery_backtracking_cleanup_pending_refuses_even_after_worker_exit() {
    prove("pending");
}

#[test]
fn recovery_backtracking_historical_graph_does_not_gain_routes() {
    prove("historical");
}
