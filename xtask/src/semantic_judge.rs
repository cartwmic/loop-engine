//! Language-neutral semantic-judge v2 scheduling and contract validation.
//!
//! Repository configuration supplies one executable and four focused rubrics.
//! This module supplies canonical JSON over stdin/stdout, concurrent focused
//! execution, one bounded correction attempt, final coherence, and mechanical
//! fail-closed disposition.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::candidate::PreparedCandidate;
use crate::config::{Environment, Semantic, SemanticAxis};
use crate::git::{PRIVATE_STAGED_INDEX_ENVIRONMENT, ensure_success};
use crate::process::{
    self, CancellationHandle, EnvironmentChanges, ProcessOutcome, ProcessSpec,
    RegisteredRunningProcess,
};
use crate::quality::{CandidateBinding, DeterministicPhase, DeterministicResult};

pub const SCHEMA_VERSION: u32 = 2;
pub const FOCUSED_AXIS_COUNT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    Axis,
    Correction,
    Coherence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticStatus {
    Pass,
    Block,
    Indeterminate,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticDisposition {
    Pass,
    SemanticBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationKind {
    Rubric,
    Candidate,
    DeterministicEvidence,
    AxisResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub kind: CitationKind,
    pub reference: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedResult {
    pub id: String,
    pub status: SemanticStatus,
    pub citations: Vec<Citation>,
    pub message: String,
    pub attempts: Vec<AttemptRecord>,
    pub source_verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRecord {
    pub request_kind: RequestKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment: EnvironmentChanges,
    pub timeout_millis: u64,
    pub max_output_bytes: usize,
    pub scratch_root: PathBuf,
    pub process: ProcessOutcome,
    pub contract_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticResult {
    pub binding: CandidateBinding,
    pub axes: Vec<NormalizedResult>,
    pub coherence: NormalizedResult,
    pub disposition: SemanticDisposition,
    pub source_mutation: Option<String>,
}

impl SemanticResult {
    pub fn passed(&self) -> bool {
        self.disposition == SemanticDisposition::Pass
    }
}

#[derive(Debug, Clone, Serialize)]
struct Request {
    schema_version: u32,
    request_kind: RequestKind,
    axis_id: String,
    base_revision: String,
    candidate_revision: String,
    candidate_tree: String,
    rubric: RubricPayload,
    diff: BytePayload,
    resulting_files: Vec<ResultingFile>,
    deterministic_evidence: Value,
    axis_results: Vec<SharedResult>,
    correction: Option<CorrectionPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct RubricPayload {
    id: String,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct BytePayload {
    encoding: PayloadEncoding,
    data: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum PayloadEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    Base64,
}

#[derive(Debug, Clone, Serialize)]
struct ResultingFile {
    path: String,
    kind: ResultingFileKind,
    content: Option<BytePayload>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResultingFileKind {
    Regular,
    Symlink,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
struct SharedResult {
    id: String,
    status: SemanticStatus,
    citations: Vec<Citation>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct CorrectionPayload {
    original_request_kind: RequestKind,
    invalid_response: BytePayload,
    contract_error: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    schema_version: u32,
    request_kind: RequestKind,
    axis_id: String,
    base_revision: String,
    candidate_revision: String,
    candidate_tree: String,
    status: SemanticStatus,
    citations: Vec<Citation>,
    message: String,
}

#[derive(Debug, Clone)]
struct InvocationConfig {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    environment: EnvironmentChanges,
    timeout: Duration,
    max_output_bytes: usize,
}

struct SharedInput {
    binding: CandidateBinding,
    diff: BytePayload,
    resulting_files: Vec<ResultingFile>,
    deterministic_evidence: Value,
    deterministic_ids: BTreeSet<String>,
    candidate_references: BTreeSet<String>,
}

#[derive(Default)]
struct MutationCoordinator {
    state: Mutex<MutationState>,
    external: process::Cancellation,
}

#[derive(Default)]
struct MutationState {
    mutation: Option<String>,
    active: BTreeMap<usize, CancellationHandle>,
}

impl MutationCoordinator {
    fn new(external: process::Cancellation) -> Self {
        Self {
            state: Mutex::new(MutationState::default()),
            external,
        }
    }

    fn spawn(
        &self,
        slot: usize,
        spec: ProcessSpec,
    ) -> std::result::Result<RegisteredRunningProcess, String> {
        let mut state = lock(&self.state);
        if let Some(message) = &state.mutation {
            return Err(message.clone());
        }
        let running = process::spawn_with_cancellation(spec, &self.external)
            .ok_or_else(|| "external validation cancellation was already requested".to_owned())?;
        state.active.insert(slot, running.cancellation_handle());
        Ok(running)
    }

    fn unregister(&self, slot: usize) {
        lock(&self.state).active.remove(&slot);
    }

    fn report(&self, message: String) {
        let handles = {
            let mut state = lock(&self.state);
            if state.mutation.is_some() {
                return;
            }
            state.mutation = Some(message);
            state.active.values().cloned().collect::<Vec<_>>()
        };
        for handle in handles {
            handle.cancel();
        }
    }

    fn message(&self) -> Option<String> {
        lock(&self.state).mutation.clone()
    }
}

/// Run four focused axes concurrently, then one coherence invocation.
///
/// Callers run deterministic publication checks first. Binding mismatch and an
/// invalid semantic topology are configuration failures rather than model
/// results, so they return `Err` and no semantic child starts.
pub fn run(
    candidate: &PreparedCandidate,
    deterministic: &DeterministicResult,
) -> Result<SemanticResult> {
    run_with_cancellation(candidate, deterministic, &process::Cancellation::new())
}

/// Run semantic pipeline under caller-owned cancellation shared by every child.
pub fn run_with_cancellation(
    candidate: &PreparedCandidate,
    deterministic: &DeterministicResult,
    cancellation: &process::Cancellation,
) -> Result<SemanticResult> {
    if deterministic.phase != DeterministicPhase::Publication {
        bail!("semantic pipeline requires publication-phase deterministic evidence");
    }
    require_deterministic_binding(candidate, deterministic)?;
    if !deterministic.passed() {
        bail!("semantic pipeline requires a passing deterministic result");
    }
    let semantic = candidate
        .manifest()
        .manifest()
        .semantic()
        .context("semantic configuration is required")?;
    if semantic.axes().len() != FOCUSED_AXIS_COUNT {
        bail!(
            "semantic configuration must contain exactly {FOCUSED_AXIS_COUNT} focused axes; found {}",
            semantic.axes().len()
        );
    }

    if cancellation.is_cancelled() {
        bail!("semantic pipeline cancelled before setup");
    }
    validate_response_schema(candidate, semantic)?;
    let shared = Arc::new(build_shared_input(candidate, deterministic)?);
    let coordinator = Arc::new(MutationCoordinator::new(cancellation.clone()));
    let mut axis_results = thread::scope(|scope| {
        let mut joins = Vec::with_capacity(semantic.axes().len());
        for (index, axis) in semantic.axes().iter().enumerate() {
            let shared = Arc::clone(&shared);
            let coordinator = Arc::clone(&coordinator);
            joins.push((
                axis.id().to_owned(),
                scope.spawn(move || {
                    run_one(
                        candidate,
                        semantic,
                        axis,
                        RequestKind::Axis,
                        &[],
                        index,
                        &shared,
                        &coordinator,
                    )
                }),
            ));
        }
        joins
            .into_iter()
            .map(|(id, join)| {
                join.join().unwrap_or_else(|_| {
                    unavailable_for(&id, "semantic axis worker panicked", Vec::new(), None)
                })
            })
            .collect::<Vec<_>>()
    });
    require_complete_axes(semantic, &axis_results)?;

    let mutation = coordinator.message();
    if let Some(message) = &mutation {
        for result in &mut axis_results {
            if result.source_verified != Some(true) {
                let id = result.id.clone();
                *result = unavailable_for(
                    &id,
                    &format!("unavailable after candidate mutation: {message}"),
                    std::mem::take(&mut result.attempts),
                    result.source_verified,
                );
            }
        }
    }

    let coherence = if let Some(message) = &mutation {
        unavailable_for(
            semantic.coherence().id(),
            &format!("coherence suppressed after candidate mutation: {message}"),
            Vec::new(),
            None,
        )
    } else if cancellation.is_cancelled() {
        unavailable_for(
            semantic.coherence().id(),
            "coherence suppressed after external validation cancellation",
            Vec::new(),
            None,
        )
    } else {
        let shared_results = axis_results
            .iter()
            .map(|result| SharedResult {
                id: result.id.clone(),
                status: result.status,
                citations: result.citations.clone(),
                message: result.message.clone(),
            })
            .collect::<Vec<_>>();
        run_one(
            candidate,
            semantic,
            semantic.coherence(),
            RequestKind::Coherence,
            &shared_results,
            FOCUSED_AXIS_COUNT,
            &shared,
            &coordinator,
        )
    };

    let mutation = coordinator.message();
    let disposition = if mutation.is_none()
        && axis_results
            .iter()
            .all(|result| result.status == SemanticStatus::Pass)
        && coherence.status == SemanticStatus::Pass
    {
        SemanticDisposition::Pass
    } else {
        SemanticDisposition::SemanticBlock
    };

    Ok(SemanticResult {
        binding: shared.binding.clone(),
        axes: axis_results,
        coherence,
        disposition,
        source_mutation: mutation,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    candidate: &PreparedCandidate,
    semantic: &Semantic,
    axis: &SemanticAxis,
    initial_kind: RequestKind,
    axis_results: &[SharedResult],
    slot: usize,
    shared: &SharedInput,
    coordinator: &MutationCoordinator,
) -> NormalizedResult {
    let scratch_root = candidate
        .scratch_root()
        .join("semantic")
        .join(format!("invocation-{slot:02}"));
    if let Err(error) = fs::create_dir_all(&scratch_root) {
        return unavailable_for(
            axis.id(),
            &format!("failed creating semantic scratch root: {error}"),
            Vec::new(),
            None,
        );
    }
    let config = match invocation_config(candidate, semantic, &scratch_root) {
        Ok(config) => config,
        Err(error) => {
            return unavailable_for(
                axis.id(),
                &format!("invalid semantic process configuration: {error:#}"),
                Vec::new(),
                None,
            );
        }
    };
    let rubric = match read_rubric(candidate, axis) {
        Ok(rubric) => rubric,
        Err(error) => {
            return unavailable_for(
                axis.id(),
                &format!("failed reading semantic rubric: {error:#}"),
                Vec::new(),
                None,
            );
        }
    };
    let started = Instant::now();
    let request = Request {
        schema_version: SCHEMA_VERSION,
        request_kind: initial_kind,
        axis_id: axis.id().to_owned(),
        base_revision: shared.binding.base_revision.clone(),
        candidate_revision: shared.binding.candidate_revision.clone(),
        candidate_tree: shared.binding.candidate_tree.clone(),
        rubric,
        diff: shared.diff.clone(),
        resulting_files: shared.resulting_files.clone(),
        deterministic_evidence: shared.deterministic_evidence.clone(),
        axis_results: axis_results.to_vec(),
        correction: None,
    };

    let (first, first_bytes) = invoke(
        candidate,
        request,
        initial_kind,
        axis,
        axis_results,
        shared,
        &config,
        &scratch_root,
        slot,
        coordinator,
    );
    let mut attempts = first.attempt.into_iter().collect::<Vec<_>>();
    if first.source_mutated {
        return unavailable_for(
            axis.id(),
            "semantic process mutated candidate source",
            attempts,
            Some(false),
        );
    }
    if let Some(response) = first.response {
        return normalized(axis.id(), response, attempts, Some(true));
    }
    if !first.correctable || coordinator.message().is_some() {
        return unavailable_for(axis.id(), &first.error, attempts, first.source_verified);
    }

    let elapsed = started.elapsed();
    let Some(remaining) = config.timeout.checked_sub(elapsed) else {
        return unavailable_for(
            axis.id(),
            "semantic correction had no remaining timeout",
            attempts,
            Some(true),
        );
    };
    if remaining.is_zero() {
        return unavailable_for(
            axis.id(),
            "semantic correction had no remaining timeout",
            attempts,
            Some(true),
        );
    }
    let correction = Request {
        schema_version: SCHEMA_VERSION,
        request_kind: RequestKind::Correction,
        axis_id: axis.id().to_owned(),
        base_revision: shared.binding.base_revision.clone(),
        candidate_revision: shared.binding.candidate_revision.clone(),
        candidate_tree: shared.binding.candidate_tree.clone(),
        rubric: match read_rubric(candidate, axis) {
            Ok(rubric) => rubric,
            Err(error) => {
                return unavailable_for(
                    axis.id(),
                    &format!("failed rereading semantic rubric: {error:#}"),
                    attempts,
                    Some(true),
                );
            }
        },
        diff: shared.diff.clone(),
        resulting_files: shared.resulting_files.clone(),
        deterministic_evidence: shared.deterministic_evidence.clone(),
        axis_results: axis_results.to_vec(),
        correction: Some(CorrectionPayload {
            original_request_kind: initial_kind,
            invalid_response: BytePayload::from_bytes(&first_bytes),
            contract_error: first.error,
        }),
    };
    let mut correction_config = config;
    correction_config.timeout = remaining;
    let (second, _) = invoke(
        candidate,
        correction,
        RequestKind::Correction,
        axis,
        axis_results,
        shared,
        &correction_config,
        &scratch_root,
        slot,
        coordinator,
    );
    attempts.extend(second.attempt);
    if second.source_mutated {
        return unavailable_for(
            axis.id(),
            "semantic correction mutated candidate source",
            attempts,
            Some(false),
        );
    }
    match second.response {
        Some(response) => normalized(axis.id(), response, attempts, Some(true)),
        None => unavailable_for(axis.id(), &second.error, attempts, second.source_verified),
    }
}

struct InvocationResult {
    response: Option<Response>,
    error: String,
    correctable: bool,
    source_mutated: bool,
    source_verified: Option<bool>,
    attempt: Option<AttemptRecord>,
}

#[allow(clippy::too_many_arguments)]
fn invoke(
    candidate: &PreparedCandidate,
    request: Request,
    expected_kind: RequestKind,
    axis: &SemanticAxis,
    axis_results: &[SharedResult],
    shared: &SharedInput,
    config: &InvocationConfig,
    scratch_root: &Path,
    slot: usize,
    coordinator: &MutationCoordinator,
) -> (InvocationResult, Vec<u8>) {
    let stdin = match serde_json::to_vec(&request) {
        Ok(bytes) => bytes,
        Err(error) => {
            let outcome = process::execute(ProcessSpec::new(
                "",
                vec![],
                candidate.source_root(),
                candidate.source_root(),
                Duration::from_secs(1),
                1,
            ));
            return (
                InvocationResult {
                    response: None,
                    error: format!("failed serializing semantic request: {error}"),
                    correctable: false,
                    source_mutated: false,
                    source_verified: None,
                    attempt: Some(attempt_record(
                        expected_kind,
                        config,
                        scratch_root,
                        outcome,
                        Some(format!("request serialization failed: {error}")),
                    )),
                },
                Vec::new(),
            );
        }
    };
    let spec = ProcessSpec::new(
        config.program.clone(),
        config.args.clone(),
        candidate.source_root(),
        &config.cwd,
        config.timeout,
        config.max_output_bytes,
    )
    .with_environment(config.environment.clone())
    .with_stdin(stdin);
    let running = match coordinator.spawn(slot, spec) {
        Ok(running) => running,
        Err(message) => {
            return (
                InvocationResult {
                    response: None,
                    error: format!("semantic child suppressed after candidate mutation: {message}"),
                    correctable: false,
                    source_mutated: false,
                    source_verified: None,
                    attempt: None,
                },
                Vec::new(),
            );
        }
    };
    let outcome = running.await_completion();
    coordinator.unregister(slot);
    let stdout = outcome.stdout.exact_bytes().to_vec();

    let verification = candidate.verify_unchanged();
    let source_mutated = verification.is_err();
    if let Err(error) = verification {
        coordinator.report(format!(
            "candidate verification after `{}` failed: {error:#}",
            axis.id()
        ));
    }

    let (response, error, correctable) = if !outcome.success() {
        (
            None,
            format!(
                "semantic process did not succeed: {:?}",
                outcome.termination
            ),
            false,
        )
    } else {
        match parse_response(&stdout, expected_kind, axis, axis_results, shared) {
            Ok(response) => (Some(response), String::new(), false),
            Err(error) => (None, error, true),
        }
    };
    let contract_error = (!error.is_empty()).then(|| error.clone());
    (
        InvocationResult {
            response,
            error,
            correctable,
            source_mutated,
            source_verified: Some(!source_mutated),
            attempt: Some(attempt_record(
                expected_kind,
                config,
                scratch_root,
                outcome,
                contract_error,
            )),
        },
        stdout,
    )
}

fn attempt_record(
    request_kind: RequestKind,
    config: &InvocationConfig,
    scratch_root: &Path,
    process: ProcessOutcome,
    contract_error: Option<String>,
) -> AttemptRecord {
    AttemptRecord {
        request_kind,
        program: config.program.clone(),
        args: config.args.clone(),
        cwd: config.cwd.clone(),
        environment: config.environment.clone(),
        timeout_millis: u64::try_from(config.timeout.as_millis()).unwrap_or(u64::MAX),
        max_output_bytes: config.max_output_bytes,
        scratch_root: scratch_root.to_owned(),
        process,
        contract_error,
    }
}

fn parse_response(
    bytes: &[u8],
    expected_kind: RequestKind,
    axis: &SemanticAxis,
    axis_results: &[SharedResult],
    shared: &SharedInput,
) -> std::result::Result<Response, String> {
    let value = parse_json_without_duplicates(bytes)?;
    let response: Response = serde_json::from_value(value)
        .map_err(|error| format!("response does not match v2 contract: {error}"))?;
    if response.schema_version != SCHEMA_VERSION {
        return Err(format!("response schema_version must be {SCHEMA_VERSION}"));
    }
    if response.request_kind != expected_kind {
        return Err(format!(
            "response request_kind {:?} does not match {:?}",
            response.request_kind, expected_kind
        ));
    }
    if response.axis_id != axis.id() {
        return Err(format!(
            "response axis_id `{}` does not match `{}`",
            response.axis_id,
            axis.id()
        ));
    }
    if response.base_revision != shared.binding.base_revision
        || response.candidate_revision != shared.binding.candidate_revision
        || response.candidate_tree != shared.binding.candidate_tree
    {
        return Err("response revision/tree binding does not match request".to_owned());
    }
    if response.message.trim().is_empty() {
        return Err("response message must be non-empty".to_owned());
    }
    if response.status == SemanticStatus::Unavailable {
        if !response.citations.is_empty() {
            return Err("unavailable response must have no citations".to_owned());
        }
    } else if response.citations.is_empty() {
        return Err("pass/block/indeterminate response requires citations".to_owned());
    }
    let focused_ids = axis_results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<BTreeSet<_>>();
    for citation in &response.citations {
        if citation.reference.is_empty() || citation.detail.trim().is_empty() {
            return Err("citation reference and detail must be non-empty".to_owned());
        }
        let valid = match citation.kind {
            CitationKind::Rubric => citation.reference == axis.id(),
            CitationKind::Candidate => shared.candidate_references.contains(&citation.reference),
            CitationKind::DeterministicEvidence => {
                shared.deterministic_ids.contains(&citation.reference)
            }
            CitationKind::AxisResult => {
                expected_kind != RequestKind::Axis
                    && focused_ids.contains(citation.reference.as_str())
            }
        };
        if !valid {
            return Err(format!(
                "citation {:?} references unavailable input `{}`",
                citation.kind, citation.reference
            ));
        }
    }
    Ok(response)
}

fn normalized(
    id: &str,
    response: Response,
    attempts: Vec<AttemptRecord>,
    source_verified: Option<bool>,
) -> NormalizedResult {
    NormalizedResult {
        id: id.to_owned(),
        status: response.status,
        citations: response.citations,
        message: response.message,
        attempts,
        source_verified,
    }
}

fn unavailable_for(
    id: &str,
    message: &str,
    attempts: Vec<AttemptRecord>,
    source_verified: Option<bool>,
) -> NormalizedResult {
    NormalizedResult {
        id: id.to_owned(),
        status: SemanticStatus::Unavailable,
        citations: Vec::new(),
        message: message.to_owned(),
        attempts,
        source_verified,
    }
}

fn build_shared_input(
    candidate: &PreparedCandidate,
    deterministic: &DeterministicResult,
) -> Result<SharedInput> {
    let output = candidate.repository().output(
        [
            OsStr::new("diff"),
            OsStr::new("--binary"),
            OsStr::new("--no-ext-diff"),
            OsStr::new(candidate.base_revision()),
            OsStr::new(candidate.candidate_revision()),
            OsStr::new("--"),
        ],
        None,
    )?;
    ensure_success(&output, "git diff for semantic request")?;
    let resulting_files = resulting_files(candidate)?;
    let mut candidate_references = resulting_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    candidate_references.insert("diff".to_owned());
    let deterministic_ids = deterministic
        .prerequisites
        .iter()
        .chain(&deterministic.checks)
        .map(|record| record.id.clone())
        .collect();
    Ok(SharedInput {
        binding: CandidateBinding {
            base_revision: candidate.base_revision().to_owned(),
            candidate_revision: candidate.candidate_revision().to_owned(),
            candidate_tree: candidate.candidate_tree().to_owned(),
        },
        diff: BytePayload::from_bytes(&output.stdout),
        resulting_files,
        deterministic_evidence: serde_json::to_value(deterministic)
            .context("failed serializing deterministic evidence")?,
        deterministic_ids,
        candidate_references,
    })
}

fn resulting_files(candidate: &PreparedCandidate) -> Result<Vec<ResultingFile>> {
    candidate
        .changed_paths()
        .iter()
        .map(|relative| {
            let path = relative
                .to_str()
                .with_context(|| {
                    format!("semantic changed path is not UTF-8: {}", relative.display())
                })?
                .to_owned();
            let source = candidate.source_root().join(relative);
            match fs::symlink_metadata(&source) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let target = fs::read_link(&source)?;
                    let bytes = target.as_os_str().as_encoded_bytes();
                    Ok(ResultingFile {
                        path,
                        kind: ResultingFileKind::Symlink,
                        content: Some(BytePayload::from_bytes(bytes)),
                    })
                }
                Ok(metadata) if metadata.is_file() => Ok(ResultingFile {
                    path,
                    kind: ResultingFileKind::Regular,
                    content: Some(BytePayload::from_bytes(&fs::read(&source)?)),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ResultingFile {
                    path,
                    kind: ResultingFileKind::Deleted,
                    content: None,
                }),
                Ok(_) => bail!("unsupported resulting candidate entry `{path}`"),
                Err(error) => Err(error.into()),
            }
        })
        .collect()
}

impl BytePayload {
    fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self {
                encoding: PayloadEncoding::Utf8,
                data: text.to_owned(),
            },
            Err(_) => Self {
                encoding: PayloadEncoding::Base64,
                data: encode_base64(bytes),
            },
        }
    }
}

fn validate_response_schema(candidate: &PreparedCandidate, semantic: &Semantic) -> Result<()> {
    let root = candidate.source_root().canonicalize()?;
    let path = candidate.source_root().join(semantic.response_schema());
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "failed resolving semantic response schema `{}`",
            semantic.response_schema().display()
        )
    })?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        bail!(
            "semantic response schema must be a regular file beneath candidate root: {}",
            semantic.response_schema().display()
        );
    }
    let bytes = fs::read(&canonical)?;
    parse_json_without_duplicates(&bytes)
        .map_err(anyhow::Error::msg)
        .context("semantic response schema is not duplicate-free JSON")?;
    Ok(())
}

fn require_complete_axes(semantic: &Semantic, results: &[NormalizedResult]) -> Result<()> {
    if results.len() != FOCUSED_AXIS_COUNT {
        bail!("semantic scheduler produced missing focused results");
    }
    let configured = semantic
        .axes()
        .iter()
        .map(SemanticAxis::id)
        .collect::<BTreeSet<_>>();
    let produced = results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<BTreeSet<_>>();
    if produced.len() != results.len() || produced != configured {
        bail!("semantic scheduler produced duplicate, missing, or unknown focused results");
    }
    Ok(())
}

fn require_deterministic_binding(
    candidate: &PreparedCandidate,
    deterministic: &DeterministicResult,
) -> Result<()> {
    if deterministic.binding.base_revision != candidate.base_revision()
        || deterministic.binding.candidate_revision != candidate.candidate_revision()
        || deterministic.binding.candidate_tree != candidate.candidate_tree()
    {
        bail!("deterministic evidence binding does not match semantic candidate");
    }
    Ok(())
}

fn read_rubric(candidate: &PreparedCandidate, axis: &SemanticAxis) -> Result<RubricPayload> {
    let path = candidate.source_root().join(axis.rubric());
    let content = fs::read_to_string(&path).with_context(|| {
        format!(
            "rubric `{}` must be readable UTF-8",
            axis.rubric().display()
        )
    })?;
    if content.is_empty() {
        bail!("rubric `{}` must be non-empty", axis.rubric().display());
    }
    Ok(RubricPayload {
        id: axis.id().to_owned(),
        content,
    })
}

fn invocation_config(
    candidate: &PreparedCandidate,
    semantic: &Semantic,
    scratch_root: &Path,
) -> Result<InvocationConfig> {
    let expand = |value: &str| expand_value(candidate, scratch_root, value);
    let program = expand(semantic.program())?;
    let args = semantic
        .args()
        .iter()
        .map(|arg| expand(arg))
        .collect::<Result<Vec<_>>>()?;
    let cwd = PathBuf::from(expand(semantic.cwd())?);
    let environment = merge_environment(
        candidate.manifest().manifest().defaults().environment(),
        semantic.environment(),
        &expand,
    )?;
    let max_output_bytes = usize::try_from(semantic.max_output_bytes())
        .context("semantic max_output_bytes exceeds platform limit")?;
    Ok(InvocationConfig {
        program,
        args,
        cwd,
        environment,
        timeout: Duration::from_secs(semantic.timeout_seconds()),
        max_output_bytes,
    })
}

fn expand_value(candidate: &PreparedCandidate, scratch_root: &Path, input: &str) -> Result<String> {
    let mut output = String::new();
    let mut remainder = input;
    while let Some(start) = remainder.find('{') {
        output.push_str(&remainder[..start]);
        let rest = &remainder[start + 1..];
        let end = rest.find('}').context("unclosed semantic placeholder")?;
        let replacement = match &rest[..end] {
            "git_directory" => path_text(candidate.repository().git_directory())?,
            "candidate_root" => path_text(candidate.source_root())?,
            "scratch_root" => path_text(scratch_root)?,
            "cache_root" => path_text(candidate.cache_root())?,
            "target_root" => path_text(candidate.target_root())?,
            "base_revision" => candidate.base_revision(),
            "candidate_revision" => candidate.candidate_revision(),
            "candidate_tree" => candidate.candidate_tree(),
            name => bail!("unknown semantic placeholder `{{{name}}}`"),
        };
        output.push_str(replacement);
        remainder = &rest[end + 1..];
    }
    if remainder.contains('}') {
        bail!("unmatched closing brace in semantic value");
    }
    output.push_str(remainder);
    Ok(output)
}

fn merge_environment(
    defaults: &Environment,
    semantic: &Environment,
    expand: &impl Fn(&str) -> Result<String>,
) -> Result<EnvironmentChanges> {
    let mut set = defaults.set().clone();
    set.extend(semantic.set().clone());
    let mut unset = defaults
        .unset()
        .iter()
        .chain(semantic.unset())
        .cloned()
        .collect::<BTreeSet<_>>();
    unset.insert("GIT_INDEX_FILE".to_owned());
    unset.insert(PRIVATE_STAGED_INDEX_ENVIRONMENT.to_owned());
    for name in &unset {
        set.remove(name);
    }
    let set = set
        .into_iter()
        .map(|(name, value)| Ok((name, expand(&value)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(EnvironmentChanges::new(set, unset))
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("semantic path is not UTF-8: {}", path.display()))
}

fn parse_json_without_duplicates(bytes: &[u8]) -> std::result::Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = NoDuplicates::deserialize(&mut deserializer)
        .map_err(|error| format!("response is not one duplicate-free JSON document: {error}"))?
        .0;
    deserializer
        .end()
        .map_err(|error| format!("response has trailing data: {error}"))?;
    Ok(value)
}

struct NoDuplicates(Value);

impl<'de> Deserialize<'de> for NoDuplicates {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = NoDuplicates;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON value without duplicate object keys")
            }
            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::Bool(value)))
            }
            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::Number(value.into())))
            }
            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .map(NoDuplicates)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::String(value.to_owned())))
            }
            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::String(value)))
            }
            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::Null))
            }
            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(NoDuplicates(Value::Null))
            }
            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                NoDuplicates::deserialize(deserializer)
            }
            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<NoDuplicates>()? {
                    values.push(value.0);
                }
                Ok(NoDuplicates(Value::Array(values)))
            }
            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, NoDuplicates>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate object key `{key}`")));
                    }
                }
                Ok(NoDuplicates(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn mutation_observation_suppresses_all_barrier_released_late_starts() {
        let root = TempDir::new().unwrap();
        let coordinator = Arc::new(MutationCoordinator::new(process::Cancellation::new()));
        let barrier = Arc::new(Barrier::new(6));
        let reporter = {
            let coordinator = Arc::clone(&coordinator);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                coordinator.report("observed mutation".to_owned());
                barrier.wait();
            })
        };

        let ids = [
            "documentation",
            "observability",
            "architecture",
            "behavioral-evidence",
            "late-correction",
        ];
        let root_path = root.path().to_owned();
        let records = thread::scope(|scope| {
            let joins = ids
                .iter()
                .enumerate()
                .map(|(slot, id)| {
                    let coordinator = Arc::clone(&coordinator);
                    let barrier = Arc::clone(&barrier);
                    let candidate_root = root_path.clone();
                    let marker = candidate_root.join(format!("started-{slot}"));
                    scope.spawn(move || {
                        barrier.wait();
                        let spec = ProcessSpec::new(
                            "/usr/bin/touch",
                            vec![marker.to_string_lossy().into_owned()],
                            &candidate_root,
                            &candidate_root,
                            Duration::from_secs(2),
                            1024,
                        );
                        match coordinator.spawn(slot, spec) {
                            Ok(running) => {
                                let _ = running.await_completion();
                                unavailable_for(id, "unexpected late child start", Vec::new(), None)
                            }
                            Err(message) => unavailable_for(
                                id,
                                &format!("suppressed after candidate mutation: {message}"),
                                Vec::new(),
                                None,
                            ),
                        }
                    })
                })
                .collect::<Vec<_>>();
            joins
                .into_iter()
                .map(|join| join.join().unwrap())
                .collect::<Vec<_>>()
        });
        reporter.join().unwrap();

        assert_eq!(records.len(), ids.len());
        assert!(records.iter().all(|record| {
            record.status == SemanticStatus::Unavailable
                && record.attempts.is_empty()
                && record.message.contains("observed mutation")
        }));
        for slot in 0..ids.len() {
            assert!(!root.path().join(format!("started-{slot}")).exists());
        }
    }
}
