//! Passive deterministic observation with an optional separate advisory subprocess.
#[path = "monitor_summary.rs"]
mod monitor_summary;
use crate::visibility;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const HELP: &str = "Usage: loop-engine monitor [--run ID]... [--capture-dir ABS]... [--invocation ID] [--engine ABS] [--database PATH] [--observation ABS]... [--json] [--attention-seconds N] [--poll-seconds N] [--summary-config FILE --output-dir ABS]\nOptional external summaries are advisory; config/call failure never stops observation.\nNative reads use non-arming status. --engine selects released compatibility reads (list/history/invocation-progress only). At least one source is required. --invocation requires exactly one run. JSON mode emits flushed JSONL snapshots and source-identified completion/attention events; inspect workflow_lane, execution, worker, conformance, acceptance, evidence, freshness and uncertainty separately. An explicit attributed observation judgment with result accepted/rejected and evidence locators may populate acceptance; opaque, stale, conflicting, or mismatched judgments remain unknown. owner_update is assistant-owned active-chat guidance, not a notification channel. Human output is stderr. Both follow until interrupted. Attention deadline is observer elapsed time, not ETA. Restart rereads durable evidence; no exactly-once promise. Stopping this observer never cancels work. Unknown judgment is not approval.\n";

#[derive(Default)]
struct Options {
    runs: Vec<String>,
    captures: Vec<PathBuf>,
    observations: Vec<PathBuf>,
    invocation: Option<String>,
    engine: Option<PathBuf>,
    database: Option<String>,
    json: bool,
    attention: Option<f64>,
    poll: f64,
    summary_config: Option<PathBuf>,
    output_dir: Option<PathBuf>,
}

pub fn cli(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some("monitor") {
        return None;
    }
    if args.iter().any(|s| s == "--help" || s == "-h") {
        print!("{HELP}");
        return Some(0);
    }
    Some(match parse(&args[1..]).and_then(run) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("monitor: {e}");
            20
        }
    })
}
fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        poll: 1.0,
        ..Options::default()
    };
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        if key == "--json" {
            o.json = true;
            i += 1;
            continue;
        }
        i += 1;
        let v = args
            .get(i)
            .ok_or_else(|| format!("missing value for {key}"))?;
        match key.as_str() {
            "--summary-config" => o.summary_config = Some(PathBuf::from(v)),
            "--output-dir" => o.output_dir = Some(PathBuf::from(v)),
            "--run" => o.runs.push(v.clone()),
            "--capture-dir" => o.captures.push(PathBuf::from(v)),
            "--observation" => o.observations.push(PathBuf::from(v)),
            "--invocation" => o.invocation = Some(v.clone()),
            "--engine" => o.engine = Some(PathBuf::from(v)),
            "--database" => o.database = Some(v.clone()),
            "--attention-seconds" => o.attention = Some(number(v)?),
            "--poll-seconds" => o.poll = number(v)?,
            _ => return Err(format!("unknown option {key}")),
        }
        i += 1;
    }
    if o.runs.is_empty() && o.captures.is_empty() {
        return Err("at least one explicit source required".into());
    }
    if o.invocation.is_some() && o.runs.len() != 1 {
        return Err("--invocation requires exactly one --run".into());
    }
    if o.captures
        .iter()
        .chain(o.observations.iter())
        .any(|p| !p.is_absolute())
        || o.engine.as_ref().is_some_and(|p| !p.is_absolute())
    {
        return Err("capture and engine paths must be absolute".into());
    }
    Ok(o)
}
fn number(v: &str) -> Result<f64, String> {
    let n = v.parse::<f64>().map_err(|e| e.to_string())?;
    if !n.is_finite() || n <= 0.0 || Duration::try_from_secs_f64(n).is_err() {
        Err("seconds must be finite and positive".into())
    } else {
        Ok(n)
    }
}
fn read(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
fn unknown(reason: impl ToString) -> Value {
    json!({"state":"unknown","reason":reason.to_string()})
}
fn packet(source: &str) -> Value {
    json!({"source":source,"workflow":unknown("not a run source"),"helper":unknown("no helper evidence"),"worker":unknown("no selected worker evidence"),"conformance":unknown("no conformance evidence"),"judgment":unknown("no attributed judgment supplied"),"diagnostics":[],"boundary":null})
}
fn attention(p: &mut Value, reason: &str) {
    p["boundary"] = json!({"event":"attention","reason":reason});
}
fn capture(root: &Path) -> Value {
    let mut p = packet(&format!("capture:{}", root.display()));
    if root.join("index.json").exists() || root.join("state.json").exists() {
        let result = (|| -> Result<(), String> {
            let index = read(&root.join("index.json"))?;
            let state = read(&root.join("state.json"))?;
            p["helper"] = state.clone();
            let selections = index["receipts"]
                .as_array()
                .ok_or("missing receipt selections")?;
            let mut workers = Vec::new();
            let mut failed = false;
            for s in selections {
                let path = s["receipt"].as_str().ok_or("missing receipt path")?;
                let r = read(Path::new(path))?;
                if r["id"] != s["id"] {
                    return Err("conflicting receipt identity".into());
                }
                for stream in ["stdout", "stderr"] {
                    let relative = r[stream].as_str().ok_or("missing stream path")?;
                    let bytes = fs::read(
                        Path::new(path)
                            .parent()
                            .ok_or("receipt parent missing")?
                            .join(relative),
                    )
                    .map_err(|e| e.to_string())?;
                    let digest = format!("{:x}", Sha256::digest(bytes));
                    if r[format!("{stream}_sha256")] != digest {
                        return Err("stale/conflicting stream digest".into());
                    }
                }
                if r["cleanup"] != "complete" {
                    return Err("unresolved cleanup".into());
                }
                if !r["exit_code"].is_i64() && r["signal"].is_null() && r["spawn_error"].is_null() {
                    return Err("partial receipt".into());
                }
                failed |= r["exit_code"] != 0
                    || r["timed_out"] != false
                    || r["aborted"] != false
                    || !r["signal"].is_null()
                    || !r["spawn_error"].is_null()
                    || !r["capture_error"].is_null();
                workers.push(json!({"source":path,"receipt":r}));
            }
            p["worker"] = json!(workers);
            if state["cleanup"] != "complete" {
                attention(&mut p, "unresolved cleanup");
            } else if failed || state["status"] == "failed" {
                attention(&mut p, "selected execution failed");
            } else if state["status"] == "completed" {
                let count = state["row_count"].as_u64().ok_or("missing row count")?;
                if count == 0
                    || state["selected_count"] != count
                    || selections.len() as u64 != count
                {
                    return Err("conflicting/partial completed matrix".into());
                }
                p["boundary"] = json!({"event":"completion","reason":"all selected capture rows completed; execution only"});
            }
            if read(&root.join("state.json"))? != state || read(&root.join("index.json"))? != index
            {
                return Err("conflicting reads; source changed during sampling".into());
            }
            Ok(())
        })();
        if let Err(e) = result {
            p["diagnostics"] = json!([e]);
            attention(&mut p, "missing/partial/conflicting capture evidence");
        }
    } else {
        match read(&root.join("summary.json")) {
            Ok(mut summary) => {
                let inventory = summary["expected_assignment_ids"].as_array().map(|ids| {
                    json!({"workers":ids.iter().map(|id| json!({"assignment_id":id})).collect::<Vec<_>>()})
                });
                if let Some(auxiliary) = summary["auxiliary_workers"].as_array().cloned() {
                    if let Some(workers) = summary["workers"].as_array_mut() {
                        workers.extend(auxiliary);
                    }
                }
                p["worker"] = summary["workers"].clone();
                // Summary presence or helper exit alone never establishes complete graph coverage.
                p["conformance"] = json!({"source":root.join("summary.json"),"workers":summary["workers"],"meaning":"declared mechanical output checks only; absent nodes unknown"});
                if let Some(rows) = summary["workers"].as_array() {
                    if rows.iter().any(|w| {
                        w["exit_code"].as_i64().is_some_and(|n| n != 0) || w["status"] == "failed"
                    }) {
                        attention(&mut p, "worker execution/conformance failure");
                    }
                }
                p["diagnostics"]=json!(["graph coverage and absent nodes unknown; summary alone is not workload completion"]);
                if let Some(spec) = inventory.or_else(|| read(&root.join("fan-out-spec.json")).ok())
                {
                    if let (Some(expected), Some(actual)) =
                        (spec["workers"].as_array(), summary["workers"].as_array())
                    {
                        let complete = !expected.is_empty()
                            && expected.len() == actual.len()
                            && expected.iter().all(|e| {
                                actual
                                    .iter()
                                    .filter(|a| a["assignment_id"] == e["assignment_id"])
                                    .count()
                                    == 1
                            })
                            && actual.iter().all(|a| {
                                a["exit_code"] == 0
                                    && (a["status"].is_null() || a["status"] == "succeeded")
                            });
                        let intact = actual.iter().all(|a| {
                            let Some(path) = a["selected_output_path"]
                                .as_str()
                                .or_else(|| a["stdout_path"].as_str())
                            else {
                                return false;
                            };
                            let Ok(bytes) = fs::read(root.join(path)) else {
                                return false;
                            };
                            a["selected_output_sha256"].as_str().is_none_or(|digest| {
                                digest.strip_prefix("sha256:").unwrap_or(digest)
                                    == format!("{:x}", Sha256::digest(bytes))
                            })
                        });
                        let conformance = expected.iter().all(|e| {
                            (e["output_schema"].is_null() && e["full_output_schema"].is_null())
                                || actual.iter().any(|a| {
                                    a["assignment_id"] == e["assignment_id"]
                                        && a["status"] == "succeeded"
                                })
                        });
                        if !intact {
                            attention(&mut p, "missing/stale graph output evidence");
                        }
                        if complete && intact && conformance {
                            p["diagnostics"] = json!([]);
                            p["boundary"] = json!({"event":"completion","reason":"all declared workers completed with intact output; execution/mechanical checks only"});
                        }
                    }
                }
            }
            Err(e) => {
                // A regular external command may not produce a fan-out
                // summary. Keep its execution completion separate from the
                // unknown worker/conformance lanes; graph callers still use
                // the explicit inventory check below to raise attention.
                p["diagnostics"] = json!([e]);
            }
        }
    }
    visibility::enrich_monitor_packet(&mut p);
    p
}
fn backend(o: &Options, tail: &[String]) -> Result<Value, String> {
    let mut argv = vec!["--json".to_owned()];
    if let Some(db) = &o.database {
        argv.extend(["--database".into(), db.clone()]);
    }
    argv.extend_from_slice(tail);
    use std::io::Read;
    use std::process::Stdio;
    let engine = o
        .engine
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(engine)
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let second = send.clone();
    std::thread::spawn(move || {
        let mut b = String::new();
        let r = stdout_pipe.read_to_string(&mut b);
        let _ = send.send((true, r.map(|_| b)));
    });
    std::thread::spawn(move || {
        let mut b = String::new();
        let r = stderr_pipe.read_to_string(&mut b);
        let _ = second.send((false, r.map(|_| b)));
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("read backend exceeded five seconds; observed work not cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    for _ in 0..2 {
        let (is_stdout, value) = receive
            .recv_timeout(Duration::from_secs(1))
            .map_err(|e| e.to_string())?;
        if is_stdout {
            stdout = value.map_err(|e| e.to_string())?;
        } else {
            stderr = value.map_err(|e| e.to_string())?;
        }
    }
    let code = status.code().unwrap_or(20);
    let v: Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("backend invalid JSON: {e}; {stderr}"))?;
    if code != 0 || v["status"] != "completed" {
        return Err(format!("backend read failed ({code}): {v}"));
    }
    Ok(v["result"].clone())
}
fn run_source(o: &Options, id: &str) -> Value {
    let mut p = packet(&format!("run:{id}"));
    let result = (|| -> Result<(), String> {
        let workflow = if o.engine.is_some() {
            let list = backend(o, &["list".into()])?;
            let rows = list
                .as_array()
                .or_else(|| list["runs"].as_array())
                .ok_or("missing run list")?;
            rows.iter()
                .find(|r| r["run_id"] == id || r["id"] == id)
                .cloned()
                .ok_or("selected run missing")?
        } else {
            backend(
                o,
                &["show".into(), id.into(), "--view".into(), "status".into()],
            )?
        };
        let mut workflow = workflow;
        visibility::strip_observation_clocks(&mut workflow);
        p["workflow"] = workflow.clone();
        p["workflow_source"] = json!({"engine":o.engine,"database":o.database,"interface":if o.engine.is_some(){"list"}else{"show --view status"}});
        let history = backend(o, &["history".into(), id.into()])?;
        p["judgment"] = json!({"state":"unknown","reason":"history is attributed historical evidence, not current semantic approval","history":history});
        let mut observations = Vec::new();
        for path in &o.observations {
            match read(path) {Ok(v) if v["run_id"]==id && v["sampled_at_ms"].is_u64() && v["attesting_driver"].as_str().is_some_and(|s|!s.is_empty()) => observations.push(json!({"source":path,"freshness":"dated driver observation; current applicability unknown","attribution":v})),Ok(_)=>observations.push(unknown(format!("{}: mismatched or incomplete dated observation",path.display()))),Err(e)=>observations.push(unknown(e))}
        }
        p["judgment"]["driver_observations"] = json!(observations);
        let mut args = vec!["invocation-progress".into(), id.into()];
        if let Some(inv) = &o.invocation {
            args.push(inv.clone());
        }
        match backend(o, &args) {
            Ok(mut progress) => {
                if let Some(steps) = progress
                    .pointer_mut("/graph/steps")
                    .and_then(Value::as_array_mut)
                {
                    for step in steps {
                        if step["state"] == "not_started" {
                            step["state"] = json!("unknown");
                            step["reason"] = json!(
                                "backend does not distinguish absent node from not-started helper"
                            );
                        }
                    }
                }
                let selected = progress["invocation_id"].as_str();
                if let Some(rows) = history.as_array() {
                    if let Some(entry) = rows.iter().rev().find(|r| {
                        r["action"]["kind"] == "invocation_status_changed"
                            && r["action"]["invocation_id"].as_str() == selected
                            && selected.is_some()
                    }) {
                        p["execution_status_source"] = entry.clone();
                        match entry["action"]["status"].as_str() {
                            Some("succeeded") => {
                                p["boundary"] = json!({"event":"completion","reason":"selected invocation execution completed; not semantic approval"})
                            }
                            Some("failed") => {
                                attention(&mut p, "selected invocation execution failed")
                            }
                            _ => {}
                        }
                    }
                }
                p["helper"] = progress.clone();
                if let Some(dir) = progress["capture_dir"].as_str().filter(|s| {
                    !s.is_empty()
                        && (o.engine.is_none() || o.captures.iter().any(|p| p == Path::new(s)))
                }) {
                    let cp = capture(Path::new(dir));
                    p["worker"] = cp["worker"].clone();
                    p["conformance"] = cp["conformance"].clone();
                    p["diagnostics"] = cp["diagnostics"].clone();
                    if cp["boundary"]["event"] == "attention" {
                        p["boundary"] = cp["boundary"].clone();
                    }
                    if progress["graph"].is_object()
                        && cp["boundary"].is_null()
                        && p["boundary"]["event"] == "completion"
                    {
                        attention(&mut p, "selected helper completed but graph worker coverage/conformance is unknown");
                    }
                    if progress["ownership"]["cleanup_pending"] == true {
                        attention(&mut p, "invocation cleanup requires attention");
                    }
                }
            }
            Err(e) => p["diagnostics"] = json!([e]),
        }
        if let Some(rows) = workflow["work_slot_invocations"].as_array() {
            for row in rows.iter().filter(|r| {
                o.invocation
                    .as_ref()
                    .is_none_or(|id| r["invocation_id"] == *id)
            }) {
                if row["ownership"]["cleanup_pending"] == true {
                    attention(&mut p, "unresolved invocation cleanup");
                } else if row["status"] == "overrun" {
                    attention(
                        &mut p,
                        "invocation allowance exceeded; no automatic cancellation",
                    );
                } else if row["status"] == "running"
                    && row["invocation_id"] == p["helper"]["invocation_id"]
                    && p["boundary"]["event"] == "completion"
                {
                    attention(
                        &mut p,
                        "conflicting active status and completed history; resample",
                    );
                }
            }
        }
        if o.invocation.is_none()
            && p["boundary"].is_null()
            && matches!(
                workflow["lifecycle"].as_str(),
                Some("completed" | "completed-with-overrides" | "terminated")
            )
        {
            p["boundary"] = json!({"event":"completion","reason":"selected workflow terminal; not semantic approval"});
        }
        Ok(())
    })();
    if let Err(e) = result {
        p["diagnostics"] = json!([e]);
        attention(&mut p, "selected source is unavailable or needs inspection");
    }
    visibility::enrich_monitor_packet(&mut p);
    p
}
fn emit(o: &Options, mut p: Value, event: &str) -> Result<(), String> {
    p["event"] = json!(event);
    let sampled_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    p["sampled_at_ms"] = json!(sampled_at_ms);
    visibility::add_rendered_freshness(&mut p, sampled_at_ms);
    if o.json {
        let mut out = io::stdout().lock();
        serde_json::to_writer(&mut out, &p).map_err(|e| e.to_string())?;
        writeln!(out)
            .and_then(|()| out.flush())
            .map_err(|e| e.to_string())?;
    } else {
        eprintln!("{event}: {p}");
    }
    Ok(())
}
fn advisory_summary_failure(detail: impl Into<String>, summaries_disabled: bool) -> Value {
    json!({
        "source": "advisory-summary",
        "status": "summary-failed",
        "detail": detail.into(),
        "summaries_disabled": summaries_disabled,
        "evidence": {"state":"unknown","source_locators":[]},
        "freshness": {"state":"unknown","meaning":"no usable advisory input was retained"},
        "uncertainty": {"state":"present","reasons":["advisory summary is unavailable; deterministic observation continues"]}
    })
}

fn run(o: Options) -> Result<(), String> {
    let start = Instant::now();
    let mut previous = BTreeMap::new();
    let mut deadline_sources = BTreeSet::new();
    let mut boundaries = BTreeMap::new();
    let mut summary = match (&o.summary_config, &o.output_dir) {
        (None, _) => None,
        (Some(config), Some(root)) if root.is_absolute() => {
            match monitor_summary::Summary::open(config, root) {
                Ok(s) => Some(s),
                Err(e) => {
                    emit(&o, advisory_summary_failure(e, false), "summary")?;
                    None
                }
            }
        }
        _ => {
            emit(
                &o,
                advisory_summary_failure("summaries require --output-dir ABS", false),
                "summary",
            )?;
            None
        }
    };
    let mut last_summary = Value::Null;
    loop {
        let packets = o
            .runs
            .iter()
            .map(|r| run_source(&o, r))
            .chain(o.captures.iter().map(|p| capture(p)))
            .collect::<Vec<_>>();
        if let Some(s) = summary.as_mut() {
            let status = s.tick(&packets);
            match status {
                Ok(p) => {
                    if p != last_summary {
                        emit(&o, p.clone(), "summary")?;
                        last_summary = p;
                    }
                }
                Err(e) => {
                    emit(&o, advisory_summary_failure(e, true), "summary")?;
                    summary = None;
                }
            }
        }
        for p in packets {
            let key = p["source"].as_str().unwrap().to_owned();
            let changed = previous.get(&key) != Some(&p);
            let mut rendered = p.clone();
            rendered["owner_update"] = visibility::owner_update(previous.get(&key), &p);
            if changed {
                emit(&o, rendered.clone(), "snapshot")?;
                let identity = json!([
                    p["boundary"],
                    p["helper"]["invocation_id"],
                    p["execution_status_source"],
                    p["helper"]["attempt"]
                ]);
                if boundaries.get(&key) != Some(&identity) {
                    if let Some(event) = p["boundary"]["event"].as_str() {
                        emit(&o, rendered.clone(), event)?;
                    }
                    boundaries.insert(key.clone(), identity);
                }
                previous.insert(key.clone(), p.clone());
            }
            if o.attention
                .is_some_and(|n| start.elapsed().as_secs_f64() >= n)
                && deadline_sources.insert(key)
            {
                let mut timed = rendered;
                attention(&mut timed, "configured observer attention deadline reached");
                timed["owner_update"] = json!({
                    "required_before_next_decision": true,
                    "channel": "active Pi conversation",
                    "observed_change": "observer attention deadline reached",
                    "needed_action_or_decision": timed["next_action"],
                    "before_next_decision": ["wait", "inspect", "help"],
                    "machine_notification_is_not_owner_update": true
                });
                emit(&o, timed, "attention")?;
            }
        }
        std::thread::sleep(Duration::from_secs_f64(o.poll));
    }
}
