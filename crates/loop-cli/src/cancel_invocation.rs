//! The public cancel command is the cleanup controller, not a request queue.
//! One ten-second monotonic clock starts at controller-lock acquisition; an
//! interrupted attempt remains incomplete and the durable admission stop stays.
use crate::{now_timestamp, CliError};
use loop_core::{InvocationId, OperationOutcome, Persistence, RunId, WorkSlotInvocation};
use loop_integrations::{
    ownership::{self, Admission},
    SqlitePersistence,
};
use serde::{Deserialize, Serialize};
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
        .load_work_slot_invocations(run_id)
        .map_err(catalog_error)?
        .into_iter()
        .find(|row| row.invocation_id == *id)
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
    #[cfg(unix)]
    let waiter_alive = unsafe { crate::unix_signal::kill(row.waiter_pid as i32, 0) == 0 };
    #[cfg(not(unix))]
    let waiter_alive = false;
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Process {
    pid: i32,
    parent: i32,
    group: i32,
    state: String,
}

/// ps is available on both supported local platforms. A zombie remains in the
/// inventory until its actual parent/adopter reaps it: kill(0) alone is not a
/// cleanup receipt. The command itself is bounded by the same attempt clock.
fn processes(directory: &Path, deadline: Instant, attempt: u64) -> io::Result<Vec<Process>> {
    let path = directory.join(format!("control-{attempt}-process-snapshot.txt"));
    let mut child = Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,stat="])
        .stdin(Stdio::null())
        .stdout(File::create(&path)?)
        .stderr(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(io::Error::other("process inventory failed"));
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("process inventory deadline"));
        }
        thread::sleep(Duration::from_millis(5));
    }
    fs::read_to_string(path)?
        .lines()
        .map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let parse = |n: usize| {
                fields
                    .get(n)
                    .ok_or_else(|| io::Error::other("malformed ps row"))?
                    .parse()
                    .map_err(io::Error::other)
            };
            Ok(Process {
                pid: parse(0)?,
                parent: parse(1)?,
                group: parse(2)?,
                state: fields.get(3).unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

fn cleanup(
    row: &WorkSlotInvocation,
    started: Instant,
    deadline: Instant,
    attempt: u64,
) -> io::Result<Value> {
    let directory = ownership::directory(&row.capture_dir);
    let mut known = BTreeSet::new();
    let mut groups = BTreeSet::new();
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
        // Discover helper roots admitted before the marker, and recover roots
        // discovered by an interrupted prior controller after they reparented.
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "ownership.json"
                || name.starts_with("helper-")
                || name.starts_with("child-")
                || name.starts_with("observed-")
            {
                let value: Value = serde_json::from_slice(&fs::read(entry.path())?)?;
                if let Some(pid) = value["root_pid"].as_i64() {
                    known.insert(pid as i32);
                }
                if let Some(group) = value["process_group_id"].as_i64() {
                    groups.insert(group as i32);
                }
            }
        }
        let snapshot = processes(&directory, deadline, attempt)?;
        loop {
            let before = known.len();
            for process in &snapshot {
                if known.contains(&process.parent) || groups.contains(&process.group) {
                    known.insert(process.pid);
                }
            }
            if before == known.len() {
                break;
            }
        }
        let live = snapshot
            .iter()
            .filter(|process| known.contains(&process.pid))
            .collect::<Vec<_>>();
        for process in &live {
            if !observed.contains_key(&process.pid) {
                write_json(
                    &directory.join(format!("observed-{}.json", process.pid)),
                    &json!({"root_pid":process.pid,"observation":process}),
                )?;
            }
            observed.insert(process.pid, (*process).clone());
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
                stop = Some(
                    Command::new("dagu")
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
                        .spawn()?,
                );
                if let Some(child) = stop.as_ref() {
                    write_json(
                        &directory.join(format!("observed-stop-{attempt}.json")),
                        &json!({"root_pid":child.id()}),
                    )?;
                }
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
            return Ok(
                json!({"verified_no_survivors":true,"reaping":"all observed owned PIDs absent from OS process inventory (including zombies)",
                "observed":observed.values().collect::<Vec<_>>(),"verified_absent_pids":known,"verified_empty_groups":groups,"signals":signals,"dagu_stop_exit":stop_exit}),
            );
        }
        // Signal leaves first, leaving their parents alive to waitpid. As
        // children disappear the next sweep reaches their parents. A stopped
        // parent is continued so it can reap; no SIGCONT authorizes new work
        // because the durable admission marker is already present.
        let escalation = started.elapsed() >= GRACE;
        for process in &live {
            if stop
                .as_ref()
                .is_some_and(|child| child.id() == process.pid as u32)
            {
                continue;
            }
            if process.state.contains('T') {
                #[cfg(target_os = "macos")]
                signal_pid(process.pid, 19)?;
                #[cfg(not(target_os = "macos"))]
                signal_pid(process.pid, 18)?;
            }
            if live
                .iter()
                .any(|other| other.parent == process.pid && !other.state.starts_with('Z'))
            {
                continue;
            }
            let sent = if escalation { &mut kill } else { &mut term };
            if sent.insert(process.pid) {
                let signal = if escalation { 9 } else { 15 };
                signal_pid(process.pid, signal)?;
                signals.push(json!({"pid":process.pid,"signal":signal,"elapsed_ms":started.elapsed().as_millis()}));
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn signal_pid(pid: i32, signal: i32) -> io::Result<()> {
    if pid <= 1 || pid == std::process::id() as i32 {
        return Err(io::Error::other("invalid owned process identity"));
    }
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        if unsafe { kill(pid, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(3) {
                return Err(error);
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        Err(io::Error::other(
            "local cancellation unsupported on this platform",
        ))
    }
}
