//! Aggregate publication / pre-push gate (R003; supersedes T028 scheduling).
//!
//! One accepted push is one publication checkpoint. The gate evaluates the exact
//! remote destination tip to candidate local head once: one aggregate diff, one
//! candidate-head worktree and quality run, and one base-rubric semantic request.
//! Internal commits are working history and are never separate quality or judge
//! boundaries. Unavailable, indeterminate, and fail all block publication.

use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::quality::{self, QualityManifest, QualityReport};
use crate::semantic_judge::{self, Disposition, JudgeOptions, JudgeOutcome, Mode};

/// One `git pre-push` stdin line: local ref/sha and remote ref/sha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushUpdate {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_ref: String,
    pub remote_sha: String,
}

/// Options for the aggregate publication gate.
#[derive(Debug, Clone)]
pub struct PublicationOptions {
    pub repo_root: PathBuf,
    pub judge_executable: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
    /// Explicit owner-authorized migration rubric for the one foundation-base
    /// aggregate policy transition. Never selected from candidate content implicitly.
    pub publication_migration_rubric: Option<PathBuf>,
    /// Optional Git root used to load immutable foundation seed blobs in tests.
    pub foundation_git_root: Option<PathBuf>,
    /// Test-only manifest override. Normal publication always loads the manifest
    /// from candidate head and compares it with the exact base revision.
    pub quality_manifest: Option<PathBuf>,
    /// Exact destination supplied by Git's pre-push hook. New branches require
    /// both values so their base comes from advertised `refs/heads/main`.
    pub remote_name: Option<String>,
    pub remote_url: Option<String>,
}

impl PublicationOptions {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            judge_executable: None,
            timeout_seconds: None,
            publication_migration_rubric: None,
            foundation_git_root: None,
            quality_manifest: None,
            remote_name: None,
            remote_url: None,
        }
    }
}

/// One base-to-head publication-checkpoint outcome.
#[derive(Debug, Clone)]
pub struct CheckpointPublicationOutcome {
    pub base_revision: String,
    pub candidate_revision: String,
    pub quality: QualityReport,
    pub judge: JudgeOutcome,
}

/// Aggregate publication outcome for one pushed ref.
#[derive(Debug, Clone)]
pub struct PublicationOutcome {
    pub checkpoint: CheckpointPublicationOutcome,
}

/// Revision-bound deterministic quality evidence transferable between trusted CI phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationQualityEvidence {
    pub schema_version: u32,
    pub base_revision: String,
    pub candidate_revision: String,
    pub quality: QualityReport,
}

enum CandidateQualityPlan {
    PreManifestBaseline,
    Manifest(QualityManifest, PathBuf),
}

/// Parse all `pre-push` remote-update lines from stdin.
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
    validate_push_oid(parts[1]).context("invalid local object ID")?;
    validate_push_oid(parts[3]).context("invalid remote object ID")?;
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

fn validate_push_oid(oid: &str) -> Result<()> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("object ID must be 40 or 64 hexadecimal characters");
    }
    Ok(())
}

/// Resolve the exact publication base for one pushed ref.
///
/// Existing refs use Git's supplied destination SHA. New branches use the exact
/// advertised `refs/heads/main` SHA from the named destination. Deletes return
/// `None` and need no content gate.
pub fn publication_base_for_update(
    repo_root: &Path,
    update: &PushUpdate,
    remote_name: Option<&str>,
    remote_url: Option<&str>,
) -> Result<Option<String>> {
    if is_zero_oid(&update.local_sha) {
        return Ok(None);
    }
    if !is_zero_oid(&update.remote_sha) {
        return Ok(Some(resolve_commit(repo_root, &update.remote_sha).with_context(|| {
            format!(
                "advertised destination tip {} is absent locally; fetch from the destination remote before pushing",
                update.remote_sha
            )
        })?));
    }

    let remote_name = remote_name
        .context("new-branch publication requires exact destination remote name from pre-push")?;
    validate_remote_name(remote_name)?;
    let remote_url = remote_url
        .context("new-branch publication requires exact destination remote URL from pre-push")?;
    let base = advertised_ref_oid(repo_root, remote_url, "refs/heads/main").with_context(|| {
        format!("failed resolving integration base from destination remote `{remote_name}`")
    })?;
    Ok(Some(resolve_commit(repo_root, &base).with_context(|| {
        format!(
            "advertised integration base {base} is absent locally; fetch/rebase from `{remote_name}` before pushing"
        )
    })?))
}

/// Run one aggregate checkpoint for the push's unique non-delete ref update.
///
/// Multi-content pushes are rejected: one accepted push is one unambiguous
/// destination-base-to-candidate-head publication checkpoint.
pub fn publish_updates(
    options: &PublicationOptions,
    updates: &[PushUpdate],
) -> Result<Vec<PublicationOutcome>> {
    let root = resolve_root(Some(&options.repo_root))?;
    let content_updates: Vec<&PushUpdate> = updates
        .iter()
        .filter(|update| !is_zero_oid(&update.local_sha))
        .collect();
    if content_updates.len() > 1 {
        bail!(
            "publication accepts at most one non-delete ref update per push; push refs separately"
        );
    }
    let Some(update) = content_updates.first() else {
        return Ok(Vec::new());
    };

    let checked_out_head = git_output_trimmed(&root, &["rev-parse", "HEAD"])?;
    let local_object = git_output_trimmed(&root, &["rev-parse", "--verify", &update.local_sha])
        .context("failed resolving exact pre-push local object")?;
    if local_object != checked_out_head {
        bail!(
            "pre-push local object {local_object} differs from checked-out HEAD {checked_out_head}; check out the pushed branch and push it separately"
        );
    }
    let head = resolve_commit(&root, &local_object)?;

    let Some(base) = publication_base_for_update(
        &root,
        update,
        options.remote_name.as_deref(),
        options.remote_url.as_deref(),
    )?
    else {
        return Ok(Vec::new());
    };
    if base == head {
        return Ok(Vec::new());
    }
    Ok(vec![publish_range(options, &base, &head)?])
}

/// Gate one exact `base_exclusive..candidate_head` publication checkpoint.
pub fn publish_range(
    options: &PublicationOptions,
    base_exclusive: &str,
    candidate_head: &str,
) -> Result<PublicationOutcome> {
    let root = resolve_root(Some(&options.repo_root))?;
    let base = resolve_commit(&root, base_exclusive)?;
    let head = resolve_commit(&root, candidate_head)?;
    if base == head {
        bail!("publication range {base}..{head} contains no candidate change");
    }
    ensure_linear_descendant_range(&root, &base, &head)?;

    with_user_tree_purity(&root, || {
        let checkpoint = gate_checkpoint(options, &root, &base, &head).with_context(|| {
            format!("publication blocked for aggregate checkpoint {base}..{head}")
        })?;
        Ok(PublicationOutcome { checkpoint })
    })
}

/// CLI entrypoint for `xtask publication`.
pub fn run_cli(
    repo_root: Option<&Path>,
    from_exclusive: Option<&str>,
    to_inclusive: Option<&str>,
    judge_executable: Option<&Path>,
    timeout_seconds: Option<u64>,
    quality_report_out: Option<&Path>,
    quality_report_in: Option<&Path>,
) -> Result<()> {
    let root = resolve_root(repo_root)?;
    let mut options = PublicationOptions::new(&root);
    options.judge_executable = judge_executable.map(Path::to_path_buf);
    options.publication_migration_rubric =
        std::env::var_os("LOOP_ENGINE_OWNER_MIGRATION_RUBRIC").map(PathBuf::from);
    options.timeout_seconds = timeout_seconds;

    if let Some(from) = from_exclusive {
        let to = match to_inclusive {
            Some(value) => value.to_owned(),
            None => git_output_trimmed(&root, &["rev-parse", "HEAD"])?,
        };
        if let Some(path) = quality_report_out {
            let evidence = produce_quality_evidence(&options, from, &to)?;
            write_quality_evidence(path, &evidence)?;
            println!(
                "publication quality ok {}..{} ({} checks)",
                evidence.base_revision,
                evidence.candidate_revision,
                evidence.quality.checks.len()
            );
            return Ok(());
        }
        if let Some(path) = quality_report_in {
            let evidence = read_quality_evidence(path)?;
            let outcome = publish_with_quality_evidence(&options, from, &to, evidence)?;
            emit_publication_outcome(&outcome);
            return Ok(());
        }
        let outcome = publish_range(&options, from, &to)?;
        emit_publication_outcome(&outcome);
        return Ok(());
    }
    if to_inclusive.is_some() || quality_report_out.is_some() || quality_report_in.is_some() {
        bail!("--to and quality-report phase options require --from");
    }

    let updates = read_push_updates_from_stdin()?;
    if updates.is_empty() {
        bail!("publication requires --from <rev> or pre-push remote-update lines on stdin");
    }
    let outcomes = publish_updates(&options, &updates)?;
    for outcome in &outcomes {
        emit_publication_outcome(outcome);
    }
    Ok(())
}

fn emit_publication_outcome(outcome: &PublicationOutcome) {
    let checkpoint = &outcome.checkpoint;
    println!(
        "publication ok {}..{} ({} quality checks, one aggregate judgment)",
        checkpoint.base_revision,
        checkpoint.candidate_revision,
        checkpoint.quality.checks.len()
    );
}

fn emit_judge_response(judge: &JudgeOutcome) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&judge.response).context("serialize publication judge response")?
    );
    Ok(())
}

/// Run only deterministic candidate quality and return revision-bound evidence.
pub fn produce_quality_evidence(
    options: &PublicationOptions,
    base_revision: &str,
    candidate_revision: &str,
) -> Result<PublicationQualityEvidence> {
    let root = resolve_root(Some(&options.repo_root))?;
    let base = resolve_commit(&root, base_revision)?;
    let candidate = resolve_commit(&root, candidate_revision)?;
    if base == candidate {
        bail!("publication range {base}..{candidate} contains no candidate change");
    }
    ensure_linear_descendant_range(&root, &base, &candidate)?;
    with_user_tree_purity(&root, || {
        enforce_revision_diff_check(&root, &base, &candidate)?;
        let quality = run_checkpoint_quality(options, &root, &base, &candidate)?;
        Ok(PublicationQualityEvidence {
            schema_version: 1,
            base_revision: base,
            candidate_revision: candidate,
            quality,
        })
    })
}

/// Run only aggregate semantic judgment using trusted prior quality evidence.
pub fn publish_with_quality_evidence(
    options: &PublicationOptions,
    base_revision: &str,
    candidate_revision: &str,
    evidence: PublicationQualityEvidence,
) -> Result<PublicationOutcome> {
    let root = resolve_root(Some(&options.repo_root))?;
    let base = resolve_commit(&root, base_revision)?;
    let candidate = resolve_commit(&root, candidate_revision)?;
    if base == candidate {
        bail!("publication range {base}..{candidate} contains no candidate change");
    }
    ensure_linear_descendant_range(&root, &base, &candidate)?;
    with_user_tree_purity(&root, || {
        enforce_revision_diff_check(&root, &base, &candidate)?;
        validate_quality_evidence(options, &root, &base, &candidate, &evidence)?;
        let checkpoint = judge_checkpoint(options, &root, &base, &candidate, evidence.quality)?;
        Ok(PublicationOutcome { checkpoint })
    })
}

fn with_user_tree_purity<T>(repo_root: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let before_head = git_output_trimmed(repo_root, &["rev-parse", "HEAD"])?;
    let before_status = git_output_trimmed(repo_root, &["status", "--porcelain"])?;
    let result = operation();
    let after_head = git_output_trimmed(repo_root, &["rev-parse", "HEAD"])?;
    let after_status = git_output_trimmed(repo_root, &["status", "--porcelain"])?;
    if before_head != after_head || before_status != after_status {
        let detail = result
            .err()
            .map(|error| format!("; gate also failed: {error:#}"))
            .unwrap_or_default();
        bail!(
            "publication gate must not rewrite user tree (HEAD/status changed during aggregate evaluation){detail}"
        );
    }
    result
}

fn write_quality_evidence(path: &Path, evidence: &PublicationQualityEvidence) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let bytes =
        serde_json::to_vec_pretty(evidence).context("serialize publication quality evidence")?;
    fs::write(path, bytes).with_context(|| format!("failed writing {}", path.display()))
}

fn read_quality_evidence(path: &Path) -> Result<PublicationQualityEvidence> {
    let bytes = fs::read(path).with_context(|| format!("failed reading {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid publication quality evidence {}", path.display()))
}

fn validate_quality_evidence(
    options: &PublicationOptions,
    repo_root: &Path,
    base_revision: &str,
    candidate_revision: &str,
    evidence: &PublicationQualityEvidence,
) -> Result<()> {
    if evidence.schema_version != 1
        || evidence.base_revision != base_revision
        || evidence.candidate_revision != candidate_revision
        || evidence.quality.candidate_revision != candidate_revision
    {
        bail!("publication quality evidence revision binding or schema mismatch");
    }
    if !evidence.quality.passed()
        || evidence
            .quality
            .checks
            .iter()
            .any(|check| check.evidence.candidate_revision != candidate_revision || !check.ok)
    {
        bail!("publication quality evidence contains failed or unbound checks");
    }

    let plan = resolve_candidate_quality_plan_from_git(
        options,
        repo_root,
        candidate_revision,
        base_revision,
    )?;
    let expected: Vec<(String, String)> = match plan {
        CandidateQualityPlan::PreManifestBaseline => Vec::new(),
        CandidateQualityPlan::Manifest(manifest, _) => manifest
            .checks
            .into_iter()
            .map(|check| (check.id, check.runner))
            .collect(),
    };
    let actual: Vec<(String, String)> = evidence
        .quality
        .checks
        .iter()
        .map(|check| (check.id.clone(), check.runner.clone()))
        .collect();
    if actual != expected {
        bail!("publication quality evidence check set does not match candidate manifest");
    }
    Ok(())
}

fn gate_checkpoint(
    options: &PublicationOptions,
    repo_root: &Path,
    base_revision: &str,
    candidate_revision: &str,
) -> Result<CheckpointPublicationOutcome> {
    enforce_revision_diff_check(repo_root, base_revision, candidate_revision)?;
    let quality_report =
        run_checkpoint_quality(options, repo_root, base_revision, candidate_revision)?;
    judge_checkpoint(
        options,
        repo_root,
        base_revision,
        candidate_revision,
        quality_report,
    )
}

fn run_checkpoint_quality(
    options: &PublicationOptions,
    repo_root: &Path,
    base_revision: &str,
    candidate_revision: &str,
) -> Result<QualityReport> {
    let worktree = DetachedWorktree::create(repo_root, candidate_revision)?;
    let plan = resolve_candidate_quality_plan(
        options,
        repo_root,
        &worktree.path,
        candidate_revision,
        base_revision,
    )?;
    match plan {
        CandidateQualityPlan::PreManifestBaseline => Ok(QualityReport {
            manifest_path: worktree.path.join(quality::MANIFEST_PATH),
            candidate_revision: candidate_revision.to_owned(),
            checks: Vec::new(),
        }),
        CandidateQualityPlan::Manifest(manifest, manifest_path) => quality::run_manifest(
            &worktree.path,
            &manifest,
            &manifest_path,
            candidate_revision,
        )
        .with_context(|| {
            format!(
                "manifest-driven quality checks failed for candidate head {candidate_revision} in detached worktree {}",
                worktree.path.display()
            )
        }),
    }
}

fn judge_checkpoint(
    options: &PublicationOptions,
    repo_root: &Path,
    base_revision: &str,
    candidate_revision: &str,
    quality_report: QualityReport,
) -> Result<CheckpointPublicationOutcome> {
    let mut judge_options = JudgeOptions::new(repo_root, Mode::Publication);
    judge_options.executable = options.judge_executable.clone();
    judge_options.timeout_seconds = options.timeout_seconds;
    judge_options.foundation_git_root = options.foundation_git_root.clone();
    judge_options.publication_migration_rubric = options.publication_migration_rubric.clone();
    judge_options.extra_deterministic_evidence = quality_report.deterministic_evidence();

    let judge =
        semantic_judge::judge_revision_pair(&judge_options, base_revision, candidate_revision)
            .unwrap_or_else(|error| {
                semantic_judge::unavailable_outcome(
                    Mode::Publication,
                    base_revision,
                    candidate_revision,
                    format!("failed constructing aggregate semantic request: {error:#}"),
                )
            });
    emit_judge_response(&judge)?;

    match judge.disposition {
        Disposition::Allow => Ok(CheckpointPublicationOutcome {
            base_revision: base_revision.to_owned(),
            candidate_revision: candidate_revision.to_owned(),
            quality: quality_report,
            judge,
        }),
        Disposition::WarnAllow => bail!(
            "internal error: publication disposition WarnAllow for {base_revision}..{candidate_revision} (verdict={:?})",
            judge.response.verdict
        ),
        Disposition::Block => bail!(
            "semantic judge blocked aggregate publication {base_revision}..{candidate_revision} (verdict={:?}): {}",
            judge.response.verdict,
            judge.response.message
        ),
    }
}

fn enforce_revision_diff_check(
    repo_root: &Path,
    base_revision: &str,
    candidate_revision: &str,
) -> Result<()> {
    let output = Command::new("git")
        .args(["diff", "--check", base_revision, candidate_revision])
        .current_dir(repo_root)
        .output()
        .context("failed to execute git diff --check")?;
    if !output.status.success() {
        bail!(
            "git diff --check failed for {base_revision}..{candidate_revision}: {}{}",
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
    base_revision: &str,
) -> Result<CandidateQualityPlan> {
    let plan = resolve_candidate_quality_plan_from_git(
        options,
        repo_root,
        candidate_revision,
        base_revision,
    )?;
    Ok(match plan {
        CandidateQualityPlan::PreManifestBaseline => CandidateQualityPlan::PreManifestBaseline,
        CandidateQualityPlan::Manifest(manifest, _) => {
            CandidateQualityPlan::Manifest(manifest, worktree_path.join(quality::MANIFEST_PATH))
        }
    })
}

fn resolve_candidate_quality_plan_from_git(
    options: &PublicationOptions,
    repo_root: &Path,
    candidate_revision: &str,
    base_revision: &str,
) -> Result<CandidateQualityPlan> {
    if let Some(override_path) = &options.quality_manifest {
        let manifest = quality::load_manifest(repo_root, Some(override_path))?;
        return Ok(CandidateQualityPlan::Manifest(
            manifest,
            override_path.clone(),
        ));
    }

    if let Some(manifest) = load_regular_manifest_blob(repo_root, candidate_revision)? {
        let base_manifest = quality::load_manifest_at_revision(repo_root, base_revision)?;
        quality::enforce_manifest_monotonic_evolution(base_manifest.as_ref(), &manifest)?;
        return Ok(CandidateQualityPlan::Manifest(
            manifest,
            PathBuf::from(quality::MANIFEST_PATH),
        ));
    }

    if quality::load_manifest_at_revision(repo_root, base_revision)?.is_some() {
        bail!(
            "quality manifest regression: candidate {candidate_revision} removed {MANIFEST_PATH} after base {base_revision} introduced it",
            MANIFEST_PATH = quality::MANIFEST_PATH
        );
    }
    Ok(CandidateQualityPlan::PreManifestBaseline)
}

fn load_regular_manifest_blob(repo_root: &Path, revision: &str) -> Result<Option<QualityManifest>> {
    let entry = git_output_trimmed(
        repo_root,
        &["ls-tree", revision, "--", quality::MANIFEST_PATH],
    )?;
    if entry.is_empty() {
        return Ok(None);
    }
    let (metadata, path) = entry
        .split_once('\t')
        .context("invalid quality manifest tree entry")?;
    let mut fields = metadata.split_whitespace();
    let mode = fields.next().unwrap_or_default();
    let kind = fields.next().unwrap_or_default();
    let _object = fields.next().unwrap_or_default();
    if fields.next().is_some()
        || mode != "100644"
        || kind != "blob"
        || path != quality::MANIFEST_PATH
    {
        bail!(
            "candidate quality manifest must be a regular 100644 Git blob at {}",
            quality::MANIFEST_PATH
        );
    }
    let text = git_output_trimmed(
        repo_root,
        &["show", &format!("{revision}:{}", quality::MANIFEST_PATH)],
    )?;
    quality::parse_manifest(&text)
        .context("invalid candidate quality manifest blob")
        .map(Some)
}

fn ensure_linear_descendant_range(repo_root: &Path, base: &str, head: &str) -> Result<()> {
    let ancestor = Command::new("git")
        .args(["merge-base", "--is-ancestor", base, head])
        .current_dir(repo_root)
        .status()
        .context("failed to verify publication ancestry")?;
    if !ancestor.success() {
        bail!(
            "candidate head {head} is not a fast-forward descendant of remote base {base}; fetch and rebase before publication"
        );
    }

    let merges = git_output_trimmed(
        repo_root,
        &["rev-list", "--merges", &format!("{base}..{head}")],
    )?;
    if let Some(commit) = merges.lines().find(|line| !line.trim().is_empty()) {
        bail!(
            "unsupported merge commit {} in publication range",
            commit.trim()
        );
    }
    Ok(())
}

fn validate_remote_name(remote_name: &str) -> Result<()> {
    if remote_name.is_empty()
        || !remote_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("invalid destination remote name `{remote_name}`");
    }
    Ok(())
}

fn advertised_ref_oid(repo_root: &Path, remote_url: &str, reference: &str) -> Result<String> {
    let output = Command::new("git")
        .args([
            "ls-remote",
            "--refs",
            "--end-of-options",
            remote_url,
            reference,
        ])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to query advertised ref `{reference}`"))?;
    if !output.status.success() {
        bail!(
            "git ls-remote failed for `{reference}`: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("git ls-remote stdout is not UTF-8")?;
    let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next().with_context(|| {
        format!("destination does not advertise required integration ref `{reference}`")
    })?;
    if lines.next().is_some() {
        bail!("destination advertised duplicate `{reference}` entries");
    }
    let mut fields = line.split_whitespace();
    let oid = fields.next().context("advertised ref omitted object id")?;
    let advertised_ref = fields.next().context("advertised ref omitted ref name")?;
    if advertised_ref != reference || fields.next().is_some() {
        bail!("malformed advertised ref line `{line}`");
    }
    Ok(oid.to_owned())
}

fn resolve_commit(repo_root: &Path, revision: &str) -> Result<String> {
    git_output_trimmed(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )
    .with_context(|| format!("failed resolving commit `{revision}`"))
}

struct DetachedWorktree {
    repo_root: PathBuf,
    path: PathBuf,
}

impl DetachedWorktree {
    fn create(repo_root: &Path, commit: &str) -> Result<Self> {
        let path = create_temp_dir("publication-checkpoint")?;
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
