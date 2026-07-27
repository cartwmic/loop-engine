//! Typed schema-v2 validation manifest and exact candidate-policy bindings.
//!
//! This module parses configuration only. Process execution and policy scheduling
//! belong to later runner layers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Schema version accepted by this parser.
pub const SCHEMA_VERSION: u32 = 2;

const PLACEHOLDERS: [&str; 8] = [
    "git_directory",
    "candidate_root",
    "scratch_root",
    "cache_root",
    "target_root",
    "base_revision",
    "candidate_revision",
    "candidate_tree",
];

/// Whether a caller requires publication-grade semantic configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticRequirement {
    /// Deterministic-only callers permit an absent `[semantic]` table.
    Optional,
    /// Publication and advisory callers require a complete `[semantic]` table.
    Required,
}

/// One deterministic validation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    PreCommit,
    Publication,
}

/// Input scope delivered to one configured check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    Repository,
    ChangedFiles,
}

/// Exact parsed document. Original bytes stay attached for binding computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDocument {
    manifest: Manifest,
    exact_bytes: Vec<u8>,
}

impl ManifestDocument {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn exact_bytes(&self) -> &[u8] {
        &self.exact_bytes
    }
}

/// Immutable schema-v2 repository validation configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    schema_version: u32,
    defaults: Defaults,
    runner: Runner,
    prerequisites: Vec<Prerequisite>,
    checks: Vec<Check>,
    semantic: Option<Semantic>,
}

impl Manifest {
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn defaults(&self) -> &Defaults {
        &self.defaults
    }

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn prerequisites(&self) -> &[Prerequisite] {
        &self.prerequisites
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    pub fn semantic(&self) -> Option<&Semantic> {
        self.semantic.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    timeout_seconds: u64,
    max_output_bytes: u64,
    environment: Environment,
}

impl Defaults {
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    unset: Vec<String>,
    set: BTreeMap<String, String>,
}

impl Environment {
    pub fn unset(&self) -> &[String] {
        &self.unset
    }

    pub fn set(&self) -> &BTreeMap<String, String> {
        &self.set
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runner {
    inputs: Vec<PathBuf>,
}

impl Runner {
    pub fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    id: String,
    program: String,
    args: Vec<String>,
    stdout_equals: Option<String>,
    install_hint: String,
}

impl Prerequisite {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn stdout_equals(&self) -> Option<&str> {
        self.stdout_equals.as_deref()
    }

    pub fn install_hint(&self) -> &str {
        &self.install_hint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    id: String,
    phases: Vec<Phase>,
    scope: Scope,
    program: String,
    args: Vec<String>,
    cwd: String,
    timeout_seconds: u64,
    max_output_bytes: u64,
    environment: Environment,
}

impl Check {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semantic {
    program: String,
    args: Vec<String>,
    cwd: String,
    timeout_seconds: u64,
    max_output_bytes: u64,
    response_schema: PathBuf,
    environment: Environment,
    axes: Vec<SemanticAxis>,
    coherence: SemanticAxis,
}

impl Semantic {
    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }

    pub fn response_schema(&self) -> &Path {
        &self.response_schema
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn axes(&self) -> &[SemanticAxis] {
        &self.axes
    }

    pub fn coherence(&self) -> &SemanticAxis {
        &self.coherence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAxis {
    id: String,
    rubric: PathBuf,
}

impl SemanticAxis {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rubric(&self) -> &Path {
        &self.rubric
    }
}

/// Exact candidate policy digest set. Rubric keys are repository-relative and sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingDigests {
    manifest_digest: String,
    rubric_digests: BTreeMap<PathBuf, String>,
}

impl BindingDigests {
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }

    pub fn rubric_digests(&self) -> &BTreeMap<PathBuf, String> {
        &self.rubric_digests
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u32,
    defaults: RawDefaults,
    runner: RawRunner,
    #[serde(default)]
    prerequisites: Vec<RawPrerequisite>,
    #[serde(default)]
    checks: Vec<RawCheck>,
    semantic: Option<RawSemantic>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    timeout_seconds: u64,
    max_output_bytes: u64,
    #[serde(default)]
    environment: RawEnvironment,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnvironment {
    #[serde(default)]
    unset: Vec<String>,
    #[serde(default)]
    set: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRunner {
    inputs: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrerequisite {
    id: String,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    stdout_equals: Option<String>,
    install_hint: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    id: String,
    phases: Vec<Phase>,
    scope: Scope,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: String,
    timeout_seconds: Option<u64>,
    max_output_bytes: Option<u64>,
    #[serde(default)]
    environment: RawEnvironment,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSemantic {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: String,
    timeout_seconds: Option<u64>,
    max_output_bytes: Option<u64>,
    response_schema: PathBuf,
    #[serde(default)]
    environment: RawEnvironment,
    #[serde(default)]
    axes: Vec<RawSemanticAxis>,
    coherence: RawSemanticAxis,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSemanticAxis {
    id: String,
    rubric: PathBuf,
}

/// Parse exact manifest bytes and retain them for later binding computation.
pub fn parse_manifest(
    bytes: &[u8],
    semantic_requirement: SemanticRequirement,
) -> Result<ManifestDocument> {
    let text = std::str::from_utf8(bytes).context("quality manifest must be UTF-8")?;
    let raw: RawManifest =
        toml::from_str(text).context("invalid schema-v2 quality manifest TOML")?;
    let manifest = validate_manifest(raw, semantic_requirement)?;
    Ok(ManifestDocument {
        manifest,
        exact_bytes: bytes.to_vec(),
    })
}

/// Read and parse a manifest without normalizing its exact bytes.
pub fn load_manifest(
    path: &Path,
    semantic_requirement: SemanticRequirement,
) -> Result<ManifestDocument> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read quality manifest at {}", path.display()))?;
    parse_manifest(&bytes, semantic_requirement)
        .with_context(|| format!("invalid quality manifest at {}", path.display()))
}

/// Compute all candidate-policy bindings through one exact-byte API.
///
/// Manifest bytes come from [`ManifestDocument`]. Every semantic rubric is read
/// exactly once from `candidate_root`; returned rubric keys use stable path order.
pub fn compute_binding(
    document: &ManifestDocument,
    candidate_root: &Path,
) -> Result<BindingDigests> {
    let manifest_digest = sha256_hex(document.exact_bytes());
    let mut rubric_digests = BTreeMap::new();
    let Some(semantic) = document.manifest().semantic() else {
        return Ok(BindingDigests {
            manifest_digest,
            rubric_digests,
        });
    };

    let canonical_root = candidate_root.canonicalize().with_context(|| {
        format!(
            "failed to resolve candidate root for binding: {}",
            candidate_root.display()
        )
    })?;
    let paths = semantic
        .axes()
        .iter()
        .chain(std::iter::once(semantic.coherence()))
        .map(SemanticAxis::rubric)
        .collect::<BTreeSet<_>>();

    for relative in paths {
        let source = candidate_root.join(relative);
        let canonical_source = source.canonicalize().with_context(|| {
            format!("failed to resolve configured rubric {}", relative.display())
        })?;
        if !canonical_source.starts_with(&canonical_root) {
            bail!(
                "configured rubric escapes candidate root: {}",
                relative.display()
            );
        }
        let bytes = fs::read(&canonical_source)
            .with_context(|| format!("failed to read configured rubric {}", relative.display()))?;
        rubric_digests.insert(relative.to_path_buf(), sha256_hex(&bytes));
    }

    Ok(BindingDigests {
        manifest_digest,
        rubric_digests,
    })
}

fn validate_manifest(
    raw: RawManifest,
    semantic_requirement: SemanticRequirement,
) -> Result<Manifest> {
    if raw.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported quality manifest schema_version {}; expected {}",
            raw.schema_version,
            SCHEMA_VERSION
        );
    }
    positive("defaults.timeout_seconds", raw.defaults.timeout_seconds)?;
    positive("defaults.max_output_bytes", raw.defaults.max_output_bytes)?;
    let default_environment =
        validate_environment(raw.defaults.environment, "defaults.environment")?;
    let defaults = Defaults {
        timeout_seconds: raw.defaults.timeout_seconds,
        max_output_bytes: raw.defaults.max_output_bytes,
        environment: default_environment,
    };

    if raw.runner.inputs.is_empty() {
        bail!("runner.inputs must contain at least one repository-relative path");
    }
    let mut input_paths = BTreeSet::new();
    for path in &raw.runner.inputs {
        validate_repository_path(path, "runner input")?;
        if !input_paths.insert(path.clone()) {
            bail!("duplicate runner input `{}`", path.display());
        }
    }
    let runner = Runner {
        inputs: raw.runner.inputs,
    };

    let mut command_ids = BTreeSet::new();
    let prerequisites = raw
        .prerequisites
        .into_iter()
        .map(|raw| {
            validate_id(&raw.id, "prerequisite")?;
            if !command_ids.insert(raw.id.clone()) {
                bail!("duplicate prerequisite/check id `{}`", raw.id);
            }
            validate_program_and_args(&raw.program, &raw.args, "prerequisite")?;
            if raw
                .stdout_equals
                .as_ref()
                .is_some_and(|expected| expected.contains('\r') || expected.contains('\n'))
            {
                bail!("prerequisite `{}` stdout_equals must be one line", raw.id);
            }
            if raw.install_hint.trim().is_empty() {
                bail!("prerequisite `{}` install_hint must be non-empty", raw.id);
            }
            Ok(Prerequisite {
                id: raw.id,
                program: raw.program,
                args: raw.args,
                stdout_equals: raw.stdout_equals,
                install_hint: raw.install_hint,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    if raw.checks.is_empty() {
        bail!("quality manifest must declare at least one [[checks]] entry");
    }
    let checks = raw
        .checks
        .into_iter()
        .map(|raw| {
            validate_id(&raw.id, "check")?;
            if !command_ids.insert(raw.id.clone()) {
                bail!("duplicate check id `{}`", raw.id);
            }
            if raw.phases.is_empty() {
                bail!("check `{}` phases must be non-empty", raw.id);
            }
            let mut phases = BTreeSet::new();
            for phase in &raw.phases {
                if !phases.insert(*phase) {
                    bail!("check `{}` has duplicate phase", raw.id);
                }
            }
            validate_program_and_args(&raw.program, &raw.args, "check")?;
            validate_candidate_cwd(&raw.cwd, &format!("check `{}` cwd", raw.id))?;
            let timeout_seconds = raw.timeout_seconds.unwrap_or(defaults.timeout_seconds());
            let max_output_bytes = raw.max_output_bytes.unwrap_or(defaults.max_output_bytes());
            positive("check timeout_seconds", timeout_seconds)?;
            positive("check max_output_bytes", max_output_bytes)?;
            let environment =
                validate_environment(raw.environment, &format!("check `{}` environment", raw.id))?;
            Ok(Check {
                id: raw.id,
                phases: raw.phases,
                scope: raw.scope,
                program: raw.program,
                args: raw.args,
                cwd: raw.cwd,
                timeout_seconds,
                max_output_bytes,
                environment,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    if semantic_requirement == SemanticRequirement::Required && raw.semantic.is_none() {
        bail!("semantic configuration is required for publication or advisory validation");
    }
    let semantic = raw
        .semantic
        .map(|raw| validate_semantic(raw, &defaults))
        .transpose()?;

    Ok(Manifest {
        schema_version: raw.schema_version,
        defaults,
        runner,
        prerequisites,
        checks,
        semantic,
    })
}

fn validate_semantic(raw: RawSemantic, defaults: &Defaults) -> Result<Semantic> {
    validate_program_and_args(&raw.program, &raw.args, "semantic")?;
    validate_candidate_cwd(&raw.cwd, "semantic cwd")?;
    validate_repository_path(&raw.response_schema, "semantic response_schema")?;
    let timeout_seconds = raw.timeout_seconds.unwrap_or(defaults.timeout_seconds());
    let max_output_bytes = raw.max_output_bytes.unwrap_or(defaults.max_output_bytes());
    positive("semantic timeout_seconds", timeout_seconds)?;
    positive("semantic max_output_bytes", max_output_bytes)?;
    let environment = validate_environment(raw.environment, "semantic environment")?;
    if raw.axes.is_empty() {
        bail!("semantic.axes must contain at least one axis");
    }

    let mut ids = BTreeSet::new();
    let mut rubrics = BTreeSet::new();
    let axes = raw
        .axes
        .into_iter()
        .map(|axis| validate_axis(axis, "semantic axis", &mut ids, &mut rubrics))
        .collect::<Result<Vec<_>>>()?;
    let coherence = validate_axis(raw.coherence, "semantic coherence", &mut ids, &mut rubrics)?;

    Ok(Semantic {
        program: raw.program,
        args: raw.args,
        cwd: raw.cwd,
        timeout_seconds,
        max_output_bytes,
        response_schema: raw.response_schema,
        environment,
        axes,
        coherence,
    })
}

fn validate_axis(
    raw: RawSemanticAxis,
    kind: &str,
    ids: &mut BTreeSet<String>,
    rubrics: &mut BTreeSet<PathBuf>,
) -> Result<SemanticAxis> {
    validate_id(&raw.id, kind)?;
    if !ids.insert(raw.id.clone()) {
        bail!("duplicate semantic axis/coherence id `{}`", raw.id);
    }
    validate_repository_path(&raw.rubric, &format!("{kind} rubric"))?;
    if !rubrics.insert(raw.rubric.clone()) {
        bail!("duplicate semantic rubric path `{}`", raw.rubric.display());
    }
    Ok(SemanticAxis {
        id: raw.id,
        rubric: raw.rubric,
    })
}

fn validate_environment(raw: RawEnvironment, context: &str) -> Result<Environment> {
    let mut unset = BTreeSet::new();
    for name in &raw.unset {
        validate_environment_name(name, context)?;
        if !unset.insert(name) {
            bail!("{context} contains duplicate unset variable `{name}`");
        }
    }
    for (name, value) in &raw.set {
        validate_environment_name(name, context)?;
        validate_placeholders(value)
            .with_context(|| format!("invalid placeholder in {context} value for `{name}`"))?;
    }
    Ok(Environment {
        unset: raw.unset,
        set: raw.set,
    })
}

fn validate_environment_name(name: &str, context: &str) -> Result<()> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        bail!("{context} has invalid environment variable name `{name}`");
    }
    Ok(())
}

fn validate_program_and_args(program: &str, args: &[String], kind: &str) -> Result<()> {
    if program.is_empty() {
        bail!("{kind} program must be non-empty");
    }
    validate_placeholders(program)
        .with_context(|| format!("invalid placeholder in {kind} program"))?;
    for (index, arg) in args.iter().enumerate() {
        validate_placeholders(arg)
            .with_context(|| format!("invalid placeholder in {kind} args[{index}]"))?;
    }
    Ok(())
}

fn validate_placeholders(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                let remainder = &value[index + 1..];
                let Some(end) = remainder.find('}') else {
                    bail!("unclosed placeholder in `{value}`");
                };
                let name = &remainder[..end];
                if !PLACEHOLDERS.contains(&name) {
                    bail!("unknown placeholder `{{{name}}}`");
                }
                index += end + 2;
            }
            b'}' => bail!("unmatched closing brace in `{value}`"),
            _ => index += 1,
        }
    }
    Ok(())
}

fn validate_candidate_cwd(value: &str, context: &str) -> Result<()> {
    validate_placeholders(value).with_context(|| format!("invalid {context}"))?;
    let Some(suffix) = value.strip_prefix("{candidate_root}") else {
        bail!("{context} must start at {{candidate_root}}");
    };
    if suffix.is_empty() {
        return Ok(());
    }
    let Some(relative) = suffix.strip_prefix('/') else {
        bail!("{context} must be {{candidate_root}} or a directory beneath it");
    };
    if relative.is_empty()
        || relative
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        bail!("{context} contains a path escape or non-normal component");
    }
    if relative.contains('{') || relative.contains('}') {
        bail!("{context} may not contain additional placeholders");
    }
    Ok(())
}

fn validate_repository_path(path: &Path, context: &str) -> Result<()> {
    let text = path
        .to_str()
        .with_context(|| format!("{context} must be valid UTF-8"))?;
    if text.is_empty()
        || text.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || text
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        bail!(
            "{context} must be a non-empty normalized repository-relative path: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_id(id: &str, kind: &str) -> Result<()> {
    if id.is_empty() || id.trim() != id {
        bail!("{kind} id must be non-empty and have no surrounding whitespace");
    }
    Ok(())
}

fn positive(field: &str, value: u64) -> Result<()> {
    if value == 0 {
        bail!("{field} must be positive");
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
