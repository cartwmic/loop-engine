use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_root(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/docs-check")
        .join(fixture)
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
fn docs_check_command_passes_for_current_repository() {
    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("docs-check")
        .status()
        .expect("xtask binary should execute");

    assert!(
        status.success(),
        "docs-check command should pass for the current repository"
    );
}

#[test]
fn docs_check_passes_for_valid_fixture() {
    xtask::docs_check::run(Some(&fixture_root("valid")))
        .expect("valid fixture should satisfy documentation checks");
}

#[test]
fn docs_check_rejects_invalid_utf8_fixture() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("valid"), temp.path());
    fs::write(
        temp.path().join("docs/architecture.md"),
        b"# Invalid UTF-8\n\n\xff\n",
    )
    .expect("inject invalid UTF-8 canary");

    let error = xtask::docs_check::run(Some(temp.path()))
        .expect_err("invalid-utf8 fixture should fail UTF-8 validation");

    let message = error.to_string();
    assert!(
        message.contains("utf-8"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_missing_final_newline_fixture() {
    let error = xtask::docs_check::run(Some(&fixture_root("missing-final-newline")))
        .expect_err("missing-final-newline fixture should fail final-newline validation");

    let message = error.to_string();
    assert!(
        message.contains("final-newline"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_trailing_whitespace_fixture() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    copy_tree(&fixture_root("valid"), temp.path());
    fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("docs/architecture.md"))
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(b"intentional trailing whitespace  \n")
        })
        .expect("inject trailing-whitespace canary");

    let error = xtask::docs_check::run(Some(temp.path()))
        .expect_err("trailing-whitespace fixture should fail trailing-whitespace validation");

    let message = error.to_string();
    assert!(
        message.contains("trailing-whitespace"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_broken_relative_link_fixture() {
    let error = xtask::docs_check::run(Some(&fixture_root("broken-relative-link")))
        .expect_err("broken-relative-link fixture should fail relative-link validation");

    let message = error.to_string();
    assert!(
        message.contains("relative-link"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_duplicate_heading_fixture() {
    let error = xtask::docs_check::run(Some(&fixture_root("duplicate-heading")))
        .expect_err("duplicate-heading fixture should fail duplicate-heading validation");

    let message = error.to_string();
    assert!(
        message.contains("duplicate-heading"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_missing_required_file_fixture() {
    let error = xtask::docs_check::run(Some(&fixture_root("missing-required-file")))
        .expect_err("missing-required-file fixture should fail required-file validation");

    let message = error.to_string();
    assert!(
        message.contains("required-file"),
        "unexpected error message: {message}"
    );
}

#[test]
fn docs_check_rejects_forbidden_terminology_fixture() {
    let error = xtask::docs_check::run(Some(&fixture_root("forbidden-terminology")))
        .expect_err("forbidden-terminology fixture should fail frozen-terminology validation");

    let message = error.to_string();
    assert!(
        message.contains("frozen-terminology"),
        "unexpected error message: {message}"
    );
}

#[test]
fn default_repository_root_points_at_repository_root() {
    let root = xtask::docs_check::default_repository_root();
    assert!(
        root.is_dir(),
        "default repository root should exist at {root}"
    );
    assert!(
        root.join("Cargo.toml").is_file(),
        "default repository root should contain Cargo.toml"
    );
}
