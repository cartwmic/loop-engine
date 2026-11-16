use loop_core::{ContextRecord, DurableEvaluation, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Value};

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
    /// Accepted for protocol compatibility; the gates read only the context.
    #[allow(dead_code)]
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
/// The `admit` reply that lets a record be appended.
pub fn admitted() -> Value {
    json!({})
}
/// The `admit` reply that refuses a record; the engine names the record and this rule to the caller.
pub fn refused(rule: &str, message: &str) -> Value {
    json!({"refused":{"rule":rule,"message":message}})
}
