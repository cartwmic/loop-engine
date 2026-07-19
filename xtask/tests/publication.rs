use std::fs;
use std::io::Cursor;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use xtask::hooks::{HOOK_VERSION, PRE_PUSH_HOOK_PATH};
use xtask::publication::{
    self, PublicationOptions, PushUpdate, is_zero_oid, publication_base_for_update,
    read_push_updates,
};
use xtask::quality::MANIFEST_PATH;
use xtask::semantic_judge::{self, Disposition, Verdict};

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/publication")
        .join(name)
}

fn docs_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs-check/valid")
}

fn architecture_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/architecture/allowed")
}

fn real_repo_root() -> PathBuf {
    semantic_judge::default_repository_root()
}

fn publication_manifest_text() -> String {
    fs::read_to_string(fixture_root("manifest.toml")).expect("publication fixture manifest")
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
    fs::write(path, content).expect("write");
}

fn copy_dir(src: &Path, dst: &Path) {
    if src.is_dir() {
        fs::create_dir_all(dst).expect("mkdir");
        for entry in fs::read_dir(src).expect("read_dir") {
            let entry = entry.expect("dir entry");
            copy_dir(&entry.path(), &dst.join(entry.file_name()));
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::copy(src, dst).expect("copy fixture file");
    }
}

fn init_seeded_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "publication@test"]);
    git(dir.path(), &["config", "user.name", "Publication Test"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    copy_dir(&architecture_fixture_root(), dir.path());
    copy_dir(&docs_fixture_root(), dir.path());
    write(dir.path(), MANIFEST_PATH, &publication_manifest_text());
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "seed"]);
    dir
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn options(repo: &Path, judge: impl Into<PathBuf>) -> PublicationOptions {
    let mut options = PublicationOptions::new(repo);
    options.judge_executable = Some(judge.into());
    options.timeout_seconds = Some(5);
    options.foundation_git_root = Some(real_repo_root());
    options
}

fn edit_intent(repo: &Path, body: &str) {
    write(
        repo,
        "docs/intent.md",
        &format!("# Valid\n\nValid fixture documentation.\n\n{body}\n"),
    );
}

fn recording_judge(dir: &Path) -> (PathBuf, PathBuf) {
    let script = dir.join("judge-record");
    let ledger = dir.join("requests.jsonl");
    let ledger_literal = serde_json::to_string(&ledger.to_string_lossy()).expect("path json");
    write(
        dir,
        "judge-record",
        &format!(
            r#"#!/usr/bin/env python3
import json, pathlib, sys
request = json.load(sys.stdin)
with pathlib.Path({ledger_literal}).open("a", encoding="utf-8") as out:
    out.write(json.dumps(request, sort_keys=True) + "\n")
response = {{
    "schema_version": 1,
    "parent_revision": request["parent_revision"],
    "candidate_revision": request["candidate_revision"],
    "verdict": "pass",
    "citations": [{{"rubric_id": request["rubrics"][0]["id"], "rule": "I47", "lines": ["docs/first.md:1"]}}],
    "message": "recorded aggregate request"
}}
json.dump(response, sys.stdout)
"#,
        ),
    );
    let mut permissions = fs::metadata(&script)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("chmod");
    (script, ledger)
}

#[test]
fn remote_update_input_parses_all_lines() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let c = "c".repeat(40);
    let d = "d".repeat(40);
    let input =
        format!("refs/heads/main {a} refs/heads/main {b}\nrefs/heads/x {c} refs/heads/x {d}\n");
    let updates = read_push_updates(Cursor::new(input)).expect("parse updates");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].remote_sha, b);
}

#[test]
fn multi_ref_content_push_is_rejected_before_execution() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let head = commit_all(repo.path(), "first");
    let updates = vec![
        PushUpdate {
            local_ref: "refs/heads/main".into(),
            local_sha: head.clone(),
            remote_ref: "refs/heads/main".into(),
            remote_sha: base.clone(),
        },
        PushUpdate {
            local_ref: "refs/tags/checkpoint".into(),
            local_sha: head,
            remote_ref: "refs/tags/checkpoint".into(),
            remote_sha: "0000000000000000000000000000000000000000".into(),
        },
    ];

    let error =
        publication::publish_updates(&options(repo.path(), fixture_root("judge-pass")), &updates)
            .expect_err("multi-content push must fail");
    assert!(error.to_string().contains("at most one non-delete"));
}

#[test]
fn pushed_candidate_must_equal_checked_out_head() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["checkout", "-b", "other"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let other = commit_all(repo.path(), "other");
    git(repo.path(), &["checkout", "main"]);
    let update = PushUpdate {
        local_ref: "refs/heads/other".into(),
        local_sha: other,
        remote_ref: "refs/heads/other".into(),
        remote_sha: base,
    };

    let error =
        publication::publish_updates(&options(repo.path(), fixture_root("judge-pass")), &[update])
            .expect_err("different checked-out head must fail");
    assert!(error.to_string().contains("differs from checked-out HEAD"));
}

#[test]
fn annotated_tag_object_cannot_alias_checked_out_head() {
    let repo = init_seeded_repo();
    git(
        repo.path(),
        &["tag", "-a", "checkpoint", "-m", "checkpoint"],
    );
    let tag_object = git(repo.path(), &["rev-parse", "refs/tags/checkpoint"]);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_ne!(tag_object, head);
    let update = PushUpdate {
        local_ref: "refs/tags/checkpoint".into(),
        local_sha: tag_object,
        remote_ref: "refs/tags/checkpoint".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    };

    let error =
        publication::publish_updates(&options(repo.path(), fixture_root("judge-pass")), &[update])
            .expect_err("annotated tag object must not alias HEAD commit");
    assert!(error.to_string().contains("differs from checked-out HEAD"));
}

#[test]
fn split_quality_and_semantic_phases_preserve_one_bound_checkpoint() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let head = commit_all(repo.path(), "first");
    let judge_dir = TempDir::new().expect("judge tempdir");
    let (judge, ledger) = recording_judge(judge_dir.path());
    let options = options(repo.path(), judge);

    let evidence = publication::produce_quality_evidence(&options, &base, &head)
        .expect("credential-free quality phase");
    assert_eq!(evidence.base_revision, base);
    assert_eq!(evidence.candidate_revision, head);
    assert_eq!(evidence.quality.checks.len(), 2);

    let outcome = publication::publish_with_quality_evidence(&options, &base, &head, evidence)
        .expect("semantic-only phase");
    assert_eq!(outcome.checkpoint.base_revision, base);
    assert_eq!(outcome.checkpoint.candidate_revision, head);
    assert_eq!(
        fs::read_to_string(ledger).expect("ledger").lines().count(),
        1
    );
}

#[test]
fn split_semantic_phase_rejects_unbound_quality_evidence() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let head = commit_all(repo.path(), "first");
    let options = options(repo.path(), fixture_root("judge-pass"));
    let mut evidence =
        publication::produce_quality_evidence(&options, &base, &head).expect("quality evidence");
    evidence.candidate_revision = base.clone();

    let error = publication::publish_with_quality_evidence(&options, &base, &head, evidence)
        .expect_err("unbound evidence must fail");
    assert!(error.to_string().contains("revision binding"));
}

#[test]
fn multi_commit_update_runs_one_aggregate_checkpoint() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    commit_all(repo.path(), "first");
    write(repo.path(), "docs/second.md", "# Second\n");
    let head = commit_all(repo.path(), "second");
    let judge_dir = TempDir::new().expect("judge tempdir");
    let (judge, ledger) = recording_judge(judge_dir.path());

    let update = PushUpdate {
        local_ref: "refs/heads/main".into(),
        local_sha: head.clone(),
        remote_ref: "refs/heads/main".into(),
        remote_sha: base.clone(),
    };
    let outcomes = publication::publish_updates(&options(repo.path(), judge), &[update])
        .expect("aggregate publication");

    assert_eq!(outcomes.len(), 1);
    let checkpoint = &outcomes[0].checkpoint;
    assert_eq!(checkpoint.base_revision, base);
    assert_eq!(checkpoint.candidate_revision, head);
    assert_eq!(checkpoint.quality.checks.len(), 2);
    assert_eq!(checkpoint.judge.disposition, Disposition::Allow);

    let requests: Vec<serde_json::Value> = fs::read_to_string(ledger)
        .expect("request ledger")
        .lines()
        .map(|line| serde_json::from_str(line).expect("request json"))
        .collect();
    assert_eq!(requests.len(), 1, "one push range must invoke judge once");
    assert!(
        requests[0]["diff"]
            .as_str()
            .unwrap()
            .contains("docs/first.md")
    );
    assert!(
        requests[0]["diff"]
            .as_str()
            .unwrap()
            .contains("docs/second.md")
    );
}

#[test]
fn internal_bad_commit_may_be_repaired_before_candidate_head() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "FAIL_MIDDLE");
    commit_all(repo.path(), "incomplete internal commit");
    edit_intent(repo.path(), "Repaired final checkpoint");
    let head = commit_all(repo.path(), "repair before publication");

    let outcome = publication::publish_range(
        &options(repo.path(), fixture_root("judge-fail-on-middle")),
        &base,
        &head,
    )
    .expect("aggregate final state should pass");
    assert_eq!(outcome.checkpoint.judge.response.verdict, Verdict::Pass);
}

#[test]
fn bad_candidate_head_blocks_whole_range() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "intermediate valid");
    commit_all(repo.path(), "intermediate");
    edit_intent(repo.path(), "FAIL_MIDDLE");
    let head = commit_all(repo.path(), "bad final head");

    let error = publication::publish_range(
        &options(repo.path(), fixture_root("judge-fail-on-middle")),
        &base,
        &head,
    )
    .expect_err("bad final checkpoint must block");
    assert!(format!("{error:#}").contains("blocked aggregate publication"));
}

#[test]
fn unavailable_and_indeterminate_block_publication() {
    for judge in ["judge-unavailable", "judge-indeterminate"] {
        let repo = init_seeded_repo();
        let base = git(repo.path(), &["rev-parse", "HEAD"]);
        edit_intent(repo.path(), judge);
        let head = commit_all(repo.path(), judge);
        let error =
            publication::publish_range(&options(repo.path(), fixture_root(judge)), &base, &head)
                .expect_err("non-pass must block");
        assert!(format!("{error:#}").contains("blocked aggregate publication"));
    }
}

#[test]
fn divergent_candidate_is_rejected_before_quality_or_judge() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "local");
    let head = commit_all(repo.path(), "local");
    git(repo.path(), &["checkout", "--detach", &base]);
    edit_intent(repo.path(), "other");
    let other = commit_all(repo.path(), "other");
    git(repo.path(), &["checkout", "main"]);

    let error = publication::publish_range(
        &options(repo.path(), fixture_root("judge-pass")),
        &other,
        &head,
    )
    .expect_err("divergent replacement must fail");
    assert!(format!("{error:#}").contains("not a fast-forward descendant"));
}

#[test]
fn merge_commit_in_range_is_rejected() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["checkout", "-b", "side"]);
    write(repo.path(), "docs/side.md", "# Side\n");
    commit_all(repo.path(), "side");
    git(repo.path(), &["checkout", "main"]);
    write(repo.path(), "docs/main.md", "# Main\n");
    commit_all(repo.path(), "main");
    git(repo.path(), &["merge", "--no-ff", "side", "-m", "merge"]);
    let head = git(repo.path(), &["rev-parse", "HEAD"]);

    let error = publication::publish_range(
        &options(repo.path(), fixture_root("judge-pass")),
        &base,
        &head,
    )
    .expect_err("merge range must fail");
    assert!(format!("{error:#}").contains("unsupported merge commit"));
}

#[test]
fn candidate_manifest_cannot_weaken_base_manifest() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(
        repo.path(),
        MANIFEST_PATH,
        "schema_version = 1\n\n[[checks]]\nid = \"docs-check\"\nrunner = \"docs-check\"\n",
    );
    let head = commit_all(repo.path(), "weaken manifest");

    let error = publication::publish_range(
        &options(repo.path(), fixture_root("judge-pass")),
        &base,
        &head,
    )
    .expect_err("manifest weakening must block");
    assert!(format!("{error:#}").contains("manifest"));
}

#[test]
fn candidate_manifest_symlink_is_rejected_without_following_it() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    fs::remove_file(repo.path().join(MANIFEST_PATH)).expect("remove manifest");
    symlink("../../docs/intent.md", repo.path().join(MANIFEST_PATH)).expect("symlink manifest");
    let head = commit_all(repo.path(), "symlink manifest");

    let error = publication::produce_quality_evidence(
        &options(repo.path(), fixture_root("judge-pass")),
        &base,
        &head,
    )
    .expect_err("manifest symlink must fail closed");
    assert!(format!("{error:#}").contains("regular 100644 Git blob"));
}

#[test]
fn candidate_cannot_remove_base_manifest() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    fs::remove_file(repo.path().join(MANIFEST_PATH)).expect("remove manifest");
    let head = commit_all(repo.path(), "remove manifest");

    let error = publication::publish_range(
        &options(repo.path(), fixture_root("judge-pass")),
        &base,
        &head,
    )
    .expect_err("manifest removal must block");
    assert!(format!("{error:#}").contains("removed quality/manifest.toml"));
}

#[test]
fn new_branch_uses_exact_advertised_main_not_tracking_ref() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    let remote = TempDir::new().expect("remote tempdir");
    git(remote.path(), &["init", "--bare"]);
    let remote_url = remote.path().to_string_lossy().to_string();
    git(repo.path(), &["push", &remote_url, "main:main"]);

    edit_intent(repo.path(), "feature one");
    commit_all(repo.path(), "feature one");
    edit_intent(repo.path(), "feature two");
    let head = commit_all(repo.path(), "feature two");
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/main", &head],
    );

    let update = PushUpdate {
        local_ref: "refs/heads/feature".into(),
        local_sha: head,
        remote_ref: "refs/heads/feature".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    };
    let selected =
        publication_base_for_update(repo.path(), &update, Some("origin"), Some(&remote_url))
            .expect("resolve advertised base")
            .expect("non-delete base");
    assert_eq!(selected, base);
}

#[test]
fn new_branch_requires_advertised_main() {
    let repo = init_seeded_repo();
    edit_intent(repo.path(), "feature");
    let head = commit_all(repo.path(), "feature");
    let remote = TempDir::new().expect("remote tempdir");
    git(remote.path(), &["init", "--bare"]);
    let remote_url = remote.path().to_string_lossy().to_string();
    let update = PushUpdate {
        local_ref: "refs/heads/feature".into(),
        local_sha: head,
        remote_ref: "refs/heads/feature".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    };

    let error =
        publication_base_for_update(repo.path(), &update, Some("origin"), Some(&remote_url))
            .expect_err("missing integration ref must block");
    assert!(format!("{error:#}").contains("refs/heads/main"));
}

#[test]
fn delete_update_needs_no_gate() {
    let repo = init_seeded_repo();
    let update = PushUpdate {
        local_ref: "(delete)".into(),
        local_sha: "0000000000000000000000000000000000000000".into(),
        remote_ref: "refs/heads/old".into(),
        remote_sha: git(repo.path(), &["rev-parse", "HEAD"]),
    };
    assert!(is_zero_oid(&update.local_sha));
    assert!(
        publication_base_for_update(repo.path(), &update, None, None)
            .expect("delete")
            .is_none()
    );
}

#[test]
fn aggregate_gate_does_not_rewrite_user_tree() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "candidate");
    let head = commit_all(repo.path(), "candidate");
    write(repo.path(), "scratch.txt", "untracked\n");
    let before = git(repo.path(), &["status", "--porcelain"]);

    publication::publish_range(
        &options(repo.path(), fixture_root("judge-pass")),
        &base,
        &head,
    )
    .expect("publication");
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), before);
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), head);
}

#[test]
fn purity_violation_is_reported_even_when_judge_blocks() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let head = commit_all(repo.path(), "candidate");
    let judge = repo.path().join("mutating-judge");
    write(
        repo.path(),
        "mutating-judge",
        r#"#!/usr/bin/env python3
import json, pathlib, sys
request = json.load(sys.stdin)
pathlib.Path("judge-mutated.txt").write_text("mutated\n", encoding="utf-8")
json.dump({
  "schema_version": 1,
  "parent_revision": request["parent_revision"],
  "candidate_revision": request["candidate_revision"],
  "verdict": "fail",
  "citations": [{"rubric_id": request["rubrics"][0]["id"], "rule": "I47", "lines": ["docs/first.md:1"]}],
  "message": "blocked"
}, sys.stdout)
"#,
    );
    let mut permissions = fs::metadata(&judge).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&judge, permissions).expect("chmod");

    let error = publication::publish_range(&options(repo.path(), judge), &base, &head)
        .expect_err("mutation plus judge failure must block");
    assert!(format!("{error:#}").contains("must not rewrite user tree"));
}

#[test]
fn split_semantic_purity_violation_is_reported_on_failure() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "docs/first.md", "# First\n");
    let head = commit_all(repo.path(), "candidate");
    let initial = options(repo.path(), fixture_root("judge-pass"));
    let evidence =
        publication::produce_quality_evidence(&initial, &base, &head).expect("quality evidence");

    let judge_dir = TempDir::new().expect("judge tempdir");
    let judge = judge_dir.path().join("mutating-judge");
    write(
        judge_dir.path(),
        "mutating-judge",
        r#"#!/usr/bin/env python3
import json, pathlib, sys
request = json.load(sys.stdin)
pathlib.Path("judge-mutated.txt").write_text("mutated\n", encoding="utf-8")
json.dump({
  "schema_version": 1,
  "parent_revision": request["parent_revision"],
  "candidate_revision": request["candidate_revision"],
  "verdict": "fail",
  "citations": [{"rubric_id": request["rubrics"][0]["id"], "rule": "I47", "lines": ["docs/first.md:1"]}],
  "message": "blocked"
}, sys.stdout)
"#,
    );
    let mut permissions = fs::metadata(&judge).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&judge, permissions).expect("chmod");

    let error = publication::publish_with_quality_evidence(
        &options(repo.path(), judge),
        &base,
        &head,
        evidence,
    )
    .expect_err("split semantic mutation must block");
    assert!(format!("{error:#}").contains("must not rewrite user tree"));
}

#[test]
fn ci_uses_protected_workflow_and_separate_credential_phase() {
    let workflow = fs::read_to_string(real_repo_root().join(".github/workflows/quality.yml"))
        .expect("quality workflow");
    assert!(workflow.contains("pull_request_target:\n    branches: [main]"));
    assert!(!workflow.contains("\n  pull_request:\n"));
    assert_eq!(
        workflow
            .matches("Checkout candidate objects without execution")
            .count(),
        1,
        "privileged semantic job must consume only inert Git bundle"
    );
    assert!(workflow.contains("Import exact candidate objects from inert bundle"));
    assert!(workflow.contains("needs: publication-quality-evidence"));
    assert!(workflow.contains("--quality-report-out"));
    assert!(workflow.contains("--quality-report-in"));
    assert!(workflow.contains("Build trusted semantic gate before provisioning credentials"));
    assert!(workflow.contains("trusted/quality/semantic-judge/v1/judge"));
}

#[test]
fn versioned_pre_push_hook_declares_current_version_and_remote_inputs() {
    let body = fs::read_to_string(real_repo_root().join(PRE_PUSH_HOOK_PATH))
        .expect("versioned pre-push hook");
    assert_eq!(
        xtask::hooks::parse_hook_version(&body).expect("version marker"),
        HOOK_VERSION
    );
    assert!(body.contains("hooks pre-push"));
    assert!(body.contains("--remote-name") && body.contains("--remote-url"));
    assert!(body.contains("git diff --quiet HEAD") && body.contains("git ls-files --others"));
    assert!(body.contains("CARGO_TARGET_DIR") && body.contains("target/publication-cargo"));
}
