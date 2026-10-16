//! Provider-owned narrative binding contract, not workflow evaluation.
//!
//! `binding_schema` compiles the bounded shape schema once. `validate_binding`
//! additionally checks the composite identity, exact phase file pairing and
//! SHA-256 encoding, which the bounded schema language cannot express.
//! No helper here reads files or proves review clearance, admission, freshness,
//! or that the cited approved-artifact record exists; gate evaluation owns those.

#![allow(dead_code)]

use crate::schema::{validate_schema, InstanceViolation, ValidatedSchema};
use crate::workflow;
use loop_core::ContextRecord;
use serde_json::Value;
use std::sync::OnceLock;

pub(crate) const NARRATIVE_PHASES: &[&str] = &["design", "plan", "validation"];
pub(crate) const CHECKPOINT_PHASES: &[&str] =
    &["intent", "design", "plan", "implementation", "validation"];

/// The canonical schema cannot be replaced by caller `artifact_schemas`.
pub(crate) fn binding_schema() -> &'static ValidatedSchema {
    static SCHEMA: OnceLock<ValidatedSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let value: Value = serde_json::from_str(include_str!("../data/narrative-schema.json"))
            .expect("narrative schema is valid JSON");
        validate_schema(&value).expect("narrative schema is valid provider schema")
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NarrativeAuthor<'a> {
    pub(crate) name: &'a str,
    pub(crate) kind: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactBinding<'a> {
    pub(crate) subject: &'a str,
    pub(crate) revision: &'a str,
    pub(crate) sha256: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownBinding<'a> {
    pub(crate) file: &'a str,
    pub(crate) sha256: &'a str,
}

/// Borrowed checked shape; not evidence that this binding has been admitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NarrativeBinding<'a> {
    pub(crate) revision: &'a str,
    pub(crate) author: NarrativeAuthor<'a>,
    pub(crate) artifact: ArtifactBinding<'a>,
    pub(crate) approved_artifact_record_id: &'a str,
    pub(crate) markdown: MarkdownBinding<'a>,
}

/// Validate closed shape plus binding-local invariants. The counter is a
/// positive decimal without leading zeroes, and the prefix equals the exact
/// artifact revision (which remains an opaque string). Digests use 64 lowercase
/// hexadecimal characters. Callers separately compare these claims with files
/// and the latest approved/admitted context records.
pub(crate) fn validate_binding(
    value: &Value,
) -> Result<NarrativeBinding<'_>, Vec<InstanceViolation>> {
    let report = binding_schema().evaluate(value);
    if !report.is_valid() {
        return Err(report.into_violations());
    }
    fn string(v: &Value) -> &str {
        v.as_str().expect("schema checked string")
    }
    let binding = NarrativeBinding {
        revision: string(&value["revision"]),
        author: NarrativeAuthor {
            name: string(&value["author"]["name"]),
            kind: string(&value["author"]["kind"]),
        },
        artifact: ArtifactBinding {
            subject: string(&value["artifact"]["subject"]),
            revision: string(&value["artifact"]["revision"]),
            sha256: string(&value["artifact"]["sha256"]),
        },
        approved_artifact_record_id: string(&value["approved_artifact_record_id"]),
        markdown: MarkdownBinding {
            file: string(&value["markdown"]["file"]),
            sha256: string(&value["markdown"]["sha256"]),
        },
    };
    let mut violations = Vec::new();
    let composite_valid = binding
        .revision
        .rsplit_once('/')
        .is_some_and(|(prefix, counter)| {
            prefix == binding.artifact.revision
                && counter.starts_with(|c: char| c.is_ascii_digit() && c != '0')
                && counter.bytes().all(|b| b.is_ascii_digit())
        });
    if !composite_valid {
        violations.push(InstanceViolation {
            path: "/revision".to_owned(),
            rule: "narrative-revision".to_owned(),
            message: "revision must be '<artifact revision>/<positive decimal counter>' without leading zeroes".to_owned(),
        });
    }
    let expected = match binding.artifact.subject {
        "design.json" => "design-narrative.md",
        "plan.json" => "plan-narrative.md",
        "validation-report.json" => "validation-narrative.md",
        _ => unreachable!("schema checked subject"),
    };
    if binding.markdown.file != expected {
        violations.push(InstanceViolation {
            path: "/markdown/file".to_owned(),
            rule: "narrative-file".to_owned(),
            message: format!("artifact subject requires markdown file `{expected}`"),
        });
    }
    for (path, digest) in [
        ("/artifact/sha256", binding.artifact.sha256),
        ("/markdown/sha256", binding.markdown.sha256),
    ] {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            violations.push(InstanceViolation {
                path: path.to_owned(),
                rule: "sha256".to_owned(),
                message: "sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            });
        }
    }
    if violations.is_empty() {
        Ok(binding)
    } else {
        Err(violations)
    }
}

/// Select a fact without inventing a context record or ID. A colliding hop
/// stores fixed optional facts inside its original single append. The actual
/// enclosing record's identity and sequence remain authoritative.
pub(crate) fn phase_record_fact<'a>(record: &'a ContextRecord, kind: &str) -> Option<&'a Value> {
    if record.kind == kind {
        return Some(&record.data);
    }
    match (record.kind.as_str(), kind) {
        (workflow::RECONCILIATION_DECISION_KIND, workflow::ARTIFACT_ADMITTED_KIND) => {
            record.data.get("artifact_admitted")
        }
        (
            workflow::RECONCILIATION_DECISION_KIND | workflow::ARTIFACT_ADMITTED_KIND,
            workflow::APPROVED_ARTIFACT_KIND,
        ) => record.data.get("approved_artifact"),
        _ => None,
    }
}

pub(crate) fn latest_phase_record<'a>(
    context: &'a [ContextRecord],
    kind: &str,
    phase: &str,
    subject: &str,
) -> Option<(&'a ContextRecord, &'a Value)> {
    context
        .iter()
        .filter_map(|r| {
            let fact = phase_record_fact(r, kind)?;
            ((kind == workflow::APPROVED_ARTIFACT_KIND || fact["phase"] == phase)
                && (kind == workflow::NARRATIVE_ADMITTED_KIND || fact["subject"] == subject))
                .then_some((r, fact))
        })
        .max_by_key(|(r, _)| r.sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example(subject: &str, file: &str) -> Value {
        json!({
            "revision": "3/1",
            "author": {"name": "writer", "kind": "agent"},
            "artifact": {"subject": subject, "revision": "3", "sha256": "a".repeat(64)},
            "approved_artifact_record_id": "approved-design-3",
            "markdown": {"file": file, "sha256": "b".repeat(64)}
        })
    }

    #[test]
    fn narrative_schema_and_helper_accept_each_phase() {
        for (subject, file) in [
            ("design.json", "design-narrative.md"),
            ("plan.json", "plan-narrative.md"),
            ("validation-report.json", "validation-narrative.md"),
        ] {
            let value = example(subject, file);
            assert!(binding_schema().evaluate(&value).is_valid());
            let binding = validate_binding(&value).unwrap();
            assert_eq!(binding.revision, "3/1");
            assert_eq!(binding.artifact.subject, subject);
            assert_eq!(binding.markdown.file, file);
        }
    }

    #[test]
    fn narrative_binding_rejects_shape_and_local_invariants() {
        let original = example("design.json", "design-narrative.md");
        for (pointer, bad) in [
            ("/revision", json!("2/1")),
            ("/revision", json!("3")),
            ("/revision", json!("3/0")),
            ("/revision", json!("3/01")),
            ("/revision", json!("3/-1")),
            ("/revision", json!("3/1x")),
            ("/artifact/revision", json!("")),
            ("/artifact/subject", json!("intent.json")),
            ("/artifact/sha256", json!("a".repeat(63))),
            ("/markdown/sha256", json!("A".repeat(64))),
            ("/markdown/sha256", json!("g".repeat(64))),
            ("/markdown/file", json!("plan-narrative.md")),
            ("/markdown/file", json!("../design-narrative.md")),
            ("/approved_artifact_record_id", json!("")),
            ("/author/kind", json!("unknown")),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(validate_binding(&value).is_err(), "{pointer}: {value}");
        }
        for pointer in ["", "/author", "/artifact", "/markdown"] {
            let mut value = original.clone();
            value.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
            assert!(validate_binding(&value).is_err(), "{pointer}");
        }
        for field in [
            "revision",
            "author",
            "artifact",
            "approved_artifact_record_id",
            "markdown",
        ] {
            let mut value = original.clone();
            value.as_object_mut().unwrap().remove(field);
            assert!(validate_binding(&value).is_err(), "{field}");
        }
    }

    #[test]
    fn narrative_counter_preserves_opaque_artifact_revision() {
        let mut value = example("design.json", "design-narrative.md");
        value["artifact"]["revision"] = json!("release/3");
        value["revision"] = json!("release/3/2");
        assert!(validate_binding(&value).is_ok());
    }
}
