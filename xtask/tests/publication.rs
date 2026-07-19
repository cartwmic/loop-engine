use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use xtask::hooks::{HOOK_VERSION, PRE_PUSH_HOOK_PATH};
use xtask::publication::{
    self, PublicationOptions, PushUpdate, is_zero_oid, read_push_updates,
    unpublished_commits_for_update,
};
use xtask::quality::MANIFEST_PATH;
use xtask::semantic_judge::{self, Disposition, FOUNDATION_PARENT_REVISION, Verdict};

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
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/publication/manifest.toml"),
    )
    .expect("publication fixture manifest")
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

fn options(repo: &Path, judge: &str) -> PublicationOptions {
    let mut options = PublicationOptions::new(repo);
    options.judge_executable = Some(fixture_root(judge));
    options.timeout_seconds = Some(5);
    options.foundation_git_root = Some(real_repo_root());
    options.quality_manifest = Some(repo.join(MANIFEST_PATH));
    options
}

fn edit_intent(repo: &Path, body: &str) {
    write(
        repo,
        "docs/intent.md",
        &format!("# Valid\n\nValid fixture documentation.\n\n{body}\n"),
    );
}

#[test]
fn remote_update_input_enumerates_unpublished_commits_oldest_first() {
    let repo = init_seeded_repo();
    let remote = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let c1 = commit_all(repo.path(), "c1");
    edit_intent(repo.path(), "c2");
    let c2 = commit_all(repo.path(), "c2");

    let update = PushUpdate {
        local_ref: "refs/heads/topic".into(),
        local_sha: c2.clone(),
        remote_ref: "refs/heads/topic".into(),
        remote_sha: remote.clone(),
    };
    let commits =
        unpublished_commits_for_update(repo.path(), &update, None, None).expect("enumerate");
    assert_eq!(commits, vec![c1, c2]);
}

#[test]
fn read_push_updates_parses_stdin_lines() {
    let stdin = "refs/heads/main abc refs/heads/main 0000000000000000000000000000000000000000\n";
    let updates = read_push_updates(Cursor::new(stdin)).expect("parse");
    assert_eq!(updates.len(), 1);
    assert!(is_zero_oid(&updates[0].remote_sha));
    assert_eq!(updates[0].local_sha, "abc");
}

#[test]
fn multi_commit_range_passes_with_parent_rubric_and_quality() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let _c1 = commit_all(repo.path(), "c1");
    edit_intent(repo.path(), "c2");
    let head = commit_all(repo.path(), "c2");

    let outcome = publication::publish_range(&options(repo.path(), "judge-pass"), &base, &head)
        .expect("multi-commit pass");
    assert_eq!(outcome.commits.len(), 2);
    assert!(
        outcome
            .commits
            .iter()
            .all(|commit| commit.judge.response.verdict == Verdict::Pass
                && commit.judge.disposition == Disposition::Allow
                && commit.quality.passed())
    );
}

#[test]
fn unavailable_judge_fails_closed_for_publication() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let head = commit_all(repo.path(), "c1");

    let error =
        publication::publish_range(&options(repo.path(), "judge-unavailable"), &base, &head)
            .expect_err("unavailable must block publication");
    let message = format!("{error:#}");
    assert!(
        message.contains("unavailable") || message.contains("blocked"),
        "unexpected error: {message}"
    );
}

#[test]
fn indeterminate_judge_fails_closed_for_publication() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let head = commit_all(repo.path(), "c1");

    let error =
        publication::publish_range(&options(repo.path(), "judge-indeterminate"), &base, &head)
            .expect_err("indeterminate must block publication");
    let message = format!("{error:#}");
    assert!(
        message.contains("indeterminate") || message.contains("blocked"),
        "unexpected error: {message}"
    );
}

#[test]
fn failing_middle_commit_cannot_be_repaired_by_later_good_commit() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

    edit_intent(repo.path(), "good-1");
    let _c1 = commit_all(repo.path(), "good-1");

    // Middle commit is semantically bad (marker) and also docs-clean so quality
    // passes; the judge fixture fails only this middle commit.
    edit_intent(repo.path(), "FAIL_MIDDLE bad middle");
    let middle = commit_all(repo.path(), "bad-middle");

    edit_intent(repo.path(), "good-3 repaired tip");
    let head = commit_all(repo.path(), "good-3");

    let error =
        publication::publish_range(&options(repo.path(), "judge-fail-on-middle"), &base, &head)
            .expect_err("failing middle must block whole range");
    let message = format!("{error:#}");
    assert!(
        message.contains(&middle) || message.contains("blocked") || message.contains("fail"),
        "unexpected error: {message}"
    );

    // Tip alone would pass; the range gate must still refuse because middle failed.
    let tip_parent = git(repo.path(), &["rev-parse", &format!("{head}^")]);
    let tip_only = publication::publish_range(
        &options(repo.path(), "judge-fail-on-middle"),
        &tip_parent,
        &head,
    )
    .expect("later good commit alone passes");
    assert_eq!(tip_only.commits.len(), 1);
    assert_eq!(tip_only.commits[0].judge.response.verdict, Verdict::Pass);
}

#[test]
fn failing_middle_quality_cannot_be_repaired_by_later_docs_fix() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

    edit_intent(repo.path(), "good-1");
    let _c1 = commit_all(repo.path(), "good-1");

    // Middle commit introduces trailing whitespace — docs-check fails in that
    // commit's detached worktree even if a later commit repairs the file.
    write(
        repo.path(),
        "docs/intent.md",
        "# Valid\n\nValid fixture documentation.   \n\nmiddle broken\n",
    );
    let middle = commit_all(repo.path(), "bad-docs-middle");

    edit_intent(repo.path(), "repaired tip");
    let head = commit_all(repo.path(), "repair-docs");

    let error = publication::publish_range(&options(repo.path(), "judge-pass"), &base, &head)
        .expect_err("middle docs failure must block");
    let message = format!("{error:#}");
    assert!(
        message.contains("docs-check")
            || message.contains(&middle)
            || message.contains("quality check"),
        "unexpected error: {message}"
    );
}

#[test]
fn changed_rubric_applies_only_to_following_commit_through_publication_gate() {
    let repo = init_seeded_repo();

    let seed = {
        // Build foundation-seed content via staged judge path in an empty-manifest sense:
        // reuse committed workspace foundation seed text.
        fs::read_to_string(real_repo_root().join("quality/rubrics/foundation-seed.v1.md"))
            .expect("foundation seed")
    };
    let digest = {
        let output = Command::new("python3")
            .args([
                "-c",
                "import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.as_mut().unwrap().write_all(seed.as_bytes())?;
                Ok(child.wait_with_output()?.stdout)
            })
            .expect("digest");
        String::from_utf8(output).unwrap().trim().to_owned()
    };

    let manifest_v1 = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": digest
        }]
    });
    write(
        repo.path(),
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(&manifest_v1).unwrap(),
    );
    write(repo.path(), "quality/rubrics/foundation-seed.v1.md", &seed);
    let parent_with_v1 = commit_all(repo.path(), "rubric-v1");

    let changed = format!("{seed}\n\n## Changed rubric marker\n\nnext-commit-only\n");
    let changed_digest = {
        let output = Command::new("python3")
            .args([
                "-c",
                "import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(changed.as_bytes())?;
                Ok(child.wait_with_output()?.stdout)
            })
            .expect("digest");
        String::from_utf8(output).unwrap().trim().to_owned()
    };
    let manifest_v2 = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": changed_digest
        }]
    });
    write(
        repo.path(),
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(&manifest_v2).unwrap(),
    );
    write(
        repo.path(),
        "quality/rubrics/foundation-seed.v1.md",
        &changed,
    );
    edit_intent(repo.path(), "rubric-change-commit");
    let rubric_change_commit = commit_all(repo.path(), "change-rubric");

    edit_intent(repo.path(), "following-commit");
    let following = commit_all(repo.path(), "following");

    let outcome = publication::publish_range(
        &options(repo.path(), "judge-pass"),
        &parent_with_v1,
        &following,
    )
    .expect("rubric-change range");
    assert_eq!(outcome.commits.len(), 2);

    let changing = &outcome.commits[0];
    assert_eq!(changing.candidate_revision, rubric_change_commit);
    assert_eq!(
        changing.judge.request["rubrics"][0]["content"]
            .as_str()
            .unwrap_or_default(),
        seed
    );
    assert!(
        !changing.judge.request["rubrics"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("next-commit-only")
    );

    let next = &outcome.commits[1];
    assert_eq!(next.candidate_revision, following);
    assert!(
        next.judge.request["rubrics"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("next-commit-only")
    );
}

#[test]
fn publication_gate_does_not_rewrite_user_tree() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let head = commit_all(repo.path(), "c1");

    // Contaminate the working tree after commit.
    write(
        repo.path(),
        "docs/intent.md",
        "# Valid\n\nValid fixture documentation.   \n\ncontaminated working tree\n",
    );
    let before = fs::read_to_string(repo.path().join("docs/intent.md")).expect("read");
    assert!(before.contains("documentation.   \n"));

    let _outcome = publication::publish_range(&options(repo.path(), "judge-pass"), &base, &head)
        .expect("gate against exact commits");

    let after = fs::read_to_string(repo.path().join("docs/intent.md")).expect("read after");
    assert_eq!(before, after, "user working tree must remain untouched");
}

#[test]
fn versioned_pre_push_hook_delegates_to_xtask() {
    let body = fs::read_to_string(real_repo_root().join(PRE_PUSH_HOOK_PATH))
        .expect("versioned pre-push hook");
    let version = xtask::hooks::parse_hook_version(&body).expect("version marker");
    assert_eq!(version, HOOK_VERSION);
    assert!(
        body.contains("cargo run") && body.contains("hooks pre-push"),
        "thin hook must delegate to xtask hooks pre-push"
    );
    assert!(
        body.contains("env -u RUSTUP_TOOLCHAIN") && body.contains("git rev-parse --local-env-vars"),
        "hook must honor the toolchain pin and isolate nested fixture Git repositories"
    );
    assert!(
        body.contains("--remote-name")
            && body.contains("${1:")
            && body.contains("--remote-url")
            && body.contains("${2:"),
        "thin hook must forward Git's exact destination remote name and URL"
    );
    assert!(
        !body.contains("git worktree"),
        "thin hook must not embed canonical worktree logic"
    );
}

#[test]
fn new_branch_publish_updates_excludes_commits_on_remote_refs() {
    let repo = init_seeded_repo();
    let published = git(repo.path(), &["rev-parse", "HEAD"]);
    let remote = TempDir::new().expect("remote tempdir");
    git(remote.path(), &["init", "--bare"]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            remote.path().to_str().expect("remote path"),
        ],
    );
    let refspec = format!("{published}:refs/heads/main");
    git(repo.path(), &["push", "origin", &refspec]);
    edit_intent(repo.path(), "new-branch-c1");
    let c1 = commit_all(repo.path(), "new-branch-c1");
    edit_intent(repo.path(), "new-branch-c2");
    let c2 = commit_all(repo.path(), "new-branch-c2");
    git(
        repo.path(),
        &["update-ref", "refs/remotes/backup/topic", &c2],
    );

    let updates = [PushUpdate {
        local_ref: "refs/heads/new-branch".into(),
        local_sha: c2.clone(),
        remote_ref: "refs/heads/new-branch".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    }];
    let mut opts = options(repo.path(), "judge-pass");
    opts.remote_name = Some("origin".into());
    opts.remote_url = Some(remote.path().to_string_lossy().into_owned());
    let outcomes = publication::publish_updates(&opts, &updates).expect("publish new branch");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].commits.len(), 2);
    assert_eq!(outcomes[0].commits[0].candidate_revision, c1);
    assert_eq!(outcomes[0].commits[1].candidate_revision, c2);
}

#[test]
fn quality_command_evidence_is_bound_into_publication_judge_request() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "quality-evidence");
    let head = commit_all(repo.path(), "quality-evidence");

    let evidence_manifest = repo.path().join("quality-evidence.toml");
    fs::write(
        &evidence_manifest,
        r#"schema_version = 1
[[checks]]
id = "check"
runner = "cargo-check"
[[checks]]
id = "test"
runner = "cargo-test"
"#,
    )
    .expect("write evidence manifest");

    let mut opts = options(repo.path(), "judge-pass");
    opts.quality_manifest = Some(evidence_manifest);
    let outcome = publication::publish_range(&opts, &base, &head).expect("publish with evidence");
    let evidence = outcome.commits[0].judge.request["deterministic_evidence"]
        .as_array()
        .expect("deterministic evidence array");
    for command in [
        "cargo check --workspace --locked",
        "cargo test --workspace --locked",
    ] {
        let item = evidence
            .iter()
            .find(|item| item["command"] == command)
            .unwrap_or_else(|| panic!("missing `{command}` evidence: {evidence:?}"));
        assert_eq!(item["exit_code"], 0);
        assert_eq!(item["candidate_revision"], head);
    }
}

#[test]
fn every_blocking_judge_verdict_is_emitted_before_exit() {
    for (fixture, verdict) in [
        ("judge-fail", "fail"),
        ("judge-indeterminate", "indeterminate"),
        ("judge-unavailable", "unavailable"),
    ] {
        let repo = init_seeded_repo();
        let base = git(repo.path(), &["rev-parse", "HEAD"]);
        edit_intent(repo.path(), verdict);
        let head = commit_all(repo.path(), verdict);
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args([
                "publication",
                "--from",
                &base,
                "--to",
                &head,
                "--executable",
                fixture_root(fixture).to_str().unwrap(),
            ])
            .current_dir(repo.path())
            .output()
            .expect("xtask publication");
        assert!(!output.status.success(), "{verdict} must block publication");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let responses: Vec<serde_json::Value> = stdout
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert_eq!(responses.len(), 1, "stdout={stdout}");
        assert_eq!(responses[0]["verdict"], verdict);
        assert_eq!(responses[0]["candidate_revision"], head);
    }
}

#[test]
fn failing_middle_cli_emits_prior_pass_and_blocking_response() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "good-first");
    let first = commit_all(repo.path(), "good-first");
    edit_intent(repo.path(), "FAIL_MIDDLE bad");
    let middle = commit_all(repo.path(), "bad-middle");
    edit_intent(repo.path(), "unreached-later");
    let head = commit_all(repo.path(), "unreached-later");

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "publication",
            "--from",
            &base,
            "--to",
            &head,
            "--executable",
            fixture_root("judge-fail-on-middle").to_str().unwrap(),
        ])
        .current_dir(repo.path())
        .output()
        .expect("xtask publication");
    assert!(!output.status.success());
    let responses: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert_eq!(responses.len(), 2, "only attempted commits emit responses");
    assert_eq!(responses[0]["candidate_revision"], first);
    assert_eq!(responses[0]["verdict"], "pass");
    assert_eq!(responses[1]["candidate_revision"], middle);
    assert_eq!(responses[1]["verdict"], "fail");
}

#[test]
fn publication_cli_exposes_no_quality_manifest_override() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["publication", "--manifest", "reduced.toml"])
        .output()
        .expect("xtask publication parse");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--manifest'"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn publish_updates_from_remote_input_gates_range() {
    let repo = init_seeded_repo();
    let remote = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "c1");
    let _c1 = commit_all(repo.path(), "c1");
    edit_intent(repo.path(), "c2");
    let head = commit_all(repo.path(), "c2");

    let updates = [PushUpdate {
        local_ref: "refs/heads/topic".into(),
        local_sha: head,
        remote_ref: "refs/heads/topic".into(),
        remote_sha: remote,
    }];
    let outcomes = publication::publish_updates(&options(repo.path(), "judge-pass"), &updates)
        .expect("publish updates");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].commits.len(), 2);
}
fn init_repo_without_manifest() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "publication@test"]);
    git(dir.path(), &["config", "user.name", "Publication Test"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);

    copy_dir(&architecture_fixture_root(), dir.path());
    copy_dir(&docs_fixture_root(), dir.path());

    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "seed-without-manifest"]);
    dir
}

#[test]
fn pre_manifest_commits_use_baseline_without_tip_manifest() {
    let repo = init_repo_without_manifest();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "pre-manifest-only");
    let head = commit_all(repo.path(), "pre-manifest");

    let mut opts = options(repo.path(), "judge-pass");
    opts.quality_manifest = None;
    let outcome = publication::publish_range(&opts, &base, &head)
        .expect("pre-manifest baseline should pass with judge only");
    assert_eq!(outcome.commits.len(), 1);
    assert!(
        outcome.commits[0].quality.checks.is_empty(),
        "pre-manifest commits must not run tip manifest checks"
    );
}

#[test]
fn pre_manifest_baseline_blocks_diff_check_failure() {
    let repo = init_repo_without_manifest();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    write(repo.path(), "bad.txt", "trailing whitespace   \n");
    let head = commit_all(repo.path(), "bad-whitespace");

    let mut opts = options(repo.path(), "judge-pass");
    opts.quality_manifest = None;
    let error = publication::publish_range(&opts, &base, &head)
        .expect_err("pre-manifest diff-check failure must block");
    assert!(
        format!("{error:#}").contains("git diff --check"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn manifest_check_removal_attack_is_rejected() {
    let repo = init_repo_without_manifest();
    let full = publication_manifest_text();
    write(repo.path(), MANIFEST_PATH, &full);
    let with_manifest = commit_all(repo.path(), "introduce-manifest");

    let reduced = r#"
schema_version = 1

[[checks]]
id = "docs-check"
runner = "docs-check"
"#;
    write(repo.path(), MANIFEST_PATH, reduced);
    edit_intent(repo.path(), "remove-architecture-check");
    let head = commit_all(repo.path(), "attack-remove-check");

    let mut opts = options(repo.path(), "judge-pass");
    opts.quality_manifest = None;
    let error = publication::publish_range(&opts, &with_manifest, &head)
        .expect_err("removing manifest checks must fail");
    let message = format!("{error:#}");
    assert!(
        message.contains("removed check") || message.contains("regression"),
        "unexpected error: {message}"
    );
}

#[test]
fn new_branch_enumerates_all_local_commits_oldest_first() {
    let repo = init_seeded_repo();
    let published = git(repo.path(), &["rev-parse", "HEAD"]);
    let remote = TempDir::new().expect("remote tempdir");
    git(remote.path(), &["init", "--bare"]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            remote.path().to_str().expect("remote path"),
        ],
    );
    let refspec = format!("{published}:refs/heads/main");
    git(repo.path(), &["push", "origin", &refspec]);

    // Destination advances with an object absent locally, and local tracking
    // state is removed. Enumeration must still discover the shared seed from
    // the destination's fetched graph rather than attempt to republish root.
    let remote_work = TempDir::new().expect("remote work tempdir");
    git(
        remote_work.path(),
        &["clone", remote.path().to_str().expect("remote path"), "."],
    );
    git(remote_work.path(), &["config", "user.email", "remote@test"]);
    git(remote_work.path(), &["config", "user.name", "Remote Test"]);
    write(remote_work.path(), "remote-only.txt", "remote-only\n");
    commit_all(remote_work.path(), "remote-only");
    git(remote_work.path(), &["push", "origin", "HEAD:main"]);
    git(
        repo.path(),
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );

    edit_intent(repo.path(), "c1");
    let c1 = commit_all(repo.path(), "c1");
    // Forged/stale tracking refs are not destination publication evidence.
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/stale", &c1],
    );
    edit_intent(repo.path(), "c2");
    let c2 = commit_all(repo.path(), "c2");

    let update = PushUpdate {
        local_ref: "refs/heads/new-branch".into(),
        local_sha: c2.clone(),
        remote_ref: "refs/heads/new-branch".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    };
    let commits = unpublished_commits_for_update(
        repo.path(),
        &update,
        Some("origin"),
        Some(remote.path().to_str().expect("remote path")),
    )
    .expect("enumerate");
    assert!(commits.contains(&c1));
    assert!(commits.contains(&c2));
    assert_eq!(commits.first().map(String::as_str), Some(c1.as_str()));
    assert_eq!(commits.last().map(String::as_str), Some(c2.as_str()));
}

#[test]
fn new_branch_without_destination_remote_name_fails_closed() {
    let repo = init_seeded_repo();
    let local_sha = git(repo.path(), &["rev-parse", "HEAD"]);
    let update = PushUpdate {
        local_ref: "refs/heads/new-branch".into(),
        local_sha,
        remote_ref: "refs/heads/new-branch".into(),
        remote_sha: "0000000000000000000000000000000000000000".into(),
    };
    let error = unpublished_commits_for_update(repo.path(), &update, None, None)
        .expect_err("missing destination remote must block");
    assert!(error.to_string().contains("destination remote name"));
}

#[test]
fn divergent_update_enumerates_only_new_commits() {
    let repo = init_seeded_repo();
    let seed = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "remote-only");
    let remote = commit_all(repo.path(), "remote-only");
    git(repo.path(), &["reset", "--hard", &seed]);
    edit_intent(repo.path(), "c1");
    let c1 = commit_all(repo.path(), "c1");
    edit_intent(repo.path(), "c2");
    let c2 = commit_all(repo.path(), "c2");

    let update = PushUpdate {
        local_ref: "refs/heads/topic".into(),
        local_sha: c2.clone(),
        remote_ref: "refs/heads/topic".into(),
        remote_sha: remote,
    };
    let commits =
        unpublished_commits_for_update(repo.path(), &update, None, None).expect("enumerate");
    assert_eq!(commits, vec![c1, c2]);
}

#[test]
fn merge_commit_in_range_is_rejected() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

    git(repo.path(), &["checkout", "-b", "side"]);
    write(repo.path(), "side.txt", "side\n");
    let side = commit_all(repo.path(), "side");
    git(repo.path(), &["checkout", "-"]);
    write(repo.path(), "main.txt", "main\n");
    let _main = commit_all(repo.path(), "main");
    git(
        repo.path(),
        &["merge", "--no-ff", &side, "-m", "merge side"],
    );

    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let error = publication::publish_range(&options(repo.path(), "judge-pass"), &base, &head)
        .expect_err("merge commits must be rejected");
    let message = format!("{error:#}");
    assert!(
        message.contains("merge") || message.contains("nonlinear"),
        "unexpected error: {message}"
    );
}

#[test]
fn force_push_divergent_range_gates_only_new_commits() {
    let repo = init_seeded_repo();
    let remote = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "old-tip");
    let _old = commit_all(repo.path(), "old");

    // Simulate force-push rewrite: discard an old local tip and build a new
    // line from the remote commit.
    git(repo.path(), &["reset", "--hard", &remote]);
    edit_intent(repo.path(), "rewritten-1");
    let c1 = commit_all(repo.path(), "rewritten-1");
    edit_intent(repo.path(), "rewritten-2");
    let c2 = commit_all(repo.path(), "rewritten-2");

    let update = PushUpdate {
        local_ref: "refs/heads/main".into(),
        local_sha: c2.clone(),
        remote_ref: "refs/heads/main".into(),
        remote_sha: remote,
    };
    let commits =
        unpublished_commits_for_update(repo.path(), &update, None, None).expect("enumerate");
    assert_eq!(commits, vec![c1, c2]);
}

#[test]
fn judge_response_emitted_before_publication_block() {
    let repo = init_seeded_repo();
    let base = git(repo.path(), &["rev-parse", "HEAD"]);
    edit_intent(repo.path(), "FAIL_MIDDLE bad");
    let head = commit_all(repo.path(), "bad");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "publication",
            "--from",
            &base,
            "--to",
            &head,
            "--executable",
            fixture_root("judge-fail-on-middle").to_str().unwrap(),
        ])
        .current_dir(repo.path())
        .env(
            "LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE",
            fixture_root("judge-fail"),
        )
        .output()
        .expect("xtask publication");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""verdict":"fail""#) || stdout.contains(r#""verdict": "fail""#),
        "judge response must be emitted before block; stdout={stdout}"
    );
    assert!(!output.status.success(), "blocked publication must fail");
}
