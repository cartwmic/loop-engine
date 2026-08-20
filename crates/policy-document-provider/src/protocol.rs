use loop_core::{ContextRecord, DurableEvaluation, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Value};

/// `initial_input` is accepted and ignored so engine start can always send the
/// caller object. Topology stays input-independent. Unknown keys fail closed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribeRequest {
    pub operation: String,
    #[serde(default)]
    pub initial_input: Option<Value>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluateRequest {
    pub operation: String,
    pub workflow: Workflow,
    pub initial_input: Value,
    pub context: Vec<ContextRecord>,
    pub transition: Transition,
    pub prior_evaluations: Vec<DurableEvaluation>,
}
pub fn allow() -> Value {
    json!({"result":"allow"})
}
pub fn unsupported() -> Value {
    json!({"result":"unsupported"})
}
pub fn deny(code: &str, message: &str, details: Value) -> Value {
    json!({"result":"deny","feedback":{"code":code,"message":message,"details":details}})
}
