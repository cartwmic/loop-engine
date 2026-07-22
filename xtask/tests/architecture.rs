use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use camino::Utf8PathBuf;

fn manifest_path(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/architecture")
        .join(fixture)
        .join("Cargo.toml")
}

fn fixture_root(fixture: &str) -> PathBuf {
    manifest_path(fixture)
        .parent()
        .expect("fixture manifest parent")
        .to_path_buf()
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture destination");
    for entry in fs::read_dir(source).expect("read fixture") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
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
fn architecture_check_rejects_outer_capability_fixture() {
    let error = xtask::architecture::run(Some(&manifest_path("capabilities")))
        .expect_err("capability fixture should reject outer construction in core");
    assert!(
        error.to_string().contains("forbidden product dependency"),
        "unexpected error message: {error}"
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
fn architecture_check_rejects_cli_persistence_bypass_outside_composition() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("allowed"), temp.path());
    let bypass = temp
        .path()
        .join("crates/loop-engine-cli/src/diagnostics.rs");
    fs::write(
        bypass,
        "use rusqlite::Connection;\npub fn probe() { let _ = Connection::open(\"state.db\"); }\n",
    )
    .expect("write bypass source");

    let error = xtask::architecture::run(Some(&temp.path().join("Cargo.toml")))
        .expect_err("CLI persistence bypass must stay in composition.rs");

    let message = error.to_string();
    assert!(
        message.contains("persistence construction bypass") && message.contains("composition.rs"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_rejects_cli_provider_process_bypass_outside_composition() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("allowed"), temp.path());
    let bypass = temp.path().join("crates/loop-engine-cli/src/render.rs");
    fs::write(
        bypass,
        "pub fn spawn() { let _ = std::process::Command::new(\"true\").spawn(); }\n",
    )
    .expect("write bypass source");

    let error = xtask::architecture::run(Some(&temp.path().join("Cargo.toml")))
        .expect_err("CLI provider-process bypass must stay in composition.rs");

    let message = error.to_string();
    assert!(
        message.contains("provider-process construction bypass")
            && message.contains("composition.rs"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_rejects_cli_dispatch_bypass_outside_dispatch() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("allowed"), temp.path());
    let bypass = temp.path().join("crates/loop-engine-cli/src/commands.rs");
    fs::write(
        bypass,
        "use loop_engine_core::operations::Catalog;\npub fn dispatch() -> Catalog { Catalog }\n",
    )
    .expect("write bypass source");

    let error = xtask::architecture::run(Some(&temp.path().join("Cargo.toml")))
        .expect_err("CLI operation dispatch bypass must stay in dispatch.rs");

    let message = error.to_string();
    assert!(
        message.contains("operation-dispatch bypass") && message.contains("dispatch.rs"),
        "unexpected error message: {message}"
    );
}

#[test]
fn architecture_check_allows_composition_module_construction() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("allowed"), temp.path());
    let composition = temp
        .path()
        .join("crates/loop-engine-cli/src/composition.rs");
    fs::write(
        composition,
        r#"use loop_engine_integrations::persistence::SqliteStore;
use loop_engine_integrations::provider_protocol::SubprocessProviderInvoker;
use loop_engine_integrations::sha256_digest::Sha256DigestComputer;
use loop_engine_integrations::system_clock::SystemTimeSource;
use loop_engine_integrations::uuid_ids::UuidV7Generator;
use std::sync::{Arc, Mutex};

pub fn build() {
    let _ = UuidV7Generator;
    let _ = SystemTimeSource;
    let _ = Sha256DigestComputer;
    let _ = SqliteStore::open("state.db");
    let _ = SubprocessProviderInvoker::new(Arc::new(Mutex::new(
        loop_engine_integrations::trace::TraceWriter::create(
            std::path::Path::new("traces"),
            "request",
        )
        .unwrap(),
    )));
}
"#,
    )
    .expect("write composition source");

    xtask::architecture::run(Some(&temp.path().join("Cargo.toml")))
        .expect("composition.rs should remain the sole CLI construction choke point");
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
