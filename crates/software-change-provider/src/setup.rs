//! Deterministic per-run profile construction from embedded software-change data.
//!
//! `setup` assembles caller-supplied worker commands with a shipped profile. It
//! performs no workflow start or worker launch. The only intentional file write
//! is the atomically replaced profile named by `--output`.

use crate::{config, embedded_data, overlay};
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
const PREAMBLE_PATH: &str = "crates/software-change-provider/data/review-worker-preamble.txt";
const OUTPUT_SCHEMA_PATH: &str =
    "crates/software-change-provider/data/review-worker-output-schema.json";
const REVIEW_CONCURRENCY: &str = "2";
const IMPLEMENTATION_CONCURRENCY: &str = "1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewRosterEntry {
    author: String,
    command: String,
    args: Vec<String>,
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
    rigor: Rigor,
    roster_path: PathBuf,
    engine: String,
    provider: String,
    output: PathBuf,
    bookends: bool,
    draft_worker_path: Option<PathBuf>,
    implementation_path: Option<PathBuf>,
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
    "usage: software-change setup --rigor minimal|standard|high --roster PATH --engine ABS --provider ABS --output PATH [--bookends] [--draft-worker PATH] [--implementation PATH]"
}

fn parse_args(args: &[String]) -> Result<SetupArgs, String> {
    let mut rigor = None;
    let mut roster_path = None;
    let mut engine = None;
    let mut provider = None;
    let mut output = None;
    let mut bookends = false;
    let mut draft_worker_path = None;
    let mut implementation_path = None;
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

        let (name, inline) = if let Some(value) = token.strip_prefix("--rigor=") {
            ("--rigor", Some(value.to_owned()))
        } else if token == "--rigor" {
            ("--rigor", None)
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
            "--roster" => set_once(&mut roster_path, PathBuf::from(value), name)?,
            "--engine" => set_once(&mut engine, value, name)?,
            "--provider" => set_once(&mut provider, value, name)?,
            "--output" => set_once(&mut output, PathBuf::from(value), name)?,
            "--draft-worker" => set_once(&mut draft_worker_path, PathBuf::from(value), name)?,
            "--implementation" => set_once(&mut implementation_path, PathBuf::from(value), name)?,
            _ => unreachable!(),
        }
        index += 1;
    }

    let output = output.ok_or("missing required option `--output`")?;
    if output.as_os_str().is_empty() {
        return Err("--output must name a file".to_owned());
    }

    Ok(SetupArgs {
        rigor: rigor.ok_or("missing required option `--rigor`")?,
        roster_path: roster_path.ok_or("missing required option `--roster`")?,
        engine: absolute_command(engine, "--engine")?,
        provider: absolute_command(provider, "--provider")?,
        output,
        bookends,
        draft_worker_path,
        implementation_path,
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
    bookends_enabled: bool,
    effective_policy: Value,
    roster: Vec<ReviewRosterEntry>,
    output_path: String,
    output_byte_length: usize,
    output_sha256: String,
    output_sha256_digest: String,
    output_bytes: String,
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
    validate_command_paths(&args)?;

    let profile = shipped_profile(args.rigor)?;
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
    let effective_policy =
        effective_policy(&effective_profile, &bindings, implementation.is_some());

    Ok(SetupReport {
        status: "ready",
        rigor: args.rigor.as_str(),
        bookends_enabled: args.bookends,
        effective_policy,
        roster,
        output_path: args.output.to_string_lossy().into_owned(),
        output_byte_length: profile_bytes.len(),
        output_sha256: sha256,
        output_sha256_digest: digest,
        output_bytes: String::from_utf8(profile_bytes)
            .map_err(|error| format!("generated profile is not UTF-8: {error}"))?,
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

fn shipped_profile(rigor: Rigor) -> Result<Value, String> {
    let path = format!("{PROFILE_PREFIX}{}.json", rigor.profile_name());
    embedded_json(&path)
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
                "context_filter": {"command": provider, "args": ["commission"]}
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
                let mut worker_preamble = String::from(preamble);
                worker_preamble.push_str("FROZEN REVIEW ASSIGNMENT\n");
                worker_preamble.push_str("provider: software-change\n");
                worker_preamble.push_str("slot_id: ");
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
                worker_preamble.push('\n');
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
                        "data":{"type":"object"}
                    }
                }
            }),
        );
    }
    Ok(())
}

fn effective_policy(profile: &Value, bindings: &Value, implementation: bool) -> Value {
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
        "bookends_enabled": overlay::enabled(profile),
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
    fn roster_rejects_duplicate_authors() {
        let path = std::env::temp_dir().join(format!(
            "software-change-setup-roster-{}-{}.json",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            br#"[{"author":"a","command":"one","args":[]},{"author":"a","command":"two","args":[]}]"#,
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
