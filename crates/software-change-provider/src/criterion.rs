//! Deterministic AC-N identity and reference checks for software-change artifacts.
//!
//! The schema validator owns types and the AC-N token grammar.  This module
//! owns the small cross-item rules that a bounded instance schema cannot
//! express: intent IDs are unique, and present downstream references are
//! unique within their local collection and members of the current intent.

#![allow(dead_code)]

use crate::schema::ValidatedSchema;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) const CRITERION_ID_PATTERN: &str = r"^AC-[1-9][0-9]*$";

const DESIGN_SUBJECT: &str = "design.json";
const PLAN_SUBJECT: &str = "plan.json";
const IMPLEMENTATION_SUBJECT: &str = "implementation-report.json";
const VALIDATION_SUBJECT: &str = "validation-report.json";

/// One deterministic criterion identity or reference violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CriterionViolation {
    pub(crate) path: String,
    pub(crate) rule: String,
    pub(crate) message: String,
}

impl CriterionViolation {
    fn new(path: impl Into<String>, rule: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            rule: rule.into(),
            message: message.into(),
        }
    }
}

/// Mechanically resolve reviewer-supplied evidence citations against exact source bytes.
/// This verifies references only; it does not decide whether a source supports a judgment.
pub(crate) fn validate_grounding(grounds: &Value, artifact_root: &Path) -> Result<(), String> {
    let reason = grounds
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .ok_or("grounds.reason must be a non-empty reviewer rationale")?;
    if reason.len() > 1200 {
        return Err("grounds.reason exceeds 1200 bytes".to_owned());
    }
    let evidence = grounds
        .get("evidence")
        .and_then(Value::as_array)
        .filter(|evidence| !evidence.is_empty() && evidence.len() <= 8)
        .ok_or("grounds.evidence must contain 1 to 8 source references")?;
    let root = fs::canonicalize(artifact_root).map_err(|error| error.to_string())?;
    let cwd = fs::canonicalize(std::env::current_dir().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    for reference in evidence {
        let locator = reference
            .get("locator")
            .and_then(Value::as_str)
            .filter(|locator| !locator.trim().is_empty())
            .ok_or("evidence locator must be a non-empty repository-relative path and selector")?;
        let digest = reference
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|digest| digest.starts_with("sha256:") && digest.len() == 71)
            .ok_or("evidence sha256 must be `sha256:` plus 64 lowercase hex digits")?;
        if !digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !seen.insert(locator)
        {
            return Err("evidence digest is malformed or locator is duplicated".to_owned());
        }
        let (file, selector) = locator.split_once('#').ok_or_else(|| {
            format!("evidence locator `{locator}` needs a JSON Pointer or line selector")
        })?;
        let relative = Path::new(file);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "evidence locator `{locator}` must stay beneath a source root"
            ));
        }
        let path = resolve_evidence_file(&root, &cwd, relative)?;
        let bytes = fs::read(&path)
            .map_err(|error| format!("evidence source `{locator}` is unreadable: {error}"))?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != digest {
            return Err(format!(
                "evidence digest does not match exact source bytes for `{file}`"
            ));
        }
        if selector.starts_with('/') {
            let json: Value = serde_json::from_slice(&bytes)
                .map_err(|_| format!("evidence selector `{locator}` requires a JSON source"))?;
            if json.pointer(selector).is_none() {
                return Err(format!(
                    "evidence JSON Pointer in `{locator}` does not resolve"
                ));
            }
        } else {
            validate_line_selector(selector, &bytes, locator)?;
        }
    }
    Ok(())
}

/// Check retained citation structure without rebinding a historical digest to
/// a changed current file. Current-source citations use `validate_grounding`.
pub(crate) fn validate_grounding_shape(grounds: &Value) -> Result<(), String> {
    let reason = grounds
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .ok_or("grounds.reason must be a non-empty reviewer rationale")?;
    if reason.len() > 1200 {
        return Err("grounds.reason exceeds 1200 bytes".to_owned());
    }
    let evidence = grounds
        .get("evidence")
        .and_then(Value::as_array)
        .filter(|evidence| !evidence.is_empty() && evidence.len() <= 8)
        .ok_or("grounds.evidence must contain 1 to 8 source references")?;
    let mut seen = BTreeSet::new();
    for reference in evidence {
        let locator = reference
            .get("locator")
            .and_then(Value::as_str)
            .filter(|locator| !locator.trim().is_empty())
            .ok_or("evidence locator must be a non-empty repository-relative path and selector")?;
        let digest = reference
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|digest| digest.starts_with("sha256:") && digest.len() == 71)
            .ok_or("evidence sha256 must be `sha256:` plus 64 lowercase hex digits")?;
        if !digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !seen.insert(locator)
        {
            return Err("evidence digest is malformed or locator is duplicated".to_owned());
        }
        let (file, selector) = locator
            .split_once('#')
            .ok_or_else(|| format!("evidence locator `{locator}` needs a selector"))?;
        let relative = Path::new(file);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "evidence locator `{locator}` must stay beneath a source root"
            ));
        }
        if !selector.starts_with('/') && !selector.starts_with('L') {
            return Err(format!("evidence selector in `{locator}` is malformed"));
        }
    }
    Ok(())
}

fn resolve_evidence_file(root: &Path, cwd: &Path, relative: &Path) -> Result<PathBuf, String> {
    for candidate in [root.join(relative), cwd.join(relative)] {
        let Ok(path) = fs::canonicalize(&candidate) else {
            continue;
        };
        if path.is_file() && (path.starts_with(root) || path.starts_with(cwd)) {
            return Ok(path);
        }
    }
    Err(format!(
        "evidence source `{}` was not found beneath artifact_root or repository cwd",
        relative.display()
    ))
}

fn validate_line_selector(selector: &str, bytes: &[u8], locator: &str) -> Result<(), String> {
    let raw = selector.strip_prefix('L').ok_or_else(|| {
        format!("evidence selector in `{locator}` must be a JSON Pointer or `Lstart-Lend`")
    })?;
    let (start, end) = match raw.split_once("-L") {
        Some((start, end)) => (start, end),
        None => (raw, raw),
    };
    let start = start.parse::<usize>().ok().filter(|line| *line > 0);
    let end = end.parse::<usize>().ok().filter(|line| *line > 0);
    let (Some(start), Some(end)) = (start, end) else {
        return Err(format!(
            "evidence line selector in `{locator}` is malformed"
        ));
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|_| format!("evidence line selector in `{locator}` requires UTF-8 source"))?;
    let line_count = text.lines().count();
    if start > end || end > line_count {
        return Err(format!(
            "evidence line range in `{locator}` is outside the source"
        ));
    }
    Ok(())
}

/// Validate the AC-N grammar used by the criterion spine.
///
/// This deliberately mirrors the bounded schema pattern without introducing
/// a general regular-expression implementation.
pub(crate) fn is_criterion_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("AC-") else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_digit() && first != '0')
        && chars.all(|character| character.is_ascii_digit())
}

/// Return whether a downstream artifact contains any optional criterion
/// reference field declared by its frozen artifact schema.  Presence,
/// including an empty or malformed field, is intentional: an authored
/// reference must be checked rather than silently ignored.
pub(crate) fn has_references(
    schema: Option<&ValidatedSchema>,
    subject: &str,
    value: &Value,
) -> bool {
    let Some((collection, field)) = reference_field(subject) else {
        return false;
    };
    if !schema.is_some_and(|schema| schema.array_item_declares_property(collection, field)) {
        return false;
    }
    has_scalar_references(value, collection, field)
}

/// Validate the current intent's criterion declarations.
///
/// A missing `acceptance` field is left to the configured artifact schema.  A
/// present field is checked here as well because downstream link evaluation
/// may encounter a current intent whose bytes changed after its own draft
/// hop.  Only schemas declaring object acceptance items activate this AC-N
/// identity rule; scalar acceptance items remain governed by their schema.
pub(crate) fn validate_intent(
    schema: Option<&ValidatedSchema>,
    value: &Value,
) -> Vec<CriterionViolation> {
    if !schema.is_some_and(|schema| schema.array_items_are_objects("acceptance")) {
        return Vec::new();
    }

    let Some(acceptance) = value.get("acceptance") else {
        return Vec::new();
    };
    let Some(entries) = acceptance.as_array() else {
        return vec![CriterionViolation::new(
            "/acceptance",
            "criterion-identity",
            "intent `acceptance` must be an array of criterion records",
        )];
    };

    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let entry_path = format!("/acceptance/{index}");
        let Some(entry) = entry.as_object() else {
            violations.push(CriterionViolation::new(
                entry_path,
                "criterion-identity",
                "criterion record must be an object",
            ));
            continue;
        };

        match entry.get("id") {
            None => violations.push(CriterionViolation::new(
                format!("{entry_path}/id"),
                "criterion-identity",
                "criterion record requires an `id`",
            )),
            Some(value) => match value.as_str() {
                Some(id) if !is_criterion_id(id) => violations.push(CriterionViolation::new(
                    format!("{entry_path}/id"),
                    "criterion-identity",
                    format!("criterion ID `{id}` does not match `{CRITERION_ID_PATTERN}`"),
                )),
                Some(id) => {
                    if !seen.insert(id.to_owned()) {
                        violations.push(CriterionViolation::new(
                            format!("{entry_path}/id"),
                            "criterion-identity",
                            format!("duplicate criterion ID `{id}` in current intent"),
                        ));
                    }
                }
                None => violations.push(CriterionViolation::new(
                    format!("{entry_path}/id"),
                    "criterion-identity",
                    "criterion ID must be a string",
                )),
            },
        }

        match entry.get("statement") {
            None => violations.push(CriterionViolation::new(
                format!("{entry_path}/statement"),
                "criterion-identity",
                "criterion record requires a `statement`",
            )),
            Some(value) => match value.as_str() {
                Some(statement) if !statement.is_empty() => {}
                Some(_) => violations.push(CriterionViolation::new(
                    format!("{entry_path}/statement"),
                    "criterion-identity",
                    "criterion statement must not be empty",
                )),
                None => violations.push(CriterionViolation::new(
                    format!("{entry_path}/statement"),
                    "criterion-identity",
                    "criterion statement must be a string",
                )),
            },
        }
    }

    sort_violations(&mut violations);
    violations
}

/// Collect declared criterion IDs from an intent.  Callers normally invoke
/// this only after `validate_intent` succeeds; retaining only string values
/// also makes this projection harmless for a malformed target.
pub(crate) fn intent_ids(value: &Value) -> BTreeSet<String> {
    value
        .get("acceptance")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter(|id| is_criterion_id(id))
        .map(str::to_owned)
        .collect()
}

/// Validate optional references on one downstream subject against the IDs in
/// its currently linked intent.  Each artifact surface is one local
/// collection; a plan task's `criterion_ids` array is its own collection so
/// the same criterion may be used by separate tasks without becoming an
/// exact-once or cross-surface rule.
pub(crate) fn validate_references(
    subject: &str,
    value: &Value,
    known_ids: &BTreeSet<String>,
) -> Vec<CriterionViolation> {
    validate_references_with_task_requirement(subject, value, known_ids, false)
}

/// Validate references for a contract. Contract v3 additionally requires
/// every plan task to name at least one unique current criterion. The schema
/// normally enforces presence; keeping the rule here also protects callers
/// using a custom but otherwise valid schema.
pub(crate) fn validate_references_with_task_requirement(
    subject: &str,
    value: &Value,
    known_ids: &BTreeSet<String>,
    require_plan_task_criteria: bool,
) -> Vec<CriterionViolation> {
    let mut violations = Vec::new();
    match subject {
        DESIGN_SUBJECT => {
            validate_scalar_collection(
                value,
                "coverage",
                "criterion_id",
                known_ids,
                &mut violations,
            );
        }
        PLAN_SUBJECT => {
            validate_plan_task_collections(
                value,
                known_ids,
                require_plan_task_criteria,
                &mut violations,
            );
        }
        IMPLEMENTATION_SUBJECT => {
            validate_scalar_collection(
                value,
                "validation",
                "criterion_id",
                known_ids,
                &mut violations,
            );
        }
        VALIDATION_SUBJECT => {
            validate_scalar_collection(
                value,
                "requirements",
                "criterion_id",
                known_ids,
                &mut violations,
            );
        }
        _ => {}
    }
    sort_violations(&mut violations);
    violations
}

fn reference_field(subject: &str) -> Option<(&'static str, &'static str)> {
    match subject {
        DESIGN_SUBJECT => Some(("coverage", "criterion_id")),
        PLAN_SUBJECT => Some(("tasks", "criterion_ids")),
        IMPLEMENTATION_SUBJECT => Some(("validation", "criterion_id")),
        VALIDATION_SUBJECT => Some(("requirements", "criterion_id")),
        _ => None,
    }
}

fn has_scalar_references(value: &Value, collection: &str, field: &str) -> bool {
    value
        .get(collection)
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .as_object()
                    .is_some_and(|object| object.contains_key(field))
            })
        })
}

fn validate_scalar_collection(
    value: &Value,
    collection: &str,
    field: &str,
    known_ids: &BTreeSet<String>,
    violations: &mut Vec<CriterionViolation>,
) {
    let Some(entries) = value.get(collection).and_then(Value::as_array) else {
        return;
    };
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let Some(reference) = object.get(field) else {
            continue;
        };
        validate_reference(
            reference,
            &format!("/{collection}/{index}/{field}"),
            &mut seen,
            known_ids,
            violations,
        );
    }
}

fn validate_plan_task_collections(
    value: &Value,
    known_ids: &BTreeSet<String>,
    require_criteria: bool,
    violations: &mut Vec<CriterionViolation>,
) {
    let Some(tasks) = value.get("tasks").and_then(Value::as_array) else {
        return;
    };
    for (task_index, task) in tasks.iter().enumerate() {
        let Some(object) = task.as_object() else {
            continue;
        };
        let Some(references) = object.get("criterion_ids") else {
            if require_criteria {
                violations.push(CriterionViolation::new(
                    format!("/tasks/{task_index}/criterion_ids"),
                    "criterion-reference",
                    "every plan task must name at least one current criterion",
                ));
            }
            continue;
        };
        let path = format!("/tasks/{task_index}/criterion_ids");
        let Some(references) = references.as_array() else {
            violations.push(CriterionViolation::new(
                path,
                "criterion-reference",
                "`criterion_ids` must be an array of AC-N strings",
            ));
            continue;
        };
        if references.is_empty() {
            violations.push(CriterionViolation::new(
                path,
                "criterion-reference",
                "`criterion_ids` must not be empty when present",
            ));
            continue;
        }

        let mut seen = BTreeSet::new();
        for (reference_index, reference) in references.iter().enumerate() {
            validate_reference(
                reference,
                &format!("/tasks/{task_index}/criterion_ids/{reference_index}"),
                &mut seen,
                known_ids,
                violations,
            );
        }
    }
}

fn validate_reference(
    reference: &Value,
    path: &str,
    seen: &mut BTreeSet<String>,
    known_ids: &BTreeSet<String>,
    violations: &mut Vec<CriterionViolation>,
) {
    let Some(id) = reference.as_str() else {
        violations.push(CriterionViolation::new(
            path,
            "criterion-reference",
            "criterion reference must be a string matching AC-N",
        ));
        return;
    };
    if !is_criterion_id(id) {
        violations.push(CriterionViolation::new(
            path,
            "criterion-reference",
            format!("criterion reference `{id}` does not match `{CRITERION_ID_PATTERN}`"),
        ));
        return;
    }
    if !seen.insert(id.to_owned()) {
        violations.push(CriterionViolation::new(
            path,
            "criterion-reference",
            format!("duplicate criterion reference `{id}` in local field"),
        ));
    }
    if !known_ids.contains(id) {
        violations.push(CriterionViolation::new(
            path,
            "criterion-reference",
            format!("criterion reference `{id}` is not present in the current intent"),
        ));
    }
}

fn sort_violations(violations: &mut [CriterionViolation]) {
    violations.sort_by(|left, right| {
        (&left.path, &left.rule, &left.message).cmp(&(&right.path, &right.rule, &right.message))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(violations: &[CriterionViolation]) -> Vec<&str> {
        violations
            .iter()
            .map(|violation| violation.rule.as_str())
            .collect()
    }

    fn structured_criterion_schema() -> ValidatedSchema {
        crate::schema::validate_schema(&json!({
            "type": "object",
            "properties": {
                "acceptance": {
                    "type": "array",
                    "items": {"type": "object"}
                }
            }
        }))
        .expect("structured criterion schema")
    }

    #[test]
    fn criterion_id_grammar_is_ascii_and_one_based() {
        for value in ["AC-1", "AC-9", "AC-10", "AC-123"] {
            assert!(is_criterion_id(value), "{value}");
        }
        for value in ["AC-", "AC-0", "AC-01", "AC-1x", "ac-1", "AC-١"] {
            assert!(!is_criterion_id(value), "{value}");
        }
    }

    #[test]
    fn intent_identity_reports_duplicate_and_empty_or_malformed_records() {
        let schema = structured_criterion_schema();
        let violations = validate_intent(
            Some(&schema),
            &json!({
                "acceptance": [
                    {"id": "AC-1", "statement": "one"},
                    {"id": "AC-1", "statement": ""},
                    {"id": "bad", "statement": 1},
                    {"statement": "missing id"}
                ]
            }),
        );
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("duplicate criterion ID")));
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("must not be empty")));
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("does not match")));
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("requires an `id`")));
        assert_eq!(rules(&violations).len(), 5);
    }

    #[test]
    fn references_are_optional_but_present_references_are_local_unique_and_known() {
        let known = BTreeSet::from(["AC-1".to_owned()]);
        assert!(!has_references(
            None,
            DESIGN_SUBJECT,
            &json!({"coverage": [{"delivered_by": "part"}]})
        ));
        assert!(validate_references(
            DESIGN_SUBJECT,
            &json!({"coverage": [{"delivered_by": "part"}] }),
            &known
        )
        .is_empty());

        let violations = validate_references(
            DESIGN_SUBJECT,
            &json!({
                "coverage": [
                    {"criterion_id": "AC-1"},
                    {"criterion_id": "AC-1"},
                    {"criterion_id": "AC-2"}
                ]
            }),
            &known,
        );
        assert_eq!(violations.len(), 2);
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("duplicate")));
        assert!(violations
            .iter()
            .any(|violation| violation.message.contains("not present")));
    }

    #[test]
    fn grounding_checks_exact_source_digest_and_json_pointer_or_lines() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!(
            "software-change-grounding-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let json_bytes = br#"{"revision":"1","outcome":"delivered"}"#;
        fs::write(root.join("intent.json"), json_bytes).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(json_bytes));
        let valid = json!({"reason":"I inspected the current outcome.","evidence":[
            {"locator":"intent.json#/outcome","sha256":digest}
        ]});
        assert!(validate_grounding(&valid, &root).is_ok());
        let bad_pointer = json!({"reason":"checked","evidence":[
            {"locator":"intent.json#/missing","sha256":digest}
        ]});
        assert!(validate_grounding(&bad_pointer, &root)
            .unwrap_err()
            .contains("does not resolve"));
        let bad_digest = json!({"reason":"checked","evidence":[
            {"locator":"intent.json#/outcome","sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}
        ]});
        assert!(validate_grounding(&bad_digest, &root)
            .unwrap_err()
            .contains("does not match"));
        let escaped = json!({"reason":"checked","evidence":[
            {"locator":"../intent.json#/outcome","sha256":digest}
        ]});
        assert!(validate_grounding(&escaped, &root)
            .unwrap_err()
            .contains("stay beneath"));
        fs::write(root.join("notes.md"), b"one\ntwo\nthree\n").unwrap();
        let notes_digest = format!("sha256:{:x}", Sha256::digest(b"one\ntwo\nthree\n"));
        let lines = json!({"reason":"checked","evidence":[
            {"locator":"notes.md#L2-L3","sha256":notes_digest}
        ]});
        assert!(validate_grounding(&lines, &root).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plan_references_are_unique_per_task_not_across_tasks() {
        let known = BTreeSet::from(["AC-1".to_owned()]);
        let duplicate = validate_references(
            PLAN_SUBJECT,
            &json!({
                "tasks": [
                    {"criterion_ids": ["AC-1", "AC-1"]},
                    {"criterion_ids": ["AC-1"]}
                ]
            }),
            &known,
        );
        assert_eq!(duplicate.len(), 1);
        assert!(duplicate[0].message.contains("duplicate"));
    }
}
