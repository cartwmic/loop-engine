//! Public publication-history regressions for backlog T08.
//!
//! These tests drive the shipped checker CLI, a shallow clone, and the actual
//! update-stream gate used by publication CI. They intentionally use temporary
//! repositories and a local bare remote so no external service is part of the proof.

use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn checker() -> PathBuf {
    workspace_integration::binary("bookends-check")
}

fn git(repo: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("repo directory");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Bookends T08"]);
    git(&repo, &["config", "user.email", "t08@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    repo
}

fn write_file(repo: &Path, relative: &str, text: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("file parent");
    }
    fs::write(path, text).expect("write fixture");
}

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn enable_graph(repo: &Path, prd: &str, citation: &str) {
    write_file(
        repo,
        "bookends.toml",
        "prd = \"docs/PRD.md\"\n\n[classes.e2e_journey]\npathspecs = [\"tests/**\"]\nrequired_ci_jobs = [\"journey\"]\n",
    );
    write_file(repo, "docs/PRD.md", prd);
    write_file(repo, "tests/journey.py", citation);
    write_file(
        repo,
        ".github/workflows/ci.yml",
        "jobs:\n  journey:\n    steps:\n      - run: python3 tests/journey.py\n",
    );
}

fn live(id: &str, title: &str) -> String {
    format!("### {id}: {title}\n- Status: live\n- Coverage: e2e/journey\n")
}

fn tombstone(id: &str, title: &str) -> String {
    format!("### {id}: {title}\n- Status: tombstone\n")
}

fn run_checker(repo: &Path, updates: &str, receipt_root: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(checker());
    command
        .current_dir(repo)
        .args(["--repo", repo.to_str().expect("repo path")])
        .args(["--updates-stdin", "--receipt-root"])
        .arg(receipt_root)
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn checker");
    child
        .stdin
        .take()
        .expect("checker stdin")
        .write_all(updates.as_bytes())
        .expect("write updates");
    child.wait_with_output().expect("wait checker")
}

fn first_line(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout)
        .expect("checker stdout")
        .lines()
        .next()
        .unwrap_or("")
}

fn one_receipt(root: &Path) -> String {
    let mut files = fs::read_dir(root)
        .expect("receipt directory")
        .map(|entry| entry.expect("receipt entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        });
    let path = files.next().expect("one receipt");
    assert!(files.next().is_none(), "unexpected extra receipt");
    fs::read_to_string(path).expect("receipt text")
}

struct GateFixture {
    root: tempfile::TempDir,
    checkout: PathBuf,
    bare: PathBuf,
    base: String,
    bin_dir: PathBuf,
}

fn gate_fixture(with_obsolete_ref: bool, shallow: bool) -> GateFixture {
    let root = tempfile::tempdir().expect("tempdir");
    let source = init_repo(root.path());
    enable_graph(&source, &live("LE-1", "One"), "# bookends:LE-1\n");
    let base = commit(&source, "base");

    let bare = root.path().join("remote.git");
    let bare_output = Command::new("git")
        .args([
            "init",
            "-q",
            "--bare",
            "--initial-branch=main",
            bare.to_str().expect("bare path"),
        ])
        .output()
        .expect("init bare");
    assert!(
        bare_output.status.success(),
        "{}",
        String::from_utf8_lossy(&bare_output.stderr)
    );
    git(
        &source,
        &["remote", "add", "origin", bare.to_str().expect("bare path")],
    );
    git(
        &source,
        &["push", "-q", "origin", &format!("{base}:refs/heads/main")],
    );
    if with_obsolete_ref {
        git(
            &source,
            &[
                "push",
                "-q",
                "origin",
                &format!("{base}:refs/heads/obsolete"),
            ],
        );
    }

    let checkout = root.path().join("checkout");
    let mut clone_command = Command::new("git");
    clone_command.args(["clone", "-q"]);
    if shallow {
        clone_command.args(["--depth", "1"]);
    }
    let clone = clone_command
        .args([
            &format!("file://{}", bare.display()),
            checkout.to_str().expect("checkout path"),
        ])
        .output()
        .expect("clone checkout");
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    git(&checkout, &["config", "user.name", "Bookends T08 gate"]);
    git(
        &checkout,
        &["config", "user.email", "t08-gate@example.invalid"],
    );
    git(&checkout, &["config", "commit.gpgsign", "false"]);

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scripts = checkout.join("scripts");
    fs::create_dir_all(&scripts).expect("scripts directory");
    let gate = scripts.join("bookends-check-gate.sh");
    fs::copy(source_root.join("scripts/bookends-check-gate.sh"), &gate).expect("copy gate");
    fs::set_permissions(gate, fs::Permissions::from_mode(0o755)).expect("executable gate");

    let bin_dir = root.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin directory");
    let checker_path = bin_dir.join("bookends-check");
    fs::copy(checker(), &checker_path).expect("copy checker");
    fs::set_permissions(&checker_path, fs::Permissions::from_mode(0o755))
        .expect("checker executable");

    GateFixture {
        root,
        checkout,
        bare,
        base,
        bin_dir,
    }
}

fn run_gate(fixture: &GateFixture, updates: &str, receipt_name: &str) -> Output {
    let path = format!(
        "{}:{}",
        fixture.bin_dir.display(),
        env::var("PATH").expect("PATH")
    );
    let mut child = Command::new(fixture.checkout.join("scripts/bookends-check-gate.sh"))
        .current_dir(&fixture.checkout)
        .env("PATH", path)
        .env("BOOKENDS_UPDATE_STREAM", "1")
        .env("BOOKENDS_REMOTE", "origin")
        .env_remove("BOOKENDS_BYPASS")
        .env_remove("BOOKENDS_MAX_COMMITS")
        .env("XDG_STATE_HOME", fixture.root.path().join("state"))
        .env(
            "BOOKENDS_RECEIPT_ROOT",
            fixture.root.path().join(receipt_name),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn update-stream gate");
    child
        .stdin
        .take()
        .expect("gate stdin")
        .write_all(updates.as_bytes())
        .expect("write gate updates");
    child.wait_with_output().expect("wait gate")
}

fn assert_gate_updates(fixture: &GateFixture, receipt_name: &str, updates: &str) {
    let root = fixture.root.path().join(receipt_name);
    assert!(
        root.is_dir(),
        "update stream receipt missing: {}",
        root.display()
    );
    let receipt = one_receipt(&root);
    assert!(receipt.contains("remote: origin"), "{receipt}");
    assert!(receipt.contains("complete: true"), "{receipt}");
    let recorded = receipt
        .split_once("\nupdates:\n")
        .and_then(|(_, rest)| rest.split_once("\nranges:"))
        .expect("update stream receipt fields missing")
        .0;
    // Compare the public receipt's update section, not ranges (which repeat
    // these fields). Normalize YAML quoting, including all-zero deletion OIDs.
    let actual: Vec<(&str, &str)> = recorded
        .lines()
        .map(|line| {
            let (key, value) = line
                .trim()
                .trim_start_matches("- ")
                .split_once(": ")
                .expect("receipt update field");
            (key, value.trim_matches(['\'', '"']))
        })
        .collect();
    let expected: Vec<(&str, &str)> = updates
        .lines()
        .flat_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 4, "fixture update protocol");
            ["local_ref", "new_oid", "remote_ref", "old_oid"]
                .into_iter()
                .zip(fields)
        })
        .collect();
    assert_eq!(
        actual, expected,
        "update stream was not processed exactly: {receipt}"
    );
}

fn gate_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_gate_marker(output: &Output, marker: &str) {
    let text = gate_output(output);
    assert!(
        text.lines().any(|line| line == marker),
        "missing {marker} gate result: {text}"
    );
}

fn ref_oid(repo: &Path, reference: &str) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", reference])
        .output()
        .expect("read remote ref");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[test]
fn backlog_t08_intermediate_coverage_gap_is_note_when_tip_is_clean() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&repo, "base");

    write_file(
        &repo,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-2", "Two")),
    );
    let uncovered = commit(&repo, "intermediate adds uncovered LE-2");
    write_file(
        &repo,
        "tests/journey.py",
        "# bookends:LE-1\n# bookends:LE-2\n",
    );
    let new = commit(&repo, "tip covers LE-2");

    let receipts = root.path().join("receipts");
    let updates = format!("refs/heads/main {new} refs/heads/main {old}\n");
    let output = run_checker(&repo, &updates, &receipts, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(first_line(&output), "GREEN", "{stdout}");
    assert!(
        stdout.contains(&uncovered),
        "intermediate note missing: {stdout}"
    );
    let receipt = one_receipt(&receipts);
    assert!(receipt.contains("outcome: GREEN"), "{receipt}");
    assert!(receipt.contains("complete: true"), "{receipt}");
    // bookends:LE-133 — tip-only coverage timing: the intermediate gap is
    // retained as a visible diagnostic even though the pushed tip is clean.
    assert!(
        receipt.contains(&uncovered) && receipt.contains(&new),
        "{receipt}"
    );
}

#[test]
fn backlog_t08_intermediate_continuity_violation_still_blocks() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(
        &repo,
        &(live("LE-1", "One") + &live("LE-2", "Two")),
        "# bookends:LE-1\n# bookends:LE-2\n",
    );
    let old = commit(&repo, "base");

    write_file(&repo, "docs/PRD.md", &live("LE-1", "One"));
    let removal = commit(&repo, "intermediate silently drops LE-2");
    write_file(
        &repo,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-2", "Two")),
    );
    let new = commit(&repo, "tip restores LE-2");

    let receipts = root.path().join("receipts");
    let updates = format!("refs/heads/main {new} refs/heads/main {old}\n");
    let output = run_checker(&repo, &updates, &receipts, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(first_line(&output), "RED", "{stdout}");
    assert!(
        stdout.contains(&removal),
        "continuity finding missing: {stdout}"
    );
}

#[test]
fn backlog_t08_dirty_tip_blocks_despite_clean_intermediates() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&repo, "base");

    write_file(&repo, "tests/journey.py", "# bookends:LE-1\n# touched\n");
    let _middle = commit(&repo, "clean intermediate");
    write_file(
        &repo,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-3", "Three")),
    );
    let new = commit(&repo, "tip adds uncovered LE-3");

    let receipts = root.path().join("receipts");
    let updates = format!("refs/heads/main {new} refs/heads/main {old}\n");
    let output = run_checker(&repo, &updates, &receipts, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(first_line(&output), "RED", "{stdout}");
    assert!(
        stdout.contains(&new) && stdout.contains("LE-3"),
        "tip finding missing: {stdout}"
    );
}

#[test]
fn backlog_t08_merge_commit_checks_the_non_first_parent() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&repo, "base");

    git(&repo, &["checkout", "-q", "-b", "feature"]);
    write_file(
        &repo,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-2", "Feature")),
    );
    write_file(
        &repo,
        "tests/journey.py",
        "# bookends:LE-1\n# bookends:LE-2\n",
    );
    let feature = commit(&repo, "feature requirement");

    git(&repo, &["checkout", "-q", "main"]);
    write_file(&repo, "tests/main-side.py", "main side\n");
    commit(&repo, "main side");
    git(
        &repo,
        &["merge", "--no-ff", "-q", "feature", "-m", "merge feature"],
    );
    write_file(&repo, "docs/PRD.md", &live("LE-1", "One"));
    write_file(&repo, "tests/journey.py", "# bookends:LE-1\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "--amend", "--no-edit", "-q"]);
    let merge = git(&repo, &["rev-parse", "HEAD"]);
    let parents = git(&repo, &["rev-list", "--parents", "-n", "1", &merge]);
    assert!(
        parents.split_whitespace().count() == 3,
        "not a merge: {parents}"
    );

    let receipts = root.path().join("receipts");
    let updates = format!("refs/heads/main {merge} refs/heads/main {old}\n");
    let output = run_checker(&repo, &updates, &receipts, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_eq!(first_line(&output), "RED", "{stdout}");
    assert!(
        stdout.contains(&merge) && stdout.contains(&feature),
        "{stdout}"
    );
    assert!(
        stdout.contains("parent") && stdout.contains("LE-2"),
        "{stdout}"
    );
}

#[test]
fn backlog_t08_new_ref_allows_pre_adoption_history_and_uses_pushed_tip() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    write_file(&repo, "README.md", "history before adoption\n");
    commit(&repo, "pre-adoption");
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let adoption = commit(&repo, "first adoption");

    let receipts = root.path().join("adoption-receipts");
    let zero = "0".repeat(40);
    let updates = format!("refs/heads/main {adoption} refs/heads/main {zero}\n");
    let output = run_checker(&repo, &updates, &receipts, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(first_line(&output), "GREEN");

    git(&repo, &["checkout", "-q", "-b", "bad-checkout"]);
    write_file(&repo, "tests/journey.py", "print('missing citation')\n");
    commit(&repo, "bad checked-out branch");
    git(&repo, &["checkout", "-q", "main"]);
    write_file(&repo, "tests/extra.py", "not a citation\n");
    let pushed_tip = commit(&repo, "good pushed tip");
    git(&repo, &["checkout", "-q", "bad-checkout"]);

    let pushed_receipts = root.path().join("pushed-receipts");
    let updates = format!("refs/heads/main {pushed_tip} refs/heads/main {adoption}\n");
    let output = run_checker(&repo, &updates, &pushed_receipts, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_eq!(first_line(&output), "GREEN", "{stdout}");
}

#[test]
fn backlog_t08_multiple_ref_updates_each_get_their_own_range() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&repo, "base");

    git(&repo, &["checkout", "-q", "-b", "feature"]);
    write_file(&repo, "tests/feature.py", "feature support\n");
    let feature = commit(&repo, "feature tip");
    git(&repo, &["checkout", "-q", "main"]);
    write_file(&repo, "tests/main.py", "main support\n");
    let main = commit(&repo, "main tip");

    let receipts = root.path().join("receipts");
    let zero = "0".repeat(40);
    let updates = format!(
        "refs/heads/main {main} refs/heads/main {old}\nrefs/heads/feature {feature} refs/heads/feature {zero}\n"
    );
    let output = run_checker(&repo, &updates, &receipts, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(first_line(&output), "GREEN");
    let receipt = one_receipt(&receipts);
    assert_eq!(receipt.matches("local_ref:").count(), 4, "{receipt}");
    assert!(
        receipt.contains(&main) && receipt.contains(&feature),
        "{receipt}"
    );
}

#[test]
// bookends:LE-125 — the public checker obtains shallow parent history and agrees with the full clone without mutating local checkout identity.
fn backlog_t08_shallow_and_full_clones_agree_after_fetch_without_local_mutation() {
    let root = tempfile::tempdir().expect("tempdir");
    let source = init_repo(root.path());
    enable_graph(&source, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&source, "base");
    write_file(&source, "tests/extra.py", "extra proof support\n");
    let new = commit(&source, "new tip");

    let bare = root.path().join("remote.git");
    let bare_output = Command::new("git")
        .args([
            "init",
            "-q",
            "--bare",
            "--initial-branch=main",
            bare.to_str().expect("bare path"),
        ])
        .output()
        .expect("init bare");
    assert!(bare_output.status.success());
    git(
        &source,
        &["remote", "add", "origin", bare.to_str().expect("bare path")],
    );
    git(&source, &["push", "-q", "origin", "main:main"]);

    let shallow = root.path().join("shallow");
    let clone = Command::new("git")
        .args([
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", bare.display()),
            shallow.to_str().expect("shallow path"),
        ])
        .output()
        .expect("clone shallow");
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    git(&shallow, &["config", "user.name", "Bookends T08"]);
    git(&shallow, &["config", "user.email", "t08@example.invalid"]);
    let before_head = git(&shallow, &["rev-parse", "HEAD"]);
    let before_branch = git(&shallow, &["symbolic-ref", "--short", "HEAD"]);
    let before_refs = git(
        &shallow,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
        ],
    );
    let before_index = git(&shallow, &["ls-files", "-s"]);
    let before_status = git(
        &shallow,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );

    let full_receipts = root.path().join("full-receipts");
    let updates = format!("refs/heads/main {new} refs/heads/main {old}\n");
    let full = run_checker(&source, &updates, &full_receipts, &["--remote", "origin"]);
    assert_eq!(
        first_line(&full),
        "GREEN",
        "{}",
        String::from_utf8_lossy(&full.stdout)
    );

    let shallow_receipts = root.path().join("shallow-receipts");
    let shallow_result = run_checker(
        &shallow,
        &updates,
        &shallow_receipts,
        &["--remote", "origin"],
    );
    assert_eq!(
        first_line(&shallow_result),
        "GREEN",
        "{}",
        String::from_utf8_lossy(&shallow_result.stdout)
    );
    assert_eq!(git(&shallow, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(
        git(&shallow, &["symbolic-ref", "--short", "HEAD"]),
        before_branch
    );
    assert_eq!(
        git(
            &shallow,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads"
            ]
        ),
        before_refs
    );
    assert_eq!(git(&shallow, &["ls-files", "-s"]), before_index);
    assert_eq!(
        git(
            &shallow,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        before_status
    );
    let receipt = one_receipt(&shallow_receipts);
    assert!(
        receipt.contains(&old) && receipt.contains(&new) && receipt.contains("complete: true"),
        "{receipt}"
    );
}

#[test]
// bookends:LE-126 — unavailable history and receipt writes fail closed; explicitly attested bypass has retained receipts and never claims GREEN.
fn backlog_t08_missing_history_and_limit_are_red_and_bypass_is_recorded() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = init_repo(root.path());
    enable_graph(&repo, &live("LE-1", "One"), "# bookends:LE-1\n");
    let old = commit(&repo, "base");
    write_file(&repo, "tests/extra.py", "extra\n");
    let new = commit(&repo, "tip");
    let updates = format!("refs/heads/main {new} refs/heads/main {old}\n");

    let limited_receipts = root.path().join("limited");
    let limited = run_checker(&repo, &updates, &limited_receipts, &["--max-commits", "1"]);
    let limited_stdout = String::from_utf8_lossy(&limited.stdout);
    assert_eq!(limited.status.code(), Some(1), "{limited_stdout}");
    assert!(
        limited_stdout.contains("RED") && limited_stdout.contains("limit"),
        "{limited_stdout}"
    );
    assert!(one_receipt(&limited_receipts).contains("complete: false"));

    let blocked_receipts = root.path().join("blocked-receipts");
    fs::write(&blocked_receipts, "not a directory\n").expect("blocked receipt path");
    let blocked = run_checker(&repo, &updates, &blocked_receipts, &[]);
    let blocked_stdout = String::from_utf8_lossy(&blocked.stdout);
    assert_eq!(blocked.status.code(), Some(1), "{blocked_stdout}");
    assert!(
        blocked_stdout.contains("RED") && blocked_stdout.contains("receipt unavailable"),
        "{blocked_stdout}"
    );

    let missing_receipts = root.path().join("missing");
    let missing = run_checker(
        &repo,
        &updates,
        &missing_receipts,
        &["--remote", "no-such-remote"],
    );
    let missing_stdout = String::from_utf8_lossy(&missing.stdout);
    // The full repository needs no fetch, so this is a valid check.  Force a
    // shallow source to exercise the unavailable-remote path below.
    assert_eq!(first_line(&missing), "GREEN", "{missing_stdout}");

    let shallow = root.path().join("shallow");
    let clone = Command::new("git")
        .args([
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", repo.display()),
            shallow.to_str().expect("shallow path"),
        ])
        .output()
        .expect("shallow clone");
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    let unavailable_receipts = root.path().join("unavailable");
    let unavailable = run_checker(
        &shallow,
        &updates,
        &unavailable_receipts,
        &["--remote", "no-such-remote"],
    );
    let unavailable_stdout = String::from_utf8_lossy(&unavailable.stdout);
    assert_eq!(unavailable.status.code(), Some(1), "{unavailable_stdout}");
    assert!(
        unavailable_stdout.contains("RED") && unavailable_stdout.contains("fetch"),
        "{unavailable_stdout}"
    );
    assert!(one_receipt(&unavailable_receipts).contains("complete: false"));

    let bypass_receipts = root.path().join("bypass");
    let bypass = run_checker(
        &shallow,
        &updates,
        &bypass_receipts,
        &[
            "--remote",
            "no-such-remote",
            "--bypass",
            "history:temporary unavailable ancestry",
        ],
    );
    let bypass_stdout = String::from_utf8_lossy(&bypass.stdout);
    assert_eq!(bypass.status.code(), Some(0), "{bypass_stdout}");
    assert_eq!(first_line(&bypass), "BYPASS", "{bypass_stdout}");
    let receipts: Vec<PathBuf> = fs::read_dir(&bypass_receipts)
        .expect("bypass receipts")
        .map(|entry| entry.expect("receipt entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .collect();
    assert_eq!(receipts.len(), 2, "{receipts:?}");
    assert!(receipts.iter().any(|path| {
        fs::read_to_string(path)
            .expect("bypass receipt")
            .contains("reason: temporary unavailable ancestry")
    }));
}

#[test]
fn backlog_t08_gate_notes_intermediate_when_tip_is_clean() {
    let fixture = gate_fixture(false, true);
    write_file(
        &fixture.checkout,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-2", "Uncovered")),
    );
    let noted = commit(&fixture.checkout, "uncovered middle commit");
    write_file(
        &fixture.checkout,
        "docs/PRD.md",
        &(live("LE-1", "One") + &tombstone("LE-2", "Uncovered")),
    );
    let tip = commit(&fixture.checkout, "tip tombstones LE-2");
    let updates = format!("refs/heads/main {tip} refs/heads/main {}\n", fixture.base);
    let noted_gate = run_gate(&fixture, &updates, "noted-receipts");
    // The fixture publishes only after the public gate accepts. CI itself
    // checks an already-published range; it does not undo a remote push.
    if noted_gate.status.success() {
        git(&fixture.checkout, &["push", "origin", "main"]);
    }
    assert_gate_updates(&fixture, "noted-receipts", &updates);
    assert!(noted_gate.status.success(), "{}", gate_output(&noted_gate));
    assert_gate_marker(&noted_gate, "GREEN");
    // bookends:LE-133 — tip-only coverage timing: the intermediate gap is a
    // visible note, not a block, when the pushed tip is independently clean.
    assert!(
        gate_output(&noted_gate).contains(&noted),
        "{}",
        gate_output(&noted_gate)
    );
    assert_eq!(git(&fixture.bare, &["rev-parse", "refs/heads/main"]), tip);

    write_file(
        &fixture.checkout,
        "tests/extra.py",
        "ordinary untagged support\n",
    );
    let pushed = commit(&fixture.checkout, "valid publication");
    let updates = format!("refs/heads/main {pushed} refs/heads/main {tip}\n");
    let accepted = run_gate(&fixture, &updates, "accepted-receipts");
    if accepted.status.success() {
        git(&fixture.checkout, &["push", "origin", "main"]);
    }
    assert_gate_updates(&fixture, "accepted-receipts", &updates);
    assert!(accepted.status.success(), "{}", gate_output(&accepted));
    assert_gate_marker(&accepted, "GREEN");
    assert_eq!(
        git(&fixture.bare, &["rev-parse", "refs/heads/main"]),
        pushed
    );
}

#[test]
fn backlog_t08_gate_accepts_direct_sha_and_head_updates() {
    // bookends:LE-133 — real gate updates using a direct SHA and HEAD are
    // accepted after checking each published range and its tip content.
    let fixture = gate_fixture(false, false);

    write_file(
        &fixture.checkout,
        "tests/direct-sha.py",
        "direct SHA publication\n",
    );
    let direct_sha = commit(&fixture.checkout, "direct SHA publication");
    let updates = format!(
        "{direct_sha} {direct_sha} refs/heads/main {}\n",
        fixture.base
    );
    let direct = run_gate(&fixture, &updates, "direct-receipts");
    if direct.status.success() {
        git(
            &fixture.checkout,
            &["push", "origin", &format!("{direct_sha}:refs/heads/main")],
        );
    }
    assert_gate_updates(&fixture, "direct-receipts", &updates);
    assert!(direct.status.success(), "{}", gate_output(&direct));
    assert_gate_marker(&direct, "GREEN");
    assert_eq!(
        ref_oid(&fixture.bare, "refs/heads/main"),
        Some(direct_sha.clone())
    );

    write_file(&fixture.checkout, "tests/head.py", "HEAD publication\n");
    let head = commit(&fixture.checkout, "HEAD publication");
    let updates = format!("HEAD {head} refs/heads/main {direct_sha}\n");
    let head_gate = run_gate(&fixture, &updates, "head-receipts");
    if head_gate.status.success() {
        git(
            &fixture.checkout,
            &["push", "origin", "HEAD:refs/heads/main"],
        );
    }
    assert_gate_updates(&fixture, "head-receipts", &updates);
    assert!(head_gate.status.success(), "{}", gate_output(&head_gate));
    assert_gate_marker(&head_gate, "GREEN");
    assert_eq!(ref_oid(&fixture.bare, "refs/heads/main"), Some(head));
}

#[test]
fn backlog_t08_gate_accepts_deletion_only_and_mixed_valid_updates() {
    // bookends:LE-133 — real gate acceptance of deletion-only and mixed
    // publication/deletion updates preserves deletion semantics while checking
    // the published range and tip in the same update stream.
    let deletion_fixture = gate_fixture(true, false);
    let zero = "0".repeat(40);
    let updates = format!(
        "(delete) {zero} refs/heads/obsolete {}\n",
        deletion_fixture.base
    );
    let deletion = run_gate(&deletion_fixture, &updates, "deletion-receipts");
    if deletion.status.success() {
        git(
            &deletion_fixture.checkout,
            &["push", "origin", "--delete", "obsolete"],
        );
    }
    assert_gate_updates(&deletion_fixture, "deletion-receipts", &updates);
    assert!(deletion.status.success(), "{}", gate_output(&deletion));
    assert_gate_marker(&deletion, "GREEN");
    assert_eq!(ref_oid(&deletion_fixture.bare, "refs/heads/obsolete"), None);

    let mixed_fixture = gate_fixture(true, false);
    write_file(
        &mixed_fixture.checkout,
        "tests/mixed.py",
        "mixed publication\n",
    );
    let published = commit(&mixed_fixture.checkout, "mixed publication");
    let updates = format!(
        "{published} {published} refs/heads/main {base}\n(delete) {zero} refs/heads/obsolete {base}\n",
        base = mixed_fixture.base,
    );
    let mixed = run_gate(&mixed_fixture, &updates, "mixed-receipts");
    if mixed.status.success() {
        git(
            &mixed_fixture.checkout,
            &[
                "push",
                "origin",
                &format!("{published}:refs/heads/main"),
                ":refs/heads/obsolete",
            ],
        );
    }
    assert_gate_updates(&mixed_fixture, "mixed-receipts", &updates);
    assert!(mixed.status.success(), "{}", gate_output(&mixed));
    assert_gate_marker(&mixed, "GREEN");
    assert_eq!(
        ref_oid(&mixed_fixture.bare, "refs/heads/main"),
        Some(published)
    );
    assert_eq!(ref_oid(&mixed_fixture.bare, "refs/heads/obsolete"), None);
}

#[test]
fn backlog_t08_gate_rejects_mixed_invalid_update_without_remote_mutation() {
    // bookends:LE-133 — a mixed stream with an invalid published transition is
    // rejected before the fixture publishes or applies deletion, preserving
    // range continuity and leaving every remote ref unchanged.
    let fixture = gate_fixture(true, false);
    write_file(
        &fixture.checkout,
        "docs/PRD.md",
        &(live("LE-1", "One") + &live("LE-2", "Uncovered")),
    );
    let invalid = commit(&fixture.checkout, "invalid mixed publication");
    let zero = "0".repeat(40);
    let updates = format!(
        "{invalid} {invalid} refs/heads/main {base}\n(delete) {zero} refs/heads/obsolete {base}\n",
        base = fixture.base,
    );
    let rejected = run_gate(&fixture, &updates, "rejected-receipts");
    if rejected.status.success() {
        git(
            &fixture.checkout,
            &[
                "push",
                "origin",
                &format!("{invalid}:refs/heads/main"),
                ":refs/heads/obsolete",
            ],
        );
    }
    assert_gate_updates(&fixture, "rejected-receipts", &updates);
    let output = gate_output(&rejected);
    assert!(!rejected.status.success(), "{output}");
    assert_gate_marker(&rejected, "RED");
    assert!(output.contains("RED"), "{output}");
    assert!(output.contains(&invalid), "{output}");
    assert_eq!(
        ref_oid(&fixture.bare, "refs/heads/main"),
        Some(fixture.base.clone())
    );
    assert_eq!(
        ref_oid(&fixture.bare, "refs/heads/obsolete"),
        Some(fixture.base)
    );
}
