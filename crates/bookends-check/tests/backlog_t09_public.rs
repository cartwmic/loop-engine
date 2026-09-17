//! Public proof-collection regressions for backlog T09.
//!
//! These cases use the shipped checker binary and small committed temporary
//! repositories.  The central workspace target imports the Rust proof module
//! with the same `#[path]` shape used by this repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn checker() -> PathBuf {
    workspace_integration::binary("bookends-check")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    fs::create_dir_all(&repo).expect("repo directory");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Bookends T09"]);
    git(&repo, &["config", "user.email", "t09@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    repo
}

fn write_file(repo: &Path, relative: &str, text: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("file parent");
    }
    fs::write(path, text).expect("write fixture");
}

fn commit(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
}

fn run_checker(repo: &Path) -> Output {
    Command::new(checker())
        .current_dir(repo)
        .args(["--repo", repo.to_str().expect("repo path")])
        .output()
        .expect("spawn checker")
}

fn first_line(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout)
        .expect("checker stdout")
        .lines()
        .next()
        .unwrap_or("")
}

fn assert_green(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(first_line(output), "GREEN", "{output:?}");
}

fn assert_red(output: &Output, finding: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(first_line(output), "RED", "{stdout}");
    assert!(stdout.contains(finding), "missing {finding:?}: {stdout}");
}

fn write_central_collection_fixture(repo: &Path) {
    write_file(
        repo,
        "Cargo.toml",
        "[workspace]\nmembers = [\"tests/central\"]\nresolver = \"2\"\n",
    );
    write_file(
        repo,
        "tests/central/Cargo.toml",
        "[package]\nname = \"central\"\nversion = \"0.1.0\"\nedition = \"2021\"\nautotests = false\n\n[[test]]\nname = \"workspace\"\npath = \"tests/workspace.rs\"\n",
    );
    write_file(
        repo,
        "tests/central/tests/workspace.rs",
        "#[path = \"../../../proof.rs\"]\nmod public_contract;\n",
    );
    write_file(
        repo,
        "proof.rs",
        "// bookends:LE-1\n#[test]\nfn public_contract_asserts_an_observable_result() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    write_file(
        repo,
        "tests/central/tests/workspace/unlinked.rs",
        "// bookends:LE-1\n#[test]\nfn unlinked_source_is_not_collected() {}\n",
    );
    write_file(repo, "journey.py", "# bookends:LE-1\nassert 2 + 2 == 4\n");
    write_file(
        repo,
        ".github/workflows/ci.yml",
        "jobs:\n  proof:\n    steps:\n      - run: python3 journey.py\n      - run: cargo test --workspace\n",
    );
    write_file(
        repo,
        "bookends.toml",
        "prd = \"docs/PRD.md\"\n\n[classes.e2e_journey]\npathspecs = [\"journey.py\"]\nrequired_ci_jobs = [\"proof\"]\n\n[classes.contract]\npathspecs = [\"proof.rs\", \"tests/central/tests/workspace/unlinked.rs\"]\nrequired_ci_jobs = [\"proof\"]\n",
    );
    write_file(
        repo,
        "docs/PRD.md",
        "### LE-1: Public contract\n- Status: live\n- Coverage: e2e/journey, contract\n",
    );
}

#[test]
fn backlog_t09_public_central_path_import_is_collected_and_unlinked_source_is_not() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path(), "central");
    write_central_collection_fixture(&repo);
    commit(&repo, "central public proof");
    assert_green(&run_checker(&repo));

    // The path remains approved, but the central #[path] edge is removed from
    // the proof's only collected root.  The same citation in unlinked.rs must
    // not be promoted by the directory pathspec.
    write_file(
        &repo,
        "proof.rs",
        "#[test]\nfn public_contract_asserts_an_observable_result() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    commit(&repo, "remove collected citation");
    assert_red(&run_checker(&repo), "no eligible contract citation");

    // A contract citation cannot fill the separately mandatory journey class.
    write_file(
        &repo,
        "proof.rs",
        "// bookends:LE-1\n#[test]\nfn public_contract_asserts_an_observable_result() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    write_file(&repo, "journey.py", "assert 2 + 2 == 4\n");
    commit(&repo, "remove mandatory journey citation");
    assert_red(&run_checker(&repo), "no eligible e2e/journey citation");
}

#[test]
fn backlog_t09_public_directive_scanning_ignores_literals_and_honors_real_skip() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path(), "directives");
    write_file(
        &repo,
        "bookends.toml",
        "prd = \"docs/PRD.md\"\n\n[classes.e2e_journey]\npathspecs = [\"tests/journey.py\", \"docs/example.md\", \"docs/guide.py\", \"tests/fixtures/example.py\", \"generated/journey.py\", \"vendor/journey.py\"]\nrequired_ci_jobs = [\"journey\"]\n",
    );
    write_file(
        &repo,
        "docs/PRD.md",
        "### LE-1: Public directive\n- Status: live\n- Coverage: e2e/journey\n",
    );
    write_file(
        &repo,
        ".github/workflows/ci.yml",
        "jobs:\n  journey:\n    steps:\n      - run: python3 tests/journey.py\n      - run: python3 docs/example.md\n      - run: python3 docs/guide.py\n      - run: python3 tests/fixtures/example.py\n      - run: python3 generated/journey.py\n      - run: python3 vendor/journey.py\n",
    );
    write_file(
        &repo,
        "tests/journey.py",
        "example = \"bookends:LE-1\"\ndoc = \"\"\"\n# bookends:LE-1\n\"\"\"\nfalse_skip = \"bookends:skip\"\n# bookends:LE-1\nassert True\n",
    );
    write_file(&repo, "docs/example.md", "# bookends:LE-1\n");
    write_file(
        &repo,
        "generated/journey.py",
        "# bookends:LE-1\nassert False\n",
    );
    write_file(
        &repo,
        "vendor/journey.py",
        "# bookends:LE-1\nassert False\n",
    );
    write_file(
        &repo,
        "tests/fixtures/example.py",
        "# bookends:LE-1\nassert False\n",
    );
    write_file(&repo, "docs/guide.py", "# bookends:LE-1\nassert False\n");
    commit(&repo, "directive proof");
    // bookends:LE-134 — public assertions prove strings/doc examples and
    // generated/vendor files do not create or replace a real directive.
    assert_green(&run_checker(&repo));

    write_file(
        &repo,
        "tests/journey.py",
        "example = \"bookends:LE-1\"\ndoc = \"\"\"\n# bookends:LE-1\n\"\"\"\nfalse_skip = \"bookends:skip\"\nassert True\n",
    );
    commit(&repo, "remove real directive");
    assert_red(&run_checker(&repo), "no eligible e2e/journey citation");

    write_file(
        &repo,
        "tests/journey.py",
        "false_skip = \"bookends:skip\"\n# bookends:skip\n# bookends:LE-1\nassert True\n",
    );
    commit(&repo, "honor skip directive");
    assert_red(&run_checker(&repo), "no eligible e2e/journey citation");
}
