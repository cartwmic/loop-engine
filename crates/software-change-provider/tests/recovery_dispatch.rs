//! Reject unknown external driver scenarios before doing work.
use std::process::Command;

#[test]
fn recovery_dispatch_unknown_scenario_fails_before_work() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
            "/not-used/engine",
            "--provider",
            "/not-used/provider",
            "--data-root",
            ".",
            "--work-root",
            "/not-used/recovery-dispatch",
            "--profile",
            "/not-used/profile",
            "--traversal-depth",
            "full",
            "--scenario",
            "unknown-scenario",
        ])
        .output()
        .expect("run public Python journey selector");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("unrecognized arguments: --scenario unknown-scenario"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("passed"));
}
