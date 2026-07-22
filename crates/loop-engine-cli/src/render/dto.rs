use loop_engine_core::model::bounded::{
    BoundError, FILESYSTEM_PATH_UTF8_BYTES, IDENTIFIER_UTF8_BYTES,
};
use loop_engine_core::model::diagnostic::Diagnostic;
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::outcome::{OutcomeClass, PublicOutcome};
use loop_engine_core::operations::catalog::OperationId;
use serde_json::{Map, Value, json};
use thiserror::Error;

/// Frozen structured CLI outcome envelope schema version ([cli-contract.md]).
///
/// [cli-contract.md]: ../../../../docs/cli-contract.md
pub const SCHEMA_VERSION: u64 = 1;

/// Maximum encoded UTF-8 bytes for one structured CLI outcome envelope on stdout.
pub const STRUCTURED_CLI_ENVELOPE_BYTES: usize = 4_194_304;

const CORE_DATA_KEYS: &[&str] = &["run", "requestable_events", "evidence_recorded"];

/// Inputs required to render one post-dispatch structured outcome envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeRenderRequest<'a> {
    pub operation: OperationId,
    pub request_id: &'a str,
    pub trace_path: &'a str,
    pub outcome: &'a PublicOutcome,
    /// Operation-specific `data` extensions (`items`, `registration`, `graph`, …).
    /// Core-derived run summary fields are merged first and cannot be overridden.
    pub operation_data: Value,
}

impl<'a> OutcomeRenderRequest<'a> {
    pub fn new(
        operation: OperationId,
        request_id: &'a str,
        trace_path: &'a str,
        outcome: &'a PublicOutcome,
    ) -> Self {
        Self {
            operation,
            request_id,
            trace_path,
            outcome,
            operation_data: Value::Object(Default::default()),
        }
    }

    pub fn with_operation_data(mut self, operation_data: Value) -> Self {
        self.operation_data = operation_data;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OutcomeRenderError {
    #[error("request_id must not be empty")]
    EmptyRequestId,
    #[error("trace path must not be empty")]
    EmptyTracePath,
    #[error("operation_data must be a JSON object when present")]
    InvalidOperationData,
    #[error("operation_data cannot override core outcome data field `{field}`")]
    OperationDataOverridesCore { field: &'static str },
    #[error("structured CLI envelope exceeds {max} UTF-8 bytes (actual {actual})")]
    EnvelopeTooLarge { max: usize, actual: usize },
    #[error(transparent)]
    Bound(#[from] BoundError),
}

pub(crate) fn validate_invocation_fields(
    request_id: &str,
    trace_path: &str,
) -> Result<(), OutcomeRenderError> {
    if request_id.is_empty() {
        return Err(OutcomeRenderError::EmptyRequestId);
    }
    if trace_path.is_empty() {
        return Err(OutcomeRenderError::EmptyTracePath);
    }
    if request_id.len() > IDENTIFIER_UTF8_BYTES {
        return Err(BoundError::TooLong {
            field: "request_id",
            max: IDENTIFIER_UTF8_BYTES,
            actual: request_id.len(),
        }
        .into());
    }
    if trace_path.len() > FILESYSTEM_PATH_UTF8_BYTES {
        return Err(BoundError::TooLong {
            field: "trace",
            max: FILESYSTEM_PATH_UTF8_BYTES,
            actual: trace_path.len(),
        }
        .into());
    }
    Ok(())
}

pub(crate) fn outcome_wire(class: OutcomeClass) -> &'static str {
    match class {
        OutcomeClass::Completed => "completed",
        OutcomeClass::Rejected => "rejected",
        OutcomeClass::Error => "error",
    }
}

pub(crate) fn lifecycle_wire(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Final => "final",
        Lifecycle::Terminated => "terminated",
    }
}

pub(crate) fn merge_operation_data(
    core_data: Value,
    operation_data: &Value,
) -> Result<Value, OutcomeRenderError> {
    let Value::Object(mut merged) = core_data else {
        return Err(OutcomeRenderError::InvalidOperationData);
    };
    let Value::Object(extensions) = operation_data else {
        if operation_data.is_null() {
            return Ok(Value::Object(merged));
        }
        return Err(OutcomeRenderError::InvalidOperationData);
    };
    for (key, value) in extensions {
        if CORE_DATA_KEYS.contains(&key.as_str()) {
            return Err(OutcomeRenderError::OperationDataOverridesCore {
                field: CORE_DATA_KEYS
                    .iter()
                    .copied()
                    .find(|candidate| *candidate == key.as_str())
                    .unwrap_or("run"),
            });
        }
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

/// Maps one core diagnostic into the structured CLI diagnostics entry shape.
///
/// `Diagnostic::path()` is an optional path string, not JSON context. When
/// present it becomes `{"path": <string>}`; callers must never parse it as JSON.
pub(crate) fn render_diagnostic_entry(diagnostic: &Diagnostic) -> Value {
    let mut object = Map::new();
    object.insert("code".into(), json!(diagnostic.code()));
    object.insert("message".into(), json!(diagnostic.message()));
    if let Some(path) = diagnostic.path() {
        let mut context = Map::new();
        context.insert("path".into(), Value::String(path.to_owned()));
        object.insert("context".into(), Value::Object(context));
    }
    Value::Object(object)
}

pub(crate) fn render_diagnostics(diagnostics: &[Diagnostic]) -> Value {
    Value::Array(diagnostics.iter().map(render_diagnostic_entry).collect())
}

#[cfg(test)]
mod diagnostic_tests {
    use loop_engine_core::model::diagnostic::Diagnostic;
    use serde_json::json;

    use super::render_diagnostic_entry;

    #[test]
    fn path_maps_to_context_path_field_without_json_parsing() {
        let diagnostic = Diagnostic::new(
            "provider.invocation",
            "Role describe timed out after 60 seconds",
            Some(r#"{"role":"describe","timeout_seconds":60}"#.into()),
        )
        .unwrap();
        let entry = render_diagnostic_entry(&diagnostic);
        assert_eq!(
            entry["context"],
            json!({"path": r#"{"role":"describe","timeout_seconds":60}"#})
        );
    }

    #[test]
    fn filesystem_path_maps_to_context_path_field() {
        let diagnostic = Diagnostic::new(
            "input.invalid",
            "invalid input file",
            Some("/inputs/name".into()),
        )
        .unwrap();
        let entry = render_diagnostic_entry(&diagnostic);
        assert_eq!(entry["context"], json!({"path": "/inputs/name"}));
    }

    #[test]
    fn diagnostics_without_path_omit_context() {
        let diagnostic = Diagnostic::new("provider.invocation", "first", None).unwrap();
        let entry = render_diagnostic_entry(&diagnostic);
        assert!(entry.get("context").is_none());
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::model::ids::{RunId, StateId};
    use loop_engine_core::model::lifecycle::Lifecycle;
    use loop_engine_core::model::outcome::{OutcomeClass, OutcomeData, PublicOutcome, RunSnapshot};
    use loop_engine_core::model::reason::{Reason, ReasonCode};
    use loop_engine_core::operations::catalog::OperationId;
    use serde_json::json;

    use super::{
        OutcomeRenderError, OutcomeRenderRequest, merge_operation_data, outcome_wire,
        validate_invocation_fields,
    };

    #[test]
    fn invocation_fields_reject_empty_and_oversized_values() {
        assert!(matches!(
            validate_invocation_fields("", "/trace.jsonl"),
            Err(OutcomeRenderError::EmptyRequestId)
        ));
        assert!(matches!(
            validate_invocation_fields("req", ""),
            Err(OutcomeRenderError::EmptyTracePath)
        ));
        assert!(validate_invocation_fields("req", "/trace.jsonl").is_ok());
    }

    #[test]
    fn outcome_wire_matches_frozen_contract_classes() {
        assert_eq!(outcome_wire(OutcomeClass::Completed), "completed");
        assert_eq!(outcome_wire(OutcomeClass::Rejected), "rejected");
        assert_eq!(outcome_wire(OutcomeClass::Error), "error");
    }

    #[test]
    fn operation_data_cannot_override_core_run_summary_fields() {
        let core = json!({"run": {"id": "run"}});
        let extensions = json!({"items": [], "run": {"id": "other"}});
        assert!(matches!(
            merge_operation_data(core, &extensions),
            Err(OutcomeRenderError::OperationDataOverridesCore { field: "run" })
        ));
    }

    #[test]
    fn render_request_carries_operation_outcome_and_extensions() {
        let run = RunSnapshot {
            run_id: RunId::parse("01J9X3K2M4N5P6Q7R8S9T0V2X").unwrap(),
            label: None,
            lifecycle: Lifecycle::Active,
            current_state: StateId::parse("explore").unwrap(),
            state_changed: false,
        };
        let outcome = PublicOutcome::new(
            OutcomeClass::Rejected,
            Some(Reason::new(ReasonCode::GateFailed, "gate failed").unwrap()),
            OutcomeData::new(Some(run), Some(vec![]), None).unwrap(),
            vec![],
        )
        .unwrap();
        let request = OutcomeRenderRequest::new(
            OperationId::parse("run.request").unwrap(),
            "01J9X3K2M4N5P6Q7R8S9T0V3Y",
            "/tmp/trace.jsonl",
            &outcome,
        )
        .with_operation_data(json!({"provider_executed": true}));
        assert_eq!(request.operation.as_str(), "run.request");
        assert_eq!(request.operation_data["provider_executed"], json!(true));
    }
}
