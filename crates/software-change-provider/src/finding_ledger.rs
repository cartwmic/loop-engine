//! Driver-authored finding-ledger snapshots.
//!
//! The provider treats this data as an opaque record of driver decisions.  It
//! only checks the frozen mechanical contract: closed JSON shape, freshness,
//! identifier membership, stable IDs, raw-source linkage, and agreement with
//! the current failing review evidence.  It does not classify findings or
//! choose their disposition, owner, or route.

use crate::checkpoint;
use crate::config::ValidatedConfig;
use crate::workflow::FINDING_LEDGER_KIND;
use loop_core::ContextRecord;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[allow(dead_code)]
pub(crate) const ADVISORY_FINDING_PROPOSAL_KIND: &str = "advisory-finding-proposal";
const SCHEMA_VERSION: &str = "1";
const IMPLEMENTATION_SUBJECT: &str = "implementation-report.json";
const VALIDATION_SUBJECT: &str = "validation-report.json";
const PLAN_SUBJECT: &str = "plan.json";
const SNAPSHOT_FIELDS: &[&str] = &[
    "schema_version",
    "gate",
    "subject",
    "subject_revision",
    "author",
    "repository_state",
    "findings",
];
const AUTHOR_FIELDS: &[&str] = &["name", "kind"];
const FINDING_FIELDS: &[&str] = &[
    "id",
    "source",
    "policy_id",
    "statement",
    "disposition",
    "reason",
    "owner_phase",
    "task_ids",
    "review_axes",
    "status",
];
const WORK_SLOT_SOURCE_FIELDS: &[&str] = &[
    "kind",
    "invocation_id",
    "worker_index",
    "attempt",
    "output_sha256",
];
const EXTERNAL_SOURCE_FIELDS: &[&str] = &["kind", "relative_path", "output_sha256"];
/// A source reference retained by a driver disposition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FindingSource {
    WorkSlot {
        invocation_id: String,
        worker_index: u64,
        attempt: u8,
        output_sha256: String,
    },
    ExternalArtifact {
        relative_path: String,
        output_sha256: String,
    },
}

impl FindingSource {
    #[allow(dead_code)]
    pub(crate) fn output_sha256(&self) -> &str {
        match self {
            Self::WorkSlot { output_sha256, .. } | Self::ExternalArtifact { output_sha256, .. } => {
                output_sha256
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FindingSourceIdentity {
    WorkSlot {
        invocation_id: String,
        worker_index: u64,
        attempt: u8,
        output_sha256: String,
    },
    ExternalArtifact {
        relative_path: String,
        output_sha256: String,
    },
}

/// One driver disposition in a snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Finding {
    pub(crate) id: String,
    pub(crate) source: FindingSource,
    pub(crate) policy_id: String,
    pub(crate) statement: String,
    pub(crate) disposition: FindingDisposition,
    pub(crate) reason: String,
    pub(crate) owner_phase: Option<String>,
    pub(crate) task_ids: Vec<String>,
    pub(crate) review_axes: Vec<String>,
    pub(crate) status: FindingStatus,
}

impl Finding {
    pub(crate) fn is_accepted_unresolved(&self) -> bool {
        self.disposition == FindingDisposition::Accepted && self.status == FindingStatus::Unresolved
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindingDisposition {
    Accepted,
    Rejected,
    Advisory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindingStatus {
    Unresolved,
    Resolved,
    Stale,
    Recorded,
}

/// A well-formed snapshot, independent of whether it is fresh for the
/// currently read artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FindingLedgerSnapshot {
    pub(crate) schema_version: String,
    pub(crate) gate: String,
    pub(crate) subject: String,
    pub(crate) subject_revision: String,
    pub(crate) author: LedgerAuthor,
    pub(crate) repository_state: Option<String>,
    pub(crate) findings: Vec<Finding>,
}

impl FindingLedgerSnapshot {
    pub(crate) fn accepted_unresolved(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.is_accepted_unresolved())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerAuthor {
    pub(crate) name: String,
    pub(crate) kind: String,
}

/// Mechanical outcome of the latest ledger history for one gate/subject pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FindingLedgerStatus {
    Present,
    Missing,
    Malformed {
        reasons: Vec<String>,
    },
    StaleSubject {
        record_revision: String,
        current_revision: String,
    },
    StaleRepositoryState {
        record_state: String,
        current_state: Option<String>,
    },
    SetMismatch {
        accepted_unresolved: BTreeSet<(String, String)>,
        failing_evidence: BTreeSet<(String, String)>,
    },
}

/// Evaluation projection used by gates and by later routing code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FindingLedgerEvaluation {
    pub(crate) status: FindingLedgerStatus,
    pub(crate) snapshot: Option<FindingLedgerSnapshot>,
}

impl FindingLedgerEvaluation {
    pub(crate) fn is_satisfied(&self) -> bool {
        matches!(self.status, FindingLedgerStatus::Present)
    }

    #[allow(dead_code)]
    pub(crate) fn current_snapshot(&self) -> Option<&FindingLedgerSnapshot> {
        match &self.status {
            FindingLedgerStatus::Present | FindingLedgerStatus::SetMismatch { .. } => {
                self.snapshot.as_ref()
            }
            FindingLedgerStatus::Missing
            | FindingLedgerStatus::Malformed { .. }
            | FindingLedgerStatus::StaleSubject { .. }
            | FindingLedgerStatus::StaleRepositoryState { .. } => None,
        }
    }

    pub(crate) fn details_value(&self) -> Value {
        match &self.status {
            FindingLedgerStatus::Present => serde_json::json!({
                "status": "present"
            }),
            FindingLedgerStatus::Missing => serde_json::json!({
                "status": "missing"
            }),
            FindingLedgerStatus::Malformed { reasons } => serde_json::json!({
                "status": "malformed",
                "reasons": reasons
            }),
            FindingLedgerStatus::StaleSubject {
                record_revision,
                current_revision,
            } => serde_json::json!({
                "status": "stale_subject",
                "record_revision": record_revision,
                "current_revision": current_revision
            }),
            FindingLedgerStatus::StaleRepositoryState {
                record_state,
                current_state,
            } => serde_json::json!({
                "status": "stale_repository_state",
                "record_state": record_state,
                "current_state": current_state
            }),
            FindingLedgerStatus::SetMismatch {
                accepted_unresolved,
                failing_evidence,
            } => serde_json::json!({
                "status": "set_mismatch",
                "accepted_unresolved": set_to_values(accepted_unresolved),
                "failing_evidence": set_to_values(failing_evidence)
            }),
        }
    }
}

/// Validate the append-only ledger history for one exact gate and subject.
///
/// Context records are immutable and ordered by their semantic sequence.  A
/// malformed record is not a current snapshot; the last well-formed record is
/// the candidate current view.  Stable IDs are nevertheless checked across
/// every well-formed snapshot in that history.
pub(crate) fn evaluate_finding_ledger(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    current_revision: &str,
    config: &ValidatedConfig,
    failing_evidence: &BTreeSet<(String, String)>,
) -> FindingLedgerEvaluation {
    let mut records: Vec<&ContextRecord> = context
        .iter()
        .filter(|record| record.kind == FINDING_LEDGER_KIND)
        .filter(|record| ledger_record_matches_pair(record, gate, subject))
        .collect();
    records.sort_by_key(|record| record.sequence);

    let mut snapshots = Vec::new();
    let mut malformed_reasons = Vec::new();
    for record in records {
        match parse_snapshot(
            &record.data,
            gate,
            subject,
            config.artifact_root(),
            Some(config),
        ) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(reasons) => malformed_reasons.extend(
                reasons
                    .into_iter()
                    .map(|reason| format!("context {}: {reason}", record.id)),
            ),
        }
    }

    if snapshots.is_empty() {
        return FindingLedgerEvaluation {
            status: if malformed_reasons.is_empty() {
                FindingLedgerStatus::Missing
            } else {
                FindingLedgerStatus::Malformed {
                    reasons: malformed_reasons,
                }
            },
            snapshot: None,
        };
    }

    if let Err(reasons) = check_id_continuity(&snapshots) {
        return FindingLedgerEvaluation {
            status: FindingLedgerStatus::Malformed { reasons },
            snapshot: snapshots.last().cloned(),
        };
    }

    let snapshot = snapshots
        .last()
        .cloned()
        .expect("nonempty snapshots have a latest snapshot");
    if snapshot.subject_revision != current_revision {
        return FindingLedgerEvaluation {
            status: FindingLedgerStatus::StaleSubject {
                record_revision: snapshot.subject_revision.clone(),
                current_revision: current_revision.to_owned(),
            },
            snapshot: Some(snapshot),
        };
    }

    if let Some(record_state) = snapshot.repository_state.as_ref() {
        let current_state = current_checkpoint_identity(config.artifact_root(), subject).ok();
        if current_state.as_deref() != Some(record_state.as_str()) {
            return FindingLedgerEvaluation {
                status: FindingLedgerStatus::StaleRepositoryState {
                    record_state: record_state.clone(),
                    current_state,
                },
                snapshot: Some(snapshot),
            };
        }
    }

    let accepted_unresolved = snapshot
        .accepted_unresolved()
        .map(|finding| (finding.policy_id.clone(), finding.statement.clone()))
        .collect::<BTreeSet<_>>();
    if &accepted_unresolved != failing_evidence {
        return FindingLedgerEvaluation {
            status: FindingLedgerStatus::SetMismatch {
                accepted_unresolved,
                failing_evidence: failing_evidence.clone(),
            },
            snapshot: Some(snapshot),
        };
    }

    FindingLedgerEvaluation {
        status: FindingLedgerStatus::Present,
        snapshot: Some(snapshot),
    }
}

/// Project the driver-confirmed implementation findings for one exact plan
/// task.  This is a mechanical view for the plan runner, not a second
/// authority: only the latest well-formed snapshot for each gate/subject pair
/// is considered, and only accepted unresolved findings owned by
/// `implementation` and explicitly routed to `task_id` survive.
///
/// The returned values are the original closed finding objects.  Keeping the
/// ledger fields intact lets a worker see the driver's reason and declared
/// review axes without receiving unrelated ledger history.
pub(crate) fn project_implementation_findings_at(
    context: &[ContextRecord],
    artifact_root: &Path,
    task_id: &str,
    working_directory: &Path,
) -> Result<Vec<Value>, String> {
    let root = fs::canonicalize(artifact_root).map_err(|error| {
        format!("could not canonicalize artifact_root for finding routing: {error}")
    })?;
    if !root.is_dir() {
        return Err("artifact_root for finding routing must be a directory".to_owned());
    }
    let latest = latest_routing_snapshots(context, &root, working_directory)?
        .into_iter()
        .filter(|(_, _, _, snapshot)| {
            routing_snapshot_is_current(&root, snapshot, working_directory)
        })
        .collect::<Vec<_>>();

    let mut projected = Vec::new();
    for (_, _, _, snapshot) in latest {
        for finding in snapshot.findings {
            if finding.is_accepted_unresolved()
                && finding.owner_phase.as_deref() == Some("implementation")
                && finding
                    .task_ids
                    .iter()
                    .any(|candidate| candidate == task_id)
            {
                projected.push(finding_to_value(&finding));
            }
        }
    }
    Ok(projected)
}

/// Resolve an exact bound ad-hoc repair selection from the latest well-formed
/// finding-ledger snapshots. Every requested ID must occur exactly once and
/// must be a current accepted unresolved implementation finding with no
/// frozen plan-task route.
pub(crate) fn project_implementation_repair_findings_at(
    context: &[ContextRecord],
    artifact_root: &Path,
    requested_ids: &[String],
    working_directory: &Path,
) -> Result<Vec<Value>, String> {
    let root = fs::canonicalize(artifact_root).map_err(|error| {
        format!("could not canonicalize artifact_root for finding routing: {error}")
    })?;
    if !root.is_dir() {
        return Err("artifact_root for finding routing must be a directory".to_owned());
    }
    let latest = latest_routing_snapshots(context, &root, working_directory)?;
    let requested = requested_ids.iter().collect::<BTreeSet<_>>();
    let mut candidates: BTreeMap<String, Vec<(bool, String, String, Finding)>> = BTreeMap::new();
    for (_, gate, subject, snapshot) in latest {
        let current = routing_snapshot_is_current(&root, &snapshot, working_directory);
        for finding in snapshot.findings {
            if requested.contains(&finding.id) {
                candidates.entry(finding.id.clone()).or_default().push((
                    current,
                    gate.clone(),
                    subject.clone(),
                    finding,
                ));
            }
        }
    }

    let mut projected = Vec::with_capacity(requested_ids.len());
    for id in requested_ids {
        let matches = candidates.get(id).map(Vec::as_slice).unwrap_or_default();
        if matches.is_empty() {
            return Err(format!("repair selection names unknown finding `{id}`"));
        }
        if matches.len() > 1 {
            return Err(format!(
                "repair selection finding `{id}` occurs in multiple ledger contexts"
            ));
        }
        let (current, _gate, subject, finding) = &matches[0];
        if subject != IMPLEMENTATION_SUBJECT {
            return Err(format!(
                "repair selection finding `{id}` belongs to `{subject}`, not `{IMPLEMENTATION_SUBJECT}`"
            ));
        }
        if !*current {
            return Err(format!(
                "repair selection finding `{id}` is stale for the current subject or repository checkpoint"
            ));
        }
        if finding.disposition != FindingDisposition::Accepted
            || finding.status != FindingStatus::Unresolved
        {
            return Err(format!(
                "repair selection finding `{id}` must be accepted and unresolved"
            ));
        }
        if finding.owner_phase.as_deref() != Some("implementation") {
            return Err(format!(
                "repair selection finding `{id}` is not implementation-owned"
            ));
        }
        if !finding.task_ids.is_empty() {
            return Err(format!(
                "repair selection finding `{id}` is routed to plan tasks"
            ));
        }
        projected.push(finding_to_value(finding));
    }
    Ok(projected)
}

fn latest_routing_snapshots(
    context: &[ContextRecord],
    root: &Path,
    _working_directory: &Path,
) -> Result<Vec<(u64, String, String, FindingLedgerSnapshot)>, String> {
    let root_value = Value::String(root.to_string_lossy().into_owned());
    let mut records = context
        .iter()
        .filter(|record| record.kind == FINDING_LEDGER_KIND)
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.sequence);

    let mut histories: BTreeMap<(String, String), Vec<(u64, FindingLedgerSnapshot)>> =
        BTreeMap::new();
    for record in records {
        let Some(object) = record.data.as_object() else {
            continue;
        };
        let Some(gate) = object.get("gate").and_then(Value::as_str) else {
            continue;
        };
        let Some(subject) = object.get("subject").and_then(Value::as_str) else {
            continue;
        };
        if !crate::config::GATE_IDS.contains(&gate)
            || !crate::config::SUBJECT_NAMES.contains(&subject)
            || !ledger_pair_is_valid(gate, subject)
        {
            continue;
        }
        let Ok(snapshot) = parse_snapshot(&record.data, gate, subject, Some(&root_value), None)
        else {
            // A malformed record is not a current view. The latest
            // well-formed snapshot remains the routing candidate.
            continue;
        };
        histories
            .entry((gate.to_owned(), subject.to_owned()))
            .or_default()
            .push((record.sequence.as_u64(), snapshot));
    }

    let mut latest = Vec::new();
    for ((gate, subject), snapshots) in histories {
        let snapshots_only = snapshots
            .iter()
            .map(|(_, snapshot)| snapshot.clone())
            .collect::<Vec<_>>();
        if check_id_continuity(&snapshots_only).is_err() {
            continue;
        }
        let Some((sequence, snapshot)) = snapshots.into_iter().last() else {
            continue;
        };
        latest.push((sequence, gate, subject, snapshot));
    }
    latest.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(latest)
}

/// Project the entries relevant to one assigned review axis from a current
/// snapshot.  Review packets still transport the immutable ledger history;
/// this exact projection is the worker-facing selection rule described by the
/// shipped preamble and is useful to callers that render a compact view.
#[allow(dead_code)]
pub(crate) fn project_review_findings(
    snapshot: &FindingLedgerSnapshot,
    review_axis: &str,
) -> Vec<Value> {
    snapshot
        .findings
        .iter()
        .filter(|finding| finding.review_axes.iter().any(|axis| axis == review_axis))
        .map(finding_to_value)
        .collect()
}

fn routing_snapshot_is_current(
    root: &Path,
    snapshot: &FindingLedgerSnapshot,
    working_directory: &Path,
) -> bool {
    let subject_path = root.join(&snapshot.subject);
    let Ok(bytes) = fs::read(subject_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let Some(revision) = value.get("revision").and_then(Value::as_str) else {
        return false;
    };
    if revision != snapshot.subject_revision {
        return false;
    }
    match snapshot.repository_state.as_ref() {
        Some(expected) => {
            checkpoint::phase_for_subject(&snapshot.subject)
                .and_then(|phase| checkpoint::verify(phase, root, working_directory).ok())
                .map(|checkpoint| checkpoint::state_identity(&checkpoint).to_owned())
                .as_deref()
                == Some(expected.as_str())
        }
        None => true,
    }
}

fn finding_to_value(finding: &Finding) -> Value {
    let source = match &finding.source {
        FindingSource::WorkSlot {
            invocation_id,
            worker_index,
            attempt,
            output_sha256,
        } => serde_json::json!({
            "kind": "work-slot",
            "invocation_id": invocation_id,
            "worker_index": worker_index,
            "attempt": attempt,
            "output_sha256": output_sha256,
        }),
        FindingSource::ExternalArtifact {
            relative_path,
            output_sha256,
        } => serde_json::json!({
            "kind": "external-artifact",
            "relative_path": relative_path,
            "output_sha256": output_sha256,
        }),
    };
    serde_json::json!({
        "id": finding.id,
        "source": source,
        "policy_id": finding.policy_id,
        "statement": finding.statement,
        "disposition": match finding.disposition {
            FindingDisposition::Accepted => "accepted",
            FindingDisposition::Rejected => "rejected",
            FindingDisposition::Advisory => "advisory",
        },
        "reason": finding.reason,
        "owner_phase": finding.owner_phase,
        "task_ids": finding.task_ids,
        "review_axes": finding.review_axes,
        "status": match finding.status {
            FindingStatus::Unresolved => "unresolved",
            FindingStatus::Resolved => "resolved",
            FindingStatus::Stale => "stale",
            FindingStatus::Recorded => "recorded",
        },
    })
}

fn ledger_pair_is_valid(gate: &str, subject: &str) -> bool {
    crate::workflow::PHASES.iter().any(|phase| {
        phase.subject == subject
            && (phase.parent_review == gate || phase.adversarial_review == gate)
    })
}

fn ledger_record_matches_pair(record: &ContextRecord, gate: &str, subject: &str) -> bool {
    let Some(data) = record.data.as_object() else {
        return false;
    };
    if data.get("gate").and_then(Value::as_str) != Some(gate) {
        return false;
    }
    match data.get("subject") {
        Some(Value::String(value)) => value == subject,
        // A missing or wrongly typed subject is attributable to the gate and
        // must be reported as malformed rather than silently ignored.
        Some(_) | None => true,
    }
}

fn parse_snapshot(
    value: &Value,
    expected_gate: &str,
    expected_subject: &str,
    artifact_root: Option<&Value>,
    config: Option<&ValidatedConfig>,
) -> Result<FindingLedgerSnapshot, Vec<String>> {
    let Some(object) = value.as_object() else {
        return Err(vec!["snapshot must be an object".to_owned()]);
    };
    let mut reasons = unknown_fields(object, SNAPSHOT_FIELDS, "snapshot");

    let schema_version = required_non_empty_string(object, "schema_version", &mut reasons);
    if schema_version.as_deref() != Some(SCHEMA_VERSION) {
        reasons.push("`schema_version` must be the constant string \"1\"".to_owned());
    }
    let gate = required_non_empty_string(object, "gate", &mut reasons);
    let subject = required_non_empty_string(object, "subject", &mut reasons);
    let subject_revision = required_non_empty_string(object, "subject_revision", &mut reasons);
    if gate.as_deref() != Some(expected_gate) {
        reasons.push(format!("`gate` must equal `{expected_gate}`"));
    }
    if subject.as_deref() != Some(expected_subject) {
        reasons.push(format!("`subject` must equal `{expected_subject}`"));
    }

    let author = parse_author(object.get("author"), &mut reasons);
    let repository_state = parse_repository_state(
        object.get("repository_state"),
        expected_subject,
        &mut reasons,
    );
    let findings = parse_findings(
        object.get("findings"),
        gate.as_deref().unwrap_or(expected_gate),
        artifact_root,
        config,
        &mut reasons,
    );

    if !reasons.is_empty() {
        return Err(reasons);
    }

    Ok(FindingLedgerSnapshot {
        schema_version: schema_version.expect("schema version checked above"),
        gate: gate.expect("gate checked above"),
        subject: subject.expect("subject checked above"),
        subject_revision: subject_revision.expect("revision checked above"),
        author: author.expect("author checked above"),
        repository_state,
        findings: findings.expect("findings checked above"),
    })
}

fn parse_author(value: Option<&Value>, reasons: &mut Vec<String>) -> Option<LedgerAuthor> {
    let Some(object) = value.and_then(Value::as_object) else {
        reasons.push("`author` must be an object".to_owned());
        return None;
    };
    reasons.extend(unknown_fields(object, AUTHOR_FIELDS, "author"));
    let name = required_non_empty_string(object, "name", reasons);
    let kind = required_non_empty_string(object, "kind", reasons);
    if let Some(kind) = kind.as_deref() {
        if kind != "human" && kind != "agent" {
            reasons.push("`author.kind` must be `human` or `agent`".to_owned());
        }
    }
    match (name, kind) {
        (Some(name), Some(kind)) if kind == "human" || kind == "agent" => {
            Some(LedgerAuthor { name, kind })
        }
        _ => None,
    }
}

fn parse_repository_state(
    value: Option<&Value>,
    subject: &str,
    reasons: &mut Vec<String>,
) -> Option<String> {
    let Some(value) = value else {
        reasons.push("missing `repository_state`".to_owned());
        return None;
    };
    let report_subject = matches!(subject, IMPLEMENTATION_SUBJECT | VALIDATION_SUBJECT);
    if !report_subject {
        if !value.is_null() {
            reasons.push(format!("`repository_state` must be null for `{subject}`"));
        }
        return None;
    }

    let Some(state) = value.as_str() else {
        reasons.push(format!(
            "`repository_state` must be a sha256 digest for `{subject}`"
        ));
        return None;
    };
    if !valid_sha256(state) {
        reasons.push("`repository_state` must be sha256:<64 lowercase hex>".to_owned());
        return None;
    }
    Some(state.to_owned())
}

fn parse_findings(
    value: Option<&Value>,
    gate: &str,
    artifact_root: Option<&Value>,
    config: Option<&ValidatedConfig>,
    reasons: &mut Vec<String>,
) -> Option<Vec<Finding>> {
    let Some(value) = value else {
        reasons.push("missing `findings`".to_owned());
        return None;
    };
    let Some(items) = value.as_array() else {
        reasons.push("`findings` must be an array".to_owned());
        return None;
    };

    let mut ids = BTreeSet::new();
    let mut findings = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match parse_finding(item, index, gate, artifact_root, config) {
            Ok(finding) => {
                if !ids.insert(finding.id.clone()) {
                    reasons.push(format!("duplicate finding id `{}`", finding.id));
                }
                findings.push(finding);
            }
            Err(item_reasons) => reasons.extend(item_reasons),
        }
    }
    Some(findings)
}

fn parse_finding(
    value: &Value,
    index: usize,
    gate: &str,
    artifact_root: Option<&Value>,
    config: Option<&ValidatedConfig>,
) -> Result<Finding, Vec<String>> {
    let path = format!("findings[{index}]");
    let Some(object) = value.as_object() else {
        return Err(vec![format!("`{path}` must be an object")]);
    };
    let mut reasons = unknown_fields(object, FINDING_FIELDS, &path);

    let id = required_non_empty_string(object, "id", &mut reasons);
    if id.as_deref().is_some_and(|id| !valid_finding_id(id)) {
        reasons.push(format!(
            "`{path}.id` must match F-[a-z0-9][a-z0-9_-]{{0,63}}"
        ));
    }
    let source = parse_source(
        object.get("source"),
        &path,
        gate,
        artifact_root,
        &mut reasons,
    );
    let policy_id = required_non_empty_string(object, "policy_id", &mut reasons);
    let statement = required_non_empty_string(object, "statement", &mut reasons);
    let disposition = parse_disposition(object.get("disposition"), &path, &mut reasons);
    let reason = required_non_empty_string(object, "reason", &mut reasons);
    let owner_phase = parse_owner_phase(object.get("owner_phase"), &path, &mut reasons);
    let task_ids = parse_string_array(
        object.get("task_ids"),
        &format!("{path}.task_ids"),
        &mut reasons,
    );
    let review_axes = parse_string_array(
        object.get("review_axes"),
        &format!("{path}.review_axes"),
        &mut reasons,
    );
    let status = parse_status(object.get("status"), &path, &mut reasons);

    if let (Some(disposition), Some(status), Some(owner_phase), Some(task_ids), Some(review_axes)) = (
        disposition,
        status,
        owner_phase.as_ref(),
        task_ids.as_ref(),
        review_axes.as_ref(),
    ) {
        validate_combination(
            disposition,
            status,
            owner_phase,
            task_ids,
            review_axes,
            &path,
            &mut reasons,
        );
    }

    if let Some(config) = config {
        if let Some(policy_id) = policy_id.as_ref() {
            if !configured_policy_id(config, policy_id) {
                reasons.push(format!("`{path}.policy_id` is not a configured policy id"));
            }
        }
        if let Some(review_axes) = review_axes.as_ref() {
            for axis in review_axes {
                if !configured_policy_id(config, axis) {
                    reasons.push(format!(
                        "`{path}.review_axes` contains unknown policy id `{axis}`"
                    ));
                }
            }
        }
    }
    if let Some(task_ids) = task_ids.as_ref() {
        if !task_ids.is_empty() {
            match current_plan_task_ids(artifact_root) {
                Ok(current) => {
                    for task_id in task_ids {
                        if !current.contains(task_id) {
                            reasons.push(format!(
                                "`{path}.task_ids` contains unknown plan task id `{task_id}`"
                            ));
                        }
                    }
                }
                Err(error) => reasons.push(format!("cannot validate `{path}.task_ids`: {error}")),
            }
        }
    }

    if !reasons.is_empty() {
        return Err(reasons);
    }

    Ok(Finding {
        id: id.expect("id checked above"),
        source: source.expect("source checked above"),
        policy_id: policy_id.expect("policy id checked above"),
        statement: statement.expect("statement checked above"),
        disposition: disposition.expect("disposition checked above"),
        reason: reason.expect("reason checked above"),
        owner_phase: owner_phase.expect("owner phase checked above"),
        task_ids: task_ids.expect("task ids checked above"),
        review_axes: review_axes.expect("review axes checked above"),
        status: status.expect("status checked above"),
    })
}

fn parse_source(
    value: Option<&Value>,
    path: &str,
    gate: &str,
    artifact_root: Option<&Value>,
    reasons: &mut Vec<String>,
) -> Option<FindingSource> {
    let Some(value) = value else {
        reasons.push(format!("`{path}.source` is missing"));
        return None;
    };
    let Some(object) = value.as_object() else {
        reasons.push(format!("`{path}.source` must be an object"));
        return None;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        reasons.extend(unknown_fields(
            object,
            &[
                "kind",
                "invocation_id",
                "worker_index",
                "attempt",
                "output_sha256",
                "relative_path",
            ],
            &format!("{path}.source"),
        ));
        reasons.push(format!("`{path}.source.kind` must be a string"));
        return None;
    };

    let source = match kind {
        "work-slot" => {
            reasons.extend(unknown_fields(
                object,
                WORK_SLOT_SOURCE_FIELDS,
                &format!("{path}.source"),
            ));
            let invocation_id = required_non_empty_string(object, "invocation_id", reasons);
            let worker_index = match object.get("worker_index").and_then(Value::as_u64) {
                Some(index) => Some(index),
                None => {
                    reasons.push(format!(
                        "`{path}.source.worker_index` must be a nonnegative integer"
                    ));
                    None
                }
            };
            let attempt = match object.get("attempt").and_then(Value::as_u64) {
                Some(1) => Some(1_u8),
                Some(2) => Some(2_u8),
                _ => {
                    reasons.push(format!("`{path}.source.attempt` must be 1 or 2"));
                    None
                }
            };
            let output_sha256 = parse_digest(
                object.get("output_sha256"),
                &format!("{path}.source.output_sha256"),
                reasons,
            );
            match (invocation_id, worker_index, attempt, output_sha256) {
                (Some(invocation_id), Some(worker_index), Some(attempt), Some(output_sha256)) => {
                    let source = FindingSource::WorkSlot {
                        invocation_id,
                        worker_index,
                        attempt,
                        output_sha256,
                    };
                    if let Err(error) = validate_source_bytes(artifact_root, gate, path, &source) {
                        reasons.push(error);
                    }
                    Some(source)
                }
                _ => None,
            }
        }
        "external-artifact" => {
            reasons.extend(unknown_fields(
                object,
                EXTERNAL_SOURCE_FIELDS,
                &format!("{path}.source"),
            ));
            let relative_path = match object.get("relative_path").and_then(Value::as_str) {
                Some(value) if !value.is_empty() => match normalize_relative_path(value) {
                    Ok(normalized) => Some(normalized.to_string_lossy().into_owned()),
                    Err(_) => {
                        reasons.push(format!(
                            "`{path}.source.relative_path` must be a relative path inside artifact_root"
                        ));
                        None
                    }
                },
                Some(_) => {
                    reasons.push(format!("`{path}.source.relative_path` must not be empty"));
                    None
                }
                None => {
                    reasons.push(format!("`{path}.source.relative_path` must be a string"));
                    None
                }
            };
            let output_sha256 = parse_digest(
                object.get("output_sha256"),
                &format!("{path}.source.output_sha256"),
                reasons,
            );
            match (relative_path, output_sha256) {
                (Some(relative_path), Some(output_sha256)) => {
                    let source = FindingSource::ExternalArtifact {
                        relative_path,
                        output_sha256,
                    };
                    if let Err(error) = validate_source_bytes(artifact_root, gate, path, &source) {
                        reasons.push(error);
                    }
                    Some(source)
                }
                _ => None,
            }
        }
        _ => {
            reasons.extend(unknown_fields(object, &["kind"], &format!("{path}.source")));
            reasons.push(format!(
                "`{path}.source.kind` must be `work-slot` or `external-artifact`"
            ));
            None
        }
    };
    source
}

fn parse_disposition(
    value: Option<&Value>,
    path: &str,
    reasons: &mut Vec<String>,
) -> Option<FindingDisposition> {
    match value.and_then(Value::as_str) {
        Some("accepted") => Some(FindingDisposition::Accepted),
        Some("rejected") => Some(FindingDisposition::Rejected),
        Some("advisory") => Some(FindingDisposition::Advisory),
        Some(_) => {
            reasons.push(format!(
                "`{path}.disposition` must be accepted, rejected, or advisory"
            ));
            None
        }
        None => {
            reasons.push(format!("`{path}.disposition` must be a string"));
            None
        }
    }
}

fn parse_status(
    value: Option<&Value>,
    path: &str,
    reasons: &mut Vec<String>,
) -> Option<FindingStatus> {
    match value.and_then(Value::as_str) {
        Some("unresolved") => Some(FindingStatus::Unresolved),
        Some("resolved") => Some(FindingStatus::Resolved),
        Some("stale") => Some(FindingStatus::Stale),
        Some("recorded") => Some(FindingStatus::Recorded),
        Some(_) => {
            reasons.push(format!(
                "`{path}.status` must be unresolved, resolved, stale, or recorded"
            ));
            None
        }
        None => {
            reasons.push(format!("`{path}.status` must be a string"));
            None
        }
    }
}

fn parse_owner_phase(
    value: Option<&Value>,
    path: &str,
    reasons: &mut Vec<String>,
) -> Option<Option<String>> {
    match value {
        Some(Value::Null) => Some(None),
        Some(Value::String(value)) if !value.is_empty() => Some(Some(value.clone())),
        Some(Value::String(_)) => {
            reasons.push(format!("`{path}.owner_phase` must not be empty"));
            None
        }
        Some(_) => {
            reasons.push(format!("`{path}.owner_phase` must be a string or null"));
            None
        }
        None => {
            reasons.push(format!("missing `{path}.owner_phase`"));
            None
        }
    }
}

fn parse_string_array(
    value: Option<&Value>,
    path: &str,
    reasons: &mut Vec<String>,
) -> Option<Vec<String>> {
    let Some(value) = value else {
        reasons.push(format!("missing `{path}`"));
        return None;
    };
    let Some(items) = value.as_array() else {
        reasons.push(format!("`{path}` must be an array"));
        return None;
    };
    let mut parsed = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(value) if !value.is_empty() => parsed.push(value.to_owned()),
            Some(_) => reasons.push(format!("`{path}[{index}]` must not be empty")),
            None => reasons.push(format!("`{path}[{index}]` must be a string")),
        }
    }
    Some(parsed)
}

fn validate_combination(
    disposition: FindingDisposition,
    status: FindingStatus,
    owner_phase: &Option<String>,
    task_ids: &[String],
    review_axes: &[String],
    path: &str,
    reasons: &mut Vec<String>,
) {
    match disposition {
        FindingDisposition::Accepted => {
            if !matches!(
                status,
                FindingStatus::Unresolved | FindingStatus::Resolved | FindingStatus::Stale
            ) {
                reasons.push(format!(
                    "`{path}` accepted findings must have status unresolved, resolved, or stale"
                ));
            }
            match owner_phase.as_deref() {
                Some("intent" | "design" | "plan" | "implementation" | "validation") => {}
                Some(_) => reasons.push(format!(
                    "`{path}.owner_phase` must be intent, design, plan, implementation, or validation"
                )),
                None => reasons.push(format!("`{path}.owner_phase` is required for accepted findings")),
            }
        }
        FindingDisposition::Rejected | FindingDisposition::Advisory => {
            if !matches!(status, FindingStatus::Recorded | FindingStatus::Stale) {
                reasons.push(format!(
                    "`{path}` rejected and advisory findings must have status recorded or stale"
                ));
            }
            if owner_phase.is_some() {
                reasons.push(format!(
                    "`{path}.owner_phase` must be null for rejected or advisory findings"
                ));
            }
            if !task_ids.is_empty() {
                reasons.push(format!(
                    "`{path}.task_ids` must be empty for rejected or advisory findings"
                ));
            }
            if !review_axes.is_empty() {
                reasons.push(format!(
                    "`{path}.review_axes` must be empty for rejected or advisory findings"
                ));
            }
        }
    }
}

fn configured_policy_id(config: &ValidatedConfig, id: &str) -> bool {
    config.axis_namespace().values().any(|ids| ids.contains(id))
}

fn current_plan_task_ids(artifact_root: Option<&Value>) -> Result<BTreeSet<String>, String> {
    let value = read_json_under_root(artifact_root, PLAN_SUBJECT)?;
    let Some(tasks) = value.get("tasks").and_then(Value::as_array) else {
        return Err("plan.json must contain an array `tasks`".to_owned());
    };
    let mut ids = BTreeSet::new();
    for (index, task) in tasks.iter().enumerate() {
        let Some(task) = task.as_object() else {
            return Err(format!("plan.json tasks[{index}] must be an object"));
        };
        let Some(id) = task.get("id").and_then(Value::as_str) else {
            return Err(format!("plan.json tasks[{index}].id must be a string"));
        };
        if id.is_empty() {
            return Err(format!("plan.json tasks[{index}].id must not be empty"));
        }
        ids.insert(id.to_owned());
    }
    Ok(ids)
}

fn check_id_continuity(snapshots: &[FindingLedgerSnapshot]) -> Result<(), Vec<String>> {
    let mut known: BTreeMap<String, FindingIdentity> = BTreeMap::new();
    let mut reasons = Vec::new();
    for snapshot in snapshots {
        for finding in &snapshot.findings {
            let identity = FindingIdentity::owned(finding);
            if let Some(previous) = known.get(&finding.id) {
                if previous != &identity {
                    reasons.push(format!(
                        "finding id `{}` changed source, policy_id, or statement; use a new ID",
                        finding.id
                    ));
                }
            } else {
                known.insert(finding.id.clone(), identity);
            }
        }
    }

    // Within one subject revision, an accepted unresolved finding cannot
    // disappear from a later valid snapshot.  Keep the previous snapshot for
    // each revision so an intervening revision cannot hide an omission.  The
    // driver must carry it forward with an explicit resolved or stale status
    // (or leave it unresolved).
    let mut previous_unresolved_by_revision: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for snapshot in snapshots {
        let current_ids = snapshot
            .findings
            .iter()
            .map(|finding| finding.id.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(previous_unresolved) =
            previous_unresolved_by_revision.get(&snapshot.subject_revision)
        {
            for finding_id in previous_unresolved {
                if !current_ids.contains(finding_id.as_str()) {
                    reasons.push(format!(
                        "accepted unresolved finding `{finding_id}` was omitted from a later snapshot; record resolved or stale explicitly"
                    ));
                }
            }
        }
        previous_unresolved_by_revision.insert(
            snapshot.subject_revision.clone(),
            snapshot
                .accepted_unresolved()
                .map(|finding| finding.id.clone())
                .collect(),
        );
    }

    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FindingIdentity {
    source: FindingSourceIdentity,
    policy_id: String,
    statement: String,
}

impl FindingIdentity {
    fn owned(finding: &Finding) -> FindingIdentity {
        let source = match &finding.source {
            FindingSource::WorkSlot {
                invocation_id,
                worker_index,
                attempt,
                output_sha256,
            } => FindingSourceIdentity::WorkSlot {
                invocation_id: invocation_id.clone(),
                worker_index: *worker_index,
                attempt: *attempt,
                output_sha256: output_sha256.clone(),
            },
            FindingSource::ExternalArtifact {
                relative_path,
                output_sha256,
            } => FindingSourceIdentity::ExternalArtifact {
                relative_path: relative_path.clone(),
                output_sha256: output_sha256.clone(),
            },
        };
        Self {
            source,
            policy_id: finding.policy_id.clone(),
            statement: finding.statement.clone(),
        }
    }
}

fn validate_source_bytes(
    artifact_root: Option<&Value>,
    gate: &str,
    finding_path: &str,
    source: &FindingSource,
) -> Result<(), String> {
    match source {
        FindingSource::WorkSlot {
            invocation_id,
            worker_index,
            attempt,
            output_sha256,
        } => {
            if !single_path_component(invocation_id) {
                return Err(format!(
                    "`{finding_path}.source.invocation_id` must be a safe invocation id"
                ));
            }
            let root = canonical_root(artifact_root)?;
            let capture_dir = root
                .join("work-slot-captures")
                .join(gate)
                .join(invocation_id)
                .join(worker_index.to_string());
            let worker_dir = canonical_contained_dir(&root, &capture_dir)?;
            let attempts_path = worker_dir.join("attempts.json");
            let attempts_bytes = read_contained_file(&root, &attempts_path)?;
            let manifest: AttemptsManifest = serde_json::from_slice(&attempts_bytes)
                .map_err(|error| format!("work-slot attempts.json is malformed: {error}"))?;
            validate_attempts_manifest(&manifest)?;
            if manifest.selected_attempt != Some(u64::from(*attempt)) {
                return Err(format!(
                    "work-slot source attempt {} is not the selected_attempt in attempts.json",
                    attempt
                ));
            }
            let entry = manifest
                .attempts
                .iter()
                .find(|entry| entry.number == u64::from(*attempt))
                .ok_or_else(|| format!("attempts.json does not list attempt {attempt}"))?;
            // Validate every manifest entry, not only the selected one. The
            // append-only capture contract keeps both attempts available, so
            // a damaged non-selected attempt must not become an undetectable
            // hole in the immutable reviewer evidence.
            for listed in &manifest.attempts {
                let attempt_dir = worker_dir.join("attempts").join(listed.number.to_string());
                let stdout_path = attempt_dir.join("stdout");
                let stdout = read_contained_file(&root, &stdout_path)?;
                if sha256_digest(&stdout) != listed.stdout_sha256 {
                    return Err(format!(
                        "attempts.json stdout digest does not match raw bytes for attempt {}",
                        listed.number
                    ));
                }
                let stderr_path = attempt_dir.join("stderr");
                let stderr = read_contained_file(&root, &stderr_path)?;
                if sha256_digest(&stderr) != listed.stderr_sha256 {
                    return Err(format!(
                        "attempts.json stderr digest does not match raw bytes for attempt {}",
                        listed.number
                    ));
                }
            }
            let stdout_path = worker_dir
                .join("attempts")
                .join(attempt.to_string())
                .join("stdout");
            let stdout = read_contained_file(&root, &stdout_path)?;
            let actual = sha256_digest(&stdout);
            if actual != *output_sha256 || actual != entry.stdout_sha256 {
                return Err(format!(
                    "work-slot source stdout digest does not match output_sha256 and attempts.json for attempt {attempt}"
                ));
            }
            Ok(())
        }
        FindingSource::ExternalArtifact {
            relative_path,
            output_sha256,
        } => {
            let root = canonical_root(artifact_root)?;
            let relative = normalize_relative_path(relative_path)?;
            let path = root.join(relative);
            let bytes = read_contained_file(&root, &path)?;
            let actual = sha256_digest(&bytes);
            if actual != *output_sha256 {
                return Err(format!(
                    "external-artifact source `{relative_path}` digest does not match raw bytes"
                ));
            }
            Ok(())
        }
    }
}

fn canonical_root(artifact_root: Option<&Value>) -> Result<PathBuf, String> {
    let Some(value) = artifact_root.and_then(Value::as_str) else {
        return Err("artifact_root is required for finding source linkage".to_owned());
    };
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("artifact_root must be an absolute path".to_owned());
    }
    let root = fs::canonicalize(path)
        .map_err(|error| format!("could not canonicalize artifact_root: {error}"))?;
    if !root.is_dir() {
        return Err("artifact_root must name a directory".to_owned());
    }
    Ok(root)
}

fn canonical_contained_dir(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("could not resolve finding source directory: {error}"))?;
    if !is_contained(root, &canonical) {
        return Err("finding source path escapes artifact_root".to_owned());
    }
    if !canonical.is_dir() {
        return Err("finding source capture path must be a directory".to_owned());
    }
    Ok(canonical)
}

fn read_contained_file(root: &Path, path: &Path) -> Result<Vec<u8>, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("could not resolve finding source file: {error}"))?;
    if !is_contained(root, &canonical) {
        return Err("finding source file escapes artifact_root".to_owned());
    }
    fs::read(&canonical).map_err(|error| format!("could not read finding source file: {error}"))
}

fn is_contained(root: &Path, target: &Path) -> bool {
    target != root
        && target
            .strip_prefix(root)
            .map(|relative| !relative.as_os_str().is_empty())
            .unwrap_or(false)
}

fn read_json_under_root(artifact_root: Option<&Value>, subject: &str) -> Result<Value, String> {
    let root = canonical_root(artifact_root)?;
    let path = root.join(subject);
    let bytes = read_contained_file(&root, &path)?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{subject} is not valid JSON: {error}"))
}

fn current_checkpoint_identity(
    artifact_root: Option<&Value>,
    subject: &str,
) -> Result<String, String> {
    let phase = checkpoint::phase_for_subject(subject)
        .ok_or_else(|| format!("subject `{subject}` has no checkpoint identity"))?;
    let root = artifact_root
        .and_then(Value::as_str)
        .ok_or_else(|| "artifact_root is required for checkpoint identity".to_owned())?;
    checkpoint::current_state_from_cwd(phase, Path::new(root))
}

fn validate_attempts_manifest(manifest: &AttemptsManifest) -> Result<(), String> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err("attempts.json schema_version must be the constant string \"1\"".to_owned());
    }
    if manifest.attempts.is_empty() || manifest.attempts.len() > 2 {
        return Err("attempts.json must list one or two attempts".to_owned());
    }
    let mut numbers = BTreeSet::new();
    for (index, attempt) in manifest.attempts.iter().enumerate() {
        if attempt.number != index as u64 + 1 {
            return Err(
                "attempts.json attempt numbers must be consecutive starting at 1".to_owned(),
            );
        }
        if !numbers.insert(attempt.number) {
            return Err("attempts.json contains duplicate attempt numbers".to_owned());
        }
        if !valid_sha256(&attempt.stdout_sha256) || !valid_sha256(&attempt.stderr_sha256) {
            return Err("attempts.json contains an invalid digest".to_owned());
        }
    }
    match (manifest.selected_attempt, manifest.exhausted) {
        (Some(selected), false) if numbers.contains(&selected) => {
            let selected_entry = manifest
                .attempts
                .iter()
                .find(|attempt| attempt.number == selected)
                .expect("selected number is in the set");
            if !selected_entry.validation_errors.is_empty() {
                return Err("attempts.json selected_attempt has validation errors".to_owned());
            }
        }
        (None, true) => {}
        _ => {
            return Err(
                "attempts.json selected_attempt/exhausted fields are inconsistent".to_owned(),
            )
        }
    }
    Ok(())
}

fn normalize_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err("path is absolute".to_owned());
    }
    let mut normalized = PathBuf::new();
    let mut saw_component = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                normalized.push(part);
                saw_component = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("path escapes artifact root".to_owned());
                }
            }
            Component::RootDir | Component::Prefix(_) => return Err("path is absolute".to_owned()),
        }
    }
    if !saw_component || normalized.as_os_str().is_empty() {
        return Err("path is empty".to_owned());
    }
    Ok(normalized)
}

fn single_path_component(value: &str) -> bool {
    let path = Path::new(value);
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn required_non_empty_string(
    object: &Map<String, Value>,
    field: &str,
    reasons: &mut Vec<String>,
) -> Option<String> {
    match object.get(field).and_then(Value::as_str) {
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

fn parse_digest(value: Option<&Value>, path: &str, reasons: &mut Vec<String>) -> Option<String> {
    match value.and_then(Value::as_str) {
        Some(value) if valid_sha256(value) => Some(value.to_owned()),
        Some(_) => {
            reasons.push(format!("`{path}` must be sha256:<64 lowercase hex>"));
            None
        }
        None => {
            reasons.push(format!("`{path}` must be a string"));
            None
        }
    }
}

fn unknown_fields(object: &Map<String, Value>, allowed: &[&str], path: &str) -> Vec<String> {
    object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(|key| format!("{path} has unknown field `{key}`"))
        .collect()
}

fn valid_finding_id(value: &str) -> bool {
    let mut chars = value.chars();
    if chars.next() != Some('F') || chars.next() != Some('-') {
        return false;
    }
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    let rest = chars.collect::<Vec<_>>();
    if rest.len() > 63 {
        return false;
    }
    rest.into_iter().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-'
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == "sha256:".len() + 64
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn set_to_values(set: &BTreeSet<(String, String)>) -> Vec<Value> {
    set.iter()
        .map(|(policy_id, statement)| {
            serde_json::json!({
                "policy_id": policy_id,
                "statement": statement
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptsManifest {
    schema_version: String,
    attempts: Vec<AttemptEntry>,
    selected_attempt: Option<u64>,
    exhausted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptEntry {
    number: u64,
    stdout_sha256: String,
    stderr_sha256: String,
    validation_errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_initial_input;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new() -> Self {
            let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "software-change-finding-ledger-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn value(&self) -> Value {
            Value::String(self.path.to_string_lossy().into_owned())
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn config(root: &TempRoot) -> ValidatedConfig {
        parse_initial_input(&json!({
            "config_version": "test-1",
            "artifact_root": root.value(),
            "review_policies": {"intent-review": [{"id": "axis", "description": "axis"}, {"id": "other-axis", "description": "other"}]},
            "artifact_schemas": {
                "intent.json": {
                    "type": "object",
                    "properties": {
                        "revision": {"type": "string"},
                        "author": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                            },
                            "required": ["name", "kind"],
                            "additionalProperties": false
                        }
                    },
                    "required": ["revision", "author"],
                    "additionalProperties": false
                }
            }
        }))
        .unwrap()
    }

    fn external_source(root: &TempRoot, bytes: &[u8]) -> Value {
        fs::write(root.path.join("review.stdout"), bytes).unwrap();
        json!({
            "kind": "external-artifact",
            "relative_path": "review.stdout",
            "output_sha256": sha256_digest(bytes)
        })
    }

    fn snapshot(root: &TempRoot, statement: &str) -> Value {
        json!({
            "schema_version": "1",
            "gate": "intent-review",
            "subject": "intent.json",
            "subject_revision": "r1",
            "author": {"name": "driver", "kind": "agent"},
            "repository_state": null,
            "findings": [{
                "id": "F-one",
                "source": external_source(root, b"review"),
                "policy_id": "axis",
                "statement": statement,
                "disposition": "rejected",
                "reason": "driver rejected it",
                "owner_phase": null,
                "task_ids": [],
                "review_axes": [],
                "status": "recorded"
            }]
        })
    }

    fn context(sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            format!("ledger-{sequence}"),
            FINDING_LEDGER_KIND,
            data,
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    #[test]
    fn id_pattern_and_closed_fields_are_enforced() {
        let root = TempRoot::new();
        let config = config(&root);
        let mut data = snapshot(&root, "statement");
        data["findings"][0]["id"] = json!("F-A");
        let result = evaluate_finding_ledger(
            &[context(1, data)],
            "intent-review",
            "intent.json",
            "r1",
            &config,
            &BTreeSet::new(),
        );
        assert!(matches!(
            result.status,
            FindingLedgerStatus::Malformed { .. }
        ));
    }

    #[test]
    fn empty_rejected_snapshot_agrees_with_empty_failing_set() {
        let root = TempRoot::new();
        let config = config(&root);
        let mut data = snapshot(&root, "statement");
        data["findings"] = json!([]);
        let result = evaluate_finding_ledger(
            &[context(1, data)],
            "intent-review",
            "intent.json",
            "r1",
            &config,
            &BTreeSet::new(),
        );
        assert!(result.is_satisfied());
    }

    #[test]
    fn routing_rejects_cross_phase_gate_subject_pairs() {
        assert!(ledger_pair_is_valid("plan-review", "plan.json"));
        assert!(ledger_pair_is_valid(
            "implementation-adversarial-review",
            "implementation-report.json"
        ));
        assert!(!ledger_pair_is_valid("intent-review", "plan.json"));
        assert!(!ledger_pair_is_valid("plan-review", "design.json"));
    }

    #[test]
    fn review_projection_selects_only_the_assigned_axis() {
        let root = TempRoot::new();
        let config = config(&root);
        let mut data = snapshot(&root, "statement");
        data["findings"][0]["disposition"] = json!("accepted");
        data["findings"][0]["status"] = json!("unresolved");
        data["findings"][0]["owner_phase"] = json!("design");
        data["findings"][0]["review_axes"] = json!(["axis"]);
        let first = data["findings"][0].clone();
        let mut second = first.clone();
        second["id"] = json!("F-two");
        second["review_axes"] = json!(["other-axis"]);
        data["findings"] = json!([first, second]);
        let parsed = parse_snapshot(
            &data,
            "intent-review",
            "intent.json",
            Some(&root.value()),
            Some(&config),
        )
        .expect("well-formed snapshot");
        let projected = project_review_findings(&parsed, "axis");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["id"], "F-one");
    }
}
