//! The central target drives the same Python/public-CLI proof as the journey.
use std::process::Command;
use std::time::Duration;

fn prove(case: &str) {
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Keep public captures/catalogs available after nextest, including failures.
    let root = tempfile::Builder::new()
        .prefix("recovery-cancellation-nextest-")
        .tempdir()
        .unwrap()
        .keep();
    let mut command = Command::new("python3");
    command
        .current_dir(&checkout)
        .arg(checkout.join("scripts/recovery_cancellation.py"))
        .arg(workspace_integration::binary("loop-engine"))
        .arg(workspace_integration::binary("software-change"))
        .arg(&root)
        .arg(case);
    let output = super::bounded_process::run_with_deadline(
        &mut command,
        "public recovery cancellation",
        Duration::from_secs(55),
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
fn recovery_cancellation_graph_term_resistant_tree() {
    prove("graph");
}

#[test]
fn recovery_cancellation_waiter_loss() {
    prove("waiter");
}

#[test]
fn recovery_cancellation_interrupted_controller_eligible_gap() {
    prove("interrupted");
}

#[test]
fn recovery_cancellation_unverified_deadline_stays_blocking() {
    prove("deadline");
}

#[test]
fn recovery_cancellation_natural_completion_races() {
    prove("races");
}

#[test]
fn recovery_cancellation_live_termination_refusal_then_cleanup_and_termination() {
    prove("termination");
}
