use std::collections::BTreeMap;

use jiff::{Timestamp, Unit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TRACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceCategory {
    Invocation,
    Driver,
    Parse,
    Provider,
    Persistence,
    Decision,
    Trace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TraceEvent {
    pub trace_schema_version: u32,
    pub ts: String,
    pub request_id: String,
    pub category: TraceCategory,
    pub event: String,
    #[serde(flatten)]
    pub payload: BTreeMap<String, Value>,
}

impl TraceEvent {
    pub fn new(
        request_id: impl Into<String>,
        category: TraceCategory,
        event: impl Into<String>,
        payload: BTreeMap<String, Value>,
    ) -> Self {
        let ts = Timestamp::now()
            .round(Unit::Millisecond)
            .expect("current timestamp rounds to milliseconds")
            .to_string();
        Self {
            trace_schema_version: TRACE_SCHEMA_VERSION,
            ts,
            request_id: request_id.into(),
            category,
            event: event.into(),
            payload,
        }
    }
}
