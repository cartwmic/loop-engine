use loop_core::{ContextRecord, DurableEvaluation, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribeRequest {
    pub operation: String,
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
