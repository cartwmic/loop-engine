//! `invoke` work-slot delegation.

use super::{persistence_error, require_current_observation, show};
use crate::{
    instruction_digest, work_slot_binding_digest, AssignmentLabel, ContextRecord,
    CreateWorkSlotInvocationRequest, FanOutRecoveryInput, FanOutRecoverySource, HistoryAction,
    HistoryEntry, InvocationId, OperationOutcome, Persistence, ProcessError, RunId, Timestamp,
    WaiterSpawnArgs, WaiterWrittenStatus, WorkSlotBinding, WorkSlotId, WorkSlotInvocation,
    WorkSlotProcess,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Caller-supplied values needed to invoke a bound work slot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub run_id: RunId,
    pub slot_id: WorkSlotId,
    pub invocation_id: InvocationId,
    pub database: PathBuf,
    /// Optional assignment identities to run. `None` means all enumerable
    /// assignments (and preserves the historical full-execution path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_selection: Option<Vec<String>>,
    /// Optional opaque JSON for the bound worker. Core only frames, stores,
    /// and transports this value; the provider owns its meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_input: Option<Value>,
    #[serde(default)]
    pub controls: crate::InvocationControls,
    #[serde(default)]
    pub preview: bool,
}

impl Request {
    pub fn new(
        run_id: impl Into<RunId>,
        slot_id: impl Into<WorkSlotId>,
        invocation_id: impl Into<InvocationId>,
        database: impl Into<PathBuf>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            slot_id: slot_id.into(),
            invocation_id: invocation_id.into(),
            database: database.into(),
            assignment_selection: None,
            invocation_input: None,
            controls: crate::InvocationControls::default(),
            preview: false,
        }
    }

    pub fn with_assignment_selection(mut self, selection: Option<Vec<String>>) -> Self {
        self.assignment_selection = selection;
        self
    }

    pub fn with_invocation_input(mut self, input: Option<Value>) -> Self {
        self.invocation_input = input;
        self
    }
}

/// Successful `invoke` data returned to the composition root.
///
/// `waiter_pid` is internal and is not part of this caller-facing result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Result {
    pub invocation_id: InvocationId,
    pub slot_id: WorkSlotId,
    pub started_at: Timestamp,
    pub allowed_time_ms: u64,
    pub capture_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Value>,
}

#[derive(Serialize)]
struct WorkerPacket {
    run_id: String,
    slot_id: String,
    state_visit: u64,
    binding_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_origin: Option<Value>,
    artifact_root: String,
    instruction_body: String,
    capture_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Vec<ContextRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignment_selection: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invocation_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    standing_assignment_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    assignment_labels: Vec<AssignmentLabel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition_history: Option<Vec<HistoryEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controls: Option<crate::InvocationControls>,
}

#[derive(Serialize)]
struct WaiterEnvelope {
    command: String,
    args: Vec<String>,
    worker_packet: WorkerPacket,
}

/// Execute `invoke` through persistence and the work-slot process port.
///
/// Rejection checks run before spawn. On accept, create `capture_dir` after
/// admission and before waiter spawn. Launch order is then spawn waiter
/// (no waitpid, no stdin yet), read pid, create the running invocation, then
/// write the waiter envelope and detach. Invoke does not spawn the bound worker.
pub fn execute<P, Proc>(
    request: Request,
    persistence: &P,
    process: &Proc,
    now: Timestamp,
    allowed_time_ms: u64,
) -> OperationOutcome<Result>
where
    P: Persistence + ?Sized,
    Proc: WorkSlotProcess + ?Sized,
{
    let run = match persistence.load_authoritative_run(&request.run_id) {
        Ok(run) => run,
        Err(error) => return persistence_error(error),
    };

    // Preserve the operation's existing lifecycle refusal before any invoke
    // preflight (including assignment enumeration) or observation check.
    if !run.lifecycle.is_active() {
        return OperationOutcome::rejected(
            "run-not-active",
            format!("run `{}` is not active ({:?})", run.id, run.lifecycle),
        );
    }

    let Some(slot) = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.id == request.slot_id)
    else {
        return OperationOutcome::rejected(
            "unknown-work-slot",
            format!(
                "work slot `{}` is not in the workflow catalog for run `{}`",
                request.slot_id, request.run_id
            ),
        );
    };

    let Some(binding) = crate::effective_binding(&run, &request.slot_id) else {
        return OperationOutcome::rejected(
            "unbound-work-slot",
            format!(
                "work slot `{}` has no frozen work_slot_bindings entry",
                request.slot_id
            ),
        );
    };
    let binding = match binding {
        Ok(binding) => binding,
        Err(message) => {
            return OperationOutcome::rejected("invalid-work-slot-binding", message);
        }
    };

    if request.assignment_selection.is_some() && request.invocation_input.is_some() {
        return OperationOutcome::rejected(
            "assignment-input-conflict",
            "`assignment_selection` may not be combined with `invocation_input`",
        );
    }
    let recovery_input = match parse_fan_out_recovery_input(request.invocation_input.as_ref()) {
        Ok(input) => input,
        Err(message) => return OperationOutcome::rejected("invalid-fan-out-recovery", message),
    };

    let controls =
        match process.prepare_controls(&binding, &run.provider_association, &request.controls) {
            Ok(controls) => controls,
            Err(error) => return process_error(error),
        };

    let assignment_selection = match validate_assignment_selection(
        process,
        &binding,
        request.assignment_selection.as_deref(),
    ) {
        Ok(selection) => selection,
        Err(outcome) => return outcome,
    };

    let invocations = match persistence.load_work_slot_invocations(&request.run_id) {
        Ok(invocations) => invocations,
        Err(error) => return persistence_error(error),
    };
    for record in invocations
        .iter()
        .filter(|record| record.slot_id == request.slot_id)
    {
        let waiter_alive = if record.ownership.is_some() && record.waiter_identity.is_none() {
            false
        } else {
            process.waiter_alive_for_identity(record.waiter_pid, record.waiter_identity.as_ref())
        };
        if crate::invocation_owns_work(record, waiter_alive) {
            return OperationOutcome::rejected(
                "work-slot-already-running",
                format!(
                    "work slot `{}` has live owned work or pending cleanup in invocation `{}` (including overrun); wait or cancel-invocation {} {} before retry",
                    request.slot_id, record.invocation_id, request.run_id, record.invocation_id
                ),
            );
        }
    }

    let Some(state) = run
        .workflow
        .states
        .iter()
        .find(|state| state.id == slot.state)
    else {
        return OperationOutcome::error(
            "invalid-run",
            format!(
                "work slot `{}` names state `{}` which is not in the workflow",
                request.slot_id, slot.state
            ),
        );
    };
    let instruction_body = state.instructions.clone();
    let digest = instruction_digest(&instruction_body);
    let binding_digest = work_slot_binding_digest(&binding);

    let subject = match persistence.get_current_slot_subject(&request.run_id, &request.slot_id) {
        Ok(Some(subject)) => subject,
        Ok(None) => {
            return OperationOutcome::rejected(
                "no-current-visit-subject",
                format!(
                    "work slot `{}` has no current visit subject",
                    request.slot_id
                ),
            );
        }
        Err(error) => return persistence_error(error),
    };

    let recovery_origin = if let Some(recovery) = recovery_input.as_ref() {
        match validate_fan_out_recovery(
            recovery,
            &run,
            RecoveryTarget {
                slot_id: &request.slot_id,
                binding: &binding,
                binding_digest: &binding_digest,
                instruction_digest: &digest,
                current_subject: &subject,
            },
            &invocations,
            process,
        ) {
            Ok(origin) => Some(origin),
            Err(message) => return OperationOutcome::rejected("invalid-fan-out-recovery", message),
        }
    } else {
        None
    };

    let forwarded_context = if slot.stdin_context_kinds.is_empty() {
        None
    } else {
        match persistence.load_context_records(&request.run_id) {
            Ok(records) => Some(
                records
                    .into_iter()
                    .filter(|record| {
                        slot.stdin_context_kinds
                            .iter()
                            .any(|kind| kind == &record.kind)
                    })
                    .collect(),
            ),
            Err(error) => return persistence_error(error),
        }
    };

    let transition_history = match persistence.load_transition_history(&run.id) {
        Ok(Some(history)) => {
            let transitions = history
                .into_iter()
                .filter(|entry| matches!(entry.action, HistoryAction::Transition { .. }))
                .collect::<Vec<_>>();
            (transitions.len() <= 4096).then_some(transitions)
        }
        Ok(None) => None,
        Err(error) => return persistence_error(error),
    };

    let artifact_root = artifact_root_from_input(&run.initial_input);
    if artifact_root.is_empty() {
        return OperationOutcome::error(
            "capture-directory-failed",
            "cannot allocate capture_dir because artifact_root is empty",
        );
    }

    if let Err(outcome) =
        require_current_observation::<P, Result>(persistence, &run.id, run.control_revision)
    {
        return outcome;
    }

    let forwarded_context = match prepare_context(
        process,
        &binding,
        &run,
        &request.slot_id,
        &artifact_root,
        &json!({"max_active": controls.max_active, "force_fresh": controls.force_fresh, "timeout_ms": allowed_time_ms}),
        forwarded_context,
    ) {
        Ok(context) => context,
        Err(error) => return process_error(error),
    };

    // Freeze standing before admission, using the same projection as show.
    let standing_assignment_ids =
        if controls.force_fresh || (forwarded_context.is_some() && invocations.is_empty()) {
            Some(Vec::new())
        } else if forwarded_context.is_some() {
            let context = match persistence.load_context_records(&request.run_id) {
                Ok(context) => context,
                Err(error) => return persistence_error(error),
            };
            let data = crate::ShowData {
                run: run.clone(),
                context,
                checked_evaluations: Vec::new(),
            };
            let mut subjects = std::collections::BTreeMap::new();
            for id in invocations
                .iter()
                .map(|item| item.slot_id.clone())
                .collect::<BTreeSet<_>>()
            {
                match persistence.get_current_slot_subject(&request.run_id, &id) {
                    Ok(Some(subject)) => {
                        subjects.insert(id, subject);
                    }
                    Ok(None) => {}
                    Err(error) => return persistence_error(error),
                }
            }
            match show::project_with_invocations_and_subjects(
                data,
                &invocations,
                now,
                |_| false,
                &subjects,
            ) {
                Ok(projection) => Some(show::standing_assignment_ids(&projection)),
                Err(error) => return OperationOutcome::error(error.code(), error.to_string()),
            }
        } else {
            None
        };
    let mut preparation_packet = json!({
        "run_id": request.run_id, "slot_id": request.slot_id, "artifact_root": artifact_root,
        "instruction_body": instruction_body, "capture_dir": artifact_root,
        "context": forwarded_context, "invocation_input": request.invocation_input,
        "standing_assignment_ids": standing_assignment_ids, "transition_history": transition_history,
        "controls": controls, "preview": true
    });
    preparation_packet
        .as_object_mut()
        .expect("packet object")
        .retain(|_, value| !value.is_null());
    let facade_preparation =
        match process.prepare_facade(&binding, &run.provider_association, &preparation_packet) {
            Ok(prepared) => prepared,
            Err(error) => return process_error(error),
        };
    let assignment_labels = match process.assignment_labels(&binding, &run.provider_association) {
        Ok(Some(labels)) => labels,
        Ok(None) => match labels_from_facade_preparation(facade_preparation.as_ref()) {
            Ok(labels) => labels,
            Err(message) => return OperationOutcome::error("invalid-assignment-labels", message),
        },
        Err(error) => return process_error(error),
    };
    let assignment_labels = match validate_assignment_labels(assignment_labels) {
        Ok(labels) => labels,
        Err(message) => return OperationOutcome::error("invalid-assignment-labels", message),
    };
    if request.preview {
        return OperationOutcome::completed(Result {
            invocation_id: request.invocation_id,
            slot_id: request.slot_id.clone(),
            started_at: now,
            allowed_time_ms,
            capture_dir: String::new(),
            preview: Some(json!({
                "slot_id": request.slot_id,
                "binding": binding, "controls": controls, "allowed_time_ms": allowed_time_ms,
                "context": forwarded_context, "assignment_selection": assignment_selection,
                "invocation_input": request.invocation_input, "instruction_body": instruction_body,
                "artifact_root": artifact_root, "standing_assignment_ids": standing_assignment_ids,
                "transition_history": transition_history,
                "facade_preparation": facade_preparation,
                "assignment_labels": assignment_labels
            })),
        });
    }

    let capture_dir_path =
        capture_dir_path(&artifact_root, &request.slot_id, &request.invocation_id);
    let capture_dir = capture_dir_path.to_string_lossy().into_owned();
    if let Err(error) = std::fs::create_dir_all(&capture_dir_path) {
        return OperationOutcome::error(
            "capture-directory-failed",
            format!("could not create capture directory `{capture_dir}`: {error}"),
        );
    }

    let waiter = match process.spawn_wait_invocation(WaiterSpawnArgs::new(
        request.database.clone(),
        request.run_id.clone(),
        request.invocation_id.clone(),
    )) {
        Ok(waiter) => waiter,
        Err(error) => return process_error(error),
    };

    let waiter_identity = waiter.identity.clone();
    let create = CreateWorkSlotInvocationRequest::new(
        request.run_id.clone(),
        request.invocation_id.clone(),
        request.slot_id.clone(),
        binding.clone(),
        digest,
        subject,
        waiter.pid,
        now,
        allowed_time_ms,
        capture_dir.clone(),
    )
    .with_waiter_identity_opt(waiter_identity)
    .with_controls(controls.clone())
    .with_state_visit(run.control_revision.as_u64())
    .with_routed_inputs(forwarded_context.clone().unwrap_or_default())
    .with_frozen_run_identity(json!({
        "provider": run.provider_association.as_json(),
        "input": run.initial_input,
    }))
    .with_assignment_selection(assignment_selection.clone())
    .with_assignment_labels(assignment_labels.clone())
    .with_invocation_input(request.invocation_input.clone());
    if let Err(error) = persistence.create_work_slot_invocation(create) {
        return persistence_error(error);
    }

    let envelope = WaiterEnvelope {
        command: binding.command,
        args: binding.args,
        worker_packet: WorkerPacket {
            run_id: request.run_id.as_str().to_owned(),
            slot_id: request.slot_id.as_str().to_owned(),
            state_visit: run.control_revision.as_u64(),
            binding_sha256: binding_digest,
            recovery_origin,
            artifact_root,
            instruction_body,
            capture_dir: capture_dir.clone(),
            context: forwarded_context,
            assignment_selection,
            invocation_input: request.invocation_input,
            standing_assignment_ids,
            assignment_labels,
            transition_history,
            controls: (controls != crate::InvocationControls::default()).then_some(controls),
        },
    };
    let envelope_json = match serde_json::to_vec(&envelope) {
        Ok(bytes) => bytes,
        Err(error) => {
            return OperationOutcome::error(
                "waiter-envelope-serialization-failed",
                format!("could not serialize waiter envelope: {error}"),
            );
        }
    };
    if let Err(error) = process.send_envelope_and_detach(waiter, &envelope_json) {
        return process_error(error);
    }

    OperationOutcome::completed(Result {
        invocation_id: request.invocation_id,
        slot_id: request.slot_id,
        started_at: now,
        allowed_time_ms,
        capture_dir,
        preview: None,
    })
}

/// Shared launch/preview seam. Selection is bounded by the slot's eligible
/// kinds; the callback cannot replace records or protected execution values.
pub fn prepare_context<P: WorkSlotProcess + ?Sized>(
    process: &P,
    binding: &WorkSlotBinding,
    run: &crate::Run,
    slot_id: &WorkSlotId,
    artifact_root: &str,
    controls: &Value,
    eligible: Option<Vec<ContextRecord>>,
) -> std::result::Result<Option<Vec<ContextRecord>>, ProcessError> {
    let Some(filter) = &binding.context_filter else {
        return Ok(eligible);
    };
    let records = eligible.unwrap_or_default();
    let packet = json!({
        "run_id": run.id, "slot_id": slot_id,
        "work_slots": run.workflow.work_slots,
        "artifact_root": artifact_root, "controls": controls,
        "context": records,
    });
    let selection = process.filter_context(filter, &packet)?;
    crate::resolve_context_filter(&records, &selection)
        .map(Some)
        .map_err(|message| ProcessError::new("invalid-context-filter-selection", message))
}

const FAN_OUT_RECOVERY_PROTOCOL: &str = "fan-out-recovery-v1";

fn parse_fan_out_recovery_input(
    input: Option<&Value>,
) -> std::result::Result<Option<FanOutRecoveryInput>, String> {
    let Some(input) = input else {
        return Ok(None);
    };
    let Some(object) = input.as_object() else {
        return Ok(None);
    };
    let Some(protocol) = object.get("protocol").and_then(Value::as_str) else {
        if ["origin_invocation_id", "pending_assignment_ids", "sources"]
            .iter()
            .any(|key| object.contains_key(*key))
        {
            return Err("fan-out recovery input requires its versioned protocol field".to_owned());
        }
        return Ok(None);
    };
    if protocol != FAN_OUT_RECOVERY_PROTOCOL {
        if protocol.starts_with("fan-out-recovery-") {
            return Err(format!(
                "unsupported fan-out recovery protocol `{protocol}`"
            ));
        }
        return Ok(None);
    }
    serde_json::from_value(input.clone())
        .map(Some)
        .map_err(|error| format!("fan-out recovery input is malformed: {error}"))
}

struct RecoveryTarget<'a> {
    slot_id: &'a WorkSlotId,
    binding: &'a WorkSlotBinding,
    binding_digest: &'a str,
    instruction_digest: &'a str,
    current_subject: &'a str,
}

fn validate_fan_out_recovery<P: WorkSlotProcess + ?Sized>(
    recovery: &FanOutRecoveryInput,
    run: &crate::Run,
    target: RecoveryTarget<'_>,
    invocations: &[WorkSlotInvocation],
    process: &P,
) -> std::result::Result<Value, String> {
    let RecoveryTarget {
        slot_id,
        binding,
        binding_digest,
        instruction_digest,
        current_subject,
    } = target;
    if recovery.protocol != FAN_OUT_RECOVERY_PROTOCOL
        || recovery.run_id != run.id.as_str()
        || recovery.slot_id != slot_id.as_str()
        || recovery.state_visit != run.control_revision.as_u64()
        || recovery.subject != current_subject
        || recovery.binding_sha256 != binding_digest
    {
        return Err(
            "recovery target, state visit, subject, or frozen binding does not match this invoke"
                .to_owned(),
        );
    }
    if recovery.origin_invocation_id.trim().is_empty() {
        return Err("recovery origin invocation ID must be non-empty".to_owned());
    }
    let origins = invocations
        .iter()
        .filter(|record| record.invocation_id.as_str() == recovery.origin_invocation_id)
        .collect::<Vec<_>>();
    let Some(origin) = origins.first().copied() else {
        return Err(format!(
            "recovery origin invocation `{}` is not in this run",
            recovery.origin_invocation_id
        ));
    };
    if origins.len() != 1
        || origin.slot_id != *slot_id
        || origin.state_visit != recovery.state_visit
        || origin.subject != current_subject
        || origin.instruction_digest != instruction_digest
        || origin.binding != *binding
        || origin.status != Some(WaiterWrittenStatus::Failed)
    {
        return Err(
            "recovery origin is not a failed invocation from this exact slot visit and binding"
                .to_owned(),
        );
    }
    let ownership = origin
        .ownership
        .as_ref()
        .ok_or_else(|| "recovery origin has no verifiable owned-work cleanup record".to_owned())?;
    if ownership.live_owned_work || ownership.cleanup_pending {
        return Err("recovery origin still has live owned work or cleanup pending".to_owned());
    }
    let artifact_root = artifact_root_from_input(&run.initial_input);
    let expected_capture = capture_dir_path(&artifact_root, slot_id, &origin.invocation_id);
    if Path::new(&origin.capture_dir) != expected_capture {
        return Err(
            "recovery origin capture is not the engine-allocated capture for this run and slot"
                .to_owned(),
        );
    }
    let available = process
        .enumerate_assignments(binding)
        .map_err(|error| format!("could not enumerate frozen fan-out assignments: {error}"))?
        .filter(|assignments| !assignments.is_empty())
        .ok_or_else(|| {
            "recovery requires the current engine fan-out binding with enumerable assignments"
                .to_owned()
        })?;
    let available = available.into_iter().collect::<BTreeSet<_>>();
    let mut covered = BTreeSet::new();
    for source in &recovery.sources {
        if source.assignment_id.trim().is_empty() || !covered.insert(source.assignment_id.clone()) {
            return Err("recovery source assignments must be non-empty and unique".to_owned());
        }
        if !available.contains(&source.assignment_id) {
            return Err(format!(
                "recovery source assignment `{}` is not in the frozen binding",
                source.assignment_id
            ));
        }
        let workers = origin
            .inner_workers
            .iter()
            .filter(|worker| worker.assignment_id == source.assignment_id)
            .collect::<Vec<_>>();
        let Some(worker) = workers.first().copied() else {
            return Err(format!(
                "recovery source assignment `{}` has no retained origin result",
                source.assignment_id
            ));
        };
        if workers.len() != 1 || worker.started != Some(true) || worker.exit_code != 0 {
            return Err(format!(
                "recovery source assignment `{}` is not a genuine completed worker",
                source.assignment_id
            ));
        }
        if !is_sha256(&source.raw_stdout_sha256)
            || source.raw_attempt == 0
            || worker.raw_output_sha256.as_deref() != Some(source.raw_stdout_sha256.as_str())
            || worker.raw_output_attempt != Some(source.raw_attempt)
        {
            return Err(format!(
                "recovery source assignment `{}` has a wrong raw attempt or digest",
                source.assignment_id
            ));
        }
        verify_origin_stream(
            origin,
            worker.raw_output_path.as_deref(),
            &source.raw_stdout_sha256,
        )?;
        match source.source_class.as_str() {
            "original-raw" => {
                if worker.conformance_status.as_deref() != Some("succeeded")
                    || worker.selected_attempt != Some(source.raw_attempt)
                    || worker.selected_output_sha256.as_deref()
                        != Some(source.raw_stdout_sha256.as_str())
                    || worker.selected_output_path.is_none()
                    || source.selected_output_path.is_some()
                    || source.selected_output_sha256.is_some()
                    || source.derivation.is_some()
                    || source.owner_approval.is_some()
                    || source.fidelity_approval.is_some()
                {
                    return Err(format!(
                        "raw source `{}` is not the conforming selected origin output",
                        source.assignment_id
                    ));
                }
                verify_origin_stream(
                    origin,
                    worker.selected_output_path.as_deref(),
                    &source.raw_stdout_sha256,
                )?;
            }
            "eligible-derived" => {
                if worker.conformance_status.as_deref() != Some("failed")
                    || worker.declared_output_contract.is_none()
                {
                    return Err(format!("derived source `{}` does not originate from a contracted conformance failure", source.assignment_id));
                }
                let Some(path) = source.selected_output_path.as_deref() else {
                    return Err(format!(
                        "derived source `{}` has no selected bytes path",
                        source.assignment_id
                    ));
                };
                let Some(digest) = source.selected_output_sha256.as_deref() else {
                    return Err(format!(
                        "derived source `{}` has no selected bytes digest",
                        source.assignment_id
                    ));
                };
                verify_artifact_file(&artifact_root, path, digest)?;
                validate_recovery_derivation(source, &artifact_root, recovery, invocations)?;
            }
            other => {
                return Err(format!(
                    "unsupported selected recovery source class `{other}`"
                ))
            }
        }
    }
    for assignment in &recovery.pending_assignment_ids {
        if assignment.trim().is_empty() || !covered.insert(assignment.clone()) {
            return Err(
                "pending recovery assignments must be non-empty and disjoint from selected sources"
                    .to_owned(),
            );
        }
        if !available.contains(assignment) {
            return Err(format!(
                "pending recovery assignment `{assignment}` is not in the frozen binding"
            ));
        }
        if origin.inner_workers.iter().any(|worker| {
            worker.assignment_id == *assignment
                && worker.started == Some(true)
                && worker.exit_code == 0
                && worker.conformance_status.as_deref() == Some("succeeded")
                && worker.selected_output_sha256.is_some()
                && worker.selected_output_path.is_some()
        }) {
            return Err(format!("pending recovery assignment `{assignment}` already has a conforming completed source"));
        }
    }
    if covered != available {
        return Err("recovery source and pending assignment groups do not cover the complete frozen binding".to_owned());
    }
    if covered.is_empty() {
        return Err("recovery selection is empty".to_owned());
    }
    Ok(recovery_origin_packet(origin, binding_digest))
}

fn validate_recovery_derivation(
    source: &FanOutRecoverySource,
    artifact_root: &str,
    recovery: &FanOutRecoveryInput,
    invocations: &[WorkSlotInvocation],
) -> std::result::Result<(), String> {
    let derivation = source.derivation.as_ref().ok_or_else(|| {
        format!(
            "derived source `{}` has no derivation record",
            source.assignment_id
        )
    })?;
    if !derivation.difference.is_object() && !derivation.difference.is_array() {
        return Err(format!(
            "derived source `{}` has no explicit difference",
            source.assignment_id
        ));
    }
    for (label, approval) in [
        ("owner", source.owner_approval.as_ref()),
        ("fidelity", source.fidelity_approval.as_ref()),
    ] {
        let approval = approval.ok_or_else(|| {
            format!(
                "derived source `{}` lacks {label} approval",
                source.assignment_id
            )
        })?;
        if approval.name.trim().is_empty() || approval.reason.trim().is_empty() {
            return Err(format!(
                "derived source `{}` has empty {label} approval",
                source.assignment_id
            ));
        }
    }
    match derivation.kind.as_str() {
        "mechanical" if derivation.adapter.is_none() => Ok(()),
        "scripted" => {
            let adapter = derivation
                .adapter
                .as_ref()
                .ok_or_else(|| "scripted repair is missing adapter usage accounting".to_owned())?;
            if adapter.command.trim().is_empty()
                || adapter.model_id.is_some()
                || adapter.capture.is_some()
                || adapter.calls != 1
                || adapter.max_calls == 0
                || adapter.calls > adapter.max_calls
                || adapter.elapsed_ms == 0
                || adapter.max_time_ms == 0
                || adapter.elapsed_ms > adapter.max_time_ms
                || adapter.max_cost_micros == 0
                || adapter.metered_cost_micros > adapter.max_cost_micros
                || !adapter.usage_accounted
            {
                return Err("scripted repair exceeded or omitted its positive call/time/metered-cost bounds".to_owned());
            }
            Ok(())
        }
        "model" => {
            let adapter = derivation
                .adapter
                .as_ref()
                .ok_or_else(|| "model repair is missing attributed usage accounting".to_owned())?;
            let model_id = adapter
                .model_id
                .as_deref()
                .filter(|model_id| !model_id.trim().is_empty())
                .ok_or_else(|| "model repair is missing an explicit model identity".to_owned())?;
            if adapter.command.trim().is_empty()
                || adapter.calls != 1
                || adapter.max_calls == 0
                || adapter.calls > adapter.max_calls
                || adapter.elapsed_ms == 0
                || adapter.max_time_ms == 0
                || adapter.elapsed_ms > adapter.max_time_ms
                || adapter.max_cost_micros == 0
                || adapter.metered_cost_micros > adapter.max_cost_micros
                || !adapter.usage_accounted
            {
                return Err(
                    "model repair exceeded or omitted its positive call/time/metered-cost bounds"
                        .to_owned(),
                );
            }
            verify_model_repair_capture(
                source,
                artifact_root,
                recovery,
                invocations,
                model_id,
                adapter,
            )
        }
        other => Err(format!("unsupported recovery derivation kind `{other}`")),
    }
}

fn verify_model_repair_capture(
    source: &FanOutRecoverySource,
    artifact_root: &str,
    recovery: &FanOutRecoveryInput,
    invocations: &[WorkSlotInvocation],
    model_id: &str,
    selected_usage: &crate::FanOutRecoveryAdapterUsage,
) -> std::result::Result<(), String> {
    let capture = selected_usage
        .capture
        .as_ref()
        .ok_or_else(|| "model repair has no retained adapter capture identity".to_owned())?;
    let root = std::fs::canonicalize(artifact_root)
        .map_err(|error| format!("artifact_root is unavailable: {error}"))?;
    let assignment_key = sha256_prefixed(
        format!(
            "{}:{}:{}:{}:{}",
            recovery.run_id,
            recovery.slot_id,
            recovery.state_visit,
            recovery.binding_sha256,
            source.assignment_id,
        )
        .as_bytes(),
    );
    let expected_budget_dir = root
        .join("recovery-adapter-attempts")
        .join("model")
        .join(&assignment_key[7..]);
    let budget_dir = std::fs::canonicalize(&expected_budget_dir)
        .map_err(|error| format!("model repair assignment history is unavailable: {error}"))?;
    if !budget_dir.starts_with(&root) {
        return Err("model repair assignment history escapes artifact_root".to_owned());
    }
    if budget_dir.join("budget.lock").exists() {
        return Err("model repair assignment history is locked or unfinished".to_owned());
    }
    let selected_dir = std::fs::canonicalize(&capture.directory)
        .map_err(|error| format!("selected model repair capture is unavailable: {error}"))?;
    if selected_dir.parent() != Some(budget_dir.as_path()) || !selected_dir.is_dir() {
        return Err(
            "selected model repair capture does not match its assignment history".to_owned(),
        );
    }
    let selected_request_sha = capture.request_sha256.as_str();
    let selected_stdout_sha = capture.stdout_sha256.as_str();
    let selected_stderr_sha = capture.stderr_sha256.as_str();
    let mut total_calls = 0_u64;
    let mut total_elapsed_ms = 0_u64;
    let mut total_cost_micros = 0_u64;
    let mut selected_seen = false;
    for entry in std::fs::read_dir(&budget_dir)
        .map_err(|error| format!("could not read model repair assignment history: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not read model repair attempt: {error}"))?;
        if entry.file_name() == "budget.lock" {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| format!("could not inspect model repair attempt: {error}"))?
            .is_dir()
        {
            return Err("model repair assignment history contains an unexpected entry".to_owned());
        }
        let attempt_dir = std::fs::canonicalize(entry.path())
            .map_err(|error| format!("model repair attempt is unavailable: {error}"))?;
        if attempt_dir.parent() != Some(budget_dir.as_path()) {
            return Err("model repair attempt escapes its assignment history".to_owned());
        }
        let configuration = read_recovery_json(&attempt_dir.join("configuration.json"), 64 * 1024)?;
        let args = serde_json::to_value(&selected_usage.args).map_err(|error| error.to_string())?;
        if configuration.get("kind").and_then(Value::as_str) != Some("model")
            || configuration.get("model_id").and_then(Value::as_str) != Some(model_id)
            || configuration.get("command").and_then(Value::as_str)
                != Some(selected_usage.command.as_str())
            || configuration.get("args") != Some(&args)
            || configuration.get("run_id").and_then(Value::as_str) != Some(recovery.run_id.as_str())
            || configuration.get("slot_id").and_then(Value::as_str)
                != Some(recovery.slot_id.as_str())
            || configuration.get("state_visit").and_then(Value::as_u64)
                != Some(recovery.state_visit)
            || configuration.get("binding_sha256").and_then(Value::as_str)
                != Some(recovery.binding_sha256.as_str())
            || configuration.get("assignment_id").and_then(Value::as_str)
                != Some(source.assignment_id.as_str())
            || configuration.get("max_calls").and_then(Value::as_u64)
                != Some(u64::from(selected_usage.max_calls))
            || configuration.get("max_time_ms").and_then(Value::as_u64)
                != Some(selected_usage.max_time_ms)
            || configuration.get("max_cost_micros").and_then(Value::as_u64)
                != Some(selected_usage.max_cost_micros)
        {
            return Err(
                "model repair attempt identity or assignment bounds do not match".to_owned(),
            );
        }
        let attempt_origin_id = configuration
            .get("origin_invocation_id")
            .and_then(Value::as_str)
            .filter(|identity| !identity.trim().is_empty())
            .ok_or("model repair attempt omitted its original invocation identity")?;
        let attempt_origin_capture = configuration
            .get("origin_capture_dir")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .ok_or("model repair attempt omitted its raw-origin capture directory")?;
        let raw_attempt = configuration
            .get("raw_attempt")
            .and_then(Value::as_u64)
            .filter(|attempt| *attempt > 0)
            .ok_or("model repair attempt omitted its raw-attempt number")?;
        let raw_path = configuration
            .get("raw_stdout_path")
            .and_then(Value::as_str)
            .ok_or("model repair attempt omitted its raw-attempt path")?;
        let raw_digest = configuration
            .get("raw_stdout_sha256")
            .and_then(Value::as_str)
            .filter(|digest| is_sha256(digest))
            .ok_or("model repair attempt omitted its raw-attempt digest")?;
        let matching_origins = invocations
            .iter()
            .filter(|origin| origin.invocation_id.as_str() == attempt_origin_id)
            .collect::<Vec<_>>();
        let attempt_origin = matching_origins
            .first()
            .copied()
            .ok_or("model repair raw origin is not present in run history")?;
        if matching_origins.len() != 1
            || attempt_origin.slot_id.as_str() != recovery.slot_id
            || attempt_origin.state_visit != recovery.state_visit
            || attempt_origin.subject != recovery.subject
            || work_slot_binding_digest(&attempt_origin.binding) != recovery.binding_sha256
            || attempt_origin.status != Some(WaiterWrittenStatus::Failed)
            || attempt_origin
                .ownership
                .as_ref()
                .is_none_or(|ownership| ownership.cleanup_pending || ownership.live_owned_work)
            || attempt_origin.capture_dir != attempt_origin_capture
        {
            return Err("model repair raw origin does not match this failed assignment visit or cleanup state".to_owned());
        }
        let origin_workers = attempt_origin
            .inner_workers
            .iter()
            .filter(|worker| worker.assignment_id == source.assignment_id)
            .collect::<Vec<_>>();
        let origin_worker = origin_workers
            .first()
            .copied()
            .ok_or("model repair raw origin has no captured assignment")?;
        if origin_workers.len() != 1
            || origin_worker.raw_output_attempt != u32::try_from(raw_attempt).ok()
            || origin_worker.raw_output_sha256.as_deref() != Some(raw_digest)
            || origin_worker.raw_output_path.as_deref() != Some(raw_path)
            || (attempt_origin_id == recovery.origin_invocation_id
                && (raw_attempt != u64::from(source.raw_attempt)
                    || raw_digest != source.raw_stdout_sha256))
        {
            return Err(
                "model repair raw origin attempt does not match its assignment capture".to_owned(),
            );
        }
        verify_origin_stream(attempt_origin, Some(raw_path), raw_digest)?;
        let request = read_recovery_file(&attempt_dir.join("request.json"), 2 * 1024 * 1024)?;
        let stdout = read_recovery_file(&attempt_dir.join("stdout"), 1024 * 1024)?;
        let stderr = read_recovery_file(&attempt_dir.join("stderr"), 1024 * 1024)?;
        let usage = read_recovery_json(&attempt_dir.join("usage.json"), 64 * 1024)?;
        let request_sha = sha256_prefixed(&request);
        if configuration.get("request_sha256").and_then(Value::as_str) != Some(request_sha.as_str())
        {
            return Err(
                "model repair configuration does not identify its retained request".to_owned(),
            );
        }
        let stdout_sha = sha256_prefixed(&stdout);
        let stderr_sha = sha256_prefixed(&stderr);
        let request_value: Value = serde_json::from_slice(&request)
            .map_err(|error| format!("model repair request capture is malformed: {error}"))?;
        if usage.get("usage_accounted").and_then(Value::as_bool) != Some(true)
            || usage.get("model_id").and_then(Value::as_str) != Some(model_id)
            || usage.get("command").and_then(Value::as_str) != Some(selected_usage.command.as_str())
            || usage.get("args") != Some(&args)
            || usage.get("request_sha256").and_then(Value::as_str) != Some(request_sha.as_str())
            || usage.get("stdout_sha256").and_then(Value::as_str) != Some(stdout_sha.as_str())
            || usage.get("stderr_sha256").and_then(Value::as_str) != Some(stderr_sha.as_str())
            || request_value
                .pointer("/model_adapter/model_id")
                .and_then(Value::as_str)
                != Some(model_id)
            || request_value
                .pointer("/model_adapter/command")
                .and_then(Value::as_str)
                != Some(selected_usage.command.as_str())
            || request_value.pointer("/model_adapter/args") != Some(&args)
            || request_value.pointer("/run_id").and_then(Value::as_str)
                != Some(recovery.run_id.as_str())
            || request_value.pointer("/slot_id").and_then(Value::as_str)
                != Some(recovery.slot_id.as_str())
            || request_value
                .pointer("/state_visit")
                .and_then(Value::as_u64)
                != Some(recovery.state_visit)
            || request_value
                .pointer("/binding_sha256")
                .and_then(Value::as_str)
                != Some(recovery.binding_sha256.as_str())
            || request_value
                .pointer("/origin_invocation_id")
                .and_then(Value::as_str)
                != Some(attempt_origin_id)
            || request_value
                .pointer("/assignment_id")
                .and_then(Value::as_str)
                != Some(source.assignment_id.as_str())
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
            || request_value.pointer("/budget/max_calls") != configuration.get("max_calls")
            || request_value.pointer("/budget/max_time_ms") != configuration.get("max_time_ms")
            || request_value.pointer("/budget/max_cost_micros")
                != configuration.get("max_cost_micros")
            || request_value.pointer("/budget/remaining_calls")
                != configuration.get("remaining_calls")
            || request_value.pointer("/budget/remaining_time_ms")
                != configuration.get("remaining_time_ms")
            || request_value.pointer("/budget/remaining_cost_micros")
                != configuration.get("remaining_cost_micros")
        {
            return Err(
                "model repair attempt has missing or unverifiable usage attribution".to_owned(),
            );
        }
        let calls = usage
            .get("calls")
            .and_then(Value::as_u64)
            .filter(|calls| *calls > 0)
            .ok_or("model repair attempt omitted positive metered call usage")?;
        let elapsed = usage
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .filter(|elapsed| *elapsed > 0)
            .ok_or("model repair attempt omitted positive elapsed usage")?;
        let cost = usage
            .get("metered_cost_micros")
            .and_then(Value::as_u64)
            .ok_or("model repair attempt omitted metered-cost usage")?;
        for key in ["max_calls", "max_time_ms", "max_cost_micros"] {
            if usage.get(key) != configuration.get(key) {
                return Err(
                    "model repair attempt usage disagrees with its positive bounds".to_owned(),
                );
            }
        }
        let remaining_time = configuration
            .get("remaining_time_ms")
            .and_then(Value::as_u64)
            .filter(|remaining| *remaining > 0)
            .ok_or("model repair attempt omitted its remaining time bound")?;
        let remaining_calls = configuration
            .get("remaining_calls")
            .and_then(Value::as_u64)
            .filter(|remaining| *remaining > 0)
            .ok_or("model repair attempt omitted its remaining call bound")?;
        let remaining_cost = configuration
            .get("remaining_cost_micros")
            .and_then(Value::as_u64)
            .filter(|remaining| *remaining > 0)
            .ok_or("model repair attempt omitted its remaining cost bound")?;
        if calls > remaining_calls || elapsed > remaining_time || cost > remaining_cost {
            return Err(
                "model repair attempt exceeded its remaining per-assignment bound".to_owned(),
            );
        }
        total_calls = total_calls
            .checked_add(calls)
            .ok_or("model repair call accounting overflowed")?;
        total_elapsed_ms = total_elapsed_ms
            .checked_add(elapsed)
            .ok_or("model repair time accounting overflowed")?;
        total_cost_micros = total_cost_micros
            .checked_add(cost)
            .ok_or("model repair cost accounting overflowed")?;
        if attempt_dir == selected_dir {
            selected_seen = attempt_origin_id == recovery.origin_invocation_id
                && raw_attempt == u64::from(source.raw_attempt)
                && raw_digest == source.raw_stdout_sha256
                && request_sha == selected_request_sha
                && stdout_sha == selected_stdout_sha
                && stderr_sha == selected_stderr_sha
                && calls == u64::from(selected_usage.calls)
                && elapsed == selected_usage.elapsed_ms
                && cost == selected_usage.metered_cost_micros;
        }
    }
    if !selected_seen
        || total_calls > u64::from(selected_usage.max_calls)
        || total_elapsed_ms > selected_usage.max_time_ms
        || total_cost_micros > selected_usage.max_cost_micros
    {
        return Err(
            "model repair selected source has unverifiable or exhausted assignment usage"
                .to_owned(),
        );
    }
    Ok(())
}

fn read_recovery_file(path: &std::path::Path, limit: u64) -> std::result::Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        format!(
            "model repair capture `{}` is unavailable: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "model repair capture `{}` exceeds its bound",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|error| {
        format!(
            "could not read model repair capture `{}`: {error}",
            path.display()
        )
    })
}

fn read_recovery_json(path: &std::path::Path, limit: u64) -> std::result::Result<Value, String> {
    serde_json::from_slice(&read_recovery_file(path, limit)?).map_err(|error| {
        format!(
            "model repair capture `{}` is malformed: {error}",
            path.display()
        )
    })
}

fn verify_origin_stream(
    origin: &WorkSlotInvocation,
    relative: Option<&str>,
    expected_digest: &str,
) -> std::result::Result<(), String> {
    let relative =
        relative.ok_or_else(|| "origin selected stream has no retained path".to_owned())?;
    let relative_path = PathBuf::from(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("origin selected stream path is not a safe capture-relative path".to_owned());
    }
    let path = PathBuf::from(&origin.capture_dir).join(relative_path);
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("origin stream `{}` is unavailable: {error}", path.display()))?;
    if sha256_prefixed(&bytes) != expected_digest {
        return Err(format!(
            "origin stream `{}` no longer matches its retained digest",
            path.display()
        ));
    }
    Ok(())
}

fn verify_artifact_file(
    artifact_root: &str,
    path: &str,
    expected_digest: &str,
) -> std::result::Result<(), String> {
    if !is_sha256(expected_digest) {
        return Err("derived output digest is malformed".to_owned());
    }
    let root = std::fs::canonicalize(artifact_root)
        .map_err(|error| format!("artifact_root is unavailable: {error}"))?;
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("derived output is unavailable: {error}"))?;
    if !path.starts_with(&root) {
        return Err("derived output must be contained in the run artifact_root".to_owned());
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "derived output `{}` is unavailable: {error}",
            path.display()
        )
    })?;
    if sha256_prefixed(&bytes) != expected_digest {
        return Err("derived output bytes do not match their declared digest".to_owned());
    }
    Ok(())
}

fn is_sha256(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn recovery_origin_packet(origin: &WorkSlotInvocation, binding_digest: &str) -> Value {
    let assignments = origin
        .inner_workers
        .iter()
        .map(|worker| {
            json!({
                "assignment_id": worker.assignment_id,
                "command": worker.command,
                "args": worker.args,
                "exit_code": worker.exit_code,
                "started": worker.started,
                "selected_attempt": worker.selected_attempt,
                "selected_output_sha256": worker.selected_output_sha256,
                "selected_output_path": worker.selected_output_path,
                "raw_output_sha256": worker.raw_output_sha256,
                "raw_output_path": worker.raw_output_path,
                "raw_output_attempt": worker.raw_output_attempt,
                "conformance_status": worker.conformance_status,
                "conformance_error": worker.conformance_error,
                "declared_output_contract": worker.declared_output_contract,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "protocol": "fan-out-origin-v1",
        "invocation_id": origin.invocation_id,
        "slot_id": origin.slot_id,
        "state_visit": origin.state_visit,
        "subject": origin.subject,
        "binding_sha256": binding_digest,
        "capture_dir": origin.capture_dir,
        "exit_code": origin.exit_code,
        "status": "failed",
        "quiescent": true,
        "assignments": assignments,
    })
}

fn labels_from_facade_preparation(
    preparation: Option<&Value>,
) -> std::result::Result<Vec<AssignmentLabel>, String> {
    let Some(value) = preparation.and_then(|value| value.get("assignment_labels")) else {
        return Ok(Vec::new());
    };
    serde_json::from_value::<Vec<AssignmentLabel>>(value.clone())
        .map_err(|error| format!("facade assignment_labels are malformed: {error}"))
}

fn validate_assignment_labels(
    labels: Vec<AssignmentLabel>,
) -> std::result::Result<Vec<AssignmentLabel>, String> {
    if labels.len() > 4096 {
        return Err("assignment label inventory exceeds 4096 entries".to_owned());
    }
    let mut identities = BTreeSet::new();
    for label in &labels {
        if label.assignment_id.trim().is_empty()
            || label.title.trim().is_empty()
            || label.role.trim().is_empty()
        {
            return Err(
                "assignment labels require non-empty assignment_id, title, and role".to_owned(),
            );
        }
        if label.assignment_id.len() > 1024 || label.title.len() > 1024 || label.role.len() > 1024 {
            return Err("assignment label fields may not exceed 1024 bytes".to_owned());
        }
        if !identities.insert(label.assignment_id.as_str()) {
            return Err(format!(
                "assignment label `{}` is duplicated",
                label.assignment_id
            ));
        }
    }
    Ok(labels)
}

fn validate_assignment_selection<P: WorkSlotProcess + ?Sized>(
    process: &P,
    binding: &WorkSlotBinding,
    requested: Option<&[String]>,
) -> std::result::Result<Option<Vec<String>>, OperationOutcome<Result>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if requested.is_empty() {
        return Err(OperationOutcome::rejected(
            "empty-assignment-selection",
            "assignment selection must contain at least one identity",
        ));
    }
    let available = match process.enumerate_assignments(binding) {
        Ok(Some(available)) if !available.is_empty() => available,
        Ok(Some(_)) | Ok(None) => {
            return Err(OperationOutcome::rejected(
                "assignments-not-enumerable",
                "the bound work slot does not expose enumerable assignments",
            ));
        }
        Err(error) => return Err(process_error(error)),
    };
    let available = available.into_iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for identity in requested {
        if identity.is_empty() {
            return Err(OperationOutcome::rejected(
                "empty-assignment-identity",
                "assignment identities must be non-empty",
            ));
        }
        if !seen.insert(identity) {
            return Err(OperationOutcome::rejected(
                "duplicate-assignment-selection",
                format!("assignment `{identity}` was selected more than once"),
            ));
        }
        if !available.contains(identity) {
            return Err(OperationOutcome::rejected(
                "unknown-assignment",
                format!("assignment `{identity}` is not in the bound work slot"),
            ));
        }
    }
    Ok(Some(requested.to_vec()))
}

fn artifact_root_from_input(initial_input: &Value) -> String {
    initial_input
        .get("artifact_root")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn capture_dir_path(
    artifact_root: &str,
    slot_id: &WorkSlotId,
    invocation_id: &InvocationId,
) -> PathBuf {
    PathBuf::from(artifact_root)
        .join("work-slot-captures")
        .join(slot_id.as_str())
        .join(invocation_id.as_str())
}

fn process_error<T>(error: ProcessError) -> OperationOutcome<T> {
    OperationOutcome::error_with_issue(crate::OutcomeIssue::new(error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppendContextRequest, AppendContextResult, CheckedEvaluationSnapshot,
        CheckedEvaluationSnapshotRequest, CommitTransitionRequest, CommitTransitionResult,
        CompleteWorkSlotInvocationRequest, CompleteWorkSlotInvocationResult, ContextRecord,
        CreateRunRequest, CreateRunResult, CreateWorkSlotInvocationResult, HistoryEntry, Lifecycle,
        PersistenceError, PersistenceFailure, ProviderAssociation, RecordDenialRequest,
        RecordDenialResult, Run, RunSummary, ShowData, StartedWaiter, State, TerminateRequest,
        TerminateResult, Transition, WaiterWrittenStatus, WorkSlot, WorkSlotInvocation, Workflow,
    };
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeSet, HashMap};
    use std::rc::Rc;

    type CallLog = Rc<RefCell<Vec<&'static str>>>;

    fn workflow() -> Workflow {
        Workflow::new(
            "workflow",
            "start",
            vec![
                State::new("start", "Start", "Do the slot work", false),
                State::new("done", "Done", "Finished", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        )
        .with_work_slots(vec![WorkSlot::new("slot-1", "start", "finish")])
    }

    fn bound_input(artifact_root: &str) -> Value {
        json!({
            "artifact_root": artifact_root,
            "work_slot_bindings": {
                "slot-1": {"command": "echo", "args": ["hello"]}
            }
        })
    }

    fn expected_capture_dir(artifact_root: &str, slot_id: &str, invocation_id: &str) -> String {
        PathBuf::from(artifact_root)
            .join("work-slot-captures")
            .join(slot_id)
            .join(invocation_id)
            .to_string_lossy()
            .into_owned()
    }

    fn sample_run(initial_input: Value) -> Run {
        Run::new(
            "run-1",
            None,
            workflow(),
            ProviderAssociation::new(json!({"provider": "fake"})),
            initial_input,
            "start",
            Lifecycle::Active,
            0_u64.into(),
            1_u64.into(),
            Timestamp::from_unix_millis(10),
        )
    }

    fn invoke_request() -> Request {
        Request::new("run-1", "slot-1", "inv-1", "/tmp/loop.db")
    }

    fn invocation(
        slot_id: &str,
        started_at: i64,
        allowed_time_ms: u64,
        status: Option<WaiterWrittenStatus>,
        waiter_pid: u32,
    ) -> WorkSlotInvocation {
        WorkSlotInvocation::new(
            "inv-existing",
            slot_id,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()]),
            "digest",
            "subject-existing",
            waiter_pid,
            Timestamp::from_unix_millis(started_at),
            allowed_time_ms,
            status,
            None,
            None,
            String::new(),
            Vec::new(),
        )
    }

    fn unavailable<T>() -> std::result::Result<T, PersistenceError> {
        Err(PersistenceError::failure(PersistenceFailure::new(
            "fake-failure",
            "fake persistence failure",
        )))
    }

    struct FakePersistence {
        run: RefCell<std::result::Result<Run, PersistenceError>>,
        invocations: RefCell<Vec<WorkSlotInvocation>>,
        created: RefCell<Vec<CreateWorkSlotInvocationRequest>>,
        subjects: RefCell<HashMap<(String, String), String>>,
        set_subject_calls: RefCell<Vec<(String, String, String)>>,
        context_records: RefCell<Vec<ContextRecord>>,
        log: CallLog,
    }

    impl FakePersistence {
        fn new(run: Run, log: CallLog) -> Self {
            Self {
                run: RefCell::new(Ok(run)),
                invocations: RefCell::new(Vec::new()),
                created: RefCell::new(Vec::new()),
                subjects: RefCell::new(HashMap::new()),
                set_subject_calls: RefCell::new(Vec::new()),
                context_records: RefCell::new(Vec::new()),
                log,
            }
        }

        fn with_subject(self, slot_id: &str, subject: &str) -> Self {
            self.subjects
                .borrow_mut()
                .insert(("run-1".to_owned(), slot_id.to_owned()), subject.to_owned());
            self
        }

        fn with_invocations(self, invocations: Vec<WorkSlotInvocation>) -> Self {
            *self.invocations.borrow_mut() = invocations;
            self
        }

        fn with_context_records(self, records: Vec<ContextRecord>) -> Self {
            *self.context_records.borrow_mut() = records;
            self
        }
    }

    impl Persistence for FakePersistence {
        fn create_run(
            &self,
            _request: CreateRunRequest,
        ) -> std::result::Result<CreateRunResult, PersistenceError> {
            unavailable()
        }

        fn append_context(
            &self,
            _request: AppendContextRequest,
        ) -> std::result::Result<AppendContextResult, PersistenceError> {
            unavailable()
        }

        fn commit_transition(
            &self,
            _request: CommitTransitionRequest,
        ) -> std::result::Result<CommitTransitionResult, PersistenceError> {
            unavailable()
        }

        fn record_denial(
            &self,
            _request: RecordDenialRequest,
        ) -> std::result::Result<RecordDenialResult, PersistenceError> {
            unavailable()
        }

        fn terminate(
            &self,
            _request: TerminateRequest,
        ) -> std::result::Result<TerminateResult, PersistenceError> {
            unavailable()
        }

        fn load_authoritative_run(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Run, PersistenceError> {
            match &*self.run.borrow() {
                Ok(run) => Ok(run.clone()),
                Err(error) => Err(error.clone()),
            }
        }

        fn list_runs(&self) -> std::result::Result<Vec<RunSummary>, PersistenceError> {
            unavailable()
        }

        fn load_context_records(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<ContextRecord>, PersistenceError> {
            Ok(self.context_records.borrow().clone())
        }

        fn load_history(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<HistoryEntry>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<crate::DurableEvaluation>, PersistenceError> {
            unavailable()
        }

        fn load_checked_evaluation_snapshot(
            &self,
            _request: CheckedEvaluationSnapshotRequest,
        ) -> std::result::Result<CheckedEvaluationSnapshot, PersistenceError> {
            unavailable()
        }

        fn load_show_data(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<ShowData, PersistenceError> {
            unavailable()
        }

        fn create_work_slot_invocation(
            &self,
            request: CreateWorkSlotInvocationRequest,
        ) -> std::result::Result<CreateWorkSlotInvocationResult, PersistenceError> {
            self.log.borrow_mut().push("create");
            self.created.borrow_mut().push(request.clone());
            Ok(CreateWorkSlotInvocationResult {
                invocation: WorkSlotInvocation::new(
                    request.invocation_id.clone(),
                    request.slot_id.clone(),
                    request.binding.clone(),
                    request.instruction_digest.clone(),
                    request.subject.clone(),
                    request.waiter_pid,
                    request.started_at,
                    request.allowed_time_ms,
                    None,
                    None,
                    None,
                    request.capture_dir.clone(),
                    Vec::new(),
                ),
                history: HistoryEntry::invocation_started(
                    2_u64.into(),
                    request.started_at,
                    request.invocation_id,
                ),
            })
        }

        fn complete_work_slot_invocation(
            &self,
            _request: CompleteWorkSlotInvocationRequest,
        ) -> std::result::Result<CompleteWorkSlotInvocationResult, PersistenceError> {
            unavailable()
        }

        fn get_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &WorkSlotId,
        ) -> std::result::Result<Option<String>, PersistenceError> {
            Ok(self
                .subjects
                .borrow()
                .get(&(run_id.as_str().to_owned(), slot_id.as_str().to_owned()))
                .cloned())
        }

        fn set_current_slot_subject(
            &self,
            run_id: &RunId,
            slot_id: &WorkSlotId,
            subject: String,
        ) -> std::result::Result<(), PersistenceError> {
            self.set_subject_calls.borrow_mut().push((
                run_id.as_str().to_owned(),
                slot_id.as_str().to_owned(),
                subject.clone(),
            ));
            self.subjects.borrow_mut().insert(
                (run_id.as_str().to_owned(), slot_id.as_str().to_owned()),
                subject,
            );
            Ok(())
        }

        fn load_work_slot_invocations(
            &self,
            _run_id: &RunId,
        ) -> std::result::Result<Vec<WorkSlotInvocation>, PersistenceError> {
            Ok(self.invocations.borrow().clone())
        }
    }

    struct FakeProcess {
        log: CallLog,
        spawn_args: RefCell<Vec<WaiterSpawnArgs>>,
        envelopes: RefCell<Vec<Vec<u8>>>,
        next_pid: Cell<u32>,
        alive: RefCell<HashMap<u32, bool>>,
        default_alive: bool,
        assignments: Option<Vec<String>>,
        waited: Cell<bool>,
    }

    impl FakeProcess {
        fn new(log: CallLog) -> Self {
            Self {
                log,
                spawn_args: RefCell::new(Vec::new()),
                envelopes: RefCell::new(Vec::new()),
                next_pid: Cell::new(4242),
                alive: RefCell::new(HashMap::new()),
                default_alive: true,
                assignments: None,
                waited: Cell::new(false),
            }
        }

        fn set_alive(&self, pid: u32, alive: bool) {
            self.alive.borrow_mut().insert(pid, alive);
        }

        fn with_assignments(mut self, assignments: &[&str]) -> Self {
            self.assignments = Some(assignments.iter().map(|id| (*id).to_owned()).collect());
            self
        }
    }

    impl WorkSlotProcess for FakeProcess {
        type Handle = ();

        fn waiter_alive(&self, pid: u32) -> bool {
            self.alive
                .borrow()
                .get(&pid)
                .copied()
                .unwrap_or(self.default_alive)
        }

        fn enumerate_assignments(
            &self,
            _binding: &WorkSlotBinding,
        ) -> std::result::Result<Option<Vec<String>>, ProcessError> {
            Ok(self.assignments.clone())
        }

        fn spawn_wait_invocation(
            &self,
            args: WaiterSpawnArgs,
        ) -> std::result::Result<StartedWaiter<()>, ProcessError> {
            self.log.borrow_mut().push("spawn");
            self.spawn_args.borrow_mut().push(args);
            Ok(StartedWaiter::new(self.next_pid.get(), ()))
        }

        fn send_envelope_and_detach(
            &self,
            _waiter: StartedWaiter<()>,
            envelope_json: &[u8],
        ) -> std::result::Result<(), ProcessError> {
            self.log.borrow_mut().push("send");
            self.envelopes.borrow_mut().push(envelope_json.to_vec());
            Ok(())
        }
    }

    fn harness(run: Run) -> (FakePersistence, FakeProcess, CallLog) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let persistence = FakePersistence::new(run, log.clone());
        let process = FakeProcess::new(log.clone());
        (persistence, process, log)
    }

    #[test]
    fn unknown_slot_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let request = Request::new("run-1", "missing-slot", "inv-1", "/tmp/loop.db");

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "unknown-work-slot");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
        assert!(!process.waited.get());
    }

    #[test]
    fn terminal_run_keeps_run_not_active_refusal_before_observation_guard() {
        let mut run = sample_run(bound_input("/tmp/artifacts"));
        run.lifecycle = crate::Lifecycle::Final;
        let (persistence, process, log) = harness(run);
        let persistence = persistence.with_subject("slot-1", "visit-1");

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "run-not-active");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn unbound_missing_empty_or_omitted_slot_is_rejected_without_spawn() {
        let inputs = [
            json!({"artifact_root": "/tmp/artifacts"}),
            json!({"artifact_root": "/tmp/artifacts", "work_slot_bindings": {}}),
            json!({
                "artifact_root": "/tmp/artifacts",
                "work_slot_bindings": {
                    "other-slot": {"command": "echo", "args": []}
                }
            }),
        ];
        for input in inputs {
            let (persistence, process, log) = harness(sample_run(input));
            let persistence = persistence.with_subject("slot-1", "visit-1");

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );

            assert!(outcome.is_rejected(), "{outcome:?}");
            assert_eq!(outcome.issue().unwrap().code, "unbound-work-slot");
            assert!(process.spawn_args.borrow().is_empty());
            assert!(persistence.created.borrow().is_empty());
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn overlay_running_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_invocations(vec![invocation("slot-1", 1_000, 5_000, None, 42)]);
        process.set_alive(42, true);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(2_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "work-slot-already-running");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn recovery_cancellation_overrun_live_waiter_refuses_retry() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_invocations(vec![invocation("slot-1", 1_000, 5_000, None, 42)]);
        process.set_alive(42, true);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(6_000),
            30_000,
        );

        assert!(outcome.is_rejected(), "{outcome:?}");
        assert!(outcome
            .issue()
            .unwrap()
            .message
            .contains("cancel-invocation"));
        assert!(log.borrow().is_empty());
        assert!(!process.waited.get());
    }

    #[test]
    fn overlay_failed_and_succeeded_are_not_already_running() {
        for status in [
            Some(WaiterWrittenStatus::Failed),
            Some(WaiterWrittenStatus::Succeeded),
        ] {
            let artifacts = tempfile::tempdir().expect("temp artifact root");
            let artifact_root = artifacts.path().to_string_lossy().into_owned();
            let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
            let persistence = persistence
                .with_subject("slot-1", "visit-1")
                .with_invocations(vec![invocation("slot-1", 1_000, 5_000, status, 42)]);
            process.set_alive(42, true);

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(2_000),
                30_000,
            );

            assert!(outcome.is_completed(), "{status:?} {outcome:?}");
            assert_eq!(&*log.borrow(), &["spawn", "create", "send"]);
            assert!(!process.waited.get());
        }
    }

    #[test]
    fn happy_path_creates_invocation_then_sends_envelope_without_waitpid() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let expected_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-1");
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let now = Timestamp::from_unix_millis(1_000);
        let allowed_time_ms = 12_345;

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            now,
            allowed_time_ms,
        );

        assert!(outcome.is_completed(), "{outcome:?}");
        let result = outcome.value().expect("completed invoke");
        assert_eq!(result.invocation_id.as_str(), "inv-1");
        assert_eq!(result.slot_id.as_str(), "slot-1");
        assert_eq!(result.started_at, now);
        assert_eq!(result.allowed_time_ms, allowed_time_ms);
        assert_eq!(result.capture_dir, expected_dir);
        assert!(
            PathBuf::from(&result.capture_dir).is_dir(),
            "capture_dir should exist: {}",
            result.capture_dir
        );
        assert_eq!(&*log.borrow(), &["spawn", "create", "send"]);
        assert!(!process.waited.get());
        assert!(persistence.set_subject_calls.borrow().is_empty());

        let created = persistence.created.borrow();
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].instruction_digest,
            instruction_digest("Do the slot work")
        );
        assert_eq!(created[0].subject, "visit-1");
        assert_eq!(created[0].allowed_time_ms, allowed_time_ms);
        assert_eq!(created[0].waiter_pid, 4242);
        assert_eq!(created[0].started_at, now);
        assert_eq!(created[0].capture_dir, expected_dir);
        assert_eq!(
            created[0].binding,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()])
        );

        let spawn = &process.spawn_args.borrow()[0];
        assert_eq!(spawn.run_id.as_str(), "run-1");
        assert_eq!(spawn.invocation_id.as_str(), "inv-1");
        assert_eq!(spawn.database, PathBuf::from("/tmp/loop.db"));

        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope json");
        assert_eq!(envelope["command"], "echo");
        assert_eq!(envelope["args"], json!(["hello"]));
        let packet = envelope["worker_packet"]
            .as_object()
            .expect("worker_packet object");
        let keys = packet
            .keys()
            .map(|key| key.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "run_id",
                "slot_id",
                "artifact_root",
                "instruction_body",
                "capture_dir",
                "state_visit",
                "binding_sha256",
            ])
        );
        assert_eq!(packet["run_id"], "run-1");
        assert_eq!(packet["slot_id"], "slot-1");
        assert_eq!(packet["artifact_root"], artifact_root);
        assert_eq!(packet["instruction_body"], "Do the slot work");
        assert_eq!(packet["capture_dir"], expected_dir);
        assert!(packet.get("command").is_none());
    }

    #[test]
    fn selected_assignments_are_recorded_and_forwarded_without_rewriting_binding() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, _log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let process = process.with_assignments(&["worker-0", "worker-1"]);
        let request = invoke_request().with_assignment_selection(Some(vec!["worker-1".to_owned()]));

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_completed(), "{outcome:?}");
        assert_eq!(
            persistence.created.borrow()[0].assignment_selection,
            Some(vec!["worker-1".to_owned()])
        );
        assert_eq!(
            persistence.created.borrow()[0].binding,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()])
        );
        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope");
        assert_eq!(
            envelope["worker_packet"]["assignment_selection"],
            json!(["worker-1"])
        );
    }

    #[test]
    fn opaque_invocation_input_is_recorded_and_forwarded_without_binding_changes() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, _log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let invocation_input = json!({"plan_revision": "r2", "task_roots": ["task-b"]});
        let request = invoke_request().with_invocation_input(Some(invocation_input.clone()));

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_completed(), "{outcome:?}");
        let created = &persistence.created.borrow()[0];
        assert_eq!(created.invocation_input, Some(invocation_input.clone()));
        assert_eq!(
            created.binding,
            WorkSlotBinding::new("echo", vec!["hello".to_owned()])
        );
        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope");
        assert_eq!(
            envelope["worker_packet"]["invocation_input"],
            invocation_input
        );
    }

    #[test]
    fn invocation_input_and_assignment_selection_are_rejected_before_waiter_spawn() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let process = process.with_assignments(&["worker-0"]);
        let request = invoke_request()
            .with_assignment_selection(Some(vec!["worker-0".to_owned()]))
            .with_invocation_input(Some(json!({"opaque": true})));

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected(), "{outcome:?}");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn invalid_assignment_selection_is_rejected_before_waiter_spawn() {
        for selection in [
            Vec::new(),
            vec!["unknown".to_owned()],
            vec!["worker-0".to_owned(), "worker-0".to_owned()],
        ] {
            let artifacts = tempfile::tempdir().expect("temp artifact root");
            let artifact_root = artifacts.path().to_string_lossy().into_owned();
            let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
            let persistence = persistence.with_subject("slot-1", "visit-1");
            let process = process.with_assignments(&["worker-0", "worker-1"]);
            let request = invoke_request().with_assignment_selection(Some(selection));

            let outcome = execute(
                request,
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );

            assert!(outcome.is_rejected(), "{outcome:?}");
            assert!(process.spawn_args.borrow().is_empty());
            assert!(persistence.created.borrow().is_empty());
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn selection_on_opaque_binding_is_rejected_before_waiter_spawn() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");
        let request = invoke_request().with_assignment_selection(Some(vec!["worker-0".to_owned()]));

        let outcome = execute(
            request,
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected(), "{outcome:?}");
        assert_eq!(outcome.issue().unwrap().code, "assignments-not-enumerable");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn empty_artifact_root_errors_before_spawn() {
        let inputs = [
            json!({
                "work_slot_bindings": {
                    "slot-1": {"command": "echo", "args": ["hello"]}
                }
            }),
            json!({
                "artifact_root": "",
                "work_slot_bindings": {
                    "slot-1": {"command": "echo", "args": ["hello"]}
                }
            }),
        ];
        for input in inputs {
            let (persistence, process, log) = harness(sample_run(input));
            let persistence = persistence.with_subject("slot-1", "visit-1");

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );

            assert!(outcome.is_error(), "{outcome:?}");
            assert_eq!(outcome.issue().unwrap().code, "capture-directory-failed");
            assert!(process.spawn_args.borrow().is_empty());
            assert!(persistence.created.borrow().is_empty());
            assert!(log.borrow().is_empty());
        }
    }

    #[test]
    fn capture_dir_create_failure_errors_without_spawn() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let blocker = PathBuf::from(&artifact_root)
            .join("work-slot-captures")
            .join("slot-1");
        std::fs::create_dir_all(blocker.parent().expect("parent")).expect("create parent");
        std::fs::write(&blocker, b"not a directory").expect("write blocker file");

        let (persistence, process, log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_error(), "{outcome:?}");
        assert_eq!(outcome.issue().unwrap().code, "capture-directory-failed");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(persistence.created.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn second_invoke_uses_a_distinct_capture_dir_and_leaves_the_first() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let first_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-1");
        let second_dir = expected_capture_dir(&artifact_root, "slot-1", "inv-2");
        let (persistence, process, _log) = harness(sample_run(bound_input(&artifact_root)));
        let persistence = persistence.with_subject("slot-1", "visit-1");

        let first = execute(
            Request::new("run-1", "slot-1", "inv-1", "/tmp/loop.db"),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(first.is_completed(), "{first:?}");
        let first_result = first.value().expect("first invoke");
        assert_eq!(first_result.capture_dir, first_dir);
        assert!(PathBuf::from(&first_dir).is_dir());
        std::fs::write(PathBuf::from(&first_dir).join("marker.txt"), b"keep").expect("marker");

        let second = execute(
            Request::new("run-1", "slot-1", "inv-2", "/tmp/loop.db"),
            &persistence,
            &process,
            Timestamp::from_unix_millis(2_000),
            30_000,
        );
        assert!(second.is_completed(), "{second:?}");
        let second_result = second.value().expect("second invoke");
        assert_eq!(second_result.capture_dir, second_dir);
        assert_ne!(second_result.capture_dir, first_result.capture_dir);
        assert!(PathBuf::from(&second_dir).is_dir());
        assert!(PathBuf::from(&first_dir).is_dir());
        assert_eq!(
            std::fs::read(PathBuf::from(&first_dir).join("marker.txt")).expect("read marker"),
            b"keep"
        );
        assert_eq!(persistence.created.borrow()[0].capture_dir, first_dir);
        assert_eq!(persistence.created.borrow()[1].capture_dir, second_dir);
    }

    #[test]
    fn missing_current_subject_is_rejected_without_spawn() {
        let (persistence, process, log) = harness(sample_run(bound_input("/tmp/artifacts")));

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );

        assert!(outcome.is_rejected());
        assert_eq!(outcome.issue().unwrap().code, "no-current-visit-subject");
        assert!(process.spawn_args.borrow().is_empty());
        assert!(log.borrow().is_empty());
    }

    fn run_with_slot(artifact_root: &str, slot: WorkSlot) -> Run {
        let mut run = sample_run(bound_input(artifact_root));
        run.workflow.work_slots = vec![slot];
        run
    }

    fn record(id: &str, kind: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            crate::SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }

    fn worker_packet(process: &FakeProcess) -> Value {
        let envelope: Value =
            serde_json::from_slice(&process.envelopes.borrow()[0]).expect("envelope json");
        envelope["worker_packet"].clone()
    }

    #[test]
    fn omitted_and_empty_stdin_context_kinds_keep_five_key_packet() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let slots = [
            WorkSlot::new("slot-1", "start", "finish"),
            WorkSlot::new("slot-1", "start", "finish").with_stdin_context_kinds(Vec::new()),
        ];
        for slot in slots {
            let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
            let persistence = persistence
                .with_subject("slot-1", "visit-1")
                .with_context_records(vec![record(
                    "ctx-1",
                    "kind-a",
                    1,
                    json!({"payload": "stored"}),
                )]);

            let outcome = execute(
                invoke_request(),
                &persistence,
                &process,
                Timestamp::from_unix_millis(1_000),
                30_000,
            );
            assert!(outcome.is_completed(), "{outcome:?}");
            let packet = worker_packet(&process);
            let object = packet.as_object().expect("packet");
            assert!(object.get("context").is_none());
            let keys = object
                .keys()
                .map(|key| key.as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                keys,
                BTreeSet::from([
                    "run_id",
                    "slot_id",
                    "artifact_root",
                    "instruction_body",
                    "capture_dir",
                    "state_visit",
                    "binding_sha256",
                ])
            );
        }
    }

    #[test]
    fn work_slot_omitting_stdin_context_kinds_round_trips_without_the_key() {
        let omitted = json!({
            "id": "slot-1",
            "state": "start",
            "event": "finish"
        });
        let slot: WorkSlot = serde_json::from_value(omitted).unwrap();
        assert_eq!(slot, WorkSlot::new("slot-1", "start", "finish"));
        assert!(slot.stdin_context_kinds.is_empty());
        let encoded = serde_json::to_value(&slot).unwrap();
        assert!(encoded.get("stdin_context_kinds").is_none());
        assert_eq!(
            encoded,
            json!({
                "id": "slot-1",
                "state": "start",
                "event": "finish"
            })
        );

        let empty = WorkSlot::new("slot-1", "start", "finish").with_stdin_context_kinds(Vec::new());
        assert!(serde_json::to_value(&empty)
            .unwrap()
            .get("stdin_context_kinds")
            .is_none());
    }

    #[test]
    fn nonempty_stdin_context_kinds_forwards_matching_records_in_append_order() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let older = record(
            "ctx-old",
            "kind-a",
            1,
            json!({"rev": "1", "note": "historical"}),
        );
        let other_kind = record("ctx-other", "kind-b", 2, json!({"rev": "skip"}));
        let between = record("ctx-mid", "kind-c", 3, json!({"rev": "mid"}));
        let newer = record(
            "ctx-new",
            "kind-a",
            4,
            json!({"rev": "2", "note": "current"}),
        );
        let slot = WorkSlot::new("slot-1", "start", "finish")
            .with_stdin_context_kinds(vec!["kind-a".to_owned(), "kind-c".to_owned()]);
        let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_context_records(vec![
                older.clone(),
                other_kind,
                between.clone(),
                newer.clone(),
            ]);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        let packet = worker_packet(&process);
        let forwarded = packet["context"].as_array().expect("context array");
        assert_eq!(packet["standing_assignment_ids"], json!([]));
        assert_eq!(forwarded.len(), 3);
        assert_eq!(forwarded[0], serde_json::to_value(&older).unwrap());
        assert_eq!(forwarded[1], serde_json::to_value(&between).unwrap());
        assert_eq!(forwarded[2], serde_json::to_value(&newer).unwrap());
        assert_eq!(
            forwarded[0]["data"],
            json!({"rev": "1", "note": "historical"})
        );
        assert_eq!(forwarded[2]["data"], json!({"rev": "2", "note": "current"}));
    }

    #[test]
    fn nonempty_stdin_context_kinds_with_no_matches_still_emits_context_key() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let slot = WorkSlot::new("slot-1", "start", "finish")
            .with_stdin_context_kinds(vec!["kind-a".to_owned()]);
        let (persistence, process, _log) = harness(run_with_slot(&artifact_root, slot));
        let persistence = persistence
            .with_subject("slot-1", "visit-1")
            .with_context_records(vec![record("ctx-other", "kind-b", 1, json!({}))]);

        let outcome = execute(
            invoke_request(),
            &persistence,
            &process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(outcome.is_completed(), "{outcome:?}");
        let packet = worker_packet(&process);
        assert_eq!(packet["context"], json!([]));
        assert_eq!(packet["standing_assignment_ids"], json!([]));
    }

    #[test]
    fn review_slot_forwards_finding_ledger_and_draft_slot_omits_context() {
        let artifacts = tempfile::tempdir().expect("temp artifact root");
        let artifact_root = artifacts.path().to_string_lossy().into_owned();
        let draft = WorkSlot::new("intent-draft", "explore", "intent-ready");
        let review = WorkSlot::new("intent-review", "intent-review", "approved")
            .with_stdin_context_kinds(vec!["finding-ledger".to_owned()]);
        let findings = record(
            "accepted-1",
            "finding-ledger",
            1,
            json!({
                "gate": "intent-review",
                "subject": "intent.json",
                "subject_revision": "1",
                "findings": []
            }),
        );
        let evidence = record(
            "evidence-1",
            "review-evidence",
            2,
            json!({"gate": "intent-review", "policy_id": "axis", "result": "pass"}),
        );
        let mut run = sample_run(json!({
            "artifact_root": artifact_root,
            "work_slot_bindings": {
                "intent-draft": {"command": "echo", "args": ["draft"]},
                "intent-review": {"command": "echo", "args": ["review"]}
            }
        }));
        run.workflow = Workflow::new(
            "workflow",
            "explore",
            vec![
                State::new("explore", "Explore", "Draft", false),
                State::new("intent-review", "Intent review", "Review", false),
                State::new("end", "End", "Done", true),
            ],
            vec![
                Transition::checked("explore", "intent-ready", "intent-review"),
                Transition::checked("intent-review", "approved", "end"),
            ],
        )
        .with_work_slots(vec![draft, review]);

        let (draft_persistence, draft_process, _draft_log) = harness(run.clone());
        let draft_persistence = draft_persistence
            .with_subject("intent-draft", "visit-draft")
            .with_context_records(vec![findings.clone(), evidence.clone()]);
        let draft_outcome = execute(
            Request::new("run-1", "intent-draft", "inv-draft", "/tmp/loop.db"),
            &draft_persistence,
            &draft_process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(draft_outcome.is_completed(), "{draft_outcome:?}");
        let draft_packet = worker_packet(&draft_process);
        assert_eq!(draft_packet["slot_id"], "intent-draft");
        assert!(draft_packet.get("context").is_none());
        let draft_keys = draft_packet
            .as_object()
            .expect("packet")
            .keys()
            .map(|key| key.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            draft_keys,
            BTreeSet::from([
                "run_id",
                "slot_id",
                "artifact_root",
                "instruction_body",
                "capture_dir",
                "state_visit",
                "binding_sha256",
            ])
        );

        let (review_persistence, review_process, _review_log) = harness(run);
        let review_persistence = review_persistence
            .with_subject("intent-review", "visit-review")
            .with_context_records(vec![findings.clone(), evidence]);
        let review_outcome = execute(
            Request::new("run-1", "intent-review", "inv-review", "/tmp/loop.db"),
            &review_persistence,
            &review_process,
            Timestamp::from_unix_millis(1_000),
            30_000,
        );
        assert!(review_outcome.is_completed(), "{review_outcome:?}");
        let review_packet = worker_packet(&review_process);
        assert_eq!(review_packet["slot_id"], "intent-review");
        let forwarded = review_packet["context"].as_array().expect("context");
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0], serde_json::to_value(&findings).unwrap());
    }
}
