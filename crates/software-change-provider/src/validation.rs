//! Named deterministic execution and checkpoint-bound external judgments.
//! No catalog writes or semantic judgment. Pending names are never records.
use crate::{checkpoint, criterion, protocol::ProofCommand, recovery_contract::RecoveryContract};
use loop_core::ContextRecord;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn contract(input: &Value) -> Result<RecoveryContract, String> {
    RecoveryContract::from_input(input)
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty {key}"))
}
fn author_identity(value: &Value) -> Result<Value, String> {
    let name = text(value, "name")?;
    let kind = text(value, "kind")?;
    if !["human", "agent", "script"].contains(&kind) {
        return Err("invalid author identity".into());
    }
    Ok(json!({"name":name,"kind":kind}))
}
fn read(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| e.to_string())
}
fn write(path: &Path, value: &Value) -> Result<(), String> {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).map_err(|e| e.to_string())
}
fn ids(v: &Value) -> Result<Vec<String>, String> {
    let values: Vec<String> =
        serde_json::from_value(v.clone()).map_err(|_| "expected string ID array")?;
    if values.is_empty()
        || values.iter().any(|s| s.trim().is_empty())
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
    {
        return Err("missing or duplicate IDs".into());
    }
    Ok(values)
}
fn current_criteria(intent: &Value) -> Result<BTreeSet<String>, String> {
    let known = criterion::intent_ids(intent);
    let rows = intent["acceptance"]
        .as_array()
        .ok_or("current intent has no acceptance criteria")?;
    if known.is_empty()
        || known.len() != rows.len()
        || rows.iter().any(|r| text(r, "statement").is_err())
    {
        return Err("current intent has omitted, duplicate or malformed AC-N identities".into());
    }
    Ok(known)
}
fn record<'a>(context: &'a [ContextRecord], id: &str) -> Result<&'a ContextRecord, String> {
    let found: Vec<_> = context.iter().filter(|r| r.id.as_str() == id).collect();
    if found.len() != 1 {
        return Err(format!("missing or duplicate context ID `{id}`"));
    }
    Ok(found[0])
}
fn commands(values: &Value) -> Result<Vec<ProofCommand>, String> {
    let commands: Vec<ProofCommand> =
        serde_json::from_value(values.clone()).map_err(|e| format!("proof_commands: {e}"))?;
    let mut seen = BTreeSet::new();
    if commands.is_empty() {
        return Err("plan has no named proof_commands".into());
    }
    for c in &commands {
        if [&c.id, &c.command, &c.owner, &c.obligation]
            .iter()
            .any(|s| s.trim().is_empty())
            || !seen.insert(&c.id)
        {
            return Err("invalid or duplicate proof command".into());
        }
    }
    Ok(commands)
}

/// Hidden capture worker: actual argv and cwd, both streams, real inner exit,
/// timing and before/after repository identity are recorded once in raw capture.
pub(crate) fn capture(args: &[String]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err(
            "validation-command requires cwd, command JSON and timeout milliseconds".into(),
        );
    }
    let timeout: u64 = args[2].parse().map_err(|_| "invalid command timeout")?;
    if timeout == 0 {
        return Err("command timeout must be positive".into());
    }
    let cwd = Path::new(&args[0]);
    let spec: ProofCommand = serde_json::from_str(&args[1]).map_err(|e| e.to_string())?;
    let before = checkpoint::repository_identity(cwd)?;
    let (receipt, index, receipt_path) = timed_command(&spec, cwd, timeout)?;
    let after = checkpoint::repository_identity(cwd)?;
    let attempt = receipt_path.parent().ok_or("receipt has no parent")?;
    let stdout = fs::read(attempt.join("stdout")).map_err(|e| e.to_string())?;
    let stderr = fs::read(attempt.join("stderr")).map_err(|e| e.to_string())?;
    if receipt["cleanup"] != "complete"
        || !receipt["capture_error"].is_null()
        || receipt["aborted"] == true
    {
        return Err(format!(
            "common capture did not complete: {}",
            receipt_path.display()
        ));
    }
    Ok(
        json!({"spec":spec,"cwd":cwd,"repository_before":before,"repository_after":after,"elapsed_ms":(receipt["wall_seconds"].as_f64().ok_or("missing capture duration")? * 1000.0) as u64,"exit_code":receipt["exit_code"],"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"spawn_error":receipt["spawn_error"],"timed_out":receipt["timed_out"],"capture_index":index,"capture_receipt":receipt_path}),
    )
}

fn timed_command(
    spec: &ProofCommand,
    cwd: &Path,
    timeout: u64,
) -> Result<(Value, std::path::PathBuf, std::path::PathBuf), String> {
    use loop_integrations::capture::{run_matrix_with_forwarding, Matrix, Row};
    use std::io::Read;
    let mut bytes = String::new();
    std::io::stdin()
        .read_to_string(&mut bytes)
        .map_err(|e| e.to_string())?;
    let packet: Value = if bytes.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&bytes).map_err(|e| format!("validation capture instructions: {e}"))?
    };
    let base = packet
        .get("artifact_root")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if !base.is_absolute() || !base.is_dir() {
        return Err("capture artifact_root must be an existing absolute directory".into());
    }
    let repository = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if !repository.status.success() {
        return Err("capture requires a Git repository".into());
    }
    let repository = std::path::PathBuf::from(String::from_utf8_lossy(&repository.stdout).trim());
    if base
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(repository.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("capture artifact_root must be outside the checkout".into());
    }
    let root = base.join(format!(
        "validation-capture-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).map_err(|e| e.to_string())?;
    let inherited = packet
        .get("inherit_environment")
        .cloned()
        .unwrap_or(json!([]));
    let environment = packet.get("environment").cloned().unwrap_or(json!({}));
    let row = Row {
        id: spec.id.clone(),
        argv: std::iter::once(spec.command.clone())
            .chain(spec.args.clone())
            .collect(),
        environment: serde_json::from_value(environment).map_err(|e| e.to_string())?,
        inherit_environment: serde_json::from_value(inherited).map_err(|e| e.to_string())?,
        timeout_ms: timeout,
        obligations: vec![spec.obligation.clone()],
    };
    run_matrix_with_forwarding(&Matrix { rows: vec![row] }, cwd, &root, false, true)
        .map_err(|e| e.to_string())?;
    let index = root.join("index.json");
    let selected = read(&index)?;
    let path = std::path::PathBuf::from(text(&selected["receipts"][0], "receipt")?);
    Ok((read(&path)?, index, path))
}

fn capture_row(
    spec: &ProofCommand,
    settings: &Value,
) -> Result<loop_integrations::capture::Row, String> {
    Ok(loop_integrations::capture::Row {
        id: spec.id.clone(),
        argv: std::iter::once(spec.command.clone())
            .chain(spec.args.clone())
            .collect(),
        environment: serde_json::from_value(
            settings.get("environment").cloned().unwrap_or(json!({})),
        )
        .map_err(|e| e.to_string())?,
        inherit_environment: serde_json::from_value(
            settings
                .get("inherit_environment")
                .cloned()
                .unwrap_or(json!([])),
        )
        .map_err(|e| e.to_string())?,
        timeout_ms: settings["timeout_ms"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("missing positive timeout_ms")?,
        obligations: vec![spec.obligation.clone()],
    })
}

/// Inert draft only: no execution, file writes, checkpoint or catalog mutation.
pub(crate) fn prepare(input: &Value) -> Result<Value, String> {
    let show = &input["show"];
    if show["operation"] != "show"
        || show["status"] != "completed"
        || show["result"]["current_state"] != "validation"
    {
        return Err("prepare-validation requires completed full validation show".into());
    }
    let cwd = Path::new(text(input, "working_directory")?);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("working_directory must exist and be absolute".into());
    }
    let revision = text(input, "revision")?;
    let packet = &show["result"];
    let root = Path::new(text(&packet["initial_input"], "artifact_root")?);
    let policy = contract(&packet["initial_input"])?;
    let commission =
        software_change_provider::commission::inspect(show, Some("validation-draft"), None)?;
    let mut specs = commands(&commission["commission"]["proof_commands"])?;
    let mut seen: BTreeSet<_> = specs.iter().map(|s| s.id.clone()).collect();
    let mut additions = vec![];
    for value in input
        .get("additions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let spec = commands(&json!([value]))?.remove(0);
        if !seen.insert(spec.id.clone()) {
            return Err("addition duplicates/replaces a proof ID".into());
        }
        additions.push(json!({"kind":"validation-command","record_id":format!("validation-{revision}-addition-{}",spec.id),"data":spec}));
        specs.push(spec);
    }
    let indexes: Vec<String> =
        serde_json::from_value(input["capture_indexes"].clone()).map_err(|e| e.to_string())?;
    let settings = &input["execution_settings"];
    let implementation = read(&root.join("implementation-report.json"))?;
    let intent = read(&root.join("intent.json"))?;
    let prefix = format!("validation-{revision}");
    let author = author_identity(&input["author"])?;
    let mut report = json!({"revision":revision,"author":author,"implementation_revision":implementation["revision"],"command_evidence_ids":[],"criteria":[],"goal_verdict_ids":[]});
    let mut diagnostics = vec![];
    let mut candidates = vec![];
    for spec in &specs {
        let id = format!("{prefix}-command-{}", spec.id);
        report["command_evidence_ids"]
            .as_array_mut()
            .unwrap()
            .push(json!(id));
        let row = capture_row(spec, settings)?;
        let mut valid = vec![];
        for index in &indexes {
            let path = match contained(root, Path::new(index)) {
                Ok(path) => path,
                Err(error) => {
                    diagnostics.push(format!("{}: {error}", spec.id));
                    continue;
                }
            };
            let selection = match read(&path) {
                Ok(value) => value,
                Err(error) => {
                    diagnostics.push(format!("{}: {error}", spec.id));
                    continue;
                }
            };
            if !selection["receipts"]
                .as_array()
                .is_some_and(|r| r.iter().any(|s| s["id"] == spec.id))
            {
                continue;
            }
            match loop_integrations::capture::verify_selected(&path, &row, cwd) {
                Ok(_) => valid.push(path),
                Err(e) => diagnostics.push(format!("{}: {e}", spec.id)),
            }
        }
        if valid.len() != 1 {
            diagnostics.push(format!(
                "{}: expected one valid selected receipt, got {}",
                spec.id,
                valid.len()
            ));
            continue;
        }
        candidates.push(json!({"kind":"command-evidence","record_id":id,"data":{"proof_id":spec.id,"spec":spec,"execution_settings":settings,"capture":{"format":"common-capture-v1","index":valid[0]}}}));
    }
    let mut batches = vec![];
    for criterion in current_criteria(&intent)? {
        let ids: Vec<_> = (1..=policy.criterion_policy.required_authors.get())
            .map(|n| format!("{prefix}-{criterion}-{n}"))
            .collect();
        report["criteria"]
            .as_array_mut()
            .unwrap()
            .push(json!({"criterion_id":criterion,"verdict_ids":ids}));
        for id in ids {
            batches.push(json!({"kind":"criterion-verdict","record_id":id,"criterion_id":criterion,"status":"pending-finalized-index-checkpoint"}));
        }
    }
    let goals: Vec<_> = (1..=policy.criterion_policy.goal_required_authors.get())
        .map(|n| format!("{prefix}-goal-{n}"))
        .collect();
    report["goal_verdict_ids"] = json!(goals);
    for id in goals {
        batches.push(json!({"kind":"goal-verdict","record_id":id,"status":"pending-finalized-index-checkpoint"}));
    }
    let mut names = BTreeSet::new();
    for row in additions
        .iter()
        .chain(candidates.iter())
        .chain(batches.iter())
    {
        let id = text(row, "record_id")?;
        if !names.insert(id)
            || packet["context"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|r| r["id"] == id))
        {
            return Err("preselected record ID collision; choose a fresh report revision".into());
        }
    }
    Ok(
        json!({"report_draft":report,"addition_candidates":additions,"command_candidates":candidates,"diagnostics":diagnostics,"commands_complete":diagnostics.is_empty(),"judgment_batches":batches,"excluded_authors":[author,implementation["author"]],"status":"draft-only"}),
    )
}

pub(crate) fn run(args: &[String], show: &Value) -> Result<Value, String> {
    let mut options = std::collections::BTreeMap::new();
    for pair in args.chunks(2) {
        if pair.len() != 2
            || ![
                "--engine",
                "--working-directory",
                "--revision",
                "--commands",
                "--timeout-ms",
            ]
            .contains(&pair[0].as_str())
            || options.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err("invalid run-validation arguments".into());
        }
    }
    let get = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let engine = Path::new(get("--engine")?);
    let cwd = Path::new(get("--working-directory")?);
    let revision = get("--revision")?;
    let timeout: u64 = options
        .get("--timeout-ms")
        .copied()
        .unwrap_or("1200000")
        .parse()
        .map_err(|_| "invalid timeout")?;
    if timeout == 0 {
        return Err("timeout must be positive".into());
    }
    if !engine.is_absolute()
        || !engine.is_file()
        || !cwd.is_absolute()
        || !cwd.is_dir()
        || revision.trim().is_empty()
    {
        return Err(
            "engine and working-directory must be existing absolute paths; revision nonempty"
                .into(),
        );
    }
    if show["operation"] != "show"
        || show["status"] != "completed"
        || show["result"]["current_state"] != "validation"
    {
        return Err("run-validation requires current completed validation show".into());
    }
    let packet = &show["result"];
    let policy = contract(&packet["initial_input"])?;
    let root = Path::new(text(&packet["initial_input"], "artifact_root")?);
    let context: Vec<ContextRecord> =
        serde_json::from_value(packet["context"].clone()).map_err(|e| e.to_string())?;
    let commission =
        software_change_provider::commission::inspect(show, Some("validation-draft"), None)?;
    let specs = commands(&commission["commission"]["proof_commands"])?;
    let selected = options
        .get("--commands")
        .map(|s| ids(&json!(s.split(',').collect::<Vec<_>>())))
        .transpose()?
        .unwrap_or_else(|| specs.iter().map(|s| s.id.clone()).collect());
    if selected.iter().any(|id| !specs.iter().any(|s| &s.id == id)) {
        return Err("unknown named command".into());
    }
    let intent = read(&root.join("intent.json"))?;
    let implementation = read(&root.join("implementation-report.json"))?;
    let prefix = format!("validation-{revision}");
    let mut report = json!({"revision":revision,"author":{"name":"software-change run-validation","kind":"script"},"implementation_revision":implementation["revision"],"command_evidence_ids":[],"criteria":[],"goal_verdict_ids":[]});
    let mut names = Vec::new();
    for id in current_criteria(&intent)? {
        let verdicts: Vec<_> = (1..=policy.criterion_policy.required_authors.get())
            .map(|n| format!("{prefix}-{id}-{n}"))
            .collect();
        names.extend(verdicts.clone());
        report["criteria"]
            .as_array_mut()
            .unwrap()
            .push(json!({"criterion_id":id,"verdict_ids":verdicts}));
    }
    let goals: Vec<_> = (1..=policy.criterion_policy.goal_required_authors.get())
        .map(|n| format!("{prefix}-goal-{n}"))
        .collect();
    names.extend(goals.clone());
    report["goal_verdict_ids"] = json!(goals);
    let command_ids: Vec<_> = specs
        .iter()
        .map(|s| format!("{prefix}-command-{}", s.id))
        .collect();
    names.extend(command_ids.clone());
    if names
        .iter()
        .any(|id| context.iter().any(|r| r.id.as_str() == id))
    {
        return Err("preselected record ID collision; choose a fresh report revision".into());
    }
    // A selected execution is not a waiver of the other final obligations.
    // Existing command records can be explicitly indexed by the driver afterward.
    report["command_evidence_ids"] = json!(command_ids);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let receipt = root.join(format!("validation-execution-{stamp}"));
    fs::create_dir(&receipt).map_err(|e| e.to_string())?;
    // Once execution begins, an incomplete attempt must not leave an older
    // passing index current. Unfulfilled names and the old checkpoint now fail.
    write(&root.join("validation-report.json"), &report)?;
    let instructions = receipt.join("instructions.json");
    write(&instructions, &json!({"artifact_root":root}))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for spec in specs.iter().filter(|s| selected.contains(&s.id)) {
        let mut command = Command::new(engine);
        command
            .current_dir(&receipt)
            .args(["fan-out", "--instructions"])
            .arg(&instructions)
            .args(["--max-active", "1", "--worker"]);
        let worker = json!({"command":exe,"args":["validation-command",cwd,serde_json::to_string(spec).unwrap(),timeout.to_string()]});
        let started = Instant::now();
        let output = command
            .arg(worker.to_string())
            .output()
            .map_err(|e| e.to_string())?;
        fs::write(receipt.join(format!("{}.stdout", spec.id)), &output.stdout)
            .map_err(|e| e.to_string())?;
        fs::write(receipt.join(format!("{}.stderr", spec.id)), &output.stderr)
            .map_err(|e| e.to_string())?;
        let summary: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
            format!(
                "capture failed ({}): {e}; stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
        })?;
        let summary_path = Path::new(text(&summary, "output_dir")?).join("summary.json");
        let stored = read(&summary_path)?;
        let row = stored["workers"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or("missing captured command")?;
        candidates.push(json!({"record_id":format!("{prefix}-command-{}",spec.id),"kind":"command-evidence","data":{"proof_id":spec.id,"capture":{"summary":summary_path,"assignment_id":row["assignment_id"]}},"capture_elapsed_ms":started.elapsed().as_millis()}));
    }
    let diagnostics: Vec<_> = candidates
        .iter()
        .filter_map(|c| command_evidence(&c["data"], root, None, None, true).err())
        .collect();
    let result = json!({"commands_passed":diagnostics.is_empty(),"diagnostics":diagnostics,"report":report,"command_candidates":candidates,"steering_ids":commission["commission"]["steering_ids"],"receipt":receipt,"inert":true});
    write(&receipt.join("candidates.json"), &result)?;
    Ok(result)
}

fn contained(root: &Path, path: &Path) -> Result<std::path::PathBuf, String> {
    let path =
        fs::canonicalize(path).map_err(|e| format!("missing capture {}: {e}", path.display()))?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("capture escapes this run artifact root".into());
    }
    Ok(path)
}
fn command_evidence(
    data: &Value,
    root: &Path,
    expected: Option<&ProofCommand>,
    current_repository: Option<&Value>,
    require_pass: bool,
) -> Result<Value, String> {
    if data["capture"]["format"] == "common-capture-v1" {
        let declared: ProofCommand =
            serde_json::from_value(data["spec"].clone()).map_err(|e| e.to_string())?;
        if expected.is_some_and(|s| s != &declared) {
            return Err("common capture differs from effective proof obligation".into());
        }
        let spec = &declared;
        let index = contained(root, Path::new(text(&data["capture"], "index")?))?;
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let row = capture_row(spec, &data["execution_settings"])?;
        let (_, receipt) = loop_integrations::capture::verify_selected(&index, &row, &cwd)
            .map_err(|e| e.to_string())?;
        if data["proof_id"] != spec.id {
            return Err("common capture proof ID mismatch".into());
        }
        // This format uses report-proof identity. Provider checkpoint validation
        // independently checks its own identity; never substitute their digests.
        return Ok(receipt);
    }
    let summary_path = contained(root, Path::new(text(&data["capture"], "summary")?))?;
    let summary = read(&summary_path)?;
    let assignment = text(&data["capture"], "assignment_id")?;
    let workers = summary["workers"].as_array().ok_or("incomplete summary")?;
    let matches: Vec<_> = workers
        .iter()
        .filter(|r| r["assignment_id"] == assignment)
        .collect();
    if matches.len() != 1 {
        return Err("missing or duplicate command assignment".into());
    }
    let worker = matches[0];
    if worker["exit_code"] != 0 {
        return Err("capture worker did not complete".into());
    }
    let path = contained(
        summary_path.parent().unwrap(),
        Path::new(text(worker, "stdout_path")?),
    )?;
    let _stderr = contained(
        summary_path.parent().unwrap(),
        Path::new(text(worker, "stderr_path")?),
    )?;
    use sha2::{Digest, Sha256};
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if worker["selected_output_sha256"] != format!("sha256:{:x}", Sha256::digest(&bytes))
        || worker["selected_attempt"].as_u64().is_none_or(|n| n == 0)
    {
        return Err("command selected capture digest/attempt mismatch".into());
    }
    let selected = Path::new(text(worker, "selected_output_path")?);
    let selected = if selected.is_absolute() {
        selected.to_path_buf()
    } else {
        summary_path.parent().unwrap().join(selected)
    };
    if contained(summary_path.parent().unwrap(), &selected)? != path {
        return Err("command selected output location mismatch".into());
    }
    let raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let args = worker["args"].as_array().ok_or("missing actual argv")?;
    if args.len() != 4
        || args[0] != "validation-command"
        || args[1] != raw["cwd"]
        || serde_json::from_str::<Value>(args[2].as_str().ok_or("bad command spec")?)
            .map_err(|e| e.to_string())?
            != raw["spec"]
    {
        return Err("captured command identity mismatch".into());
    }
    if raw["spec"]["id"] != data["proof_id"]
        || expected.is_some_and(|s| serde_json::to_value(s).unwrap() != raw["spec"])
    {
        return Err("command differs from named effective proof obligation".into());
    }
    if !raw["stdout"].is_string()
        || !raw["stderr"].is_string()
        || !raw["elapsed_ms"].is_number()
        || !raw["timed_out"].is_boolean()
        || raw.get("spawn_error").is_none()
        || raw.get("exit_code").is_none()
        || !(raw["exit_code"].is_null() || raw["exit_code"].is_i64())
        || !(raw["spawn_error"].is_null() || raw["spawn_error"].is_string())
    {
        return Err("incomplete command output".into());
    }
    for key in ["repository_before", "repository_after"] {
        let identity = text(&raw, key)?;
        if identity.len() != 71
            || !identity.starts_with("sha256:")
            || !identity[7..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("incomplete command repository identity".into());
        }
    }
    // A real failed execution can support a failing historical judgment. It
    // never supports a pass or discharges the current deterministic command.
    if require_pass
        && (raw["exit_code"] != 0 || raw["timed_out"] != false || !raw["spawn_error"].is_null())
    {
        return Err("command failed, missing executable, or incomplete output".into());
    }
    if current_repository.is_some()
        && fs::canonicalize(Path::new(text(&raw, "cwd")?)).map_err(|e| e.to_string())?
            != fs::canonicalize(std::env::current_dir().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
    {
        return Err("command working directory differs from current repository directory".into());
    }
    if (require_pass && raw["repository_before"] != raw["repository_after"])
        || current_repository.is_some_and(|r| *r != raw["repository_after"])
    {
        return Err("command repository identity is not the current checkpoint tree".into());
    }
    Ok(raw)
}

/// Validate the immutable source's shape without imposing current freshness.
pub(crate) fn source(
    data: &Value,
    kind: &str,
    context: &[ContextRecord],
    root: &Path,
) -> Result<(), String> {
    let object = data.as_object().ok_or("verdict must be object")?;
    let fields = [
        "criterion_id",
        "subject",
        "subject_revision",
        "checkpoint",
        "author",
        "result",
        "findings",
        "evidence_context_ids",
        "origin",
        loop_core::ENGINE_ORIGIN_KEY,
    ];
    if object.keys().any(|k| !fields.contains(&k.as_str()))
        || (kind == "goal-verdict" && object.contains_key("criterion_id"))
    {
        return Err("unknown verdict fields".into());
    }
    if kind == "criterion-verdict" && !criterion::is_criterion_id(text(data, "criterion_id")?) {
        return Err("invalid criterion ID".into());
    }
    if data["subject"] != "validation-report.json"
        || data["checkpoint"] != "validation-checkpoint.json"
    {
        return Err("invalid verdict subject/checkpoint".into());
    }
    text(data, "subject_revision")?;
    text(&data["author"], "name")?;
    if !["agent", "human", "script"].contains(&text(&data["author"], "kind")?)
        || data["author"].as_object().is_none_or(|o| o.len() != 2)
    {
        return Err("invalid independent author".into());
    }
    let findings: Vec<String> = serde_json::from_value(data["findings"].clone())
        .map_err(|_| "findings must be string array")?;
    if (data["result"] == "pass" && !findings.is_empty())
        || (data["result"] == "fail"
            && (findings.is_empty() || findings.iter().any(|s| s.trim().is_empty())))
        || ![json!("pass"), json!("fail")].contains(&data["result"])
    {
        return Err("invalid verdict result/findings".into());
    }
    verify_verdict_origin(data, kind)?;
    for id in ids(&data["evidence_context_ids"])? {
        let evidence = record(context, &id)?;
        if evidence.kind != "command-evidence" {
            return Err(format!(
                "unsupported evidence reference `{id}`; name retained command evidence"
            ));
        }
        command_evidence(&evidence.data, root, None, None, data["result"] == "pass")?;
    }
    Ok(())
}

fn verify_verdict_origin(data: &Value, kind: &str) -> Result<Option<String>, String> {
    use sha2::{Digest, Sha256};
    if data.get("origin").is_none() && data.get(loop_core::ENGINE_ORIGIN_KEY).is_none() {
        return Ok(None);
    }
    let origin: loop_core::OriginReference =
        serde_json::from_value(data["origin"].clone()).map_err(|e| e.to_string())?;
    let engine: loop_core::EngineOrigin =
        serde_json::from_value(data[loop_core::ENGINE_ORIGIN_KEY].clone())
            .map_err(|e| e.to_string())?;
    if origin.kind != "selected-assignment-output"
        || origin.id != engine.invocation_id.as_str()
        || origin.assignment_id.as_deref() != Some(engine.assignment_id.as_str())
        || engine.selected_attempt == 0
    {
        return Err("invalid engine-resolved verdict origin".into());
    }
    let path = Path::new(&engine.selected_output_path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(&engine.capture_dir).join(path)
    };
    let bytes =
        fs::read(contained(Path::new(&engine.capture_dir), &path)?).map_err(|e| e.to_string())?;
    if format!("sha256:{:x}", Sha256::digest(&bytes)) != engine.selected_output_sha256 {
        return Err("verdict selected capture digest mismatch".into());
    }
    let raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut claimed = data.clone();
    claimed.as_object_mut().unwrap().remove("origin");
    claimed
        .as_object_mut()
        .unwrap()
        .remove(loop_core::ENGINE_ORIGIN_KEY);
    let matches: Vec<_> = raw["validation_verdicts"]
        .as_array()
        .ok_or("capture has no validation verdicts")?
        .iter()
        .filter(|r| r["kind"] == kind && r["data"] == claimed && raw["author"] == claimed["author"])
        .collect();
    if matches.len() != 1 {
        return Err("verdict disagrees with uniquely selected raw judgment".into());
    }
    Ok(Some(text(matches[0], "record_id")?.into()))
}

pub(crate) fn evaluate(
    input: &Value,
    context: &[ContextRecord],
    root: &Path,
    report: &Value,
    complete: bool,
) -> Result<Value, String> {
    let policy = contract(input)?;
    let intent = read(&root.join("intent.json"))?;
    let implementation = read(&root.join("implementation-report.json"))?;
    let report_author = author_identity(&report["author"])?;
    let implementation_author = author_identity(&implementation["author"])?;
    if report["implementation_revision"] != implementation["revision"] {
        return Err("validation index implementation revision is stale".into());
    }
    let plan = read(&root.join("plan.json"))?;
    let slots = crate::workflow::describe_workflow(Some(input))?.work_slots;
    let selected = software_change_provider::commission::select(
        context,
        &slots,
        "validation-draft",
        Some(&plan),
        None,
    )?;
    let specs = commands(&json!(selected.proof_commands))?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let repository = checkpoint::repository_identity(&cwd)?;
    let mut executed = BTreeSet::new();
    let command_ids = ids(&report["command_evidence_ids"])?;
    for id in &command_ids {
        let r = record(context, id)?;
        if r.kind != "command-evidence" {
            return Err("command index references wrong record kind".into());
        }
        let proof_id = text(&r.data, "proof_id")?;
        let spec = specs
            .iter()
            .find(|s| s.id == proof_id)
            .ok_or("unknown command proof ID")?;
        if !executed.insert(proof_id.to_owned()) {
            return Err("duplicate command proof ID".into());
        }
        command_evidence(&r.data, root, Some(spec), Some(&repository), true)?;
    }
    if executed.len() != specs.len() {
        return Err("missing required named proof command".into());
    }
    let revision = text(report, "revision")?;
    let known = current_criteria(&intent)?;
    let rows = report["criteria"]
        .as_array()
        .ok_or("missing criterion index")?;
    let mut covered = BTreeSet::new();
    let mut selected_ids = BTreeSet::new();
    let mut projection = Vec::new();
    let repair = context
        .iter()
        .rev()
        .find(|r| r.kind == "criterion-revalidation" && r.data["subject_revision"] == revision);
    let affected = match repair {
        Some(r) => {
            text(&r.data, "reason")?;
            let affected: Vec<String> = serde_json::from_value(r.data["affected_criteria"].clone())
                .map_err(|_| "invalid affected criteria")?;
            if affected.iter().collect::<BTreeSet<_>>().len() != affected.len()
                || affected.iter().any(|id| !known.contains(id))
            {
                return Err("unknown or duplicate affected criteria".into());
            }
            if ![json!("material"), json!("report-index-only")].contains(&r.data["change_kind"]) {
                return Err("revalidation requires explicit change_kind".into());
            }
            affected
        }
        None => Vec::new(),
    };
    let mut groups = Vec::new();
    for row in rows {
        let id = text(row, "criterion_id")?;
        if !known.contains(id) || !covered.insert(id.to_owned()) {
            return Err("unknown or duplicate criterion coverage".into());
        }
        groups.push((
            Some(id),
            ids(&row["verdict_ids"])?,
            policy.criterion_policy.required_authors.get(),
        ));
    }
    if covered != known || known.is_empty() {
        return Err("omitted current criterion coverage".into());
    }
    groups.push((
        None,
        ids(&report["goal_verdict_ids"])?,
        policy.criterion_policy.goal_required_authors.get(),
    ));
    let ledger = crate::finding_ledger::evaluate_finding_ledger(
        context,
        "validation-review",
        "validation-report.json",
        revision,
        &crate::config::parse_initial_input(input).map_err(|e| e.to_string())?,
    );
    if context
        .iter()
        .any(|r| r.kind == "finding-ledger" && r.data["gate"] == "validation-review")
        && !ledger.is_satisfied()
    {
        return Err(format!(
            "criterion finding ledger invalid: {}",
            ledger.details_value()
        ));
    }
    for (criterion, verdict_ids, floor) in groups {
        let mut authors = BTreeSet::new();
        for id in verdict_ids {
            if !selected_ids.insert(id.clone()) {
                return Err("duplicate selected verdict ID".into());
            }
            let r = match record(context, &id) {
                Ok(r) => r,
                Err(_) if !complete => continue,
                Err(e) => return Err(e),
            };
            let kind = if criterion.is_some() {
                "criterion-verdict"
            } else {
                "goal-verdict"
            };
            let mut original = r;
            let carried = r.kind == "evidence-applicability";
            if carried {
                let d = &r.data;
                let applicability: loop_core::EvidenceApplicability =
                    serde_json::from_value(d.clone())
                        .map_err(|e| format!("malformed criterion applicability: {e}"))?;
                if applicability.origin.assignment_id.is_some()
                    || applicability
                        .attesting_driver
                        .as_object()
                        .is_none_or(|o| o.len() != 2)
                    || !["human", "agent", "script"]
                        .contains(&text(&applicability.attesting_driver, "kind")?)
                {
                    return Err("invalid criterion applicability attestation".into());
                }
                if d["origin"]["kind"] != "context-record"
                    || d["target"]["subject"] != "validation-report.json"
                    || d["target"]["subject_revision"] != revision
                    || d["target"]["checkpoint"]
                        != json!({"phase":"validation","report_revision":revision})
                {
                    return Err("stale criterion applicability target/checkpoint".into());
                }
                text(d, "reason")?;
                text(&d["attesting_driver"], "name")?;
                text(&d["attesting_driver"], "kind")?;
                original = record(context, text(&d["origin"], "id")?)?;
                if original.sequence >= r.sequence {
                    return Err("carry requires original earlier verdict".into());
                }
                if repair.is_none()
                    || criterion.is_some_and(|id| affected.contains(&id.to_owned()))
                    || (criterion.is_none()
                        && repair.unwrap().data["change_kind"] != "report-index-only")
                {
                    return Err("affected criteria and material goal require fresh verdicts; declare unaffected repair scope".into());
                }
            }
            if original.kind != kind
                || original.data.get("criterion_id").and_then(Value::as_str) != criterion
            {
                return Err("verdict kind or criterion identity mismatch".into());
            }
            source(&original.data, kind, context, root)?;
            if verify_verdict_origin(&original.data, kind)?
                .is_some_and(|id| id != original.id.as_str())
            {
                return Err("verdict record ID differs from captured prechosen ID".into());
            }
            for evidence_id in ids(&original.data["evidence_context_ids"])? {
                if record(context, &evidence_id)?.sequence >= original.sequence {
                    return Err("verdict evidence must be an earlier genuine record".into());
                }
            }
            let d = &original.data;
            if !carried && d["subject_revision"] != revision {
                return Err("stale verdict target".into());
            }
            if d["author"] == report_author || d["author"] == implementation_author {
                return Err("self-authorship cannot satisfy criterion policy".into());
            }
            if ledger.current_snapshot().is_some_and(|s| {
                s.disposition_for(original.id.as_str())
                    == Some(crate::finding_ledger::FindingDisposition::RetiredAuthor)
            }) {
                projection.push(
                    json!({"selected_id":id,"source_id":original.id,"mode":"retired","verdict":d}),
                );
                continue;
            }
            if !authors.insert(d["author"].to_string()) {
                return Err("duplicate criterion author".into());
            }
            if !carried
                && ids(&d["evidence_context_ids"])?
                    .iter()
                    .any(|id| !command_ids.contains(id))
            {
                return Err("fresh verdict must reference current indexed command evidence".into());
            }
            if d["result"] == "fail"
                && !ledger.current_snapshot().is_some_and(|s| {
                    s.disposition_for(original.id.as_str()).is_some_and(|v| {
                        matches!(
                            v,
                            crate::finding_ledger::FindingDisposition::Accepted
                                | crate::finding_ledger::FindingDisposition::Rejected
                        )
                    })
                })
            {
                return Err(format!(
                    "unresolved failing verdict `{}`; disposition this exact source",
                    original.id
                ));
            }
            projection.push(json!({"selected_id":id,"source_id":original.id,"mode":if carried{"carried"}else{"fresh"},"verdict":d}));
        }
        if complete && authors.len() < floor {
            return Err(format!(
                "insufficient independent authors for {}",
                criterion.unwrap_or("whole-intent goal")
            ));
        }
    }
    Ok(json!({"rows":projection,"complete":complete}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_criterion_named_commands_are_closed_not_shell_prose() {
        let command = json!({"id":"proof","command":"cargo","args":["test"],"owner":"driver","obligation":"retained outcome"});
        assert!(commands(&json!([command.clone()])).is_ok());
        assert!(commands(&json!([command.clone(), command])).is_err());
        assert!(commands(&json!(["cargo test all the things"])).is_err());
        assert!(commands(&json!([])).is_err());
    }
    #[test]
    fn recovery_criterion_ids_and_current_spine_are_exact() {
        assert!(ids(&json!(["chosen"])).is_ok());
        for value in [json!([]), json!(["x", "x"]), json!([""]), json!([2])] {
            assert!(ids(&value).is_err());
        }
        let criterion = json!({"id":"AC-1","statement":"outcome"});
        assert!(current_criteria(&json!({"acceptance":[criterion.clone()]})).is_ok());
        assert!(current_criteria(&json!({"acceptance":[criterion.clone(),criterion]})).is_err());
    }
    #[test]
    fn recovery_criterion_v2_index_schema_replaces_freehand_results() {
        let schema = crate::schema::validate_schema(
            &serde_json::from_str::<Value>(include_str!("../data/validation-report-schema.json"))
                .unwrap(),
        )
        .unwrap();
        let mut index = json!({"revision":"1","author":{"name":"driver","kind":"agent"},"implementation_revision":"1","command_evidence_ids":["cmd"],"criteria":[{"criterion_id":"AC-1","verdict_ids":["future-real-record"]}],"goal_verdict_ids":["goal"]});
        assert!(schema.evaluate(&index).is_valid());
        index["outcome"] = json!("unsupported success declaration");
        assert!(!schema.evaluate(&index).is_valid());
    }
}
