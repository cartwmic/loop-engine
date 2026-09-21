use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::check::tree_graph_findings;
use crate::config::{parse_repo_config, RepoConfig};
use crate::continuity::continuity_findings;
use crate::git::{self, Snapshot, TreeReader};
use crate::prd::{parse_prd, Prd};
use crate::CheckStatus;

/// Default bound for one direct history enumeration.  Reaching the bound is
/// an incomplete check, never an implicit truncation.
pub const DEFAULT_MAX_COMMITS: usize = 100_000;

/// One line from Git's pre-push update protocol:
/// `local-ref local-oid remote-ref remote-oid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefUpdate {
    pub local_ref: String,
    pub new_oid: String,
    pub remote_ref: String,
    pub old_oid: String,
}

/// Parse the exact four-field update protocol emitted to a pre-push hook.
pub fn parse_ref_updates(input: &str) -> Result<Vec<RefUpdate>, String> {
    let mut updates = Vec::new();
    for (line_number, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 4 {
            return Err(format!(
                "pre-push update line {} must contain local ref, new object, remote ref, and old object",
                line_number + 1
            ));
        }
        validate_local_token(fields[0], line_number + 1)?;
        validate_ref(fields[2], line_number + 1, "remote")?;
        validate_oid(fields[1], line_number + 1, "new")?;
        validate_oid(fields[3], line_number + 1, "old")?;
        updates.push(RefUpdate {
            local_ref: fields[0].to_owned(),
            new_oid: fields[1].to_owned(),
            remote_ref: fields[2].to_owned(),
            old_oid: fields[3].to_owned(),
        });
    }
    Ok(updates)
}

fn validate_ref(value: &str, line: usize, side: &str) -> Result<(), String> {
    if value.is_empty()
        || !value.starts_with("refs/")
        || value.starts_with('-')
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "pre-push update line {line} has invalid {side} ref {value:?}"
        ));
    }
    Ok(())
}

fn validate_local_token(value: &str, line: usize) -> Result<(), String> {
    // Git's local-ref field is descriptive protocol data: it may be a ref,
    // HEAD, an object expression, or `(delete)`.  It is never resolved or
    // passed back to Git as a refspec.
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(format!(
            "pre-push update line {line} has invalid local ref token {value:?}"
        ));
    }
    Ok(())
}

fn validate_oid(value: &str, line: usize, side: &str) -> Result<(), String> {
    let valid_length = matches!(value.len(), 40 | 64);
    if !valid_length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "pre-push update line {line} has invalid {side} object name"
        ));
    }
    Ok(())
}

/// Acquisition and local-state failures fail closed wherever they appear;
/// they never become tip-only diagnostics.
fn is_acquisition_failure(finding: &str) -> bool {
    [
        "historical snapshot unavailable",
        "historical parents unavailable",
        "history enumeration incomplete",
        "required historical PRD unavailable",
        "cannot establish historical Bookends state",
        "local Git state",
        "could not capture",
        "not a git work tree",
        "publication history is incomplete",
    ]
    .iter()
    .any(|marker| finding.contains(marker))
}

fn is_zero_oid(oid: &str) -> bool {
    oid.bytes().all(|byte| byte == b'0')
}

/// Options that affect only the mechanical publication walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationOptions {
    pub remote: String,
    pub max_commits: usize,
}

impl Default for PublicationOptions {
    fn default() -> Self {
        Self {
            remote: "origin".to_owned(),
            max_commits: DEFAULT_MAX_COMMITS,
        }
    }
}

/// Durable, per-ref account of the actual publication range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicationRange {
    pub local_ref: String,
    pub remote_ref: String,
    pub old_oid: String,
    pub new_oid: String,
    pub old_commit: Option<String>,
    pub new_commit: Option<String>,
    pub introduced_commits: Vec<String>,
    pub complete: bool,
    pub error: Option<String>,
}

/// Result of checking all supplied ref updates.  `complete` describes the
/// history walk; a complete walk may still be RED because a snapshot or edge
/// violates the Bookends contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationReport {
    pub status: CheckStatus,
    pub live_ids: Vec<String>,
    pub complete: bool,
    pub updates: Vec<RefUpdate>,
    pub ranges: Vec<PublicationRange>,
    pub checked_commits: Vec<String>,
    pub findings: Vec<String>,
    /// Findings before an explicit bypass clears the display projection.
    pub diagnostics: Vec<String>,
}

/// Check each commit reachable from every new tip but not its corresponding
/// old tip.  A zero old tip means the complete reachable history of the new
/// tip.  Historical config, PRD, workflow, and proof bytes are read directly
/// from each commit tree; the current checkout is used only to prove that
/// acquisition did not change local Git state.
pub fn check_publication(
    repo_root: &Path,
    updates: &[RefUpdate],
    options: &PublicationOptions,
    bypass: Option<(&str, &str)>,
) -> Result<PublicationReport, io::Error> {
    let meta =
        std::fs::metadata(repo_root).map_err(|err| crate::io_err_reading_root(repo_root, err))?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("repo root is not a directory: {}", repo_root.display()),
        ));
    }

    let state_before = git::local_state(repo_root).ok();
    let mut findings = Vec::new();
    if state_before.is_none() {
        findings.push("could not capture local Git state before history acquisition".to_owned());
    }
    if !git::is_git_repo(repo_root) {
        findings.push("repository is not a git work tree".to_owned());
    }

    let mut ranges = Vec::new();
    let mut checked_commits = Vec::new();
    let mut live_ids = Vec::new();
    // Tip-only publication rule (LE-133): coverage-timing findings on
    // intermediate introduced commits are retained as visible diagnostics
    // because eligible citations may legitimately land later in the same
    // pushed range; the published tip tree must still be independently clean.
    // Per-commit continuity findings (removal, reassignment, revival,
    // tombstone and adoption games) and any acquisition/state failure still
    // block wherever they appear.
    let mut intermediate = Vec::new();
    for update in updates {
        let (range, range_findings, range_live_ids) = check_update(repo_root, update, options);
        checked_commits.extend(range.introduced_commits.iter().cloned());
        let tip_mark = format!("{}:", update.new_oid);
        let tip_paren = format!("{} (", update.new_oid);
        for finding in range_findings {
            if finding.starts_with(&tip_mark)
                || finding.starts_with(&tip_paren)
                || finding.contains(" (parent ")
                || is_acquisition_failure(&finding)
            {
                findings.push(finding);
            } else {
                intermediate.push(finding);
            }
        }
        live_ids.extend(range_live_ids);
        ranges.push(range);
    }

    let state_after = git::local_state(repo_root).ok();
    if state_after.is_none() {
        findings.push("could not capture local Git state after history acquisition".to_owned());
    }
    if let (Some(before), Some(after)) = (&state_before, &state_after) {
        if before != after {
            findings.push(
                "local Git state changed during history acquisition; HEAD, local branches, index, and worktree must remain unchanged"
                    .to_owned(),
            );
            for range in &mut ranges {
                range.complete = false;
                if range.error.is_none() {
                    range.error = Some("local Git state changed during acquisition".to_owned());
                }
            }
        }
    }

    let complete = state_before.is_some()
        && state_after.is_some()
        && ranges.iter().all(|range| range.complete);
    if !complete {
        findings.push(
            "publication history is incomplete; missing objects, failed fetch, or incomplete enumeration cannot be GREEN"
                .to_owned(),
        );
    }

    live_ids.sort();
    live_ids.dedup();
    checked_commits.sort();
    checked_commits.dedup();
    let mut diagnostics = findings.clone();
    diagnostics.extend(intermediate.iter().cloned());
    let status = if findings.is_empty() {
        CheckStatus::Green
    } else if let Some((class, reason)) = bypass {
        CheckStatus::Bypass {
            class: class.to_owned(),
            reason: reason.to_owned(),
        }
    } else {
        CheckStatus::Red
    };
    let mut findings = if matches!(status, CheckStatus::Bypass { .. }) {
        Vec::new()
    } else {
        findings
    };
    if matches!(status, CheckStatus::Red) {
        findings.extend(intermediate);
    }

    Ok(PublicationReport {
        status,
        live_ids,
        complete,
        updates: updates.to_vec(),
        ranges,
        checked_commits,
        findings,
        diagnostics,
    })
}

fn check_update(
    repo: &Path,
    update: &RefUpdate,
    options: &PublicationOptions,
) -> (PublicationRange, Vec<String>, Vec<String>) {
    let mut range = PublicationRange {
        local_ref: update.local_ref.clone(),
        remote_ref: update.remote_ref.clone(),
        old_oid: update.old_oid.clone(),
        new_oid: update.new_oid.clone(),
        old_commit: None,
        new_commit: None,
        introduced_commits: Vec::new(),
        complete: true,
        error: None,
    };
    if is_zero_oid(&update.new_oid) {
        // Deleting a ref publishes no new tree.  Retain the real update in
        // the receipt rather than pretending it was a checked commit range.
        return (range, Vec::new(), Vec::new());
    }

    let mut findings = Vec::new();
    let mut live_ids = Vec::new();
    let (new_commit, old_commit) = match ensure_history(repo, update, options) {
        Ok(tips) => tips,
        Err(error) => {
            range.complete = false;
            range.error = Some(error.clone());
            findings.push(format!(
                "{}: history acquisition incomplete: {error}",
                update.new_oid
            ));
            return (range, findings, live_ids);
        }
    };
    range.new_commit = Some(new_commit.clone());
    range.old_commit = old_commit.clone();

    let new_reachable = match git::reachable_commits(
        repo,
        std::slice::from_ref(&new_commit),
        options.max_commits,
    ) {
        Ok(commits) => commits,
        Err(error) => {
            return incomplete_after_walk(range, findings, error);
        }
    };
    let old_reachable = match old_commit {
        Some(ref old) => {
            match git::reachable_commits(repo, std::slice::from_ref(old), options.max_commits) {
                Ok(commits) => commits,
                Err(error) => return incomplete_after_walk(range, findings, error),
            }
        }
        None => BTreeSet::new(),
    };

    let introduced: Vec<String> = new_reachable.difference(&old_reachable).cloned().collect();
    range.introduced_commits = introduced.clone();

    for commit in &introduced {
        let snapshot = Snapshot::new(repo, commit.clone());
        let evaluation = match evaluate_snapshot(&snapshot) {
            Ok(evaluation) => evaluation,
            Err(error) => {
                range.complete = false;
                range.error = Some(error.clone());
                findings.push(format!(
                    "{commit}: historical snapshot unavailable: {error}"
                ));
                continue;
            }
        };
        if commit == &new_commit {
            live_ids.extend(evaluation.live_ids.iter().cloned());
        }
        findings.extend(
            evaluation
                .findings
                .iter()
                .map(|finding| format!("{commit}: {finding}")),
        );

        let parents = match snapshot.parents() {
            Ok(parents) => parents,
            Err(error) => {
                range.complete = false;
                range.error = Some(error.clone());
                findings.push(format!("{commit}: historical parents unavailable: {error}"));
                continue;
            }
        };
        if !evaluation.enabled {
            for parent in parents {
                match parent_is_adopted(repo, &parent) {
                    Ok(false) => {}
                    Ok(true) => findings.push(format!(
                        "{commit} (parent {parent}): Bookends config or PRD was removed after adoption"
                    )),
                    Err(error) => {
                        range.complete = false;
                        range.error = Some(error.clone());
                        findings.push(format!(
                            "{commit} (parent {parent}): cannot establish historical Bookends state: {error}"
                        ));
                    }
                }
            }
            continue;
        }

        let Some(current_prd) = evaluation.prd.as_ref() else {
            continue;
        };
        for parent in parents {
            match parent_prd(repo, &parent) {
                Ok(Some(previous)) => {
                    findings.extend(
                        continuity_findings(current_prd, Some(&previous))
                            .into_iter()
                            .map(|finding| format!("{commit} (parent {parent}): {finding}")),
                    );
                }
                Ok(None) => {}
                Err(ParentPrdError::Unavailable(error)) => {
                    range.complete = false;
                    range.error = Some(error.clone());
                    findings.push(format!(
                        "{commit} (parent {parent}): required historical PRD unavailable: {error}"
                    ));
                }
                Err(ParentPrdError::Invalid(error)) => {
                    // A historical enabled parent whose PRD is malformed is
                    // not a first-adoption baseline, but it was read fully.
                    findings.push(format!("{commit} (parent {parent}): {error}"));
                }
            }
        }
    }

    (range, findings, live_ids)
}

fn incomplete_after_walk(
    mut range: PublicationRange,
    mut findings: Vec<String>,
    error: git::ReachableError,
) -> (PublicationRange, Vec<String>, Vec<String>) {
    let detail = reachable_error(&error);
    range.complete = false;
    range.error = Some(detail.clone());
    findings.push(format!(
        "{}: history enumeration incomplete: {detail}",
        range.new_oid
    ));
    (range, findings, Vec::new())
}

fn ensure_history(
    repo: &Path,
    update: &RefUpdate,
    options: &PublicationOptions,
) -> Result<(String, Option<String>), String> {
    if options.remote.is_empty()
        || options.remote.starts_with('-')
        || options.remote.chars().any(char::is_whitespace)
    {
        return Err("source remote is empty or invalid".to_owned());
    }
    if options.max_commits == 0 {
        return Err("history enumeration limit must be greater than zero".to_owned());
    }

    if git::is_shallow(repo)? {
        git::fetch_unshallow(repo, &options.remote)
            .map_err(|error| format!("could not complete shallow history fetch: {error}"))?;
    }

    let mut fetch_attempts = 0usize;
    loop {
        let new_commit = match git::resolve_commit(repo, &update.new_oid) {
            Ok(commit) => commit,
            Err(error) => {
                fetch_attempts = fetch_attempts.saturating_add(1);
                fetch_if_possible(
                    repo,
                    &options.remote,
                    &update.new_oid,
                    fetch_attempts,
                    error,
                )?;
                continue;
            }
        };
        let old_commit = if is_zero_oid(&update.old_oid) {
            None
        } else {
            match git::resolve_commit(repo, &update.old_oid) {
                Ok(commit) => Some(commit),
                Err(error) => {
                    fetch_attempts = fetch_attempts.saturating_add(1);
                    fetch_if_possible(
                        repo,
                        &options.remote,
                        &update.old_oid,
                        fetch_attempts,
                        error,
                    )?;
                    continue;
                }
            }
        };

        let mut tips = vec![new_commit.clone()];
        if let Some(old) = &old_commit {
            tips.push(old.clone());
        }
        match git::reachable_commits(repo, &tips, options.max_commits) {
            Ok(_) => return Ok((new_commit, old_commit)),
            Err(git::ReachableError::Limit { limit }) => {
                return Err(format!(
                    "history enumeration reached limit {limit}; completeness is unknown"
                ));
            }
            Err(git::ReachableError::Missing { object, detail }) => {
                fetch_attempts = fetch_attempts.saturating_add(1);
                fetch_if_possible(repo, &options.remote, &object, fetch_attempts, detail)?;
            }
        }
    }
}

fn fetch_if_possible(
    repo: &Path,
    remote: &str,
    object: &str,
    attempts: usize,
    previous_error: String,
) -> Result<(), String> {
    // A broken/misconfigured remote must not create an unbounded fetch loop.
    if attempts > 128 {
        return Err(format!(
            "history object {object} remains unavailable after {attempts} fetch attempts: {previous_error}"
        ));
    }
    git::fetch_object(repo, remote, object).map_err(|fetch_error| {
        format!(
            "required history object {object} is unavailable ({previous_error}); fetch failed: {fetch_error}"
        )
    })
}

fn reachable_error(error: &git::ReachableError) -> String {
    match error {
        git::ReachableError::Missing { object, detail } => {
            format!("required object {object} is unavailable: {detail}")
        }
        git::ReachableError::Limit { limit } => {
            format!("history enumeration reached limit {limit}; completeness is unknown")
        }
    }
}

struct SnapshotEvaluation {
    enabled: bool,
    prd: Option<Prd>,
    live_ids: Vec<String>,
    findings: Vec<String>,
}

enum ConfigState {
    Absent,
    Valid(RepoConfig),
    Malformed(String),
}

fn historical_config(snapshot: &Snapshot) -> Result<ConfigState, String> {
    let Some(text) = snapshot.read_text("bookends.toml")? else {
        return Ok(ConfigState::Absent);
    };
    match parse_repo_config(&text) {
        Ok(config) => Ok(ConfigState::Valid(config)),
        Err(error) => Ok(ConfigState::Malformed(error)),
    }
}

fn evaluate_snapshot(snapshot: &Snapshot) -> Result<SnapshotEvaluation, String> {
    let config = historical_config(snapshot)?;
    let ConfigState::Valid(config) = config else {
        return Ok(match config {
            ConfigState::Absent => SnapshotEvaluation {
                enabled: false,
                prd: None,
                live_ids: Vec::new(),
                findings: Vec::new(),
            },
            ConfigState::Malformed(error) => SnapshotEvaluation {
                enabled: true,
                prd: None,
                live_ids: Vec::new(),
                findings: vec![error],
            },
            ConfigState::Valid(_) => unreachable!(),
        });
    };

    let Some(prd_text) = snapshot.read_text(&config.prd)? else {
        return Ok(SnapshotEvaluation {
            enabled: true,
            prd: None,
            live_ids: Vec::new(),
            findings: vec![format!(
                "{} is missing; enabled historical inputs fail closed",
                config.prd
            )],
        });
    };
    let prd = match parse_prd(&prd_text) {
        Ok(prd) => prd,
        Err(errors) => {
            return Ok(SnapshotEvaluation {
                enabled: true,
                prd: None,
                live_ids: Vec::new(),
                findings: errors,
            });
        }
    };
    let findings = tree_graph_findings(snapshot, &config, &prd);
    Ok(SnapshotEvaluation {
        enabled: true,
        live_ids: prd.live_ids(),
        prd: Some(prd),
        findings,
    })
}

fn parent_is_adopted(repo: &Path, parent: &str) -> Result<bool, String> {
    let snapshot = Snapshot::new(repo, parent.to_owned());
    match historical_config(&snapshot)? {
        ConfigState::Absent => Ok(false),
        ConfigState::Malformed(_) => Ok(true),
        ConfigState::Valid(config) => Ok(snapshot.read_text(&config.prd)?.is_some()),
    }
}

enum ParentPrdError {
    Unavailable(String),
    Invalid(String),
}

fn parent_prd(repo: &Path, parent: &str) -> Result<Option<Prd>, ParentPrdError> {
    let snapshot = Snapshot::new(repo, parent.to_owned());
    let config = historical_config(&snapshot).map_err(ParentPrdError::Unavailable)?;
    match config {
        ConfigState::Absent => Ok(None),
        ConfigState::Malformed(error) => Err(ParentPrdError::Invalid(format!(
            "historical parent config malformed: {error}"
        ))),
        ConfigState::Valid(config) => {
            let text = snapshot
                .read_text(&config.prd)
                .map_err(ParentPrdError::Unavailable)?;
            let Some(text) = text else {
                // A config-only parent is still the setup half of first
                // adoption and supplies no continuity baseline.
                return Ok(None);
            };
            parse_prd(&text).map(Some).map_err(|errors| {
                ParentPrdError::Invalid(format!(
                    "historical parent PRD is malformed: {}",
                    errors.join("; ")
                ))
            })
        }
    }
}
