//! Generic sequential deterministic validation scheduler.
//!
//! Repository configuration owns every executable and argument. This module
//! expands the fixed manifest placeholders, applies typed environment changes,
//! invokes the project-neutral process executor, and records ordered evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::candidate::PreparedCandidate;
use crate::config::{Check, Environment, Phase, Prerequisite, Scope};
use crate::git::PRIVATE_STAGED_INDEX_ENVIRONMENT;
use crate::process::{self, EnvironmentChanges, ProcessOutcome, ProcessSpec};

/// Whether evidence was produced for a prerequisite or deterministic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Prerequisite,
    Check,
}

/// Scope retained in check evidence without coupling records to config decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandScope {
    Repository,
    ChangedFiles,
}

impl From<Scope> for CommandScope {
    fn from(value: Scope) -> Self {
        match value {
            Scope::Repository => Self::Repository,
            Scope::ChangedFiles => Self::ChangedFiles,
        }
    }
}

/// Phase retained in deterministic evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeterministicPhase {
    PreCommit,
    Publication,
}

impl From<Phase> for DeterministicPhase {
    fn from(value: Phase) -> Self {
        match value {
            Phase::PreCommit => Self::PreCommit,
            Phase::Publication => Self::Publication,
        }
    }
}

/// Mechanically derived state of one configured command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Passed,
    Failed,
    SkippedSuccess,
    Blocked,
}

/// Typed scheduler-level failure. Process-level categories stay in `process`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFailureKind {
    Configuration,
    Process,
    StdoutMismatch,
    CandidateMutation,
}

/// Scheduler failure detail retained beside exact command/process evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandFailure {
    pub kind: CommandFailureKind,
    pub message: String,
}

/// Exact Git-object binding for one deterministic run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateBinding {
    pub base_revision: String,
    pub candidate_revision: String,
    pub candidate_tree: String,
}

/// Declared value and its independent expansion result.
///
/// `expanded` is absent only when expansion failed. `error` then retains the
/// exact reason, while `declared` always preserves manifest evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueExpansion {
    pub declared: String,
    pub expanded: Option<String>,
    pub error: Option<String>,
}

/// Per-value environment expansion evidence after deterministic set/unset merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentExpansion {
    pub set: BTreeMap<String, ValueExpansion>,
    pub unset: BTreeSet<String>,
}

/// Ordered evidence for one prerequisite or phase-selected check.
#[derive(Debug, Clone, Serialize)]
pub struct CommandRecord {
    pub id: String,
    pub kind: CommandKind,
    pub scope: Option<CommandScope>,
    pub status: CommandStatus,
    /// Best available value: expanded when successful, otherwise declared.
    pub program: String,
    /// Best available values: expanded when successful, otherwise declared.
    pub args: Vec<String>,
    /// Best available value: expanded when successful, otherwise declared.
    pub cwd: PathBuf,
    /// Best available merged environment values.
    pub environment: EnvironmentChanges,
    pub program_expansion: ValueExpansion,
    pub args_expansion: Vec<ValueExpansion>,
    pub cwd_expansion: ValueExpansion,
    pub environment_expansion: EnvironmentExpansion,
    pub timeout_seconds: u64,
    pub max_output_bytes: u64,
    pub install_hint: Option<String>,
    pub process: Option<ProcessOutcome>,
    /// `None` means no child launched. Every launched child has `Some` evidence.
    pub source_verified: Option<bool>,
    pub failure: Option<CommandFailure>,
}

/// Final-neutral deterministic result consumed by validation/report callers.
#[derive(Debug, Clone, Serialize)]
pub struct DeterministicResult {
    pub phase: DeterministicPhase,
    pub binding: CandidateBinding,
    pub prerequisites: Vec<CommandRecord>,
    pub checks: Vec<CommandRecord>,
    pub final_source_verified: bool,
    pub final_failure: Option<CommandFailure>,
}

impl DeterministicResult {
    /// True only when all selected commands and final candidate verification pass.
    pub fn passed(&self) -> bool {
        self.final_source_verified
            && self.final_failure.is_none()
            && self.prerequisites.iter().chain(&self.checks).all(|record| {
                matches!(
                    record.status,
                    CommandStatus::Passed | CommandStatus::SkippedSuccess
                )
            })
    }
}

#[derive(Debug)]
struct ExpandedCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    environment: EnvironmentChanges,
    program_expansion: ValueExpansion,
    args_expansion: Vec<ValueExpansion>,
    cwd_expansion: ValueExpansion,
    environment_expansion: EnvironmentExpansion,
    expansion_errors: Vec<String>,
    timeout_seconds: u64,
    max_output_bytes: u64,
}

struct Expansion<'a> {
    candidate: &'a PreparedCandidate,
}

impl<'a> Expansion<'a> {
    fn new(candidate: &'a PreparedCandidate) -> Self {
        Self { candidate }
    }

    fn replacement(&self, name: &str) -> Result<&str, String> {
        match name {
            "git_directory" => {
                utf8_path(self.candidate.repository().git_directory(), "git directory")
            }
            "candidate_root" => utf8_path(self.candidate.source_root(), "candidate root"),
            "scratch_root" => utf8_path(self.candidate.scratch_root(), "scratch root"),
            "cache_root" => utf8_path(self.candidate.cache_root(), "cache root"),
            "target_root" => utf8_path(self.candidate.target_root(), "target root"),
            "base_revision" => Ok(self.candidate.base_revision()),
            "candidate_revision" => Ok(self.candidate.candidate_revision()),
            "candidate_tree" => Ok(self.candidate.candidate_tree()),
            _ => Err(format!("unknown placeholder `{{{name}}}`")),
        }
    }

    /// Expand exactly placeholders present in the original value. Replacement
    /// contents are never rescanned, so braces in filesystem paths stay literal.
    fn value(&self, input: &str) -> Result<String, String> {
        let mut output = String::with_capacity(input.len());
        let mut remainder = input;
        while let Some(start) = remainder.find('{') {
            output.push_str(&remainder[..start]);
            let after_open = &remainder[start + 1..];
            let end = after_open
                .find('}')
                .ok_or_else(|| format!("unclosed placeholder in `{input}`"))?;
            let name = &after_open[..end];
            output.push_str(self.replacement(name)?);
            remainder = &after_open[end + 1..];
        }
        if remainder.contains('}') {
            return Err(format!("unmatched closing brace in `{input}`"));
        }
        output.push_str(remainder);
        Ok(output)
    }
}

/// Run prerequisites first, then phase-selected checks in manifest order.
///
/// Ordinary command failures are aggregated. Candidate mutation is exceptional:
/// it blocks every remaining child, records complete blocked evidence, and still
/// performs final source verification before deriving the result.
pub fn run(candidate: &PreparedCandidate, phase: Phase) -> DeterministicResult {
    run_with_cancellation(candidate, phase, &process::Cancellation::new())
}

/// Run with cancellation owned by the top-level validation lifecycle.
///
/// Once requested, no later child starts. An active child is cancelled and fully
/// awaited by the process layer before this function returns to candidate cleanup.
pub fn run_with_cancellation(
    candidate: &PreparedCandidate,
    phase: Phase,
    cancellation: &process::Cancellation,
) -> DeterministicResult {
    let binding = CandidateBinding {
        base_revision: candidate.base_revision().to_owned(),
        candidate_revision: candidate.candidate_revision().to_owned(),
        candidate_tree: candidate.candidate_tree().to_owned(),
    };
    let expansion = Expansion::new(candidate);
    let manifest = candidate.manifest().manifest();
    let mut prerequisites = Vec::with_capacity(manifest.prerequisites().len());
    let mut checks = Vec::with_capacity(manifest.checks().len());
    let mut mutation: Option<String> = None;

    for prerequisite in manifest.prerequisites() {
        let expanded = expand_prerequisite(candidate, prerequisite, &expansion);
        let record = schedule_command(
            candidate,
            prerequisite.id(),
            CommandKind::Prerequisite,
            None,
            Some(prerequisite.install_hint()),
            prerequisite.stdout_equals(),
            expanded,
            mutation.as_deref(),
            false,
            cancellation,
        );
        retain_mutation(&record, &mut mutation);
        prerequisites.push(record);
    }

    let changed_paths: BTreeSet<String> = candidate
        .changed_paths()
        .iter()
        .filter_map(|path| path.to_str().map(str::to_owned))
        .collect();
    let changed_path_error = candidate
        .changed_paths()
        .iter()
        .find(|path| path.to_str().is_none())
        .map(|path| format!("changed path is not valid UTF-8: {}", path.display()));

    for check in manifest
        .checks()
        .iter()
        .filter(|check| check.phases().contains(&phase))
    {
        let mut expanded = expand_check(candidate, check, &expansion, &changed_paths);
        if let Some(error) = &changed_path_error
            && check.scope() == Scope::ChangedFiles
        {
            expanded.expansion_errors.push(error.clone());
        }
        let empty_changed = check.scope() == Scope::ChangedFiles && changed_paths.is_empty();
        let record = schedule_command(
            candidate,
            check.id(),
            CommandKind::Check,
            Some(check.scope().into()),
            None,
            None,
            expanded,
            mutation.as_deref(),
            empty_changed,
            cancellation,
        );
        retain_mutation(&record, &mut mutation);
        checks.push(record);
    }

    let final_verification = candidate.verify_unchanged();
    let final_source_verified = final_verification.is_ok();
    let final_failure = final_verification.err().map(|error| CommandFailure {
        kind: CommandFailureKind::CandidateMutation,
        message: format!("final candidate verification failed: {error:#}"),
    });

    DeterministicResult {
        phase: phase.into(),
        binding,
        prerequisites,
        checks,
        final_source_verified,
        final_failure,
    }
}

fn retain_mutation(record: &CommandRecord, mutation: &mut Option<String>) {
    if mutation.is_none()
        && record
            .failure
            .as_ref()
            .is_some_and(|failure| failure.kind == CommandFailureKind::CandidateMutation)
    {
        *mutation = record
            .failure
            .as_ref()
            .map(|failure| failure.message.clone());
    }
}

#[allow(clippy::too_many_arguments)]
fn schedule_command(
    candidate: &PreparedCandidate,
    id: &str,
    kind: CommandKind,
    scope: Option<CommandScope>,
    install_hint: Option<&str>,
    stdout_equals: Option<&str>,
    expanded: ExpandedCommand,
    mutation: Option<&str>,
    empty_changed: bool,
    cancellation: &process::Cancellation,
) -> CommandRecord {
    let raw_failure =
        (!expanded.expansion_errors.is_empty()).then(|| expanded.expansion_errors.join("; "));
    let mut record = CommandRecord {
        id: id.to_owned(),
        kind,
        scope,
        status: CommandStatus::Failed,
        program: expanded.program,
        args: expanded.args,
        cwd: expanded.cwd,
        environment: expanded.environment,
        program_expansion: expanded.program_expansion,
        args_expansion: expanded.args_expansion,
        cwd_expansion: expanded.cwd_expansion,
        environment_expansion: expanded.environment_expansion,
        timeout_seconds: expanded.timeout_seconds,
        max_output_bytes: expanded.max_output_bytes,
        install_hint: install_hint.map(str::to_owned),
        process: None,
        source_verified: None,
        failure: None,
    };

    if let Some(message) = mutation {
        record.status = CommandStatus::Blocked;
        record.failure = Some(CommandFailure {
            kind: CommandFailureKind::CandidateMutation,
            message: format!("blocked after candidate mutation: {message}"),
        });
        return record;
    }
    if cancellation.is_cancelled() {
        record.status = CommandStatus::Blocked;
        record.failure = Some(CommandFailure {
            kind: CommandFailureKind::Process,
            message: "blocked after validation cancellation".to_owned(),
        });
        return record;
    }
    if let Some(message) = raw_failure {
        record.failure = Some(CommandFailure {
            kind: CommandFailureKind::Configuration,
            message,
        });
        return record;
    }
    let max_output_bytes = match usize::try_from(record.max_output_bytes) {
        Ok(value) => value,
        Err(_) => {
            record.failure = Some(CommandFailure {
                kind: CommandFailureKind::Configuration,
                message: format!(
                    "max_output_bytes {} exceeds platform limit",
                    record.max_output_bytes
                ),
            });
            return record;
        }
    };
    let spec = ProcessSpec::new(
        record.program.clone(),
        record.args.clone(),
        candidate.source_root(),
        &record.cwd,
        Duration::from_secs(record.timeout_seconds),
        max_output_bytes,
    )
    .with_environment(record.environment.clone());
    if empty_changed {
        match process::preflight_cwd(&spec) {
            None => record.status = CommandStatus::SkippedSuccess,
            Some(outcome) => {
                record.failure = Some(CommandFailure {
                    kind: CommandFailureKind::Process,
                    message: format!(
                        "process preflight did not succeed: {:?}",
                        outcome.termination
                    ),
                });
                record.process = Some(outcome);
            }
        }
        return record;
    }
    let outcome = process::execute_with_cancellation(spec, cancellation);
    let launched = outcome.termination.spawn_failure_kind().is_none();
    let mut failure = if outcome.success() {
        stdout_equals.and_then(|expected| stdout_mismatch(&outcome, expected))
    } else {
        Some(CommandFailure {
            kind: CommandFailureKind::Process,
            message: format!("process did not succeed: {:?}", outcome.termination),
        })
    };
    record.process = Some(outcome);

    if launched {
        match candidate.verify_unchanged() {
            Ok(()) => record.source_verified = Some(true),
            Err(error) => {
                record.source_verified = Some(false);
                let process_detail = failure
                    .take()
                    .map(|failure| format!("; command also failed: {}", failure.message))
                    .unwrap_or_default();
                failure = Some(CommandFailure {
                    kind: CommandFailureKind::CandidateMutation,
                    message: format!(
                        "candidate verification after `{id}` failed: {error:#}{process_detail}"
                    ),
                });
            }
        }
    }

    record.status = if failure.is_none() {
        CommandStatus::Passed
    } else {
        CommandStatus::Failed
    };
    record.failure = failure;
    record
}

fn stdout_mismatch(outcome: &ProcessOutcome, expected: &str) -> Option<CommandFailure> {
    let actual = match std::str::from_utf8(outcome.stdout.exact_bytes()) {
        Ok(value) if outcome.stdout.complete() => value,
        Ok(_) => {
            return Some(CommandFailure {
                kind: CommandFailureKind::StdoutMismatch,
                message: "prerequisite stdout was incomplete".to_owned(),
            });
        }
        Err(error) => {
            return Some(CommandFailure {
                kind: CommandFailureKind::StdoutMismatch,
                message: format!("prerequisite stdout was not UTF-8: {error}"),
            });
        }
    };
    let actual = actual
        .strip_suffix('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .unwrap_or(actual);
    if actual.contains(['\r', '\n']) || actual != expected {
        return Some(CommandFailure {
            kind: CommandFailureKind::StdoutMismatch,
            message: format!("prerequisite stdout mismatch: expected `{expected}`, got `{actual}`"),
        });
    }
    None
}

fn expand_prerequisite(
    candidate: &PreparedCandidate,
    prerequisite: &Prerequisite,
    expansion: &Expansion<'_>,
) -> ExpandedCommand {
    let program_expansion = expand_value(expansion, prerequisite.program());
    let args_expansion = prerequisite
        .args()
        .iter()
        .map(|arg| expand_value(expansion, arg))
        .collect::<Vec<_>>();
    let cwd_expansion = expand_value(expansion, "{candidate_root}");
    let (environment, environment_expansion) = expand_environment(
        candidate.manifest().manifest().defaults().environment(),
        None,
        expansion,
    );
    expanded_command(
        program_expansion,
        args_expansion,
        cwd_expansion,
        environment,
        environment_expansion,
        candidate.manifest().manifest().defaults().timeout_seconds(),
        candidate
            .manifest()
            .manifest()
            .defaults()
            .max_output_bytes(),
        &[],
    )
}

fn expand_check(
    candidate: &PreparedCandidate,
    check: &Check,
    expansion: &Expansion<'_>,
    changed_paths: &BTreeSet<String>,
) -> ExpandedCommand {
    let program_expansion = expand_value(expansion, check.program());
    let mut args_expansion = check
        .args()
        .iter()
        .map(|arg| expand_value(expansion, arg))
        .collect::<Vec<_>>();
    if check.scope() == Scope::ChangedFiles {
        args_expansion.extend(changed_paths.iter().map(|path| ValueExpansion {
            declared: path.clone(),
            expanded: Some(path.clone()),
            error: None,
        }));
    }
    let cwd_expansion = expand_value(expansion, check.cwd());
    let (environment, environment_expansion) = expand_environment(
        candidate.manifest().manifest().defaults().environment(),
        Some(check.environment()),
        expansion,
    );
    expanded_command(
        program_expansion,
        args_expansion,
        cwd_expansion,
        environment,
        environment_expansion,
        check.timeout_seconds(),
        check.max_output_bytes(),
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn expanded_command(
    program_expansion: ValueExpansion,
    args_expansion: Vec<ValueExpansion>,
    cwd_expansion: ValueExpansion,
    environment: EnvironmentChanges,
    environment_expansion: EnvironmentExpansion,
    timeout_seconds: u64,
    max_output_bytes: u64,
    additional_errors: &[String],
) -> ExpandedCommand {
    let mut expansion_errors = Vec::new();
    collect_expansion_error("program", &program_expansion, &mut expansion_errors);
    for (index, value) in args_expansion.iter().enumerate() {
        collect_expansion_error(&format!("args[{index}]"), value, &mut expansion_errors);
    }
    collect_expansion_error("cwd", &cwd_expansion, &mut expansion_errors);
    for (name, value) in &environment_expansion.set {
        collect_expansion_error(
            &format!("environment.set.{name}"),
            value,
            &mut expansion_errors,
        );
    }
    expansion_errors.extend_from_slice(additional_errors);

    let program = best_value(&program_expansion).to_owned();
    let args = args_expansion
        .iter()
        .map(|value| best_value(value).to_owned())
        .collect();
    let cwd = PathBuf::from(best_value(&cwd_expansion));
    ExpandedCommand {
        program,
        args,
        cwd,
        environment,
        program_expansion,
        args_expansion,
        cwd_expansion,
        environment_expansion,
        expansion_errors,
        timeout_seconds,
        max_output_bytes,
    }
}

fn expand_value(expansion: &Expansion<'_>, declared: &str) -> ValueExpansion {
    match expansion.value(declared) {
        Ok(expanded) => ValueExpansion {
            declared: declared.to_owned(),
            expanded: Some(expanded),
            error: None,
        },
        Err(error) => ValueExpansion {
            declared: declared.to_owned(),
            expanded: None,
            error: Some(error),
        },
    }
}

fn collect_expansion_error(context: &str, value: &ValueExpansion, errors: &mut Vec<String>) {
    if let Some(error) = &value.error {
        errors.push(format!("failed to expand {context}: {error}"));
    }
}

fn best_value(value: &ValueExpansion) -> &str {
    value.expanded.as_deref().unwrap_or(&value.declared)
}

fn expand_environment(
    defaults: &Environment,
    override_environment: Option<&Environment>,
    expansion: &Expansion<'_>,
) -> (EnvironmentChanges, EnvironmentExpansion) {
    let mut declared_set = defaults.set().clone();
    let mut unset: BTreeSet<String> = defaults.unset().iter().cloned().collect();
    if let Some(environment) = override_environment {
        declared_set.extend(environment.set().clone());
        unset.extend(environment.unset().iter().cloned());
    }
    // Candidate commands must never inherit caller-selected index state.
    unset.insert("GIT_INDEX_FILE".to_owned());
    unset.insert(PRIVATE_STAGED_INDEX_ENVIRONMENT.to_owned());
    for name in &unset {
        declared_set.remove(name);
    }
    let expanded_set = declared_set
        .into_iter()
        .map(|(name, value)| (name, expand_value(expansion, &value)))
        .collect::<BTreeMap<_, _>>();
    let best_set = expanded_set
        .iter()
        .map(|(name, value)| (name.clone(), best_value(value).to_owned()))
        .collect();
    (
        EnvironmentChanges::new(best_set, unset.clone()),
        EnvironmentExpansion {
            set: expanded_set,
            unset,
        },
    )
}

fn utf8_path<'a>(path: &'a Path, description: &str) -> Result<&'a str, String> {
    path.to_str()
        .ok_or_else(|| format!("{description} is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(declared: &str, expanded: Option<&str>, error: Option<&str>) -> ValueExpansion {
        ValueExpansion {
            declared: declared.to_owned(),
            expanded: expanded.map(str::to_owned),
            error: error.map(str::to_owned),
        }
    }

    #[test]
    fn failed_expansion_preserves_declared_and_successful_value_evidence() {
        let command = expanded_command(
            value("{bad}/tool", None, Some("bad path")),
            vec![value("literal", Some("literal"), None)],
            value("{candidate_root}", Some("/candidate"), None),
            EnvironmentChanges::default(),
            EnvironmentExpansion {
                set: BTreeMap::new(),
                unset: BTreeSet::new(),
            },
            17,
            4096,
            &[],
        );

        assert_eq!(command.program, "{bad}/tool");
        assert_eq!(command.args, ["literal"]);
        assert_eq!(command.cwd, Path::new("/candidate"));
        assert_eq!(command.timeout_seconds, 17);
        assert_eq!(command.max_output_bytes, 4096);
        assert_eq!(command.program_expansion.declared, "{bad}/tool");
        assert!(command.program_expansion.expanded.is_none());
        assert_eq!(
            command.args_expansion[0].expanded.as_deref(),
            Some("literal")
        );
        assert_eq!(
            command.cwd_expansion.expanded.as_deref(),
            Some("/candidate")
        );
        assert_eq!(
            command.expansion_errors,
            ["failed to expand program: bad path"]
        );
    }
}
