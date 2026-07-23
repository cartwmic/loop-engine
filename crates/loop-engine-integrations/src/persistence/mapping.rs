//! Corruption-validating mappings from persistence DTOs to core models (T106).

use std::collections::BTreeMap;

use loop_engine_core::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::ids::{
    EvidenceId, EvidenceKind, GraphRevision, ProviderHandle, RegistrationId, RunId,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::provider::ProviderRegistration;
use loop_engine_core::model::run::{Run, RunRestoreError};
use loop_engine_core::model::run_input::{InputDeclarations, RunInputs};
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::version::{LifecycleVersion, WorkflowStateVersion};
use serde_json::Value;
use thiserror::Error;

use crate::persistence::records::{
    EvidenceRecordRow, JOURNAL_PAYLOAD_SCHEMA_VERSION, JournalPayloadV1, JournalRecord,
    ProviderRegistrationRecord, RunRecord,
};
use crate::provider_protocol::canonical::graph_bytes;
use crate::provider_protocol::dto::{
    CanonicalGraphDto, CanonicalGuidanceDto, GraphDto, InputDeclarationDto, StateDto,
    StaticGuidanceDeclarationDto, StaticGuidanceDto, TransitionDto,
};
use crate::provider_protocol::graph::{GraphMappingError, map_graph};
use crate::provider_protocol::mapping::{
    self as protocol_mapping, MappingError as ProtocolMappingError,
};
use crate::sha256_digest::sha256_label;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("malformed JSON in {field}: {message}")]
    MalformedJson {
        field: &'static str,
        message: String,
    },
    #[error("unsupported version in {field}: {value}")]
    UnsupportedVersion { field: &'static str, value: u64 },
    #[error("unsupported enum in {field}: {value}")]
    UnsupportedEnum { field: &'static str, value: String },
    #[error("bounded semantic value rejected at {field}: {message}")]
    BoundedSemanticValue {
        field: &'static str,
        message: String,
    },
    #[error("graph semantics invalid: {message}")]
    GraphSemantics { message: String },
    #[error("graph digest mismatch: stored {stored}, computed {computed}")]
    GraphDigestMismatch { stored: String, computed: String },
    #[error("invalid lifecycle or state: {message}")]
    InvalidLifecycleState { message: String },
    #[error("invalid version in {field}: {message}")]
    InvalidVersion {
        field: &'static str,
        message: String,
    },
}

pub(crate) fn format_observed_at(at: &ObservedAt) -> String {
    at.as_timestamp()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

pub fn registration_from_record(
    record: &ProviderRegistrationRecord,
) -> Result<ProviderRegistration, MappingError> {
    parse_observed_at("created_at", &record.created_at)?;
    parse_observed_at("updated_at", &record.updated_at)?;
    validate_positive_version("config_revision", record.config_revision)?;
    let id = parse_registration_id(&record.registration_id)?;
    let handle = record
        .handle
        .as_ref()
        .map(|value| parse_provider_handle(value))
        .transpose()?;
    ProviderRegistration::restore(id, handle, record.config_revision, record.enabled).ok_or_else(
        || MappingError::InvalidLifecycleState {
            message: "enabled/handle invariant violated".into(),
        },
    )
}

pub fn config_from_record(
    record: &ProviderRegistrationRecord,
) -> Result<ProviderConfig, MappingError> {
    let argv: Vec<String> =
        serde_json::from_str(&record.argv_json).map_err(|error| MappingError::MalformedJson {
            field: "argv_json",
            message: error.to_string(),
        })?;
    ProviderConfig::new(
        record.executable.clone(),
        argv,
        record.working_directory.clone(),
        record.timeout_seconds,
    )
    .map_err(|error| MappingError::BoundedSemanticValue {
        field: "provider_config",
        message: error.to_string(),
    })
}

pub fn resolved_config_from_record(
    record: &ProviderRegistrationRecord,
) -> Result<ResolvedProviderConfig, MappingError> {
    if !record.enabled {
        return Err(MappingError::InvalidLifecycleState {
            message: "registration is disabled".into(),
        });
    }
    let registration_id = parse_registration_id(&record.registration_id)?;
    let handle = parse_provider_handle(record.handle.as_deref().ok_or_else(|| {
        MappingError::InvalidLifecycleState {
            message: "enabled registration requires handle".into(),
        }
    })?)?;
    let config = config_from_record(record)?;
    ResolvedProviderConfig::new(registration_id, handle, record.config_revision, config).map_err(
        |error| MappingError::BoundedSemanticValue {
            field: "resolved_provider_config",
            message: error.to_string(),
        },
    )
}

pub fn provider_registration_record(
    registration: &ProviderRegistration,
    config: &ProviderConfig,
    created_at: &str,
    updated_at: &str,
) -> Result<ProviderRegistrationRecord, MappingError> {
    let argv: Vec<&str> = config.argv().iter().map(|value| value.as_str()).collect();
    let argv_json = serde_json::to_string(&argv).map_err(|error| MappingError::MalformedJson {
        field: "argv_json",
        message: error.to_string(),
    })?;
    Ok(ProviderRegistrationRecord {
        registration_id: registration.id().as_str().to_owned(),
        handle: registration
            .handle()
            .map(|handle| handle.as_str().to_owned()),
        enabled: registration.enabled(),
        config_revision: registration.config_revision(),
        executable: config.executable().to_owned(),
        argv_json,
        working_directory: config.working_directory().to_owned(),
        timeout_seconds: config.timeout_seconds(),
        created_at: created_at.to_owned(),
        updated_at: updated_at.to_owned(),
    })
}

pub fn run_from_record(record: &RunRecord) -> Result<Run, MappingError> {
    parse_observed_at("created_at", &record.created_at)?;
    validate_positive_version(
        "config_revision_at_create",
        record.config_revision_at_create,
    )?;
    validate_positive_version("workflow_state_version", record.workflow_state_version)?;
    validate_positive_version("lifecycle_version", record.lifecycle_version)?;
    validate_positive_version("label_version", record.label_version)?;

    if record.canonical_graph_version != 1 {
        return Err(MappingError::UnsupportedVersion {
            field: "canonical_graph_version",
            value: record.canonical_graph_version,
        });
    }

    let lifecycle = parse_lifecycle(&record.lifecycle)?;
    let validated = verify_stored_graph_snapshot(record)?;
    let inputs = parse_stored_inputs(&record.inputs_json, validated.graph().inputs())?;
    let workflow_state_version = WorkflowStateVersion::try_from(record.workflow_state_version)
        .map_err(|_| MappingError::InvalidVersion {
            field: "workflow_state_version",
            message: "must be positive".into(),
        })?;
    let lifecycle_version = LifecycleVersion::try_from(record.lifecycle_version).map_err(|_| {
        MappingError::InvalidVersion {
            field: "lifecycle_version",
            message: "must be positive".into(),
        }
    })?;

    Run::restore(
        parse_run_id(&record.run_id)?,
        parse_registration_id(&record.registration_id)?,
        validated,
        GraphRevision::parse(record.graph_revision.clone()).map_err(map_identifier_error)?,
        inputs,
        protocol_mapping::parse_state_id(record.current_state.clone(), "/current_state")
            .map_err(map_protocol_error)?,
        lifecycle,
        workflow_state_version,
        lifecycle_version,
        record.label.clone(),
    )
    .map_err(map_run_restore_error)
}

pub fn evidence_from_record(record: &EvidenceRecordRow) -> Result<EvidenceRecord, MappingError> {
    let metadata = match &record.metadata_json {
        None => None,
        Some(json) => {
            let raw: BTreeMap<String, Value> =
                serde_json::from_str(json).map_err(|error| MappingError::MalformedJson {
                    field: "metadata_json",
                    message: error.to_string(),
                })?;
            if raw.is_empty() {
                None
            } else {
                protocol_mapping::metadata(Some(raw), "/metadata").map_err(map_protocol_error)?
            }
        }
    };
    EvidenceRecord::new(
        EvidenceId::parse(record.evidence_id.clone()).map_err(map_identifier_error)?,
        EvidenceKind::parse(record.kind.clone()).map_err(map_identifier_error)?,
        record.locator.clone(),
        record.digest.clone(),
        record.media_type.clone(),
        metadata,
        parse_evidence_source(&record.source)?,
        parse_observed_at("created_at", &record.created_at)?,
    )
    .map_err(|error| MappingError::BoundedSemanticValue {
        field: "evidence_record",
        message: error.to_string(),
    })
}

pub fn evidence_record_row(
    run_id: &RunId,
    evidence: &EvidenceRecord,
) -> Result<EvidenceRecordRow, MappingError> {
    let metadata_json = evidence
        .metadata()
        .map(|metadata| {
            let value = Value::Object(
                crate::provider_protocol::canonical::metadata_value(metadata)
                    .into_iter()
                    .collect(),
            );
            serde_json::to_string(&value).map_err(|error| MappingError::MalformedJson {
                field: "metadata_json",
                message: error.to_string(),
            })
        })
        .transpose()?;
    Ok(EvidenceRecordRow {
        run_id: run_id.as_str().to_owned(),
        evidence_id: evidence.id().as_str().to_owned(),
        kind: evidence.kind().as_str().to_owned(),
        locator: evidence.locator().to_owned(),
        digest: evidence.digest().map(str::to_owned),
        media_type: evidence.media_type().map(str::to_owned),
        metadata_json,
        source: evidence_source_label(evidence.source()).to_owned(),
        created_at: format_observed_at(&evidence.observed_at()),
    })
}

pub fn validate_journal_record(record: &JournalRecord) -> Result<JournalPayloadV1, MappingError> {
    validate_positive_version("sequence", record.sequence)?;
    parse_outcome_class(&record.outcome)?;
    let payload: JournalPayloadV1 =
        serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
            MappingError::MalformedJson {
                field: "encoded_payload_json",
                message: error.to_string(),
            }
        })?;
    if payload.journal_schema_version != JOURNAL_PAYLOAD_SCHEMA_VERSION {
        return Err(MappingError::UnsupportedVersion {
            field: "journal_schema_version",
            value: payload.journal_schema_version as u64,
        });
    }
    if payload.sequence != record.sequence {
        return Err(MappingError::InvalidLifecycleState {
            message: "payload sequence does not match row sequence".into(),
        });
    }
    if payload.run_id != record.run_id {
        return Err(MappingError::InvalidLifecycleState {
            message: "payload run_id does not match row run_id".into(),
        });
    }
    if payload.outcome != record.outcome {
        return Err(MappingError::InvalidLifecycleState {
            message: "payload outcome does not match row outcome".into(),
        });
    }
    Ok(payload)
}

fn verify_stored_graph_snapshot(
    record: &RunRecord,
) -> Result<loop_engine_core::model::graph_validation::ValidatedGraph, MappingError> {
    let stored_bytes = record.graph_canonical_projection_json.as_bytes();
    let canonical: CanonicalGraphDto =
        serde_json::from_str(&record.graph_canonical_projection_json).map_err(|error| {
            MappingError::MalformedJson {
                field: "graph_canonical_projection_json",
                message: error.to_string(),
            }
        })?;
    let graph_dto = canonical_to_graph_dto(canonical)?;
    let validated = map_graph(graph_dto).map_err(map_graph_error)?;
    let projection = SemanticGraphProjection::from_validated(&validated);
    let canonical_bytes =
        graph_bytes(&projection).map_err(|error| MappingError::GraphSemantics {
            message: error.to_string(),
        })?;
    if canonical_bytes.as_slice() != stored_bytes {
        return Err(MappingError::GraphSemantics {
            message: "stored graph snapshot is not in strict canonical form".into(),
        });
    }
    let computed_digest = sha256_label(stored_bytes);
    if computed_digest != record.graph_revision {
        return Err(MappingError::GraphDigestMismatch {
            stored: record.graph_revision.clone(),
            computed: computed_digest,
        });
    }
    Ok(validated)
}

fn canonical_to_graph_dto(dto: CanonicalGraphDto) -> Result<GraphDto, MappingError> {
    if dto.canonical_graph_version != 1 {
        return Err(MappingError::UnsupportedVersion {
            field: "canonical_graph_version",
            value: dto.canonical_graph_version,
        });
    }
    Ok(GraphDto {
        initial_state: dto.initial_state_id,
        states: dto
            .states
            .into_iter()
            .map(|state| StateDto {
                id: state.id,
                final_state: state.final_state,
                static_guidance: match state.static_guidance {
                    CanonicalGuidanceDto::Text { text } => StaticGuidanceDto::Text(text),
                    CanonicalGuidanceDto::None => {
                        StaticGuidanceDto::Declaration(StaticGuidanceDeclarationDto::None)
                    }
                },
                metadata: state.metadata,
            })
            .collect(),
        transitions: dto
            .transitions
            .into_iter()
            .map(|transition| TransitionDto {
                source_state: transition.source_state_id,
                event: transition.event_id,
                target_state: transition.target_state_id,
                gate_ids: transition.gate_ids,
                metadata: transition.metadata,
            })
            .collect(),
        input_declarations: dto
            .input_declarations
            .into_iter()
            .map(|input| InputDeclarationDto {
                id: input.id,
                kind: input.kind,
                required: input.required,
                metadata: input.metadata,
            })
            .collect(),
        live_guidance_supported: dto.live_guidance_supported,
        metadata: dto.metadata,
    })
}

fn parse_stored_inputs(
    inputs_json: &str,
    _declarations: &InputDeclarations,
) -> Result<RunInputs, MappingError> {
    let raw: BTreeMap<String, Value> =
        serde_json::from_str(inputs_json).map_err(|error| MappingError::MalformedJson {
            field: "inputs_json",
            message: error.to_string(),
        })?;
    let candidate = raw
        .into_iter()
        .map(|(key, value)| {
            let name = loop_engine_core::model::ids::InputName::parse(key)
                .map_err(map_identifier_error)?;
            let core =
                protocol_mapping::core_value(value, "/inputs").map_err(map_protocol_error)?;
            Ok((name, core))
        })
        .collect::<Result<Vec<_>, MappingError>>()?;
    RunInputs::from_wire_candidate(candidate).map_err(|error| MappingError::BoundedSemanticValue {
        field: "inputs_json",
        message: error.to_string(),
    })
}

fn parse_lifecycle(value: &str) -> Result<Lifecycle, MappingError> {
    match value {
        "active" => Ok(Lifecycle::Active),
        "final" => Ok(Lifecycle::Final),
        "terminated" => Ok(Lifecycle::Terminated),
        other => Err(MappingError::UnsupportedEnum {
            field: "lifecycle",
            value: other.to_owned(),
        }),
    }
}

fn parse_evidence_source(value: &str) -> Result<EvidenceSource, MappingError> {
    match value {
        "caller" => Ok(EvidenceSource::Caller),
        "provider" => Ok(EvidenceSource::Provider),
        other => Err(MappingError::UnsupportedEnum {
            field: "source",
            value: other.to_owned(),
        }),
    }
}

fn evidence_source_label(source: EvidenceSource) -> &'static str {
    match source {
        EvidenceSource::Caller => "caller",
        EvidenceSource::Provider => "provider",
    }
}

fn parse_outcome_class(value: &str) -> Result<(), MappingError> {
    match value {
        "completed" | "rejected" | "error" => Ok(()),
        other => Err(MappingError::UnsupportedEnum {
            field: "outcome",
            value: other.to_owned(),
        }),
    }
}

fn parse_observed_at(field: &'static str, value: &str) -> Result<ObservedAt, MappingError> {
    ObservedAt::parse(value).map_err(|error| MappingError::BoundedSemanticValue {
        field,
        message: error.to_string(),
    })
}

fn validate_positive_version(field: &'static str, value: u64) -> Result<(), MappingError> {
    if value == 0 {
        Err(MappingError::InvalidVersion {
            field,
            message: "must be positive".into(),
        })
    } else {
        Ok(())
    }
}

fn parse_registration_id(value: &str) -> Result<RegistrationId, MappingError> {
    RegistrationId::parse(value).map_err(map_identifier_error)
}

fn parse_provider_handle(value: &str) -> Result<ProviderHandle, MappingError> {
    ProviderHandle::parse(value).map_err(map_identifier_error)
}

fn parse_run_id(value: &str) -> Result<RunId, MappingError> {
    RunId::parse(value).map_err(map_identifier_error)
}

fn map_identifier_error(error: loop_engine_core::model::ids::IdentifierError) -> MappingError {
    MappingError::BoundedSemanticValue {
        field: "identifier",
        message: error.to_string(),
    }
}

fn map_protocol_error(error: ProtocolMappingError) -> MappingError {
    match error {
        ProtocolMappingError::Field { path, message } => MappingError::BoundedSemanticValue {
            field: "provider_field",
            message: format!("{path}: {message}"),
        },
        ProtocolMappingError::Diagnostics(message)
        | ProtocolMappingError::Compatibility(message) => MappingError::BoundedSemanticValue {
            field: "provider_mapping",
            message,
        },
    }
}

fn map_graph_error(error: GraphMappingError) -> MappingError {
    match error {
        GraphMappingError::Mapping(error) => map_protocol_error(error),
        GraphMappingError::Semantic(message) => MappingError::GraphSemantics { message },
    }
}

fn map_run_restore_error(error: RunRestoreError) -> MappingError {
    match error {
        RunRestoreError::UnknownCurrentState(state) => MappingError::InvalidLifecycleState {
            message: format!("stored current state absent from graph: {state}"),
        },
        RunRestoreError::InvalidLifecycle => MappingError::InvalidLifecycleState {
            message: "stored lifecycle inconsistent with stored current state".into(),
        },
        RunRestoreError::Bound(error) => MappingError::BoundedSemanticValue {
            field: "run_label",
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::model::bounded::Metadata;
    use loop_engine_core::model::bounded::Value as CoreValue;
    use loop_engine_core::model::graph::{State, WorkflowGraph};
    use loop_engine_core::model::graph_validation::ValidatedGraph;
    use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use loop_engine_core::model::ids::{
        EvidenceId, EvidenceKind, GraphRevision, InputKind, InputName, ProviderHandle,
        RegistrationId, RunId, StateId,
    };
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::model::provider::ProviderRegistration;
    use loop_engine_core::model::run_input::{InputDeclaration, InputDeclarations};
    use loop_engine_core::model::time::ObservedAt;

    use super::*;

    use crate::persistence::records::{GV01_CANONICAL_GRAPH_JSON, GV01_GRAPH_REVISION};

    fn sample_config() -> ProviderConfig {
        ProviderConfig::new("/bin/provider", vec!["--flag".into()], "/work", 60).unwrap()
    }

    fn sample_registration() -> ProviderRegistration {
        ProviderRegistration::new(
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
            ProviderHandle::parse("provider").unwrap(),
        )
    }

    fn sample_run_record() -> RunRecord {
        RunRecord {
            run_id: "019f0000-0000-7000-8000-000000000101".into(),
            registration_id: "019f0000-0000-7000-8000-000000000001".into(),
            config_revision_at_create: 1,
            current_state: "draft".into(),
            lifecycle: "active".into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: Some("active-run".into()),
            graph_revision: GV01_GRAPH_REVISION.into(),
            canonical_graph_version: 1,
            graph_canonical_projection_json: GV01_CANONICAL_GRAPH_JSON.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:01.000Z".into(),
        }
    }

    fn validated_graph() -> ValidatedGraph {
        let state = State::new(
            StateId::parse("draft").unwrap(),
            false,
            StaticGuidance::Text(
                loop_engine_core::model::bounded::BoundedText::non_empty(
                    "static_guidance",
                    "Prepare the change.",
                )
                .unwrap(),
            ),
            None,
        );
        ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
            StateId::parse("draft").unwrap(),
            vec![state],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        ))
        .unwrap()
    }

    #[test]
    fn stored_inputs_restore_provider_accepted_values_without_revalidating_declarations() {
        let declarations = InputDeclarations::new(vec![InputDeclaration::new(
            InputName::parse("ticket").unwrap(),
            InputKind::parse("text").unwrap(),
            true,
            None,
        )])
        .unwrap();
        let accepted_missing_required = parse_stored_inputs("{}", &declarations)
            .expect("provider-accepted missing required value remains readable");
        assert!(accepted_missing_required.values().is_empty());

        let accepted_undeclared = parse_stored_inputs(
            r#"{"provider_accepted":"value"}"#,
            &InputDeclarations::default(),
        )
        .expect("stored provider-accepted undeclared value remains readable");
        assert_eq!(accepted_undeclared.values().len(), 1);
    }

    #[test]
    fn provider_registration_round_trip() {
        let registration = sample_registration();
        let config = sample_config();
        let record = provider_registration_record(
            &registration,
            &config,
            "2026-07-17T12:00:00.000Z",
            "2026-07-17T12:00:00.000Z",
        )
        .unwrap();
        let restored = registration_from_record(&record).unwrap();
        let resolved = resolved_config_from_record(&record).unwrap();
        assert_eq!(restored, registration);
        assert_eq!(resolved.registration_id(), registration.id());
        assert_eq!(resolved.config_revision(), registration.config_revision());
        assert_eq!(resolved.config().executable(), config.executable());
    }

    #[test]
    fn provider_registration_rejects_malformed_argv_json() {
        let mut record = provider_registration_record(
            &sample_registration(),
            &sample_config(),
            "2026-07-17T12:00:00.000Z",
            "2026-07-17T12:00:00.000Z",
        )
        .unwrap();
        record.argv_json = "{".into();
        assert!(matches!(
            config_from_record(&record),
            Err(MappingError::MalformedJson {
                field: "argv_json",
                ..
            })
        ));
    }

    #[test]
    fn provider_registration_rejects_disabled_resolved_lookup() {
        let mut record = provider_registration_record(
            &sample_registration(),
            &sample_config(),
            "2026-07-17T12:00:00.000Z",
            "2026-07-17T12:00:00.000Z",
        )
        .unwrap();
        record.enabled = false;
        record.handle = None;
        assert!(registration_from_record(&record).is_ok());
        assert!(matches!(
            resolved_config_from_record(&record),
            Err(MappingError::InvalidLifecycleState { .. })
        ));
    }

    #[test]
    fn provider_registration_rejects_invalid_created_at_timestamp() {
        let mut record = provider_registration_record(
            &sample_registration(),
            &sample_config(),
            "2026-07-17T12:00:00.000Z",
            "2026-07-17T12:00:00.000Z",
        )
        .unwrap();
        record.created_at = "not-a-timestamp".into();
        assert!(matches!(
            registration_from_record(&record),
            Err(MappingError::BoundedSemanticValue {
                field: "created_at",
                ..
            })
        ));
    }

    #[test]
    fn provider_registration_rejects_invalid_updated_at_timestamp() {
        let mut record = provider_registration_record(
            &sample_registration(),
            &sample_config(),
            "2026-07-17T12:00:00.000Z",
            "2026-07-17T12:00:00.000Z",
        )
        .unwrap();
        record.updated_at = "still-not-a-timestamp".into();
        assert!(matches!(
            registration_from_record(&record),
            Err(MappingError::BoundedSemanticValue {
                field: "updated_at",
                ..
            })
        ));
    }

    #[test]
    fn run_record_round_trip_via_restore() {
        let run = run_from_record(&sample_run_record()).unwrap();
        assert_eq!(run.id().as_str(), "019f0000-0000-7000-8000-000000000101");
        assert_eq!(run.lifecycle(), Lifecycle::Active);
        assert_eq!(run.label(), Some("active-run"));
        assert_eq!(run.graph_revision().as_str(), GV01_GRAPH_REVISION);
    }

    #[test]
    fn run_record_rejects_unsupported_canonical_graph_version() {
        let mut record = sample_run_record();
        record.canonical_graph_version = 2;
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::UnsupportedVersion {
                field: "canonical_graph_version",
                value: 2
            })
        ));
    }

    #[test]
    fn run_record_rejects_malformed_graph_json() {
        let mut record = sample_run_record();
        record.graph_canonical_projection_json = "{".into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::MalformedJson {
                field: "graph_canonical_projection_json",
                ..
            })
        ));
    }

    #[test]
    fn run_record_rejects_graph_digest_mismatch() {
        let mut record = sample_run_record();
        record.graph_revision =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::GraphDigestMismatch { .. })
        ));
    }

    #[test]
    fn run_record_rejects_unknown_lifecycle() {
        let mut record = sample_run_record();
        record.lifecycle = "paused".into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::UnsupportedEnum {
                field: "lifecycle",
                ..
            })
        ));
    }

    #[test]
    fn run_record_rejects_invalid_lifecycle_state_pair() {
        let mut record = sample_run_record();
        record.lifecycle = "final".into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::InvalidLifecycleState { .. })
        ));
    }

    #[test]
    fn run_record_rejects_non_positive_versions() {
        let mut record = sample_run_record();
        record.workflow_state_version = 0;
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::InvalidVersion {
                field: "workflow_state_version",
                ..
            })
        ));
    }

    #[test]
    fn run_record_rejects_invalid_created_at_timestamp() {
        let mut record = sample_run_record();
        record.created_at = "yesterday".into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::BoundedSemanticValue {
                field: "created_at",
                ..
            })
        ));
    }

    #[test]
    fn run_record_rejects_graph_semantics() {
        let mut record = sample_run_record();
        record.graph_canonical_projection_json = r#"{"canonical_graph_version":1,"initial_state_id":"missing","input_declarations":[],"live_guidance_supported":false,"states":[],"transitions":[]}"#.into();
        assert!(matches!(
            run_from_record(&record),
            Err(MappingError::GraphSemantics { .. })
        ));
    }

    #[test]
    fn evidence_record_round_trip() {
        let run_id = RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap();
        let metadata = Metadata::new(
            "evidence_metadata",
            BTreeMap::from([("key".into(), CoreValue::String("value".into()))]),
            4096,
        )
        .unwrap();
        let evidence = EvidenceRecord::new(
            EvidenceId::parse("evidence-1").unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            "opaque://locator",
            Some(format!("sha256:{}", "a".repeat(64))),
            Some("application/json".into()),
            metadata,
            EvidenceSource::Provider,
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
        )
        .unwrap();
        let row = evidence_record_row(&run_id, &evidence).unwrap();
        let restored = evidence_from_record(&row).unwrap();
        assert_eq!(restored.id(), evidence.id());
        assert_eq!(restored.locator(), evidence.locator());
        assert_eq!(restored.source(), evidence.source());
        assert!(restored.metadata().is_some());
        assert_eq!(row.created_at, "2026-07-18T00:00:00.000Z");
    }

    #[test]
    fn format_observed_at_emits_millisecond_precision_utc() {
        let zero = ObservedAt::parse("2026-07-18T00:00:00Z").unwrap();
        assert_eq!(format_observed_at(&zero), "2026-07-18T00:00:00.000Z");

        let micro = ObservedAt::parse("2026-07-18T00:00:00.123456Z").unwrap();
        assert_eq!(format_observed_at(&micro), "2026-07-18T00:00:00.123Z");

        let nano = ObservedAt::parse("2026-07-18T00:00:00.123456789Z").unwrap();
        assert_eq!(format_observed_at(&nano), "2026-07-18T00:00:00.123Z");
    }

    #[test]
    fn format_observed_at_truncates_submillisecond_to_millisecond_instant() {
        let submillisecond = ObservedAt::parse("2026-07-18T00:00:00.123456789Z").unwrap();
        let millisecond = ObservedAt::parse("2026-07-18T00:00:00.123Z").unwrap();
        assert_eq!(
            format_observed_at(&submillisecond),
            format_observed_at(&millisecond)
        );

        let formatted = format_observed_at(&submillisecond);
        assert!(formatted.ends_with('Z'));
        assert_eq!(
            formatted
                .split('.')
                .nth(1)
                .unwrap()
                .trim_end_matches('Z')
                .len(),
            3
        );
        assert_eq!(ObservedAt::parse(&formatted).unwrap(), millisecond,);
    }

    #[test]
    fn evidence_record_rejects_malformed_metadata_json() {
        let row = EvidenceRecordRow {
            run_id: "run".into(),
            evidence_id: "e1".into(),
            kind: "artifact".into(),
            locator: "loc".into(),
            digest: None,
            media_type: None,
            metadata_json: Some("{".into()),
            source: "caller".into(),
            created_at: "2026-07-18T00:00:00Z".into(),
        };
        assert!(matches!(
            evidence_from_record(&row),
            Err(MappingError::MalformedJson {
                field: "metadata_json",
                ..
            })
        ));
    }

    #[test]
    fn evidence_record_rejects_unsupported_source() {
        let row = EvidenceRecordRow {
            run_id: "run".into(),
            evidence_id: "e1".into(),
            kind: "artifact".into(),
            locator: "loc".into(),
            digest: None,
            media_type: None,
            metadata_json: None,
            source: "unknown".into(),
            created_at: "2026-07-18T00:00:00Z".into(),
        };
        assert!(matches!(
            evidence_from_record(&row),
            Err(MappingError::UnsupportedEnum {
                field: "source",
                ..
            })
        ));
    }

    fn sample_journal_record(payload: &str) -> JournalRecord {
        JournalRecord {
            run_id: "019f0000-0000-7000-8000-000000000101".into(),
            sequence: 1,
            outcome: "completed".into(),
            encoded_payload_json: payload.into(),
        }
    }

    #[test]
    fn journal_record_validates_consistent_payload() {
        let payload = r#"{"journal_schema_version":1,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"completed","operation":"run.create","entry_kind":"run.created"}"#;
        let validated = validate_journal_record(&sample_journal_record(payload)).unwrap();
        assert_eq!(validated.journal_schema_version, 1);
        assert_eq!(validated.sequence, 1);
    }

    #[test]
    fn journal_record_rejects_malformed_payload_json() {
        assert!(matches!(
            validate_journal_record(&sample_journal_record("{")),
            Err(MappingError::MalformedJson {
                field: "encoded_payload_json",
                ..
            })
        ));
    }

    #[test]
    fn journal_record_rejects_unsupported_schema_version() {
        let payload = r#"{"journal_schema_version":2,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"completed"}"#;
        assert!(matches!(
            validate_journal_record(&sample_journal_record(payload)),
            Err(MappingError::UnsupportedVersion {
                field: "journal_schema_version",
                value: 2
            })
        ));
    }

    #[test]
    fn journal_record_rejects_sequence_mismatch() {
        let payload = r#"{"journal_schema_version":1,"sequence":2,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"completed"}"#;
        assert!(matches!(
            validate_journal_record(&sample_journal_record(payload)),
            Err(MappingError::InvalidLifecycleState { .. })
        ));
    }

    #[test]
    fn journal_record_rejects_outcome_mismatch() {
        let payload = r#"{"journal_schema_version":1,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"rejected"}"#;
        assert!(matches!(
            validate_journal_record(&sample_journal_record(payload)),
            Err(MappingError::InvalidLifecycleState { .. })
        ));
    }

    #[test]
    fn journal_record_rejects_unsupported_row_outcome() {
        let mut record = sample_journal_record(
            r#"{"journal_schema_version":1,"sequence":1,"run_id":"019f0000-0000-7000-8000-000000000101","outcome":"completed"}"#,
        );
        record.outcome = "pending".into();
        assert!(matches!(
            validate_journal_record(&record),
            Err(MappingError::UnsupportedEnum {
                field: "outcome",
                ..
            })
        ));
    }

    #[test]
    fn run_create_produces_matching_graph_revision() {
        let revision = GraphRevision::parse(GV01_GRAPH_REVISION).unwrap();
        let run = loop_engine_core::model::run::Run::create(
            RunId::parse("019f0000-0000-7000-8000-000000000101").unwrap(),
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap(),
            validated_graph(),
            revision,
            RunInputs::default(),
            Some("active-run".into()),
        )
        .unwrap();
        let record = sample_run_record();
        let restored = run_from_record(&record).unwrap();
        assert_eq!(restored.current_state(), run.current_state());
        assert_eq!(restored.lifecycle(), run.lifecycle());
    }
}
