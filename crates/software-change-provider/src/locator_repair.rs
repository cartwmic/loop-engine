//! Provider-checked locator repair for one reviewer return.
//!
//! A driver may hand `review-candidates` the reviewer's original structured return and a
//! corrected copy. This module decides whether the two differ ONLY in evidence locators and
//! only in the permitted way. It reads sources to resolve locators; it never writes, never
//! executes anything and never suggests which pointer the reviewer meant.
//!
//! Rules (all checked; every difference found is named in the refusal):
//! * the original is valid in every respect except that some evidence locators do not resolve
//!   (shape, closed evidence entries, duplicates, every non-evidence field, every judgment);
//! * top-level fields other than `judgments` are equal;
//! * judgments match one to one, in order, and every field except `grounds.evidence` is equal;
//! * a judgment with no unresolvable locator keeps its evidence unchanged;
//! * otherwise the locators that already resolve stay, in their original order; each
//!   unresolvable one is replaced (replacements are taken in order) or dropped; nothing else
//!   is added; every corrected locator resolves; at least one entry remains.
use loop_core::ContextRecord;
use serde_json::{Map, Value};
use std::path::Path;

/// One before/after for a corrected or dropped locator.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    pub axis: String,
    pub before: String,
    /// `None` when the locator was dropped rather than replaced.
    pub after: Option<String>,
}

fn axis_of(judgment: &Value, index: usize) -> String {
    judgment
        .get("axis")
        .and_then(Value::as_str)
        .map_or_else(|| format!("#{index}"), str::to_owned)
}

fn evidence_locators(grounds: &Value) -> Option<Vec<String>> {
    grounds
        .get("evidence")?
        .as_array()?
        .iter()
        .map(|entry| {
            let object = entry.as_object()?;
            if object.len() != 1 {
                return None;
            }
            object.get("locator")?.as_str().map(str::to_owned)
        })
        .collect()
}

fn without<'a>(object: &'a Map<String, Value>, key: &str) -> Vec<(&'a String, &'a Value)> {
    object.iter().filter(|(k, _)| k.as_str() != key).collect()
}

/// Check a repair. `Ok` carries the corrections (empty for judgments that were already
/// fine); `Err` lists every reason the request is refused.
pub fn check(
    original: &Value,
    corrected: &Value,
    root: &Path,
    context: &[ContextRecord],
) -> Result<Vec<Correction>, String> {
    let (Some(o), Some(c)) = (original.as_object(), corrected.as_object()) else {
        return Err("original and repaired returns must both be structured JSON objects".into());
    };
    let (Some(oj), Some(cj)) = (
        o.get("judgments").and_then(Value::as_array),
        c.get("judgments").and_then(Value::as_array),
    ) else {
        return Err("original and repaired returns must both contain a judgments array".into());
    };
    let mut differences: Vec<String> = Vec::new();
    if without(o, "judgments") != without(c, "judgments") {
        let mut keys: Vec<&String> = o
            .keys()
            .chain(c.keys())
            .filter(|key| key.as_str() != "judgments" && o.get(*key) != c.get(*key))
            .collect();
        keys.sort();
        keys.dedup();
        let named: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();
        differences.push(format!(
            "a top-level field other than judgments differs: {} (author, stage, group, subject, revision, contract version or assessment identity may not change)",
            named.join(", ")
        ));
    }
    if oj.len() != cj.len() {
        differences.push(format!(
            "the repaired return has {} judgments but the original has {}; judgments may not be added, removed or reordered",
            cj.len(),
            oj.len()
        ));
        return Err(differences.join("; "));
    }
    let mut corrections = Vec::new();
    let mut any_unresolvable = false;
    for (index, (before, after)) in oj.iter().zip(cj).enumerate() {
        let axis = axis_of(before, index);
        let (Some(bo), Some(ao)) = (before.as_object(), after.as_object()) else {
            differences.push(format!("judgment {axis}: both versions must be objects"));
            continue;
        };
        if axis_of(after, index) != axis {
            differences.push(format!(
                "judgment {index} changes axis from {axis} to {}; judgments may not be reordered or relabeled",
                axis_of(after, index)
            ));
            continue;
        }
        for key in bo
            .keys()
            .chain(ao.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            if key != "grounds" && bo.get(key) != ao.get(key) {
                differences.push(format!(
                    "judgment {axis}: field {key} differs; only evidence locators may change"
                ));
            }
        }
        let (Some(bg), Some(ag)) = (
            before.get("grounds").filter(|g| g.is_object()),
            after.get("grounds").filter(|g| g.is_object()),
        ) else {
            differences.push(format!(
                "judgment {axis}: the original has no grounds object, so it is not a locator-only defect (reused and ungrounded rows cannot be repaired)"
            ));
            continue;
        };
        if without(bg.as_object().unwrap(), "evidence")
            != without(ag.as_object().unwrap(), "evidence")
        {
            differences.push(format!(
                "judgment {axis}: grounds other than evidence differ (the reason may not change)"
            ));
        }
        // The original must be valid except for locator resolution.
        let unresolvable = match crate::criterion::unresolvable_locators(bg, root, Some(3), context)
        {
            Ok(found) => found,
            Err(error) => {
                differences.push(format!(
                    "judgment {axis}: the original has a defect other than unresolvable locators: {error}"
                ));
                continue;
            }
        };
        let (Some(original_locators), Some(corrected_locators)) =
            (evidence_locators(bg), evidence_locators(ag))
        else {
            differences.push(format!(
                "judgment {axis}: every evidence entry must be exactly {{\"locator\": ...}}"
            ));
            continue;
        };
        if corrected_locators.is_empty() {
            differences.push(format!(
                "judgment {axis}: no evidence entry remains; at least one resolving entry must be kept"
            ));
            continue;
        }
        let bad: Vec<&(usize, String, String)> = unresolvable.iter().collect();
        if bad.is_empty() {
            if original_locators != corrected_locators {
                differences.push(format!(
                    "judgment {axis}: it has no unresolvable locator, so its evidence may not change"
                ));
            }
        } else {
            any_unresolvable = true;
            let bad_set: std::collections::BTreeSet<&str> =
                bad.iter().map(|(_, locator, _)| locator.as_str()).collect();
            let keep: Vec<&String> = original_locators
                .iter()
                .filter(|l| !bad_set.contains(l.as_str()))
                .collect();
            let kept_in_corrected: Vec<&String> = corrected_locators
                .iter()
                .filter(|l| keep.contains(l))
                .collect();
            if kept_in_corrected != keep {
                differences.push(format!(
                    "judgment {axis}: locators that already resolve must be kept, in their original order"
                ));
            }
            let replacements: Vec<&String> = corrected_locators
                .iter()
                .filter(|l| !keep.contains(l))
                .collect();
            if replacements.len() > bad.len() {
                differences.push(format!(
                    "judgment {axis}: {} replacement locators were supplied for {} unresolvable ones; evidence may not be added",
                    replacements.len(),
                    bad.len()
                ));
            }
            for (slot, (_, locator, _)) in bad.iter().enumerate() {
                corrections.push(Correction {
                    axis: axis.clone(),
                    before: locator.clone(),
                    after: replacements.get(slot).map(|l| (*l).clone()),
                });
            }
        }
        // Every corrected locator must resolve (and the grounds must stay well formed).
        match crate::criterion::unresolvable_locators(ag, root, Some(3), context) {
            Err(error) => differences.push(format!(
                "judgment {axis}: the repaired evidence is malformed: {error}"
            )),
            Ok(still) => {
                for (_, locator, error) in still {
                    differences.push(format!(
                        "judgment {axis}: corrected locator {locator} still does not resolve: {error}"
                    ));
                }
            }
        }
    }
    if differences.is_empty() && !any_unresolvable {
        differences.push(
            "the original return has no unresolvable evidence locator; there is nothing to repair"
                .into(),
        );
    }
    if differences.is_empty() {
        Ok(corrections)
    } else {
        Err(differences.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    struct Fixture {
        root: std::path::PathBuf,
    }
    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "software-change-locator-repair-{tag}-{}-{:?}",
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
                serde_json::to_vec(
                    &json!({"revision":"1","outcome":"x","acceptance":acceptance,
                    "operating_context":{"outside_obligations":["a","b","c","d"]}}),
                )
                .unwrap(),
            )
            .unwrap();
            Fixture { root }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn judgment(axis: &str, locators: &[&str]) -> Value {
        json!({"axis":axis,"result":"pass","findings":"","grounds":{"reason":"checked the intent",
            "evidence": locators.iter().map(|l| json!({"locator": l})).collect::<Vec<_>>()}})
    }
    fn batch(judgments: Vec<Value>) -> Value {
        json!({"review_contract_version":3,"review_stage":"aggregate",
            "author":{"name":"reviewer","kind":"agent"},"judgments":judgments})
    }

    #[test]
    fn the_two_observed_pointers_are_repaired_by_replacement_and_by_dropping() {
        let f = Fixture::new("observed");
        let original = batch(vec![
            judgment(
                "scope-fenced",
                &["intent.json#/outcome", "intent.json#/acceptance/12"],
            ),
            judgment(
                "problem-grounded",
                &["intent.json#/outside_obligations/3", "intent.json#/outcome"],
            ),
        ]);
        let corrected = batch(vec![
            judgment(
                "scope-fenced",
                &["intent.json#/outcome", "intent.json#/acceptance/11"],
            ),
            judgment("problem-grounded", &["intent.json#/outcome"]),
        ]);
        let corrections = check(&original, &corrected, &f.root, &[]).unwrap();
        assert_eq!(
            corrections,
            vec![
                Correction {
                    axis: "scope-fenced".into(),
                    before: "intent.json#/acceptance/12".into(),
                    after: Some("intent.json#/acceptance/11".into())
                },
                Correction {
                    axis: "problem-grounded".into(),
                    before: "intent.json#/outside_obligations/3".into(),
                    after: None
                },
            ]
        );
    }

    fn refused(f: &Fixture, original: Value, corrected: Value, why: &str) {
        let error = check(&original, &corrected, &f.root, &[]).unwrap_err();
        assert!(error.contains(why), "expected {why:?} in {error:?}");
    }

    #[test]
    fn anything_beyond_locators_is_refused_and_named() {
        let f = Fixture::new("refused");
        let bad = ["intent.json#/outcome", "intent.json#/acceptance/12"];
        let good = ["intent.json#/outcome", "intent.json#/acceptance/11"];
        let original = batch(vec![
            judgment("a", &bad),
            judgment("b", &["intent.json#/outcome"]),
        ]);
        let fix = |edit: &dyn Fn(&mut Value)| {
            let mut c = batch(vec![
                judgment("a", &good),
                judgment("b", &["intent.json#/outcome"]),
            ]);
            edit(&mut c);
            c
        };
        refused(
            &f,
            original.clone(),
            fix(&|c| c["judgments"][0]["result"] = json!("fail")),
            "field result differs",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["judgments"][0]["findings"] = json!("new")),
            "field findings differs",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["judgments"][0]["grounds"]["reason"] = json!("other")),
            "reason may not change",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["judgments"][0]["axis"] = json!("z")),
            "changes axis",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["author"]["name"] = json!("someone")),
            "top-level field",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["assessment_id"] = json!("new")),
            "top-level field",
        );
        for key in ["group", "subject", "assessment_id"] {
            refused(
                &f,
                original.clone(),
                fix(&|c| c[key] = json!("changed")),
                &format!("differs: `{key}`"),
            );
        }
        refused(
            &f,
            original.clone(),
            fix(&|c| {
                c["judgments"].as_array_mut().unwrap().pop();
            }),
            "may not be added, removed or reordered",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| {
                c["judgments"].as_array_mut().unwrap().swap(0, 1);
            }),
            "changes axis",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| {
                c["judgments"][1]["grounds"]["evidence"] =
                    json!([{"locator":"intent.json#/acceptance/1"}])
            }),
            "may not change",
        );
        refused(
            &f,
            original.clone(),
            fix(
                &|c| c["judgments"][0]["grounds"]["evidence"] = json!([{"locator":"intent.json#/outcome"},{"locator":"intent.json#/acceptance/11"},{"locator":"intent.json#/acceptance/10"}]),
            ),
            "evidence may not be added",
        );
        refused(
            &f,
            original.clone(),
            fix(
                &|c| c["judgments"][0]["grounds"]["evidence"] = json!([{"locator":"intent.json#/outcome"},{"locator":"intent.json#/acceptance/13"}]),
            ),
            "still does not resolve",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| c["judgments"][0]["grounds"]["evidence"] = json!([])),
            "no evidence entry remains",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| {
                c["judgments"][0]["grounds"]["evidence"] =
                    json!([{"locator":"intent.json#/acceptance/11"}])
            }),
            "already resolve must be kept",
        );
        refused(
            &f,
            original.clone(),
            fix(&|c| {
                c["judgments"][0]["grounds"]["evidence"][0] =
                    json!({"locator":"intent.json#/outcome","sha256":"x"})
            }),
            "must be exactly",
        );
        // Nothing to repair.
        refused(
            &f,
            batch(vec![judgment("a", &["intent.json#/outcome"])]),
            batch(vec![judgment("a", &["intent.json#/outcome"])]),
            "nothing to repair",
        );
    }

    #[test]
    fn a_non_locator_defect_in_the_original_is_never_erased_by_the_correction() {
        let f = Fixture::new("original-defect");
        // Two identical unresolvable locators replaced and dropped into a valid list.
        refused(
            &f,
            batch(vec![judgment(
                "a",
                &["intent.json#/acceptance/12", "intent.json#/acceptance/12"],
            )]),
            batch(vec![judgment("a", &["intent.json#/acceptance/11"])]),
            "defect other than unresolvable locators",
        );
        // A sibling judgment with a non-locator defect rides along with a repaired one.
        let mut sibling = judgment("b", &["intent.json#/outcome"]);
        sibling["grounds"]["reason"] = json!("");
        refused(
            &f,
            batch(vec![
                judgment("a", &["intent.json#/acceptance/12"]),
                sibling.clone(),
            ]),
            batch(vec![
                judgment("a", &["intent.json#/acceptance/11"]),
                sibling,
            ]),
            "defect other than unresolvable locators",
        );
        // A reuse row has no grounds and cannot be repaired.
        refused(
            &f,
            batch(vec![
                judgment("a", &["intent.json#/acceptance/12"]),
                json!({"axis":"b","reuse":"carry"}),
            ]),
            batch(vec![
                judgment("a", &["intent.json#/acceptance/11"]),
                json!({"axis":"b","reuse":"carry"}),
            ]),
            "no grounds object",
        );
    }
}
