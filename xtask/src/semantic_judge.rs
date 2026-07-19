//! Semantic-judge runner (T024).
//!
//! Builds exact parent/candidate requests in Rust, invokes the configured generic
//! judge executable, validates the response schema and revision binding, and maps
//! local vs publication dispositions. Product runtime has no dependency on this
//! module.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Foundation commit that consumed the one-time bootstrap publication exception.
pub const FOUNDATION_PARENT_REVISION: &str = "7552af5968b4a2c10aefd01fbfa6c351817e1b8b";
const FOUNDATION_SEED_ID: &str = "foundation-seed";
const FOUNDATION_SEED_SHA256: &str =
    "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd";
const MANIFEST_PATH: &str = "quality/rubrics/manifest.json";
const RUBRICS_DIR: &str = "quality/rubrics";
const DEFAULT_EXECUTABLE_RELATIVE: &str = "quality/semantic-judge/v1/judge";
const DEFAULT_TIMEOUT_SECONDS: u64 = 900;
const EXECUTABLE_ENV: &str = "LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE";

const FOUNDATION_SEED_SOURCES: &[FoundationSeedSource] = &[
    FoundationSeedSource {
        path: "docs/invariants.md",
        section_header: "### I47. Every commit is documentation-coherent",
        rubric_header: "## docs/invariants.md — I47",
        blob_sha256: "8034714761107b669b5e5c9ab2941d257b5a69e562221d7e4dbb58db06b82b28",
    },
    FoundationSeedSource {
        path: "docs/testing.md",
        section_header: "## Git enforcement direction",
        rubric_header: "## docs/testing.md — Git enforcement direction",
        blob_sha256: "204ccaab4a5f44f578f256b4b5dc4ba851febf0155ce8bc87c8c267a0d3a4037",
    },
    FoundationSeedSource {
        path: "docs/tenets.md",
        section_header: "## 27. Documentation evolves with every commit",
        rubric_header: "## docs/tenets.md — 27. Documentation evolves with every commit",
        blob_sha256: "f2cb60c8cd68087b94ca284b901a36909f74367f8378d01885275ad341503fe4",
    },
    FoundationSeedSource {
        path: "docs/architecture.md",
        section_header: "## Composition and enforcement",
        rubric_header: "## docs/architecture.md — Composition and enforcement",
        blob_sha256: "6bea0ef07491ceaa68158f90ce1162bf9778ae40560fcc18cc32baa187420633",
    },
];

struct FoundationSeedSource {
    path: &'static str,
    section_header: &'static str,
    rubric_header: &'static str,
    blob_sha256: &'static str,
}

/// Judge disposition mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Local,
    Publication,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Publication => "publication",
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "local" => Ok(Self::Local),
            "publication" => Ok(Self::Publication),
            other => bail!("unsupported semantic-judge mode `{other}`"),
        }
    }
}

/// Judge verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Indeterminate,
    Unavailable,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Indeterminate => "indeterminate",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Local/publication disposition after mapping a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Allow,
    WarnAllow,
    Block,
}

/// Options for a single judge invocation.
#[derive(Debug, Clone)]
pub struct JudgeOptions {
    pub repo_root: PathBuf,
    pub mode: Mode,
    pub executable: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
    /// Explicit second-bootstrap claim. Always rejected.
    pub claim_bootstrap_exception: bool,
    /// Git repository used to load immutable foundation source blobs.
    /// Defaults to `repo_root` when unset; tests may point at the real loop-engine
    /// repository while judging a hermetic temporary repository.
    pub foundation_git_root: Option<PathBuf>,
    /// Additional deterministic evidence (for example quality command output).
    pub extra_deterministic_evidence: Vec<Value>,
}

impl JudgeOptions {
    pub fn new(repo_root: impl Into<PathBuf>, mode: Mode) -> Self {
        Self {
            repo_root: repo_root.into(),
            mode,
            executable: None,
            timeout_seconds: None,
            claim_bootstrap_exception: false,
            foundation_git_root: None,
            extra_deterministic_evidence: Vec::new(),
        }
    }
}

/// Validated judge response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeResponse {
    pub schema_version: u32,
    pub parent_revision: Option<String>,
    pub candidate_revision: Option<String>,
    pub verdict: Verdict,
    pub citations: Vec<Citation>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub rubric_id: String,
    pub rule: String,
    pub lines: Vec<String>,
}

/// Outcome of one judge attempt including disposition mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeOutcome {
    pub request: Value,
    pub response: JudgeResponse,
    pub disposition: Disposition,
    pub warning: Option<String>,
}

/// One commit judgment inside an unpublished range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeCommitOutcome {
    pub parent_revision: String,
    pub candidate_revision: String,
    pub outcome: JudgeOutcome,
}

/// Result of judging every commit in an unpublished range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeOutcome {
    pub commits: Vec<RangeCommitOutcome>,
}

/// Repository root containing this crate's workspace.
pub fn default_repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest directory should have a parent")
        .to_path_buf()
}

/// Map a verdict to local/publication disposition. Never silently upgrades to pass.
pub fn disposition_for(mode: Mode, verdict: Verdict) -> Disposition {
    match (mode, verdict) {
        (_, Verdict::Pass) => Disposition::Allow,
        (_, Verdict::Fail) => Disposition::Block,
        (Mode::Local, Verdict::Indeterminate | Verdict::Unavailable) => Disposition::WarnAllow,
        (Mode::Publication, Verdict::Indeterminate | Verdict::Unavailable) => Disposition::Block,
    }
}

/// Reject any attempt to claim a second bootstrap publication exception.
pub fn reject_second_bootstrap(claim: bool) -> Result<()> {
    if claim {
        bail!(
            "bootstrap publication exception was consumed by foundation commit {FOUNDATION_PARENT_REVISION}; no second bootstrap is permitted"
        );
    }
    Ok(())
}

/// CLI arguments for `xtask judge`.
#[derive(Debug, Clone)]
pub struct RunArgs<'a> {
    pub repo_root: Option<&'a Path>,
    pub staged: bool,
    pub parent: Option<&'a str>,
    pub candidate: Option<&'a str>,
    pub unpublished_from: Option<&'a str>,
    pub mode: Mode,
    pub executable: Option<&'a Path>,
    pub timeout_seconds: Option<u64>,
    pub claim_bootstrap_exception: bool,
}

/// Run `xtask judge` CLI entrypoints.
pub fn run(args: RunArgs<'_>) -> Result<()> {
    let root = args
        .repo_root
        .map(Path::to_path_buf)
        .unwrap_or_else(default_repository_root);
    let mut options = JudgeOptions::new(&root, args.mode);
    options.executable = args.executable.map(Path::to_path_buf);
    options.timeout_seconds = args.timeout_seconds;
    options.claim_bootstrap_exception = args.claim_bootstrap_exception;

    if args.staged {
        if args.parent.is_some() || args.candidate.is_some() || args.unpublished_from.is_some() {
            bail!("--staged cannot be combined with --parent/--candidate/--unpublished-from");
        }
        options.mode = Mode::Local;
        let outcome = judge_staged(&options)?;
        emit_outcome(&outcome)?;
        return disposition_to_result(outcome.disposition, &outcome);
    }

    if let Some(from) = args.unpublished_from {
        if args.parent.is_some() || args.candidate.is_some() {
            bail!("--unpublished-from cannot be combined with --parent/--candidate");
        }
        options.mode = Mode::Publication;
        let to = git_output_trimmed(&root, &["rev-parse", "HEAD"])?;
        let range = judge_unpublished_range(&options, from.trim(), to.trim())?;
        for commit in &range.commits {
            println!(
                "{}",
                serde_json::to_string(&commit.outcome.response)
                    .context("serialize range commit response")?
            );
            if let Some(warning) = &commit.outcome.warning {
                eprintln!("warning: {warning}");
            }
            if commit.outcome.disposition == Disposition::Block {
                bail!(
                    "publication blocked for {}..{} with verdict {}",
                    commit.parent_revision,
                    commit.candidate_revision,
                    commit.outcome.response.verdict.as_str()
                );
            }
        }
        return Ok(());
    }

    let parent = args
        .parent
        .context("--parent is required unless --staged or --unpublished-from")?;
    let candidate = args
        .candidate
        .context("--candidate is required unless --staged or --unpublished-from")?;
    let outcome = judge_revision_pair(&options, parent, candidate)?;
    emit_outcome(&outcome)?;
    disposition_to_result(outcome.disposition, &outcome)
}

fn emit_outcome(outcome: &JudgeOutcome) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&outcome.response).context("serialize judge response")?
    );
    if let Some(warning) = &outcome.warning {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn disposition_to_result(disposition: Disposition, outcome: &JudgeOutcome) -> Result<()> {
    match disposition {
        Disposition::Allow | Disposition::WarnAllow => Ok(()),
        Disposition::Block => bail!(
            "semantic judge blocked with verdict {}",
            outcome.response.verdict.as_str()
        ),
    }
}

/// Judge the exact staged index tree against `HEAD`.
pub fn judge_staged(options: &JudgeOptions) -> Result<JudgeOutcome> {
    reject_second_bootstrap(options.claim_bootstrap_exception)?;
    let request = build_exact_staged_request(options)?;
    invoke_and_map(options, request)
}

/// Judge an exact parent/candidate revision pair.
pub fn judge_revision_pair(
    options: &JudgeOptions,
    parent_revision: &str,
    candidate_revision: &str,
) -> Result<JudgeOutcome> {
    reject_second_bootstrap(options.claim_bootstrap_exception)?;
    if candidate_revision == FOUNDATION_PARENT_REVISION && options.mode == Mode::Publication {
        // Foundation already published under the consumed bootstrap exception.
        // Re-judging it is allowed, but claiming a fresh bootstrap is not.
    }
    let request = build_revision_pair_request(options, parent_revision, candidate_revision)?;
    invoke_and_map(options, request)
}

/// Enumerate and judge every commit in `from_exclusive..to_inclusive` (publication).
///
/// The first unpublished post-foundation range must receive determinate passes with
/// no second bootstrap exception.
pub fn judge_unpublished_range(
    options: &JudgeOptions,
    from_exclusive: &str,
    to_inclusive: &str,
) -> Result<RangeOutcome> {
    reject_second_bootstrap(options.claim_bootstrap_exception)?;
    if options.mode != Mode::Publication {
        bail!("unpublished range judgment requires publication mode");
    }

    let commits = unpublished_commits(&options.repo_root, from_exclusive, to_inclusive)?;
    if commits.is_empty() {
        bail!("unpublished range {from_exclusive}..{to_inclusive} contains no commits to judge");
    }

    let mut outcomes = Vec::with_capacity(commits.len());

    for candidate in commits {
        if candidate == FOUNDATION_PARENT_REVISION {
            bail!(
                "refusing second bootstrap: foundation commit {FOUNDATION_PARENT_REVISION} is not eligible for unpublished-range bootstrap exception"
            );
        }
        let parent = resolve_first_parent(&options.repo_root, &candidate)?;
        let outcome = judge_revision_pair(options, &parent, &candidate)?;
        outcomes.push(RangeCommitOutcome {
            parent_revision: parent.clone(),
            candidate_revision: candidate.clone(),
            outcome,
        });
    }

    Ok(RangeOutcome { commits: outcomes })
}

/// List commit SHAs in `from_exclusive..to_inclusive` oldest-first.
pub fn unpublished_commits(
    repo_root: &Path,
    from_exclusive: &str,
    to_inclusive: &str,
) -> Result<Vec<String>> {
    let range = format!("{from_exclusive}..{to_inclusive}");
    let text = git_output_trimmed(repo_root, &["rev-list", "--reverse", &range])?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn invoke_and_map(options: &JudgeOptions, request: Value) -> Result<JudgeOutcome> {
    let parent = request
        .get("parent_revision")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let candidate = request
        .get("candidate_revision")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let timeout_seconds = request
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);

    let response = match invoke_executable(options, &request, timeout_seconds) {
        Ok(raw) => match parse_and_validate_response(&raw, &parent, &candidate, &request) {
            Ok(response) => response,
            Err(error) => unavailable_response(
                &parent,
                &candidate,
                format!("malformed judge response: {error:#}"),
            ),
        },
        Err(error) => unavailable_response(&parent, &candidate, format!("{error:#}")),
    };

    let disposition = disposition_for(options.mode, response.verdict);
    let warning = match disposition {
        Disposition::WarnAllow => Some(format!(
            "semantic judge {} (local warn; commit allowed): {}",
            response.verdict.as_str(),
            response.message
        )),
        Disposition::Allow | Disposition::Block => None,
    };

    Ok(JudgeOutcome {
        request,
        response,
        disposition,
        warning,
    })
}

fn unavailable_response(parent: &str, candidate: &str, message: String) -> JudgeResponse {
    JudgeResponse {
        schema_version: 1,
        parent_revision: non_empty(parent),
        candidate_revision: non_empty(candidate),
        verdict: Verdict::Unavailable,
        citations: Vec::new(),
        message,
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn invoke_executable(
    options: &JudgeOptions,
    request: &Value,
    timeout_seconds: u64,
) -> Result<String> {
    let executable = resolve_executable(options)?;
    let payload = serde_json::to_vec(request).context("serialize judge request")?;
    let timeout = Duration::from_secs(timeout_seconds.max(1));

    let mut child = Command::new(&executable)
        .current_dir(&options.repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn judge executable {}", executable.display()))?;

    let mut stdin = child
        .stdin
        .take()
        .context("judge executable stdin unavailable")?;
    // Pipe writes can block when a stalled judge never drains stdin. Keep the
    // caller thread free to enforce the same deadline over input, output, and
    // process completion.
    let stdin_handle = thread::spawn(move || stdin.write_all(&payload));

    let mut stdout_pipe = child
        .stdout
        .take()
        .context("judge executable stdout unavailable")?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .context("judge executable stderr unavailable")?;

    let stdout_handle = thread::spawn(move || {
        let mut buffer = Vec::new();
        stdout_pipe.read_to_end(&mut buffer).map(|_| buffer)
    });
    let stderr_handle = thread::spawn(move || {
        let mut buffer = Vec::new();
        stderr_pipe.read_to_end(&mut buffer).map(|_| buffer)
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .context("failed polling judge executable")?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdin_handle.join();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                bail!("judge executable timed out after {timeout_seconds}s");
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };

    stdin_handle
        .join()
        .map_err(|_| anyhow::anyhow!("judge stdin writer panicked"))?
        .context("failed writing judge request to stdin")?;
    let stdout_bytes = stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("judge stdout reader panicked"))?
        .context("failed reading judge stdout")?;
    let stderr_bytes = stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("judge stderr reader panicked"))?
        .context("failed reading judge stderr")?;

    let stdout = String::from_utf8(stdout_bytes).context("judge stdout is not valid UTF-8")?;
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    if !status.success() {
        bail!(
            "judge executable exited with status {status}; stderr: {}",
            stderr.trim()
        );
    }

    Ok(stdout)
}

fn resolve_executable(options: &JudgeOptions) -> Result<PathBuf> {
    if let Some(path) = &options.executable {
        return Ok(path.clone());
    }
    if let Ok(path) = std::env::var(EXECUTABLE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(options.repo_root.join(DEFAULT_EXECUTABLE_RELATIVE))
}

fn parse_and_validate_response(
    raw: &str,
    expected_parent: &str,
    expected_candidate: &str,
    request: &Value,
) -> Result<JudgeResponse> {
    let value: Value = serde_json::from_str(raw.trim()).context("response is not JSON")?;
    validate_response_schema(&value).context("response failed schema validation")?;

    let response: JudgeResponse =
        serde_json::from_value(value).context("response failed typed decode")?;

    validate_response_citations(&response, request)?;

    match (&response.parent_revision, &response.candidate_revision) {
        (Some(parent), Some(candidate)) => {
            if parent != expected_parent || candidate != expected_candidate {
                bail!(
                    "response revision binding mismatch: got parent={parent} candidate={candidate}, expected parent={expected_parent} candidate={expected_candidate}"
                );
            }
        }
        (None, None) => bail!(
            "response for a valid bound request must echo parent/candidate revisions, including unavailable verdicts"
        ),
        _ => bail!("response revision fields must bind atomically"),
    }

    Ok(response)
}

fn validate_response_citations(response: &JudgeResponse, request: &Value) -> Result<()> {
    if response.verdict == Verdict::Unavailable {
        return Ok(());
    }

    let rubrics = request["rubrics"]
        .as_array()
        .context("request rubrics must be array")?
        .iter()
        .filter_map(|rubric| {
            Some((
                rubric.get("id")?.as_str()?,
                rubric.get("content")?.as_str()?,
            ))
        })
        .collect::<HashMap<_, _>>();
    let resulting_docs = request
        .get("relevant_docs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|doc| Some((doc.get("path")?.as_str()?, doc.get("content")?.as_str()?)))
        .collect::<HashMap<_, _>>();
    let mut changed_paths = resulting_docs.keys().copied().collect::<HashSet<_>>();
    for line in request["diff"].as_str().unwrap_or_default().lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/")
            && let Some((left, right)) = rest.split_once(" b/")
        {
            changed_paths.insert(left);
            changed_paths.insert(right);
        }
    }

    for citation in &response.citations {
        let rubric_content = rubrics.get(citation.rubric_id.as_str()).with_context(|| {
            format!(
                "citation references unknown parent rubric_id `{}`",
                citation.rubric_id
            )
        })?;
        if !rubric_rule_exists(rubric_content, &citation.rule) {
            bail!(
                "citation rule `{}` is not an exact heading or identifier in parent rubric `{}`",
                citation.rule,
                citation.rubric_id
            );
        }
        for location in &citation.lines {
            let (path, line_number) = match location.rsplit_once(':') {
                Some((path, text))
                    if !path.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()) =>
                {
                    let number = text
                        .parse::<usize>()
                        .with_context(|| format!("invalid citation line number: {location}"))?;
                    if number == 0 {
                        bail!("citation line number must be positive: {location}");
                    }
                    (path, Some(number))
                }
                _ => (location.as_str(), None),
            };
            if Path::new(path).is_absolute()
                || Path::new(path)
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                bail!("invalid repository-relative citation path: {location}");
            }
            if !changed_paths.contains(path) {
                bail!("citation does not name a changed/resulting path: {location}");
            }
            if let Some(line_number) = line_number {
                let content = resulting_docs.get(path).with_context(|| {
                    format!("numbered citation requires resulting-document content: {location}")
                })?;
                let line_count = content.lines().count();
                if line_number > line_count {
                    bail!("citation line exceeds resulting document length: {location}");
                }
            }
        }
    }
    Ok(())
}

fn rubric_rule_exists(content: &str, rule: &str) -> bool {
    let rule = rule.trim().to_lowercase();
    if rule.is_empty() {
        return false;
    }
    let heading_match = content.lines().any(|line| {
        line.starts_with('#') && line.trim_start_matches('#').trim().to_lowercase() == rule
    });
    if heading_match {
        return true;
    }
    content
        .split(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-')))
        .filter(|token| {
            token.chars().next().is_some_and(char::is_uppercase)
                && token.chars().any(|ch| ch.is_ascii_digit())
        })
        .any(|token| token.to_lowercase() == rule)
}

fn build_exact_staged_request(options: &JudgeOptions) -> Result<Value> {
    let repo = &options.repo_root;
    let parent_revision = git_output_trimmed(repo, &["rev-parse", "HEAD"])?;
    let candidate_revision = git_output_trimmed(repo, &["write-tree"])?;
    let diff = git_output(repo, &["diff", "--cached", &parent_revision])?;
    let status_text = git_output(
        repo,
        &["diff", "--cached", "--name-status", &parent_revision],
    )?;
    let check = git_run(repo, &["diff", "--cached", "--check", &parent_revision])?;
    let (rubrics, rubric_evidence) = load_parent_rubrics(options, &parent_revision)?;

    let mut evidence = vec![json_evidence(
        format!("git diff --cached --check {parent_revision}"),
        check.status.code().unwrap_or(-1),
        &check.stdout,
        &check.stderr,
    )];
    evidence.extend(rubric_evidence);
    evidence.extend(options.extra_deterministic_evidence.clone());

    Ok(serde_json::json!({
        "schema_version": 1,
        "mode": options.mode.as_str(),
        "parent_revision": parent_revision,
        "candidate_revision": candidate_revision,
        "diff": diff,
        "relevant_docs": build_relevant_docs_from_status(repo, &status_text)?,
        "rubrics": rubrics,
        "deterministic_evidence": evidence,
        "timeout_seconds": options.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    }))
}

fn build_revision_pair_request(
    options: &JudgeOptions,
    parent_revision: &str,
    candidate_revision: &str,
) -> Result<Value> {
    let repo = &options.repo_root;
    let diff = git_output(repo, &["diff", parent_revision, candidate_revision])?;
    let status_text = git_output(
        repo,
        &["diff", "--name-status", parent_revision, candidate_revision],
    )?;
    let check = git_run(
        repo,
        &["diff", "--check", parent_revision, candidate_revision],
    )?;
    let (rubrics, rubric_evidence) = load_parent_rubrics(options, parent_revision)?;

    let mut evidence = vec![json_evidence(
        format!("git diff --check {parent_revision} {candidate_revision}"),
        check.status.code().unwrap_or(-1),
        &check.stdout,
        &check.stderr,
    )];
    evidence.extend(rubric_evidence);
    evidence.extend(options.extra_deterministic_evidence.clone());

    Ok(serde_json::json!({
        "schema_version": 1,
        "mode": options.mode.as_str(),
        "parent_revision": parent_revision,
        "candidate_revision": candidate_revision,
        "diff": diff,
        "relevant_docs": build_relevant_docs_from_revision_diff(
            repo,
            candidate_revision,
            &status_text,
        )?,
        "rubrics": rubrics,
        "deterministic_evidence": evidence,
        "timeout_seconds": options.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    }))
}

fn load_parent_rubrics(
    options: &JudgeOptions,
    parent_revision: &str,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let foundation_root = resolve_foundation_git_root(options)?;

    match git_show_revision(&options.repo_root, parent_revision, MANIFEST_PATH)? {
        None => {
            let rubric = compose_foundation_seed_rubric(&foundation_root)?;
            let evidence = foundation_seed_provenance_evidence(&foundation_root)?;
            Ok((vec![rubric], evidence))
        }
        Some(manifest_text) => {
            let manifest: Value =
                serde_json::from_str(&manifest_text).context("invalid parent manifest json")?;
            enforce_manifest_bootstrap_policy(&manifest)?;
            let rubrics =
                load_rubrics_from_parent_manifest(&options.repo_root, parent_revision, &manifest)?;
            Ok((rubrics, Vec::new()))
        }
    }
}

fn resolve_foundation_git_root(options: &JudgeOptions) -> Result<PathBuf> {
    if let Some(root) = &options.foundation_git_root {
        return Ok(root.clone());
    }
    if git_show_revision(
        &options.repo_root,
        FOUNDATION_PARENT_REVISION,
        "docs/invariants.md",
    )?
    .is_some()
    {
        return Ok(options.repo_root.clone());
    }
    let workspace = default_repository_root();
    if git_show_revision(&workspace, FOUNDATION_PARENT_REVISION, "docs/invariants.md")?.is_some() {
        return Ok(workspace);
    }
    bail!(
        "unable to locate immutable foundation revision {FOUNDATION_PARENT_REVISION} for seed rubric fallback"
    );
}

fn enforce_manifest_bootstrap_policy(manifest: &Value) -> Result<()> {
    let parent = manifest
        .get("parent_revision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if parent != FOUNDATION_PARENT_REVISION {
        bail!(
            "parent manifest parent_revision must be foundation parent {FOUNDATION_PARENT_REVISION}, got {parent:?}"
        );
    }

    let consumed = manifest
        .get("bootstrap_publication_consumed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let no_second = manifest
        .get("no_second_bootstrap")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !consumed || !no_second {
        bail!(
            "rejecting second bootstrap: parent manifest must declare bootstrap_publication_consumed=true and no_second_bootstrap=true"
        );
    }
    Ok(())
}

fn load_rubrics_from_parent_manifest(
    repo_root: &Path,
    parent_revision: &str,
    manifest: &Value,
) -> Result<Vec<Value>> {
    let entries = manifest
        .get("rubrics")
        .and_then(Value::as_array)
        .context("parent manifest rubrics must be a non-empty array")?;
    if entries.is_empty() {
        bail!("parent manifest rubrics must be a non-empty array");
    }

    let mut rubrics = Vec::with_capacity(entries.len());
    for entry in entries {
        let rubric_id = entry
            .get("id")
            .and_then(Value::as_str)
            .context("parent manifest rubric entry missing id")?;
        let content_path = entry
            .get("content_path")
            .and_then(Value::as_str)
            .with_context(|| format!("parent manifest rubric {rubric_id} missing content_path"))?;
        let repo_relative_path = format!("{RUBRICS_DIR}/{content_path}");
        let content = git_show_revision(repo_root, parent_revision, &repo_relative_path)?
            .with_context(|| {
                format!(
                    "parent revision {parent_revision} missing rubric content at {repo_relative_path}"
                )
            })?;
        if let Some(expected_digest) = entry.get("content_sha256").and_then(Value::as_str)
            && !expected_digest.is_empty()
        {
            let actual = sha256_hex(content.as_bytes());
            if actual != expected_digest {
                bail!(
                    "parent rubric {rubric_id} digest mismatch at {repo_relative_path}: expected {expected_digest}, got {actual}"
                );
            }
        }
        rubrics.push(serde_json::json!({
            "id": rubric_id,
            "content": content,
        }));
    }
    Ok(rubrics)
}

fn compose_foundation_seed_rubric(git_repo_root: &Path) -> Result<Value> {
    let mut parts = vec![
        "# Foundation seed rubric v1".to_owned(),
        String::new(),
        format!("Parent revision: `{FOUNDATION_PARENT_REVISION}`"),
        String::new(),
        "This rubric is frozen by T012. Focused rubric files under `quality/rubrics/*.md` apply only to commits after T025.".to_owned(),
        String::new(),
    ];

    for source in FOUNDATION_SEED_SOURCES {
        let source_blob =
            load_foundation_seed_source_blob(git_repo_root, source.path, source.blob_sha256)?;
        let section_body = extract_markdown_section(&source_blob, source.section_header)?;
        parts.push(source.rubric_header.to_owned());
        parts.push(String::new());
        parts.push(section_body);
        parts.push(String::new());
    }

    let content = format!("{}\n", parts.join("\n").trim_end_matches('\n'));
    let digest = sha256_hex(content.as_bytes());
    if digest != FOUNDATION_SEED_SHA256 {
        bail!(
            "composed foundation-seed digest mismatch: expected {FOUNDATION_SEED_SHA256}, got {digest}"
        );
    }
    Ok(serde_json::json!({
        "id": FOUNDATION_SEED_ID,
        "content": content,
    }))
}

fn foundation_seed_provenance_evidence(git_repo_root: &Path) -> Result<Vec<Value>> {
    let mut evidence = Vec::new();
    for source in FOUNDATION_SEED_SOURCES {
        let _ = load_foundation_seed_source_blob(git_repo_root, source.path, source.blob_sha256)?;
        let stdout = serde_json::json!({
            "path": source.path,
            "revision": FOUNDATION_PARENT_REVISION,
            "content_sha256": source.blob_sha256,
            "section_header": source.section_header,
            "rubric_header": source.rubric_header,
        });
        evidence.push(json_evidence(
            format!("git show {FOUNDATION_PARENT_REVISION}:{}", source.path),
            0,
            &serde_json::to_string(&stdout)?,
            "",
        ));
    }

    let compose_stdout = serde_json::json!({
        "rubric_id": FOUNDATION_SEED_ID,
        "content_sha256": FOUNDATION_SEED_SHA256,
        "source_revision": FOUNDATION_PARENT_REVISION,
        "source_paths": FOUNDATION_SEED_SOURCES.iter().map(|source| source.path).collect::<Vec<_>>(),
    });
    evidence.push(json_evidence(
        format!("compose foundation-seed rubric from {FOUNDATION_PARENT_REVISION} source blobs"),
        0,
        &serde_json::to_string(&compose_stdout)?,
        "",
    ));
    Ok(evidence)
}

fn load_foundation_seed_source_blob(
    git_repo_root: &Path,
    path: &str,
    expected_digest: &str,
) -> Result<String> {
    let content = git_show_revision(git_repo_root, FOUNDATION_PARENT_REVISION, path)?
        .with_context(|| {
            format!("foundation revision {FOUNDATION_PARENT_REVISION} missing source at {path}")
        })?;
    let actual = sha256_hex(content.as_bytes());
    if actual != expected_digest {
        bail!(
            "foundation source digest mismatch at {path}: expected {expected_digest}, got {actual}"
        );
    }
    Ok(content)
}

fn extract_markdown_section(text: &str, header: &str) -> Result<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let header_level = header.chars().take_while(|ch| *ch == '#').count();
    let mut start = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim_end_matches('\n') == header {
            start = Some(index + 1);
            break;
        }
    }
    let start =
        start.with_context(|| format!("foundation source section not found: {header:?}"))?;

    let mut body = String::new();
    for line in &lines[start..] {
        let stripped = line.trim_end_matches('\n');
        if stripped.starts_with('#') {
            let level = stripped.chars().take_while(|ch| *ch == '#').count();
            if level <= header_level {
                break;
            }
        }
        body.push_str(line);
    }
    Ok(body.trim_matches('\n').to_owned())
}

fn build_relevant_docs_from_status(repo_root: &Path, status_text: &str) -> Result<Vec<Value>> {
    let mut docs = Vec::new();
    for raw_line in status_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (status, path) = parse_name_status_line(line)?;
        if !path.ends_with(".md") || status.starts_with('D') {
            continue;
        }
        docs.push(serde_json::json!({
            "path": path,
            "content": staged_resulting_doc_content(repo_root, &status, &path)?,
        }));
    }
    Ok(docs)
}

fn build_relevant_docs_from_revision_diff(
    repo_root: &Path,
    candidate_revision: &str,
    status_text: &str,
) -> Result<Vec<Value>> {
    let mut docs = Vec::new();
    for raw_line in status_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (status, path) = parse_name_status_line(line)?;
        if !path.ends_with(".md") || status.starts_with('D') {
            continue;
        }
        let content = git_show_revision(repo_root, candidate_revision, &path)?
            .unwrap_or_else(|| "(see exact diff)".to_owned());
        docs.push(serde_json::json!({
            "path": path,
            "content": content,
        }));
    }
    Ok(docs)
}

fn staged_resulting_doc_content(repo_root: &Path, status: &str, path: &str) -> Result<String> {
    if status.starts_with('D') {
        return Ok("(deleted in staged candidate)".to_owned());
    }
    if status.starts_with('R')
        || status.starts_with('C')
        || status.starts_with('A')
        || status.starts_with('M')
        || status.starts_with('T')
    {
        return Ok(
            git_show_index(repo_root, path)?.unwrap_or_else(|| "(see exact diff)".to_owned())
        );
    }
    Ok("(see exact diff)".to_owned())
}

fn parse_name_status_line(line: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 2 {
        bail!("invalid name-status line: {line:?}");
    }
    let status = parts[0].to_owned();
    if status.starts_with('R') || status.starts_with('C') {
        if parts.len() < 3 {
            bail!("invalid rename/copy status line: {line:?}");
        }
        return Ok((status, parts[parts.len() - 1].to_owned()));
    }
    Ok((status, parts[1].to_owned()))
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
        bail!("unsupported merge commit {commit} in unpublished range (nonlinear history)");
    }
    Ok(first)
}
fn json_evidence(command: String, exit_code: i32, stdout: &str, stderr: &str) -> Value {
    serde_json::json!({
        "command": command,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
    })
}

fn git_output(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = git_run(repo_root, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            output.stderr.trim_end()
        );
    }
    Ok(output.stdout)
}

fn git_output_trimmed(repo_root: &Path, args: &[&str]) -> Result<String> {
    Ok(git_output(repo_root, args)?.trim_end().to_owned())
}

struct GitOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn git_run(repo_root: &Path, args: &[&str]) -> Result<GitOutput> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    Ok(GitOutput {
        status: output.status,
        stdout: String::from_utf8(output.stdout).context("git stdout is not UTF-8")?,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn git_show_revision(repo_root: &Path, revision: &str, path: &str) -> Result<Option<String>> {
    let output = git_run(repo_root, &["show", &format!("{revision}:{path}")])?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

fn git_show_index(repo_root: &Path, path: &str) -> Result<Option<String>> {
    let output = git_run(repo_root, &["show", &format!(":{path}")])?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

fn validate_response_schema(instance: &Value) -> Result<()> {
    let schema: Value = serde_json::from_str(RESPONSE_SCHEMA_JSON)
        .context("embedded response schema must parse")?;
    let mut errors = Vec::new();
    validate_node(instance, &schema, &schema, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "));
    }
}

fn validate_node(
    instance: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let schema = resolve_schema(schema, root);

    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            validate_node(instance, subschema, root, path, errors);
        }
        // JSON Schema applies `allOf` alongside sibling keywords. Continue so
        // root `type`, `required`, `properties`, and additional-property rules
        // cannot be skipped by a valid conditional branch.
    }

    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let mut matches = 0usize;
        let mut branch_errors = Vec::new();
        for subschema in one_of {
            let mut candidate_errors = Vec::new();
            validate_node(instance, subschema, root, path, &mut candidate_errors);
            if candidate_errors.is_empty() {
                matches += 1;
            } else {
                branch_errors.extend(candidate_errors);
            }
        }
        if matches != 1 {
            errors.push(format!(
                "{path}: expected exactly one oneOf branch to match (matched {matches})"
            ));
            if matches == 0 {
                errors.extend(branch_errors);
            }
        }
        return;
    }

    if let Some(if_schema) = schema.get("if") {
        let mut if_errors = Vec::new();
        validate_node(instance, if_schema, root, path, &mut if_errors);
        let branch = if if_errors.is_empty() {
            schema.get("then")
        } else {
            schema.get("else")
        };
        if let Some(branch) = branch {
            validate_node(instance, branch, root, path, errors);
        }
        return;
    }

    if let Some(const_value) = schema.get("const") {
        if instance != const_value {
            errors.push(format!("{path}: expected const {const_value}"));
        }
        return;
    }

    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|value| value == instance) {
            errors.push(format!("{path}: value not in enum"));
        }
        return;
    }

    if let Some(expected_type) = schema.get("type").and_then(Value::as_str)
        && !value_matches_type(instance, expected_type)
    {
        errors.push(format!(
            "{path}: expected type {expected_type}, got {}",
            json_type_name(instance)
        ));
        return;
    }

    if let Some(min_length) = schema.get("minLength").and_then(Value::as_u64)
        && let Some(text) = instance.as_str()
        && (text.len() as u64) < min_length
    {
        errors.push(format!(
            "{path}: string shorter than minLength {min_length}"
        ));
    }

    if let Some(min_items) = schema.get("minItems").and_then(Value::as_u64)
        && let Some(items) = instance.as_array()
        && (items.len() as u64) < min_items
    {
        errors.push(format!("{path}: array shorter than minItems {min_items}"));
    }

    if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64)
        && let Some(items) = instance.as_array()
        && (items.len() as u64) > max_items
    {
        errors.push(format!("{path}: array longer than maxItems {max_items}"));
    }

    if schema.get("type").and_then(Value::as_str) == Some("object")
        || schema.get("properties").is_some()
    {
        validate_object(instance, schema, root, path, errors);
    }

    if schema.get("type").and_then(Value::as_str) == Some("array")
        && let Some(item_schema) = schema.get("items")
        && let Some(items) = instance.as_array()
    {
        for (index, item) in items.iter().enumerate() {
            validate_node(item, item_schema, root, &format!("{path}[{index}]"), errors);
        }
    }
}

fn validate_object(
    instance: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let Some(object) = instance.as_object() else {
        return;
    };

    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let Some(properties) = schema.get("properties").and_then(Value::as_object)
    {
        let allowed: HashSet<&str> = properties.keys().map(String::as_str).collect();
        for key in object.keys() {
            if !allowed.contains(key.as_str()) {
                errors.push(format!(
                    "{path}: additional property `{key}` is not allowed"
                ));
            }
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                errors.push(format!("{path}: missing required property `{key}`"));
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, property_schema) in properties {
            if let Some(value) = object.get(key) {
                validate_node(
                    value,
                    property_schema,
                    root,
                    &format!("{path}.{key}"),
                    errors,
                );
            }
        }
    }
}

fn resolve_schema<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let Some(target) = reference.strip_prefix("#/") else {
            panic!("unsupported $ref {reference}");
        };
        let mut current = root;
        for segment in target.split('/') {
            current = current
                .get(segment)
                .unwrap_or_else(|| panic!("missing $ref segment {segment} in {reference}"));
        }
        return current;
    }
    schema
}

fn value_matches_type(instance: &Value, expected_type: &str) -> bool {
    match expected_type {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "integer" => instance.as_i64().is_some(),
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "number" => instance.is_number(),
        _ => false,
    }
}

fn json_type_name(instance: &Value) -> &'static str {
    match instance {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) if instance.as_i64().is_some() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Product crates must not depend on the semantic-judge executable or schemas.
pub fn assert_product_runtime_has_no_judge_dependency(repo_root: &Path) -> Result<()> {
    let product_manifests = [
        "crates/loop-engine-core/Cargo.toml",
        "crates/loop-engine-integrations/Cargo.toml",
        "crates/loop-engine-cli/Cargo.toml",
    ];
    for relative in product_manifests {
        let path = repo_root.join(relative);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read product manifest {}", path.display()))?;
        for forbidden in [
            "semantic-judge",
            "semantic_judge",
            "quality/semantic-judge",
            "LOOP_ENGINE_SEMANTIC_JUDGE",
        ] {
            if text.contains(forbidden) {
                bail!(
                    "product runtime manifest {} must not reference semantic judge ({forbidden})",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha256::digest(bytes)
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Minimal SHA-256 (FIPS 180-4) for digest checks without adding crate deps.
mod sha256 {
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut state: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        let bit_len = (data.len() as u64).saturating_mul(8);
        let mut buffer = data.to_vec();
        buffer.push(0x80);
        while buffer.len() % 64 != 56 {
            buffer.push(0);
        }
        buffer.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in buffer.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (i, word) in chunk.chunks_exact(4).enumerate().take(16) {
                w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let mut a = state[0];
            let mut b = state[1];
            let mut c = state[2];
            let mut d = state[3];
            let mut e = state[4];
            let mut f = state[5];
            let mut g = state[6];
            let mut h = state[7];

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);

                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }

            state[0] = state[0].wrapping_add(a);
            state[1] = state[1].wrapping_add(b);
            state[2] = state[2].wrapping_add(c);
            state[3] = state[3].wrapping_add(d);
            state[4] = state[4].wrapping_add(e);
            state[5] = state[5].wrapping_add(f);
            state[6] = state[6].wrapping_add(g);
            state[7] = state[7].wrapping_add(h);
        }

        let mut out = [0u8; 32];
        for (index, word) in state.iter().enumerate() {
            out[index * 4..(index + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
}

// Embedded response schema subset authority: kept in sync with
// quality/semantic-judge/v1/response.schema.json (T023). The runner validates against
// this frozen shape so malformed executable output cannot silently pass.
const RESPONSE_SCHEMA_JSON: &str =
    include_str!("../../quality/semantic-judge/v1/response.schema.json");

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn disposition_matrix_matches_policy() {
        assert_eq!(
            disposition_for(Mode::Local, Verdict::Pass),
            Disposition::Allow
        );
        assert_eq!(
            disposition_for(Mode::Local, Verdict::Fail),
            Disposition::Block
        );
        assert_eq!(
            disposition_for(Mode::Local, Verdict::Indeterminate),
            Disposition::WarnAllow
        );
        assert_eq!(
            disposition_for(Mode::Local, Verdict::Unavailable),
            Disposition::WarnAllow
        );
        assert_eq!(
            disposition_for(Mode::Publication, Verdict::Pass),
            Disposition::Allow
        );
        assert_eq!(
            disposition_for(Mode::Publication, Verdict::Fail),
            Disposition::Block
        );
        assert_eq!(
            disposition_for(Mode::Publication, Verdict::Indeterminate),
            Disposition::Block
        );
        assert_eq!(
            disposition_for(Mode::Publication, Verdict::Unavailable),
            Disposition::Block
        );
    }

    #[test]
    fn second_bootstrap_claim_is_rejected() {
        let error = reject_second_bootstrap(true).expect_err("second bootstrap must fail");
        assert!(error.to_string().contains("no second bootstrap"));
        reject_second_bootstrap(false).expect("no claim should succeed");
    }

    #[test]
    fn response_schema_rejects_uncited_or_malformed_passes() {
        let parent = "parent";
        let candidate = "candidate";
        let valid = serde_json::json!({
            "schema_version": 1,
            "parent_revision": parent,
            "candidate_revision": candidate,
            "verdict": "pass",
            "citations": [{
                "rubric_id": "documentation",
                "rule": "coherence",
                "lines": ["docs/development-policy.md:1"]
            }],
            "message": "coherent"
        });
        validate_response_schema(&valid).expect("valid response");

        for invalid in [
            serde_json::json!({
                "schema_version": 2,
                "parent_revision": parent,
                "candidate_revision": candidate,
                "verdict": "pass",
                "citations": [{
                    "rubric_id": "",
                    "rule": "",
                    "lines": []
                }],
                "message": ""
            }),
            serde_json::json!({
                "schema_version": 1,
                "parent_revision": parent,
                "candidate_revision": candidate,
                "verdict": "pass",
                "citations": [],
                "message": "uncited"
            }),
            serde_json::json!({
                "schema_version": 1,
                "parent_revision": parent,
                "candidate_revision": candidate,
                "verdict": "pass",
                "citations": [{
                    "rubric_id": "documentation",
                    "rule": "coherence",
                    "lines": ["docs/development-policy.md:1"],
                    "extra": true
                }],
                "message": "extra property"
            }),
        ] {
            validate_response_schema(&invalid).expect_err("malformed pass must fail schema");
        }
    }

    #[test]
    fn numbered_citation_requires_resulting_document_content() {
        let request = serde_json::json!({
            "rubrics": [{"id": "foundation-seed", "content": "# I47"}],
            "relevant_docs": [],
            "diff": "diff --git a/xtask/src/lib.rs b/xtask/src/lib.rs\n"
        });
        let mut response = JudgeResponse {
            schema_version: 1,
            parent_revision: Some("parent".into()),
            candidate_revision: Some("candidate".into()),
            verdict: Verdict::Pass,
            citations: vec![Citation {
                rubric_id: "foundation-seed".into(),
                rule: "I47".into(),
                lines: vec!["xtask/src/lib.rs:999999".into()],
            }],
            message: "invalid locator".into(),
        };
        validate_response_citations(&response, &request)
            .expect_err("unverifiable numbered code citation must fail");
        response.citations[0].lines = vec!["xtask/src/lib.rs".into()];
        validate_response_citations(&response, &request)
            .expect("path-only changed-file citation is allowed by schema");
    }

    #[test]
    fn unavailable_response_must_echo_valid_request_binding() {
        let request = serde_json::json!({
            "rubrics": [{"id": "foundation-seed", "content": "# I47"}],
            "relevant_docs": [],
            "diff": ""
        });
        let raw = serde_json::json!({
            "schema_version": 1,
            "parent_revision": null,
            "candidate_revision": null,
            "verdict": "unavailable",
            "citations": [],
            "message": "unavailable"
        })
        .to_string();
        let error = parse_and_validate_response(&raw, "parent", "candidate", &request)
            .expect_err("unbound unavailable must be rejected for bound request");
        assert!(error.to_string().contains("must echo"));
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
