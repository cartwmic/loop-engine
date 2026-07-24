use std::fmt::Write as _;

use serde_json::{Map, Value};

use super::dto::{OutcomeRenderError, OutcomeRenderRequest, STRUCTURED_CLI_ENVELOPE_BYTES};
use super::json::build_outcome_envelope;

/// Renders exactly one bounded human-readable outcome presentation for stdout.
pub fn render_human_outcome(
    request: &OutcomeRenderRequest<'_>,
) -> Result<String, OutcomeRenderError> {
    let envelope = build_outcome_envelope(request)?;
    render_human_envelope(&envelope)
}

/// Renders the human presentation as UTF-8 bytes after enforcing the stdout byte bound.
pub fn render_human_outcome_bytes(
    request: &OutcomeRenderRequest<'_>,
) -> Result<Vec<u8>, OutcomeRenderError> {
    let rendered = render_human_outcome(request)?;
    Ok(rendered.into_bytes())
}

/// Renders one human presentation from the authoritative structured outcome envelope.
pub fn render_human_envelope(envelope: &Value) -> Result<String, OutcomeRenderError> {
    let mut lines = Vec::new();

    lines.push(format!(
        "Operation: {}",
        envelope["operation"]
            .as_str()
            .expect("build_outcome_envelope always sets operation")
    ));

    let outcome = envelope["outcome"]
        .as_str()
        .expect("build_outcome_envelope always sets outcome");
    lines.push(format!("Outcome: {outcome}"));

    if let Some(reason) = envelope.get("reason").and_then(Value::as_object) {
        let code = reason["code"].as_str().unwrap_or("");
        let message = reason["message"].as_str().unwrap_or("");
        lines.push(format!("Reason: {code} — {message}"));
    }

    let data = envelope
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(value) = data.get("provider_executed") {
        lines.push(format!(
            "Provider executed: {}",
            yes_no(value.as_bool().unwrap_or(false))
        ));
    }

    if let Some(registration) = data.get("registration").and_then(Value::as_object) {
        if let Some(id) = registration.get("id").and_then(Value::as_str) {
            lines.push(format!("Registration ID: {id}"));
        }
        if let Some(handle) = registration.get("handle").and_then(Value::as_str) {
            lines.push(format!("Handle: {handle}"));
        }
    }

    if let Some(conformance) = data.get("conformance").and_then(Value::as_object) {
        if let Some(major) = conformance.get("protocol_major") {
            lines.push(format!("Protocol major: {major}"));
        }
        if let Some(status) = conformance.get("graph_status").and_then(Value::as_str) {
            lines.push(format!("Graph: {status}"));
        }
        if let Some(revision) = conformance.get("graph_revision").and_then(Value::as_str) {
            lines.push(format!("Graph revision: {revision}"));
        }
    }

    if let Some(run) = data.get("run").and_then(Value::as_object) {
        if let Some(id) = run.get("id").and_then(Value::as_str) {
            lines.push(format!("Run: {id}"));
        }
        if let Some(label) = run.get("label")
            && !label.is_null()
        {
            lines.push(format!("Label: {}", label.as_str().unwrap_or("")));
        }
        if let Some(lifecycle) = run.get("lifecycle").and_then(Value::as_str) {
            lines.push(format!("Lifecycle: {lifecycle}"));
        }
        if let Some(state) = run.get("state").and_then(Value::as_str) {
            let state_changed = run
                .get("state_changed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if state_changed {
                lines.push(format!("State: {state}"));
                lines.push("State changed: yes".into());
            } else if outcome == "completed" {
                lines.push(format!("State: {state}"));
            } else {
                lines.push(format!("State: {state} (unchanged)"));
            }
        } else if run
            .get("state_changed")
            .and_then(Value::as_bool)
            .is_some_and(|changed| changed)
        {
            lines.push("State changed: yes".into());
        }
    }

    if let Some(revision) = data.get("graph_revision").and_then(Value::as_str) {
        lines.push(format!("Graph revision: {revision}"));
    }

    if let Some(inputs) = data.get("inputs").and_then(Value::as_object) {
        let rendered = serde_json::to_string(inputs)
            .expect("structured outcome data is always JSON serializable");
        lines.push(format!("Inputs: {rendered}"));
    }

    if let Some(guidance) = data.get("static_guidance").and_then(Value::as_object) {
        match guidance.get("kind").and_then(Value::as_str) {
            Some("text") => lines.push(format!(
                "Guidance: {}",
                guidance.get("text").and_then(Value::as_str).unwrap_or("")
            )),
            Some("none_required") => lines.push("Guidance: no additional guidance required".into()),
            _ => {}
        }
    }

    if let Some(capability) = data.get("live_guidance").and_then(Value::as_str) {
        lines.push(format!("Live guidance: {capability}"));
    }

    if let Some(selected) = data.get("selected_evidence").and_then(Value::as_array) {
        if selected.is_empty() {
            lines.push("Selected evidence: none".into());
        } else {
            lines.push(format!(
                "Selected evidence: {}",
                selected
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(details) = data
        .get("requestable_event_details")
        .and_then(Value::as_array)
        && !details.is_empty()
    {
        lines.push("Requestable event details:".into());
        for detail in details {
            let event = detail.get("event").and_then(Value::as_str).unwrap_or("");
            let target = detail.get("target").and_then(Value::as_str).unwrap_or("");
            let gates = detail
                .get("required_gates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let gates = if gates.is_empty() {
                "none".to_owned()
            } else {
                gates.join(", ")
            };
            lines.push(format!("  {event} -> {target} (required gates: {gates})"));
        }
    }

    if data.get("evidence_added").and_then(Value::as_bool) == Some(true) {
        lines.push("Evidence recorded: yes".into());
    }

    if let Some(status) = data.get("evidence_recorded") {
        lines.extend(render_evidence_recorded(status, outcome));
    }

    if let Some(verdicts) = data.get("gate_verdicts") {
        lines.extend(render_gate_verdicts(verdicts));
    }

    if let Some(findings) = data.get("findings") {
        lines.extend(render_findings(findings));
    }

    if let Some(guidance) = data.get("guidance").and_then(Value::as_str) {
        lines.push(format!("Guidance: {guidance}"));
    }

    if let Some(export) = data.get("export").and_then(Value::as_object) {
        if let Some(output) = export.get("output").and_then(Value::as_str) {
            lines.push(format!("Output: {output}"));
        }
        if let Some(manifest) = export.get("manifest_file").and_then(Value::as_str) {
            lines.push(format!("Manifest: {manifest}"));
        }
        if let Some(state_file) = export.get("state_file").and_then(Value::as_str) {
            lines.push(format!("State file: {state_file}"));
        }
        if let Some(journal_file) = export.get("journal_file").and_then(Value::as_str) {
            lines.push(format!("Journal file: {journal_file}"));
        }
    }

    if let Some(events) = data.get("requestable_events").and_then(Value::as_array) {
        lines.push("Requestable events:".into());
        for event in events {
            if let Some(name) = event.as_str() {
                lines.push(format!("  {name}"));
            }
        }
    }

    if let Some(items) = data.get("items").and_then(Value::as_array) {
        if active_graph_items(items) {
            lines.extend(render_active_graphs(items));
        } else {
            lines.extend(render_list_items(items));
        }
    }

    if let Some(diagnostics) = envelope.get("diagnostics").and_then(Value::as_array)
        && !diagnostics.is_empty()
    {
        lines.push("Diagnostics:".into());
        for entry in diagnostics {
            lines.push(render_diagnostic_line(entry));
        }
    }

    lines.push(format!(
        "Request ID: {}",
        envelope["request_id"]
            .as_str()
            .expect("build_outcome_envelope always sets request_id")
    ));
    lines.push(format!(
        "Trace: {}",
        envelope["trace"]
            .as_str()
            .expect("build_outcome_envelope always sets trace")
    ));

    let rendered = lines.join("\n");
    ensure_presentation_bound(&rendered)?;
    Ok(rendered)
}

fn render_evidence_recorded(status: &Value, outcome: &str) -> Vec<String> {
    let inline = status["inline"].as_bool().unwrap_or(false);
    let selected = status["selected_associations"].as_bool().unwrap_or(false);
    let provider = status["provider"].as_bool().unwrap_or(false);

    if inline && selected && provider {
        return vec!["Evidence recorded: yes".into()];
    }

    if outcome == "error" || !(inline || selected || provider) {
        return vec![
            format!("Submitted inline evidence recorded: {}", yes_no(inline)),
            format!(
                "Selected evidence associations recorded: {}",
                yes_no(selected)
            ),
            format!("Provider evidence recorded: {}", yes_no(provider)),
        ];
    }

    vec!["Evidence recorded: yes".into()]
}

fn render_gate_verdicts(value: &Value) -> Vec<String> {
    let mut lines = vec!["Gate verdicts:".into()];
    let Some(items) = value.as_array() else {
        return lines;
    };

    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let gate = object
            .get("gate")
            .or_else(|| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let verdict = object
            .get("verdict")
            .or_else(|| object.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("");
        lines.push(format!("  {gate}: {verdict}"));
        if let Some(message) = object
            .get("message")
            .or_else(|| object.get("detail"))
            .and_then(Value::as_str)
        {
            lines.push(format!("    {message}"));
        }
    }

    lines
}

fn render_findings(value: &Value) -> Vec<String> {
    let mut lines = vec!["Findings:".into()];
    let Some(items) = value.as_array() else {
        return lines;
    };

    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let key = finding_key(object);
        let status = object.get("status").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("  {key}: {status}"));
        if let Some(message) = object
            .get("message")
            .or_else(|| object.get("detail"))
            .and_then(Value::as_str)
        {
            lines.push(format!("    {message}"));
        }
    }

    lines
}

fn active_graph_items(items: &[Value]) -> bool {
    items.iter().any(|item| {
        item.as_object().is_some_and(|object| {
            object.contains_key("run_id")
                || object.contains_key("graph_revision")
                || object.contains_key("revision")
        })
    })
}

fn render_active_graphs(items: &[Value]) -> Vec<String> {
    let mut lines = vec!["Active graphs:".into()];
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let run_id = object
            .get("run_id")
            .or_else(|| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let revision = object
            .get("graph_revision")
            .or_else(|| object.get("revision"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let status = object
            .get("status")
            .or_else(|| object.get("compatibility"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if revision.is_empty() {
            lines.push(format!("  {run_id} {status}"));
        } else {
            lines.push(format!("  {run_id} {revision} {status}"));
        }
        if let Some(message) = object.get("message").and_then(Value::as_str) {
            lines.push(format!("    {message}"));
        }
    }
    lines
}

fn render_list_items(items: &[Value]) -> Vec<String> {
    let mut lines = Vec::new();
    for item in items {
        if let Some(object) = item.as_object() {
            let summary = list_item_summary(object);
            if !summary.is_empty() {
                lines.push(summary);
            }
        }
    }
    lines
}

fn list_item_summary(object: &Map<String, Value>) -> String {
    if let (Some(id), Some(handle)) = (
        object.get("id").and_then(Value::as_str),
        object.get("handle").and_then(Value::as_str),
    ) {
        return format!("{id}\t{handle}");
    }
    if let Some(id) = object.get("id").and_then(Value::as_str) {
        return id.to_owned();
    }
    object
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|text| format!("{key}={text}")))
        .collect::<Vec<_>>()
        .join("\t")
}

fn finding_key(object: &Map<String, Value>) -> String {
    for field in ["capability", "key", "id", "name"] {
        if let Some(value) = object.get(field).and_then(Value::as_str) {
            return value.to_owned();
        }
    }
    String::new()
}

fn render_diagnostic_line(entry: &Value) -> String {
    let code = entry["code"].as_str().unwrap_or("");
    let message = entry["message"].as_str().unwrap_or("");
    let mut line = format!("- {code}: {message}");
    if let Some(context) = entry.get("context").and_then(Value::as_object) {
        for (key, value) in context {
            let _ = write!(line, "\n    {key}: {}", render_context_value(value));
        }
    }
    line
}

fn render_context_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn ensure_presentation_bound(rendered: &str) -> Result<(), OutcomeRenderError> {
    if rendered.len() > STRUCTURED_CLI_ENVELOPE_BYTES {
        return Err(OutcomeRenderError::EnvelopeTooLarge {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: rendered.len(),
        });
    }
    Ok(())
}
