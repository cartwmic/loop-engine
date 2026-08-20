//! Pure review-evidence aggregation.
//!
//! This module owns no filesystem access and no transition routing.  It
//! consumes already-validated configuration projections and engine-supplied
//! context records, then applies technical-design §7's six stages in order.

#![allow(dead_code)]

use crate::config::PolicyAxis;
use loop_core::ContextRecord;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::workflow::ACCEPTED_FINDINGS_KIND;

const REVIEW_EVIDENCE_KIND: &str = "review-evidence";
const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];

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
    Independence {
        required: u64,
        distinct_present: usize,
    },
}

impl EvidenceDiagnostic {
    pub(crate) fn category(&self) -> &'static str {
        match self {
            Self::Missing { .. } => "missing",
            Self::Failed { .. } => "failed",
            Self::Malformed { .. } => "malformed",
            Self::Stale { .. } => "stale",
            Self::StaleConfig { .. } => "stale_config",
            Self::Independence { .. } => "independence",
        }
    }
}

/// Diagnostics for one configured axis.  Axis ordering is deterministic
/// because callers supply the semantically keyed BTreeMap from config.rs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct AxisDiagnostic {
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

/// Presence/shape outcome for one gate's accepted-findings record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AcceptedFindingsStatus {
    Present,
    Missing,
    Malformed {
        reasons: Vec<String>,
    },
    Stale {
        record_revision: String,
        current_revision: String,
    },
}

/// Result of the accepted-findings presence check. Contents, quiet/progress/
/// thrash, and provenance are not judged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedFindingsEvaluation {
    pub(crate) status: AcceptedFindingsStatus,
}

impl AcceptedFindingsEvaluation {
    pub(crate) fn is_satisfied(&self) -> bool {
        matches!(self.status, AcceptedFindingsStatus::Present)
    }

    pub(crate) fn details_value(&self) -> Value {
        match &self.status {
            AcceptedFindingsStatus::Present => serde_json::json!({"status": "present"}),
            AcceptedFindingsStatus::Missing => serde_json::json!({"status": "missing"}),
            AcceptedFindingsStatus::Malformed { reasons } => {
                serde_json::json!({"status": "malformed", "reasons": reasons})
            }
            AcceptedFindingsStatus::Stale {
                record_revision,
                current_revision,
            } => serde_json::json!({
                "status": "stale",
                "record_revision": record_revision,
                "current_revision": current_revision
            }),
        }
    }
}

struct WellFormedAcceptedFindings {
    subject_revision: String,
}

/// Require a well-formed current-revision accepted-findings record for `gate`.
///
/// Attribution is by `data.gate`. Malformed attributable records block until a
/// later well-formed record for the same gate supersedes them. Optional `author`
/// is ignored. Statements are not judged.
pub(crate) fn evaluate_accepted_findings(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
) -> AcceptedFindingsEvaluation {
    let mut malformed_reasons: Option<Vec<String>> = None;
    let mut latest_well_formed: Option<WellFormedAcceptedFindings> = None;

    for record in context {
        if record.kind != ACCEPTED_FINDINGS_KIND {
            continue;
        }
        let Some(data) = record.data.as_object() else {
            continue;
        };
        let Some(record_gate) = data.get("gate").and_then(Value::as_str) else {
            continue;
        };
        if record_gate != gate {
            continue;
        }
        match parse_well_formed_accepted_findings(data, subject) {
            Ok(parsed) => {
                malformed_reasons = None;
                latest_well_formed = Some(parsed);
            }
            Err(reasons) => {
                malformed_reasons = Some(reasons);
                latest_well_formed = None;
            }
        }
    }

    if let Some(reasons) = malformed_reasons {
        return AcceptedFindingsEvaluation {
            status: AcceptedFindingsStatus::Malformed { reasons },
        };
    }
    match latest_well_formed {
        None => AcceptedFindingsEvaluation {
            status: AcceptedFindingsStatus::Missing,
        },
        Some(parsed) if parsed.subject_revision != current_revision => AcceptedFindingsEvaluation {
            status: AcceptedFindingsStatus::Stale {
                record_revision: parsed.subject_revision,
                current_revision: current_revision.to_owned(),
            },
        },
        Some(_) => AcceptedFindingsEvaluation {
            status: AcceptedFindingsStatus::Present,
        },
    }
}

fn parse_well_formed_accepted_findings(
    data: &Map<String, Value>,
    expected_subject: &str,
) -> Result<WellFormedAcceptedFindings, Vec<String>> {
    let mut reasons = Vec::new();
    let _gate = non_empty_string(data, "gate", &mut reasons);
    let subject = non_empty_string(data, "subject", &mut reasons);
    let subject_revision = non_empty_string(data, "subject_revision", &mut reasons);
    match data.get("findings") {
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let Some(object) = item.as_object() else {
                    reasons.push(format!("`findings[{index}]` must be an object"));
                    continue;
                };
                match object.get("policy_id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => {}
                    Some(_) => {
                        reasons.push(format!("`findings[{index}].policy_id` must not be empty"))
                    }
                    None => reasons.push(format!(
                        "missing or non-string `findings[{index}].policy_id`"
                    )),
                }
                match object.get("statement").and_then(Value::as_str) {
                    Some(statement) if !statement.is_empty() => {}
                    Some(_) => {
                        reasons.push(format!("`findings[{index}].statement` must not be empty"))
                    }
                    None => reasons.push(format!(
                        "missing or non-string `findings[{index}].statement`"
                    )),
                }
            }
        }
        Some(_) => reasons.push("`findings` must be an array".to_owned()),
        None => reasons.push("missing `findings`".to_owned()),
    }
    if subject.as_deref() != Some(expected_subject) {
        reasons.push(format!(
            "`subject` must equal expected subject `{expected_subject}`"
        ));
    }
    if !reasons.is_empty() {
        return Err(reasons);
    }
    Ok(WellFormedAcceptedFindings {
        subject_revision: subject_revision.expect("revision checked by empty-reasons branch"),
    })
}

#[derive(Clone, Debug)]
struct ConformingEvidence {
    result: EvidenceResult,
    findings: String,
    author: AuthorIdentity,
    subject_revision: String,
    config_version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvidenceResult {
    Pass,
    Fail,
}

#[derive(Clone, Debug)]
enum Attribution {
    Current { axis: String },
    OtherConfigured,
    Inert(InertEvidence),
}

/// Run §7's six-stage evidence pipeline.
///
/// `context` is already in engine-supplied sequence order.  This function
/// deliberately trusts that order and never sorts by timestamps or performs
/// any filesystem access.
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
) -> EvidenceEvaluation {
    let mut malformed: BTreeMap<String, Vec<String>> = axes
        .keys()
        .cloned()
        .map(|axis| (axis, Vec::new()))
        .collect();
    let mut latest: BTreeMap<(String, String, AuthorIdentity), ConformingEvidence> =
        BTreeMap::new();
    let mut inert_records = Vec::new();

    // Stages 1–3: filter, attribute, then structurally conform.  Iteration
    // order is the supplied append/sequence order; map insertion below is
    // therefore latest-wins without relying on wall-clock metadata.
    for (context_index, record) in context.iter().enumerate() {
        if record.kind != REVIEW_EVIDENCE_KIND || !record.data.is_object() {
            continue;
        }
        let data = record
            .data
            .as_object()
            .expect("object check above guarantees object data");
        match classify_attribution(data, context_index, gate, axis_namespace) {
            Attribution::Current { axis } => {
                match parse_conforming(data, subject) {
                    Ok(conforming) => {
                        // Any later conforming record clears the malformed
                        // block, regardless of author/revision/config.
                        malformed
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
                    Err(reasons) => {
                        // A malformed attributable record blocks only its own
                        // axis until a later conforming record for that axis.
                        malformed
                            .get_mut(&axis)
                            .expect("attribution only returns configured axis")
                            .extend(reasons);
                    }
                }
            }
            Attribution::OtherConfigured => {
                // Valid evidence for another configured gate belongs there;
                // never diagnose it during this gate's evaluation.
            }
            Attribution::Inert(inert) => inert_records.push(inert),
        }
    }

    let mut diagnostics = Vec::new();
    let mut informational = Vec::new();
    let mut all_axes_satisfied = true;

    // Stages 4–6: latest-wins supersession happened above through the full
    // (axis, subject_revision, author) key.  Judge each axis against current
    // subject metadata and then emit all applicable categories.
    for (axis, policy) in axes {
        let mut failed_findings = Vec::new();
        let mut stale = Vec::new();
        let mut stale_config = Vec::new();
        let mut distinct_present = BTreeSet::new();
        let mut pass_authors = BTreeSet::new();
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
            if record.author == *subject_author {
                continue;
            }

            current_records_present += 1;
            distinct_present.insert(record.author.clone());
            match record.result {
                EvidenceResult::Pass => {
                    pass_authors.insert(record.author.clone());
                }
                EvidenceResult::Fail => failed_findings.push(record.findings.clone()),
            }
        }

        let malformed_reasons = malformed
            .get(axis)
            .expect("all configured axes have malformed state");
        let has_malformed = !malformed_reasons.is_empty();
        let has_fail = !failed_findings.is_empty();
        let enough_passes = pass_authors.len() as u64 >= policy.required_authors();
        let satisfied = !has_malformed && !has_fail && enough_passes;

        if !satisfied {
            all_axes_satisfied = false;
            let mut axis_diagnostics = Vec::new();
            if current_records_present == 0 {
                axis_diagnostics.push(EvidenceDiagnostic::Missing {
                    required: policy.required_authors(),
                });
            }
            axis_diagnostics.extend(
                failed_findings
                    .into_iter()
                    .map(|findings| EvidenceDiagnostic::Failed { findings }),
            );
            if has_malformed {
                axis_diagnostics.push(EvidenceDiagnostic::Malformed {
                    reasons: malformed_reasons.clone(),
                });
            }
            if (distinct_present.len() as u64) < policy.required_authors() {
                axis_diagnostics.push(EvidenceDiagnostic::Independence {
                    required: policy.required_authors(),
                    distinct_present: distinct_present.len(),
                });
            }
            diagnostics.push(AxisDiagnostic {
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
    }
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

fn parse_conforming(
    data: &Map<String, Value>,
    expected_subject: &str,
) -> Result<ConformingEvidence, Vec<String>> {
    let mut reasons = Vec::new();
    let _gate = non_empty_string(data, "gate", &mut reasons);
    let _policy_id = non_empty_string(data, "policy_id", &mut reasons);
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
    let subject = non_empty_string(data, "subject", &mut reasons);
    let subject_revision = non_empty_string(data, "subject_revision", &mut reasons);
    let config_version = non_empty_string(data, "config_version", &mut reasons);

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
        return Err(reasons);
    }

    Ok(ConformingEvidence {
        result: result.expect("result checked by empty-reasons branch"),
        findings: findings.expect("findings checked by empty-reasons branch"),
        author: author.expect("author checked by empty-reasons branch"),
        subject_revision: subject_revision.expect("revision checked by empty-reasons branch"),
        config_version: config_version.expect("config version checked by empty-reasons branch"),
    })
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
                matches!(diagnostic, EvidenceDiagnostic::Failed { findings } if findings == "other failure")
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
                        distinct_present: 1
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

    fn accepted_findings_data(gate: &str, subject: &str, revision: &str, findings: Value) -> Value {
        json!({
            "gate": gate,
            "subject": subject,
            "subject_revision": revision,
            "findings": findings
        })
    }

    fn accepted_record(index: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            format!("accepted-{index}"),
            crate::workflow::ACCEPTED_FINDINGS_KIND,
            data,
            SemanticSequence::new(index),
            Timestamp::from_unix_millis(index as i64),
        )
    }

    #[test]
    fn empty_findings_array_is_well_formed_current_revision_presence() {
        let result = evaluate_accepted_findings(
            &[accepted_record(
                1,
                accepted_findings_data(GATE, SUBJECT, "1", json!([])),
            )],
            GATE,
            SUBJECT,
            "1",
        );
        assert!(result.is_satisfied());
    }

    #[test]
    fn optional_author_is_ignored_for_presence() {
        let mut data = accepted_findings_data(GATE, SUBJECT, "1", json!([]));
        data["author"] = json!({"name": "driver", "kind": "human"});
        let result = evaluate_accepted_findings(&[accepted_record(1, data)], GATE, SUBJECT, "1");
        assert!(result.is_satisfied());
    }

    #[test]
    fn missing_record_is_not_satisfied() {
        let result = evaluate_accepted_findings(&[], GATE, SUBJECT, "1");
        assert!(!result.is_satisfied());
        assert_eq!(result.status, AcceptedFindingsStatus::Missing);
    }

    #[test]
    fn malformed_attributable_record_blocks_until_later_well_formed_supersedes() {
        let mut malformed = accepted_findings_data(GATE, SUBJECT, "1", json!("not-an-array"));
        let blocked = evaluate_accepted_findings(
            &[accepted_record(1, malformed.clone())],
            GATE,
            SUBJECT,
            "1",
        );
        assert!(!blocked.is_satisfied());
        assert!(matches!(
            blocked.status,
            AcceptedFindingsStatus::Malformed { .. }
        ));

        malformed["findings"] = json!([{"policy_id": "axis", "statement": "accepted"}]);
        let superseded = evaluate_accepted_findings(
            &[
                accepted_record(1, accepted_findings_data(GATE, SUBJECT, "1", json!("bad"))),
                accepted_record(2, malformed),
            ],
            GATE,
            SUBJECT,
            "1",
        );
        assert!(superseded.is_satisfied());
    }

    #[test]
    fn other_gate_record_does_not_satisfy_or_block() {
        let result = evaluate_accepted_findings(
            &[accepted_record(
                1,
                accepted_findings_data("intent-adversarial-review", SUBJECT, "1", json!([])),
            )],
            GATE,
            SUBJECT,
            "1",
        );
        assert_eq!(result.status, AcceptedFindingsStatus::Missing);
    }

    #[test]
    fn subject_revision_bump_requires_a_new_current_revision_record() {
        let result = evaluate_accepted_findings(
            &[accepted_record(
                1,
                accepted_findings_data(GATE, SUBJECT, "old", json!([])),
            )],
            GATE,
            SUBJECT,
            "new",
        );
        assert_eq!(
            result.status,
            AcceptedFindingsStatus::Stale {
                record_revision: "old".to_owned(),
                current_revision: "new".to_owned(),
            }
        );
    }
}
