use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use xtask::hooks::validate_staged;
use xtask::process::{Cancellation, ProcessTermination};
use xtask::quality::CommandStatus;

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
        .expect("git UTF-8")
        .trim()
        .to_owned()
}

fn write(repo: &Path, path: &str, text: &str) {
    let path = repo.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, text).expect("write fixture");
}

fn init(manifest: &str, runner: &str, commit: bool) -> TempDir {
    let repo = TempDir::new().expect("repo");
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "hooks@test"]);
    git(repo.path(), &["config", "user.name", "Hooks Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    write(repo.path(), "quality/manifest.toml", manifest);
    write(repo.path(), "runner", runner);
    fs::set_permissions(
        repo.path().join("runner"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(repo.path(), &["add", "quality/manifest.toml", "runner"]);
    if commit {
        git(repo.path(), &["commit", "-m", "base"]);
    }
    repo
}

fn manifest(checks: &str) -> String {
    format!(
        r#"schema_version = 2

[defaults]
timeout_seconds = 30
max_output_bytes = 65536

[defaults.environment.set]
GIT_INDEX_FILE = "configured-index-must-not-survive"
LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE = "configured-private-index-must-not-survive"

[runner]
inputs = ["quality/manifest.toml", "runner"]

{checks}
"#
    )
}

const RUNNER: &str = r#"#!/usr/bin/env python3
import pathlib, sys
mode = sys.argv[1]
if mode == "content":
    assert pathlib.Path(sys.argv[2]).read_text() == sys.argv[3]
elif mode == "fail":
    print(sys.argv[2], file=sys.stderr)
    raise SystemExit(int(sys.argv[3]))
elif mode == "mutate":
    path = pathlib.Path(sys.argv[2])
    path.chmod(0o644)
    path.write_text("mutated\n")
elif mode == "environment-absent":
    import os
    for name in ("GIT_INDEX_FILE", "LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE"):
        assert name not in os.environ, f"{name} leaked into validation child"
    print("clean")
else:
    raise SystemExit("bad mode")
"#;

#[test]
fn exact_index_excludes_unstaged_and_untracked_product_content() {
    let checks = r#"[[checks]]
id = "index-content"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["content", "{candidate_root}/subject", "staged\n"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    write(repo.path(), "subject", "staged\n");
    git(repo.path(), &["add", "subject"]);
    write(repo.path(), "subject", "unstaged\n");
    write(repo.path(), "untracked", "must not enter candidate\n");

    let result = validate_staged(repo.path(), &Cancellation::new()).expect("staged validation");
    assert!(result.passed(), "{result:#?}");
    assert_eq!(result.checks[0].status, CommandStatus::Passed);
}

#[test]
fn explicit_fixture_index_is_the_accepted_candidate() {
    let checks = r#"[[prerequisites]]
id = "clean-prerequisite-environment"
program = "{candidate_root}/runner"
args = ["environment-absent"]
stdout_equals = "clean"
install_hint = "validation runner must be executable"

[[checks]]
id = "alternate-index"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["content", "{candidate_root}/subject", "alternate\n"]
cwd = "{candidate_root}"

[[checks]]
id = "clean-check-environment"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["environment-absent"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    let alternate = repo.path().join(".git/fixture-index");
    fs::copy(repo.path().join(".git/index"), &alternate).expect("copy fixture index");
    write(repo.path(), "subject", "alternate\n");
    let output = Command::new("/usr/bin/git")
        .args(["add", "subject"])
        .current_dir(repo.path())
        .env("GIT_INDEX_FILE", &alternate)
        .output()
        .expect("stage alternate index");
    assert!(output.status.success());
    write(repo.path(), "subject", "working tree only\n");

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["validate", "--staged"])
        .current_dir(repo.path())
        .env("GIT_INDEX_FILE", &alternate)
        .output()
        .expect("validate alternate index");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn runner_input_mismatch_fails_before_any_check() {
    let checks = r#"[[checks]]
id = "never"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["content", "{candidate_root}/runner", "impossible"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    write(repo.path(), "runner", "#!/bin/sh\nexit 99\n");

    let error = validate_staged(repo.path(), &Cancellation::new()).unwrap_err();
    assert!(format!("{error:#}").contains("runner input"));
}

#[test]
fn unborn_repository_and_staged_deletion_are_explicitly_supported() {
    let no_checks = manifest(
        r#"[[checks]]
id = "unborn-pass"
phases = ["pre-commit"]
scope = "repository"
program = "/usr/bin/true"
args = []
cwd = "{candidate_root}"
"#,
    );
    let unborn = init(&no_checks, RUNNER, false);
    let unborn_result = validate_staged(unborn.path(), &Cancellation::new()).expect("unborn");
    assert!(unborn_result.passed());
    assert_eq!(
        unborn_result.binding.candidate_revision,
        unborn_result.binding.candidate_tree
    );

    let checks = r#"[[checks]]
id = "deleted"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["content", "{candidate_root}/kept", "kept\n"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    write(repo.path(), "deleted", "gone\n");
    write(repo.path(), "kept", "kept\n");
    git(repo.path(), &["add", "deleted", "kept"]);
    git(repo.path(), &["commit", "-m", "files"]);
    fs::remove_file(repo.path().join("deleted")).unwrap();
    git(repo.path(), &["add", "-u"]);
    let result = validate_staged(repo.path(), &Cancellation::new()).expect("staged deletion");
    assert!(result.passed(), "{result:#?}");
}

#[test]
fn deterministic_failures_are_aggregated_in_manifest_order() {
    let checks = r#"[[checks]]
id = "first"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["fail", "first failed", "3"]
cwd = "{candidate_root}"

[[checks]]
id = "second"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["fail", "second failed", "7"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    let result = validate_staged(repo.path(), &Cancellation::new()).expect("validation result");
    assert!(!result.passed());
    assert_eq!(
        result
            .checks
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(
        result
            .checks
            .iter()
            .all(|record| record.status == CommandStatus::Failed)
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
fn staged_candidate_mutation_blocks_later_commands_and_final_verdict() {
    let checks = r#"[[checks]]
id = "mutation"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["mutate", "{candidate_root}/subject"]
cwd = "{candidate_root}"

[[checks]]
id = "must-not-run"
phases = ["pre-commit"]
scope = "repository"
program = "/usr/bin/true"
args = []
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    write(repo.path(), "subject", "original\n");
    git(repo.path(), &["add", "subject"]);
    let result = validate_staged(repo.path(), &Cancellation::new()).expect("mutation evidence");
    assert!(!result.passed());
    assert_eq!(result.checks[0].status, CommandStatus::Failed);
    assert_eq!(result.checks[1].status, CommandStatus::Blocked);
    assert!(result.checks[1].process.is_none());
    assert!(!result.final_source_verified);
}

#[test]
fn staged_cli_propagates_failure_exit() {
    let checks = r#"[[checks]]
id = "failure"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["fail", "expected diagnostic", "9"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["validate", "--staged"])
        .current_dir(repo.path())
        .output()
        .expect("run xtask");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failure"), "{stderr}");
    assert!(stderr.contains("expected diagnostic"), "{stderr}");
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

#[test]
fn interruption_cancels_group_before_candidate_cleanup() {
    let checks = r#"[[checks]]
id = "long"
phases = ["pre-commit"]
scope = "repository"
program = "/bin/sh"
args = ["-c", "trap '' TERM; sleep 30 & echo $! > \"$1/child.pid\"; touch \"$1/started\"; wait", "runner", "{git_directory}"]
cwd = "{candidate_root}"
timeout_seconds = 60
"#;
    let repo = init(&manifest(checks), RUNNER, true);
    let temp = TempDir::new().expect("candidate temp parent");
    let child = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["validate", "--staged"])
        .current_dir(repo.path())
        .env("TMPDIR", temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn staged validation");
    let started = repo.path().join(".git/started");
    wait_for(&started);
    let status = Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(status.success());
    let repeated = Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send repeated interruption");
    assert!(repeated.success());
    let output = child.wait_with_output().expect("await staged validation");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interrupted"));

    let child_pid = fs::read_to_string(repo.path().join(".git/child.pid")).unwrap();
    let live = Command::new("/bin/kill")
        .args(["-0", child_pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("probe child")
        .success();
    assert!(!live, "validation descendant survived interruption");
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        0,
        "candidate root leaked"
    );
}

#[test]
fn tracked_adapter_is_tiny_scrubs_git_environment_and_execs_stable_command() {
    let tracked = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.githooks/pre-commit");
    let hook = fs::read_to_string(&tracked).unwrap();
    assert!(hook.contains("/usr/bin/git rev-parse --show-toplevel"));
    assert!(hook.contains("/usr/bin/git rev-parse --local-env-vars"));
    assert!(hook.contains("exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --staged"));
    assert!(hook.contains("LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE"));
    assert!(!hook.contains("quality/manifest.toml"));
    assert!(!hook.contains("GATE_PATHS"));
    assert!(hook.lines().count() <= 15);
    assert_eq!(
        fs::metadata(&tracked).unwrap().permissions().mode() & 0o111,
        0o111
    );

    let parent = TempDir::new().expect("adapter parent");
    let repo = parent.path().join("repo with spaces");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    fs::create_dir_all(repo.join(".githooks")).unwrap();
    fs::copy(&tracked, repo.join(".githooks/pre-commit")).unwrap();
    fs::set_permissions(
        repo.join(".githooks/pre-commit"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::create_dir(repo.join("nested")).unwrap();
    let bin = parent.path().join("bin");
    fs::create_dir(&bin).unwrap();
    write(
        parent.path(),
        "bin/cargo",
        "#!/bin/sh\nprintf '%s\\n%s\\n%s\\n%s\\n%s\\n%s\\n' \"$PWD\" \"$*\" \"${GIT_PREFIX-unset}\" \"${GIT_INDEX_FILE-unset}\" \"${LOOP_ENGINE_INTERNAL_GIT_INDEX_FILE-unset}\" \"${RUSTUP_TOOLCHAIN-unset}\" > .git/adapter.log\nexit 23\n",
    );
    fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    let fixture_index = repo.join(".git/fixture-index");
    fs::write(&fixture_index, []).unwrap();
    let output = Command::new(repo.join(".githooks/pre-commit"))
        .current_dir(repo.join("nested"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("GIT_PREFIX", "must-be-scrubbed")
        .env("GIT_INDEX_FILE", &fixture_index)
        .env("RUSTUP_TOOLCHAIN", "must-be-scrubbed")
        .output()
        .expect("run adapter");
    assert_eq!(output.status.code(), Some(23));
    let log = fs::read_to_string(repo.join(".git/adapter.log")).unwrap();
    let lines = log.lines().collect::<Vec<_>>();
    assert_eq!(
        Path::new(lines[0]).canonicalize().unwrap(),
        repo.canonicalize().unwrap()
    );
    assert_eq!(lines[1], "xtask validate --staged");
    assert_eq!(lines[2], "unset");
    assert_eq!(lines[3], "unset");
    assert_eq!(Path::new(lines[4]), fixture_index);
    assert_eq!(lines[5], "unset");

    for malformed in ["", "../.git"] {
        let rejected = Command::new(repo.join(".githooks/pre-commit"))
            .current_dir(repo.join("nested"))
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("GIT_INDEX_FILE", malformed)
            .output()
            .expect("run malformed-index adapter");
        assert_eq!(rejected.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("pre-commit:"));
    }
}

#[test]
fn adapter_commit_uses_relative_alternate_index_and_scrubs_check_processes() {
    let prerequisites_and_checks = r#"[[prerequisites]]
id = "clean-prerequisite-environment"
program = "{candidate_root}/runner"
args = ["environment-absent"]
stdout_equals = "clean"
install_hint = "validation runner must be executable"

[[checks]]
id = "committed-candidate"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["content", "{candidate_root}/subject", "alternate commit\n"]
cwd = "{candidate_root}"

[[checks]]
id = "clean-check-environment"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["environment-absent"]
cwd = "{candidate_root}"
"#;
    let repo = init(&manifest(prerequisites_and_checks), RUNNER, true);
    write(repo.path(), "subject", "base\n");
    git(repo.path(), &["add", "subject"]);
    git(repo.path(), &["commit", "-m", "subject base"]);

    let tracked = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.githooks/pre-commit");
    fs::create_dir_all(repo.path().join(".githooks")).unwrap();
    fs::copy(&tracked, repo.path().join(".githooks/pre-commit")).unwrap();
    fs::set_permissions(
        repo.path().join(".githooks/pre-commit"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(repo.path(), &["config", "core.hooksPath", ".githooks"]);

    let alternate = repo.path().join(".git/alternate-index");
    fs::copy(repo.path().join(".git/index"), &alternate).unwrap();
    write(repo.path(), "subject", "alternate commit\n");
    let staged = Command::new("/usr/bin/git")
        .args(["add", "subject"])
        .current_dir(repo.path())
        .env("GIT_INDEX_FILE", ".git/alternate-index")
        .output()
        .unwrap();
    assert!(
        staged.status.success(),
        "{}",
        String::from_utf8_lossy(&staged.stderr)
    );
    write(repo.path(), "subject", "working tree only\n");

    let bin = repo.path().join("fake-bin");
    fs::create_dir(&bin).unwrap();
    write(
        repo.path(),
        "fake-bin/cargo",
        "#!/bin/sh\n[ \"$*\" = \"xtask validate --staged\" ] || exit 97\nexec \"$LOOP_ENGINE_XTASK\" validate --staged\n",
    );
    fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    let committed = Command::new("/usr/bin/git")
        .args(["commit", "-m", "alternate candidate"])
        .current_dir(repo.path())
        .env("GIT_INDEX_FILE", ".git/alternate-index")
        .env("LOOP_ENGINE_XTASK", env!("CARGO_BIN_EXE_xtask"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(
        committed.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&committed.stdout),
        String::from_utf8_lossy(&committed.stderr)
    );
    assert_eq!(
        git(repo.path(), &["show", "HEAD:subject"]),
        "alternate commit"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("subject")).unwrap(),
        "working tree only\n"
    );
}

fn installer_manifest(prerequisite_program: &str) -> String {
    format!(
        r#"schema_version = 2

[defaults]
timeout_seconds = 30
max_output_bytes = 65536

[runner]
inputs = [".githooks", "quality/manifest.toml", "runner"]

[[prerequisites]]
id = "required-tool"
program = "{prerequisite_program}"
args = ["probe"]
stdout_equals = "ready"
install_hint = "install the required test tool"

[[checks]]
id = "unused-install-check"
phases = ["pre-commit"]
scope = "repository"
program = "{{candidate_root}}/runner"
args = ["check"]
cwd = "{{candidate_root}}"
"#
    )
}

const INSTALL_RUNNER: &str =
    "#!/bin/sh\ncase \"$1\" in probe) printf 'ready\\n' ;; check) exit 91 ;; *) exit 92 ;; esac\n";

fn init_install_repository(commit: bool) -> TempDir {
    let repo = TempDir::new().expect("install repo");
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "hooks-install@test"]);
    git(repo.path(), &["config", "user.name", "Hooks Install Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    fs::create_dir_all(repo.path().join(".githooks")).unwrap();
    fs::create_dir_all(repo.path().join("quality")).unwrap();
    for hook in ["pre-commit", "pre-push"] {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.githooks")
            .join(hook);
        let destination = repo.path().join(".githooks").join(hook);
        fs::copy(source, &destination).unwrap();
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755)).unwrap();
    }
    write(
        repo.path(),
        "quality/manifest.toml",
        &installer_manifest("{candidate_root}/runner"),
    );
    write(repo.path(), "runner", INSTALL_RUNNER);
    fs::set_permissions(
        repo.path().join("runner"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(repo.path(), &["add", ".githooks", "quality", "runner"]);
    if commit {
        git(repo.path(), &["commit", "-m", "install fixture"]);
    }
    repo
}

fn run_install(repo: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["hooks", "install"])
        .current_dir(repo)
        .output()
        .expect("run hooks install")
}

fn assert_install_failed_without_config(repo: &Path, expected: &str) {
    let output = run_install(repo);
    assert!(
        !output.status.success(),
        "installation unexpectedly succeeded"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "missing `{expected}` in stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let query = Command::new("/usr/bin/git")
        .args(["config", "--local", "--get-all", "core.hooksPath"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(1));
    assert!(query.stdout.is_empty());
}

#[test]
fn fresh_install_sets_only_local_path_and_both_adapters_invoke_final_v2_commands() {
    let repo = init_install_repository(true);
    let before_status = git(repo.path(), &["status", "--porcelain=v1"]);
    let before_hooks = [
        fs::read(repo.path().join(".githooks/pre-commit")).unwrap(),
        fs::read(repo.path().join(".githooks/pre-push")).unwrap(),
    ];

    let output = run_install(repo.path());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        git(
            repo.path(),
            &["config", "--local", "--get-all", "core.hooksPath"]
        ),
        ".githooks"
    );
    assert_eq!(
        git(repo.path(), &["status", "--porcelain=v1"]),
        before_status
    );
    assert_eq!(
        fs::read(repo.path().join(".githooks/pre-commit")).unwrap(),
        before_hooks[0]
    );
    assert_eq!(
        fs::read(repo.path().join(".githooks/pre-push")).unwrap(),
        before_hooks[1]
    );
    assert!(!repo.path().join(".git/hooks/pre-commit").exists());
    assert!(!repo.path().join(".git/hooks/pre-push").exists());

    let bin = repo.path().join("fake-cargo-bin");
    fs::create_dir(&bin).unwrap();
    write(
        repo.path(),
        "fake-cargo-bin/cargo",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> .git/installed-adapters.log\n",
    );
    fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let pre_commit = Command::new(repo.path().join(".githooks/pre-commit"))
        .current_dir(repo.path())
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(pre_commit.status.success());
    let mut pre_push = Command::new(repo.path().join(".githooks/pre-push"))
        .current_dir(repo.path())
        .env("PATH", &path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    pre_push
        .stdin
        .take()
        .unwrap()
        .write_all(b"refs/heads/main 0000 refs/heads/main 0000\n")
        .unwrap();
    assert!(pre_push.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(repo.path().join(".git/installed-adapters.log")).unwrap(),
        "xtask validate --staged\nxtask validate --publication --updates-stdin\n"
    );
}

#[test]
fn exact_existing_install_is_idempotent_after_full_validation() {
    let repo = init_install_repository(true);
    assert!(run_install(repo.path()).status.success());
    let config = fs::read(repo.path().join(".git/config")).unwrap();
    assert!(run_install(repo.path()).status.success());
    assert_eq!(fs::read(repo.path().join(".git/config")).unwrap(), config);
}

#[test]
fn conflicting_local_configuration_is_reported_and_preserved() {
    let repo = init_install_repository(true);
    git(
        repo.path(),
        &["config", "--local", "core.hooksPath", "custom-hooks"],
    );
    let before = fs::read(repo.path().join(".git/config")).unwrap();
    let output = run_install(repo.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("conflicting local core.hooksPath"));
    assert!(stderr.contains("`custom-hooks`"));
    assert_eq!(fs::read(repo.path().join(".git/config")).unwrap(), before);
}

#[test]
fn unborn_head_fails_before_configuration_change() {
    let repo = init_install_repository(false);
    assert_install_failed_without_config(repo.path(), "unborn HEAD");
}

#[test]
fn missing_hook_fails_before_configuration_change() {
    let repo = init_install_repository(true);
    git(repo.path(), &["rm", ".githooks/pre-push"]);
    git(repo.path(), &["commit", "-m", "remove pre-push"]);
    assert_install_failed_without_config(repo.path(), "pre-push` is missing from HEAD");
}

#[test]
fn non_executable_hook_fails_before_configuration_change() {
    let repo = init_install_repository(true);
    fs::set_permissions(
        repo.path().join(".githooks/pre-commit"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    git(repo.path(), &["add", ".githooks/pre-commit"]);
    git(repo.path(), &["commit", "-m", "remove executable bit"]);
    assert_install_failed_without_config(repo.path(), "pre-commit` is not executable in HEAD");
}

#[test]
fn wrong_adapter_content_and_final_command_fail_before_configuration_change() {
    let repo = init_install_repository(true);
    let path = repo.path().join(".githooks/pre-push");
    let wrong = fs::read_to_string(&path)
        .unwrap()
        .replace("--updates-stdin", "--staged");
    fs::write(&path, wrong).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    git(repo.path(), &["add", ".githooks/pre-push"]);
    git(repo.path(), &["commit", "-m", "wrong pre-push command"]);
    assert_install_failed_without_config(repo.path(), "final v2 adapter contract");
}

#[test]
fn prerequisite_tool_failure_prints_hint_and_does_not_configure_hooks() {
    let repo = init_install_repository(true);
    write(
        repo.path(),
        "quality/manifest.toml",
        &installer_manifest("missing-install-prerequisite-tool"),
    );
    git(repo.path(), &["add", "quality/manifest.toml"]);
    git(repo.path(), &["commit", "-m", "missing prerequisite"]);
    let output = run_install(repo.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("required-tool"));
    assert!(stderr.contains("install the required test tool"));
    let query = Command::new("/usr/bin/git")
        .args(["config", "--local", "--get", "core.hooksPath"])
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert_eq!(query.code(), Some(1));
}

#[test]
fn sigterm_cancels_install_prerequisite_group_before_cleanup_and_configuration() {
    let repo = init_install_repository(true);
    let slow_manifest = r#"schema_version = 2

[defaults]
timeout_seconds = 60
max_output_bytes = 65536

[runner]
inputs = [".githooks", "quality/manifest.toml", "runner"]

[[prerequisites]]
id = "slow-tool"
program = "{candidate_root}/runner"
args = ["slow", "{git_directory}", "{candidate_root}", "{scratch_root}", "{cache_root}", "{target_root}"]
install_hint = "slow fixture"

[[checks]]
id = "unused-install-check"
phases = ["pre-commit"]
scope = "repository"
program = "{candidate_root}/runner"
args = ["unused"]
cwd = "{candidate_root}"
"#;
    let slow_runner = r#"#!/bin/sh
set -eu
[ "$1" = slow ] || exit 97
git_directory=$2
printf '%s\n' "$3" "$4" "$5" "$6" > "$git_directory/candidate-roots"
trap '' TERM
(
    trap '' TERM
    sleep 2
    touch "$git_directory/descendant-marker"
    sleep 30
) &
echo $! > "$git_directory/descendant.pid"
touch "$git_directory/prerequisite-started"
wait
"#;
    write(repo.path(), "quality/manifest.toml", slow_manifest);
    write(repo.path(), "runner", slow_runner);
    fs::set_permissions(
        repo.path().join("runner"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(repo.path(), &["add", "quality/manifest.toml", "runner"]);
    git(repo.path(), &["commit", "-m", "slow install prerequisite"]);

    let source_status = git(repo.path(), &["status", "--porcelain=v1"]);
    let source_runner = fs::read(repo.path().join("runner")).unwrap();
    let temp = TempDir::new().expect("candidate temp parent");
    let child = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["hooks", "install"])
        .current_dir(repo.path())
        .env("TMPDIR", temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hook installation");
    wait_for(&repo.path().join(".git/prerequisite-started"));

    let signal = Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(signal.success());
    let output = child.wait_with_output().expect("await hook installation");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hook installation interrupted by signal 15"),
        "{stderr}"
    );

    let descendant_pid = fs::read_to_string(repo.path().join(".git/descendant.pid")).unwrap();
    let descendant_live = Command::new("/bin/kill")
        .args(["-0", descendant_pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("probe prerequisite descendant")
        .success();
    assert!(!descendant_live, "prerequisite descendant survived SIGTERM");
    thread::sleep(Duration::from_millis(2200));
    assert!(
        !repo.path().join(".git/descendant-marker").exists(),
        "cancelled descendant wrote its survival marker"
    );

    let materialized_roots = fs::read_to_string(repo.path().join(".git/candidate-roots")).unwrap();
    for root in materialized_roots.lines() {
        assert!(
            !Path::new(root).exists(),
            "materialized candidate root leaked: {root}"
        );
    }
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        0,
        "candidate storage leaked"
    );
    let query = Command::new("/usr/bin/git")
        .args(["config", "--local", "--get-all", "core.hooksPath"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(1));
    assert!(query.stdout.is_empty());
    assert_eq!(
        git(repo.path(), &["status", "--porcelain=v1"]),
        source_status
    );
    assert_eq!(fs::read(repo.path().join("runner")).unwrap(), source_runner);
}

#[test]
fn git_configuration_failure_leaves_source_and_candidate_untouched() {
    let repo = init_install_repository(true);
    let before_status = git(repo.path(), &["status", "--porcelain=v1"]);
    fs::write(repo.path().join(".git/config.lock"), b"locked").unwrap();
    let output = run_install(repo.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("git config --local"));
    assert_eq!(
        git(repo.path(), &["status", "--porcelain=v1"]),
        before_status
    );
    fs::remove_file(repo.path().join(".git/config.lock")).unwrap();
    let query = Command::new("/usr/bin/git")
        .args(["config", "--local", "--get", "core.hooksPath"])
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert_eq!(query.code(), Some(1));
}

#[test]
fn linked_worktree_installs_through_shared_local_repository_config() {
    let repo = init_install_repository(true);
    let linked = repo.path().join("linked-worktree");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "linked-install-test",
            linked.to_str().unwrap(),
        ],
    );
    let common = git(&linked, &["rev-parse", "--git-common-dir"]);
    let directory = git(&linked, &["rev-parse", "--git-dir"]);
    assert_ne!(Path::new(&common), Path::new(&directory));

    let output = run_install(&linked);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        git(&linked, &["config", "--local", "--get", "core.hooksPath"]),
        ".githooks"
    );
    assert_eq!(
        git(
            repo.path(),
            &["config", "--local", "--get", "core.hooksPath"]
        ),
        ".githooks"
    );
    assert!(!PathBuf::from(directory).join("config").exists());
}
