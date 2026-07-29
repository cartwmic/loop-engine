use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;
use xtask::candidate::{Candidate, PreparedCandidate};
use xtask::config::{SemanticRequirement, parse_manifest};
use xtask::quality::{CandidateBinding, DeterministicPhase, DeterministicResult};
use xtask::semantic_judge::{SemanticDisposition, SemanticStatus, run};

static TEST_MUTEX: Mutex<()> = Mutex::new(());

const AXES: [&str; 4] = [
    "documentation",
    "observability",
    "architecture",
    "behavioral-evidence",
];

fn serial() -> MutexGuard<'static, ()> {
    TEST_MUTEX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic/judge.py")
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
        .expect("git UTF-8")
        .trim_end()
        .to_owned()
}

fn write(repo: &Path, path: &str, text: &str) {
    let path = repo.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn manifest(timeout: u64, axes: &[&str]) -> String {
    let mut text = format!(
        r#"schema_version = 2

[defaults]
timeout_seconds = {timeout}
max_output_bytes = 262144

[defaults.environment]
unset = ["RUSTUP_TOOLCHAIN", "REMOVE_ME"]

[defaults.environment.set]
TYPED_BINDING = "{{candidate_tree}}"
REMOVE_ME = "removed"

[runner]
inputs = ["quality/manifest.toml", "quality/semantic-judge/v2", "quality/rubrics", "judge.py", "behaviors.json"]

[[checks]]
id = "fixture-check"
phases = ["publication"]
scope = "repository"
program = "/usr/bin/true"
args = []
cwd = "{{candidate_root}}"

[semantic]
program = "{{candidate_root}}/judge.py"
args = []
cwd = "{{candidate_root}}"
timeout_seconds = {timeout}
max_output_bytes = 262144
response_schema = "quality/semantic-judge/v2/response.schema.json"

[semantic.environment]
unset = ["REMOVE_ME"]

[semantic.environment.set]
SEMANTIC_SCRATCH = "{{scratch_root}}"
"#
    );
    for axis in axes {
        text.push_str(&format!(
            "\n[[semantic.axes]]\nid = \"{axis}\"\nrubric = \"quality/rubrics/{axis}.md\"\n"
        ));
    }
    text.push_str(
        "\n[semantic.coherence]\nid = \"coherence\"\nrubric = \"quality/rubrics/coherence.md\"\n",
    );
    text
}

fn prepared(behaviors: Value, timeout: u64) -> (TempDir, PreparedCandidate) {
    prepared_with_axes(behaviors, timeout, &AXES)
}

fn prepared_with_axes(
    behaviors: Value,
    timeout: u64,
    axes: &[&str],
) -> (TempDir, PreparedCandidate) {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "semantic@test"]);
    git(repo.path(), &["config", "user.name", "Semantic Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    fs::copy(fixture(), repo.path().join("judge.py")).unwrap();
    fs::set_permissions(
        repo.path().join("judge.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(
        repo.path(),
        "quality/manifest.toml",
        &manifest(timeout, axes),
    );
    write(
        repo.path(),
        "quality/semantic-judge/v2/response.schema.json",
        "{}\n",
    );
    for axis in axes.iter().copied().chain(["coherence"]) {
        write(
            repo.path(),
            &format!("quality/rubrics/{axis}.md"),
            &format!("# {axis}\n\nFixture rubric {axis}.\n"),
        );
    }
    write(repo.path(), "behaviors.json", "{}\n");
    write(repo.path(), "protected.txt", "original\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "base"]);
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

    write(
        repo.path(),
        "behaviors.json",
        &format!("{}\n", serde_json::to_string(&behaviors).unwrap()),
    );
    write(repo.path(), "ordinary.txt", "candidate content\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "candidate"]);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let candidate = Candidate::revision(repo.path(), Some(OsStr::new(&base)), OsStr::new(&head))
        .unwrap()
        .prepare(SemanticRequirement::Required)
        .unwrap();
    (repo, candidate)
}

fn deterministic(candidate: &PreparedCandidate) -> DeterministicResult {
    DeterministicResult {
        phase: DeterministicPhase::Publication,
        binding: CandidateBinding {
            base_revision: candidate.base_revision().to_owned(),
            candidate_revision: candidate.candidate_revision().to_owned(),
            candidate_tree: candidate.candidate_tree().to_owned(),
        },
        prerequisites: Vec::new(),
        checks: Vec::new(),
        final_source_verified: true,
        final_failure: None,
    }
}

fn request(result: &xtask::semantic_judge::NormalizedResult) -> Value {
    let path = result.attempts[0].scratch_root.join(format!(
        "{}-request.json",
        if result.id == "coherence" {
            "coherence"
        } else {
            "axis"
        }
    ));
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn tracked_v2_schemas_freeze_request_kinds_and_normalized_statuses() {
    let _serial = serial();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let request: Value = serde_json::from_slice(
        &fs::read(root.join("quality/semantic-judge/v2/request.schema.json")).unwrap(),
    )
    .unwrap();
    let response: Value = serde_json::from_slice(
        &fs::read(root.join("quality/semantic-judge/v2/response.schema.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        request["properties"]["request_kind"]["enum"],
        json!(["axis", "correction", "coherence"])
    );
    assert_eq!(
        response["properties"]["status"]["enum"],
        json!(["pass", "block", "indeterminate", "unavailable"])
    );
    for schema in [&request, &response] {
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
    }
    let judge = fs::metadata(root.join("quality/semantic-judge/v2/judge")).unwrap();
    assert_ne!(judge.permissions().mode() & 0o111, 0);
}

#[test]
fn focused_axes_fan_out_with_isolated_rubrics_scratch_and_typed_candidate_context() {
    let _serial = serial();
    let behavior = json!({"default": {
        "sleep": 0.4,
        "require_typed_env": true,
        "require_candidate_cwd": true
    }});
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert_eq!(result.disposition, SemanticDisposition::Pass);
    assert_eq!(
        result
            .axes
            .iter()
            .map(|axis| axis.id.as_str())
            .collect::<Vec<_>>(),
        AXES
    );
    assert_eq!(result.coherence.id, "coherence");

    let roots = result
        .axes
        .iter()
        .chain([&result.coherence])
        .map(|record| record.attempts[0].scratch_root.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(roots.len(), 5);

    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for axis in &result.axes {
        let payload = request(axis);
        assert_eq!(payload["request_kind"], "axis");
        assert_eq!(payload["rubric"]["id"], axis.id);
        assert_eq!(payload["axis_results"], json!([]));
        starts.push(
            fs::read_to_string(axis.attempts[0].scratch_root.join("axis-start"))
                .unwrap()
                .parse::<f64>()
                .unwrap(),
        );
        ends.push(
            fs::read_to_string(axis.attempts[0].scratch_root.join("axis-end"))
                .unwrap()
                .parse::<f64>()
                .unwrap(),
        );
    }
    assert!(
        starts.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            < ends.iter().copied().fold(f64::INFINITY, f64::min)
    );
    let coherence_request = request(&result.coherence);
    assert_eq!(coherence_request["request_kind"], "coherence");
    assert_eq!(
        coherence_request["axis_results"].as_array().unwrap().len(),
        4
    );
}

#[test]
fn status_matrix_and_coherence_are_monotonic() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"status": "block"},
        "observability": {"status": "indeterminate"},
        "architecture": {"status": "unavailable"},
        "behavioral-evidence": {"status": "pass"},
        "coherence": {"status": "pass"}
    });
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert_eq!(
        result
            .axes
            .iter()
            .map(|axis| axis.status)
            .collect::<Vec<_>>(),
        [
            SemanticStatus::Block,
            SemanticStatus::Indeterminate,
            SemanticStatus::Unavailable,
            SemanticStatus::Pass
        ]
    );
    assert_eq!(result.coherence.status, SemanticStatus::Pass);
    assert_eq!(result.disposition, SemanticDisposition::SemanticBlock);
}

#[test]
fn malformed_output_gets_exactly_one_bounded_correction() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"invalid": "first"},
        "observability": {"invalid": "always"}
    });
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert_eq!(result.axes[0].status, SemanticStatus::Pass);
    assert_eq!(result.axes[0].attempts.len(), 2);
    assert_eq!(
        result.axes[0].attempts[1].request_kind,
        xtask::semantic_judge::RequestKind::Correction
    );
    assert_eq!(result.axes[1].status, SemanticStatus::Unavailable);
    assert_eq!(result.axes[1].attempts.len(), 2);
}

#[test]
fn malformed_contract_fields_fail_closed_after_one_correction() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"response_changes": {"extra": true}},
        "observability": {"response_changes": {"message": "__DELETE__"}},
        "architecture": {"response_changes": {"status": "maybe"}},
        "behavioral-evidence": {"response_changes": {"request_kind": "coherence"}}
    });
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert!(
        result
            .axes
            .iter()
            .all(|axis| axis.status == SemanticStatus::Unavailable)
    );
    assert!(result.axes.iter().all(|axis| axis.attempts.len() == 2));

    let behavior = json!({
        "documentation": {"duplicate_status": true},
        "observability": {"response_changes": {"candidate_revision": "wrong"}},
        "architecture": {"response_changes": {"citations": [{"kind":"candidate","reference":"not-supplied","detail":"bad"}]}},
        "behavioral-evidence": {"response_changes": {"axis_id": "documentation"}}
    });
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert!(
        result
            .axes
            .iter()
            .all(|axis| axis.status == SemanticStatus::Unavailable)
    );
}

#[test]
fn timeout_and_adapter_failure_normalize_without_correction() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"sleep": 3},
        "observability": {"exit": 7}
    });
    let (_repo, candidate) = prepared(behavior, 1);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert_eq!(result.axes[0].status, SemanticStatus::Unavailable);
    assert_eq!(result.axes[0].attempts.len(), 1);
    assert_eq!(result.axes[1].status, SemanticStatus::Unavailable);
    assert_eq!(result.axes[1].attempts.len(), 1);
}

#[test]
fn axis_mutation_cancels_siblings_and_suppresses_coherence() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"sleep": 0.1, "mutate": "axis"},
        "observability": {"sleep": 5},
        "architecture": {"sleep": 5},
        "behavioral-evidence": {"sleep": 5}
    });
    let (_repo, candidate) = prepared(behavior, 10);
    let started = Instant::now();
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:#?}",
        started.elapsed()
    );
    assert_eq!(result.axes.len(), 4);
    assert_eq!(
        result
            .axes
            .iter()
            .map(|axis| axis.id.as_str())
            .collect::<Vec<_>>(),
        AXES
    );
    assert!(
        result
            .axes
            .iter()
            .all(|axis| axis.status == SemanticStatus::Unavailable)
    );
    assert!(result.coherence.attempts.is_empty());
    assert_eq!(result.coherence.status, SemanticStatus::Unavailable);
    assert_eq!(result.disposition, SemanticDisposition::SemanticBlock);
}

#[test]
fn correction_mutation_cancels_siblings_and_suppresses_later_children() {
    let _serial = serial();
    let behavior = json!({
        "documentation": {"invalid": "first", "mutate": "correction"},
        "observability": {"sleep": 5},
        "architecture": {"sleep": 5},
        "behavioral-evidence": {"sleep": 5}
    });
    let (_repo, candidate) = prepared(behavior, 10);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert!(result.source_mutation.is_some());
    assert_eq!(result.axes[0].attempts.len(), 2);
    assert!(
        result
            .axes
            .iter()
            .all(|axis| axis.status == SemanticStatus::Unavailable)
    );
    assert!(result.coherence.attempts.is_empty());
}

#[test]
fn coherence_mutation_forces_semantic_block() {
    let _serial = serial();
    let behavior = json!({"coherence": {"mutate": "coherence"}});
    let (_repo, candidate) = prepared(behavior, 5);
    let result = run(&candidate, &deterministic(&candidate)).unwrap();
    assert!(
        result
            .axes
            .iter()
            .all(|axis| axis.status == SemanticStatus::Pass)
    );
    assert_eq!(result.coherence.status, SemanticStatus::Unavailable);
    assert_eq!(result.coherence.source_verified, Some(false));
    assert_eq!(result.disposition, SemanticDisposition::SemanticBlock);
}

#[test]
fn missing_or_duplicate_focused_results_fail_closed_before_semantic_spawn() {
    let _serial = serial();
    let (_repo, candidate) = prepared_with_axes(json!({}), 5, &AXES[..3]);
    let error = run(&candidate, &deterministic(&candidate)).unwrap_err();
    assert!(error.to_string().contains("exactly 4 focused axes"));

    let duplicate = manifest(
        5,
        &[
            "documentation",
            "observability",
            "architecture",
            "documentation",
        ],
    );
    let error = parse_manifest(duplicate.as_bytes(), SemanticRequirement::Required).unwrap_err();
    assert!(error.to_string().contains("duplicate semantic axis"));
}

#[test]
fn passing_precommit_evidence_fails_before_semantic_setup_or_child_spawn() {
    let _serial = serial();
    let (_repo, candidate) = prepared(json!({}), 5);
    let mut evidence = deterministic(&candidate);
    evidence.phase = DeterministicPhase::PreCommit;

    let error = run(&candidate, &evidence).unwrap_err();

    assert!(error.to_string().contains("publication-phase"));
    assert!(!candidate.scratch_root().join("semantic").exists());
}

#[test]
fn tracked_adapter_shortens_provider_tmpdir_without_leaving_alias() {
    let root = TempDir::new().unwrap();
    let scratch = root
        .path()
        .join("assigned-semantic-scratch-with-a-deliberately-long-component");
    fs::create_dir(&scratch).unwrap();
    let record = root.path().join("provider-tmpdir.txt");
    let provider = root.path().join("fake-pi.py");
    fs::write(
        &provider,
        r#"#!/usr/bin/env python3
import json
import os
import pathlib
import sys

request = json.loads(sys.stdin.read().split("\n\n", 1)[1])
alias = pathlib.Path(os.environ["TMPDIR"])
pathlib.Path(os.environ["TMP_RECORD"]).write_text(
    f"{alias}\n{alias.resolve()}\n", encoding="utf-8"
)
print(json.dumps({
    "schema_version": 2,
    "request_kind": request["request_kind"],
    "axis_id": request["axis_id"],
    "base_revision": request["base_revision"],
    "candidate_revision": request["candidate_revision"],
    "candidate_tree": request["candidate_tree"],
    "status": "pass",
    "citations": [{
        "kind": "rubric",
        "reference": request["rubric"]["id"],
        "detail": "adapter test",
    }],
    "message": "adapter test",
}, separators=(",", ":")))
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&provider).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&provider, permissions).unwrap();

    let request = json!({
        "schema_version": 2,
        "request_kind": "axis",
        "axis_id": "documentation",
        "base_revision": "base",
        "candidate_revision": "candidate",
        "candidate_tree": "tree",
        "rubric": {"id": "documentation", "content": "rubric"},
        "diff": {"encoding": "utf-8", "data": ""},
        "resulting_files": [],
        "deterministic_evidence": {},
        "axis_results": [],
        "correction": null,
    });
    let adapter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("quality/semantic-judge/v2/adapter.py");
    let output = Command::new("python3")
        .arg(adapter)
        .env("TMPDIR", &scratch)
        .env("TMP_RECORD", &record)
        .env("LOOP_ENGINE_SEMANTIC_JUDGE_PI", &provider)
        .env("PI_CODING_AGENT_DIR", root.path().join("empty-agent"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(serde_json::to_string(&request).unwrap().as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"],
        "pass"
    );

    let recorded = fs::read_to_string(record).unwrap();
    let mut lines = recorded.lines();
    let alias = PathBuf::from(lines.next().unwrap());
    let resolved = PathBuf::from(lines.next().unwrap());
    assert_eq!(alias.parent(), Some(Path::new("/tmp")));
    assert!(
        alias
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("le-sem-")
    );
    assert_eq!(resolved, scratch.canonicalize().unwrap());
    assert!(!alias.exists(), "temporary alias was not removed");
}

#[test]
fn tracked_adapter_removes_tmpdir_alias_when_process_group_is_terminated() {
    use std::io::Write;
    use std::os::unix::process::CommandExt;

    let root = TempDir::new().unwrap();
    let scratch = root.path().join("assigned-semantic-scratch");
    fs::create_dir(&scratch).unwrap();
    let record = root.path().join("provider-tmpdir.txt");
    let provider = root.path().join("blocking-pi.py");
    fs::write(
        &provider,
        r#"#!/usr/bin/env python3
import os
import pathlib
import signal

alias = pathlib.Path(os.environ["TMPDIR"])
pathlib.Path(os.environ["TMP_RECORD"]).write_text(str(alias), encoding="utf-8")
while True:
    signal.pause()
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&provider).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&provider, permissions).unwrap();

    let adapter = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("quality/semantic-judge/v2/adapter.py");
    let request = json!({
        "schema_version": 2,
        "request_kind": "axis",
        "axis_id": "documentation",
        "base_revision": "base",
        "candidate_revision": "candidate",
        "candidate_tree": "tree",
        "rubric": {"id": "documentation", "content": "rubric"},
        "diff": {"encoding": "utf-8", "data": ""},
        "resulting_files": [],
        "deterministic_evidence": {},
        "axis_results": [],
        "correction": null,
    });
    let mut command = Command::new("python3");
    command
        .arg(adapter)
        .env("TMPDIR", &scratch)
        .env("TMP_RECORD", &record)
        .env("LOOP_ENGINE_SEMANTIC_JUDGE_PI", &provider)
        .env("PI_CODING_AGENT_DIR", root.path().join("empty-agent"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&request).unwrap().as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !record.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let alias = PathBuf::from(fs::read_to_string(&record).expect("provider started"));
    assert!(alias.exists());

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(-(child.id() as i32)),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        !alias.exists(),
        "temporary alias survived process-group termination: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn binding_mismatch_and_nonpassing_deterministic_evidence_fail_before_spawn() {
    let _serial = serial();
    let (_repo, candidate) = prepared(json!({}), 5);
    let mut evidence = deterministic(&candidate);
    evidence.binding.candidate_revision = "wrong".to_owned();
    assert!(
        run(&candidate, &evidence)
            .unwrap_err()
            .to_string()
            .contains("binding")
    );
    let mut evidence = deterministic(&candidate);
    evidence.final_source_verified = false;
    assert!(
        run(&candidate, &evidence)
            .unwrap_err()
            .to_string()
            .contains("passing")
    );
}
