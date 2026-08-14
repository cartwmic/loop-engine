//! Strict provider wire DTOs and response constructors.
//!
//! The engine sends one flat JSON envelope for each operation. Request parsing
//! owns protocol shape; evaluation modules own domain behavior.

use loop_core::{ContextRecord, DurableEvaluation, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Value};

/// The input envelope accepted by `describe`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescribeRequest {
    pub(crate) operation: String,
}

/// The complete flat input envelope accepted by `evaluate`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluateRequest {
    pub(crate) operation: String,
    pub(crate) workflow: Workflow,
    pub(crate) initial_input: Value,
    pub(crate) context: Vec<ContextRecord>,
    pub(crate) transition: Transition,
    pub(crate) prior_evaluations: Vec<DurableEvaluation>,
}

pub(crate) fn allow_response() -> Value {
    json!({"result": "allow"})
}

pub(crate) fn unsupported_response() -> Value {
    json!({"result": "unsupported"})
}

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
