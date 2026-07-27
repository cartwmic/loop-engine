use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use xtask::process::Cancellation;
use xtask::report::{DerivedDisposition, Store};
use xtask::{run_advisory, run_advisory_with_cancellation};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn write(repo: &Path, path: &str, contents: &str) {
    let path = repo.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn judge_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic/judge.py")
}

fn manifest() -> String {
    let mut text = r#"schema_version = 2
[defaults]
timeout_seconds = 10
max_output_bytes = 262144
[defaults.environment]
unset = ["RUSTUP_TOOLCHAIN"]
[runner]
inputs = ["quality/manifest.toml", "quality/rubrics", "quality/semantic-judge/v2", "judge.py", "check.py", "behaviors.json"]
[[prerequisites]]
id = "fixture-prerequisite"
program = "/usr/bin/true"
args = []
install_hint = "fixture"
[[checks]]
id = "precommit-must-not-run"
phases = ["pre-commit"]
scope = "repository"
program = "/usr/bin/false"
args = []
cwd = "{candidate_root}"
[[checks]]
id = "publication-one"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/check.py"
args = []
cwd = "{candidate_root}"
[[checks]]
id = "publication-two"
phases = ["publication"]
scope = "repository"
program = "/usr/bin/true"
args = []
cwd = "{candidate_root}"
[semantic]
program = "{candidate_root}/judge.py"
args = []
cwd = "{candidate_root}"
timeout_seconds = 10
max_output_bytes = 262144
response_schema = "quality/semantic-judge/v2/response.schema.json"
[semantic.environment.set]
SEMANTIC_SCRATCH = "{scratch_root}"
"#.to_owned();
    for axis in [
        "documentation",
        "observability",
        "architecture",
        "behavioral-evidence",
    ] {
        text.push_str(&format!(
            "[[semantic.axes]]\nid = \"{axis}\"\nrubric = \"quality/rubrics/{axis}.md\"\n"
        ));
    }
    text.push_str(
        "[semantic.coherence]\nid = \"coherence\"\nrubric = \"quality/rubrics/coherence.md\"\n",
    );
    text
}

fn repository(semantic_status: &str, deterministic_pass: bool) -> (TempDir, String, String) {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "validation@test"]);
    git(repo.path(), &["config", "user.name", "Validation Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    write(repo.path(), "seed.txt", "seed\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "seed"]);
    let arbitrary_base = git(repo.path(), &["rev-parse", "HEAD"]);

    write(repo.path(), "intermediate.txt", "not selected as base\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "intermediate"]);

    write(repo.path(), "quality/manifest.toml", &manifest());
    write(
        repo.path(),
        "quality/semantic-judge/v2/response.schema.json",
        "{}\n",
    );
    for axis in [
        "documentation",
        "observability",
        "architecture",
        "behavioral-evidence",
        "coherence",
    ] {
        write(
            repo.path(),
            &format!("quality/rubrics/{axis}.md"),
            &format!("# {axis}\n"),
        );
    }
    fs::copy(judge_fixture(), repo.path().join("judge.py")).unwrap();
    fs::set_permissions(
        repo.path().join("judge.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(
        repo.path(),
        "check.py",
        &format!(
            "#!/usr/bin/env python3\nraise SystemExit({})\n",
            if deterministic_pass { 0 } else { 7 }
        ),
    );
    fs::set_permissions(
        repo.path().join("check.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(
        repo.path(),
        "behaviors.json",
        &format!("{{\"default\":{{\"status\":\"{semantic_status}\"}}}}\n"),
    );
    write(repo.path(), "candidate.txt", "candidate\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "candidate"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);
    (repo, arbitrary_base, candidate)
}

fn assert_evaluation_only(repo: &Path, digest: &str) {
    let store = Store::open(repo).unwrap();
    assert!(
        store
            .root()
            .join("reports")
            .join(format!("{digest}.json"))
            .is_file()
    );
    assert!(!store.root().join("attempts").exists());
    assert!(!store.root().join("approvals").exists());
}

#[test]
fn advisory_success_uses_arbitrary_base_complete_publication_phase_and_no_other_evidence() {
    let (repo, base, candidate) = repository("pass", true);
    let outcome = run_advisory(repo.path(), OsStr::new(&base), OsStr::new(&candidate)).unwrap();
    assert_eq!(
        outcome.evaluation.derived_disposition,
        DerivedDisposition::Pass
    );
    assert_eq!(outcome.evaluation.base_revision, base);
    assert_eq!(
        outcome.evaluation.deterministic_results.prerequisites[0].id,
        "fixture-prerequisite"
    );
    let check_ids: Vec<_> = outcome
        .evaluation
        .deterministic_results
        .checks
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(check_ids, ["publication-one", "publication-two"]);
    assert!(
        outcome
            .evaluation
            .axis_results
            .iter()
            .all(|result| result.attempts.len() == 1)
    );
    assert_evaluation_only(repo.path(), &outcome.report_digest);
}

#[test]
fn advisory_semantic_block_writes_evaluation_without_approval_or_attempt() {
    let (repo, base, candidate) = repository("block", true);
    let approval_trap = repo
        .path()
        .join(".git/loop-engine/validation/v1/approvals/lookup-trap");
    let approval_directory = approval_trap.parent().unwrap();
    fs::create_dir_all(approval_directory).unwrap();
    fs::write(&approval_trap, "advisory must not read or replace this\n").unwrap();
    fs::set_permissions(approval_directory, fs::Permissions::from_mode(0o000)).unwrap();
    let outcome = run_advisory(repo.path(), OsStr::new(&base), OsStr::new(&candidate)).unwrap();
    fs::set_permissions(approval_directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        outcome.evaluation.derived_disposition,
        DerivedDisposition::SemanticBlock
    );
    assert_eq!(outcome.evaluation.axis_results.len(), 4);
    assert!(outcome.evaluation.coherence_result.is_some());
    let store = Store::open(repo.path()).unwrap();
    assert!(
        store
            .root()
            .join("reports")
            .join(format!("{}.json", outcome.report_digest))
            .is_file()
    );
    assert!(!store.root().join("attempts").exists());
    assert_eq!(
        fs::read_to_string(approval_trap).unwrap(),
        "advisory must not read or replace this\n"
    );
}

#[test]
fn advisory_deterministic_failure_excludes_precommit_and_never_starts_semantic() {
    let (repo, base, candidate) = repository("pass", false);
    let outcome = run_advisory(repo.path(), OsStr::new(&base), OsStr::new(&candidate)).unwrap();
    assert_eq!(
        outcome.evaluation.derived_disposition,
        DerivedDisposition::DeterministicBlock
    );
    let check_ids: Vec<_> = outcome
        .evaluation
        .deterministic_results
        .checks
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(check_ids, ["publication-one", "publication-two"]);
    assert!(outcome.evaluation.axis_results.is_empty());
    assert!(outcome.evaluation.coherence_result.is_none());
    assert_evaluation_only(repo.path(), &outcome.report_digest);
}

#[test]
fn advisory_requires_candidate_to_be_current_head() {
    let (repo, base, candidate) = repository("pass", true);
    write(repo.path(), "later.txt", "later\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "later"]);
    let error = run_advisory(repo.path(), OsStr::new(&base), OsStr::new(&candidate)).unwrap_err();
    assert!(format!("{error:#}").contains("not current checkout HEAD"));
    let store = Store::open(repo.path()).unwrap();
    assert!(!store.root().exists());
}

#[test]
fn advisory_deterministic_cancellation_cleans_candidate_and_writes_no_evaluation() {
    let (repo, base, _) = repository("pass", true);
    let marker = repo.path().join("deterministic-candidate-root");
    write(
        repo.path(),
        "check.py",
        &format!(
            "#!/usr/bin/env python3\nimport os,pathlib,time\npathlib.Path({marker:?}).write_text(os.getcwd())\ntime.sleep(30)\n"
        ),
    );
    fs::set_permissions(
        repo.path().join("check.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "slow deterministic"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);
    let cancellation = Cancellation::new();
    let worker_cancellation = cancellation.clone();
    let result = thread::scope(|scope| {
        let worker = scope.spawn(|| {
            run_advisory_with_cancellation(
                repo.path(),
                OsStr::new(&base),
                OsStr::new(&candidate),
                &worker_cancellation,
            )
        });
        wait_for_path(&marker);
        cancellation.cancel();
        worker.join().unwrap()
    });
    assert!(format!("{:#}", result.unwrap_err()).contains("interrupted"));
    let source = PathBuf::from(fs::read_to_string(marker).unwrap());
    assert!(!source.parent().unwrap().exists());
    assert!(!Store::open(repo.path()).unwrap().root().exists());
}

#[test]
fn cli_advisory_signal_cancels_all_semantic_children_and_writes_no_evaluation() {
    let (repo, base, _) = repository("pass", true);
    let marker = repo.path().join("semantic-candidate-root");
    let judge_path = repo.path().join("judge.py");
    let judge = fs::read_to_string(&judge_path).unwrap().replace(
        "request = json.load(sys.stdin)",
        &format!(
            "request = json.load(sys.stdin)\npathlib.Path({marker:?}).write_text(os.getcwd())"
        ),
    );
    fs::write(&judge_path, judge).unwrap();
    fs::set_permissions(&judge_path, fs::Permissions::from_mode(0o755)).unwrap();
    write(
        repo.path(),
        "behaviors.json",
        "{\"default\":{\"status\":\"pass\",\"sleep\":30}}\n",
    );
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "slow semantic"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);
    let child = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "validate",
            "--semantic",
            "--base",
            &base,
            "--candidate",
            &candidate,
        ])
        .current_dir(repo.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_path(&marker);
    let signal = Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interrupted by signal"));
    assert!(String::from_utf8(output.stdout).unwrap().is_empty());
    let source = PathBuf::from(fs::read_to_string(marker).unwrap());
    assert!(!source.parent().unwrap().exists());
    assert!(!Store::open(repo.path()).unwrap().root().exists());
}

#[test]
fn advisory_error_path_explicitly_cleans_candidate_state() {
    let (repo, base, _) = repository("pass", true);
    let marker = repo.path().join("error-candidate-root");
    write(
        repo.path(),
        "check.py",
        &format!(
            "#!/usr/bin/env python3\nimport os,pathlib\npathlib.Path({marker:?}).write_text(os.getcwd())\n"
        ),
    );
    fs::set_permissions(
        repo.path().join("check.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let manifest = fs::read_to_string(repo.path().join("quality/manifest.toml"))
        .unwrap()
        .replace(
            "quality/semantic-judge/v2/response.schema.json",
            "quality/semantic-judge/v2/missing.schema.json",
        );
    write(repo.path(), "quality/manifest.toml", &manifest);
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "semantic setup error"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);

    let error = run_advisory(repo.path(), OsStr::new(&base), OsStr::new(&candidate)).unwrap_err();
    assert!(format!("{error:#}").contains("response schema"));
    let source = PathBuf::from(fs::read_to_string(marker).unwrap());
    assert!(!source.parent().unwrap().exists());
    assert!(!Store::open(repo.path()).unwrap().root().exists());
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cli_advisory_and_approval_dispatch_write_expected_evidence() {
    let (repo, base, candidate) = repository("block", true);
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "validate",
            "--semantic",
            "--base",
            &base,
            "--candidate",
            &candidate,
        ])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report_digest = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    assert_eq!(report_digest.len(), 64);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("advisory semantic validation blocked")
    );

    let approval = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "validation",
            "approve",
            "--report",
            &report_digest,
            "--reason",
            "owner CLI reason",
        ])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        approval.status.success(),
        "{}",
        String::from_utf8_lossy(&approval.stderr)
    );
    let approval_digest = String::from_utf8(approval.stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(approval_digest.len(), 64);
    assert_eq!(
        Store::open(repo.path())
            .unwrap()
            .read_approval(&report_digest, &approval_digest)
            .unwrap()
            .reason,
        "owner CLI reason"
    );
    assert!(
        !Store::open(repo.path())
            .unwrap()
            .root()
            .join("attempts")
            .exists()
    );
}
