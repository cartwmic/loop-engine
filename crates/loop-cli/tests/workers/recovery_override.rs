//! Same public override path used by the named software-change journey.
use std::process::Command;
use std::time::Duration;

fn prove(case: &str) {
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::Builder::new()
        .prefix("recovery-override-nextest-")
        .tempdir()
        .unwrap()
        .keep();
    let mut command = Command::new("python3");
    command
        .current_dir(&checkout)
        .arg(checkout.join("scripts/recovery_override.py"))
        .arg(workspace_integration::binary("loop-engine"))
        .arg(workspace_integration::binary("software-change"))
        .arg(workspace_integration::binary("bookends-check"))
        .arg(&root)
        .arg(case);
    let output = super::bounded_process::run_with_deadline(
        &mut command,
        "public recovery override",
        Duration::from_secs(120),
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
fn recovery_override_software_retains_real_denials_and_non_green_evidence() {
    prove("software");
}

#[test]
fn recovery_override_normal_and_exceptional_terminal_persistence() {
    prove("comparison");
}

#[test]
fn recovery_override_live_elapsed_and_cleanup_pending_refuse() {
    prove("live");
}
