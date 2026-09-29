//! Read-only projection of selected bound software-change review output.
//!
//! This command deliberately consumes the ordinary JSON `show` envelope rather
//! than opening the Loop Engine catalog.  Invocation and assignment metadata
//! remains engine-owned; this module only resolves the selected bytes named by
//! that metadata and normalizes the provider-owned review judgment.

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: &str = "3";
const ATTEMPTS_SCHEMA_VERSION: &str = "1";
const WORKFLOW_ID: &str = "software-change";
const ATTEMPTS_FILE: &str = "attempts.json";
const REVIEW_FIELDS: &[&str] = &["axis", "author", "result", "findings"];
const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];

/// The closed machine-readable result of `software-change review-candidates`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewCandidatesDocument {
    pub schema_version: &'static str,
    pub candidates: Vec<ReviewCandidate>,
    /// Exact, inert context-record candidates for explicit driver append.
    pub records: Vec<ReviewRecordPreview>,
}

/// Read-only source-checked preview of one factual append or reuse diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewRecordPreview {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub gate: String,
    pub subject: String,
    pub subject_revision: String,
    pub config_version: String,
    pub review_stage: String,
    pub origin: CandidateOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_checkpoint: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

/// One inert candidate or mechanical diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status")]
pub enum ReviewCandidate {
    #[serde(rename = "verdict-ready")]
    VerdictReady {
        origin: CandidateOrigin,
        #[serde(skip_serializing_if = "Option::is_none")]
        review_contract_version: Option<u32>,
        record_id: String,
        kind: String,
        data: Value,
    },
    #[serde(rename = "ready")]
    Ready {
        origin: CandidateOrigin,
        review_stage: String,
        axis: String,
        author: CandidateAuthor,
        result: String,
        findings: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        review_contract_version: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        grounds: Option<Value>,
    },
    #[serde(rename = "carried")]
    Carried {
        origin: CandidateOrigin,
        review_stage: String,
        axis: String,
        author: CandidateAuthor,
        #[serde(skip_serializing_if = "Option::is_none")]
        review_contract_version: Option<u32>,
        applicability_id: String,
    },
    #[serde(rename = "malformed")]
    Malformed {
        origin: CandidateOrigin,
        diagnostic: String,
    },
    #[serde(rename = "unavailable")]
    Unavailable {
        origin: CandidateOrigin,
        diagnostic: String,
    },
    #[serde(rename = "missing-selection")]
    MissingSelection {
        origin: CandidateOrigin,
        diagnostic: String,
    },
    #[serde(rename = "exhausted")]
    Exhausted {
        origin: CandidateOrigin,
        diagnostic: String,
    },
}

/// The only source identity copied into a candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateOrigin {
    pub kind: &'static str,
    pub id: String,
    pub assignment_id: String,
}

/// The normalized reviewer author claim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateAuthor {
    pub name: String,
    pub kind: String,
}

/// An invalid or unusable input show document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionError {
    message: String,
}

impl ProjectionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectionError {}

/// Project one ordinary, completed Loop Engine `show` envelope.
///
/// The input is never changed.  The returned candidates contain no selected
/// path, digest, attempt, command, binding, or capture metadata.
pub fn project(input: &Value) -> Result<ReviewCandidatesDocument, ProjectionError> {
    let result = show_result(input)?;
    let initial_input = result
        .get("initial_input")
        .and_then(Value::as_object)
        .ok_or_else(|| ProjectionError::new("show result is missing object `initial_input`"))?;
    let policies = initial_input
        .get("review_policies")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ProjectionError::new("show result initial_input is missing object `review_policies`")
        })?;
    let eligible_slots = policies
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    let invocations = result
        .get("work_slot_invocations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProjectionError::new("show result is missing array `work_slot_invocations`")
        })?;

    let context: Vec<loop_core::ContextRecord> = serde_json::from_value(
        result
            .get("context")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|error| ProjectionError::new(format!("show result context is malformed: {error}")))?;
    let mut candidates = Vec::new();
    let mut records = Vec::new();
    for invocation in invocations {
        let invocation = invocation.as_object().ok_or_else(|| {
            ProjectionError::new("show result work_slot_invocations contains a non-object")
        })?;
        let Some(slot_id) = invocation.get("slot_id").and_then(Value::as_str) else {
            continue;
        };
        if !eligible_slots.contains(slot_id) {
            // Draft, task, summarizer, and other non-review slots are outside
            // this provider-owned projection.
            continue;
        }

        let invocation_id = invocation
            .get("invocation_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProjectionError::new(
                    "eligible review invocation is missing non-empty `invocation_id`",
                )
            })?;
        match invocation.get("status").and_then(Value::as_str) {
            Some("running") | Some("overrun") => continue,
            Some("succeeded") | Some("failed") => {}
            Some(status) => {
                return Err(ProjectionError::new(format!(
                    "eligible review invocation has unsupported status `{status}`"
                )))
            }
            None => {
                return Err(ProjectionError::new(
                    "eligible review invocation is missing string `status`",
                ))
            }
        }

        let workers = invocation
            .get("inner_workers")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProjectionError::new("eligible review invocation is missing array `inner_workers`")
            })?;
        let capture_dir = invocation.get("capture_dir").and_then(Value::as_str);
        for (worker_index, worker) in workers.iter().enumerate() {
            let worker = worker.as_object().ok_or_else(|| {
                ProjectionError::new("eligible review invocation contains a non-object worker")
            })?;
            let Some(contract) = worker.get("declared_output_contract") else {
                continue;
            };
            let recovered_review_contract =
                worker.get("recovery_source").is_some_and(|source| {
                    source.get("source_class").and_then(Value::as_str) == Some("eligible-derived")
                }) && derived_legacy_review_schema(contract, slot_id, initial_input).is_some();
            if contract.is_null()
                || (!looks_like_review_contract(contract) && !recovered_review_contract)
            {
                continue;
            }

            let assignment_id = worker
                .get("assignment_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ProjectionError::new(
                        "eligible review assignment is missing non-empty `assignment_id`",
                    )
                })?;
            let origin = CandidateOrigin {
                kind: "selected-assignment-output",
                id: invocation_id.to_owned(),
                assignment_id: assignment_id.to_owned(),
            };
            let require_fresh_aggregate = initial_input
                .get("contract_version")
                .and_then(Value::as_u64)
                == Some(3)
                && policies
                    .get(slot_id)
                    .and_then(Value::as_array)
                    .is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry
                                .get("review_stage")
                                .or_else(|| entry.get("stage"))
                                .and_then(Value::as_str)
                                == Some("individual")
                        })
                    });
            let assignment_candidates = project_assignment(
                origin.clone(),
                worker,
                contract,
                capture_dir,
                worker_index,
                slot_id,
                require_fresh_aggregate,
                invocation,
                initial_input,
                &context,
                &mut records,
            );
            candidates.extend(assignment_candidates);
        }
    }

    Ok(ReviewCandidatesDocument {
        schema_version: SCHEMA_VERSION,
        candidates,
        records,
    })
}

/// Read stdin, project it, and write one JSON document.  This is the provider
/// command entry point; it performs no catalog or Loop Engine access.
pub fn run_from_stdin() -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("review-candidates could not read stdin: {error}");
        return 2;
    }
    let input = match serde_json::from_str::<Value>(&input) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("review-candidates input is malformed JSON: {error}");
            return 2;
        }
    };
    let document = match project(&input) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("review-candidates input is not an ordinary completed show: {error}");
            return 2;
        }
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match serde_json::to_writer(&mut stdout, &document) {
        Ok(()) => match stdout.flush() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("review-candidates could not flush output: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("review-candidates could not serialize output: {error}");
            1
        }
    }
}

fn show_result(input: &Value) -> Result<&Map<String, Value>, ProjectionError> {
    let envelope = input
        .as_object()
        .ok_or_else(|| ProjectionError::new("input must be a JSON object"))?;
    if envelope.get("operation").and_then(Value::as_str) != Some("show") {
        return Err(ProjectionError::new("input operation must be `show`"));
    }
    if envelope.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(ProjectionError::new(
            "input show envelope status must be `completed`",
        ));
    }
    let result = envelope
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ProjectionError::new("completed show envelope is missing object `result`")
        })?;
    match result.get("workflow_id").and_then(Value::as_str) {
        Some(WORKFLOW_ID) => Ok(result),
        Some(_) => Err(ProjectionError::new(
            "show result workflow_id must be `software-change`",
        )),
        None => Err(ProjectionError::new(
            "show result is missing string `workflow_id`",
        )),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "assignment projection independently checks captured worker, contract, invocation and run context"
)]
fn project_assignment(
    origin: CandidateOrigin,
    worker: &Map<String, Value>,
    contract: &Value,
    capture_dir: Option<&str>,
    worker_index: usize,
    gate: &str,
    require_fresh_aggregate: bool,
    invocation: &Map<String, Value>,
    initial_input: &Map<String, Value>,
    context: &[loop_core::ContextRecord],
    records: &mut Vec<ReviewRecordPreview>,
) -> Vec<ReviewCandidate> {
    let recovered = worker
        .get("recovery_source")
        .is_some_and(|value| !value.is_null());
    if worker.get("selected_attempt").is_none_or(Value::is_null) && !recovered {
        return vec![if reports_exhausted(capture_dir, worker_index) {
            ReviewCandidate::Exhausted {
                origin,
                diagnostic:
                    "review output exhausted conformance attempts without a selected attempt".into(),
            }
        } else {
            ReviewCandidate::MissingSelection {
                origin,
                diagnostic: "assignment has no selected review output".into(),
            }
        }];
    }
    let bytes = (|| -> Result<Vec<u8>, String> {
        if recovered {
            if worker
                .get("selected_attempt")
                .is_some_and(|value| !value.is_null())
            {
                return Err("recovered output must not claim a new selected attempt".into());
            }
        } else if !worker["selected_attempt"]
            .as_u64()
            .is_some_and(|n| n > 0 && u32::try_from(n).is_ok())
        {
            return Err("selected output metadata is unavailable".into());
        }
        let digest = worker["selected_output_sha256"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("selected output metadata is unavailable")?;
        let path = worker["selected_output_path"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("selected output metadata is unavailable")?;
        let bytes = read_selected_output(capture_dir, path)
            .map_err(|_| "selected review output is unavailable")?;
        if sha256_digest(&bytes) != digest {
            return Err("selected review output digest does not match recorded digest".into());
        }
        Ok(bytes)
    })();
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(diagnostic) => return vec![ReviewCandidate::Unavailable { origin, diagnostic }],
    };
    if recovered {
        let Some(source) = worker.get("recovery_source") else {
            return vec![ReviewCandidate::Malformed {
                origin,
                diagnostic: "recovered output has no source metadata".into(),
            }];
        };
        if let Err(diagnostic) = crate::evidence::verify_recovery_source(
            source,
            Path::new(capture_dir.unwrap_or_default()),
            worker["selected_output_path"].as_str().unwrap_or_default(),
            worker["selected_output_sha256"]
                .as_str()
                .unwrap_or_default(),
        ) {
            return vec![ReviewCandidate::Malformed { origin, diagnostic }];
        }
        let binding = match invocation
            .get("binding")
            .cloned()
            .ok_or_else(|| "recovered assignment has no frozen binding".to_owned())
            .and_then(|value| {
                serde_json::from_value::<loop_core::WorkSlotBinding>(value)
                    .map_err(|error| error.to_string())
            }) {
            Ok(binding) => binding,
            Err(diagnostic) => return vec![ReviewCandidate::Malformed { origin, diagnostic }],
        };
        if review_subject_for_gate(gate).is_none() {
            return vec![ReviewCandidate::Malformed {
                origin,
                diagnostic: "recovered assignment has an unknown review gate".into(),
            }];
        }
        let Some(subject) = invocation.get("subject").and_then(Value::as_str) else {
            return vec![ReviewCandidate::Malformed {
                origin,
                diagnostic: "recovered assignment has no current visit subject".into(),
            }];
        };
        if let Err(diagnostic) = crate::evidence::verify_recovery_source_identity(
            source,
            &origin.assignment_id,
            gate,
            Some(subject),
            &binding,
        ) {
            return vec![ReviewCandidate::Malformed { origin, diagnostic }];
        }
    }
    let value = match parse_selected_value(&bytes) {
        Ok(value) => value,
        Err(diagnostic) => return vec![ReviewCandidate::Malformed { origin, diagnostic }],
    };
    if contract.pointer("/properties/judgments").is_some() {
        return project_batch(
            origin,
            contract,
            &value,
            capture_dir,
            gate,
            require_fresh_aggregate,
            worker,
            invocation,
            initial_input,
            context,
            records,
        );
    }
    let derived_contract;
    let candidate_contract = if recovered {
        match derived_legacy_review_schema(contract, gate, initial_input) {
            Some(schema) => {
                derived_contract = schema;
                &derived_contract
            }
            None => {
                return vec![ReviewCandidate::Malformed {
                    origin,
                    diagnostic:
                        "derived legacy output does not contain the frozen review judgment fields"
                            .into(),
                }]
            }
        }
    } else {
        contract
    };
    match normalize_review_output(candidate_contract, &value) {
        Ok(judgment) => {
            if judgment.review_contract_version == Some(2) {
                let Some(root) = initial_input.get("artifact_root").and_then(Value::as_str) else {
                    return vec![ReviewCandidate::Malformed {
                        origin,
                        diagnostic:
                            "version-2 review candidate has no artifact_root for grounding checks"
                                .into(),
                    }];
                };
                if let Err(diagnostic) = crate::criterion::validate_grounding(
                    judgment.grounds.as_ref().unwrap_or(&Value::Null),
                    Path::new(root),
                ) {
                    return vec![ReviewCandidate::Malformed { origin, diagnostic }];
                }
            }
            push_review_record(
                records,
                origin.clone(),
                gate,
                initial_input,
                context,
                worker,
                invocation,
                &judgment.review_stage,
                &judgment.axis,
                &judgment.author,
                &judgment.result,
                &judgment.findings,
                judgment.review_contract_version,
                judgment.grounds.as_ref(),
            );
            vec![ReviewCandidate::Ready {
                origin,
                review_stage: judgment.review_stage,
                axis: judgment.axis,
                author: judgment.author,
                result: judgment.result,
                findings: judgment.findings,
                review_contract_version: judgment.review_contract_version,
                grounds: judgment.grounds,
            }]
        }
        Err(diagnostic) => vec![ReviewCandidate::Malformed { origin, diagnostic }],
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "batch projection retains independent captured contract, worker, invocation and context checks"
)]
fn project_batch(
    origin: CandidateOrigin,
    contract: &Value,
    value: &Value,
    capture_dir: Option<&str>,
    gate: &str,
    require_fresh_aggregate: bool,
    worker: &Map<String, Value>,
    invocation: &Map<String, Value>,
    initial_input: &Map<String, Value>,
    context: &[loop_core::ContextRecord],
    records_out: &mut Vec<ReviewRecordPreview>,
) -> Vec<ReviewCandidate> {
    let result = (|| -> Result<Vec<ReviewCandidate>, String> {
        let capture = capture_dir.ok_or("missing capture directory")?;
        let (captured_schema, location) =
            crate::review_batch::captured_commission(Path::new(capture), &origin.assignment_id)?;
        if captured_schema != *contract {
            return Err("captured contract disagrees with selected assignment".into());
        }
        let subject = match gate {
            "intent-review" | "intent-adversarial-review" => "intent.json",
            "design-review" | "design-adversarial-review" => "design.json",
            "plan-review" | "plan-adversarial-review" => "plan.json",
            "implementation-review" | "implementation-adversarial-review" => {
                "implementation-report.json"
            }
            "validation-review" | "validation-adversarial-review" => "validation-report.json",
            _ => return Err("unknown review gate".into()),
        };
        let root = location["artifact_root"]
            .as_str()
            .ok_or("missing artifact root")?;
        let config_version = initial_input
            .get("config_version")
            .and_then(Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .ok_or("show initial_input has no config_version")?;
        let target: Value = serde_json::from_slice(
            &fs::read(Path::new(root).join(subject)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let revision = target["revision"]
            .as_str()
            .ok_or("subject has no revision")?;
        let review_stage = declared_review_stage(contract, value)?;
        let review_contract_version = contract
            .pointer("/properties/review_contract_version/const")
            .and_then(Value::as_u64)
            .and_then(|version| u32::try_from(version).ok());
        let rows = crate::review_batch::rows_for_stage_with_options(
            contract,
            value,
            &location,
            gate,
            subject,
            revision,
            &review_stage,
            review_stage == "individual",
            require_fresh_aggregate,
        )?;
        let author = CandidateAuthor {
            name: value["author"]["name"]
                .as_str()
                .ok_or("missing author name")?
                .into(),
            kind: value["author"]["kind"]
                .as_str()
                .ok_or("missing author kind")?
                .into(),
        };
        let target_checkpoint = crate::checkpoint::current_target(subject, Path::new(root))?;
        let mut candidates = Vec::new();
        for row in rows {
            let axis = row["axis"].as_str().ok_or("missing axis")?.to_owned();
            if let Some(reuse) = row.get("reuse") {
                let applicability_id = reuse.as_str().ok_or("missing applicability ID")?;
                match crate::review_batch::validate_reuse_row(
                    row,
                    &location,
                    gate,
                    &axis,
                    &value["author"],
                    config_version,
                    subject,
                    revision,
                    &review_stage,
                    review_stage == "individual",
                    require_fresh_aggregate,
                ) {
                    Ok(()) => candidates.push(ReviewCandidate::Carried {
                        origin: origin.clone(),
                        review_stage: review_stage.clone(),
                        axis: axis.clone(),
                        author: author.clone(),
                        review_contract_version,
                        applicability_id: applicability_id.into(),
                    }),
                    Err(diagnostic) => {
                        candidates.push(ReviewCandidate::Malformed {
                            origin: origin.clone(),
                            diagnostic: format!("reuse row `{axis}` is invalid: {diagnostic}"),
                        });
                        records_out.push(ReviewRecordPreview {
                            status: "invalid-reuse".into(),
                            record_id: None,
                            kind: None,
                            gate: gate.into(),
                            subject: subject.into(),
                            subject_revision: revision.into(),
                            config_version: config_version.into(),
                            review_stage: review_stage.clone(),
                            origin: origin.clone(),
                            axis: Some(axis),
                            applicability_id: Some(applicability_id.into()),
                            data: None,
                            target_checkpoint: target_checkpoint.clone(),
                            diagnostic: Some(diagnostic),
                        });
                    }
                }
                continue;
            }

            let grounds = row.get("grounds").cloned();
            if review_contract_version == Some(2) {
                if let Err(diagnostic) = crate::criterion::validate_grounding(
                    grounds
                        .as_ref()
                        .ok_or("version-2 review row is missing grounds")?,
                    Path::new(root),
                ) {
                    candidates.push(ReviewCandidate::Malformed {
                        origin: origin.clone(),
                        diagnostic: format!("row `{axis}` has invalid grounds: {diagnostic}"),
                    });
                    continue;
                }
            }
            let result = row["result"].as_str().ok_or("missing result")?.to_owned();
            let findings = row["findings"]
                .as_str()
                .ok_or("missing findings")?
                .to_owned();
            push_review_record(
                records_out,
                origin.clone(),
                gate,
                initial_input,
                context,
                worker,
                invocation,
                &review_stage,
                &axis,
                &author,
                &result,
                &findings,
                review_contract_version,
                grounds.as_ref(),
            );
            candidates.push(ReviewCandidate::Ready {
                origin: origin.clone(),
                review_stage: review_stage.clone(),
                axis,
                author: author.clone(),
                result,
                findings,
                review_contract_version,
                grounds,
            });
        }
        if let Some(verdicts) = value.get("validation_verdicts") {
            if gate != "validation-review" || review_stage != "aggregate" {
                return Err(
                    "only ordinary validation commissions produce criterion/goal verdicts".into(),
                );
            }
            let mut seen = std::collections::BTreeSet::new();
            let delivered_context: Vec<loop_core::ContextRecord> = serde_json::from_value(
                location
                    .get("context")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .map_err(|e| format!("invalid captured validation context: {e}"))?;
            let config_version = initial_input
                .get("config_version")
                .and_then(Value::as_str)
                .ok_or("show initial_input has no config_version")?;
            for row in verdicts.as_array().ok_or("invalid validation_verdicts")? {
                let id = row["record_id"]
                    .as_str()
                    .ok_or("missing verdict record_id")?;
                let kind = row["kind"].as_str().ok_or("missing verdict kind")?;
                let data = &row["data"];
                let selected = if kind == "goal-verdict" {
                    target["goal_verdict_ids"]
                        .as_array()
                        .is_some_and(|ids| ids.contains(&Value::String(id.into())))
                } else if kind == "criterion-verdict" {
                    target["criteria"].as_array().is_some_and(|rows| {
                        rows.iter().any(|r| {
                            r["criterion_id"] == data["criterion_id"]
                                && r["verdict_ids"]
                                    .as_array()
                                    .is_some_and(|ids| ids.contains(&Value::String(id.into())))
                        })
                    })
                } else {
                    false
                };
                if !selected
                    || !seen.insert(id)
                    || data["author"] != value["author"]
                    || data["subject_revision"] != revision
                {
                    return Err(
                        "unassigned, duplicate, wrong-author or stale criterion verdict".into(),
                    );
                }
                if review_contract_version == Some(2)
                    && data
                        .get("reason")
                        .and_then(Value::as_str)
                        .is_none_or(|reason| reason.trim().is_empty() || reason.len() > 1200)
                {
                    return Err(
                        "version-2 criterion/goal verdict needs a concise reason of at most 1200 bytes".to_owned()
                    );
                }
                let mut candidate_data = data.clone();
                candidate_data["gate"] = Value::String(gate.into());
                candidate_data["config_version"] = Value::String(config_version.into());
                candidate_data["review_stage"] = Value::String(review_stage.clone());
                candidate_data["origin"] =
                    serde_json::to_value(&origin).map_err(|error| error.to_string())?;
                let mut source_check_data = candidate_data.clone();
                source_check_data[loop_core::ENGINE_ORIGIN_KEY] =
                    expected_engine_origin(worker, invocation, &origin);
                crate::validation::source_for_candidate(
                    &source_check_data,
                    kind,
                    &delivered_context,
                    Path::new(root),
                    gate,
                    config_version,
                )
                .map_err(|error| format!("invalid retained validation verdict: {error}"))?;
                records_out.push(preview_record(
                    context,
                    id,
                    kind,
                    gate,
                    "validation-report.json",
                    revision,
                    config_version,
                    &review_stage,
                    origin.clone(),
                    data.get("criterion_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    candidate_data.clone(),
                    expected_engine_origin(worker, invocation, &origin),
                    target_checkpoint.clone(),
                ));
                candidates.push(ReviewCandidate::VerdictReady {
                    origin: origin.clone(),
                    review_contract_version,
                    record_id: id.into(),
                    kind: kind.into(),
                    data: candidate_data,
                });
            }
        }
        Ok(candidates)
    })();
    result.unwrap_or_else(|diagnostic| vec![ReviewCandidate::Malformed { origin, diagnostic }])
}

#[allow(clippy::too_many_arguments)]
fn push_review_record(
    records: &mut Vec<ReviewRecordPreview>,
    origin: CandidateOrigin,
    gate: &str,
    initial_input: &Map<String, Value>,
    context: &[loop_core::ContextRecord],
    worker: &Map<String, Value>,
    invocation: &Map<String, Value>,
    review_stage: &str,
    axis: &str,
    author: &CandidateAuthor,
    result: &str,
    findings: &str,
    review_contract_version: Option<u32>,
    grounds: Option<&Value>,
) {
    let Some(root) = initial_input.get("artifact_root").and_then(Value::as_str) else {
        return;
    };
    let Some(config_version) = initial_input.get("config_version").and_then(Value::as_str) else {
        return;
    };
    let Some(subject) = review_subject_for_gate(gate) else {
        return;
    };
    let target = (|| -> Result<(String, Option<Value>), String> {
        let bytes = fs::read(Path::new(root).join(subject)).map_err(|error| error.to_string())?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        let revision = value["revision"]
            .as_str()
            .ok_or("current review target has no revision")?
            .to_owned();
        let checkpoint = crate::checkpoint::current_target(subject, Path::new(root))?;
        Ok((revision, checkpoint))
    })();
    let (subject_revision, target_checkpoint, target_diagnostic) = match target {
        Ok((revision, checkpoint)) => (revision, checkpoint, None),
        Err(diagnostic) => (String::new(), None, Some(diagnostic)),
    };
    let record_id = review_record_id(&origin, axis);
    let mut data = serde_json::json!({
        "gate": gate,
        "policy_id": axis,
        "review_stage": review_stage,
        "result": result,
        "findings": findings,
        "author": {"name": author.name, "kind": author.kind},
        "subject": subject,
        "subject_revision": subject_revision,
        "config_version": config_version,
        "origin": origin.clone(),
    });
    if let Some(version) = review_contract_version {
        data["review_contract_version"] = serde_json::json!(version);
    }
    if let Some(grounds) = grounds {
        data["grounds"] = grounds.clone();
    }
    let expected_origin = expected_engine_origin(worker, invocation, &origin);
    let mut preview = preview_record(
        context,
        &record_id,
        "review-evidence",
        gate,
        subject,
        &subject_revision,
        config_version,
        review_stage,
        origin,
        Some(axis.to_owned()),
        data,
        expected_origin,
        target_checkpoint,
    );
    if let Some(diagnostic) = target_diagnostic {
        preview.status = "unavailable".into();
        preview.diagnostic = Some(diagnostic);
    }
    records.push(preview);
}

fn review_record_id(origin: &CandidateOrigin, axis: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(origin.id.as_bytes());
    hash.update([0]);
    hash.update(origin.assignment_id.as_bytes());
    hash.update([0]);
    hash.update(axis.as_bytes());
    format!("review-evidence-{:x}", hash.finalize())
}

fn review_subject_for_gate(gate: &str) -> Option<&'static str> {
    match gate {
        "intent-review" | "intent-adversarial-review" => Some("intent.json"),
        "design-review" | "design-adversarial-review" => Some("design.json"),
        "plan-review" | "plan-adversarial-review" => Some("plan.json"),
        "implementation-review" | "implementation-adversarial-review" => {
            Some("implementation-report.json")
        }
        "validation-review" | "validation-adversarial-review" => Some("validation-report.json"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn preview_record(
    context: &[loop_core::ContextRecord],
    record_id: &str,
    kind: &str,
    gate: &str,
    subject: &str,
    subject_revision: &str,
    config_version: &str,
    review_stage: &str,
    origin: CandidateOrigin,
    axis: Option<String>,
    data: Value,
    expected_engine_origin: Value,
    target_checkpoint: Option<Value>,
) -> ReviewRecordPreview {
    let existing: Vec<_> = context
        .iter()
        .filter(|record| record.id.as_str() == record_id)
        .collect();
    let (status, diagnostic) = match existing.as_slice() {
        [] => ("ready", None),
        [existing] => {
            let mut existing_data = existing.data.clone();
            let engine_origin = existing_data
                .as_object_mut()
                .and_then(|object| object.remove(loop_core::ENGINE_ORIGIN_KEY));
            if existing.kind == kind
                && existing_data == data
                && engine_origin.as_ref() == Some(&expected_engine_origin)
            {
                ("already-applied", None)
            } else {
                (
                    "conflict",
                    Some("record ID exists with different bytes, source, or target".into()),
                )
            }
        }
        _ => (
            "conflict",
            Some("record ID is ambiguous in current context".into()),
        ),
    };
    ReviewRecordPreview {
        status: status.to_owned(),
        record_id: Some(record_id.to_owned()),
        kind: Some(kind.to_owned()),
        gate: gate.to_owned(),
        subject: subject.to_owned(),
        subject_revision: subject_revision.to_owned(),
        config_version: config_version.to_owned(),
        review_stage: review_stage.to_owned(),
        origin,
        axis,
        applicability_id: None,
        data: Some(data),
        target_checkpoint,
        diagnostic,
    }
}

fn expected_engine_origin(
    worker: &Map<String, Value>,
    invocation: &Map<String, Value>,
    origin: &CandidateOrigin,
) -> Value {
    let mut expected = serde_json::json!({
        "invocation_id": origin.id,
        "assignment_id": origin.assignment_id,
        "selected_output_sha256": worker.get("selected_output_sha256").cloned().unwrap_or(Value::Null),
        "selected_output_path": worker.get("selected_output_path").cloned().unwrap_or(Value::Null),
        "capture_dir": invocation.get("capture_dir").cloned().unwrap_or(Value::Null),
        "command": worker.get("command").cloned().unwrap_or(Value::Null),
        "args": worker.get("args").cloned().unwrap_or_else(|| serde_json::json!([])),
        "binding": invocation.get("binding").cloned().unwrap_or(Value::Null),
        "slot_id": invocation.get("slot_id").cloned().unwrap_or(Value::Null),
    });
    if let Some(attempt) = worker
        .get("selected_attempt")
        .filter(|value| !value.is_null())
    {
        expected["selected_attempt"] = attempt.clone();
    }
    if let Some(source) = worker
        .get("recovery_source")
        .filter(|value| !value.is_null())
    {
        expected["recovery_source"] = source.clone();
    }
    expected
}

fn reports_exhausted(capture_dir: Option<&str>, worker_index: usize) -> bool {
    let Some(capture_dir) = capture_dir.filter(|value| !value.is_empty()) else {
        return false;
    };
    let Some(capture) = absolute_path(Path::new(capture_dir)) else {
        return false;
    };
    let Ok(capture) = fs::canonicalize(capture) else {
        return false;
    };
    if !capture.is_dir() {
        return false;
    }
    let manifest_path = capture.join(worker_index.to_string()).join(ATTEMPTS_FILE);
    let Ok(manifest_path) = fs::canonicalize(manifest_path) else {
        return false;
    };
    if manifest_path == capture || !manifest_path.starts_with(&capture) || !manifest_path.is_file()
    {
        return false;
    }
    let Ok(bytes) = fs::read(manifest_path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    manifest.as_object().is_some_and(|object| {
        object.get("schema_version").and_then(Value::as_str) == Some(ATTEMPTS_SCHEMA_VERSION)
            && object.get("exhausted").and_then(Value::as_bool) == Some(true)
            && object.get("selected_attempt").is_some_and(Value::is_null)
            && object.get("attempts").and_then(Value::as_array).is_some()
    })
}

fn read_selected_output(capture_dir: Option<&str>, selected_path: &str) -> Result<Vec<u8>, ()> {
    let capture_dir = capture_dir.filter(|value| !value.is_empty()).ok_or(())?;
    let capture = absolute_path(Path::new(capture_dir)).ok_or(())?;
    let capture = fs::canonicalize(capture).map_err(|_| ())?;
    if !capture.is_dir() {
        return Err(());
    }

    let selected = PathBuf::from(selected_path);
    let selected = if selected.is_absolute() {
        selected
    } else {
        capture.join(selected)
    };
    let selected = fs::canonicalize(selected).map_err(|_| ())?;
    if selected == capture || !selected.starts_with(&capture) || !selected.is_file() {
        return Err(());
    }
    fs::read(selected).map_err(|_| ())
}

fn absolute_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return Some(path.to_owned());
    }
    std::env::current_dir().ok().map(|cwd| cwd.join(path))
}

fn parse_selected_value(bytes: &[u8]) -> Result<Value, String> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => Ok(value),
        Err(_) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| "selected output is not valid UTF-8 JSON".to_owned())?;
            let lines = text.lines().collect::<Vec<_>>();
            let openings = lines
                .iter()
                .enumerate()
                .filter_map(|(index, line)| (line.trim() == "```json").then_some(index))
                .collect::<Vec<_>>();
            let opening = match openings.as_slice() {
                [opening] => *opening,
                [] => return Err("selected output is not JSON".to_owned()),
                _ => return Err("selected output contains multiple JSON fenced blocks".to_owned()),
            };
            let closing = lines
                .iter()
                .enumerate()
                .skip(opening + 1)
                .find_map(|(index, line)| (line.trim() == "```").then_some(index))
                .ok_or_else(|| "selected output has an unterminated JSON fence".to_owned())?;
            let raw = lines[opening + 1..closing].join("\n");
            serde_json::from_str(&raw)
                .map_err(|_| "selected output contains invalid fenced JSON".to_owned())
        }
    }
}

fn declared_review_stage(contract: &Value, value: &Value) -> Result<String, String> {
    let schema_stage = contract
        .pointer("/properties/review_stage/const")
        .and_then(Value::as_str);
    let value_stage = value.get("review_stage").and_then(Value::as_str);
    if schema_stage.is_some() && value_stage.is_none() {
        return Err("review output is missing frozen review_stage".to_owned());
    }
    if let (Some(schema_stage), Some(value_stage)) = (schema_stage, value_stage) {
        if schema_stage != value_stage {
            return Err("review output review_stage disagrees with its frozen contract".to_owned());
        }
    }
    let stage = schema_stage.or(value_stage).unwrap_or("aggregate");
    if !matches!(stage, "individual" | "aggregate") {
        return Err("review output review_stage must be `individual` or `aggregate`".to_owned());
    }
    Ok(stage.to_owned())
}

/// P05's captured required-key output contract cannot itself express a full
/// review schema. A derived selection is eligible only when that frozen
/// contract required every v2 judgment field and the run's single-stage gate
/// supplies the axis and stage domain.
fn derived_legacy_review_schema(
    contract: &Value,
    gate: &str,
    initial_input: &Map<String, Value>,
) -> Option<Value> {
    let required = contract.get("required")?.as_array()?;
    let required = required
        .iter()
        .filter_map(Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if ![
        "review_contract_version",
        "review_stage",
        "author",
        "axis",
        "result",
        "findings",
        "grounds",
    ]
    .iter()
    .all(|field| required.contains(field))
    {
        return None;
    }
    let policies = initial_input
        .get("review_policies")?
        .get(gate)?
        .as_array()?;
    let axes = policies
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    let stages = policies
        .iter()
        .filter_map(|entry| {
            entry
                .get("review_stage")
                .or_else(|| entry.get("stage"))
                .and_then(Value::as_str)
        })
        .collect::<std::collections::BTreeSet<_>>();
    if axes.is_empty() || stages.len() != 1 {
        return None;
    }
    let stage = *stages.iter().next()?;
    if !matches!(stage, "individual" | "aggregate") {
        return None;
    }
    Some(serde_json::json!({
        "type":"object","additionalProperties":false,
        "required":["review_contract_version","review_stage","author","axis","result","findings","grounds"],
        "properties":{
            "review_contract_version":{"type":"integer","const":2},
            "review_stage":{"type":"string","const":stage},
            "author":{"type":"object","additionalProperties":false,"required":["name","kind"],
                "properties":{"name":{"type":"string","minLength":1},"kind":{"type":"string","enum":["human","agent","script"]}}},
            "axis":{"type":"string","enum":axes.into_iter().collect::<Vec<_>>()},
            "result":{"type":"string","enum":["pass","fail"]},
            "findings":{"type":"string"},
            "grounds":{"type":"object","additionalProperties":false,"required":["reason","evidence"],
                "properties":{"reason":{"type":"string","minLength":1,"maxLength":1200},
                    "evidence":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"object","additionalProperties":false,
                        "required":["locator","sha256"],"properties":{"locator":{"type":"string","minLength":1,"maxLength":1024},
                            "sha256":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}}}}}}
        },
        "oneOf":[
            {"properties":{"result":{"const":"pass"},"findings":{"const":""}}},
            {"properties":{"result":{"const":"fail"},"findings":{"type":"string","minLength":1}}}
        ]
    }))
}

fn looks_like_review_contract(contract: &Value) -> bool {
    let Some(object) = contract.as_object() else {
        return false;
    };
    // Legacy required-key presence contracts are deliberately not eligible:
    // this projection normalizes only a declared full review schema.
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        return false;
    };
    (properties.contains_key("author") && properties.contains_key("judgments"))
        || REVIEW_FIELDS
            .iter()
            .all(|field| properties.contains_key(*field))
}

struct NormalizedJudgment {
    review_stage: String,
    axis: String,
    author: CandidateAuthor,
    result: String,
    findings: String,
    review_contract_version: Option<u32>,
    grounds: Option<Value>,
}

fn normalize_review_output(contract: &Value, value: &Value) -> Result<NormalizedJudgment, String> {
    let mut violations = Vec::new();
    validate_contract_instance(contract, value, "$", &mut violations);

    let Some(object) = value.as_object() else {
        violations.push("review output must be a JSON object".to_owned());
        return Err(malformed_diagnostic(&violations));
    };
    let review_contract_version = contract
        .pointer("/properties/review_contract_version/const")
        .and_then(Value::as_u64)
        .or_else(|| {
            object
                .get("review_contract_version")
                .and_then(Value::as_u64)
        })
        .and_then(|version| u32::try_from(version).ok());
    if object
        .get("review_contract_version")
        .and_then(Value::as_u64)
        .is_some_and(|version| version != 2)
    {
        violations.push("review_contract_version must be 2 when present".to_owned());
    }
    let grounds = object.get("grounds").cloned();
    if review_contract_version == Some(2) && grounds.is_none() {
        violations.push("version-2 review judgment is missing grounds".to_owned());
    }
    let review_stage = match contract
        .pointer("/properties/review_stage/const")
        .and_then(Value::as_str)
        .or_else(|| object.get("review_stage").and_then(Value::as_str))
    {
        Some(stage) if matches!(stage, "individual" | "aggregate") => Some(stage.to_owned()),
        Some(_) => {
            violations
                .push("review output review_stage must be `individual` or `aggregate`".to_owned());
            None
        }
        None => Some("aggregate".to_owned()),
    };
    let axis = match object.get("axis").and_then(Value::as_str) {
        Some(axis) if !axis.is_empty() => Some(axis.to_owned()),
        _ => {
            violations.push("review output axis must be a non-empty string".to_owned());
            None
        }
    };
    let author = match object.get("author").and_then(Value::as_object) {
        Some(author) => {
            let name = author.get("name").and_then(Value::as_str);
            let kind = author.get("kind").and_then(Value::as_str);
            if name.is_none_or(str::is_empty) {
                violations.push("review output author name must be a non-empty string".to_owned());
            }
            if !kind.is_some_and(|kind| AUTHOR_KINDS.contains(&kind)) {
                violations
                    .push("review output author kind must be human, agent, or script".to_owned());
            }
            match (
                name.filter(|name| !name.is_empty()),
                kind.filter(|kind| AUTHOR_KINDS.contains(kind)),
            ) {
                (Some(name), Some(kind)) => Some(CandidateAuthor {
                    name: name.to_owned(),
                    kind: kind.to_owned(),
                }),
                _ => None,
            }
        }
        None => {
            violations.push("review output author must be an object".to_owned());
            None
        }
    };
    let result = match object.get("result").and_then(Value::as_str) {
        Some(result) if result == "pass" || result == "fail" => Some(result.to_owned()),
        _ => {
            violations.push("review output result must be `pass` or `fail`".to_owned());
            None
        }
    };
    let findings = match object.get("findings").and_then(Value::as_str) {
        Some(findings) => Some(findings.to_owned()),
        None => {
            violations.push("review output findings must be a string".to_owned());
            None
        }
    };
    if let (Some(result), Some(findings)) = (result.as_deref(), findings.as_deref()) {
        match result {
            "pass" if !findings.is_empty() => {
                violations.push("review output pass findings must be the empty string".to_owned())
            }
            "fail" if findings.is_empty() => {
                violations.push("review output fail findings must be non-empty".to_owned())
            }
            _ => {}
        }
    }

    if violations.is_empty() {
        Ok(NormalizedJudgment {
            review_stage: review_stage.expect("validated review stage"),
            axis: axis.expect("validated axis"),
            author: author.expect("validated author"),
            result: result.expect("validated result"),
            findings: findings.expect("validated findings"),
            review_contract_version,
            grounds,
        })
    } else {
        Err(malformed_diagnostic(&violations))
    }
}

fn malformed_diagnostic(violations: &[String]) -> String {
    if violations.is_empty() {
        "selected output does not satisfy the frozen review contract".to_owned()
    } else {
        format!(
            "selected output does not satisfy the frozen review contract: {}",
            violations.join("; ")
        )
    }
}

/// Validate the small JSON Schema subset used by the frozen full review
/// output contract.  The provider's general artifact schema validator is not
/// used here because this contract additionally carries assignment-specific
/// JSON `const` values and `oneOf` pass/fail rules.
fn validate_contract_instance(
    schema: &Value,
    instance: &Value,
    path: &str,
    violations: &mut Vec<String>,
) {
    let Some(schema) = schema.as_object() else {
        violations.push(format!("malformed declared review contract at {path}"));
        return;
    };

    if let Some(expected) = schema.get("const") {
        if instance != expected {
            violations.push(format!(
                "review output differs from declared constant at {path}"
            ));
        }
    }

    if let Some(type_value) = schema.get("type") {
        let Some(type_name) = type_value.as_str() else {
            violations.push(format!("malformed declared review contract type at {path}"));
            return;
        };
        if !matches_json_type(type_name, instance) {
            violations.push(format!(
                "review output has the wrong type at {path}; expected `{type_name}`"
            ));
            return;
        }
    }

    if let Some(required) = schema.get("required") {
        let Some(required) = required.as_array() else {
            violations.push(format!(
                "malformed declared review contract required at {path}"
            ));
            return;
        };
        let Some(object) = instance.as_object() else {
            return;
        };
        for name in required {
            let Some(name) = name.as_str() else {
                violations.push(format!(
                    "malformed declared review contract required entry at {path}"
                ));
                continue;
            };
            if !object.contains_key(name) {
                violations.push(format!("review output is missing `{name}`"));
            }
        }
    }

    if let Some(properties) = schema.get("properties") {
        let Some(properties) = properties.as_object() else {
            violations.push(format!(
                "malformed declared review contract properties at {path}"
            ));
            return;
        };
        if let Some(object) = instance.as_object() {
            for (name, property_schema) in properties {
                if let Some(property) = object.get(name) {
                    validate_contract_instance(
                        property_schema,
                        property,
                        &format!("{path}.{name}"),
                        violations,
                    );
                }
            }
            if schema
                .get("additionalProperties")
                .is_some_and(|value| value == &Value::Bool(false))
            {
                for name in object.keys().filter(|name| !properties.contains_key(*name)) {
                    violations.push(format!("review output has unexpected `{name}`"));
                }
            }
        }
    } else if schema
        .get("additionalProperties")
        .is_some_and(|value| value == &Value::Bool(false))
        && instance.is_object()
    {
        violations.push(format!(
            "malformed declared review contract has closed fields but no properties at {path}"
        ));
    }

    if let Some(additional) = schema.get("additionalProperties") {
        if !additional.is_boolean() {
            violations.push(format!(
                "malformed declared review contract additionalProperties at {path}"
            ));
        }
    }

    if let Some(min_length) = schema.get("minLength") {
        let Some(min_length) = min_length.as_u64() else {
            violations.push(format!(
                "malformed declared review contract minLength at {path}"
            ));
            return;
        };
        if let Some(string) = instance.as_str() {
            if string.chars().count() < min_length as usize {
                violations.push(format!("review output string is too short at {path}"));
            }
        }
    }

    if let Some(enum_values) = schema.get("enum") {
        let Some(enum_values) = enum_values.as_array() else {
            violations.push(format!("malformed declared review contract enum at {path}"));
            return;
        };
        if !enum_values.iter().any(|expected| expected == instance) {
            violations.push(format!(
                "review output is outside the declared enum at {path}"
            ));
        }
    }

    if let Some(one_of) = schema.get("oneOf") {
        let Some(one_of) = one_of.as_array() else {
            violations.push(format!(
                "malformed declared review contract oneOf at {path}"
            ));
            return;
        };
        let matching = one_of
            .iter()
            .filter(|branch| {
                let mut branch_violations = Vec::new();
                validate_contract_instance(branch, instance, path, &mut branch_violations);
                branch_violations.is_empty()
            })
            .count();
        if matching != 1 {
            violations.push(format!(
                "review output matches {matching} declared oneOf branches at {path}"
            ));
        }
    }
}

fn matches_json_type(type_name: &str, value: &Value) -> bool {
    match type_name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempCapture {
        path: PathBuf,
    }

    impl TempCapture {
        fn new(label: &str) -> Self {
            let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "software-change-review-candidates-{label}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create capture");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempCapture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn contract(axis: &str, author: &str) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["axis", "author", "result", "findings"],
            "properties": {
                "axis": {"type": "string", "minLength": 1, "const": axis},
                "author": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "kind"],
                    "properties": {
                        "name": {"type": "string", "minLength": 1},
                        "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                    },
                    "const": {"name": author, "kind": "agent"}
                },
                "result": {"type": "string", "enum": ["pass", "fail"]},
                "findings": {"type": "string"},
            },
            "oneOf": [
                {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
                {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}}
            ]
        })
    }

    fn worker(
        assignment_id: &str,
        contract: Value,
        selected_attempt: Option<u32>,
        selected_path: Option<&Path>,
        selected_digest: Option<String>,
    ) -> Value {
        json!({
            "assignment_id": assignment_id,
            "command": "/bin/worker",
            "args": [],
            "exit_code": 0,
            "selected_attempt": selected_attempt,
            "selected_output_sha256": selected_digest,
            "selected_output_path": selected_path.map(|path| path.to_string_lossy().to_string()),
            "declared_output_contract": contract,
        })
    }

    fn envelope(capture_dir: &Path, workers: Vec<Value>) -> Value {
        json!({
            "operation": "show",
            "status": "completed",
            "result": {
                "workflow_id": "software-change",
                "initial_input": {"review_policies": {"design-review": [{"id": "axis"}]}},
                "work_slot_invocations": [{
                    "invocation_id": "invocation-1",
                    "slot_id": "design-review",
                    "status": "succeeded",
                    "capture_dir": capture_dir.to_string_lossy(),
                    "inner_workers": workers,
                }],
            }
        })
    }

    #[test]
    fn selected_retry_output_is_ready_and_serialization_is_stable() {
        let capture = TempCapture::new("retry");
        let selected = capture.path().join("0/attempts/2/stdout");
        fs::create_dir_all(capture.path().join("0/attempts/1")).expect("first attempt dir");
        fs::create_dir_all(selected.parent().expect("attempt directory"))
            .expect("selected attempt dir");
        let bytes = br#"{"axis":"axis","author":{"name":"reviewer","kind":"agent"},"result":"pass","findings":""}"#;
        fs::write(capture.path().join("0/attempts/1/stdout"), b"malformed").expect("first attempt");
        fs::write(&selected, bytes).expect("selected output");
        let digest = sha256_digest(bytes);
        let input = envelope(
            capture.path(),
            vec![worker(
                "assignment-1",
                contract("axis", "reviewer"),
                Some(2),
                Some(&selected),
                Some(digest),
            )],
        );

        let first = project(&input).expect("projection");
        let second = project(&input).expect("repeat projection");
        assert_eq!(
            serde_json::to_vec(&first).expect("serialize first"),
            serde_json::to_vec(&second).expect("serialize second")
        );
        assert_eq!(first.candidates.len(), 1);
        assert_eq!(
            first.candidates[0],
            ReviewCandidate::Ready {
                origin: CandidateOrigin {
                    kind: "selected-assignment-output",
                    id: "invocation-1".to_owned(),
                    assignment_id: "assignment-1".to_owned(),
                },
                review_stage: "aggregate".to_owned(),
                axis: "axis".to_owned(),
                author: CandidateAuthor {
                    name: "reviewer".to_owned(),
                    kind: "agent".to_owned(),
                },
                result: "pass".to_owned(),
                findings: String::new(),
                review_contract_version: None,
                grounds: None,
            }
        );
        assert_eq!(fs::read(&selected).expect("raw selected"), bytes);
        assert_eq!(
            fs::read(capture.path().join("0/attempts/1/stdout")).expect("raw first"),
            b"malformed"
        );
    }

    #[test]
    fn findings_rule_and_all_mechanical_non_ready_states_are_closed() {
        let capture = TempCapture::new("statuses");
        let malformed_path = capture.path().join("0/malformed");
        let unavailable_path = capture.path().join("1/output");
        let exhausted_dir = capture.path().join("3");
        fs::create_dir_all(malformed_path.parent().expect("malformed parent")).expect("parent");
        fs::create_dir_all(unavailable_path.parent().expect("unavailable parent")).expect("parent");
        fs::create_dir_all(&exhausted_dir).expect("exhausted dir");
        let malformed = br#"{"axis":"axis","author":{"name":"reviewer","kind":"agent"},"result":"pass","findings":"not empty"}"#;
        fs::write(&malformed_path, malformed).expect("malformed output");
        fs::write(&unavailable_path, b"not the recorded bytes").expect("unavailable output");
        fs::write(
            exhausted_dir.join(ATTEMPTS_FILE),
            br#"{"schema_version":"1","attempts":[{"number":1,"validation_errors":["bad"]},{"number":2,"validation_errors":["still bad"]}],"selected_attempt":null,"exhausted":true}"#,
        )
        .expect("manifest");

        let mut input = envelope(
            capture.path(),
            vec![
                worker(
                    "malformed",
                    contract("axis", "reviewer"),
                    Some(1),
                    Some(&malformed_path),
                    Some(sha256_digest(malformed)),
                ),
                worker(
                    "unavailable",
                    contract("axis", "reviewer"),
                    Some(1),
                    Some(&unavailable_path),
                    Some(
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .to_owned(),
                    ),
                ),
                worker("missing", contract("axis", "reviewer"), None, None, None),
            ],
        );
        input["result"]["work_slot_invocations"][0]["inner_workers"]
            .as_array_mut()
            .expect("workers")
            .push(worker(
                "exhausted",
                contract("axis", "reviewer"),
                None,
                None,
                None,
            ));

        let document = project(&input).expect("projection");
        let statuses = document
            .candidates
            .iter()
            .map(|candidate| match candidate {
                ReviewCandidate::Ready { .. } => "ready",
                ReviewCandidate::Carried { .. } => "carried",
                ReviewCandidate::Malformed { .. } => "malformed",
                ReviewCandidate::Unavailable { .. } => "unavailable",
                ReviewCandidate::MissingSelection { .. } => "missing-selection",
                ReviewCandidate::Exhausted { .. } => "exhausted",
                ReviewCandidate::VerdictReady { .. } => "verdict-ready",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            vec!["malformed", "unavailable", "missing-selection", "exhausted"]
        );
        for candidate in document.candidates {
            assert!(!matches!(candidate, ReviewCandidate::Ready { .. }));
            let value = serde_json::to_value(candidate).expect("candidate JSON");
            assert!(value.get("axis").is_none());
            assert!(value.get("author").is_none());
            assert!(value.get("result").is_none());
            assert!(value.get("findings").is_none());
            assert!(value.get("origin").is_some());
            assert!(value.get("diagnostic").is_some());
        }
    }

    #[test]
    fn non_review_workers_are_ignored_and_input_must_be_show() {
        let capture = TempCapture::new("ignored");
        let output = capture.path().join("0/stdout");
        fs::create_dir_all(output.parent().expect("output parent")).expect("parent");
        let bytes = br#"{"axis":"axis","author":{"name":"reviewer","kind":"agent"},"result":"pass","findings":""}"#;
        fs::write(&output, bytes).expect("output");
        let mut input = envelope(
            &capture.path().join("review"),
            vec![worker(
                "task-0",
                json!({"type":"object","required":["result"]}),
                Some(1),
                Some(&output),
                Some(sha256_digest(bytes)),
            )],
        );
        let mut foreign_workflow = input.clone();
        foreign_workflow["result"]["workflow_id"] = json!("research");
        assert!(project(&foreign_workflow)
            .expect_err("foreign workflow must not project")
            .to_string()
            .contains("software-change"));
        input["result"]["work_slot_invocations"][0]["slot_id"] = json!("implement");
        assert!(project(&input)
            .expect("ignored projection")
            .candidates
            .is_empty());

        input["operation"] = json!("evaluate");
        assert!(project(&input).is_err());
        input["operation"] = json!("show");
        input["status"] = json!("rejected");
        assert!(project(&input).is_err());
    }
}
