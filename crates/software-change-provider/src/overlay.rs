//! Evaluate-time bookends overlay.
//!
//! Shipped profile JSON stays free of bookends keys. When a frozen per-run
//! copy sets `extra.bookends.enabled` to JSON `true`, evaluate injects
//! `requirement_ids` into selected schemas and extra review axes. Extra is
//! otherwise opaque.

#![allow(dead_code)]

use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const OVERLAY_SUBJECTS: &[&str] = &[
    "intent.json",
    "design.json",
    "plan.json",
    "validation-report.json",
];
const REQUIREMENT_ID_PATTERN: &str = r"^LE-[1-9][0-9]*$";

const IDS_GROUNDED_ID: &str = "ids-grounded";
const IDS_GROUNDED_DESCRIPTION: &str = "Cited requirement_ids are live PRD IDs relevant to this change. Do not re-judge bookends checker red/green.";
const IDS_GROUNDED_PROMPT: &str = "Judge ids-grounded only. Confirm every cited requirement_ids value is a live PRD ID relevant to this change. Do not re-judge bookends checker red/green.";

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
        inject_requirement_ids(schemas);
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

/// Extra schema denials for overlay-on `requirement_ids` values.
///
/// Missing arrays and empty arrays are left to the injected schema
/// (`required` / `minItems`). Missing or tombstoned IDs are not bypassable.
pub(crate) fn requirement_id_violations(
    instance: &Value,
    live_ids: &[String],
) -> Vec<OverlayViolation> {
    let Some(ids) = instance.get("requirement_ids").and_then(Value::as_array) else {
        return Vec::new();
    };
    let live: BTreeSet<&str> = live_ids.iter().map(String::as_str).collect();
    let mut violations = Vec::new();
    for (index, value) in ids.iter().enumerate() {
        let Some(id) = value.as_str() else {
            continue;
        };
        let path = format!("/requirement_ids/{index}");
        if !is_requirement_id(id) {
            violations.push(OverlayViolation {
                path: path.clone(),
                rule: "pattern".to_owned(),
                message: format!("string does not match `{REQUIREMENT_ID_PATTERN}`"),
            });
            continue;
        }
        if !live.contains(id) {
            violations.push(OverlayViolation {
                path,
                rule: "requirement-ids-live".to_owned(),
                message: format!("requirement ID `{id}` is not a live non-tombstoned PRD ID"),
            });
        }
    }
    violations
}

fn inject_requirement_ids(schemas: &mut Map<String, Value>) {
    for subject in OVERLAY_SUBJECTS {
        let schema = schemas
            .entry((*subject).to_owned())
            .or_insert_with(minimal_requirement_schema);
        let Some(object) = schema.as_object_mut() else {
            continue;
        };
        let properties = object.entry("properties").or_insert_with(|| json!({}));
        if let Some(properties) = properties.as_object_mut() {
            // The overlay owns this field even when a caller's per-run copy
            // already contains a stale or weaker declaration.
            properties.insert("requirement_ids".to_owned(), requirement_ids_schema());
        }
        let required = object.entry("required").or_insert_with(|| json!([]));
        if let Some(required) = required.as_array_mut() {
            let already = required
                .iter()
                .any(|entry| entry.as_str() == Some("requirement_ids"));
            if !already {
                required.push(json!("requirement_ids"));
            }
        }
    }
}

fn minimal_requirement_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": []
    })
}

fn requirement_ids_schema() -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "items": {
            "type": "string",
            "pattern": REQUIREMENT_ID_PATTERN
        }
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
    fn apply_injects_requirement_ids_into_four_schemas_only() {
        let input = json!({
            "artifact_schemas": {
                "intent.json": {
                    "type": "object",
                    "properties": {"revision": {"type": "string"}},
                    "required": ["revision"],
                    "additionalProperties": false
                },
                "design.json": {
                    "type": "object",
                    "properties": {"revision": {"type": "string"}},
                    "required": ["revision"]
                },
                "plan.json": {
                    "type": "object",
                    "properties": {"revision": {"type": "string"}},
                    "required": ["revision"]
                },
                "implementation-report.json": {
                    "type": "object",
                    "properties": {"revision": {"type": "string"}},
                    "required": ["revision"]
                },
                "validation-report.json": {
                    "type": "object",
                    "properties": {"revision": {"type": "string"}},
                    "required": ["revision"]
                }
            },
            "review_policies": {}
        });
        let overlayed = apply(&input);
        for subject in OVERLAY_SUBJECTS {
            let schema = &overlayed["artifact_schemas"][subject];
            assert!(
                schema_required(schema).contains(&"requirement_ids"),
                "{subject} required"
            );
            assert_eq!(schema["properties"]["requirement_ids"]["type"], "array");
            assert_eq!(schema["properties"]["requirement_ids"]["minItems"], 1);
            assert_eq!(
                schema["properties"]["requirement_ids"]["items"]["type"],
                "string"
            );
            assert_eq!(
                schema["properties"]["requirement_ids"]["items"]["pattern"],
                REQUIREMENT_ID_PATTERN
            );
        }
        assert!(
            overlayed["artifact_schemas"]["implementation-report.json"]["properties"]
                .get("requirement_ids")
                .is_none()
        );
        assert!(
            !schema_required(&overlayed["artifact_schemas"]["implementation-report.json"])
                .contains(&"requirement_ids")
        );
        assert_eq!(
            input["artifact_schemas"]["intent.json"]["required"],
            json!(["revision"])
        );
    }

    #[test]
    fn apply_adds_id_only_schemas_when_a_custom_profile_omits_them() {
        let input = json!({"review_policies": {}});
        let overlayed = apply(&input);
        for subject in OVERLAY_SUBJECTS {
            let schema = &overlayed["artifact_schemas"][subject];
            assert_eq!(schema["type"], "object");
            assert!(schema_required(schema).contains(&"requirement_ids"));
            assert_eq!(schema["properties"]["requirement_ids"]["minItems"], 1);
        }
        assert!(input.get("artifact_schemas").is_none());
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
    fn empty_and_tombstoned_ids_are_denied() {
        let live = vec!["LE-1".to_owned()];
        assert!(requirement_id_violations(&json!({}), &live).is_empty());
        assert!(requirement_id_violations(&json!({"requirement_ids": []}), &live).is_empty());
        let tombstoned = requirement_id_violations(&json!({"requirement_ids": ["LE-2"]}), &live);
        assert_eq!(tombstoned.len(), 1);
        assert_eq!(tombstoned[0].rule, "requirement-ids-live");
        let malformed = requirement_id_violations(&json!({"requirement_ids": ["nope"]}), &live);
        assert_eq!(malformed[0].rule, "pattern");
        assert!(requirement_id_violations(&json!({"requirement_ids": ["LE-1"]}), &live).is_empty());
    }
}
