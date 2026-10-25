//! Passive deterministic observation; external execution remains driver-owned.
use crate::visibility;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const HELP: &str = "Usage: loop-engine monitor --run ID [--database PATH] [--observation ABS] [--json] [--attention-seconds N] [--poll-seconds N]\nPassive non-arming workflow/history/context observation. External execution is unknown. Stopping observation never cancels work.\n";

#[derive(Default)]
struct Options {
    runs: Vec<String>,
    observations: Vec<PathBuf>,
    database: Option<String>,
    json: bool,
    attention: Option<f64>,
    poll: f64,
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
            "--run" => o.runs.push(v.clone()),
            "--observation" => o.observations.push(PathBuf::from(v)),
            "--database" => o.database = Some(v.clone()),
            "--attention-seconds" => o.attention = Some(number(v)?),
            "--poll-seconds" => o.poll = number(v)?,
            _ => return Err(format!("unknown option {key}")),
        }
        i += 1;
    }
    if o.runs.is_empty() {
        return Err("at least one explicit run required".into());
    }
    if o.observations.iter().any(|p| !p.is_absolute()) {
        return Err("observation paths must be absolute".into());
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
fn backend(o: &Options, tail: &[String]) -> Result<Value, String> {
    let mut argv = vec!["--json".to_owned()];
    if let Some(db) = &o.database {
        argv.extend(["--database".into(), db.clone()]);
    }
    argv.extend_from_slice(tail);
    let execution = crate::execute(argv);
    let v: Value = serde_json::from_str(&execution.stdout)
        .map_err(|e| format!("read invalid JSON: {e}; {}", execution.stderr))?;
    if execution.exit_code != 0 || v["status"] != "completed" {
        return Err(format!("read failed ({}): {v}", execution.exit_code));
    }
    Ok(v["result"].clone())
}
fn read_workflow(o: &Options, id: &str) -> Result<Value, String> {
    let options = crate::CliOptions {
        database: o.database.as_ref().map(PathBuf::from),
        ..Default::default()
    };
    let paths = crate::resolve_paths(&options).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let persistence = loop_integrations::SqlitePersistence::open_for_observation(
        &paths.database,
        deadline,
        false,
    )
    .map_err(|e| e.to_string())?;
    match loop_core::operations::show::execute(
        loop_core::ShowRequest::new(id),
        &persistence,
        crate::now_timestamp(),
        false,
    ) {
        loop_core::OperationOutcome::Completed(projection) => {
            let mut value = serde_json::to_value(crate::CliShowProjection::from(projection))
                .map_err(|e| e.to_string())?;
            value["mutation_armed"] = json!(false);
            Ok(value)
        }
        loop_core::OperationOutcome::Rejected(issue)
        | loop_core::OperationOutcome::Error(issue) => {
            Err(format!("{}: {}", issue.code, issue.message))
        }
    }
}

fn run_source(o: &Options, id: &str) -> Value {
    let mut p = packet(&format!("run:{id}"));
    let result = (|| -> Result<(), String> {
        let mut workflow = read_workflow(o, id)?;
        visibility::strip_observation_clocks(&mut workflow);
        p["workflow"] = workflow.clone();
        p["workflow_source"] =
            json!({"database":o.database,"interface":"passive durable projection"});
        let after = workflow["delta_sequence"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(20)
            .to_string();
        let mut history = backend(
            o,
            &[
                "read".into(),
                id.into(),
                "--kind".into(),
                "delta".into(),
                "--cursor".into(),
                after,
                "--limit".into(),
                "20".into(),
            ],
        )?;
        visibility::strip_observation_clocks(&mut history);
        p["judgment"] = json!({"state":"unknown","reason":"history is attributed historical evidence, not current semantic approval","history":history});
        let mut observations = Vec::new();
        for path in &o.observations {
            match read(path) {Ok(v) if v["run_id"]==id && v["sampled_at_ms"].is_u64() && v["attesting_driver"].as_str().is_some_and(|s|!s.is_empty()) => observations.push(json!({"source":path,"freshness":"dated driver observation; current applicability unknown","attribution":v})),Ok(_)=>observations.push(unknown(format!("{}: mismatched or incomplete dated observation",path.display()))),Err(e)=>observations.push(unknown(e))}
        }
        p["judgment"]["driver_observations"] = json!(observations);
        if matches!(workflow["lifecycle"].as_str(), Some("final" | "terminated")) {
            p["boundary"] = json!({"event":"completion","reason":"selected workflow terminal; not semantic approval"});
        }
        Ok(())
    })();
    if let Err(e) = result {
        p["diagnostics"] = json!([e]);
        attention(&mut p, "selected source is unavailable or needs inspection");
    }
    visibility::enrich_monitor_packet(&mut p);
    p["execution"] = unknown("external execution is driver-owned");
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
        let line = serde_json::to_string(&p).map_err(|e| e.to_string())?;
        let mut out = io::stdout().lock();
        if let Err(e) = writeln!(out, "{line}").and_then(|()| out.flush()) {
            // A reader that closed the pipe has all it wanted.
            if e.kind() == io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            return Err(e.to_string());
        }
    } else {
        eprintln!("{event}: {p}");
    }
    Ok(())
}
fn run(o: Options) -> Result<(), String> {
    let start = Instant::now();
    let mut previous = BTreeMap::new();
    let mut deadline_sources = BTreeSet::new();
    let mut boundaries = BTreeMap::new();
    loop {
        let packets = o.runs.iter().map(|r| run_source(&o, r)).collect::<Vec<_>>();
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

#[cfg(test)]
mod retirement_tests {
    use super::*;
    #[test]
    fn summary_executor_options_are_unknown() {
        for option in ["--summary-config", "--output-dir"] {
            let args = ["--run", "fixture", option, "/tmp/unused"].map(str::to_owned);
            assert!(parse(&args).err().unwrap().contains("unknown option"));
        }
    }
}
