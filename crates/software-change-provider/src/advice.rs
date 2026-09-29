//! Provider-owned, evidence-selected typed advice preparation.
//!
//! This module only prepares requests from an explicit full-show selection. It
//! neither launches the configured advisor nor decides or records its answers.

use loop_core::{
    AdviceAdmissibility, AdviceQuestion, AdviceRequest, AdviceScoreLevel, ADVICE_COMMAND_INPUT_KEY,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const ARTIFACTS: &[&str] = &[
    "intent.json",
    "design.json",
    "plan.json",
    "reconciliation.json",
    "implementation-report.json",
    "validation-report.json",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedDocument {
    path: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparationPacket {
    show: Value,
    occasion_id: String,
    admissibility: AdviceAdmissibility,
    /// Driver-supplied atomic claims with caller-chosen stable question IDs.
    judgments: BTreeMap<String, String>,
    source_context_ids: Vec<String>,
    #[serde(default)]
    source_invocation_ids: Vec<String>,
    #[serde(default)]
    artifact_names: Vec<String>,
    #[serde(default)]
    documents: Vec<SelectedDocument>,
}

#[derive(Clone)]
struct SelectedContext {
    id: String,
    kind: String,
    data: Value,
    sequence: u64,
}

/// Read one full `show` envelope and emit the exact typed request for the
/// named frozen occasion. The input IDs are selectors, not evidence by
/// themselves; each must resolve in this show's immutable projection.
pub fn prepare_from_stdin() -> Result<Value, String> {
    let packet: PreparationPacket = serde_json::from_reader(std::io::stdin())
        .map_err(|error| format!("advice-request input is invalid JSON: {error}"))?;
    prepare(packet)
}

fn prepare(packet: PreparationPacket) -> Result<Value, String> {
    packet.admissibility.validate()?;
    if packet.judgments.is_empty()
        || packet
            .judgments
            .iter()
            .any(|(id, claim)| id.trim().is_empty() || claim.trim().is_empty())
    {
        return Err("advice-request requires explicit nonempty bounded judgments from the driver; broad occasion questions are not advisor assignments".to_owned());
    }
    if packet.occasion_id.trim().is_empty() {
        return Err("occasion_id must be non-empty".to_owned());
    }
    let result = packet
        .show
        .get("result")
        .filter(|_| packet.show["status"] == "completed")
        .ok_or("advice-request needs a completed full show envelope")?;
    if result.get("lifecycle").and_then(Value::as_str) != Some("active") {
        return Err("advice questions can only be prepared for an ACTIVE run".to_owned());
    }
    let initial = result
        .get("initial_input")
        .and_then(Value::as_object)
        .ok_or("full show omitted frozen initial_input")?;
    let advice_command = initial
        .get(ADVICE_COMMAND_INPUT_KEY)
        .ok_or("advice is disabled for this run; setup must configure a command")?;
    let command: loop_core::AdviceCommandConfig = serde_json::from_value(advice_command.clone())
        .map_err(|error| format!("frozen advice command is invalid: {error}"))?;
    command.validate()?;

    let family = packet
        .occasion_id
        .split_once(':')
        .map(|(family, _)| family)
        .ok_or("occasion_id must identify a frozen software-change occasion")?;
    let occasions = initial
        .get("extra")
        .and_then(|extra| extra.get("advice"))
        .and_then(|advice| advice.get("occasion_map"))
        .and_then(Value::as_array)
        .ok_or("frozen profile omitted its software-change advice occasion map")?;
    if !occasions
        .iter()
        .any(|occasion| occasion.as_str() == Some(family))
    {
        return Err(format!(
            "occasion family `{family}` is not in the frozen profile map"
        ));
    }
    let run_id = text(result, "run_id")?;
    let current_state = text(result, "current_state")?;
    let mapped = initial
        .get("advice_departures")
        .and_then(|map| map.get("occasions"))
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find(|row| {
                row.get("occasion_id").and_then(Value::as_str) == Some(packet.occasion_id.as_str())
            })
        });
    if mapped.is_none() {
        return Err(format!(
            "occasion `{}` is not in the frozen departure map",
            packet.occasion_id
        ));
    }
    let state_visit = result
        .get("state_visit")
        .and_then(Value::as_u64)
        .ok_or("full show omitted state_visit")?;
    let context = result
        .get("context")
        .and_then(Value::as_array)
        .ok_or("full show omitted context")?;
    let mut context_by_id = BTreeMap::new();
    for row in context {
        let id = text(row, "id")?.to_owned();
        let kind = text(row, "kind")?.to_owned();
        let data = row
            .get("data")
            .cloned()
            .ok_or_else(|| format!("context `{id}` omitted data"))?;
        let sequence = row.get("sequence").and_then(Value::as_u64).unwrap_or(0);
        if context_by_id
            .insert(
                id.clone(),
                SelectedContext {
                    id: id.clone(),
                    kind,
                    data,
                    sequence,
                },
            )
            .is_some()
        {
            return Err(format!("full show contains duplicate context ID `{id}`"));
        }
    }
    validate_unique_ids(&packet.source_context_ids, "source_context_ids")?;
    let mut sources = packet
        .source_context_ids
        .iter()
        .map(|id| {
            context_by_id
                .get(id)
                .cloned()
                .ok_or_else(|| format!("selected evidence context `{id}` is absent from this run"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sources.is_empty() {
        return Err("at least one actual source_context_id must be selected".to_owned());
    }
    let mut selected_ids: BTreeSet<String> =
        sources.iter().map(|source| source.id.clone()).collect();
    let mut ledger_sources = Vec::new();
    for ledger in sources
        .iter()
        .filter(|source| source.kind == "finding-ledger")
    {
        let findings = ledger
            .data
            .get("findings")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!(
                    "selected finding ledger `{}` omitted its findings array",
                    ledger.id
                )
            })?;
        for finding in findings {
            let id = finding
                .pointer("/source/id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "selected finding ledger `{}` has a finding without a source ID",
                        ledger.id
                    )
                })?;
            ledger_sources.push(id.to_owned());
        }
    }
    for id in ledger_sources {
        let source = context_by_id
            .get(&id)
            .ok_or_else(|| format!("selected finding ledger refers to missing evidence `{id}`"))?;
        if source.kind != "review-evidence" {
            return Err(format!(
                "finding ledger source `{id}` is not review-evidence"
            ));
        }
        if selected_ids.insert(id) {
            sources.push(source.clone());
        }
    }
    validate_unique_ids(&packet.source_invocation_ids, "source_invocation_ids")?;
    let invocation_rows = result
        .get("work_slot_invocations")
        .and_then(Value::as_array)
        .ok_or("full show omitted work_slot_invocations")?;
    let invocations = packet
        .source_invocation_ids
        .iter()
        .map(|id| {
            invocation_rows
                .iter()
                .find(|row| row.get("invocation_id").and_then(Value::as_str) == Some(id))
                .cloned()
                .ok_or_else(|| format!("selected invocation `{id}` is absent from this run"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_unique_ids(&packet.artifact_names, "artifact_names")?;

    let artifact_root = initial
        .get("artifact_root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or("full show omitted an absolute artifact_root")?;
    if !artifact_root.is_absolute() {
        return Err("frozen artifact_root must be absolute".to_owned());
    }
    let artifacts = read_selected_artifacts(&artifact_root, &packet.artifact_names)?;
    let documents = read_selected_documents(&packet.documents)?;
    let family_evidence: Vec<Value> = sources
        .iter()
        .map(|source| {
            json!({
                "id": source.id,
                "kind": source.kind,
                "sequence": source.sequence,
                "data": source.data
            })
        })
        .collect();
    let target = json!({
        "run_id": run_id,
        "state": current_state,
        "state_visit": state_visit,
        "source_context_ids": sources.iter().map(|source| source.id.clone()).collect::<Vec<_>>(),
        "source_invocation_ids": packet.source_invocation_ids,
        "artifact_sha256": artifacts.iter().map(|(name, value)| (name.clone(), value["sha256"].clone())).collect::<Map<_,_>>(),
        "document_sha256": documents.iter().map(|value| (value["locator"].as_str().unwrap_or_default().to_owned(), value["sha256"].clone())).collect::<Map<_,_>>()
    });
    let selected_policies = selected_policies(initial, &sources, &context_by_id);
    let selected_checks = if family == "final-completion" {
        let plan = artifacts
            .get("plan.json")
            .and_then(|row| row.get("document"))
            .ok_or("final-completion request must select current plan.json")?;
        let report = artifacts
            .get("validation-report.json")
            .and_then(|row| row.get("document"))
            .ok_or("final-completion request must select current validation-report.json")?;
        selected_check_assertions(report, plan, &sources, &artifact_root)?
    } else {
        Vec::new()
    };
    let state = json!({
        "occasion_family": family,
        "current_state": current_state,
        "state_visit": state_visit,
        "selected_evidence": family_evidence,
        "selected_artifacts": artifacts,
        "selected_documents": documents,
        "selected_invocations": invocations,
        "selected_policies": selected_policies,
        "selected_check_assertions": selected_checks,
        "agent_judgments": packet.judgments
    });

    // The occasion catalog checks evidence and available topics. Its broad
    // decision rubrics are not dispatched to a non-reasoning advisor.
    let _available_topics = build_questions(
        family,
        &sources,
        &artifacts,
        &documents,
        &invocations,
        initial,
    )?;
    let mut questions = BTreeMap::new();
    for (id, claim) in &packet.judgments {
        questions.insert(id.clone(), choice_question(
            format!("Classify only the supplied evidence's direct support for this driver-supplied bounded claim: {claim}\nDo not investigate, plan, infer missing facts, assess the whole workflow, or perform multi-step reasoning. The driver owns evidence sufficiency and all resulting decisions. If direct support or contradiction is absent, select not-established; do not fill the gap."),
            &[
                ("supported", "The supplied evidence directly supports the bounded claim."),
                ("contradicted", "The supplied evidence directly contradicts the bounded claim."),
                ("not-established", "The supplied evidence does not directly establish or contradict the bounded claim."),
            ],
        ));
    }
    let request = AdviceRequest {
        version: loop_core::ADVICE_PROTOCOL_VERSION,
        admissibility: packet.admissibility,
        state,
        target,
        occasion: packet.occasion_id,
        questions,
    };
    request.validate()?;
    serde_json::to_value(request)
        .map_err(|error| format!("could not encode advice request: {error}"))
}

fn read_selected_artifacts(
    artifact_root: &Path,
    names: &[String],
) -> Result<BTreeMap<String, Value>, String> {
    let mut result = BTreeMap::new();
    for name in names {
        if !ARTIFACTS.contains(&name.as_str()) {
            return Err(format!(
                "artifact `{name}` is not a supported software-change subject"
            ));
        }
        let path = artifact_root.join(name);
        let bytes = fs::read(&path)
            .map_err(|error| format!("selected artifact `{name}` is unavailable: {error}"))?;
        let document: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("selected artifact `{name}` is invalid JSON: {error}"))?;
        result.insert(
            name.clone(),
            json!({
                "locator": name,
                "sha256": format!("sha256:{:x}", Sha256::digest(&bytes)),
                "document": document
            }),
        );
    }
    Ok(result)
}

fn read_selected_documents(rows: &[SelectedDocument]) -> Result<Vec<Value>, String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        if row.path.trim().is_empty() {
            return Err("selected document path must be nonempty".to_owned());
        }
        let path = PathBuf::from(&row.path);
        if !path.is_absolute() {
            return Err(format!(
                "selected document path `{}` must be absolute",
                row.path
            ));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("selected document `{}` is unavailable: {error}", row.path))?;
        if !seen.insert(canonical.clone()) {
            return Err(format!("selected document `{}` is duplicated", row.path));
        }
        let bytes = fs::read(&canonical)
            .map_err(|error| format!("selected document `{}` cannot be read: {error}", row.path))?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        if row
            .sha256
            .as_ref()
            .is_some_and(|expected| expected != &digest)
        {
            return Err(format!(
                "selected document `{}` changed from its declared digest",
                row.path
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|error| format!("selected document `{}` is not UTF-8: {error}", row.path))?;
        result.push(json!({"locator":canonical.to_string_lossy(),"sha256":digest,"excerpt":text}));
    }
    Ok(result)
}

fn build_questions(
    family: &str,
    sources: &[SelectedContext],
    artifacts: &BTreeMap<String, Value>,
    documents: &[Value],
    invocations: &[Value],
    initial_input: &Map<String, Value>,
) -> Result<BTreeMap<String, AdviceQuestion>, String> {
    let mut questions = BTreeMap::new();
    match family {
        "review-candidates" => {
            let intent = artifact(artifacts, "intent.json")?;
            if !intent.get("acceptance").and_then(Value::as_array).is_some() {
                return Err("selected current intent omitted its acceptance criteria".to_owned());
            }
            let mut findings = 0;
            for source in sources
                .iter()
                .filter(|source| source.kind == "review-evidence")
            {
                if source.data.get("result").and_then(Value::as_str) != Some("fail") {
                    continue;
                }
                let claim = source
                    .data
                    .get("findings")
                    .and_then(Value::as_str)
                    .filter(|claim| !claim.trim().is_empty())
                    .ok_or_else(|| {
                        format!(
                            "failed selected review source `{}` has no finding",
                            source.id
                        )
                    })?;
                let subject = text(&source.data, "subject")?;
                let subject_artifact = artifact(artifacts, subject)?;
                let subject_revision = text(&source.data, "subject_revision")?;
                if subject_artifact.get("revision").and_then(Value::as_str)
                    != Some(subject_revision)
                {
                    return Err(format!(
                        "selected finding `{}` is not for the current subject revision",
                        source.id
                    ));
                }
                let gate = text(&source.data, "gate")?;
                let axis = text(&source.data, "policy_id")?;
                if initial_policy_for(initial_input, gate, axis).is_none() {
                    return Err(format!(
                        "selected finding `{}` names no frozen policy `{gate}/{axis}`",
                        source.id
                    ));
                }
                let prefix = format!("finding.{}", source.id);
                questions.insert(
                    format!("{prefix}.support"),
                    score_question(
                        format!("Assess support for the exact finding in selected evidence `{}` against its supplied policy and current artifacts. Do not search the repository or add findings.", source.id),
                        &[
                            ("no-source-or-obligation", "No actual source or named obligation is supplied."),
                            ("claim-or-locator-only", "Only a claim or locator is supplied; the underlying evidence is not shown."),
                            ("source-and-obligation-incomplete-burden", "The actual source and obligation are shown, but the consequence or validation gap is not established."),
                            ("source-obligation-consequence-gap", "The actual source, obligation, concrete consequence, and validation gap are all supported by supplied evidence."),
                        ],
                    ),
                );
                questions.insert(
                    format!("{prefix}.materiality"),
                    choice_question(
                        format!("Could the supplied finding `{claim}` plausibly affect delivery against the actual supplied obligation? Judge only the selected evidence."),
                        &[
                            ("material", "A plausible delivery consequence is evidenced."),
                            ("nonmaterial", "The supplied facts do not show a plausible delivery consequence."),
                            ("unclear", "The relevant consequence cannot be determined from supplied facts."),
                        ],
                    ),
                );
                questions.insert(
                    format!("{prefix}.scope"),
                    choice_question(
                        "Classify the supplied finding against the actual frozen intent and named obligation; do not infer an owner decision.".to_owned(),
                        &[
                            ("original-obligation", "The defect violates an obligation in the approved scope."),
                            ("introduced-by-change", "The supplied evidence shows this change introduced the defect."),
                            ("outside-scope", "The supplied finding is outside the approved boundary and was not introduced by this change."),
                            ("unclear", "Scope cannot be decided from the supplied artifacts."),
                        ],
                    ),
                );
                findings += 1;
            }
            if findings == 0 {
                return Err("review-candidates requires at least one selected current failed review-evidence row".to_owned());
            }
        }
        "accepted-defect" => {
            let findings = accepted_findings(sources, artifacts)?;
            for (ledger_id, finding) in findings {
                let id = text(&finding, "id")?;
                questions.insert(
                    format!("defect.{id}.owner"),
                    choice_question(
                        format!("Select the owning correction phase for accepted finding `{id}` from selected ledger `{ledger_id}` and supplied current artifacts."),
                        &[
                            ("validation-proof", "The defect is confined to validation report or proof evidence."),
                            ("implementation", "The defect is implementation-owned."),
                            ("plan", "The task decomposition or plan is wrong."),
                            ("design", "The accepted design is wrong."),
                            ("intent", "The accepted intent or requirement is wrong."),
                            ("unclear", "The owning phase cannot be determined from supplied evidence."),
                        ],
                    ),
                );
            }
        }
        "implementation-correction" => {
            let findings = accepted_findings(sources, artifacts)?;
            if findings.iter().any(|(_, finding)| {
                finding.get("owner_phase").and_then(Value::as_str) != Some("implementation")
            }) {
                return Err("implementation routing requires the driver-triaged owner_phase `implementation`".to_owned());
            }
            if !sources.iter().any(|source| {
                source.kind == "driver-note"
                    && source.data.get("correction_owner").and_then(Value::as_str)
                        == Some("implementation")
            }) {
                return Err(
                    "implementation routing must await an explicit driver correction-owner triage"
                        .to_owned(),
                );
            }
            require_artifact(artifacts, "plan.json")?;
            for (ledger_id, finding) in findings {
                let id = text(&finding, "id")?;
                questions.insert(
                    format!("defect.{id}.route"),
                    choice_question(
                        format!("Given accepted implementation finding `{id}`, the frozen task/dependency plan, and selected active-work/proof facts, which route is worth driver consideration? This is advice only, not routing permission."),
                        &[
                            ("task-and-dependants", "A frozen task and affected dependants can honestly own the correction."),
                            ("focused-no-task-bound-repair", "No task honestly owns it, but an authorized narrow bound repair path exists."),
                            ("narrow-direct-driver-act", "A small understood direct code/artifact act may qualify only when accepted intent/design/decomposition are unchanged, no conflicting work is active, authorship is recorded, and affected proof/review is refreshed."),
                            ("upstream-revision", "The frozen decomposition or upstream intent/design must be revised."),
                            ("unclear", "The supplied facts do not establish a safe route."),
                        ],
                    ),
                );
                let _ = ledger_id;
            }
        }
        "execution-or-authority-issue" => {
            if invocations.is_empty()
                && !sources.iter().any(|source| {
                    matches!(source.kind.as_str(), "command-evidence" | "execution-issue")
                })
            {
                return Err("execution-or-authority-issue requires a selected failure/authority record or invocation".to_owned());
            }
            questions.insert(
                "blocker.authority".to_owned(),
                choice_question(
                    "Using only the selected execution/authority diagnostics, which next consideration is supported? Advice does not grant authority, start work, retry, or cancel.".to_owned(),
                    &[
                        ("recover-within-authority", "Recovery appears possible after verified cleanup within current authority."),
                        ("focused-reconsideration", "The supplied issue calls for focused independent reconsideration."),
                        ("owner-decision", "A consequential scope, authority, or trade-off decision belongs to the owner."),
                        ("blocked-prerequisite", "A concrete operational prerequisite is unavailable."),
                        ("unclear", "The supplied diagnostics do not distinguish a safe next consideration."),
                    ],
                ),
            );
        }
        "evidence-applicability" => {
            let applicability: Vec<_> = sources
                .iter()
                .filter(|source| source.kind == "evidence-applicability")
                .collect();
            if applicability.is_empty() {
                return Err("evidence-applicability requires an explicitly selected applicability declaration".to_owned());
            }
            for source in applicability {
                questions.insert(
                    format!("source.{}.applicability", source.id),
                    choice_question(
                        format!("Assess only the explicitly selected original source, current target, and driver applicability reason in `{}`. The driver declares applicability; this answer does not." , source.id),
                        &[
                            ("unaffected", "The supplied before/after evidence supports that the original claim is unaffected."),
                            ("fresh-judgment-required", "The selected change may affect the original claim; obtain a fresh judgment."),
                            ("insufficient-evidence", "The supplied before/after evidence is insufficient to decide."),
                        ],
                    ),
                );
            }
            for source in sources.iter().filter(|source| {
                source
                    .data
                    .get("before_assertion")
                    .and_then(Value::as_str)
                    .is_some()
                    && source
                        .data
                        .get("after_assertion")
                        .and_then(Value::as_str)
                        .is_some()
            }) {
                questions.insert(
                    format!("source.{}.same-assertion", source.id),
                    noul_question(
                        format!("Do the supplied before and after assertions in selected evidence `{}` describe the same observable behavior?", source.id),
                        "The actual before and after assertions identify the same observable behavior.",
                    ),
                );
            }
        }
        "requirements-reconciliation" => {
            if artifacts.get("reconciliation.json").is_none() || documents.is_empty() {
                return Err("requirements-reconciliation requires the selected reconciliation artifact and explicitly selected authoritative document excerpts".to_owned());
            }
            if !sources.iter().any(|source| source.kind == "driver-note") {
                return Err("requirements-reconciliation requires selected driver observations of delivered behavior".to_owned());
            }
            questions.insert(
                "outcome.branch".to_owned(),
                choice_question(
                    "Given only the selected delivery observations, reconciliation artifact, and exact authoritative document excerpts, which reconciliation branch deserves driver inspection? Do not draft or accept requirements.".to_owned(),
                    &[
                        ("sufficient-existing-wording", "Existing accepted wording is sufficient; correct an implementation defect under it without inventing a requirement."),
                        ("change-specific-proof", "The claim is specific to this change's proof or document account, not a new enduring requirement."),
                        ("missing-or-changed-enduring-meaning", "The supplied documents appear to lack or change enduring meaning; exact owner acceptance and separately authorized application/commit remain required."),
                        ("unclear", "The supplied evidence does not support a branch."),
                    ],
                ),
            );
        }
        "review-round-departure" => {
            let review_rows: Vec<_> = sources
                .iter()
                .filter(|source| source.kind == "review-evidence")
                .collect();
            let ledgers: Vec<_> = sources
                .iter()
                .filter(|source| source.kind == "finding-ledger")
                .collect();
            if review_rows.is_empty() || ledgers.is_empty() {
                return Err("review-round-departure requires selected current review evidence and driver ledger state".to_owned());
            }
            questions.insert(
                "gate.departure".to_owned(),
                choice_question(
                    "Given the selected current reviewer outputs, driver ledger, and fix observations, what does the evidence support before this review-round departure? The driver still chooses and known material findings are not waived.".to_owned(),
                    &[
                        ("supports-departure", "Current evidence supports the proposed departure, subject to ordinary gates."),
                        ("repair-needed", "An accepted unresolved issue still needs correction."),
                        ("more-review-needed", "The supplied evidence calls for further independent review."),
                        ("unclear", "The supplied round history is insufficient."),
                    ],
                ),
            );
            let mut by_gate: BTreeMap<&str, Vec<&SelectedContext>> = BTreeMap::new();
            for ledger in &ledgers {
                let gate = text(&ledger.data, "gate")?;
                by_gate.entry(gate).or_default().push(ledger);
            }
            for (gate, _) in by_gate.into_iter().filter(|(_, rows)| rows.len() > 1) {
                questions.insert(
                    format!("gate.{gate}.quiet"),
                    noul_question(
                        format!("Did the supplied consecutive ledger snapshots for `{gate}` gain no newly accepted findings?"),
                        "The accepted finding set gained no newly accepted statements this round.",
                    ),
                );
                questions.insert(
                    format!("gate.{gate}.progress"),
                    noul_question(
                        format!("Do the supplied ledger snapshots and fix observations for `{gate}` show genuine reduction of accepted issues?"),
                        "Accepted issues were fixed or the applicable accepted set genuinely shrank.",
                    ),
                );
                questions.insert(
                    format!("gate.{gate}.thrash"),
                    noul_question(
                        format!("Do the supplied consecutive ledger snapshots and fix observations for `{gate}` show cycling/reopened issues without genuine progress?"),
                        "The same accepted statements cycle without a genuine fix.",
                    ),
                );
            }
        }
        "final-completion" => {
            let intent = artifact(artifacts, "intent.json")?;
            let plan = artifact(artifacts, "plan.json")?;
            let report = artifact(artifacts, "validation-report.json")?;
            let criteria = intent
                .get("acceptance")
                .and_then(Value::as_array)
                .ok_or("current intent has no acceptance array")?;
            let artifact_root = initial_input
                .get("artifact_root")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or("final-completion omitted artifact_root")?;
            let checks = selected_check_assertions(report, plan, sources, &artifact_root)?;
            let check_summary = selected_assertion_summary(&checks);
            let verdicts = sources
                .iter()
                .filter(|source| {
                    matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                })
                .collect::<Vec<_>>();
            if verdicts.is_empty() {
                return Err(
                    "final-completion requires selected current criterion/goal verdict evidence"
                        .to_owned(),
                );
            }
            for criterion in criteria {
                let id = text(criterion, "id")?;
                if !id.starts_with("AC-") {
                    continue;
                }
                let statement = text(criterion, "statement")?;
                let indexed_verdict_ids = report
                    .get("criteria")
                    .and_then(Value::as_array)
                    .and_then(|rows| {
                        rows.iter()
                            .find(|row| row.get("criterion_id").and_then(Value::as_str) == Some(id))
                    })
                    .and_then(|row| row.get("verdict_ids"))
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        format!("validation report omitted the current criterion index for `{id}`")
                    })?;
                let selected_verdict_ids: BTreeSet<&str> = verdicts
                    .iter()
                    .filter(|source| {
                        source.kind == "criterion-verdict"
                            && source.data.get("criterion_id").and_then(Value::as_str) == Some(id)
                    })
                    .map(|source| source.id.as_str())
                    .collect();
                if indexed_verdict_ids.iter().any(|verdict_id| {
                    verdict_id
                        .as_str()
                        .is_none_or(|verdict_id| !selected_verdict_ids.contains(verdict_id))
                }) || selected_verdict_ids.is_empty()
                {
                    return Err(format!(
                        "final-completion omitted selected indexed criterion verdict for `{id}`"
                    ));
                }
                let key = format!("criterion.{id}.fulfillment");
                questions.insert(
                    key,
                    choice_question(
                        format!("Assess current criterion `{id}` from its exact statement, selected command assertions, and selected independent verdicts: {statement}"),
                        fulfillment_criteria(),
                    ),
                );
                questions.insert(
                    format!("criterion.{id}.checks-could-miss"),
                    noul_question(
                        format!("Could all of these selected checks pass while criterion `{id}` still fails? The criterion statement is: {statement}. Actual selected assertions and captured output excerpts: {check_summary}"),
                        "Every selected check passes but the stated criterion still fails.",
                    ),
                );
            }
            let indexed_goal_ids = report
                .get("goal_verdict_ids")
                .and_then(Value::as_array)
                .ok_or("validation report omitted goal_verdict_ids")?;
            let selected_goal_ids: BTreeSet<&str> = verdicts
                .iter()
                .filter(|source| source.kind == "goal-verdict")
                .map(|source| source.id.as_str())
                .collect();
            if indexed_goal_ids.iter().any(|verdict_id| {
                verdict_id
                    .as_str()
                    .is_none_or(|verdict_id| !selected_goal_ids.contains(verdict_id))
            }) || selected_goal_ids.is_empty()
            {
                return Err(
                    "final-completion omitted selected indexed whole-goal verdict evidence"
                        .to_owned(),
                );
            }
            questions.insert(
                "goal.fulfillment".to_owned(),
                choice_question(
                    "Assess the whole approved goal from the actual intent, current validation report, selected check assertions, and independent goal verdicts.".to_owned(),
                    fulfillment_criteria(),
                ),
            );
            questions.insert(
                "goal.checks-could-miss".to_owned(),
                noul_question(
                    format!("Could every selected check pass while the whole approved goal still fails? Actual selected assertions and captured output excerpts: {check_summary}"),
                    "Every selected check passes but the whole approved goal still fails.",
                ),
            );
        }
        other => {
            return Err(format!(
                "unknown software-change advice occasion family `{other}`"
            ))
        }
    }
    if questions.is_empty() {
        return Err(format!(
            "occasion family `{family}` has no questions for the selected evidence"
        ));
    }
    Ok(questions)
}

fn selected_assertion_summary(checks: &[Value]) -> String {
    checks
        .iter()
        .map(|check| {
            let stdout = check
                .pointer("/capture_excerpt/streams/stdout/excerpt")
                .or_else(|| check.pointer("/capture_excerpt/stdout"))
                .and_then(Value::as_str)
                .unwrap_or("<selected capture had no stdout excerpt>");
            format!(
                "{}: {}; stdout excerpt: {stdout}",
                check["proof_id"].as_str().unwrap_or_default(),
                check["assertion"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn selected_check_assertions(
    report: &Value,
    plan: &Value,
    sources: &[SelectedContext],
    artifact_root: &Path,
) -> Result<Vec<Value>, String> {
    let expected = report
        .get("command_evidence_ids")
        .and_then(Value::as_array)
        .ok_or("current validation report omitted command_evidence_ids")?;
    let mut checks = Vec::new();
    for id in expected {
        let id = id
            .as_str()
            .ok_or("validation command evidence ID is not a string")?;
        let source = sources
            .iter()
            .find(|source| source.id == id && source.kind == "command-evidence")
            .ok_or_else(|| format!("final-completion omitted selected command evidence `{id}`"))?;
        let proof_id = source
            .data
            .get("proof_id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("selected command evidence `{id}` omitted proof_id"))?;
        let plan_command = plan
            .get("proof_commands")
            .and_then(Value::as_array)
            .and_then(|commands| {
                commands
                    .iter()
                    .find(|command| command.get("id").and_then(Value::as_str) == Some(proof_id))
            });
        let assertion = source
            .data
            .get("assertion")
            .and_then(Value::as_str)
            .or_else(|| {
                source
                    .data
                    .pointer("/spec/obligation")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                plan_command
                    .and_then(|command| command.get("obligation"))
                    .and_then(Value::as_str)
            })
            .filter(|assertion| !assertion.trim().is_empty())
            .ok_or_else(|| {
                format!("selected command evidence `{id}` has no actual assertion/obligation")
            })?;
        let capture_excerpt = selected_capture_excerpt(&source.data, artifact_root)?;
        checks.push(
            json!({"id": id, "proof_id": proof_id, "assertion": assertion,
            "capture_excerpt": capture_excerpt, "data": source.data}),
        );
    }
    if checks.is_empty() {
        return Err(
            "final-completion needs at least one selected actual check assertion".to_owned(),
        );
    }
    Ok(checks)
}

fn selected_capture_excerpt(data: &Value, artifact_root: &Path) -> Result<Value, String> {
    let capture = data
        .get("capture")
        .filter(|capture| !capture.is_null())
        .ok_or("selected command evidence omitted its capture")?;
    if capture.get("format").and_then(Value::as_str) == Some("common-capture-v1") {
        let root = artifact_root.canonicalize().map_err(|error| {
            format!("artifact_root is unavailable while reading selected check capture: {error}")
        })?;
        let path = PathBuf::from(text(capture, "index")?);
        let index_path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let index_path = index_path
            .canonicalize()
            .map_err(|error| format!("selected check-capture index is unavailable: {error}"))?;
        if !index_path.starts_with(&root) {
            return Err("selected check-capture index escapes artifact_root".to_owned());
        }
        let index: Value =
            serde_json::from_slice(&fs::read(&index_path).map_err(|error| error.to_string())?)
                .map_err(|error| {
                    format!("selected check-capture index is invalid JSON: {error}")
                })?;
        let proof_id = text(data, "proof_id")?;
        let receipts = index
            .get("receipts")
            .and_then(Value::as_array)
            .ok_or("selected check-capture index omitted receipts")?;
        let matching: Vec<_> = receipts
            .iter()
            .filter(|row| row.get("id").and_then(Value::as_str) == Some(proof_id))
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "selected check-capture index has {} rows for `{proof_id}`",
                matching.len()
            ));
        }
        let receipt_path = PathBuf::from(text(matching[0], "receipt")?);
        let receipt_path = if receipt_path.is_absolute() {
            receipt_path
        } else {
            index_path.parent().unwrap().join(receipt_path)
        };
        let receipt_path = receipt_path
            .canonicalize()
            .map_err(|error| format!("selected check receipt is unavailable: {error}"))?;
        if !receipt_path.starts_with(&root) {
            return Err("selected check receipt escapes artifact_root".to_owned());
        }
        let receipt: Value =
            serde_json::from_slice(&fs::read(&receipt_path).map_err(|error| error.to_string())?)
                .map_err(|error| format!("selected check receipt is invalid JSON: {error}"))?;
        let spec = data
            .get("spec")
            .ok_or("selected command evidence omitted its proof spec")?;
        let mut expected_argv = vec![text(spec, "command")?.to_owned()];
        expected_argv.extend(
            spec.get("args")
                .and_then(Value::as_array)
                .ok_or("selected proof spec omitted args")?
                .iter()
                .map(|argument| {
                    argument
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("proof arg is not a string")
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
        let expected_argv = expected_argv
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        let actual_argv = receipt
            .get("argv")
            .and_then(Value::as_array)
            .ok_or("selected check receipt omitted argv")?;
        if actual_argv != &expected_argv
            || receipt.get("id").and_then(Value::as_str) != Some(proof_id)
            || receipt.get("exit_code").and_then(Value::as_i64) != Some(0)
            || receipt.get("timed_out").and_then(Value::as_bool) != Some(false)
            || receipt
                .get("spawn_error")
                .is_some_and(|error| !error.is_null())
        {
            return Err(format!(
                "selected check `{proof_id}` did not complete with the declared command"
            ));
        }
        let mut excerpts = Map::new();
        for stream in ["stdout", "stderr"] {
            let relative = text(&receipt, stream)?;
            let stream_path = receipt_path.parent().unwrap().join(relative);
            let stream_path = stream_path
                .canonicalize()
                .map_err(|error| format!("selected check {stream} is unavailable: {error}"))?;
            if !stream_path.starts_with(&root) {
                return Err(format!("selected check {stream} escapes artifact_root"));
            }
            let bytes = fs::read(&stream_path).map_err(|error| error.to_string())?;
            let expected_digest = text(&receipt, &format!("{stream}_sha256"))?;
            let actual_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
            let expected_digest = expected_digest
                .strip_prefix("sha256:")
                .unwrap_or(expected_digest);
            if expected_digest != &actual_digest[7..] {
                return Err(format!(
                    "selected check {stream} no longer matches its captured digest"
                ));
            }
            let limit = bytes.len().min(8192);
            excerpts.insert(
                stream.to_owned(),
                json!({
                    "excerpt": String::from_utf8_lossy(&bytes[..limit]),
                    "byte_length": bytes.len(),
                    "sha256": actual_digest,
                    "truncated": limit != bytes.len()
                }),
            );
        }
        return Ok(json!({"format":"common-capture-v1","receipt":receipt,"streams":excerpts}));
    }
    if let Some(summary_value) = capture.get("summary").and_then(Value::as_str) {
        let root = artifact_root.canonicalize().map_err(|error| {
            format!("artifact_root is unavailable while reading selected check capture: {error}")
        })?;
        let summary_path = PathBuf::from(summary_value);
        let summary_path = if summary_path.is_absolute() {
            summary_path
        } else {
            root.join(summary_path)
        };
        let summary_path = summary_path
            .canonicalize()
            .map_err(|error| format!("selected command summary is unavailable: {error}"))?;
        if !summary_path.starts_with(&root) {
            return Err("selected command summary escapes artifact_root".to_owned());
        }
        let summary: Value =
            serde_json::from_slice(&fs::read(&summary_path).map_err(|error| error.to_string())?)
                .map_err(|error| format!("selected command summary is invalid JSON: {error}"))?;
        let assignment_id = text(capture, "assignment_id")?;
        let workers = summary
            .get("workers")
            .and_then(Value::as_array)
            .ok_or("selected command summary omitted workers")?;
        let matching: Vec<_> = workers
            .iter()
            .filter(|worker| {
                worker.get("assignment_id").and_then(Value::as_str) == Some(assignment_id)
            })
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "selected command summary has {} rows for `{assignment_id}`",
                matching.len()
            ));
        }
        let worker = matching[0];
        if worker.get("exit_code").and_then(Value::as_i64) != Some(0)
            || worker
                .get("selected_attempt")
                .and_then(Value::as_u64)
                .is_none_or(|attempt| attempt == 0)
        {
            return Err(format!(
                "selected command worker `{assignment_id}` did not succeed"
            ));
        }
        let output_path = PathBuf::from(text(worker, "selected_output_path")?);
        let output_path = if output_path.is_absolute() {
            output_path
        } else {
            summary_path.parent().unwrap().join(output_path)
        };
        let output_path = output_path
            .canonicalize()
            .map_err(|error| format!("selected command output is unavailable: {error}"))?;
        if !output_path.starts_with(summary_path.parent().unwrap()) {
            return Err("selected command output escapes its capture directory".to_owned());
        }
        let bytes = fs::read(&output_path).map_err(|error| error.to_string())?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        if worker.get("selected_output_sha256").and_then(Value::as_str) != Some(digest.as_str()) {
            return Err("selected command output no longer matches its captured digest".to_owned());
        }
        let limit = bytes.len().min(8192);
        return Ok(json!({
            "format":"assignment-summary-v1","assignment_id":assignment_id,
            "selected_attempt":worker.get("selected_attempt"),"selected_output_sha256":digest,
            "stdout_excerpt":String::from_utf8_lossy(&bytes[..limit]),
            "byte_length":bytes.len(),"truncated":limit != bytes.len(),
            "selected_worker":worker
        }));
    }

    // Small public fixture packets may carry an exact child result excerpt
    // directly; the full future-run path uses verified common-capture-v1 data.
    if capture.get("exit_code").and_then(Value::as_i64) != Some(0) {
        return Err("selected fixture command capture is not a successful execution".to_owned());
    }
    let stdout = capture
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(json!({"format":"selected-capture-excerpt","stdout":stdout,"exit_code":0}))
}

fn accepted_findings(
    sources: &[SelectedContext],
    artifacts: &BTreeMap<String, Value>,
) -> Result<Vec<(String, Value)>, String> {
    let mut accepted = Vec::new();
    for source in sources
        .iter()
        .filter(|source| source.kind == "finding-ledger")
    {
        let findings = source
            .data
            .get("findings")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("selected finding ledger `{}` omitted findings", source.id))?;
        for finding in findings {
            if finding.get("disposition").and_then(Value::as_str) == Some("accepted")
                && finding.get("status").and_then(Value::as_str) == Some("unresolved")
            {
                let subject = text(&source.data, "subject")?;
                let subject_revision = text(&source.data, "subject_revision")?;
                let current = artifact(artifacts, subject)?;
                if current.get("revision").and_then(Value::as_str) != Some(subject_revision) {
                    return Err(format!(
                        "accepted finding ledger `{}` is stale for current subject `{subject}`",
                        source.id
                    ));
                }
                accepted.push((source.id.clone(), finding.clone()));
            }
        }
    }
    if accepted.is_empty() {
        return Err(
            "dependent advice requires a selected driver-triaged accepted unresolved finding"
                .to_owned(),
        );
    }
    Ok(accepted)
}

fn artifact<'a>(artifacts: &'a BTreeMap<String, Value>, name: &str) -> Result<&'a Value, String> {
    artifacts
        .get(name)
        .and_then(|row| row.get("document"))
        .ok_or_else(|| format!("request must explicitly select current artifact `{name}`"))
}

fn require_artifact(artifacts: &BTreeMap<String, Value>, name: &str) -> Result<(), String> {
    artifact(artifacts, name).map(|_| ())
}

fn selected_policies(
    initial_input: &Map<String, Value>,
    sources: &[SelectedContext],
    context_by_id: &BTreeMap<String, SelectedContext>,
) -> Vec<Value> {
    let mut selected = BTreeMap::new();
    let mut add = |gate: &str, axis: &str| {
        if let Some(policy) = initial_policy_for(initial_input, gate, axis) {
            selected.insert((gate.to_owned(), axis.to_owned()), policy.clone());
        }
    };
    for source in sources {
        if source.kind == "review-evidence" {
            if let (Some(gate), Some(axis)) = (
                source.data.get("gate").and_then(Value::as_str),
                source.data.get("policy_id").and_then(Value::as_str),
            ) {
                add(gate, axis);
            }
        } else if source.kind == "finding-ledger" {
            for finding in source
                .data
                .get("findings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(source_id) = finding.pointer("/source/id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(review) = context_by_id
                    .get(source_id)
                    .filter(|row| row.kind == "review-evidence")
                else {
                    continue;
                };
                if let (Some(gate), Some(axis)) = (
                    review.data.get("gate").and_then(Value::as_str),
                    review.data.get("policy_id").and_then(Value::as_str),
                ) {
                    add(gate, axis);
                }
            }
        }
    }
    selected
        .into_iter()
        .map(|((gate, axis), policy)| json!({"gate":gate,"axis":axis,"policy":policy}))
        .collect()
}

fn initial_policy_for<'a>(
    initial_input: &'a Map<String, Value>,
    gate: &str,
    axis: &str,
) -> Option<&'a Value> {
    initial_input
        .get("review_policies")
        .and_then(|policies| policies.get(gate))
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(axis))
        })
}

fn choice_question(instructions: String, criteria: &[(&str, &str)]) -> AdviceQuestion {
    AdviceQuestion::Choice {
        instructions,
        criteria: criteria
            .iter()
            .map(|(key, description)| ((*key).to_owned(), json!(description)))
            .collect(),
    }
}

fn score_question(instructions: String, levels: &[(&str, &str)]) -> AdviceQuestion {
    AdviceQuestion::Score {
        instructions,
        levels: levels
            .iter()
            .map(|(description, criteria)| AdviceScoreLevel {
                description: (*description).to_owned(),
                criteria: json!(criteria),
            })
            .collect(),
    }
}

fn noul_question(instructions: String, proposition: &str) -> AdviceQuestion {
    AdviceQuestion::Noul {
        instructions,
        proposition: proposition.to_owned(),
        true_criteria: None,
        false_criteria: None,
    }
}

fn fulfillment_criteria() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "demonstrated",
            "The actual operator path and selected current evidence demonstrate this outcome.",
        ),
        (
            "missing-proof",
            "The supplied evidence does not prove this outcome.",
        ),
        (
            "delivery-failed",
            "The supplied evidence shows the delivered behavior fails this outcome.",
        ),
        (
            "intent-ambiguous",
            "The approved criterion or goal is materially ambiguous in the supplied evidence.",
        ),
    ]
}

fn validate_unique_ids(values: &[String], label: &str) -> Result<(), String> {
    if values.iter().any(|value| value.trim().is_empty())
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
    {
        return Err(format!("{label} must contain unique non-empty IDs"));
    }
    Ok(())
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing non-empty `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (Value, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "software-change-advice-request-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let intent = json!({
            "revision":"intent-r1",
            "acceptance":[{"id":"AC-1","statement":"The operator can inspect the actual selected evidence."}]
        });
        let plan = json!({"revision":"plan-r1","tasks":[{"id":"P11","dependencies":[]}]});
        let reconciliation =
            json!({"revision":"reconciliation-r1","branch":"sufficient-existing-wording"});
        let validation = json!({
            "revision":"validation-r1",
            "command_evidence_ids":["cmd-1"],
            "criteria":[{"criterion_id":"AC-1","verdict_ids":["cv-1"]}],
            "goal_verdict_ids":["gv-1"]
        });
        for (name, document) in [
            ("intent.json", &intent),
            ("plan.json", &plan),
            ("reconciliation.json", &reconciliation),
            ("validation-report.json", &validation),
        ] {
            fs::write(root.join(name), serde_json::to_vec(document).unwrap()).unwrap();
        }
        let doc_path = root.join("PRD.md");
        fs::write(
            &doc_path,
            "### LE-1: Accepted behavior\n\nThe operator can inspect selected evidence.\n",
        )
        .unwrap();
        let fail_review = json!({
            "gate":"intent-review","policy_id":"solution-agnostic","review_stage":"aggregate",
            "result":"fail","findings":"The selected excerpt may not prove the named outcome.",
            "author":{"name":"reviewer-a","kind":"script"},"subject":"intent.json",
            "subject_revision":"intent-r1","config_version":"high-rigor-11"
        });
        let finding = json!({
            "id":"F-advice-1","source":{"kind":"context-record","id":"review-fail"},
            "policy_id":"solution-agnostic","statement":"Selected proof may miss the outcome.",
            "disposition":"accepted","reason":"The driver accepted this exact source for the fixture.",
            "owner_phase":"implementation","task_ids":["P11"],"review_axes":["solution-agnostic"],"status":"unresolved"
        });
        let ledger_data = json!({
            "schema_version":"1","gate":"intent-review","subject":"intent.json",
            "subject_revision":"intent-r1","author":{"name":"driver","kind":"script"},"findings":[finding]
        });
        let context = json!([
            {"id":"review-fail","kind":"review-evidence","sequence":1,"data":fail_review},
            {"id":"ledger-accepted","kind":"finding-ledger","sequence":2,"data":ledger_data},
            {"id":"ledger-second","kind":"finding-ledger","sequence":3,"data":ledger_data},
            {"id":"applicability-1","kind":"evidence-applicability","sequence":4,"data":{
                "origin":{"kind":"context-record","id":"review-fail"},
                "target":{"subject":"intent.json","revision":"intent-r1","checkpoint":null},
                "attesting_driver":{"name":"driver","kind":"script"},
                "reason":"The exact selected claim was compared with the current target."
            }},
            {"id":"driver-observation","kind":"driver-note","sequence":5,"data":{
                "observation":"The delivered assertion was inspected.",
                "correction_owner":"implementation",
                "before_assertion":"Before, the selected evidence was not shown.",
                "after_assertion":"After, the selected evidence is shown."
            }},
            {"id":"execution-issue","kind":"execution-issue","sequence":6,"data":{"status":"failed","reason":"The selected scripted command returned nonzero."}},
            {"id":"cmd-1","kind":"command-evidence","sequence":7,"data":{"proof_id":"check-1","assertion":"The public command showed the selected evidence and returned its exact source ID.","capture":{"stdout":"assertion: current AC-1 selected evidence\n","stderr":"","exit_code":0}}},
            {"id":"cv-1","kind":"criterion-verdict","sequence":8,"data":{"criterion_id":"AC-1","result":"pass","reason":"The supplied public assertion matches the criterion."}},
            {"id":"gv-1","kind":"goal-verdict","sequence":9,"data":{"result":"pass","reason":"The supplied public assertion supports the whole goal."}}
        ]);
        let families = [
            "review-candidates",
            "accepted-defect",
            "implementation-correction",
            "execution-or-authority-issue",
            "evidence-applicability",
            "requirements-reconciliation",
            "review-round-departure",
            "final-completion",
        ];
        let departures = families.iter().map(|family| json!({
            "state":"intent-review","event":"approved","occasion_id":format!("{family}:intent-review:approved")
        })).collect::<Vec<_>>();
        let show = json!({
            "status":"completed",
            "result":{
                "run_id":"advice-fixture","lifecycle":"active","current_state":"intent-review","state_visit":4,
                "initial_input":{
                    "contract_version":3,"config_version":"high-rigor-11","artifact_root":root,
                    "advice_command":{"command":"/bin/true","args":[],"timeout_ms":500,
                        "max_request_bytes":4096,"max_response_bytes":4096},
                    "extra":{"advice":{"occasion_map":families}},
                    "advice_departures":{"version":1,"occasions":departures},
                    "review_policies":{"intent-review":[{"id":"solution-agnostic","description":"The intent states a product outcome.",
                        "example_prompt":"Judge the selected intent outcome.","review_stage":"aggregate","required_authors":2}]}
                },
                "context":context,"work_slot_invocations":[{"invocation_id":"inv-failed","slot_id":"intent-review",
                    "state_visit":4,"status":"failed","exit_code":7,"inner_workers":[]}]
            }
        });
        (show, root)
    }

    fn request_for(
        show: &Value,
        root: &Path,
        family: &str,
        source_context_ids: &[&str],
        artifacts: &[&str],
        invocation_ids: &[&str],
        with_document: bool,
    ) -> PreparationPacket {
        let documents = if with_document {
            let path = root.join("PRD.md");
            let digest = format!("sha256:{:x}", Sha256::digest(fs::read(&path).unwrap()));
            json!([{"path":path,"sha256":digest}])
        } else {
            json!([])
        };
        let family_id = format!("{family}:intent-review:approved");
        let topic = match family {
            "review-candidates" => "finding.review-fail.support",
            "accepted-defect" => "defect.F-advice-1.owner",
            "implementation-correction" => "defect.F-advice-1.route",
            "execution-or-authority-issue" => "blocker.authority",
            "evidence-applicability" => "source.driver-observation.same-assertion",
            "requirements-reconciliation" => "outcome.branch",
            "review-round-departure" => "gate.intent-review.quiet",
            "final-completion" => "criterion.AC-1.checks-could-miss",
            _ => unreachable!(),
        };
        let packet = json!({
            "show":show,"occasion_id":family_id,
            "admissibility":{"bounded_judgment":true,"evidence_sufficient":true},
            "judgments":{topic:"The selected excerpt describes completed work rather than a future plan."},
            "source_context_ids":source_context_ids,"source_invocation_ids":invocation_ids,
            "artifact_names":artifacts,"documents":documents
        });
        serde_json::from_value(packet).unwrap()
    }

    #[test]
    fn prepares_eight_evidence_selected_question_families_and_waits_for_triage() {
        let (show, root) = fixture();
        type AdviceCase<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str], bool);
        let cases: [AdviceCase<'_>; 8] = [
            (
                "review-candidates",
                &["review-fail"],
                &["intent.json"],
                &[],
                false,
            ),
            (
                "accepted-defect",
                &["ledger-accepted"],
                &["intent.json"],
                &[],
                false,
            ),
            (
                "implementation-correction",
                &["ledger-accepted", "driver-observation"],
                &["plan.json", "intent.json"],
                &[],
                false,
            ),
            (
                "execution-or-authority-issue",
                &["execution-issue"],
                &[],
                &["inv-failed"],
                false,
            ),
            (
                "evidence-applicability",
                &["applicability-1", "driver-observation"],
                &[],
                &[],
                false,
            ),
            (
                "requirements-reconciliation",
                &["driver-observation"],
                &["reconciliation.json"],
                &[],
                true,
            ),
            (
                "review-round-departure",
                &["review-fail", "ledger-accepted", "ledger-second"],
                &[],
                &[],
                false,
            ),
            (
                "final-completion",
                &["cmd-1", "cv-1", "gv-1"],
                &["intent.json", "plan.json", "validation-report.json"],
                &[],
                false,
            ),
        ];
        let mut prepared = BTreeMap::new();
        for (family, source_ids, artifacts, invocations, documents) in cases {
            let packet = request_for(
                &show,
                &root,
                family,
                source_ids,
                artifacts,
                invocations,
                documents,
            );
            let value = prepare(packet).expect(family);
            let request =
                AdviceRequest::parse(&serde_json::to_vec(&value).unwrap()).expect("typed request");
            assert_eq!(request.occasion.split(':').next(), Some(family));
            assert!(request.state["selected_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .all(|source| source.get("id").is_some() && source.get("data").is_some()));
            prepared.insert(family, request);
        }
        for request in prepared.values() {
            assert_eq!(
                request.questions.len(),
                1,
                "no automatic broad question batch"
            );
            let (id, question) = request.questions.iter().next().unwrap();
            assert_eq!(
                request.state["agent_judgments"][id],
                "The selected excerpt describes completed work rather than a future plan."
            );
            let AdviceQuestion::Choice {
                instructions,
                criteria,
            } = question
            else {
                panic!("prepared advice is a bounded support judgment");
            };
            assert!(instructions.contains("Do not investigate, plan, infer missing facts"));
            assert_eq!(
                criteria.keys().cloned().collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    "supported".to_owned(),
                    "contradicted".to_owned(),
                    "not-established".to_owned()
                ])
            );
        }
        assert!(prepared["review-candidates"]
            .questions
            .contains_key("finding.review-fail.support"));
        assert!(prepared["evidence-applicability"]
            .questions
            .contains_key("source.driver-observation.same-assertion"));
        assert!(prepared["review-round-departure"]
            .questions
            .contains_key("gate.intent-review.quiet"));
        let final_questions = &prepared["final-completion"].questions;
        assert!(final_questions.contains_key("criterion.AC-1.checks-could-miss"));
        assert!(!final_questions.contains_key("goal.fulfillment"));
        let serialized = serde_json::to_string(&prepared["final-completion"]).unwrap();
        assert!(serialized.contains("The public command showed the selected evidence"));

        let no_triage = request_for(
            &show,
            &root,
            "accepted-defect",
            &["review-fail"],
            &["intent.json"],
            &[],
            false,
        );
        assert!(prepare(no_triage)
            .unwrap_err()
            .contains("driver-triaged accepted unresolved finding"));
        let no_fix_route = request_for(
            &show,
            &root,
            "implementation-correction",
            &["review-fail"],
            &["plan.json"],
            &[],
            false,
        );
        assert!(prepare(no_fix_route)
            .unwrap_err()
            .contains("driver-triaged accepted unresolved finding"));
        assert!(!serialized.contains("answer"));
        for field in ["bounded_judgment", "evidence_sufficient"] {
            let mut packet = request_for(
                &show,
                &root,
                "review-candidates",
                &["review-fail"],
                &["intent.json"],
                &[],
                false,
            );
            if field == "bounded_judgment" {
                packet.admissibility.bounded_judgment = false;
            } else {
                packet.admissibility.evidence_sufficient = false;
            }
            assert!(prepare(packet).is_err());
        }
        let mut packet = request_for(
            &show,
            &root,
            "review-candidates",
            &["review-fail"],
            &["intent.json"],
            &[],
            false,
        );
        packet.judgments.clear();
        assert!(prepare(packet)
            .unwrap_err()
            .contains("explicit nonempty bounded judgments"));
        let _ = fs::remove_dir_all(root);
    }
}
