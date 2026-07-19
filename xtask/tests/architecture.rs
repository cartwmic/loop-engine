use std::path::PathBuf;
use std::process::Command;

use camino::Utf8PathBuf;

fn manifest_path(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/architecture")
        .join(fixture)
        .join("Cargo.toml")
}

#[test]
fn architecture_command_passes_for_current_repository() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("architecture")
        .status()
        .expect("xtask binary should execute");

    assert!(
        status.success(),
        "architecture command should pass for the current repository"
    );
}

#[test]
fn architecture_check_passes_for_allowed_fixture() {
    xtask::architecture::run(Some(&manifest_path("allowed")))
        .expect("allowed fixture should satisfy product crate architecture");
}

#[test]
fn architecture_check_rejects_forbidden_edge_fixture() {
    let error = xtask::architecture::run(Some(&manifest_path("forbidden-edge")))
        .expect_err("forbidden-edge fixture should violate product crate architecture");

    let message = error.to_string();
    assert!(
        message.contains("loop-engine-core")
            && message.contains("loop-engine-integrations")
            && message.contains("forbidden product dependency"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_rejects_bypass_fixture() {
    let error = xtask::architecture::run(Some(&manifest_path("bypass")))
        .expect_err("bypass fixture should violate construction choke points");

    let message = error.to_string();
    assert!(
        message.contains("provider-process construction")
            || message.contains("persistence construction"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_rejects_reversed_core_fixture() {
    let error = xtask::architecture::run(Some(&manifest_path("reversed-core")))
        .expect_err("reversed-core fixture should violate core internal direction");

    let message = error.to_string();
    assert!(
        message.contains("core internal dependency direction") && message.contains("operations"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_rejects_catch_all_fixture() {
    let error = xtask::architecture::run(Some(&manifest_path("catch-all")))
        .expect_err("catch-all fixture should violate catch-all module policy");

    let message = error.to_string();
    assert!(
        message.contains("catch-all"),
        "unexpected error message: {message}"
    );
}

#[test]
fn default_workspace_manifest_points_at_repository_root() {
    let manifest = xtask::architecture::default_workspace_manifest();
    assert!(
        manifest.is_file(),
        "default manifest should exist at {}",
        manifest
    );

    let workspace_root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    assert_eq!(manifest, workspace_root.join("Cargo.toml"));
}
