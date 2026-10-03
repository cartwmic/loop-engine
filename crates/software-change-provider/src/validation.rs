//! Named deterministic execution and checkpoint-bound external judgments.
//! No catalog writes or semantic judgment. Pending names are never records.
use crate::{checkpoint, criterion, protocol::ProofCommand, recovery_contract::RecoveryContract};
use loop_core::ContextRecord;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

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

fn capture_row(
    spec: &ProofCommand,
    settings: &Value,
) -> Result<loop_integrations::proof_metadata::Row, String> {
    Ok(loop_integrations::proof_metadata::Row {
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
    let counts = software_change_provider::author_counts::resolve(
        &packet["initial_input"],
        &packet["context"],
    );
    if !counts.blocked.is_empty() {
        return Err(format!(
            "invalid author-count amendment: {:?}",
            counts.amendments
        ));
    }
    let policy = contract(&counts.effective)?;
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
        additions.push(json!({"kind":"validation-command","record_id":format!("validation-{revision}-addition-{}",spec.id),"data":{"spec":spec}}));
        specs.push(spec);
    }
    let intent = read(&root.join("intent.json"))?;
    let implementation = read(&root.join("implementation-report.json"))?;
    let author = author_identity(&input["author"])?;
    let prefix = format!("validation-{revision}");
    let mut report = json!({"revision":revision,"author":author,"implementation_revision":implementation["revision"],"command_evidence_ids":[],"criteria":[],"goal_verdict_ids":[]});
    let results = input["command_results"]
        .as_array()
        .ok_or("missing command_results array")?;
    let mut candidates = vec![];
    let mut diagnostics = vec![];
    for spec in &specs {
        let id = format!("validation-{revision}-command-{}", spec.id);
        report["command_evidence_ids"]
            .as_array_mut()
            .unwrap()
            .push(json!(id));
        let selected: Vec<_> = results.iter().filter(|r| r["id"] == spec.id).collect();
        if selected.len() != 1 {
            diagnostics.push(json!({"proof_id":spec.id,"status":"missing-or-duplicate"}));
            continue;
        }
        let result = selected[0];
        verify_external_shape(result, spec)?;
        let identity = crate::source_identity::read(Path::new(text(result, "cwd")?))?;
        if result["source"]["head"] != identity["head"]
            || result["source"]["state_sha256"] != identity["state_sha256"]
        {
            diagnostics.push(json!({"proof_id":spec.id,"status":"stale-source"}));
        } else if result["exit_code"] != 0 {
            diagnostics.push(json!({"proof_id":spec.id,"status":"failed"}));
        }
        let mut external_result = result.clone();
        external_result["external"] = json!(true);
        candidates.push(json!({"kind":"command-evidence","record_id":id,"data":{"proof_id":spec.id,"spec":spec,"external_result":external_result}}));
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

fn verify_external_shape(result: &Value, spec: &ProofCommand) -> Result<(), String> {
    if result["id"] != spec.id
        || result["command"] != spec.command
        || result["args"] != json!(spec.args)
    {
        return Err("external command result does not match proof command".into());
    }
    let cwd = Path::new(text(result, "cwd")?);
    if !cwd.is_absolute() || !cwd.is_dir() || result["exit_code"].as_i64().is_none() {
        return Err("external result requires existing absolute cwd and integer exit_code".into());
    }
    let producer = &result["producer"];
    if [text(producer, "name")?, text(producer, "kind")?]
        .iter()
        .any(|v| v.to_ascii_lowercase().contains("engine"))
    {
        return Err("external producer must not claim engine execution".into());
    }
    text(&result["source"], "head")?;
    text(&result["source"], "state_sha256")?;
    if result["source"]
        .get("scope")
        .is_some_and(|scope| !scope.is_string())
    {
        return Err("source.scope must be a string".into());
    }
    if !["stdout", "stdout_ref", "stderr", "stderr_ref"]
        .iter()
        .any(|key| result[*key].is_string())
    {
        return Err("external result requires output text or reference".into());
    }
    for key in ["stdout", "stderr", "stdout_ref", "stderr_ref"] {
        if let Some(value) = result.get(key) {
            if !value.is_string()
                || (key.ends_with("_ref") && value.as_str().unwrap().trim().is_empty())
            {
                return Err(format!("invalid {key}"));
            }
        }
    }
    Ok(())
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
    if let Some(result) = data.get("external_result") {
        let spec: ProofCommand =
            serde_json::from_value(data["spec"].clone()).map_err(|e| e.to_string())?;
        if expected.is_some_and(|expected| expected != &spec) || data["proof_id"] != spec.id {
            return Err("external result differs from effective proof obligation".into());
        }
        if result["external"] != true
            || data.get("origin").is_some()
            || data.get(loop_core::ENGINE_ORIGIN_KEY).is_some()
        {
            return Err("external command evidence must not claim engine origin".into());
        }
        verify_external_shape(result, &spec)?;
        let cwd = Path::new(text(result, "cwd")?);
        let evaluation_cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        if checkpoint::repository_identity(cwd)?
            != checkpoint::repository_identity(&evaluation_cwd)?
        {
            return Err("external result is not from the checkpoint repository".into());
        }
        let identity = crate::source_identity::read(cwd)?;
        if result["source"]["head"] != identity["head"]
            || result["source"]["state_sha256"] != identity["state_sha256"]
        {
            return Err("stale-source: external command result identity changed".into());
        }
        if require_pass && result["exit_code"] != 0 {
            return Err("failed external command result".into());
        }
        return Ok(result.clone());
    }
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
        let (_, receipt) = loop_integrations::proof_metadata::verify_selected(&index, &row, &cwd)
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
    source_with_expected_target(data, kind, context, root, None, None)
}

pub(crate) fn source_for_candidate(
    data: &Value,
    kind: &str,
    context: &[ContextRecord],
    root: &Path,
    gate: &str,
    config_version: &str,
) -> Result<(), String> {
    source_with_expected_target(data, kind, context, root, Some(gate), Some(config_version))
}

fn source_with_expected_target(
    data: &Value,
    kind: &str,
    context: &[ContextRecord],
    root: &Path,
    expected_gate: Option<&str>,
    expected_config_version: Option<&str>,
) -> Result<(), String> {
    validate_source_target(data, expected_gate, expected_config_version)?;
    source_with_verified_commands(data, kind, context, root, None)
}

fn validate_source_target(
    data: &Value,
    expected_gate: Option<&str>,
    expected_config_version: Option<&str>,
) -> Result<(), String> {
    let linked = data.get("origin").is_some() || data.get(loop_core::ENGINE_ORIGIN_KEY).is_some();
    let current_contract = data
        .get(loop_core::ENGINE_ORIGIN_KEY)
        .and_then(|origin| origin.get("slot_id"))
        .is_some();
    let has_metadata = data.get("gate").is_some()
        || data.get("config_version").is_some()
        || data.get("review_stage").is_some();
    if linked && (current_contract || has_metadata) {
        let gate = text(data, "gate")?;
        let config_version = text(data, "config_version")?;
        if text(data, "review_stage")? != "aggregate"
            || (expected_gate.is_some_and(|expected| gate != expected))
            || (expected_config_version.is_some_and(|expected| config_version != expected))
        {
            return Err("source-linked verdict has wrong gate, stage, or config version".into());
        }
    }
    Ok(())
}

fn source_with_verified_commands(
    data: &Value,
    kind: &str,
    context: &[ContextRecord],
    root: &Path,
    verified_command_evidence_ids: Option<&BTreeSet<String>>,
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
        "reason",
        "evidence_context_ids",
        "origin",
        loop_core::ENGINE_ORIGIN_KEY,
        "gate",
        "config_version",
        "review_stage",
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
    if let Some(reason) = data.get("reason") {
        if reason
            .as_str()
            .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err("verdict reason must be a non-empty string when present".into());
        }
    }

    if (data["result"] == "pass" && !findings.is_empty())
        || (data["result"] == "fail"
            && (findings.is_empty() || findings.iter().any(|s| s.trim().is_empty())))
        || ![json!("pass"), json!("fail")].contains(&data["result"])
    {
        return Err("invalid verdict result/findings".into());
    }
    verify_verdict_origin(data, kind)?;
    if data["result"] == "pass"
        && data.get("reason").is_some()
        && data["evidence_context_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("passing criterion/goal verdict requires inspected evidence references".into());
    }
    let evidence_ids = ids(&data["evidence_context_ids"])?;
    for id in evidence_ids {
        let evidence = record(context, &id)?;
        if evidence.kind != "command-evidence" {
            return Err(format!(
                "unsupported evidence reference `{id}`; name retained command evidence"
            ));
        }
        if !verified_command_evidence_ids.is_some_and(|ids| ids.contains(evidence.id.as_str())) {
            command_evidence(&evidence.data, root, None, None, data["result"] == "pass")?;
        }
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
        || matches!(engine.selected_attempt, Some(0))
        || (engine
            .slot_id
            .as_deref()
            .is_some_and(|slot| data.get("gate").and_then(Value::as_str) != Some(slot)))
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
    match (engine.selected_attempt, engine.recovery_source.as_ref()) {
        (Some(attempt), None) if attempt > 0 => {}
        (None, Some(source)) => {
            crate::evidence::verify_recovery_source(
                source,
                Path::new(&engine.capture_dir),
                &engine.selected_output_path,
                &engine.selected_output_sha256,
            )?;
            crate::evidence::verify_recovery_source_identity(
                source,
                &engine.assignment_id,
                text(data, "gate")?,
                None,
                &engine.binding,
            )?;
        }
        _ => return Err("verdict has no verifiable selected raw or derived source".into()),
    }
    let raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if matches!(
        raw.get("review_contract_version").and_then(Value::as_u64),
        Some(2 | 3)
    ) && (data
        .get("reason")
        .and_then(Value::as_str)
        .is_none_or(|reason| {
            reason.trim().is_empty()
                || (raw.get("review_contract_version").and_then(Value::as_u64) == Some(2)
                    && reason.len() > 1200)
        })
        || ids(&data["evidence_context_ids"])?.is_empty())
    {
        return Err(
            "grounded criterion/goal source needs a nonempty reason and inspected evidence".into(),
        );
    }
    if raw.get("review_stage").and_then(Value::as_str) != Some("aggregate") {
        return Err("criterion/goal verdicts must come from an aggregate review output".into());
    }
    let mut claimed = data.clone();
    claimed.as_object_mut().unwrap().remove("origin");
    claimed.as_object_mut().unwrap().remove("gate");
    claimed.as_object_mut().unwrap().remove("config_version");
    claimed.as_object_mut().unwrap().remove("review_stage");
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
    // This cache is deliberately scoped to this evaluation request. The
    // command index above has already performed the full current-tree capture
    // verification; source validation still checks every verdict and falls
    // back to a fresh verification for historical/non-indexed evidence.
    let mut verified_command_evidence_ids = BTreeSet::new();
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
        verified_command_evidence_ids.insert(id.clone());
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
        let mut authors: BTreeSet<(String, String)> = BTreeSet::new();
        let mut self_authors = 0usize;
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
            validate_source_target(
                &original.data,
                Some("validation-review"),
                input.get("config_version").and_then(Value::as_str),
            )?;
            source_with_verified_commands(
                &original.data,
                kind,
                context,
                root,
                Some(&verified_command_evidence_ids),
            )?;
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
            if !(if criterion.is_some() {
                policy.criterion_policy.self_review
            } else {
                policy.criterion_policy.goal_self_review
            }) && (d["author"] == report_author || d["author"] == implementation_author)
            {
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
            let author_key = (
                text(&d["author"], "name")?.to_owned(),
                text(&d["author"], "kind")?.to_owned(),
            );
            if d["author"] == report_author || d["author"] == implementation_author {
                self_authors += 1;
            }
            if !authors.insert(author_key) {
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
        if complete && authors.len().saturating_sub(self_authors.saturating_sub(1)) < floor {
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
    fn external_result_shape_requires_attribution_output_and_integer_status() {
        let spec: ProofCommand = serde_json::from_value(json!({"id":"proof","command":"cargo","args":["test"],"owner":"driver","obligation":"outcome"})).unwrap();
        let result = json!({"id":"proof","command":"cargo","args":["test"],"cwd":std::env::current_dir().unwrap(),"exit_code":0,"stdout_ref":"driver.log","producer":{"name":"local-driver","kind":"external"},"source":{"head":"head","state_sha256":"sha256:digest"}});
        verify_external_shape(&result, &spec).unwrap();
        for (field, value) in [
            ("exit_code", json!(0.5)),
            ("producer", json!({"name":"loop-engine","kind":"script"})),
            ("producer", json!({"name":"driver","kind":""})),
            ("stdout_ref", json!("")),
            ("args", json!(["wrong"])),
        ] {
            let mut bad = result.clone();
            bad[field] = value;
            assert!(verify_external_shape(&bad, &spec).is_err(), "{field}");
        }
        let mut missing = result;
        missing.as_object_mut().unwrap().remove("stdout_ref");
        assert!(verify_external_shape(&missing, &spec).is_err());
        missing["stdout"] = json!("");
        verify_external_shape(&missing, &spec).unwrap();
    }

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
    fn passing_criterion_verdict_checks_inspected_evidence_when_reason_is_present() {
        use loop_core::{ContextRecord, SemanticSequence, Timestamp};
        let context = vec![ContextRecord::new(
            "cmd",
            "command-evidence",
            json!({}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        )];
        let data = json!({
            "criterion_id":"AC-1","subject":"validation-report.json","subject_revision":"r1",
            "checkpoint":"validation-checkpoint.json","author":{"name":"reviewer","kind":"agent"},
            "result":"pass","findings":[],"reason":"The captured result matches the outcome.",
            "evidence_context_ids":["cmd"]
        });
        let retained = BTreeSet::from(["cmd".to_owned()]);
        assert!(source_with_verified_commands(
            &data,
            "criterion-verdict",
            &context,
            Path::new("/unused"),
            Some(&retained)
        )
        .is_ok());
        let mut no_reason = data.clone();
        no_reason.as_object_mut().unwrap().remove("reason");
        assert!(
            source_with_verified_commands(
                &no_reason,
                "criterion-verdict",
                &context,
                Path::new("/unused"),
                Some(&retained)
            )
            .is_ok(),
            "legacy external rows retain their frozen shape"
        );
        let mut no_evidence = data.clone();
        no_evidence["evidence_context_ids"] = json!([]);
        assert!(source_with_verified_commands(
            &no_evidence,
            "criterion-verdict",
            &context,
            Path::new("/unused"),
            Some(&retained)
        )
        .unwrap_err()
        .contains("requires inspected evidence"));
        let mut oversized_reason = data;
        oversized_reason["reason"] = json!("x".repeat(1201));
        assert!(source_with_verified_commands(
            &oversized_reason,
            "criterion-verdict",
            &context,
            Path::new("/unused"),
            Some(&retained)
        )
        .is_ok());
    }

    #[test]
    fn source_linked_candidate_requires_current_gate_stage_and_config_claims() {
        let context = [];
        let data = json!({
            "gate":"validation-adversarial-review",
            "config_version":"fixture-1",
            "review_stage":"aggregate",
            "origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"worker-0"},
            loop_core::ENGINE_ORIGIN_KEY:{"slot_id":"validation-adversarial-review"}
        });
        assert!(source_for_candidate(
            &data,
            "criterion-verdict",
            &context,
            Path::new("/unused"),
            "validation-review",
            "fixture-1",
        )
        .unwrap_err()
        .contains("wrong gate, stage, or config"));
        let mut wrong_config = data.clone();
        wrong_config["gate"] = json!("validation-review");
        wrong_config[loop_core::ENGINE_ORIGIN_KEY]["slot_id"] = json!("validation-review");
        wrong_config["config_version"] = json!("stale-config");
        assert!(source_for_candidate(
            &wrong_config,
            "criterion-verdict",
            &context,
            Path::new("/unused"),
            "validation-review",
            "fixture-1",
        )
        .unwrap_err()
        .contains("wrong gate, stage, or config"));
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
