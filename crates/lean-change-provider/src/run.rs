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
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TAIL_BYTES: usize = 2000;

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

/// Drain a stream, keeping only its last TAIL_BYTES.
fn drain_tail(mut stream: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(n) = stream.read(&mut buffer) {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buffer[..n]);
            if tail.len() > TAIL_BYTES {
                tail.drain(..tail.len() - TAIL_BYTES);
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

fn run_check(repo: &Path, check: &Check) -> Result<(Option<i32>, bool, String), String> {
    let mut child = Command::new(&check.argv[0])
        .args(&check.argv[1..])
        .current_dir(repo)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", check.argv[0]))?;
    let pid = child.id();
    let out = drain_tail(child.stdout.take().ok_or("no stdout")?);
    let err = drain_tail(child.stderr.take().ok_or("no stderr")?);
    let started = Instant::now();
    let limit = Duration::from_millis(check.timeout_ms);
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if started.elapsed() >= limit => {
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
    let from = tail.len().saturating_sub(TAIL_BYTES);
    Ok((
        code,
        timed_out,
        String::from_utf8_lossy(&tail[from..]).into_owned(),
    ))
}

/// The old tree: a checkout of the base commit outside the repository, with the current proof files
/// laid over it. Removed when dropped.
struct OldTree {
    root: PathBuf,
}

impl OldTree {
    fn build(input: &InitialInput, proof_paths: &[String]) -> Result<Self, String> {
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
        git_bytes(
            repo,
            &[
                "clone",
                "--shared",
                "--no-checkout",
                "--quiet",
                input.repo.as_str(),
                root_arg,
            ],
            None,
            None,
        )?;
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
    // Base files the repository no longer has go first. A directory they leave empty is in the way of a current
    // file that replaces it, and `clear_target` removes it.
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

/// Clears the way for a current proof file: a file the base has there, or a directory an obsolete base file left
/// empty. A directory that still holds files is an error, so nothing is removed from it.
fn clear_target(target: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir(target).map_err(|e| format!("{}: {e}", target.display()))
        }
        _ => remove_file_if_present(target),
    }
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
        _ => Ok(()),
    }
}

/// One run on the old tree. A command that cannot start there is a run with no exit code, not an error.
fn old_run(root: &Path, check: &Check) -> OldRun {
    match run_check(root, check) {
        Ok((exit_code, timed_out, output_tail)) => OldRun {
            exit_code,
            timed_out,
            output_tail,
        },
        Err(message) => OldRun {
            exit_code: None,
            timed_out: false,
            output_tail: message,
        },
    }
}

pub fn verify(show: &Value) -> Result<Verification, String> {
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
            Some(old) if needs_old_runs => Some(OldTree::build(&input, &old.proof_paths)?),
            _ => None,
        };
        for criterion in &intent.criteria {
            let (exit_code, timed_out, output_tail) = run_check(repo, &criterion.check)?;
            trees.push(tree_identity(&input)?);
            let old = criterion.old.as_ref().map(|expectation| OldResult {
                expect: expectation.expect,
                runs: old_tree.as_ref().map_or_else(Vec::new, |tree| {
                    (0..expectation.expect.old_runs())
                        .map(|_| old_run(&tree.root, &criterion.check))
                        .collect()
                }),
            });
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
