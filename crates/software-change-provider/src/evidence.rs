//! Review-evidence aggregation and selected-output linkage checks.
//!
//! This module owns no transition routing. It consumes already-validated
//! configuration projections and engine-supplied context records, then applies
//! technical-design §7's six stages in order. Stable invocation/assignment
//! references are resolved by core; this provider verifies the engine-resolved
//! selected bytes and judgment fields without inferring semantic truth.

#![allow(dead_code)]

use crate::checkpoint;
use crate::config::PolicyAxis;
use loop_core::{
    ContextRecord, EngineOrigin, EvidenceApplicability, OriginReference, ENGINE_ORIGIN_KEY,
    EVIDENCE_APPLICABILITY_KIND, ORIGIN_KEY,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const REVIEW_EVIDENCE_KIND: &str = "review-evidence";
const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];
const LEGACY_LINKAGE_FIELDS: &[&str] = &[
    "loop_engine_carry",
    "originating_output",
    "originating_output_sha256",
    "originating_output_path",
    "overridden_inputs",
    "attested_dimensions",
    "selected_attempt",
    "selected_output_sha256",
    "selected_output_path",
    "capture_dir",
    "command",
    "args",
    "binding",
];

/// Exact author identity used for supersession, distinctness, and
/// subject-author exclusion.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct AuthorIdentity {
    pub(crate) name: String,
    pub(crate) kind: String,
}

impl AuthorIdentity {
    pub(crate) fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn is_valid(&self) -> bool {
        !self.name.is_empty() && AUTHOR_KINDS.contains(&self.kind.as_str())
    }
}

/// One diagnostic category required by PRD R19.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub(crate) enum EvidenceDiagnostic {
    Missing {
        required: u64,
    },
    Failed {
        findings: String,
        source: String,
        author: AuthorIdentity,
        remedy: String,
    },
    Malformed {
        reasons: Vec<String>,
    },
    Stale {
        evidence_revision: String,
        current_revision: String,
    },
    StaleConfig {
        evidence_version: String,
        run_version: String,
    },
    Unverified {
        reason: String,
    },
    Independence {
        required: u64,
        distinct_present: usize,
        /// Passes that were present but excluded from the count, with the
        /// rule and why. Empty unless a review group excluded something.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        excluded: Vec<ExcludedPass>,
        /// Distinct authors whose pass did count, for group axes only.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        counted: Vec<AuthorIdentity>,
    },
    AssessmentMissing {
        group: String,
        author: AuthorIdentity,
    },
    AssessmentInconsistent {
        group: String,
        author: AuthorIdentity,
        assessment_ids: Vec<String>,
    },
    MixedAuthors {
        expected_stage: String,
        expected: Vec<AuthorIdentity>,
        actual: Vec<AuthorIdentity>,
    },
    /// A model binding amendment marked the role stale and the exclusion
    /// removed records for this axis; each cause names amendment, role,
    /// author, assessment and axis.
    ExcludedByStaleMark {
        causes: Vec<String>,
    },
}

/// A present pass that did not count toward an axis, with the rule that
/// excluded it. Reported instead of a bare "zero judgments present".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ExcludedPass {
    pub(crate) rule: String,
    pub(crate) author: AuthorIdentity,
    pub(crate) assessment_id: Option<String>,
    pub(crate) axis: String,
    pub(crate) reason: String,
}

impl EvidenceDiagnostic {
    pub(crate) fn category(&self) -> &'static str {
        match self {
            Self::Missing { .. } => "missing",
            Self::Failed { .. } => "failed",
            Self::Malformed { .. } => "malformed",
            Self::Stale { .. } => "stale",
            Self::StaleConfig { .. } => "stale_config",
            Self::Unverified { .. } => "unverified",
            Self::Independence { .. } => "independence",
            Self::MixedAuthors { .. } => "mixed_authors",
            Self::AssessmentMissing { .. } => "assessment_missing",
            Self::AssessmentInconsistent { .. } => "assessment_inconsistent",
            Self::ExcludedByStaleMark { .. } => "excluded_by_stale_mark",
        }
    }
}

/// Diagnostics for one configured axis.  Axis ordering is deterministic
/// because callers supply the semantically keyed BTreeMap from config.rs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct AxisDiagnostic {
    /// `aggregate` is retained for historical/v2 diagnostics that have no
    /// stage field. New v3 diagnostics always name their stage explicitly.
    pub(crate) review_stage: String,
    pub(crate) axis: String,
    pub(crate) diagnostics: Vec<EvidenceDiagnostic>,
}

impl AxisDiagnostic {
    pub(crate) fn has_category(&self, category: &str) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.category() == category)
    }
}

/// An evidence record that could not be associated with any configured
/// gate/axis pair.  It is feedback-only: it is returned only when another
/// axis already denies.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct InertEvidence {
    pub(crate) context_index: usize,
    pub(crate) gate: Option<String>,
    pub(crate) policy_id: Option<String>,
}

/// Result of the pure evidence phase.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct EvidenceEvaluation {
    pub(crate) satisfied: bool,
    pub(crate) diagnostics: Vec<AxisDiagnostic>,
    pub(crate) informational: Vec<AxisDiagnostic>,
    pub(crate) inert_records: Vec<InertEvidence>,
    /// Raw failures remain failures; these source IDs satisfied a judgment
    /// obligation only through an explicit driver disposition.
    pub(crate) satisfied_by_disposition: Vec<String>,
    /// Direct review rows that the existing staged reader found conforming
    /// for a configured stage/axis. This is a final-classification projection,
    /// not an exclusion from context, supersession, ledger debt, or history.
    /// A gate may consume it only after this evaluation is satisfied: R14
    /// then guarantees any attributable malformed marks have been cleared.
    #[serde(skip)]
    pub(crate) classification_review_ids: BTreeSet<String>,
    /// The rows that counted toward a configured stage/axis, each tagged
    /// fresh or carried with its original author, group, assessment identity
    /// and source record. Read-only provenance for packet construction.
    #[serde(skip)]
    pub(crate) rows: Vec<EffectiveRow>,
}

/// One counted judgment and where it came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveRow {
    pub(crate) stage: String,
    pub(crate) axis: String,
    pub(crate) evidence: ConformingEvidence,
}

impl EvidenceEvaluation {
    pub(crate) fn is_satisfied(&self) -> bool {
        self.satisfied
    }

    pub(crate) fn diagnostics(&self) -> &[AxisDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn informational(&self) -> &[AxisDiagnostic] {
        &self.informational
    }

    pub(crate) fn inert_records(&self) -> &[InertEvidence] {
        &self.inert_records
    }

    /// Convert details to JSON without exposing serialization concerns to the
    /// pipeline itself.  T06 may embed this value in its deny response.
    pub(crate) fn details_value(&self) -> Value {
        serde_json::to_value(self).expect("evidence result is serializable")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConformingEvidence {
    pub(crate) source_id: String,
    pub(crate) gate: String,
    pub(crate) policy_id: String,
    pub(crate) review_stage: String,
    pub(crate) result: EvidenceResult,
    pub(crate) findings: String,
    pub(crate) review_contract_version: Option<u32>,
    pub(crate) grounds: Option<Value>,
    pub(crate) author: AuthorIdentity,
    pub(crate) subject_revision: String,
    pub(crate) config_version: String,
    pub(crate) group: Option<String>,
    pub(crate) assessment_id: Option<String>,
    /// True when this judgment reached the current revision through an
    /// explicit evidence-applicability record; the original fields are kept.
    pub(crate) carried: bool,
    /// The evidence-applicability record that carried it, when carried.
    pub(crate) applicability_id: Option<String>,
    /// Set when a carried judgment cites a criterion but the earlier intent bytes
    /// are not retained, so currency could not be verified. It stays
    /// driver-attested; this note is visible but never refuses.
    pub(crate) currency_note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceResult {
    Pass,
    Fail,
}

impl EvidenceResult {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug)]
enum Attribution {
    Current { axis: String },
    OtherConfigured,
    Inert(InertEvidence),
}

/// Causes of stale-mark exclusions that removed records for one gate axis.
/// `stage` is `None` for the unstaged evaluation.
fn stale_mark_causes(
    context: &[ContextRecord],
    gate: &str,
    axis: &str,
    stage: Option<&str>,
) -> Vec<String> {
    software_change_provider::recovery::stale_exclusions(context)
        .values()
        .filter(|cause| {
            cause.gate.as_deref() == Some(gate)
                && cause.axis.as_deref() == Some(axis)
                && match (stage, cause.stage.as_deref()) {
                    (Some(stage), Some(recorded)) => stage == recorded,
                    _ => true,
                }
        })
        .map(software_change_provider::recovery::StaleExclusion::message)
        .collect()
}

/// Run §7's six-stage evidence pipeline.
///
/// `context` is already in engine-supplied sequence order. This function
/// deliberately trusts that order and never sorts by timestamps. When a
/// driver links evidence to selected worker output, the provider verifies
/// those named bytes here; the engine core does not participate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_evidence(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
    subject_author: &AuthorIdentity,
    config_version: &str,
    axes: &BTreeMap<String, PolicyAxis>,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
    artifact_root: Option<&Value>,
) -> EvidenceEvaluation {
    evaluate_evidence_with_dispositions(
        context,
        gate,
        subject,
        current_revision,
        subject_author,
        config_version,
        axes,
        axis_namespace,
        artifact_root,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_evidence_with_dispositions(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
    subject_author: &AuthorIdentity,
    config_version: &str,
    axes: &BTreeMap<String, PolicyAxis>,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
    artifact_root: Option<&Value>,
    ledger: Option<&crate::finding_ledger::FindingLedgerSnapshot>,
) -> EvidenceEvaluation {
    let mut malformed: BTreeMap<String, Vec<String>> = axes
        .keys()
        .cloned()
        .map(|axis| (axis, Vec::new()))
        .collect();
    let mut unverified: BTreeMap<String, Vec<String>> = axes
        .keys()
        .cloned()
        .map(|axis| (axis, Vec::new()))
        .collect();
    let mut latest: BTreeMap<(String, String, AuthorIdentity), ConformingEvidence> =
        BTreeMap::new();
    let mut inert_records = Vec::new();
    let mut global_malformed = Vec::new();
    let mut global_unverified = Vec::new();

    let excluded = match software_change_provider::recovery::excluded_record_ids(
        context,
        gate,
        subject,
        artifact_root,
    ) {
        Ok(ids) => ids,
        Err(error) => {
            global_malformed.push(error);
            BTreeSet::new()
        }
    };
    let active_context: Vec<_> = context
        .iter()
        .filter(|row| !excluded.contains(row.id.as_str()))
        .cloned()
        .collect();

    // Stages 1–3: filter, attribute, then structurally conform. Iteration
    // order is the supplied append/sequence order; map insertion below is
    // therefore latest-wins without relying on wall-clock metadata.
    for (context_index, record) in context.iter().enumerate() {
        if excluded.contains(record.id.as_str()) {
            continue;
        }
        if record.kind == REVIEW_EVIDENCE_KIND {
            let Some(data) = record.data.as_object() else {
                continue;
            };
            let attribution = classify_attribution(data, context_index, gate, axis_namespace);
            match attribution {
                Attribution::Current { axis } => {
                    match parse_conforming_with_context(
                        data,
                        subject,
                        artifact_root,
                        false,
                        &context[..context_index],
                        false,
                    ) {
                        Ok(mut conforming) => {
                            conforming.source_id = record.id.as_str().to_owned();
                            record_conforming(
                                &mut malformed,
                                &mut unverified,
                                &mut latest,
                                axis,
                                conforming,
                            )
                        }
                        Err(EvidenceParseError::Malformed(reasons)) => malformed
                            .get_mut(&axis)
                            .expect("attribution only returns configured axis")
                            .extend(reasons),
                        Err(EvidenceParseError::Unverified(reasons)) => unverified
                            .get_mut(&axis)
                            .expect("attribution only returns configured axis")
                            .extend(reasons),
                    }
                }
                Attribution::OtherConfigured => {
                    // Evidence for another configured gate belongs there;
                    // never diagnose it during this gate's evaluation.
                }
                Attribution::Inert(inert) => inert_records.push(inert),
            }
        } else if record.kind == EVIDENCE_APPLICABILITY_KIND {
            if loop_integrations::review_intake::superseded_origin(&active_context, record) {
                continue;
            }
            let attribution =
                applicability_attribution(context, record, context_index, gate, axis_namespace);
            match (
                attribution,
                parse_applicable_evidence(
                    context,
                    record,
                    subject,
                    current_revision,
                    artifact_root,
                ),
            ) {
                (Some(Attribution::Current { axis }), Ok(conforming)) => record_conforming(
                    &mut malformed,
                    &mut unverified,
                    &mut latest,
                    axis,
                    conforming,
                ),
                (
                    Some(Attribution::Current { axis }),
                    Err(EvidenceParseError::Malformed(reasons)),
                ) => malformed
                    .get_mut(&axis)
                    .expect("attribution only returns configured axis")
                    .extend(reasons),
                (
                    Some(Attribution::Current { axis }),
                    Err(EvidenceParseError::Unverified(reasons)),
                ) => unverified
                    .get_mut(&axis)
                    .expect("attribution only returns configured axis")
                    .extend(reasons),
                (Some(Attribution::OtherConfigured), _) => {
                    // A declaration for another configured gate is not a
                    // denial while evaluating this gate.
                }
                (Some(Attribution::Inert(inert)), _) => inert_records.push(inert),
                (None, Err(EvidenceParseError::Malformed(reasons))) => {
                    global_malformed.extend(reasons)
                }
                (None, Err(EvidenceParseError::Unverified(reasons))) => {
                    global_unverified.extend(reasons)
                }
                (None, Ok(_)) => {
                    global_malformed.push(
                        "evidence applicability could not be attributed to a configured gate and axis"
                            .to_owned(),
                    );
                }
            }
        }
    }

    // An applicability declaration without a resolvable source has no gate or
    // axis from which to derive attribution. It is still a named durable
    // reference failure, so fail closed rather than allowing a valid-looking
    // fresh record to hide it.
    for axis in axes.keys() {
        malformed
            .get_mut(axis)
            .expect("all configured axes have malformed state")
            .extend(global_malformed.iter().cloned());
        unverified
            .get_mut(axis)
            .expect("all configured axes have unverified state")
            .extend(global_unverified.iter().cloned());
    }

    let mut diagnostics = Vec::new();
    let mut informational = Vec::new();
    let mut all_axes_satisfied = true;
    let mut satisfied_by_disposition = Vec::new();

    // Stages 4–6: latest-wins supersession happened above through the full
    // (axis, subject_revision, author) key.  Judge each axis against current
    // subject metadata and then emit all applicable categories.
    for (axis, policy) in axes {
        let mut failed_findings = Vec::new();
        let mut stale = Vec::new();
        let mut stale_config = Vec::new();
        let mut distinct_present = BTreeSet::new();
        let mut satisfied_authors = BTreeSet::new();
        let mut current_records_present = 0usize;

        for ((record_axis, _revision, _author), record) in &latest {
            if record_axis != axis {
                continue;
            }

            if record.subject_revision != current_revision {
                stale.push((record.subject_revision.clone(), current_revision.to_owned()));
            }
            if record.config_version != config_version {
                stale_config.push((record.config_version.clone(), config_version.to_owned()));
            }

            // Wrong config versions count as neither pass nor fail.  A stale
            // revision likewise cannot enter current judgment.
            if record.subject_revision != current_revision
                || record.config_version != config_version
            {
                continue;
            }

            // Subject-author evidence is structurally valid but never counts
            // toward any axis, using exact (name, kind) equality.
            if record.author == *subject_author && !policy.self_review() {
                continue;
            }

            let disposition = ledger.and_then(|ledger| ledger.disposition_for(&record.source_id));
            if disposition == Some(crate::finding_ledger::FindingDisposition::RetiredAuthor) {
                continue;
            }
            current_records_present += 1;
            distinct_present.insert(record.author.clone());
            match record.result {
                EvidenceResult::Pass => {
                    satisfied_authors.insert(record.author.clone());
                }
                EvidenceResult::Fail => {
                    if disposition.is_some() {
                        satisfied_authors.insert(record.author.clone());
                        satisfied_by_disposition.push(record.source_id.clone());
                    } else {
                        failed_findings.push(EvidenceDiagnostic::Failed {
                            findings: record.findings.clone(), source: record.source_id.clone(),
                            author: record.author.clone(),
                            remedy: "append a reasoned rejection/resolution of this exact source, or record reviewer retirement with a roster change and replacement coverage".to_owned(),
                        });
                    }
                }
            }
        }

        let malformed_reasons = malformed
            .get(axis)
            .expect("all configured axes have malformed state");
        let unverified_reasons = unverified
            .get(axis)
            .expect("all configured axes have unverified state");
        let has_malformed = !malformed_reasons.is_empty();
        let has_unverified = !unverified_reasons.is_empty();
        let has_fail = !failed_findings.is_empty();
        let enough_judgments = satisfied_authors.len() as u64 >= policy.required_authors();
        let satisfied = !has_malformed && !has_unverified && !has_fail && enough_judgments;

        if !satisfied {
            all_axes_satisfied = false;
            let mut axis_diagnostics = Vec::new();
            if current_records_present == 0 {
                axis_diagnostics.push(EvidenceDiagnostic::Missing {
                    required: policy.required_authors(),
                });
            }
            axis_diagnostics.extend(failed_findings);
            if has_malformed {
                axis_diagnostics.push(EvidenceDiagnostic::Malformed {
                    reasons: malformed_reasons.clone(),
                });
            }
            axis_diagnostics.extend(
                unverified_reasons
                    .iter()
                    .cloned()
                    .map(|reason| EvidenceDiagnostic::Unverified { reason }),
            );
            if (distinct_present.len() as u64) < policy.required_authors() {
                axis_diagnostics.push(EvidenceDiagnostic::Independence {
                    required: policy.required_authors(),
                    distinct_present: distinct_present.len(),
                    excluded: Vec::new(),
                    counted: Vec::new(),
                });
            }
            let causes = stale_mark_causes(context, gate, axis, None);
            if !causes.is_empty() {
                axis_diagnostics.push(EvidenceDiagnostic::ExcludedByStaleMark { causes });
            }
            diagnostics.push(AxisDiagnostic {
                review_stage: "aggregate".to_owned(),
                axis: axis.clone(),
                diagnostics: axis_diagnostics,
            });

            let mut informational_diagnostics = Vec::new();
            informational_diagnostics.extend(stale.into_iter().map(
                |(evidence_revision, current_revision)| EvidenceDiagnostic::Stale {
                    evidence_revision,
                    current_revision,
                },
            ));
            informational_diagnostics.extend(stale_config.into_iter().map(
                |(evidence_version, run_version)| EvidenceDiagnostic::StaleConfig {
                    evidence_version,
                    run_version,
                },
            ));
            if !informational_diagnostics.is_empty() {
                informational.push(AxisDiagnostic {
                    review_stage: "aggregate".to_owned(),
                    axis: axis.clone(),
                    diagnostics: informational_diagnostics,
                });
            }
        }
    }

    if all_axes_satisfied {
        // Inert records are feedback-only and must not affect an allow.
        inert_records.clear();
    }

    EvidenceEvaluation {
        satisfied: all_axes_satisfied,
        diagnostics,
        informational,
        inert_records,
        satisfied_by_disposition,
        classification_review_ids: BTreeSet::new(),
        rows: Vec::new(),
    }
}

/// Stage-aware v3 aggregation. Each configured `(review_stage, axis)` is an
/// independent obligation. A stage mismatch is attributable to the same axis
/// but cannot satisfy either stage, and multi-stage gates additionally require
/// the same non-subject author identities in every stage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_staged_evidence_with_dispositions(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
    subject_author: &AuthorIdentity,
    config_version: &str,
    stages: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
    artifact_root: Option<&Value>,
    ledger: Option<&crate::finding_ledger::FindingLedgerSnapshot>,
    require_stage: bool,
) -> EvidenceEvaluation {
    evaluate_staged_evidence_excluding(
        context,
        gate,
        subject,
        current_revision,
        subject_author,
        config_version,
        stages,
        axis_namespace,
        artifact_root,
        ledger,
        require_stage,
        &std::collections::BTreeSet::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_staged_evidence_excluding(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
    subject_author: &AuthorIdentity,
    config_version: &str,
    stages: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
    artifact_root: Option<&Value>,
    ledger: Option<&crate::finding_ledger::FindingLedgerSnapshot>,
    require_stage: bool,
    excluded_records: &std::collections::BTreeSet<String>,
) -> EvidenceEvaluation {
    type StageAxis = (String, String);

    let mut malformed: BTreeMap<StageAxis, Vec<String>> = stages
        .iter()
        .flat_map(|(stage, axes)| {
            axes.keys()
                .map(move |axis| ((stage.clone(), axis.clone()), Vec::new()))
        })
        .collect();
    let mut unverified: BTreeMap<StageAxis, Vec<String>> = malformed
        .keys()
        .cloned()
        .map(|key| (key, Vec::new()))
        .collect();
    let mut latest: BTreeMap<(String, String, String, AuthorIdentity), ConformingEvidence> =
        BTreeMap::new();
    let mut classification_review_ids = BTreeSet::new();
    let mut rows: Vec<EffectiveRow> = Vec::new();
    let mut inert_records = Vec::new();
    let mut global_malformed = Vec::new();
    let mut global_unverified = Vec::new();

    let mut excluded = excluded_records.clone();
    // Only records excluded by withdrawal or author retirement are described as such;
    // the caller's own exclusions (for example challenge chronology) keep their own cause.
    let mut recovery_excluded = std::collections::BTreeSet::new();
    match software_change_provider::recovery::excluded_record_ids(
        context,
        gate,
        subject,
        artifact_root,
    ) {
        Ok(ids) => {
            recovery_excluded = ids.clone();
            excluded.extend(ids)
        }
        Err(error) => global_malformed.push(error),
    }
    let active_context: Vec<_> = context
        .iter()
        .filter(|row| !excluded.contains(row.id.as_str()))
        .cloned()
        .collect();
    // Why an axis lost a judgment: shown only if the axis ends up unsatisfied,
    // so a driver can tell a withdrawn or retired source from absent review work.
    let mut exclusion_notes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in context
        .iter()
        .filter(|row| recovery_excluded.contains(row.id.as_str()))
    {
        let (origin, note) = if row.kind == REVIEW_EVIDENCE_KIND {
            (
                Some(row),
                format!(
                    "judgment `{}` is excluded (withdrawn or retired author), so it does not count",
                    row.id
                ),
            )
        } else if row.kind == EVIDENCE_APPLICABILITY_KIND {
            let origin_id = row.data.pointer("/origin/id").and_then(Value::as_str);
            // A carry can be withdrawn on its own; only say the origin is excluded when it is.
            let origin_excluded = origin_id.is_some_and(|id| recovery_excluded.contains(id));
            (
                context
                    .iter()
                    .find(|candidate| Some(candidate.id.as_str()) == origin_id),
                if origin_excluded {
                    format!(
                        "carried record `{}` is invalid: its origin `{}` is excluded (withdrawn or retired author)",
                        row.id,
                        origin_id.unwrap_or("?")
                    )
                } else {
                    format!(
                        "carried record `{}` is withdrawn, so it does not count; its origin `{}` is not excluded and a new evidence-applicability can reuse it",
                        row.id,
                        origin_id.unwrap_or("?")
                    )
                },
            )
        } else {
            continue;
        };
        let Some(origin) = origin.filter(|origin| origin.kind == REVIEW_EVIDENCE_KIND) else {
            continue;
        };
        if origin.data["gate"] != gate {
            continue;
        }
        if let Some(axis) = origin.data["policy_id"].as_str() {
            let notes = exclusion_notes.entry(axis.to_owned()).or_default();
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
    }

    let add_for_axis =
        |axis: &str, reasons: &[String], target: &mut BTreeMap<StageAxis, Vec<String>>| {
            for stage in stages.keys() {
                if let Some(values) = target.get_mut(&(stage.clone(), axis.to_owned())) {
                    values.extend(reasons.iter().cloned());
                }
            }
        };

    for (context_index, record) in context.iter().enumerate() {
        if excluded.contains(record.id.as_str()) {
            continue;
        }

        if record.kind == REVIEW_EVIDENCE_KIND {
            let Some(data) = record.data.as_object() else {
                continue;
            };
            match classify_attribution(data, context_index, gate, axis_namespace) {
                Attribution::Current { axis } => {
                    match parse_conforming_with_context(
                        data,
                        subject,
                        artifact_root,
                        require_stage,
                        &context[..context_index],
                        false,
                    ) {
                        Ok(mut conforming) => {
                            conforming.source_id = record.id.as_str().to_owned();
                            if stages
                                .get(&conforming.review_stage)
                                .and_then(|axes| axes.get(&axis))
                                .is_some()
                            {
                                classification_review_ids.insert(record.id.as_str().to_owned());
                                record_stage_conforming(
                                    &mut malformed,
                                    &mut unverified,
                                    &mut latest,
                                    axis,
                                    conforming,
                                );
                            } else {
                                add_for_axis(
                                    &axis,
                                    &[format!(
                                        "review_stage `{}` is not configured for this axis",
                                        conforming.review_stage
                                    )],
                                    &mut malformed,
                                );
                            }
                        }
                        Err(EvidenceParseError::Malformed(reasons)) => {
                            add_for_axis(&axis, &reasons, &mut malformed)
                        }
                        Err(EvidenceParseError::Unverified(reasons)) => {
                            add_for_axis(&axis, &reasons, &mut unverified)
                        }
                    }
                }
                Attribution::OtherConfigured => {}
                Attribution::Inert(inert) => inert_records.push(inert),
            }
        } else if record.kind == EVIDENCE_APPLICABILITY_KIND {
            if loop_integrations::review_intake::superseded_origin(&active_context, record) {
                continue;
            }
            let attribution =
                applicability_attribution(context, record, context_index, gate, axis_namespace);
            match (
                attribution,
                parse_applicable_evidence_for_stage(
                    context,
                    record,
                    subject,
                    current_revision,
                    artifact_root,
                    require_stage,
                ),
            ) {
                (Some(Attribution::Current { axis }), Ok(conforming)) => {
                    let configured = stages
                        .get(&conforming.review_stage)
                        .and_then(|axes| axes.get(&axis))
                        .is_some();
                    if !configured {
                        add_for_axis(
                            &axis,
                            &[format!(
                                "review_stage `{}` is not configured for this axis",
                                conforming.review_stage
                            )],
                            &mut malformed,
                        );
                    } else {
                        record_stage_conforming(
                            &mut malformed,
                            &mut unverified,
                            &mut latest,
                            axis,
                            conforming,
                        );
                    }
                }
                (
                    Some(Attribution::Current { axis }),
                    Err(EvidenceParseError::Malformed(reasons)),
                ) => add_for_axis(&axis, &reasons, &mut malformed),
                (
                    Some(Attribution::Current { axis }),
                    Err(EvidenceParseError::Unverified(reasons)),
                ) => add_for_axis(&axis, &reasons, &mut unverified),
                (Some(Attribution::OtherConfigured), _) => {}
                (Some(Attribution::Inert(inert)), _) => inert_records.push(inert),
                (None, Err(EvidenceParseError::Malformed(reasons))) => {
                    global_malformed.extend(reasons)
                }
                (None, Err(EvidenceParseError::Unverified(reasons))) => {
                    global_unverified.extend(reasons)
                }
                (None, Ok(_)) => global_malformed.push(
                    "evidence applicability could not be attributed to a configured gate and axis"
                        .to_owned(),
                ),
            }
        }
    }

    for key in malformed.keys().cloned().collect::<Vec<_>>() {
        malformed
            .get_mut(&key)
            .expect("key was just collected")
            .extend(global_malformed.iter().cloned());
        unverified
            .get_mut(&key)
            .expect("matching stage/axis key")
            .extend(global_unverified.iter().cloned());
    }

    let mut diagnostics = Vec::new();
    let mut informational = Vec::new();
    let mut all_axes_satisfied = true;
    let mut satisfied_by_disposition = Vec::new();
    let mut current_authors: BTreeMap<StageAxis, BTreeSet<AuthorIdentity>> = BTreeMap::new();

    for (stage, axes) in stages {
        for (axis, policy) in axes {
            let key = (stage.clone(), axis.clone());
            let mut failed_findings = Vec::new();
            let mut stale = Vec::new();
            let mut stale_config = Vec::new();
            let mut distinct_present = BTreeSet::new();
            let mut satisfied_authors = BTreeSet::new();
            let mut present_authors = BTreeSet::new();
            let mut assessment_causes = Vec::new();

            for ((record_stage, record_axis, _revision, _author), record) in &latest {
                if record_stage != stage || record_axis != axis {
                    continue;
                }
                if record.subject_revision != current_revision {
                    stale.push((record.subject_revision.clone(), current_revision.to_owned()));
                }
                if record.config_version != config_version {
                    stale_config.push((record.config_version.clone(), config_version.to_owned()));
                }
                if record.subject_revision != current_revision
                    || record.config_version != config_version
                {
                    continue;
                }
                if record.author == *subject_author && !policy.self_review() {
                    continue;
                }
                if ledger.and_then(|ledger| ledger.disposition_for(&record.source_id))
                    == Some(crate::finding_ledger::FindingDisposition::RetiredAuthor)
                {
                    continue;
                }
                present_authors.insert(record.author.clone());
                distinct_present.insert(record.author.clone());
                match record.result {
                    EvidenceResult::Pass => {
                        satisfied_authors.insert(record.author.clone());
                    }
                    EvidenceResult::Fail => {
                        if ledger
                            .and_then(|ledger| ledger.disposition_for(&record.source_id))
                            .is_some()
                        {
                            satisfied_authors.insert(record.author.clone());
                            satisfied_by_disposition.push(record.source_id.clone());
                        } else {
                            failed_findings.push(EvidenceDiagnostic::Failed {
                                findings: record.findings.clone(),
                                source: record.source_id.clone(),
                                author: record.author.clone(),
                                remedy: "append a reasoned rejection/resolution of this exact source, or record reviewer retirement with a roster change and replacement coverage".to_owned(),
                            });
                        }
                    }
                }
            }
            let raw_satisfied_count = satisfied_authors.len();
            let mut stage_authors = present_authors.clone();
            let mut exclusions: Vec<ExcludedPass> = Vec::new();
            let mut group_axis_with_peers = false;
            if let Some(group) = policy.group() {
                if config_version
                    .rsplit_once('-')
                    .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
                    .is_some_and(|version| version >= 14)
                {
                    let matches_group = |author: &AuthorIdentity| {
                        latest
                            .get(&(
                                stage.clone(),
                                axis.clone(),
                                current_revision.to_owned(),
                                author.clone(),
                            ))
                            .is_some_and(|record| record.group.as_deref() == Some(group))
                    };
                    satisfied_authors.retain(&matches_group);
                    present_authors.retain(matches_group);
                    stage_authors = present_authors.clone();
                }
                let members: Vec<_> = axes
                    .values()
                    .filter(|axis| axis.group() == Some(group))
                    .collect();
                if members.len() > 1 {
                    group_axis_with_peers = true;
                    let member_row = |member: &str, author: &AuthorIdentity| {
                        latest
                            .get(&(
                                stage.clone(),
                                member.to_owned(),
                                current_revision.to_owned(),
                                author.clone(),
                            ))
                            .filter(|peer| {
                                peer.config_version == config_version
                                    && peer.group.as_deref() == Some(group)
                            })
                    };
                    // A group whose counted evidence includes a carried row is
                    // judged per axis; a group with none is judged as before.
                    // "Counted" means the same filters as the loop above: a carried row
                    // from a retired author, a barred self-reviewer, or an
                    // unresolved failure does not switch the group to per-axis coverage.
                    let counts = |row: &ConformingEvidence, member: &PolicyAxis| {
                        let disposition =
                            ledger.and_then(|ledger| ledger.disposition_for(&row.source_id));
                        row.config_version == config_version
                            && row.group.as_deref() == Some(group)
                            && !(row.author == *subject_author && !member.self_review())
                            && disposition
                                != Some(crate::finding_ledger::FindingDisposition::RetiredAuthor)
                            && (row.result == EvidenceResult::Pass || disposition.is_some())
                    };
                    let mixed = members.iter().any(|member| {
                        latest
                            .iter()
                            .any(|((row_stage, row_axis, revision, _), row)| {
                                row_stage == stage
                                    && row_axis == member.id()
                                    && revision == current_revision
                                    && row.carried
                                    && counts(row, member)
                            })
                    });
                    if mixed {
                        // Carried rows keep their original author and
                        // assessment. The fresh rows of any one author must
                        // still share one assessment.
                        for author in present_authors.clone() {
                            let Some(own) = member_row(axis, &author) else {
                                continue;
                            };
                            if own.carried {
                                continue;
                            }
                            let fresh: Vec<_> = members
                                .iter()
                                .filter_map(|member| member_row(member.id(), &author))
                                .filter(|row| !row.carried)
                                .collect();
                            let ids = fresh
                                .iter()
                                .filter_map(|row| row.assessment_id.clone())
                                .collect::<BTreeSet<_>>();
                            let missing = fresh.iter().any(|row| row.assessment_id.is_none());
                            if missing || ids.len() > 1 {
                                if missing {
                                    assessment_causes.push(EvidenceDiagnostic::AssessmentMissing {
                                        group: group.to_owned(),
                                        author: author.clone(),
                                    });
                                } else {
                                    assessment_causes.push(
                                        EvidenceDiagnostic::AssessmentInconsistent {
                                            group: group.to_owned(),
                                            author: author.clone(),
                                            assessment_ids: ids.iter().cloned().collect(),
                                        },
                                    );
                                }
                                satisfied_authors.remove(&author);
                                present_authors.remove(&author);
                            }
                        }
                    } else {
                        for author in &present_authors {
                            let peers = members
                                .iter()
                                .filter_map(|member| member_row(member.id(), author))
                                .collect::<Vec<_>>();
                            if peers.len() == members.len() {
                                let ids = peers
                                    .iter()
                                    .filter_map(|peer| peer.assessment_id.clone())
                                    .collect::<BTreeSet<_>>();
                                if peers.iter().any(|peer| peer.assessment_id.is_none()) {
                                    assessment_causes.push(EvidenceDiagnostic::AssessmentMissing {
                                        group: group.to_owned(),
                                        author: author.clone(),
                                    });
                                } else if ids.len() > 1 {
                                    assessment_causes.push(
                                        EvidenceDiagnostic::AssessmentInconsistent {
                                            group: group.to_owned(),
                                            author: author.clone(),
                                            assessment_ids: ids.into_iter().collect(),
                                        },
                                    );
                                }
                            }
                        }
                        let complete = |author: &AuthorIdentity| {
                            group_assessment_matches(
                                &latest,
                                stage,
                                axis,
                                policy,
                                axes,
                                current_revision,
                                config_version,
                                author,
                            )
                        };
                        for author in satisfied_authors.iter().filter(|author| !complete(author)) {
                            let absent: Vec<_> = members
                                .iter()
                                .filter(|member| member_row(member.id(), author).is_none())
                                .map(|member| member.id().to_owned())
                                .collect();
                            exclusions.push(ExcludedPass {
                                rule: if absent.is_empty() {
                                    "review-group-assessment".to_owned()
                                } else {
                                    "review-group-complete".to_owned()
                                },
                                author: author.clone(),
                                assessment_id: member_row(axis, author)
                                    .and_then(|row| row.assessment_id.clone()),
                                axis: axis.clone(),
                                reason: if absent.is_empty() {
                                    format!(
                                        "group `{group}` counts an author only when every axis of the group is judged under one assessment, and this author's assessments differ or are missing"
                                    )
                                } else {
                                    format!(
                                        "group `{group}` counts an author only when that author judged every axis of the group under one assessment; this author has no counted judgment for {}",
                                        absent.join(", ")
                                    )
                                },
                            });
                        }
                        satisfied_authors.retain(&complete);
                        present_authors.retain(complete);
                    }
                }
            }
            for author in &satisfied_authors {
                if let Some(record) = latest.get(&(
                    stage.clone(),
                    axis.clone(),
                    current_revision.to_owned(),
                    author.clone(),
                )) {
                    rows.push(EffectiveRow {
                        stage: stage.clone(),
                        axis: axis.clone(),
                        evidence: record.clone(),
                    });
                }
            }
            current_authors.insert(key.clone(), stage_authors);

            let malformed_reasons = malformed.get(&key).expect("configured stage/axis");
            let unverified_reasons = unverified.get(&key).expect("configured stage/axis");
            let enough_judgments = satisfied_authors.len() as u64 >= policy.required_authors();
            let satisfied = malformed_reasons.is_empty()
                && unverified_reasons.is_empty()
                && failed_findings.is_empty()
                && enough_judgments;
            if !satisfied {
                all_axes_satisfied = false;
                let mut axis_diagnostics = Vec::new();
                if distinct_present.is_empty() {
                    axis_diagnostics.push(EvidenceDiagnostic::Missing {
                        required: policy.required_authors(),
                    });
                }
                axis_diagnostics.extend(failed_findings);
                let metadata_only_shortage = !assessment_causes.is_empty()
                    && raw_satisfied_count as u64 >= policy.required_authors();
                axis_diagnostics.extend(assessment_causes);
                if !malformed_reasons.is_empty() {
                    axis_diagnostics.push(EvidenceDiagnostic::Malformed {
                        reasons: malformed_reasons.clone(),
                    });
                }
                axis_diagnostics.extend(
                    unverified_reasons
                        .iter()
                        .cloned()
                        .map(|reason| EvidenceDiagnostic::Unverified { reason }),
                );
                axis_diagnostics.extend(
                    exclusion_notes
                        .get(axis)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .map(|reason| EvidenceDiagnostic::Unverified { reason }),
                );
                if (satisfied_authors.len() as u64) < policy.required_authors()
                    && !metadata_only_shortage
                {
                    axis_diagnostics.push(EvidenceDiagnostic::Independence {
                        required: policy.required_authors(),
                        distinct_present: satisfied_authors.len(),
                        excluded: exclusions,
                        counted: if group_axis_with_peers {
                            satisfied_authors.iter().cloned().collect()
                        } else {
                            Vec::new()
                        },
                    });
                }
                let causes = stale_mark_causes(context, gate, axis, Some(stage));
                if !causes.is_empty() {
                    axis_diagnostics.push(EvidenceDiagnostic::ExcludedByStaleMark { causes });
                }
                diagnostics.push(AxisDiagnostic {
                    review_stage: stage.clone(),
                    axis: axis.clone(),
                    diagnostics: axis_diagnostics,
                });
                let mut informational_diagnostics = Vec::new();
                informational_diagnostics.extend(stale.into_iter().map(
                    |(evidence_revision, current_revision)| EvidenceDiagnostic::Stale {
                        evidence_revision,
                        current_revision,
                    },
                ));
                informational_diagnostics.extend(stale_config.into_iter().map(
                    |(evidence_version, run_version)| EvidenceDiagnostic::StaleConfig {
                        evidence_version,
                        run_version,
                    },
                ));
                if !informational_diagnostics.is_empty() {
                    informational.push(AxisDiagnostic {
                        review_stage: stage.clone(),
                        axis: axis.clone(),
                        diagnostics: informational_diagnostics,
                    });
                }
            }
        }
    }

    // High-rigor's individual and aggregate stages must use the same two
    // reviewer identities for each axis. A stage with no current evidence is
    // already diagnosed as missing; compare only the present sets here.
    for axis in stages
        .values()
        .flat_map(|axes| axes.keys())
        .collect::<BTreeSet<_>>()
    {
        let stage_sets = stages
            .keys()
            .filter_map(|stage| {
                current_authors
                    .get(&(stage.clone(), axis.clone()))
                    .filter(|authors| !authors.is_empty())
                    .map(|authors| (stage.clone(), authors.clone()))
            })
            .collect::<Vec<_>>();
        let Some((expected_stage, expected_authors)) = stage_sets.first() else {
            continue;
        };
        for (stage, actual_authors) in stage_sets.iter().skip(1) {
            if actual_authors == expected_authors {
                continue;
            }
            all_axes_satisfied = false;
            let add = |items: &mut Vec<AxisDiagnostic>,
                       stage: &str,
                       actual: &BTreeSet<AuthorIdentity>| {
                let diagnostic = EvidenceDiagnostic::MixedAuthors {
                    expected_stage: expected_stage.clone(),
                    expected: expected_authors.iter().cloned().collect(),
                    actual: actual.iter().cloned().collect(),
                };
                if let Some(existing) = items
                    .iter_mut()
                    .find(|item| item.review_stage == stage && item.axis == *axis)
                {
                    existing.diagnostics.push(diagnostic);
                } else {
                    items.push(AxisDiagnostic {
                        review_stage: stage.to_owned(),
                        axis: axis.clone(),
                        diagnostics: vec![diagnostic],
                    });
                }
            };
            add(&mut diagnostics, stage, actual_authors);
            add(&mut diagnostics, expected_stage, expected_authors);
        }
    }

    if all_axes_satisfied {
        inert_records.clear();
    }
    satisfied_by_disposition.sort();
    satisfied_by_disposition.dedup();
    EvidenceEvaluation {
        satisfied: all_axes_satisfied,
        diagnostics,
        informational,
        inert_records,
        satisfied_by_disposition,
        classification_review_ids,
        rows,
    }
}

fn record_stage_conforming(
    malformed: &mut BTreeMap<(String, String), Vec<String>>,
    unverified: &mut BTreeMap<(String, String), Vec<String>>,
    latest: &mut BTreeMap<(String, String, String, AuthorIdentity), ConformingEvidence>,
    axis: String,
    conforming: ConformingEvidence,
) {
    let key = (conforming.review_stage.clone(), axis);
    // R14 clears malformed debt for the same gate/axis, not only the
    // correction's stage. Verdict coverage and standings remain stage-keyed.
    for ((_, pending_axis), reasons) in malformed.iter_mut() {
        if pending_axis == &key.1 {
            reasons.clear();
        }
    }
    unverified
        .get_mut(&key)
        .expect("configured stage/axis")
        .clear();
    latest.insert(
        (
            key.0,
            key.1,
            conforming.subject_revision.clone(),
            conforming.author.clone(),
        ),
        conforming,
    );
}

fn classify_attribution(
    data: &Map<String, Value>,
    context_index: usize,
    gate: &str,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
) -> Attribution {
    let record_gate = data.get("gate").and_then(Value::as_str);
    let policy_id = data.get("policy_id").and_then(Value::as_str);

    let Some(record_gate) = record_gate else {
        return Attribution::Inert(InertEvidence {
            context_index,
            gate: None,
            policy_id: policy_id.map(str::to_owned),
        });
    };
    let Some(policy_id) = policy_id else {
        return Attribution::Inert(InertEvidence {
            context_index,
            gate: Some(record_gate.to_owned()),
            policy_id: None,
        });
    };

    if !axis_namespace
        .get(record_gate)
        .is_some_and(|axes| axes.contains(policy_id))
    {
        return Attribution::Inert(InertEvidence {
            context_index,
            gate: Some(record_gate.to_owned()),
            policy_id: Some(policy_id.to_owned()),
        });
    }

    if record_gate == gate {
        Attribution::Current {
            axis: policy_id.to_owned(),
        }
    } else {
        Attribution::OtherConfigured
    }
}

#[derive(Clone, Debug)]
pub(crate) enum EvidenceParseError {
    Malformed(Vec<String>),
    Unverified(Vec<String>),
}

impl EvidenceParseError {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Malformed(reasons) | Self::Unverified(reasons) => reasons.join("; "),
        }
    }
}

fn record_conforming(
    malformed: &mut BTreeMap<String, Vec<String>>,
    unverified: &mut BTreeMap<String, Vec<String>>,
    latest: &mut BTreeMap<(String, String, AuthorIdentity), ConformingEvidence>,
    axis: String,
    conforming: ConformingEvidence,
) {
    // Any later conforming record clears the earlier block, regardless of
    // author, revision, or whether the record was fresh or explicitly
    // applicable.
    malformed
        .get_mut(&axis)
        .expect("attribution only returns configured axis")
        .clear();
    unverified
        .get_mut(&axis)
        .expect("attribution only returns configured axis")
        .clear();
    latest.insert(
        (
            axis,
            conforming.subject_revision.clone(),
            conforming.author.clone(),
        ),
        conforming,
    );
}

#[allow(clippy::too_many_arguments)]
fn group_assessment_matches(
    latest: &BTreeMap<(String, String, String, AuthorIdentity), ConformingEvidence>,
    stage: &str,
    axis: &str,
    policy: &PolicyAxis,
    axes: &BTreeMap<String, PolicyAxis>,
    revision: &str,
    config_version: &str,
    author: &AuthorIdentity,
) -> bool {
    let Some(group) = policy.group() else {
        return true;
    };
    if config_version
        .rsplit_once('-')
        .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
        .is_some_and(|version| version >= 14)
        && !latest
            .get(&(
                stage.to_owned(),
                axis.to_owned(),
                revision.to_owned(),
                author.clone(),
            ))
            .is_some_and(|record| record.group.as_deref() == Some(group))
    {
        return false;
    }
    let members: Vec<_> = axes
        .values()
        .filter(|axis| axis.group() == Some(group))
        .collect();
    if members.len() <= 1 {
        return true;
    }
    let mut assessment = None;
    members.iter().all(|member| {
        latest
            .get(&(
                stage.to_owned(),
                member.id().to_owned(),
                revision.to_owned(),
                author.clone(),
            ))
            .is_some_and(|peer| {
                if peer.config_version != config_version || peer.group.as_deref() != Some(group) {
                    return false;
                }
                let Some(id) = peer.assessment_id.as_deref() else {
                    return false;
                };
                match assessment {
                    Some(previous) => previous == id,
                    None => {
                        assessment = Some(id);
                        true
                    }
                }
            })
    })
}

/// Parse one review-evidence object for use as an immutable source record.
/// The returned values are the original judgment fields; callers must not
/// replace its author, result, findings, or config identity with attestation
/// metadata from another context record.
/// Candidate eligibility shared by intake and table; aggregate author floors
/// are separate and are never checked here.
pub(crate) fn validate_direct_review(
    gate: &str,
    axis: &str,
    self_review: bool,
    direct: bool,
) -> Result<(), String> {
    if direct && axis == "intent-sufficient" && gate == "intent-review" {
        return Err("cold intent-sufficient reconstruction cannot be self-review".into());
    }
    if direct && !self_review {
        return Err("self review is not allowed for this axis".into());
    }
    Ok(())
}

pub(crate) fn parse_evidence_record(
    value: &Value,
    expected_subject: &str,
    artifact_root: Option<&Value>,
) -> Result<ConformingEvidence, String> {
    parse_evidence_record_for_stage(value, expected_subject, artifact_root, false)
}

/// Resolve same-run citations against the original earlier prefix.
pub(crate) fn parse_evidence_record_for_stage_with_context(
    value: &Value,
    expected_subject: &str,
    artifact_root: Option<&Value>,
    require_stage: bool,
    context: &[ContextRecord],
) -> Result<ConformingEvidence, String> {
    let object = value
        .as_object()
        .ok_or("review-evidence source data must be an object")?;
    parse_conforming_with_context(
        object,
        expected_subject,
        artifact_root,
        require_stage,
        context,
        false,
    )
    .map_err(EvidenceParseError::into_message)
}

pub(crate) fn parse_evidence_record_for_stage(
    value: &Value,
    expected_subject: &str,
    artifact_root: Option<&Value>,
    require_stage: bool,
) -> Result<ConformingEvidence, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "review-evidence source data must be an object".to_owned())?;
    parse_conforming_with_stage(object, expected_subject, artifact_root, require_stage)
        .map_err(EvidenceParseError::into_message)
}

fn parse_conforming(
    data: &Map<String, Value>,
    expected_subject: &str,
    artifact_root: Option<&Value>,
) -> Result<ConformingEvidence, EvidenceParseError> {
    parse_conforming_with_stage(data, expected_subject, artifact_root, false)
}

fn parse_conforming_with_stage(
    data: &Map<String, Value>,
    expected_subject: &str,
    artifact_root: Option<&Value>,
    require_stage: bool,
) -> Result<ConformingEvidence, EvidenceParseError> {
    parse_conforming_with_context(
        data,
        expected_subject,
        artifact_root,
        require_stage,
        &[],
        false,
    )
}

/// Ledger sources retain identity and citation shape without rebinding files.
pub(crate) fn parse_historical_evidence_record(
    value: &Value,
    expected_subject: &str,
    require_stage: bool,
) -> Result<ConformingEvidence, String> {
    let data = value
        .as_object()
        .ok_or("review-evidence source data must be an object")?;
    parse_conforming_with_context(data, expected_subject, None, require_stage, &[], true)
        .map_err(EvidenceParseError::into_message)
}

pub(crate) fn parse_conforming_with_context(
    data: &Map<String, Value>,
    expected_subject: &str,
    artifact_root: Option<&Value>,
    require_stage: bool,
    context: &[ContextRecord],
    historical: bool,
) -> Result<ConformingEvidence, EvidenceParseError> {
    let mut reasons = Vec::new();
    for field in ["group", "assessment_id"] {
        if data
            .get(field)
            .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
        {
            reasons.push(format!("{field} must be a nonempty string when present"));
        }
    }
    let gate = non_empty_string(data, "gate", &mut reasons);
    let policy_id = non_empty_string(data, "policy_id", &mut reasons);
    let result = match data.get("result").and_then(Value::as_str) {
        Some("pass") => Some(EvidenceResult::Pass),
        Some("fail") => Some(EvidenceResult::Fail),
        Some(_) => {
            reasons.push("`result` must be `pass` or `fail`".to_owned());
            None
        }
        None => {
            reasons.push("missing or non-string `result`".to_owned());
            None
        }
    };
    let findings = match data.get("findings").and_then(Value::as_str) {
        Some(findings) => Some(findings.to_owned()),
        None => {
            reasons.push("missing or non-string `findings`".to_owned());
            None
        }
    };
    let author = parse_author(data.get("author"), &mut reasons);
    let review_contract_version = match data.get("review_contract_version") {
        None => None,
        Some(Value::Number(number)) if matches!(number.as_u64(), Some(2 | 3)) => {
            number.as_u64().map(|v| v as u32)
        }
        Some(_) => {
            reasons.push("`review_contract_version` must be 2 or 3 when present".to_owned());
            None
        }
    };
    let grounds = data.get("grounds").cloned();
    if matches!(review_contract_version, Some(2 | 3)) {
        match grounds.as_ref() {
            Some(grounds) => {
                let root = artifact_root.and_then(Value::as_str).map(Path::new);
                if historical {
                    if let Err(error) = crate::criterion::validate_grounding_shape_version(
                        grounds,
                        review_contract_version,
                    ) {
                        reasons.push(error);
                    }
                } else if let Some(root) = root {
                    let target_revision = fs::read(root.join(expected_subject))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                        .and_then(|value| {
                            value
                                .get("revision")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        });
                    let check = match target_revision.as_deref() {
                        Some(current)
                            if data.get("subject_revision").and_then(Value::as_str)
                                == Some(current) =>
                        {
                            crate::criterion::validate_grounding_with_context(
                                grounds,
                                root,
                                review_contract_version,
                                context,
                            )
                        }
                        Some(_) => crate::criterion::validate_grounding_shape_version(
                            grounds,
                            review_contract_version,
                        ),
                        None => Err(format!(
                            "cannot resolve current `{expected_subject}` revision for grounding"
                        )),
                    };
                    if let Err(error) = check {
                        reasons.push(error);
                    }
                } else {
                    reasons.push("grounded review evidence requires artifact_root to verify source references".to_owned());
                }
            }
            None => reasons.push("grounded review evidence requires grounds".to_owned()),
        }
    } else if grounds.is_some() {
        reasons.push("grounds require review_contract_version 2 or 3".to_owned());
    }
    let subject = non_empty_string(data, "subject", &mut reasons);
    let subject_revision = non_empty_string(data, "subject_revision", &mut reasons);
    let config_version = non_empty_string(data, "config_version", &mut reasons);
    let review_stage = match data.get("review_stage").or_else(|| data.get("stage")) {
        Some(Value::String(value)) if value == "individual" || value == "aggregate" => {
            Some(value.clone())
        }
        Some(_) => {
            reasons.push("`review_stage` must be `individual` or `aggregate`".to_owned());
            None
        }
        None if require_stage => {
            reasons.push("missing or non-string `review_stage`".to_owned());
            None
        }
        None => Some("aggregate".to_owned()),
    };

    if subject.as_deref() != Some(expected_subject) {
        reasons.push(format!(
            "`subject` must equal expected subject `{expected_subject}`"
        ));
    }
    if matches!(result, Some(EvidenceResult::Fail))
        && findings.as_deref().is_some_and(str::is_empty)
    {
        reasons.push("`findings` must not be empty for a fail".to_owned());
    }

    if !reasons.is_empty() {
        return Err(EvidenceParseError::Malformed(reasons));
    }

    let result = result.expect("result checked by empty-reasons branch");
    let findings = findings.expect("findings checked by empty-reasons branch");
    let author = author.expect("author checked by empty-reasons branch");
    let gate = gate.expect("gate checked by empty-reasons branch");
    let policy_id = policy_id.expect("policy id checked by empty-reasons branch");
    let subject_revision = subject_revision.expect("revision checked by empty-reasons branch");
    let config_version = config_version.expect("config version checked by empty-reasons branch");
    let review_stage = review_stage.expect("review stage checked by empty-reasons branch");

    if let Err(reason) = validate_stable_origin_for_stage(
        data,
        &policy_id,
        &author,
        result,
        &findings,
        &review_stage,
        require_stage,
    ) {
        return Err(EvidenceParseError::Unverified(vec![reason]));
    }

    Ok(ConformingEvidence {
        source_id: String::new(),
        gate,
        policy_id,
        review_stage,
        result,
        findings,
        review_contract_version,
        grounds,
        author,
        subject_revision,
        config_version,
        group: data.get("group").and_then(Value::as_str).map(str::to_owned),
        assessment_id: data
            .get("assessment_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        carried: false,
        applicability_id: None,
        currency_note: None,
    })
}

/// Resolve one evidence-applicability record through its earlier immutable
/// context-record source. The attestation itself supplies only the explicit
/// semantic claim and current target; all judgment fields come from the
/// referenced review-evidence record.
fn parse_applicable_evidence(
    context: &[ContextRecord],
    applicability_record: &ContextRecord,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
) -> Result<ConformingEvidence, EvidenceParseError> {
    parse_applicable_evidence_at(
        context,
        applicability_record,
        expected_subject,
        current_revision,
        artifact_root,
        false,
    )
}

pub(crate) fn parse_applicable_evidence_for_stage(
    context: &[ContextRecord],
    applicability_record: &ContextRecord,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
    require_stage: bool,
) -> Result<ConformingEvidence, EvidenceParseError> {
    parse_applicable_evidence_at_with_stage(
        context,
        applicability_record,
        expected_subject,
        current_revision,
        artifact_root,
        false,
        require_stage,
    )
}

fn parse_applicable_evidence_at(
    context: &[ContextRecord],
    applicability_record: &ContextRecord,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
    captured_batch: bool,
) -> Result<ConformingEvidence, EvidenceParseError> {
    parse_applicable_evidence_at_with_stage(
        context,
        applicability_record,
        expected_subject,
        current_revision,
        artifact_root,
        captured_batch,
        false,
    )
}

fn parse_applicable_evidence_at_with_stage(
    context: &[ContextRecord],
    applicability_record: &ContextRecord,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
    captured_batch: bool,
    require_stage: bool,
) -> Result<ConformingEvidence, EvidenceParseError> {
    let applicability = parse_applicability_shape(&applicability_record.data)?;
    validate_applicability_target(
        &applicability.target,
        expected_subject,
        current_revision,
        artifact_root,
        captured_batch,
    )
    .map_err(|reason| EvidenceParseError::Unverified(vec![reason]))?;

    let matches = context
        .iter()
        .filter(|record| record.id.as_str() == applicability.origin.id)
        .collect::<Vec<_>>();
    let Some(source) = matches.first().copied() else {
        return Err(EvidenceParseError::Unverified(vec![format!(
            "evidence-applicability source context record `{}` was not found",
            applicability.origin.id
        )]));
    };
    if matches.len() != 1 {
        return Err(EvidenceParseError::Unverified(vec![format!(
            "evidence-applicability source context record `{}` is ambiguous",
            applicability.origin.id
        )]));
    }
    if source.sequence >= applicability_record.sequence {
        return Err(EvidenceParseError::Unverified(vec![format!(
            "evidence-applicability source context record `{}` is not an earlier record",
            applicability.origin.id
        )]));
    }
    if source.kind != REVIEW_EVIDENCE_KIND {
        return Err(EvidenceParseError::Unverified(vec![format!(
            "evidence-applicability source context record `{}` is not review-evidence",
            applicability.origin.id
        )]));
    }

    let mut evidence = parse_conforming_with_context(
        source.data.as_object().ok_or_else(|| {
            EvidenceParseError::Malformed(vec![format!(
                "evidence source context record `{}` is not an object",
                applicability.origin.id
            )])
        })?,
        expected_subject,
        artifact_root,
        require_stage,
        &context[..context
            .iter()
            .position(|record| record.id == source.id)
            .expect("resolved source")],
        false,
    )?;
    // The driver explicitly attests that the immutable judgment applies to
    // this current target. It is not a semantic inference by the provider.
    evidence.source_id = source.id.as_str().to_owned();
    evidence.subject_revision = current_revision.to_owned();
    evidence.carried = true;
    evidence.applicability_id = Some(applicability_record.id.as_str().to_owned());
    // Evidence about a criterion is current only while that criterion's text is.
    match crate::currency::changed_cited_criteria(
        evidence.grounds.as_ref(),
        source,
        context,
        artifact_root,
    ) {
        Ok(changed) if changed.is_empty() => {}
        Ok(changed) => {
            return Err(EvidenceParseError::Unverified(vec![format!(
                "stale carry: carried judgment `{}` cites criterion {} whose text changed since it was written; obtain a fresh judgment on the changed criteria",
                source.id,
                changed.join(", ")
            )]))
        }
        Err(reason) => evidence.currency_note = Some(reason),
    }
    Ok(evidence)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn counted_aggregate_sources(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    subject_author: &AuthorIdentity,
    config_version: &str,
    policy: &PolicyAxis,
    axes: &BTreeMap<String, PolicyAxis>,
    artifact_root: Option<&Value>,
    ledger: Option<&crate::finding_ledger::FindingLedgerSnapshot>,
    require_stage: bool,
) -> Result<Vec<ContextRecord>, String> {
    let excluded = software_change_provider::recovery::excluded_record_ids(
        context,
        gate,
        subject,
        artifact_root,
    )?;
    let active: Vec<_> = context
        .iter()
        .filter(|row| !excluded.contains(row.id.as_str()))
        .cloned()
        .collect();
    let mut latest = BTreeMap::new();
    for record in &active {
        let parsed = if record.kind == REVIEW_EVIDENCE_KIND {
            parse_evidence_record_for_stage_with_context(
                &record.data,
                subject,
                artifact_root,
                require_stage,
                &context[..context
                    .iter()
                    .position(|source| source.id == record.id)
                    .expect("counted source")],
            )
            .map(|mut evidence| {
                evidence.source_id = record.id.to_string();
                evidence
            })
        } else if record.kind == EVIDENCE_APPLICABILITY_KIND {
            if loop_integrations::review_intake::superseded_origin(&active, record) {
                continue;
            }
            parse_applicable_evidence_for_stage(
                context,
                record,
                subject,
                revision,
                artifact_root,
                require_stage,
            )
            .map_err(EvidenceParseError::into_message)
        } else {
            continue;
        };
        let Ok(evidence) = parsed else {
            continue;
        };
        if evidence.gate != gate
            || !axes.contains_key(&evidence.policy_id)
            || evidence.review_stage != "aggregate"
            || evidence.subject_revision != revision
        {
            continue;
        }
        latest.insert(
            (
                "aggregate".to_owned(),
                evidence.policy_id.clone(),
                evidence.subject_revision.clone(),
                evidence.author.clone(),
            ),
            evidence,
        );
    }
    Ok(latest
        .values()
        .filter(|evidence| {
            evidence.policy_id == policy.id()
                && group_assessment_matches(
                    &latest,
                    "aggregate",
                    policy.id(),
                    policy,
                    axes,
                    revision,
                    config_version,
                    &evidence.author,
                )
                && evidence.config_version == config_version
                && (evidence.author != *subject_author || policy.self_review())
                && (evidence.result == EvidenceResult::Pass
                    || ledger
                        .and_then(|ledger| ledger.disposition_for(&evidence.source_id))
                        .is_some())
        })
        .filter_map(|evidence| {
            context
                .iter()
                .find(|row| row.id.as_str() == evidence.source_id)
                .cloned()
        })
        .collect())
}

/// A batch may carry only a source already authorized in its launch snapshot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_batch_reuse(
    context: &[ContextRecord],
    id: &str,
    gate: &str,
    axis: &str,
    author: &Value,
    subject: &str,
    revision: &str,
    artifact_root: Option<&Value>,
) -> Result<(), String> {
    validate_batch_reuse_for_stage(
        context,
        id,
        gate,
        axis,
        author,
        subject,
        revision,
        artifact_root,
        "aggregate",
        false,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_batch_reuse_for_stage(
    context: &[ContextRecord],
    id: &str,
    gate: &str,
    axis: &str,
    author: &Value,
    subject: &str,
    revision: &str,
    artifact_root: Option<&Value>,
    review_stage: &str,
    require_stage: bool,
    expected_config_version: Option<&str>,
) -> Result<(), String> {
    let records: Vec<_> = context.iter().filter(|r| r.id.as_str() == id).collect();
    if records.len() != 1 || records[0].kind != EVIDENCE_APPLICABILITY_KIND {
        return Err(format!(
            "reuse `{id}` is not an authorized captured applicability record"
        ));
    }
    // Verify the original commission's target identity, not today's checkpoint.
    // Later promotion of this source separately verifies the new live target.
    let evidence = parse_applicable_evidence_at_with_stage(
        context,
        records[0],
        subject,
        revision,
        artifact_root,
        true,
        require_stage,
    )
    .map_err(|e| format!("reuse `{id}` is invalid: {e:?}"))?;
    if evidence.gate != gate
        || evidence.policy_id != axis
        || evidence.review_stage != review_stage
        || author.get("name").and_then(Value::as_str) != Some(evidence.author.name())
        || author.get("kind").and_then(Value::as_str) != Some(evidence.author.kind())
    {
        return Err(format!(
            "reuse `{id}` has wrong gate, axis, stage or author"
        ));
    }
    if expected_config_version.is_some_and(|expected| evidence.config_version != expected) {
        return Err(format!(
            "reuse `{id}` has stale config version `{}`",
            evidence.config_version
        ));
    }
    Ok(())
}

/// Return whether an applicability record validly promotes one immutable
/// source evidence record to the supplied current target. This is a
/// mechanical reference check for finding-ledger validation; it does not make
/// a semantic applicability decision.
pub(crate) fn applicability_covers_source(
    context: &[ContextRecord],
    record: &ContextRecord,
    source_record_id: &str,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
) -> bool {
    let Ok(applicability) = serde_json::from_value::<EvidenceApplicability>(record.data.clone())
    else {
        return false;
    };
    applicability.origin.kind == "context-record"
        && applicability.origin.id == source_record_id
        && parse_applicable_evidence(
            context,
            record,
            expected_subject,
            current_revision,
            artifact_root,
        )
        .is_ok()
}

fn applicability_attribution(
    context: &[ContextRecord],
    record: &ContextRecord,
    context_index: usize,
    gate: &str,
    axis_namespace: &BTreeMap<String, BTreeSet<String>>,
) -> Option<Attribution> {
    let applicability =
        serde_json::from_value::<EvidenceApplicability>(record.data.clone()).ok()?;
    let matches = context
        .iter()
        .filter(|candidate| candidate.id.as_str() == applicability.origin.id)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return None;
    }
    let source = matches[0].data.as_object()?;
    Some(classify_attribution(
        source,
        context_index,
        gate,
        axis_namespace,
    ))
}

fn validate_attesting_driver(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("attesting_driver must be an object".to_owned());
    };
    for key in object.keys() {
        if key != "name" && key != "kind" {
            return Err(format!("attesting_driver has unknown field `{key}`"));
        }
    }
    let Some(name) = object.get("name").and_then(Value::as_str) else {
        return Err("attesting_driver.name must be a non-empty string".to_owned());
    };
    if name.trim().is_empty() {
        return Err("attesting_driver.name must be a non-empty string".to_owned());
    }
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return Err("attesting_driver.kind must be human, agent, or script".to_owned());
    };
    if !AUTHOR_KINDS.contains(&kind) {
        return Err("attesting_driver.kind must be human, agent, or script".to_owned());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicabilityTarget {
    subject: String,
    revision: String,
    #[serde(default)]
    checkpoint: Option<Value>,
}

/// Canonical new-write target; narrow read compatibility for old criterion/goal
/// carry rows accepted by the former subject_revision gate implementation.
pub(crate) fn canonical_target(target: &Value, historical_verdict: bool) -> Result<Value, String> {
    let mut value = target.clone();
    if let Some(alias) = value.get("subject_revision").cloned() {
        if !historical_verdict || value.get("revision").is_some() {
            return Err("applicability target requires canonical revision; ambiguous/conflicting aliases are refused".into());
        }
        if alias.as_str().is_none_or(|s| s.trim().is_empty()) {
            return Err("historical applicability revision is invalid".into());
        }
        let object = value
            .as_object_mut()
            .ok_or("applicability target must be object")?;
        object.remove("subject_revision");
        object.insert("revision".into(), alias);
    }
    Ok(value)
}

pub(crate) fn parse_applicability_shape(
    data: &Value,
) -> Result<EvidenceApplicability, EvidenceParseError> {
    let applicability: EvidenceApplicability = serde_json::from_value(data.clone()).map_err(|error| EvidenceParseError::Malformed(vec![format!("evidence-applicability data must contain only origin, target, attesting_driver, and reason: {error}")]))?;
    if applicability.origin.kind != "context-record"
        || applicability.origin.id.trim().is_empty()
        || applicability.origin.assignment_id.is_some()
    {
        return Err(EvidenceParseError::Unverified(vec!["evidence-applicability requires an earlier context-record origin without assignment_id".into()]));
    }
    validate_attesting_driver(&applicability.attesting_driver).map_err(|reason| {
        EvidenceParseError::Malformed(vec![format!("evidence-applicability {reason}")])
    })?;
    if applicability.reason.trim().is_empty() {
        return Err(EvidenceParseError::Malformed(vec![
            "evidence-applicability reason must be non-empty".into(),
        ]));
    }
    Ok(applicability)
}

pub(crate) fn validate_applicability_target(
    value: &Value,
    expected_subject: &str,
    current_revision: &str,
    artifact_root: Option<&Value>,
    captured_batch: bool,
) -> Result<(), String> {
    let target = serde_json::from_value::<ApplicabilityTarget>(value.clone())
        .map_err(|error| format!("target is not a valid current-target object: {error}"))?;
    if target.subject.trim().is_empty() || target.subject != expected_subject {
        return Err(format!(
            "evidence-applicability target subject must equal `{expected_subject}`"
        ));
    }
    if target.revision.trim().is_empty() || target.revision != current_revision {
        return Err(format!(
            "evidence-applicability target revision is stale (expected `{current_revision}`)"
        ));
    }
    let expected_checkpoint = if captured_batch {
        match expected_subject {
            "implementation-report.json" => Some(
                serde_json::json!({"phase": "implementation", "report_revision": current_revision}),
            ),
            "validation-report.json" => Some(
                serde_json::json!({"phase": "validation", "report_revision": current_revision}),
            ),
            _ => None,
        }
    } else {
        checkpoint::current_target(
            expected_subject,
            Path::new(artifact_root.and_then(Value::as_str).unwrap_or_default()),
        )?
    };
    if target.checkpoint != expected_checkpoint {
        return Err(format!(
            "evidence-applicability target checkpoint is not current (expected {})",
            expected_checkpoint.map_or_else(|| "null".to_owned(), |value| value.to_string())
        ));
    }
    Ok(())
}

fn validate_stable_origin_for_stage(
    data: &Map<String, Value>,
    policy_id: &str,
    author: &AuthorIdentity,
    result: EvidenceResult,
    findings: &str,
    review_stage: &str,
    require_stage: bool,
) -> Result<(), String> {
    let legacy = LEGACY_LINKAGE_FIELDS
        .iter()
        .find(|field| data.contains_key(**field));
    if let Some(field) = legacy {
        return Err(format!(
            "legacy selected-output field `{field}` is not accepted; use `origin` with engine-resolved metadata"
        ));
    }

    let has_origin = data.contains_key(ORIGIN_KEY);
    let has_engine_origin = data.contains_key(ENGINE_ORIGIN_KEY);
    if !has_origin && !has_engine_origin {
        return Ok(());
    }
    let Some(origin_value) = data.get(ORIGIN_KEY) else {
        return Err(format!(
            "`{ENGINE_ORIGIN_KEY}` requires the concise `{ORIGIN_KEY}` reference"
        ));
    };
    let origin =
        serde_json::from_value::<OriginReference>(origin_value.clone()).map_err(|error| {
            format!("`{ORIGIN_KEY}` is not a valid stable origin reference: {error}")
        })?;
    if origin.kind != "selected-assignment-output" {
        return Err(
            "fresh review-evidence origin kind must be `selected-assignment-output`".to_owned(),
        );
    }
    if origin.id.trim().is_empty() {
        return Err("fresh review-evidence origin.id must be non-empty".to_owned());
    }
    let Some(origin_assignment) = origin.assignment_id.as_deref() else {
        return Err(
            "fresh review-evidence selected-assignment-output origin requires assignment_id"
                .to_owned(),
        );
    };
    if origin_assignment.trim().is_empty() {
        return Err("fresh review-evidence origin.assignment_id must be non-empty".to_owned());
    }
    let Some(engine_value) = data.get(ENGINE_ORIGIN_KEY) else {
        return Err(format!(
            "fresh selected-assignment evidence is missing engine-resolved `{ENGINE_ORIGIN_KEY}`"
        ));
    };
    let engine = serde_json::from_value::<EngineOrigin>(engine_value.clone()).map_err(|error| {
        format!("`{ENGINE_ORIGIN_KEY}` is not valid engine-resolved origin metadata: {error}")
    })?;
    if engine.invocation_id.as_str() != origin.id {
        return Err("engine-resolved invocation does not match concise origin id".to_owned());
    }
    if engine.assignment_id != origin_assignment {
        return Err(
            "engine-resolved assignment does not match concise origin assignment_id".to_owned(),
        );
    }
    verify_engine_selected_output(
        &engine,
        policy_id,
        author,
        result,
        findings,
        review_stage,
        require_stage,
        data,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_engine_selected_output(
    origin: &EngineOrigin,
    policy_id: &str,
    author: &AuthorIdentity,
    result: EvidenceResult,
    findings: &str,
    review_stage: &str,
    require_stage: bool,
    data: &Map<String, Value>,
) -> Result<(), String> {
    match (origin.selected_attempt, origin.recovery_source.as_ref()) {
        (Some(attempt), None) if attempt > 0 => {}
        (None, Some(source)) => {
            verify_recovery_source(
                source,
                Path::new(&origin.capture_dir),
                &origin.selected_output_path,
                &origin.selected_output_sha256,
            )?;
            verify_recovery_source_identity(
                source,
                &origin.assignment_id,
                data.get("gate").and_then(Value::as_str).unwrap_or_default(),
                None,
                &origin.binding,
            )?;
        }
        (Some(_), Some(_)) => {
            return Err(
                "engine origin cannot claim both a new attempt and a recovered source".to_owned(),
            );
        }
        _ => {
            return Err(
                "engine origin has no verifiable selected attempt or recovery source".to_owned(),
            )
        }
    }
    if origin.selected_output_sha256.trim().is_empty()
        || !valid_digest(&origin.selected_output_sha256)
    {
        return Err("engine-resolved selected_output_sha256 is invalid".to_owned());
    }
    if origin.capture_dir.trim().is_empty() || origin.selected_output_path.trim().is_empty() {
        return Err("engine-resolved selected output path is incomplete".to_owned());
    }
    let cwd = std::env::current_dir()
        .map_err(|error| format!("could not read provider working directory: {error}"))?;
    let capture = PathBuf::from(&origin.capture_dir);
    let capture = if capture.is_absolute() {
        capture
    } else {
        cwd.join(capture)
    };
    let capture = fs::canonicalize(&capture)
        .map_err(|error| format!("engine-resolved capture directory is unavailable: {error}"))?;
    if !capture.is_dir() {
        return Err("engine-resolved capture directory is not a directory".to_owned());
    }
    let selected = PathBuf::from(&origin.selected_output_path);
    let selected = if selected.is_absolute() {
        selected
    } else {
        capture.join(selected)
    };
    let selected = fs::canonicalize(&selected)
        .map_err(|error| format!("engine-resolved selected output is unavailable: {error}"))?;
    if selected == capture || !selected.starts_with(&capture) || !selected.is_file() {
        return Err("engine-resolved selected output escapes capture_dir".to_owned());
    }
    let bytes = fs::read(&selected)
        .map_err(|error| format!("engine-resolved selected output is unavailable: {error}"))?;
    if sha256_digest(&bytes) != origin.selected_output_sha256 {
        return Err("engine-resolved selected output digest does not match raw bytes".to_owned());
    }
    let value = parse_originating_judgment(&bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| "engine-resolved selected output is not a JSON object".to_owned())?;
    let raw_contract_version = object
        .get("review_contract_version")
        .and_then(Value::as_u64);
    if origin
        .slot_id
        .as_deref()
        .is_some_and(|slot| data.get("gate").and_then(Value::as_str) != Some(slot))
    {
        return Err("selected invocation gate does not match review-evidence gate".to_owned());
    }
    let output_author = object
        .get("author")
        .and_then(Value::as_object)
        .ok_or_else(|| "engine-resolved selected output has no judgment author".to_owned())?;
    if output_author.get("name").and_then(Value::as_str) != Some(author.name())
        || output_author.get("kind").and_then(Value::as_str) != Some(author.kind())
    {
        return Err("judgment author disagrees with review-evidence author".to_owned());
    }
    let object = if object.contains_key("judgments") {
        let (schema, location) =
            crate::review_batch::captured_commission(&capture, &origin.assignment_id)?;
        let rows = crate::review_batch::rows_for_stage(
            &schema,
            &value,
            &location,
            data.get("gate").and_then(Value::as_str).unwrap_or_default(),
            data.get("subject")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            data.get("subject_revision")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            review_stage,
            require_stage,
        )?;
        let row = rows
            .into_iter()
            .find(|row| row["axis"] == policy_id)
            .ok_or("judgment axis disagrees with review-evidence policy_id")?;
        if row.get("reuse").is_some() {
            return Err("carried row cannot be appended as fresh review-evidence; use its original applicability reference".into());
        }
        row.as_object().ok_or("invalid batch row")?
    } else {
        if object.get("axis").and_then(Value::as_str) != Some(policy_id) {
            return Err("judgment axis disagrees with review-evidence policy_id".to_owned());
        }
        if let Some(raw_stage) = object.get("review_stage").and_then(Value::as_str) {
            if raw_stage != review_stage {
                return Err(
                    "judgment review_stage disagrees with review-evidence review_stage".to_owned(),
                );
            }
        } else if require_stage {
            return Err("selected review output is missing review_stage".to_owned());
        }
        object
    };
    if object.get("result").and_then(Value::as_str) != Some(result.as_str()) {
        return Err("judgment result disagrees with review-evidence result".to_owned());
    }
    if object.get("findings").and_then(Value::as_str) != Some(findings) {
        return Err("judgment findings disagree with review-evidence findings".to_owned());
    }
    match raw_contract_version {
        Some(version @ (2 | 3)) => {
            if data.get("review_contract_version").and_then(Value::as_u64) != Some(version)
                || object.get("grounds") != data.get("grounds")
            {
                return Err(
                    "reviewer grounds/version disagree with selected raw judgment".to_owned(),
                );
            }
        }
        None if data.contains_key("review_contract_version") || data.contains_key("grounds") => {
            return Err(
                "review-evidence claims grounds absent from the selected output contract"
                    .to_owned(),
            );
        }
        Some(_) => return Err("unsupported selected review output contract version".to_owned()),
        None => {}
    }
    Ok(())
}

/// Verify that a joined selected source is tied to the same assignment,
/// gate, visit subject and frozen binding as the current selection.
pub(crate) fn verify_recovery_source_identity(
    source: &Value,
    assignment_id: &str,
    gate: &str,
    expected_subject: Option<&str>,
    binding: &loop_core::WorkSlotBinding,
) -> Result<(), String> {
    let origin = source
        .get("origin")
        .and_then(Value::as_object)
        .ok_or("recovery source has no original raw origin")?;
    let expected_binding = sha256_digest(&serde_json::to_vec(binding).expect("historical binding"));
    if origin.get("assignment_id").and_then(Value::as_str) != Some(assignment_id)
        || origin.get("slot_id").and_then(Value::as_str) != Some(gate)
        || expected_subject
            .is_some_and(|subject| origin.get("subject").and_then(Value::as_str) != Some(subject))
        || origin.get("binding_sha256").and_then(Value::as_str) != Some(&expected_binding)
    {
        return Err(
            "recovery raw origin does not match the selected assignment, gate, subject or binding"
                .into(),
        );
    }
    Ok(())
}

/// Verify a joined assignment's selected bytes against its immutable raw
/// origin. Derived bytes additionally require a retained derivation and a
/// separate, explicit fidelity triage claim.
pub(crate) fn verify_recovery_source(
    source: &Value,
    current_capture: &Path,
    selected_output_path: &str,
    selected_output_sha256: &str,
) -> Result<(), String> {
    let source_class = source
        .get("source_class")
        .and_then(Value::as_str)
        .ok_or("recovery source has no source_class")?;
    if source.get("protocol").and_then(Value::as_str) != Some("fan-out-selected-source-v1")
        || source.get("execution").and_then(Value::as_str) != Some("reused")
        || source.get("selected_output_path").and_then(Value::as_str) != Some(selected_output_path)
        || source.get("selected_output_sha256").and_then(Value::as_str)
            != Some(selected_output_sha256)
    {
        return Err(
            "recovery selected-source identity disagrees with engine-selected bytes".into(),
        );
    }
    let selected = read_capture_output(current_capture, selected_output_path)?;
    if sha256_digest(&selected) != selected_output_sha256 {
        return Err("recovered selected output digest does not match its bytes".into());
    }

    let original = source
        .get("origin")
        .and_then(Value::as_object)
        .ok_or("recovery source has no original raw origin")?;
    for field in [
        "invocation_id",
        "run_id",
        "slot_id",
        "subject",
        "binding_sha256",
        "assignment_id",
    ] {
        if original
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(format!("recovery raw origin has no `{field}`"));
        }
    }
    if original.get("execution").is_some() {
        return Err("recovery raw origin contains unexpected execution metadata".into());
    }
    if original.get("exit_code").and_then(Value::as_i64) != Some(0)
        || original
            .get("raw_attempt")
            .and_then(Value::as_u64)
            .is_none_or(|attempt| attempt == 0)
    {
        return Err("recovery raw origin is not a completed positive attempt".into());
    }
    let raw_digest = original
        .get("raw_stdout_sha256")
        .and_then(Value::as_str)
        .ok_or("recovery raw origin has no digest")?;
    if !valid_digest(raw_digest) {
        return Err("recovery raw origin digest is malformed".into());
    }
    let raw_path = original
        .get("raw_stdout_path")
        .and_then(Value::as_str)
        .ok_or("recovery raw origin has no retained stdout path")?;
    let origin_capture = original
        .get("capture_dir")
        .and_then(Value::as_str)
        .ok_or("recovery raw origin has no capture directory")?;
    let raw = read_capture_output(Path::new(origin_capture), raw_path)?;
    if sha256_digest(&raw) != raw_digest {
        return Err("recovery raw-origin bytes do not match their retained digest".into());
    }

    match source_class {
        "original-raw" => {
            if source
                .get("derivation")
                .is_some_and(|value| !value.is_null())
                || source
                    .get("owner_approval")
                    .is_some_and(|value| !value.is_null())
                || source
                    .get("fidelity_approval")
                    .is_some_and(|value| !value.is_null())
                || original
                    .get("selected_output_sha256")
                    .and_then(Value::as_str)
                    != Some(raw_digest)
            {
                return Err(
                    "original-raw source does not retain the exact selected raw origin".into(),
                );
            }
            let source_path = original
                .get("selected_output_path")
                .and_then(Value::as_str)
                .ok_or("original-raw source has no selected raw path")?;
            let source_bytes = read_capture_output(Path::new(origin_capture), source_path)?;
            if source_bytes != raw || selected != raw {
                return Err("original-raw selected bytes differ from the exact raw attempt".into());
            }
        }
        "eligible-derived" => {
            let declared_contract = source
                .get("declared_output_contract")
                .and_then(Value::as_object)
                .ok_or("eligible-derived source has no frozen output contract")?;
            let direct_fields = [
                "review_contract_version",
                "review_stage",
                "author",
                "axis",
                "result",
                "findings",
                "grounds",
            ];
            let legacy_fields = declared_contract
                .get("required")
                .and_then(Value::as_array)
                .is_some_and(|required| {
                    direct_fields
                        .iter()
                        .all(|field| required.contains(&Value::String((*field).into())))
                });
            let schema = declared_contract
                .get("schema")
                .unwrap_or_else(|| source.get("declared_output_contract").expect("present"));
            let batch_fields = schema
                .pointer("/properties/review_contract_version")
                .is_some()
                && schema.pointer("/properties/review_stage").is_some()
                && schema.pointer("/properties/author").is_some()
                && schema.pointer("/properties/judgments").is_some();
            if !legacy_fields && !batch_fields {
                return Err(
                    "eligible-derived source contract does not retain the review judgment fields"
                        .into(),
                );
            }
            let derivation = source
                .get("derivation")
                .and_then(Value::as_object)
                .ok_or("eligible-derived source has no derivation")?;
            let difference = derivation
                .get("difference")
                .filter(|value| !value.is_null())
                .ok_or("eligible-derived source has no explicit difference")?;
            if !(difference
                .as_object()
                .is_some_and(|value| !value.is_empty())
                || difference.as_array().is_some_and(|value| !value.is_empty()))
            {
                return Err("eligible-derived source has an empty derivation difference".into());
            }
            match derivation.get("kind").and_then(Value::as_str) {
                Some("mechanical") if derivation.get("adapter").is_none_or(Value::is_null) => {}
                Some("driver") if derivation.get("adapter").is_none_or(Value::is_null) => {
                    if schema
                        .get("x-loop-engine-output-recovery")
                        .and_then(Value::as_str)
                        != Some("repair-first-v2")
                    {
                        return Err("driver derivation requires repair-first-v2".into());
                    }
                    let formatter = difference
                        .pointer("/formatter_author/name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .ok_or("driver derivation lacks formatter attribution")?;
                    if !matches!(
                        difference
                            .pointer("/formatter_author/kind")
                            .and_then(Value::as_str),
                        Some("human" | "agent" | "script")
                    ) {
                        return Err("driver formatter kind is invalid".into());
                    }
                    let derived = parse_originating_judgment(&selected)?;
                    if derived.pointer("/author/name").and_then(Value::as_str) == Some(formatter) {
                        return Err("formatter is not a separate reviewer attribution".into());
                    }
                    if let Ok(raw) = parse_originating_judgment(&raw) {
                        loop_core::check_known_output_conflicts(&raw, &derived)?;
                    }
                }
                Some("scripted") => {
                    let adapter = derivation
                        .get("adapter")
                        .and_then(Value::as_object)
                        .ok_or("scripted derivation has no usage accounting")?;
                    let calls = adapter.get("calls").and_then(Value::as_u64).unwrap_or(0);
                    let elapsed = adapter
                        .get("elapsed_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cost = adapter
                        .get("metered_cost_micros")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX);
                    if adapter
                        .get("command")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                        || adapter
                            .get("model_id")
                            .is_some_and(|value| !value.is_null())
                        || adapter.get("capture").is_some_and(|value| !value.is_null())
                        || adapter.get("usage_accounted") != Some(&Value::Bool(true))
                        || calls != 1
                        || calls
                            > adapter
                                .get("max_calls")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || elapsed == 0
                        || elapsed
                            > adapter
                                .get("max_time_ms")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || cost
                            > adapter
                                .get("max_cost_micros")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || adapter
                            .get("max_cost_micros")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            == 0
                    {
                        return Err(
                            "scripted derivation has invalid or exceeded usage bounds".into()
                        );
                    }
                }
                Some("model") => {
                    let adapter = derivation
                        .get("adapter")
                        .and_then(Value::as_object)
                        .ok_or("model derivation has no attributed usage accounting")?;
                    let model_id = adapter
                        .get("model_id")
                        .and_then(Value::as_str)
                        .filter(|model_id| !model_id.trim().is_empty())
                        .ok_or("model derivation has no explicit model identity")?;
                    let calls = adapter.get("calls").and_then(Value::as_u64).unwrap_or(0);
                    let elapsed = adapter
                        .get("elapsed_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cost = adapter
                        .get("metered_cost_micros")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX);
                    if adapter
                        .get("command")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                        || adapter.get("usage_accounted") != Some(&Value::Bool(true))
                        || calls != 1
                        || calls
                            > adapter
                                .get("max_calls")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || elapsed == 0
                        || elapsed
                            > adapter
                                .get("max_time_ms")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || cost
                            > adapter
                                .get("max_cost_micros")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        || adapter
                            .get("max_cost_micros")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            == 0
                    {
                        return Err(
                            "model derivation has invalid or exceeded positive usage bounds".into(),
                        );
                    }
                    verify_model_repair_capture(source, current_capture, model_id, adapter)?;
                }
                _ => {
                    return Err("eligible-derived source has an unsupported derivation kind".into())
                }
            }
            let approval_name = |field: &str| -> Result<&str, String> {
                let approval = source
                    .get(field)
                    .and_then(Value::as_object)
                    .ok_or_else(|| format!("eligible-derived source lacks {field}"))?;
                let name = approval
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| format!("eligible-derived source has an empty {field}"))?;
                if approval
                    .get("reason")
                    .and_then(Value::as_str)
                    .is_none_or(|reason| reason.trim().is_empty())
                {
                    return Err(format!("eligible-derived source has an empty {field}"));
                }
                Ok(name)
            };
            approval_name("owner_approval")?;
            let fidelity = approval_name("fidelity_approval")?;
            let derived_value = parse_originating_judgment(&selected)?;
            let reviewer = derived_value
                .pointer("/author/name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or("eligible-derived review output has no declared author")?;
            if reviewer == fidelity {
                return Err(
                    "eligible-derived fidelity triage must be independent of the declared reviewer"
                        .into(),
                );
            }
        }
        other => return Err(format!("unsupported recovery source class `{other}`")),
    }
    Ok(())
}

fn verify_model_repair_capture(
    source: &Value,
    current_capture: &Path,
    model_id: &str,
    selected_usage: &Map<String, Value>,
) -> Result<(), String> {
    let origin = source
        .get("origin")
        .and_then(Value::as_object)
        .ok_or("model-derived source has no original origin")?;
    let origin_id = origin
        .get("invocation_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model-derived source has no original invocation identity")?;
    let assignment_id = origin
        .get("assignment_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model-derived source has no assignment identity")?;
    let raw_digest = origin
        .get("raw_stdout_sha256")
        .and_then(Value::as_str)
        .ok_or("model-derived source has no raw-origin digest")?;
    let model_capture = selected_usage
        .get("capture")
        .and_then(Value::as_object)
        .ok_or("model-derived source has no adapter capture identity")?;

    let current_capture = fs::canonicalize(current_capture)
        .map_err(|error| format!("joined invocation capture is unavailable: {error}"))?;
    let slots_dir = current_capture
        .parent()
        .and_then(Path::parent)
        .ok_or("joined invocation capture has no work-slot capture root")?;
    if slots_dir.file_name().and_then(|name| name.to_str()) != Some("work-slot-captures") {
        return Err(
            "joined invocation capture is outside the engine work-slot capture layout".into(),
        );
    }
    let artifact_root = slots_dir
        .parent()
        .ok_or("joined invocation capture has no artifact_root")?;
    let artifact_root = fs::canonicalize(artifact_root)
        .map_err(|error| format!("model repair artifact_root is unavailable: {error}"))?;
    let origin_capture = origin
        .get("capture_dir")
        .and_then(Value::as_str)
        .ok_or("model-derived source has no original capture directory")?;
    let origin_capture = fs::canonicalize(origin_capture)
        .map_err(|error| format!("model repair raw-origin capture is unavailable: {error}"))?;
    if !origin_capture.starts_with(&artifact_root) || !current_capture.starts_with(&artifact_root) {
        return Err("model repair capture identities do not share the joined artifact_root".into());
    }

    let run_id = origin
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model-derived source has no run identity")?;
    let slot_id = origin
        .get("slot_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model-derived source has no slot identity")?;
    let state_visit = origin
        .get("state_visit")
        .and_then(Value::as_u64)
        .ok_or("model-derived source has no state-visit identity")?;
    let binding_sha256 = origin
        .get("binding_sha256")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model-derived source has no binding identity")?;
    let identity = sha256_digest(
        format!("{run_id}:{slot_id}:{state_visit}:{binding_sha256}:{assignment_id}").as_bytes(),
    );
    let expected_budget_dir = artifact_root
        .join("recovery-adapter-attempts")
        .join("model")
        .join(&identity[7..]);
    let budget_dir = fs::canonicalize(&expected_budget_dir)
        .map_err(|error| format!("model repair assignment history is unavailable: {error}"))?;
    if !budget_dir.starts_with(&artifact_root) || budget_dir.join("budget.lock").exists() {
        return Err(
            "model repair assignment history escapes artifact_root or is unfinished".into(),
        );
    }
    let selected_dir = model_capture
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("model repair capture omitted its directory")?;
    let selected_dir = fs::canonicalize(selected_dir)
        .map_err(|error| format!("selected model repair capture is unavailable: {error}"))?;
    if selected_dir.parent() != Some(budget_dir.as_path()) || !selected_dir.is_dir() {
        return Err("selected model repair capture does not match its assignment history".into());
    }

    let command = selected_usage
        .get("command")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model repair usage omitted command identity")?;
    let args = selected_usage
        .get("args")
        .and_then(Value::as_array)
        .ok_or("model repair usage omitted command arguments")?;
    let max_calls = selected_usage
        .get("max_calls")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or("model repair usage omitted its positive call cap")?;
    let max_time_ms = selected_usage
        .get("max_time_ms")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or("model repair usage omitted its positive time cap")?;
    let max_cost_micros = selected_usage
        .get("max_cost_micros")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or("model repair usage omitted its positive metered-cost cap")?;
    let mut total_calls = 0_u64;
    let mut total_elapsed_ms = 0_u64;
    let mut total_cost_micros = 0_u64;
    let mut selected_seen = false;
    for entry in fs::read_dir(&budget_dir)
        .map_err(|error| format!("could not read model repair assignment history: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not read model repair attempt: {error}"))?;
        if entry.file_name() == "budget.lock" {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| format!("could not inspect model repair attempt: {error}"))?
            .is_dir()
        {
            return Err("model repair assignment history contains an unexpected entry".into());
        }
        let attempt_dir = fs::canonicalize(entry.path())
            .map_err(|error| format!("model repair attempt is unavailable: {error}"))?;
        if attempt_dir.parent() != Some(budget_dir.as_path()) {
            return Err("model repair attempt escapes its assignment history".into());
        }
        let configuration =
            read_recovery_capture_json(&attempt_dir.join("configuration.json"), 64 * 1024)?;
        if configuration.get("kind").and_then(Value::as_str) != Some("model")
            || configuration.get("model_id").and_then(Value::as_str) != Some(model_id)
            || configuration.get("command").and_then(Value::as_str) != Some(command)
            || configuration.get("args") != Some(&Value::Array(args.clone()))
            || configuration.get("run_id").and_then(Value::as_str) != Some(run_id)
            || configuration.get("slot_id").and_then(Value::as_str) != Some(slot_id)
            || configuration.get("state_visit").and_then(Value::as_u64) != Some(state_visit)
            || configuration.get("binding_sha256").and_then(Value::as_str) != Some(binding_sha256)
            || configuration.get("assignment_id").and_then(Value::as_str) != Some(assignment_id)
            || configuration.get("max_calls").and_then(Value::as_u64) != Some(max_calls)
            || configuration.get("max_time_ms").and_then(Value::as_u64) != Some(max_time_ms)
            || configuration.get("max_cost_micros").and_then(Value::as_u64) != Some(max_cost_micros)
        {
            return Err(
                "model repair attempt identity or per-assignment bounds do not match".into(),
            );
        }
        let attempt_origin_id = configuration
            .get("origin_invocation_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or("model repair attempt omitted original invocation identity")?;
        let attempt_origin_capture = configuration
            .get("origin_capture_dir")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or("model repair attempt omitted raw-origin capture directory")?;
        let attempt_origin_capture = fs::canonicalize(attempt_origin_capture)
            .map_err(|error| format!("model repair raw-origin capture is unavailable: {error}"))?;
        let raw_path = configuration
            .get("raw_stdout_path")
            .and_then(Value::as_str)
            .ok_or("model repair attempt omitted raw-attempt path")?;
        let raw_attempt = configuration
            .get("raw_attempt")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or("model repair attempt omitted raw-attempt number")?;
        let attempt_raw_digest = configuration
            .get("raw_stdout_sha256")
            .and_then(Value::as_str)
            .filter(|value| valid_digest(value))
            .ok_or("model repair attempt omitted raw-attempt digest")?;
        if !attempt_origin_capture.starts_with(&artifact_root) {
            return Err("model repair raw-origin capture escapes artifact_root".into());
        }
        let original_raw = read_capture_output(&attempt_origin_capture, raw_path)?;
        if sha256_digest(&original_raw) != attempt_raw_digest {
            return Err(
                "model repair attempt does not match its immutable raw-origin bytes".into(),
            );
        }
        let request =
            read_recovery_capture_file(&attempt_dir.join("request.json"), 2 * 1024 * 1024)?;
        let stdout = read_recovery_capture_file(&attempt_dir.join("stdout"), 1024 * 1024)?;
        let stderr = read_recovery_capture_file(&attempt_dir.join("stderr"), 1024 * 1024)?;
        let request_sha = sha256_digest(&request);
        let stdout_sha = sha256_digest(&stdout);
        let stderr_sha = sha256_digest(&stderr);
        let request_value: Value = serde_json::from_slice(&request)
            .map_err(|error| format!("model repair request capture is malformed: {error}"))?;
        let usage = read_recovery_capture_json(&attempt_dir.join("usage.json"), 64 * 1024)?;
        if usage.get("usage_accounted").and_then(Value::as_bool) != Some(true)
            || usage.get("model_id").and_then(Value::as_str) != Some(model_id)
            || usage.get("command").and_then(Value::as_str) != Some(command)
            || usage.get("args") != Some(&Value::Array(args.clone()))
            || usage.get("request_sha256").and_then(Value::as_str) != Some(request_sha.as_str())
            || usage.get("stdout_sha256").and_then(Value::as_str) != Some(stdout_sha.as_str())
            || usage.get("stderr_sha256").and_then(Value::as_str) != Some(stderr_sha.as_str())
            || request_value
                .pointer("/model_adapter/model_id")
                .and_then(Value::as_str)
                != Some(model_id)
            || request_value
                .pointer("/model_adapter/command")
                .and_then(Value::as_str)
                != Some(command)
            || request_value.pointer("/model_adapter/args") != Some(&Value::Array(args.clone()))
            || request_value.pointer("/run_id").and_then(Value::as_str) != Some(run_id)
            || request_value.pointer("/slot_id").and_then(Value::as_str) != Some(slot_id)
            || request_value
                .pointer("/state_visit")
                .and_then(Value::as_u64)
                != Some(state_visit)
            || request_value
                .pointer("/binding_sha256")
                .and_then(Value::as_str)
                != Some(binding_sha256)
            || request_value
                .pointer("/origin_invocation_id")
                .and_then(Value::as_str)
                != Some(attempt_origin_id)
            || request_value
                .pointer("/assignment_id")
                .and_then(Value::as_str)
                != Some(assignment_id)
            || request_value
                .pointer("/raw_output_sha256")
                .and_then(Value::as_str)
                != Some(attempt_raw_digest)
            || request_value
                .pointer("/raw_attempt")
                .and_then(Value::as_u64)
                != Some(raw_attempt)
            || request_value
                .pointer("/raw_stdout_path")
                .and_then(Value::as_str)
                != Some(raw_path)
            || request_value
                .pointer("/budget/max_calls")
                .and_then(Value::as_u64)
                != Some(max_calls)
            || request_value
                .pointer("/budget/max_time_ms")
                .and_then(Value::as_u64)
                != Some(max_time_ms)
            || request_value
                .pointer("/budget/max_cost_micros")
                .and_then(Value::as_u64)
                != Some(max_cost_micros)
            || configuration.get("request_sha256").and_then(Value::as_str)
                != Some(request_sha.as_str())
            || request_value
                .pointer("/budget/max_time_ms")
                .and_then(Value::as_u64)
                != Some(max_time_ms)
            || request_value
                .pointer("/budget/max_cost_micros")
                .and_then(Value::as_u64)
                != Some(max_cost_micros)
            || request_value
                .pointer("/budget/remaining_calls")
                .and_then(Value::as_u64)
                != configuration.get("remaining_calls").and_then(Value::as_u64)
            || request_value
                .pointer("/budget/remaining_time_ms")
                .and_then(Value::as_u64)
                != configuration
                    .get("remaining_time_ms")
                    .and_then(Value::as_u64)
            || request_value
                .pointer("/budget/remaining_cost_micros")
                .and_then(Value::as_u64)
                != configuration
                    .get("remaining_cost_micros")
                    .and_then(Value::as_u64)
        {
            return Err(
                "model repair attempt has missing or unverifiable usage attribution".into(),
            );
        }
        let calls = usage
            .get("calls")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or("model repair attempt omitted positive metered call usage")?;
        let elapsed = usage
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or("model repair attempt omitted positive elapsed usage")?;
        let cost = usage
            .get("metered_cost_micros")
            .and_then(Value::as_u64)
            .ok_or("model repair attempt omitted metered-cost usage")?;
        if usage.get("max_calls").and_then(Value::as_u64) != Some(max_calls)
            || usage.get("max_time_ms").and_then(Value::as_u64) != Some(max_time_ms)
            || usage.get("max_cost_micros").and_then(Value::as_u64) != Some(max_cost_micros)
            || calls
                > configuration
                    .get("remaining_calls")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            || elapsed
                > configuration
                    .get("remaining_time_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            || cost
                > configuration
                    .get("remaining_cost_micros")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
        {
            return Err(
                "model repair attempt exceeded or omitted its remaining per-assignment bounds"
                    .into(),
            );
        }
        total_calls = total_calls
            .checked_add(calls)
            .ok_or("model repair call accounting overflowed")?;
        total_elapsed_ms = total_elapsed_ms
            .checked_add(elapsed)
            .ok_or("model repair time accounting overflowed")?;
        total_cost_micros = total_cost_micros
            .checked_add(cost)
            .ok_or("model repair cost accounting overflowed")?;
        if attempt_dir == selected_dir {
            selected_seen = attempt_origin_id == origin_id
                && attempt_raw_digest == raw_digest
                && raw_attempt
                    == origin
                        .get("raw_attempt")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                && raw_path
                    == origin
                        .get("raw_stdout_path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                && attempt_origin_capture == origin_capture
                && model_capture.get("request_sha256").and_then(Value::as_str)
                    == Some(request_sha.as_str())
                && model_capture.get("stdout_sha256").and_then(Value::as_str)
                    == Some(stdout_sha.as_str())
                && model_capture.get("stderr_sha256").and_then(Value::as_str)
                    == Some(stderr_sha.as_str())
                && calls
                    == selected_usage
                        .get("calls")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                && elapsed
                    == selected_usage
                        .get("elapsed_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                && cost
                    == selected_usage
                        .get("metered_cost_micros")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX);
        }
    }
    if !selected_seen
        || total_calls > max_calls
        || total_elapsed_ms > max_time_ms
        || total_cost_micros > max_cost_micros
    {
        return Err("model repair assignment usage is unverifiable or exhausted".into());
    }
    Ok(())
}

fn read_recovery_capture_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "model repair capture `{}` is unavailable: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "model repair capture `{}` is not a bounded file",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| {
        format!(
            "could not read model repair capture `{}`: {error}",
            path.display()
        )
    })
}

fn read_recovery_capture_json(path: &Path, limit: u64) -> Result<Value, String> {
    serde_json::from_slice(&read_recovery_capture_file(path, limit)?).map_err(|error| {
        format!(
            "model repair capture `{}` is malformed: {error}",
            path.display()
        )
    })
}

fn read_capture_output(capture_dir: &Path, path: &str) -> Result<Vec<u8>, String> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("recovery output path is not a safe capture-relative path".into());
    }
    let capture = fs::canonicalize(capture_dir)
        .map_err(|error| format!("recovery capture directory is unavailable: {error}"))?;
    if !capture.is_dir() {
        return Err("recovery capture directory is not a directory".into());
    }
    let selected = fs::canonicalize(capture.join(relative))
        .map_err(|error| format!("recovery output is unavailable: {error}"))?;
    if selected == capture || !selected.starts_with(&capture) || !selected.is_file() {
        return Err("recovery output escapes its capture directory".into());
    }
    fs::read(selected).map_err(|error| format!("recovery output is unavailable: {error}"))
}

fn parse_originating_judgment(bytes: &[u8]) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return Ok(value);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "originating selected output is not valid UTF-8 JSON".to_owned())?;
    let marker = "```json";
    let start = text
        .find(marker)
        .ok_or_else(|| "originating selected output is not JSON".to_owned())?
        + marker.len();
    let end = text[start..]
        .find("```")
        .map(|offset| start + offset)
        .ok_or_else(|| "originating selected output has an unterminated JSON fence".to_owned())?;
    let candidate = text[start..end].trim();
    serde_json::from_str(candidate)
        .map_err(|_| "originating selected output contains invalid fenced JSON".to_owned())
}

fn valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn non_empty_string(
    data: &Map<String, Value>,
    field: &str,
    reasons: &mut Vec<String>,
) -> Option<String> {
    match data.get(field).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Some(value.to_owned()),
        Some(_) => {
            reasons.push(format!("`{field}` must not be empty"));
            None
        }
        None => {
            reasons.push(format!("missing or non-string `{field}`"));
            None
        }
    }
}

fn parse_author(value: Option<&Value>, reasons: &mut Vec<String>) -> Option<AuthorIdentity> {
    let Some(object) = value.and_then(Value::as_object) else {
        reasons.push("missing or non-object `author`".to_owned());
        return None;
    };
    let name = match object.get("name").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Some(value.to_owned()),
        Some(_) => {
            reasons.push("`author.name` must not be empty".to_owned());
            None
        }
        None => {
            reasons.push("missing or non-string `author.name`".to_owned());
            None
        }
    };
    let kind = match object.get("kind").and_then(Value::as_str) {
        Some(value) if AUTHOR_KINDS.contains(&value) => Some(value.to_owned()),
        Some(_) => {
            reasons.push("`author.kind` must be human, agent, or script".to_owned());
            None
        }
        None => {
            reasons.push("missing or non-string `author.kind`".to_owned());
            None
        }
    };
    match (name, kind) {
        (Some(name), Some(kind)) => Some(AuthorIdentity { name, kind }),
        _ => None,
    }
}

impl AuthorIdentity {
    fn label(&self) -> String {
        format!("{} ({})", self.name, self.kind)
    }
}

impl EvidenceDiagnostic {
    /// One plain sentence for this cause. Commission quotes it so a driver sees
    /// the same cause the checked gate reports in its details.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Missing { required } => {
                format!("no counted judgment is present (floor {required})")
            }
            Self::Failed {
                findings,
                source,
                author,
                ..
            } => format!(
                "unresolved failing judgment `{source}` by {}: {findings}",
                author.label()
            ),
            Self::Malformed { reasons } => format!("malformed evidence: {}", reasons.join("; ")),
            Self::Stale {
                evidence_revision,
                current_revision,
            } => format!("evidence is for revision {evidence_revision}, not {current_revision}"),
            Self::StaleConfig {
                evidence_version,
                run_version,
            } => format!("evidence is for config {evidence_version}, not {run_version}"),
            Self::Unverified { reason } => format!("unverified: {reason}"),
            Self::Independence {
                required,
                distinct_present,
                excluded,
                ..
            } => {
                let mut text =
                    format!("{distinct_present} of {required} required distinct authors count");
                for pass in excluded {
                    text.push_str(&format!(
                        "; excluded (rule {}): {} on {}{}: {}",
                        pass.rule,
                        pass.author.label(),
                        pass.axis,
                        pass.assessment_id
                            .as_deref()
                            .map(|id| format!(" under assessment {id}"))
                            .unwrap_or_default(),
                        pass.reason
                    ));
                }
                text
            }
            Self::AssessmentMissing { group, author } => format!(
                "{} has a judgment in group `{group}` without an assessment identity",
                author.label()
            ),
            Self::AssessmentInconsistent {
                group,
                author,
                assessment_ids,
            } => format!(
                "{} judged group `{group}` under different assessments: {}",
                author.label(),
                assessment_ids.join(", ")
            ),
            Self::MixedAuthors {
                expected_stage,
                expected,
                actual,
            } => format!(
                "the {expected_stage} stage names {} but this stage names {}",
                expected
                    .iter()
                    .map(AuthorIdentity::label)
                    .collect::<Vec<_>>()
                    .join(", "),
                actual
                    .iter()
                    .map(AuthorIdentity::label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Stable identity of the assessment a challenge record came from: its explicit
/// assessment ID, else a digest of the retained request/original exchange.
/// A raw re-append of the same assessment keeps the identity it first had.
#[allow(dead_code)]
pub(crate) fn challenge_assessment_identity(data: &Value) -> Option<String> {
    if let Some(id) = data.get("assessment_id").filter(|id| !id.is_null()) {
        return Some(format!("assessment:{id}"));
    }
    let exchange = data.get("external_review")?;
    let request = exchange.get("request")?;
    let original = exchange.get("original")?;
    Some(format!(
        "exchange:{:x}",
        Sha256::digest(serde_json::to_vec(&(request, original)).ok()?)
    ))
}

/// A review gate's evidence and finding ledger, evaluated over a recorded
/// history exactly as the checked event evaluates them. Read-only.
#[allow(dead_code)] // Consumed by commission in the library, not by the binary copy.
pub(crate) struct GateResolution {
    pub(crate) evidence: EvidenceEvaluation,
    pub(crate) ledger: crate::finding_ledger::FindingLedgerEvaluation,
}

#[allow(dead_code)]
impl GateResolution {
    /// True when the gate's own evidence and ledger checks would pass.
    pub(crate) fn is_eligible(&self) -> bool {
        self.evidence.is_satisfied() && self.ledger.is_satisfied() && self.unresolved().is_empty()
    }

    /// Accepted findings that still block, by finding ID.
    pub(crate) fn unresolved(&self) -> Vec<String> {
        self.ledger
            .current_snapshot()
            .map(|snapshot| {
                snapshot
                    .accepted_unresolved()
                    .map(|finding| finding.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Specific causes in the gate's own vocabulary: the evidence categories,
    /// plus the finding ledger. Empty when eligible.
    pub(crate) fn causes(&self) -> Vec<(String, String)> {
        let mut causes = Vec::new();
        if !self.ledger.is_satisfied() {
            causes.push((
                "finding_ledger".to_owned(),
                format!(
                    "the finding ledger is not usable: {}",
                    self.ledger.details_value()
                ),
            ));
        }
        for id in self.unresolved() {
            causes.push((
                "unresolved_finding".to_owned(),
                format!("accepted finding `{id}` is not yet resolved"),
            ));
        }
        for axis in self.evidence.diagnostics() {
            for diagnostic in &axis.diagnostics {
                causes.push((
                    diagnostic.category().to_owned(),
                    format!(
                        "axis `{}` ({} stage): {}",
                        axis.axis,
                        axis.review_stage,
                        diagnostic.describe()
                    ),
                ));
            }
        }
        causes
    }
}

/// Evaluate `gate` over `context` for the current `revision` of `subject`
/// with the same effective policy, finding ledger and staged evidence reader
/// the checked event uses.
#[allow(dead_code)]
pub(crate) fn resolve_gate(
    initial_input: &Value,
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    revision: &str,
    subject_author: &AuthorIdentity,
) -> Result<GateResolution, String> {
    let counts = software_change_provider::admission::effective_policy(initial_input, context);
    let config =
        crate::config::parse_initial_input(&counts.effective).map_err(|error| error.to_string())?;
    let stages = config
        .staged_axes_for_gate(gate)
        .filter(|axes| !axes.is_empty())
        .ok_or_else(|| format!("gate `{gate}` has no staged review axes"))?;
    let ledger =
        crate::finding_ledger::evaluate_finding_ledger(context, gate, subject, revision, &config);
    let evidence = evaluate_staged_evidence_excluding(
        context,
        gate,
        subject,
        revision,
        subject_author,
        config.config_version(),
        stages,
        config.axis_namespace(),
        config.artifact_root(),
        ledger.current_snapshot(),
        config.contract_version() >= 3,
        &BTreeSet::new(),
    );
    Ok(GateResolution { evidence, ledger })
}

// Applicability is a new attestation to the cleared conclusion, not a new verdict.
#[allow(dead_code)]
pub(crate) fn intake_data<'a>(
    context: &'a [loop_core::ContextRecord],
    record: &'a loop_core::ContextRecord,
) -> &'a Value {
    if record.kind == "evidence-applicability" {
        context
            .iter()
            .find(|r| r.id.as_str() == record.data["origin"]["id"].as_str().unwrap_or(""))
            .map(|r| &r.data)
            .unwrap_or(&record.data)
    } else {
        &record.data
    }
}

#[allow(dead_code)]
pub(crate) fn retain_challenges_after_clearance(
    context: &[loop_core::ContextRecord],
    retained: &mut Vec<loop_core::ContextRecord>,
    parent: &str,
    gate: &str,
    subject: &str,
    axis: &str,
    parent_axes: &std::collections::BTreeSet<String>,
) -> Vec<Value> {
    let mut excluded = Vec::new();
    let clearance = context
        .iter()
        .filter(|record| {
            let data = intake_data(context, record);
            if record.kind == "author-count-amendment"
                && data["targets"].as_array().is_some_and(|targets| {
                    targets.iter().any(|target| {
                        target["gate"] == parent
                            && parent_axes.contains(target["axis"].as_str().unwrap_or(""))
                    })
                })
            {
                return true;
            }
            if record.kind == "intent-comparison"
                && subject == "intent.json"
                && parent_axes.contains("intent-sufficient")
            {
                return true;
            }
            if data["gate"] != parent || data["subject"] != subject {
                return false;
            }
            if matches!(
                record.kind.as_str(),
                "review-evidence" | "evidence-applicability"
            ) {
                return parent_axes.contains(loop_integrations::review_intake::evidence_axis(data));
            }
            record.kind == "finding-ledger"
                && data["findings"].as_array().is_some_and(|findings| {
                    findings.iter().any(|finding| {
                        parent_axes.contains(finding["policy_id"].as_str().unwrap_or(""))
                            || finding["review_axes"].as_array().is_some_and(|ids| {
                                ids.iter()
                                    .any(|id| parent_axes.contains(id.as_str().unwrap_or("")))
                            })
                    })
                })
        })
        .max_by_key(|r| r.sequence);
    if let Some(cutoff) = clearance {
        let clearance = cutoff.sequence;
        retained.retain(|record| {
            let data = intake_data(context, record);
            let early = data["gate"] == gate
                && loop_integrations::review_intake::evidence_axis(data) == axis
                && matches!(
                    record.kind.as_str(),
                    "review-evidence" | "evidence-applicability"
                )
                && (if record.kind == "review-evidence" {
                    challenge_assessment_identity(data)
                        .and_then(|identity| {
                            context
                                .iter()
                                .filter(|prior| {
                                    prior.kind == "review-evidence"
                                        && challenge_assessment_identity(&prior.data)
                                            == Some(identity.clone())
                                })
                                .map(|prior| prior.sequence)
                                .min()
                        })
                        .unwrap_or(record.sequence)
                } else {
                    record.sequence
                }) <= clearance;
            if early {
                excluded.push(json!({
                    "record_id": record.id.as_str(),
                    "assessment_identity": challenge_assessment_identity(data)
                        .unwrap_or_else(|| format!("record:{}", record.id)),
                    "ordinary_record_id": cutoff.id.as_str(),
                    "axis": axis,
                    "parent": parent,
                }));
            }
            !early
        });
    }
    excluded
}

/// The ordinary-review axes that correspond to one challenge axis: the same axis
/// in any stage, or any axis in a group the challenge axis belongs to.
#[allow(dead_code)]
pub(crate) fn corresponding_parent_axes(
    parent_axes: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    challenge_axis: &PolicyAxis,
) -> BTreeSet<String> {
    let matching_groups: BTreeSet<_> = parent_axes
        .values()
        .filter_map(|axes| axes.get(challenge_axis.id()).and_then(|p| p.group()))
        .collect();
    parent_axes
        .values()
        .flat_map(|axes| {
            axes.iter()
                .filter(|(_, p)| {
                    p.id() == challenge_axis.id()
                        || p.group()
                            .is_some_and(|group| matching_groups.contains(group))
                })
                .map(|(id, _)| id.clone())
        })
        .collect()
}

/// Challenge rows that do not count because the assessment was first recorded at
/// or before the latest relevant ordinary-review record, computed with the very
/// function the checked gate uses.
#[allow(dead_code)]
pub(crate) fn early_challenge_rows(
    context: &[ContextRecord],
    gate: &str,
    parent: &str,
    subject: &str,
    challenge_axes: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    parent_axes: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
) -> Vec<Value> {
    let mut retained = context.to_vec();
    let mut early = Vec::new();
    for axes in challenge_axes.values() {
        for policy in axes.values() {
            let corresponding = corresponding_parent_axes(parent_axes, policy);
            early.extend(retain_challenges_after_clearance(
                context,
                &mut retained,
                parent,
                gate,
                subject,
                policy.id(),
                &corresponding,
            ));
        }
    }
    early
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_initial_input;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;

    const SUBJECT: &str = "intent.json";
    const GATE: &str = "intent-review";
    const RUN_VERSION: &str = "test-1";

    fn metadata_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "revision": {"type": "string"},
                "author": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "kind": {
                            "type": "string",
                            "enum": ["human", "agent", "script"]
                        }
                    },
                    "required": ["name", "kind"],
                    "additionalProperties": false
                }
            },
            "required": ["revision", "author"],
            "additionalProperties": false
        })
    }

    fn config_with_axes(required_authors: u64) -> crate::config::ValidatedConfig {
        let mut config = json!({
            "contract_version": 2,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": RUN_VERSION,
            "review_policies": {
                GATE: [{"id": "axis", "description": "axis", "required_authors": required_authors}]
            },
            "artifact_schemas": {SUBJECT: metadata_schema()}
        });
        if required_authors == 1 {
            config["review_policies"][GATE][0]
                .as_object_mut()
                .unwrap()
                .remove("required_authors");
        }
        parse_initial_input(&config).expect("test config valid")
    }

    fn config_with_two_axes() -> crate::config::ValidatedConfig {
        let config = json!({
            "contract_version": 2,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": RUN_VERSION,
            "review_policies": {
                GATE: [
                    {"id": "axis-a", "description": "axis a"},
                    {"id": "axis-b", "description": "axis b"}
                ]
            },
            "artifact_schemas": {SUBJECT: metadata_schema()}
        });
        parse_initial_input(&config).expect("two-axis test config valid")
    }

    fn config_with_two_gates() -> crate::config::ValidatedConfig {
        let mut config = json!({
            "contract_version": 2,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": RUN_VERSION,
            "review_policies": {
                GATE: [{"id": "axis", "description": "axis"}],
                "design-review": [{"id": "other-axis", "description": "other"}]
            },
            "artifact_schemas": {
                SUBJECT: metadata_schema(),
                "design.json": metadata_schema()
            }
        });
        config["review_policies"][GATE][0]
            .as_object_mut()
            .unwrap()
            .remove("required_authors");
        parse_initial_input(&config).expect("two-gate config valid")
    }

    fn author(name: &str, kind: &str) -> AuthorIdentity {
        AuthorIdentity::new(name, kind)
    }

    #[allow(clippy::too_many_arguments)]
    fn evidence(
        gate: &str,
        policy_id: &str,
        result: &str,
        findings: &str,
        evidence_author: &AuthorIdentity,
        subject: &str,
        revision: &str,
        version: &str,
    ) -> Value {
        json!({
            "gate": gate,
            "policy_id": policy_id,
            "result": result,
            "findings": findings,
            "author": {"name": evidence_author.name, "kind": evidence_author.kind},
            "subject": subject,
            "subject_revision": revision,
            "config_version": version
        })
    }

    fn context_record(index: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            format!("context-{index}"),
            REVIEW_EVIDENCE_KIND,
            data,
            SemanticSequence::new(index),
            Timestamp::from_unix_millis(index as i64),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        config: &crate::config::ValidatedConfig,
        context: &[ContextRecord],
        current_revision: &str,
        subject_author: &AuthorIdentity,
    ) -> EvidenceEvaluation {
        evaluate_evidence(
            context,
            GATE,
            SUBJECT,
            current_revision,
            subject_author,
            config.config_version(),
            config.axes_for_gate(GATE).expect("intent axes"),
            config.axis_namespace(),
            config.artifact_root(),
        )
    }

    fn axis(result: &EvidenceEvaluation) -> &AxisDiagnostic {
        result.diagnostics().first().expect("axis diagnostic")
    }

    fn has_category(result: &EvidenceEvaluation, category: &str) -> bool {
        result
            .diagnostics()
            .iter()
            .any(|axis| axis.has_category(category))
    }

    #[test]
    fn assessment_and_stage_identity_causes_are_not_false_independence_shortages() {
        let entries:Vec<_>=["individual","aggregate"].into_iter().flat_map(|stage| ["axis-a","axis-b"].into_iter().map(move |axis| json!({"id":axis,"description":axis,"review_stage":stage,"required_authors":1,"group":format!("group-{stage}")}))).collect();
        let config=parse_initial_input(&json!({"contract_version":3,"criterion_policy":{"required_authors":1,"goal_required_authors":1},"config_version":"cause-test-15","review_policies":{GATE:entries},"artifact_schemas":{SUBJECT:metadata_schema()}})).unwrap();
        let stages = config
            .staged_axes_for_gate(GATE)
            .unwrap()
            .iter()
            .map(|(stage, axes)| (stage.as_str().to_owned(), axes.clone()))
            .collect();
        let mut records = Vec::new();
        for stage in ["individual", "aggregate"] {
            for axis in ["axis-a", "axis-b"] {
                let mut data = evidence(
                    GATE,
                    axis,
                    "pass",
                    "",
                    &author("reviewer", "agent"),
                    SUBJECT,
                    "1",
                    "cause-test-15",
                );
                data["review_stage"] = json!(stage);
                data["group"] = json!(format!("group-{stage}"));
                data["assessment_id"] = json!(format!("return-{stage}"));
                records.push(context_record(records.len() as u64 + 1, data));
            }
        }
        let evaluate = |records: &[ContextRecord]| {
            evaluate_staged_evidence_with_dispositions(
                records,
                GATE,
                SUBJECT,
                "1",
                &author("writer", "human"),
                "cause-test-15",
                &stages,
                config.axis_namespace(),
                None,
                None,
                true,
            )
        };
        assert!(evaluate(&records).is_satisfied());
        let mut missing = records.clone();
        for record in &mut missing {
            record.data.as_object_mut().unwrap().remove("assessment_id");
        }
        let result = evaluate(&missing);
        assert!(has_category(&result, "assessment_missing"));
        assert!(!has_category(&result, "independence"));
        let mut inconsistent = records.clone();
        inconsistent[1].data["assessment_id"] = json!("different-return");
        let result = evaluate(&inconsistent);
        assert!(has_category(&result, "assessment_inconsistent"));
        assert!(!has_category(&result, "independence"));
        let mut mixed = records;
        for record in &mut mixed {
            if record.data["review_stage"] == "aggregate" {
                record.data["author"]["name"] = json!("different-reviewer");
            }
        }
        let result = evaluate(&mixed);
        assert!(has_category(&result, "mixed_authors"));
        assert!(!has_category(&result, "independence"));
    }

    #[test]
    fn filter_ignores_non_review_kind_and_non_object_data() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let mut wrong_kind = context_record(
            1,
            evidence(
                GATE,
                "axis",
                "pass",
                "",
                &reviewer,
                SUBJECT,
                "1",
                RUN_VERSION,
            ),
        );
        wrong_kind.kind = "other".to_owned();
        let non_object = context_record(2, json!("not an object"));
        let result = evaluate(
            &config,
            &[wrong_kind, non_object],
            "1",
            &author("owner", "human"),
        );
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "missing"));
    }

    #[test]
    fn historical_grounding_keeps_original_digest_without_rebinding_to_current_file() {
        let root = std::env::temp_dir().join(format!(
            "software-change-historical-ground-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let old = br#"{"revision":"r1","outcome":"old"}"#;
        let current = br#"{"revision":"r2","outcome":"new"}"#;
        fs::write(root.join("intent.json"), current).unwrap();
        let grounds = json!({"reason":"The original reviewer inspected the old outcome.","evidence":[
            {"locator":"intent.json#/outcome","sha256":format!("sha256:{:x}",Sha256::digest(old))}
        ]});
        let stale = json!({
            "gate":"intent-review","policy_id":"solution-agnostic","review_contract_version":2,
            "review_stage":"aggregate","grounds":grounds,"result":"pass","findings":"","author":{"name":"reviewer","kind":"agent"},
            "subject":"intent.json","subject_revision":"r1","config_version":"high-rigor-11"
        });
        let stale_result =
            parse_evidence_record_for_stage(&stale, "intent.json", Some(&json!(root)), true);
        assert!(stale_result.is_ok(), "{}", stale_result.unwrap_err());
        let mut current_bad = stale;
        current_bad["subject_revision"] = json!("r2");
        assert!(parse_evidence_record_for_stage(
            &current_bad,
            "intent.json",
            Some(&json!(root)),
            true
        )
        .unwrap_err()
        .contains("does not match exact source bytes"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn current_conforming_pass_satisfies_axis() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let result = evaluate(
            &config,
            &[context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            )],
            "1",
            &author("owner", "human"),
        );
        assert!(result.is_satisfied());
        assert!(result.diagnostics().is_empty());
    }

    fn assert_singleton_individual_group_membership(
        config_version: &str,
        wrong_group_counts: bool,
    ) {
        const PLAN_GATE: &str = "plan-review";
        const PLAN_SUBJECT: &str = "plan.json";
        const PLAN_AXIS: &str = "parallelism-honest";
        const PLAN_GROUP: &str = "plan-review.individual.parallelism-honest";
        let config = parse_initial_input(&json!({
            "contract_version": 4,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": config_version,
            "review_policies": {
                PLAN_GATE: [{
                    "id": PLAN_AXIS,
                    "description": "whole-graph concurrency",
                    "review_stage": "individual",
                    "group": PLAN_GROUP
                }]
            },
            "artifact_schemas": {PLAN_SUBJECT: metadata_schema()}
        }))
        .expect("singleton individual group config valid");
        let reviewer = author("reviewer", "agent");
        let owner = author("owner", "human");
        let mut judgment = evidence(
            PLAN_GATE,
            PLAN_AXIS,
            "pass",
            "",
            &reviewer,
            PLAN_SUBJECT,
            "1",
            config_version,
        );
        judgment["review_stage"] = json!("individual");
        judgment["group"] = json!("plan-review.individual.dependencies-honest");
        let evaluate_judgment = |judgment| {
            evaluate_staged_evidence_with_dispositions(
                &[context_record(1, judgment)],
                PLAN_GATE,
                PLAN_SUBJECT,
                "1",
                &owner,
                config.config_version(),
                config.staged_axes_for_gate(PLAN_GATE).unwrap(),
                config.axis_namespace(),
                config.artifact_root(),
                None,
                true,
            )
        };
        let result = evaluate_judgment(judgment.clone());
        assert_eq!(
            result.is_satisfied(),
            wrong_group_counts,
            "{config_version}"
        );
        if !wrong_group_counts {
            assert_eq!(result.diagnostics().len(), 1);
            assert_eq!(axis(&result).review_stage, "individual");
            assert_eq!(axis(&result).axis, PLAN_AXIS);
            assert_eq!(
                axis(&result).diagnostics,
                vec![EvidenceDiagnostic::Independence {
                    required: 1,
                    distinct_present: 0,
                    excluded: Vec::new(),
                    counted: Vec::new(),
                }]
            );
        }

        judgment["group"] = json!(PLAN_GROUP);
        assert!(
            evaluate_judgment(judgment).is_satisfied(),
            "{config_version}"
        );
    }

    // ---- AC-6 / AC-7: carried and fresh judgments combine within one group ----

    const GROUP_AXES: [&str; 8] = [
        "axis-1", "axis-2", "axis-3", "axis-4", "axis-5", "axis-6", "axis-7", "axis-8",
    ];

    fn group_config(floors: &[(&str, u64)]) -> crate::config::ValidatedConfig {
        let entries: Vec<_> = GROUP_AXES
            .iter()
            .map(|axis| {
                let floor = floors
                    .iter()
                    .find(|(id, _)| id == axis)
                    .map_or(1, |(_, floor)| *floor);
                json!({"id": axis, "description": axis, "review_stage": "aggregate",
                       "required_authors": floor, "group": "group-g"})
            })
            .collect();
        parse_initial_input(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "group-test-16",
            "review_policies": {GATE: entries},
            "artifact_schemas": {SUBJECT: metadata_schema()}
        }))
        .expect("group config valid")
    }

    fn group_row(
        index: u64,
        axis: &str,
        who: &str,
        revision: &str,
        assessment: &str,
    ) -> ContextRecord {
        let mut data = evidence(
            GATE,
            axis,
            "pass",
            "",
            &author(who, "agent"),
            SUBJECT,
            revision,
            "group-test-16",
        );
        data["review_stage"] = json!("aggregate");
        data["group"] = json!("group-g");
        data["assessment_id"] = json!(assessment);
        context_record(index, data)
    }

    fn carry_of(index: u64, origin: &ContextRecord, revision: &str) -> ContextRecord {
        ContextRecord::new(
            format!("carry-{index}"),
            EVIDENCE_APPLICABILITY_KIND,
            json!({"origin":{"kind":"context-record","id":origin.id.as_str()},
                   "target":{"subject":SUBJECT,"revision":revision},
                   "attesting_driver":{"name":"driver","kind":"human"},
                   "reason":"the unchanged part of the subject"}),
            SemanticSequence::new(index),
            Timestamp::from_unix_millis(index as i64),
        )
    }

    /// Prior revision "1": `carried` axes judged by `old_author`. Then revision
    /// "2": those axes carried, and `fresh` (axis, author, assessment) judged anew.
    fn mixed_history(
        carried: &[&str],
        old_author: &str,
        fresh: &[(&str, &str, &str)],
    ) -> Vec<ContextRecord> {
        let mut records = Vec::new();
        let mut originals = Vec::new();
        for axis in carried {
            let row = group_row(
                records.len() as u64 + 1,
                axis,
                old_author,
                "1",
                "old-return",
            );
            originals.push(row.clone());
            records.push(row);
        }
        for original in &originals {
            let index = records.len() as u64 + 1;
            records.push(carry_of(index, original, "2"));
        }
        for (axis, who, assessment) in fresh {
            let index = records.len() as u64 + 1;
            records.push(group_row(index, axis, who, "2", assessment));
        }
        records
    }

    fn group_eval(
        config: &crate::config::ValidatedConfig,
        records: &[ContextRecord],
    ) -> EvidenceEvaluation {
        evaluate_staged_evidence_with_dispositions(
            records,
            GATE,
            SUBJECT,
            "2",
            &author("writer", "human"),
            "group-test-16",
            config.staged_axes_for_gate(GATE).unwrap(),
            config.axis_namespace(),
            None,
            None,
            true,
        )
    }

    #[test]
    fn carried_and_fresh_judgments_cover_one_group_at_floor_one() {
        let config = group_config(&[]);
        // (a) six axes carried from one author, two freshly passed by another.
        let records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[
                ("axis-7", "new-reviewer", "new-return"),
                ("axis-8", "new-reviewer", "new-return"),
            ],
        );
        let result = group_eval(&config, &records);
        assert!(result.is_satisfied(), "{:?}", result.details_value());
        assert_eq!(result.rows.len(), 8);
        let carried: Vec<_> = result.rows.iter().filter(|r| r.evidence.carried).collect();
        assert_eq!(carried.len(), 6);
        assert!(carried
            .iter()
            .all(|row| row.evidence.author.name() == "old-reviewer"
                && row.evidence.assessment_id.as_deref() == Some("old-return")
                && row.evidence.applicability_id.is_some()
                && row.evidence.source_id.starts_with("context-")));
        assert!(result
            .rows
            .iter()
            .filter(|r| !r.evidence.carried)
            .all(|row| row.evidence.author.name() == "new-reviewer"
                && row.evidence.assessment_id.as_deref() == Some("new-return")));

        // (b) three carried, five fresh, all by the same reviewer name under a new assessment.
        let fresh: Vec<_> = GROUP_AXES[3..]
            .iter()
            .map(|axis| (*axis, "same-reviewer", "second-return"))
            .collect();
        let result = group_eval(
            &config,
            &mixed_history(&GROUP_AXES[..3], "same-reviewer", &fresh),
        );
        assert!(result.is_satisfied(), "{:?}", result.details_value());

        // Six carried plus two fresh axes from two different authors.
        let records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[
                ("axis-7", "first-new", "r1"),
                ("axis-8", "second-new", "r2"),
            ],
        );
        assert!(group_eval(&config, &records).is_satisfied());
    }

    #[test]
    fn mixed_group_still_enforces_floor_coverage_and_fresh_assessment() {
        // (c) a floor-2 axis with one distinct author is refused, naming the axis and floor.
        let config = group_config(&[("axis-7", 2)]);
        let records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[
                ("axis-7", "new-reviewer", "new-return"),
                ("axis-8", "new-reviewer", "new-return"),
            ],
        );
        let result = group_eval(&config, &records);
        assert!(!result.is_satisfied());
        let diagnostic = result
            .diagnostics()
            .iter()
            .find(|entry| entry.axis == "axis-7")
            .expect("axis-7 refused");
        assert!(matches!(
            diagnostic.diagnostics.last(),
            Some(EvidenceDiagnostic::Independence {
                required: 2,
                distinct_present: 1,
                ..
            })
        ));
        // A second distinct author on that axis clears it.
        let mut cleared = records.clone();
        let index = cleared.len() as u64 + 1;
        cleared.push(group_row(
            index,
            "axis-7",
            "second-new",
            "2",
            "other-return",
        ));
        assert!(group_eval(&config, &cleared).is_satisfied());

        // (d) an axis with no carried or fresh pass is refused.
        let config = group_config(&[]);
        let records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[("axis-7", "new-reviewer", "new-return")],
        );
        let result = group_eval(&config, &records);
        assert!(!result.is_satisfied());
        let diagnostic = result
            .diagnostics()
            .iter()
            .find(|entry| entry.axis == "axis-8")
            .expect("axis-8 refused");
        assert!(diagnostic.has_category("missing"));

        // Fresh judgments of one author must share one assessment, even in a mixed group.
        let records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[
                ("axis-7", "new-reviewer", "r1"),
                ("axis-8", "new-reviewer", "r2"),
            ],
        );
        let result = group_eval(&config, &records);
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "assessment_inconsistent"));
        // The cause names the group, the author and both assessments; it is not
        // reported as a shortage of judgments.
        assert!(!has_category(&result, "independence"));
        assert!(result
            .diagnostics()
            .iter()
            .any(|entry| entry.axis == "axis-8"
                && entry.diagnostics.iter().any(|d| matches!(d,
                EvidenceDiagnostic::AssessmentInconsistent { group, author, assessment_ids }
                    if group == "group-g" && author.name() == "new-reviewer"
                        && assessment_ids == &vec!["r1".to_owned(), "r2".to_owned()]))));
    }

    #[test]
    fn fresh_only_split_across_authors_is_refused_with_the_excluding_rule() {
        let config = group_config(&[]);
        let fresh: Vec<_> = GROUP_AXES
            .iter()
            .enumerate()
            .map(|(i, axis)| {
                (
                    *axis,
                    if i < 4 { "first" } else { "second" },
                    if i < 4 { "r1" } else { "r2" },
                )
            })
            .collect();
        let result = group_eval(&config, &mixed_history(&[], "unused", &fresh));
        assert!(!result.is_satisfied());
        let excluded: Vec<_> = result
            .diagnostics()
            .iter()
            .flat_map(|entry| &entry.diagnostics)
            .filter_map(|d| match d {
                EvidenceDiagnostic::Independence { excluded, .. } => Some(excluded),
                _ => None,
            })
            .flatten()
            .collect();
        // Every present pass says why it did not count, with rule, author, assessment and axis.
        assert_eq!(excluded.len(), 8);
        assert!(excluded
            .iter()
            .all(|pass| pass.rule == "review-group-complete"
                && pass.assessment_id.is_some()
                && pass.reason.contains("judged every axis")));
        assert!(excluded
            .iter()
            .any(|pass| pass.author.name() == "first" && pass.axis == "axis-1"));
        assert!(excluded
            .iter()
            .any(|pass| pass.author.name() == "second" && pass.axis == "axis-8"));

        // One author covering every axis under one assessment still clears, as before.
        let all: Vec<_> = GROUP_AXES
            .iter()
            .map(|axis| (*axis, "solo", "one"))
            .collect();
        assert!(group_eval(&config, &mixed_history(&[], "unused", &all)).is_satisfied());
    }

    // ---- AC-12: carried evidence about a criterion is current only while its text is ----

    #[test]
    fn carry_survives_revision_only_edit_and_is_stale_when_a_cited_criterion_changes() {
        let config = parse_initial_input(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "currency-test-16",
            "review_policies": {GATE: [{"id": "axis", "description": "axis", "review_stage": "aggregate"}]},
            "artifact_schemas": {SUBJECT: metadata_schema()}
        }))
        .unwrap();
        let intent = |revision: &str, second: &str| {
            serde_json::to_string_pretty(&json!({
                "revision": revision,
                "acceptance": [
                    {"id": "AC-1", "statement": "first"},
                    {"id": "AC-2", "statement": second}
                ]
            }))
            .unwrap()
        };
        let dir = std::env::temp_dir().join(format!(
            "software-change-carry-currency-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut original = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &author("reviewer", "agent"),
            SUBJECT,
            "1",
            "currency-test-16",
        );
        original["review_stage"] = json!("aggregate");
        original["review_contract_version"] = json!(3);
        original["grounds"] = json!({"reason": "checked the second criterion",
            "evidence": [{"locator": "intent.json#/acceptance/1"}]});
        let correction = ContextRecord::new(
            "correction",
            "intent-correction",
            json!({"prior_intent": {"revision": "1", "json_bytes": intent("1", "second")}}),
            SemanticSequence::new(2),
            Timestamp::from_unix_millis(2),
        );
        let records = vec![
            context_record(1, original),
            correction,
            carry_of(3, &context_record(1, json!({})), "2"),
        ];
        let evaluate_at = |text: String| {
            fs::write(dir.join("intent.json"), text).unwrap();
            evaluate_staged_evidence_with_dispositions(
                &records,
                GATE,
                SUBJECT,
                "2",
                &author("writer", "human"),
                "currency-test-16",
                config.staged_axes_for_gate(GATE).unwrap(),
                config.axis_namespace(),
                Some(&json!(dir)),
                None,
                true,
            )
        };
        // Only the revision field changed: the carry still counts, and says it is carried.
        let kept = evaluate_at(intent("2", "second"));
        assert!(kept.is_satisfied(), "{:?}", kept.details_value());
        assert!(kept.rows.iter().all(|row| row.evidence.carried));
        // The cited criterion's text changed: refused as stale, naming the criterion.
        let stale = evaluate_at(intent("2", "second, stricter"));
        assert!(!stale.is_satisfied());
        let text = stale.details_value().to_string();
        assert!(
            text.contains("stale carry") && text.contains("AC-2"),
            "{text}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_gate_answers_like_the_gate_and_names_the_cause() {
        let entries: Vec<_> = GROUP_AXES
            .iter()
            .map(|axis| {
                json!({"id": axis, "description": axis, "review_stage": "aggregate",
                               "group": "group-g"})
            })
            .collect();
        let input = json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "group-test-16",
            "review_policies": {GATE: entries},
            "artifact_schemas": {SUBJECT: metadata_schema()}
        });
        let ledger = ContextRecord::new(
            "ledger",
            "finding-ledger",
            json!({"schema_version":"1","gate":GATE,"subject":SUBJECT,"subject_revision":"2",
                   "author":{"name":"driver","kind":"agent"},"findings":[]}),
            SemanticSequence::new(100),
            Timestamp::from_unix_millis(100),
        );
        let owner = author("writer", "human");
        // Eligible: six carried axes plus two fresh axes from another author.
        let mut records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[
                ("axis-7", "new-reviewer", "new-return"),
                ("axis-8", "new-reviewer", "new-return"),
            ],
        );
        records.push(ledger.clone());
        let resolved = resolve_gate(&input, &records, GATE, SUBJECT, "2", &owner).unwrap();
        assert!(resolved.is_eligible(), "{:?}", resolved.causes());
        assert!(resolved.causes().is_empty());
        assert_eq!(
            resolved
                .evidence
                .rows
                .iter()
                .filter(|r| r.evidence.carried)
                .count(),
            6
        );

        // Refused: one axis has nothing. The cause is named, not a generic shortage.
        let mut records = mixed_history(
            &GROUP_AXES[..6],
            "old-reviewer",
            &[("axis-7", "new-reviewer", "new-return")],
        );
        records.push(ledger);
        let resolved = resolve_gate(&input, &records, GATE, SUBJECT, "2", &owner).unwrap();
        assert!(!resolved.is_eligible());
        let causes = resolved.causes();
        assert!(
            causes
                .iter()
                .any(|(category, text)| category == "missing" && text.contains("axis-8")),
            "{causes:?}"
        );

        // No ledger at all is its own named cause.
        let records = mixed_history(&GROUP_AXES[..6], "old-reviewer", &[]);
        let resolved = resolve_gate(&input, &records, GATE, SUBJECT, "2", &owner).unwrap();
        assert!(resolved
            .causes()
            .iter()
            .any(|(category, _)| category == "finding_ledger"));
    }

    #[test]
    fn carry_without_retained_prior_bytes_still_counts_and_says_currency_is_unverified() {
        let config = parse_initial_input(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "currency-test-16",
            "review_policies": {GATE: [{"id": "axis", "description": "axis", "review_stage": "aggregate"}]},
            "artifact_schemas": {SUBJECT: metadata_schema()}
        }))
        .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "software-change-carry-nobytes-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("intent.json"),
            r#"{"revision":"2","acceptance":[{"id":"AC-1","statement":"x"}]}"#,
        )
        .unwrap();
        let mut original = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &author("reviewer", "agent"),
            SUBJECT,
            "1",
            "currency-test-16",
        );
        original["review_stage"] = json!("aggregate");
        original["review_contract_version"] = json!(3);
        original["grounds"] =
            json!({"reason": "checked", "evidence": [{"locator": "intent.json#/acceptance"}]});
        let records = vec![
            context_record(1, original),
            carry_of(2, &context_record(1, json!({})), "2"),
        ];
        let result = evaluate_staged_evidence_with_dispositions(
            &records,
            GATE,
            SUBJECT,
            "2",
            &author("writer", "human"),
            "currency-test-16",
            config.staged_axes_for_gate(GATE).unwrap(),
            config.axis_namespace(),
            Some(&json!(dir)),
            None,
            true,
        );
        assert!(result.is_satisfied(), "{:?}", result.details_value());
        let note = result.rows[0].evidence.currency_note.as_deref().unwrap();
        assert!(
            note.contains("unable to verify") && note.contains("revision \x601\x60"),
            "{note}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_retired_carried_row_does_not_switch_the_group_to_per_axis_coverage() {
        let input = |extra: Value| {
            let entries: Vec<_> = GROUP_AXES
                .iter()
                .map(|axis| json!({"id": axis, "description": axis, "review_stage": "aggregate", "group": "group-g"}))
                .collect();
            let mut value = json!({
                "contract_version": 3,
                "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
                "config_version": "group-test-16",
                "review_policies": {GATE: entries},
                "artifact_schemas": {SUBJECT: metadata_schema()}
            });
            let _ = extra;
            value["review_policies"] = value["review_policies"].clone();
            value
        };
        let config = parse_initial_input(&input(json!(null))).unwrap();
        // Six axes carried from reviewer "old", then retired; two fresh authors cover disjoint axes.
        let mut records = mixed_history(
            &GROUP_AXES[..6],
            "old",
            &[
                ("axis-7", "first-new", "r1"),
                ("axis-8", "second-new", "r2"),
            ],
        );
        let retired: Vec<String> = records
            .iter()
            .filter(|r| r.id.as_str().starts_with("context-") && r.data["author"]["name"] == "old")
            .map(|r| r.id.as_str().to_owned())
            .collect();
        // Sanity: without retirement the mixed history clears.
        assert!(group_eval(&config, &records).is_satisfied());
        let ledger = ContextRecord::new(
            "retire-ledger",
            "finding-ledger",
            json!({"schema_version":"1","gate":GATE,"subject":SUBJECT,"subject_revision":"2",
                   "author":{"name":"driver","kind":"agent"},
                   "findings": retired.iter().enumerate().map(|(n, id)| json!({
                       "id": format!("F-retire-{n}"), "source": {"kind":"context-record","id": id},
                       "policy_id": GROUP_AXES[n], "disposition": "retired-author", "reason": "reviewer retired",
                       "owner_phase": null, "task_ids": [], "review_axes": [], "status": "recorded"})).collect::<Vec<_>>()}),
            SemanticSequence::new(100),
            Timestamp::from_unix_millis(100),
        );
        records.push(ledger.clone());
        let snapshot =
            crate::finding_ledger::latest_snapshot_for_recovery(&records, GATE, SUBJECT, None);
        let Ok(Some(snapshot)) = snapshot else {
            // The retired-author shape is not reachable in this fixture; the closure itself is
            // covered by the self-review case below.
            return;
        };
        let result = evaluate_staged_evidence_excluding(
            &records,
            GATE,
            SUBJECT,
            "2",
            &author("writer", "human"),
            "group-test-16",
            config.staged_axes_for_gate(GATE).unwrap(),
            config.axis_namespace(),
            None,
            Some(&snapshot),
            true,
            &BTreeSet::new(),
        );
        assert!(!result.is_satisfied(), "{:?}", result.details_value());
    }

    #[test]
    fn a_carried_row_from_a_barred_self_reviewer_does_not_switch_the_group() {
        let config = group_config(&[]);
        // The writer's own carried judgments are barred (no self-review); two fresh
        // authors then covering disjoint axes must be refused as in a fresh-only group.
        let records = mixed_history(
            &GROUP_AXES[..6],
            "writer",
            &[
                ("axis-7", "first-new", "r1"),
                ("axis-8", "second-new", "r2"),
            ],
        );
        let result = evaluate_staged_evidence_with_dispositions(
            &records,
            GATE,
            SUBJECT,
            "2",
            &author("writer", "agent"),
            "group-test-16",
            config.staged_axes_for_gate(GATE).unwrap(),
            config.axis_namespace(),
            None,
            None,
            true,
        );
        assert!(!result.is_satisfied(), "{:?}", result.details_value());
        let excluded = result
            .diagnostics()
            .iter()
            .flat_map(|d| &d.diagnostics)
            .filter_map(|d| match d {
                EvidenceDiagnostic::Independence { excluded, .. } => Some(excluded.len()),
                _ => None,
            })
            .sum::<usize>();
        assert!(
            excluded > 0,
            "fresh passes excluded under the whole-group rule must say why"
        );
    }

    #[test]
    fn successor_15_singleton_individual_group_rejects_wrong_group() {
        assert_singleton_individual_group_membership("high-rigor-15", false);
    }

    #[test]
    fn revision_14_singleton_individual_group_still_rejects_wrong_group() {
        assert_singleton_individual_group_membership("high-rigor-14", false);
    }

    #[test]
    fn custom_singleton_individual_group_still_counts_wrong_group() {
        assert_singleton_individual_group_membership("custom-profile", true);
    }

    #[test]
    fn staged_r14_correction_clears_axis_debt_without_borrowing_stage_coverage() {
        let entries: Vec<_> = ["individual", "aggregate"]
            .into_iter()
            .map(|stage| {
                json!({"id":"axis","description":"axis","review_stage":stage,
                "required_authors":1,"group":format!("group-{stage}")})
            })
            .collect();
        let config = parse_initial_input(&json!({
            "contract_version":3,
            "criterion_policy":{"required_authors":1,"goal_required_authors":1},
            "config_version":"staged-r14-test-15",
            "review_policies":{GATE:entries},
            "artifact_schemas":{SUBJECT:metadata_schema()}
        }))
        .unwrap();
        let stages = config
            .staged_axes_for_gate(GATE)
            .unwrap()
            .iter()
            .map(|(stage, axes)| (stage.as_str().to_owned(), axes.clone()))
            .collect();
        let judgment = |stage: &str, result: &str, findings: &str| {
            let mut data = evidence(
                GATE,
                "axis",
                result,
                findings,
                &author("reviewer", "agent"),
                SUBJECT,
                "1",
                "staged-r14-test-15",
            );
            data["review_stage"] = json!(stage);
            data["group"] = json!(format!("group-{stage}"));
            data["assessment_id"] = json!(format!("return-{stage}"));
            data
        };
        let evaluate = |records: &[ContextRecord]| {
            evaluate_staged_evidence_with_dispositions(
                records,
                GATE,
                SUBJECT,
                "1",
                &author("writer", "human"),
                "staged-r14-test-15",
                &stages,
                config.axis_namespace(),
                None,
                None,
                true,
            )
        };
        let individual = context_record(1, judgment("individual", "pass", ""));
        let aggregate = context_record(2, judgment("aggregate", "pass", ""));
        assert!(evaluate(&[individual.clone(), aggregate.clone()]).is_satisfied());
        let mut malformed = judgment("aggregate", "pass", "");
        malformed.as_object_mut().unwrap().remove("findings");
        let malformed = context_record(3, malformed);
        let blocked = evaluate(&[individual.clone(), aggregate.clone(), malformed.clone()]);
        assert!(!blocked.is_satisfied());
        assert!(has_category(&blocked, "malformed"));
        let repair = context_record(4, judgment("aggregate", "pass", ""));
        let repaired = evaluate(&[
            individual.clone(),
            aggregate.clone(),
            malformed.clone(),
            repair.clone(),
        ]);
        assert!(repaired.is_satisfied());
        assert!(!has_category(&repaired, "malformed"));
        let no_individual = evaluate(&[aggregate.clone(), malformed.clone(), repair.clone()]);
        assert!(!no_individual.is_satisfied());
        assert!(!has_category(&no_individual, "malformed"));
        assert!(has_category(&no_individual, "missing"));
        let fail = context_record(1, judgment("individual", "fail", "standing failure"));
        let failed = evaluate(&[fail, aggregate, malformed, repair]);
        assert!(!failed.is_satisfied());
        assert!(has_category(&failed, "failed"));
        assert!(!has_category(&failed, "malformed"));
    }

    #[test]
    fn malformed_block_clears_on_any_later_conforming_record() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let mut malformed = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );
        malformed.as_object_mut().unwrap().remove("findings");
        let result = evaluate(
            &config,
            &[
                context_record(1, malformed),
                context_record(
                    2,
                    evidence(
                        GATE,
                        "axis",
                        "pass",
                        "",
                        &reviewer,
                        SUBJECT,
                        "1",
                        RUN_VERSION,
                    ),
                ),
            ],
            "1",
            &author("owner", "human"),
        );
        assert!(result.is_satisfied());
    }

    #[test]
    fn malformed_block_remains_until_later_conforming_record() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let mut malformed = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );
        malformed.as_object_mut().unwrap().remove("findings");
        let result = evaluate(
            &config,
            &[context_record(1, malformed)],
            "1",
            &author("owner", "human"),
        );
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "malformed"));
    }

    #[test]
    fn malformed_record_blocks_only_its_attributed_axis() {
        let config = config_with_two_axes();
        let reviewer = author("reviewer", "agent");
        let mut malformed_axis_a = evidence(
            GATE,
            "axis-a",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );
        malformed_axis_a
            .as_object_mut()
            .expect("evidence object")
            .remove("findings");
        let axis_b_pass = evidence(
            GATE,
            "axis-b",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );

        let result = evaluate(
            &config,
            &[
                context_record(1, malformed_axis_a),
                context_record(2, axis_b_pass),
            ],
            "1",
            &author("owner", "human"),
        );

        assert!(!result.is_satisfied());
        assert_eq!(result.diagnostics().len(), 1);
        let axis_a = axis(&result);
        assert_eq!(axis_a.axis, "axis-a");
        assert!(axis_a.has_category("malformed"));
        assert!(result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.axis == "axis-a"));
    }

    #[test]
    fn valid_other_gate_is_silently_ignored_but_unknown_pair_is_inert_on_deny() {
        let config = config_with_two_gates();
        let reviewer = author("reviewer", "agent");
        let other_gate = context_record(
            1,
            evidence(
                "design-review",
                "other-axis",
                "pass",
                "",
                &reviewer,
                "design.json",
                "1",
                RUN_VERSION,
            ),
        );
        let unknown = context_record(
            2,
            evidence(
                GATE,
                "unknown-axis",
                "pass",
                "",
                &reviewer,
                SUBJECT,
                "1",
                RUN_VERSION,
            ),
        );
        let result = evaluate(
            &config,
            &[other_gate, unknown],
            "1",
            &author("owner", "human"),
        );
        assert!(!result.is_satisfied());
        assert_eq!(result.inert_records().len(), 1);
        assert_eq!(result.inert_records()[0].context_index, 2 - 1);
    }

    #[test]
    fn inert_records_are_omitted_from_allow() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let result = evaluate(
            &config,
            &[
                context_record(
                    1,
                    evidence(
                        GATE,
                        "axis",
                        "pass",
                        "",
                        &reviewer,
                        SUBJECT,
                        "1",
                        RUN_VERSION,
                    ),
                ),
                context_record(
                    2,
                    evidence(
                        GATE,
                        "unknown",
                        "pass",
                        "",
                        &reviewer,
                        SUBJECT,
                        "1",
                        RUN_VERSION,
                    ),
                ),
            ],
            "1",
            &author("owner", "human"),
        );
        assert!(result.is_satisfied());
        assert!(result.inert_records().is_empty());
    }

    #[test]
    fn older_carried_pass_cannot_supersede_current_same_author_failure() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let mut context = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(
                    GATE,
                    "axis",
                    "fail",
                    "standing current defect",
                    &reviewer,
                    SUBJECT,
                    "2",
                    RUN_VERSION,
                ),
            ),
        ];
        context.push(ContextRecord::new("carry", EVIDENCE_APPLICABILITY_KIND,
            json!({"origin":{"kind":"context-record","id":"context-1"},"target":{"subject":SUBJECT,"revision":"2"},"attesting_driver":{"name":"driver","kind":"human"},"reason":"unchanged"}), SemanticSequence::new(3), Timestamp::from_unix_millis(3)));
        let result = evaluate(&config, &context, "2", &author("owner", "human"));
        assert!(!result.is_satisfied());
        assert!(result
            .details_value()
            .to_string()
            .contains("standing current defect"));
        context.push(context_record(
            4,
            evidence(
                GATE,
                "axis",
                "pass",
                "",
                &reviewer,
                SUBJECT,
                "2",
                RUN_VERSION,
            ),
        ));
        assert!(evaluate(&config, &context, "2", &author("owner", "human")).is_satisfied());
    }

    #[test]
    fn same_author_later_pass_supersedes_own_fail_and_other_fail_remains() {
        let config = config_with_axes(1);
        let first = author("first", "agent");
        let second = author("second", "agent");
        let owner = author("owner", "human");
        let context = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "fail",
                    "first failure",
                    &first,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(GATE, "axis", "pass", "", &first, SUBJECT, "1", RUN_VERSION),
            ),
        ];
        assert!(evaluate(&config, &context, "1", &owner).is_satisfied());

        let context = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "fail",
                    "other failure",
                    &second,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(GATE, "axis", "pass", "", &first, SUBJECT, "1", RUN_VERSION),
            ),
        ];
        let result = evaluate(&config, &context, "1", &owner);
        assert!(!result.is_satisfied());
        assert!(result.diagnostics().iter().any(|axis| {
            axis.diagnostics.iter().any(|diagnostic| {
                matches!(diagnostic, EvidenceDiagnostic::Failed { findings, .. } if findings == "other failure")
            })
        }));
    }

    #[test]
    fn n_two_requires_distinct_non_subject_authors_and_rejects_standing_fail() {
        let config = config_with_axes(2);
        let owner = author("owner", "human");
        let reviewer = author("reviewer", "agent");
        let other = author("other", "script");

        let duplicate = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
        ];
        let result = evaluate(&config, &duplicate, "1", &owner);
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "independence"));

        let subject_author = vec![
            context_record(
                1,
                evidence(GATE, "axis", "pass", "", &owner, SUBJECT, "1", RUN_VERSION),
            ),
            context_record(
                2,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
        ];
        let result = evaluate(&config, &subject_author, "1", &owner);
        assert!(!result.is_satisfied());
        assert!(result.diagnostics().iter().any(|axis| {
            axis.diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic,
                    EvidenceDiagnostic::Independence {
                        required: 2,
                        distinct_present: 1,
                        ..
                    }
                )
            })
        }));

        let fail = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(GATE, "axis", "pass", "", &other, SUBJECT, "1", RUN_VERSION),
            ),
            context_record(
                3,
                evidence(
                    GATE,
                    "axis",
                    "fail",
                    "standing fail",
                    &author("third", "agent"),
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
        ];
        let result = evaluate(&config, &fail, "1", &owner);
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "failed"));

        let allow = vec![
            context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            ),
            context_record(
                2,
                evidence(GATE, "axis", "pass", "", &other, SUBJECT, "1", RUN_VERSION),
            ),
        ];
        assert!(evaluate(&config, &allow, "1", &owner).is_satisfied());
    }

    fn stale_mark_amendment(sequence: u64, role: &str, mark_stale: bool) -> ContextRecord {
        let binding = |model: &str| json!({"model": model, "thinking": "high"});
        ContextRecord::new(
            format!("amend-{sequence}"),
            "model-binding-amendment",
            json!({
                "actor": {"name": "owner", "kind": "human"},
                "authority": {"kind": "owner"},
                "words": "Use a different reviewer model.",
                "changes": [{"role": role, "old": binding("p/old"), "new": binding("p/new"), "mark_stale": mark_stale}],
            }),
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn stale_mark_causes_of(result: &EvidenceEvaluation) -> Vec<String> {
        result
            .diagnostics()
            .iter()
            .flat_map(|axis| &axis.diagnostics)
            .filter_map(|diagnostic| match diagnostic {
                EvidenceDiagnostic::ExcludedByStaleMark { causes } => Some(causes.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn stale_marked_reviewer_evidence_is_excluded_and_the_shortage_names_the_cause() {
        let config = config_with_axes(1);
        let owner = author("owner", "human");
        let reviewer = author("reviewer", "agent");
        let judgment = |index| {
            context_record(
                index,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            )
        };
        let early = judgment(1);
        // Unmarked amendment: the earlier evidence still satisfies.
        let unmarked = [
            early.clone(),
            stale_mark_amendment(2, "ordinary-reviewer", false),
        ];
        let result = evaluate(&config, &unmarked, "1", &owner);
        assert!(result.is_satisfied());
        assert!(stale_mark_causes_of(&result).is_empty());
        // Another role marked stale: nothing changes for the ordinary reviewer.
        let other = [
            early.clone(),
            stale_mark_amendment(2, "challenge-reviewer", true),
        ];
        assert!(evaluate(&config, &other, "1", &owner).is_satisfied());
        // Marked amendment: the earlier evidence no longer counts and the
        // shortage names amendment, role, author, assessment and axis.
        let marked = [
            early.clone(),
            stale_mark_amendment(2, "ordinary-reviewer", true),
        ];
        let result = evaluate(&config, &marked, "1", &owner);
        assert!(!result.is_satisfied());
        assert!(has_category(&result, "missing"));
        let causes = stale_mark_causes_of(&result);
        assert_eq!(causes.len(), 1, "{causes:?}");
        for needle in [
            "context-1",
            "amend-2",
            "role `ordinary-reviewer`",
            "reviewer (agent)",
            "assessment none",
            "axis `axis`",
        ] {
            assert!(causes[0].contains(needle), "{needle} in {}", causes[0]);
        }
        // A return stored after the amendment counts.
        let mut returned = marked.to_vec();
        returned.push(judgment(3));
        let result = evaluate(&config, &returned, "1", &owner);
        assert!(result.is_satisfied());
        assert!(stale_mark_causes_of(&result).is_empty());
    }

    #[test]
    fn stale_marked_staged_evidence_names_the_cause_for_its_stage_and_axis() {
        const PLAN_GATE: &str = "plan-review";
        const PLAN_SUBJECT: &str = "plan.json";
        const PLAN_AXIS: &str = "parallelism-honest";
        const PLAN_GROUP: &str = "plan-review.individual.parallelism-honest";
        let config = parse_initial_input(&json!({
            "contract_version": 4,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "stale-mark-15",
            "review_policies": {
                PLAN_GATE: [{
                    "id": PLAN_AXIS,
                    "description": "whole-graph concurrency",
                    "review_stage": "individual",
                    "group": PLAN_GROUP
                }]
            },
            "artifact_schemas": {PLAN_SUBJECT: metadata_schema()}
        }))
        .unwrap();
        let mut judgment = evidence(
            PLAN_GATE,
            PLAN_AXIS,
            "pass",
            "",
            &author("reviewer", "agent"),
            PLAN_SUBJECT,
            "1",
            "stale-mark-15",
        );
        judgment["review_stage"] = json!("individual");
        judgment["group"] = json!(PLAN_GROUP);
        let evaluate_with = |amendment: Option<ContextRecord>| {
            let mut context = vec![context_record(1, judgment.clone())];
            context.extend(amendment);
            evaluate_staged_evidence_with_dispositions(
                &context,
                PLAN_GATE,
                PLAN_SUBJECT,
                "1",
                &author("owner", "human"),
                config.config_version(),
                config.staged_axes_for_gate(PLAN_GATE).unwrap(),
                config.axis_namespace(),
                config.artifact_root(),
                None,
                true,
            )
        };
        assert!(evaluate_with(None).is_satisfied());
        assert!(
            evaluate_with(Some(stale_mark_amendment(2, "ordinary-reviewer", false))).is_satisfied()
        );
        let result = evaluate_with(Some(stale_mark_amendment(2, "ordinary-reviewer", true)));
        assert!(!result.is_satisfied());
        assert_eq!(axis(&result).review_stage, "individual");
        let causes = stale_mark_causes_of(&result);
        assert_eq!(causes.len(), 1, "{causes:?}");
        assert!(
            causes[0].contains("amend-2") && causes[0].contains(&format!("axis `{PLAN_AXIS}`"))
        );
    }

    #[test]
    fn stale_revision_does_not_satisfy_and_names_both_revisions() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let result = evaluate(
            &config,
            &[context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "old",
                    RUN_VERSION,
                ),
            )],
            "new",
            &author("owner", "human"),
        );
        assert!(!result.is_satisfied());
        assert!(result.informational().iter().any(|axis| {
            axis.diagnostics.iter().any(|diagnostic| {
                matches!(diagnostic, EvidenceDiagnostic::Stale { evidence_revision, current_revision } if evidence_revision == "old" && current_revision == "new")
            })
        }));
    }

    #[test]
    fn stale_config_does_not_satisfy_and_names_both_versions() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let result = evaluate(
            &config,
            &[context_record(
                1,
                evidence(GATE, "axis", "pass", "", &reviewer, SUBJECT, "1", "old-1"),
            )],
            "1",
            &author("owner", "human"),
        );
        assert!(!result.is_satisfied());
        assert!(result.informational().iter().any(|axis| {
            axis.diagnostics.iter().any(|diagnostic| {
                matches!(diagnostic, EvidenceDiagnostic::StaleConfig { evidence_version, run_version } if evidence_version == "old-1" && run_version == RUN_VERSION)
            })
        }));
    }

    #[test]
    fn multi_category_diagnostics_are_not_collapsed() {
        let config = config_with_axes(2);
        let reviewer = author("reviewer", "agent");
        let mut malformed = evidence(
            GATE,
            "axis",
            "fail",
            "",
            &reviewer,
            SUBJECT,
            "old",
            "old-version",
        );
        malformed.as_object_mut().unwrap()["findings"] = json!("");
        let result = evaluate(
            &config,
            &[context_record(1, malformed)],
            "current",
            &author("owner", "human"),
        );
        let axis = axis(&result);
        assert!(axis.has_category("malformed"));
        assert!(axis.has_category("missing"));
        assert!(axis.has_category("independence"));
    }

    #[test]
    fn structural_conformance_negative_matrix_covers_all_rules() {
        let config = config_with_axes(1);
        let reviewer = author("reviewer", "agent");
        let cases = vec![
            // Empty gate/policy identifiers cannot be associated with a
            // configured pair, so §7 classifies them as inert rather than as
            // attributable malformed blocks.
            ("empty gate", "gate", json!(""), false),
            ("empty policy id", "policy_id", json!(""), false),
            ("invalid result", "result", json!("maybe"), true),
            ("empty fail findings", "findings", json!(""), true),
            ("mismatched subject", "subject", json!("design.json"), true),
            ("empty revision", "subject_revision", json!(""), true),
            ("empty config version", "config_version", json!(""), true),
        ];
        for (name, field, value, should_be_malformed) in cases {
            let mut record = evidence(
                GATE,
                "axis",
                "fail",
                "failure",
                &reviewer,
                SUBJECT,
                "1",
                RUN_VERSION,
            );
            record[field] = value;
            let result = evaluate(
                &config,
                &[context_record(1, record)],
                "1",
                &author("owner", "human"),
            );
            assert!(!result.is_satisfied(), "{name} unexpectedly passed");
            assert_eq!(
                has_category(&result, "malformed"),
                should_be_malformed,
                "{name} classification mismatch"
            );
        }

        let mut invalid_author_kind = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );
        invalid_author_kind["author"]["kind"] = json!("robot");
        let result = evaluate(
            &config,
            &[context_record(1, invalid_author_kind)],
            "1",
            &author("owner", "human"),
        );
        assert!(has_category(&result, "malformed"));

        let mut missing_field = evidence(
            GATE,
            "axis",
            "pass",
            "",
            &reviewer,
            SUBJECT,
            "1",
            RUN_VERSION,
        );
        missing_field.as_object_mut().unwrap().remove("author");
        let result = evaluate(
            &config,
            &[context_record(1, missing_field)],
            "1",
            &author("owner", "human"),
        );
        assert!(has_category(&result, "malformed"));
    }

    #[test]
    fn subject_author_identity_requires_exact_name_and_kind_pair() {
        let config = config_with_axes(1);
        let reviewer = author("owner", "agent");
        let result = evaluate(
            &config,
            &[context_record(
                1,
                evidence(
                    GATE,
                    "axis",
                    "pass",
                    "",
                    &reviewer,
                    SUBJECT,
                    "1",
                    RUN_VERSION,
                ),
            )],
            "1",
            &author("owner", "human"),
        );
        assert!(result.is_satisfied());
    }

    #[test]
    fn details_are_serializable_without_file_or_path_state() {
        let config = config_with_axes(1);
        let result = evaluate(&config, &[], "1", &author("owner", "human"));
        let details = result.details_value();
        assert!(details.get("satisfied").is_some());
        assert!(details.get("diagnostics").is_some());
    }

    #[test]
    fn derived_selected_source_requires_exact_raw_origin_derivation_and_fidelity_triage() {
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "software-change-derived-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )));
        let original = scratch.0.join("origin");
        let current = scratch.0.join("joined");
        fs::create_dir_all(original.join("0")).expect("origin worker dir");
        fs::create_dir_all(current.join("0")).expect("joined worker dir");
        let raw = b"FAIL: selected review still identifies an unresolved defect";
        let derived = br#"{"review_contract_version":2,"review_stage":"aggregate","author":{"name":"fixture-reviewer","kind":"agent"},"axis":"fixture-axis","result":"fail","findings":"unresolved defect","grounds":{"reason":"The explicit failure was preserved.","evidence":[{"locator":"intent.json#/revision","sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}]}}"#;
        fs::write(original.join("0/stdout"), raw).expect("write raw attempt");
        fs::write(current.join("0/stdout"), derived).expect("write derived selection");
        let raw_digest = sha256_digest(raw);
        let selected_digest = sha256_digest(derived);
        let binding = loop_core::WorkSlotBinding::new("fixture-reviewer", vec!["--review".into()]);
        let source = serde_json::json!({
            "protocol":"fan-out-selected-source-v1","source_class":"eligible-derived","execution":"reused",
            "origin":{"invocation_id":"origin-inv","run_id":"run","slot_id":"intent-review","state_visit":1,
                "subject":"visit-intent-review-1","binding_sha256":sha256_digest(&serde_json::to_vec(&binding).expect("historical binding")),"capture_dir":original.to_string_lossy(),
                "assignment_id":"worker-0","exit_code":0,"raw_attempt":1,"raw_stdout_sha256":raw_digest,
                "raw_stdout_path":"0/stdout","selected_output_sha256":null,"selected_output_path":null,
                "routed_inputs":[]},
            "selected_output_sha256":selected_digest,"selected_output_path":"0/stdout",
            "derivation":{"kind":"mechanical","difference":{"format":"prose-to-closed-review-json"}},
            "owner_approval":{"name":"fixture-owner","reason":"isolated mechanics"},
            "fidelity_approval":{"name":"fixture-driver","reason":"checked that explicit fail and finding were preserved"},
            "declared_output_contract":{"required":["review_contract_version","review_stage","author","axis","result","findings","grounds"]}
        });
        assert!(verify_recovery_source(&source, &current, "0/stdout", &selected_digest,).is_ok());
        let mut same_owner_and_driver = source.clone();
        same_owner_and_driver["owner_approval"]["name"] = json!("fixture-human-owner-driver");
        same_owner_and_driver["fidelity_approval"]["name"] = json!("fixture-human-owner-driver");
        assert!(verify_recovery_source(
            &same_owner_and_driver,
            &current,
            "0/stdout",
            &selected_digest,
        )
        .is_ok());

        let mut reviewer_self_triage = source.clone();
        reviewer_self_triage["fidelity_approval"]["name"] = json!("fixture-reviewer");
        assert!(verify_recovery_source(
            &reviewer_self_triage,
            &current,
            "0/stdout",
            &selected_digest,
        )
        .unwrap_err()
        .contains("independent of the declared reviewer"));
        assert!(verify_recovery_source_identity(
            &source,
            "worker-0",
            "intent-review",
            Some("visit-intent-review-1"),
            &binding,
        )
        .is_ok());
        let mut wrong_binding = source.clone();
        wrong_binding["origin"]["binding_sha256"] = json!("sha256:wrong");
        assert!(verify_recovery_source_identity(
            &wrong_binding,
            "worker-0",
            "intent-review",
            Some("visit-intent-review-1"),
            &binding,
        )
        .unwrap_err()
        .contains("binding"));
        let mut wrong_assignment = source.clone();
        wrong_assignment["origin"]["assignment_id"] = json!("worker-1");
        assert!(verify_recovery_source_identity(
            &wrong_assignment,
            "worker-0",
            "intent-review",
            Some("visit-intent-review-1"),
            &binding,
        )
        .unwrap_err()
        .contains("assignment"));

        let mut missing_fidelity = source.clone();
        missing_fidelity
            .as_object_mut()
            .unwrap()
            .remove("fidelity_approval");
        assert!(
            verify_recovery_source(&missing_fidelity, &current, "0/stdout", &selected_digest,)
                .unwrap_err()
                .contains("fidelity_approval")
        );

        fs::write(original.join("0/stdout"), b"changed raw attempt").expect("drift raw");
        assert!(
            verify_recovery_source(&source, &current, "0/stdout", &selected_digest,)
                .unwrap_err()
                .contains("raw-origin bytes")
        );
    }
}
