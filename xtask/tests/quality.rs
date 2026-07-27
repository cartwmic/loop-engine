use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;
use xtask::candidate::{Candidate, PreparedCandidate};
use xtask::config::{Phase, SemanticRequirement};
use xtask::process::{ProcessTermination, SpawnFailureKind};
use xtask::quality::{CommandFailureKind, CommandKind, CommandStatus, run};

fn runner_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/quality/runner.py")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git spawn");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout UTF-8")
        .trim_end()
        .to_owned()
}

fn write(repo: &Path, relative: &str, contents: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write fixture");
}

fn manifest(prerequisites: &str, checks: &str) -> String {
    format!(
        r#"schema_version = 2

[defaults]
timeout_seconds = 10
max_output_bytes = 65536

[defaults.environment]
unset = ["RUSTUP_TOOLCHAIN", "REMOVE_ME"]

[defaults.environment.set]
DEFAULT_VALUE = "{{candidate_tree}}"
OVERRIDE_VALUE = "default"
SCRATCH = "{{scratch_root}}/nested"
CACHE = "{{cache_root}}/nested"
TARGET = "{{target_root}}/nested"
REMOVE_ME = "must-not-survive"

[runner]
inputs = ["quality/manifest.toml", "runner.py"]

{prerequisites}
{checks}
"#
    )
}

fn prepared(manifest_text: &str, changes: &[(&str, &str)]) -> (TempDir, PreparedCandidate) {
    let repo = TempDir::new().expect("repo tempdir");
    let candidate = prepared_at(repo.path(), manifest_text, changes);
    (repo, candidate)
}

fn prepared_at(repo: &Path, manifest_text: &str, changes: &[(&str, &str)]) -> PreparedCandidate {
    fs::create_dir_all(repo).expect("create repo");
    git(repo, &["init", "-b", "main"]);
    git(repo, &["config", "user.email", "quality@test"]);
    git(repo, &["config", "user.name", "Quality Test"]);
    git(repo, &["config", "commit.gpgsign", "false"]);

    fs::copy(runner_fixture(), repo.join("runner.py")).expect("copy runner");
    fs::set_permissions(repo.join("runner.py"), fs::Permissions::from_mode(0o755))
        .expect("runner executable");
    write(repo, "quality/manifest.toml", manifest_text);
    write(repo, "protected.txt", "original\n");
    write(repo, "subdir/.keep", "fixture\n");
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", "base"]);
    let base = git(repo, &["rev-parse", "HEAD"]);

    for (path, contents) in changes {
        write(repo, path, contents);
    }
    git(repo, &["add", "-A"]);
    if changes.is_empty() {
        git(repo, &["commit", "--allow-empty", "-m", "candidate"]);
    } else {
        git(repo, &["commit", "-m", "candidate"]);
    }
    let head = git(repo, &["rev-parse", "HEAD"]);

    Candidate::revision(repo, Some(OsStr::new(&base)), OsStr::new(&head))
        .expect("revision candidate")
        .prepare(SemanticRequirement::Optional)
        .expect("prepared candidate")
}

fn base_checks() -> &'static str {
    r#"[[checks]]
id = "pre-only"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["emit", "pre"]
cwd = "{candidate_root}"

[[checks]]
id = "publication-record"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["record", "literal space", "", "{base_revision}", "{candidate_revision}", "{git_directory}"]
cwd = "{candidate_root}/subdir"

[checks.environment]
unset = ["OVERRIDE_VALUE"]

[checks.environment.set]
OVERRIDE_VALUE = "check"
REMOVE_ME = "still-must-not-survive"
"#
}

#[test]
fn prerequisites_run_first_then_phase_checks_with_exact_expansion_and_evidence() {
    let prerequisites = r#"[[prerequisites]]
id = "probe-one"
program = "{candidate_root}/runner.py"
args = ["emit", "tool 1.2.3"]
stdout_equals = "tool 1.2.3"
install_hint = "{candidate_root}/runner.py install SHOULD-NOT-RUN"

[[prerequisites]]
id = "probe-two"
program = "{candidate_root}/runner.py"
args = ["emit", "ready"]
install_hint = "never execute this"
"#;
    let (_repo, candidate) = prepared(
        &manifest(prerequisites, base_checks()),
        &[("ordinary.txt", "changed\n")],
    );

    let result = run(&candidate, Phase::Publication);
    assert!(result.passed(), "{result:#?}");
    assert_eq!(result.prerequisites.len(), 2);
    assert_eq!(result.checks.len(), 1);
    assert_eq!(result.prerequisites[0].id, "probe-one");
    assert_eq!(result.prerequisites[1].id, "probe-two");
    assert_eq!(result.checks[0].id, "publication-record");
    assert!(
        result
            .prerequisites
            .iter()
            .all(|record| record.kind == CommandKind::Prerequisite)
    );
    assert_eq!(result.checks[0].kind, CommandKind::Check);
    assert_eq!(result.checks[0].status, CommandStatus::Passed);
    assert_eq!(result.checks[0].cwd, candidate.source_root().join("subdir"));
    assert_eq!(result.checks[0].args[0..3], ["record", "literal space", ""]);
    assert_eq!(result.checks[0].args[3], candidate.base_revision());
    assert_eq!(result.checks[0].args[4], candidate.candidate_revision());
    assert_eq!(
        result.checks[0].args[5],
        candidate.repository().git_directory().to_string_lossy()
    );
    assert_eq!(
        result.checks[0]
            .environment
            .set()
            .get("DEFAULT_VALUE")
            .unwrap(),
        candidate.candidate_tree()
    );
    assert!(
        !result.checks[0]
            .environment
            .set()
            .contains_key("OVERRIDE_VALUE")
    );
    assert!(
        result.checks[0]
            .environment
            .unset()
            .contains("OVERRIDE_VALUE")
    );
    assert!(!result.checks[0].environment.set().contains_key("REMOVE_ME"));
    assert!(result.checks[0].environment.unset().contains("REMOVE_ME"));
    assert!(candidate.scratch_root().join("nested/probe-0").is_file());
    assert!(candidate.cache_root().join("nested/probe-1").is_file());
    assert!(candidate.target_root().join("nested/probe-2").is_file());
    let process = result.checks[0].process.as_ref().expect("process evidence");
    assert!(process.success());
    assert!(process.stdout.complete());
    assert!(process.stderr.complete());
    assert!(result.checks[0].source_verified == Some(true));
    assert!(result.final_source_verified);
    let machine = serde_json::to_value(&result).expect("machine-readable evidence");
    assert_eq!(machine["phase"], "publication");
    assert!(machine["checks"][0]["process"]["duration_millis"].is_u64());
    assert!(machine["checks"][0]["process"]["cleanup"]["kind"].is_string());
    assert_eq!(result.binding.base_revision, candidate.base_revision());
    assert_eq!(
        result.binding.candidate_revision,
        candidate.candidate_revision()
    );
    assert_eq!(result.binding.candidate_tree, candidate.candidate_tree());

    let output: Value = serde_json::from_slice(process.stdout.exact_bytes()).expect("record JSON");
    assert_eq!(
        output["cwd"],
        candidate
            .source_root()
            .join("subdir")
            .canonicalize()
            .expect("canonical cwd")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(output["removed"], Value::Null);
}

#[test]
fn exact_stdout_mismatch_and_multiple_process_failures_are_all_collected() {
    let prerequisites = r#"[[prerequisites]]
id = "wrong-version"
program = "{candidate_root}/runner.py"
args = ["emit", "tool 2"]
stdout_equals = "tool 1"
install_hint = "{candidate_root}/runner.py install {scratch_root}/installed"
"#;
    let checks = r#"[[checks]]
id = "first-failure"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["fail", "first", "3"]
cwd = "{candidate_root}"

[[checks]]
id = "second-failure"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["fail", "second", "7"]
cwd = "{candidate_root}"
"#;
    let (_repo, candidate) = prepared(
        &manifest(prerequisites, checks),
        &[("ordinary.txt", "changed\n")],
    );

    let result = run(&candidate, Phase::Publication);
    assert!(!result.passed());
    assert_eq!(result.prerequisites[0].status, CommandStatus::Failed);
    assert_eq!(
        result.prerequisites[0].failure.as_ref().unwrap().kind,
        CommandFailureKind::StdoutMismatch
    );
    assert_eq!(
        result.prerequisites[0].install_hint.as_deref(),
        Some("{candidate_root}/runner.py install {scratch_root}/installed")
    );
    assert!(
        !candidate.scratch_root().join("installed").exists(),
        "install_hint must be evidence only"
    );
    assert_eq!(result.checks.len(), 2);
    assert!(
        result
            .checks
            .iter()
            .all(|record| record.status == CommandStatus::Failed)
    );
    assert!(
        result
            .checks
            .iter()
            .all(|record| record.source_verified == Some(true))
    );
    assert!(matches!(
        result.checks[0].process.as_ref().unwrap().termination,
        ProcessTermination::Exit { code: 3 }
    ));
    assert!(matches!(
        result.checks[1].process.as_ref().unwrap().termination,
        ProcessTermination::Exit { code: 7 }
    ));
}

#[test]
fn changed_file_scope_appends_sorted_distinct_unusual_paths() {
    let checks = r#"[[checks]]
id = "changed"
phases = ["publication"]
scope = "changed-files"
program = "{candidate_root}/runner.py"
args = ["record"]
cwd = "{candidate_root}"
"#;
    let (_repo, candidate) = prepared(
        &manifest("", checks),
        &[
            ("z space.txt", "z\n"),
            ("-option.txt", "o\n"),
            ("a[bracket].txt", "a\n"),
        ],
    );

    let result = run(&candidate, Phase::Publication);
    assert!(result.passed(), "{result:#?}");
    assert_eq!(
        &result.checks[0].args[1..],
        ["-option.txt", "a[bracket].txt", "z space.txt"]
    );
}

#[test]
fn empty_changed_file_scope_is_skipped_success_without_spawn() {
    let checks = r#"[[checks]]
id = "changed"
phases = ["pre-commit"]
scope = "changed-files"
program = "missing-program-must-not-spawn"
args = []
cwd = "{candidate_root}"
"#;
    let (_repo, candidate) = prepared(&manifest("", checks), &[]);

    let result = run(&candidate, Phase::PreCommit);
    assert!(result.passed(), "{result:#?}");
    assert_eq!(result.checks[0].status, CommandStatus::SkippedSuccess);
    assert!(result.checks[0].process.is_none());
    assert_eq!(result.checks[0].source_verified, None);
}

#[test]
fn empty_changed_file_scope_preflights_cwd_before_skipped_success() {
    let checks = r#"[[checks]]
id = "changed"
phases = ["pre-commit"]
scope = "changed-files"
program = "missing-program-must-not-spawn"
args = []
cwd = "{candidate_root}/missing"
"#;
    let (_repo, candidate) = prepared(&manifest("", checks), &[]);

    let result = run(&candidate, Phase::PreCommit);
    assert!(!result.passed());
    assert_eq!(result.checks[0].status, CommandStatus::Failed);
    assert_eq!(
        result.checks[0]
            .process
            .as_ref()
            .expect("typed cwd preflight")
            .termination
            .spawn_failure_kind(),
        Some(SpawnFailureKind::InvalidCwd)
    );
    assert_eq!(result.checks[0].source_verified, None);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_repository_path_is_resolved_only_when_placeholder_is_referenced() {
    let parent = TempDir::new().expect("parent tempdir");
    let repo = parent
        .path()
        .join(OsString::from_vec(vec![b'r', b'e', b'p', b'o', b'-', 0xff]));
    let checks = r#"[[checks]]
id = "unrelated"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["emit", "ok"]
cwd = "{candidate_root}"

[[checks]]
id = "references-git-path"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["record", "literal", "{candidate_revision}", "{git_directory}"]
cwd = "{candidate_root}/subdir"
"#;
    let candidate = prepared_at(
        &repo,
        &manifest("", checks),
        &[("ordinary.txt", "changed\n")],
    );

    let result = run(&candidate, Phase::Publication);
    assert!(!result.passed());
    assert_eq!(result.checks[0].status, CommandStatus::Passed);

    let failed = &result.checks[1];
    assert_eq!(failed.status, CommandStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().expect("failure").kind,
        CommandFailureKind::Configuration
    );
    assert!(failed.process.is_none());
    assert_eq!(
        failed.program,
        "{candidate_root}/runner.py".replace(
            "{candidate_root}",
            candidate.source_root().to_str().unwrap()
        )
    );
    assert_eq!(
        failed.program_expansion.declared,
        "{candidate_root}/runner.py"
    );
    assert_eq!(
        failed.program_expansion.expanded.as_deref(),
        Some(failed.program.as_str())
    );
    assert_eq!(
        failed.args,
        [
            "record",
            "literal",
            candidate.candidate_revision(),
            "{git_directory}"
        ]
    );
    assert!(failed.args_expansion[0].expanded.is_some());
    assert!(failed.args_expansion[2].expanded.is_some());
    assert!(failed.args_expansion[3].expanded.is_none());
    assert!(
        failed.args_expansion[3]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("git directory is not valid UTF-8"))
    );
    assert_eq!(failed.cwd, candidate.source_root().join("subdir"));
    assert_eq!(failed.cwd_expansion.declared, "{candidate_root}/subdir");
    assert!(failed.cwd_expansion.expanded.is_some());
    assert_eq!(failed.timeout_seconds, 10);
    assert_eq!(failed.max_output_bytes, 65536);

    let evidence = serde_json::to_value(failed).expect("lossless machine evidence");
    assert_eq!(evidence["args_expansion"][3]["declared"], "{git_directory}");
    assert!(evidence["args_expansion"][3]["expanded"].is_null());
    assert_eq!(evidence["timeout_seconds"], 10);
    assert_eq!(evidence["max_output_bytes"], 65536);
}

#[test]
fn pre_spawn_rejection_is_typed_and_does_not_hide_later_failure() {
    let checks = r#"[[checks]]
id = "bad-cwd"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["emit", "not run"]
cwd = "{candidate_root}/missing"

[[checks]]
id = "later-failure"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/runner.py"
args = ["fail", "later", "9"]
cwd = "{candidate_root}"
"#;
    let (_repo, candidate) = prepared(&manifest("", checks), &[("ordinary.txt", "changed\n")]);

    let result = run(&candidate, Phase::Publication);
    assert!(!result.passed());
    assert_eq!(result.checks.len(), 2);
    let bad = result.checks[0]
        .process
        .as_ref()
        .expect("typed pre-spawn outcome");
    assert_eq!(
        bad.termination.spawn_failure_kind(),
        Some(SpawnFailureKind::InvalidCwd)
    );
    assert_eq!(result.checks[0].source_verified, None);
    assert!(matches!(
        result.checks[1].process.as_ref().unwrap().termination,
        ProcessTermination::Exit { code: 9 }
    ));
}

fn mutation_blocks_following(mode: &str) {
    let checks = format!(
        r#"[[checks]]
id = "mutator"
phases = ["publication"]
scope = "repository"
program = "{{candidate_root}}/runner.py"
args = ["{mode}", "{{candidate_root}}/protected.txt"]
cwd = "{{candidate_root}}"

[[checks]]
id = "must-be-blocked"
phases = ["publication"]
scope = "repository"
program = "{{candidate_root}}/runner.py"
args = ["install", "{{scratch_root}}/should-not-exist"]
cwd = "{{candidate_root}}"
"#
    );
    let (_repo, candidate) = prepared(&manifest("", &checks), &[("ordinary.txt", "changed\n")]);

    let result = run(&candidate, Phase::Publication);
    assert!(!result.passed());
    assert_eq!(result.checks.len(), 2);
    assert_eq!(result.checks[0].status, CommandStatus::Failed);
    assert_eq!(
        result.checks[0].failure.as_ref().unwrap().kind,
        CommandFailureKind::CandidateMutation
    );
    assert_eq!(result.checks[0].source_verified, Some(false));
    assert_eq!(result.checks[1].status, CommandStatus::Blocked);
    assert_eq!(
        result.checks[1].failure.as_ref().unwrap().kind,
        CommandFailureKind::CandidateMutation
    );
    assert!(result.checks[1].process.is_none());
    assert!(!candidate.scratch_root().join("should-not-exist").exists());
    assert!(!result.final_source_verified);
    assert!(result.final_failure.is_some());
}

#[test]
fn content_mutation_fails_closed_and_blocks_following_child() {
    mutation_blocks_following("mutate-content");
}

#[test]
fn mode_mutation_fails_closed_and_blocks_following_child() {
    mutation_blocks_following("mutate-mode");
}
