//! Criterion currency for carried review evidence.
//!
//! Evidence "cites" a criterion when its grounds locate an acceptance entry
//! (`intent.json#/acceptance/N`), the whole acceptance array, or a line range that
//! overlaps one or more entries. A citation of the array or of a range counts as
//! citing every criterion it holds. A carry keeps working while the cited
//! criteria are unchanged; changed text on any cited criterion makes it stale.
//! Judgments that cite no acceptance entry are not tied to a criterion.

use loop_core::ContextRecord;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

enum Cited {
    All,
    Index(usize),
    Lines(usize, usize),
}

fn cited(grounds: &Value) -> Vec<Cited> {
    let mut cites = Vec::new();
    for reference in grounds
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some((file, selector)) = reference
            .get("locator")
            .and_then(Value::as_str)
            .and_then(|locator| locator.split_once('#'))
        else {
            continue;
        };
        if file != "intent.json" {
            continue;
        }
        if let Some(pointer) = selector.strip_prefix('/') {
            let mut tokens = pointer.split('/');
            if tokens.next() == Some("acceptance") {
                match tokens.next() {
                    None => cites.push(Cited::All),
                    Some(index) => {
                        if let Ok(index) = index.parse() {
                            cites.push(Cited::Index(index));
                        }
                    }
                }
            }
        } else if let Some(range) = selector.strip_prefix('L') {
            let (start, end) = range.split_once('-').unwrap_or((range, range));
            if let (Ok(start), Ok(end)) = (start.parse(), end.trim_start_matches('L').parse()) {
                cites.push(Cited::Lines(start, end));
            }
        }
    }
    cites
}

/// Start and end line (1-based, inclusive) of each element of the top-level
/// `acceptance` array, in order.
fn acceptance_line_spans(text: &str) -> Vec<(usize, usize)> {
    let (mut line, mut in_string, mut escaped) = (1usize, false, false);
    let (mut string, mut last_string, mut key) = (String::new(), String::new(), String::new());
    let mut stack: Vec<char> = Vec::new();
    let (mut acceptance_level, mut open_at) = (0usize, 0usize);
    let mut spans = Vec::new();
    for c in text.chars() {
        if c == '\n' {
            line += 1;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                last_string = std::mem::take(&mut string);
            } else {
                string.push(c);
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            ':' if stack.len() == 1 => key = last_string.clone(),
            '[' | '{' => {
                if c == '[' && stack.len() == 1 && key == "acceptance" && acceptance_level == 0 {
                    acceptance_level = 2;
                }
                if c == '{' && acceptance_level == 2 && stack.len() == 2 {
                    open_at = line;
                }
                stack.push(c);
            }
            ']' | '}' => {
                stack.pop();
                if c == '}' && acceptance_level == 2 && stack.len() == 2 {
                    spans.push((open_at, line));
                }
                if c == ']' && acceptance_level == 2 && stack.len() == 1 {
                    acceptance_level = 3;
                }
            }
            _ => {}
        }
    }
    spans
}

fn entries(intent: &Value) -> Vec<(String, Value)> {
    intent
        .get("acceptance")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some((entry.get("id")?.as_str()?.to_owned(), entry.clone())))
        .collect()
}

fn retained_intent(context: &[ContextRecord], record: &ContextRecord) -> Option<String> {
    let entry = record.data["artifacts"]
        .as_array()?
        .iter()
        .find(|entry| entry["name"] == "intent.json")?;
    match entry["same_as"].as_str() {
        Some(holder) => retained_intent(
            context,
            context.iter().find(|row| row.id.as_str() == holder)?,
        ),
        None => entry["content"].as_str().map(str::to_owned),
    }
}

fn revision_of(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("revision")?
        .as_str()
        .map(str::to_owned)
}

/// Exact earlier intent bytes for `revision`, from the exact-prior-intent bytes
/// of a correction or steering record, or a retained copy.
fn intent_at_revision(context: &[ContextRecord], revision: &str) -> Option<String> {
    context
        .iter()
        .find_map(|record| match record.kind.as_str() {
            "intent-correction" if record.data["prior_intent"]["revision"] == revision => record
                .data["prior_intent"]["json_bytes"]
                .as_str()
                .map(str::to_owned),
            "user-steering" if record.data["intent_baseline"]["revision"] == revision => record
                .data["intent_baseline"]["json_bytes"]
                .as_str()
                .map(str::to_owned),
            "retained-artifacts" => retained_intent(context, record)
                .filter(|text| revision_of(text).as_deref() == Some(revision)),
            _ => None,
        })
}

/// Criteria cited by a carried judgment whose text changed since the judgment
/// was written. `Ok(empty)` keeps the carry; `Err` means currency could not be
/// verified and must not be claimed.
pub(crate) fn changed_cited_criteria(
    grounds: Option<&Value>,
    source: &ContextRecord,
    context: &[ContextRecord],
    artifact_root: Option<&Value>,
) -> Result<Vec<String>, String> {
    let Some(grounds) = grounds else {
        return Ok(Vec::new());
    };
    let cites = cited(grounds);
    let Some(root) = artifact_root.and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    if cites.is_empty() {
        return Ok(Vec::new());
    }
    let Ok(current_text) = std::fs::read_to_string(std::path::Path::new(root).join("intent.json"))
    else {
        return Ok(Vec::new());
    };
    let Ok(current) = serde_json::from_str::<Value>(&current_text) else {
        return Ok(Vec::new());
    };
    let current_revision = current["revision"].as_str().unwrap_or_default();
    let earlier = context.iter().filter(|row| row.sequence < source.sequence);
    let origin_text = if source.data["subject"] == "intent.json" {
        let revision = source.data["subject_revision"].as_str().unwrap_or_default();
        if revision == current_revision {
            return Ok(Vec::new());
        }
        intent_at_revision(context, revision).ok_or_else(|| {
            format!(
                "unable to verify carried judgment `{}`: its cited criteria cannot be compared because no exact bytes of intent revision `{revision}` are retained",
                source.id
            )
        })?
    } else {
        // Other subjects: the intent that was current is the latest retained
        // copy before the judgment. Without one the judgment stays driver-attested.
        let earlier: Vec<_> = earlier.cloned().collect();
        let Some(text) = earlier
            .iter()
            .rev()
            .filter(|row| row.kind == "retained-artifacts")
            .find_map(|row| retained_intent(context, row))
        else {
            return Ok(Vec::new());
        };
        if revision_of(&text).as_deref() == Some(current_revision) {
            return Ok(Vec::new());
        }
        text
    };
    let origin: Value = serde_json::from_str(&origin_text)
        .map_err(|error| format!("unable to verify carried judgment `{}`: {error}", source.id))?;
    let before: BTreeMap<String, Value> = entries(&origin).into_iter().collect();
    let after: BTreeMap<String, Value> = entries(&current).into_iter().collect();
    let origin_ids: Vec<String> = entries(&origin).into_iter().map(|(id, _)| id).collect();
    let spans = acceptance_line_spans(&origin_text);
    let mut ids = BTreeSet::new();
    for cite in cites {
        match cite {
            Cited::All => ids.extend(before.keys().chain(after.keys()).cloned()),
            Cited::Index(index) => ids.extend(origin_ids.get(index).cloned()),
            Cited::Lines(start, end) => {
                for (index, (first, last)) in spans.iter().enumerate() {
                    if *first <= end && start <= *last {
                        ids.extend(origin_ids.get(index).cloned());
                    }
                }
            }
        }
    }
    Ok(ids
        .into_iter()
        .filter(|id| before.get(id) != after.get(id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;

    fn row(id: &str, kind: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn intent(revision: &str, second: &str) -> String {
        serde_json::to_string_pretty(&json!({
            "revision": revision,
            "outcome": "x",
            "acceptance": [
                {"id": "AC-1", "statement": "first"},
                {"id": "AC-2", "statement": second},
                {"id": "AC-3", "statement": "third"}
            ]
        }))
        .unwrap()
    }

    fn grounds(locator: &str) -> Value {
        json!({"reason": "checked", "evidence": [{"locator": locator}]})
    }

    /// Directory holding the current intent, plus a correction record that
    /// keeps the exact bytes of revision 1.
    fn fixture(current: &str) -> (std::path::PathBuf, Value, Vec<ContextRecord>, ContextRecord) {
        let dir = std::env::temp_dir().join(format!(
            "software-change-currency-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("intent.json"), current).unwrap();
        let source = row(
            "source",
            "review-evidence",
            2,
            json!({"subject": "intent.json", "subject_revision": "1"}),
        );
        let correction = row(
            "correction",
            "intent-correction",
            5,
            json!({"prior_intent": {"revision": "1", "json_bytes": intent("1", "second")}}),
        );
        (
            dir.clone(),
            json!(dir),
            vec![source.clone(), correction],
            source,
        )
    }

    #[test]
    fn revision_only_change_keeps_every_carry_and_text_change_names_the_criterion() {
        let (dir, root, context, source) = fixture(&intent("2", "second"));
        for locator in [
            "intent.json#/acceptance/1",
            "intent.json#/acceptance",
            "intent.json#L1-L30",
        ] {
            assert_eq!(
                changed_cited_criteria(Some(&grounds(locator)), &source, &context, Some(&root)),
                Ok(Vec::new()),
                "{locator}"
            );
        }
        std::fs::write(dir.join("intent.json"), intent("2", "second, now stricter")).unwrap();
        let g = |l: &str| changed_cited_criteria(Some(&grounds(l)), &source, &context, Some(&root));
        // An entry citation of a changed criterion is stale, naming it.
        assert_eq!(g("intent.json#/acceptance/1"), Ok(vec!["AC-2".to_owned()]));
        // An entry citation of an unchanged criterion stays current.
        assert_eq!(g("intent.json#/acceptance/0"), Ok(Vec::new()));
        // The whole array, or a range holding the changed entry, cites every criterion it holds.
        assert_eq!(g("intent.json#/acceptance"), Ok(vec!["AC-2".to_owned()]));
        assert_eq!(g("intent.json#L1-L30"), Ok(vec!["AC-2".to_owned()]));
        // A range that holds only unchanged entries stays current.
        let spans = acceptance_line_spans(&intent("1", "second"));
        assert_eq!(spans.len(), 3);
        let (first_start, first_end) = spans[0];
        assert_eq!(
            g(&format!("intent.json#L{first_start}-L{first_end}")),
            Ok(Vec::new())
        );
        // Judgments citing no criterion are not tied to one.
        assert_eq!(g("intent.json#/outcome"), Ok(Vec::new()));
        // Missing earlier bytes are reported as unable to verify, never as current.
        let without = vec![source.clone()];
        let error = changed_cited_criteria(
            Some(&grounds("intent.json#/acceptance/1")),
            &source,
            &without,
            Some(&root),
        )
        .unwrap_err();
        assert!(
            error.contains("unable to verify") && error.contains("revision `1`"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
