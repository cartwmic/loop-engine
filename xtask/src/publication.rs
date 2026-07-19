//! Exact-commit publication / pre-push gate (T028).
//!
//! Enumerates unpublished commits from remote-update input, evaluates each
//! commit in a temporary detached worktree against the candidate revision's
//! own quality manifest (or the immutable pre-manifest baseline), and requires
//! a determinate parent-rubric semantic-judge `pass` in publication mode.
//! Unavailable/indeterminate/fail block. A later good commit cannot repair a
//! failing middle commit. Never rewrites the user working tree.

use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::quality::{self, QualityManifest, QualityReport};
use crate::semantic_judge::{
    self, Disposition, JudgeOptions, JudgeOutcome, Mode, unpublished_commits,
};

/// One `git pre-push` stdin line: local ref/sha and remote ref/sha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushUpdate {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_ref: String,
    pub remote_sha: String,
}

/// Options for the exact-commit publication gate.
#[derive(Debug, Clone)]
pub struct PublicationOptions {
    pub repo_root: PathBuf,
    pub judge_executable: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
    /// Optional Git root used to load immutable foundation seed blobs in tests.
    pub foundation_git_root: Option<PathBuf>,
    /// Test-only override for the quality manifest path. Normal publication history
    /// always loads `quality/manifest.toml` from each candidate's detached tree.
    pub quality_manifest: Option<PathBuf>,
    /// Exact Git remote name and URL supplied by the pre-push hook. Both are
    /// required for new refs; advertised destination refs, never mutable local
    /// remote-tracking refs, determine which commits are already published.
    pub remote_name: Option<String>,
    pub remote_url: Option<String>,
}

impl PublicationOptions {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            judge_executable: None,
            timeout_seconds: None,
            foundation_git_root: None,
            quality_manifest: None,
            remote_name: None,
            remote_url: None,
        }
    }
}

/// Per-commit publication gate outcome.
#[derive(Debug, Clone)]
pub struct CommitPublicationOutcome {
    pub parent_revision: String,
    pub candidate_revision: String,
    pub quality: QualityReport,
    pub judge: JudgeOutcome,
    /// Absolute path that was used as the detached worktree (removed afterward).
    pub worktree_path: PathBuf,
}

/// Aggregate publication outcome for one unpublished range.
#[derive(Debug, Clone)]
pub struct PublicationOutcome {
    pub commits: Vec<CommitPublicationOutcome>,
}

enum CandidateQualityPlan {
    PreManifestBaseline,
    Manifest(QualityManifest, PathBuf),
}

/// Parse all `pre-push` remote-update lines from `stdin`.
pub fn read_push_updates_from_stdin() -> Result<Vec<PushUpdate>> {
    read_push_updates(io::stdin().lock())
}

/// Parse remote-update lines from any buffered reader.
pub fn read_push_updates<R: BufRead>(reader: R) -> Result<Vec<PushUpdate>> {
    let mut updates = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("failed reading push update line {}", index + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        updates.push(
            parse_push_update_line(trimmed).with_context(|| {
                format!("invalid push update on line {}: `{trimmed}`", index + 1)
            })?,
        );
    }
    Ok(updates)
}

/// Parse one `<local_ref> <local_sha> <remote_ref> <remote_sha>` line.
pub fn parse_push_update_line(line: &str) -> Result<PushUpdate> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 4 {
        bail!("expected 4 fields, found {}", parts.len());
    }
    Ok(PushUpdate {
        local_ref: parts[0].to_owned(),
        local_sha: parts[1].to_owned(),
        remote_ref: parts[2].to_owned(),
        remote_sha: parts[3].to_owned(),
    })
}

/// True when an OID is the all-zero deleted/new sentinel.
pub fn is_zero_oid(oid: &str) -> bool {
    !oid.is_empty() && oid.chars().all(|ch| ch == '0')
}

/// Enumerate unpublished commit SHAs for one remote update (oldest-first).
///
/// - Branch delete (`local_sha` zero): empty list.
/// - New branch (`remote_sha` zero): commits reachable from `local_sha` but not
///   from any locally known remote ref, oldest-first.
/// - Update / divergent push: `rev-list --reverse <remote_sha>..<local_sha>`.
pub fn unpublished_commits_for_update(
    repo_root: &Path,
    update: &PushUpdate,
    remote_name: Option<&str>,
    remote_url: Option<&str>,
) -> Result<Vec<String>> {
    if is_zero_oid(&update.local_sha) {
        return Ok(Vec::new());
    }
    let commits = if is_zero_oid(&update.remote_sha) {
        let remote_name = remote_name.context(
            "new-branch publication requires the exact destination remote name from pre-push",
        )?;
        if remote_name.is_empty()
            || !remote_name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            bail!("invalid destination remote name `{remote_name}`");
        }
        let remote_url = remote_url.context(
            "new-branch publication requires the exact destination remote URL from pre-push",
        )?;
        unpublished_against_advertised_destination(
            repo_root,
            &update.local_sha,
            remote_name,
            remote_url,
        )?
    } else {
        unpublished_commits(repo_root, &update.remote_sha, &update.local_sha)?
    };
    reject_merge_commits(repo_root, &commits)?;
    Ok(commits)
}

fn unpublished_against_advertised_destination(
    repo_root: &Path,
    local_sha: &str,
    remote_name: &str,
    remote_url: &str,
) -> Result<Vec<String>> {
    let graph = create_temp_dir("destination-graph")?;
    let result = (|| -> Result<Vec<String>> {
        git_run(
            repo_root,
            &[
                "init",
                "--quiet",
                "--bare",
                graph
                    .to_str()
                    .context("temporary graph path is not UTF-8")?,
            ],
        )?;

        let object_path = git_output_trimmed(repo_root, &["rev-parse", "--git-path", "objects"])?;
        let object_path = PathBuf::from(object_path);
        let object_path = if object_path.is_absolute() {
            object_path
        } else {
            repo_root.join(object_path)
        };
        let alternates = graph.join("objects/info/alternates");
        fs::create_dir_all(
            alternates
                .parent()
                .context("alternates path has no parent")?,
        )?;
        fs::write(&alternates, format!("{}\n", object_path.display()))?;

        let graph_arg = format!("--git-dir={}", graph.display());
        let advertised = git_output_trimmed(repo_root, &["ls-remote", "--refs", remote_url])
            .with_context(|| {
                format!("failed reading advertised refs from destination remote `{remote_name}`")
            })?;
        if !advertised.is_empty() {
            let destination_refspec = "+refs/*:refs/remotes/destination/*";
            git_run(
                repo_root,
                &[
                    &graph_arg,
                    "fetch",
                    "--quiet",
                    "--no-tags",
                    remote_url,
                    destination_refspec,
                ],
            )
            .with_context(|| {
                format!("failed fetching advertised graph from destination remote `{remote_name}`")
            })?;
        }
        let text = git_output_trimmed(
            repo_root,
            &[
                &graph_arg,
                "rev-list",
                "--reverse",
                local_sha,
                "--not",
                "--remotes=destination",
            ],
        )?;
        Ok(lines_nonempty(&text))
    })();
    let _ = fs::remove_dir_all(&graph);
    result
}

/// Run the publication gate for every unpublished commit in `updates`.
pub fn publish_updates(
    options: &PublicationOptions,
    updates: &[PushUpdate],
) -> Result<Vec<PublicationOutcome>> {
    let root = resolve_root(Some(&options.repo_root))?;
    let mut outcomes = Vec::new();
    for update in updates {
        if is_zero_oid(&update.local_sha) {
            continue;
        }
        let commits = unpublished_commits_for_update(
            &root,
            update,
            options.remote_name.as_deref(),
            options.remote_url.as_deref(),
        )?;
        if commits.is_empty() {
            continue;
        }
        let from_exclusive = if is_zero_oid(&update.remote_sha) {
            first_parent_or_empty(&root, &commits[0])?
        } else {
            git_output_trimmed(
                &root,
                &["rev-parse", &format!("{}^{{commit}}", update.remote_sha)],
            )?
        };
        let to_inclusive = git_output_trimmed(&root, &["rev-parse", &update.local_sha])?;
        outcomes.push(publish_range(options, &from_exclusive, &to_inclusive)?);
    }
    if outcomes.is_empty() {
        return Ok(outcomes);
    }
    Ok(outcomes)
}

/// Publish/gate every commit in `from_exclusive..to_inclusive` (oldest-first).
pub fn publish_range(
    options: &PublicationOptions,
    from_exclusive: &str,
    to_inclusive: &str,
) -> Result<PublicationOutcome> {
    let root = resolve_root(Some(&options.repo_root))?;
    let commits = if from_exclusive.is_empty() {
        let text = git_output_trimmed(&root, &["rev-list", "--reverse", to_inclusive])?;
        lines_nonempty(&text)
    } else {
        unpublished_commits(&root, from_exclusive, to_inclusive)?
    };
    if commits.is_empty() {
        bail!("publication range {from_exclusive}..{to_inclusive} contains no commits to gate");
    }
    reject_merge_commits(&root, &commits)?;

    let before_head = git_output_trimmed(&root, &["rev-parse", "HEAD"])?;
    let before_status = git_output_trimmed(&root, &["status", "--porcelain"])?;

    let mut outcomes = Vec::with_capacity(commits.len());
    for candidate in commits {
        let parent_revision = resolve_first_parent(&root, &candidate)?;
        if parent_revision.is_empty() {
            bail!(
                "refusing to publish root commit {candidate} without a parent revision; foundation bootstrap is consumed"
            );
        }

        let gated =
            gate_one_commit(options, &root, &parent_revision, &candidate).with_context(|| {
                format!(
                    "publication blocked for {parent_revision}..{candidate} (exact-commit gate)"
                )
            })?;
        outcomes.push(gated);
    }

    let after_head = git_output_trimmed(&root, &["rev-parse", "HEAD"])?;
    let after_status = git_output_trimmed(&root, &["status", "--porcelain"])?;
    if before_head != after_head || before_status != after_status {
        bail!(
            "publication gate must not rewrite the user tree (HEAD/status changed during exact-commit evaluation)"
        );
    }

    Ok(PublicationOutcome { commits: outcomes })
}

/// CLI entrypoint for `xtask publication`.
pub fn run_cli(
    repo_root: Option<&Path>,
    from_exclusive: Option<&str>,
    to_inclusive: Option<&str>,
    judge_executable: Option<&Path>,
    timeout_seconds: Option<u64>,
) -> Result<()> {
    let root = resolve_root(repo_root)?;
    let mut options = PublicationOptions::new(&root);
    options.judge_executable = judge_executable.map(Path::to_path_buf);
    options.timeout_seconds = timeout_seconds;

    if let Some(from) = from_exclusive {
        let to = match to_inclusive {
            Some(value) => value.to_owned(),
            None => git_output_trimmed(&root, &["rev-parse", "HEAD"])?,
        };
        let outcome = publish_range(&options, from, &to)?;
        emit_publication_outcome(&outcome)?;
        return Ok(());
    }

    if to_inclusive.is_some() {
        bail!("--to requires --from");
    }

    let updates = read_push_updates_from_stdin()?;
    if updates.is_empty() {
        bail!("publication requires --from <rev> or pre-push remote-update lines on stdin");
    }
    let outcomes = publish_updates(&options, &updates)?;
    for outcome in &outcomes {
        emit_publication_outcome(outcome)?;
    }
    Ok(())
}

fn emit_publication_outcome(outcome: &PublicationOutcome) -> Result<()> {
    for commit in &outcome.commits {
        println!(
            "publication ok {}..{} ({} quality checks)",
            commit.parent_revision,
            commit.candidate_revision,
            commit.quality.checks.len()
        );
    }
    Ok(())
}

fn emit_commit_judge_response(judge: &JudgeOutcome) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&judge.response).context("serialize publication judge response")?
    );
    Ok(())
}

fn gate_one_commit(
    options: &PublicationOptions,
    repo_root: &Path,
    parent_revision: &str,
    candidate_revision: &str,
) -> Result<CommitPublicationOutcome> {
    enforce_revision_diff_check(repo_root, parent_revision, candidate_revision)?;
    let worktree = DetachedWorktree::create(repo_root, candidate_revision)?;
    let plan = resolve_candidate_quality_plan(
        options,
        repo_root,
        &worktree.path,
        candidate_revision,
        parent_revision,
    )?;

    let quality_report = match plan {
        CandidateQualityPlan::PreManifestBaseline => QualityReport {
            manifest_path: worktree.path.join(quality::MANIFEST_PATH),
            candidate_revision: candidate_revision.to_owned(),
            checks: Vec::new(),
        },
        CandidateQualityPlan::Manifest(manifest, manifest_path) => {
            quality::run_manifest(
                &worktree.path,
                &manifest,
                &manifest_path,
                candidate_revision,
            )
            .with_context(|| {
                format!(
                    "manifest-driven quality checks failed for exact commit {candidate_revision} in detached worktree {}",
                    worktree.path.display()
                )
            })?
        }
    };

    let worktree_path = worktree.path.clone();
    drop(worktree);

    let mut judge_options = JudgeOptions::new(repo_root, Mode::Publication);
    judge_options.executable = options.judge_executable.clone();
    judge_options.timeout_seconds = options.timeout_seconds;
    judge_options.foundation_git_root = options.foundation_git_root.clone();
    judge_options.extra_deterministic_evidence = quality_report.deterministic_evidence();

    let judge =
        semantic_judge::judge_revision_pair(&judge_options, parent_revision, candidate_revision)?;

    emit_commit_judge_response(&judge)?;

    match judge.disposition {
        Disposition::Allow => Ok(CommitPublicationOutcome {
            parent_revision: parent_revision.to_owned(),
            candidate_revision: candidate_revision.to_owned(),
            quality: quality_report,
            judge,
            worktree_path,
        }),
        Disposition::WarnAllow => {
            bail!(
                "internal error: publication disposition WarnAllow for {}..{} (verdict={:?})",
                parent_revision,
                candidate_revision,
                judge.response.verdict
            );
        }
        Disposition::Block => bail!(
            "semantic judge blocked publication for {}..{} (verdict={:?}): {}",
            parent_revision,
            candidate_revision,
            judge.response.verdict,
            judge.response.message
        ),
    }
}

fn enforce_revision_diff_check(
    repo_root: &Path,
    parent_revision: &str,
    candidate_revision: &str,
) -> Result<()> {
    let output = Command::new("git")
        .args(["diff", "--check", parent_revision, candidate_revision])
        .current_dir(repo_root)
        .output()
        .context("failed to execute git diff --check")?;
    if !output.status.success() {
        bail!(
            "git diff --check failed for {parent_revision}..{candidate_revision}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn resolve_candidate_quality_plan(
    options: &PublicationOptions,
    repo_root: &Path,
    worktree_path: &Path,
    candidate_revision: &str,
    parent_revision: &str,
) -> Result<CandidateQualityPlan> {
    if let Some(override_path) = &options.quality_manifest {
        let manifest = quality::load_manifest(repo_root, Some(override_path))?;
        return Ok(CandidateQualityPlan::Manifest(
            manifest,
            override_path.clone(),
        ));
    }

    let manifest_path = worktree_path.join(quality::MANIFEST_PATH);
    if manifest_path.is_file() {
        let manifest = quality::load_manifest(worktree_path, Some(&manifest_path))?;
        let parent_manifest = quality::load_manifest_at_revision(repo_root, parent_revision)?;
        quality::enforce_manifest_monotonic_evolution(parent_manifest.as_ref(), &manifest)?;
        return Ok(CandidateQualityPlan::Manifest(manifest, manifest_path));
    }

    if quality::load_manifest_at_revision(repo_root, parent_revision)?.is_some() {
        bail!(
            "quality manifest regression: candidate {candidate_revision} removed {MANIFEST_PATH} after parent {parent_revision} introduced it",
            MANIFEST_PATH = quality::MANIFEST_PATH
        );
    }

    let _ = candidate_revision;
    Ok(CandidateQualityPlan::PreManifestBaseline)
}

fn reject_merge_commits(repo_root: &Path, commits: &[String]) -> Result<()> {
    for commit in commits {
        let parents = git_output_trimmed(repo_root, &["rev-list", "--parents", "-n", "1", commit])?;
        let parent_count = parents.split_whitespace().count().saturating_sub(1);
        if parent_count > 1 {
            bail!(
                "unsupported merge commit {commit} in publication range (nonlinear history with {parent_count} parents)"
            );
        }
    }
    Ok(())
}

fn resolve_first_parent(repo_root: &Path, commit: &str) -> Result<String> {
    let parents = git_output_trimmed(repo_root, &["rev-list", "--parents", "-n", "1", commit])?;
    let mut parts = parents.split_whitespace();
    let _commit = parts
        .next()
        .context("missing commit oid in rev-list output")?;
    let first = parts
        .next()
        .with_context(|| format!("commit {commit} has no parent"))?
        .to_owned();
    if parts.next().is_some() {
        bail!("unsupported merge commit {commit} in publication range (nonlinear history)");
    }
    Ok(first)
}

struct DetachedWorktree {
    repo_root: PathBuf,
    path: PathBuf,
}

impl DetachedWorktree {
    fn create(repo_root: &Path, commit: &str) -> Result<Self> {
        let path = create_temp_dir("exact-commit")?;
        let _ = fs::remove_dir_all(&path);
        git_run(
            repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().context("worktree path is not UTF-8")?,
                commit,
            ],
        )
        .with_context(|| {
            format!(
                "failed to create detached worktree for {commit} at {}",
                path.display()
            )
        })?;
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            path,
        })
    }
}

impl Drop for DetachedWorktree {
    fn drop(&mut self) {
        let path = self.path.to_string_lossy().to_string();
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force", &path])
            .current_dir(&self.repo_root)
            .output();
        let _ = fs::remove_dir_all(&self.path);
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.repo_root)
            .output();
    }
}

fn first_parent_or_empty(repo_root: &Path, commit: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", &format!("{commit}^")])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve first parent")?;
    if !output.status.success() {
        return Ok(String::new());
    }
    let stdout = String::from_utf8(output.stdout).context("git stdout is not UTF-8")?;
    Ok(stdout.trim_end().to_owned())
}

fn resolve_root(repo_root: Option<&Path>) -> Result<PathBuf> {
    let path = match repo_root {
        Some(path) => path.to_path_buf(),
        None => find_repository_root().unwrap_or_else(semantic_judge::default_repository_root),
    };
    if !path.is_dir() {
        bail!("repository root does not exist: {}", path.display());
    }
    Ok(path)
}

fn find_repository_root() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        if current.join(".git").exists() && current.join("Cargo.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn create_temp_dir(label: &str) -> Result<PathBuf> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "loop-engine-{label}-{}-{nanos}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create temporary directory {}", path.display()))?;
    Ok(path)
}

fn git_run(repo_root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    Ok(())
}

fn git_output_trimmed(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("git stdout is not UTF-8")?;
    Ok(stdout.trim_end().to_owned())
}

fn lines_nonempty(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_push_update_line_reads_four_fields() {
        let update = parse_push_update_line(
            "refs/heads/main abcdef0123456789012345678901234567890aaa refs/heads/main 0000000000000000000000000000000000000000",
        )
        .unwrap();
        assert_eq!(update.local_ref, "refs/heads/main");
        assert!(is_zero_oid(&update.remote_sha));
        assert!(!is_zero_oid(&update.local_sha));
    }

    #[test]
    fn zero_oid_requires_all_zeros() {
        assert!(is_zero_oid("0000000000000000000000000000000000000000"));
        assert!(!is_zero_oid("0000000000000000000000000000000000000001"));
        assert!(!is_zero_oid(""));
    }
}
