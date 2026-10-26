//! `lean-change setup` and `lean-change review-packet`.
use crate::run::{from_show, tree_identity};
use lean_change_provider::evaluate::verification_ready;
use lean_change_provider::model::*;
use loop_core::ContextRecord;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(repo: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let out = command.output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
        Some("b") => (true, "lean-b-1"),
        Some("c") => (false, "lean-c-1"),
        _ => return Err("--profile must be b or c".into()),
    };
    let repo = std::fs::canonicalize(repo.ok_or("--repo is required")?)
        .map_err(|e| format!("--repo: {e}"))?;
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
    let index = std::env::temp_dir().join(format!("lean-change-index-{}", std::process::id()));
    let diff = git(repo, &["read-tree", &input.base], Some(&index))
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
