#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct Repo {
    pub dir: TempDir,
}

impl Repo {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "Bookends Test"]);
        git(
            dir.path(),
            &["config", "user.email", "bookends-test@example.com"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, body: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, body).expect("write");
    }

    pub fn commit_all(&self, message: &str) {
        git(self.path(), &["add", "-A"]);
        git(self.path(), &["commit", "-m", message]);
    }

    pub fn check(&self) -> bookends_check::CheckReport {
        bookends_check::check_repo(self.path(), None).expect("check_repo")
    }

    pub fn check_bypass(&self, class: &str, reason: &str) -> bookends_check::CheckReport {
        bookends_check::check_repo(self.path(), Some((class, reason))).expect("check_repo")
    }
}

pub const GREEN_TOML: &str = r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]
"#;

pub const GREEN_PRD: &str =
    "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n";

pub const GREEN_WORKFLOW: &str = r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: python3 tests/journey.py
"#;

pub const GREEN_TEST: &str = "# bookends:LE-1\nprint('ok')\n";

pub fn green_graph(repo: &Repo) {
    repo.write("bookends.toml", GREEN_TOML);
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(".github/workflows/ci.yml", GREEN_WORKFLOW);
    repo.write("tests/journey.py", GREEN_TEST);
    repo.commit_all("enable bookends");
}

pub fn write_prd(repo: &Repo, body: &str) {
    repo.write("docs/PRD.md", body);
}

fn git(repo: &Path, args: &[&str]) {
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
}

pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bookends-check"))
}
