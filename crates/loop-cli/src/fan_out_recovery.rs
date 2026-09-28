//! Same-binding, caller-selected fan-out recovery and output-only repair.
//!
//! Recovery remains an ordinary public `invoke --input` operation. The
//! helper joins only retained, explicitly selected assignment sources and
//! launches the pending workers named by that input.

use crate::fan_out::{
    self, CollectorError, FanOutIdentity, FanOutSummary, RecoveryOutput, WorkerCli,
};
use crate::{dagu, fan_out::InvokePacket};
use loop_core::FanOutRecoveryInput;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn run_recovery_collector(
    packet: &InvokePacket,
    workers: &[WorkerCli],
    second_group_start: Option<usize>,
    max_active: Option<u32>,
    recovery: &FanOutRecoveryInput,
) -> Result<FanOutSummary, CollectorError> {
    fan_out::ensure_workers(workers)?;
    if packet.controls.force_fresh {
        return Err(CollectorError::Invalid(
            "recovery selection cannot be combined with force_fresh".to_owned(),
        ));
    }
    let origin = packet.recovery_origin.as_ref().ok_or_else(|| {
        CollectorError::Invalid("recovery origin was not verified by invoke".to_owned())
    })?;
    let binding_sha256 = packet.binding_sha256.as_deref().ok_or_else(|| {
        CollectorError::Invalid(
            "fan-out recovery requires an engine-computed binding digest".to_owned(),
        )
    })?;
    let state_visit = packet.state_visit.ok_or_else(|| {
        CollectorError::Invalid(
            "fan-out recovery requires an engine-computed state visit".to_owned(),
        )
    })?;
    if recovery.run_id != packet.run_id
        || recovery.slot_id != packet.slot_id
        || recovery.state_visit != state_visit
        || recovery.binding_sha256 != binding_sha256
        || origin.get("protocol").and_then(Value::as_str) != Some("fan-out-origin-v1")
        || origin.get("invocation_id").and_then(Value::as_str)
            != Some(recovery.origin_invocation_id.as_str())
        || origin.get("status").and_then(Value::as_str) != Some("failed")
        || origin.get("quiescent").and_then(Value::as_bool) != Some(true)
        || origin.get("state_visit").and_then(Value::as_u64) != Some(state_visit)
        || origin.get("binding_sha256").and_then(Value::as_str) != Some(binding_sha256)
    {
        return Err(CollectorError::Invalid(
            "fan-out recovery target or verified origin does not match this bound invocation"
                .to_owned(),
        ));
    }
    let origin_capture = origin
        .get("capture_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            CollectorError::Invalid("verified recovery origin omitted its capture".to_owned())
        })?;
    let origin_capture = fs::canonicalize(&origin_capture).map_err(|error| {
        CollectorError::Invalid(format!(
            "verified recovery origin capture is unavailable: {error}"
        ))
    })?;
    let origin_spec = fan_out::read_spec(&origin_capture)?;
    if origin_spec.run_id.as_deref() != Some(recovery.run_id.as_str())
        || origin_spec.slot_id.as_deref() != Some(recovery.slot_id.as_str())
        || origin_spec.state_visit != Some(state_visit)
        || origin_spec.binding_sha256.as_deref() != Some(binding_sha256)
        || origin_spec.second_group_start != second_group_start
        || origin_spec.workers.len() != workers.len()
    {
        return Err(CollectorError::Invalid(
            "origin capture does not match the current fan-out binding and barrier".to_owned(),
        ));
    }
    for (index, (old, current)) in origin_spec.workers.iter().zip(workers).enumerate() {
        if old.assignment_id != fan_out::assignment_id(index)
            || old.command != current.command
            || old.args != current.args
            || old.preamble != current.preamble
            || old.title != current.title
            || old.role != current.role
            || old.output_schema != current.output_schema
            || old.full_output_schema != current.full_output_schema
        {
            return Err(CollectorError::Invalid(format!(
                "origin assignment {} does not match the current frozen worker contract",
                fan_out::assignment_id(index)
            )));
        }
    }

    let origin_assignments = origin
        .get("assignments")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CollectorError::Invalid("verified recovery origin omitted assignment facts".to_owned())
        })?;
    let mut origin_by_id = BTreeMap::new();
    for row in origin_assignments {
        let id = row
            .get("assignment_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CollectorError::Invalid("origin assignment omitted its stable ID".to_owned())
            })?;
        if origin_by_id.insert(id.to_owned(), row).is_some() {
            return Err(CollectorError::Invalid(format!(
                "origin contains duplicate assignment `{id}`"
            )));
        }
    }

    let mut sources = BTreeMap::new();
    let mut source_descriptors = Vec::new();
    for source in &recovery.sources {
        let index = assignment_index(&source.assignment_id, workers.len())?;
        let old_spec = &origin_spec.workers[index];
        let origin_worker = origin_by_id
            .get(&source.assignment_id)
            .copied()
            .ok_or_else(|| {
                CollectorError::Invalid(format!(
                    "origin has no retained result for `{}`",
                    source.assignment_id
                ))
            })?;
        if origin_worker.get("started").and_then(Value::as_bool) != Some(true)
            || origin_worker.get("exit_code").and_then(Value::as_i64) != Some(0)
        {
            return Err(CollectorError::Invalid(format!(
                "recovery source `{}` was not a genuine completed exit-0 worker",
                source.assignment_id
            )));
        }
        let raw_digest = source.raw_stdout_sha256.as_str();
        let raw_path = origin_worker
            .get("raw_output_path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CollectorError::Invalid(format!(
                    "origin raw output path is unavailable for `{}`",
                    source.assignment_id
                ))
            })?;
        let raw_attempt = origin_worker
            .get("raw_output_attempt")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                CollectorError::Invalid(format!(
                    "origin raw attempt is unavailable for `{}`",
                    source.assignment_id
                ))
            })?;
        if raw_attempt != u64::from(source.raw_attempt)
            || origin_worker
                .get("raw_output_sha256")
                .and_then(Value::as_str)
                != Some(raw_digest)
        {
            return Err(CollectorError::Invalid(format!(
                "raw origin identity does not match `{}`",
                source.assignment_id
            )));
        }
        let _raw_bytes = read_capture_stream(&origin_capture, raw_path, raw_digest)?;
        let (delivered_input, delivered_input_path, delivered_input_sha256) =
            read_origin_delivery(&origin_capture, &old_spec.stdin_path)?;
        let current = &workers[index];
        let contract = declared_contract(current);
        if origin_worker.get("declared_output_contract") != contract.as_ref() {
            return Err(CollectorError::Invalid(format!(
                "origin output contract changed for `{}`",
                source.assignment_id
            )));
        }
        let (selected_bytes, selected_digest) = match source.source_class.as_str() {
            "original-raw" => {
                if origin_worker
                    .get("conformance_status")
                    .and_then(Value::as_str)
                    != Some("succeeded")
                    || origin_worker
                        .get("selected_attempt")
                        .and_then(Value::as_u64)
                        != Some(raw_attempt)
                    || origin_worker
                        .get("selected_output_sha256")
                        .and_then(Value::as_str)
                        != Some(raw_digest)
                    || source.selected_output_path.is_some()
                    || source.selected_output_sha256.is_some()
                    || source.derivation.is_some()
                    || source.owner_approval.is_some()
                    || source.fidelity_approval.is_some()
                {
                    return Err(CollectorError::Invalid(format!(
                        "`{}` is not a conforming original-raw source",
                        source.assignment_id
                    )));
                }
                let selected_path = origin_worker
                    .get("selected_output_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CollectorError::Invalid(format!(
                            "selected raw path is unavailable for `{}`",
                            source.assignment_id
                        ))
                    })?;
                (
                    read_capture_stream(&origin_capture, selected_path, raw_digest)?,
                    raw_digest.to_owned(),
                )
            }
            "eligible-derived" => {
                if origin_worker
                    .get("conformance_status")
                    .and_then(Value::as_str)
                    != Some("failed")
                    || source.selected_output_path.is_none()
                    || source.selected_output_sha256.is_none()
                {
                    return Err(CollectorError::Invalid(format!(
                        "`{}` is not an eligible derived source",
                        source.assignment_id
                    )));
                }
                let selected_path = source
                    .selected_output_path
                    .as_deref()
                    .expect("checked above");
                let selected_digest = source
                    .selected_output_sha256
                    .as_deref()
                    .expect("checked above");
                let selected_bytes =
                    read_artifact_stream(&packet.artifact_root, selected_path, selected_digest)?;
                validate_recovered_output(current, &selected_bytes)?;
                (selected_bytes, selected_digest.to_owned())
            }
            class => {
                return Err(CollectorError::Invalid(format!(
                    "unsupported recovery source class `{class}`"
                )));
            }
        };
        if source.source_class == "original-raw" {
            validate_recovered_output(current, &selected_bytes)?;
        }
        let destination_path = format!("{index}/stdout");
        let recovery_source = json!({
            "protocol":"fan-out-selected-source-v1",
            "source_class":source.source_class,
            "execution":"reused",
            "origin":{
                "invocation_id":recovery.origin_invocation_id,
                "run_id":recovery.run_id,
                "slot_id":recovery.slot_id,
                "state_visit":recovery.state_visit,
                "subject":recovery.subject,
                "binding_sha256":recovery.binding_sha256,
                "capture_dir":origin_capture,
                "assignment_id":source.assignment_id,
                "exit_code":origin_worker.get("exit_code"),
                "raw_attempt":source.raw_attempt,
                "raw_stdout_sha256":raw_digest,
                "raw_stdout_path":raw_path,
                "delivered_input_path":delivered_input_path,
                "delivered_input_sha256":delivered_input_sha256,
                "selected_output_sha256":origin_worker.get("selected_output_sha256"),
                "selected_output_path":origin_worker.get("selected_output_path"),
                "routed_inputs":old_spec.routed_inputs,
            },
            "selected_output_sha256":selected_digest,
            "selected_output_path":destination_path,
            "derivation":source.derivation,
            "owner_approval":source.owner_approval,
            "fidelity_approval":source.fidelity_approval,
            "declared_output_contract":contract,
        });
        sources.insert(
            source.assignment_id.clone(),
            RecoveryOutput {
                bytes: selected_bytes,
                delivered_input,
                source: recovery_source.clone(),
            },
        );
        source_descriptors.push(recovery_source);
    }

    let pending: BTreeSet<_> = recovery.pending_assignment_ids.iter().cloned().collect();
    if pending.len() != recovery.pending_assignment_ids.len() {
        return Err(CollectorError::Invalid(
            "pending recovery assignments contain duplicates".to_owned(),
        ));
    }
    let all = (0..workers.len())
        .map(fan_out::assignment_id)
        .collect::<BTreeSet<_>>();
    let covered = sources
        .keys()
        .cloned()
        .chain(pending.iter().cloned())
        .collect::<BTreeSet<_>>();
    if covered != all || sources.keys().any(|id| pending.contains(id)) {
        return Err(CollectorError::Invalid(
            "recovery groups do not cover the complete frozen assignment set".to_owned(),
        ));
    }
    for assignment in &pending {
        if origin_by_id.get(assignment).is_some_and(|row| {
            row.get("started").and_then(Value::as_bool) == Some(true)
                && row.get("exit_code").and_then(Value::as_i64) == Some(0)
                && row.get("conformance_status").and_then(Value::as_str) == Some("succeeded")
                && row
                    .get("selected_output_sha256")
                    .and_then(Value::as_str)
                    .is_some()
                && row
                    .get("selected_output_path")
                    .and_then(Value::as_str)
                    .is_some()
        }) {
            return Err(CollectorError::Invalid(format!(
                "pending assignment `{assignment}` already has a conforming completed source"
            )));
        }
    }

    let pending_ids = recovery.pending_assignment_ids.clone();
    let boundary = second_group_start.unwrap_or(workers.len());
    let first_group = (0..boundary)
        .map(fan_out::assignment_id)
        .collect::<Vec<_>>();
    let second_group = (boundary..workers.len())
        .map(fan_out::assignment_id)
        .collect::<Vec<_>>();
    let recovery_metadata = json!({
        "protocol":"fan-out-recovery-result-v1",
        "origin_invocation_id":recovery.origin_invocation_id,
        "origin_status":"failed",
        "state_visit":state_visit,
        "binding_sha256":binding_sha256,
        "subject":recovery.subject,
        "selected_sources":source_descriptors,
        "pending_assignment_ids":pending_ids,
        "barriers":{"second_group_start":second_group_start,"first_group":first_group,"second_group":second_group},
    });
    let assignment_ids = (0..workers.len())
        .map(fan_out::assignment_id)
        .collect::<Vec<_>>();
    let artifact_root = packet.artifact_root.clone();
    let payloads = workers
        .iter()
        .map(|worker| {
            fan_out::bound_worker_payload(
                worker,
                &artifact_root,
                packet.context.as_deref(),
                packet.controls.force_fresh.then_some(&packet.controls),
            )
        })
        .collect::<Vec<_>>();
    let output_dir = PathBuf::from(&packet.capture_dir);
    let dagu = dagu::resolve_dagu().map_err(|error| CollectorError::Failed(error.to_string()))?;
    fan_out::run_dagu_graph(
        &dagu,
        workers,
        &assignment_ids,
        &payloads,
        Some(packet.context.as_deref().unwrap_or_default()),
        &output_dir,
        max_active,
        second_group_start,
        Some(FanOutIdentity {
            run_id: packet.run_id.clone(),
            slot_id: packet.slot_id.clone(),
            state_visit: packet.state_visit,
            binding_sha256: packet.binding_sha256.clone(),
        }),
        Some(recovery_metadata),
        Some(&sources),
    )
}

fn attempt_manifest_projection(capture: &Path, worker: &Value) -> Option<Value> {
    let relative = worker.get("attempts_path")?.as_str()?;
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let path = capture.join(relative);
    if fs::metadata(&path).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let manifest: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    Some(json!({
        "schema_version":manifest.get("schema_version"),
        "recovery_state":manifest.get("recovery_state"),
        "selected_attempt":manifest.get("selected_attempt"),
        "exhausted":manifest.get("exhausted"),
        "attempt_count":manifest.get("attempts").and_then(Value::as_array).map(Vec::len),
    }))
}

fn assignment_index(id: &str, count: usize) -> Result<usize, CollectorError> {
    id.strip_prefix("worker-")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|index| *index < count && fan_out::assignment_id(*index) == id)
        .ok_or_else(|| CollectorError::Invalid(format!("invalid recovery assignment ID `{id}`")))
}

fn declared_contract(worker: &WorkerCli) -> Option<Value> {
    worker.full_output_schema.clone().or_else(|| {
        worker
            .output_schema
            .as_ref()
            .map(|schema| serde_json::to_value(schema).unwrap_or(Value::Null))
    })
}

fn read_origin_delivery(
    capture: &Path,
    delivered_path: &str,
) -> Result<(Vec<u8>, String, String), CollectorError> {
    let delivered_path = Path::new(delivered_path);
    if !delivered_path.is_absolute() {
        return Err(CollectorError::Invalid(
            "origin worker input path is not absolute".to_owned(),
        ));
    }
    let canonical_path = fs::canonicalize(delivered_path).map_err(|error| {
        CollectorError::Invalid(format!("origin worker input is unavailable: {error}"))
    })?;
    if !canonical_path.starts_with(capture) || !canonical_path.is_file() {
        return Err(CollectorError::Invalid(
            "origin worker input escapes its capture directory".to_owned(),
        ));
    }
    let relative = canonical_path
        .strip_prefix(capture)
        .map_err(|error| CollectorError::Invalid(error.to_string()))?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CollectorError::Invalid(
            "origin worker input path is not capture-relative".to_owned(),
        ));
    }
    let relative = relative
        .to_str()
        .ok_or_else(|| CollectorError::Invalid("origin worker input path is not UTF-8".to_owned()))?
        .to_owned();
    let bytes = fs::read(&canonical_path).map_err(|error| {
        CollectorError::Invalid(format!("could not read origin worker input: {error}"))
    })?;
    let digest = fan_out::sha256_digest(&bytes);
    Ok((bytes, relative, digest))
}

fn read_capture_stream(
    capture: &Path,
    relative: &str,
    digest: &str,
) -> Result<Vec<u8>, CollectorError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CollectorError::Invalid(
            "origin stream path is not capture-relative".to_owned(),
        ));
    }
    let path = capture.join(relative);
    let bytes = fs::read(&path).map_err(|error| {
        CollectorError::Invalid(format!(
            "origin stream `{}` is unavailable: {error}",
            path.display()
        ))
    })?;
    if fan_out::sha256_digest(&bytes) != digest {
        return Err(CollectorError::Invalid(format!(
            "origin stream `{}` does not match its digest",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_artifact_stream(root: &str, path: &str, digest: &str) -> Result<Vec<u8>, CollectorError> {
    let root = fs::canonicalize(root).map_err(|error| {
        CollectorError::Invalid(format!("artifact_root is unavailable: {error}"))
    })?;
    let path = fs::canonicalize(path).map_err(|error| {
        CollectorError::Invalid(format!("derived output is unavailable: {error}"))
    })?;
    if !path.starts_with(&root) {
        return Err(CollectorError::Invalid(
            "derived output is outside the run artifact_root".to_owned(),
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        CollectorError::Invalid(format!("derived output is unavailable: {error}"))
    })?;
    if fan_out::sha256_digest(&bytes) != digest {
        return Err(CollectorError::Invalid(
            "derived output bytes do not match their digest".to_owned(),
        ));
    }
    Ok(bytes)
}

fn validate_recovered_output(worker: &WorkerCli, bytes: &[u8]) -> Result<(), CollectorError> {
    if let Some(schema) = worker.full_output_schema.as_ref() {
        let errors = fan_out::complete_schema_validation_errors(
            bytes,
            schema,
            worker.output_schema.as_ref(),
        );
        if !errors.is_empty() {
            return Err(CollectorError::Invalid(format!(
                "recovered output does not conform to the frozen contract: {}",
                errors.join("; ")
            )));
        }
        return Ok(());
    }
    if let Some(schema) = worker.output_schema.as_ref() {
        return fan_out::evaluate_output_conformance_bytes(bytes, schema)
            .map_err(CollectorError::Invalid);
    }
    Err(CollectorError::Invalid(
        "recovery source has no frozen output contract".to_owned(),
    ))
}

/// Build an inert source/pending preview from one complete public `show`.
pub(crate) fn preview_recovery(show_json: &str, origin_id: &str) -> Result<Value, String> {
    let show: Value = serde_json::from_str(show_json)
        .map_err(|error| format!("show input is not JSON: {error}"))?;
    let result = show.get("result").ok_or("show envelope omitted result")?;
    if result.get("lifecycle").and_then(Value::as_str) != Some("active") {
        return Err("recovery preview requires an active run".to_owned());
    }
    let run_id = result
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or("show omitted run_id")?;
    let state_visit = result
        .get("state_visit")
        .and_then(Value::as_u64)
        .ok_or("show omitted state_visit")?;
    let current_state = result
        .get("current_state")
        .and_then(Value::as_str)
        .ok_or("show omitted current_state")?;
    let initial = result
        .get("initial_input")
        .and_then(Value::as_object)
        .ok_or("show omitted initial_input")?;
    let artifact_root = initial
        .get("artifact_root")
        .and_then(Value::as_str)
        .ok_or("show omitted artifact_root")?;
    if !Path::new(artifact_root).is_absolute() {
        return Err("recovery preview requires an absolute artifact_root".to_owned());
    }
    let invocations = result
        .get("work_slot_invocations")
        .and_then(Value::as_array)
        .ok_or("full show omitted work_slot_invocations")?;
    let origin = invocations
        .iter()
        .find(|row| row.get("invocation_id").and_then(Value::as_str) == Some(origin_id))
        .ok_or_else(|| format!("origin invocation `{origin_id}` is not in this run"))?;
    let slot_id = origin
        .get("slot_id")
        .and_then(Value::as_str)
        .ok_or("origin omitted slot_id")?;
    if origin.get("status").and_then(Value::as_str) != Some("failed") {
        return Err("recovery origin must be a failed or verified-cancelled invocation".to_owned());
    }
    let ownership = origin
        .get("ownership")
        .and_then(Value::as_object)
        .ok_or("origin has no verifiable ownership/cleanup record")?;
    if ownership.get("live_owned_work").and_then(Value::as_bool) != Some(false)
        || ownership.get("cleanup_pending").and_then(Value::as_bool) != Some(false)
    {
        return Err("recovery origin still has live owned work or cleanup pending".to_owned());
    }
    if origin.get("state_visit").and_then(Value::as_u64) != Some(state_visit)
        || current_state
            != result
                .get("workflow_graph")
                .and_then(|graph| graph.get("work_slots"))
                .and_then(Value::as_array)
                .and_then(|slots| {
                    slots
                        .iter()
                        .find(|slot| slot.get("id").and_then(Value::as_str) == Some(slot_id))
                })
                .and_then(|slot| slot.get("state"))
                .and_then(Value::as_str)
                .unwrap_or("")
    {
        return Err("recovery origin is not from the current slot visit".to_owned());
    }
    let binding_value = origin
        .get("binding")
        .cloned()
        .ok_or("origin omitted frozen binding")?;
    let binding: loop_core::WorkSlotBinding = serde_json::from_value(binding_value.clone())
        .map_err(|error| format!("origin binding is malformed: {error}"))?;
    let effective_value = result
        .get("effective_bindings")
        .and_then(Value::as_object)
        .and_then(|bindings| bindings.get(slot_id))
        .ok_or("show omitted effective slot binding")?;
    let effective: loop_core::WorkSlotBinding = serde_json::from_value(effective_value.clone())
        .map_err(|error| format!("effective binding is malformed: {error}"))?;
    if binding != effective {
        return Err("recovery refuses an origin whose effective frozen binding changed".to_owned());
    }
    let binding_sha256 = loop_core::work_slot_binding_digest(&binding);
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("could not identify loop-engine executable: {error}"))?;
    let current_exe = fs::canonicalize(current_exe).unwrap_or_else(|_| PathBuf::from(""));
    let binding_exe =
        fs::canonicalize(&binding.command).unwrap_or_else(|_| PathBuf::from(&binding.command));
    if current_exe != binding_exe {
        return Err("recovery preview requires the engine's own frozen fan-out binding".to_owned());
    }
    let parsed = fan_out::parse_fan_out_args(binding.args.iter().skip(1).map(String::as_str))
        .map_err(|error| format!("frozen fan-out binding is invalid: {error}"))?;
    if binding.args.first().map(String::as_str) != Some("fan-out")
        || parsed.instructions_path.is_some()
        || parsed.workers.is_empty()
    {
        return Err("origin is not an enumerable bound fan-out invocation".to_owned());
    }
    let capture_dir = origin
        .get("capture_dir")
        .and_then(Value::as_str)
        .ok_or("origin omitted capture_dir")?;
    let expected_capture = Path::new(artifact_root)
        .join("work-slot-captures")
        .join(slot_id)
        .join(origin_id);
    if PathBuf::from(capture_dir) != expected_capture {
        return Err(
            "origin capture is not the engine-allocated capture for this run and slot".to_owned(),
        );
    }
    let capture_root = fs::canonicalize(capture_dir)
        .map_err(|error| format!("origin capture is unavailable: {error}"))?;
    let artifact_root_canon = fs::canonicalize(artifact_root)
        .map_err(|error| format!("artifact_root is unavailable: {error}"))?;
    if !capture_root.starts_with(&artifact_root_canon) {
        return Err("origin capture is outside artifact_root".to_owned());
    }
    let spec = fan_out::read_spec(&capture_root).map_err(|error| error.to_string())?;
    if spec.run_id.as_deref() != Some(run_id)
        || spec.slot_id.as_deref() != Some(slot_id)
        || spec.state_visit != Some(state_visit)
        || spec.binding_sha256.as_deref() != Some(binding_sha256.as_str())
        || spec.second_group_start != parsed.second_group_start
        || spec.workers.len() != parsed.workers.len()
    {
        return Err(
            "origin fan-out spec does not match its recorded binding, visit, and barrier"
                .to_owned(),
        );
    }
    let mut inner_by_id = BTreeMap::new();
    if let Some(inner) = origin.get("inner_workers").and_then(Value::as_array) {
        for worker in inner {
            let id = worker
                .get("assignment_id")
                .and_then(Value::as_str)
                .ok_or("origin worker omitted stable assignment ID")?;
            if inner_by_id.insert(id.to_owned(), worker).is_some() {
                return Err(format!(
                    "origin contains duplicate worker assignment `{id}`"
                ));
            }
        }
    }
    let mut assignment_rows = Vec::new();
    let mut sources = Vec::new();
    let mut pending = Vec::new();
    for (index, (spec_worker, configured)) in spec.workers.iter().zip(&parsed.workers).enumerate() {
        let id = fan_out::assignment_id(index);
        if spec_worker.assignment_id != id
            || spec_worker.command != configured.command
            || spec_worker.args != configured.args
            || spec_worker.preamble != configured.preamble
            || spec_worker.title != configured.title
            || spec_worker.role != configured.role
            || spec_worker.output_schema != configured.output_schema
            || spec_worker.full_output_schema != configured.full_output_schema
        {
            return Err(format!(
                "origin capture worker `{id}` differs from its frozen binding"
            ));
        }
        let group = if parsed
            .second_group_start
            .is_some_and(|boundary| index >= boundary)
        {
            "second"
        } else {
            "first"
        };
        let mut classification = "unknown";
        let mut source = None;
        let mut raw_output = None;
        if let Some(row) = inner_by_id.get(&id).copied() {
            let started = row.get("started").and_then(Value::as_bool);
            let exit_code = row.get("exit_code").and_then(Value::as_i64);
            let raw_digest = row.get("raw_output_sha256").and_then(Value::as_str);
            let raw_path = row.get("raw_output_path").and_then(Value::as_str);
            let raw_attempt = row.get("raw_output_attempt").and_then(Value::as_u64);
            if let (Some(digest), Some(path), Some(attempt)) = (raw_digest, raw_path, raw_attempt) {
                let raw_valid = read_capture_stream(&capture_root, path, digest).is_ok();
                raw_output = Some(
                    json!({"attempt":attempt,"sha256":digest,"path":path,"available":raw_valid}),
                );
            }
            let contract = declared_contract(&configured);
            let contract_matches = row.get("declared_output_contract") == contract.as_ref();
            let selected = row
                .get("selected_output_sha256")
                .and_then(Value::as_str)
                .zip(row.get("selected_output_path").and_then(Value::as_str))
                .filter(|(digest, path)| read_capture_stream(&capture_root, path, digest).is_ok());
            let conforms = match selected {
                Some((_, path)) => {
                    let bytes = read_capture_stream(
                        &capture_root,
                        path,
                        row["selected_output_sha256"].as_str().unwrap_or(""),
                    );
                    bytes
                        .ok()
                        .is_some_and(|bytes| validate_recovered_output(&configured, &bytes).is_ok())
                }
                None => false,
            };
            if started == Some(false) {
                classification = "never-started";
            } else if started != Some(true) {
                classification = "unknown";
            } else if exit_code != Some(0) {
                classification = "unfinished-or-failed";
            } else if !contract_matches
                || row.get("conformance_status").and_then(Value::as_str) != Some("succeeded")
                || !conforms
            {
                classification = "invalid";
            } else if row.get("recovery_source").is_some() {
                classification = "recovered-source-needs-explicit-lineage-review";
            } else if let (Some((_, _)), Some(digest), Some(attempt)) = (
                selected,
                row.get("raw_output_sha256").and_then(Value::as_str),
                row.get("raw_output_attempt").and_then(Value::as_u64),
            ) {
                if Some(digest) == row.get("selected_output_sha256").and_then(Value::as_str)
                    && Some(attempt) == row.get("selected_attempt").and_then(Value::as_u64)
                {
                    classification = "conforming-completed";
                    source = Some(
                        json!({"assignment_id":id,"source_class":"original-raw","raw_attempt":attempt,"raw_stdout_sha256":digest}),
                    );
                    sources.push(source.clone().unwrap());
                } else {
                    classification = "invalid";
                }
            } else {
                classification = "invalid";
            }
        }
        if source.is_none() {
            pending.push(id.clone());
        }
        let attempt_manifest = inner_by_id
            .get(&id)
            .and_then(|row| attempt_manifest_projection(&capture_root, row));
        assignment_rows.push(json!({
            "assignment_id":id,"group":group,"classification":classification,
            "started":inner_by_id.get(&id).and_then(|row|row.get("started")).cloned(),
            "exit_code":inner_by_id.get(&id).and_then(|row|row.get("exit_code")).cloned(),
            "conformance_status":inner_by_id.get(&id).and_then(|row|row.get("conformance_status")).cloned(),
            "conformance_error":inner_by_id.get(&id).and_then(|row|row.get("conformance_error")).cloned(),
            "declared_output_contract":inner_by_id.get(&id).and_then(|row|row.get("declared_output_contract")).cloned().or_else(||declared_contract(&configured)),
            "raw_output":raw_output,"attempt_manifest":attempt_manifest,"selected_source":source,
        }));
    }
    let subject = origin
        .get("subject")
        .and_then(Value::as_str)
        .ok_or("origin omitted subject")?;
    let first_group = (0..parsed.second_group_start.unwrap_or(parsed.workers.len()))
        .map(fan_out::assignment_id)
        .collect::<Vec<_>>();
    let second_group = (parsed.second_group_start.unwrap_or(parsed.workers.len())
        ..parsed.workers.len())
        .map(fan_out::assignment_id)
        .collect::<Vec<_>>();
    let recovery_input = json!({
        "protocol":"fan-out-recovery-v1","run_id":run_id,"slot_id":slot_id,
        "state_visit":state_visit,"subject":subject,"binding_sha256":binding_sha256,
        "origin_invocation_id":origin_id,"pending_assignment_ids":pending,"sources":sources,
    });
    Ok(json!({
        "schema_version":1,"ready":true,
        "target":{"run_id":run_id,"slot_id":slot_id,"state_visit":state_visit,"subject":subject,"binding_sha256":binding_sha256,"artifact_root":artifact_root},
        "origin":{"invocation_id":origin_id,"status":"failed","exit_code":origin.get("exit_code"),"capture_dir":capture_dir,
                   "quiescent":true,"cleanup_pending":false,"live_owned_work":false},
        "barriers":{"second_group_start":parsed.second_group_start,"first_group":first_group,"second_group":second_group},
        "assignments":assignment_rows,"recovery_input":recovery_input,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoverOutputRequest {
    version: u32,
    preview: Value,
    assignment_id: String,
    #[serde(default, alias = "scripted_adapter")]
    adapter: Option<RepairAdapterConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairAdapterConfig {
    kind: String,
    #[serde(default)]
    model_id: Option<String>,
    command: String,
    args: Vec<String>,
    max_calls: u32,
    max_time_ms: u64,
    max_cost_micros: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairAdapterResponse {
    output: String,
    usage: RepairAdapterUsage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairAdapterUsage {
    calls: u32,
    metered_cost_micros: u64,
}

struct RepairAdapterRun {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    elapsed_ms: u64,
    status_success: bool,
    timed_out: bool,
}

const MAX_REPAIR_RAW_BYTES: usize = 1024 * 1024;
const MAX_ADAPTER_PACKET_BYTES: usize = 2 * 1024 * 1024;

/// Repair a captured output without changing its original invocation/capture.
/// The deterministic effect-key rename always runs first. A separately
/// configured scripted adapter is considered only when that normalizer
/// explicitly declines the raw bytes.
pub(crate) fn recover_output(request_json: &str) -> Result<Value, String> {
    let request: RecoverOutputRequest = serde_json::from_str(request_json)
        .map_err(|error| format!("repair request is malformed: {error}"))?;
    if request.version != 1 {
        return Err("repair request version must be 1".to_owned());
    }
    let preview = &request.preview;
    if preview.get("ready").and_then(Value::as_bool) != Some(true) {
        return Err("repair requires a ready recovery preview".to_owned());
    }
    let origin = preview.get("origin").ok_or("preview omitted origin")?;
    if origin.get("status").and_then(Value::as_str) != Some("failed")
        || origin.get("quiescent").and_then(Value::as_bool) != Some(true)
        || origin.get("cleanup_pending").and_then(Value::as_bool) != Some(false)
        || origin.get("live_owned_work").and_then(Value::as_bool) != Some(false)
    {
        return Err("output repair requires a verified quiescent failed origin".to_owned());
    }
    let assignments = preview
        .get("assignments")
        .and_then(Value::as_array)
        .ok_or("preview omitted assignments")?;
    let matches = assignments
        .iter()
        .filter(|row| {
            row.get("assignment_id").and_then(Value::as_str) == Some(request.assignment_id.as_str())
        })
        .collect::<Vec<_>>();
    let row = matches.first().copied().ok_or_else(|| {
        format!(
            "assignment `{}` is not in the recovery preview",
            request.assignment_id
        )
    })?;
    if matches.len() != 1 || row.get("classification").and_then(Value::as_str) != Some("invalid") {
        return Err("output repair requires one captured invalid contracted assignment".to_owned());
    }
    let raw = row
        .get("raw_output")
        .and_then(Value::as_object)
        .ok_or("invalid assignment has no raw-output identity")?;
    if raw.get("available").and_then(Value::as_bool) != Some(true) {
        return Err("original raw output is unavailable or changed".to_owned());
    }
    let raw_path = raw
        .get("path")
        .and_then(Value::as_str)
        .ok_or("raw output omitted path")?;
    let raw_digest = raw
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or("raw output omitted digest")?;
    let raw_attempt = raw
        .get("attempt")
        .and_then(Value::as_u64)
        .ok_or("raw output omitted attempt")?;
    let capture_dir = origin
        .get("capture_dir")
        .and_then(Value::as_str)
        .ok_or("origin omitted capture_dir")?;
    let capture_root = fs::canonicalize(capture_dir)
        .map_err(|error| format!("origin capture is unavailable: {error}"))?;
    let raw_bytes = read_capture_stream(&capture_root, raw_path, raw_digest)
        .map_err(|error| error.to_string())?;
    if raw_bytes.len() > MAX_REPAIR_RAW_BYTES {
        return Err("raw output exceeds the bounded repair size".to_owned());
    }
    let slot_id = preview
        .pointer("/target/slot_id")
        .and_then(Value::as_str)
        .ok_or("preview omitted target slot")?;
    let index = request
        .assignment_id
        .strip_prefix("worker-")
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or("assignment ID is not a fan-out identity")?;
    let spec = fan_out::read_spec(&capture_root).map_err(|error| error.to_string())?;
    let spec_worker = spec
        .workers
        .get(index)
        .ok_or("assignment is not in the origin fan-out spec")?;
    if spec_worker.assignment_id != request.assignment_id {
        return Err("assignment identity differs from the origin capture".to_owned());
    }
    let legacy_schema = spec_worker.output_schema.as_ref();
    let full_schema = spec_worker.full_output_schema.as_ref();
    if legacy_schema.is_none() && full_schema.is_none() {
        return Err("output-only repair requires a frozen output contract".to_owned());
    }
    if let Some(full_schema) = full_schema {
        if !fan_out::uses_repair_first_output_recovery(full_schema) {
            return Err(
                "output-only repair refuses legacy full_output_schema same-worker retry captures"
                    .to_owned(),
            );
        }
        verify_repair_first_attempt_manifest(
            &capture_root,
            index,
            raw_attempt,
            raw_digest,
            full_schema,
            legacy_schema,
        )?;
    }
    let root = preview
        .pointer("/target/artifact_root")
        .and_then(Value::as_str)
        .or_else(|| {
            preview
                .pointer("/run/artifact_root")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            preview
                .pointer("/initial_input/artifact_root")
                .and_then(Value::as_str)
        })
        .or_else(|| preview.get("artifact_root").and_then(Value::as_str));
    let root = root.ok_or("preview omitted artifact_root")?;
    let root =
        fs::canonicalize(root).map_err(|error| format!("artifact_root is unavailable: {error}"))?;
    if !capture_root.starts_with(&root) {
        return Err("origin capture is outside artifact_root".to_owned());
    }
    let contract = declared_contract_from_spec(spec_worker);
    let raw_value = fan_out::locate_stdout_value(&raw_bytes);
    let mechanical = raw_value.and_then(|value| {
        if let Some(schema) = legacy_schema {
            mechanical_normalize(value, schema)
        } else if let Some(schema) = full_schema {
            mechanical_normalize_full_schema(value, schema, legacy_schema)
        } else {
            Err("frozen output contract is unavailable".to_owned())
        }
    });
    let (derived_bytes, method, difference, adapter_evidence, adapter_capture) = match mechanical {
        Ok((bytes, difference)) => (bytes, "mechanical".to_owned(), difference, None, None),
        Err(mechanical_decline) => {
            let adapter = request.adapter.as_ref().ok_or_else(|| {
                format!("mechanical normalizer declined ({mechanical_decline}); no output-only adapter is configured")
            })?;
            if adapter.command.trim().is_empty()
                || adapter.max_calls == 0
                || adapter.max_time_ms == 0
                || adapter.max_cost_micros == 0
            {
                return Err(
                    "output-only adapter requires positive call, time, and metered-cost caps"
                        .to_owned(),
                );
            }
            match adapter.kind.as_str() {
                "model" => {
                    if !full_schema.is_some_and(fan_out::uses_repair_first_output_recovery) {
                        return Err("configured model repair requires a repair-first-v1 output contract; legacy contracts remain unchanged".to_owned());
                    }
                    let model_id = adapter
                        .model_id
                        .as_deref()
                        .filter(|model_id| !model_id.trim().is_empty())
                        .ok_or("model adapter requires an explicit non-empty model_id")?;
                    let (output, usage, capture) = run_model_adapter(
                        &root,
                        preview,
                        origin,
                        &request.assignment_id,
                        raw_path,
                        raw_attempt,
                        raw_digest,
                        &raw_bytes,
                        full_schema,
                        legacy_schema,
                        adapter,
                        model_id,
                    )?;
                    validate_recovered_contract(&output, full_schema, legacy_schema).map_err(
                        |error| format!(
                            "model adapter output does not satisfy the frozen contract: {error}; retained capture: {}",
                            capture.get("directory").and_then(Value::as_str).unwrap_or("unknown")
                        ),
                    )?;
                    let diff = json!({
                        "kind":"representation-only",
                        "mechanical_decline":mechanical_decline,
                        "raw_sha256":raw_digest,
                        "derived_sha256":fan_out::sha256_digest(&output),
                        "raw_bytes":raw_bytes.len(),
                        "derived_bytes":output.len(),
                        "raw_text":String::from_utf8_lossy(&raw_bytes),
                        "derived_text":String::from_utf8_lossy(&output),
                    });
                    (output, "model".to_owned(), diff, Some(usage), Some(capture))
                }
                "scripted" => {
                    if adapter.model_id.is_some() {
                        return Err("scripted adapter must not claim a model_id".to_owned());
                    }
                    let request_packet = json!({
                        "version":1,
                        "representation_only":true,
                        "assignment_id":request.assignment_id,
                        "raw_output_hex":hex_encode(&raw_bytes),
                        "raw_output_sha256":raw_digest,
                        "required_keys":legacy_schema.map(|schema| schema.required.clone()).or_else(|| {
                            full_schema
                                .and_then(|schema| schema.get("required"))
                                .and_then(Value::as_array)
                                .map(|required| required.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                        }),
                        "full_output_schema":full_schema,
                    });
                    let packet_bytes =
                        serde_json::to_vec(&request_packet).map_err(|error| error.to_string())?;
                    if packet_bytes.len() > MAX_ADAPTER_PACKET_BYTES {
                        return Err("scripted adapter packet exceeds its bound".to_owned());
                    }
                    let run = run_repair_adapter(
                        &adapter.command,
                        &adapter.args,
                        &packet_bytes,
                        adapter.max_time_ms,
                    )?;
                    let attempt_key = fan_out::sha256_digest(
                        format!(
                            "{}:{}:{}:{}",
                            origin
                                .get("invocation_id")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            request.assignment_id,
                            std::process::id(),
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos(),
                        )
                        .as_bytes(),
                    );
                    let adapter_dir = root
                        .join("recovery-adapter-attempts")
                        .join(&attempt_key[7..]);
                    fs::create_dir_all(&adapter_dir).map_err(|error| {
                        format!("could not create adapter attempt capture: {error}")
                    })?;
                    write_immutable(&adapter_dir.join("request.json"), &packet_bytes)?;
                    write_immutable(&adapter_dir.join("stdout"), &run.stdout)?;
                    write_immutable(&adapter_dir.join("stderr"), &run.stderr)?;
                    write_immutable(
                        &adapter_dir.join("configuration.json"),
                        &serde_json::to_vec(&json!({
                            "kind":adapter.kind,"command":adapter.command,"args":adapter.args,
                            "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
                            "max_cost_micros":adapter.max_cost_micros,
                        }))
                        .map_err(|error| error.to_string())?,
                    )?;
                    if run.timed_out {
                        return Err(format!(
                            "scripted adapter exceeded its positive time cap; retained capture: {}",
                            adapter_dir.display()
                        ));
                    }
                    if !run.status_success {
                        return Err(format!(
                            "configured scripted adapter exited unsuccessfully; retained capture: {}",
                            adapter_dir.display()
                        ));
                    }
                    if run.elapsed_ms > adapter.max_time_ms {
                        return Err(format!(
                            "scripted adapter exceeded its positive time cap; retained capture: {}",
                            adapter_dir.display()
                        ));
                    }
                    let response: RepairAdapterResponse = serde_json::from_slice(&run.stdout)
                        .map_err(|error|format!("scripted adapter usage/response is missing or malformed; retained capture {}: {error}",adapter_dir.display()))?;
                    let output = response.output.into_bytes();
                    write_immutable(
                        &adapter_dir.join("usage.json"),
                        &serde_json::to_vec(&json!({
                            "usage_accounted":true,"calls":response.usage.calls,"elapsed_ms":run.elapsed_ms,
                            "metered_cost_micros":response.usage.metered_cost_micros,
                            "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
                            "max_cost_micros":adapter.max_cost_micros,
                        }))
                        .map_err(|error| error.to_string())?,
                    )?;
                    if response.usage.calls != 1 || response.usage.calls > adapter.max_calls {
                        return Err(format!(
                            "scripted adapter exceeded or omitted its positive call cap; retained capture: {}",
                            adapter_dir.display()
                        ));
                    }
                    if response.usage.metered_cost_micros > adapter.max_cost_micros {
                        return Err(format!(
                            "scripted adapter exceeded its positive metered-cost cap; retained capture: {}",
                            adapter_dir.display()
                        ));
                    }
                    validate_recovered_contract(&output, full_schema, legacy_schema)
                        .map_err(|error| format!("scripted adapter output does not satisfy the frozen contract: {error}; retained capture: {}",adapter_dir.display()))?;
                    let diff = json!({
                        "kind":"representation-only",
                        "mechanical_decline":mechanical_decline,
                        "raw_sha256":raw_digest,
                        "derived_sha256":fan_out::sha256_digest(&output),
                        "raw_bytes":raw_bytes.len(),
                        "derived_bytes":output.len(),
                        "raw_text":String::from_utf8_lossy(&raw_bytes),
                        "derived_text":String::from_utf8_lossy(&output),
                    });
                    let usage = json!({
                        "command":adapter.command,"args":adapter.args,
                        "usage_accounted":true,"calls":response.usage.calls,"elapsed_ms":run.elapsed_ms,
                        "metered_cost_micros":response.usage.metered_cost_micros,
                        "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
                        "max_cost_micros":adapter.max_cost_micros,
                    });
                    let capture = json!({"directory":adapter_dir,"stdout_sha256":fan_out::sha256_digest(&run.stdout),"stderr_sha256":fan_out::sha256_digest(&run.stderr),"request_sha256":fan_out::sha256_digest(&packet_bytes)});
                    (
                        output,
                        "scripted".to_owned(),
                        diff,
                        Some(usage),
                        Some(capture),
                    )
                }
                other => return Err(format!("unsupported output-only adapter kind `{other}`")),
            }
        }
    };
    validate_recovered_contract(&derived_bytes, full_schema, legacy_schema)
        .map_err(|error| format!("derived output does not satisfy the frozen contract: {error}"))?;
    let derived_digest = fan_out::sha256_digest(&derived_bytes);
    let root_path = PathBuf::from(root);
    let path_key = fan_out::sha256_digest(
        format!(
            "{}:{}",
            origin
                .get("invocation_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            request.assignment_id
        )
        .as_bytes(),
    );
    let derived_dir = root_path
        .join("recovery-derived")
        .join(&path_key[7..])
        .join(&derived_digest[7..]);
    fs::create_dir_all(&derived_dir)
        .map_err(|error| format!("could not create derived-output capture: {error}"))?;
    let derived_path = derived_dir.join("derived-output.json");
    write_immutable(&derived_path, &derived_bytes)?;
    let mut adapter_capture_json = Value::Null;
    if let Some(capture) = adapter_capture {
        adapter_capture_json = json!({"capture":capture,"usage":adapter_evidence.clone()});
    }
    Ok(json!({
        "protocol":"fan-out-derived-output-v1","source_class":"eligible-derived",
        "origin":{"run_id":preview.pointer("/target/run_id"),"slot_id":slot_id,
                  "state_visit":preview.pointer("/target/state_visit"),"subject":preview.pointer("/target/subject"),
                  "binding_sha256":preview.pointer("/target/binding_sha256"),
                  "invocation_id":origin.get("invocation_id"),"assignment_id":request.assignment_id,
                  "raw_attempt":raw_attempt,"raw_stdout_sha256":raw_digest,"raw_stdout_path":raw_path,
                  "capture_dir":capture_dir},
        "selected":{"path":derived_path,"sha256":derived_digest,"byte_length":derived_bytes.len()},
        "derivation":{"kind":method,"difference":difference,"adapter":adapter_evidence},
        "adapter_capture":adapter_capture_json,
        "declared_output_contract":contract,
    }))
}

struct ModelBudgetLock {
    path: PathBuf,
}

impl Drop for ModelBudgetLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Default)]
struct ModelBudgetUsage {
    calls: u64,
    elapsed_ms: u64,
    metered_cost_micros: u64,
}

fn model_budget_directory(
    root: &Path,
    run_id: &str,
    slot_id: &str,
    state_visit: u64,
    binding_sha256: &str,
    assignment_id: &str,
) -> PathBuf {
    let identity = fan_out::sha256_digest(
        format!("{run_id}:{slot_id}:{state_visit}:{binding_sha256}:{assignment_id}").as_bytes(),
    );
    root.join("recovery-adapter-attempts")
        .join("model")
        .join(&identity[7..])
}

fn acquire_model_budget_lock(directory: &Path) -> Result<ModelBudgetLock, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create model budget capture directory: {error}"))?;
    let path = directory.join("budget.lock");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "model repair assignment budget is already locked or has unfinished accounting"
                    .to_owned()
            } else {
                format!("could not lock model repair assignment budget: {error}")
            }
        })?;
    if let Err(error) = writeln!(
        file,
        "pid={} started_at_ns={}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
    .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(format!(
            "could not retain model budget lock identity: {error}"
        ));
    }
    Ok(ModelBudgetLock { path })
}

fn load_model_budget_usage(
    root: &Path,
    directory: &Path,
    run_id: &str,
    slot_id: &str,
    state_visit: u64,
    binding_sha256: &str,
    assignment_id: &str,
    adapter: &RepairAdapterConfig,
    model_id: &str,
) -> Result<ModelBudgetUsage, String> {
    let mut total = ModelBudgetUsage::default();
    let adapter_args = Value::Array(adapter.args.iter().cloned().map(Value::String).collect());
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("could not inspect model repair budget history: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not inspect model repair attempt: {error}"))?;
        if entry.file_name() == "budget.lock" {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect model repair attempt: {error}"))?;
        if !file_type.is_dir() {
            return Err("model repair history contains an unexpected entry".to_owned());
        }
        let attempt_dir = entry.path();
        let configuration = read_json_bounded(&attempt_dir.join("configuration.json"), 64 * 1024)?;
        if configuration.get("kind").and_then(Value::as_str) != Some("model")
            || configuration.get("model_id").and_then(Value::as_str) != Some(model_id)
            || configuration.get("command").and_then(Value::as_str)
                != Some(adapter.command.as_str())
            || configuration.get("args") != Some(&adapter_args)
            || configuration.get("run_id").and_then(Value::as_str) != Some(run_id)
            || configuration.get("slot_id").and_then(Value::as_str) != Some(slot_id)
            || configuration.get("state_visit").and_then(Value::as_u64) != Some(state_visit)
            || configuration.get("binding_sha256").and_then(Value::as_str) != Some(binding_sha256)
            || configuration.get("assignment_id").and_then(Value::as_str) != Some(assignment_id)
            || configuration.get("max_calls").and_then(Value::as_u64)
                != Some(u64::from(adapter.max_calls))
            || configuration.get("max_time_ms").and_then(Value::as_u64) != Some(adapter.max_time_ms)
            || configuration.get("max_cost_micros").and_then(Value::as_u64)
                != Some(adapter.max_cost_micros)
        {
            return Err(
                "model repair identity or per-assignment bounds differ from prior attempts"
                    .to_owned(),
            );
        }
        let request =
            read_bounded_file(&attempt_dir.join("request.json"), MAX_ADAPTER_PACKET_BYTES)?;
        let request_value: Value = serde_json::from_slice(&request)
            .map_err(|error| format!("prior model repair request is malformed: {error}"))?;
        let raw_digest = configuration
            .get("raw_stdout_sha256")
            .and_then(Value::as_str)
            .ok_or("prior model repair capture omitted its raw-attempt digest")?;
        let raw_path = configuration
            .get("raw_stdout_path")
            .and_then(Value::as_str)
            .ok_or("prior model repair capture omitted its raw-attempt path")?;
        let raw_attempt = configuration
            .get("raw_attempt")
            .and_then(Value::as_u64)
            .filter(|attempt| *attempt > 0)
            .ok_or("prior model repair capture omitted its raw-attempt number")?;
        let origin_capture_dir = configuration
            .get("origin_capture_dir")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .ok_or("prior model repair capture omitted its origin capture directory")?;
        let origin_capture_dir = fs::canonicalize(origin_capture_dir).map_err(|error| {
            format!("prior model repair origin capture is unavailable: {error}")
        })?;
        if !origin_capture_dir.starts_with(root) {
            return Err("prior model repair origin capture is outside artifact_root".to_owned());
        }
        let _verified_raw = read_capture_stream(&origin_capture_dir, raw_path, raw_digest)
            .map_err(|error| {
                format!("prior model repair raw attempt is unavailable or changed: {error}")
            })?;
        if configuration.get("request_sha256").and_then(Value::as_str)
            != Some(fan_out::sha256_digest(&request).as_str())
            || configuration
                .get("origin_invocation_id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || request_value.pointer("/run_id").and_then(Value::as_str) != Some(run_id)
            || request_value.pointer("/slot_id").and_then(Value::as_str) != Some(slot_id)
            || request_value
                .pointer("/state_visit")
                .and_then(Value::as_u64)
                != Some(state_visit)
            || request_value
                .pointer("/binding_sha256")
                .and_then(Value::as_str)
                != Some(binding_sha256)
            || request_value
                .pointer("/assignment_id")
                .and_then(Value::as_str)
                != Some(assignment_id)
            || request_value
                .pointer("/origin_invocation_id")
                .and_then(Value::as_str)
                != configuration
                    .get("origin_invocation_id")
                    .and_then(Value::as_str)
            || configuration
                .get("origin_invocation_id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || request_value
                .pointer("/raw_output_sha256")
                .and_then(Value::as_str)
                != Some(raw_digest)
            || request_value
                .pointer("/raw_attempt")
                .and_then(Value::as_u64)
                != Some(raw_attempt)
            || request_value
                .pointer("/raw_stdout_path")
                .and_then(Value::as_str)
                != Some(raw_path)
            || request_value
                .pointer("/model_adapter/model_id")
                .and_then(Value::as_str)
                != Some(model_id)
            || request_value
                .pointer("/model_adapter/command")
                .and_then(Value::as_str)
                != Some(adapter.command.as_str())
            || request_value.pointer("/model_adapter/args") != Some(&adapter_args)
            || request_value
                .pointer("/budget/max_calls")
                .and_then(Value::as_u64)
                != configuration.get("max_calls").and_then(Value::as_u64)
            || request_value
                .pointer("/budget/max_time_ms")
                .and_then(Value::as_u64)
                != configuration.get("max_time_ms").and_then(Value::as_u64)
            || request_value
                .pointer("/budget/max_cost_micros")
                .and_then(Value::as_u64)
                != configuration.get("max_cost_micros").and_then(Value::as_u64)
            || request_value
                .pointer("/budget/remaining_calls")
                .and_then(Value::as_u64)
                != configuration.get("remaining_calls").and_then(Value::as_u64)
            || request_value
                .pointer("/budget/remaining_time_ms")
                .and_then(Value::as_u64)
                != configuration
                    .get("remaining_time_ms")
                    .and_then(Value::as_u64)
            || request_value
                .pointer("/budget/remaining_cost_micros")
                .and_then(Value::as_u64)
                != configuration
                    .get("remaining_cost_micros")
                    .and_then(Value::as_u64)
        {
            return Err(
                "prior model repair request identity or raw origin is not verifiable".to_owned(),
            );
        }
        let stdout = read_bounded_file(&attempt_dir.join("stdout"), MAX_REPAIR_RAW_BYTES)?;
        let stderr = read_bounded_file(&attempt_dir.join("stderr"), MAX_REPAIR_RAW_BYTES)?;
        let usage = read_json_bounded(&attempt_dir.join("usage.json"), 64 * 1024)?;
        if usage.get("usage_accounted").and_then(Value::as_bool) != Some(true)
            || usage.get("model_id").and_then(Value::as_str) != Some(model_id)
            || usage.get("command").and_then(Value::as_str) != Some(adapter.command.as_str())
            || usage.get("args") != Some(&adapter_args)
            || usage.get("request_sha256").and_then(Value::as_str)
                != Some(fan_out::sha256_digest(&request).as_str())
            || usage.get("stdout_sha256").and_then(Value::as_str)
                != Some(fan_out::sha256_digest(&stdout).as_str())
            || usage.get("stderr_sha256").and_then(Value::as_str)
                != Some(fan_out::sha256_digest(&stderr).as_str())
        {
            return Err(
                "prior model repair attempt has missing or unverifiable usage accounting"
                    .to_owned(),
            );
        }
        let calls = usage
            .get("calls")
            .and_then(Value::as_u64)
            .filter(|calls| *calls > 0)
            .ok_or("prior model repair attempt omitted positive metered call usage")?;
        let elapsed_ms = usage
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .filter(|elapsed| *elapsed > 0)
            .ok_or("prior model repair attempt omitted positive elapsed usage")?;
        let metered_cost_micros = usage
            .get("metered_cost_micros")
            .and_then(Value::as_u64)
            .ok_or("prior model repair attempt omitted metered-cost usage")?;
        for (key, expected) in [
            ("max_calls", u64::from(adapter.max_calls)),
            ("max_time_ms", adapter.max_time_ms),
            ("max_cost_micros", adapter.max_cost_micros),
        ] {
            if usage.get(key).and_then(Value::as_u64) != Some(expected) {
                return Err(
                    "prior model repair usage recorded different assignment bounds".to_owned(),
                );
            }
        }
        total.calls = total
            .calls
            .checked_add(calls)
            .ok_or("model repair call accounting overflowed")?;
        total.elapsed_ms = total
            .elapsed_ms
            .checked_add(elapsed_ms)
            .ok_or("model repair time accounting overflowed")?;
        total.metered_cost_micros = total
            .metered_cost_micros
            .checked_add(metered_cost_micros)
            .ok_or("model repair cost accounting overflowed")?;
    }
    Ok(total)
}

fn read_bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "model repair capture `{}` is unavailable: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(format!(
            "model repair capture `{}` is not a bounded file",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| {
        format!(
            "could not read model repair capture `{}`: {error}",
            path.display()
        )
    })
}

fn read_json_bounded(path: &Path, limit: usize) -> Result<Value, String> {
    serde_json::from_slice(&read_bounded_file(path, limit)?).map_err(|error| {
        format!(
            "model repair capture `{}` is malformed: {error}",
            path.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn run_model_adapter(
    root: &Path,
    preview: &Value,
    origin: &Value,
    assignment_id: &str,
    raw_path: &str,
    raw_attempt: u64,
    raw_digest: &str,
    raw_bytes: &[u8],
    full_schema: Option<&Value>,
    legacy_schema: Option<&fan_out::OutputSchema>,
    adapter: &RepairAdapterConfig,
    model_id: &str,
) -> Result<(Vec<u8>, Value, Value), String> {
    let origin_id = origin
        .get("invocation_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or("model repair requires an origin invocation identity")?;
    let run_id = preview
        .pointer("/target/run_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model repair requires a run identity")?;
    let slot_id = preview
        .pointer("/target/slot_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model repair requires a slot identity")?;
    let state_visit = preview
        .pointer("/target/state_visit")
        .and_then(Value::as_u64)
        .ok_or("model repair requires a state-visit identity")?;
    let binding_sha256 = preview
        .pointer("/target/binding_sha256")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("model repair requires a frozen-binding identity")?;
    let budget_dir = model_budget_directory(
        root,
        run_id,
        slot_id,
        state_visit,
        binding_sha256,
        assignment_id,
    );
    let _budget_lock = acquire_model_budget_lock(&budget_dir)?;
    let used = load_model_budget_usage(
        root,
        &budget_dir,
        run_id,
        slot_id,
        state_visit,
        binding_sha256,
        assignment_id,
        adapter,
        model_id,
    )?;
    let remaining_calls = u64::from(adapter.max_calls)
        .checked_sub(used.calls)
        .filter(|remaining| *remaining > 0)
        .ok_or("model repair per-assignment call cap is exhausted")?;
    let remaining_time_ms = adapter
        .max_time_ms
        .checked_sub(used.elapsed_ms)
        .filter(|remaining| *remaining > 0)
        .ok_or("model repair per-assignment time cap is exhausted")?;
    let remaining_cost_micros = adapter
        .max_cost_micros
        .checked_sub(used.metered_cost_micros)
        .filter(|remaining| *remaining > 0)
        .ok_or("model repair per-assignment metered-cost cap is exhausted")?;
    let request_packet = json!({
        "version":1,
        "representation_only":true,
        "run_id":run_id,"slot_id":slot_id,"state_visit":state_visit,
        "binding_sha256":binding_sha256,"origin_invocation_id":origin_id,
        "assignment_id":assignment_id,
        "raw_attempt":raw_attempt,"raw_stdout_path":raw_path,
        "raw_output_hex":hex_encode(raw_bytes),
        "raw_output_sha256":raw_digest,
        "required_keys":legacy_schema.map(|schema| schema.required.clone()).or_else(|| {
            full_schema
                .and_then(|schema| schema.get("required"))
                .and_then(Value::as_array)
                .map(|required| required.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        }),
        "full_output_schema":full_schema,
        "model_adapter":{"model_id":model_id,"command":adapter.command,"args":adapter.args},
        "budget":{
            "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
            "max_cost_micros":adapter.max_cost_micros,
            "remaining_calls":remaining_calls,"remaining_time_ms":remaining_time_ms,
            "remaining_cost_micros":remaining_cost_micros,
        },
    });
    let packet_bytes = serde_json::to_vec(&request_packet).map_err(|error| error.to_string())?;
    if packet_bytes.len() > MAX_ADAPTER_PACKET_BYTES {
        return Err("model adapter packet exceeds its bound".to_owned());
    }
    let request_sha256 = fan_out::sha256_digest(&packet_bytes);
    let attempt_key = fan_out::sha256_digest(
        format!(
            "{}:{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            request_sha256
        )
        .as_bytes(),
    );
    let attempt_dir = budget_dir.join(&attempt_key[7..]);
    fs::create_dir(&attempt_dir)
        .map_err(|error| format!("could not create model adapter attempt capture: {error}"))?;
    write_immutable(&attempt_dir.join("request.json"), &packet_bytes)?;
    write_immutable(
        &attempt_dir.join("configuration.json"),
        &serde_json::to_vec(&json!({
            "kind":"model","model_id":model_id,"command":adapter.command,"args":adapter.args,
            "run_id":run_id,"slot_id":slot_id,"state_visit":state_visit,
            "binding_sha256":binding_sha256,"origin_invocation_id":origin_id,
            "origin_capture_dir":origin.get("capture_dir").and_then(Value::as_str),
            "assignment_id":assignment_id,"raw_attempt":raw_attempt,
            "raw_stdout_path":raw_path,"raw_stdout_sha256":raw_digest,
            "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
            "max_cost_micros":adapter.max_cost_micros,
            "remaining_calls":remaining_calls,"remaining_time_ms":remaining_time_ms,
            "remaining_cost_micros":remaining_cost_micros,"request_sha256":request_sha256,
        }))
        .map_err(|error| error.to_string())?,
    )?;
    let run = match run_repair_adapter(
        &adapter.command,
        &adapter.args,
        &packet_bytes,
        remaining_time_ms,
    ) {
        Ok(run) => run,
        Err(error) => {
            let stdout_sha256 = fan_out::sha256_digest(&[]);
            let stderr_sha256 = fan_out::sha256_digest(&[]);
            write_immutable(&attempt_dir.join("stdout"), &[])?;
            write_immutable(&attempt_dir.join("stderr"), &[])?;
            write_model_usage_capture(
                &attempt_dir,
                adapter,
                model_id,
                1,
                1,
                None,
                false,
                &request_sha256,
                &stdout_sha256,
                &stderr_sha256,
            )?;
            return Err(format!(
                "model adapter execution failed; retained capture {}: {error}",
                attempt_dir.display()
            ));
        }
    };
    write_immutable(&attempt_dir.join("stdout"), &run.stdout)?;
    write_immutable(&attempt_dir.join("stderr"), &run.stderr)?;
    let stdout_sha256 = fan_out::sha256_digest(&run.stdout);
    let stderr_sha256 = fan_out::sha256_digest(&run.stderr);
    let response = if run.timed_out || !run.status_success {
        None
    } else {
        serde_json::from_slice::<RepairAdapterResponse>(&run.stdout).ok()
    };
    let (calls, cost, accounted) = response
        .as_ref()
        .map(|response| {
            (
                response.usage.calls,
                Some(response.usage.metered_cost_micros),
                true,
            )
        })
        .unwrap_or((1, None, false));
    write_model_usage_capture(
        &attempt_dir,
        adapter,
        model_id,
        calls,
        run.elapsed_ms,
        cost,
        accounted,
        &request_sha256,
        &stdout_sha256,
        &stderr_sha256,
    )?;
    if run.timed_out || run.elapsed_ms > remaining_time_ms {
        return Err(format!(
            "model adapter exceeded its remaining per-assignment time cap; retained capture: {}",
            attempt_dir.display()
        ));
    }
    if !run.status_success {
        return Err(format!(
            "configured model adapter exited unsuccessfully; retained capture: {}",
            attempt_dir.display()
        ));
    }
    let response = response.ok_or_else(|| {
        format!(
            "model adapter usage/response is missing or malformed; retained capture: {}",
            attempt_dir.display()
        )
    })?;
    if response.usage.calls != 1 {
        return Err(format!(
            "model adapter must report exactly one bounded model call; retained capture: {}",
            attempt_dir.display()
        ));
    }
    if response.usage.metered_cost_micros > remaining_cost_micros {
        return Err(format!(
            "model adapter exceeded its remaining per-assignment metered-cost cap; retained capture: {}",
            attempt_dir.display()
        ));
    }
    if used.calls + u64::from(response.usage.calls) > u64::from(adapter.max_calls)
        || used.elapsed_ms + run.elapsed_ms > adapter.max_time_ms
        || used.metered_cost_micros + response.usage.metered_cost_micros > adapter.max_cost_micros
    {
        return Err(format!(
            "model adapter exceeded its positive per-assignment call/time/metered-cost bounds; retained capture: {}",
            attempt_dir.display()
        ));
    }
    let output = response.output.into_bytes();
    let capture = json!({
        "directory":attempt_dir,
        "request_sha256":request_sha256,
        "stdout_sha256":stdout_sha256,
        "stderr_sha256":stderr_sha256,
    });
    let usage = json!({
        "model_id":model_id,"command":adapter.command,"args":adapter.args,
        "usage_accounted":true,"calls":response.usage.calls,"elapsed_ms":run.elapsed_ms,
        "metered_cost_micros":response.usage.metered_cost_micros,
        "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
        "max_cost_micros":adapter.max_cost_micros,"capture":capture,
    });
    Ok((output, usage, capture))
}

fn write_model_usage_capture(
    attempt_dir: &Path,
    adapter: &RepairAdapterConfig,
    model_id: &str,
    calls: u32,
    elapsed_ms: u64,
    metered_cost_micros: Option<u64>,
    usage_accounted: bool,
    request_sha256: &str,
    stdout_sha256: &str,
    stderr_sha256: &str,
) -> Result<(), String> {
    write_immutable(
        &attempt_dir.join("usage.json"),
        &serde_json::to_vec(&json!({
            "kind":"model","model_id":model_id,"command":adapter.command,"args":adapter.args,
            "usage_accounted":usage_accounted,"calls":calls,"elapsed_ms":elapsed_ms,
            "metered_cost_micros":metered_cost_micros,
            "max_calls":adapter.max_calls,"max_time_ms":adapter.max_time_ms,
            "max_cost_micros":adapter.max_cost_micros,
            "request_sha256":request_sha256,"stdout_sha256":stdout_sha256,
            "stderr_sha256":stderr_sha256,
        }))
        .map_err(|error| error.to_string())?,
    )
}

fn verify_repair_first_attempt_manifest(
    capture: &Path,
    worker_index: usize,
    raw_attempt: u64,
    raw_digest: &str,
    schema: &Value,
    legacy_schema: Option<&fan_out::OutputSchema>,
) -> Result<(), String> {
    if raw_attempt != 1 {
        return Err("repair-first output must reference the original first raw attempt".to_owned());
    }
    let manifest_path = capture.join(worker_index.to_string()).join("attempts.json");
    let metadata = fs::metadata(&manifest_path)
        .map_err(|error| format!("repair-first attempts manifest is unavailable: {error}"))?;
    if metadata.len() > 1024 * 1024 {
        return Err("repair-first attempts manifest exceeds its bound".to_owned());
    }
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|error| format!("could not read repair-first attempts manifest: {error}"))?,
    )
    .map_err(|error| format!("repair-first attempts manifest is malformed: {error}"))?;
    let attempts = manifest
        .get("attempts")
        .and_then(Value::as_array)
        .ok_or("repair-first attempts manifest omitted attempts")?;
    if manifest.get("schema_version").and_then(Value::as_str) != Some("2")
        || manifest.get("recovery_state").and_then(Value::as_str) != Some("awaiting-output-repair")
        || manifest
            .get("selected_attempt")
            .is_none_or(|value| !value.is_null())
        || manifest.get("exhausted").and_then(Value::as_bool) != Some(false)
        || attempts.len() != 1
        || attempts[0].get("number").and_then(Value::as_u64) != Some(1)
    {
        return Err("origin is not a truthful one-attempt repair-first capture".to_owned());
    }
    let raw_path = format!("{worker_index}/attempts/1/stdout");
    let raw =
        read_capture_stream(capture, &raw_path, raw_digest).map_err(|error| error.to_string())?;
    let attempt = &attempts[0];
    let errors = fan_out::complete_schema_validation_errors(&raw, schema, legacy_schema);
    if errors.is_empty()
        || attempt.get("stdout_sha256").and_then(Value::as_str) != Some(raw_digest)
        || attempt.get("validation_errors") != Some(&json!(errors))
    {
        return Err("first raw output does not match its recorded schema failure".to_owned());
    }
    let stderr_path = format!("{worker_index}/attempts/1/stderr");
    let stderr_digest = attempt
        .get("stderr_sha256")
        .and_then(Value::as_str)
        .ok_or("first attempt omitted stderr digest")?;
    read_capture_stream(capture, &stderr_path, stderr_digest).map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_recovered_contract(
    bytes: &[u8],
    full_schema: Option<&Value>,
    legacy_schema: Option<&fan_out::OutputSchema>,
) -> Result<(), String> {
    if let Some(schema) = full_schema {
        let errors = fan_out::complete_schema_validation_errors(bytes, schema, legacy_schema);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    } else if let Some(schema) = legacy_schema {
        fan_out::evaluate_output_conformance_bytes(bytes, schema)
    } else {
        Err("frozen output contract is unavailable".to_owned())
    }
}

fn declared_contract_from_spec(worker: &fan_out::FanOutSpecWorker) -> Value {
    worker
        .full_output_schema
        .clone()
        .or_else(|| {
            worker
                .output_schema
                .as_ref()
                .map(|schema| serde_json::to_value(schema).unwrap_or(Value::Null))
        })
        .unwrap_or(Value::Null)
}

fn mechanical_normalize_full_schema(
    mut value: Value,
    schema: &Value,
    legacy_schema: Option<&fan_out::OutputSchema>,
) -> Result<(Vec<u8>, Value), String> {
    let mut changes = Vec::new();
    restore_schema_constants(&mut value, schema, "$", &mut changes)?;
    if changes.is_empty() {
        return Err("no unambiguous frozen structure or constant repair applies".to_owned());
    }
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    validate_recovered_contract(&bytes, Some(schema), legacy_schema).map_err(|error| {
        format!("mechanical constant restoration remains nonconforming: {error}")
    })?;
    Ok((
        bytes,
        json!({"kind":"restore-frozen-constants","changes":changes}),
    ))
}

fn restore_schema_constants(
    value: &mut Value,
    schema: &Value,
    path: &str,
    changes: &mut Vec<Value>,
) -> Result<(), String> {
    if let Some(constant) = schema.get("const") {
        if value != constant {
            let (Some(actual), Some(expected)) = (value.as_object_mut(), constant.as_object())
            else {
                return Err(format!(
                    "{path} conflicts with a frozen commission constant"
                ));
            };
            if actual.keys().any(|key| !expected.contains_key(key)) {
                return Err(format!(
                    "{path} contains data beyond its frozen commission constant"
                ));
            }
            for (key, expected_value) in expected {
                match actual.get(key) {
                    Some(actual_value) if actual_value != expected_value => {
                        return Err(format!(
                            "{path}/{key} conflicts with a frozen commission constant"
                        ));
                    }
                    Some(_) => {}
                    None => {
                        actual.insert(key.clone(), expected_value.clone());
                        changes.push(json!({
                            "kind":"restore-frozen-constant",
                            "path":format!("{path}/{key}"),
                            "value":expected_value,
                        }));
                    }
                }
            }
        }
    }

    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let matching = one_of
            .iter()
            .filter(|branch| schema_compatible(value, branch))
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(format!(
                "{path} does not identify exactly one frozen output structure"
            ));
        }
        return restore_schema_constants(value, matching[0], path, changes);
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let object = value
                .as_object_mut()
                .ok_or_else(|| format!("{path} is not an explicit JSON object"))?;
            let properties = schema.get("properties").and_then(Value::as_object);
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if object.contains_key(key) {
                        continue;
                    }
                    let property = properties
                        .and_then(|properties| properties.get(key))
                        .ok_or_else(|| format!("{path}/{key} has no frozen property contract"))?;
                    let repair = property.get("const").cloned().or_else(|| {
                        property
                            .get("enum")
                            .and_then(Value::as_array)
                            .filter(|values| values.len() == 1)
                            .and_then(|values| values.first().cloned())
                    });
                    let Some(repair) = repair else {
                        return Err(format!(
                            "{path}/{key} is missing substantive content, not a frozen constant"
                        ));
                    };
                    object.insert(key.to_owned(), repair.clone());
                    changes.push(json!({
                        "kind":"restore-frozen-constant",
                        "path":format!("{path}/{key}"),
                        "value":repair,
                    }));
                }
            }
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if let Some(property_schema) =
                    properties.and_then(|properties| properties.get(&key))
                {
                    let property = object.get_mut(&key).expect("key collected from object");
                    restore_schema_constants(
                        property,
                        property_schema,
                        &format!("{path}/{key}"),
                        changes,
                    )?;
                }
            }
        }
        Some("array") => {
            let values = value
                .as_array_mut()
                .ok_or_else(|| format!("{path} is not an explicit JSON array"))?;
            if let Some(items) = schema.get("items") {
                for (index, item) in values.iter_mut().enumerate() {
                    restore_schema_constants(item, items, &format!("{path}/{index}"), changes)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn schema_compatible(value: &Value, schema: &Value) -> bool {
    if schema
        .get("const")
        .is_some_and(|constant| constant != value)
        || schema
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.contains(value))
    {
        return false;
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        if one_of
            .iter()
            .filter(|branch| schema_compatible(value, branch))
            .count()
            != 1
        {
            return false;
        }
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") if !value.is_object() => return false,
        Some("array") if !value.is_array() => return false,
        Some("string") if !value.is_string() => return false,
        Some("integer") if value.as_i64().is_none() && value.as_u64().is_none() => return false,
        Some("number") if !value.is_number() => return false,
        Some("boolean") if !value.is_boolean() => return false,
        Some("null") if !value.is_null() => return false,
        _ => {}
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if schema.get("additionalProperties") == Some(&Value::Bool(false))
            && object
                .keys()
                .any(|key| !properties.is_some_and(|properties| properties.contains_key(key)))
        {
            return false;
        }
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required.iter().filter_map(Value::as_str).any(|key| {
                    !object.contains_key(key)
                        && !properties
                            .and_then(|properties| properties.get(key))
                            .is_some_and(has_frozen_constant)
                })
            })
        {
            return false;
        }
        if !object.iter().all(|(key, value)| {
            properties
                .and_then(|properties| properties.get(key))
                .is_none_or(|property| schema_compatible(value, property))
        }) {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        if schema.get("items").is_some_and(|item_schema| {
            !items
                .iter()
                .all(|item| schema_compatible(item, item_schema))
        }) {
            return false;
        }
    }
    true
}

fn has_frozen_constant(schema: &Value) -> bool {
    schema.get("const").is_some()
        || schema
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| values.len() == 1)
}

fn mechanical_normalize(
    mut value: Value,
    schema: &fan_out::OutputSchema,
) -> Result<(Vec<u8>, Value), String> {
    let object = value
        .as_object_mut()
        .ok_or("mechanical normalizer requires a JSON object")?;
    if !schema.required.iter().any(|key| key == "repository_effect") {
        return Err("frozen contract does not require repository_effect".to_owned());
    }
    if object.contains_key("repository_effect") {
        return Err("destination key already exists; key rename is ambiguous".to_owned());
    }
    let effect = object
        .remove("effect")
        .ok_or("raw output has no explicit effect key to rename")?;
    object.insert("repository_effect".to_owned(), effect);
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    fan_out::evaluate_output_conformance_bytes(&bytes, schema)
        .map_err(|error| format!("mechanical rename remains nonconforming: {error}"))?;
    Ok((
        bytes,
        json!({"kind":"rename-key","from":"effect","to":"repository_effect"}),
    ))
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file
            .write_all(bytes)
            .map_err(|error| format!("could not write `{}`: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|read_error| {
                format!("could not read existing `{}`: {read_error}", path.display())
            })?;
            if existing == bytes {
                Ok(())
            } else {
                Err(format!(
                    "existing derived capture `{}` has different bytes",
                    path.display()
                ))
            }
        }
        Err(error) => Err(format!("could not create `{}`: {error}", path.display())),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}

fn run_repair_adapter(
    adapter_command: &str,
    adapter_args: &[String],
    packet: &[u8],
    max_time_ms: u64,
) -> Result<RepairAdapterRun, String> {
    const CLEANUP_GRACE: Duration = Duration::from_secs(2);

    let mut command = Command::new(adapter_command);
    command
        .args(adapter_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // The adapter owns a fresh group so ordinary backend subprocesses
        // inheriting its pipes are included in timeout cleanup.
        command.process_group(0);
    }
    #[cfg(not(unix))]
    return Err("bounded adapter process-group cleanup is unsupported on this platform".to_owned());

    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not launch configured output-only adapter: {error}"))?;
    let process_group = child.id() as i32;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("output-only adapter stdin was not available")?;
    let writer_packet = packet.to_vec();
    let writer = thread::spawn(move || stdin.write_all(&writer_packet));
    let mut stdout = child
        .stdout
        .take()
        .ok_or("output-only adapter stdout was not available")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("output-only adapter stderr was not available")?;
    let out_reader = thread::spawn(move || read_capped(&mut stdout));
    let err_reader = thread::spawn(move || read_capped(&mut stderr));

    let mut status = None;
    let mut wait_error = None;
    let mut timed_out = false;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(next) => status = next,
                Err(error) => {
                    wait_error = Some(error);
                    break;
                }
            }
        }
        if started.elapsed() >= Duration::from_millis(max_time_ms) {
            timed_out = true;
            break;
        }
        if status.is_some()
            && writer.is_finished()
            && out_reader.is_finished()
            && err_reader.is_finished()
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }

    if timed_out || wait_error.is_some() {
        let kill_error = kill_repair_process_group(process_group).err();
        if kill_error.is_some() {
            // Retain a safe direct-child fallback while the Child handle owns
            // that identity; descendants remain protected by the group kill.
            let _ = child.kill();
        }
        let cleanup_deadline = Instant::now() + CLEANUP_GRACE;
        loop {
            if status.is_none() {
                match child.try_wait() {
                    Ok(next) => status = next,
                    Err(error) => {
                        wait_error = Some(error);
                        break;
                    }
                }
            }
            if status.is_some()
                && writer.is_finished()
                && out_reader.is_finished()
                && err_reader.is_finished()
            {
                break;
            }
            if Instant::now() >= cleanup_deadline {
                return Err(format!(
                    "timed-out output-only adapter process group or pipes did not clean up within {} ms{}{}",
                    CLEANUP_GRACE.as_millis(),
                    wait_error
                        .as_ref()
                        .map(|error| format!("; wait error: {error}"))
                        .unwrap_or_default(),
                    kill_error
                        .as_ref()
                        .map(|error| format!("; process-group signal error: {error}"))
                        .unwrap_or_default(),
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        if let Some(error) = wait_error {
            return Err(format!("could not wait for output-only adapter: {error}"));
        }
        if let Some(error) = kill_error {
            return Err(format!(
                "could not terminate output-only adapter process group: {error}"
            ));
        }
    }

    let status = status.ok_or("output-only adapter ended without an exit status")?;
    let writer_result = writer
        .join()
        .map_err(|_| "output-only adapter input writer panicked")?;
    if !timed_out {
        writer_result
            .map_err(|error| format!("could not send output-only adapter packet: {error}"))?;
    }
    let stdout = out_reader
        .join()
        .map_err(|_| "output-only adapter stdout reader panicked")?
        .map_err(|error| format!("could not retain output-only adapter stdout: {error}"))?;
    let stderr = err_reader
        .join()
        .map_err(|_| "output-only adapter stderr reader panicked")?
        .map_err(|error| format!("could not retain output-only adapter stderr: {error}"))?;
    timed_out |= started.elapsed() >= Duration::from_millis(max_time_ms);
    let elapsed_ms = started
        .elapsed()
        .as_micros()
        .saturating_add(999)
        .checked_div(1000)
        .unwrap_or(1)
        .max(1)
        .min(u128::from(u64::MAX)) as u64;
    Ok(RepairAdapterRun {
        stdout,
        stderr,
        elapsed_ms,
        status_success: status.success(),
        timed_out,
    })
}

#[cfg(unix)]
fn kill_repair_process_group(process_group: i32) -> std::io::Result<()> {
    if process_group <= 1 || process_group == std::process::id() as i32 {
        return Err(std::io::Error::other(
            "invalid output-only adapter process group",
        ));
    }
    // SAFETY: run_repair_adapter created this group for the live Child and
    // retained its Child handle; the negative id targets only that group.
    if unsafe { libc::kill(-process_group, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(not(unix))]
fn kill_repair_process_group(_process_group: i32) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "bounded adapter process-group cleanup is unsupported on this platform",
    ))
}

fn read_capped(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_REPAIR_RAW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_REPAIR_RAW_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "captured stream exceeded repair bound",
        ));
    }
    Ok(bytes)
}
