//! Deterministic per-run profile construction from embedded software-change data.
//!
//! `setup` assembles caller-supplied worker commands with a shipped profile. It
//! performs no workflow start or worker launch. The only intentional file write
//! is the atomically replaced profile named by `--output`.

use crate::{config, embedded_data, overlay, workflow};
use loop_core::{AdviceCommandConfig, AdviceDeparture, AdviceDepartureMap};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const REVIEW_GATES: &[&str] = &[
    "intent-review",
    "intent-adversarial-review",
    "design-review",
    "design-adversarial-review",
    "plan-review",
    "plan-adversarial-review",
    "implementation-review",
    "implementation-adversarial-review",
    "validation-review",
    "validation-adversarial-review",
];
const PROFILE_PREFIX: &str = "crates/software-change-provider/data/configs/";
const LEGACY_SHIPPED_PROFILE_VERSIONS: &[&str] = &[
    "minimal-10",
    "standard-10",
    "high-rigor-10",
    "minimal-11",
    "standard-11",
    "high-rigor-11",
];
const PREAMBLE_PATH: &str = "crates/software-change-provider/data/review-worker-preamble.txt";
const OUTPUT_SCHEMA_PATH: &str =
    "crates/software-change-provider/data/review-worker-output-schema-v2.json";
const REVIEW_CONCURRENCY: &str = "2";
const IMPLEMENTATION_CONCURRENCY: &str = "1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ReviewTokenBudget {
    model_id: String,
    context_window_tokens: u64,
    system_tokens: u64,
    framing_tokens: u64,
    output_reserve_tokens: u64,
    reasoning_reserve_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewRosterEntry {
    author: String,
    command: String,
    args: Vec<String>,
    token_budget: ReviewTokenBudget,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DraftWorkerInput {
    command: String,
    args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImplementationInput {
    command: String,
    args: Vec<String>,
    working_directory: String,
}

#[derive(Clone, Debug)]
struct SetupArgs {
    profile_source: ProfileSource,
    roster_path: PathBuf,
    engine: String,
    provider: String,
    output: PathBuf,
    bookends: bool,
    draft_worker_path: Option<PathBuf>,
    implementation_path: Option<PathBuf>,
    advice_config_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rigor {
    Minimal,
    Standard,
    High,
}

impl Rigor {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "minimal" => Ok(Self::Minimal),
            "standard" => Ok(Self::Standard),
            "high" => Ok(Self::High),
            other => Err(format!(
                "invalid --rigor `{other}`; expected minimal, standard, or high"
            )),
        }
    }

    fn profile_name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::High => "high-rigor",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Debug)]
enum ProfileSource {
    Bundled(Rigor),
    File(PathBuf),
}

impl ProfileSource {
    fn display_name(&self) -> &'static str {
        match self {
            Self::Bundled(rigor) => rigor.as_str(),
            Self::File(_) => "custom",
        }
    }
}

/// Parse setup arguments and run the non-run-state setup command.
pub fn run_from_args(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        if args.len() == 1 {
            println!("{}", usage());
            return 0;
        }
        eprintln!("setup help accepts no additional arguments; {}", usage());
        return 2;
    }

    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("setup: {error}; {}", usage());
            return 2;
        }
    };

    match build(parsed) {
        Ok(report) => match serde_json::to_writer(io::stdout(), &report) {
            Ok(()) => {
                println!();
                0
            }
            Err(error) => {
                eprintln!("setup could not write report: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("setup failed: {error}");
            1
        }
    }
}

fn usage() -> &'static str {
    "usage: software-change setup (--rigor minimal|standard|high | --profile PATH) --roster PATH --engine ABS --provider ABS --output PATH (--advice-config PATH | --decline-advice) [--bookends] [--draft-worker PATH] [--implementation PATH]\n\nThe standalone minimal, standard, and high-rigor examples expose the same eight advice occasions: review candidates, accepted defects, implementation correction, execution/authority issues, evidence applicability, requirements reconciliation, review-round departure, and final completion. Choose exactly one: --advice-config PATH reads a closed command/argv/timeout/request-limit/response-limit JSON object; --decline-advice freezes advice disabled. Setup does not select or call a backend."
}

fn parse_args(args: &[String]) -> Result<SetupArgs, String> {
    let mut rigor = None;
    let mut profile_path = None;
    let mut roster_path = None;
    let mut engine = None;
    let mut provider = None;
    let mut output = None;
    let mut bookends = false;
    let mut draft_worker_path = None;
    let mut implementation_path = None;
    let mut advice_config_path = None;
    let mut decline_advice = false;
    let mut index = 0;

    while index < args.len() {
        let token = &args[index];
        if token == "--bookends" {
            if bookends {
                return Err("--bookends may be supplied once".to_owned());
            }
            bookends = true;
            index += 1;
            continue;
        }
        if token == "--decline-advice" {
            if decline_advice {
                return Err("--decline-advice may be supplied once".to_owned());
            }
            decline_advice = true;
            index += 1;
            continue;
        }

        let (name, inline) = if let Some(value) = token.strip_prefix("--rigor=") {
            ("--rigor", Some(value.to_owned()))
        } else if token == "--rigor" {
            ("--rigor", None)
        } else if let Some(value) = token.strip_prefix("--profile=") {
            ("--profile", Some(value.to_owned()))
        } else if token == "--profile" {
            ("--profile", None)
        } else if let Some(value) = token.strip_prefix("--roster=") {
            ("--roster", Some(value.to_owned()))
        } else if token == "--roster" {
            ("--roster", None)
        } else if let Some(value) = token.strip_prefix("--engine=") {
            ("--engine", Some(value.to_owned()))
        } else if token == "--engine" {
            ("--engine", None)
        } else if let Some(value) = token.strip_prefix("--provider=") {
            ("--provider", Some(value.to_owned()))
        } else if token == "--provider" {
            ("--provider", None)
        } else if let Some(value) = token.strip_prefix("--output=") {
            ("--output", Some(value.to_owned()))
        } else if token == "--output" {
            ("--output", None)
        } else if let Some(value) = token.strip_prefix("--draft-worker=") {
            ("--draft-worker", Some(value.to_owned()))
        } else if token == "--draft-worker" {
            ("--draft-worker", None)
        } else if let Some(value) = token.strip_prefix("--implementation=") {
            ("--implementation", Some(value.to_owned()))
        } else if token == "--implementation" {
            ("--implementation", None)
        } else if let Some(value) = token.strip_prefix("--advice-config=") {
            ("--advice-config", Some(value.to_owned()))
        } else if token == "--advice-config" {
            ("--advice-config", None)
        } else {
            return Err(format!("unknown or unexpected argument `{token}`"));
        };

        let value = match inline {
            Some(value) => value,
            None => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("option `{name}` requires a value"))?;
                if value.starts_with('-') && value != "-" {
                    return Err(format!("option `{name}` requires a value"));
                }
                index += 1;
                value.clone()
            }
        };

        match name {
            "--rigor" => set_once(&mut rigor, Rigor::parse(&value)?, name)?,
            "--profile" => set_once(&mut profile_path, PathBuf::from(value), name)?,
            "--roster" => set_once(&mut roster_path, PathBuf::from(value), name)?,
            "--engine" => set_once(&mut engine, value, name)?,
            "--provider" => set_once(&mut provider, value, name)?,
            "--output" => set_once(&mut output, PathBuf::from(value), name)?,
            "--draft-worker" => set_once(&mut draft_worker_path, PathBuf::from(value), name)?,
            "--implementation" => set_once(&mut implementation_path, PathBuf::from(value), name)?,
            "--advice-config" => set_once(&mut advice_config_path, PathBuf::from(value), name)?,
            _ => unreachable!(),
        }
        index += 1;
    }

    let output = output.ok_or("missing required option `--output`")?;
    if output.as_os_str().is_empty() {
        return Err("--output must name a file".to_owned());
    }

    if decline_advice == advice_config_path.is_some() {
        return Err("choose exactly one of --advice-config JSON or --decline-advice".to_owned());
    }

    let profile_source = match (rigor, profile_path) {
        (Some(_), Some(_)) => {
            return Err("`--profile` and `--rigor` are mutually exclusive".to_owned())
        }
        (Some(rigor), None) => ProfileSource::Bundled(rigor),
        (None, Some(path)) if !path.as_os_str().is_empty() => ProfileSource::File(path),
        (None, Some(_)) => return Err("--profile must name a file".to_owned()),
        (None, None) => return Err("one of `--profile` or `--rigor` is required".to_owned()),
    };

    Ok(SetupArgs {
        profile_source,
        roster_path: roster_path.ok_or("missing required option `--roster`")?,
        engine: absolute_command(engine, "--engine")?,
        provider: absolute_command(provider, "--provider")?,
        output,
        bookends,
        draft_worker_path,
        implementation_path,
        advice_config_path,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{name} may be supplied once"));
    }
    *slot = Some(value);
    Ok(())
}

fn absolute_command(value: Option<String>, name: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("missing required option `{name}`"))?;
    if !Path::new(&value).is_absolute() {
        return Err(format!("{name} must be an absolute command path"));
    }
    Ok(value)
}

#[derive(Serialize)]
struct SetupReport {
    status: &'static str,
    rigor: &'static str,
    profile_selection: Value,
    bookends_enabled: bool,
    enablement: Value,
    effective_policy: Value,
    effective_bindings: Value,
    roster: Vec<ReviewRosterEntry>,
    output_path: String,
    output_byte_length: usize,
    output_sha256: String,
    output_sha256_digest: String,
    output_bytes: String,
    advice_configuration: Value,
    preview: Value,
    started: bool,
}

fn build(args: SetupArgs) -> Result<SetupReport, String> {
    let roster = read_roster(&args.roster_path)?;
    let draft_worker = args
        .draft_worker_path
        .as_deref()
        .map(read_draft_worker)
        .transpose()?;
    let implementation = args
        .implementation_path
        .as_deref()
        .map(read_implementation)
        .transpose()?;
    let advice_config = args
        .advice_config_path
        .as_deref()
        .map(read_advice_config)
        .transpose()?;
    validate_command_paths(&args)?;

    let selected = load_selected_profile(&args.profile_source)?;
    let profile = selected.profile.clone();
    refuse_legacy_shipped_profile(&profile)?;
    let mut output_profile = profile.clone();
    if args.bookends {
        enable_bookends(&mut output_profile)?;
    }
    validate_profile(&output_profile, "output profile")?;

    let effective_profile = if args.bookends {
        overlay::apply(&output_profile)
    } else {
        output_profile.clone()
    };
    validate_profile(&effective_profile, "effective profile")?;

    let preamble = embedded_text(PREAMBLE_PATH)?;
    let output_schema = embedded_json(OUTPUT_SCHEMA_PATH)?;
    let bindings = build_bindings(
        &effective_profile,
        &roster,
        &args.engine,
        &args.provider,
        &preamble,
        &output_schema,
        draft_worker.as_ref(),
        implementation.as_ref(),
    )?;
    output_profile["work_slot_bindings"] = bindings.clone();
    let advice_occasions = advice_occasions(&output_profile)?;
    let advice_departures = if advice_config.is_some() {
        Some(build_advice_departures(&output_profile, &advice_occasions)?)
    } else {
        None
    };
    if let Some(config) = &advice_config {
        output_profile["advice_command"] = serde_json::to_value(config)
            .map_err(|error| format!("could not encode advice command config: {error}"))?;
        output_profile["advice_departures"] = serde_json::to_value(
            advice_departures
                .as_ref()
                .expect("configured advice has departures"),
        )
        .map_err(|error| format!("could not encode advice departure map: {error}"))?;
    }
    validate_profile(&output_profile, "effective output profile")?;

    let profile_bytes = profile_bytes(&output_profile)?;
    let preview = preview_bindings(&args.engine, &bindings)?;
    if preview
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err("preview-bindings reported binding errors".to_owned());
    }
    atomic_write(&args.output, &profile_bytes)?;

    let sha256 = format!("{:x}", Sha256::digest(&profile_bytes));
    let digest = format!("sha256:{sha256}");
    let bookends_enabled = overlay::enabled(&effective_profile);
    let advice_configuration = match &advice_config {
        Some(config) => json!({
            "decision": "configure",
            "enabled": true,
            "configured": true,
            "command": config,
            "limits": {
                "timeout_ms": config.timeout_ms,
                "max_request_bytes": config.max_request_bytes,
                "max_response_bytes": config.max_response_bytes
            },
            "occasion_map": advice_occasions,
            "occasion_descriptions": workflow::advice_occasion_descriptions(),
            "departure_map": advice_departures
        }),
        None => json!({
            "decision": "decline",
            "enabled": false,
            "configured": false,
            "reason": "operator explicitly declined advice; no backend is selected",
            "occasion_map": advice_occasions,
            "occasion_descriptions": workflow::advice_occasion_descriptions()
        }),
    };
    let effective_policy = effective_policy(
        &effective_profile,
        &bindings,
        implementation.is_some(),
        &advice_configuration,
    );
    let selected_bytes = String::from_utf8(selected.bytes.clone())
        .map_err(|error| format!("selected profile is not UTF-8: {error}"))?;
    let selected_sha256 = format!("{:x}", Sha256::digest(&selected.bytes));
    let selected_name = Path::new(&selected.display_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("embedded profile");
    let profile_selection = json!({
        "kind": selected.kind,
        "path": selected.display_path,
        "basename": selected_name,
        "byte_length": selected.bytes.len(),
        "sha256": selected_sha256,
        "sha256_digest": format!("sha256:{selected_sha256}"),
        "bytes": selected_bytes
    });
    let enablement = json!({
        "bookends": {"enabled": bookends_enabled},
        "advice": advice_configuration
    });

    Ok(SetupReport {
        status: "ready",
        rigor: args.profile_source.display_name(),
        profile_selection,
        bookends_enabled,
        enablement,
        effective_policy,
        effective_bindings: bindings,
        roster,
        output_path: args.output.to_string_lossy().into_owned(),
        output_byte_length: profile_bytes.len(),
        output_sha256: sha256,
        output_sha256_digest: digest,
        output_bytes: String::from_utf8(profile_bytes)
            .map_err(|error| format!("generated profile is not UTF-8: {error}"))?,
        advice_configuration,
        preview,
        started: false,
    })
}

fn validate_command_paths(args: &SetupArgs) -> Result<(), String> {
    for (name, value) in [("--engine", &args.engine), ("--provider", &args.provider)] {
        let metadata = fs::metadata(value)
            .map_err(|error| format!("{name} `{value}` is not an existing file: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("{name} `{value}` is not a file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(format!("{name} `{value}` is not executable"));
            }
        }
    }
    Ok(())
}

fn read_roster(path: &Path) -> Result<Vec<ReviewRosterEntry>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read roster {}: {error}", path.display()))?;
    let roster: Vec<ReviewRosterEntry> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("roster {} is invalid: {error}", path.display()))?;
    if roster.is_empty() {
        return Err("roster must be a non-empty array".to_owned());
    }
    let mut authors = std::collections::BTreeSet::new();
    for (index, entry) in roster.iter().enumerate() {
        if entry.author.trim().is_empty() {
            return Err(format!("roster entry {index} has an empty author"));
        }
        if !authors.insert(entry.author.clone()) {
            return Err(format!("roster has duplicate author `{}`", entry.author));
        }
        if entry.command.trim().is_empty() {
            return Err(format!("roster entry {index} has an empty command"));
        }
        if entry.command.contains(['\n', '\r', '\0']) {
            return Err(format!(
                "roster entry {index} command cannot contain a line break or NUL"
            ));
        }
        if let Some(argument_index) = entry
            .args
            .iter()
            .position(|argument| argument.contains(['\n', '\r', '\0']))
        {
            return Err(format!(
                "roster entry {index} argument {argument_index} cannot contain a line break or NUL"
            ));
        }
        let budget = &entry.token_budget;
        if budget.model_id.trim().is_empty()
            || budget.context_window_tokens == 0
            || budget.system_tokens == 0
            || budget.framing_tokens == 0
            || budget.output_reserve_tokens == 0
            || budget.reasoning_reserve_tokens == 0
            || budget
                .system_tokens
                .saturating_add(budget.framing_tokens)
                .saturating_add(budget.output_reserve_tokens)
                .saturating_add(budget.reasoning_reserve_tokens)
                >= budget.context_window_tokens
        {
            return Err(format!(
                "roster entry {index} needs a real model ID/window and positive system, framing, output, and reasoning reserves that leave input capacity"
            ));
        }
    }
    Ok(roster)
}

fn read_draft_worker(path: &Path) -> Result<DraftWorkerInput, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read draft worker {}: {error}", path.display()))?;
    let worker: DraftWorkerInput = serde_json::from_slice(&bytes)
        .map_err(|error| format!("draft worker {} is invalid: {error}", path.display()))?;
    validate_worker_command(&worker.command, &worker.args, "draft worker")?;
    Ok(worker)
}

fn validate_worker_command(command: &str, args: &[String], label: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err(format!("{label} command must be non-empty"));
    }
    if command.contains(['\n', '\r', '\0']) {
        return Err(format!(
            "{label} command cannot contain a line break or NUL"
        ));
    }
    if let Some(argument_index) = args
        .iter()
        .position(|argument| argument.contains(['\n', '\r', '\0']))
    {
        return Err(format!(
            "{label} argument {argument_index} cannot contain a line break or NUL"
        ));
    }
    Ok(())
}

fn read_implementation(path: &Path) -> Result<ImplementationInput, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read implementation binding {}: {error}",
            path.display()
        )
    })?;
    let implementation: ImplementationInput = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "implementation binding {} is invalid: {error}",
            path.display()
        )
    })?;
    validate_worker_command(
        &implementation.command,
        &implementation.args,
        "implementation",
    )?;
    if is_prewrapped_task_worker(&implementation.command, &implementation.args) {
        return Err("implementation must be an unwrapped task-worker command; setup adds the run-plan-graph --task-worker wrapper".to_owned());
    }
    if implementation.working_directory.trim().is_empty()
        || !Path::new(&implementation.working_directory).is_absolute()
    {
        return Err("implementation working_directory must be an absolute directory".to_owned());
    }
    let metadata = fs::metadata(&implementation.working_directory).map_err(|error| {
        format!(
            "implementation working_directory `{}` is not an existing directory: {error}",
            implementation.working_directory
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "implementation working_directory `{}` is not a directory",
            implementation.working_directory
        ));
    }
    Ok(implementation)
}

struct SelectedProfile {
    kind: &'static str,
    display_path: String,
    bytes: Vec<u8>,
    profile: Value,
}

fn load_selected_profile(source: &ProfileSource) -> Result<SelectedProfile, String> {
    let (kind, display_path, bytes) = match source {
        ProfileSource::Bundled(rigor) => {
            let path = format!("{PROFILE_PREFIX}{}.json", rigor.profile_name());
            let bytes = embedded_bytes(&path)?.to_vec();
            ("bundled", path, bytes)
        }
        ProfileSource::File(path) => {
            let bytes = fs::read(path).map_err(|error| {
                format!(
                    "could not read selected profile {}: {error}",
                    path.display()
                )
            })?;
            let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            ("file", resolved.to_string_lossy().into_owned(), bytes)
        }
    };
    let profile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("selected profile {} is invalid JSON: {error}", display_path))?;
    Ok(SelectedProfile {
        kind,
        display_path,
        bytes,
        profile,
    })
}

fn refuse_legacy_shipped_profile(profile: &Value) -> Result<(), String> {
    let Some(config_version) = profile.get("config_version").and_then(Value::as_str) else {
        return Ok(());
    };
    if LEGACY_SHIPPED_PROFILE_VERSIONS.contains(&config_version) {
        return Err(format!(
            "known historical shipped profile config_version `{config_version}` is unsupported for new setup; use its original provider or revise a caller-owned profile to an explicit custom version"
        ));
    }
    Ok(())
}

fn is_prewrapped_task_worker(command: &str, args: &[String]) -> bool {
    let executable = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);
    match executable {
        "loop-engine" => args
            .iter()
            .any(|arg| matches!(arg.as_str(), "fan-out" | "--worker" | "--task-worker")),
        "software-change" => args
            .iter()
            .any(|arg| matches!(arg.as_str(), "run-plan-graph" | "--task-worker")),
        _ => args.iter().any(|arg| arg == "--task-worker"),
    }
}

const ADVICE_OCCASIONS: &[&str] = &[
    "review-candidates",
    "accepted-defect",
    "implementation-correction",
    "execution-or-authority-issue",
    "evidence-applicability",
    "requirements-reconciliation",
    "review-round-departure",
    "final-completion",
];

fn advice_occasions(profile: &Value) -> Result<Vec<String>, String> {
    let Some(value) = profile
        .get("extra")
        .and_then(Value::as_object)
        .and_then(|extra| extra.get("advice"))
    else {
        return Ok(Vec::new());
    };
    let Some(occasions) = value
        .as_object()
        .and_then(|advice| advice.get("occasion_map"))
    else {
        return Ok(Vec::new());
    };
    let occasions = occasions
        .as_array()
        .ok_or("profile extra.advice.occasion_map must be an array")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(occasions.len());
    for (index, occasion) in occasions.iter().enumerate() {
        let id = occasion
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| format!("profile advice occasion {index} must be a non-empty string"))?;
        if !seen.insert(id) {
            return Err(format!("profile advice occasion map duplicates `{id}`"));
        }
        result.push(id.to_owned());
    }
    Ok(result)
}

fn read_advice_config(path: &Path) -> Result<AdviceCommandConfig, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read advice command config {}: {error}",
            path.display()
        )
    })?;
    let config: AdviceCommandConfig = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "advice command config {} is invalid JSON: {error}",
            path.display()
        )
    })?;
    config.validate()?;
    Ok(config)
}

fn build_advice_departures(
    profile: &Value,
    profile_occasions: &[String],
) -> Result<AdviceDepartureMap, String> {
    let actual: std::collections::BTreeSet<_> =
        profile_occasions.iter().map(String::as_str).collect();
    let required: std::collections::BTreeSet<_> = ADVICE_OCCASIONS.iter().copied().collect();
    if actual != required {
        return Err(
            "configured advice requires the complete eight-family profile occasion map".to_owned(),
        );
    }
    let graph = workflow::describe_workflow(Some(profile))?;
    let mut departures = Vec::new();
    for family in ADVICE_OCCASIONS {
        let mut matched = 0;
        for transition in &graph.transitions {
            let source = transition.source.as_str();
            let event = transition.event.as_str();
            let review_state = source.ends_with("-review");
            let target_final = graph
                .states
                .iter()
                .any(|state| state.id == transition.target && state.is_final);
            let selected = match *family {
                "review-candidates" | "review-round-departure" => review_state,
                "accepted-defect" => review_state && event != "approved" && event != "passed",
                "implementation-correction" => {
                    event == "revise-implementation"
                        || (source == "implement" && event == "implementation-ready")
                        || (review_state
                            && (source.starts_with("implementation-")
                                || source.starts_with("validation-"))
                            && event == "revise")
                }
                "execution-or-authority-issue" => {
                    source == "implement" || source == "reconciliation"
                }
                "evidence-applicability" => {
                    (review_state && event == "approved") || event == "reconciliation-ready"
                }
                "requirements-reconciliation" => source == "reconciliation",
                "final-completion" => target_final,
                _ => false,
            };
            if selected {
                departures.push(AdviceDeparture {
                    state: transition.source.clone(),
                    event: transition.event.clone(),
                    occasion_id: format!("{family}:{source}:{event}"),
                });
                matched += 1;
            }
        }
        if matched == 0 {
            return Err(format!(
                "profile workflow has no departure for advice family `{family}`"
            ));
        }
    }
    Ok(AdviceDepartureMap {
        version: 1,
        occasions: departures,
    })
}

fn embedded_bytes(path: &str) -> Result<&'static [u8], String> {
    embedded_data::FILES
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.bytes)
        .ok_or_else(|| format!("provider data is missing embedded file `{path}`"))
}

fn embedded_text(path: &str) -> Result<String, String> {
    let bytes = embedded_bytes(path)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|error| format!("embedded file `{path}` is not UTF-8: {error}"))
}

fn embedded_json(path: &str) -> Result<Value, String> {
    let bytes = embedded_bytes(path)?;
    serde_json::from_slice(bytes)
        .map_err(|error| format!("embedded file `{path}` is invalid JSON: {error}"))
}

fn validate_profile(profile: &Value, label: &str) -> Result<(), String> {
    if profile["contract_version"].as_u64() != Some(3) {
        return Err(format!("{label} must use contract_version 3"));
    }
    config::validate_config(profile)
        .map(|_| ())
        .map_err(|error| format!("{label} is invalid: {error}"))
}

fn enable_bookends(profile: &mut Value) -> Result<(), String> {
    let root = profile
        .as_object_mut()
        .ok_or("shipped profile must be a JSON object")?;
    let extra = root.entry("extra".to_owned()).or_insert_with(|| json!({}));
    let extra = extra
        .as_object_mut()
        .ok_or("profile extra must be an object")?;
    let bookends = extra
        .entry("bookends".to_owned())
        .or_insert_with(|| json!({}));
    let bookends = bookends
        .as_object_mut()
        .ok_or("profile extra.bookends must be an object")?;
    bookends.insert("enabled".to_owned(), Value::Bool(true));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_bindings(
    profile: &Value,
    roster: &[ReviewRosterEntry],
    engine: &str,
    provider: &str,
    preamble: &str,
    output_schema: &Value,
    draft_worker: Option<&DraftWorkerInput>,
    implementation: Option<&ImplementationInput>,
) -> Result<Value, String> {
    let policies = profile
        .get("review_policies")
        .and_then(Value::as_object)
        .ok_or("effective profile is missing object review_policies")?;
    let mut bindings = Map::new();
    if let Some(worker) = draft_worker {
        bindings.insert(
            "intent-draft".to_owned(),
            json!({"command": worker.command, "args": worker.args}),
        );
    }

    for gate in REVIEW_GATES {
        let Some(entries) = policies.get(*gate).and_then(Value::as_array) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        let workers = review_workers(profile, gate, entries, roster, preamble, output_schema)?;
        let used_authors: std::collections::BTreeSet<_> = workers
            .iter()
            .filter_map(|worker| worker["preamble"].as_str())
            .filter_map(|worker_preamble| {
                worker_preamble
                    .lines()
                    .find_map(|line| line.strip_prefix("required_author_claim: "))
            })
            .collect();
        let call_budgets: Vec<_> = roster
            .iter()
            .filter(|entry| used_authors.contains(entry.author.as_str()))
            .map(|entry| {
                json!({
                    "author": entry.author,
                    "model_id": entry.token_budget.model_id,
                    "context_window_tokens": entry.token_budget.context_window_tokens,
                    "system_tokens": entry.token_budget.system_tokens,
                    "framing_tokens": entry.token_budget.framing_tokens,
                    "output_reserve_tokens": entry.token_budget.output_reserve_tokens,
                    "reasoning_reserve_tokens": entry.token_budget.reasoning_reserve_tokens
                })
            })
            .collect();
        let call_budgets = serde_json::to_string(&call_budgets)
            .map_err(|error| format!("could not encode reviewer token budgets: {error}"))?;
        let mut args = vec![
            "fan-out".to_owned(),
            "--max-active".to_owned(),
            REVIEW_CONCURRENCY.to_owned(),
        ];
        for (index, worker) in workers.iter().enumerate() {
            if index > 0
                && worker["review_stage"] == "aggregate"
                && workers[index - 1]["review_stage"] == "individual"
            {
                args.push("--then".to_owned());
            }
            let mut worker = worker.clone();
            worker
                .as_object_mut()
                .expect("generated worker is an object")
                .remove("review_stage");
            args.push("--worker".to_owned());
            args.push(
                serde_json::to_string(&worker)
                    .map_err(|error| format!("could not encode review worker: {error}"))?,
            );
        }
        bindings.insert(
            (*gate).to_owned(),
            json!({
                "command": engine,
                "args": args,
                "context_filter": {"command": provider, "args": ["commission", "--call-budgets", call_budgets]}
            }),
        );
    }

    if let Some(implementation) = implementation {
        let task_worker = json!({
            "command": implementation.command,
            "args": implementation.args
        });
        bindings.insert(
            "implement".to_owned(),
            json!({
                "command": provider,
                "args": [
                    "run-plan-graph",
                    "--working-directory",
                    implementation.working_directory,
                    "--max-active",
                    IMPLEMENTATION_CONCURRENCY,
                    "--task-worker",
                    serde_json::to_string(&task_worker).map_err(|error| format!("could not encode implementation worker: {error}"))?
                ]
            }),
        );
    }

    if bindings.is_empty() {
        return Err(
            "effective profile has no live review policies or implementation binding".to_owned(),
        );
    }
    Ok(Value::Object(bindings))
}

fn review_subject(gate: &str) -> &'static str {
    match gate {
        "intent-review" | "intent-adversarial-review" => "intent.json",
        "design-review" | "design-adversarial-review" => "design.json",
        "plan-review" | "plan-adversarial-review" => "plan.json",
        "implementation-review" | "implementation-adversarial-review" => {
            "implementation-report.json"
        }
        "validation-review" | "validation-adversarial-review" => "validation-report.json",
        _ => "unknown subject",
    }
}

fn review_workers(
    profile: &Value,
    gate: &str,
    entries: &[Value],
    roster: &[ReviewRosterEntry],
    preamble: &str,
    output_schema: &Value,
) -> Result<Vec<Value>, String> {
    let mut policies = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut max_authors = 1_u64;
    let mut has_individual = false;
    for (index, entry) in entries.iter().enumerate() {
        let object = entry
            .as_object()
            .ok_or_else(|| format!("policy {gate}[{index}] must be an object"))?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("policy {gate}[{index}] has no non-empty id"))?;
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .filter(|description| !description.is_empty())
            .ok_or_else(|| format!("policy {gate}[{index}] has no non-empty description"))?;
        let prompt = object
            .get("example_prompt")
            .and_then(Value::as_str)
            .filter(|prompt| !prompt.is_empty())
            .ok_or_else(|| format!("policy {gate}[{index}] has no non-empty example_prompt"))?;
        let stage = object
            .get("review_stage")
            .or_else(|| object.get("stage"))
            .and_then(Value::as_str)
            .unwrap_or("aggregate");
        if !matches!(stage, "individual" | "aggregate") {
            return Err(format!(
                "policy {gate}[{index}] has invalid review stage `{stage}`"
            ));
        }
        let required_authors = object
            .get("required_authors")
            .and_then(Value::as_u64)
            .filter(|count| *count > 0)
            .ok_or_else(|| format!("policy {gate}[{index}] has invalid required_authors"))?;
        if !seen.insert((stage.to_owned(), id.to_owned())) {
            return Err(format!(
                "policy {gate} duplicates axis `{id}` in stage `{stage}`"
            ));
        }
        max_authors = max_authors.max(required_authors);
        has_individual |= stage == "individual";
        policies.push((
            id.to_owned(),
            description.to_owned(),
            prompt.to_owned(),
            stage.to_owned(),
            required_authors,
        ));
    }
    if max_authors as usize > roster.len() {
        return Err(format!(
            "roster has {} authors but `{gate}` requires {max_authors}",
            roster.len()
        ));
    }

    let stages: Vec<&str> = if has_individual {
        vec!["individual", "aggregate"]
    } else {
        vec!["aggregate"]
    };
    let contract_version = profile["contract_version"].as_u64();
    let mut workers = Vec::new();
    for stage in stages {
        for (roster_index, roster_entry) in roster.iter().enumerate() {
            let assigned: Vec<_> = policies
                .iter()
                .filter(|policy| policy.3 == stage && policy.4 as usize > roster_index)
                .cloned()
                .collect();
            if assigned.is_empty() {
                continue;
            }
            let assigned_groups: Vec<Vec<_>> = if stage == "individual" {
                assigned.into_iter().map(|policy| vec![policy]).collect()
            } else {
                vec![assigned]
            };
            for assigned in assigned_groups {
                let axes: Vec<&str> = assigned.iter().map(|policy| policy.0.as_str()).collect();
                let mut worker_schema = output_schema.clone();
                configure_worker_schema(
                    &mut worker_schema,
                    stage,
                    &roster_entry.author,
                    &axes,
                    contract_version,
                    gate,
                )?;
                worker_schema["x-loop-engine-output-recovery"] = json!("repair-first-v1");
                let mut worker_preamble = String::from(preamble);
                worker_preamble.push_str("FROZEN REVIEW ASSIGNMENT\n");
                worker_preamble.push_str("review_contract_version: 2\n");
                worker_preamble.push_str("provider: software-change\n");
                worker_preamble.push_str("subject: ");
                worker_preamble.push_str(review_subject(gate));
                worker_preamble.push_str("\nslot_id: ");
                worker_preamble.push_str(gate);
                worker_preamble.push_str("\nreview_stage: ");
                worker_preamble.push_str(stage);
                worker_preamble.push_str("\nassigned_policies: ");
                worker_preamble.push_str(
                    &serde_json::to_string(
                        &assigned
                            .iter()
                            .map(|policy| {
                                json!({
                                    "id": policy.0,
                                    "description": policy.1,
                                    "example_prompt": policy.2,
                                    "review_stage": policy.3,
                                    "required_authors": policy.4
                                })
                            })
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|error| format!("could not encode assigned policies: {error}"))?,
                );
                worker_preamble.push_str("\nrequired_author_claim: ");
                worker_preamble.push_str(&roster_entry.author);
                worker_preamble.push_str("\nPER-CALL TOKEN WINDOW: model_id=");
                worker_preamble.push_str(&roster_entry.token_budget.model_id);
                worker_preamble.push_str(" window=");
                worker_preamble
                    .push_str(&roster_entry.token_budget.context_window_tokens.to_string());
                worker_preamble.push_str(" system_reserve=");
                worker_preamble.push_str(&roster_entry.token_budget.system_tokens.to_string());
                worker_preamble.push_str(" framing_reserve=");
                worker_preamble.push_str(&roster_entry.token_budget.framing_tokens.to_string());
                worker_preamble.push_str(" output_reserve=");
                worker_preamble
                    .push_str(&roster_entry.token_budget.output_reserve_tokens.to_string());
                worker_preamble.push_str(" reasoning_reserve=");
                worker_preamble.push_str(
                    &roster_entry
                        .token_budget
                        .reasoning_reserve_tokens
                        .to_string(),
                );
                worker_preamble.push_str(". This exact model and reserve is frozen; do not substitute or split duties.\n");
                if profile["contract_version"] == 3
                    && gate == "validation-review"
                    && stage == "aggregate"
                {
                    worker_preamble.push_str("criterion_author_number: ");
                    worker_preamble.push_str(&(roster_index + 1).to_string());
                    worker_preamble.push_str("\nRead the frozen validation-report index and checkpoint. Return assigned criterion/goal judgments in validation_verdicts: [{record_id,kind,data}], using the prechosen IDs for your author number under criterion_policy. Do not create placeholders, edit the index, or run commands. Consume retained command evidence. For focused repair, leave unaffected applicability rows alone; judge affected rows freshly. Axis judgments consume this collection.\n");
                } else if gate == "validation-adversarial-review" {
                    worker_preamble.push_str("Consume the existing criterion/goal collection; do not commission it again or run proof commands.\n");
                }
                if has_individual && stage == "aggregate" {
                    worker_preamble.push_str("FIRST AGGREGATE REVIEW\nThis is a fresh independent reviewer session on the unchanged subject. Do not inspect, read, or rely on individual-stage captures, outputs, findings, or judgments; the aggregate group receives none of them.\n");
                }
                if gate.ends_with("adversarial-review") {
                    worker_preamble.push_str("CHALLENGE GROUNDS\nInspect the completed ordinary aggregate judgments for this exact subject and cite the actual parent's reason and evidence locators. Do not infer parent grounds from the finding ledger or replace missing parent reasoning with a paraphrase.\n");
                }
                if gate.starts_with("intent-") {
                    worker_preamble.push_str("OWNER-SOURCE COMPARISON\nCompare the full current intent with retained qualified owner-source statements and their supersession. Distinguish exact owner-authored source records from driver inference or summaries; a driver paraphrase is not an owner instruction. Identify material lost qualifications and unresolved meaning without deciding for the owner.\n");
                }
                worker_preamble.push_str("GROUNDED OUTPUT\nReturn review_contract_version 2. Every fresh judgment includes grounds.reason and one or more grounds.evidence references using a repository-relative file#JSON-Pointer or file#Lx-Ly locator and the exact source file SHA-256. Do not invent citations. For validation criterion/goal passes, include a concise reason and retained evidence_context_ids that you actually inspected.\n");
                workers.push(json!({
                    "command": roster_entry.command,
                    "args": roster_entry.args,
                    "review_stage": stage,
                    "preamble": worker_preamble,
                    "full_output_schema": worker_schema
                }));
            }
        }
    }
    Ok(workers)
}

fn configure_worker_schema(
    schema: &mut Value,
    stage: &str,
    author: &str,
    axes: &[&str],
    contract_version: Option<u64>,
    gate: &str,
) -> Result<(), String> {
    let properties = schema
        .as_object_mut()
        .and_then(|root| root.get_mut("properties"))
        .and_then(Value::as_object_mut)
        .ok_or("review output schema must have object properties")?;
    let review_stage = properties
        .get_mut("review_stage")
        .and_then(Value::as_object_mut)
        .ok_or("review output schema is missing review_stage object")?;
    review_stage.insert("const".to_owned(), Value::String(stage.to_owned()));
    let author_schema = properties
        .get_mut("author")
        .and_then(Value::as_object_mut)
        .ok_or("review output schema is missing author object")?;
    author_schema.insert("const".to_owned(), json!({"name":author,"kind":"agent"}));
    let judgments = properties
        .get_mut("judgments")
        .and_then(Value::as_object_mut)
        .ok_or("review output schema is missing judgments array")?;
    judgments.insert("minItems".to_owned(), json!(axes.len()));
    judgments.insert("maxItems".to_owned(), json!(axes.len()));
    let items = judgments
        .get_mut("items")
        .and_then(Value::as_object_mut)
        .ok_or("review output schema is missing judgments.items")?;
    let branches = items
        .get_mut("oneOf")
        .and_then(Value::as_array_mut)
        .ok_or("review output schema is missing judgments.items.oneOf")?;
    if branches.len() != 2 {
        return Err("review output schema must have two judgment branches".to_owned());
    }
    for branch in branches {
        let axis = branch
            .as_object_mut()
            .and_then(|branch| branch.get_mut("properties"))
            .and_then(Value::as_object_mut)
            .and_then(|properties| properties.get_mut("axis"))
            .ok_or("review output schema judgment branch is missing axis")?;
        axis["enum"] = json!(axes);
    }
    judgments.insert(
        "allOf".to_owned(),
        Value::Array(
            axes.iter()
                .map(|axis| {
                    json!({"contains":{"type":"object","required":["axis"],"properties":{"axis":{"const":axis}}}})
                })
                .collect(),
        ),
    );

    if contract_version == Some(3) && gate == "validation-review" && stage == "aggregate" {
        properties.insert(
            "validation_verdicts".to_owned(),
            json!({
                "type":"array",
                "items": {
                    "type":"object",
                    "additionalProperties":false,
                    "required":["record_id","kind","data"],
                    "properties": {
                        "record_id":{"type":"string","minLength":1},
                        "kind":{"type":"string","enum":["criterion-verdict","goal-verdict"]},
                        "data":{
                        "type":"object",
                        "required":["reason","evidence_context_ids"],
                        "properties":{
                            "reason":{"type":"string","minLength":1,"maxLength":1200},
                            "evidence_context_ids":{"type":"array","minItems":1,"items":{"type":"string","minLength":1}}
                        }
                    }
                    }
                }
            }),
        );
    }
    Ok(())
}

fn effective_policy(
    profile: &Value,
    bindings: &Value,
    implementation: bool,
    advice_configuration: &Value,
) -> Value {
    let mut gates = Map::new();
    if let Some(policies) = profile.get("review_policies").and_then(Value::as_object) {
        for gate in REVIEW_GATES {
            if let Some(entries) = policies.get(*gate) {
                gates.insert((*gate).to_owned(), entries.clone());
            }
        }
    }
    let binding_slots = bindings
        .as_object()
        .map(|bindings| bindings.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "config_version": profile["config_version"],
        "contract_version": profile["contract_version"],
        "criterion_policy": profile["criterion_policy"],
        "artifact_schemas": profile["artifact_schemas"],
        "bookends_enabled": overlay::enabled(profile),
        "advice_enabled": advice_configuration["enabled"],
        "advice": advice_configuration,
        "review_policies": gates,
        "binding_slots": binding_slots,
        "review_concurrency": 2,
        "implementation_concurrency": implementation.then_some(1)
    })
}

fn profile_bytes(profile: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| format!("could not serialize generated profile: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn preview_bindings(engine: &str, bindings: &Value) -> Result<Value, String> {
    let input = serde_json::to_vec(&json!({"work_slot_bindings": bindings}))
        .map_err(|error| format!("could not encode preview input: {error}"))?;
    let mut command = Command::new(engine);
    command
        .arg("preview-bindings")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not run `{engine} preview-bindings`: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("preview process stdin was unavailable")?
        .write_all(&input)
        .map_err(|error| format!("could not send preview input: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not wait for `{engine} preview-bindings`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "preview-bindings failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let preview: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "preview-bindings returned invalid JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    let object = preview
        .as_object()
        .ok_or("preview-bindings returned a non-object report")?;
    for key in ["bindings", "models", "warnings"] {
        if !object.get(key).is_some_and(Value::is_array) {
            return Err(format!("preview-bindings report is missing array `{key}`"));
        }
    }
    if let Some(errors) = object.get("errors") {
        let errors = errors
            .as_array()
            .ok_or("preview-bindings report `errors` must be an array")?;
        if !errors.is_empty() {
            return Err("preview-bindings reported binding errors".to_owned());
        }
    }
    Ok(preview)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        let metadata = fs::metadata(parent).map_err(|error| {
            format!("output parent {} is unavailable: {error}", parent.display())
        })?;
        if !metadata.is_dir() {
            return Err(format!(
                "output parent {} is not a directory",
                parent.display()
            ));
        }
    }
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.setup-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("profile"),
        std::process::id(),
        suffix
    ));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("could not atomically write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigor_names_match_embedded_profiles() {
        assert_eq!(Rigor::Minimal.profile_name(), "minimal");
        assert_eq!(Rigor::Standard.profile_name(), "standard");
        assert_eq!(Rigor::High.profile_name(), "high-rigor");
    }

    #[test]
    fn profile_path_and_rigor_are_mutually_exclusive() {
        let error = parse_args(&[
            "--profile".into(),
            "/tmp/custom.json".into(),
            "--rigor".into(),
            "minimal".into(),
            "--roster".into(),
            "/tmp/roster.json".into(),
            "--engine".into(),
            "/bin/true".into(),
            "--provider".into(),
            "/bin/true".into(),
            "--output".into(),
            "/tmp/output.json".into(),
            "--decline-advice".into(),
        ])
        .expect_err("both profile selectors");
        assert!(error.contains("mutually exclusive"));
    }

    #[test]
    fn setup_refuses_known_legacy_shipped_profile_ids_only() {
        for version in LEGACY_SHIPPED_PROFILE_VERSIONS {
            let error = refuse_legacy_shipped_profile(&json!({"config_version": version}))
                .expect_err("known shipped successor id must be refused");
            assert!(error.contains(version), "{error}");
            assert!(error.contains("original provider"), "{error}");
        }
        for version in ["local-profile-11", "custom-10", "custom-revision"] {
            refuse_legacy_shipped_profile(&json!({"config_version": version}))
                .expect("caller-managed version claims are not guessed from their suffix");
        }
        assert!(refuse_legacy_shipped_profile(&json!({"config_version": 12})).is_ok());
    }

    #[test]
    fn setup_rejects_already_wrapped_implementation_task_workers() {
        assert!(is_prewrapped_task_worker(
            "/tmp/loop-engine",
            &["fan-out".into(), "--worker".into()]
        ));
        assert!(!is_prewrapped_task_worker(
            "/bin/echo",
            &["implementation".into()]
        ));
    }

    #[test]
    fn bundled_profiles_expose_the_same_advice_occasion_extension_point() {
        let occasion_sets: Vec<_> = [Rigor::Minimal, Rigor::Standard, Rigor::High]
            .into_iter()
            .map(|rigor| {
                let profile =
                    load_selected_profile(&ProfileSource::Bundled(rigor)).expect("bundled profile");
                advice_occasions(&profile.profile).expect("occasion map")
            })
            .collect();
        assert!(occasion_sets.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(occasion_sets[0].len(), 8);
        for rigor in [Rigor::Minimal, Rigor::Standard, Rigor::High] {
            let profile = load_selected_profile(&ProfileSource::Bundled(rigor)).expect("profile");
            let occasions = advice_occasions(&profile.profile).expect("occasion map");
            let departures = build_advice_departures(&profile.profile, &occasions)
                .expect("mapped advice departures");
            assert_eq!(departures.version, 1);
            assert!(ADVICE_OCCASIONS.iter().all(|family| departures
                .occasions
                .iter()
                .any(|row| row.occasion_id.starts_with(&format!("{family}:")))));
        }
    }

    #[test]
    fn setup_requires_an_explicit_advice_choice() {
        let args = [
            "--rigor",
            "minimal",
            "--roster",
            "/tmp/roster.json",
            "--engine",
            "/bin/true",
            "--provider",
            "/bin/true",
            "--output",
            "/tmp/output.json",
        ]
        .map(str::to_owned);
        assert!(parse_args(&args)
            .unwrap_err()
            .contains("choose exactly one"));
        let with_both = [
            "--rigor",
            "minimal",
            "--roster",
            "/tmp/roster.json",
            "--engine",
            "/bin/true",
            "--provider",
            "/bin/true",
            "--output",
            "/tmp/output.json",
            "--decline-advice",
            "--advice-config",
            "/tmp/advice.json",
        ]
        .map(str::to_owned);
        assert!(parse_args(&with_both)
            .unwrap_err()
            .contains("choose exactly one"));
    }

    #[test]
    fn roster_rejects_duplicate_authors() {
        let path = std::env::temp_dir().join(format!(
            "software-change-setup-roster-{}-{}.json",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            br#"[{"author":"a","command":"one","args":[],"token_budget":{"model_id":"fixture","context_window_tokens":64000,"system_tokens":1000,"framing_tokens":1000,"output_reserve_tokens":1000,"reasoning_reserve_tokens":1000}},{"author":"a","command":"two","args":[],"token_budget":{"model_id":"fixture","context_window_tokens":64000,"system_tokens":1000,"framing_tokens":1000,"output_reserve_tokens":1000,"reasoning_reserve_tokens":1000}}]"#,
        )
        .expect("roster");
        let error = read_roster(&path).expect_err("duplicate author");
        assert!(error.contains("duplicate author"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn draft_worker_is_a_closed_command_and_args_object() {
        let path = std::env::temp_dir().join(format!(
            "software-change-setup-draft-worker-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, br#"{"command":"worker","args":["--draft"]}"#).expect("draft worker");
        assert_eq!(
            read_draft_worker(&path).expect("valid draft worker"),
            DraftWorkerInput {
                command: "worker".to_owned(),
                args: vec!["--draft".to_owned()]
            }
        );
        fs::write(
            &path,
            br#"{"command":"worker","args":[],"preamble":"not allowed"}"#,
        )
        .expect("unknown field");
        assert!(read_draft_worker(&path)
            .expect_err("unknown draft worker field")
            .contains("unknown field"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn draft_worker_binding_is_added_without_review_policy_inputs() {
        let draft = DraftWorkerInput {
            command: "/engine".to_owned(),
            args: vec!["fan-out".to_owned(), "--worker".to_owned()],
        };
        let bindings = build_bindings(
            &json!({"review_policies": {}}),
            &[],
            "/engine",
            "/provider",
            "preamble",
            &json!({"type":"object"}),
            Some(&draft),
            None,
        )
        .expect("draft-only binding");
        assert_eq!(
            bindings["intent-draft"],
            json!({"command":"/engine","args":["fan-out","--worker"]})
        );
    }
}
