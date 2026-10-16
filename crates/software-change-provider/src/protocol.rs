//! Strict provider wire DTOs and response constructors.
//!
//! The engine sends one flat JSON envelope for each operation.  Keep this
//! module deliberately boring: request parsing owns protocol shape, while
//! evaluation modules own only domain behavior and response detail content.

use loop_core::{ContextRecord, DurableEvaluation, Transition, Workflow};
use serde::Deserialize;

use serde_json::{json, Value};
/// Inert `validation-command` context data. Supplemental IDs must be new;
/// consumers validate against frozen required IDs before using this record.
/// Appending it does not execute, waive proof, or create acceptance criteria.
#[cfg(test)]
use software_change_provider::commission::ProofCommand;

#[cfg(test)]
mod command_contract_tests {
    use super::*;

    #[test]
    fn additive_command_uses_existing_fields_without_execution_or_waivers() {
        let value = json!({"id":"extra", "command":"checker", "args":[], "owner":"driver", "obligation":"additional proof"});
        let command: ProofCommand = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), value);
        let mut waiver = value;
        waiver["replaces"] = json!("required-command");
        assert!(serde_json::from_value::<ProofCommand>(waiver).is_err());
    }
}

/// The input envelope accepted by `describe`.
///
/// `initial_input` is the same caller object start freezes. Omitted means the
/// union catalog. Unknown keys still fail closed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescribeRequest {
    pub(crate) operation: String,
    #[serde(default)]
    pub(crate) initial_input: Option<Value>,
}

/// The complete flat input envelope accepted by `evaluate`.
///
/// These fields mirror `loop-integrations`' provider gateway exactly.  Do not
/// add run-history, provider identity, or other engine-internal fields here.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluateRequest {
    pub(crate) operation: String,
    pub(crate) workflow: Workflow,
    pub(crate) initial_input: Value,
    pub(crate) context: Vec<ContextRecord>,
    pub(crate) transition: Transition,
    pub(crate) prior_evaluations: Vec<DurableEvaluation>,
    #[serde(default)]
    pub(crate) driver_act: Option<loop_core::DriverActEvidence>,
}

/// Flat, closed optional admission envelope. Candidate DTO is closed too.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdmitRequest {
    pub(crate) operation: String,
    pub(crate) workflow: Workflow,
    pub(crate) initial_input: Value,
    pub(crate) context: Vec<ContextRecord>,
    pub(crate) current_state: loop_core::StateId,
    pub(crate) state_visit: u64,
    pub(crate) record: loop_core::CandidateRecord,
}

#[cfg(test)]
mod admit_contract_tests {
    use super::*;
    #[test]
    fn admit_is_flat_closed_and_requires_complete_typed_facts() {
        let workflow = crate::workflow::describe_workflow(None).unwrap();
        let value = json!({"operation":"admit","workflow":workflow,"initial_input":{},"context":[],"current_state":"explore","state_visit":0,"record":{"id":"note","kind":"opaque-note","data":false}});
        assert!(serde_json::from_value::<AdmitRequest>(value.clone()).is_ok());
        for key in [
            "workflow",
            "initial_input",
            "context",
            "current_state",
            "state_visit",
            "record",
        ] {
            let mut bad = value.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<AdmitRequest>(bad).is_err(),
                "{key}"
            );
        }
        let mut bad = value.clone();
        bad["unexpected"] = json!(true);
        assert!(serde_json::from_value::<AdmitRequest>(bad).is_err());
        let mut bad = value;
        bad["record"]["sequence"] = json!(1);
        assert!(serde_json::from_value::<AdmitRequest>(bad).is_err());
    }
}

/// Build the only valid allow response shape.
#[allow(dead_code)]
pub(crate) fn allow_response() -> Value {
    json!({"result": "allow"})
}

/// Build the only valid unsupported response shape.
#[allow(dead_code)]
pub(crate) fn unsupported_response() -> Value {
    json!({"result": "unsupported"})
}

/// Build a strict deny response.
///
/// `details` is intentionally opaque at this layer.  Later evaluation code
/// supplies its structured diagnostics, while this constructor guarantees the
/// gateway's `result`/`feedback` envelope and permits no extra response keys.
#[allow(dead_code)]
pub(crate) fn deny_response(
    code: impl Into<String>,
    message: impl Into<String>,
    details: Option<Value>,
) -> Value {
    let mut feedback = serde_json::Map::new();
    feedback.insert("code".to_owned(), Value::String(code.into()));
    feedback.insert("message".to_owned(), Value::String(message.into()));
    if let Some(details) = details {
        feedback.insert("details".to_owned(), details);
    }

    let mut response = serde_json::Map::new();
    response.insert("result".to_owned(), Value::String("deny".to_owned()));
    response.insert("feedback".to_owned(), Value::Object(feedback));
    Value::Object(response)
}
