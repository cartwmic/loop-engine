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
pub fn check_projection(original: &Value, projection: &Value, field: &str) -> Result<(), String> {
    let Some(items) = original[field].as_array() else {
        return Ok(());
    };
    let projected = projection[field]
        .as_array()
        .ok_or("projection omits decoded outcomes")?;
    if items.len() != projected.len() {
        return Err("projection changes decoded outcomes".into());
    }
    for (before, after) in items.iter().zip(projected) {
        for key in ["result", "verdict"] {
            if before.get(key).is_some_and(|v| after.get(key) != Some(v)) {
                return Err(format!("projection contradicts decoded {key}"));
            }
        }
        let nonempty = |v: &Value| match v {
            Value::String(s) => !s.trim().is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null => false,
            _ => true,
        };
        if before
            .get("findings")
            .is_some_and(|v| nonempty(v) != after.get("findings").is_some_and(nonempty))
        {
            return Err("projection contradicts decoded findings".into());
        }
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
        assert!(super::check_projection(&original, &projection, "items").is_ok());
        projection["items"][0]["result"] = serde_json::json!("pass");
        assert!(super::check_projection(&original, &projection, "items").is_err());
        projection["items"][0]["result"] = serde_json::json!("fail");
        projection["items"][0]["findings"] = serde_json::json!("");
        assert!(super::check_projection(&original, &projection, "items").is_err());
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
