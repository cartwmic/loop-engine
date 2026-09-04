//! Evaluate-time bookends overlay.
//!
//! Shipped profile JSON stays free of bookends keys. When a frozen per-run
//! copy sets `extra.bookends.enabled` to JSON `true`, evaluate injects the
//! PRD-traceability disposition schema and extra review axes. Extra is
//! otherwise opaque.

#![allow(dead_code)]

use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const REQUIREMENT_ID_PATTERN: &str = r"^LE-[1-9][0-9]*$";
const PRD_TRACEABILITY_FIELD: &str = "prd_traceability";
const LINKED_LIVE: &str = "linked-live";
const CANDIDATE: &str = "candidate";
const NOT_APPLICABLE: &str = "not-applicable";
const PRD_TRACEABILITY_TYPES: &[&str] = &[LINKED_LIVE, CANDIDATE, NOT_APPLICABLE];

const IDS_GROUNDED_ID: &str = "ids-grounded";
const IDS_GROUNDED_DESCRIPTION: &str = "Linked-live disposition IDs are live PRD IDs relevant to this change. Do not re-judge bookends checker red/green.";
const IDS_GROUNDED_PROMPT: &str = "Judge ids-grounded only. Confirm every linked-live disposition ID is a live PRD ID relevant to this change. Do not re-judge bookends checker red/green.";

const BYPASS_NOT_GREEN_ID: &str = "bypass-not-green";
const BYPASS_NOT_GREEN_DESCRIPTION: &str = "The validation report does not present an in-process bookends Red or other non-Green result as a green check or as validation passed. A repository pre-push or CI bypass is not a green check.";
const BYPASS_NOT_GREEN_PROMPT: &str = "Judge bypass-not-green only. Confirm the validation report does not present an in-process bookends Red or other non-Green result as a green check or as validation passed. A repository pre-push or CI bypass is not a green check.";

const VALIDATION_GATES: &[&str] = &["validation-review", "validation-adversarial-review"];

/// One extra instance violation emitted after schema evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverlayViolation {
    pub(crate) path: String,
    pub(crate) rule: String,
    pub(crate) message: String,
}

/// `extra.bookends.enabled` is JSON `true` on the frozen initial_input.
pub(crate) fn enabled(initial_input: &Value) -> bool {
    initial_input
        .get("extra")
        .and_then(Value::as_object)
        .and_then(|extra| extra.get("bookends"))
        .and_then(Value::as_object)
        .and_then(|bookends| bookends.get("enabled"))
        == Some(&Value::Bool(true))
}

/// Clone `initial_input` and inject overlay schema fields and review axes.
pub(crate) fn apply(initial_input: &Value) -> Value {
    let mut overlayed = initial_input.clone();
    let schemas = overlayed
        .as_object_mut()
        .expect("provider configuration is validated as an object")
        .entry("artifact_schemas")
        .or_insert_with(|| json!({}));
    if let Some(schemas) = schemas.as_object_mut() {
        inject_prd_traceability(schemas);
    }
    if let Some(policies) = overlayed
        .get_mut("review_policies")
        .and_then(Value::as_object_mut)
    {
        inject_axes(policies);
    }
    overlayed
}

/// `^LE-[1-9][0-9]*$`
pub(crate) fn is_requirement_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("LE-") else {
        return false;
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) if first.is_ascii_digit() && first != '0' => chars.all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// Validate the one PRD-traceability disposition attached to every current
/// intent criterion when the overlay is enabled.
pub(crate) fn intent_overlay_violations(
    intent: &Value,
    live_ids: &[String],
) -> Vec<OverlayViolation> {
    let Some(criteria) = intent.get("acceptance").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for (index, criterion) in criteria.iter().enumerate() {
        let path = format!("/acceptance/{index}/{PRD_TRACEABILITY_FIELD}");
        let Some(criterion) = criterion.as_object() else {
            violations.push(OverlayViolation {
                path: format!("/acceptance/{index}"),
                rule: "type".to_owned(),
                message: "current criterion must be an object".to_owned(),
            });
            continue;
        };
        let Some(disposition) = criterion.get(PRD_TRACEABILITY_FIELD) else {
            violations.push(OverlayViolation {
                path,
                rule: "prd-traceability".to_owned(),
                message: "each current criterion requires exactly one PRD traceability disposition"
                    .to_owned(),
            });
            continue;
        };
        let Some(disposition) = disposition.as_object() else {
            violations.push(OverlayViolation {
                path,
                rule: "prd-traceability".to_owned(),
                message: "PRD traceability disposition must be an object".to_owned(),
            });
            continue;
        };

        let Some(kind) = disposition.get("type").and_then(Value::as_str) else {
            violations.push(OverlayViolation {
                path,
                rule: "prd-traceability".to_owned(),
                message: "PRD traceability disposition requires a string `type`".to_owned(),
            });
            continue;
        };

        match kind {
            LINKED_LIVE => {
                require_fields(
                    disposition,
                    &path,
                    &["type", "live_ids"],
                    &["live_ids"],
                    &mut violations,
                );
                if let Some(ids) = disposition.get("live_ids") {
                    validate_linked_live_ids(
                        ids,
                        &format!("{path}/live_ids"),
                        live_ids,
                        &mut violations,
                    );
                }
            }
            CANDIDATE => {
                require_fields(
                    disposition,
                    &path,
                    &["type", "proposed_id", "record_markdown"],
                    &["proposed_id", "record_markdown"],
                    &mut violations,
                );
                validate_candidate_disposition(disposition, &path, live_ids, &mut violations);
            }
            NOT_APPLICABLE => {
                require_fields(
                    disposition,
                    &path,
                    &["type", "reason"],
                    &["reason"],
                    &mut violations,
                );
                match disposition.get("reason") {
                    Some(Value::String(reason)) if !reason.is_empty() => {}
                    Some(Value::String(_)) => violations.push(OverlayViolation {
                        path: format!("{path}/reason"),
                        rule: "minLength".to_owned(),
                        message: "not-applicable reason must not be empty".to_owned(),
                    }),
                    Some(_) => violations.push(OverlayViolation {
                        path: format!("{path}/reason"),
                        rule: "type".to_owned(),
                        message: "not-applicable reason must be a string".to_owned(),
                    }),
                    None => {}
                }
            }
            other => violations.push(OverlayViolation {
                path: format!("{path}/type"),
                rule: "enum".to_owned(),
                message: format!("unknown PRD traceability disposition `{other}`"),
            }),
        }
    }
    sort_overlay_violations(&mut violations);
    violations
}

/// Whether the current intent contains an active provisional candidate.
pub(crate) fn has_candidate(intent: &Value) -> bool {
    intent
        .get("acceptance")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|criterion| criterion.get(PRD_TRACEABILITY_FIELD))
        .any(|disposition| disposition.get("type").and_then(Value::as_str) == Some(CANDIDATE))
}

fn require_fields(
    object: &Map<String, Value>,
    path: &str,
    allowed: &[&str],
    required: &[&str],
    violations: &mut Vec<OverlayViolation>,
) {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            violations.push(OverlayViolation {
                path: format!("{path}/{key}"),
                rule: "additionalProperties".to_owned(),
                message: format!("property `{key}` is not allowed for this disposition"),
            });
        }
    }
    for key in required {
        if !object.contains_key(*key) {
            violations.push(OverlayViolation {
                path: path.to_owned(),
                rule: "required".to_owned(),
                message: format!("disposition requires `{key}`"),
            });
        }
    }
}

fn validate_linked_live_ids(
    value: &Value,
    path: &str,
    live_ids: &[String],
    violations: &mut Vec<OverlayViolation>,
) {
    let Some(ids) = value.as_array() else {
        violations.push(OverlayViolation {
            path: path.to_owned(),
            rule: "type".to_owned(),
            message: "linked-live `live_ids` must be an array".to_owned(),
        });
        return;
    };
    if ids.is_empty() {
        violations.push(OverlayViolation {
            path: path.to_owned(),
            rule: "minItems".to_owned(),
            message: "linked-live `live_ids` must contain at least one ID".to_owned(),
        });
    }
    let live = live_ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for (index, value) in ids.iter().enumerate() {
        let path = format!("{path}/{index}");
        let Some(id) = value.as_str() else {
            violations.push(OverlayViolation {
                path,
                rule: "type".to_owned(),
                message: "linked-live ID must be a string".to_owned(),
            });
            continue;
        };
        if !is_requirement_id(id) {
            violations.push(OverlayViolation {
                path: path.clone(),
                rule: "pattern".to_owned(),
                message: format!("string does not match `{REQUIREMENT_ID_PATTERN}`"),
            });
            continue;
        }
        if !seen.insert(id.to_owned()) {
            violations.push(OverlayViolation {
                path: path.clone(),
                rule: "requirement-ids-duplicate".to_owned(),
                message: format!("duplicate linked live requirement ID `{id}`"),
            });
        }
        if !live.contains(id) {
            violations.push(OverlayViolation {
                path,
                rule: "requirement-ids-live".to_owned(),
                message: format!("requirement ID `{id}` is not a live non-tombstoned PRD ID"),
            });
        }
    }
}

fn validate_candidate_disposition(
    disposition: &Map<String, Value>,
    path: &str,
    live_ids: &[String],
    violations: &mut Vec<OverlayViolation>,
) {
    let proposed = disposition.get("proposed_id").and_then(Value::as_str);
    if let Some(id) = proposed {
        if !is_requirement_id(id) {
            violations.push(OverlayViolation {
                path: format!("{path}/proposed_id"),
                rule: "pattern".to_owned(),
                message: format!("string does not match `{REQUIREMENT_ID_PATTERN}`"),
            });
        } else if live_ids.iter().any(|live| live == id) {
            violations.push(OverlayViolation {
                path: format!("{path}/proposed_id"),
                rule: "candidate-live-id".to_owned(),
                message: format!("candidate requirement ID `{id}` is already live"),
            });
        }
    }

    let Some(markdown) = disposition.get("record_markdown").and_then(Value::as_str) else {
        return;
    };
    let parsed = match bookends_check::candidate_ids(markdown) {
        Ok(ids) => ids,
        Err(errors) => {
            violations.push(OverlayViolation {
                path: format!("{path}/record_markdown"),
                rule: "candidate-parser".to_owned(),
                message: errors.join("; "),
            });
            return;
        }
    };
    if parsed.len() != 1 {
        violations.push(OverlayViolation {
            path: format!("{path}/record_markdown"),
            rule: "candidate-parser".to_owned(),
            message: format!(
                "candidate must contain exactly one parsed requirement record, found {}",
                parsed.len()
            ),
        });
        return;
    }
    let parsed_id = &parsed[0];
    if proposed != Some(parsed_id.as_str()) {
        violations.push(OverlayViolation {
            path: format!("{path}/proposed_id"),
            rule: "candidate-id-binding".to_owned(),
            message: format!("proposed_id must equal parser-extracted candidate ID `{parsed_id}`"),
        });
    }
}

fn sort_overlay_violations(violations: &mut [OverlayViolation]) {
    violations.sort_by(|left, right| {
        (&left.path, &left.rule, &left.message).cmp(&(&right.path, &right.rule, &right.message))
    });
}

fn inject_prd_traceability(schemas: &mut Map<String, Value>) {
    let Some(schema) = schemas.get_mut("intent.json") else {
        return;
    };
    let Some(schema) = schema.as_object_mut() else {
        return;
    };
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(acceptance) = properties
        .get_mut("acceptance")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(item) = acceptance.get_mut("items").and_then(Value::as_object_mut) else {
        return;
    };

    let item_properties = item.entry("properties").or_insert_with(|| json!({}));
    let Some(item_properties) = item_properties.as_object_mut() else {
        return;
    };
    item_properties.insert(PRD_TRACEABILITY_FIELD.to_owned(), prd_traceability_schema());

    let required = item.entry("required").or_insert_with(|| json!([]));
    if let Some(required) = required.as_array_mut() {
        if !required
            .iter()
            .any(|entry| entry.as_str() == Some(PRD_TRACEABILITY_FIELD))
        {
            required.push(json!(PRD_TRACEABILITY_FIELD));
        }
    }
}

fn prd_traceability_schema() -> Value {
    json!({
        "type": "object",
        "required": ["type"],
        "properties": {
            "type": {
                "type": "string",
                "enum": PRD_TRACEABILITY_TYPES
            },
            "live_ids": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "string",
                    "pattern": REQUIREMENT_ID_PATTERN
                }
            },
            "proposed_id": {
                "type": "string",
                "pattern": REQUIREMENT_ID_PATTERN
            },
            "record_markdown": {
                "type": "string",
                "minLength": 1
            },
            "reason": {
                "type": "string",
                "minLength": 1
            }
        },
        "additionalProperties": false
    })
}

fn inject_axes(policies: &mut Map<String, Value>) {
    let gates: Vec<String> = policies.keys().cloned().collect();
    for gate in gates {
        let Some(axes) = policies.get_mut(&gate).and_then(Value::as_array_mut) else {
            continue;
        };
        if axes.is_empty() {
            continue;
        }
        push_axis_if_absent(
            axes,
            IDS_GROUNDED_ID,
            IDS_GROUNDED_DESCRIPTION,
            IDS_GROUNDED_PROMPT,
        );
        if VALIDATION_GATES.contains(&gate.as_str()) {
            push_axis_if_absent(
                axes,
                BYPASS_NOT_GREEN_ID,
                BYPASS_NOT_GREEN_DESCRIPTION,
                BYPASS_NOT_GREEN_PROMPT,
            );
        }
    }
}

fn push_axis_if_absent(axes: &mut Vec<Value>, id: &str, description: &str, example_prompt: &str) {
    let present = axes
        .iter()
        .any(|axis| axis.get("id").and_then(Value::as_str) == Some(id));
    if present {
        return;
    }
    axes.push(json!({
        "id": id,
        "description": description,
        "example_prompt": example_prompt
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema_required(schema: &Value) -> Vec<&str> {
        schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn axis_ids<'a>(policies: &'a Value, gate: &str) -> Vec<&'a str> {
        policies[gate]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|axis| axis.get("id").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn enabled_requires_json_true() {
        assert!(!enabled(&json!({})));
        assert!(!enabled(
            &json!({"extra": {"bookends": {"enabled": false}}})
        ));
        assert!(!enabled(
            &json!({"extra": {"bookends": {"enabled": "true"}}})
        ));
        assert!(!enabled(&json!({"extra": {"bookends": {"enabled": 1}}})));
        assert!(!enabled(&json!({"extra": {"profile": "high-rigor"}})));
        assert!(enabled(
            &json!({"extra": {"bookends": {"enabled": true}, "profile": "minimal"}})
        ));
    }

    #[test]
    fn apply_injects_closed_prd_traceability_into_intent_criteria_only() {
        let input = json!({
            "artifact_schemas": {
                "intent.json": {
                    "type": "object",
                    "properties": {
                        "acceptance": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["id", "statement"],
                                "properties": {
                                    "id": {"type": "string"},
                                    "statement": {"type": "string"}
                                },
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["acceptance"]
                },
                "design.json": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            "review_policies": {}
        });
        let overlayed = apply(&input);
        let item =
            &overlayed["artifact_schemas"]["intent.json"]["properties"]["acceptance"]["items"];
        assert!(schema_required(item).contains(&PRD_TRACEABILITY_FIELD));
        let disposition = &item["properties"][PRD_TRACEABILITY_FIELD];
        assert_eq!(disposition["type"], "object");
        assert_eq!(disposition["additionalProperties"], false);
        assert_eq!(
            disposition["properties"]["type"]["enum"],
            json!(PRD_TRACEABILITY_TYPES)
        );
        assert_eq!(
            disposition["properties"]["live_ids"]["items"]["pattern"],
            REQUIREMENT_ID_PATTERN
        );
        assert!(overlayed["artifact_schemas"]["design.json"]["properties"]
            .get(PRD_TRACEABILITY_FIELD)
            .is_none());
        assert!(
            input["artifact_schemas"]["intent.json"]["properties"]["acceptance"]["items"]
                ["properties"]
                .get(PRD_TRACEABILITY_FIELD)
                .is_none()
        );
    }

    #[test]
    fn apply_injects_ids_grounded_on_existing_gates_and_bypass_only_on_validation() {
        let input = json!({
            "review_policies": {
                "intent-review": [{"id": "solution-agnostic", "description": "d"}],
                "design-review": [{"id": "intent-faithful", "description": "d"}],
                "plan-review": [],
                "implementation-review": [{"id": "tasks-actually-done", "description": "d"}],
                "validation-review": [{"id": "intent-delivered", "description": "d"}],
                "validation-adversarial-review": [{"id": "intent-delivered", "description": "d"}]
            }
        });
        let overlayed = apply(&input);
        let policies = &overlayed["review_policies"];
        assert!(axis_ids(policies, "intent-review").contains(&"ids-grounded"));
        assert!(!axis_ids(policies, "intent-review").contains(&"bypass-not-green"));
        assert!(axis_ids(policies, "design-review").contains(&"ids-grounded"));
        assert!(axis_ids(policies, "plan-review").is_empty());
        assert!(axis_ids(policies, "implementation-review").contains(&"ids-grounded"));
        assert!(!axis_ids(policies, "implementation-review").contains(&"bypass-not-green"));
        let validation = axis_ids(policies, "validation-review");
        assert!(validation.contains(&"ids-grounded"));
        assert!(validation.contains(&"bypass-not-green"));
        let adversarial = axis_ids(policies, "validation-adversarial-review");
        assert!(adversarial.contains(&"ids-grounded"));
        assert!(adversarial.contains(&"bypass-not-green"));
    }

    #[test]
    fn overlay_dispositions_accept_linked_candidate_and_not_applicable() {
        let intent = json!({
            "acceptance": [
                {
                    "id": "AC-1",
                    "statement": "The live requirement is delivered.",
                    "prd_traceability": {
                        "type": "linked-live",
                        "live_ids": ["LE-1"]
                    }
                },
                {
                    "id": "AC-2",
                    "statement": "The new requirement is proposed.",
                    "prd_traceability": {
                        "type": "candidate",
                        "proposed_id": "LE-9",
                        "record_markdown": "### LE-9: Proposed requirement\n- Status: live\n- Coverage: e2e/journey\n"
                    }
                },
                {
                    "id": "AC-3",
                    "statement": "The repository-only condition remains binding.",
                    "prd_traceability": {
                        "type": "not-applicable",
                        "reason": "Repository-only condition, not an enduring product requirement."
                    }
                }
            ]
        });
        assert!(intent_overlay_violations(&intent, &["LE-1".to_owned()]).is_empty());
        assert!(has_candidate(&intent));
    }

    #[test]
    fn candidate_binding_and_live_collision_are_mechanical() {
        let mismatch = json!({
            "acceptance": [{
                "id": "AC-1",
                "statement": "candidate",
                "prd_traceability": {
                    "type": "candidate",
                    "proposed_id": "LE-8",
                    "record_markdown": "### LE-9: Proposed\n- Status: live\n- Coverage: e2e/journey\n"
                }
            }]
        });
        let mismatch_rules = intent_overlay_violations(&mismatch, &[])
            .into_iter()
            .map(|violation| violation.rule)
            .collect::<Vec<_>>();
        assert!(mismatch_rules.contains(&"candidate-id-binding".to_owned()));

        let collision = json!({
            "acceptance": [{
                "id": "AC-1",
                "statement": "candidate",
                "prd_traceability": {
                    "type": "candidate",
                    "proposed_id": "LE-1",
                    "record_markdown": "### LE-1: Existing\n- Status: live\n- Coverage: e2e/journey\n"
                }
            }]
        });
        let collision_rules = intent_overlay_violations(&collision, &["LE-1".to_owned()])
            .into_iter()
            .map(|violation| violation.rule)
            .collect::<Vec<_>>();
        assert!(collision_rules.contains(&"candidate-live-id".to_owned()));

        let multiple = json!({
            "acceptance": [{
                "id": "AC-1",
                "statement": "candidate",
                "prd_traceability": {
                    "type": "candidate",
                    "proposed_id": "LE-8",
                    "record_markdown": "### LE-8: One\n- Status: live\n- Coverage: e2e/journey\n\n### LE-9: Two\n- Status: live\n- Coverage: e2e/journey\n"
                }
            }]
        });
        let multiple_rules = intent_overlay_violations(&multiple, &[])
            .into_iter()
            .map(|violation| violation.rule)
            .collect::<Vec<_>>();
        assert!(multiple_rules.contains(&"candidate-parser".to_owned()));
    }

    #[test]
    fn not_applicable_does_not_look_like_a_candidate() {
        let intent = json!({
            "acceptance": [{
                "id": "AC-1",
                "statement": "A binding criterion.",
                "prd_traceability": {
                    "type": "not-applicable",
                    "reason": "Only a change-specific validation condition."
                }
            }]
        });
        assert!(intent_overlay_violations(&intent, &[]).is_empty());
        assert!(!has_candidate(&intent));
        let candidate = json!({
            "acceptance": [{
                "id": "AC-1",
                "statement": "A binding criterion.",
                "prd_traceability": {
                    "type": "candidate",
                    "proposed_id": "LE-8",
                    "record_markdown": "### LE-8: Proposed\n- Status: live\n- Coverage: e2e/journey\n"
                }
            }]
        });
        assert!(has_candidate(&candidate));
    }

    #[test]
    fn requirement_id_pattern_matches_schema_token() {
        assert!(is_requirement_id("LE-1"));
        assert!(is_requirement_id("LE-99"));
        assert!(!is_requirement_id("LE-0"));
        assert!(!is_requirement_id("LE-01"));
        assert!(!is_requirement_id("LE-"));
        assert!(!is_requirement_id("le-1"));
        assert!(!is_requirement_id("SPEC-1"));
    }

    #[test]
    fn linked_live_disposition_reuses_live_id_checks() {
        let empty = json!({
            "acceptance": [{
                "prd_traceability": {"type": "linked-live", "live_ids": []}
            }]
        });
        assert!(intent_overlay_violations(&empty, &["LE-1".to_owned()])
            .iter()
            .any(|violation| violation.rule == "minItems"));

        let tombstoned = json!({
            "acceptance": [{
                "prd_traceability": {"type": "linked-live", "live_ids": ["LE-2"]}
            }]
        });
        assert!(intent_overlay_violations(&tombstoned, &["LE-1".to_owned()])
            .iter()
            .any(|violation| violation.rule == "requirement-ids-live"));

        let live = json!({
            "acceptance": [{
                "prd_traceability": {"type": "linked-live", "live_ids": ["LE-1"]}
            }]
        });
        assert!(intent_overlay_violations(&live, &["LE-1".to_owned()]).is_empty());
    }
}
