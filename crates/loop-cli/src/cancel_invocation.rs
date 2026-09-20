//! The public cancel command is the cleanup controller, not a request queue.
//! One ten-second monotonic clock starts at controller-lock acquisition; an
//! interrupted attempt remains incomplete and the durable admission stop stays.
use crate::{now_timestamp, CliError};
use loop_core::{InvocationId, OperationOutcome, Persistence, RunId, WorkSlotInvocation};
use loop_integrations::{
    ownership::{self, Admission},
    SqlitePersistence,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const LIMIT: Duration = Duration::from_secs(10);
const GRACE: Duration = Duration::from_secs(3);

pub(crate) fn execute(
    persistence: &SqlitePersistence,
    run: RunId,
    id: InvocationId,
) -> OperationOutcome<Value> {
    match control(persistence, &run, &id) {
        Ok(value) => OperationOutcome::completed(value),
        Err(error)
            if matches!(
                error.code.as_str(),
                "invocation-not-found"
                    | "invocation-not-running"
                    | "invocation-not-current"
                    | "ownership-unavailable"
                    | "cancellation-controller-active"
                    | "run-not-active"
            ) =>
        {
            OperationOutcome::rejected(error.code, error.message)
        }
        Err(error) => OperationOutcome::error(error.code, error.message),
    }
}

fn failure(error: impl std::fmt::Display) -> CliError {
    CliError::new("cancellation-cleanup-pending", format!("{error}; cleanup is unverified; wait or retry cancel-invocation (the live-work barrier remains)"))
}

fn target(
    persistence: &SqlitePersistence,
    run_id: &RunId,
    id: &InvocationId,
) -> Result<WorkSlotInvocation, CliError> {
    let catalog_error =
        |error: loop_core::PersistenceError| CliError::new(error.code(), error.to_string());
    let run = persistence
        .load_authoritative_run(run_id)
        .map_err(catalog_error)?;
    let row = persistence
        .load_work_slot_invocation(run_id, id)
        .map_err(catalog_error)?
        .ok_or_else(|| CliError::new("invocation-not-found", "invocation is not on this run"))?;
    if !run.lifecycle.is_active() {
        return Err(CliError::new("run-not-active", "run is not active"));
    }
    if row.status.is_some() && !ownership::cleanup_pending(&row.capture_dir) {
        return Err(CliError::new(
            "invocation-not-running",
            "invocation already completed; no outstanding cancellation",
        ));
    }
    let subject = persistence
        .get_current_slot_subject(run_id, &row.slot_id)
        .map_err(catalog_error)?;
    if subject.as_deref() != Some(row.subject.as_str())
        || !run
            .workflow
            .work_slots
            .iter()
            .any(|slot| slot.id == row.slot_id && slot.state == run.current_state)
    {
        return Err(CliError::new(
            "invocation-not-current",
            "invocation does not belong to the current state visit",
        ));
    }
    let Some(owned) = row.ownership.as_ref() else {
        return Err(CliError::new(
            "ownership-unavailable",
            "unsupported historical or not-yet-published target; wait for ownership publication",
        ));
    };
    if !ownership::identity_available(&row.capture_dir).map_err(|error| {
        CliError::new(
            "ownership-unavailable",
            format!("could not read native ownership identity: {error}"),
        )
    })? {
        return Err(CliError::new(
            "ownership-unavailable",
            "recorded ownership has no usable native process incarnation",
        ));
    }
    let waiter_alive =
        ownership::process_identity_matches(row.waiter_pid, row.waiter_identity.as_ref())
            .unwrap_or(false);
    if !owned.live_owned_work && !owned.cleanup_pending && !waiter_alive {
        return Err(CliError::new(
            "invocation-not-running",
            "no owned work, waiter, or outstanding cancellation remains",
        ));
    }
    Ok(row)
}

fn control(
    persistence: &SqlitePersistence,
    run: &RunId,
    id: &InvocationId,
) -> Result<Value, CliError> {
    let row = target(persistence, run, id)?;
    let directory = ownership::directory(&row.capture_dir);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("controller.lock"))
        .map_err(failure)?;
    lock.try_lock().map_err(|error| {
        CliError::new(
            "cancellation-controller-active",
            format!("another controller owns this request: {error}"),
        )
    })?;
    let started = Instant::now(); // This clock is never reset by any phase.
    let acquired_at = now_timestamp();
    let deadline = started + LIMIT;
    let admission = Admission::acquire_until(&directory, deadline).map_err(failure)?;
    let row = target(persistence, run, id)?; // arbitrate natural completion under admission
    let attempt = fs::read_dir(&directory)
        .map_err(failure)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0)
        + 1;
    let name = format!("attempt-{attempt}.json");
    let mut record = json!({"attempt":attempt,"controller_pid":std::process::id(),
        "acquired_at":acquired_at,"outcome":"incomplete","deadline_ms":10000,"graceful_ms":3000,
        "progress_file":format!("control-{attempt}-progress.json")});
    // An admitted stop must never lack its interrupted-attempt record.
    admission.write(&name, &record).map_err(failure)?;
    admission
        .admit_stop(
            &json!({"run_id":run,"invocation_id":id,"requested_at":acquired_at,
        "deadline_ms_per_attempt":10000,"graceful_ms":3000}),
        )
        .map_err(failure)?;
    drop(admission);

    let cleanup = cleanup(&row, started, deadline, attempt);
    record["elapsed_ms"] = json!(started.elapsed().as_millis());
    match cleanup {
        Ok(receipt) => {
            record["cleanup"] = receipt;
            let admission = Admission::acquire_until(&directory, deadline).map_err(failure)?;
            admission
                .write("cleanup-verified.json", &record)
                .map_err(failure)?;
            drop(admission);
            if started.elapsed() >= LIMIT {
                return finish_error(
                    &directory,
                    &name,
                    record,
                    "deadline elapsed before acknowledgment",
                );
            }
            let acknowledgment = loop_core::CancellationAcknowledgment {
                invocation_id: id.clone(),
                attempt,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            if let Err(error) = persistence.acknowledge_cancellation(
                run,
                id,
                now_timestamp(),
                &acknowledgment,
                deadline,
            ) {
                return finish_error(&directory, &name, record, &error.to_string());
            }
            record["elapsed_ms"] = json!(started.elapsed().as_millis());
            record["outcome"] = json!("acknowledged");
            Admission::acquire_until(&directory, deadline)
                .map_err(failure)?
                .write(&name, &record)
                .map_err(failure)?;
            Ok(
                json!({"run_id":run,"invocation_id":id,"status":"failed","cancelled":true,
                "capture_dir":row.capture_dir,"attempt":record,"workflow_advanced":false}),
            )
        }
        Err(error) => finish_error(&directory, &name, record, &error.to_string()),
    }
}

fn finish_error(
    directory: &Path,
    name: &str,
    mut record: Value,
    error: &str,
) -> Result<Value, CliError> {
    record["outcome"] = json!("unverified");
    record["error"] = json!(error);
    // No admission is needed: controller lock exclusively owns attempt records.
    write_json(&directory.join(name), &record).map_err(failure)?;
    Err(failure(error))
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

#[derive(Clone, Debug, Serialize)]
struct Process {
    pid: u32,
    parent: u32,
    group: u32,
    state: String,
    identity: loop_core::ProcessIdentity,
}

impl From<loop_integrations::ownership::ProcessRecord> for Process {
    fn from(process: loop_integrations::ownership::ProcessRecord) -> Self {
        Self {
            pid: process.identity.pid,
            parent: process.parent_pid,
            group: process.process_group_id,
            state: process.state,
            identity: process.identity,
        }
    }
}

/// Keep the old diagnostic snapshot and its test-owned interruption point, but
/// never parse its shell-formatted text. Native process records below are the
/// sole authority for identity, ancestry, and group membership.
fn processes(directory: &Path, deadline: Instant, attempt: u64) -> io::Result<Vec<Process>> {
    let path = directory.join(format!("control-{attempt}-process-snapshot.txt"));
    let diagnostic = File::create(&path)?;
    match Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,stat="])
        .stdin(Stdio::null())
        .stdout(Stdio::from(diagnostic))
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(io::Error::other("process inventory diagnostic failed"));
                }
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other("process inventory deadline"));
            }
            thread::sleep(Duration::from_millis(5));
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    loop_integrations::ownership::read_processes()
        .map(|processes| processes.into_iter().map(Process::from).collect())
}

fn cleanup(
    row: &WorkSlotInvocation,
    started: Instant,
    deadline: Instant,
    attempt: u64,
) -> io::Result<Value> {
    let directory = ownership::directory(&row.capture_dir);
    let mut observed = BTreeMap::new();
    let mut signals = Vec::new();
    let mut stop: Option<Child> = None;
    let mut stop_exit = None;
    let mut stop_started = false;
    let mut term = BTreeSet::new();
    let mut kill = BTreeSet::new();
    loop {
        if Instant::now() >= deadline {
            if let Some(mut child) = stop {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(io::Error::other(
                "ten-second cancellation deadline; process disappearance/reaping unverified",
            ));
        }

        let snapshot = processes(&directory, deadline, attempt)?;
        let native = snapshot
            .iter()
            .map(|process| loop_integrations::ownership::ProcessRecord {
                identity: process.identity.clone(),
                parent_pid: process.parent,
                process_group_id: process.group,
                state: process.state.clone(),
            })
            .collect::<Vec<_>>();
        let owned = ownership::discover_owned_processes(&directory, &native)?;
        if !owned.identity_usable {
            return Err(io::Error::other(
                "recorded ownership has no usable native process identity",
            ));
        }
        let live = owned
            .live
            .iter()
            .filter_map(|native| {
                snapshot
                    .iter()
                    .find(|process| process.identity == native.identity)
                    .cloned()
            })
            .collect::<Vec<_>>();
        for process in &live {
            if !observed.contains_key(&process.identity) {
                write_json(
                    &directory.join(format!(
                        "observed-{}-{}.json",
                        process.pid, process.identity.start_time
                    )),
                    &json!({
                        "root_pid": process.pid,
                        "parent_pid": process.parent,
                        "process_group_id": process.group,
                        "identity": process.identity,
                        "observation": process,
                    }),
                )?;
            }
            observed.insert(process.identity.clone(), process.clone());
        }
        write_json(
            &directory.join(format!("control-{attempt}-progress.json")),
            &json!({"elapsed_ms":started.elapsed().as_millis(),"remaining":live,"signals":signals}),
        )?;

        if !stop_started {
            stop_started = true;
            if let Some(locator) = row
                .ownership
                .as_ref()
                .and_then(|o| o.execution.graph_locator.as_ref())
                .filter(|path| path.is_file())
            {
                let locator: crate::dagu::DaguLocator =
                    serde_json::from_slice(&fs::read(locator)?)?;
                // The launch already version-checked Dagu. Do not run an
                // unbounded second version probe inside this shutdown clock.
                let child = Command::new("dagu")
                    .args([
                        "stop",
                        "--dagu-home",
                        &locator.dagu_home,
                        "--run-id",
                        &locator.run_name,
                        &locator.dag_name,
                    ])
                    .stdin(Stdio::null())
                    .stdout(File::create(
                        directory.join(format!("control-{attempt}-dagu-stop.stdout")),
                    )?)
                    .stderr(File::create(
                        directory.join(format!("control-{attempt}-dagu-stop.stderr")),
                    )?)
                    .spawn()?;
                // The scheduler stop process is a controller helper, not
                // invocation-owned work. Keep its Child handle for bounded
                // wait/kill, but never make its caller process group an
                // ownership anchor.
                stop = Some(child);
            }
        }
        if let Some(child) = stop.as_mut() {
            if let Some(status) = child.try_wait()? {
                stop_exit = Some(crate::inner_waitpid_as_i32(status));
                stop = None;
            } else if started.elapsed() >= GRACE {
                child.kill()?;
                stop_exit = Some(crate::inner_waitpid_as_i32(child.wait()?));
                stop = None;
            }
        }
        if live.is_empty() && stop.is_none() {
            let mut known_identities = owned
                .records
                .iter()
                .map(|record| record.identity.clone())
                .collect::<BTreeSet<_>>();
            known_identities.extend(observed.keys().cloned());
            let verified_absent_pids = known_identities
                .iter()
                .filter(|identity| !snapshot.iter().any(|process| process.pid == identity.pid))
                .map(|identity| identity.pid)
                .collect::<BTreeSet<_>>();
            let verified_absent_identities = known_identities
                .iter()
                .filter(|identity| {
                    !snapshot
                        .iter()
                        .any(|process| process.identity == **identity)
                })
                .cloned()
                .collect::<Vec<_>>();
            return Ok(
                json!({"verified_no_survivors":true,"reaping":"all observed owned PIDs absent from native process inventory (including zombies)",
                "observed":observed.values().collect::<Vec<_>>(),"verified_absent_pids":verified_absent_pids,"verified_absent_identities":verified_absent_identities,"verified_empty_groups":owned.anchored_groups,"signals":signals,"dagu_stop_exit":stop_exit}),
            );
        }
        // Signal leaves first, leaving their parents alive to waitpid. As
        // children disappear the next sweep reaches their parents. A stopped
        // parent is continued so it can reap; no SIGCONT authorizes new work
        // because the durable admission marker is already present.
        let escalation = started.elapsed() >= GRACE;
        for process in &live {
            if stop.as_ref().is_some_and(|child| child.id() == process.pid) {
                continue;
            }
            if process.state.contains('T') {
                #[cfg(target_os = "macos")]
                let _ = ownership::signal_process(&process.identity, 19)?;
                #[cfg(not(target_os = "macos"))]
                let _ = ownership::signal_process(&process.identity, 18)?;
            }
            if live
                .iter()
                .any(|other| other.parent == process.pid && !other.state.starts_with('Z'))
            {
                continue;
            }
            let sent = if escalation { &mut kill } else { &mut term };
            if sent.insert(process.identity.clone()) {
                let signal = if escalation { 9 } else { 15 };
                if ownership::signal_process(&process.identity, signal)? {
                    signals.push(json!({
                        "pid": process.pid,
                        "identity": process.identity,
                        "signal": signal,
                        "elapsed_ms": started.elapsed().as_millis()
                    }));
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}
