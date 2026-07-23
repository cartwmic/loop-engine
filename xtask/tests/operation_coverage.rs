use std::process::Command;

#[test]
fn baseline_command_rejects_exposed_runtime_catalogs() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["operation-coverage", "--mode", "baseline"])
        .status()
        .expect("xtask operation-coverage should execute");
    assert!(!status.success());
}

#[test]
fn baseline_rejects_open_operation_allowance() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "operation-coverage",
            "--mode",
            "baseline",
            "--allow-open",
            "run.show",
        ])
        .status()
        .expect("xtask operation-coverage should execute");
    assert!(!status.success());
}

#[test]
fn exposed_command_accepts_closed_runtime_catalogs() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["operation-coverage", "--mode", "exposed"])
        .status()
        .expect("xtask operation-coverage should execute");
    assert!(status.success());
}

#[test]
fn candidate_rejects_unknown_open_operation() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "operation-coverage",
            "--mode",
            "candidate",
            "--allow-open",
            "totally.not-an-operation",
        ])
        .status()
        .expect("xtask operation-coverage should execute");
    assert!(!status.success());
}
