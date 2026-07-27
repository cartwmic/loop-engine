use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;
use xtask::candidate::{Candidate, PreparedCandidate};
use xtask::config::SemanticRequirement;
use xtask::git::Repository;

const MINIMAL_MANIFEST: &str = r#"schema_version = 2

[defaults]
timeout_seconds = 30
max_output_bytes = 4096

[runner]
inputs = ["quality/manifest.toml"]

[[checks]]
id = "test"
phases = ["pre-commit"]
scope = "repository"
program = "true"
args = []
cwd = "{candidate_root}"
"#;

fn git_output(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git_output(repo, args))
        .expect("utf-8 git output")
        .trim()
        .to_owned()
}

fn init_repo() -> TempDir {
    let repo = TempDir::new().expect("temp repo");
    git(repo.path(), &["init", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "candidate@test"]);
    git(repo.path(), &["config", "user.name", "Candidate Test"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    write(
        repo.path(),
        "quality/manifest.toml",
        MINIMAL_MANIFEST.as_bytes(),
    );
    git(repo.path(), &["add", "quality/manifest.toml"]);
    repo
}

fn write(repo: &Path, path: &str, contents: &[u8]) {
    let path = repo.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(path, contents).expect("write");
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn prepared(repo: &Path) -> PreparedCandidate {
    Candidate::staged(repo)
        .expect("candidate")
        .prepare(SemanticRequirement::Optional)
        .expect("prepared candidate")
}

#[test]
fn repository_resolves_common_dir_from_subdirectory_command_cwd_in_linked_worktree() {
    let repo = init_repo();
    write(repo.path(), "tracked", b"one\n");
    commit_all(repo.path(), "seed");
    let linked_parent = TempDir::new().expect("linked parent");
    let linked = linked_parent.path().join("linked");
    git(
        repo.path(),
        &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
    );
    fs::create_dir(linked.join("deep")).unwrap();

    let resolved = Repository::resolve(&linked.join("deep")).expect("resolve linked repository");
    assert!(resolved.git_directory().is_absolute());
    assert!(resolved.git_common_directory().is_absolute());
    assert_ne!(resolved.git_directory(), resolved.git_common_directory());
    assert_eq!(resolved.worktree_root(), linked.canonicalize().unwrap());
    assert_eq!(
        resolved.git_common_directory(),
        repo.path().join(".git").canonicalize().unwrap()
    );
}

#[test]
fn unborn_staged_candidate_uses_empty_tree_base_and_index_tree() {
    let repo = init_repo();
    write(repo.path(), "first.txt", b"first\n");
    git(repo.path(), &["add", "first.txt"]);

    let repository = Repository::resolve(repo.path()).expect("resolve repository");
    assert_eq!(repository.head().expect("resolve unborn HEAD"), None);

    let candidate = prepared(repo.path());
    assert_eq!(candidate.candidate_revision(), candidate.candidate_tree());
    assert_ne!(candidate.base_revision(), candidate.candidate_tree());
    assert_eq!(
        candidate.changed_paths(),
        &[
            PathBuf::from("first.txt"),
            PathBuf::from("quality/manifest.toml")
        ]
    );
    assert_eq!(
        fs::read(candidate.source_root().join("first.txt")).unwrap(),
        b"first\n"
    );
    candidate.verify_unchanged().expect("initial tree identity");
}

#[test]
fn dangling_symbolic_head_outside_local_branches_fails_closed() {
    let repo = init_repo();
    write(repo.path(), ".git/HEAD", b"ref: refs/tags/missing\n");

    let repository = Repository::resolve(repo.path()).expect("resolve repository");
    let error = repository.head().expect_err("dangling HEAD must fail");
    assert!(
        format!("{error:#}").contains("is not a local branch"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn corrupt_symbolic_head_fails_closed() {
    let repo = init_repo();
    write(repo.path(), ".git/HEAD", b"ref: refs/heads/bad..name\n");

    let repository = Repository::resolve(repo.path()).expect("resolve repository");
    repository.head().expect_err("corrupt HEAD must fail");
}

#[test]
fn dangling_symbolic_branch_ref_fails_closed() {
    let repo = init_repo();
    write(
        repo.path(),
        ".git/refs/heads/main",
        b"ref: refs/heads/missing\n",
    );

    let repository = Repository::resolve(repo.path()).expect("resolve repository");
    let error = repository
        .head()
        .expect_err("dangling symbolic branch ref must fail");
    assert!(
        format!("{error:#}").contains("exists but does not resolve"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn branch_ref_with_missing_or_corrupt_object_fails_closed() {
    for contents in [
        b"1111111111111111111111111111111111111111\n".as_slice(),
        b"not-an-object-id\n".as_slice(),
    ] {
        let repo = init_repo();
        write(repo.path(), ".git/refs/heads/main", contents);

        let repository = Repository::resolve(repo.path()).expect("resolve repository");
        repository
            .head()
            .expect_err("invalid branch object must fail");
    }
}

#[test]
fn branch_ref_with_missing_or_corrupt_commit_object_fails_closed() {
    for corrupt in [false, true] {
        let repo = init_repo();
        let head = commit_all(repo.path(), "seed");
        let object = repo
            .path()
            .join(".git/objects")
            .join(&head[..2])
            .join(&head[2..]);
        if corrupt {
            fs::set_permissions(&object, fs::Permissions::from_mode(0o600))
                .expect("make commit object writable");
            fs::write(&object, b"corrupt object").expect("corrupt commit object");
        } else {
            fs::remove_file(&object).expect("remove commit object");
        }

        let repository = Repository::resolve(repo.path()).expect("resolve repository");
        repository
            .head()
            .expect_err("missing or corrupt commit object must fail");
    }
}

#[test]
fn raw_blob_materialization_ignores_eol_and_never_invokes_smudge_filter() {
    let repo = init_repo();
    let marker = repo.path().join("smudge-ran");
    let filter = repo.path().join("smudge-filter");
    fs::write(&filter, b"#!/bin/sh\nprintf invoked > \"$1\"\ncat\n").unwrap();
    fs::set_permissions(&filter, fs::Permissions::from_mode(0o755)).unwrap();
    git(repo.path(), &["config", "core.autocrlf", "true"]);
    git(repo.path(), &["config", "filter.side.clean", "cat"]);
    git(
        repo.path(),
        &[
            "config",
            "filter.side.smudge",
            &format!("{} {}", filter.display(), marker.display()),
        ],
    );
    write(
        repo.path(),
        ".gitattributes",
        b"*.txt filter=side text eol=crlf\n",
    );
    write(repo.path(), "eol.txt", b"line-one\r\nline-two\r\n");
    git(repo.path(), &["add", ".gitattributes", "eol.txt"]);
    let exact_blob = git_output(repo.path(), &["show", ":eol.txt"]);

    let candidate = prepared(repo.path());
    assert_eq!(
        fs::read(candidate.source_root().join("eol.txt")).unwrap(),
        exact_blob
    );
    assert!(!marker.exists(), "smudge filter must never execute");
}

#[test]
fn staged_candidate_excludes_unstaged_and_untracked_content() {
    let repo = init_repo();
    write(repo.path(), "tracked.txt", b"base\n");
    commit_all(repo.path(), "seed");
    write(repo.path(), "tracked.txt", b"staged\n");
    git(repo.path(), &["add", "tracked.txt"]);
    write(repo.path(), "tracked.txt", b"unstaged\n");
    write(repo.path(), "untracked.txt", b"untracked\n");

    let candidate = prepared(repo.path());
    assert_eq!(
        fs::read(candidate.source_root().join("tracked.txt")).unwrap(),
        b"staged\n"
    );
    assert!(!candidate.source_root().join("untracked.txt").exists());
}

#[test]
fn preserves_deletion_rename_mode_and_raw_safe_symlink() {
    let repo = init_repo();
    write(repo.path(), "delete.txt", b"delete\n");
    write(repo.path(), "rename-old.txt", b"rename\n");
    write(repo.path(), "script", b"#!/bin/sh\n");
    write(repo.path(), "dir/target", b"target\n");
    commit_all(repo.path(), "seed");

    fs::remove_file(repo.path().join("delete.txt")).unwrap();
    fs::rename(
        repo.path().join("rename-old.txt"),
        repo.path().join("rename-new.txt"),
    )
    .unwrap();
    fs::set_permissions(
        repo.path().join("script"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("target", repo.path().join("dir/link")).unwrap();
    git(repo.path(), &["add", "-A"]);

    let candidate = prepared(repo.path());
    assert!(!candidate.source_root().join("delete.txt").exists());
    assert_eq!(
        fs::read(candidate.source_root().join("rename-new.txt")).unwrap(),
        b"rename\n"
    );
    assert_ne!(
        fs::symlink_metadata(candidate.source_root().join("script"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        fs::read_link(candidate.source_root().join("dir/link")).unwrap(),
        PathBuf::from("target")
    );
}

#[test]
fn non_utf8_symlink_target_bytes_survive_but_escaping_links_fail() {
    let repo = init_repo();
    let target = OsString::from_vec(vec![b't', 0xff]);
    symlink(&target, repo.path().join("raw-link")).unwrap();
    git(repo.path(), &["add", "raw-link"]);
    let candidate = prepared(repo.path());
    assert_eq!(
        fs::read_link(candidate.source_root().join("raw-link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        target.as_bytes()
    );

    for bad in ["../../outside", "/tmp/outside"] {
        let repo = init_repo();
        fs::create_dir_all(repo.path().join("dir")).unwrap();
        symlink(bad, repo.path().join("dir/link")).unwrap();
        git(repo.path(), &["add", "dir/link"]);
        let error = Candidate::staged(repo.path()).expect_err("escaping link must fail");
        assert!(error.to_string().contains("symlink"), "{error:#}");
    }
}

#[test]
fn unsupported_submodule_mode_is_rejected_before_materialization() {
    let repo = init_repo();
    write(repo.path(), "seed", b"seed\n");
    let commit = commit_all(repo.path(), "seed");
    git(
        repo.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},submodule"),
        ],
    );
    let error = Candidate::staged(repo.path()).expect_err("gitlink must fail");
    assert!(
        error.to_string().contains("unsupported Git mode/type"),
        "{error:#}"
    );
}

#[test]
fn revision_requires_supplied_object_itself_to_be_commit_and_is_option_safe() {
    let repo = init_repo();
    write(repo.path(), "value", b"base\n");
    let commit = commit_all(repo.path(), "base");
    git(
        repo.path(),
        &["tag", "-a", "annotated", "-m", "tag", &commit],
    );

    let tagged = Candidate::revision(repo.path(), None, OsStr::new("annotated"))
        .expect_err("annotated tag must not peel");
    assert!(tagged.to_string().contains("tag object"), "{tagged:#}");
    assert!(Candidate::revision(repo.path(), None, OsStr::new("--help")).is_err());
}

#[test]
fn revision_candidate_materializes_requested_commit_not_worktree() {
    let repo = init_repo();
    write(repo.path(), "value", b"base\n");
    let base = commit_all(repo.path(), "base");
    write(repo.path(), "value", b"candidate\n");
    let revision = commit_all(repo.path(), "candidate");
    write(repo.path(), "value", b"worktree\n");

    let candidate =
        Candidate::revision(repo.path(), Some(OsStr::new(&base)), OsStr::new(&revision))
            .unwrap()
            .prepare(SemanticRequirement::Optional)
            .unwrap();
    assert_eq!(candidate.candidate_revision(), revision);
    assert_eq!(
        fs::read(candidate.source_root().join("value")).unwrap(),
        b"candidate\n"
    );
}

#[test]
fn revision_preparation_rejects_candidate_commit_off_checkout_head() {
    let repo = init_repo();
    write(repo.path(), "value", b"base\n");
    let base = commit_all(repo.path(), "base");
    write(repo.path(), "value", b"candidate\n");
    let candidate_revision = commit_all(repo.path(), "candidate");
    write(repo.path(), "value", b"later\n");
    commit_all(repo.path(), "later");

    let error = Candidate::revision(
        repo.path(),
        Some(OsStr::new(&base)),
        OsStr::new(&candidate_revision),
    )
    .unwrap()
    .prepare(SemanticRequirement::Optional)
    .expect_err("off-HEAD revision must fail before runner parity");
    assert!(
        error.to_string().contains("not current checkout HEAD"),
        "{error:#}"
    );
}

#[test]
fn unusual_utf8_paths_survive_and_report_visible_non_utf8_paths_fail_closed() {
    let repo = init_repo();
    write(repo.path(), "space and\nnewline-雪", b"odd\n");
    git(repo.path(), &["add", "-A"]);
    let candidate = prepared(repo.path());
    assert!(
        candidate
            .changed_paths()
            .contains(&PathBuf::from("space and\nnewline-雪"))
    );
    drop(candidate);

    let raw = OsString::from_vec(vec![b'b', b'a', b'd', 0xff]);
    let mut hash = Command::new("/usr/bin/git")
        .args(["hash-object", "-w", "--stdin"])
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hash.stdin.take().unwrap().write_all(b"bad\n").unwrap();
    let hash = hash.wait_with_output().unwrap();
    let mut record =
        format!("100644 {}\t", String::from_utf8_lossy(&hash.stdout).trim()).into_bytes();
    record.extend_from_slice(raw.as_bytes());
    record.push(0);
    let mut update = Command::new("/usr/bin/git")
        .args(["update-index", "-z", "--index-info"])
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    update.stdin.take().unwrap().write_all(&record).unwrap();
    assert!(update.wait().unwrap().success());
    let error = Candidate::staged(repo.path()).expect_err("non-utf-8 report path must fail");
    assert!(error.to_string().contains("UTF-8"), "{error:#}");
}

#[test]
fn preparation_loads_manifest_and_enforces_runner_parity_including_empty_directory() {
    let repo = init_repo();
    let manifest = MINIMAL_MANIFEST.replace(
        "inputs = [\"quality/manifest.toml\"]",
        "inputs = [\"quality/manifest.toml\", \"runner\"]",
    );
    write(repo.path(), "quality/manifest.toml", manifest.as_bytes());
    write(repo.path(), "runner/tool", b"candidate\n");
    git(
        repo.path(),
        &["add", "quality/manifest.toml", "runner/tool"],
    );

    let candidate = Candidate::staged(repo.path()).unwrap();
    fs::create_dir(repo.path().join("runner/empty-untracked")).unwrap();
    let error = candidate
        .prepare(SemanticRequirement::Optional)
        .expect_err("empty untracked directory changes runner namespace");
    assert!(error.to_string().contains("runner input"), "{error:#}");

    fs::remove_dir(repo.path().join("runner/empty-untracked")).unwrap();
    let candidate = Candidate::staged(repo.path())
        .unwrap()
        .prepare(SemanticRequirement::Optional)
        .unwrap();
    assert_eq!(candidate.manifest().manifest().schema_version(), 2);
}

#[test]
fn runner_parity_rejects_unstaged_symlink_ancestor_redirect() {
    let repo = init_repo();
    let manifest = MINIMAL_MANIFEST.replace(
        "inputs = [\"quality/manifest.toml\"]",
        "inputs = [\"quality/manifest.toml\", \"runner/tool\"]",
    );
    write(repo.path(), "quality/manifest.toml", manifest.as_bytes());
    write(repo.path(), "runner/tool", b"candidate\n");
    git(
        repo.path(),
        &["add", "quality/manifest.toml", "runner/tool"],
    );
    let candidate = Candidate::staged(repo.path()).unwrap();

    let external = TempDir::new().unwrap();
    write(external.path(), "tool", b"candidate\n");
    fs::remove_dir_all(repo.path().join("runner")).unwrap();
    symlink(external.path(), repo.path().join("runner")).unwrap();

    let error = candidate
        .prepare(SemanticRequirement::Optional)
        .expect_err("worktree symlink ancestor must not redirect parity walk");
    assert!(error.to_string().contains("symlink ancestor"), "{error:#}");
}

#[test]
fn preparation_honors_semantic_requirement() {
    let repo = init_repo();
    let error = Candidate::staged(repo.path())
        .unwrap()
        .prepare(SemanticRequirement::Required)
        .expect_err("required semantic policy must be present");
    assert!(format!("{error:#}").contains("semantic"), "{error:#}");

    let semantic = format!(
        "{MINIMAL_MANIFEST}\n[semantic]\nprogram = \"judge\"\nargs = []\ncwd = \"{{candidate_root}}\"\nresponse_schema = \"quality/response.json\"\n\n[[semantic.axes]]\nid = \"axis\"\nrubric = \"quality/axis.md\"\n\n[semantic.coherence]\nid = \"coherence\"\nrubric = \"quality/coherence.md\"\n"
    );
    write(repo.path(), "quality/manifest.toml", semantic.as_bytes());
    git(repo.path(), &["add", "quality/manifest.toml"]);
    let prepared = Candidate::staged(repo.path())
        .unwrap()
        .prepare(SemanticRequirement::Required)
        .expect("required semantic candidate");
    assert!(prepared.manifest().manifest().semantic().is_some());
}

#[test]
fn empty_tree_and_candidate_ids_follow_repository_hash_algorithm() {
    let repo = TempDir::new().unwrap();
    let output = Command::new("/usr/bin/git")
        .args(["init", "--object-format=sha256", "-b", "main"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    if !output.status.success() {
        eprintln!("SHA-256 repository unsupported by installed Git; skipping");
        return;
    }
    write(
        repo.path(),
        "quality/manifest.toml",
        MINIMAL_MANIFEST.as_bytes(),
    );
    git(repo.path(), &["add", "quality/manifest.toml"]);
    let candidate = prepared(repo.path());
    assert_eq!(candidate.base_revision().len(), 64);
    assert_eq!(candidate.candidate_tree().len(), 64);
}

#[test]
fn source_is_sealed_and_auxiliary_roots_are_distinct_writable_external_paths() {
    let repo = init_repo();
    write(repo.path(), "file", b"content\n");
    git(repo.path(), &["add", "file"]);
    let candidate = prepared(repo.path());

    assert_eq!(
        fs::symlink_metadata(candidate.storage_root())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::symlink_metadata(candidate.source_root())
            .unwrap()
            .permissions()
            .mode()
            & 0o222,
        0
    );
    let write_result = fs::write(candidate.source_root().join("file"), b"mutation");
    if write_result.is_ok() {
        assert_eq!(
            git(repo.path(), &["rev-parse", "--is-inside-work-tree"]),
            "true"
        );
        assert_eq!(
            String::from_utf8(
                Command::new("/usr/bin/id")
                    .arg("-u")
                    .output()
                    .unwrap()
                    .stdout
            )
            .unwrap()
            .trim(),
            "0",
            "only root may bypass sealed source mode"
        );
    }
    let roots = [
        candidate.scratch_root(),
        candidate.cache_root(),
        candidate.target_root(),
    ];
    for (index, root) in roots.iter().enumerate() {
        assert!(!root.starts_with(candidate.source_root()));
        fs::write(root.join(format!("writable-{index}")), b"ok").unwrap();
    }
    assert_ne!(roots[0], roots[1]);
    assert_ne!(roots[1], roots[2]);
    assert_ne!(roots[0], roots[2]);
}

#[test]
fn verify_unchanged_detects_writable_file_and_directory_without_tree_mode_change() {
    let repo = init_repo();
    write(repo.path(), "dir/file", b"content\n");
    git(repo.path(), &["add", "dir/file"]);
    let candidate = prepared(repo.path());

    let file = candidate.source_root().join("dir/file");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    let error = candidate.verify_unchanged().expect_err("writable file");
    assert!(error.to_string().contains("writable"), "{error:#}");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();

    let directory = candidate.source_root().join("dir");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let error = candidate
        .verify_unchanged()
        .expect_err("writable directory");
    assert!(error.to_string().contains("writable"), "{error:#}");
}

#[test]
fn verify_unchanged_detects_content_exec_mode_and_namespace_changes() {
    let repo = init_repo();
    write(repo.path(), "file", b"content\n");
    git(repo.path(), &["add", "file"]);
    let candidate = prepared(repo.path());
    let file = candidate.source_root().join("file");

    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&file, b"changed\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(candidate.verify_unchanged().is_err());

    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&file, b"content\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o555)).unwrap();
    assert!(candidate.verify_unchanged().is_err());

    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(candidate.source_root(), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(candidate.source_root().join("added"), b"added\n").unwrap();
    fs::set_permissions(
        candidate.source_root().join("added"),
        fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    fs::set_permissions(candidate.source_root(), fs::Permissions::from_mode(0o555)).unwrap();
    assert!(candidate.verify_unchanged().is_err());
}

#[test]
fn held_real_index_lock_does_not_block_or_mutate_candidate_snapshot() {
    let repo = init_repo();
    write(repo.path(), "file", b"content\n");
    git(repo.path(), &["add", "file"]);
    let index_before = fs::read(repo.path().join(".git/index")).unwrap();
    fs::write(repo.path().join(".git/index.lock"), b"held").unwrap();

    let candidate = prepared(repo.path());
    assert_eq!(
        fs::read(candidate.source_root().join("file")).unwrap(),
        b"content\n"
    );
    assert_eq!(
        fs::read(repo.path().join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(repo.path().join(".git/index.lock")).unwrap(),
        b"held"
    );
}

#[test]
fn alternate_partial_index_is_authoritative() {
    if std::env::var_os("LOOP_ENGINE_ALT_INDEX_CHILD").is_some() {
        let repo = PathBuf::from(std::env::var_os("LOOP_ENGINE_TEST_REPO").unwrap());
        let output = PathBuf::from(std::env::var_os("LOOP_ENGINE_TEST_OUTPUT").unwrap());
        let candidate = prepared(&repo);
        fs::write(
            output,
            fs::read(candidate.source_root().join("alternate-only")).unwrap(),
        )
        .unwrap();
        assert!(!candidate.source_root().join("real-only").exists());
        return;
    }

    let repo = init_repo();
    write(repo.path(), "base", b"base\n");
    commit_all(repo.path(), "base");
    write(repo.path(), "real-only", b"real\n");
    git(repo.path(), &["add", "real-only"]);
    write(repo.path(), "alternate-only", b"alternate\n");
    let alternate = repo.path().join("alternate.index");
    let status = Command::new("/usr/bin/git")
        .args(["read-tree", "--empty"])
        .env("GIT_INDEX_FILE", &alternate)
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("/usr/bin/git")
        .args(["add", "quality/manifest.toml", "alternate-only"])
        .env("GIT_INDEX_FILE", &alternate)
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert!(status.success());
    let output = repo.path().join("child-output");
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "alternate_partial_index_is_authoritative",
            "--nocapture",
        ])
        .env("LOOP_ENGINE_ALT_INDEX_CHILD", "1")
        .env("LOOP_ENGINE_TEST_REPO", repo.path())
        .env("LOOP_ENGINE_TEST_OUTPUT", &output)
        .env("GIT_INDEX_FILE", &alternate)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(output).unwrap(), b"alternate\n");
}

#[test]
fn git_control_and_config_environment_contamination_is_scrubbed() {
    if std::env::var_os("LOOP_ENGINE_CONTAMINATION_CHILD").is_some() {
        let repo = PathBuf::from(std::env::var_os("LOOP_ENGINE_TEST_REPO").unwrap());
        let output = PathBuf::from(std::env::var_os("LOOP_ENGINE_TEST_OUTPUT").unwrap());
        let candidate = prepared(&repo);
        fs::write(
            output,
            candidate
                .repository()
                .git_directory()
                .as_os_str()
                .as_bytes(),
        )
        .unwrap();
        return;
    }
    let repo = init_repo();
    write(repo.path(), "file", b"content\n");
    commit_all(repo.path(), "seed");
    let output = repo.path().join("child-output");
    let poison = repo.path().join("poison");
    fs::create_dir(&poison).unwrap();
    let global = repo.path().join("poison.gitconfig");
    fs::write(&global, b"[core]\n\tbare = true\n").unwrap();
    let marker = repo.path().join("fsmonitor-marker");
    let fsmonitor = repo.path().join("fsmonitor-hook");
    fs::write(
        &fsmonitor,
        format!("#!/bin/sh\nprintf invoked > '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&fsmonitor, fs::Permissions::from_mode(0o755)).unwrap();
    let config_parameters = format!("'core.fsmonitor={}'", fsmonitor.display());
    assert!(
        Command::new("/usr/bin/git")
            .args(["status", "--porcelain"])
            .current_dir(repo.path())
            .env("GIT_CONFIG_PARAMETERS", &config_parameters)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(marker.exists(), "fsmonitor injection control did not run");
    fs::remove_file(&marker).unwrap();

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "git_control_and_config_environment_contamination_is_scrubbed",
            "--nocapture",
        ])
        .env("LOOP_ENGINE_CONTAMINATION_CHILD", "1")
        .env("LOOP_ENGINE_TEST_REPO", repo.path())
        .env("LOOP_ENGINE_TEST_OUTPUT", &output)
        .env("GIT_DIR", &poison)
        .env("GIT_WORK_TREE", &poison)
        .env("GIT_COMMON_DIR", &poison)
        .env("GIT_OBJECT_DIRECTORY", &poison)
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &poison)
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_PARAMETERS", &config_parameters)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.bare")
        .env("GIT_CONFIG_VALUE_0", "true")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        PathBuf::from(OsString::from_vec(fs::read(output).unwrap())),
        repo.path().join(".git").canonicalize().unwrap()
    );
    assert!(!marker.exists(), "scrubbed Git invoked injected fsmonitor");
}

#[test]
fn explicit_cleanup_consumes_candidate_and_does_not_follow_auxiliary_symlink() {
    let repo = init_repo();
    let candidate = prepared(repo.path());
    let root = candidate.storage_root().to_owned();
    let external = TempDir::new().unwrap();
    fs::write(external.path().join("marker"), b"keep").unwrap();
    symlink(external.path(), candidate.scratch_root().join("external")).unwrap();

    candidate.cleanup().expect("explicit cleanup");
    assert!(!root.exists());
    assert_eq!(fs::read(external.path().join("marker")).unwrap(), b"keep");
}

#[test]
fn failed_consuming_cleanup_retains_owner_and_root_for_explicit_retry() {
    let repo = init_repo();
    let candidate = prepared(repo.path());
    let root = candidate.storage_root().to_owned();
    let backup = root.with_extension("backup");
    fs::rename(&root, &backup).unwrap();
    fs::write(&root, b"not a directory").unwrap();

    let failure = candidate.cleanup().expect_err("cleanup must fail");
    assert_eq!(failure.storage_root(), root);
    fs::remove_file(&root).unwrap();
    fs::rename(&backup, &root).unwrap();
    failure.retry().expect("cleanup retry");
    assert!(!root.exists());
}

#[test]
fn controlled_unwind_drops_candidate_and_removes_root() {
    let repo = init_repo();
    let mut root = None;
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let candidate = prepared(repo.path());
        root = Some(candidate.storage_root().to_owned());
        panic!("controlled interruption");
    }));
    assert!(unwind.is_err());
    let root = root.expect("candidate root recorded before unwind");
    assert!(!root.exists(), "RAII cleanup left {}", root.display());
}
