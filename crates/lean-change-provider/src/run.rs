//! lean-change verify: run each approved check on the current tree and, in an old-vs-new profile, on the old tree.
use crate::packet::git_bytes;
use lean_change_provider::model::*;
use loop_integrations::proof_metadata::repository_proof_identity;
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TAIL_BYTES: usize = 2000;
/// Room for the lines kgate adds around a check's output, so that dropping them leaves the check's last TAIL_BYTES.
const KGATE_LINES_ALLOWANCE: usize = 4096;
/// How long a timed-out kgate client may take to delete its pod after SIGTERM before it is killed.
const KGATE_TERM_GRACE: Duration = Duration::from_secs(60);

/// Where a criterion's check runs.
#[derive(Clone, Debug)]
pub enum Runner {
    /// A child of this process, in the repository (and in the old tree).
    Host,
    /// `<program> --profile <profile> -- <check argv>`, from the repository or the old tree: kgate ships that tree
    /// to a pod and runs the check there. The profile is the repository's own, for the old tree as well.
    Kgate { program: String, profile: PathBuf },
}

impl Runner {
    /// The runner for `lean-change verify --runner kgate`: the `kgate` on PATH with the repository's profile file.
    /// `profile` is relative to the repository root and may not leave it.
    pub fn kgate(repo: &Path, profile: &str) -> Result<Self, String> {
        let relative = Path::new(profile);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(format!(
                "--kgate-profile {profile} must be a path inside the repository"
            ));
        }
        let profile = repo.join(relative);
        if !profile.is_file() {
            return Err(format!(
                "--runner kgate needs {} (the repository's kgate profile)",
                profile.display()
            ));
        }
        Ok(Self::Kgate {
            program: "kgate".into(),
            profile,
        })
    }

    fn is_kgate(&self) -> bool {
        matches!(self, Self::Kgate { .. })
    }
}

/// kgate's own exit codes, for a job that did not run the check to its end: 64 bad command line, 69 could not
/// start or finish on the cluster, 75 pod unschedulable, 130 and 143 interrupted or cancelled; no code at all is a
/// signal. A check that exits with one of these itself is read the same way. That is the safe side: the run is not
/// recorded, so it is neither a pass nor a failure, and the driver runs verify again.
fn kgate_did_not_run(exit_code: Option<i32>, timed_out: bool) -> bool {
    !timed_out && matches!(exit_code, None | Some(64 | 69 | 75 | 130 | 143))
}

/// Drops the lines kgate prints around the check (`kgate: ...` and `kgate[<job>]: ...`): they carry job ids and
/// timestamps, which differ from run to run, and `old: same` compares output exactly.
fn without_kgate_lines(tail: &[u8]) -> Vec<u8> {
    tail.split_inclusive(|byte| *byte == b'\n')
        .filter(|line| !line.starts_with(b"kgate: ") && !line.starts_with(b"kgate["))
        .flatten()
        .copied()
        .collect()
}

/// The root of the Git worktree that contains `dir`, canonicalized.
pub fn worktree_root(dir: &Path) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{} is not inside a Git worktree: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let top = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    std::fs::canonicalize(&top).map_err(|e| format!("{top}: {e}"))
}

/// The adapter behind the core's TreeIdentity port. It measures the whole worktree, so the
/// frozen repo must be the worktree root: a subdirectory would miss edits outside it.
pub fn tree_identity(input: &InitialInput) -> Result<String, String> {
    let given =
        std::fs::canonicalize(&input.repo).map_err(|e| format!("repo {}: {e}", input.repo))?;
    if worktree_root(&given)? != given {
        return Err(format!(
            "repo {} is not the root of its Git worktree; set the run up with the worktree root",
            input.repo
        ));
    }
    repository_proof_identity(&given)
        .map_err(|e| format!("could not identify the repository tree: {e}"))
}

/// The show envelope the driver pipes in: input, context and the latest intent record.
pub fn from_show(show: &Value) -> Result<(InitialInput, String, Intent), String> {
    if show["operation"] != "show"
        || show["status"] != "completed"
        || show["result"]["workflow_id"] != WORKFLOW_ID
    {
        return Err(format!(
            "requires a completed {WORKFLOW_ID} full show envelope"
        ));
    }
    let result = &show["result"];
    let input = InitialInput::parse(&result["initial_input"])?;
    let record = result["context"]
        .as_array()
        .ok_or("show envelope has no context; use --view full")?
        .iter()
        .filter(|r| r["kind"] == "intent")
        .max_by_key(|r| r["sequence"].as_u64())
        .ok_or("no intent record yet")?;
    let id = record["id"]
        .as_str()
        .ok_or("intent record has no id")?
        .to_owned();
    let intent = Intent::parse(&record["data"])?;
    intent.check_old(input.old_vs_new.is_some())?;
    Ok((input, id, intent))
}

/// Drain a stream, keeping only its last `keep` bytes.
fn drain_tail(mut stream: impl Read + Send + 'static, keep: usize) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(n) = stream.read(&mut buffer) {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buffer[..n]);
            if tail.len() > keep {
                tail.drain(..tail.len() - keep);
            }
        }
        tail
    })
}

/// The check runs in its own process group so that its shell children die with it.
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn run_check(
    repo: &Path,
    check: &Check,
    runner: &Runner,
) -> Result<(Option<i32>, bool, String), String> {
    let (program, args): (&str, Vec<&str>) = match runner {
        Runner::Host => (
            &check.argv[0],
            check.argv[1..].iter().map(String::as_str).collect(),
        ),
        Runner::Kgate { program, profile } => (
            program,
            [
                "--profile",
                profile.to_str().ok_or("kgate profile path is not UTF-8")?,
                "--",
            ]
            .into_iter()
            .chain(check.argv.iter().map(String::as_str))
            .collect(),
        ),
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(repo)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    let pid = child.id();
    let keep = TAIL_BYTES
        + if runner.is_kgate() {
            KGATE_LINES_ALLOWANCE
        } else {
            0
        };
    let out = drain_tail(child.stdout.take().ok_or("no stdout")?, keep);
    let err = drain_tail(child.stderr.take().ok_or("no stderr")?, keep);
    let started = Instant::now();
    let limit = Duration::from_millis(check.timeout_ms);
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if started.elapsed() >= limit => {
                if runner.is_kgate() {
                    // SIGKILL would leave the pod running; SIGTERM makes the kgate client delete it first.
                    stop_kgate_client(&mut child);
                }
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                kill_group(pid);
                return Err(e.to_string());
            }
        }
    };
    // Nothing the check started may outlive it or edit the tree after it is measured.
    kill_group(pid);
    let mut tail = out.join().map_err(|_| "output reader failed")?;
    tail.extend(err.join().map_err(|_| "output reader failed")?);
    if runner.is_kgate() {
        tail = without_kgate_lines(&tail);
    }
    let from = tail.len().saturating_sub(TAIL_BYTES);
    Ok((
        code,
        timed_out,
        String::from_utf8_lossy(&tail[from..]).into_owned(),
    ))
}

/// SIGTERM to the kgate client alone, then up to KGATE_TERM_GRACE for it to delete its pod and exit.
fn stop_kgate_client(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-s", "TERM", "--", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let waited = Instant::now();
    while waited.elapsed() < KGATE_TERM_GRACE {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The old tree: a checkout of the base commit outside the repository, with the current proof files
/// laid over it. Removed when dropped.
struct OldTree {
    root: PathBuf,
}

impl OldTree {
    /// `shared` borrows the repository's objects, which is cheap here but leaves a tree whose git history cannot be
    /// read anywhere else. A tree that kgate ships to a pod needs its own objects.
    fn build(input: &InitialInput, proof_paths: &[String], shared: bool) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let repo = Path::new(&input.repo);
        let root = std::env::temp_dir().join(format!(
            "lean-change-old-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Owned before the clone runs, so that a clone that fails part way is removed as well.
        let tree = Self { root };
        let root_arg = tree
            .root
            .to_str()
            .ok_or("the temporary directory is not UTF-8")?;
        let mut clone = vec!["clone", "--no-checkout", "--quiet"];
        if shared {
            clone.push("--shared");
        }
        clone.extend([input.repo.as_str(), root_arg]);
        git_bytes(repo, &clone, None, None)?;
        git_bytes(
            &tree.root,
            &["checkout", "--quiet", "--detach", input.base.as_str()],
            None,
            None,
        )?;
        lay_over_proof(&tree.root, repo, proof_paths)?;
        Ok(tree)
    }
}

impl Drop for OldTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Makes the proof paths of `old` the current ones: every proof file of the repository is copied in
/// (tracked or not ignored, as the proof identity counts them), and every other file under the proof
/// paths is removed. Files outside the proof paths stay as the base commit has them.
fn lay_over_proof(old: &Path, repo: &Path, proof_paths: &[String]) -> Result<(), String> {
    // With no proof paths, git would list every file: the old tree is the base alone.
    if proof_paths.is_empty() {
        return Ok(());
    }
    let current = pathspec_files(
        repo,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
        ],
        proof_paths,
    )?;
    let base = pathspec_files(old, &["ls-files", "-z", "--"], proof_paths)?;
    // Base files the repository no longer has go first. A directory they leave with only empty directories in it is
    // in the way of a current file that replaces it, and `clear_target` removes it.
    for name in base.difference(&current) {
        remove_file_if_present(&old.join(OsStr::from_bytes(name)))?;
    }
    for name in &current {
        let target = old.join(OsStr::from_bytes(name));
        clear_target(&target)?;
        // A file the repository no longer has stays removed.
        let source = repo.join(OsStr::from_bytes(name));
        let placed = match std::fs::symlink_metadata(&source) {
            Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(&source)
                .and_then(|link| create_parent(&target).and_then(|()| symlink(link, &target))),
            Ok(meta) if meta.is_file() => {
                create_parent(&target).and_then(|()| std::fs::copy(&source, &target).map(|_| ()))
            }
            // Deleted from the repository: the removal above is all that stays.
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(std::io::Error::other("unsupported repository proof entry")),
            Err(e) => Err(e),
        };
        placed.map_err(|e| format!("{}: {e}", source.display()))?;
    }
    Ok(())
}

/// The names git lists for the given command under the pathspecs, as bytes (names need not be UTF-8).
fn pathspec_files(
    dir: &Path,
    command: &[&str],
    proof_paths: &[String],
) -> Result<BTreeSet<Vec<u8>>, String> {
    let mut args = command.to_vec();
    args.extend(proof_paths.iter().map(String::as_str));
    let listed = git_bytes(dir, &args, None, None)?;
    Ok(listed
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

/// A proof file may sit in a directory the base commit does not have.
fn create_parent(target: &Path) -> std::io::Result<()> {
    target.parent().map_or(Ok(()), std::fs::create_dir_all)
}

/// Clears the way for a current proof file: a file the base has there, or a directory whose obsolete base files left
/// only empty directories. Those directories are removed deepest first. A file anywhere in the directory is an error,
/// and then nothing is removed.
fn clear_target(target: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => {
            let mut dirs = vec![target.to_path_buf()];
            collect_directories(target, &mut dirs)?;
            // Parents come before their children, so the reverse removes each directory after those below it.
            for dir in dirs.iter().rev() {
                std::fs::remove_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            Ok(())
        }
        _ => remove_file_if_present(target),
    }
}

/// Appends the directories under `dir`, each before the directories below it. A file or symlink under `dir` is an
/// error.
fn collect_directories(dir: &Path, dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        // The entry's file type does not follow a symlink, so a symlink counts as a file.
        let is_dir = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .is_dir();
        if !is_dir {
            return Err(format!(
                "{}: a file remains in a directory that a current proof file replaces",
                path.display()
            ));
        }
        dirs.push(path.clone());
        collect_directories(&path, dirs)?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
        _ => Ok(()),
    }
}

/// One run on the old tree. On the host, a command that cannot start there is a run with no exit code, not an error.
/// Under kgate, a job that did not run the check is an error: it must not read as the check failing on the old tree.
fn old_run(root: &Path, criterion: &str, check: &Check, runner: &Runner) -> Result<OldRun, String> {
    match run_check(root, check, runner) {
        Ok((exit_code, timed_out, output_tail)) => {
            not_run_under_kgate(runner, criterion, "old", exit_code, timed_out, &output_tail)?;
            Ok(OldRun {
                exit_code,
                timed_out,
                output_tail,
            })
        }
        Err(message) if runner.is_kgate() => Err(not_run(criterion, "old", &message)),
        Err(message) => Ok(OldRun {
            exit_code: None,
            timed_out: false,
            output_tail: message,
        }),
    }
}

fn not_run(criterion: &str, tree: &str, why: &str) -> String {
    format!("criterion {criterion} was not run on the {tree} tree: {why}. It is neither a pass nor a failure and no verification was made. Read the end of the kgate log (${{XDG_STATE_HOME:-~/.local/state}}/kgate/log.jsonl), fix the cause and run verify again.")
}

fn not_run_under_kgate(
    runner: &Runner,
    criterion: &str,
    tree: &str,
    exit_code: Option<i32>,
    timed_out: bool,
    output_tail: &str,
) -> Result<(), String> {
    if runner.is_kgate() && kgate_did_not_run(exit_code, timed_out) {
        let code = exit_code.map_or("no exit code".to_owned(), |c| format!("exit {c}"));
        let tail = output_tail.trim();
        return Err(not_run(
            criterion,
            tree,
            &format!("kgate ended with {code} ({tail})"),
        ));
    }
    Ok(())
}

pub fn verify(show: &Value) -> Result<Verification, String> {
    verify_with(show, &Runner::Host)
}

pub fn verify_with(show: &Value, runner: &Runner) -> Result<Verification, String> {
    let (input, intent_id, intent) = from_show(show)?;
    let repo = Path::new(&input.repo);
    // A check may leave caches behind. The second pass starts from them; if the checks still
    // disagree about the tree, one of them edits what another has already vouched for.
    for _ in 0..2 {
        let mut results = Vec::new();
        // The tree the first check starts from counts: a check that edits what it checked must not pass.
        let mut trees = vec![tree_identity(&input)?];
        // Built before the first check, so it holds the proof files that `trees` identifies.
        let needs_old_runs = intent
            .criteria
            .iter()
            .any(|c| c.old.as_ref().is_some_and(|o| o.expect.old_runs() > 0));
        let old_tree = match &input.old_vs_new {
            Some(old) if needs_old_runs => Some(OldTree::build(
                &input,
                &old.proof_paths,
                !runner.is_kgate(),
            )?),
            _ => None,
        };
        for criterion in &intent.criteria {
            let (exit_code, timed_out, output_tail) = run_check(repo, &criterion.check, runner)
                .map_err(|message| {
                    if runner.is_kgate() {
                        not_run(&criterion.id, "current", &message)
                    } else {
                        message
                    }
                })?;
            not_run_under_kgate(
                runner,
                &criterion.id,
                "current",
                exit_code,
                timed_out,
                &output_tail,
            )?;
            trees.push(tree_identity(&input)?);
            let old = match &criterion.old {
                None => None,
                Some(expectation) => {
                    let mut runs = Vec::new();
                    if let Some(tree) = &old_tree {
                        for _ in 0..expectation.expect.old_runs() {
                            runs.push(old_run(
                                &tree.root,
                                &criterion.id,
                                &criterion.check,
                                runner,
                            )?);
                        }
                    }
                    Some(OldResult {
                        expect: expectation.expect,
                        runs,
                    })
                }
            };
            results.push(CheckResult {
                criterion: criterion.id.clone(),
                check_sha256: criterion.check.sha256(),
                exit_code,
                timed_out,
                output_tail,
                old,
            });
        }
        if trees.windows(2).all(|pair| pair[0] == pair[1]) {
            let tree = trees.pop().ok_or("no tree")?;
            return Ok(Verification {
                intent: intent_id,
                tree,
                results,
            });
        }
    }
    Err("the checks change the repository: a later check edits files an earlier check relied on. Make the checks read-only, or send their output to ignored paths.".into())
}
