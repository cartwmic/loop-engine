//! Repository-state checkpoints for software-change reports.
//!
//! Checkpoints are provider-owned evidence, not Git lifecycle operations.  The
//! builder runs read-only Git commands and reads the report/documents before
//! writing exactly one checkpoint file.  Verification rebuilds the same
//! object from the source bytes and current repository state; stored digests
//! are never treated as authority.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const SCHEMA_VERSION: &str = "1";
const IMPLEMENTATION_REPORT: &str = "implementation-report.json";
const VALIDATION_REPORT: &str = "validation-report.json";
const IMPLEMENTATION_PROOF_HISTORY: &str = "implementation-proof-history";
const CHECKPOINT_FIELDS: &[&str] = &[
    "schema_version",
    "phase",
    "report",
    "documents",
    "repository",
];
const REPORT_FIELDS: &[&str] = &["file", "revision", "sha256"];
const DOCUMENT_FIELDS: &[&str] = &["intent_revision", "design_revision", "plan_revision"];
const REPOSITORY_FIELDS: &[&str] = &[
    "head",
    "index_sha256",
    "status_sha256",
    "entries",
    "state_sha256",
];
const ENTRY_FIELDS: &[&str] = &["path", "tracked", "kind", "mode", "content_sha256"];
const ENTRY_KINDS: &[&str] = &["regular", "symlink", "submodule", "missing"];

/// The phase whose report and checkpoint are being bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckpointPhase {
    Implementation,
    Validation,
}

impl CheckpointPhase {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "implementation" => Ok(Self::Implementation),
            "validation" => Ok(Self::Validation),
            _ => Err(format!(
                "`--phase` must be `implementation` or `validation`, got `{value}`"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::Validation => "validation",
        }
    }

    fn report_file(self) -> &'static str {
        match self {
            Self::Implementation => IMPLEMENTATION_REPORT,
            Self::Validation => VALIDATION_REPORT,
        }
    }

    fn checkpoint_file(self) -> &'static str {
        match self {
            Self::Implementation => "implementation-checkpoint.json",
            Self::Validation => "validation-checkpoint.json",
        }
    }
}

/// Closed checkpoint object written under an artifact root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Checkpoint {
    schema_version: String,
    phase: String,
    report: ReportIdentity,
    documents: DocumentRevisions,
    repository: RepositoryIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportIdentity {
    file: String,
    revision: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct DocumentRevisions {
    intent_revision: String,
    design_revision: String,
    plan_revision: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RepositoryIdentity {
    head: String,
    index_sha256: String,
    status_sha256: String,
    entries: Vec<RepositoryEntry>,
    state_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RepositoryWithoutState {
    head: String,
    index_sha256: String,
    status_sha256: String,
    entries: Vec<RepositoryEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RepositoryEntry {
    path: String,
    tracked: bool,
    kind: String,
    mode: String,
    content_sha256: Option<String>,
}

#[derive(Clone, Debug)]
struct RepositorySnapshot {
    identity: RepositoryIdentity,
    status_changes: Vec<StatusChange>,
}

#[derive(Clone, Debug)]
struct StatusChange {
    class: &'static str,
    path: String,
}

#[derive(Clone, Debug)]
struct BuiltCheckpoint {
    checkpoint: Checkpoint,
    status_changes: Vec<StatusChange>,
}

/// Build and write one phase checkpoint.  All reads and validation happen
/// before the checkpoint path is opened.
pub(crate) fn create(
    phase: CheckpointPhase,
    artifact_root: &Path,
    working_directory: &Path,
) -> Result<Checkpoint, String> {
    let artifact_root = existing_absolute_directory(artifact_root, "artifact_root")?;
    let built = build(phase, &artifact_root, working_directory)?;
    let path = artifact_root.join(phase.checkpoint_file());
    refuse_checkpoint_symlink(&path)?;
    let bytes = serialize_checkpoint(&built.checkpoint, &path.display().to_string())?;
    fs::write(&path, bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(built.checkpoint)
}

/// Verify a phase checkpoint against the current process working directory.
/// The caller is the provider subprocess, which inherits the driver's
/// selected repository directory.
pub(crate) fn verify_from_cwd(
    phase: CheckpointPhase,
    artifact_root: &Path,
) -> Result<Checkpoint, String> {
    let working_directory = std::env::current_dir()
        .map_err(|error| format!("could not read provider working directory: {error}"))?;
    verify(phase, artifact_root, &working_directory)
}

/// Verify a phase checkpoint against an explicit repository directory.
pub(crate) fn verify(
    phase: CheckpointPhase,
    artifact_root: &Path,
    working_directory: &Path,
) -> Result<Checkpoint, String> {
    let artifact_root = existing_absolute_directory(artifact_root, "artifact_root")?;
    let checkpoint_path = artifact_root.join(phase.checkpoint_file());
    refuse_checkpoint_symlink(&checkpoint_path)?;
    let stored_bytes = fs::read(&checkpoint_path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!(
                "{} is missing; report-only completion is not accepted",
                checkpoint_path.display()
            )
        } else {
            format!("could not read {}: {error}", checkpoint_path.display())
        }
    })?;
    let stored_value: Value = serde_json::from_slice(&stored_bytes)
        .map_err(|error| format!("{} is not valid JSON: {error}", checkpoint_path.display()))?;
    validate_closed_value(&stored_value, phase, &checkpoint_path)?;
    let stored: Checkpoint = serde_json::from_value(stored_value).map_err(|error| {
        format!(
            "{} does not match the closed checkpoint schema: {error}",
            checkpoint_path.display()
        )
    })?;
    validate_checkpoint_values(&stored, phase, &checkpoint_path)?;

    let built = build(phase, &artifact_root, working_directory)?;
    let verified = compare(&stored, &built.checkpoint, &built.status_changes)?;
    if phase == CheckpointPhase::Validation {
        let implementation = verify(
            CheckpointPhase::Implementation,
            &artifact_root,
            working_directory,
        )?;
        if state_identity(&verified) != state_identity(&implementation) {
            return Err(
                "checkpoint mismatch: validation repository state differs from the latest implementation-reviewed state"
                    .to_owned(),
            );
        }
    }
    Ok(verified)
}

/// Return the state identity after verification.  This is used by the
/// finding ledger so a stored `repository_state` cannot stand in for current
/// evidence.
pub(crate) fn current_state_from_cwd(
    phase: CheckpointPhase,
    artifact_root: &Path,
) -> Result<String, String> {
    let checkpoint = verify_from_cwd(phase, artifact_root)?;
    Ok(checkpoint.repository.state_sha256)
}

/// Preserve the exact implementation checkpoint admitted to validation.
///
/// The history is content-addressed and append-only through supported provider
/// operations. Reusing one report revision for different proof creates two
/// entries and therefore fails closed at validation rather than moving an
/// authority pointer. This applies both to reviewless `implementation-ready`
/// and to the terminal implementation-review approval.
pub(crate) fn record_accepted_implementation_from_cwd(artifact_root: &Path) -> Result<(), String> {
    let artifact_root = existing_absolute_directory(artifact_root, "artifact_root")?;
    let accepted = verify_from_cwd(CheckpointPhase::Implementation, &artifact_root)?;
    let bytes = serialize_checkpoint(&accepted, "accepted implementation proof")?;
    let digest = sha256_digest(&bytes);
    let history = implementation_proof_history(&artifact_root, true)?;
    let path = history.join(format!("{}.json", digest.trim_start_matches("sha256:")));
    refuse_checkpoint_symlink(&path)?;

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file
            .write_all(&bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read(&path)
                .map_err(|read_error| format!("could not read {}: {read_error}", path.display()))?;
            if existing == bytes {
                Ok(())
            } else {
                Err(format!(
                    "content-addressed implementation proof collision at {}",
                    path.display()
                ))
            }
        }
        Err(error) => Err(format!("could not create {}: {error}", path.display())),
    }
}

/// Compare current implementation proof with the sole immutable history entry
/// for its report revision.
pub(crate) fn verify_accepted_implementation_from_cwd(artifact_root: &Path) -> Result<(), String> {
    let artifact_root = existing_absolute_directory(artifact_root, "artifact_root")?;
    let current = verify_from_cwd(CheckpointPhase::Implementation, &artifact_root)?;
    let history = implementation_proof_history(&artifact_root, false)?;
    let mut matching = Vec::new();
    for entry in fs::read_dir(&history)
        .map_err(|error| format!("could not read {}: {error}", history.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("could not read entry under {}: {error}", history.display())
        })?;
        let path = entry.path();
        refuse_checkpoint_symlink(&path)?;
        if !entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
            .is_file()
            || path.extension() != Some(OsStr::new("json"))
        {
            return Err(format!(
                "implementation proof history contains unsupported entry {}",
                path.display()
            ));
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let expected_name = format!(
            "{}.json",
            sha256_digest(&bytes).trim_start_matches("sha256:")
        );
        if path.file_name() != Some(OsStr::new(&expected_name)) {
            return Err(format!(
                "implementation proof history entry {} does not match its content digest",
                path.display()
            ));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
        validate_closed_value(&value, CheckpointPhase::Implementation, &path)?;
        let checkpoint: Checkpoint = serde_json::from_value(value).map_err(|error| {
            format!(
                "{} does not match the closed checkpoint schema: {error}",
                path.display()
            )
        })?;
        validate_checkpoint_values(&checkpoint, CheckpointPhase::Implementation, &path)?;
        let canonical = serialize_checkpoint(&checkpoint, &path.display().to_string())?;
        if bytes != canonical {
            return Err(format!(
                "{} is not the exact compact serialization of its checkpoint",
                path.display()
            ));
        }
        if checkpoint.report.revision == current.report.revision {
            matching.push(checkpoint);
        }
    }

    select_accepted_implementation(&current, &matching)
}

fn select_accepted_implementation(
    current: &Checkpoint,
    matching: &[Checkpoint],
) -> Result<(), String> {
    match matching {
        [] => Err(format!(
            "implementation proof history has no accepted checkpoint for report revision `{}`",
            current.report.revision
        )),
        [accepted] if accepted == current => Ok(()),
        [accepted] if accepted.repository.state_sha256 != current.repository.state_sha256 => {
            Err(format!(
                "checkpoint mismatch: current repository state `{}` differs from implementation-reviewed repository state `{}`",
                current.repository.state_sha256, accepted.repository.state_sha256
            ))
        }
        [_] => Err(
            "checkpoint mismatch: current implementation report or document revisions differ from accepted implementation proof"
                .to_owned(),
        ),
        _ => Err(format!(
            "implementation proof history is ambiguous for report revision `{}`; bump the implementation report revision after material changes",
            current.report.revision
        )),
    }
}

fn implementation_proof_history(artifact_root: &Path, create: bool) -> Result<PathBuf, String> {
    let path = artifact_root.join(IMPLEMENTATION_PROOF_HISTORY);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("{} must not be a symlink", path.display()))
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(format!("{} must be a directory", path.display()))
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => fs::create_dir(&path)
            .map_err(|create_error| {
                format!("could not create {}: {create_error}", path.display())
            })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "{} is missing; validation requires accepted implementation proof",
                path.display()
            ))
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    Ok(path)
}

/// Map a report subject to its checkpoint phase.
pub(crate) fn phase_for_subject(subject: &str) -> Option<CheckpointPhase> {
    match subject {
        IMPLEMENTATION_REPORT => Some(CheckpointPhase::Implementation),
        VALIDATION_REPORT => Some(CheckpointPhase::Validation),
        _ => None,
    }
}

/// The state identity is public to the provider module through this small
/// accessor; the checkpoint remains otherwise opaque to gates.
pub(crate) fn state_identity(checkpoint: &Checkpoint) -> &str {
    &checkpoint.repository.state_sha256
}

fn build(
    phase: CheckpointPhase,
    artifact_root: &Path,
    working_directory: &Path,
) -> Result<BuiltCheckpoint, String> {
    let artifact_root = existing_absolute_directory(artifact_root, "artifact_root")?;
    let report_path = contained_artifact_path(&artifact_root, phase.report_file())?;
    let report_bytes = fs::read(&report_path).map_err(|error| {
        format!(
            "could not read {} for checkpoint: {error}",
            report_path.display()
        )
    })?;
    let report = parse_json_object(&report_bytes, phase.report_file())?;
    let report_revision = required_revision(&report, phase.report_file())?;

    let intent = read_document(&artifact_root, "intent.json")?;
    let design = read_document(&artifact_root, "design.json")?;
    let plan = read_document(&artifact_root, "plan.json")?;
    let documents = DocumentRevisions {
        intent_revision: required_revision(&intent.value, "intent.json")?,
        design_revision: required_revision(&design.value, "design.json")?,
        plan_revision: required_revision(&plan.value, "plan.json")?,
    };
    let repository = collect_repository(working_directory)?;
    let report_identity = ReportIdentity {
        file: phase.report_file().to_owned(),
        revision: report_revision,
        sha256: sha256_digest(&report_bytes),
    };
    let state_without = RepositoryWithoutState {
        head: repository.identity.head.clone(),
        index_sha256: repository.identity.index_sha256.clone(),
        status_sha256: repository.identity.status_sha256.clone(),
        entries: repository.identity.entries.clone(),
    };
    let state_bytes = serde_json::to_vec(&state_without)
        .map_err(|error| format!("could not serialize checkpoint state: {error}"))?;
    let state_sha256 = sha256_digest(&state_bytes);
    let checkpoint = Checkpoint {
        schema_version: SCHEMA_VERSION.to_owned(),
        phase: phase.as_str().to_owned(),
        report: report_identity,
        documents,
        repository: RepositoryIdentity {
            state_sha256,
            ..repository.identity
        },
    };
    Ok(BuiltCheckpoint {
        checkpoint,
        status_changes: repository.status_changes,
    })
}

fn compare(
    stored: &Checkpoint,
    expected: &Checkpoint,
    status_changes: &[StatusChange],
) -> Result<Checkpoint, String> {
    let mut mismatches = Vec::new();
    if stored.schema_version != expected.schema_version {
        mismatches.push("checkpoint schema_version changed".to_owned());
    }
    if stored.phase != expected.phase {
        mismatches.push("checkpoint phase changed".to_owned());
    }
    if stored.report.file != expected.report.file {
        mismatches.push(format!(
            "report file changed: expected `{}`, got `{}`",
            expected.report.file, stored.report.file
        ));
    }
    if stored.report.revision != expected.report.revision {
        mismatches.push(format!(
            "report revision changed for `{}`",
            expected.report.file
        ));
    }
    if stored.report.sha256 != expected.report.sha256 {
        mismatches.push(format!(
            "report bytes changed at `{}`",
            expected.report.file
        ));
    }
    if stored.documents != expected.documents {
        for (name, actual, current) in [
            (
                "intent.json",
                &stored.documents.intent_revision,
                &expected.documents.intent_revision,
            ),
            (
                "design.json",
                &stored.documents.design_revision,
                &expected.documents.design_revision,
            ),
            (
                "plan.json",
                &stored.documents.plan_revision,
                &expected.documents.plan_revision,
            ),
        ] {
            if actual != current {
                mismatches.push(format!("document revision changed at `{name}`"));
            }
        }
    }

    let stored_repository = &stored.repository;
    let expected_repository = &expected.repository;
    if stored_repository.head != expected_repository.head {
        mismatches.push("repository HEAD changed at `HEAD`".to_owned());
    }
    if stored_repository.index_sha256 != expected_repository.index_sha256 {
        mismatches.push("repository index changed at `index`".to_owned());
    }
    if stored_repository.status_sha256 != expected_repository.status_sha256 {
        if status_changes.is_empty() {
            mismatches.push("repository status changed at `status`".to_owned());
        } else {
            for change in status_changes {
                mismatches.push(format!(
                    "repository {} changed at `{}`",
                    change.class, change.path
                ));
            }
        }
    }
    compare_entries(
        &stored_repository.entries,
        &expected_repository.entries,
        &mut mismatches,
    );
    if stored_repository.state_sha256 != expected_repository.state_sha256 {
        mismatches.push("repository state digest changed at `state_sha256`".to_owned());
    }
    if mismatches.is_empty() {
        Ok(stored.clone())
    } else {
        Err(format!("checkpoint mismatch: {}", mismatches.join("; ")))
    }
}

fn compare_entries(
    stored: &[RepositoryEntry],
    expected: &[RepositoryEntry],
    mismatches: &mut Vec<String>,
) {
    let stored_by_path = stored
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let expected_by_path = expected
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();

    for path in stored_by_path.keys() {
        if !expected_by_path.contains_key(path) {
            mismatches.push(format!("repository entry deleted at `{path}`"));
        }
    }
    for path in expected_by_path.keys() {
        if !stored_by_path.contains_key(path) {
            mismatches.push(format!("repository entry added at `{path}`"));
        }
    }
    for (path, old) in stored_by_path {
        let Some(current) = expected_by_path.get(path) else {
            continue;
        };
        if old.tracked != current.tracked {
            mismatches.push(format!("repository tracking changed at `{path}`"));
        }
        if old.kind != current.kind {
            mismatches.push(format!("repository file type changed at `{path}`"));
        }
        if old.mode != current.mode {
            mismatches.push(format!("repository mode changed at `{path}`"));
        }
        if old.content_sha256 != current.content_sha256 {
            mismatches.push(format!("repository bytes changed at `{path}`"));
        }
    }
}

fn validate_closed_value(value: &Value, phase: CheckpointPhase, path: &Path) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{} must be a JSON object", path.display()))?;
    require_exact_fields(object, CHECKPOINT_FIELDS, "checkpoint")?;
    let report = object
        .get("report")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} report must be an object", path.display()))?;
    require_exact_fields(report, REPORT_FIELDS, "checkpoint.report")?;
    let documents = object
        .get("documents")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} documents must be an object", path.display()))?;
    require_exact_fields(documents, DOCUMENT_FIELDS, "checkpoint.documents")?;
    let repository = object
        .get("repository")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} repository must be an object", path.display()))?;
    require_exact_fields(repository, REPOSITORY_FIELDS, "checkpoint.repository")?;
    let entries = repository
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} repository.entries must be an array", path.display()))?;
    for (index, entry) in entries.iter().enumerate() {
        let entry = entry.as_object().ok_or_else(|| {
            format!(
                "{} repository.entries[{index}] must be an object",
                path.display()
            )
        })?;
        require_exact_fields(entry, ENTRY_FIELDS, &format!("repository.entries[{index}]"))?;
    }
    let actual_phase = object
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actual_phase != phase.as_str() {
        return Err(format!(
            "{} phase must be `{}`",
            path.display(),
            phase.as_str()
        ));
    }
    Ok(())
}

fn require_exact_fields(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
    label: &str,
) -> Result<(), String> {
    for field in fields {
        if !object.contains_key(*field) {
            return Err(format!("{label} is missing `{field}`"));
        }
    }
    for key in object.keys() {
        if !fields.contains(&key.as_str()) {
            return Err(format!("{label} has unknown field `{key}`"));
        }
    }
    Ok(())
}

fn validate_checkpoint_values(
    checkpoint: &Checkpoint,
    phase: CheckpointPhase,
    path: &Path,
) -> Result<(), String> {
    if checkpoint.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "{} schema_version must be the constant string \"1\"",
            path.display()
        ));
    }
    if checkpoint.phase != phase.as_str() {
        return Err(format!(
            "{} phase must be `{}`",
            path.display(),
            phase.as_str()
        ));
    }
    if checkpoint.report.file != phase.report_file() {
        return Err(format!(
            "{} report.file must be `{}`",
            path.display(),
            phase.report_file()
        ));
    }
    for (label, value) in [
        ("report.revision", checkpoint.report.revision.as_str()),
        (
            "documents.intent_revision",
            checkpoint.documents.intent_revision.as_str(),
        ),
        (
            "documents.design_revision",
            checkpoint.documents.design_revision.as_str(),
        ),
        (
            "documents.plan_revision",
            checkpoint.documents.plan_revision.as_str(),
        ),
    ] {
        if value.is_empty() {
            return Err(format!("{} {label} must be non-empty", path.display()));
        }
    }
    for (label, value) in [
        ("report.sha256", checkpoint.report.sha256.as_str()),
        (
            "repository.index_sha256",
            checkpoint.repository.index_sha256.as_str(),
        ),
        (
            "repository.status_sha256",
            checkpoint.repository.status_sha256.as_str(),
        ),
        (
            "repository.state_sha256",
            checkpoint.repository.state_sha256.as_str(),
        ),
    ] {
        if !is_digest(value) {
            return Err(format!(
                "{} {label} must be sha256:<64 lowercase hex>",
                path.display()
            ));
        }
    }
    if !is_hex_oid(&checkpoint.repository.head) {
        return Err(format!(
            "{} repository.head must be lowercase hexadecimal",
            path.display()
        ));
    }

    let mut previous: Option<&str> = None;
    let mut paths = BTreeSet::new();
    for (index, entry) in checkpoint.repository.entries.iter().enumerate() {
        validate_repo_path(&entry.path).map_err(|error| {
            format!(
                "{} repository.entries[{index}].path: {error}",
                path.display()
            )
        })?;
        if !paths.insert(entry.path.clone()) {
            return Err(format!(
                "{} repository.entries contains duplicate path `{}`",
                path.display(),
                entry.path
            ));
        }
        if let Some(previous) = previous {
            if previous.as_bytes() >= entry.path.as_bytes() {
                return Err(format!(
                    "{} repository.entries are not sorted by UTF-8 path bytes",
                    path.display()
                ));
            }
        }
        previous = Some(entry.path.as_str());
        if !ENTRY_KINDS.contains(&entry.kind.as_str()) {
            return Err(format!(
                "{} repository.entries[{index}].kind is unsupported",
                path.display()
            ));
        }
        if !is_git_mode(&entry.mode) {
            return Err(format!(
                "{} repository.entries[{index}].mode must be six octal digits",
                path.display()
            ));
        }
        match (&entry.kind[..], entry.content_sha256.as_deref()) {
            ("missing", None) => {}
            ("missing", Some(_)) => {
                return Err(format!(
                    "{} repository.entries[{index}].content_sha256 must be null for missing",
                    path.display()
                ))
            }
            (_, None) => {
                return Err(format!(
                    "{} repository.entries[{index}].content_sha256 must not be null",
                    path.display()
                ))
            }
            (_, Some(value)) if !is_digest(value) => {
                return Err(format!(
                    "{} repository.entries[{index}].content_sha256 must be a sha256 digest",
                    path.display()
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

fn existing_absolute_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute directory"));
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("{label} must be an existing directory: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("{label} must be an existing directory"));
    }
    fs::canonicalize(path).map_err(|error| format!("could not resolve {label}: {error}"))
}

fn contained_artifact_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    let canonical = fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve artifact `{name}`: {error}"))?;
    if canonical == root || !canonical.starts_with(root) {
        return Err(format!("artifact `{name}` escapes artifact_root"));
    }
    Ok(canonical)
}

fn refuse_checkpoint_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("checkpoint path {} is a symlink", path.display()))
        }
        Ok(metadata) if metadata.is_dir() => {
            Err(format!("checkpoint path {} is a directory", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "could not inspect checkpoint path {}: {error}",
            path.display()
        )),
    }
}

struct Document {
    value: Value,
}

fn read_document(root: &Path, name: &str) -> Result<Document, String> {
    let path = contained_artifact_path(root, name)?;
    let bytes = fs::read(&path).map_err(|error| format!("could not read {name}: {error}"))?;
    Ok(Document {
        value: parse_json_object(&bytes, name)?,
    })
}

fn parse_json_object(bytes: &[u8], name: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("{name} is not valid JSON: {error}"))?;
    if !value.is_object() {
        return Err(format!("{name} must be a JSON object"));
    }
    Ok(value)
}

fn required_revision(value: &Value, name: &str) -> Result<String, String> {
    match value.get("revision").and_then(Value::as_str) {
        Some(revision) if !revision.is_empty() => Ok(revision.to_owned()),
        Some(_) => Err(format!("{name}.revision must be non-empty")),
        None => Err(format!("{name}.revision must be a non-empty string")),
    }
}

fn collect_repository(working_directory: &Path) -> Result<RepositorySnapshot, String> {
    let working_directory = existing_absolute_directory(working_directory, "working-directory")?;
    let top_level_raw = git_output(&working_directory, &["rev-parse", "--show-toplevel"])?;
    let top_level = parse_line(&top_level_raw, "git rev-parse --show-toplevel")?;
    let repository_root = existing_absolute_directory(Path::new(&top_level), "Git top-level")?;

    let head_raw = git_output(&repository_root, &["rev-parse", "HEAD"])?;
    let head = parse_line(&head_raw, "git rev-parse HEAD")?;
    if !is_hex_oid(&head) {
        return Err("git rev-parse HEAD did not return lowercase hexadecimal".to_owned());
    }

    let index_raw = git_output(&repository_root, &["ls-files", "--stage", "-z"])?;
    let status_raw = git_output(
        &repository_root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignored=no",
        ],
    )?;
    let cached_raw = git_output(&repository_root, &["ls-files", "-z", "--cached"])?;
    let others_raw = git_output(
        &repository_root,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )?;

    let index_text = utf8(&index_raw, "git ls-files --stage -z")?;
    let status_text = utf8(&status_raw, "git status --porcelain=v2 -z")?;
    let cached_text = utf8(&cached_raw, "git ls-files -z --cached")?;
    let others_text = utf8(&others_raw, "git ls-files -z --others --exclude-standard")?;

    let index_modes = parse_index(&index_text)?;
    let cached = parse_path_list(&cached_text, "git ls-files -z --cached")?;
    let others = parse_path_list(&others_text, "git ls-files -z --others --exclude-standard")?;
    let status_changes = parse_status(&status_text)?;

    let mut tracked = BTreeSet::new();
    for path in cached {
        if !tracked.insert(path.clone()) {
            return Err(format!("duplicate tracked repository path `{path}`"));
        }
        if !index_modes.contains_key(&path) {
            return Err(format!("tracked path `{path}` has no stage-0 index entry"));
        }
    }
    let mut untracked = BTreeSet::new();
    for path in others {
        if !untracked.insert(path.clone()) {
            return Err(format!("duplicate untracked repository path `{path}`"));
        }
        if tracked.contains(&path) {
            return Err(format!(
                "repository path `{path}` is both tracked and untracked"
            ));
        }
    }
    for path in index_modes.keys() {
        if !tracked.contains(path) {
            return Err(format!("index path `{path}` is absent from cached paths"));
        }
    }

    let mut entries = Vec::with_capacity(tracked.len() + untracked.len());
    for path in tracked {
        let mode = index_modes
            .get(&path)
            .expect("tracked path has an index mode")
            .clone();
        entries.push(build_entry(&repository_root, &path, true, mode)?);
    }
    for path in untracked {
        entries.push(build_entry(&repository_root, &path, false, String::new())?);
    }
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));

    Ok(RepositorySnapshot {
        identity: RepositoryIdentity {
            head,
            index_sha256: sha256_digest(&index_raw),
            status_sha256: sha256_digest(&status_raw),
            entries,
            state_sha256: String::new(),
        },
        status_changes,
    })
}

fn build_entry(
    repository_root: &Path,
    path: &str,
    tracked: bool,
    tracked_mode: String,
) -> Result<RepositoryEntry, String> {
    let full_path = repository_root.join(path);
    let metadata = match fs::symlink_metadata(&full_path) {
        Ok(metadata) => Some(metadata),
        Err(error)
            if tracked
                && matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
        {
            None
        }
        Err(error) => {
            return Err(format!(
                "could not inspect repository path `{path}`: {error}"
            ))
        }
    };

    let (kind, mode, content_sha256) = match metadata {
        None => ("missing".to_owned(), tracked_mode, None),
        Some(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(&full_path)
                .map_err(|error| format!("could not read symlink `{path}`: {error}"))?;
            (
                "symlink".to_owned(),
                if tracked {
                    tracked_mode
                } else {
                    "120000".to_owned()
                },
                Some(sha256_digest(&os_str_bytes(target.as_os_str())?)),
            )
        }
        Some(metadata) if metadata.is_file() => {
            let bytes = fs::read(&full_path)
                .map_err(|error| format!("could not read repository file `{path}`: {error}"))?;
            (
                "regular".to_owned(),
                if tracked {
                    tracked_mode
                } else {
                    regular_mode(&metadata)
                },
                Some(sha256_digest(&bytes)),
            )
        }
        Some(metadata) if metadata.is_dir() => {
            let oid = submodule_head(&full_path, path)?;
            (
                "submodule".to_owned(),
                if tracked {
                    tracked_mode
                } else {
                    "160000".to_owned()
                },
                Some(sha256_digest(&oid)),
            )
        }
        Some(_) => return Err(format!("unsupported repository item type at `{path}`")),
    };

    if mode.is_empty() {
        return Err(format!("missing tracked Git mode for `{path}`"));
    }
    if !is_git_mode(&mode) {
        return Err(format!("unsupported Git mode `{mode}` at `{path}`"));
    }
    Ok(RepositoryEntry {
        path: path.to_owned(),
        tracked,
        kind,
        mode,
        content_sha256,
    })
}

fn submodule_head(path: &Path, display_path: &str) -> Result<Vec<u8>, String> {
    let top = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("could not inspect submodule `{display_path}`: {error}"))?;
    if !top.status.success() {
        return Err(format!(
            "unsupported directory repository item at `{display_path}`"
        ));
    }
    let top = parse_line(&top.stdout, "submodule git top-level")?;
    let top = fs::canonicalize(&top).map_err(|error| {
        format!("could not resolve submodule `{display_path}` top-level: {error}")
    })?;
    let path = fs::canonicalize(path)
        .map_err(|error| format!("could not resolve submodule `{display_path}`: {error}"))?;
    if top != path {
        return Err(format!(
            "unsupported directory repository item at `{display_path}`"
        ));
    }
    let head = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("could not read submodule `{display_path}` HEAD: {error}"))?;
    if !head.status.success() {
        return Err(format!("could not read submodule `{display_path}` HEAD"));
    }
    let oid = parse_line(&head.stdout, "submodule git rev-parse HEAD")?;
    if !is_hex_oid(&oid) {
        return Err(format!(
            "submodule `{display_path}` HEAD is not lowercase hexadecimal"
        ));
    }
    Ok(oid.into_bytes())
}

fn parse_index(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut modes = BTreeMap::new();
    for (index, record) in text.split('\0').enumerate() {
        if record.is_empty() {
            if index + 1 == text.split('\0').count() {
                continue;
            }
            return Err("git index contains an empty path".to_owned());
        }
        let (metadata, path) = record
            .split_once('\t')
            .ok_or_else(|| "git index record has no path separator".to_owned())?;
        validate_repo_path(path)?;
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(format!("malformed git index record for `{path}`"));
        }
        if fields[2] != "0" {
            return Err(format!(
                "unmerged index entry at `{path}` has stage `{}`",
                fields[2]
            ));
        }
        let mode = fields[0].to_owned();
        if !is_git_mode(&mode) {
            return Err(format!("unsupported Git mode `{mode}` at `{path}`"));
        }
        if modes.insert(path.to_owned(), mode).is_some() {
            return Err(format!("duplicate stage-0 index path `{path}`"));
        }
    }
    Ok(modes)
}

fn parse_path_list(text: &str, command: &str) -> Result<Vec<String>, String> {
    let records = text.split('\0').collect::<Vec<_>>();
    let mut paths = Vec::new();
    for (index, path) in records.into_iter().enumerate() {
        if path.is_empty() {
            if index + 1 == text.split('\0').count() {
                continue;
            }
            return Err(format!("{command} returned an empty path"));
        }
        validate_repo_path(path)?;
        paths.push(path.to_owned());
    }
    Ok(paths)
}

fn parse_status(text: &str) -> Result<Vec<StatusChange>, String> {
    let records = text.split('\0').collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        if record.starts_with('#') {
            continue;
        }
        if let Some(path) = record.strip_prefix("? ") {
            validate_repo_path(path)?;
            changes.push(StatusChange {
                class: "untracked",
                path: path.to_owned(),
            });
            continue;
        }
        if record.starts_with("! ") {
            // --ignored=no should exclude these, but reject an unexpected
            // record rather than silently treating it as repository state.
            return Err("git status returned an ignored path despite --ignored=no".to_owned());
        }
        let kind = record.as_bytes().first().copied();
        match kind {
            Some(b'1') | Some(b'u') => {
                let field_count = if kind == Some(b'1') { 9 } else { 11 };
                let path = record
                    .splitn(field_count, ' ')
                    .nth(field_count - 1)
                    .ok_or_else(|| "git status record has no path field".to_owned())?;
                validate_repo_path(path)?;
                changes.push(StatusChange {
                    class: status_class(record),
                    path: path.to_owned(),
                });
            }
            Some(b'2') => {
                let path = record
                    .split_once('\t')
                    .map(|(_, path)| path)
                    .or_else(|| record.splitn(10, ' ').nth(9))
                    .ok_or_else(|| "git rename status record has no path field".to_owned())?;
                validate_repo_path(path)?;
                changes.push(StatusChange {
                    class: "rename",
                    path: path.to_owned(),
                });
                let old = records.get(index).copied().ok_or_else(|| {
                    "git rename status record is missing its original path".to_owned()
                })?;
                index += 1;
                validate_repo_path(old)?;
                changes.push(StatusChange {
                    class: "rename",
                    path: old.to_owned(),
                });
            }
            Some(other) => {
                return Err(format!(
                    "unsupported git status record type `{}`",
                    other as char
                ));
            }
            None => return Err("git status returned an empty record".to_owned()),
        }
    }
    Ok(changes)
}

fn status_class(record: &str) -> &'static str {
    let bytes = record.as_bytes();
    let xy = bytes.get(2..4).unwrap_or_default();
    if xy.contains(&b'A') {
        "added"
    } else if xy.contains(&b'D') {
        "deleted"
    } else if xy.contains(&b'T') {
        "type"
    } else {
        "status"
    }
}

fn validate_repo_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("repository path is empty".to_owned());
    }
    if Path::new(path).is_absolute() {
        return Err(format!("repository path `{path}` is absolute"));
    }
    if path.contains('\0') {
        return Err(format!("repository path `{path}` contains NUL"));
    }
    let components = Path::new(path).components();
    for component in components {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {
                return Err(format!("repository path `{path}` contains `.`"));
            }
            Component::ParentDir => {
                return Err(format!("repository path `{path}` escapes repository root"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("repository path `{path}` is absolute"));
            }
        }
    }
    Ok(())
}

fn parse_line(bytes: &[u8], command: &str) -> Result<String, String> {
    let mut line = utf8(bytes, command)?;
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    if line.contains('\n') || line.contains('\r') || line.is_empty() {
        return Err(format!("{command} returned an invalid single-line value"));
    }
    Ok(line)
}

fn utf8(bytes: &[u8], command: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| format!("{command} returned a non-UTF-8 path or value"))
}

fn git_output(working_directory: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .current_dir(working_directory)
        .output()
        .map_err(|error| format!("could not run `git {}`: {error}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            return Err(format!(
                "`git {}` failed with exit status {}",
                args.join(" "),
                output.status
            ));
        }
        return Err(format!("`git {}` failed: {detail}", args.join(" ")));
    }
    Ok(output.stdout)
}

fn os_str_bytes(value: &OsStr) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value
            .to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| "symlink target is not valid UTF-8".to_owned())
    }
}

fn regular_mode(metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            return "100755".to_owned();
        }
    }
    "100644".to_owned()
}

fn is_git_mode(mode: &str) -> bool {
    matches!(mode, "100644" | "100755" | "120000" | "160000")
}

fn is_hex_oid(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn serialize_checkpoint(checkpoint: &Checkpoint, label: &str) -> Result<Vec<u8>, String> {
    serde_json::to_vec(checkpoint).map_err(|error| format!("could not serialize {label}: {error}"))
}

fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "software-change-checkpoint-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp directory");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn digest_has_frozen_prefix_and_width() {
        let digest = sha256_digest(b"bytes");
        assert!(is_digest(&digest));
        assert_eq!(digest.len(), 71);
    }

    #[test]
    fn path_validation_rejects_escape_and_absolute_paths() {
        assert!(validate_repo_path("a/b").is_ok());
        assert!(validate_repo_path("").is_err());
        assert!(validate_repo_path("../outside").is_err());
        assert!(validate_repo_path("a/../outside").is_err());
        assert!(validate_repo_path("/absolute").is_err());
    }

    #[test]
    fn state_object_has_no_state_digest() {
        let repository = RepositoryWithoutState {
            head: "a".repeat(40),
            index_sha256: sha256_digest(b"index"),
            status_sha256: sha256_digest(b"status"),
            entries: Vec::new(),
        };
        let bytes = serde_json::to_vec(&repository).expect("state JSON");
        let text = String::from_utf8(bytes).expect("state UTF-8");
        assert_eq!(
            text,
            format!(
                "{{\"head\":\"{}\",\"index_sha256\":\"{}\",\"status_sha256\":\"{}\",\"entries\":[]}}",
                repository.head, repository.index_sha256, repository.status_sha256
            )
        );
    }

    #[test]
    fn existing_directory_requires_absolute_path() {
        let temp = TempDir::new();
        assert!(existing_absolute_directory(&temp.path, "root").is_ok());
        assert!(existing_absolute_directory(Path::new("."), "root").is_err());
    }

    fn sample_checkpoint() -> Checkpoint {
        Checkpoint {
            schema_version: SCHEMA_VERSION.to_owned(),
            phase: "implementation".to_owned(),
            report: ReportIdentity {
                file: IMPLEMENTATION_REPORT.to_owned(),
                revision: "9".to_owned(),
                sha256: sha256_digest(b"report"),
            },
            documents: DocumentRevisions {
                intent_revision: "4".to_owned(),
                design_revision: "4".to_owned(),
                plan_revision: "7".to_owned(),
            },
            repository: RepositoryIdentity {
                head: "a".repeat(40),
                index_sha256: sha256_digest(b"index"),
                status_sha256: sha256_digest(b"status"),
                entries: Vec::new(),
                state_sha256: sha256_digest(b"state"),
            },
        }
    }

    #[test]
    fn accepted_implementation_selection_fails_closed() {
        let current = sample_checkpoint();
        assert!(select_accepted_implementation(&current, &[])
            .expect_err("missing history must fail")
            .contains("no accepted checkpoint"));
        assert!(select_accepted_implementation(&current, std::slice::from_ref(&current)).is_ok());

        let mut differing = current.clone();
        differing.report.sha256 = sha256_digest(b"different report");
        assert!(
            select_accepted_implementation(&current, &[differing.clone()])
                .expect_err("differing proof must fail")
                .contains("report or document revisions differ")
        );
        assert!(
            select_accepted_implementation(&current, &[current.clone(), differing])
                .expect_err("ambiguous history must fail")
                .contains("is ambiguous")
        );
    }

    #[test]
    fn accepted_implementation_history_is_required() {
        let temp = TempDir::new();
        assert!(implementation_proof_history(&temp.path, false)
            .expect_err("missing history directory must fail")
            .contains("validation requires accepted implementation proof"));
    }
}
