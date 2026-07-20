use std::collections::BTreeMap;

use loop_engine_core::model::bounded::{BoundError, FiniteNumber, Metadata, Value as CoreValue};
use loop_engine_core::model::compatibility::{
    CompatibilityFinding, CompatibilityReport, CompatibilityStatus,
};
use loop_engine_core::model::diagnostic::{Diagnostic, Diagnostics};
use loop_engine_core::model::ids::{EventId, GateId, InputKind, InputName, StateId};
use loop_engine_core::model::run_input::{InputDeclaration, InputDeclarations};
use serde_json::Value;
use thiserror::Error;

use super::dto::{
    CompatibilityFindingDto, CompatibilityResultDto, CompatibilityStatusDto, DiagnosticDto,
    InputDeclarationDto,
};
use super::validation::GRAPH_PROJECTION_CANONICAL_BYTES;

#[derive(Debug, Error)]
pub enum MappingError {
    #[error("invalid provider field {path}: {message}")]
    Field { path: String, message: String },
    #[error("invalid provider diagnostics: {0}")]
    Diagnostics(String),
    #[error("invalid provider compatibility result: {0}")]
    Compatibility(String),
}

impl MappingError {
    pub fn field(path: impl Into<String>, error: impl std::fmt::Display) -> Self {
        Self::Field {
            path: path.into(),
            message: error.to_string(),
        }
    }
}

pub fn diagnostics(values: Vec<DiagnosticDto>, path: &str) -> Result<Diagnostics, MappingError> {
    let mapped = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            Diagnostic::new(value.code, value.message, value.path)
                .map_err(|error| MappingError::field(format!("{path}/{index}"), error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Diagnostics::new(mapped).map_err(|error| MappingError::Diagnostics(error.to_string()))
}

pub fn input_declarations(
    values: Vec<InputDeclarationDto>,
    path: &str,
) -> Result<InputDeclarations, MappingError> {
    let values = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let item = format!("{path}/{index}");
            Ok(InputDeclaration::new(
                InputName::parse(value.id)
                    .map_err(|error| MappingError::field(format!("{item}/id"), error))?,
                InputKind::parse(value.kind)
                    .map_err(|error| MappingError::field(format!("{item}/kind"), error))?,
                value.required,
                metadata(value.metadata, &format!("{item}/metadata"))?,
            ))
        })
        .collect::<Result<Vec<_>, MappingError>>()?;
    Ok(InputDeclarations::new_unvalidated(values))
}

pub fn compatibility(value: CompatibilityResultDto) -> Result<CompatibilityReport, MappingError> {
    match value {
        CompatibilityResultDto::Findings { capabilities } => {
            let findings = capabilities
                .into_iter()
                .enumerate()
                .map(|(index, value)| compatibility_finding(value, index))
                .collect::<Result<Vec<_>, _>>()?;
            CompatibilityReport::findings(findings)
                .map_err(|error| MappingError::Compatibility(error.to_string()))
        }
        CompatibilityResultDto::EvaluationError {
            diagnostics: values,
        } => {
            let diagnostics = diagnostics(values, "/result/diagnostics")?;
            CompatibilityReport::evaluation_error(diagnostics.into_vec())
                .map_err(|error| MappingError::Compatibility(error.to_string()))
        }
    }
}

fn compatibility_finding(
    value: CompatibilityFindingDto,
    index: usize,
) -> Result<CompatibilityFinding, MappingError> {
    let status = match value.status {
        CompatibilityStatusDto::Compatible => CompatibilityStatus::Compatible,
        CompatibilityStatusDto::Incompatible => CompatibilityStatus::Incompatible,
        CompatibilityStatusDto::Unknown => CompatibilityStatus::Unknown,
    };
    let diagnostics = diagnostics(
        value.diagnostics,
        &format!("/result/capabilities/{index}/diagnostics"),
    )?;
    CompatibilityFinding::new(value.capability, status, diagnostics.into_vec())
        .map_err(|error| MappingError::field(format!("/result/capabilities/{index}"), error))
}

pub fn metadata(
    values: Option<BTreeMap<String, Value>>,
    path: &str,
) -> Result<Option<Metadata>, MappingError> {
    let Some(values) = values else {
        return Ok(None);
    };
    let values = values
        .into_iter()
        .map(|(key, value)| core_value(value, path).map(|value| (key, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Metadata::new(
        "provider_metadata",
        values,
        GRAPH_PROJECTION_CANONICAL_BYTES,
    )
    .map_err(|error| MappingError::field(path, error))
}

pub fn core_value(value: Value, path: &str) -> Result<CoreValue, MappingError> {
    match value {
        Value::Null => Ok(CoreValue::Null),
        Value::Bool(value) => Ok(CoreValue::Bool(value)),
        Value::Number(value) => {
            let number = value
                .as_f64()
                .ok_or_else(|| MappingError::field(path, "number is outside binary64 domain"))?;
            Ok(CoreValue::Number(
                FiniteNumber::new("provider_number", number)
                    .map_err(|error| MappingError::field(path, error))?,
            ))
        }
        Value::String(value) => Ok(CoreValue::String(value)),
        Value::Array(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| core_value(value, &format!("{path}/{index}")))
            .collect::<Result<Vec<_>, _>>()
            .map(CoreValue::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| {
                core_value(value, &format!("{path}/{key}")).map(|value| (key, value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(CoreValue::Object),
    }
}

pub fn parse_state_id(value: String, path: &str) -> Result<StateId, MappingError> {
    StateId::parse(value).map_err(|error| MappingError::field(path, error))
}

pub fn parse_event_id(value: String, path: &str) -> Result<EventId, MappingError> {
    EventId::parse(value).map_err(|error| MappingError::field(path, error))
}

pub fn parse_gate_id(value: String, path: &str) -> Result<GateId, MappingError> {
    GateId::parse(value).map_err(|error| MappingError::field(path, error))
}

pub fn bound_error(path: &str, error: BoundError) -> MappingError {
    MappingError::field(path, error)
}
