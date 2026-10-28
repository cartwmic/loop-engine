//! lean-change verify: run each approved check on the current tree.
use lean_change_provider::model::*;
use loop_integrations::proof_metadata::repository_proof_identity;
use serde_json::Value;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    Ok((input, id, Intent::parse(&record["data"])?))
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

pub fn verify(show: &Value) -> Result<Verification, String> {
    let (input, intent_id, intent) = from_show(show)?;
    let repo = Path::new(&input.repo);
    // A check may leave caches behind. The second pass starts from them; if the checks still
    // disagree about the tree, one of them edits what another has already vouched for.
    for _ in 0..2 {
        let mut results = Vec::new();
        // The tree the first check starts from counts: a check that edits what it checked must not pass.
        let mut trees = vec![tree_identity(&input)?];
        for criterion in &intent.criteria {
            let (exit_code, timed_out, output_tail) = run_check(repo, &criterion.check)?;
            trees.push(tree_identity(&input)?);
            results.push(CheckResult {
                criterion: criterion.id.clone(),
                check_sha256: criterion.check.sha256(),
                exit_code,
                timed_out,
                output_tail,
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
