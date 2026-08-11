//! Deterministic provider fixtures for the Loop Engine reference workflows.
//!
//! This crate is test infrastructure, not a production provider SDK.  The
//! two binaries use the same v0.1 one-request/one-response subprocess wire
//! contract as real providers, while keeping reference-workflow conventions
//! local to this fixture crate.

use loop_core::{ContextRecord, DurableEvaluation, State, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The executable names built by this fixture package.
pub const SOFTWARE_CHANGE_PROVIDER: &str = "software-change-provider";
pub const POLICY_DOCUMENT_PROVIDER: &str = "policy-document-provider";

/// Reference software-change review gates.
pub const DESIGN_REVIEW_GATE: &str = "design-review";
pub const PLAN_REVIEW_GATE: &str = "plan-review";
pub const IMPLEMENTATION_REVIEW_GATE: &str = "implementation-review";
pub const VALIDATION_GATE: &str = "validation";

/// Which reference provider should process a protocol request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureProvider {
    SoftwareChange,
    PolicyDocument,
}

/// A response written by a fixture executable.
///
/// `Raw` and `Failure` intentionally exist only to make malformed-response and
/// process-failure scenarios controllable by tests.  Normal provider behavior
/// uses `Json`, which is exactly one protocol response value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureResponse {
    Json(Value),
    Raw(String),
    Failure(String),
}

/// A deterministic fixture request failure.  The executable reports these on
/// stderr and exits non-zero; they are not provider protocol responses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureError {
    message: String,
}

impl FixtureError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Explain a malformed fixture request to a test or executable caller.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FixtureError {}

/// Return the path to one of this package's fixture binaries.
///
/// Cargo exposes `CARGO_BIN_EXE_*` while running this package's integration
/// tests.  The target-directory fallback also makes the helper useful to a
/// later acceptance crate that depends on this fixture crate.
pub fn fixture_binary(name: &str) -> PathBuf {
    let environment_name = format!(
        "CARGO_BIN_EXE_{}",
        name.replace('-', "_").to_ascii_lowercase()
    );
    if let Some(path) = env::var_os(&environment_name) {
        return PathBuf::from(path);
    }

    let target_directory = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .to_path_buf()
        });
    let executable = target_directory.join("debug").join(name);
    if executable.exists() {
        return executable;
    }

    // `cargo test` may place a package binary only in `debug/deps` with a
    // hash suffix when this helper is called from another package's tests.
    let deps_directory = target_directory.join("debug").join("deps");
    if let Ok(entries) = fs::read_dir(&deps_directory) {
        let mut candidates = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|file| file.to_str())
                        .map(|file| file.starts_with(&format!("{name}-")))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        candidates.sort();
        if let Some(candidate) = candidates.into_iter().next() {
            return candidate;
        }
    }

    // Keep the returned path useful even before a caller has built the
    // fixture.  The resulting subprocess error identifies the missing binary.
    executable
}

/// Run a fixture provider against stdin and write one protocol response.
///
/// This is used by both tiny binary entry points so protocol framing remains
/// visibly identical across providers.
pub fn run_provider(provider: FixtureProvider) -> i32 {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("fixture could not read stdin: {error}");
        return 2;
    }
    let request = match serde_json::from_str::<Value>(&input) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("fixture received malformed JSON: {error}");
            return 2;
        }
    };

    let response = match process_request(provider, request) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("fixture rejected request: {error}");
            return 2;
        }
    };

    match normalize_special_response(response) {
        FixtureResponse::Json(value) => match serde_json::to_writer(io::stdout(), &value) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("fixture could not write response: {error}");
                2
            }
        },
        FixtureResponse::Raw(raw) => match io::stdout().write_all(raw.as_bytes()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("fixture could not write raw response: {error}");
                2
            }
        },
        FixtureResponse::Failure(message) => {
            eprintln!("fixture failure: {message}");
            17
        }
    }
}

/// Process one parsed provider request without spawning a process.
///
/// Keeping this small entry point public lets fixture tests prove the
/// workflow-specific policy conventions independently from subprocess setup;
/// end-to-end tests should still invoke the binaries through the gateway.
pub fn process_request(
    provider: FixtureProvider,
    request: Value,
) -> Result<FixtureResponse, FixtureError> {
    let object = request
        .as_object()
        .ok_or_else(|| FixtureError::new("provider request must be a JSON object"))?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| FixtureError::new("provider request requires a string operation"))?;

    match operation {
        "describe" => {
            if object.len() != 1 {
                return Err(FixtureError::new(
                    "describe request must contain only the operation field",
                ));
            }
            let workflow = match provider {
                FixtureProvider::SoftwareChange => software_change_workflow(),
                FixtureProvider::PolicyDocument => policy_document_workflow(),
            };
            Ok(FixtureResponse::Json(
                serde_json::to_value(workflow).map_err(|error| {
                    FixtureError::new(format!("could not encode workflow: {error}"))
                })?,
            ))
        }
        "evaluate" => {
            let request: EvaluateRequest = serde_json::from_value(Value::Object(object.clone()))
                .map_err(|error| FixtureError::new(format!("invalid evaluate request: {error}")))?;
            match provider {
                FixtureProvider::SoftwareChange => {
                    evaluate_software_change(request).map(FixtureResponse::Json)
                }
                FixtureProvider::PolicyDocument => {
                    evaluate_policy_document(request).map(FixtureResponse::Json)
                }
            }
        }
        other => Err(FixtureError::new(format!(
            "unsupported provider operation `{other}`"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluateRequest {
    operation: String,
    workflow: Workflow,
    initial_input: Value,
    context: Vec<ContextRecord>,
    transition: Transition,
    prior_evaluations: Vec<DurableEvaluation>,
}

impl EvaluateRequest {
    fn validate_operation(&self) -> Result<(), FixtureError> {
        if self.operation == "evaluate" {
            Ok(())
        } else {
            Err(FixtureError::new(
                "evaluate request has the wrong operation",
            ))
        }
    }
}

/// Exact software-change reference topology from PRD §10.
pub fn software_change_workflow() -> Workflow {
    Workflow::new(
        "software-change",
        "explore",
        vec![
            State::new(
                "explore",
                "Explore",
                "Clarify the change and capture the intent.",
                false,
            ),
            State::new(
                "design",
                "Design",
                "Develop the design that satisfies the change.",
                false,
            ),
            State::new(
                DESIGN_REVIEW_GATE,
                "Design review",
                "Obtain external review evidence for the design policies.",
                false,
            ),
            State::new(
                "plan",
                "Plan",
                "Turn the accepted design into an implementation plan.",
                false,
            ),
            State::new(
                PLAN_REVIEW_GATE,
                "Plan review",
                "Obtain external review evidence for the plan policies.",
                false,
            ),
            State::new(
                "implement",
                "Implement",
                "Make the external software change.",
                false,
            ),
            State::new(
                IMPLEMENTATION_REVIEW_GATE,
                "Implementation review",
                "Obtain external review evidence for implementation policies.",
                false,
            ),
            State::new(
                VALIDATION_GATE,
                "Validation",
                "Obtain external evidence that the completed change is valid.",
                false,
            ),
            State::new("end", "End", "The software change is complete.", true),
        ],
        vec![
            Transition::checked("explore", "intent-ready", "design"),
            Transition::checked("design", "design-ready", DESIGN_REVIEW_GATE),
            Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan"),
            Transition::check_free(DESIGN_REVIEW_GATE, "revise", "design"),
            Transition::checked("plan", "plan-ready", PLAN_REVIEW_GATE),
            Transition::checked(PLAN_REVIEW_GATE, "approved", "implement"),
            Transition::check_free(PLAN_REVIEW_GATE, "revise", "plan"),
            Transition::checked(
                "implement",
                "implementation-ready",
                IMPLEMENTATION_REVIEW_GATE,
            ),
            Transition::checked(IMPLEMENTATION_REVIEW_GATE, "approved", VALIDATION_GATE),
            Transition::check_free(IMPLEMENTATION_REVIEW_GATE, "revise", "implement"),
            Transition::checked(VALIDATION_GATE, "passed", "end"),
            Transition::check_free(VALIDATION_GATE, "revise", "implement"),
        ],
    )
}

/// Exact policy-document reference topology from PRD §11.2.
pub fn policy_document_workflow() -> Workflow {
    Workflow::new(
        "policy-document",
        "prepare",
        vec![
            State::new(
                "prepare",
                "Prepare",
                "Draft or revise the externally referenced document.",
                false,
            ),
            State::new(
                "deterministic-review",
                "Deterministic review",
                "Check mechanically testable document policies.",
                false,
            ),
            State::new(
                "semantic-review",
                "Semantic review",
                "Check the document's complete current conformance.",
                false,
            ),
            State::new("end", "End", "The document conforms to policy.", true),
        ],
        vec![
            Transition::check_free("prepare", "ready", "deterministic-review"),
            Transition::checked("deterministic-review", "passed", "semantic-review"),
            Transition::check_free("deterministic-review", "revise", "prepare"),
            Transition::checked("semantic-review", "passed", "end"),
            Transition::check_free("semantic-review", "revise", "prepare"),
        ],
    )
}

/// Build software-change immutable initial input.
pub fn software_change_initial_input(
    review_policies: Value,
    external_reference: Option<Value>,
) -> Value {
    let mut input = json!({"review_policies": review_policies});
    if let Some(reference) = external_reference {
        input["external_reference"] = reference;
    }
    input
}

/// Build software-change input with a deterministic provider behavior switch.
///
/// The switch is fixture-only and is useful for proving unsupported/failure
/// behavior without changing the provider association or workflow topology.
pub fn software_change_initial_input_with_behavior(
    review_policies: Value,
    external_reference: Option<Value>,
    behavior: &str,
) -> Value {
    let mut input = software_change_initial_input(review_policies, external_reference);
    input["fixture_behavior"] = Value::String(behavior.to_owned());
    input
}

/// Two materially different policy sets for the same software-change graph.
pub fn software_change_policy_set_a() -> Value {
    json!({
        (DESIGN_REVIEW_GATE): [
            {"id": "architecture", "description": "The design has a coherent architecture."},
            {"id": "compatibility", "description": "The design preserves compatibility."}
        ],
        (PLAN_REVIEW_GATE): [
            {"id": "coverage", "description": "The plan covers the requested change."}
        ],
        (IMPLEMENTATION_REVIEW_GATE): [
            {"id": "correctness", "description": "The implementation is correct."}
        ],
        (VALIDATION_GATE): [
            {"id": "regression", "description": "Validation covers regression risk."}
        ]
    })
}

/// A second policy configuration deliberately sharing no policy IDs with set A.
pub fn software_change_policy_set_b() -> Value {
    json!({
        (DESIGN_REVIEW_GATE): [
            {"id": "security-boundary", "description": "Security boundaries are explicit."}
        ],
        (PLAN_REVIEW_GATE): [
            {"id": "migration", "description": "The plan accounts for migration."},
            {"id": "rollback", "description": "The plan has a rollback path."}
        ],
        (IMPLEMENTATION_REVIEW_GATE): [
            {"id": "observability", "description": "Operational observability is included."}
        ],
        (VALIDATION_GATE): [
            {"id": "release", "description": "Release checks are documented."}
        ]
    })
}

/// Construct one externally produced software-change review evidence value.
pub fn software_change_review_evidence(
    gate: &str,
    policy_id: &str,
    passed: bool,
    findings: Value,
) -> Value {
    json!({
        "gate": gate,
        "policy_id": policy_id,
        "result": if passed { "pass" } else { "fail" },
        "findings": findings
    })
}

/// Construct a context record containing externally produced review evidence.
pub fn software_change_review_context(
    record_id: &str,
    gate: &str,
    policy_id: &str,
    passed: bool,
    findings: Value,
    sequence: u64,
) -> ContextRecord {
    ContextRecord::new(
        record_id,
        "review-evidence",
        software_change_review_evidence(gate, policy_id, passed, findings),
        sequence.into(),
        (sequence as i64).into(),
    )
}

/// Build one document policy using the fixture's small rule convention.
pub fn document_policy(id: &str, description: &str, rule: &str, value: &str) -> Value {
    json!({
        "id": id,
        "description": description,
        "rule": rule,
        "value": value
    })
}

/// Build policy-document initial input.  The document path/identity remains
/// in opaque input so a fresh actor can recover it from `show`.
pub fn policy_document_initial_input(
    mode: &str,
    document_path: impl AsRef<str>,
    deterministic_policies: Value,
    semantic_policies: Value,
    review_mode: &str,
) -> Value {
    json!({
        "mode": mode,
        "document": {
            "path": document_path.as_ref(),
            "identity": document_path.as_ref()
        },
        "deterministic_policies": deterministic_policies,
        "semantic_policies": semantic_policies,
        "review_mode": review_mode
    })
}

/// README-like policy input with a distinct policy shape.
pub fn readme_policy_input(document_path: impl AsRef<str>, mode: &str) -> Value {
    policy_document_initial_input(
        mode,
        document_path,
        json!([
            document_policy(
                "readme-heading",
                "The README names the project.",
                "required_text",
                "# Loop Engine"
            ),
            document_policy(
                "readme-start-command",
                "The README includes a getting-started command.",
                "required_text",
                "cargo test"
            )
        ]),
        json!([document_policy(
            "readme-purpose",
            "The README explains the product purpose.",
            "required_text",
            "workflow coordination"
        )]),
        "lineage-aware",
    )
}

/// AGENTS.md-like policy input with materially different policy IDs/rules.
pub fn agents_policy_input(document_path: impl AsRef<str>, mode: &str) -> Value {
    policy_document_initial_input(
        mode,
        document_path,
        json!([
            document_policy(
                "agents-scope",
                "Agent instructions identify the repository scope.",
                "required_text",
                "Repository scope"
            ),
            document_policy(
                "agents-validation",
                "Agent instructions identify validation commands.",
                "required_text",
                "Validation"
            )
        ]),
        json!([document_policy(
            "agents-handoff",
            "Agent instructions explain durable handoff.",
            "required_text",
            "durable handoff"
        )]),
        "independent",
    )
}

fn evaluate_software_change(request: EvaluateRequest) -> Result<Value, FixtureError> {
    request.validate_operation()?;
    let _ = request.workflow;
    let behavior = request
        .initial_input
        .get("fixture_behavior")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match behavior {
        "unsupported" => return Ok(json!({"result": "unsupported"})),
        "failure" => {
            return Ok(FixtureResponse::Failure(
                "software-change fixture requested a process failure".to_owned(),
            )
            .into_json())
        }
        "malformed" => {
            // This branch is converted to raw bytes by the executable wrapper;
            // direct callers receive the same invalid-protocol marker as data.
            return Ok(json!({"fixture_raw_response": "{"}));
        }
        _ => {}
    }

    let gate = software_change_gate(&request.transition);
    let Some(gate) = gate else {
        // Non-review checked edges are structurally checked by the provider
        // boundary but have no semantic review obligations in this fixture.
        return Ok(json!({"result": "allow"}));
    };

    let policies = parse_review_policies(&request.initial_input)?;
    let required = policies.get(gate).cloned().unwrap_or_default();
    let evidence = latest_review_evidence(&request.context);
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for policy in required {
        match evidence.get(&(gate.to_owned(), policy.id.clone())) {
            None => missing.push(policy),
            Some(entry) if !entry.passed => failed.push((policy, entry.findings.clone())),
            Some(_) => {}
        }
    }

    if missing.is_empty() && failed.is_empty() {
        return Ok(json!({"result": "allow"}));
    }

    let mut details = Map::new();
    details.insert("gate".to_owned(), Value::String(gate.to_owned()));
    details.insert(
        "missing".to_owned(),
        Value::Array(
            missing
                .iter()
                .map(|policy| json!({"id": policy.id, "description": policy.description}))
                .collect(),
        ),
    );
    details.insert(
        "failed".to_owned(),
        Value::Array(
            failed
                .iter()
                .map(|(policy, findings)| {
                    json!({
                        "id": policy.id,
                        "description": policy.description,
                        "findings": findings
                    })
                })
                .collect(),
        ),
    );

    let prior_denials = request
        .prior_evaluations
        .iter()
        .filter(|evaluation| evaluation.transition.same_lineage(&request.transition))
        .filter_map(|evaluation| evaluation.feedback())
        .map(|feedback| {
            json!({
                "code": feedback.code,
                "message": feedback.message,
                "details": feedback.details
            })
        })
        .collect::<Vec<_>>();
    if !prior_denials.is_empty() {
        details.insert("prior_denials".to_owned(), Value::Array(prior_denials));
    }

    let missing_names = missing
        .iter()
        .map(|policy| policy.id.as_str())
        .collect::<Vec<_>>();
    let failed_names = failed
        .iter()
        .map(|(policy, _)| policy.id.as_str())
        .collect::<Vec<_>>();
    let mut obligations = Vec::new();
    if !missing_names.is_empty() {
        obligations.push(format!("missing: {}", missing_names.join(", ")));
    }
    if !failed_names.is_empty() {
        obligations.push(format!("failed: {}", failed_names.join(", ")));
    }
    Ok(json!({
        "result": "deny",
        "feedback": {
            "code": "software-change-review-incomplete",
            "message": format!("{gate} review obligations are not satisfied ({})", obligations.join("; ")),
            "details": Value::Object(details)
        }
    }))
}

fn software_change_gate(transition: &Transition) -> Option<&'static str> {
    if !transition.kind.is_checked()
        || transition.event.as_str() != "approved" && transition.event.as_str() != "passed"
    {
        return None;
    }
    match transition.source.as_str() {
        DESIGN_REVIEW_GATE => Some(DESIGN_REVIEW_GATE),
        PLAN_REVIEW_GATE => Some(PLAN_REVIEW_GATE),
        IMPLEMENTATION_REVIEW_GATE => Some(IMPLEMENTATION_REVIEW_GATE),
        VALIDATION_GATE => Some(VALIDATION_GATE),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct ReviewPolicy {
    id: String,
    description: String,
}

#[derive(Clone, Debug)]
struct ReviewEvidence {
    passed: bool,
    findings: Value,
}

fn parse_review_policies(
    input: &Value,
) -> Result<BTreeMap<String, Vec<ReviewPolicy>>, FixtureError> {
    let Some(value) = input.get("review_policies") else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| FixtureError::new("review_policies must be an object grouped by gate"))?;
    let mut policies = BTreeMap::new();
    for (gate, values) in object {
        let values = values.as_array().ok_or_else(|| {
            FixtureError::new(format!("review policy gate `{gate}` must be an array"))
        })?;
        let mut parsed = Vec::new();
        for value in values {
            let policy = value.as_object().ok_or_else(|| {
                FixtureError::new(format!("policy in `{gate}` must be an object"))
            })?;
            let id = policy
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    FixtureError::new(format!("policy in `{gate}` requires a non-empty id"))
                })?;
            let description = policy
                .get("description")
                .and_then(Value::as_str)
                .filter(|description| !description.is_empty())
                .ok_or_else(|| {
                    FixtureError::new(format!("policy `{id}` in `{gate}` requires a description"))
                })?;
            parsed.push(ReviewPolicy {
                id: id.to_owned(),
                description: description.to_owned(),
            });
        }
        policies.insert(gate.clone(), parsed);
    }
    Ok(policies)
}

fn latest_review_evidence(context: &[ContextRecord]) -> BTreeMap<(String, String), ReviewEvidence> {
    let mut evidence = BTreeMap::new();
    let mut ordered = context.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|record| record.sequence);
    for record in ordered {
        if record.kind != "review-evidence" {
            continue;
        }
        let Some(object) = record.data.as_object() else {
            continue;
        };
        let Some(gate) = object.get("gate").and_then(Value::as_str) else {
            continue;
        };
        let Some(policy_id) = object.get("policy_id").and_then(Value::as_str) else {
            continue;
        };
        let passed = match object.get("result") {
            Some(Value::Bool(value)) => *value,
            Some(Value::String(value)) => matches!(value.as_str(), "pass" | "passed" | "allow"),
            _ => false,
        };
        let findings = object
            .get("findings")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        evidence.insert(
            (gate.to_owned(), policy_id.to_owned()),
            ReviewEvidence { passed, findings },
        );
    }
    evidence
}

fn evaluate_policy_document(request: EvaluateRequest) -> Result<Value, FixtureError> {
    request.validate_operation()?;
    let transition = request.transition;
    if !transition.kind.is_checked() {
        return Ok(json!({"result": "allow"}));
    }

    let mode = request
        .initial_input
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| FixtureError::new("policy-document input requires mode"))?;
    if !matches!(mode, "draft" | "audit") {
        return Ok(deny(
            "policy-document-invalid-mode",
            format!("unsupported policy-document mode `{mode}`"),
            json!({"mode": mode}),
        ));
    }

    let document = read_external_document(&request.initial_input)?;
    let deterministic = parse_policy_array(&request.initial_input, "deterministic_policies")?;
    let semantic = parse_policy_array(&request.initial_input, "semantic_policies")?;

    let mut findings = Vec::new();
    let mut deterministic_findings = Vec::new();
    if transition.source.as_str() == "deterministic-review" && transition.event.as_str() == "passed"
    {
        deterministic_findings = check_policies(&document, &deterministic, "deterministic")?;
        findings.extend(deterministic_findings.clone());
    } else if transition.source.as_str() == "semantic-review"
        && transition.event.as_str() == "passed"
    {
        // The final semantic review intentionally re-runs deterministic rules
        // against the current external document to catch regressions after a
        // previous deterministic pass.
        deterministic_findings = check_policies(&document, &deterministic, "deterministic")?;
        findings.extend(deterministic_findings.clone());
        findings.extend(check_policies(&document, &semantic, "semantic")?);
    }

    if findings.is_empty() {
        return Ok(json!({"result": "allow"}));
    }

    let review_mode = request
        .initial_input
        .get("review_mode")
        .and_then(Value::as_str)
        .unwrap_or("lineage-aware");
    let mut details = json!({
        "mode": mode,
        "findings": findings,
        "review_mode": review_mode
    });
    if review_mode != "independent" {
        let prior = request
            .prior_evaluations
            .iter()
            .filter(|evaluation| evaluation.transition.same_lineage(&transition))
            .filter_map(|evaluation| evaluation.feedback())
            .map(|feedback| {
                json!({"code": feedback.code, "message": feedback.message, "details": feedback.details})
            })
            .collect::<Vec<_>>();
        if !prior.is_empty() {
            details["prior_findings"] = Value::Array(prior);
        }
    }
    let code = if transition.source.as_str() == "deterministic-review"
        || !deterministic_findings.is_empty()
    {
        "policy-document-deterministic-failed"
    } else {
        "policy-document-semantic-failed"
    };
    let finding_ids = details["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|finding| finding.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    Ok(deny(
        code,
        format!(
            "{code}: the current document does not satisfy all required policies ({})",
            finding_ids.join(", ")
        ),
        details,
    ))
}

fn deny(code: &str, message: String, details: Value) -> Value {
    json!({
        "result": "deny",
        "feedback": {
            "code": code,
            "message": message,
            "details": details
        }
    })
}

fn read_external_document(input: &Value) -> Result<String, FixtureError> {
    let path = document_path(input).ok_or_else(|| {
        FixtureError::new("policy-document input requires document.path for external work")
    })?;
    fs::read_to_string(&path).map_err(|error| {
        FixtureError::new(format!(
            "could not read external document `{}`: {error}",
            path.display()
        ))
    })
}

fn document_path(input: &Value) -> Option<PathBuf> {
    for key in [
        "document",
        "target",
        "external_document",
        "external_reference",
    ] {
        let Some(value) = input.get(key) else {
            continue;
        };
        let path = value
            .as_str()
            .or_else(|| value.get("path").and_then(Value::as_str))
            .or_else(|| value.get("uri").and_then(Value::as_str));
        if let Some(path) = path {
            return Some(
                path.strip_prefix("file://")
                    .map_or_else(|| PathBuf::from(path), PathBuf::from),
            );
        }
    }
    None
}

fn parse_policy_array(input: &Value, key: &str) -> Result<Vec<Value>, FixtureError> {
    match input.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(policies)) => Ok(policies.clone()),
        Some(other) => Err(FixtureError::new(format!(
            "{key} must be an array, got {}",
            type_name(other)
        ))),
    }
}

fn check_policies(
    document: &str,
    policies: &[Value],
    phase: &str,
) -> Result<Vec<Value>, FixtureError> {
    let mut findings = Vec::new();
    for policy in policies {
        let object = policy
            .as_object()
            .ok_or_else(|| FixtureError::new(format!("{phase} policy must be an object")))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| FixtureError::new(format!("{phase} policy requires id")))?;
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(id);
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .or_else(|| {
                if object.contains_key("required_text") {
                    Some("required_text")
                } else if object.contains_key("forbidden_text") {
                    Some("forbidden_text")
                } else {
                    None
                }
            })
            .unwrap_or("required_text");
        let value = object
            .get("value")
            .and_then(Value::as_str)
            .or_else(|| object.get("required_text").and_then(Value::as_str))
            .or_else(|| object.get("forbidden_text").and_then(Value::as_str))
            .or_else(|| object.get("contains").and_then(Value::as_str))
            .ok_or_else(|| FixtureError::new(format!("policy `{id}` requires a string value")))?;

        let passes = match rule {
            "required_text" | "required_section" | "contains" | "contains_text"
            | "must_contain" => document.contains(value),
            "forbidden_text" | "absent" | "must_not_contain" => !document.contains(value),
            other => {
                findings.push(json!({
                    "id": id,
                    "description": description,
                    "message": format!("unsupported {phase} policy rule `{other}`")
                }));
                continue;
            }
        };
        if !passes {
            findings.push(json!({
                "id": id,
                "description": description,
                "message": format!("{phase} policy `{id}` is not satisfied by the current document"),
                "rule": rule,
                "value": value
            }));
        }
    }
    Ok(findings)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl FixtureResponse {
    fn into_json(self) -> Value {
        match self {
            Self::Json(value) => value,
            Self::Raw(raw) => json!({"fixture_raw_response": raw}),
            Self::Failure(message) => json!({"fixture_failure": message}),
        }
    }
}

/// Convert a fixture-only behavior marker to the special wire behavior used
/// by [`run_provider`].  This keeps malformed/process-failure controls out of
/// the provider protocol itself.
pub fn normalize_special_response(response: FixtureResponse) -> FixtureResponse {
    match response {
        FixtureResponse::Json(value) if value.get("fixture_raw_response").is_some() => {
            FixtureResponse::Raw("{".to_owned())
        }
        FixtureResponse::Json(value) if value.get("fixture_failure").is_some() => {
            FixtureResponse::Failure(
                value["fixture_failure"]
                    .as_str()
                    .unwrap_or("fixture failure")
                    .to_owned(),
            )
        }
        other => other,
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn software_change_topology_is_exact_and_run_input_independent() {
        let workflow = software_change_workflow();
        assert_eq!(workflow.initial_state.as_str(), "explore");
        assert_eq!(workflow.states.len(), 9);
        assert_eq!(workflow.transitions.len(), 12);
        assert!(
            workflow
                .states
                .iter()
                .find(|state| state.id.as_str() == "end")
                .unwrap()
                .is_final
        );
        assert_eq!(workflow, software_change_workflow());
    }

    #[test]
    fn policy_document_topology_is_exact() {
        let workflow = policy_document_workflow();
        assert_eq!(workflow.initial_state.as_str(), "prepare");
        assert_eq!(workflow.states.len(), 4);
        assert_eq!(workflow.transitions.len(), 5);
        assert!(
            workflow
                .states
                .iter()
                .find(|state| state.id.as_str() == "end")
                .unwrap()
                .is_final
        );
    }

    #[test]
    fn software_change_policy_and_evidence_are_opaque_json_conventions() {
        let policies = software_change_policy_set_a();
        let input =
            software_change_initial_input(policies.clone(), Some(json!({"workspace": "repo"})));
        assert_eq!(input["review_policies"], policies);
        let evidence = software_change_review_evidence(
            DESIGN_REVIEW_GATE,
            "architecture",
            false,
            json!(["clarify dependency boundary"]),
        );
        assert_eq!(evidence["gate"], DESIGN_REVIEW_GATE);
        assert_eq!(evidence["result"], "fail");
        assert!(input.get("prompt").is_none());
    }

    #[test]
    fn document_identity_is_carried_in_initial_input() {
        let input = readme_policy_input("/tmp/README.md", "draft");
        assert_eq!(input["mode"], "draft");
        assert_eq!(input["document"]["path"], "/tmp/README.md");
        assert!(input["deterministic_policies"].is_array());
    }
}
