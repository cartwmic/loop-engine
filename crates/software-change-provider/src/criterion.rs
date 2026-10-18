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
    validate_grounding_version(grounds, artifact_root, Some(2))
}

pub(crate) fn validate_grounding_version(
    grounds: &Value,
    artifact_root: &Path,
    version: Option<u32>,
) -> Result<(), String> {
    validate_grounding_with_context(grounds, artifact_root, version, &[])
}

/// Record pointers address the complete immutable stored context record.
pub(crate) fn validate_grounding_with_context(
    grounds: &Value,
    artifact_root: &Path,
    version: Option<u32>,
    context: &[loop_core::ContextRecord],
) -> Result<(), String> {
    validate_grounding_shape_version(grounds, version)?;
    if version == Some(3)
        && grounds.as_object().is_none_or(|object| {
            object.len() != 2 || !object.contains_key("reason") || !object.contains_key("evidence")
        })
    {
        return Err("v3 grounds require exactly reason and evidence".to_owned());
    }
    let reason = grounds
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .ok_or("grounds.reason must be a non-empty reviewer rationale")?;
    if version == Some(2) && reason.len() > 1200 {
        return Err("grounds.reason exceeds 1200 bytes".to_owned());
    }
    let evidence = grounds
        .get("evidence")
        .and_then(Value::as_array)
        .filter(|evidence| !evidence.is_empty() && (version != Some(2) || evidence.len() <= 8))
        .ok_or("grounds.evidence must contain source references")?;
    let mut seen = BTreeSet::new();
    for reference in evidence {
        if version == Some(3)
            && reference
                .as_object()
                .is_none_or(|object| object.len() != 1 || !object.contains_key("locator"))
        {
            return Err("v3 evidence reference must contain only locator".to_owned());
        }
        let locator = reference
            .get("locator")
            .and_then(Value::as_str)
            .filter(|locator| !locator.trim().is_empty())
            .ok_or("evidence locator must be a non-empty repository-relative path and selector")?;
        if !seen.insert(locator) {
            return Err("evidence locator is duplicated".to_owned());
        }
        let digest = if version == Some(2) {
            let digest = reference
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|digest| digest.starts_with("sha256:") && digest.len() == 71)
                .ok_or("evidence sha256 must be `sha256:` plus 64 lowercase hex digits")?;
            if !digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("evidence digest is malformed or locator is duplicated".to_owned());
            }
            Some(digest)
        } else {
            None
        };
        resolve_locator(locator, digest, artifact_root, version, context)?;
    }
    Ok(())
}

/// Resolve one already shape-checked evidence locator exactly as admission does:
/// a same-run record citation or a file citation beneath the artifact directory or a
/// declared source root. Reads sources; never repairs, suggests-and-admits or writes.
fn resolve_locator(
    locator: &str,
    digest: Option<&str>,
    artifact_root: &Path,
    version: Option<u32>,
    context: &[loop_core::ContextRecord],
) -> Result<(), String> {
    let (file, selector) = locator.split_once('#').ok_or_else(|| {
        format!("evidence locator `{locator}` needs a JSON Pointer or line selector")
    })?;
    if let Some(id) = file.strip_prefix("record:") {
        validate_record_locator(id, selector)?;
        let matches: Vec<_> = context
            .iter()
            .filter(|record| record.id.as_str() == id)
            .collect();
        if matches.len() != 1 {
            return Err(format!("record citation `{id}` is missing or ambiguous"));
        }
        let record = matches[0];
        if !REVIEW_SOURCE_KINDS.contains(&record.kind.as_str()) {
            return Err(format!(
                "record citation `{id}` has disallowed kind `{}`",
                record.kind
            ));
        }
        let stored = serde_json::to_value(record).map_err(|e| e.to_string())?;
        if stored.pointer(selector).is_none() {
            return Err(format!(
                "record citation `{locator}` does not resolve; {}",
                nearest_pointer_note(&stored, selector, &format!("record:{id}"))
            ));
        }
        if version != Some(3) {
            return Err("record citations require review contract v3".into());
        }
        return Ok(());
    }
    let root = fs::canonicalize(artifact_root).map_err(|error| error.to_string())?;
    let relative = Path::new(file.split_once(':').map_or(file, |(_, path)| path));
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
    let roots = crate::source_scopes::locator_roots(&root)?;
    let path = if let Some((scope, _)) = file.split_once(':') {
        let scope_root = crate::source_scopes::resolve(&roots, scope)?;
        resolve_evidence_file(&scope_root, Some(&scope_root), relative)?
    } else {
        let primary = roots
            .get("primary")
            .or(if roots.is_empty() { None } else { Some(&root) });
        resolve_evidence_file(&root, primary.map(PathBuf::as_path), relative)?
    };
    let bytes = fs::read(&path)
        .map_err(|error| format!("evidence source `{locator}` is unreadable: {error}"))?;
    if digest.is_some_and(|digest| format!("sha256:{:x}", Sha256::digest(&bytes)) != digest) {
        return Err(format!(
            "evidence digest does not match exact source bytes for `{file}`"
        ));
    }
    if selector.starts_with('/') {
        let json: Value = serde_json::from_slice(&bytes)
            .map_err(|_| format!("evidence selector `{locator}` requires a JSON source"))?;
        if json.pointer(selector).is_none() {
            return Err(format!(
                "evidence JSON Pointer in `{locator}` does not resolve; {}",
                nearest_pointer_note(&json, selector, file)
            ));
        }
    } else {
        validate_line_selector(selector, &bytes, locator)?;
    }
    Ok(())
}

/// The evidence locators of a grounds object that do not resolve, as
/// `(index, locator, existing diagnostic)`. The grounds must be structurally valid
/// in every other respect (shape, closed entries, no duplicates, locator syntax):
/// any such defect is returned as `Err` so it can never be mistaken for a
/// locator-resolution defect. Reads sources only.
#[allow(dead_code)] // Consumed by the binary's review-candidates repair, not by the library copy.
pub(crate) fn unresolvable_locators(
    grounds: &Value,
    artifact_root: &Path,
    version: Option<u32>,
    context: &[loop_core::ContextRecord],
) -> Result<Vec<(usize, String, String)>, String> {
    validate_grounding_shape_version(grounds, version)?;
    if version == Some(3)
        && grounds.as_object().is_none_or(|object| {
            object.len() != 2 || !object.contains_key("reason") || !object.contains_key("evidence")
        })
    {
        return Err("v3 grounds require exactly reason and evidence".to_owned());
    }
    let evidence = grounds
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or("grounds.evidence must contain source references")?;
    let mut failures = Vec::new();
    for (index, reference) in evidence.iter().enumerate() {
        let locator = reference
            .get("locator")
            .and_then(Value::as_str)
            .ok_or("evidence locator must be a non-empty repository-relative path and selector")?;
        let digest = reference.get("sha256").and_then(Value::as_str);
        if let Err(error) = resolve_locator(locator, digest, artifact_root, version, context) {
            failures.push((index, locator.to_owned(), error));
        }
    }
    Ok(failures)
}

/// Check retained citation structure without rebinding a historical digest to
/// a changed current file. Current-source citations use `validate_grounding`.
pub(crate) fn validate_grounding_shape(grounds: &Value) -> Result<(), String> {
    validate_grounding_shape_version(grounds, Some(2))
}

pub(crate) fn validate_grounding_shape_version(
    grounds: &Value,
    version: Option<u32>,
) -> Result<(), String> {
    if version == Some(3)
        && grounds.as_object().is_none_or(|object| {
            object.len() != 2 || !object.contains_key("reason") || !object.contains_key("evidence")
        })
    {
        return Err("v3 grounds require exactly reason and evidence".to_owned());
    }
    let reason = grounds
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .ok_or("grounds.reason must be a non-empty reviewer rationale")?;
    if version == Some(2) && reason.len() > 1200 {
        return Err("grounds.reason exceeds 1200 bytes".to_owned());
    }
    let evidence = grounds
        .get("evidence")
        .and_then(Value::as_array)
        .filter(|evidence| !evidence.is_empty() && (version != Some(2) || evidence.len() <= 8))
        .ok_or("grounds.evidence must contain source references")?;
    let mut seen = BTreeSet::new();
    for reference in evidence {
        if version == Some(3)
            && reference
                .as_object()
                .is_none_or(|object| object.len() != 1 || !object.contains_key("locator"))
        {
            return Err("v3 evidence reference must contain only locator".to_owned());
        }
        let locator = reference
            .get("locator")
            .and_then(Value::as_str)
            .filter(|locator| !locator.trim().is_empty())
            .ok_or("evidence locator must be a non-empty repository-relative path and selector")?;
        if !seen.insert(locator) {
            return Err("evidence locator is duplicated".to_owned());
        }
        let _digest = if version == Some(2) {
            let digest = reference
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|digest| digest.starts_with("sha256:") && digest.len() == 71)
                .ok_or("evidence sha256 must be `sha256:` plus 64 lowercase hex digits")?;
            if !digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("evidence digest is malformed or locator is duplicated".to_owned());
            }
            Some(digest)
        } else {
            None
        };
        let (file, selector) = locator
            .split_once('#')
            .ok_or_else(|| format!("evidence locator `{locator}` needs a selector"))?;
        if let Some(id) = file.strip_prefix("record:") {
            if version != Some(3) {
                return Err("record citations require review contract v3".into());
            }
            validate_record_locator(id, selector)?;
            continue;
        }
        let relative = Path::new(file.split_once(':').map_or(file, |(_, path)| path));
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

const REVIEW_SOURCE_KINDS: &[&str] = &[
    "review-evidence",
    "intent-comparison",
    "user-steering",
    "finding-ledger",
    "evidence-applicability",
    "criterion-verdict",
    "goal-verdict",
    "intent-correction",
];

fn validate_record_locator(id: &str, pointer: &str) -> Result<(), String> {
    if id.is_empty()
        || id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '#' | '/' | '\\'))
    {
        return Err("record citation ID is malformed".into());
    }
    if !pointer.starts_with('/') {
        return Err("record citation requires a JSON Pointer".into());
    }
    for token in pointer.split('/').skip(1) {
        let mut bytes = token.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
                return Err("record citation JSON Pointer escape is malformed".into());
            }
        }
    }
    Ok(())
}

fn resolve_evidence_file(
    root: &Path,
    primary: Option<&Path>,
    relative: &Path,
) -> Result<PathBuf, String> {
    for base in std::iter::once(root).chain(primary) {
        let Ok(path) = fs::canonicalize(base.join(relative)) else {
            continue;
        };
        if path.is_file()
            && (path.starts_with(root) || primary.is_some_and(|p| path.starts_with(p)))
        {
            return Ok(path);
        }
    }
    if primary.is_none() {
        return Err(format!(
            "evidence source `{}` was not found beneath the artifact directory; {}",
            relative.display(),
            crate::source_scopes::NO_LOCATOR_ROOT
        ));
    }
    Err(format!(
        "evidence source `{}` was not found beneath the artifact directory or the source root",
        relative.display()
    ))
}

fn pointer_tokens(pointer: &str) -> Vec<String> {
    pointer
        .split('/')
        .skip(1)
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn pointer_from(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("/{}", token.replace('~', "~0").replace('/', "~1")))
        .collect()
}

fn child<'a>(node: &'a Value, token: &str) -> Option<&'a Value> {
    match node {
        Value::Object(map) => map.get(token),
        Value::Array(items) => token.parse::<usize>().ok().and_then(|i| items.get(i)),
        _ => None,
    }
}

/// Closest existing pointer for one that does not resolve, or `None`. Only ever
/// returns a pointer that exists in `document`; it never repairs or admits a return.
fn nearest_pointer(document: &Value, pointer: &str) -> Option<String> {
    let tokens = pointer_tokens(pointer);
    let mut node = document;
    let mut depth = 0;
    while depth < tokens.len() {
        match child(node, &tokens[depth]) {
            Some(next) => {
                node = next;
                depth += 1;
            }
            None => break,
        }
    }
    let prefix = &tokens[..depth];
    let rest = &tokens[depth..];
    if rest.is_empty() {
        return None;
    }
    let mut candidates: Vec<Vec<String>> = Vec::new();
    // An index past the end of an array: offer the last element.
    if let Value::Array(items) = node {
        if rest[0].parse::<usize>().is_ok() && !items.is_empty() {
            let mut near = prefix.to_vec();
            near.push((items.len() - 1).to_string());
            candidates.push(near);
        }
    }
    // The cited path exists somewhere deeper beneath the last valid node.
    let mut queue = std::collections::VecDeque::from([(prefix.to_vec(), node, 0usize)]);
    while let Some((path, current, level)) = queue.pop_front() {
        if level > 0 {
            let mut found = Some(current);
            for token in rest {
                found = found.and_then(|n| child(n, token));
            }
            if found.is_some() {
                let mut near = path.clone();
                near.extend(rest.iter().cloned());
                candidates.push(near);
            }
        }
        if level < 4 {
            let children: Vec<(String, &Value)> = match current {
                Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
                Value::Array(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v))
                    .collect(),
                _ => Vec::new(),
            };
            for (key, value) in children {
                let mut next = path.clone();
                next.push(key);
                queue.push_back((next, value, level + 1));
            }
        }
    }
    candidates
        .into_iter()
        .map(|tokens| pointer_from(&tokens))
        .find(|candidate| document.pointer(candidate).is_some())
}

fn nearest_pointer_note(document: &Value, pointer: &str, file: &str) -> String {
    match nearest_pointer(document, pointer) {
        Some(near) => format!("closest existing pointer: `{file}#{near}`"),
        None => "no close existing pointer was found".to_owned(),
    }
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
    fn locator_only_grounding_has_no_reason_or_reference_quota() {
        let root = std::env::temp_dir().join(format!("locator-v3-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let subject = serde_json::json!({"items": (0..12).collect::<Vec<_>>()});
        fs::write(
            root.join("intent.json"),
            serde_json::to_vec(&subject).unwrap(),
        )
        .unwrap();
        let grounds = serde_json::json!({"reason":"根拠".repeat(1500), "evidence":(0..12).map(|i| serde_json::json!({"locator":format!("intent.json#/items/{i}")})).collect::<Vec<_>>()});
        assert!(validate_grounding_version(&grounds, &root, Some(3)).is_ok());
        assert!(validate_grounding_version(&grounds, &root, Some(2)).is_err());
        let missing = serde_json::json!({"reason":"inspected", "evidence":[{"locator":"intent.json#/missing"}]});
        assert!(validate_grounding_version(&missing, &root, Some(3))
            .unwrap_err()
            .contains("does not resolve"));
        fs::remove_dir_all(root).unwrap();
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
    fn unresolvable_locators_separates_resolution_from_every_other_defect() {
        let root = std::env::temp_dir().join(format!(
            "software-change-unresolvable-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let acceptance: Vec<Value> = (0..12)
            .map(|i| json!({"id": format!("AC-{}", i + 1)}))
            .collect();
        fs::write(
            root.join("intent.json"),
            serde_json::to_vec(&json!({"revision":"1","acceptance":acceptance,
                "operating_context":{"outside_obligations":["a","b","c","d"]}}))
            .unwrap(),
        )
        .unwrap();
        let grounds = |locators: &[&str]| json!({"reason":"checked","evidence": locators.iter().map(|l| json!({"locator": l})).collect::<Vec<_>>()});
        // The two observed 2026-10-14 mistakes are reported with the existing diagnostic.
        let found = unresolvable_locators(
            &grounds(&[
                "intent.json#/acceptance/12",
                "intent.json#/outside_obligations/3",
                "intent.json#/acceptance/11",
            ]),
            &root,
            Some(3),
            &[],
        )
        .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].0, found[0].1.as_str()),
            (0, "intent.json#/acceptance/12")
        );
        assert!(
            found[0]
                .2
                .contains("closest existing pointer: `intent.json#/acceptance/11`"),
            "{}",
            found[0].2
        );
        assert_eq!(
            (found[1].0, found[1].1.as_str()),
            (1, "intent.json#/outside_obligations/3")
        );
        // All resolving: nothing to repair.
        assert!(unresolvable_locators(
            &grounds(&["intent.json#/revision", "intent.json#/acceptance/11"]),
            &root,
            Some(3),
            &[]
        )
        .unwrap()
        .iter()
        .all(|_| false));
        // Every non-resolution defect is an Err, never a locator defect.
        for (bad, why) in [
            (
                grounds(&["intent.json#/acceptance/12", "intent.json#/acceptance/12"]),
                "duplicated",
            ),
            (grounds(&["../intent.json#/revision"]), "stay beneath"),
            (grounds(&["intent.json#bogus"]), "malformed"),
            (
                json!({"reason":"","evidence":[{"locator":"intent.json#/revision"}]}),
                "reason",
            ),
            (json!({"reason":"x","evidence":[]}), "source references"),
            (
                json!({"reason":"x","evidence":[{"locator":"intent.json#/revision","extra":1}]}),
                "only locator",
            ),
            (
                json!({"reason":"x","evidence":[{"locator":"intent.json#/revision"}],"more":1}),
                "exactly reason and evidence",
            ),
        ] {
            let error = unresolvable_locators(&bad, &root, Some(3), &[]).unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
        // Ordinary validation keeps its behavior and message for the same input.
        let ordinary = validate_grounding_with_context(
            &grounds(&["intent.json#/acceptance/12"]),
            &root,
            Some(3),
            &[],
        )
        .unwrap_err();
        assert!(
            ordinary
                .contains("evidence JSON Pointer in `intent.json#/acceptance/12` does not resolve"),
            "{ordinary}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nearest_pointer_names_only_existing_pointers_or_says_none_is_close() {
        let document = json!({
            "acceptance": (0..12).map(|i| json!({"id": format!("AC-{}", i + 1)})).collect::<Vec<_>>(),
            "operating_context": {"outside_obligations": ["a", "b", "c", "d"]},
            "outcome": "x"
        });
        // Past the end of a 12-entry array: the last real entry.
        assert_eq!(
            nearest_pointer(&document, "/acceptance/12").as_deref(),
            Some("/acceptance/11")
        );
        // A section cited at the wrong level: where it actually lives.
        assert_eq!(
            nearest_pointer(&document, "/outside_obligations/3").as_deref(),
            Some("/operating_context/outside_obligations/3")
        );
        // Nothing close: say so, never invent.
        assert_eq!(nearest_pointer(&document, "/nothing_like_this/7"), None);
        assert_eq!(nearest_pointer(&document, "/outcome"), None);
        assert!(
            nearest_pointer_note(&document, "/nothing_like_this/7", "intent.json")
                .contains("no close existing pointer")
        );
        assert!(
            nearest_pointer_note(&document, "/acceptance/12", "intent.json")
                .contains("closest existing pointer: `intent.json#/acceptance/11`")
        );
        for pointer in [
            "/acceptance/12",
            "/outside_obligations/3",
            "/acceptance/01",
            "/outcome/0",
        ] {
            if let Some(near) = nearest_pointer(&document, pointer) {
                assert!(document.pointer(&near).is_some(), "{pointer} -> {near}");
            }
        }
    }

    #[test]
    fn locator_roots_do_not_depend_on_the_process_directory() {
        use sha2::{Digest, Sha256};
        let base = std::env::temp_dir().join(format!(
            "software-change-roots-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("repo");
        fs::create_dir_all(repo.join("run")).unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(init.success());
        fs::write(repo.join("src/a.txt"), b"one\ntwo\n").unwrap();
        fs::write(repo.join("run/intent.json"), br#"{"revision":"1"}"#).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(b"one\ntwo\n"));
        let grounds = json!({"reason":"checked","evidence":[
            {"locator":"src/a.txt#L1-L2","sha256":digest}
        ]});
        // The test process runs elsewhere; the repository file still resolves
        // because the root comes from the artifact directory's git top level.
        assert!(validate_grounding(&grounds, &repo.join("run")).is_ok());
        // Without a declaration or a git work tree, repository-relative
        // locators are refused with a message naming source_scopes.
        let loose = base.join("loose");
        fs::create_dir_all(&loose).unwrap();
        fs::write(loose.join("intent.json"), br#"{"revision":"1"}"#).unwrap();
        let refusal = validate_grounding(&grounds, &loose).unwrap_err();
        assert!(refusal.contains("source_scopes"), "{refusal}");
        // A declared scope wins and works from anywhere.
        fs::write(
            loose.join("intent.json"),
            serde_json::to_vec(&json!({"revision":"1","source_scopes":{"primary":{"root":repo}}}))
                .unwrap(),
        )
        .unwrap();
        assert!(validate_grounding(&grounds, &loose).is_ok());
        let _ = fs::remove_dir_all(base);
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
