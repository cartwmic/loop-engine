//! Runtime operation coverage derived from production outcomes and traces (T145).

use std::collections::BTreeSet;

use serde_json::Value;

use super::cli::{PreDispatchFailure, StructuredDocument};
use super::trace::ParsedTrace;

/// Collects operation IDs observed from parsed production CLI output only.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuntimeCoverageRecorder {
    e2e_operations: BTreeSet<String>,
    trace_operations: BTreeSet<String>,
}

impl RuntimeCoverageRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.e2e_operations.is_empty() && self.trace_operations.is_empty()
    }

    pub fn e2e_operations(&self) -> Vec<String> {
        self.e2e_operations.iter().cloned().collect()
    }

    pub fn trace_operations(&self) -> Vec<String> {
        self.trace_operations.iter().cloned().collect()
    }

    pub fn observe_stdout(&mut self, document: &StructuredDocument) {
        if let Some(operation) = operation_from_outcome_envelope(&document.value) {
            self.e2e_operations.insert(operation);
        }
    }

    pub fn observe_stderr(&mut self, failure: &PreDispatchFailure) {
        let _ = failure;
    }

    pub fn observe_trace(&mut self, trace: &ParsedTrace) {
        for event in &trace.events {
            if let Some(operation) = operation_from_trace_event(event) {
                self.trace_operations.insert(operation);
            }
        }
    }

    pub fn observe_invocation(
        &mut self,
        stdout: Option<&StructuredDocument>,
        stderr: Option<&PreDispatchFailure>,
        trace: Option<&ParsedTrace>,
    ) {
        if let Some(document) = stdout {
            self.observe_stdout(document);
        }
        if let Some(failure) = stderr {
            self.observe_stderr(failure);
        }
        if let Some(parsed) = trace {
            self.observe_trace(parsed);
        }
    }
}

fn operation_from_outcome_envelope(value: &Value) -> Option<String> {
    if value.get("kind").is_some() {
        return None;
    }
    value
        .get("operation")
        .and_then(Value::as_str)
        .filter(|operation| !operation.is_empty())
        .map(str::to_owned)
}

fn operation_from_trace_event(event: &Value) -> Option<String> {
    let category = event.get("category")?.as_str()?;
    let name = event.get("event")?.as_str()?;
    match (category, name) {
        ("invocation", "request") => event
            .get("operation")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ("invocation", "outcome") => event
            .get("envelope")
            .and_then(|envelope| envelope.get("operation"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    #[test]
    fn observes_request_and_outcome_envelope_operations() {
        let mut recorder = RuntimeCoverageRecorder::new();
        let trace = ParsedTrace {
            path: PathBuf::from("/tmp/example.jsonl"),
            request_id: "01J9X3K2M4N5P6Q7R8S9T0V1W".to_owned(),
            events: vec![
                json!({
                    "category": "invocation",
                    "event": "request",
                    "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W",
                    "operation": "run.show"
                }),
                json!({
                    "category": "invocation",
                    "event": "outcome",
                    "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W",
                    "envelope": {
                        "operation": "run.export",
                        "request_id": "01J9X3K2M4N5P6Q7R8S9T0V1W"
                    }
                }),
            ],
        };
        recorder.observe_trace(&trace);
        assert_eq!(
            recorder.trace_operations(),
            vec!["run.export".to_owned(), "run.show".to_owned()]
        );
    }
}
