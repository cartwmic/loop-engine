//! Shared, provider-free visibility lanes for public CLI projections.
//!
//! The lanes are deliberately descriptive.  They never turn execution or
//! output facts into provider acceptance and never select a workflow action.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

const DECISION_KINDS: &[&str] = &["wait", "inspect", "help"];

fn unknown(reason: impl Into<String>) -> Value {
    json!({"state":"unknown","reason":reason.into()})
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn state(value: &Value) -> Option<&str> {
    string(value, "state")
}

fn is_unknown(value: &Value) -> bool {
    state(value) == Some("unknown")
}

fn is_nonempty_array(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

fn source_name(packet: &Value) -> String {
    string(packet, "source")
        .unwrap_or("selected source")
        .to_owned()
}

/// Remove observer-clock fields from a source snapshot before comparing it
/// with the preceding snapshot.  The fields are re-added at the output
/// boundary, so a read timestamp never becomes a fake state change.
pub(crate) fn strip_observation_clocks(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("observed_at");
            object.remove("sampled_at_ms");
            for child in object.values_mut() {
                strip_observation_clocks(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_observation_clocks(child);
            }
        }
        _ => {}
    }
}

/// Add the read timestamp to the already stable freshness lane only when a
/// packet is rendered.  The stable packet retained by the monitor therefore
/// does not produce a heartbeat on every poll.
pub(crate) fn add_rendered_freshness(packet: &mut Value, sampled_at_ms: u128) {
    if let Some(freshness) = packet.get_mut("freshness").and_then(Value::as_object_mut) {
        freshness.insert("sampled_at_ms".to_owned(), json!(sampled_at_ms));
    }
}

fn static_owner_update_guidance(next_action: &Value) -> Value {
    json!({
        "channel": "active Pi conversation",
        "read": ["public show/status", "monitor output"],
        "before_next_decision": DECISION_KINDS,
        "required_fields": ["observed_change", "needed_action_or_decision"],
        "machine_notification_is_not_owner_update": true,
        "unchanged_observation": "do not post a repetitive heartbeat",
        "needed_action_or_decision": next_action,
        "authority": "assistant-owned conversation guidance; it does not approve or control work"
    })
}

fn show_execution_lane(projection: &Value) -> Value {
    let Some(invocations) = projection
        .get("work_slot_invocations")
        .and_then(Value::as_array)
    else {
        return unknown("show did not expose invocation detail");
    };
    let active = invocations
        .iter()
        .find(|row| matches!(string(row, "status"), Some("running" | "overrun")));
    if let Some(row) = active {
        return json!({
            "state": if string(row, "status") == Some("overrun") { "attention" } else { "running" },
            "invocation_id": row["invocation_id"],
            "source": "full.work_slot_invocations",
            "meaning": "bound execution/liveness only; it is not output conformance or acceptance"
        });
    }
    if let Some(row) = invocations.last() {
        let status = string(row, "status").unwrap_or("unknown");
        return json!({
            "state": status,
            "invocation_id": row["invocation_id"],
            "source": "full.work_slot_invocations",
            "meaning": "the bound command outcome; it is not output conformance or acceptance"
        });
    }
    unknown("no work-slot invocation is recorded")
}

fn workflow_transition_acceptance(workflow: &Value, source: &str) -> Value {
    let Some(evaluations) = workflow.get("latest_evaluations").and_then(Value::as_array) else {
        return unknown("no durable checked-transition evaluation is available");
    };
    let Some(evaluation) = evaluations.last() else {
        return unknown("no durable checked-transition evaluation is available");
    };
    let result = evaluation
        .pointer("/result/result")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let state = match result {
        "allow" => "accepted",
        "deny" => "rejected",
        _ => "unknown",
    };
    json!({
        "state": state,
        "result": result,
        "source": source,
        "transition": evaluation["transition"],
        "meaning": "provider evaluation of a workflow transition; it does not establish worker-output or domain-result acceptance"
    })
}

fn show_worker_lane(projection: &Value) -> Value {
    let workers: Vec<Value> = projection
        .get("work_slot_invocations")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|rows| rows.iter())
        .flat_map(|row| {
            row.get("inner_workers")
                .and_then(Value::as_array)
                .into_iter()
        })
        .flat_map(|rows| rows.iter().cloned())
        .collect();
    if workers.is_empty() {
        return unknown(
            "worker output is unavailable or the invocation is still running; inspect the named capture",
        );
    }
    json!({
        "state": "available",
        "workers": workers,
        "source": "full.work_slot_invocations[].inner_workers",
        "meaning": "recorded worker execution facts; semantic meaning remains outside core"
    })
}

fn show_evidence_lane(projection: &Value, view: &str, database: &Path) -> Value {
    let mut locations = BTreeSet::new();
    let mut has_capture = false;
    if view != "full" {
        locations.insert(format!(
            "loop-engine --database {} --json show {} --view full",
            database.display(),
            projection["run_id"].as_str().unwrap_or("RUN_ID")
        ));
    }
    locations.insert("full.work_slot_invocations".to_owned());
    locations.insert("full.change_report".to_owned());
    locations.insert("full.evaluation_history".to_owned());
    if let Some(rows) = projection
        .get("work_slot_invocations")
        .and_then(Value::as_array)
    {
        for row in rows {
            if let Some(path) = string(row, "capture_dir") {
                has_capture = true;
                locations.insert(path.to_owned());
            }
            if let Some(workers) = row.get("inner_workers").and_then(Value::as_array) {
                for worker in workers {
                    for key in ["selected_output_path", "selected_output_sha256"] {
                        if let Some(value) = string(worker, key) {
                            if key == "selected_output_path" && !Path::new(value).is_absolute() {
                                if let Some(capture_dir) = string(row, "capture_dir") {
                                    locations.insert(
                                        Path::new(capture_dir)
                                            .join(value)
                                            .to_string_lossy()
                                            .into_owned(),
                                    );
                                } else {
                                    locations.insert(value.to_owned());
                                }
                            } else {
                                locations.insert(value.to_owned());
                            }
                        }
                    }
                }
            }
        }
    }
    json!({
        "state": if has_capture { "available" } else { "partial" },
        "locations": locations.into_iter().collect::<Vec<_>>(),
        "meaning": "locations identify retained detail; a location does not make its contents valid or accepted"
    })
}

fn show_next_action(projection: &Value, execution: &Value, worker: &Value) -> Value {
    let lifecycle = string(projection, "lifecycle").unwrap_or("unknown");
    if matches!(lifecycle, "final" | "terminated") {
        return json!({
            "kind": "inspect",
            "reason": "the run is terminal and read-only",
            "evidence": "full.work_slot_invocations and full.evaluation_history"
        });
    }
    match state(execution) {
        Some("running") => json!({
            "kind": "wait",
            "reason": "owned execution is still running",
            "evidence": "full.work_slot_invocations.capture_dir"
        }),
        Some("attention" | "failed") => json!({
            "kind": "inspect",
            "reason": "execution needs triage before retry or departure",
            "evidence": "full.work_slot_invocations.capture_dir"
        }),
        _ if !is_unknown(worker) => json!({
            "kind": "inspect",
            "reason": "worker output is recorded; inspect conformance and provider evidence",
            "evidence": "full.work_slot_invocations[].inner_workers"
        }),
        _ => json!({
            "kind": "inspect",
            "reason": "read current instructions and the available event before performing or binding work",
            "evidence": "show --view action and show --view full"
        }),
    }
}

/// Lanes added to the provider-free `show` packet.  They are additive to the
/// existing projection and intentionally keep semantic acceptance unknown.
pub(crate) fn show_lanes(projection: &Value, view: &str, database: &Path) -> Value {
    let workflow = json!({
        "state": projection["lifecycle"],
        "current_state": projection["current_state"],
        "title": projection["current_state_title"],
        "requestable_events": projection["requestable_events"],
        "source": "show",
        "mutation_armed": view != "status"
    });
    let execution = show_execution_lane(projection);
    let worker = show_worker_lane(projection);
    let conformance = json!({
        "state": "unknown",
        "reason": "show is provider-free and does not inspect or interpret worker output conformance",
        "source": "full.work_slot_invocations"
    });
    let acceptance = json!({
        "state": "unknown",
        "reason": "no explicit provider or driver acceptance record is supplied by show; execution and conformance are not acceptance",
        "workflow_transition": workflow_transition_acceptance(projection, "full.evaluation_history"),
        "sources": ["full.change_report", "full.evaluation_history"]
    });
    let evidence = show_evidence_lane(projection, view, database);
    let next_action = show_next_action(projection, &execution, &worker);
    let mut reasons = vec![
        "semantic acceptance is unknown until the provider/driver evidence process accepts the result".to_owned(),
        "output conformance is not interpreted by provider-free show".to_owned(),
    ];
    if is_unknown(&worker) {
        reasons.push("worker output is unavailable or not yet recorded".to_owned());
    }
    if state(&evidence) != Some("available") {
        reasons.push("retained evidence locations are partial".to_owned());
    }
    json!({
        "workflow": workflow,
        "execution": execution,
        "worker": worker,
        "conformance": conformance,
        "acceptance": acceptance,
        "evidence": evidence,
        "freshness": {
            "state": "observed",
            "source": "show",
            "meaning": "this is the source state read for this packet; it may change after the read"
        },
        "uncertainty": {"state":"present","reasons":reasons},
        "next_action": next_action,
        "owner_update_guidance": static_owner_update_guidance(&next_action)
    })
}

fn monitor_workflow_lane(packet: &Value) -> Value {
    let raw = &packet["workflow"];
    if is_unknown(raw) {
        return json!({
            "state": "unknown",
            "reason": raw["reason"],
            "source": source_name(packet)
        });
    }
    json!({
        "state": raw.get("lifecycle").or_else(|| raw.get("state")).cloned().unwrap_or_else(|| json!("unknown")),
        "current_state": raw["current_state"],
        "source": source_name(packet),
        "meaning": "authoritative workflow state; observation has no progression authority"
    })
}

fn monitor_execution_lane(packet: &Value) -> Value {
    if let Some(status) = packet
        .pointer("/execution_status_source/action/status")
        .and_then(Value::as_str)
    {
        return json!({
            "state": match status { "succeeded" => "succeeded", "failed" => "failed", _ => "unknown" },
            "status": status,
            "source": "history.invocation_status_changed",
            "meaning": "selected invocation execution only; not output conformance or acceptance"
        });
    }
    if let Some(status) = string(&packet["helper"], "status") {
        return json!({
            "state": match status { "completed" => "succeeded", "failed" => "failed", "running" => "running", _ => "unknown" },
            "status": status,
            "source": format!("{}#state.json", source_name(packet)),
            "meaning": "capture execution state only; not output conformance or acceptance"
        });
    }
    if let Some(state) = packet
        .pointer("/helper/visibility/execution/state")
        .and_then(Value::as_str)
    {
        return json!({"state":state,"source":"invocation-progress.visibility.execution","meaning":"helper/overlay execution only; not acceptance"});
    }
    if packet["boundary"]["event"] == "completion" {
        return json!({
            "state": "succeeded",
            "source": source_name(packet),
            "meaning": "selected execution completed; not output conformance or semantic acceptance"
        });
    }
    if packet["boundary"]["event"] == "attention" {
        return json!({
            "state": "attention",
            "source": source_name(packet),
            "meaning": "observation needs triage; no retry or cancellation authority"
        });
    }
    unknown("selected execution status is not available from this observation")
}

fn ensure_monitor_conformance_state(packet: &mut Value) {
    let current = packet["conformance"].clone();
    let mut conformance = if current.is_object() {
        current
    } else {
        unknown("no conformance evidence")
    };
    if conformance.get("state").is_none() {
        let derived = if packet["boundary"]["event"] == "completion"
            && packet["boundary"]["reason"].as_str().is_some_and(|reason| {
                reason.contains("conformance") || reason.contains("mechanical")
            }) {
            "satisfied"
        } else {
            "unknown"
        };
        conformance["state"] = json!(derived);
    }
    packet["conformance"] = conformance;
}

fn pathish(key: &str) -> bool {
    matches!(
        key,
        "source"
            | "capture_dir"
            | "receipt"
            | "stdout_path"
            | "stderr_path"
            | "selected_output_path"
            | "path"
            | "locator"
    ) || key.ends_with("_path")
}

fn collect_locations(value: &Value, locations: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if pathish(key) {
                    if let Some(text) = child.as_str().filter(|text| !text.is_empty()) {
                        locations.insert(text.to_owned());
                    }
                }
                collect_locations(child, locations);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_locations(item, locations);
            }
        }
        _ => {}
    }
}

fn monitor_evidence_lane(packet: &Value) -> Value {
    let mut locations = BTreeSet::new();
    collect_locations(packet, &mut locations);
    let source = source_name(packet);
    locations.insert(source.clone());
    let detail_available = locations.iter().any(|location| {
        location.starts_with("/") && (location.ends_with(".json") || Path::new(location).exists())
    });
    let state = if detail_available && !is_unknown(&packet["worker"]) {
        "available"
    } else if detail_available || locations.len() > 1 {
        "partial"
    } else {
        "unknown"
    };
    json!({
        "state": state,
        "locations": locations.into_iter().collect::<Vec<_>>(),
        "meaning": "locations identify retained source/detail; they do not establish validity or acceptance"
    })
}

fn monitor_next_action(packet: &Value, execution: &Value) -> Value {
    if matches!(state(execution), Some("running")) {
        return json!({"kind":"wait","reason":"selected owned execution is still running","evidence":"execution and evidence lanes"});
    }
    if matches!(state(execution), Some("failed" | "attention"))
        || state(&packet["conformance"]) != Some("satisfied")
    {
        return json!({"kind":"inspect","reason":"inspect retained execution/output evidence before retry or departure","evidence":"evidence.locations"});
    }
    json!({"kind":"help","reason":"execution and declared mechanical checks are visible, but provider/driver acceptance remains unknown","evidence":"acceptance.sources","owner_decision":"decide whether the retained result is acceptable"})
}

fn monitor_uncertainty(packet: &Value, execution: &Value, evidence: &Value) -> Value {
    let mut reasons = Vec::new();
    for (name, lane) in [
        ("workflow", &packet["workflow_lane"]),
        ("execution", execution),
        ("worker", &packet["worker"]),
        ("conformance", &packet["conformance"]),
        ("evidence", evidence),
    ] {
        if is_unknown(lane) {
            reasons.push(format!("{name} is unknown"));
        }
    }
    reasons.push(
        "semantic acceptance is unknown until an explicit provider/driver decision".to_owned(),
    );
    if is_nonempty_array(packet, "diagnostics") {
        reasons.push("diagnostics report incomplete, stale, or conflicting observation".to_owned());
    }
    json!({"state":"present","reasons":reasons})
}

/// Add stable public lanes to a monitor packet.  Existing helper/judgment
/// fields remain for compatibility; consumers can use the explicit lanes
/// without reconstructing them from raw detail.
pub(crate) fn enrich_monitor_packet(packet: &mut Value) {
    let workflow_lane = monitor_workflow_lane(packet);
    packet["workflow_lane"] = workflow_lane;
    ensure_monitor_conformance_state(packet);
    let execution = monitor_execution_lane(packet);
    packet["execution"] = execution.clone();
    packet["worker_lane"] = if is_unknown(&packet["worker"]) {
        unknown("worker output is unavailable or not yet recorded")
    } else {
        json!({"state":"available","source":"worker","meaning":"recorded worker execution facts; semantic meaning remains external"})
    };
    let evidence = monitor_evidence_lane(packet);
    packet["evidence"] = evidence.clone();
    packet["acceptance"] = json!({
        "state": "unknown",
        "reason": "no explicit provider or driver acceptance record was supplied; execution and output conformance are not acceptance",
        "workflow_transition": workflow_transition_acceptance(&packet["workflow"], "workflow.latest_evaluations"),
        "sources": ["judgment", "workflow", "evidence"]
    });
    packet["freshness"] = json!({
        "state": "observed",
        "source": source_name(packet),
        "meaning": "read during this observer sample; source-to-observation lag and current applicability remain possible"
    });
    packet["next_action"] = monitor_next_action(packet, &execution);
    packet["owner_update_guidance"] = static_owner_update_guidance(&packet["next_action"]);
    packet["uncertainty"] = monitor_uncertainty(packet, &execution, &evidence);
}

fn changed_labels(previous: Option<&Value>, current: &Value) -> Vec<&'static str> {
    let Some(previous) = previous else {
        return vec!["initial observation"];
    };
    let mut changed = Vec::new();
    for (label, key) in [
        ("workflow", "workflow_lane"),
        ("execution", "execution"),
        ("helper progress", "helper"),
        ("worker output", "worker"),
        ("output conformance", "conformance"),
        ("retained evidence", "evidence"),
        ("next action", "next_action"),
        ("attention boundary", "boundary"),
    ] {
        if previous.get(key) != current.get(key) {
            changed.push(label);
        }
    }
    changed
}

/// Build the assistant-owned conversation checkpoint after comparing stable
/// source packets.  The monitor only describes the obligation; it never sends
/// a chat message or controls the observed run.
pub(crate) fn owner_update(previous: Option<&Value>, current: &Value) -> Value {
    let changes = changed_labels(previous, current);
    let meaningful = !changes.is_empty();
    let observed_change = if meaningful {
        changes.join(", ")
    } else {
        "no meaningful change since the last observation".to_owned()
    };
    json!({
        "required_before_next_decision": meaningful,
        "channel": "active Pi conversation",
        "observed_change": observed_change,
        "needed_action_or_decision": current["next_action"],
        "before_next_decision": DECISION_KINDS,
        "machine_notification_is_not_owner_update": true,
        "unchanged_observation": "suppress the heartbeat"
    })
}
