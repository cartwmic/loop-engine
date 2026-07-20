use schemars::schema_for;
use serde_json::{Value, json};

use super::event::TraceEvent;

pub fn trace_event_schema() -> Value {
    let mut schema =
        serde_json::to_value(schema_for!(TraceEvent)).expect("trace schema serializes");
    let properties = schema["properties"]
        .as_object_mut()
        .expect("trace schema has object properties");
    properties.insert(
        "trace_schema_version".into(),
        json!({"type":"integer","const":1}),
    );
    properties
        .entry("request_id")
        .and_modify(|value| value["maxLength"] = json!(256));
    properties
        .entry("event")
        .and_modify(|value| value["maxLength"] = json!(256));
    schema["x-loop-engine-bound-markers"] = json!({
        "initial_reservation": "trace_init_reservation_bytes",
        "provider_call_reservation": "trace_provider_call_reservation_bytes",
        "file": "trace_file_max_bytes",
        "directory": "trace_directory_budget_bytes"
    });
    schema
}
