//! Shared, provider-free visibility lanes for public CLI projections.
//!
//! The lanes are deliberately descriptive.  They never turn execution or
//! output facts into provider acceptance and never select a workflow action.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

const DECISION_KINDS: &[&str] = &["wait", "inspect", "help"];

#[derive(Clone, Debug)]
struct AcceptanceCandidate {
    state: String,
    source: String,
    source_locators: Vec<String>,
    attribution: Value,
    freshness: Value,
    target: Option<Value>,
}

fn unknown(reason: impl Into<String>) -> Value {
    json!({"state":"unknown","reason":reason.into()})
}

fn acceptance_unknown(reason: impl Into<String>, source_locators: Vec<String>) -> Value {
    let reason = reason.into();
    json!({
        "state": "unknown",
        "reason": reason,
        "evidence": {
            "state": if source_locators.is_empty() { "unknown" } else { "partial" },
            "source_locators": source_locators
        },
        "freshness": {
            "state": "unknown",
            "meaning": "no current attributed acceptance decision was established"
        },
        "uncertainty": {
            "state": "present",
            "reasons": [reason]
        }
    })
}

fn nonempty_identity(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(json!(value)),
        Some(Value::Object(value)) if !value.is_empty() => Some(Value::Object(value.clone())),
        _ => None,
    }
}

fn string_list(value: Option<&Value>) -> Option<Vec<String>> {
    let values = value?.as_array()?;
    if values.is_empty() {
        return None;
    }
    let values = values
        .iter()
        .map(|value| value.as_str().filter(|value| !value.trim().is_empty()))
        .collect::<Option<Vec<_>>>()?;
    Some(values.into_iter().map(str::to_owned).collect())
}

fn locator_list(value: Option<&Value>) -> Option<Vec<String>> {
    string_list(value).or_else(|| value.and_then(|value| string_list(value.get("source_locators"))))
}

/// Parse only the explicit, attributed acceptance shape.  Generic provider
/// fields such as `result`, review `pass`, workflow allows, and exit codes are
/// intentionally ignored: they are not domain-result decisions.
fn parse_acceptance_candidate(
    container: &Value,
    source: String,
    freshness: Value,
) -> Result<Option<AcceptanceCandidate>, String> {
    let Some(acceptance) = container
        .get("acceptance")
        .or_else(|| container.get("judgment"))
    else {
        return Ok(None);
    };
    let Some(acceptance) = acceptance.as_object() else {
        return Err("explicit acceptance must be an object".to_owned());
    };
    let state = acceptance
        .get("state")
        .or_else(|| acceptance.get("result"))
        .and_then(Value::as_str)
        .ok_or_else(|| "explicit acceptance is missing state/result".to_owned())?;
    if !matches!(state, "accepted" | "rejected") {
        return Err(format!(
            "explicit acceptance state `{state}` is not accepted or rejected"
        ));
    }
    let source_locators = locator_list(
        acceptance
            .get("source_locators")
            .or_else(|| acceptance.get("evidence"))
            .or_else(|| container.get("source_locators"))
            .or_else(|| container.get("evidence")),
    )
    .ok_or_else(|| "explicit acceptance needs non-empty evidence locators".to_owned())?;
    let attribution = nonempty_identity(
        acceptance
            .get("attesting_driver")
            .or_else(|| acceptance.get("provider"))
            .or_else(|| container.get("attesting_driver"))
            .or_else(|| container.get("provider")),
    )
    .ok_or_else(|| "explicit acceptance needs an attributed provider or driver".to_owned())?;
    let target = acceptance
        .get("target")
        .or_else(|| container.get("target"))
        .or_else(|| container.get("invocation_id"))
        .cloned();
    let supplied_freshness = acceptance
        .get("freshness")
        .or_else(|| container.get("freshness"));
    if freshness
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| matches!(state, "stale" | "unknown"))
        || supplied_freshness
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "stale" | "unknown"))
    {
        return Err("explicit acceptance source is stale or has unknown freshness".to_owned());
    }
    Ok(Some(AcceptanceCandidate {
        state: state.to_owned(),
        source,
        source_locators,
        attribution,
        freshness: supplied_freshness
            .filter(|value| value.get("state").is_some())
            .cloned()
            .unwrap_or(freshness),
        target,
    }))
}

fn target_matches(
    target: Option<&Value>,
    run_id: Option<&str>,
    invocation_ids: &[String],
) -> Result<(), String> {
    let Some(target) = target else {
        return Ok(());
    };
    let target = if let Some(target) = target.as_object() {
        target
    } else if let Some(invocation_id) = target.as_str() {
        if invocation_ids.iter().any(|id| id == invocation_id) {
            return Ok(());
        }
        return Err(format!(
            "acceptance target invocation `{invocation_id}` is not the selected invocation"
        ));
    } else {
        return Err("acceptance target must be an object or invocation ID".to_owned());
    };
    if let Some(target_run) = target.get("run_id").and_then(Value::as_str) {
        if run_id != Some(target_run) {
            return Err("acceptance target run does not match the observed run".to_owned());
        }
    }
    if let Some(invocation_id) = target.get("invocation_id").and_then(Value::as_str) {
        if !invocation_ids.iter().any(|id| id == invocation_id) {
            return Err(format!(
                "acceptance target invocation `{invocation_id}` is not the selected invocation"
            ));
        }
    }
    Ok(())
}

fn render_acceptance(candidate: &AcceptanceCandidate, extra_reasons: &[String]) -> Value {
    let mut reasons = vec![
        "the provider or driver supplied this decision; visibility does not independently verify its truth".to_owned(),
    ];
    reasons.extend(extra_reasons.iter().cloned());
    let mut source_locators = vec![candidate.source.clone()];
    source_locators.extend(candidate.source_locators.iter().cloned());
    source_locators.sort();
    source_locators.dedup();
    json!({
        "state": candidate.state,
        "source": candidate.source,
        "attributed_to": candidate.attribution,
        "evidence": {
            "state": "available",
            "source_locators": source_locators,
            "meaning": "named supporting evidence only; it is not reinterpreted by visibility"
        },
        "freshness": candidate.freshness,
        "uncertainty": {"state":"present","reasons":reasons},
        "meaning": "explicit provider/driver result decision; not inferred from execution, conformance, lifecycle, or workflow evaluation"
    })
}

fn choose_acceptance(
    candidates: Vec<AcceptanceCandidate>,
    mut issues: Vec<String>,
    source_locators: Vec<String>,
) -> Value {
    if candidates.is_empty() {
        return acceptance_unknown(
            issues.pop().unwrap_or_else(|| {
                "no explicit provider or driver acceptance decision is available".to_owned()
            }),
            source_locators,
        );
    }
    if !issues.is_empty() {
        let mut locators = source_locators;
        locators.extend(candidates.iter().flat_map(|candidate| {
            std::iter::once(candidate.source.clone()).chain(candidate.source_locators.clone())
        }));
        return acceptance_unknown(
            format!(
                "explicit acceptance evidence is incomplete or conflicting: {}",
                issues.join("; ")
            ),
            locators,
        );
    }
    if candidates.len() > 1 {
        let states = candidates
            .iter()
            .map(|candidate| candidate.state.as_str())
            .collect::<BTreeSet<_>>();
        let reason = if states.len() > 1 {
            "conflicting explicit accepted and rejected results remain unresolved"
        } else {
            "multiple explicit acceptance decisions have no unambiguous current target"
        };
        return acceptance_unknown(
            reason,
            candidates
                .iter()
                .flat_map(|candidate| {
                    std::iter::once(candidate.source.clone())
                        .chain(candidate.source_locators.clone())
                })
                .collect(),
        );
    }
    let candidate = candidates
        .into_iter()
        .last()
        .expect("non-empty acceptance candidates");
    render_acceptance(&candidate, &[])
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

fn show_acceptance_lane(projection: &Value) -> Value {
    let mut candidates = Vec::new();
    let mut issues = Vec::new();
    let mut source_locators = vec!["full.context".to_owned()];
    let invocation_ids = projection
        .get("work_slot_invocations")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|rows| rows.iter())
        .filter_map(|row| string(row, "invocation_id").map(str::to_owned))
        .collect::<Vec<_>>();
    let run_id = string(projection, "run_id");
    if let Some(context) = projection.get("context").and_then(Value::as_array) {
        for record in context {
            let Some(data) = record.get("data") else {
                continue;
            };
            let record_id = string(record, "id").unwrap_or("unknown-record");
            let source = format!("full.context[{record_id}]");
            let freshness = json!({
                "state": "recorded",
                "sequence": record["sequence"],
                "created_at": record["created_at"],
                "meaning": "durable context timestamp; current applicability remains the provider/driver decision"
            });
            match parse_acceptance_candidate(data, source.clone(), freshness) {
                Ok(Some(candidate)) => {
                    // Only records that carry a decision contribute evidence
                    // locators, so unrelated appends keep the lane stable.
                    if let Err(reason) =
                        target_matches(candidate.target.as_ref(), run_id, &invocation_ids)
                    {
                        issues.push(reason);
                        source_locators.push(source);
                    } else {
                        source_locators.push(source);
                        candidates.push(candidate);
                    }
                }
                Ok(None) => {}
                Err(reason) => {
                    source_locators.push(source);
                    issues.push(reason);
                }
            }
        }
    }
    choose_acceptance(candidates, issues, source_locators)
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
    locations.insert("full.context".to_owned());
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

/// Lanes added to the provider-free `show` packet. They are additive to the
/// existing projection. An accepted-result lane is populated only from an
/// explicit attributed context record; all execution and workflow facts remain
/// non-authoritative for domain-result acceptance.
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
    let mut acceptance = show_acceptance_lane(projection);
    if let Some(object) = acceptance.as_object_mut() {
        object.insert(
            "workflow_transition".to_owned(),
            workflow_transition_acceptance(projection, "full.evaluation_history"),
        );
        object.insert(
            "workflow_transition_meaning".to_owned(),
            json!("workflow allow/deny is not a domain-result acceptance decision"),
        );
        object.insert(
            "sources".to_owned(),
            json!([
                "full.context",
                "full.change_report",
                "full.evaluation_history"
            ]),
        );
    }
    let evidence = show_evidence_lane(projection, view, database);
    let next_action = show_next_action(projection, &execution, &worker);
    let mut reasons =
        vec!["output conformance is not interpreted by provider-free show".to_owned()];
    if is_unknown(&acceptance) {
        reasons.push(
            "semantic acceptance is unknown until an explicit provider/driver result decision is supplied".to_owned(),
        );
    } else if let Some(uncertainty) = acceptance
        .get("uncertainty")
        .and_then(|value| value.get("reasons"))
        .and_then(Value::as_array)
    {
        reasons.extend(
            uncertainty
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
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

fn monitor_acceptance_lane(packet: &Value) -> Value {
    let mut candidates = Vec::new();
    let mut issues = Vec::new();
    let mut source_locators = Vec::new();
    let run_id = string(packet, "source").and_then(|source| source.strip_prefix("run:"));
    let invocation_ids = packet
        .get("helper")
        .and_then(|helper| string(helper, "invocation_id"))
        .map(|id| vec![id.to_owned()])
        .unwrap_or_default();
    if let Some(observations) = packet
        .pointer("/judgment/driver_observations")
        .and_then(Value::as_array)
    {
        for observation in observations {
            let Some(attribution) = observation.get("attribution") else {
                if let Some(reason) = observation.get("reason").and_then(Value::as_str) {
                    issues.push(reason.to_owned());
                }
                continue;
            };
            let Some(source) = observation.get("source").and_then(Value::as_str) else {
                issues.push("driver observation omitted its source locator".to_owned());
                continue;
            };
            let source = format!("observation:{source}");
            source_locators.push(source.clone());
            let freshness = json!({
                "state": "dated",
                "sampled_at_ms": attribution["sampled_at_ms"],
                "meaning": "driver-supplied observation timestamp; current applicability remains external"
            });
            match parse_acceptance_candidate(attribution, source, freshness) {
                Ok(Some(candidate)) => {
                    if let Err(reason) =
                        target_matches(candidate.target.as_ref(), run_id, &invocation_ids)
                    {
                        issues.push(reason);
                    } else {
                        candidates.push(candidate);
                    }
                }
                Ok(None) => {}
                Err(reason) => issues.push(reason),
            }
        }
    }
    choose_acceptance(candidates, issues, source_locators)
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
    if is_unknown(&packet["acceptance"]) {
        return json!({"kind":"help","reason":"execution and declared mechanical checks are visible, but provider/driver acceptance remains unknown","evidence":"acceptance.evidence.source_locators","owner_decision":"decide whether the retained result is acceptable"});
    }
    json!({"kind":"inspect","reason":"an explicit provider/driver result decision is visible; inspect its evidence and choose any workflow action from the authoritative show","evidence":"acceptance.evidence.source_locators","owner_decision":"decide whether to continue, revise, or stop; visibility does not choose"})
}

fn monitor_uncertainty(packet: &Value, execution: &Value, evidence: &Value) -> Value {
    let mut reasons = Vec::new();
    for (name, lane) in [
        ("workflow", &packet["workflow_lane"]),
        ("execution", execution),
        ("worker", &packet["worker"]),
        ("conformance", &packet["conformance"]),
        ("acceptance", &packet["acceptance"]),
        ("evidence", evidence),
    ] {
        if is_unknown(lane) {
            reasons.push(format!("{name} is unknown"));
        }
    }
    if let Some(acceptance_reasons) = packet["acceptance"]
        .pointer("/uncertainty/reasons")
        .and_then(Value::as_array)
    {
        reasons.extend(
            acceptance_reasons
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
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
    packet["acceptance"] = monitor_acceptance_lane(packet);
    let workflow_transition =
        workflow_transition_acceptance(&packet["workflow"], "workflow.latest_evaluations");
    if let Some(object) = packet["acceptance"].as_object_mut() {
        object.insert("workflow_transition".to_owned(), workflow_transition);
        object.insert(
            "workflow_transition_meaning".to_owned(),
            json!("workflow allow/deny is not a domain-result acceptance decision"),
        );
        object.insert(
            "sources".to_owned(),
            json!(["judgment", "workflow", "evidence"]),
        );
    }
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
        ("accepted result", "acceptance"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn show_fixture(context: Value, evaluations: Value) -> Value {
        json!({
            "run_id": "run-1",
            "lifecycle": "active",
            "current_state": "work",
            "current_state_title": "Work",
            "requestable_events": [],
            "context": context,
            "latest_evaluations": evaluations,
            "work_slot_invocations": []
        })
    }

    #[test]
    fn explicit_context_decisions_are_projected_without_using_workflow_allow() {
        let projection = show_fixture(
            json!([{
                "id": "decision-accepted",
                "sequence": 4,
                "created_at": 40,
                "data": {
                    "acceptance": {
                        "state": "accepted",
                        "source_locators": ["capture:/tmp/accepted/stdout"],
                        "attesting_driver": "driver-1"
                    }
                }
            }]),
            json!([{
                "result": {"result": "allow"},
                "transition": {"event": "finish"}
            }]),
        );
        let lanes = show_lanes(&projection, "full", Path::new("/tmp/run.sqlite"));
        assert_eq!(lanes["acceptance"]["state"], "accepted");
        assert_eq!(lanes["acceptance"]["attributed_to"], "driver-1");
        assert_eq!(
            lanes["acceptance"]["evidence"]["source_locators"],
            json!([
                "capture:/tmp/accepted/stdout",
                "full.context[decision-accepted]"
            ])
        );
        assert_eq!(
            lanes["acceptance"]["workflow_transition"]["state"],
            "accepted"
        );
        assert!(lanes["acceptance"]["meaning"]
            .as_str()
            .unwrap()
            .contains("not inferred"));
    }

    #[test]
    fn explicit_rejection_is_distinct_and_missing_or_conflicting_sources_stay_unknown() {
        let rejected = show_fixture(
            json!([{
                "id": "decision-rejected",
                "sequence": 5,
                "created_at": 50,
                "data": {
                    "acceptance": {
                        "state": "rejected",
                        "evidence": ["full.context[review-1]"],
                        "provider": "fixture-provider"
                    }
                }
            }]),
            json!([]),
        );
        assert_eq!(show_acceptance_lane(&rejected)["state"], "rejected");

        let arbitrary_provider_result = show_fixture(
            json!([{
                "id": "review-1",
                "sequence": 1,
                "created_at": 10,
                "data": {"result": "pass", "findings": ""}
            }]),
            json!([]),
        );
        assert_eq!(
            show_acceptance_lane(&arbitrary_provider_result)["state"],
            "unknown"
        );

        let stale = show_fixture(
            json!([{
                "id": "decision-stale",
                "sequence": 2,
                "created_at": 20,
                "data": {
                    "acceptance": {
                        "state": "accepted",
                        "source_locators": ["capture:/tmp/stale"],
                        "attesting_driver": "driver",
                        "freshness": {"state": "stale"}
                    }
                }
            }]),
            json!([]),
        );
        assert_eq!(show_acceptance_lane(&stale)["state"], "unknown");

        let conflicting = show_fixture(
            json!([
                {
                    "id": "decision-1",
                    "sequence": 1,
                    "created_at": 10,
                    "data": {
                        "acceptance": {
                            "state": "accepted",
                            "source_locators": ["capture:/tmp/one"],
                            "attesting_driver": "driver"
                        }
                    }
                },
                {
                    "id": "decision-2",
                    "sequence": 2,
                    "created_at": 20,
                    "data": {
                        "acceptance": {
                            "state": "rejected",
                            "source_locators": ["capture:/tmp/two"],
                            "attesting_driver": "driver"
                        }
                    }
                }
            ]),
            json!([]),
        );
        let lane = show_acceptance_lane(&conflicting);
        assert_eq!(lane["state"], "unknown");
        assert!(lane["reason"].as_str().unwrap().contains("conflicting"));
    }

    #[test]
    fn monitor_acceptance_requires_matching_attributed_observation() {
        let mut packet = json!({
            "source": "run-1",
            "helper": {"invocation_id": "inv-1"},
            "judgment": {"driver_observations": [{
                "source": "/tmp/observation.json",
                "attribution": {
                    "run_id": "run-1",
                    "invocation_id": "inv-1",
                    "sampled_at_ms": 123,
                    "attesting_driver": "driver-1",
                    "judgment": {
                        "result": "accepted",
                        "source_locators": ["capture:/tmp/capture/0/stdout"]
                    }
                }
            }]}
        });
        let lane = monitor_acceptance_lane(&packet);
        assert_eq!(lane["state"], "accepted");
        enrich_monitor_packet(&mut packet);
        assert_eq!(packet["acceptance"]["state"], "accepted");

        packet["judgment"]["driver_observations"][0]["attribution"]["invocation_id"] =
            json!("other-invocation");
        let lane = monitor_acceptance_lane(&packet);
        assert_eq!(lane["state"], "unknown");
        assert!(lane["reason"]
            .as_str()
            .unwrap()
            .contains("selected invocation"));
    }

    #[test]
    fn owner_update_names_change_and_decision_but_suppresses_heartbeat() {
        let current = json!({
            "acceptance": {"state": "accepted"},
            "next_action": {"kind": "inspect", "reason": "read evidence"}
        });
        let first = owner_update(None, &current);
        assert_eq!(first["required_before_next_decision"], true);
        assert!(first["observed_change"]
            .as_str()
            .unwrap()
            .contains("initial"));
        assert_eq!(first["needed_action_or_decision"]["kind"], "inspect");

        let repeat = owner_update(Some(&current), &current);
        assert_eq!(repeat["required_before_next_decision"], false);
        assert_eq!(
            repeat["observed_change"],
            "no meaningful change since the last observation"
        );
        assert_eq!(repeat["machine_notification_is_not_owner_update"], true);
    }
}
