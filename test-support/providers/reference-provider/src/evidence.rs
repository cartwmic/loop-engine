//! Provider-owned artifact and evidence conventions.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::protocol::{DiagnosticDto, EvidenceDto};

pub const INPUT_ARTIFACT_ROOT: &str = "artifact_root";

pub const KIND_INTENT: &str = "intent-document";
pub const KIND_DESIGN: &str = "design-document";
pub const KIND_DESIGN_REVIEW: &str = "design-review";
pub const KIND_PLAN: &str = "plan-document";
pub const KIND_PLAN_REVIEW: &str = "plan-review";
pub const KIND_IMPLEMENTATION: &str = "implementation-report";
pub const KIND_IMPLEMENTATION_REVIEW: &str = "implementation-review";
pub const KIND_VALIDATION: &str = "validation-report";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    pub relative_path: &'static str,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewVerdict {
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationVerdict {
    Passed,
    Failed,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LinkedDoc {
    revision: String,
    intent_revision: Option<String>,
    pub(crate) plan_revision: Option<String>,
    pub(crate) subject_revision: Option<String>,
    verdict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    Missing {
        path: String,
        code: &'static str,
        message: String,
    },
    Invalid {
        path: String,
        code: &'static str,
        message: String,
    },
    RevisionMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    VerdictMismatch {
        path: String,
        expected: &'static str,
        actual: String,
    },
}

impl ArtifactError {
    pub fn diagnostic(&self) -> DiagnosticDto {
        match self {
            Self::Missing {
                path,
                code,
                message,
            } => DiagnosticDto {
                code: (*code).to_string(),
                message: message.clone(),
                path: Some(path.clone()),
            },
            Self::Invalid {
                path,
                code,
                message,
            } => DiagnosticDto {
                code: (*code).to_string(),
                message: message.clone(),
                path: Some(path.clone()),
            },
            Self::RevisionMismatch {
                path,
                expected,
                actual,
            } => DiagnosticDto {
                code: "artifact.revision_mismatch".to_string(),
                message: format!("expected revision {expected}, found {actual}"),
                path: Some(path.clone()),
            },
            Self::VerdictMismatch {
                path,
                expected,
                actual,
            } => DiagnosticDto {
                code: "artifact.verdict_mismatch".to_string(),
                message: format!("expected verdict {expected}, found {actual}"),
                path: Some(path.clone()),
            },
        }
    }
}

pub fn artifact_root_from_inputs(
    inputs: &BTreeMap<String, Value>,
) -> Result<PathBuf, DiagnosticDto> {
    let Some(value) = inputs.get(INPUT_ARTIFACT_ROOT) else {
        return Err(DiagnosticDto {
            code: "input.missing".to_string(),
            message: "artifact_root input is required".to_string(),
            path: Some("/inputs/artifact_root".to_string()),
        });
    };
    let Some(path) = value.as_str() else {
        return Err(DiagnosticDto {
            code: "input.invalid".to_string(),
            message: "artifact_root must be a string path".to_string(),
            path: Some("/inputs/artifact_root".to_string()),
        });
    };
    if path.is_empty() {
        return Err(DiagnosticDto {
            code: "input.invalid".to_string(),
            message: "artifact_root must not be empty".to_string(),
            path: Some("/inputs/artifact_root".to_string()),
        });
    }
    Ok(PathBuf::from(path))
}

pub fn read_revision_doc(root: &Path, artifact: &ArtifactRef) -> Result<LinkedDoc, ArtifactError> {
    let path = root.join(artifact.relative_path);
    let display = path.display().to_string();
    let raw = fs::read_to_string(&path).map_err(|_| ArtifactError::Missing {
        path: display.clone(),
        code: "artifact.missing",
        message: format!("expected artifact at {}", artifact.relative_path),
    })?;
    serde_json::from_str::<LinkedDoc>(&raw).map_err(|err| ArtifactError::Invalid {
        path: display,
        code: "artifact.invalid",
        message: err.to_string(),
    })
}

pub fn require_revision(doc: &LinkedDoc, path: &str) -> Result<String, ArtifactError> {
    if doc.revision.is_empty() {
        return Err(ArtifactError::Invalid {
            path: path.to_string(),
            code: "artifact.invalid",
            message: "revision must not be empty".to_string(),
        });
    }
    Ok(doc.revision.clone())
}

pub fn require_linkage(
    doc: &LinkedDoc,
    path: &str,
    field: &str,
    expected: &str,
) -> Result<(), ArtifactError> {
    let actual = match field {
        "intent_revision" => doc.intent_revision.as_deref(),
        "plan_revision" => doc.plan_revision.as_deref(),
        "subject_revision" => doc.subject_revision.as_deref(),
        other => {
            return Err(ArtifactError::Invalid {
                path: path.to_string(),
                code: "artifact.invalid",
                message: format!("unknown linkage field {other}"),
            });
        }
    }
    .unwrap_or("");
    if actual != expected {
        return Err(ArtifactError::RevisionMismatch {
            path: path.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

pub fn require_review_verdict(
    doc: &LinkedDoc,
    path: &str,
    expected: ReviewVerdict,
) -> Result<(), ArtifactError> {
    let actual = doc.verdict.as_deref().unwrap_or("");
    let expected_str = match expected {
        ReviewVerdict::Approved => "approved",
        ReviewVerdict::ChangesRequested => "changes_requested",
    };
    if actual != expected_str {
        return Err(ArtifactError::VerdictMismatch {
            path: path.to_string(),
            expected: expected_str,
            actual: actual.to_string(),
        });
    }
    Ok(())
}

pub fn require_validation_verdict(
    doc: &LinkedDoc,
    path: &str,
    expected: ValidationVerdict,
) -> Result<(), ArtifactError> {
    let actual = doc.verdict.as_deref().unwrap_or("");
    let expected_str = match expected {
        ValidationVerdict::Passed => "passed",
        ValidationVerdict::Failed => "failed",
    };
    if actual != expected_str {
        return Err(ArtifactError::VerdictMismatch {
            path: path.to_string(),
            expected: expected_str,
            actual: actual.to_string(),
        });
    }
    Ok(())
}

pub fn evidence_for_artifact(
    artifact: ArtifactRef,
    root: &Path,
    revision: &str,
    existing_ids: &[String],
) -> EvidenceDto {
    let locator = format!("file://{}", root.join(artifact.relative_path).display());
    let digest = format!("sha256:{}", simple_digest(&locator, revision));
    let base_id = format!("{}-{}", artifact.kind, revision);
    let id = unique_evidence_id(&base_id, &locator, revision, existing_ids);

    EvidenceDto {
        id,
        kind: artifact.kind.to_string(),
        locator,
        digest: Some(digest),
        media_type: Some("application/json".to_string()),
        metadata: Some(BTreeMap::from([(
            "revision".to_string(),
            Value::String(revision.to_string()),
        )])),
        observed_at: None,
    }
}

pub fn malformed_evidence() -> EvidenceDto {
    EvidenceDto {
        id: "".to_string(),
        kind: "invalid".to_string(),
        locator: "file:///missing".to_string(),
        digest: None,
        media_type: None,
        metadata: None,
        observed_at: None,
    }
}

fn unique_evidence_id(
    base_id: &str,
    locator: &str,
    revision: &str,
    existing_ids: &[String],
) -> String {
    if !existing_ids.iter().any(|id| id == base_id) {
        return base_id.to_string();
    }
    let suffix = simple_digest(locator, revision);
    format!("{base_id}-{suffix}")
}

fn simple_digest(locator: &str, revision: &str) -> String {
    let mut acc: u64 = 0xcbf29ce484222325;
    for byte in format!("{locator}:{revision}").bytes() {
        acc ^= u64::from(byte);
        acc = acc.wrapping_mul(0x100000001b3);
    }
    format!("{acc:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked_doc(intent_revision: Option<&str>, subject_revision: Option<&str>) -> LinkedDoc {
        LinkedDoc {
            revision: "1".to_string(),
            intent_revision: intent_revision.map(str::to_string),
            plan_revision: None,
            subject_revision: subject_revision.map(str::to_string),
            verdict: None,
        }
    }

    #[test]
    fn require_linkage_checks_named_field_only() {
        let doc = linked_doc(None, Some("1"));
        assert!(require_linkage(&doc, "design.json", "intent_revision", "1").is_err());

        let doc = linked_doc(Some("1"), None);
        assert!(require_linkage(&doc, "design.json", "intent_revision", "1").is_ok());
    }
}

pub mod artifacts {
    use super::ArtifactRef;

    pub const INTENT: ArtifactRef = ArtifactRef {
        relative_path: "intent.json",
        kind: super::KIND_INTENT,
    };
    pub const DESIGN: ArtifactRef = ArtifactRef {
        relative_path: "design.json",
        kind: super::KIND_DESIGN,
    };
    pub const DESIGN_REVIEW: ArtifactRef = ArtifactRef {
        relative_path: "reviews/design-review.json",
        kind: super::KIND_DESIGN_REVIEW,
    };
    pub const PLAN: ArtifactRef = ArtifactRef {
        relative_path: "plan.json",
        kind: super::KIND_PLAN,
    };
    pub const PLAN_REVIEW: ArtifactRef = ArtifactRef {
        relative_path: "reviews/plan-review.json",
        kind: super::KIND_PLAN_REVIEW,
    };
    pub const IMPLEMENTATION: ArtifactRef = ArtifactRef {
        relative_path: "implementation.json",
        kind: super::KIND_IMPLEMENTATION,
    };
    pub const IMPLEMENTATION_REVIEW: ArtifactRef = ArtifactRef {
        relative_path: "reviews/implementation-review.json",
        kind: super::KIND_IMPLEMENTATION_REVIEW,
    };
    pub const VALIDATION: ArtifactRef = ArtifactRef {
        relative_path: "validation.json",
        kind: super::KIND_VALIDATION,
    };
}
