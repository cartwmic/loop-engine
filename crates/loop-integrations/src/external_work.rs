//! Non-executing external-content reading and mechanical metadata assembly.
//! Prose and YAML remain original text requiring driver-confirmed projection.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentSource {
    Inline(String),
    File(PathBuf),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetainedContent {
    pub source: ContentSource,
    pub original: String,
    pub structured: Option<Value>,
    pub diagnostic: Option<String>,
}

/// Read actual supplied content; never execute a reference or replace originals.
pub fn read_content(source: ContentSource) -> io::Result<RetainedContent> {
    let original = match &source {
        ContentSource::Inline(text) => text.clone(),
        ContentSource::File(path) => fs::read_to_string(path)?,
    };
    let (structured, diagnostic) = decode_content(&original);
    Ok(RetainedContent {
        source,
        original,
        structured,
        diagnostic,
    })
}

/// Only JSON and a single JSON fence are decoded. No semantic inference occurs.
pub fn decode_content(original: &str) -> (Option<Value>, Option<String>) {
    let trimmed = original.trim();
    let json = if let Some(fenced) = trimmed
        .strip_prefix("```json\n")
        .or_else(|| trimmed.strip_prefix("```\n"))
    {
        match fenced.strip_suffix("```") {
            Some(body) if !body.contains("```") => body.trim(),
            _ => {
                return (
                    None,
                    Some(
                        "invalid JSON fence; retain original and supply a driver projection".into(),
                    ),
                )
            }
        }
    } else {
        trimmed
    };
    match serde_json::from_str(json) {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(format!("not decodable JSON ({error}); retain original prose/YAML and supply a driver-confirmed projection"))),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemDiagnostic {
    pub index: usize,
    pub candidate: Option<Value>,
    pub diagnostics: Vec<String>,
}

/// Complete known metadata only. Contradictions invalidate that item, not siblings.
/// Substantive outcomes must already be present in the supplied projection.
pub fn assemble_items(items: &[Value], known: &Map<String, Value>) -> Vec<ItemDiagnostic> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let Some(mut object) = item.as_object().cloned() else {
                return ItemDiagnostic {
                    index,
                    candidate: None,
                    diagnostics: vec!["projection item must be an object".into()],
                };
            };
            let mut diagnostics = Vec::new();
            for (key, expected) in known {
                match object.get(key) {
                    Some(actual) if actual != expected => diagnostics
                        .push(format!("supplied metadata contradicts known field `{key}`")),
                    None => {
                        object.insert(key.clone(), expected.clone());
                    }
                    _ => {}
                }
            }
            ItemDiagnostic {
                index,
                candidate: diagnostics.is_empty().then_some(Value::Object(object)),
                diagnostics,
            }
        })
        .collect()
}

/// Reject substantive outcome changes while allowing mechanical locator corrections.
pub fn check_projection(
    original: &Value,
    projection: &Value,
    field: &str,
    gate: &str,
) -> Result<(), String> {
    fn check_author(before: &Value, after: &Value) -> Result<(), String> {
        for key in ["name", "kind"] {
            if before["author"]
                .get(key)
                .is_some_and(|value| after["author"].get(key) != Some(value))
            {
                return Err("projection contradicts decoded author".into());
            }
        }
        Ok(())
    }
    check_author(original, projection)?;
    let Some(items) = original[field].as_array() else {
        return Ok(());
    };
    let projected = projection[field]
        .as_array()
        .ok_or("projection omits decoded outcomes")?;
    let prefix = format!("{gate}.");
    let normalize_axis = |value: &Value| match value.as_str() {
        Some(axis) => Value::String(axis.strip_prefix(&prefix).unwrap_or(axis).to_owned()),
        None => value.clone(),
    };
    let identity = |item: &Value| {
        item.get("axis")
            .or_else(|| item.get("policy_id"))
            .map(normalize_axis)
            .unwrap_or(Value::Null)
            .to_string()
    };
    // One judgment names one obligation: a projected item whose axis and policy_id
    // disagree would re-attribute the decoded judgment to another obligation.
    for after in projected {
        if let (Some(axis), Some(policy)) = (after.get("axis"), after.get("policy_id")) {
            if normalize_axis(axis) != normalize_axis(policy) {
                return Err("projection axis and policy_id disagree".into());
            }
        }
    }
    let mut remaining: Vec<_> = projected.iter().collect();
    for before in items {
        let index = remaining
            .iter()
            .position(|after| identity(before) == identity(after))
            .ok_or("projection changes decoded axes")?;
        let after = remaining.remove(index);
        for key in ["axis", "policy_id", "result", "verdict"] {
            if before.get(key).is_some_and(|v| {
                if matches!(key, "axis" | "policy_id") {
                    after.get(key).map(normalize_axis) != Some(normalize_axis(v))
                } else {
                    after.get(key) != Some(v)
                }
            }) {
                return Err(format!("projection contradicts decoded {key}"));
            }
        }
        let mut attributed = after.clone();
        if attributed.get("author").is_none() && attributed.is_object() {
            attributed["author"] = projection["author"].clone();
        }
        check_author(before, &attributed)?;
        let normalize = |value: &Value| match value {
            Value::String(text) => {
                Value::String(text.split_whitespace().collect::<Vec<_>>().join(" "))
            }
            other => other.clone(),
        };
        if before
            .get("findings")
            .is_some_and(|value| after.get("findings").map(normalize) != Some(normalize(value)))
        {
            return Err("projection contradicts decoded findings".into());
        }
        for (before_reason, after_reason) in [
            (
                before["grounds"].get("reason"),
                after["grounds"].get("reason"),
            ),
            (before.get("reason"), after.get("reason")),
        ] {
            if before_reason
                .is_some_and(|value| after_reason.map(normalize) != Some(normalize(value)))
            {
                return Err("projection contradicts decoded reason".into());
            }
        }
    }
    if !remaining.is_empty() {
        return Err("projection changes decoded axes".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn projection_preserves_outcomes_but_allows_locator_correction() {
        let original =
            serde_json::json!({"items":[{"result":"fail","findings":"defect","locator":"wrong"}]});
        let mut projection = original.clone();
        projection["items"][0]["locator"] = serde_json::json!("correct");
        assert!(super::check_projection(&original, &projection, "items", "review").is_ok());
        projection["items"][0]["result"] = serde_json::json!("pass");
        assert!(super::check_projection(&original, &projection, "items", "review").is_err());
        projection["items"][0]["result"] = serde_json::json!("fail");
        projection["items"][0]["findings"] = serde_json::json!("");
        assert!(super::check_projection(&original, &projection, "items", "review").is_err());
    }

    #[test]
    fn projection_preserves_named_axes_authors_and_finding_text() {
        for field in ["items", "judgments"] {
            let original = serde_json::json!({"author":{"name":"Alice","kind":"agent"},
                field:[{"axis":"A","result":"fail","findings":"first defect\nsecond defect"},
                       {"axis":"B","result":"pass","findings":""}]});
            let mut projection = original.clone();
            projection[field].as_array_mut().unwrap().reverse();
            assert!(super::check_projection(&original, &projection, field, "review").is_ok());
            projection[field][0]["axis"] = serde_json::json!("A");
            projection[field][1]["axis"] = serde_json::json!("B");
            assert!(super::check_projection(&original, &projection, field, "review").is_err());
            for (key, value) in [("name", "Bob"), ("kind", "human")] {
                let mut projection = original.clone();
                projection["author"][key] = serde_json::json!(value);
                assert!(super::check_projection(&original, &projection, field, "review").is_err());
            }
            for findings in ["different defect", "first defect", ""] {
                let mut projection = original.clone();
                projection[field][0]["findings"] = serde_json::json!(findings);
                assert!(super::check_projection(&original, &projection, field, "review").is_err());
            }
            let mut projection = original.clone();
            projection[field][0]["findings"] = serde_json::json!(" first   defect second defect ");
            projection[field][0]["locator"] = serde_json::json!("corrected");
            assert!(super::check_projection(&original, &projection, field, "review").is_ok());
            // An added conflicting canonical policy_id cannot re-attribute the judgment.
            let mut projection = original.clone();
            projection[field][1]["policy_id"] = serde_json::json!("C");
            let error = super::check_projection(&original, &projection, field, "review");
            assert_eq!(error.unwrap_err(), "projection axis and policy_id disagree");
            let mut projection = original.clone();
            projection[field][1]["policy_id"] = serde_json::json!("review.B");
            assert!(super::check_projection(&original, &projection, field, "review").is_ok());
        }
    }

    #[test]
    fn projection_preserves_axis_rationales() {
        for field in ["items", "judgments"] {
            for nested in [false, true] {
                let mut original = serde_json::json!({field:[
                    {"axis":"A", "result":"pass", "findings":""},
                    {"axis":"B", "result":"pass", "findings":""}
                ]});
                for (index, reason) in ["first rationale", "second rationale"].iter().enumerate() {
                    if nested {
                        original[field][index]["grounds"] = serde_json::json!({
                            "reason":reason, "evidence":[{"locator":"wrong"}]
                        });
                    } else {
                        original[field][index]["reason"] = serde_json::json!(reason);
                    }
                }
                let path = if nested { "/grounds/reason" } else { "/reason" };
                let mut swapped = original.clone();
                *swapped[field][0].pointer_mut(path).unwrap() =
                    serde_json::json!("second rationale");
                *swapped[field][1].pointer_mut(path).unwrap() =
                    serde_json::json!("first rationale");
                assert!(super::check_projection(&original, &swapped, field, "review").is_err());
                for replacement in ["invented rationale", ""] {
                    let mut replaced = original.clone();
                    *replaced[field][0].pointer_mut(path).unwrap() = serde_json::json!(replacement);
                    assert!(
                        super::check_projection(&original, &replaced, field, "review").is_err()
                    );
                }
                let mut corrected = original.clone();
                *corrected[field][0].pointer_mut(path).unwrap() =
                    serde_json::json!(" first\n  rationale ");
                corrected[field][0]["locator"] = serde_json::json!("corrected");
                if nested {
                    corrected[field][0]["grounds"]["evidence"][0]["locator"] =
                        serde_json::json!("corrected");
                }
                assert!(super::check_projection(&original, &corrected, field, "review").is_ok());
            }
        }
    }

    #[test]
    fn projection_accepts_only_assignment_gate_prefix() {
        for (field, key) in [("items", "policy_id"), ("judgments", "axis")] {
            let original = serde_json::json!({field:[{key:"semantic-review.A", "result":"fail", "findings":"defect"}]});
            let projection =
                serde_json::json!({field:[{key:"A", "result":"fail", "findings":"defect"}]});
            assert!(
                super::check_projection(&original, &projection, field, "semantic-review").is_ok()
            );
            assert!(super::check_projection(&original, &projection, field, "other-gate").is_err());
            let wrong = serde_json::json!({field:[{key:"other-gate.A", "result":"fail", "findings":"defect"}]});
            assert!(
                super::check_projection(&wrong, &projection, field, "semantic-review").is_err()
            );
        }
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn originals_and_file_references_are_retained() {
        let text = "```json\n{\"verdict\":\"pass\"}\n```\n";
        let inline = read_content(ContentSource::Inline(text.into())).unwrap();
        assert_eq!(inline.original, text);
        assert_eq!(inline.structured, Some(json!({"verdict":"pass"})));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("return.txt");
        fs::write(&path, text).unwrap();
        let from_file = read_content(ContentSource::File(path.clone())).unwrap();
        assert_eq!(from_file.original, text);
        assert!(matches!(from_file.source, ContentSource::File(p) if p == path));
        assert!(read_content(ContentSource::File(root.path().join("missing"))).is_err());
    }

    #[test]
    fn prose_and_yaml_require_projection_without_losing_original() {
        for text in [
            "verdict: pass\nrationale: adequate",
            "This passes because the proof is adequate.",
        ] {
            let content = read_content(ContentSource::Inline(text.into())).unwrap();
            assert_eq!(content.original, text);
            assert!(content.structured.is_none());
            assert!(content.diagnostic.is_some());
        }
    }

    #[test]
    fn malformed_and_contradictory_siblings_do_not_discard_valid_items() {
        let known = json!({"target":"current", "producer":"external"})
            .as_object()
            .unwrap()
            .clone();
        let items = assemble_items(
            &[
                json!({"verdict":"pass"}),
                json!(false),
                json!({"target":"other"}),
                json!({"verdict":"fail"}),
            ],
            &known,
        );
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].candidate.as_ref().unwrap()["verdict"], "pass");
        assert!(items[1].candidate.is_none());
        assert!(items[2].candidate.is_none());
        assert_eq!(items[3].candidate.as_ref().unwrap()["verdict"], "fail");
        assert!(items[0]
            .candidate
            .as_ref()
            .unwrap()
            .get("rationale")
            .is_none());
    }
}
