use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;
use xtask::process::Cancellation;
use xtask::publication::{
    ParsedUpdateDisposition, parse_ci_event, parse_updates, run_ci_publication, run_publication,
};
use xtask::report::{
    DerivedDisposition, GateDecision, InputKind, RejectionCode, Store, UpdateKind, sha256_hex,
};

const ZERO: &str = "0000000000000000000000000000000000000000";

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

fn line(local_ref: &str, local: &str, remote_ref: &str, remote: &str) -> String {
    format!("{local_ref} {local} {remote_ref} {remote}\n")
}

fn ci_event(before: &str, after: &str, reference: &str) -> Vec<u8> {
    format!(r#"{{"before":"{before}","after":"{after}","ref":"{reference}"}}"#).into_bytes()
}

fn manifest() -> String {
    let mut text = r#"schema_version = 2
[defaults]
timeout_seconds = 10
max_output_bytes = 262144
[runner]
inputs = ["quality/manifest.toml", "quality/rubrics", "quality/semantic-judge/v2", "judge.py", "check.py", "behaviors.json", "protected.txt"]
[[checks]]
id = "publication-check"
phases = ["publication"]
scope = "repository"
program = "{candidate_root}/check.py"
args = ["{git_directory}", "{candidate_root}/protected.txt"]
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
SEMANTIC_TRAP = "{git_directory}/semantic-trap"
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

fn repository(status: &str) -> (TempDir, String, String) {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "publication@test"]);
    git(repo.path(), &["config", "user.name", "Publication Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    write(repo.path(), "seed.txt", "seed\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "seed"]);
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

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
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic/judge.py");
    let judge = fs::read_to_string(fixture).unwrap().replace(
        "request = json.load(sys.stdin)",
        "request = json.load(sys.stdin)\ntrap = pathlib.Path(os.environ['SEMANTIC_TRAP'])\ncount = trap.parent / 'semantic-count'\nwith count.open('a') as stream: stream.write('1\\n')\nif trap.exists(): raise SystemExit(29)\nif (trap.parent / 'semantic-mutate').exists() and request['request_kind'] == 'axis':\n    target = pathlib.Path('protected.txt'); target.chmod(0o644); target.write_text('mutated\\n')",
    );
    write(repo.path(), "judge.py", &judge);
    fs::set_permissions(
        repo.path().join("judge.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(
        repo.path(),
        "check.py",
        r#"#!/usr/bin/env python3
import pathlib, sys
git_dir = pathlib.Path(sys.argv[1])
count = git_dir / "deterministic-count"
count.write_text(str(int(count.read_text()) + 1) if count.exists() else "1")
if (git_dir / "deterministic-block").exists(): raise SystemExit(17)
if (git_dir / "deterministic-mutate").exists():
    target = pathlib.Path(sys.argv[2]); target.chmod(0o644); target.write_text("mutated\n")
rubric = pathlib.Path("quality/rubrics/documentation.md")
if (git_dir / "deterministic-rubric-modify").exists():
    rubric.chmod(0o644); rubric.write_text("mutated rubric\n")
if (git_dir / "deterministic-rubric-delete").exists():
    rubric.parent.chmod(0o755); rubric.unlink()
"#,
    );
    fs::set_permissions(
        repo.path().join("check.py"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(
        repo.path(),
        "behaviors.json",
        &format!("{{\"default\":{{\"status\":\"{status}\"}}}}\n"),
    );
    write(repo.path(), "protected.txt", "protected\n");
    write(repo.path(), "candidate.txt", "candidate\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "candidate"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);
    (repo, base, candidate)
}

#[test]
fn complete_parser_matrix_preserves_exact_evidence_and_frozen_rejections() {
    let malformed = parse_updates(b"not four fields\n");
    assert!(matches!(
        malformed.disposition,
        ParsedUpdateDisposition::Rejected(RejectionCode::MalformedUpdateInput)
    ));
    assert!(malformed.updates.is_empty());
    assert_eq!(malformed.input_evidence.data, "not four fields\n");

    let binary = parse_updates(&[0xff]);
    assert_eq!(binary.input_evidence.encoding, "base64");
    assert_eq!(binary.input_evidence.data, "/w==");
    assert!(matches!(
        binary.disposition,
        ParsedUpdateDisposition::Rejected(RejectionCode::MalformedUpdateInput)
    ));

    let invalid = parse_updates(line("bad", &"1".repeat(40), "refs/heads/main", ZERO).as_bytes());
    assert!(matches!(
        invalid.disposition,
        ParsedUpdateDisposition::Rejected(RejectionCode::InvalidUpdateShape)
    ));
    assert_eq!(invalid.updates.len(), 1);

    assert!(matches!(
        parse_updates(b"").disposition,
        ParsedUpdateDisposition::DeletionOnly
    ));
    let deletion = line("(delete)", ZERO, "refs/heads/old", &"2".repeat(40));
    assert!(matches!(
        parse_updates(deletion.as_bytes()).disposition,
        ParsedUpdateDisposition::DeletionOnly
    ));

    let first = line(
        "refs/heads/main",
        &"3".repeat(40),
        "refs/heads/main",
        &"1".repeat(40),
    );
    let second = line(
        "refs/heads/other",
        &"4".repeat(40),
        "refs/heads/other",
        &"2".repeat(40),
    );
    let reversed = parse_updates(format!("{second}{first}").as_bytes());
    assert!(matches!(
        reversed.disposition,
        ParsedUpdateDisposition::Rejected(RejectionCode::MultipleContentTips)
    ));
    assert_eq!(reversed.updates[0].local_ref, "refs/heads/main");
    assert_eq!(reversed.updates[1].local_ref, "refs/heads/other");

    let mixed = format!("{deletion}{first}");
    assert!(matches!(
        parse_updates(mixed.as_bytes()).disposition,
        ParsedUpdateDisposition::Content(_)
    ));
    let duplicate = format!("{first}{first}");
    let parsed = parse_updates(duplicate.as_bytes());
    assert!(matches!(
        parsed.disposition,
        ParsedUpdateDisposition::Rejected(RejectionCode::MultipleContentTips)
    ));
    assert_eq!(parsed.updates.len(), 2);
    assert_eq!(parsed.updates[0], parsed.updates[1]);
}

#[test]
fn ci_parser_requires_closed_canonical_object_and_preserves_exact_malformed_bytes() {
    let before = "1".repeat(40);
    let after = "2".repeat(40);
    let canonical = ci_event(&before, &after, "refs/heads/main");
    let parsed = parse_ci_event(&canonical);
    let ParsedUpdateDisposition::Content(update) = parsed.disposition else {
        panic!("canonical event must classify as content");
    };
    assert_eq!(parsed.input_evidence.data.as_bytes(), canonical);
    assert_eq!(parsed.updates, vec![update.clone()]);
    assert_eq!(update.local_ref, "refs/heads/main");
    assert_eq!(update.remote_ref, "refs/heads/main");
    assert_eq!(update.local_sha, after);
    assert_eq!(update.remote_sha, before);

    let deletion = parse_ci_event(&ci_event(&"3".repeat(40), ZERO, "refs/heads/removed"));
    assert!(matches!(
        deletion.disposition,
        ParsedUpdateDisposition::DeletionOnly
    ));
    assert_eq!(deletion.updates[0].local_ref, "refs/heads/removed");

    let malformed = vec![
        b"not json".to_vec(),
        vec![0xff],
        format!(
            r#"{{"before":"{before}","after":"{after}","ref":"refs/heads/main","extra":1}}"#
        )
        .into_bytes(),
        format!(r#"{{"before":"{before}","after":"{after}"}}"#).into_bytes(),
        format!(
            r#"{{"before":"{before}","before":"{before}","after":"{after}","ref":"refs/heads/main"}}"#
        )
        .into_bytes(),
        format!(
            r#"{{ "before":"{before}","after":"{after}","ref":"refs/heads/main"}}"#
        )
        .into_bytes(),
        format!(
            r#"{{"after":"{after}","before":"{before}","ref":"refs/heads/main"}}"#
        )
        .into_bytes(),
        ci_event("ABC", &after, "refs/heads/main"),
        ci_event(&before, &after, "main"),
        ci_event(ZERO, ZERO, "refs/heads/main"),
    ];
    for bytes in malformed {
        let parsed = parse_ci_event(&bytes);
        assert!(
            matches!(
                parsed.disposition,
                ParsedUpdateDisposition::Rejected(RejectionCode::MalformedCiEvent)
            ),
            "accepted malformed event: {}",
            String::from_utf8_lossy(&bytes)
        );
        assert!(parsed.updates.is_empty());
        assert_eq!(
            xtask::publication_input::decode_input_evidence(&parsed.input_evidence).unwrap(),
            bytes
        );
    }
}

#[test]
fn rejected_and_deletion_attempts_have_exact_nullability_and_only_common_dir_git_query() {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    let trace = repo.path().join("trace.json");

    let rejected = run_publication(repo.path(), b"bad\n", &Cancellation::new()).unwrap();
    assert_eq!(rejected.attempt.update_kind, UpdateKind::Rejected);
    assert_eq!(
        rejected.attempt.rejection_code,
        Some(RejectionCode::MalformedUpdateInput)
    );
    assert_eq!(
        rejected.attempt.derived_disposition,
        DerivedDisposition::DeterministicBlock
    );
    assert_eq!(rejected.attempt.gate_decision, GateDecision::Block);
    assert!(rejected.attempt.base_revision.is_none());
    assert!(rejected.attempt.fresh_deterministic_results.is_empty());

    let ci_rejected = run_ci_publication(repo.path(), &[0xff], &Cancellation::new()).unwrap();
    assert_eq!(ci_rejected.attempt.input_kind, InputKind::CiPushEvent);
    assert_eq!(ci_rejected.attempt.update_kind, UpdateKind::Rejected);
    assert_eq!(
        ci_rejected.attempt.rejection_code,
        Some(RejectionCode::MalformedCiEvent)
    );
    assert_eq!(ci_rejected.attempt.input_evidence.encoding, "base64");
    assert_eq!(ci_rejected.attempt.input_evidence.data, "/w==");

    let deletion = run_publication(repo.path(), b"", &Cancellation::new()).unwrap();
    assert_eq!(deletion.attempt.update_kind, UpdateKind::DeletionOnly);
    assert_eq!(
        deletion.attempt.derived_disposition,
        DerivedDisposition::Pass
    );
    assert_eq!(deletion.attempt.gate_decision, GateDecision::Pass);
    assert!(deletion.attempt.evaluation_report_digest.is_none());

    for input in [b"bad\n".as_slice(), b"".as_slice()] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["validate", "--publication", "--updates-stdin"])
            .current_dir(repo.path())
            .env("GIT_TRACE2_EVENT", &trace)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(input).unwrap();
        let _ = child.wait().unwrap();
    }
    for (name, bytes, succeeds) in [
        ("malformed.json", b"bad".to_vec(), false),
        (
            "deletion.json",
            ci_event(&"1".repeat(40), ZERO, "refs/heads/old"),
            true,
        ),
    ] {
        let path = repo.path().join(name);
        fs::write(&path, bytes).unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["validate", "--publication", "--ci-event"])
            .arg(&path)
            .current_dir(repo.path())
            .env("GIT_TRACE2_EVENT", &trace)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.success(), succeeds);
    }
    let trace = fs::read_to_string(trace).unwrap();
    assert_eq!(trace.matches("\"event\":\"start\"").count(), 4, "{trace}");
    assert_eq!(trace.matches("--git-common-dir").count(), 4, "{trace}");
    for forbidden in [
        "--show-toplevel",
        "--absolute-git-dir",
        "cat-file",
        "mktree",
    ] {
        assert!(!trace.contains(forbidden), "{forbidden}: {trace}");
    }
}

#[test]
fn ordinary_new_force_mixed_and_approved_retry_keep_one_aggregate_verdict() {
    let (repo, base, candidate) = repository("block");
    let ordinary_input = line("refs/heads/main", &candidate, "refs/heads/main", &base);
    let blocked =
        run_publication(repo.path(), ordinary_input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(blocked.attempt.update_kind, UpdateKind::Content);
    assert_eq!(blocked.attempt.gate_decision, GateDecision::Block);
    assert_eq!(
        blocked.attempt.derived_disposition,
        DerivedDisposition::SemanticBlock
    );
    let report = blocked.attempt.evaluation_report_digest.clone().unwrap();
    Store::open(repo.path())
        .unwrap()
        .approve(&report, "accept exact semantic evidence")
        .unwrap();
    fs::write(repo.path().join(".git/semantic-trap"), []).unwrap();

    let approved =
        run_publication(repo.path(), ordinary_input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(approved.attempt.gate_decision, GateDecision::Approved);
    assert_eq!(
        approved.attempt.derived_disposition,
        DerivedDisposition::SemanticBlock
    );
    assert!(approved.attempt.approval_digest.is_some());
    assert_eq!(
        approved.attempt.evaluation_report_digest.as_deref(),
        Some(report.as_str())
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".git/deterministic-count")).unwrap(),
        "2"
    );

    fs::remove_file(repo.path().join(".git/semantic-trap")).unwrap();
    let new_input = line("refs/heads/new", &candidate, "refs/heads/new", ZERO);
    let new_branch =
        run_publication(repo.path(), new_input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(new_branch.attempt.update_kind, UpdateKind::Content);
    assert_ne!(new_branch.attempt.base_revision.as_deref(), Some(ZERO));

    write(repo.path(), "later.txt", "later\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "later"]);
    let later = git(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["reset", "--hard", &candidate]);
    let force_input = line("refs/heads/main", &candidate, "refs/heads/main", &later);
    let force = run_publication(repo.path(), force_input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(
        force.attempt.candidate_revision.as_deref(),
        Some(candidate.as_str())
    );

    let deletion = line("(delete)", ZERO, "refs/heads/old", &base);
    let mixed = run_publication(
        repo.path(),
        format!("{deletion}{ordinary_input}").as_bytes(),
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(mixed.attempt.update_kind, UpdateKind::Content);
    assert_eq!(mixed.attempt.updates.len(), 2);
}

#[test]
fn ci_event_binds_pushed_candidate_and_ignores_local_approval() {
    let (repo, base, candidate) = repository("block");
    let local = line("refs/heads/main", &candidate, "refs/heads/main", &base);
    let blocked = run_publication(repo.path(), local.as_bytes(), &Cancellation::new()).unwrap();
    let report = blocked.attempt.evaluation_report_digest.unwrap();
    Store::open(repo.path())
        .unwrap()
        .approve(&report, "local publication only")
        .unwrap();

    let exact_event = ci_event(&base, &candidate, "refs/heads/main");
    let ci = run_ci_publication(repo.path(), &exact_event, &Cancellation::new()).unwrap();
    assert_eq!(ci.attempt.input_kind, InputKind::CiPushEvent);
    assert_eq!(ci.attempt.input_evidence.data.as_bytes(), exact_event);
    assert_eq!(ci.attempt.gate_decision, GateDecision::Block);
    assert!(ci.attempt.approval_digest.is_none());
    assert_ne!(
        ci.attempt.evaluation_report_digest.as_deref(),
        Some(report.as_str())
    );
    assert_eq!(
        ci.attempt.candidate_revision.as_deref(),
        Some(candidate.as_str())
    );
    assert_eq!(ci.attempt.base_revision.as_deref(), Some(base.as_str()));
    assert_eq!(
        ci.attempt.candidate_tree.as_deref(),
        Some(
            git(
                repo.path(),
                &["rev-parse", &format!("{candidate}^{{tree}}")]
            )
            .as_str()
        )
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".git/deterministic-count")).unwrap(),
        "2"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".git/semantic-count"))
            .unwrap()
            .lines()
            .count(),
        10
    );

    let new_branch = run_ci_publication(
        repo.path(),
        &ci_event(ZERO, &candidate, "refs/heads/new"),
        &Cancellation::new(),
    )
    .unwrap();
    assert_ne!(new_branch.attempt.base_revision.as_deref(), Some(ZERO));

    git(
        repo.path(),
        &["checkout", "--orphan", "unrelated-force-base"],
    );
    git(repo.path(), &["rm", "-rf", "."]);
    write(repo.path(), "force-base.txt", "unrelated force base\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "unrelated force base"]);
    let force_base = git(repo.path(), &["rev-parse", "HEAD"]);
    assert!(
        !Command::new("/usr/bin/git")
            .args(["merge-base", "--is-ancestor", &force_base, &candidate])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success()
    );
    git(repo.path(), &["checkout", "--detach", &candidate]);
    let force = run_ci_publication(
        repo.path(),
        &ci_event(&force_base, &candidate, "refs/heads/main"),
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(
        force.attempt.candidate_revision.as_deref(),
        Some(candidate.as_str())
    );
    assert_eq!(
        force.attempt.base_revision.as_deref(),
        Some(force_base.as_str())
    );
}

#[test]
fn approval_never_bypasses_fresh_deterministic_failure_or_changed_base() {
    let (repo, base, candidate) = repository("block");
    let input = line("refs/heads/main", &candidate, "refs/heads/main", &base);
    let first = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
    let report = first.attempt.evaluation_report_digest.unwrap();
    Store::open(repo.path())
        .unwrap()
        .approve(&report, "exact binding")
        .unwrap();

    fs::write(repo.path().join(".git/deterministic-block"), []).unwrap();
    let deterministic =
        run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(deterministic.attempt.gate_decision, GateDecision::Block);
    assert_eq!(
        deterministic.attempt.derived_disposition,
        DerivedDisposition::DeterministicBlock
    );
    assert!(deterministic.attempt.approval_digest.is_none());
    fs::remove_file(repo.path().join(".git/deterministic-block")).unwrap();

    write(repo.path(), "later.txt", "later\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "later"]);
    let changed_base = git(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["reset", "--hard", &candidate]);
    let retry = line(
        "refs/heads/main",
        &candidate,
        "refs/heads/main",
        &changed_base,
    );
    let invalidated = run_publication(repo.path(), retry.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(invalidated.attempt.gate_decision, GateDecision::Block);
    assert!(invalidated.attempt.approval_digest.is_none());
}

#[test]
fn rubric_mutation_or_deletion_stores_one_pre_run_bound_block_without_semantic() {
    for marker in ["deterministic-rubric-modify", "deterministic-rubric-delete"] {
        let (repo, base, candidate) = repository("pass");
        fs::write(repo.path().join(format!(".git/{marker}")), []).unwrap();
        fs::write(repo.path().join(".git/semantic-trap"), []).unwrap();
        let input = line("refs/heads/main", &candidate, "refs/heads/main", &base);

        let outcome = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
        assert_eq!(
            outcome.attempt.derived_disposition,
            DerivedDisposition::DeterministicBlock
        );
        assert_eq!(outcome.attempt.gate_decision, GateDecision::Block);
        assert!(outcome.attempt.approval_digest.is_none());
        assert_eq!(
            outcome
                .attempt
                .rubric_digests
                .as_ref()
                .unwrap()
                .get("quality/rubrics/documentation.md")
                .unwrap(),
            &sha256_hex(b"# documentation\n")
        );
        let expected_manifest_digest = sha256_hex(manifest().as_bytes());
        assert_eq!(
            outcome.attempt.manifest_digest.as_deref(),
            Some(expected_manifest_digest.as_str())
        );

        let store = Store::open(repo.path()).unwrap();
        let report = store
            .read_evaluation(outcome.attempt.evaluation_report_digest.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            report.derived_disposition,
            DerivedDisposition::DeterministicBlock
        );
        assert_eq!(report.manifest_digest, expected_manifest_digest);
        assert_eq!(
            &report.rubric_digests,
            outcome.attempt.rubric_digests.as_ref().unwrap()
        );
        assert_eq!(report.deterministic_results.checks.len(), 1);
        assert!(!report.deterministic_results.passed());
        assert!(report.axis_results.is_empty());
        assert!(report.coherence_result.is_none());
        assert_eq!(report.semantic_topology.axes.len(), 4);
        assert_eq!(
            report.semantic_topology.axes[0].rubric.to_str().unwrap(),
            "quality/rubrics/documentation.md"
        );
        assert_eq!(
            fs::read_dir(store.root().join("reports")).unwrap().count(),
            1
        );
        assert_eq!(
            fs::read_dir(
                store
                    .root()
                    .join("attempts/content")
                    .join(outcome.attempt.candidate_tree.as_deref().unwrap())
            )
            .unwrap()
            .count(),
            1
        );
        assert!(!repo.path().join(".git/semantic-count").exists());
    }
}

#[test]
fn candidate_mutation_blocks_and_non_head_candidate_reports_diagnostic() {
    let (repo, base, candidate) = repository("pass");
    fs::write(repo.path().join(".git/deterministic-mutate"), []).unwrap();
    let input = line("refs/heads/main", &candidate, "refs/heads/main", &base);
    let mutation = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(mutation.attempt.gate_decision, GateDecision::Block);
    assert_eq!(
        mutation.attempt.derived_disposition,
        DerivedDisposition::DeterministicBlock
    );
    assert!(!mutation.attempt.fresh_deterministic_results[0].passed());

    fs::remove_file(repo.path().join(".git/deterministic-mutate")).unwrap();
    write(repo.path(), "later.txt", "later\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "later"]);
    let error = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap_err();
    assert!(format!("{error:#}").contains("not current checkout HEAD"));
}

#[test]
fn semantic_correction_passes_and_semantic_mutation_blocks_complete_attempt() {
    let (repo, base, _) = repository("pass");
    write(
        repo.path(),
        "behaviors.json",
        "{\"default\":{\"status\":\"pass\",\"invalid\":\"first\"}}\n",
    );
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "correction candidate"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);
    let input = line("refs/heads/main", &candidate, "refs/heads/main", &base);
    let corrected = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(corrected.attempt.gate_decision, GateDecision::Pass);
    let evaluation = Store::open(repo.path())
        .unwrap()
        .read_evaluation(
            corrected
                .attempt
                .evaluation_report_digest
                .as_deref()
                .unwrap(),
        )
        .unwrap();
    assert!(
        evaluation
            .axis_results
            .iter()
            .all(|result| result.attempts.len() == 2)
    );
    assert_eq!(evaluation.coherence_result.unwrap().attempts.len(), 2);

    fs::write(repo.path().join(".git/semantic-mutate"), []).unwrap();
    let mutation = run_publication(repo.path(), input.as_bytes(), &Cancellation::new()).unwrap();
    assert_eq!(mutation.attempt.gate_decision, GateDecision::Block);
    assert_eq!(
        mutation.attempt.derived_disposition,
        DerivedDisposition::SemanticBlock
    );
    let mutation_evaluation = Store::open(repo.path())
        .unwrap()
        .read_evaluation(
            mutation
                .attempt
                .evaluation_report_digest
                .as_deref()
                .unwrap(),
        )
        .unwrap();
    assert!(
        mutation_evaluation
            .axis_results
            .iter()
            .all(|result| result.status != xtask::semantic_judge::SemanticStatus::Pass)
    );
}

#[test]
fn cli_and_tracked_hook_forward_exact_stdin_arguments_and_exit_status() {
    let tracked = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.githooks/pre-push");
    let hook = fs::read_to_string(&tracked).unwrap();
    assert!(hook.contains(
        "exec env -u RUSTUP_TOOLCHAIN cargo xtask validate --publication --updates-stdin"
    ));
    assert!(!hook.contains("quality/manifest.toml"));
    assert!(!hook.contains("GATE_PATHS"));
    assert!(hook.lines().count() <= 8);
    assert_ne!(
        fs::metadata(&tracked).unwrap().permissions().mode() & 0o111,
        0
    );

    let parent = TempDir::new().unwrap();
    let repo = parent.path().join("repo with spaces");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    fs::create_dir_all(repo.join(".githooks")).unwrap();
    fs::copy(&tracked, repo.join(".githooks/pre-push")).unwrap();
    fs::set_permissions(
        repo.join(".githooks/pre-push"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let bin = parent.path().join("bin");
    fs::create_dir(&bin).unwrap();
    write(
        parent.path(),
        "bin/cargo",
        "#!/bin/sh\nprintf '%s' \"$*\" > .git/args\ncat > .git/stdin\nprintf '%s' \"${RUSTUP_TOOLCHAIN-unset}\" > .git/toolchain\nexit 23\n",
    );
    fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    let input = b"exact stdin without final newline";
    let mut child = Command::new(repo.join(".githooks/pre-push"))
        .args(["origin", "ssh://example/repo"])
        .current_dir(&repo)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("RUSTUP_TOOLCHAIN", "must-be-unset")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(input).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(23));
    assert_eq!(
        fs::read_to_string(repo.join(".git/args")).unwrap(),
        "xtask validate --publication --updates-stdin"
    );
    assert_eq!(fs::read(repo.join(".git/stdin")).unwrap(), input);
    assert_eq!(
        fs::read_to_string(repo.join(".git/toolchain")).unwrap(),
        "unset"
    );

    let help = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["validate", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--updates-stdin"));
    assert!(help.contains("--ci-event"));
    let bad = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["validate", "--publication"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let mutually_exclusive = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "validate",
            "--publication",
            "--updates-stdin",
            "--ci-event",
            "event.json",
        ])
        .output()
        .unwrap();
    assert!(!mutually_exclusive.status.success());
}
