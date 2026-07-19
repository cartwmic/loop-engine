use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;
use tempfile::TempDir;
use xtask::semantic_judge::{
    self, Disposition, FOUNDATION_PARENT_REVISION, JudgeOptions, Mode, Verdict,
};

fn fixture_judge(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/semantic-judge")
        .join(name)
}

fn real_repo_root() -> PathBuf {
    semantic_judge::default_repository_root()
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

fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "judge@test"]);
    git(dir.path(), &["config", "user.name", "Judge Test"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

fn write(repo: &Path, relative: &str, content: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(&path, content).expect("write");
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn options(repo: &Path, mode: Mode, executable: &str) -> JudgeOptions {
    let mut options = JudgeOptions::new(repo, mode);
    options.executable = Some(fixture_judge(executable));
    options.timeout_seconds = Some(5);
    options.foundation_git_root = Some(real_repo_root());
    options
}

fn foundation_seed_content() -> String {
    // Compose through the runner path by building a staged request in a repo without
    // a parent manifest and reading the rubric payload.
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);
    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-pass"))
        .expect("staged judge should run");
    outcome.request["rubrics"][0]["content"]
        .as_str()
        .expect("rubric content")
        .to_owned()
}

#[test]
fn disposition_pass_local_and_publication_allow() {
    assert_eq!(
        semantic_judge::disposition_for(Mode::Local, Verdict::Pass),
        Disposition::Allow
    );
    assert_eq!(
        semantic_judge::disposition_for(Mode::Publication, Verdict::Pass),
        Disposition::Allow
    );
}

#[test]
fn staged_pass_allows_local_commit() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-pass"))
        .expect("pass judge");
    assert_eq!(outcome.response.verdict, Verdict::Pass);
    assert_eq!(outcome.disposition, Disposition::Allow);
    assert!(outcome.warning.is_none());
    assert!(
        outcome.request["diff"]
            .as_str()
            .unwrap_or_default()
            .contains("staged")
    );
    assert!(
        !outcome.request["diff"]
            .as_str()
            .unwrap_or_default()
            .contains("unstaged")
    );
}

#[test]
fn staged_fail_blocks_local_commit() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-fail"))
        .expect("fail judge maps locally");
    assert_eq!(outcome.response.verdict, Verdict::Fail);
    assert_eq!(outcome.disposition, Disposition::Block);
}

#[test]
fn staged_indeterminate_warns_but_allows_locally() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome =
        semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-indeterminate"))
            .expect("indeterminate judge");
    assert_eq!(outcome.response.verdict, Verdict::Indeterminate);
    assert_eq!(outcome.disposition, Disposition::WarnAllow);
    assert!(
        outcome
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("indeterminate")
    );
}

#[test]
fn staged_unavailable_warns_but_allows_locally() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome =
        semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-unavailable"))
            .expect("unavailable judge");
    assert_eq!(outcome.response.verdict, Verdict::Unavailable);
    assert_eq!(outcome.disposition, Disposition::WarnAllow);
}

#[test]
fn publication_indeterminate_and_unavailable_block() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    let parent = commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "child\n");
    let candidate = commit_all(repo.path(), "child");

    let indeterminate = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-indeterminate"),
        &parent,
        &candidate,
    )
    .expect("publication indeterminate");
    assert_eq!(indeterminate.disposition, Disposition::Block);

    let unavailable = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-unavailable"),
        &parent,
        &candidate,
    )
    .expect("publication unavailable");
    assert_eq!(unavailable.disposition, Disposition::Block);
}

#[test]
fn timeout_maps_to_unavailable_without_silent_pass() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    // Exceed common pipe capacities so timeout covers a judge that never reads stdin.
    write(
        repo.path(),
        "docs/note.md",
        &format!("{}\n", "staged".repeat(65_536)),
    );
    git(repo.path(), &["add", "docs/note.md"]);

    let mut opts = options(repo.path(), Mode::Local, "judge-timeout");
    opts.timeout_seconds = Some(1);
    let outcome = semantic_judge::judge_staged(&opts).expect("timeout becomes unavailable");
    assert_eq!(outcome.response.verdict, Verdict::Unavailable);
    assert!(outcome.response.message.contains("timed out"));
    assert_ne!(outcome.response.verdict, Verdict::Pass);
    assert_eq!(outcome.disposition, Disposition::WarnAllow);
}

#[test]
fn malformed_response_maps_to_unavailable() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome =
        semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-malformed"))
            .expect("malformed becomes unavailable");
    assert_eq!(outcome.response.verdict, Verdict::Unavailable);
    assert!(outcome.response.message.contains("malformed"));
}

#[test]
fn revision_binding_mismatch_maps_to_unavailable() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    let parent = commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "child\n");
    let candidate = commit_all(repo.path(), "child");

    let outcome = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-revision-mismatch"),
        &parent,
        &candidate,
    )
    .expect("mismatch becomes unavailable");
    assert_eq!(outcome.response.verdict, Verdict::Unavailable);
    assert!(
        outcome
            .response
            .message
            .contains("revision binding mismatch")
    );
    assert_eq!(outcome.disposition, Disposition::Block);
}

#[test]
fn request_unbound_citation_maps_to_unavailable_and_blocks_publication() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    let parent = commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "child\n");
    let candidate = commit_all(repo.path(), "child");

    let outcome = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-invalid-citation"),
        &parent,
        &candidate,
    )
    .expect("invalid citation becomes unavailable");
    assert_eq!(outcome.response.verdict, Verdict::Unavailable);
    assert!(
        outcome
            .response
            .message
            .contains("unknown parent rubric_id")
    );
    assert_eq!(outcome.disposition, Disposition::Block);
}

#[test]
fn foundation_seed_fallback_when_parent_manifest_absent() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-pass"))
        .expect("fallback path");
    let rubrics = outcome.request["rubrics"].as_array().expect("rubrics");
    assert_eq!(rubrics.len(), 1);
    assert_eq!(rubrics[0]["id"], "foundation-seed");
    let digest = {
        use std::process::Command;
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
                    .write_all(rubrics[0]["content"].as_str().unwrap().as_bytes())?;
                let output = child.wait_with_output()?;
                Ok(String::from_utf8(output.stdout).unwrap())
            })
            .expect("sha256");
        output.trim().to_owned()
    };
    assert_eq!(
        digest,
        "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
    );

    let evidence = outcome.request["deterministic_evidence"]
        .as_array()
        .expect("evidence");
    let provenance = evidence
        .iter()
        .filter(|item| {
            item["command"]
                .as_str()
                .unwrap_or_default()
                .starts_with(&format!("git show {FOUNDATION_PARENT_REVISION}:"))
        })
        .count();
    assert_eq!(provenance, 4);
}

#[test]
fn parent_manifest_rubric_ignores_candidate_working_tree() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "base");

    let seed = foundation_seed_content();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
        }]
    });
    write(
        repo.path(),
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );
    write(repo.path(), "quality/rubrics/foundation-seed.v1.md", &seed);
    let parent = commit_all(repo.path(), "add parent rubrics");

    write(
        repo.path(),
        "quality/rubrics/foundation-seed.v1.md",
        "candidate-tree rubric\n",
    );
    write(repo.path(), "docs/note.md", "candidate\n");
    let candidate = commit_all(repo.path(), "change docs and dirty rubric tree");
    // Leave working tree rubric corrupted further.
    write(
        repo.path(),
        "quality/rubrics/foundation-seed.v1.md",
        "even-dirtier-working-tree\n",
    );

    let outcome = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent,
        &candidate,
    )
    .expect("manifest path");
    assert_eq!(outcome.request["rubrics"][0]["content"], seed);
    assert_ne!(
        outcome.request["rubrics"][0]["content"],
        "candidate-tree rubric\n"
    );
    let evidence = outcome.request["deterministic_evidence"]
        .as_array()
        .unwrap();
    assert!(evidence.iter().all(|item| {
        !item["command"]
            .as_str()
            .unwrap_or_default()
            .contains("compose foundation-seed")
    }));
}

#[test]
fn second_bootstrap_claim_is_rejected() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    let mut opts = options(repo.path(), Mode::Local, "judge-pass");
    opts.claim_bootstrap_exception = true;
    let error = semantic_judge::judge_staged(&opts).expect_err("second bootstrap rejected");
    assert!(error.to_string().contains("no second bootstrap"));
}

#[test]
fn manifest_without_consumed_bootstrap_is_rejected() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "base");

    let seed = foundation_seed_content();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": false,
        "no_second_bootstrap": false,
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
        }]
    });
    write(
        repo.path(),
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );
    write(repo.path(), "quality/rubrics/foundation-seed.v1.md", &seed);
    let parent = commit_all(repo.path(), "bad bootstrap flags");
    write(repo.path(), "docs/note.md", "child\n");
    let candidate = commit_all(repo.path(), "child");

    let error = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent,
        &candidate,
    )
    .expect_err("uncconsumed bootstrap flags must fail");
    assert!(error.to_string().contains("second bootstrap"));
}

#[test]
fn first_unpublished_range_judges_every_commit_fail_closed() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "foundation\n");
    let foundation = commit_all(repo.path(), "foundation-like");
    write(repo.path(), "docs/note.md", "c1\n");
    let _c1 = commit_all(repo.path(), "c1");
    write(repo.path(), "docs/note.md", "c2\n");
    let head = commit_all(repo.path(), "c2");

    let range = semantic_judge::judge_unpublished_range(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &foundation,
        &head,
    )
    .expect("range pass");
    assert_eq!(range.commits.len(), 2);
    assert!(
        range
            .commits
            .iter()
            .all(|commit| commit.outcome.response.verdict == Verdict::Pass)
    );

    let blocked = semantic_judge::judge_unpublished_range(
        &options(repo.path(), Mode::Publication, "judge-indeterminate"),
        &foundation,
        &head,
    )
    .expect("range still returns per-commit outcomes");
    assert!(
        blocked
            .commits
            .iter()
            .any(|commit| commit.outcome.disposition == Disposition::Block)
    );
}

#[test]
fn changed_rubric_applies_only_to_following_commit() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "base\n");
    commit_all(repo.path(), "base");

    let seed = foundation_seed_content();
    let manifest_v1 = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
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
    let changed_digest =
        {
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
                child.stdin.as_mut().unwrap().write_all(changed.as_bytes())?;
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
    write(repo.path(), "docs/note.md", "rubric-change-commit\n");
    let rubric_change_commit = commit_all(repo.path(), "change-rubric");

    // The rubric-changing commit is judged by parent v1 content.
    let changing = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent_with_v1,
        &rubric_change_commit,
    )
    .expect("changing commit");
    assert_eq!(changing.request["rubrics"][0]["content"], seed);
    assert!(
        !changing.request["rubrics"][0]["content"]
            .as_str()
            .unwrap()
            .contains("next-commit-only")
    );

    write(repo.path(), "docs/note.md", "following\n");
    let following = commit_all(repo.path(), "following");
    let next = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &rubric_change_commit,
        &following,
    )
    .expect("following commit");
    assert!(
        next.request["rubrics"][0]["content"]
            .as_str()
            .unwrap()
            .contains("next-commit-only")
    );
}

#[test]
fn product_runtime_has_no_judge_dependency() {
    semantic_judge::assert_product_runtime_has_no_judge_dependency(&real_repo_root())
        .expect("product crates must not depend on semantic judge");
}

#[test]
fn judge_staged_cli_syntax_is_registered() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);

    // development-policy.md canonical wrapper after T024:
    //   cargo run -p xtask -- judge --staged
    let status = xtask::run([
        "xtask",
        "judge",
        "--staged",
        "--root",
        repo.path().to_str().unwrap(),
        "--executable",
        fixture_judge("judge-pass").to_str().unwrap(),
        "--timeout-seconds",
        "5",
    ]);
    assert_eq!(status, ExitCode::SUCCESS);
}

#[test]
fn staged_request_does_not_read_unstaged_working_tree() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);
    write(repo.path(), "docs/note.md", "unstaged\n");

    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-pass"))
        .expect("staged");
    let diff = outcome.request["diff"].as_str().unwrap();
    assert!(diff.contains("staged"));
    assert!(!diff.contains("unstaged"));
    let docs = outcome.request["relevant_docs"].as_array().unwrap();
    let note = docs
        .iter()
        .find(|doc| doc["path"] == "docs/note.md")
        .expect("note doc");
    assert!(note["content"].as_str().unwrap().contains("staged"));
    assert!(!note["content"].as_str().unwrap().contains("unstaged"));
}

#[test]
fn request_mode_echoes_local_for_staged() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "parent\n");
    commit_all(repo.path(), "parent");
    write(repo.path(), "docs/note.md", "staged\n");
    git(repo.path(), &["add", "docs/note.md"]);
    let outcome = semantic_judge::judge_staged(&options(repo.path(), Mode::Local, "judge-pass"))
        .expect("staged");
    assert_eq!(outcome.request["mode"], Value::String("local".into()));
    assert_eq!(outcome.request["schema_version"], 1);
}

fn sha256_hex(bytes: &[u8]) -> String {
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
            child.stdin.as_mut().unwrap().write_all(bytes)?;
            Ok(child.wait_with_output()?.stdout)
        })
        .expect("sha256");
    String::from_utf8(output).unwrap().trim().to_owned()
}

fn focused_rubric_ids() -> [&'static str; 4] {
    [
        "documentation",
        "observability",
        "architecture",
        "behavioral-evidence",
    ]
}

fn read_workspace_rubric(relative: &str) -> String {
    fs::read_to_string(real_repo_root().join("quality/rubrics").join(relative))
        .unwrap_or_else(|error| panic!("read quality/rubrics/{relative}: {error}"))
}

fn focused_manifest_and_contents() -> (Value, Vec<(String, String, String)>) {
    let mut entries = Vec::new();
    let mut rubrics = Vec::new();
    for id in focused_rubric_ids() {
        let content_path = format!("{id}.md");
        let content = read_workspace_rubric(&content_path);
        let digest = sha256_hex(content.as_bytes());
        rubrics.push(serde_json::json!({
            "id": id,
            "version": 1,
            "parent_revision": FOUNDATION_PARENT_REVISION,
            "content_path": content_path,
            "content_sha256": digest,
        }));
        entries.push((id.to_owned(), content_path, content));
    }
    let manifest = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "focused_rubrics_effective_after_task": "T025",
        "active_rubric_set": "focused",
        "rubrics": rubrics,
    });
    (manifest, entries)
}

fn write_focused_rubrics(repo: &Path, manifest: &Value, entries: &[(String, String, String)]) {
    write(
        repo,
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(manifest).unwrap(),
    );
    for (_id, content_path, content) in entries {
        write(repo, &format!("quality/rubrics/{content_path}"), content);
    }
}

#[test]
fn workspace_focused_manifest_hashes_match_rubric_files() {
    let manifest_text = read_workspace_rubric("manifest.json");
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifest json");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["parent_revision"], FOUNDATION_PARENT_REVISION);
    assert_eq!(manifest["bootstrap_publication_consumed"], true);
    assert_eq!(manifest["no_second_bootstrap"], true);
    assert_eq!(manifest["focused_rubrics_effective_after_task"], "T025");
    assert_eq!(manifest["active_rubric_set"], "focused");

    let rubrics = manifest["rubrics"].as_array().expect("rubrics array");
    assert_eq!(rubrics.len(), 4);
    let expected_ids = focused_rubric_ids();
    for (index, expected_id) in expected_ids.iter().enumerate() {
        let entry = &rubrics[index];
        assert_eq!(entry["id"], *expected_id);
        let content_path = entry["content_path"].as_str().expect("content_path");
        assert_eq!(content_path, format!("{expected_id}.md"));
        let content = read_workspace_rubric(content_path);
        let actual = sha256_hex(content.as_bytes());
        assert_eq!(
            entry["content_sha256"].as_str().expect("content_sha256"),
            actual,
            "digest mismatch for {expected_id}"
        );
        assert!(
            content.contains("Applies to the following commit only"),
            "{expected_id} must declare following-commit-only semantics"
        );
        assert!(
            content.contains("never read from the candidate working tree"),
            "{expected_id} must forbid candidate self-judgment"
        );
    }
}

#[test]
fn focused_parent_manifest_loads_exact_hashes_and_ignores_candidate_tree() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "base\n");
    commit_all(repo.path(), "base");

    let (manifest, entries) = focused_manifest_and_contents();
    write_focused_rubrics(repo.path(), &manifest, &entries);
    let parent = commit_all(repo.path(), "add focused rubrics");

    for (_id, content_path, _content) in &entries {
        write(
            repo.path(),
            &format!("quality/rubrics/{content_path}"),
            "candidate-tree focused rubric\n",
        );
    }
    write(repo.path(), "docs/note.md", "candidate\n");
    let candidate = commit_all(repo.path(), "change docs and dirty focused rubric tree");
    write(
        repo.path(),
        "quality/rubrics/documentation.md",
        "even-dirtier-working-tree\n",
    );

    let outcome = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent,
        &candidate,
    )
    .expect("focused parent load");
    let rubrics = outcome.request["rubrics"].as_array().expect("rubrics");
    assert_eq!(rubrics.len(), 4);
    for (index, (id, _path, content)) in entries.iter().enumerate() {
        assert_eq!(rubrics[index]["id"], *id);
        assert_eq!(rubrics[index]["content"], *content);
        assert_ne!(
            rubrics[index]["content"].as_str().unwrap(),
            "candidate-tree focused rubric\n"
        );
    }
}

#[test]
fn focused_rubrics_apply_only_to_following_commit_never_candidate_self_judgment() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "base\n");
    commit_all(repo.path(), "base");

    let seed = foundation_seed_content();
    let seed_manifest = serde_json::json!({
        "schema_version": 1,
        "parent_revision": FOUNDATION_PARENT_REVISION,
        "bootstrap_publication_consumed": true,
        "no_second_bootstrap": true,
        "active_rubric_set": "foundation-seed",
        "rubrics": [{
            "id": "foundation-seed",
            "content_path": "foundation-seed.v1.md",
            "content_sha256": "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
        }]
    });
    write(
        repo.path(),
        "quality/rubrics/manifest.json",
        &serde_json::to_string_pretty(&seed_manifest).unwrap(),
    );
    write(repo.path(), "quality/rubrics/foundation-seed.v1.md", &seed);
    let parent_with_seed = commit_all(repo.path(), "seed parent");

    let (focused_manifest, focused_entries) = focused_manifest_and_contents();
    write_focused_rubrics(repo.path(), &focused_manifest, &focused_entries);
    write(repo.path(), "docs/note.md", "introduce-focused\n");
    let introducing = commit_all(repo.path(), "introduce focused rubrics");

    let changing = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent_with_seed,
        &introducing,
    )
    .expect("introducing commit judged by seed parent");
    let changing_rubrics = changing.request["rubrics"].as_array().unwrap();
    assert_eq!(changing_rubrics.len(), 1);
    assert_eq!(changing_rubrics[0]["id"], "foundation-seed");
    assert_eq!(changing_rubrics[0]["content"], seed);

    write(repo.path(), "docs/note.md", "following\n");
    let following = commit_all(repo.path(), "following");
    let next = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &introducing,
        &following,
    )
    .expect("following commit judged by focused parent");
    let next_rubrics = next.request["rubrics"].as_array().unwrap();
    assert_eq!(next_rubrics.len(), 4);
    for (index, (id, _path, content)) in focused_entries.iter().enumerate() {
        assert_eq!(next_rubrics[index]["id"], *id);
        assert_eq!(next_rubrics[index]["content"], *content);
    }
}

#[test]
fn focused_manifest_digest_mismatch_is_rejected() {
    let repo = init_repo();
    write(repo.path(), "docs/note.md", "base\n");
    commit_all(repo.path(), "base");

    let (mut manifest, entries) = focused_manifest_and_contents();
    manifest["rubrics"][0]["content_sha256"] = Value::String("0".repeat(64));
    write_focused_rubrics(repo.path(), &manifest, &entries);
    let parent = commit_all(repo.path(), "bad digest focused set");
    write(repo.path(), "docs/note.md", "child\n");
    let candidate = commit_all(repo.path(), "child");

    let error = semantic_judge::judge_revision_pair(
        &options(repo.path(), Mode::Publication, "judge-pass"),
        &parent,
        &candidate,
    )
    .expect_err("digest mismatch must fail closed");
    assert!(
        error.to_string().contains("digest mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn focused_example_decision_fixtures_cite_parent_rubric_ids() {
    let fixtures_dir = real_repo_root().join("quality/rubrics/fixtures");
    let mut saw = 0usize;
    for entry in fs::read_dir(&fixtures_dir).expect("fixtures dir") {
        let entry = entry.expect("fixture entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("fixture text");
        let value: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("invalid fixture {}: {error}", path.display()));
        assert_eq!(value["schema_version"], 1);
        assert!(value["verdict"].as_str().is_some());
        let citations = value["citations"].as_array().expect("citations");
        assert!(!citations.is_empty(), "{} needs citations", path.display());
        for citation in citations {
            let rubric_id = citation["rubric_id"].as_str().expect("rubric_id");
            assert!(
                focused_rubric_ids().contains(&rubric_id),
                "{} cites unexpected rubric_id {rubric_id}",
                path.display()
            );
            assert!(citation["rule"].as_str().is_some());
            let lines = citation["lines"].as_array().expect("lines");
            assert!(!lines.is_empty());
            for line in lines {
                let locator = line.as_str().expect("line locator");
                assert!(
                    locator.contains(':') || locator.starts_with("quality/rubrics/"),
                    "locator must cite a repository path: {locator}"
                );
            }
        }
        saw += 1;
    }
    assert!(
        saw >= 5,
        "expected cited example-decision fixtures, found {saw}"
    );
}
