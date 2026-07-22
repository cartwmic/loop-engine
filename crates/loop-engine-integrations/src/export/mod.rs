//! Consistent read-only audit export (T116).
//!
//! Reads one SQLite snapshot inside a single deferred transaction, encodes
//! versioned `state.json` / `journal.jsonl`, and publishes via sibling staging
//! plus atomic rename. Export never mutates SQLite or dereferences locators.

pub mod journal_jsonl;
pub mod state_json;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use loop_engine_core::capabilities::audit_export::{AuditExporter, AuditSnapshot, ExportTarget};
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::bounded::BoundError;
use loop_engine_core::model::ids::{RegistrationId, RunId};
use loop_engine_core::model::provider::{DigestObservation, ProviderObservation};
use loop_engine_core::model::time::ObservedAt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::persistence::export_snapshot::{self, ExportSnapshot, ExportSnapshotError};
use crate::persistence::mapping::{self, MappingError};
use crate::persistence::records::{JournalRecord, RunRecord};
use crate::persistence::traced::{
    MutationClass, OptionalTraceSink, ReadCompleteExtras, close_read,
};
use crate::system_clock::SystemTimeSource;

const EXPORT_SCHEMA_VERSION: u32 = 1;
const EXPORT_MANIFEST_SCHEMA_VERSION: u32 = 1;
const EXPORT_OPERATION: &str = "run.export";
const STAGING_PREFIX: &str = ".loop-export-staging-";
const MANIFEST_FILENAME: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_EXPORT_PAYLOAD_FILES: usize = 2;
const MAX_UNIQUE_NAME_ATTEMPTS: u32 = 32;
const MANIFEST_TOP_LEVEL_KEYS: &[&str] = &[
    "export_manifest_schema_version",
    "export_schema_version",
    "exported_at",
    "files",
    "run_id",
];
const MANIFEST_FILE_ENTRY_KEYS: &[&str] = &["bytes", "path", "sha256"];
const EXPORT_PAYLOAD_INVENTORY: &[&str] = &["journal.jsonl", "state.json"];

/// SQLite-backed audit exporter implementing [`AuditExporter`].
#[derive(Debug, Clone)]
pub struct SqliteAuditExporter<C = SystemTimeSource> {
    path: PathBuf,
    clock: C,
    trace: OptionalTraceSink,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("run not found: {run_id}")]
    RunNotFound { run_id: RunId },
    #[error("export target is invalid: {message}")]
    TargetInvalid { message: String },
    #[error("export target is not empty")]
    TargetNotEmpty,
    #[error("persistence read failed: {message}")]
    PersistenceFailed { message: String },
    #[error("resource exhausted: {message}")]
    ResourceExhausted { message: String },
    #[error(transparent)]
    Bound(#[from] BoundError),
}

#[derive(Debug, Clone)]
pub(crate) struct ManifestFileEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct ExportPublicationMetadata {
    manifest_digest: String,
    artifact_byte_lengths: BTreeMap<String, u64>,
}

struct ExportReadOutcome {
    audit: AuditSnapshot,
    publication: ExportPublicationMetadata,
}

impl SqliteAuditExporter<SystemTimeSource> {
    /// Untraced bootstrap constructor (tests and internal wiring without an operational trace).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_trace(path, OptionalTraceSink::none())
    }

    pub fn with_trace(path: impl Into<PathBuf>, trace: OptionalTraceSink) -> Self {
        Self {
            path: path.into(),
            clock: SystemTimeSource,
            trace,
        }
    }
}

impl<C: TimeSource> SqliteAuditExporter<C> {
    pub fn with_clock(path: impl Into<PathBuf>, clock: C) -> Self {
        Self {
            path: path.into(),
            clock,
            trace: OptionalTraceSink::none(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn export_to_directory(
        &self,
        run_id: &RunId,
        target: &ExportTarget,
    ) -> Result<AuditSnapshot, ExportError> {
        self.export_consistent(run_id, target)
    }
}

impl<C: TimeSource> AuditExporter for SqliteAuditExporter<C> {
    type Error = ExportError;

    fn export_consistent(
        &self,
        run_id: &RunId,
        target: &ExportTarget,
    ) -> Result<AuditSnapshot, ExportError> {
        close_read(
            &self.trace,
            EXPORT_OPERATION,
            MutationClass::ExportRead,
            || {
                let target_path = anchored_target_path(target.as_str())?;
                validate_export_target(&target_path)?;
                let snapshot =
                    export_snapshot::load_consistent_snapshot(&self.path, run_id, &self.clock)
                        .map_err(map_snapshot_error)?;
                let audit = audit_snapshot_from_export(&snapshot)?;
                let publication = publish_artifacts(&target_path, &snapshot, &snapshot.run_record)?;
                Ok(ExportReadOutcome { audit, publication })
            },
            |outcome| ReadCompleteExtras {
                manifest_digest: Some(outcome.publication.manifest_digest.clone()),
                artifact_byte_lengths: Some(outcome.publication.artifact_byte_lengths.clone()),
                ..ReadCompleteExtras::default()
            },
            export_read_rejected,
            export_read_failure,
        )
        .map(|outcome| outcome.audit)
    }
}

fn audit_snapshot_from_export(snapshot: &ExportSnapshot) -> Result<AuditSnapshot, ExportError> {
    let run = mapping::run_from_record(&snapshot.run_record).map_err(map_mapping_error)?;
    let evidence = snapshot
        .evidence_rows
        .iter()
        .map(mapping::evidence_from_record)
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_mapping_error)?;
    let provider_observations = provider_observations_from_records(&snapshot.journal_records)?;
    Ok(AuditSnapshot {
        run,
        journal: snapshot.journal_entries.clone(),
        evidence,
        provider_observations,
    })
}

fn validate_export_target(path: &Path) -> Result<(), ExportError> {
    let path_str = path.to_str().ok_or_else(|| ExportError::TargetInvalid {
        message: "output path is not valid UTF-8".into(),
    })?;
    ExportTarget::parse(path_str).map_err(|error| ExportError::TargetInvalid {
        message: error.to_string(),
    })?;
    if path.exists() {
        if !path.is_dir() {
            return Err(ExportError::TargetInvalid {
                message: "output path exists and is not a directory".into(),
            });
        }
        match fs::read_dir(path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(ExportError::TargetNotEmpty);
                }
            }
            Err(source) => return Err(map_target_io_error(source)),
        }
    }
    let parent = export_target_parent(path)?;
    if !parent.exists() {
        return Err(ExportError::TargetInvalid {
            message: "parent directory does not exist".into(),
        });
    }
    let metadata = parent.metadata().map_err(map_target_io_error)?;
    if !metadata.is_dir() {
        return Err(ExportError::TargetInvalid {
            message: "parent path is not a directory".into(),
        });
    }
    verify_parent_writable(parent)
}

fn export_target_parent(path: &Path) -> Result<&Path, ExportError> {
    match path.parent() {
        None => Err(ExportError::TargetInvalid {
            message: "output path has no parent directory".into(),
        }),
        Some(parent) if parent.as_os_str().is_empty() => Ok(Path::new(".")),
        Some(parent) => Ok(parent),
    }
}

fn verify_parent_writable(parent: &Path) -> Result<(), ExportError> {
    let probe = create_unique_directory(parent, || unique_staging_name("probe"))
        .map_err(map_target_io_error)?
        .ok_or_else(|| ExportError::TargetInvalid {
            message: "parent directory is not writable".into(),
        })?;
    fs::remove_dir(&probe).map_err(map_target_io_error)
}

fn directory_is_empty(path: &Path) -> Result<bool, ExportError> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(source) => Err(map_io_error(source)),
    }
}

fn provider_observations_from_records(
    records: &[JournalRecord],
) -> Result<Vec<ProviderObservation>, ExportError> {
    let mut observations = Vec::new();
    for record in records {
        let root: Value = serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
            ExportError::PersistenceFailed {
                message: format!("journal payload JSON: {error}"),
            }
        })?;
        let Some(items) = root.get("provider_observations").and_then(Value::as_array) else {
            continue;
        };
        let ts = root.get("ts").and_then(Value::as_str).ok_or_else(|| {
            ExportError::PersistenceFailed {
                message: "journal entry missing ts".into(),
            }
        })?;
        let observed_at =
            ObservedAt::parse(ts).map_err(|error| ExportError::PersistenceFailed {
                message: error.to_string(),
            })?;
        for item in items {
            observations.push(provider_observation_from_wire(item, observed_at)?);
        }
    }
    Ok(observations)
}

fn provider_observation_from_wire(
    value: &Value,
    observed_at: ObservedAt,
) -> Result<ProviderObservation, ExportError> {
    let registration_id = RegistrationId::parse(
        value
            .get("registration_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ExportError::PersistenceFailed {
                message: "provider_observation.registration_id missing".into(),
            })?
            .to_owned(),
    )
    .map_err(|error| ExportError::PersistenceFailed {
        message: error.to_string(),
    })?;
    let executable = value
        .get("executable")
        .and_then(Value::as_str)
        .ok_or_else(|| ExportError::PersistenceFailed {
            message: "provider_observation.executable missing".into(),
        })?;
    let digest = match value.get("executable_digest").and_then(Value::as_str) {
        Some(raw) => DigestObservation::observed(raw.to_owned()).map_err(ExportError::Bound)?,
        None => DigestObservation::Unavailable,
    };
    let version = value
        .get("provider_version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    ProviderObservation::new(registration_id, executable, digest, version, observed_at)
        .map_err(ExportError::Bound)
}

fn publish_artifacts(
    target_path: &Path,
    snapshot: &ExportSnapshot,
    run_record: &RunRecord,
) -> Result<ExportPublicationMetadata, ExportError> {
    let parent = export_target_parent(target_path)?;
    let staging = create_staging_directory(parent)?;
    let publish_result = (|| {
        let state_bytes = state_json::encode_state_json(
            run_record,
            &snapshot.evidence_rows,
            &snapshot.exported_at,
        )?;
        write_payload_file(&staging, "state.json", &state_bytes)?;
        let journal_bytes = journal_jsonl::encode_journal_jsonl(&snapshot.journal_records)?;
        write_payload_file(&staging, "journal.jsonl", &journal_bytes)?;
        let state_entry = inventory_staged_payload(&staging, "state.json", state_bytes.len())?;
        let journal_entry =
            inventory_staged_payload(&staging, "journal.jsonl", journal_bytes.len())?;
        let manifest_bytes = encode_manifest(
            &run_record.run_id,
            &snapshot.exported_at,
            &[journal_entry, state_entry],
        )?;
        write_payload_file(&staging, "manifest.json", &manifest_bytes)?;
        sync_directory(&staging)?;
        atomic_publish(&staging, target_path)?;
        resolve_post_rename_publication(target_path, sync_directory(parent))
    })();
    finalize_publication_result(publish_result, &staging)
}

fn finalize_publication_result(
    result: Result<ExportPublicationMetadata, ExportError>,
    staging: &Path,
) -> Result<ExportPublicationMetadata, ExportError> {
    if result.is_ok() {
        return result;
    }
    let cleanup_result = match fs::symlink_metadata(staging) {
        Ok(_) => remove_staging_directory(staging),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(map_io_error(source)),
    };
    match (&result, cleanup_result) {
        // Frozen collision semantics take precedence: every publication loser
        // reports TargetNotEmpty even when its private staging cleanup also fails.
        (Err(ExportError::TargetNotEmpty), _) => result,
        (_, Ok(())) => result,
        (_, Err(cleanup_error)) => Err(cleanup_error),
    }
}

fn anchored_target_path(target: &str) -> Result<PathBuf, ExportError> {
    let path = PathBuf::from(target);
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(map_target_io_error)
}

fn create_staging_directory(parent: &Path) -> Result<PathBuf, ExportError> {
    create_unique_directory(parent, || unique_staging_name(""))
        .map_err(map_target_io_error)?
        .ok_or_else(|| ExportError::ResourceExhausted {
            message: "unable to allocate unique staging directory".into(),
        })
}

fn create_unique_directory(
    parent: &Path,
    mut next_name: impl FnMut() -> String,
) -> Result<Option<PathBuf>, std::io::Error> {
    for _ in 0..MAX_UNIQUE_NAME_ATTEMPTS {
        let candidate = parent.join(next_name());
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return Ok(Some(candidate)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(source),
        }
    }
    Ok(None)
}

fn unique_staging_name(kind: &str) -> String {
    let nonce = uuid::Uuid::now_v7();
    if kind.is_empty() {
        format!("{STAGING_PREFIX}{nonce}")
    } else {
        format!("{STAGING_PREFIX}{kind}-{nonce}")
    }
}

fn write_payload_file(staging: &Path, name: &str, bytes: &[u8]) -> Result<(), ExportError> {
    let path = staging.join(name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(map_io_error)?;
    file.write_all(bytes).map_err(map_io_error)?;
    file.sync_all().map_err(map_io_error)?;
    Ok(())
}

fn read_staged_payload_bytes(
    staging: &Path,
    name: &str,
    expected_bytes: usize,
) -> Result<Vec<u8>, ExportError> {
    let path = staging.join(name);
    let metadata = fs::metadata(&path).map_err(map_io_error)?;
    let on_disk_len = metadata.len();
    let on_disk_len = usize::try_from(on_disk_len).map_err(|_| ExportError::ResourceExhausted {
        message: format!("staged payload `{name}` exceeds addressable size"),
    })?;
    if on_disk_len != expected_bytes {
        return Err(ExportError::ResourceExhausted {
            message: format!(
                "staged payload `{name}` on-disk length {on_disk_len} != expected {expected_bytes}"
            ),
        });
    }
    let mut file = File::open(&path).map_err(map_io_error)?;
    let mut bytes = vec![0u8; expected_bytes];
    file.read_exact(&mut bytes).map_err(map_io_error)?;
    let mut extra = [0u8; 1];
    match file.read(&mut extra) {
        Ok(0) => Ok(bytes),
        Ok(_) => Err(ExportError::ResourceExhausted {
            message: format!("staged payload `{name}` has trailing bytes beyond expected length"),
        }),
        Err(source) => Err(map_io_error(source)),
    }
}

fn inventory_staged_payload(
    staging: &Path,
    name: &str,
    expected_bytes: usize,
) -> Result<ManifestFileEntry, ExportError> {
    let bytes = read_staged_payload_bytes(staging, name, expected_bytes)?;
    Ok(ManifestFileEntry {
        path: name.to_owned(),
        sha256: sha256_label(&bytes),
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    })
}

fn publication_metadata_from_target(
    target: &Path,
) -> Result<ExportPublicationMetadata, ExportError> {
    let manifest_path = target.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(map_io_error)?;
    let manifest_digest = sha256_hex(&manifest_bytes);
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        ExportError::PersistenceFailed {
            message: format!("manifest.json: {error}"),
        }
    })?;
    let mut artifact_byte_lengths = BTreeMap::new();
    if let Some(files) = manifest.get("files").and_then(Value::as_array) {
        for file in files {
            let path = file.get("path").and_then(Value::as_str).ok_or_else(|| {
                ExportError::PersistenceFailed {
                    message: "manifest file entry missing path".into(),
                }
            })?;
            let bytes = file.get("bytes").and_then(Value::as_u64).ok_or_else(|| {
                ExportError::PersistenceFailed {
                    message: format!("manifest file entry `{path}` missing bytes"),
                }
            })?;
            artifact_byte_lengths.insert(path.to_owned(), bytes);
        }
    }
    Ok(ExportPublicationMetadata {
        manifest_digest,
        artifact_byte_lengths,
    })
}

fn encode_manifest(
    run_id: &str,
    exported_at: &str,
    files: &[ManifestFileEntry],
) -> Result<Vec<u8>, ExportError> {
    let mut file_objects = Vec::with_capacity(files.len());
    for entry in files {
        file_objects.push(json!({
            "bytes": entry.bytes,
            "path": entry.path,
            "sha256": entry.sha256,
        }));
    }
    let manifest = json!({
        "export_manifest_schema_version": EXPORT_MANIFEST_SCHEMA_VERSION,
        "export_schema_version": EXPORT_SCHEMA_VERSION,
        "exported_at": exported_at,
        "files": file_objects,
        "run_id": run_id,
    });
    canonical_json_bytes(&manifest)
}

fn resolve_post_rename_publication(
    target: &Path,
    parent_sync_result: Result<(), ExportError>,
) -> Result<ExportPublicationMetadata, ExportError> {
    parent_sync_result?;
    publication_metadata_from_target(target)
}

/// Verify a previously published export from a fresh process after an
/// earlier invocation reported post-rename durability uncertainty.
///
/// Verification is observational: it never converts a new publication
/// attempt into success and never mutates or removes the target.
pub fn verify_export_directory(
    target: &ExportTarget,
    expected_run_id: &RunId,
) -> Result<(), ExportError> {
    let target = anchored_target_path(target.as_str())?;
    verify_published_export(&target, expected_run_id.as_str()).map(|_| ())
}

fn verify_published_export(
    target: &Path,
    expected_run_id: &str,
) -> Result<ExportPublicationMetadata, ExportError> {
    if !target.is_dir() {
        return Err(verification_failed("export target is not a directory"));
    }
    let manifest_path = target.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        return Err(verification_failed("manifest.json missing"));
    }
    let manifest_bytes = read_bounded_file_bytes(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest_digest = sha256_hex(&manifest_bytes);
    let manifest = verify_canonical_json_bytes(&manifest_bytes, MANIFEST_FILENAME)?;
    verify_object_has_exact_keys(&manifest, MANIFEST_TOP_LEVEL_KEYS, "manifest.json")?;
    let manifest_schema = manifest
        .get("export_manifest_schema_version")
        .and_then(Value::as_u64);
    if manifest_schema != Some(u64::from(EXPORT_MANIFEST_SCHEMA_VERSION)) {
        return Err(verification_failed(
            "manifest export_manifest_schema_version invalid",
        ));
    }
    let export_schema = manifest
        .get("export_schema_version")
        .and_then(Value::as_u64);
    if export_schema != Some(u64::from(EXPORT_SCHEMA_VERSION)) {
        return Err(verification_failed(
            "manifest export_schema_version invalid",
        ));
    }
    let run_id = manifest
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| verification_failed("manifest run_id missing"))?;
    if run_id != expected_run_id {
        return Err(verification_failed("manifest run_id mismatch"));
    }
    if manifest
        .get("exported_at")
        .and_then(Value::as_str)
        .is_none()
    {
        return Err(verification_failed("manifest exported_at missing"));
    }
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| verification_failed("manifest files missing"))?;
    if files.len() != MAX_EXPORT_PAYLOAD_FILES {
        return Err(verification_failed("manifest files count invalid"));
    }
    let mut artifact_byte_lengths = BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        verify_object_has_exact_keys(file, MANIFEST_FILE_ENTRY_KEYS, "manifest file entry")?;
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| verification_failed("manifest file entry missing path"))?;
        let expected_path = EXPORT_PAYLOAD_INVENTORY[index];
        if path != expected_path {
            return Err(verification_failed(format!(
                "manifest file entry at index {index} must be `{expected_path}`"
            )));
        }
        if !is_safe_export_payload_filename(path) {
            return Err(verification_failed(format!(
                "manifest file entry `{path}` has unsafe path"
            )));
        }
        let expected_sha = file.get("sha256").and_then(Value::as_str).ok_or_else(|| {
            verification_failed(format!("manifest file entry `{path}` missing sha256"))
        })?;
        if !is_valid_sha256_label(expected_sha) {
            return Err(verification_failed(format!(
                "manifest file entry `{path}` has invalid sha256 label"
            )));
        }
        let expected_bytes = file.get("bytes").and_then(Value::as_u64).ok_or_else(|| {
            verification_failed(format!("manifest file entry `{path}` missing bytes"))
        })?;
        let payload_bytes = read_payload_bytes_exact(target, path, expected_bytes)?;
        if sha256_label(&payload_bytes) != expected_sha {
            return Err(verification_failed(format!(
                "payload `{path}` sha256 mismatch"
            )));
        }
        artifact_byte_lengths.insert(path.to_owned(), expected_bytes);
    }
    verify_export_directory_inventory(target)?;
    Ok(ExportPublicationMetadata {
        manifest_digest,
        artifact_byte_lengths,
    })
}

fn verify_canonical_json_bytes(bytes: &[u8], field: &str) -> Result<Value, ExportError> {
    let parsed: Value = serde_json::from_slice(bytes)
        .map_err(|error| verification_failed(format!("{field}: {error}")))?;
    let canonical =
        canonical_json_bytes(&parsed).map_err(|error| verification_failed(error.to_string()))?;
    if canonical.as_slice() != bytes {
        return Err(verification_failed(format!(
            "{field} is not canonical JSON"
        )));
    }
    Ok(parsed)
}

fn verify_object_has_exact_keys(
    value: &Value,
    expected: &[&str],
    context: &str,
) -> Result<(), ExportError> {
    let map = value
        .as_object()
        .ok_or_else(|| verification_failed(format!("{context} must be a JSON object")))?;
    if map.len() != expected.len() {
        return Err(verification_failed(format!(
            "{context} has invalid key set"
        )));
    }
    for key in expected {
        if !map.contains_key(*key) {
            return Err(verification_failed(format!(
                "{context} missing required key `{key}`"
            )));
        }
    }
    Ok(())
}

fn verify_export_directory_inventory(target: &Path) -> Result<(), ExportError> {
    let expected: BTreeSet<String> = [
        MANIFEST_FILENAME,
        EXPORT_PAYLOAD_INVENTORY[0],
        EXPORT_PAYLOAD_INVENTORY[1],
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut on_disk = BTreeSet::new();
    for entry in fs::read_dir(target).map_err(map_io_error)? {
        let entry = entry.map_err(map_io_error)?;
        let file_type = entry.file_type().map_err(map_io_error)?;
        if !file_type.is_file() {
            return Err(verification_failed(
                "export target contains non-regular filesystem entry",
            ));
        }
        on_disk.insert(entry.file_name().to_string_lossy().into_owned());
    }
    if on_disk != expected {
        return Err(verification_failed("export target file inventory mismatch"));
    }
    Ok(())
}

fn read_bounded_file_bytes(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ExportError> {
    let metadata = fs::metadata(path).map_err(map_io_error)?;
    let on_disk_len = metadata.len();
    if on_disk_len > max_bytes {
        return Err(ExportError::ResourceExhausted {
            message: format!(
                "file `{}` length {on_disk_len} exceeds bound {max_bytes}",
                path.display()
            ),
        });
    }
    read_file_bytes_exact(path, on_disk_len)
}

fn read_payload_bytes_exact(
    target: &Path,
    name: &str,
    expected_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    if !is_safe_export_payload_filename(name) {
        return Err(verification_failed(format!(
            "payload `{name}` has unsafe filename"
        )));
    }
    let path = target.join(name);
    let metadata = fs::metadata(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            verification_failed(format!("payload `{name}` missing"))
        } else {
            map_io_error(source)
        }
    })?;
    let on_disk_len = metadata.len();
    if on_disk_len != expected_bytes {
        return Err(verification_failed(format!(
            "payload `{name}` on-disk length {on_disk_len} != expected {expected_bytes}"
        )));
    }
    read_file_bytes_exact(&path, on_disk_len)
}

fn read_file_bytes_exact(path: &Path, expected_bytes: u64) -> Result<Vec<u8>, ExportError> {
    let expected_bytes =
        usize::try_from(expected_bytes).map_err(|_| ExportError::ResourceExhausted {
            message: format!("file `{}` exceeds addressable size", path.display()),
        })?;
    let mut file = File::open(path).map_err(map_io_error)?;
    let mut bytes = vec![0u8; expected_bytes];
    file.read_exact(&mut bytes).map_err(map_io_error)?;
    let mut extra = [0u8; 1];
    match file.read(&mut extra) {
        Ok(0) => Ok(bytes),
        Ok(_) => Err(ExportError::ResourceExhausted {
            message: format!(
                "file `{}` has trailing bytes beyond expected length",
                path.display()
            ),
        }),
        Err(source) => Err(map_io_error(source)),
    }
}

fn is_safe_export_payload_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && (name == "journal.jsonl" || name == "state.json")
}

fn is_valid_sha256_label(label: &str) -> bool {
    let Some(hex) = label.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn verification_failed(message: impl Into<String>) -> ExportError {
    ExportError::ResourceExhausted {
        message: message.into(),
    }
}

fn atomic_publish(staging: &Path, target: &Path) -> Result<(), ExportError> {
    match fs::rename(staging, target) {
        Ok(()) => Ok(()),
        Err(source) if target.exists() => {
            if !target.is_dir() {
                return Err(ExportError::TargetNotEmpty);
            }
            if directory_is_empty(target)? {
                return Err(map_target_io_error(source));
            }
            Err(ExportError::TargetNotEmpty)
        }
        Err(source) => Err(map_target_io_error(source)),
    }
}

fn remove_staging_directory(staging: &Path) -> Result<(), ExportError> {
    if staging.exists() {
        restore_staging_permissions(staging)?;
        fs::remove_dir_all(staging).map_err(map_io_error)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restore_staging_permissions(staging: &Path) -> Result<(), ExportError> {
    use std::os::unix::fs::PermissionsExt;

    let mut dir_perms = fs::metadata(staging).map_err(map_io_error)?.permissions();
    dir_perms.set_mode(0o700);
    fs::set_permissions(staging, dir_perms).map_err(map_io_error)?;
    for entry in fs::read_dir(staging).map_err(map_io_error)? {
        let entry = entry.map_err(map_io_error)?;
        let mut perms = entry.metadata().map_err(map_io_error)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(entry.path(), perms).map_err(map_io_error)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restore_staging_permissions(_staging: &Path) -> Result<(), ExportError> {
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), ExportError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(map_io_error)
}

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, ExportError> {
    Ok(canonical_json(value).into_bytes())
}

pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&canonical_value(value)).expect("canonical json")
}

pub(crate) fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .iter()
                .map(|(key, value)| (key.as_str(), canonical_value(value)))
                .collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

pub fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn export_read_rejected(error: &ExportError) -> bool {
    matches!(
        error,
        ExportError::RunNotFound { .. }
            | ExportError::TargetInvalid { .. }
            | ExportError::TargetNotEmpty
    )
}

fn export_read_failure(error: &ExportError) -> (&'static str, Option<String>) {
    let code = match error {
        ExportError::ResourceExhausted { .. } | ExportError::Bound(_) => "resource.exhausted",
        ExportError::TargetInvalid { .. } => "export.target.invalid",
        ExportError::TargetNotEmpty => "export.target.not_empty",
        ExportError::RunNotFound { .. } => "run.not_found",
        ExportError::PersistenceFailed { .. } => "persistence.failed",
    };
    (code, Some(error.to_string()))
}

fn map_snapshot_error(error: ExportSnapshotError) -> ExportError {
    match error {
        ExportSnapshotError::RunNotFound { run_id } => ExportError::RunNotFound { run_id },
        ExportSnapshotError::Failed { message } => ExportError::PersistenceFailed { message },
        ExportSnapshotError::Bound(bound) => ExportError::Bound(bound),
    }
}

pub(super) fn map_mapping_error(error: MappingError) -> ExportError {
    ExportError::PersistenceFailed {
        message: error.to_string(),
    }
}

fn map_target_io_error(error: std::io::Error) -> ExportError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            ExportError::TargetInvalid {
                message: error.to_string(),
            }
        }
        _ => map_io_error(error),
    }
}

fn map_io_error(error: std::io::Error) -> ExportError {
    ExportError::ResourceExhausted {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::export_snapshot::test_support::seed_minimal_run;
    use crate::persistence::traced::test_support::{event_names, read_events, test_sink};
    use crate::persistence::traced::{MutationClass, ReadCompleteExtras, close_read};
    use crate::persistence::{OptionalTraceSink, SqliteStore};
    use crate::system_clock::SystemTimeSource;
    use loop_engine_core::capabilities::audit_export::ExportTarget;
    use tempfile::TempDir;

    fn orphan_staging_exists(parent: &Path) -> bool {
        fs::read_dir(parent)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(STAGING_PREFIX)
            })
    }

    #[test]
    fn export_snapshot_read_emits_intent_and_read_complete() {
        let (trace_dir, _writer, sink) = test_sink("export-read-success");
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        seed_minimal_run(store.connection(), "run-export-trace");

        let output = root.path().join("export-out");
        let exporter =
            SqliteAuditExporter::with_trace(&db_path, OptionalTraceSink { inner: Some(sink) });
        exporter
            .export_consistent(
                &RunId::parse("run-export-trace").unwrap(),
                &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
            )
            .expect("export");

        let events = read_events(&trace_dir.trace_dir().join("export-read-success.jsonl"));
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["mutation_class"], "export_read");
        assert_eq!(events[0]["operation"], "run.export");
        assert_eq!(events[1]["outcome"], "completed");

        let manifest_bytes = fs::read(output.join("manifest.json")).expect("manifest");
        assert_eq!(
            events[1]["manifest_digest"].as_str(),
            Some(sha256_hex(&manifest_bytes).as_str())
        );
        let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("manifest json");
        let mut expected_lengths = BTreeMap::new();
        for file in manifest["files"].as_array().expect("files") {
            expected_lengths.insert(
                file["path"].as_str().expect("path").to_owned(),
                file["bytes"].as_u64().expect("bytes"),
            );
        }
        let traced_lengths: BTreeMap<String, u64> = events[1]["artifact_byte_lengths"]
            .as_object()
            .expect("artifact_byte_lengths")
            .iter()
            .map(|(key, value)| (key.clone(), value.as_u64().expect("byte length")))
            .collect();
        assert_eq!(traced_lengths, expected_lengths);
    }

    #[test]
    fn export_snapshot_read_missing_run_emits_read_complete_rejected() {
        let (trace_dir, _writer, sink) = test_sink("export-read-rejected");
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        SqliteStore::open(&db_path).unwrap();

        let output = root.path().join("export-out");
        let exporter =
            SqliteAuditExporter::with_trace(&db_path, OptionalTraceSink { inner: Some(sink) });
        let error = exporter
            .export_consistent(
                &RunId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
                &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
            )
            .expect_err("missing run");
        assert!(matches!(error, ExportError::RunNotFound { .. }));

        let events = read_events(&trace_dir.trace_dir().join("export-read-rejected.jsonl"));
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["mutation_class"], "export_read");
        assert_eq!(events[1]["outcome"], "rejected");
    }

    #[test]
    fn map_target_io_error_maps_target_create_failures_to_rejected_invalid() {
        use std::io::{Error, ErrorKind};

        for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem] {
            let error = map_target_io_error(Error::from(kind));
            assert!(
                matches!(error, ExportError::TargetInvalid { .. }),
                "{kind:?} should be TargetInvalid"
            );
            assert!(export_read_rejected(&error));
            let (code, _) = export_read_failure(&error);
            assert_eq!(code, "export.target.invalid");
        }

        let other = map_target_io_error(Error::from(ErrorKind::Interrupted));
        assert!(matches!(other, ExportError::ResourceExhausted { .. }));
        assert!(!export_read_rejected(&other));
        let (code, _) = export_read_failure(&other);
        assert_eq!(code, "resource.exhausted");
    }

    #[test]
    fn export_invalid_target_emits_read_complete_rejected() {
        let (trace_dir, _writer, sink) = test_sink("export-read-invalid-target");
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        SqliteStore::open(&db_path).unwrap();
        let invalid_target = root.path().join("missing-parent").join("export-out");
        let exporter =
            SqliteAuditExporter::with_trace(&db_path, OptionalTraceSink { inner: Some(sink) });

        let error = exporter
            .export_consistent(
                &RunId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
                &ExportTarget::parse(invalid_target.to_str().unwrap()).unwrap(),
            )
            .expect_err("invalid target");
        assert!(matches!(error, ExportError::TargetInvalid { .. }));
        assert!(export_read_rejected(&error));
        let (code, _) = export_read_failure(&error);
        assert_eq!(code, "export.target.invalid");

        let events = read_events(
            &trace_dir
                .trace_dir()
                .join("export-read-invalid-target.jsonl"),
        );
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["mutation_class"], "export_read");
        assert_eq!(events[1]["outcome"], "rejected");
    }

    #[test]
    fn export_nonempty_target_emits_read_complete_rejected() {
        let (trace_dir, _writer, sink) = test_sink("export-read-nonempty-target");
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        seed_minimal_run(store.connection(), "run-export-nonempty");

        let output = root.path().join("export-out");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("marker.txt"), b"occupied").unwrap();
        let exporter =
            SqliteAuditExporter::with_trace(&db_path, OptionalTraceSink { inner: Some(sink) });

        let error = exporter
            .export_consistent(
                &RunId::parse("run-export-nonempty").unwrap(),
                &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
            )
            .expect_err("non-empty target");
        assert!(matches!(error, ExportError::TargetNotEmpty));
        assert!(export_read_rejected(&error));
        let (code, _) = export_read_failure(&error);
        assert_eq!(code, "export.target.not_empty");

        let events = read_events(
            &trace_dir
                .trace_dir()
                .join("export-read-nonempty-target.jsonl"),
        );
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[0]["mutation_class"], "export_read");
        assert_eq!(events[1]["outcome"], "rejected");
        assert!(output.join("marker.txt").exists());
        assert!(!output.join("manifest.json").exists());
        assert!(!orphan_staging_exists(output.parent().unwrap()));
    }

    #[test]
    fn export_target_race_emits_rejected_read_completion() {
        let (trace_dir, _writer, sink) = test_sink("export-read-publish-failure");
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        seed_minimal_run(store.connection(), "run-export-fail");

        let output = root.path().join("export-out");
        let parent = output.parent().unwrap().to_path_buf();
        let run_id = RunId::parse("run-export-fail").unwrap();
        let trace = OptionalTraceSink { inner: Some(sink) };

        let target_path = anchored_target_path(output.to_str().unwrap()).unwrap();
        validate_export_target(&target_path).unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("marker.txt"), b"occupied").unwrap();

        let trace_path = trace_dir
            .trace_dir()
            .join("export-read-publish-failure.jsonl");
        let error = close_read(
            &trace,
            EXPORT_OPERATION,
            MutationClass::ExportRead,
            || {
                let snapshot =
                    export_snapshot::load_consistent_snapshot(&db_path, &run_id, &SystemTimeSource)
                        .map_err(map_snapshot_error)?;
                let audit = audit_snapshot_from_export(&snapshot)?;
                publish_artifacts(&target_path, &snapshot, &snapshot.run_record)?;
                Ok(audit)
            },
            |_| ReadCompleteExtras::default(),
            export_read_rejected,
            export_read_failure,
        )
        .expect_err("publication failure");
        assert!(matches!(error, ExportError::TargetNotEmpty));

        let events = read_events(&trace_path);
        assert_eq!(event_names(&events), vec!["intent", "read_complete"]);
        assert_eq!(events[1]["outcome"], "rejected");
        assert!(output.join("marker.txt").exists());
        assert!(!orphan_staging_exists(&parent));
    }

    #[test]
    fn publish_artifacts_cleans_up_staging_when_target_is_not_empty() {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        seed_minimal_run(store.connection(), "run-partial");

        let output = root.path().join("export-partial");
        let parent = output.parent().unwrap().to_path_buf();
        let run_id = RunId::parse("run-partial").unwrap();
        let snapshot =
            export_snapshot::load_consistent_snapshot(&db_path, &run_id, &SystemTimeSource)
                .unwrap();

        let target_path = anchored_target_path(output.to_str().unwrap()).unwrap();
        validate_export_target(&target_path).unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("marker.txt"), b"occupied").unwrap();

        let error = publish_artifacts(&target_path, &snapshot, &snapshot.run_record)
            .expect_err("partial failure");
        assert!(matches!(error, ExportError::TargetNotEmpty));
        assert!(output.join("marker.txt").exists());
        assert!(!orphan_staging_exists(&parent));
    }

    #[test]
    fn finalize_publication_result_preserves_original_error_when_cleanup_succeeds() {
        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        write_payload_file(&staging, "state.json", b"{}").expect("write payload");

        let original = Err(ExportError::TargetNotEmpty);
        let result =
            finalize_publication_result(original, &staging).expect_err("publication error");
        assert!(matches!(result, ExportError::TargetNotEmpty));
        assert!(!staging.exists());
    }

    #[cfg(unix)]
    #[test]
    fn finalize_publication_result_preserves_collision_when_staging_cleanup_fails() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        write_payload_file(&staging, "state.json", b"{}").expect("write payload");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o500))
            .expect("lock staging parent");

        let original = Err(ExportError::TargetNotEmpty);
        let result = finalize_publication_result(original, &staging).expect_err("cleanup failure");
        assert!(matches!(result, ExportError::TargetNotEmpty));
        assert!(staging.exists());

        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .expect("unlock staging parent");
        let _ = remove_staging_directory(&staging);
    }

    #[test]
    fn finalize_publication_result_preserves_success_without_staging_cleanup() {
        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        let metadata = ExportPublicationMetadata {
            manifest_digest: "abc".into(),
            artifact_byte_lengths: BTreeMap::new(),
        };

        let result =
            finalize_publication_result(Ok(metadata.clone()), &staging).expect("publication ok");
        assert_eq!(result, metadata);
        assert!(staging.exists());
        let _ = remove_staging_directory(&staging);
    }

    #[test]
    fn manifest_inventory_uses_on_disk_payload_bytes() {
        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        let memory_bytes = br#"{"from":"memory"}"#;
        write_payload_file(&staging, "state.json", memory_bytes).expect("write payload");
        fs::write(staging.join("state.json"), br#"{"from":"memorx"}"#).expect("tamper payload");

        let entry = inventory_staged_payload(&staging, "state.json", memory_bytes.len())
            .expect("inventory");
        assert_ne!(entry.sha256, sha256_label(memory_bytes));
        assert_eq!(entry.sha256, sha256_label(br#"{"from":"memorx"}"#));
        assert_eq!(entry.bytes, br#"{"from":"memorx"}"#.len() as u64);
    }

    #[test]
    fn read_staged_payload_rejects_on_disk_length_mismatch() {
        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        let expected = b"0123456789";
        write_payload_file(&staging, "state.json", expected).expect("write payload");
        fs::write(staging.join("state.json"), b"short").expect("truncate payload");

        let error = read_staged_payload_bytes(&staging, "state.json", expected.len())
            .expect_err("length mismatch");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn read_staged_payload_accepts_artifacts_exceeding_collection_page_budget() {
        const OVER_PAGE_BUDGET: usize = 3_145_728 + 1;
        let root = TempDir::new().unwrap();
        let staging = create_staging_directory(root.path()).expect("staging dir");
        let payload = vec![b'x'; OVER_PAGE_BUDGET];
        write_payload_file(&staging, "state.json", &payload).expect("write payload");

        let bytes =
            read_staged_payload_bytes(&staging, "state.json", OVER_PAGE_BUDGET).expect("read");
        assert_eq!(bytes.len(), OVER_PAGE_BUDGET);

        let entry =
            inventory_staged_payload(&staging, "state.json", OVER_PAGE_BUDGET).expect("inventory");
        assert_eq!(entry.bytes, OVER_PAGE_BUDGET as u64);
        assert_eq!(entry.sha256, sha256_label(&payload));
    }

    fn seed_complete_export_dir(
        parent: &Path,
        name: &str,
        run_id: &str,
        exported_at: &str,
    ) -> (PathBuf, ExportPublicationMetadata) {
        let target = parent.join(name);
        fs::create_dir_all(&target).expect("export dir");
        let state_bytes = br#"{"export_schema_version":1}"#;
        let journal_bytes = b"{\"journal_schema_version\":1}\n";
        fs::write(target.join("state.json"), state_bytes).expect("state");
        fs::write(target.join("journal.jsonl"), journal_bytes).expect("journal");
        let state_entry = ManifestFileEntry {
            path: "state.json".into(),
            sha256: sha256_label(state_bytes),
            bytes: state_bytes.len() as u64,
        };
        let journal_entry = ManifestFileEntry {
            path: "journal.jsonl".into(),
            sha256: sha256_label(journal_bytes),
            bytes: journal_bytes.len() as u64,
        };
        let manifest_bytes =
            encode_manifest(run_id, exported_at, &[journal_entry, state_entry]).expect("manifest");
        fs::write(target.join("manifest.json"), &manifest_bytes).expect("manifest");
        let metadata = ExportPublicationMetadata {
            manifest_digest: sha256_hex(&manifest_bytes),
            artifact_byte_lengths: BTreeMap::from([
                ("journal.jsonl".into(), journal_bytes.len() as u64),
                ("state.json".into(), state_bytes.len() as u64),
            ]),
        };
        (target, metadata)
    }

    struct CurrentDirGuard(PathBuf);

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).expect("restore cwd");
        }
    }

    fn current_dir_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn anchored_target_path_resolves_relative_paths_once() {
        let _lock = current_dir_test_lock();
        let root = TempDir::new().unwrap();
        let original_cwd = std::env::current_dir().expect("cwd");
        let _restore = CurrentDirGuard(original_cwd);
        std::env::set_current_dir(root.path()).expect("set cwd");

        let relative = "nested/export-out";
        let anchored = anchored_target_path(relative).expect("anchor relative target");
        assert!(anchored.is_absolute());
        assert_eq!(anchored, std::env::current_dir().unwrap().join(relative));
    }

    #[test]
    fn export_with_relative_target_uses_anchored_absolute_path() {
        let _lock = current_dir_test_lock();
        let root = TempDir::new().unwrap();
        let original_cwd = std::env::current_dir().expect("cwd");
        let _restore = CurrentDirGuard(original_cwd);
        std::env::set_current_dir(root.path()).expect("set cwd");

        let db_path = root.path().join("state.db");
        let store = SqliteStore::open(&db_path).unwrap();
        seed_minimal_run(store.connection(), "run-relative-target");

        let relative = "export-relative";
        let exporter = SqliteAuditExporter::new(&db_path);
        exporter
            .export_consistent(
                &RunId::parse("run-relative-target").unwrap(),
                &ExportTarget::parse(relative).unwrap(),
            )
            .expect("export with relative target");

        assert!(
            std::env::current_dir()
                .unwrap()
                .join(relative)
                .join("manifest.json")
                .exists()
        );
    }

    #[test]
    fn resolve_post_rename_publication_fails_after_parent_sync_error_even_when_complete() {
        let root = TempDir::new().unwrap();
        let run_id = "run-fsync-recover";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, expected) =
            seed_complete_export_dir(root.path(), "export-complete", run_id, exported_at);
        let sync_error = ExportError::ResourceExhausted {
            message: "parent fsync failed".into(),
        };

        let error = resolve_post_rename_publication(&target, Err(sync_error))
            .expect_err("same-process parent fsync failure stays uncertain");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
        assert!(target.join("manifest.json").exists());
        assert!(target.join("state.json").exists());
        assert!(target.join("journal.jsonl").exists());

        let recovered = verify_published_export(&target, run_id)
            .expect("fresh-process manifest verification recovers complete export");
        assert_eq!(recovered, expected);
    }

    #[test]
    fn resolve_post_rename_publication_succeeds_when_parent_sync_succeeds() {
        let root = TempDir::new().unwrap();
        let run_id = "run-fsync-success";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, expected) =
            seed_complete_export_dir(root.path(), "export-fsync-success", run_id, exported_at);

        let metadata =
            resolve_post_rename_publication(&target, Ok(())).expect("parent fsync succeeded");
        assert_eq!(metadata, expected);
    }

    #[test]
    fn resolve_post_rename_publication_fails_after_parent_sync_error_when_incomplete() {
        let root = TempDir::new().unwrap();
        let run_id = "run-fsync-incomplete";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-incomplete", run_id, exported_at);
        fs::remove_file(target.join("state.json")).expect("remove payload");
        let sync_error = ExportError::ResourceExhausted {
            message: "parent fsync failed".into(),
        };

        let error = resolve_post_rename_publication(&target, Err(sync_error))
            .expect_err("incomplete export");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
        assert!(!target.join("state.json").exists());
        assert!(target.join("manifest.json").exists());
    }

    #[test]
    fn atomic_publish_loser_returns_target_not_empty_when_winner_already_published() {
        let root = TempDir::new().unwrap();
        let run_id = "run-race-loser";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-race", run_id, exported_at);
        let staging = create_staging_directory(root.path()).expect("staging");
        write_payload_file(&staging, "state.json", b"orphan").expect("orphan payload");

        let error =
            atomic_publish(&staging, &target).expect_err("concurrent export loser must not win");
        assert!(matches!(error, ExportError::TargetNotEmpty));
        assert!(staging.exists());
        let _ = remove_staging_directory(&staging);
        verify_published_export(&target, run_id).expect("verified");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_publish_maps_permission_denied_rename_to_target_invalid() {
        use std::io::{Error, ErrorKind};
        use std::os::unix::fs::PermissionsExt;

        let root = TempDir::new().unwrap();
        let readonly_parent = root.path().join("readonly-parent");
        fs::create_dir_all(&readonly_parent).expect("readonly parent");
        fs::set_permissions(&readonly_parent, fs::Permissions::from_mode(0o555))
            .expect("lock parent");
        let target = readonly_parent.join("export-out");
        let staging = create_staging_directory(root.path()).expect("staging");
        write_payload_file(&staging, "state.json", b"{}").expect("staging payload");

        let error = atomic_publish(&staging, &target).expect_err("rename denied");
        assert!(matches!(error, ExportError::TargetInvalid { .. }));
        assert!(export_read_rejected(&error));
        let (code, _) = export_read_failure(&error);
        assert_eq!(code, "export.target.invalid");
        assert!(staging.exists());

        fs::set_permissions(&readonly_parent, fs::Permissions::from_mode(0o755))
            .expect("unlock parent");
        let _ = remove_staging_directory(&staging);

        let mapped = map_target_io_error(Error::from(ErrorKind::ReadOnlyFilesystem));
        assert!(matches!(mapped, ExportError::TargetInvalid { .. }));
    }

    #[test]
    fn atomic_publish_rejects_target_with_different_manifest() {
        let root = TempDir::new().unwrap();
        let run_id = "run-collision";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _) =
            seed_complete_export_dir(root.path(), "export-collision", run_id, exported_at);
        fs::write(target.join("marker.txt"), b"foreign").expect("foreign file");
        let staging = create_staging_directory(root.path()).expect("staging");
        write_payload_file(&staging, "state.json", b"attempt").expect("staging payload");

        let error = atomic_publish(&staging, &target).expect_err("foreign target");
        assert!(matches!(error, ExportError::TargetNotEmpty));
        assert!(staging.exists());
        let _ = remove_staging_directory(&staging);
    }

    #[test]
    fn verify_published_export_rejects_corrupted_payload_bytes() {
        let root = TempDir::new().unwrap();
        let run_id = "run-corrupt-payload";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-corrupt-payload", run_id, exported_at);
        fs::write(target.join("state.json"), br#"{"tampered":true}"#).expect("tamper");

        let error = verify_published_export(&target, run_id).expect_err("corrupted payload");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_corrupted_manifest_schema() {
        let root = TempDir::new().unwrap();
        let run_id = "run-corrupt-manifest";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-corrupt-manifest", run_id, exported_at);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(target.join("manifest.json")).unwrap()).unwrap();
        manifest["export_manifest_schema_version"] = json!(2);
        let manifest_bytes = canonical_json_bytes(&manifest).unwrap();
        fs::write(target.join("manifest.json"), &manifest_bytes).expect("rewrite manifest");

        let error = verify_published_export(&target, run_id).expect_err("corrupted manifest");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_one_file_manifest() {
        let root = TempDir::new().unwrap();
        let run_id = "run-one-file-manifest";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let target = root.path().join("export-one-file");
        fs::create_dir_all(&target).unwrap();
        let state_bytes = br#"{"export_schema_version":1}"#;
        fs::write(target.join("state.json"), state_bytes).unwrap();
        let state_entry = ManifestFileEntry {
            path: "state.json".into(),
            sha256: sha256_label(state_bytes),
            bytes: state_bytes.len() as u64,
        };
        let manifest_bytes =
            encode_manifest(run_id, exported_at, &[state_entry]).expect("manifest");
        fs::write(target.join("manifest.json"), &manifest_bytes).unwrap();

        let error = verify_published_export(&target, run_id).expect_err("one-file manifest");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_path_traversal_and_extra_files() {
        let root = TempDir::new().unwrap();
        let run_id = "run-unsafe-path";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let target = root.path().join("export-unsafe");
        fs::create_dir_all(&target).unwrap();
        let state_bytes = br#"{"export_schema_version":1}"#;
        fs::write(target.join("state.json"), state_bytes).unwrap();
        let manifest = json!({
            "export_manifest_schema_version": 1,
            "export_schema_version": 1,
            "exported_at": exported_at,
            "files": [{
                "bytes": state_bytes.len(),
                "path": "../state.json",
                "sha256": sha256_label(state_bytes),
            }],
            "run_id": run_id,
        });
        let manifest_bytes = canonical_json_bytes(&manifest).unwrap();
        fs::write(target.join("manifest.json"), &manifest_bytes).unwrap();

        let traversal_error = verify_published_export(&target, run_id).expect_err("path traversal");
        assert!(matches!(
            traversal_error,
            ExportError::ResourceExhausted { .. }
        ));

        let (safe_target, _safe_expected) =
            seed_complete_export_dir(root.path(), "export-extra-file", run_id, exported_at);
        fs::write(safe_target.join("extra.txt"), b"extra").expect("extra file");
        let extra_error = verify_published_export(&safe_target, run_id).expect_err("extra file");
        assert!(matches!(extra_error, ExportError::ResourceExhausted { .. }));

        #[cfg(unix)]
        {
            fs::remove_file(safe_target.join("extra.txt")).expect("remove extra file");
            std::os::unix::fs::symlink("state.json", safe_target.join("extra-link"))
                .expect("symlink fixture");
            let special_error =
                verify_published_export(&safe_target, run_id).expect_err("extra special entry");
            assert!(matches!(
                special_error,
                ExportError::ResourceExhausted { .. }
            ));
        }
    }

    #[test]
    fn verify_published_export_rejects_reordered_manifest_inventory() {
        let root = TempDir::new().unwrap();
        let run_id = "run-reordered-manifest";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-reordered", run_id, exported_at);
        let state_bytes = fs::read(target.join("state.json")).unwrap();
        let journal_bytes = fs::read(target.join("journal.jsonl")).unwrap();
        let state_entry = ManifestFileEntry {
            path: "state.json".into(),
            sha256: sha256_label(&state_bytes),
            bytes: state_bytes.len() as u64,
        };
        let journal_entry = ManifestFileEntry {
            path: "journal.jsonl".into(),
            sha256: sha256_label(&journal_bytes),
            bytes: journal_bytes.len() as u64,
        };
        let manifest_bytes =
            encode_manifest(run_id, exported_at, &[state_entry, journal_entry]).expect("manifest");
        fs::write(target.join("manifest.json"), &manifest_bytes).expect("rewrite manifest");

        let error = verify_published_export(&target, run_id).expect_err("reordered inventory");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_noncanonical_manifest_bytes() {
        let root = TempDir::new().unwrap();
        let run_id = "run-noncanonical-manifest";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-noncanonical", run_id, exported_at);
        let manifest: Value =
            serde_json::from_slice(&fs::read(target.join("manifest.json")).unwrap()).unwrap();
        let pretty = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(target.join("manifest.json"), pretty).expect("pretty manifest");

        let error = verify_published_export(&target, run_id).expect_err("noncanonical manifest");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_manifest_with_extra_top_level_key() {
        let root = TempDir::new().unwrap();
        let run_id = "run-extra-manifest-key";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-extra-key", run_id, exported_at);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(target.join("manifest.json")).unwrap()).unwrap();
        manifest["extra"] = json!("unexpected");
        let manifest_bytes = canonical_json_bytes(&manifest).unwrap();
        fs::write(target.join("manifest.json"), &manifest_bytes).expect("rewrite manifest");

        let error = verify_published_export(&target, run_id).expect_err("extra manifest key");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_published_export_rejects_malformed_manifest_json() {
        let root = TempDir::new().unwrap();
        let run_id = "run-malformed-manifest";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-malformed", run_id, exported_at);
        fs::write(target.join("manifest.json"), b"{not json").expect("malformed manifest");

        let error = verify_published_export(&target, run_id).expect_err("malformed manifest");
        assert!(matches!(error, ExportError::ResourceExhausted { .. }));
    }

    #[test]
    fn verify_export_directory_accepts_valid_publication() {
        let root = TempDir::new().unwrap();
        let run_id = "run-verify-directory";
        let exported_at = "2026-07-21T10:00:00.000Z";
        let (target, _expected) =
            seed_complete_export_dir(root.path(), "export-verify-directory", run_id, exported_at);

        verify_export_directory(
            &ExportTarget::parse(target.to_str().unwrap()).unwrap(),
            &RunId::parse(run_id).unwrap(),
        )
        .expect("valid export directory");
    }

    #[test]
    fn export_target_parent_normalizes_single_component_relative_target() {
        let parent = export_target_parent(Path::new("export-out")).expect("parent");
        assert_eq!(parent, Path::new("."));
    }

    #[test]
    fn unique_directory_allocation_retries_real_name_collisions() {
        let root = TempDir::new().unwrap();
        let collision_names = ["collision-0", "collision-1", "collision-2"];
        for name in collision_names {
            fs::create_dir(root.path().join(name)).expect("collision fixture");
        }
        let mut names = collision_names
            .into_iter()
            .chain(std::iter::once("available"));

        let allocated = create_unique_directory(root.path(), || {
            names.next().expect("bounded candidate sequence").to_owned()
        })
        .expect("allocation")
        .expect("available candidate");

        assert_eq!(allocated, root.path().join("available"));
    }

    #[test]
    fn contract_examples_match_implementation_canonical_bytes() {
        let state_bytes = br#"{"evidence":[],"export_schema_version":1,"exported_at":"2026-07-17T15:00:00.000Z","graph":{"canonical_graph_version":1,"graph_revision":"sha256:501a3c627bb31a7e742d8e3f5466076beeadc778f034c4be6b7c9ddd2704fde6","initial_state_id":"a","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"a","static_guidance":{"kind":"none"}},{"final":true,"id":"b","static_guidance":{"kind":"none"}}],"transitions":[{"event_id":"finish","gate_ids":[],"source_state_id":"a","target_state_id":"b"}]},"inputs":{},"registration_binding":{"config_revision_at_create":1,"registration_id":"01J9X3K2M4N5P6Q7R8S9T0ABC"},"run":{"created_at":"2026-07-17T14:00:00.123Z","id":"01J9X3K2M4N5P6Q7R8S9T0V2X","label":"checkout-redesign","lifecycle":"active","lifecycle_version":1,"label_version":1,"state":"a","workflow_state_version":1},"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X"}"#;
        assert_eq!(state_bytes.len(), 873);
        assert_eq!(
            sha256_label(state_bytes),
            "sha256:d9c273186721835b47ef7696bdc9188eaaed20fbad92c84d3e544e41d00852b1"
        );

        let journal_value = json!({
            "journal_schema_version": 1,
            "sequence": 1,
            "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X",
            "ts": "2026-07-17T14:00:00.123Z",
            "operation": "run.create",
            "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W",
            "entry_kind": "run.created",
            "outcome": "completed",
            "reason": null,
            "state_before": {
                "state": "a",
                "lifecycle": "active",
                "workflow_state_version": 1,
                "lifecycle_version": 1
            },
            "state_after": {
                "state": "a",
                "lifecycle": "active",
                "workflow_state_version": 1,
                "lifecycle_version": 1
            },
            "graph_revision": "sha256:501a3c627bb31a7e742d8e3f5466076beeadc778f034c4be6b7c9ddd2704fde6"
        });
        let journal_line = canonical_json(&journal_value);
        let journal_bytes = format!("{journal_line}\n").into_bytes();
        assert_eq!(journal_bytes.len(), 528);
        assert_eq!(
            sha256_label(&journal_bytes),
            "sha256:079ba5d73eabc24926e1d22d195bd2528e3acd96153f5745bcc2658f384f2603"
        );

        let journal_entry = ManifestFileEntry {
            path: "journal.jsonl".into(),
            sha256: sha256_label(&journal_bytes),
            bytes: journal_bytes.len() as u64,
        };
        let state_entry = ManifestFileEntry {
            path: "state.json".into(),
            sha256: sha256_label(state_bytes),
            bytes: state_bytes.len() as u64,
        };
        let manifest_bytes = encode_manifest(
            "01J9X3K2M4N5P6Q7R8S9T0V2X",
            "2026-07-17T15:00:00.000Z",
            &[journal_entry, state_entry],
        )
        .expect("manifest");
        let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("manifest json");
        assert_eq!(
            canonical_json(&manifest),
            canonical_json(&json!({
                "export_manifest_schema_version": 1,
                "export_schema_version": 1,
                "exported_at": "2026-07-17T15:00:00.000Z",
                "files": [
                    {
                        "bytes": 528,
                        "path": "journal.jsonl",
                        "sha256": "sha256:079ba5d73eabc24926e1d22d195bd2528e3acd96153f5745bcc2658f384f2603"
                    },
                    {
                        "bytes": 873,
                        "path": "state.json",
                        "sha256": "sha256:d9c273186721835b47ef7696bdc9188eaaed20fbad92c84d3e544e41d00852b1"
                    }
                ],
                "run_id": "01J9X3K2M4N5P6Q7R8S9T0V2X"
            }))
        );
    }
}
