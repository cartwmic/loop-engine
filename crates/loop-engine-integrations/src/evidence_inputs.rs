//! Strict bounded loading for caller-supplied inline evidence documents.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use loop_engine_core::model::annotation::ActorMetadata;
use loop_engine_core::model::bounded::{
    ACTOR_METADATA_ENCODED_BYTES, EVIDENCE_RECORD_ENCODED_BYTES, FiniteNumber,
    INLINE_EVIDENCE_CONTEXT_TOTAL_BYTES, Metadata, Value as CoreValue,
};
use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::ids::{EvidenceId, EvidenceKind};
use loop_engine_core::model::time::ObservedAt;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::provider_protocol::validation::{ProtocolValidationError, parse_strict_value};

#[derive(Debug, Error)]
pub enum InlineEvidenceLoadError {
    #[error("failed to read inline evidence document {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Json(#[from] ProtocolValidationError),
    #[error("inline evidence document root must be an array")]
    RootNotArray,
    #[error("metadata document root must be an object")]
    RootNotObject,
    #[error("invalid inline evidence document: {0}")]
    Shape(String),
    #[error("invalid inline evidence field {path}: {message}")]
    Field { path: String, message: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineEvidenceDto {
    id: String,
    kind: String,
    locator: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    metadata: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    observed_at: Option<String>,
}

/// Loads one optional strict JSON array of caller evidence records.
///
/// Duplicate object keys, trailing JSON values, oversized documents, non-array roots,
/// unknown fields, duplicate evidence IDs, and bounded model violations fail closed.
pub fn load_optional(
    path: Option<&Path>,
    default_observed_at: ObservedAt,
) -> Result<Vec<EvidenceRecord>, InlineEvidenceLoadError> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let bytes = fs::read(path).map_err(|source| InlineEvidenceLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = parse_strict_value(&bytes, INLINE_EVIDENCE_CONTEXT_TOTAL_BYTES)?;
    if !value.is_array() {
        return Err(InlineEvidenceLoadError::RootNotArray);
    }
    let values: Vec<InlineEvidenceDto> = serde_json::from_value(value)
        .map_err(|error| InlineEvidenceLoadError::Shape(error.to_string()))?;
    let records = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| map_record(value, index, default_observed_at))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = records.iter().map(EvidenceRecord::id).collect::<Vec<_>>();
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(InlineEvidenceLoadError::Shape(
            "evidence IDs must be unique within the document".into(),
        ));
    }
    Ok(records)
}

/// Loads one optional strict JSON object as bounded evidence metadata.
pub fn load_metadata_optional(
    path: Option<&Path>,
) -> Result<Option<Metadata>, InlineEvidenceLoadError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = fs::read(path).map_err(|source| InlineEvidenceLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = parse_strict_value(&bytes, EVIDENCE_RECORD_ENCODED_BYTES)?;
    let Value::Object(values) = value else {
        return Err(InlineEvidenceLoadError::RootNotObject);
    };
    map_metadata(Some(values.into_iter().collect()), "/")
}

/// Loads one optional strict JSON object as bounded, authority-free actor metadata.
pub fn load_actor_optional(
    path: Option<&Path>,
) -> Result<Option<ActorMetadata>, InlineEvidenceLoadError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = fs::read(path).map_err(|source| InlineEvidenceLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = parse_strict_value(&bytes, ACTOR_METADATA_ENCODED_BYTES)?;
    if !value.is_object() {
        return Err(InlineEvidenceLoadError::RootNotObject);
    }
    ActorMetadata::new(map_value(value, "/")?)
        .map(Some)
        .map_err(|error| field("/", error))
}

fn map_record(
    value: InlineEvidenceDto,
    index: usize,
    default_observed_at: ObservedAt,
) -> Result<EvidenceRecord, InlineEvidenceLoadError> {
    let base = format!("/{index}");
    let observed_at = value
        .observed_at
        .map(|raw| {
            ObservedAt::parse(&raw).map_err(|error| field(format!("{base}/observed_at"), error))
        })
        .transpose()?
        .unwrap_or(default_observed_at);
    EvidenceRecord::new(
        EvidenceId::parse(value.id).map_err(|error| field(format!("{base}/id"), error))?,
        EvidenceKind::parse(value.kind).map_err(|error| field(format!("{base}/kind"), error))?,
        value.locator,
        value.digest,
        value.media_type,
        map_metadata(value.metadata, &format!("{base}/metadata"))?,
        EvidenceSource::Caller,
        observed_at,
    )
    .map_err(|error| field(base, error))
}

fn map_metadata(
    values: Option<BTreeMap<String, Value>>,
    path: &str,
) -> Result<Option<Metadata>, InlineEvidenceLoadError> {
    let Some(values) = values else {
        return Ok(None);
    };
    let values = values
        .into_iter()
        .map(|(key, value)| map_value(value, path).map(|value| (key, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Metadata::new("evidence_metadata", values, EVIDENCE_RECORD_ENCODED_BYTES)
        .map_err(|error| field(path, error))
}

fn map_value(value: Value, path: &str) -> Result<CoreValue, InlineEvidenceLoadError> {
    match value {
        Value::Null => Ok(CoreValue::Null),
        Value::Bool(value) => Ok(CoreValue::Bool(value)),
        Value::String(value) => Ok(CoreValue::String(value)),
        Value::Number(value) => {
            let number = value
                .as_f64()
                .ok_or_else(|| field(path, "number is outside binary64 domain"))?;
            FiniteNumber::new("evidence_number", number)
                .map(CoreValue::Number)
                .map_err(|error| field(path, error))
        }
        Value::Array(values) => values
            .into_iter()
            .map(|value| map_value(value, path))
            .collect::<Result<Vec<_>, _>>()
            .map(CoreValue::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| map_value(value, path).map(|value| (key, value)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(CoreValue::Object),
    }
}

fn field(path: impl Into<String>, error: impl std::fmt::Display) -> InlineEvidenceLoadError {
    InlineEvidenceLoadError::Field {
        path: path.into(),
        message: error.to_string(),
    }
}
