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

const SCHEMA_VERSION: &str = "1";
const WORKFLOW_ID: &str = "software-change";
const ATTEMPTS_FILE: &str = "attempts.json";
const REVIEW_FIELDS: &[&str] = &["axis", "author", "result", "findings"];
const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];

/// The closed machine-readable result of `software-change review-candidates`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewCandidatesDocument {
    pub schema_version: &'static str,
    pub candidates: Vec<ReviewCandidate>,
}

/// One inert candidate or mechanical diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status")]
pub enum ReviewCandidate {
    #[serde(rename = "ready")]
    Ready {
        origin: CandidateOrigin,
        axis: String,
        author: CandidateAuthor,
        result: String,
        findings: String,
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

    let mut candidates = Vec::new();
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
            if contract.is_null() || !looks_like_review_contract(contract) {
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
            candidates.push(project_assignment(
                origin,
                worker,
                contract,
                capture_dir,
                worker_index,
            ));
        }
    }

    Ok(ReviewCandidatesDocument {
        schema_version: SCHEMA_VERSION,
        candidates,
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

fn project_assignment(
    origin: CandidateOrigin,
    worker: &Map<String, Value>,
    contract: &Value,
    capture_dir: Option<&str>,
    worker_index: usize,
) -> ReviewCandidate {
    let selected_attempt = match worker.get("selected_attempt") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64().and_then(|number| u32::try_from(number).ok()) {
            Some(number) if number > 0 => Some(number),
            Some(_) | None => {
                return ReviewCandidate::Unavailable {
                    origin,
                    diagnostic: "selected output metadata is unavailable".to_owned(),
                }
            }
        },
    };

    let Some(_selected_attempt) = selected_attempt else {
        return if reports_exhausted(capture_dir, worker_index) {
            ReviewCandidate::Exhausted {
                origin,
                diagnostic:
                    "review output exhausted conformance attempts without a selected attempt"
                        .to_owned(),
            }
        } else {
            ReviewCandidate::MissingSelection {
                origin,
                diagnostic: "assignment has no selected review output".to_owned(),
            }
        };
    };

    let Some(selected_digest) = worker.get("selected_output_sha256").and_then(Value::as_str) else {
        return ReviewCandidate::Unavailable {
            origin,
            diagnostic: "selected output metadata is unavailable".to_owned(),
        };
    };
    let Some(selected_path) = worker.get("selected_output_path").and_then(Value::as_str) else {
        return ReviewCandidate::Unavailable {
            origin,
            diagnostic: "selected output metadata is unavailable".to_owned(),
        };
    };
    if selected_digest.is_empty() || selected_path.is_empty() {
        return ReviewCandidate::Unavailable {
            origin,
            diagnostic: "selected output metadata is unavailable".to_owned(),
        };
    }

    let bytes = match read_selected_output(capture_dir, selected_path) {
        Ok(bytes) => bytes,
        Err(()) => {
            return ReviewCandidate::Unavailable {
                origin,
                diagnostic: "selected review output is unavailable".to_owned(),
            }
        }
    };
    if sha256_digest(&bytes) != selected_digest {
        return ReviewCandidate::Unavailable {
            origin,
            diagnostic: "selected review output digest does not match recorded digest".to_owned(),
        };
    }

    let value = match parse_selected_value(&bytes) {
        Ok(value) => value,
        Err(diagnostic) => {
            return ReviewCandidate::Malformed { origin, diagnostic };
        }
    };
    match normalize_review_output(contract, &value) {
        Ok(judgment) => ReviewCandidate::Ready {
            origin,
            axis: judgment.axis,
            author: judgment.author,
            result: judgment.result,
            findings: judgment.findings,
        },
        Err(diagnostic) => ReviewCandidate::Malformed { origin, diagnostic },
    }
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
        object.get("schema_version").and_then(Value::as_str) == Some(SCHEMA_VERSION)
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

fn looks_like_review_contract(contract: &Value) -> bool {
    let Some(object) = contract.as_object() else {
        return false;
    };
    // Legacy required-key presence contracts are deliberately not eligible:
    // this projection normalizes only a declared full review schema.
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        return false;
    };
    REVIEW_FIELDS
        .iter()
        .all(|field| properties.contains_key(*field))
}

struct NormalizedJudgment {
    axis: String,
    author: CandidateAuthor,
    result: String,
    findings: String,
}

fn normalize_review_output(contract: &Value, value: &Value) -> Result<NormalizedJudgment, String> {
    let mut violations = Vec::new();
    validate_contract_instance(contract, value, "$", &mut violations);

    let Some(object) = value.as_object() else {
        violations.push("review output must be a JSON object".to_owned());
        return Err(malformed_diagnostic(&violations));
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
            axis: axis.expect("validated axis"),
            author: author.expect("validated author"),
            result: result.expect("validated result"),
            findings: findings.expect("validated findings"),
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
                axis: "axis".to_owned(),
                author: CandidateAuthor {
                    name: "reviewer".to_owned(),
                    kind: "agent".to_owned(),
                },
                result: "pass".to_owned(),
                findings: String::new(),
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
                ReviewCandidate::Malformed { .. } => "malformed",
                ReviewCandidate::Unavailable { .. } => "unavailable",
                ReviewCandidate::MissingSelection { .. } => "missing-selection",
                ReviewCandidate::Exhausted { .. } => "exhausted",
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
