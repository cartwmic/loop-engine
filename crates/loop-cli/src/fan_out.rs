//! Argv, stdin contracts, and Dagu-backed runtime for `fan-out`.
//!
//! Workers come only from repeated `--worker` JSON objects. Callers never
//! supply Dagu YAML. The facade does not open a database, encode a harness,
//! or emit a run-state envelope.

use crate::dagu::{names_for_capture_root, resolve_dagu, write_locator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ADHOC: AtomicU64 = AtomicU64::new(1);
const JOIN_COMMAND_OVERRIDE: &str = "LOOP_ENGINE_FAN_OUT_JOIN_COMMAND";
const SPEC_FILE: &str = "fan-out-spec.json";
const SUMMARY_FILE: &str = "summary.json";
const JOIN_COMPLETE_FILE: &str = "join-complete";
const ATTEMPTS_FILE: &str = "attempts.json";
const MAX_FULL_SCHEMA_ATTEMPTS: u32 = 2;
const RETRY_PROMPT: &str = "\n\n---\n\nSCHEMA-CONFORMANCE RETRY\nReturn only a schema-conforming reconsideration of the same frozen assignment. Do not add commentary or change the semantic review authority.\n\nFIRST_OUTPUT_BEGIN\n";
const RETRY_PROMPT_ERRORS: &str = "\nFIRST_OUTPUT_END\n\nEXACT_VALIDATION_ERRORS\n";
const RETRY_PROMPT_END: &str = "\n\nReturn only the schema-conforming reconsideration.\n";

/// Nested fan-out worker argv and optional, caller-supplied contracts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerCli {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) preamble: Option<String>,
    pub(crate) output_schema: Option<OutputSchema>,
    /// Additive complete JSON Schema contract. The field name is intentionally
    /// distinct from the legacy required-key `output_schema` contract.
    pub(crate) full_output_schema: Option<Value>,
}

/// The legacy output contract: presence of required top-level keys only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputSchema {
    pub(crate) required: Vec<String>,
}

/// Bound-worker stdin packet. Engine invoke keys plus optional routed context,
/// assignment selection, and generic standing identities.
///
/// Omitted `context` keeps today's compact `{artifact_root}` worker stdin.
/// A present `context` array is forwarded unmodified onto that compact JSON.
/// Fan-out does not interpret standing identities; accepting the engine-owned
/// field keeps review-slot packets compatible with the shared invoke envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvokePacket {
    pub(crate) run_id: String,
    pub(crate) slot_id: String,
    pub(crate) artifact_root: String,
    pub(crate) instruction_body: String,
    pub(crate) capture_dir: String,
    #[serde(default)]
    pub(crate) context: Option<Vec<Value>>,
    #[serde(default)]
    pub(crate) assignment_selection: Option<Vec<String>>,
    #[serde(default, rename = "standing_assignment_ids")]
    pub(crate) _standing_assignment_ids: Option<Vec<String>>,
}

/// Collected `fan-out` flags after the command name.  Zero `--worker` entries
/// are allowed at parse time; empty-worker execute fails closed.  Optional
/// `--max-active N` is omitted (uncapped) or a positive `u32`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FanOutArgs {
    pub(crate) workers: Vec<WorkerCli>,
    pub(crate) instructions_path: Option<PathBuf>,
    pub(crate) max_active: Option<u32>,
}

/// Bound (invoke-packet stdin) versus ad-hoc (`--instructions FILE`) fan-out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FanOutMode {
    Bound {
        packet: InvokePacket,
        workers: Vec<WorkerCli>,
    },
    AdHoc {
        instructions_path: PathBuf,
        workers: Vec<WorkerCli>,
    },
}

/// Parse failure for worker JSON, invoke packets, argv, or mode detection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// Collector failure: invalid caller input versus incomplete collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CollectorError {
    Invalid(String),
    Failed(String),
}

impl From<ParseError> for CollectorError {
    fn from(error: ParseError) -> Self {
        CollectorError::Invalid(error.to_string())
    }
}

/// JSON summary printed on collector success.  Not a run-state envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FanOutSummary {
    pub(crate) output_dir: String,
    pub(crate) workers: Vec<FanOutWorkerResult>,
}

/// One reaped worker in `--worker` order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FanOutWorkerResult {
    /// Stable within this invocation's slot; this is an assignment identity,
    /// not a worker role.
    pub(crate) assignment_id: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) exit_code: i32,
    pub(crate) stdout_path: String,
    pub(crate) stderr_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<ContractStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) conformance_error: Option<String>,
    /// Relative to the fan-out capture root. Present for retry ledgers and
    /// omitted for single-attempt workers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempts_path: Option<String>,
    /// The outer option preserves compatibility while the inner option
    /// serializes `null` whenever no output was selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) selected_attempt: Option<Option<u32>>,
    /// Digest and location of the selected attempt's originating stdout bytes.
    /// They serialize as null for a coverage gap.
    pub(crate) selected_output_sha256: Option<String>,
    pub(crate) selected_output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) declared_output_contract: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routed_inputs: Option<Value>,
}

/// Mechanical outcome for a worker that declared an output contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ContractStatus {
    Succeeded,
    Failed,
}

/// Durable per-invocation spec consumed by join and the facade fallback.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct FanOutSpec {
    workers: Vec<FanOutSpecWorker>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FanOutSpecWorker {
    #[serde(default)]
    assignment_id: String,
    command: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<OutputSchema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routed_inputs: Option<Value>,
    stdin_path: String,
    stdout_path: String,
    stderr_path: String,
    sidecar_path: String,
}

#[derive(Deserialize)]
struct SidecarFile {
    exit_code: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptCapture {
    number: u32,
    stdout_sha256: String,
    stderr_sha256: String,
    validation_errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptsManifest {
    schema_version: String,
    attempts: Vec<AttemptCapture>,
    selected_attempt: Option<u32>,
    exhausted: bool,
}

#[derive(Serialize)]
struct CaptureSummary<'a> {
    workers: &'a [FanOutWorkerResult],
}

/// Parse one nested worker CLI JSON object and validate its exact output shape.
/// Return the stable identities exposed by a frozen bound fan-out command.
/// Other commands remain opaque to the engine and therefore cannot be
/// selected safely.
pub(crate) fn enumerate_bound_assignments(
    binding: &loop_core::WorkSlotBinding,
) -> Option<Vec<String>> {
    if binding.args.first().map(String::as_str) != Some("fan-out") {
        return None;
    }
    let parsed = parse_fan_out_args(binding.args.iter().skip(1).map(String::as_str)).ok()?;
    if parsed.instructions_path.is_some() {
        return None;
    }
    Some(
        (0..parsed.workers.len())
            .map(assignment_id)
            .collect::<Vec<_>>(),
    )
}

fn assignment_id(index: usize) -> String {
    format!("worker-{index}")
}

pub(crate) fn parse_worker_cli_json(raw: &str) -> Result<WorkerCli, ParseError> {
    let raw_value: Value = serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "worker CLI JSON must contain string `command`, array-of-string `args`, and only optional string `preamble`, output_schema {{required}}, or full_output_schema: {error}"
        ))
    })?;
    if raw_value
        .as_object()
        .and_then(|object| object.get("full_output_schema"))
        .is_some_and(Value::is_null)
    {
        return Err(ParseError::new(
            "worker full_output_schema must be a JSON Schema value, not null",
        ));
    }
    let mut worker: WorkerCli = serde_json::from_value(raw_value).map_err(|error| {
        ParseError::new(format!(
            "worker CLI JSON must contain string `command`, array-of-string `args`, and only optional string `preamble`, output_schema {{required}}, or full_output_schema: {error}"
        ))
    })?;
    if let Some(schema) = &worker.output_schema {
        validate_output_schema(schema)?;
    }
    if let Some(schema) = worker.full_output_schema.as_mut() {
        normalize_full_output_schema(schema)?;
        validate_full_schema_declaration(schema)?;
    }
    Ok(worker)
}

fn validate_output_schema(schema: &OutputSchema) -> Result<(), ParseError> {
    if schema.required.is_empty() {
        return Err(ParseError::new(
            "worker output_schema.required must contain at least one key",
        ));
    }
    let mut seen = BTreeSet::new();
    for key in &schema.required {
        if key.is_empty() {
            return Err(ParseError::new(
                "worker output_schema.required keys must be non-empty strings",
            ));
        }
        if !seen.insert(key) {
            return Err(ParseError::new(format!(
                "worker output_schema.required contains duplicate key `{key}`"
            )));
        }
    }
    Ok(())
}

/// Accept the canonical direct-schema form and the equivalent explicit
/// `{schema, retry_limit: 1}` form. The retry count is deliberately not
/// configurable: a complete-schema worker gets at most one correction.
fn normalize_full_output_schema(schema: &mut Value) -> Result<(), ParseError> {
    // JSON Schema permits a boolean schema as well as an object schema.
    if schema.is_boolean() {
        return Ok(());
    }
    let Some(object) = schema.as_object() else {
        return Err(ParseError::new(
            "worker full_output_schema must be a JSON Schema value",
        ));
    };
    // `schema` and `retry_limit` are reserved for the explicit wrapper. Do
    // not let a misspelled or extended wrapper silently become an unrelated
    // JSON Schema whose unknown keywords would otherwise be ignored.
    let has_wrapper_key = object.contains_key("schema") || object.contains_key("retry_limit");
    if has_wrapper_key {
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "schema" | "retry_limit"))
        {
            return Err(ParseError::new(
                "worker full_output_schema wrapper may contain only schema and retry_limit",
            ));
        }
        let retry_limit = object.get("retry_limit").ok_or_else(|| {
            ParseError::new("worker full_output_schema wrapper requires retry_limit 1")
        })?;
        if retry_limit != &Value::from(1_u64) {
            return Err(ParseError::new(
                "worker full_output_schema retry_limit is fixed at 1",
            ));
        }
        let inner = object
            .get("schema")
            .ok_or_else(|| ParseError::new("worker full_output_schema wrapper requires schema"))?;
        if !inner.is_object() && !inner.is_boolean() {
            return Err(ParseError::new(
                "worker full_output_schema.schema must be a JSON Schema value",
            ));
        }
        *schema = inner.clone();
    }
    Ok(())
}

fn compile_full_schema(
    schema: &Value,
) -> Result<jsonschema::Validator, jsonschema::error::ValidationError<'static>> {
    jsonschema::options()
        .should_validate_formats(true)
        .build(schema)
}

fn validate_full_schema_declaration(schema: &Value) -> Result<(), ParseError> {
    compile_full_schema(schema)
        .map(|_| ())
        .map_err(|error| ParseError::new(format!("worker full_output_schema is invalid: {error}")))
}

/// Parse the five-key engine invoke packet from stdin JSON.
pub(crate) fn parse_invoke_packet(raw: &str) -> Result<InvokePacket, ParseError> {
    serde_json::from_str(raw).map_err(|error| {
        ParseError::new(format!(
            "invoke packet must be a JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, `capture_dir`, and optional `context`: {error}"
        ))
    })
}

/// Parse argv tokens after the `fan-out` command name.
///
/// Repeated `--worker JSON` (zero entries allowed).  Optional once:
/// `--instructions FILE`.  Optional once: `--max-active N` with decimal integer
/// N >= 1 that fits `u32`.  Unknown flags (including `--max-concurrency`) and
/// leftover positionals are errors.
pub(crate) fn parse_fan_out_args<I, S>(args: I) -> Result<FanOutArgs, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|token| token.as_ref().to_owned())
        .collect::<Vec<String>>();
    let mut workers = Vec::new();
    let mut instructions_path = None;
    let mut max_active = None;
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if let Some(raw) = strip_option(token, "--worker") {
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--worker")?.to_owned(),
            };
            workers.push(parse_worker_cli_json(&raw)?);
            continue;
        }
        if let Some(raw) = strip_option(token, "--instructions") {
            if instructions_path.is_some() {
                return Err(ParseError::new(
                    "`--instructions` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--instructions")?.to_owned(),
            };
            if raw.is_empty() {
                return Err(ParseError::new("option `--instructions` requires a value"));
            }
            instructions_path = Some(PathBuf::from(raw));
            continue;
        }
        if let Some(raw) = strip_option(token, "--max-active") {
            if max_active.is_some() {
                return Err(ParseError::new(
                    "`--max-active` may be supplied at most once",
                ));
            }
            let raw = match raw {
                Some(raw) => {
                    index += 1;
                    raw
                }
                None => option_value(&args, &mut index, "--max-active")?.to_owned(),
            };
            max_active = Some(parse_max_active(&raw)?);
            continue;
        }
        if token.starts_with('-') {
            return Err(ParseError::new(format!("unknown option `{token}`")));
        }
        return Err(ParseError::new(format!(
            "unexpected argument `{token}` for fan-out"
        )));
    }
    Ok(FanOutArgs {
        workers,
        instructions_path,
        max_active,
    })
}

/// Choose bound versus ad-hoc mode from parsed flags and stdin bytes.
///
/// Bound: stdin parses as an invoke packet and `--instructions` is absent.
/// Ad hoc: `--instructions FILE` is present and stdin is not a packet.
/// Combining a valid packet with `--instructions` is a parse error.
/// Ad hoc without `--instructions` (empty or non-packet stdin) is a parse error.
///
/// `--instructions FILE` is required for ad-hoc mode, but the path is not
/// opened here.  Missing-file failures are deferred to execute.
pub(crate) fn detect_mode(
    parsed_args: FanOutArgs,
    stdin_bytes: &[u8],
) -> Result<FanOutMode, ParseError> {
    let packet = std::str::from_utf8(stdin_bytes)
        .ok()
        .and_then(|raw| parse_invoke_packet(raw).ok());
    match (packet, parsed_args.instructions_path) {
        (Some(_), Some(_)) => Err(ParseError::new(
            "cannot combine an invoke packet on stdin with `--instructions`",
        )),
        (Some(packet), None) => Ok(FanOutMode::Bound {
            packet,
            workers: parsed_args.workers,
        }),
        (None, Some(instructions_path)) => Ok(FanOutMode::AdHoc {
            instructions_path,
            workers: parsed_args.workers,
        }),
        (None, None) => Err(ParseError::new(
            "ad-hoc fan-out requires `--instructions FILE`; bound fan-out requires an invoke packet on stdin",
        )),
    }
}

/// Bound fan-out reads a piped invoke packet. A terminal stdin is not a packet
/// source: draining it hangs interactive ad-hoc `--instructions FILE`.
pub(crate) fn drain_stdin(is_terminal: bool) -> bool {
    !is_terminal
}

/// Run Dagu-backed fan-out: emit an isolated type:graph, waitpid `dagu start`,
/// and return the JSON summary payload.
pub(crate) fn run_collector(
    parsed_args: FanOutArgs,
    stdin_bytes: &[u8],
    cwd: &Path,
) -> Result<FanOutSummary, CollectorError> {
    let max_active = parsed_args.max_active;
    let mode = detect_mode(parsed_args, stdin_bytes)?;
    match mode {
        FanOutMode::Bound { packet, workers } => {
            let (workers, assignment_ids) =
                select_bound_workers(&workers, packet.assignment_selection.as_deref())?;
            ensure_workers(&workers)?;
            if packet.capture_dir.is_empty() {
                return Err(CollectorError::Invalid(
                    "invoke packet capture_dir must be a non-empty path".to_owned(),
                ));
            }
            let dagu = resolve_dagu().map_err(|error| CollectorError::Failed(error.to_string()))?;
            let engine = loop_engine_exe()?;
            let artifact_root = absolute_from_cwd(cwd, Path::new(&packet.artifact_root));
            let artifact_root = path_to_string(&artifact_root);
            let payloads = workers
                .iter()
                .map(|worker| {
                    bound_worker_payload(worker, &artifact_root, packet.context.as_deref())
                })
                .collect::<Vec<_>>();
            let output_dir = absolute_from_cwd(cwd, Path::new(&packet.capture_dir));
            run_dagu_graph(
                &dagu,
                &engine,
                &workers,
                &assignment_ids,
                &payloads,
                &output_dir,
                max_active,
            )
        }
        FanOutMode::AdHoc {
            instructions_path,
            workers,
        } => {
            ensure_workers(&workers)?;
            let dagu = resolve_dagu().map_err(|error| CollectorError::Failed(error.to_string()))?;
            let engine = loop_engine_exe()?;
            let instructions_path = absolute_from_cwd(cwd, &instructions_path);
            let base_payload = fs::read(&instructions_path).map_err(|error| {
                CollectorError::Invalid(format!(
                    "could not read instructions file `{}`: {error}",
                    instructions_path.display()
                ))
            })?;
            let payloads = workers
                .iter()
                .map(|worker| ad_hoc_worker_payload(worker, &base_payload))
                .collect::<Vec<_>>();
            let unique = unique_adhoc_id();
            let output_dir = cwd.join("fan-out-adhoc").join(unique);
            let assignment_ids = (0..workers.len()).map(assignment_id).collect::<Vec<_>>();
            run_dagu_graph(
                &dagu,
                &engine,
                &workers,
                &assignment_ids,
                &payloads,
                &output_dir,
                max_active,
            )
        }
    }
}

/// Hidden `fan-out-join --capture-dir ABS`: write `summary.json` from spec and
/// sidecars. Invokes no model and does not append review-evidence.
pub(crate) fn run_fan_out_join(capture_dir: &Path) -> Result<(), CollectorError> {
    let spec = read_spec(capture_dir)?;
    let workers = summary_from_spec(&spec, false)?;
    write_summary_json(capture_dir, &workers)?;
    fs::write(capture_dir.join(JOIN_COMPLETE_FILE), b"complete\n").map_err(|error| {
        CollectorError::Failed(format!(
            "could not write fan-out join marker `{}`: {error}",
            capture_dir.join(JOIN_COMPLETE_FILE).display()
        ))
    })
}

/// Execute one complete-schema worker.  Dagu still owns graph scheduling;
/// this hidden step owns only the bounded same-worker conformance retry and
/// its immutable attempt capture.
pub(crate) fn run_fan_out_worker(
    capture_dir: &Path,
    worker_index: usize,
) -> Result<(), CollectorError> {
    let spec = read_spec(capture_dir)?;
    let worker = spec.workers.get(worker_index).ok_or_else(|| {
        CollectorError::Invalid(format!(
            "fan-out worker index {worker_index} is not present in {}",
            capture_dir.join(SPEC_FILE).display()
        ))
    })?;
    let Some(schema) = worker.full_output_schema.as_ref() else {
        return Err(CollectorError::Invalid(format!(
            "fan-out worker index {worker_index} has no full_output_schema"
        )));
    };
    let assignment = fs::read(&worker.stdin_path).map_err(|error| {
        CollectorError::Failed(format!(
            "could not read complete-schema worker assignment `{}`: {error}",
            worker.stdin_path
        ))
    })?;
    let worker_dir = Path::new(&worker.stdout_path).parent().ok_or_else(|| {
        CollectorError::Failed("complete-schema worker has no capture directory".to_owned())
    })?;
    let attempts_dir = worker_dir.join("attempts");
    fs::create_dir_all(&attempts_dir).map_err(|error| {
        CollectorError::Failed(format!(
            "could not create complete-schema attempts directory `{}`: {error}",
            attempts_dir.display()
        ))
    })?;

    let mut attempts = Vec::new();
    let mut selected_attempt = None;
    let mut selected_bytes = None;
    let mut next_assignment = assignment.clone();
    let mut last_exit_code = 1;

    for number in 1..=MAX_FULL_SCHEMA_ATTEMPTS {
        let (stdout, stderr, exit_code) =
            run_complete_schema_attempt(worker, &next_assignment, worker_dir)?;
        last_exit_code = exit_code;
        let validation_errors =
            complete_schema_validation_errors(&stdout, schema, worker.output_schema.as_ref());
        write_attempt_capture(worker_dir, number, &stdout, &stderr).map_err(|error| {
            CollectorError::Failed(format!(
                "could not write complete-schema attempt {number}: {error}"
            ))
        })?;
        attempts.push(AttemptCapture {
            number,
            stdout_sha256: sha256_digest(&stdout),
            stderr_sha256: sha256_digest(&stderr),
            validation_errors: validation_errors.clone(),
        });
        if validation_errors.is_empty() {
            selected_attempt = Some(number);
            selected_bytes = Some((stdout, stderr));
            break;
        }
        if number == 1 {
            next_assignment = retry_assignment(&assignment, &stdout, &validation_errors);
        }
        selected_bytes = Some((stdout, stderr));
    }

    let Some((stdout, stderr)) = selected_bytes else {
        return Err(CollectorError::Failed(
            "complete-schema worker produced no attempt".to_owned(),
        ));
    };
    fs::write(&worker.stdout_path, &stdout).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write selected complete-schema stdout `{}`: {error}",
            worker.stdout_path
        ))
    })?;
    fs::write(&worker.stderr_path, &stderr).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write selected complete-schema stderr `{}`: {error}",
            worker.stderr_path
        ))
    })?;

    let manifest = AttemptsManifest {
        schema_version: "1".to_owned(),
        attempts,
        selected_attempt,
        exhausted: selected_attempt.is_none(),
    };
    let manifest_path = worker_dir.join(ATTEMPTS_FILE);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        CollectorError::Failed(format!(
            "could not serialize complete-schema attempts `{}`: {error}",
            manifest_path.display()
        ))
    })?;
    fs::write(&manifest_path, manifest_bytes).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write complete-schema attempts `{}`: {error}",
            manifest_path.display()
        ))
    })?;
    write_sidecar_exit(&worker.sidecar_path, last_exit_code)
}

fn run_complete_schema_attempt(
    worker: &FanOutSpecWorker,
    assignment: &[u8],
    worker_dir: &Path,
) -> Result<(Vec<u8>, Vec<u8>, i32), CollectorError> {
    let mut command = Command::new(&worker.command);
    command
        .args(&worker.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("PI_CODING_AGENT_SESSION_DIR").is_none() {
        let sessions = worker_dir.join("sessions");
        fs::create_dir_all(&sessions).map_err(|error| {
            CollectorError::Failed(format!(
                "could not create complete-schema worker sessions `{}`: {error}",
                sessions.display()
            ))
        })?;
        let sessions = if sessions.is_absolute() {
            sessions
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    CollectorError::Failed(format!(
                        "could not resolve complete-schema worker session directory: {error}"
                    ))
                })?
                .join(sessions)
        };
        command.env("PI_CODING_AGENT_SESSION_DIR", sessions);
    }
    let mut child = command.spawn().map_err(|error| {
        CollectorError::Failed(format!(
            "could not spawn complete-schema worker `{}`: {error}",
            worker.command
        ))
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(assignment).map_err(|error| {
            CollectorError::Failed(format!(
                "could not write complete-schema worker stdin: {error}"
            ))
        })?;
    }
    let output = child.wait_with_output().map_err(|error| {
        CollectorError::Failed(format!(
            "could not wait for complete-schema worker `{}`: {error}",
            worker.command
        ))
    })?;
    Ok((
        output.stdout,
        output.stderr,
        process_exit_code(output.status),
    ))
}

fn process_exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return signal;
        }
    }
    1
}

fn write_attempt_capture(
    worker_dir: &Path,
    number: u32,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(), std::io::Error> {
    let attempt_dir = worker_dir.join("attempts").join(number.to_string());
    fs::create_dir_all(&attempt_dir)?;
    fs::write(attempt_dir.join("stdout"), stdout)?;
    fs::write(attempt_dir.join("stderr"), stderr)
}

fn write_sidecar_exit(path: &str, exit_code: i32) -> Result<(), CollectorError> {
    let sidecar = Path::new(path);
    if let Some(parent) = sidecar.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                CollectorError::Failed(format!(
                    "could not create sidecar directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
    }
    let bytes =
        serde_json::to_vec(&serde_json::json!({"exit_code": exit_code})).map_err(|error| {
            CollectorError::Failed(format!("could not serialize fan-out sidecar: {error}"))
        })?;
    fs::write(sidecar, bytes).map_err(|error| {
        CollectorError::Failed(format!("could not write fan-out sidecar `{path}`: {error}"))
    })
}

fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn retry_assignment(original_assignment: &[u8], first_stdout: &[u8], errors: &[String]) -> Vec<u8> {
    let mut assignment = original_assignment.to_vec();
    assignment.extend_from_slice(RETRY_PROMPT.as_bytes());
    assignment.extend_from_slice(first_stdout);
    assignment.extend_from_slice(RETRY_PROMPT_ERRORS.as_bytes());
    for error in errors {
        assignment.extend_from_slice(error.as_bytes());
        assignment.push(b'\n');
    }
    assignment.extend_from_slice(RETRY_PROMPT_END.as_bytes());
    assignment
}

fn complete_schema_validation_errors(
    stdout: &[u8],
    schema: &Value,
    legacy_schema: Option<&OutputSchema>,
) -> Vec<String> {
    let value = match locate_stdout_value(stdout) {
        Ok(value) => value,
        Err(error) => return vec![error],
    };
    let validator = match compile_full_schema(schema) {
        Ok(validator) => validator,
        Err(error) => return vec![format!("full_output_schema is invalid: {error}")],
    };
    let mut errors = validator
        .iter_errors(&value)
        .map(|error| {
            format!(
                "instance {}: {} (schema {})",
                error.instance_path(),
                error,
                error.schema_path()
            )
        })
        .collect::<Vec<_>>();
    if let Some(legacy_schema) = legacy_schema {
        match value.as_object() {
            Some(object) => {
                if let Err(error) = required_key_conformance(object, legacy_schema) {
                    errors.push(format!("legacy output_schema: {error}"));
                }
            }
            None => errors.push("legacy output_schema: stdout JSON must be an object".to_owned()),
        }
    }
    errors
}

// Retained as a small unit-test oracle for the legacy implementation tests;
// production complete-schema validation uses the full JSON Schema validator.
#[allow(dead_code)]
fn strip_option(token: &str, option: &str) -> Option<Option<String>> {
    if token == option {
        return Some(None);
    }
    let prefix = format!("{option}=");
    token.strip_prefix(&prefix).map(|raw| Some(raw.to_owned()))
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, ParseError> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| ParseError::new(format!("option `{option}` requires a value")))?;
    if value.starts_with('-') && value != "-" {
        return Err(ParseError::new(format!(
            "option `{option}` requires a value"
        )));
    }
    *index += 1;
    Ok(value)
}

fn parse_max_active(raw: &str) -> Result<u32, ParseError> {
    raw.parse::<u32>()
        .ok()
        .filter(|value| *value >= 1)
        .ok_or_else(|| {
            ParseError::new(format!(
                "`--max-active` requires a decimal integer >= 1, got `{raw}`"
            ))
        })
}

fn ensure_workers(workers: &[WorkerCli]) -> Result<(), CollectorError> {
    if workers.is_empty() {
        return Err(CollectorError::Invalid(
            "fan-out requires at least one `--worker`".to_owned(),
        ));
    }
    Ok(())
}

fn select_bound_workers(
    workers: &[WorkerCli],
    selection: Option<&[String]>,
) -> Result<(Vec<WorkerCli>, Vec<String>), CollectorError> {
    let all_ids = (0..workers.len()).map(assignment_id).collect::<Vec<_>>();
    let Some(selection) = selection else {
        return Ok((workers.to_vec(), all_ids));
    };
    if selection.is_empty() {
        return Err(CollectorError::Invalid(
            "assignment selection must contain at least one identity".to_owned(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    for identity in selection {
        if identity.is_empty() {
            return Err(CollectorError::Invalid(
                "assignment identities must be non-empty".to_owned(),
            ));
        }
        if !seen.insert(identity) {
            return Err(CollectorError::Invalid(format!(
                "assignment `{identity}` was selected more than once"
            )));
        }
        let Some(index) = all_ids.iter().position(|id| id == identity) else {
            return Err(CollectorError::Invalid(format!(
                "assignment `{identity}` is not in the bound fan-out"
            )));
        };
        selected.push((index, identity.clone()));
    }
    Ok((
        selected
            .iter()
            .map(|(index, _)| workers[*index].clone())
            .collect(),
        selected.into_iter().map(|(_, id)| id).collect(),
    ))
}

fn compact_location_json(artifact_root: &str, context: Option<&[Value]>) -> Vec<u8> {
    #[derive(Serialize)]
    struct Location<'a> {
        artifact_root: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<&'a [Value]>,
    }
    serde_json::to_vec(&Location {
        artifact_root,
        context,
    })
    .expect("serializing location object cannot fail")
}

fn bound_worker_payload(
    worker: &WorkerCli,
    artifact_root: &str,
    context: Option<&[Value]>,
) -> Vec<u8> {
    let location = compact_location_json(artifact_root, context);
    match &worker.preamble {
        Some(preamble) => compose_preamble_payload(preamble, Some(&location), b""),
        None => {
            let mut payload = location;
            payload.push(b'\n');
            payload
        }
    }
}

fn ad_hoc_worker_payload(worker: &WorkerCli, base: &[u8]) -> Vec<u8> {
    match &worker.preamble {
        Some(preamble) => compose_preamble_payload(preamble, None, base),
        None => base.to_vec(),
    }
}

fn compose_preamble_payload(preamble: &str, context: Option<&[u8]>, base: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        preamble.len() + context.map_or(0, |value| value.len() + 1) + 5 + base.len(),
    );
    payload.extend_from_slice(preamble.as_bytes());
    if !preamble.as_bytes().ends_with(b"\n") {
        payload.push(b'\n');
    }
    if let Some(context) = context {
        payload.extend_from_slice(context);
        payload.push(b'\n');
    }
    payload.extend_from_slice(b"---\n\n");
    payload.extend_from_slice(base);
    payload
}

fn unique_adhoc_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_ADHOC.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{sequence}", std::process::id())
}

fn absolute_from_cwd(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn loop_engine_exe() -> Result<PathBuf, CollectorError> {
    std::env::current_exe().map_err(|error| {
        CollectorError::Failed(format!("could not resolve loop-engine executable: {error}"))
    })
}

fn run_dagu_graph(
    dagu: &Path,
    engine: &Path,
    workers: &[WorkerCli],
    assignment_ids: &[String],
    payloads: &[Vec<u8>],
    output_dir: &Path,
    max_active: Option<u32>,
) -> Result<FanOutSummary, CollectorError> {
    debug_assert_eq!(workers.len(), payloads.len());
    debug_assert_eq!(workers.len(), assignment_ids.len());
    fs::create_dir_all(output_dir).map_err(|error| {
        CollectorError::Failed(format!(
            "could not create fan-out output directory `{}`: {error}",
            output_dir.display()
        ))
    })?;
    let output_dir = absolute_from_cwd(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        output_dir,
    );
    let (dag_name, run_name) = names_for_capture_root(&output_dir)
        .map_err(|error| CollectorError::Failed(error.to_string()))?;
    let locator = write_locator(&output_dir, &dag_name, &run_name)
        .map_err(|error| CollectorError::Failed(error.to_string()))?;
    let home = PathBuf::from(&locator.dagu_home);
    write_isolated_home_files(&home)?;
    let engine = fs::canonicalize(engine).unwrap_or_else(|_| engine.to_path_buf());
    let engine_str = path_to_string(&engine);

    let mut spec_workers = Vec::new();
    for (index, worker) in workers.iter().enumerate() {
        let worker_dir = output_dir.join(index.to_string());
        fs::create_dir_all(&worker_dir).map_err(|error| {
            CollectorError::Failed(format!(
                "could not create worker output directory `{}`: {error}",
                worker_dir.display()
            ))
        })?;
        let stdin_path = worker_dir.join("stdin");
        fs::write(&stdin_path, &payloads[index]).map_err(|error| {
            CollectorError::Failed(format!(
                "could not write worker stdin `{}`: {error}",
                stdin_path.display()
            ))
        })?;
        spec_workers.push(FanOutSpecWorker {
            assignment_id: assignment_ids[index].clone(),
            command: worker.command.clone(),
            args: worker.args.clone(),
            output_schema: worker.output_schema.clone(),
            full_output_schema: worker.full_output_schema.clone(),
            routed_inputs: serde_json::from_slice::<Value>(&payloads[index])
                .ok()
                .and_then(|value| value.get("context").cloned()),
            stdin_path: path_to_string(&stdin_path),
            stdout_path: path_to_string(&worker_dir.join("stdout")),
            stderr_path: path_to_string(&worker_dir.join("stderr")),
            sidecar_path: path_to_string(&worker_dir.join("inner_exit.json")),
        });
    }
    let spec = FanOutSpec {
        workers: spec_workers,
    };
    write_spec(&output_dir, &spec)?;

    let yaml = emit_graph_yaml(&dag_name, &engine_str, &output_dir, &spec, max_active);
    let dags_dir = home.join("dags");
    fs::create_dir_all(&dags_dir).map_err(|error| {
        CollectorError::Failed(format!(
            "could not create dagu dags directory `{}`: {error}",
            dags_dir.display()
        ))
    })?;
    let yaml_path = dags_dir.join(format!("{dag_name}.yaml"));
    fs::write(&yaml_path, yaml).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write fan-out DAG `{}`: {error}",
            yaml_path.display()
        ))
    })?;

    run_dagu_cli(
        dagu,
        &[
            "validate",
            "--dagu-home",
            &locator.dagu_home,
            &path_to_string(&yaml_path),
        ],
        false,
    )?;

    let start_result = run_dagu_cli(
        dagu,
        &[
            "start",
            "--quiet",
            "--dagu-home",
            &locator.dagu_home,
            "--name",
            &dag_name,
            "--run-id",
            &run_name,
            &path_to_string(&yaml_path),
        ],
        true,
    );
    // A completed graph can have a nonzero aggregate status when a nested
    // worker reports a nonzero exit through its sidecar. The join writes the
    // authoritative summary in that case. If no summary exists, the graph
    // itself failed before collection (or the join failed), which remains a
    // collector error after the fallback summary is written.
    let join_completed = output_dir.join(JOIN_COMPLETE_FILE).is_file();
    let workers = ensure_summary(&output_dir, &spec)?;
    let conformance_failed = workers
        .iter()
        .any(|worker| matches!(worker.status, Some(ContractStatus::Failed)));
    if start_result.is_err() && !join_completed {
        start_result?;
    }
    if conformance_failed {
        return Err(CollectorError::Failed(
            "one or more workers did not satisfy their declared output contract; inspect summary.json"
                .to_owned(),
        ));
    }
    Ok(FanOutSummary {
        output_dir: path_to_string(&output_dir),
        workers,
    })
}

fn run_dagu_cli(dagu: &Path, args: &[&str], allow_nonzero: bool) -> Result<(), CollectorError> {
    let output = Command::new(dagu)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            CollectorError::Failed(format!(
                "could not run `{} {}`: {error}",
                dagu.display(),
                args.join(" ")
            ))
        })?;
    if output.status.success() {
        return Ok(());
    }
    if allow_nonzero {
        let detail = dagu_failure_detail(&output);
        return Err(CollectorError::Failed(format!(
            "dagu {} did not complete successfully{detail}",
            args.first().copied().unwrap_or("command")
        )));
    }
    let detail = dagu_failure_detail(&output);
    Err(CollectorError::Failed(format!(
        "dagu {} failed{detail}",
        args.first().copied().unwrap_or("command")
    )))
}

fn dagu_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = stderr.trim();
    if !message.is_empty() {
        return format!(": {message}");
    }
    let message = stdout.trim();
    if !message.is_empty() {
        return format!(": {message}");
    }
    if let Some(code) = output.status.code() {
        format!(" (exit {code})")
    } else {
        " (terminated by signal)".to_owned()
    }
}

fn emit_graph_yaml(
    _dag_name: &str,
    engine: &str,
    capture_root: &Path,
    spec: &FanOutSpec,
    max_active: Option<u32>,
) -> String {
    let join_command = std::env::var_os(JOIN_COMMAND_OVERRIDE)
        .map(|value| path_to_string(&PathBuf::from(value)))
        .unwrap_or_else(|| engine.to_owned());
    let capture_dir = path_to_string(capture_root);
    let mut yaml = String::from("type: graph\n");
    if let Some(max_active) = max_active {
        yaml.push_str("max_active_steps: ");
        yaml.push_str(&max_active.to_string());
        yaml.push('\n');
    }
    yaml.push_str("steps:\n");
    let mut worker_names = Vec::new();
    for (index, worker) in spec.workers.iter().enumerate() {
        let name = format!("w{index}");
        worker_names.push(name.clone());
        yaml.push_str("  - name: ");
        yaml.push_str(&yaml_double_quoted(&name));
        yaml.push('\n');
        yaml.push_str("    action: exec\n");
        yaml.push_str("    with:\n");
        yaml.push_str("      command: ");
        yaml.push_str(&yaml_double_quoted(engine));
        yaml.push('\n');
        yaml.push_str("      args:\n");
        let mut args = if worker.full_output_schema.is_some() {
            vec![
                "fan-out-worker".to_owned(),
                "--capture-dir".to_owned(),
                capture_dir.clone(),
                "--worker-index".to_owned(),
                index.to_string(),
            ]
        } else {
            vec![
                "stdin-exec".to_owned(),
                "--stdin-file".to_owned(),
                worker.stdin_path.clone(),
                "--exit-mode".to_owned(),
                "sidecar".to_owned(),
                "--sidecar-file".to_owned(),
                worker.sidecar_path.clone(),
                "--".to_owned(),
                worker.command.clone(),
            ]
        };
        if worker.full_output_schema.is_none() {
            args.extend(worker.args.iter().cloned());
        }
        for arg in args {
            yaml.push_str("        - ");
            yaml.push_str(&yaml_double_quoted(&arg));
            yaml.push('\n');
        }
        yaml.push_str("    stdout: ");
        yaml.push_str(&yaml_double_quoted(&worker.stdout_path));
        yaml.push('\n');
        yaml.push_str("    stderr: ");
        yaml.push_str(&yaml_double_quoted(&worker.stderr_path));
        yaml.push('\n');
    }
    yaml.push_str("  - name: ");
    yaml.push_str(&yaml_double_quoted("join"));
    yaml.push('\n');
    yaml.push_str("    action: exec\n");
    yaml.push_str("    depends:\n");
    for name in &worker_names {
        yaml.push_str("      - ");
        yaml.push_str(&yaml_double_quoted(name));
        yaml.push('\n');
    }
    yaml.push_str("    with:\n");
    yaml.push_str("      command: ");
    yaml.push_str(&yaml_double_quoted(&join_command));
    yaml.push('\n');
    yaml.push_str("      args:\n");
    yaml.push_str("        - ");
    yaml.push_str(&yaml_double_quoted("fan-out-join"));
    yaml.push('\n');
    yaml.push_str("        - ");
    yaml.push_str(&yaml_double_quoted("--capture-dir"));
    yaml.push('\n');
    yaml.push_str("        - ");
    yaml.push_str(&yaml_double_quoted(&capture_dir));
    yaml.push('\n');
    yaml
}

fn yaml_double_quoted(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_isolated_home_files(home: &Path) -> Result<(), CollectorError> {
    let base = home.join("base.yaml");
    fs::write(&base, "type: graph\n").map_err(|error| {
        CollectorError::Failed(format!(
            "could not write isolated dagu base.yaml `{}`: {error}",
            base.display()
        ))
    })?;
    let config = home.join("config.yaml");
    fs::write(&config, "auth:\n  mode: none\n").map_err(|error| {
        CollectorError::Failed(format!(
            "could not write isolated dagu config.yaml `{}`: {error}",
            config.display()
        ))
    })
}

fn write_spec(output_dir: &Path, spec: &FanOutSpec) -> Result<(), CollectorError> {
    let path = output_dir.join(SPEC_FILE);
    let bytes = serde_json::to_vec_pretty(spec).map_err(|error| {
        CollectorError::Failed(format!(
            "could not serialize fan-out spec `{}`: {error}",
            path.display()
        ))
    })?;
    fs::write(&path, bytes).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write fan-out spec `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(())
}

fn read_spec(output_dir: &Path) -> Result<FanOutSpec, CollectorError> {
    let path = output_dir.join(SPEC_FILE);
    let bytes = fs::read(&path).map_err(|error| {
        CollectorError::Failed(format!(
            "could not read fan-out spec `{}`: {error}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CollectorError::Failed(format!(
            "could not parse fan-out spec `{}`: {error}",
            path.display()
        ))
    })
}

fn ensure_summary(
    output_dir: &Path,
    spec: &FanOutSpec,
) -> Result<Vec<FanOutWorkerResult>, CollectorError> {
    let existing = read_summary_workers(output_dir);
    if let Some(workers) = existing {
        if summary_covers_spec(spec, &workers) {
            return Ok(workers);
        }
    }
    // A failed graph can stop before every configured worker starts. Keep one
    // durable record per assignment so an outside observer sees explicit
    // coverage gaps rather than silently missing workers.
    let workers = summary_from_spec(spec, false)?;
    write_summary_json(output_dir, &workers)?;
    Ok(workers)
}

fn read_summary_workers(output_dir: &Path) -> Option<Vec<FanOutWorkerResult>> {
    let path = output_dir.join(SUMMARY_FILE);
    let bytes = fs::read(path).ok()?;
    #[derive(Deserialize)]
    struct Loaded {
        workers: Vec<FanOutWorkerResult>,
    }
    serde_json::from_slice::<Loaded>(&bytes)
        .ok()
        .map(|loaded| loaded.workers)
}

fn summary_covers_spec(spec: &FanOutSpec, workers: &[FanOutWorkerResult]) -> bool {
    workers.len() == spec.workers.len()
        && workers.iter().enumerate().all(|(index, worker)| {
            worker.assignment_id
                == if spec.workers[index].assignment_id.is_empty() {
                    assignment_id(index)
                } else {
                    spec.workers[index].assignment_id.clone()
                }
        })
}

fn worker_started(worker: &FanOutSpecWorker) -> bool {
    Path::new(&worker.stdout_path).is_file()
        || Path::new(&worker.stderr_path).is_file()
        || Path::new(&worker.sidecar_path).is_file()
}

fn summary_from_spec(
    spec: &FanOutSpec,
    started_only: bool,
) -> Result<Vec<FanOutWorkerResult>, CollectorError> {
    let mut results = Vec::new();
    for (worker_index, worker) in spec.workers.iter().enumerate() {
        let started = worker_started(worker);
        if started_only && !started {
            continue;
        }
        let exit_code = read_sidecar_exit(&worker.sidecar_path).unwrap_or(1);
        if !started {
            results.push(FanOutWorkerResult {
                assignment_id: if worker.assignment_id.is_empty() {
                    assignment_id(worker_index)
                } else {
                    worker.assignment_id.clone()
                },
                command: worker.command.clone(),
                args: worker.args.clone(),
                exit_code,
                stdout_path: worker.stdout_path.clone(),
                stderr_path: worker.stderr_path.clone(),
                status: (worker.output_schema.is_some() || worker.full_output_schema.is_some())
                    .then_some(ContractStatus::Failed),
                conformance_error: (worker.output_schema.is_some()
                    || worker.full_output_schema.is_some())
                .then(|| "worker did not start".to_owned()),
                attempts_path: worker
                    .full_output_schema
                    .as_ref()
                    .map(|_| format!("{worker_index}/{ATTEMPTS_FILE}")),
                selected_attempt: Some(None),
                selected_output_sha256: None,
                selected_output_path: None,
                declared_output_contract: worker.full_output_schema.clone().or_else(|| {
                    worker
                        .output_schema
                        .as_ref()
                        .map(|schema| serde_json::to_value(schema).unwrap_or(Value::Null))
                }),
                routed_inputs: worker.routed_inputs.clone(),
            });
            continue;
        }
        let stdout_path = Path::new(&worker.stdout_path);
        if let Some(parent) = stdout_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !stdout_path.exists() {
            let _ = fs::write(stdout_path, b"");
        }
        let stderr_path = Path::new(&worker.stderr_path);
        if !stderr_path.exists() {
            let _ = fs::write(stderr_path, b"");
        }
        let (
            status,
            conformance_error,
            selected_attempt,
            attempts_path,
            selected_output_sha256,
            selected_output_path,
        ) = if worker.full_output_schema.is_some() {
            match evaluate_full_contract(worker) {
                Ok(selected) => (
                    Some(ContractStatus::Succeeded),
                    None,
                    Some(Some(selected.number)),
                    Some(format!("{worker_index}/{ATTEMPTS_FILE}")),
                    Some(selected.sha256),
                    Some(selected.path),
                ),
                Err(error) => (
                    Some(ContractStatus::Failed),
                    Some(error),
                    Some(None),
                    Some(format!("{worker_index}/{ATTEMPTS_FILE}")),
                    None,
                    None,
                ),
            }
        } else {
            match &worker.output_schema {
                Some(schema) => match evaluate_output_conformance(stdout_path, schema) {
                    Ok(()) => {
                        let bytes = fs::read(stdout_path).map_err(|error| {
                            CollectorError::Failed(format!(
                                "could not read selected worker output `{}`: {error}",
                                stdout_path.display()
                            ))
                        })?;
                        (
                            Some(ContractStatus::Succeeded),
                            None,
                            Some(Some(1)),
                            None,
                            Some(sha256_digest(&bytes)),
                            Some(format!("{worker_index}/stdout")),
                        )
                    }
                    Err(error) => (
                        Some(ContractStatus::Failed),
                        Some(error),
                        Some(None),
                        None,
                        None,
                        None,
                    ),
                },
                None => {
                    let bytes = fs::read(stdout_path).map_err(|error| {
                        CollectorError::Failed(format!(
                            "could not read selected worker output `{}`: {error}",
                            stdout_path.display()
                        ))
                    })?;
                    (
                        None,
                        None,
                        Some(Some(1)),
                        None,
                        Some(sha256_digest(&bytes)),
                        Some(format!("{worker_index}/stdout")),
                    )
                }
            }
        };
        results.push(FanOutWorkerResult {
            assignment_id: if worker.assignment_id.is_empty() {
                assignment_id(worker_index)
            } else {
                worker.assignment_id.clone()
            },
            command: worker.command.clone(),
            args: worker.args.clone(),
            exit_code,
            stdout_path: worker.stdout_path.clone(),
            stderr_path: worker.stderr_path.clone(),
            status,
            conformance_error,
            attempts_path,
            selected_attempt,
            selected_output_sha256,
            selected_output_path,
            declared_output_contract: worker.full_output_schema.clone().or_else(|| {
                worker
                    .output_schema
                    .as_ref()
                    .map(|schema| serde_json::to_value(schema).unwrap_or(Value::Null))
            }),
            routed_inputs: worker.routed_inputs.clone(),
        });
    }
    Ok(results)
}

struct SelectedOutput {
    number: u32,
    sha256: String,
    path: String,
}

fn evaluate_full_contract(worker: &FanOutSpecWorker) -> Result<SelectedOutput, String> {
    let worker_dir = Path::new(&worker.stdout_path)
        .parent()
        .ok_or_else(|| "complete-schema worker has no capture directory".to_owned())?;
    let attempts_path = worker_dir.join(ATTEMPTS_FILE);
    let manifest: AttemptsManifest = serde_json::from_slice(
        &fs::read(&attempts_path)
            .map_err(|error| format!("could not read attempts manifest: {error}"))?,
    )
    .map_err(|error| format!("attempts manifest is malformed: {error}"))?;
    if manifest.schema_version != "1" {
        return Err(format!(
            "attempts manifest schema_version must be \"1\", got {:?}",
            manifest.schema_version
        ));
    }
    if manifest.attempts.is_empty() || manifest.attempts.len() > MAX_FULL_SCHEMA_ATTEMPTS as usize {
        return Err(format!(
            "attempts manifest must contain between 1 and {MAX_FULL_SCHEMA_ATTEMPTS} attempts"
        ));
    }
    let mut selected_bytes = None;
    for (position, attempt) in manifest.attempts.iter().enumerate() {
        let expected_number = position as u32 + 1;
        if attempt.number != expected_number {
            return Err(format!(
                "attempts manifest numbers must be consecutive starting at 1 (expected {expected_number}, got {})",
                attempt.number
            ));
        }
        let attempt_dir = worker_dir.join("attempts").join(attempt.number.to_string());
        let stdout = fs::read(attempt_dir.join("stdout")).map_err(|error| {
            format!("could not read attempt {} stdout: {error}", attempt.number)
        })?;
        let stderr = fs::read(attempt_dir.join("stderr")).map_err(|error| {
            format!("could not read attempt {} stderr: {error}", attempt.number)
        })?;
        if attempt.stdout_sha256 != sha256_digest(&stdout) {
            return Err(format!(
                "attempt {} stdout digest does not match raw bytes",
                attempt.number
            ));
        }
        if attempt.stderr_sha256 != sha256_digest(&stderr) {
            return Err(format!(
                "attempt {} stderr digest does not match raw bytes",
                attempt.number
            ));
        }
        let errors = complete_schema_validation_errors(
            &stdout,
            worker
                .full_output_schema
                .as_ref()
                .expect("full contract checked before evaluation"),
            worker.output_schema.as_ref(),
        );
        if errors != attempt.validation_errors {
            return Err(format!(
                "attempt {} validation_errors do not match deterministic validation",
                attempt.number
            ));
        }
        if manifest.selected_attempt == Some(attempt.number) {
            if !errors.is_empty() {
                return Err(format!(
                    "selected attempt {} is not schema-conforming: {}",
                    attempt.number,
                    errors.join("; ")
                ));
            }
            selected_bytes = Some((stdout, stderr));
        }
    }
    if manifest.exhausted {
        if manifest.attempts.len() != MAX_FULL_SCHEMA_ATTEMPTS as usize {
            return Err(format!(
                "exhausted attempts manifest must contain exactly {MAX_FULL_SCHEMA_ATTEMPTS} attempts"
            ));
        }
        if manifest
            .attempts
            .iter()
            .any(|attempt| attempt.validation_errors.is_empty())
        {
            return Err("exhausted attempts manifest contains a conforming attempt".to_owned());
        }
    } else {
        let Some(selected) = manifest.selected_attempt else {
            return Err("non-exhausted attempts manifest must select an attempt".to_owned());
        };
        if selected as usize != manifest.attempts.len() {
            return Err(
                "selected attempt must be the final captured attempt when not exhausted".to_owned(),
            );
        }
    }

    match (
        manifest.selected_attempt,
        manifest.exhausted,
        selected_bytes,
    ) {
        (Some(number), false, Some((stdout, stderr))) => {
            let compatibility_stdout = fs::read(&worker.stdout_path)
                .map_err(|error| format!("could not read selected stdout: {error}"))?;
            let compatibility_stderr = fs::read(&worker.stderr_path)
                .map_err(|error| format!("could not read selected stderr: {error}"))?;
            if compatibility_stdout != stdout || compatibility_stderr != stderr {
                return Err(format!(
                    "selected attempt {number} does not match compatibility stdout/stderr"
                ));
            }
            Ok(SelectedOutput {
                number,
                sha256: sha256_digest(&stdout),
                path: path_to_string(
                    &worker_dir
                        .join("attempts")
                        .join(number.to_string())
                        .join("stdout"),
                ),
            })
        }
        (None, true, None) => {
            let details = manifest
                .attempts
                .iter()
                .map(|attempt| {
                    if attempt.validation_errors.is_empty() {
                        format!("attempt {}: no validation error recorded", attempt.number)
                    } else {
                        format!(
                            "attempt {}: {}",
                            attempt.number,
                            attempt.validation_errors.join("; ")
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join(" | ");
            Err(format!(
                "full output schema conformance exhausted after {MAX_FULL_SCHEMA_ATTEMPTS} attempts: {details}"
            ))
        }
        _ => Err("attempts manifest selected_attempt/exhausted fields are inconsistent".to_owned()),
    }
}

fn read_sidecar_exit(path: &str) -> Option<i32> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<SidecarFile>(&bytes)
        .ok()
        .map(|sidecar| sidecar.exit_code)
}

fn evaluate_output_conformance(stdout_path: &Path, schema: &OutputSchema) -> Result<(), String> {
    let bytes = fs::read(stdout_path)
        .map_err(|error| format!("could not read captured stdout: {error}"))?;
    let object = locate_stdout_object(&bytes)?;
    required_key_conformance(&object, schema)
}

fn required_key_conformance(
    object: &Map<String, Value>,
    schema: &OutputSchema,
) -> Result<(), String> {
    let missing = schema
        .required
        .iter()
        .filter(|key| !object.contains_key(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stdout JSON object is missing required top-level keys: {}",
            missing.join(", ")
        ))
    }
}

fn locate_stdout_value(bytes: &[u8]) -> Result<Value, String> {
    let bare_error = match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };

    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("stdout is not valid UTF-8: {error}"))?;
    let lines = text.lines().collect::<Vec<_>>();
    let openings = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.trim() == "```json").then_some(index))
        .collect::<Vec<_>>();
    let opening = match openings.as_slice() {
        [] => {
            return Err(format!(
                "stdout bare JSON is malformed: {bare_error}; stdout contains no JSON fenced block"
            ))
        }
        [opening] => *opening,
        _ => return Err("stdout contains multiple JSON fenced blocks".to_owned()),
    };
    let closing = lines
        .iter()
        .enumerate()
        .skip(opening + 1)
        .find_map(|(index, line)| (line.trim() == "```").then_some(index))
        .ok_or_else(|| "stdout JSON fenced block is not closed".to_owned())?;
    let raw = lines[opening + 1..closing].join("\n");
    serde_json::from_str(&raw)
        .map_err(|error| format!("stdout JSON fenced block is malformed: {error}"))
}

fn locate_stdout_object(bytes: &[u8]) -> Result<Map<String, Value>, String> {
    let value = locate_stdout_value(bytes)?;
    if let Some(object) = value.as_object() {
        return Ok(object.clone());
    }
    if serde_json::from_slice::<Value>(bytes).is_ok() {
        Err("stdout JSON must be an object".to_owned())
    } else {
        Err("stdout JSON fenced block must contain an object".to_owned())
    }
}

fn write_summary_json(
    output_dir: &Path,
    workers: &[FanOutWorkerResult],
) -> Result<(), CollectorError> {
    let path = output_dir.join(SUMMARY_FILE);
    let bytes = serde_json::to_vec_pretty(&CaptureSummary { workers }).map_err(|error| {
        CollectorError::Failed(format!(
            "could not serialize capture summary `{}`: {error}",
            path.display()
        ))
    })?;
    fs::write(&path, bytes).map_err(|error| {
        CollectorError::Failed(format!(
            "could not write capture summary `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn valid_worker_json() -> &'static str {
        r#"{"command":"echo","args":["hello"]}"#
    }

    fn second_worker_json() -> &'static str {
        r#"{"command":"cat","args":["-"]}"#
    }

    fn valid_packet_json() -> &'static str {
        r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","capture_dir":"/tmp/captures/inv-1"}"#
    }

    #[test]
    fn valid_worker_json_parses_command_and_args() {
        let worker = parse_worker_cli_json(valid_worker_json()).expect("valid worker JSON");
        assert_eq!(worker.command, "echo");
        assert_eq!(worker.args, vec!["hello".to_owned()]);
        assert_eq!(worker.preamble, None);
        assert_eq!(worker.output_schema, None);
    }

    #[test]
    fn worker_json_accepts_optional_preamble_and_exact_output_schema() {
        let worker = parse_worker_cli_json(
            r#"{"command":"echo","args":[],"preamble":"role","output_schema":{"required":["axis","result"]}}"#,
        )
        .expect("contracted worker");
        assert_eq!(worker.preamble.as_deref(), Some("role"));
        assert_eq!(
            worker.output_schema.expect("schema").required,
            vec!["axis".to_owned(), "result".to_owned()]
        );
    }

    #[test]
    fn worker_json_rejects_malformed_output_schemas() {
        for raw in [
            r#"{"command":"echo","args":[],"full_output_schema":null}"#,
            r#"{"command":"echo","args":[],"output_schema":{}}"#,
            r#"{"command":"echo","args":[],"output_schema":{"required":[]}}"#,
            r#"{"command":"echo","args":[],"output_schema":{"required":[""]}}"#,
            r#"{"command":"echo","args":[],"output_schema":{"required":["a","a"]}}"#,
            r#"{"command":"echo","args":[],"output_schema":{"required":[1]}}"#,
            r#"{"command":"echo","args":[],"output_schema":{"required":["a"],"extra":true}}"#,
        ] {
            assert!(parse_worker_cli_json(raw).is_err(), "accepted {raw}");
        }
    }

    fn full_review_schema(axis: &str, author: &str) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["axis", "author", "result", "findings"],
            "properties": {
                "axis": {"type": "string", "const": axis},
                "author": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "kind"],
                    "properties": {
                        "name": {"type": "string", "const": author},
                        "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                    }
                },
                "result": {"type": "string", "enum": ["pass", "fail"]},
                "findings": {"type": "string"}
            },
            "oneOf": [
                {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
                {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}}
            ]
        })
    }

    #[test]
    fn full_schema_checks_assignment_constants_enums_types_and_relation() {
        let schema = full_review_schema("axis-a", "reviewer-a");
        let valid = json!({
            "axis": "axis-a",
            "author": {"name": "reviewer-a", "kind": "agent"},
            "result": "pass",
            "findings": ""
        });
        assert!(complete_schema_validation_errors(
            &serde_json::to_vec(&valid).expect("valid JSON"),
            &schema,
            None,
        )
        .is_empty());
        for invalid in [
            json!({"axis":"axis-b","author":{"name":"reviewer-a","kind":"agent"},"result":"pass","findings":""}),
            json!({"axis":"axis-a","author":{"name":"reviewer-b","kind":"agent"},"result":"pass","findings":""}),
            json!({"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent"},"result":"maybe","findings":""}),
            json!({"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent"},"result":"pass","findings":"not empty"}),
            json!({"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent"},"result":"fail","findings":[]}),
            json!({"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent","extra":true},"result":"pass","findings":""}),
        ] {
            let errors = complete_schema_validation_errors(
                &serde_json::to_vec(&invalid).expect("invalid JSON"),
                &schema,
                None,
            );
            assert!(
                !errors.is_empty(),
                "accepted invalid full review output: {invalid}"
            );
        }
    }

    #[test]
    fn full_schema_validates_all_declared_json_schema_constraints() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["code", "values"],
            "properties": {
                "code": {"type": "string", "pattern": "^ok-[0-9]+$"},
                "values": {
                    "type": "array",
                    "minItems": 2,
                    "uniqueItems": true,
                    "items": {"type": "integer", "minimum": 1}
                }
            }
        });
        let valid = br#"{"code":"ok-7","values":[1,2]}"#;
        assert!(complete_schema_validation_errors(valid, &schema, None).is_empty());
        let invalid = br#"{"code":"bad","values":[1,1],"extra":true}"#;
        let errors = complete_schema_validation_errors(invalid, &schema, None);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|error| error.contains("pattern")));
        assert!(errors.iter().any(|error| error.contains("unique")));
        assert!(errors
            .iter()
            .any(|error| error.contains("Additional properties")));
    }

    #[test]
    fn full_schema_retry_limit_is_fixed_at_one() {
        let schema = full_review_schema("axis-a", "reviewer-a");
        for wrapper in [
            json!({"schema": schema.clone(), "retry_limit": 2}),
            json!({"schema": schema.clone(), "retry_limit": 1, "extra": true}),
            json!({"retry_limit": 2, "type": "object"}),
        ] {
            let raw = json!({
                "command": "echo",
                "args": [],
                "full_output_schema": wrapper,
            })
            .to_string();
            let error = parse_worker_cli_json(&raw).expect_err("malformed retry wrapper");
            assert!(
                error.to_string().contains("retry_limit") || error.to_string().contains("wrapper"),
                "{error}"
            );
        }

        let raw = json!({
            "command": "echo",
            "args": [],
            "full_output_schema": {"schema": schema}
        })
        .to_string();
        let error = parse_worker_cli_json(&raw).expect_err("wrapper must declare retry_limit");
        assert!(error.to_string().contains("retry_limit"), "{error}");
    }

    #[test]
    fn worker_json_unknown_field_fails() {
        let raw = r#"{"command":"echo","args":["hello"],"extra":true}"#;
        assert!(parse_worker_cli_json(raw).is_err());
        for field in [
            "output_contract",
            "output_schema_full",
            "review_output_schema",
        ] {
            let raw = format!(r#"{{"command":"echo","args":[],"{field}":{{}}}}"#);
            assert!(
                parse_worker_cli_json(&raw).is_err(),
                "accepted alias {field}"
            );
        }
    }

    #[test]
    fn worker_json_non_object_fails() {
        assert!(parse_worker_cli_json(r#"["echo"]"#).is_err());
        assert!(parse_worker_cli_json("\"echo\"").is_err());
    }

    #[test]
    fn worker_json_non_string_command_fails() {
        assert!(parse_worker_cli_json(r#"{"command":1,"args":[]}"#).is_err());
    }

    #[test]
    fn worker_json_non_string_args_entry_fails() {
        assert!(parse_worker_cli_json(r#"{"command":"echo","args":[1]}"#).is_err());
    }

    #[test]
    fn valid_invoke_packet_parses_five_string_keys() {
        let packet = parse_invoke_packet(valid_packet_json()).expect("valid packet");
        assert_eq!(packet.run_id, "run-1");
        assert_eq!(packet.slot_id, "slot-1");
        assert_eq!(packet.artifact_root, "/tmp/artifacts");
        assert_eq!(packet.instruction_body, "Do the work");
        assert_eq!(packet.capture_dir, "/tmp/captures/inv-1");
        assert_eq!(packet.context, None);
    }

    #[test]
    fn invoke_packet_optional_context_deserializes_unmodified() {
        let raw = json!({
            "run_id": "run-1",
            "slot_id": "slot-1",
            "artifact_root": "/tmp/artifacts",
            "instruction_body": "Do the work",
            "capture_dir": "/tmp/captures/inv-1",
            "context": [{"id": "ctx-old", "kind": "kind-a", "data": {"rev": "1"}}],
            "standing_assignment_ids": ["worker-0"]
        })
        .to_string();
        let packet = parse_invoke_packet(&raw).expect("packet with context and standing IDs");
        assert_eq!(
            packet.context,
            Some(vec![json!({
                "id": "ctx-old",
                "kind": "kind-a",
                "data": {"rev": "1"}
            })])
        );
        assert_eq!(
            packet._standing_assignment_ids,
            Some(vec!["worker-0".to_owned()])
        );
    }

    #[test]
    fn invoke_packet_extra_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work","capture_dir":"/tmp/captures/inv-1","extra":"no"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_missing_key_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn invoke_packet_four_keys_without_capture_dir_fails() {
        let raw = r#"{"run_id":"run-1","slot_id":"slot-1","artifact_root":"/tmp/artifacts","instruction_body":"Do the work"}"#;
        assert!(parse_invoke_packet(raw).is_err());
    }

    #[test]
    fn zero_workers_parses() {
        let parsed = parse_fan_out_args(&[] as &[&str]).expect("zero --worker");
        assert!(parsed.workers.is_empty());
        assert!(parsed.instructions_path.is_none());
        assert!(parsed.max_active.is_none());
    }

    #[test]
    fn repeated_worker_flags_collect_in_order() {
        let parsed = parse_fan_out_args([
            "--worker",
            valid_worker_json(),
            "--worker",
            second_worker_json(),
        ])
        .expect("repeated --worker");
        assert_eq!(parsed.workers.len(), 2);
        assert_eq!(parsed.workers[0].command, "echo");
        assert_eq!(parsed.workers[0].args, vec!["hello".to_owned()]);
        assert_eq!(parsed.workers[1].command, "cat");
        assert_eq!(parsed.workers[1].args, vec!["-".to_owned()]);
        assert!(parsed.max_active.is_none());
    }

    #[test]
    fn max_active_n_is_parsed_and_omitted_stays_none() {
        let omitted = parse_fan_out_args(["--worker", valid_worker_json()]).expect("omitted");
        assert!(omitted.max_active.is_none());
        let set = parse_fan_out_args(["--max-active", "2"]).expect("--max-active 2");
        assert_eq!(set.max_active, Some(2));
        assert!(set.workers.is_empty());
        let equals = parse_fan_out_args(["--max-active=3"]).expect("--max-active=3");
        assert_eq!(equals.max_active, Some(3));
        let with_worker =
            parse_fan_out_args(["--worker", valid_worker_json(), "--max-active", "8"])
                .expect("worker plus --max-active");
        assert_eq!(with_worker.max_active, Some(8));
        assert_eq!(with_worker.workers.len(), 1);
    }

    #[test]
    fn max_active_zero_missing_non_integer_repeat_and_max_concurrency_fail() {
        assert!(parse_fan_out_args(["--max-active", "0"]).is_err());
        assert!(parse_fan_out_args(["--max-active"]).is_err());
        assert!(parse_fan_out_args(["--max-active", "nope"]).is_err());
        assert!(parse_fan_out_args(["--max-active", "1.5"]).is_err());
        assert!(parse_fan_out_args(["--max-active", "-1"]).is_err());
        assert!(parse_fan_out_args(["--max-active=-1"]).is_err());
        assert!(parse_fan_out_args(["--max-active", "2", "--max-active", "3"]).is_err());
        assert!(parse_fan_out_args(["--max-active=2", "--max-active=4"]).is_err());
        assert!(parse_fan_out_args(["--max-active", "4294967296"]).is_err());
        let unknown =
            parse_fan_out_args(["--max-concurrency", "2"]).expect_err("unknown --max-concurrency");
        assert!(unknown.to_string().contains("unknown option"), "{unknown}");
    }

    #[test]
    fn bound_mode_from_stdin_packet_without_instructions() {
        let parsed = parse_fan_out_args(["--worker", valid_worker_json()]).expect("args");
        let mode = detect_mode(parsed, valid_packet_json().as_bytes()).expect("bound mode");
        match mode {
            FanOutMode::Bound { packet, workers } => {
                assert_eq!(packet.run_id, "run-1");
                assert_eq!(workers.len(), 1);
                assert_eq!(workers[0].command, "echo");
            }
            FanOutMode::AdHoc { .. } => panic!("expected Bound mode"),
        }
    }

    #[test]
    fn ad_hoc_mode_from_instructions_when_stdin_is_not_a_packet() {
        let parsed = parse_fan_out_args(["--instructions", "/tmp/instructions.md"]).expect("args");
        let mode = detect_mode(parsed, b"not a packet").expect("ad-hoc mode");
        match mode {
            FanOutMode::AdHoc {
                instructions_path,
                workers,
            } => {
                assert_eq!(instructions_path, PathBuf::from("/tmp/instructions.md"));
                assert!(workers.is_empty());
            }
            FanOutMode::Bound { .. } => panic!("expected AdHoc mode"),
        }
    }

    #[test]
    fn valid_packet_combined_with_instructions_is_parse_error() {
        let parsed = parse_fan_out_args(["--instructions", "/tmp/instructions.md"]).expect("args");
        assert!(detect_mode(parsed, valid_packet_json().as_bytes()).is_err());
    }

    #[test]
    fn ad_hoc_missing_instructions_is_parse_error() {
        let empty = parse_fan_out_args(&[] as &[&str]).expect("zero flags");
        assert!(detect_mode(empty.clone(), b"").is_err());
        assert!(detect_mode(empty, b"not a packet").is_err());
    }

    #[test]
    fn terminal_stdin_is_not_drained() {
        assert!(
            !drain_stdin(true),
            "interactive stdin must not be drained to EOF"
        );
        assert!(drain_stdin(false), "piped stdin is the bound packet source");
    }

    #[test]
    fn bound_stdin_without_preamble_is_compact_artifact_root_json() {
        let worker = WorkerCli {
            command: "echo".to_owned(),
            args: Vec::new(),
            preamble: None,
            output_schema: None,
            full_output_schema: None,
        };
        assert_eq!(
            bound_worker_payload(&worker, "/tmp/artifacts", None),
            b"{\"artifact_root\":\"/tmp/artifacts\"}\n"
        );
        let schema_only = WorkerCli {
            output_schema: Some(OutputSchema {
                required: vec!["result".to_owned()],
            }),
            ..worker
        };
        assert_eq!(
            bound_worker_payload(&schema_only, "/absolute/root", None),
            b"{\"artifact_root\":\"/absolute/root\"}\n"
        );
    }

    #[test]
    fn bound_stdin_forwards_packet_context_unmodified_on_compact_json() {
        let worker = WorkerCli {
            command: "echo".to_owned(),
            args: Vec::new(),
            preamble: None,
            output_schema: None,
            full_output_schema: None,
        };
        let records = vec![
            json!({"id": "ctx-old", "kind": "kind-a", "data": {"rev": "1"}}),
            json!({"id": "ctx-new", "kind": "kind-a", "data": {"rev": "2"}}),
        ];
        let payload = bound_worker_payload(&worker, "/tmp/artifacts", Some(&records));
        let parsed: Value = serde_json::from_slice(&payload[..payload.len() - 1]).expect("json");
        assert_eq!(
            parsed,
            json!({
                "artifact_root": "/tmp/artifacts",
                "context": [
                    {"id": "ctx-old", "kind": "kind-a", "data": {"rev": "1"}},
                    {"id": "ctx-new", "kind": "kind-a", "data": {"rev": "2"}}
                ]
            })
        );
        assert_eq!(payload.last().copied(), Some(b'\n'));

        let with_preamble = WorkerCli {
            preamble: Some("role".to_owned()),
            ..worker
        };
        let framed = bound_worker_payload(&with_preamble, "/tmp/artifacts", Some(&records));
        let framed_text = String::from_utf8(framed).expect("utf8");
        assert!(framed_text.starts_with("role\n{"));
        assert!(framed_text.ends_with("\n---\n\n"));
        assert!(framed_text.contains("\"context\""));
        assert!(!framed_text.contains("instruction_body"));
    }

    #[test]
    fn opted_in_payloads_have_exact_bound_and_ad_hoc_framing() {
        let worker = WorkerCli {
            command: "echo".to_owned(),
            args: Vec::new(),
            preamble: Some("role".to_owned()),
            output_schema: None,
            full_output_schema: None,
        };
        assert_eq!(
            bound_worker_payload(&worker, "quoted/\"root\\tail", None),
            b"role\n{\"artifact_root\":\"quoted/\\\"root\\\\tail\"}\n---\n\n"
        );
        assert_eq!(
            ad_hoc_worker_payload(&worker, b"instructions-without-lf"),
            b"role\n---\n\ninstructions-without-lf"
        );

        let with_lf = WorkerCli {
            preamble: Some("role\n".to_owned()),
            ..worker.clone()
        };
        assert_eq!(
            ad_hoc_worker_payload(&with_lf, b"body"),
            b"role\n---\n\nbody"
        );
        let schema_only = WorkerCli {
            preamble: None,
            output_schema: Some(OutputSchema {
                required: vec!["result".to_owned()],
            }),
            ..worker
        };
        assert_eq!(ad_hoc_worker_payload(&schema_only, b"body"), b"body");
    }

    #[test]
    fn stdout_object_locator_accepts_bare_or_one_json_fence_only() {
        let bare = locate_stdout_object(br#" {"axis":"a","result":null} "#).expect("bare object");
        assert!(bare.contains_key("axis"));
        let fenced = locate_stdout_object(
            b"arbitrary prose\n```json\n{\"axis\":\"a\",\"result\":false}\n```\nmore prose\n",
        )
        .expect("fenced object");
        assert!(fenced.contains_key("result"));

        let trailing =
            locate_stdout_object(br#"{"axis":"a"}}"#).expect_err("trailing brace must fail");
        assert!(
            trailing.contains("trailing characters"),
            "retry error must preserve the exact bare-JSON parser diagnostic: {trailing}"
        );

        for stdout in [
            b"prose {\"axis\":\"a\"}".as_slice(),
            b"```json\n{bad}\n```".as_slice(),
            b"```json\n[]\n```".as_slice(),
            b"```json\n{\"a\":1}\n```\n```json\n{\"a\":2}\n```".as_slice(),
            b"```json\n{\"a\":1}".as_slice(),
        ] {
            assert!(locate_stdout_object(stdout).is_err(), "accepted {stdout:?}");
        }
    }

    #[test]
    fn conformance_checks_only_declared_top_level_key_presence() {
        let directory = tempdir().expect("tempdir");
        let stdout = directory.path().join("stdout");
        fs::write(&stdout, br#"{"axis":null,"result":{"anything":true}}"#).expect("stdout");
        let schema = OutputSchema {
            required: vec!["axis".to_owned(), "result".to_owned()],
        };
        evaluate_output_conformance(&stdout, &schema).expect("keys are present");
        let missing = OutputSchema {
            required: vec!["findings".to_owned()],
        };
        assert!(evaluate_output_conformance(&stdout, &missing)
            .expect_err("missing key")
            .contains("findings"));
    }

    #[test]
    fn zero_workers_fail_closed_in_bound_and_ad_hoc() {
        let directory = tempdir().expect("tempdir");
        let instructions = directory.path().join("instructions.txt");
        fs::write(&instructions, b"shared").expect("write instructions");
        let packet = json!({
            "run_id": "run-1",
            "slot_id": "slot-1",
            "artifact_root": directory.path().join("artifacts").to_string_lossy(),
            "instruction_body": "Do the work",
            "capture_dir": directory.path().join("captures").join("inv-1").to_string_lossy(),
        })
        .to_string();

        let bound = run_collector(
            FanOutArgs {
                workers: Vec::new(),
                instructions_path: None,
                max_active: None,
            },
            packet.as_bytes(),
            directory.path(),
        );
        assert!(matches!(bound, Err(CollectorError::Invalid(_))));

        let ad_hoc = run_collector(
            FanOutArgs {
                workers: Vec::new(),
                instructions_path: Some(instructions),
                max_active: None,
            },
            b"",
            directory.path(),
        );
        assert!(matches!(ad_hoc, Err(CollectorError::Invalid(_))));
    }

    #[test]
    fn emitted_yaml_is_type_graph_without_continue_on_or_retry() {
        let spec = FanOutSpec {
            workers: vec![FanOutSpecWorker {
                assignment_id: "worker-0".to_owned(),
                command: "echo".to_owned(),
                args: vec!["hi".to_owned()],
                output_schema: None,
                full_output_schema: None,
                routed_inputs: None,
                stdin_path: "/tmp/stdin".to_owned(),
                stdout_path: "/tmp/0/stdout".to_owned(),
                stderr_path: "/tmp/0/stderr".to_owned(),
                sidecar_path: "/tmp/0/inner_exit.json".to_owned(),
            }],
        };
        let yaml = emit_graph_yaml(
            "fanout-inv-1",
            "/abs/loop-engine",
            Path::new("/tmp/cap"),
            &spec,
            None,
        );
        assert!(yaml.starts_with("type: graph\nsteps:\n"), "{yaml}");
        assert!(yaml.contains("action: exec"), "{yaml}");
        assert!(yaml.contains("name: \"w0\""), "{yaml}");
        assert!(yaml.contains("name: \"join\""), "{yaml}");
        assert!(yaml.contains("stdin-exec"), "{yaml}");
        assert!(yaml.contains("fan-out-join"), "{yaml}");
        assert!(!yaml.contains("continue_on"), "{yaml}");
        assert!(!yaml.contains("retry_policy"), "{yaml}");
        assert!(!yaml.contains("max_active_steps"), "{yaml}");
        assert!(!yaml.contains("instruction_body"), "{yaml}");
    }

    #[test]
    fn max_active_two_emits_max_active_steps_two_and_join_depends_on_every_worker() {
        let spec = FanOutSpec {
            workers: vec![
                FanOutSpecWorker {
                    assignment_id: "worker-0".to_owned(),
                    command: "echo".to_owned(),
                    args: vec!["hi".to_owned()],
                    output_schema: None,
                    full_output_schema: None,
                    routed_inputs: None,
                    stdin_path: "/tmp/0/stdin".to_owned(),
                    stdout_path: "/tmp/0/stdout".to_owned(),
                    stderr_path: "/tmp/0/stderr".to_owned(),
                    sidecar_path: "/tmp/0/inner_exit.json".to_owned(),
                },
                FanOutSpecWorker {
                    assignment_id: "worker-1".to_owned(),
                    command: "cat".to_owned(),
                    args: vec!["-".to_owned()],
                    output_schema: None,
                    full_output_schema: None,
                    routed_inputs: None,
                    stdin_path: "/tmp/1/stdin".to_owned(),
                    stdout_path: "/tmp/1/stdout".to_owned(),
                    stderr_path: "/tmp/1/stderr".to_owned(),
                    sidecar_path: "/tmp/1/inner_exit.json".to_owned(),
                },
            ],
        };
        let yaml = emit_graph_yaml(
            "fanout-inv-1",
            "/abs/loop-engine",
            Path::new("/tmp/cap"),
            &spec,
            Some(2),
        );
        assert!(
            yaml.starts_with("type: graph\nmax_active_steps: 2\nsteps:\n"),
            "{yaml}"
        );
        assert!(yaml.contains("name: \"w0\""), "{yaml}");
        assert!(yaml.contains("name: \"w1\""), "{yaml}");
        assert!(yaml.contains("name: \"join\""), "{yaml}");
        let join = yaml.split("name: \"join\"").nth(1).expect("join step");
        assert!(
            join.contains("    depends:\n      - \"w0\"\n      - \"w1\"\n"),
            "{yaml}"
        );
        assert!(!yaml.contains("continue_on"), "{yaml}");
        assert!(!yaml.contains("retry_policy"), "{yaml}");
    }

    #[test]
    fn fallback_writes_summary_from_spec_and_sidecars() {
        let directory = tempdir().expect("tempdir");
        let worker_dir = directory.path().join("0");
        fs::create_dir_all(&worker_dir).expect("worker dir");
        let stdout = worker_dir.join("stdout");
        let stderr = worker_dir.join("stderr");
        let sidecar = worker_dir.join("inner_exit.json");
        fs::write(&stdout, b"out").expect("stdout");
        fs::write(&stderr, b"").expect("stderr");
        fs::write(&sidecar, br#"{"exit_code":3}"#).expect("sidecar");
        let unstarted_dir = directory.path().join("1");
        let spec = FanOutSpec {
            workers: vec![
                FanOutSpecWorker {
                    assignment_id: "worker-0".to_owned(),
                    command: "sh".to_owned(),
                    args: vec!["-c".to_owned(), "exit 3".to_owned()],
                    output_schema: None,
                    full_output_schema: None,
                    routed_inputs: None,
                    stdin_path: path_to_string(&worker_dir.join("stdin")),
                    stdout_path: path_to_string(&stdout),
                    stderr_path: path_to_string(&stderr),
                    sidecar_path: path_to_string(&sidecar),
                },
                FanOutSpecWorker {
                    assignment_id: "worker-1".to_owned(),
                    command: "never-started".to_owned(),
                    args: Vec::new(),
                    output_schema: Some(OutputSchema {
                        required: vec!["result".to_owned()],
                    }),
                    full_output_schema: None,
                    routed_inputs: None,
                    stdin_path: path_to_string(&unstarted_dir.join("stdin")),
                    stdout_path: path_to_string(&unstarted_dir.join("stdout")),
                    stderr_path: path_to_string(&unstarted_dir.join("stderr")),
                    sidecar_path: path_to_string(&unstarted_dir.join("inner_exit.json")),
                },
            ],
        };
        write_spec(directory.path(), &spec).expect("spec");
        let workers = ensure_summary(directory.path(), &spec).expect("summary");
        assert_eq!(workers.len(), 2);
        assert_eq!(workers[0].exit_code, 3);
        assert_eq!(workers[0].command, "sh");
        assert_eq!(workers[0].selected_attempt, Some(Some(1)));
        assert_eq!(
            workers[0].selected_output_sha256,
            Some(sha256_digest(b"out"))
        );
        assert_eq!(workers[0].selected_output_path.as_deref(), Some("0/stdout"));
        assert_eq!(workers[1].assignment_id, "worker-1");
        assert_eq!(workers[1].selected_attempt, Some(None));
        assert_eq!(workers[1].selected_output_sha256, None);
        assert_eq!(workers[1].selected_output_path, None);
        assert_eq!(workers[1].status, Some(ContractStatus::Failed));
        assert!(!unstarted_dir.exists());
        assert!(directory.path().join(SUMMARY_FILE).is_file());
    }
}
