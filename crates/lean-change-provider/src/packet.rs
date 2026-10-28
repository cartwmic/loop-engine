//! `lean-change setup`, `lean-change intent-packet` and `lean-change review-packet`.
use crate::run::{from_show, tree_identity, worktree_root};
use lean_change_provider::evaluate::verification_ready;
use lean_change_provider::model::*;
use loop_core::ContextRecord;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(repo: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    git_bytes(repo, args, index, None).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Raw git output, so that `-z` path lists survive non-UTF-8 names.
fn git_bytes(
    repo: &Path,
    args: &[&str],
    index: Option<&Path>,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut command = Command::new("git");
    command.args(args).current_dir(repo);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("git: {e}"))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes).map_err(|e| format!("git: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// Profile B (`b`: review on) or C (`c`: review off), frozen with the repo base.
pub fn setup(args: &[String]) -> Result<Value, String> {
    let mut profile = None;
    let mut repo = None;
    let mut principles = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--profile" => profile = Some(value.as_str()),
            "--repo" => repo = Some(PathBuf::from(value)),
            "--principles" => principles = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let (review, version) = match profile {
        Some("b") => (true, "lean-b-2"),
        Some("c") => (false, "lean-c-1"),
        _ => return Err("--profile must be b or c".into()),
    };
    let repo = std::fs::canonicalize(repo.ok_or("--repo is required")?)
        .map_err(|e| format!("--repo: {e}"))?;
    // The checks and the tree identity cover the whole worktree, never a subdirectory of it.
    let repo = worktree_root(&repo).map_err(|e| format!("--repo: {e}"))?;
    let base = git(&repo, &["rev-parse", "HEAD"], None)?.trim().to_owned();
    let principles = match principles {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("--principles: {e}"))?;
            Some(
                json!({"source": path.display().to_string(), "sha256": sha256_hex(text.as_bytes()), "text": text}),
            )
        }
        None if review => return Err("profile b requires --principles FILE".into()),
        None => None,
    };
    let mut input = json!({"schema_version": 1, "profile_version": version, "repo": repo.display().to_string(), "base": base, "review": review});
    if let Some(principles) = principles {
        input["principles"] = principles;
    }
    InitialInput::parse(&input)?;
    Ok(input)
}

/// Everything a fresh-context reviewer needs to check a draft intent, as plain text.
pub fn intent_packet(show: &Value) -> Result<String, String> {
    let (input, intent_id, intent) = from_show(show)?;
    if !input.review {
        return Err("this profile has no intent check".into());
    }
    let questions: String = INTENT_QUESTIONS
        .iter()
        .map(|(id, text)| format!("- {id}: {text}\n"))
        .collect();
    Ok(format!(
        "# Intent check packet\n\nYou are the reviewer. You did not write this intent. Judge only from this packet and the repository at {repo} as it is now.\nAnswer every question below with verdict pass or fail and a note. If the intent already settles a question, pass it. Otherwise fail it with one finding: text is the concrete question you would ask the owner, evidence is the intent's wording or its silence, consequence is what a delivery would have to guess. Do not invent strained readings or ask for more than the goal promises.\n\nReturn one JSON object:\n{{\"intent\": \"{intent_id}\", \"reviewer\": {{\"name\": \"...\", \"kind\": \"agent\"}}, \"fresh_context\": true, \"answers\": [{{\"question\": \"...\", \"verdict\": \"pass|fail\", \"note\": \"...\"}}], \"findings\": [{{\"id\": \"f1\", \"question\": \"...\", \"text\": \"...\", \"evidence\": \"...\", \"consequence\": \"...\"}}]}}\n\n## Questions\n{questions}\n## Draft intent ({intent_id})\n{intent}\n",
        repo = input.repo,
        intent = serde_json::to_string_pretty(&intent).map_err(|e| e.to_string())?,
    ))
}

/// Everything a fresh-context reviewer needs, as plain text.
pub fn review_packet(show: &Value) -> Result<String, String> {
    let (input, intent_id, intent) = from_show(show)?;
    let principles = input
        .principles
        .as_ref()
        .ok_or("this profile has no review")?;
    let repo = Path::new(&input.repo);
    // The reviewer judges the tree the checks vouched for: the same eligibility as `verified`.
    let records: Vec<ContextRecord> = serde_json::from_value(show["result"]["context"].clone())
        .map_err(|e| format!("show envelope context is unreadable: {e}"))?;
    verification_ready(&records, &input, &tree_identity)
        .map_err(|denial| format!("not ready for review: {}", denial["feedback"]))?;
    let verification = records
        .iter()
        .filter(|r| r.kind == "verification")
        .max_by_key(|r| r.sequence)
        .and_then(|r| serde_json::from_value::<Verification>(r.data.clone()).ok())
        .ok_or("no verification")?;
    let results: String = verification
        .results
        .iter()
        .map(|r| format!("- {}: exit {:?}\n", r.criterion, r.exit_code))
        .collect();
    // Unique per call: tests run review_packet on several threads of one process.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let index =
        std::env::temp_dir().join(format!("lean-change-index-{}-{call}", std::process::id()));
    // Start from a copy of the repository's own index so that every tracked path, including one
    // force-added under an ignore rule, stays in the diff; `add -A` then stages the working tree.
    let real_index = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        None,
    )?;
    let seeded = match std::fs::copy(real_index.trim(), &index) {
        Ok(_) => Ok(String::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            git(repo, &["read-tree", &input.base], Some(&index))
        }
        Err(e) => Err(format!("could not copy the index: {e}")),
    };
    // A copied index keeps assume-unchanged and skip-worktree bits, and `add -A` skips such
    // entries; clear them in the copy only (one call per flag: a combined call keeps skip-worktree).
    let clear = |flag: &str| -> Result<(), String> {
        let paths = git_bytes(repo, &["ls-files", "-z"], Some(&index), None)?;
        git_bytes(
            repo,
            &["update-index", flag, "-z", "--stdin"],
            Some(&index),
            Some(&paths),
        )
        .map(|_| ())
    };
    let diff = seeded
        .and_then(|_| clear("--no-assume-unchanged"))
        .and_then(|_| clear("--no-skip-worktree"))
        .and_then(|_| git(repo, &["add", "-A"], Some(&index)))
        .and_then(|_| git(repo, &["diff", "--cached", &input.base], Some(&index)));
    let _ = std::fs::remove_file(&index);
    let questions: String = QUESTIONS
        .iter()
        .map(|(id, text)| format!("- {id}: {text}\n"))
        .collect();
    Ok(format!(
        "# Review packet\n\nYou are the reviewer. You did not write this change. Judge only from this packet and the repository at {repo}.\nAnswer every question below with verdict pass or fail and a note. Every fail needs at least one finding. Every finding needs an id, the question, text, evidence and consequence.\n\nReturn one JSON object:\n{{\"intent\": \"{intent_id}\", \"tree\": \"{tree}\", \"reviewer\": {{\"name\": \"...\", \"kind\": \"agent\"}}, \"fresh_context\": true, \"answers\": [{{\"question\": \"...\", \"verdict\": \"pass|fail\", \"note\": \"...\"}}], \"findings\": [{{\"id\": \"f1\", \"question\": \"...\", \"text\": \"...\", \"evidence\": \"...\", \"consequence\": \"...\"}}]}}\n\n## Questions\n{questions}\n## Approved intent ({intent_id})\n{intent}\n\n## Checks that passed on tree {tree}\n{results}\n## Design principles (sha256 {sha})\n{principles}\n\n## Diff against base {base}\n{diff}\n",
        repo = input.repo,
        tree = verification.tree,
        intent = serde_json::to_string_pretty(&intent).map_err(|e| e.to_string())?,
        sha = principles.sha256,
        principles = principles.text,
        base = input.base,
        diff = diff?,
    ))
}
