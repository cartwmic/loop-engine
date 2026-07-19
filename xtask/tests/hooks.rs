use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use xtask::hooks::{self, HOOK_VERSION, PRE_COMMIT_HOOK_PATH, PreCommitOptions};

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hooks")
        .join(name)
}

fn docs_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs-check/valid")
}

fn architecture_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/architecture/allowed")
}

fn real_repo_root() -> PathBuf {
    hooks::default_repository_root()
}

fn canonical_pre_commit_body() -> String {
    fs::read_to_string(real_repo_root().join(PRE_COMMIT_HOOK_PATH))
        .expect("versioned .githooks/pre-commit should exist in the real repository")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to spawn: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout utf-8")
        .trim_end()
        .to_owned()
}

fn write(repo: &Path, relative: &str, content: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(&path, content).expect("write");
}

fn copy_dir(src: &Path, dst: &Path) {
    fn copy_recursive(src: &Path, dst: &Path) {
        if src.is_dir() {
            fs::create_dir_all(dst).expect("mkdir");
            for entry in fs::read_dir(src).expect("read_dir") {
                let entry = entry.expect("dir entry");
                copy_recursive(&entry.path(), &dst.join(entry.file_name()));
            }
            return;
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::copy(src, dst).expect("copy fixture file");
    }
    copy_recursive(src, dst);
}

fn init_seeded_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "hooks@test"]);
    git(dir.path(), &["config", "user.name", "Hooks Test"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);

    copy_dir(&architecture_fixture_root(), dir.path());
    copy_dir(&docs_fixture_root(), dir.path());
    write(
        dir.path(),
        PRE_COMMIT_HOOK_PATH,
        &canonical_pre_commit_body(),
    );

    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "seed"]);
    dir
}

fn stage_docs_edit(repo: &Path, content: &str) {
    write(repo, "docs/intent.md", content);
    git(repo, &["add", "docs/intent.md"]);
}

fn options(repo: &Path) -> PreCommitOptions {
    PreCommitOptions::new(repo)
}

#[test]
fn staged_pass_runs_only_bounded_checks() {
    let repo = init_seeded_repo();
    stage_docs_edit(
        repo.path(),
        "# Valid\n\nValid fixture documentation.\n\nStaged pass edit.\n",
    );

    let outcome = hooks::pre_commit(&options(repo.path())).expect("staged pass");
    assert_eq!(
        outcome.checks,
        ["diff-check", "docs-check", "architecture", "fmt"]
    );
}

#[test]
fn staged_diff_check_failure_blocks_before_materialization() {
    let repo = init_seeded_repo();
    write(repo.path(), "bad.txt", "trailing whitespace   \n");
    git(repo.path(), &["add", "bad.txt"]);

    let error =
        hooks::pre_commit(&options(repo.path())).expect_err("staged diff-check failure must block");
    assert!(
        format!("{error:#}").contains("diff --cached --check"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn staged_quality_manifest_and_tests_are_not_run() {
    let repo = init_seeded_repo();
    write(
        repo.path(),
        "quality/manifest.toml",
        r#"schema_version = 1
[[checks]]
id = "test"
runner = "cargo-test"
"#,
    );
    write(
        repo.path(),
        "crates/loop-engine-core/tests/would_fail.rs",
        "#[test]\nfn would_fail() {\n    panic!(\"full tests must not run in pre-commit\");\n}\n",
    );
    git(
        repo.path(),
        &[
            "add",
            "quality/manifest.toml",
            "crates/loop-engine-core/tests/would_fail.rs",
        ],
    );

    let outcome = hooks::pre_commit(&options(repo.path()))
        .expect("bounded pre-commit must ignore full quality manifest and tests");
    assert_eq!(outcome.checks.len(), 4);
}

#[test]
fn unstaged_contamination_is_ignored() {
    let repo = init_seeded_repo();
    stage_docs_edit(
        repo.path(),
        "# Valid\n\nValid fixture documentation.\n\nClean staged edit.\n",
    );

    // Contaminate the working tree only: trailing whitespace would fail docs-check
    // if the adapter judged the working tree instead of the exact staged tree.
    write(
        repo.path(),
        "docs/intent.md",
        "# Valid\n\nValid fixture documentation.   \n\nClean staged edit.\n",
    );
    let contaminated = fs::read_to_string(repo.path().join("docs/intent.md")).expect("read");
    assert!(
        contaminated.contains("documentation.   \n"),
        "working tree should remain contaminated before pre-commit"
    );

    let outcome = hooks::pre_commit(&options(repo.path()))
        .expect("unstaged contamination must not fail exact staged pre-commit");
    assert_eq!(outcome.checks.len(), 4);

    let after = fs::read_to_string(repo.path().join("docs/intent.md")).expect("read after");
    assert_eq!(
        after, contaminated,
        "pre-commit must never rewrite unstaged working-tree files"
    );
    assert!(
        after.contains("documentation.   \n"),
        "working-tree contamination must remain after pre-commit"
    );
}

#[test]
fn hook_version_mismatch_fails_verify() {
    let repo = init_seeded_repo();
    let mismatched = fs::read_to_string(fixture_root("mismatched-pre-commit"))
        .expect("mismatched fixture should exist");
    write(repo.path(), PRE_COMMIT_HOOK_PATH, &mismatched);

    hooks::install(Some(repo.path())).expect("install should set hooksPath");
    let error = hooks::verify(Some(repo.path())).expect_err("version mismatch must fail verify");
    let message = format!("{error:#}");
    assert!(
        message.contains("hook-version mismatch"),
        "unexpected verify error: {message}"
    );
    assert!(
        message.contains(&HOOK_VERSION.to_string()),
        "verify error should cite expected version: {message}"
    );
}

#[test]
fn install_and_verify_accept_matching_versioned_hook() {
    let repo = init_seeded_repo();
    hooks::install(Some(repo.path())).expect("install");
    hooks::verify(Some(repo.path())).expect("verify matching version");
}

#[test]
fn shell_hook_blocks_unstaged_member_manifest_before_cargo_bootstrap() {
    let repo = init_seeded_repo();
    stage_docs_edit(
        repo.path(),
        "# Valid\n\nValid fixture documentation.\n\nStaged edit.\n",
    );
    write(
        repo.path(),
        "crates/loop-engine-core/Cargo.toml",
        "this is not valid toml [",
    );

    let output = Command::new("bash")
        .arg(PRE_COMMIT_HOOK_PATH)
        .current_dir(repo.path())
        .output()
        .expect("run hook shell");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("gate implementation has unstaged changes")
    );
}

#[test]
fn versioned_pre_commit_hook_declares_current_version() {
    let body = canonical_pre_commit_body();
    let version = hooks::parse_hook_version(&body).expect("version marker");
    assert_eq!(version, HOOK_VERSION);
    assert!(
        body.contains("cargo run") && body.contains("hooks pre-commit"),
        "thin hook must delegate to xtask hooks pre-commit"
    );
    assert!(
        body.contains("env -u RUSTUP_TOOLCHAIN") && body.contains("git rev-parse --local-env-vars"),
        "hook must honor the toolchain pin and isolate nested fixture Git repositories"
    );
    assert!(
        body.contains("git diff --quiet")
            && body.contains("git ls-files --others --exclude-standard")
            && body.contains("quality/semantic-judge")
            && body.contains("crates/*/Cargo.toml")
            && body.contains("xtask"),
        "hook must reject unstaged gate-implementation contamination"
    );
}
