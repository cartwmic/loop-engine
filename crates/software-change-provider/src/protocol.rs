//! Strict provider wire DTOs and response constructors.
//!
//! The engine sends one flat JSON envelope for each operation.  Keep this
//! module deliberately boring: request parsing owns protocol shape, while
//! evaluation modules own only domain behavior and response detail content.

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
