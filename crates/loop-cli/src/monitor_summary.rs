//! Optional advisory subprocess; never participates in workflow decisions.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    executable: String,
    args: Vec<String>,
    timeout_seconds: f64,
    minimum_interval_seconds: f64,
    max_calls: u64,
}
#[derive(Default, Deserialize, Serialize)]
struct State {
    attempted_calls: u64,
    last_started: f64,
    digest: String,
    previous: Option<Value>,
}
struct Active {
    child: Child,
    start: Instant,
    directory: PathBuf,
    digest: String,
}
pub struct Summary {
    config: Config,
    root: PathBuf,
    state: State,
    active: Option<Active>,
    _lock: File,
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn load(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())
}
fn stable(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for key in [
                "observed_at",
                "sampled_at_ms",
                "elapsed_ms",
                "remaining_allowed_ms",
            ] {
                m.remove(key);
            }
            for v in m.values_mut() {
                stable(v);
            }
        }
        Value::Array(a) => {
            for v in a {
                stable(v);
            }
        }
        _ => {}
    }
}
fn valid(v: &Value) -> bool {
    let Some(m) = v.as_object() else {
        return false;
    };
    m.keys().all(|k| {
        matches!(
            k.as_str(),
            "developments" | "significance" | "uncertainty" | "corrections" | "usage" | "cost"
        )
    }) && ["developments", "significance", "uncertainty"]
        .iter()
        .all(|k| v[k].as_str().is_some_and(|s| !s.trim().is_empty()))
        && v["corrections"].as_array().is_some_and(|rows| {
            rows.iter().all(|r| {
                r.as_object().is_some_and(|m| m.len() == 2)
                    && ["prior_claim", "correcting_evidence"]
                        .iter()
                        .all(|k| r[k].as_str().is_some_and(|s| !s.trim().is_empty()))
            })
        })
        && ["usage", "cost"].iter().all(|k| {
            m.get(*k).is_none_or(|r| {
                r.as_object().is_some_and(|m| m.len() == 2)
                    && r["amount"]
                        .as_f64()
                        .is_some_and(|n| n.is_finite() && n >= 0.0)
                    && r["unit"].as_str().is_some_and(|s| !s.trim().is_empty())
            })
        })
}
impl Summary {
    pub fn open(config: &Path, root: &Path) -> Result<Self, String> {
        let config: Config = serde_json::from_value(load(config)?).map_err(|e| e.to_string())?;
        if config.executable.is_empty()
            || [config.timeout_seconds, config.minimum_interval_seconds]
                .iter()
                .any(|n| !n.is_finite() || *n <= 0.0 || Duration::try_from_secs_f64(*n).is_err())
        {
            return Err("invalid summary configuration".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("session.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|e| format!("summary session already in use: {e}"))?;
        let state = if root.join("session.json").exists() {
            serde_json::from_value(load(&root.join("session.json"))?).map_err(|e| e.to_string())?
        } else {
            State::default()
        };
        Ok(Self {
            config,
            root: root.into(),
            state,
            active: None,
            _lock: lock,
        })
    }
    fn status(&self, status: &str, detail: Value, digest: &str) -> Value {
        json!({"source":"advisory-summary","status":status,"detail":detail,"attempted_calls":self.state.attempted_calls,"max_calls":self.config.max_calls,"evidence_digest":digest,"previous_summary":self.state.previous,"previous_summary_label":if self.state.previous.as_ref().is_some_and(|p|p["evidence_digest"]==digest)&&status!="summary-failed" {"current advisory; truth unverified"}else{"older advisory; truth unverified"},"usage_cost":"only reported values in retained output; otherwise unknown"})
    }
    pub fn tick(&mut self, packets: &[Value]) -> Result<Value, String> {
        let mut evidence = json!(packets);
        stable(&mut evidence);
        let digest = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&evidence).unwrap())
        );
        if let Some(mut a) = self.active.take() {
            let mut exit = a.child.try_wait().map_err(|e| e.to_string())?;
            let timeout =
                exit.is_none() && a.start.elapsed().as_secs_f64() >= self.config.timeout_seconds;
            if timeout {
                let _ = a.child.kill();
                exit = Some(a.child.wait().map_err(|e| e.to_string())?);
            }
            if let Some(exit) = exit {
                let output = load(&a.directory.join("stdout"));
                let usable = !timeout && exit.success() && output.as_ref().is_ok_and(valid);
                let result = json!({"exit_code":exit.code(),"timed_out":timeout,"output_conformant":usable,"evidence_digest":a.digest});
                save(&a.directory.join("exit.json"), &result)?;
                if usable {
                    self.state.previous = Some(
                        json!({"output":output.unwrap(),"evidence_digest":a.digest,"attempt_directory":a.directory,"fallible":true}),
                    );
                    save(&self.root.join("session.json"), &self.state)?;
                }
                return Ok(self.status(
                    if usable {
                        "summary-usable"
                    } else {
                        "summary-failed"
                    },
                    result,
                    &digest,
                ));
            }
            self.active = Some(a);
            return Ok(self.status("summary-running", Value::Null, &digest));
        }
        if self.state.attempted_calls > 0
            && !self
                .root
                .join(format!(
                    "attempt-{:04}/exit.json",
                    self.state.attempted_calls
                ))
                .exists()
        {
            return Ok(self.status("summary-failed",json!({"reason":"interrupted attempt has no exit evidence; automatic summaries disabled, observation continues"}),&digest));
        }
        if self.state.attempted_calls >= self.config.max_calls {
            return Ok(self.status("summary-budget-exhausted", Value::Null, &digest));
        }
        if digest == self.state.digest {
            return Ok(self.status("summary-unchanged", Value::Null, &digest));
        }
        if now() - self.state.last_started < self.config.minimum_interval_seconds {
            return Ok(self.status("summary-cadence", Value::Null, &digest));
        }
        // Reserve before spawn: crashes and failures consume budget, never trigger free retries.
        self.state.attempted_calls += 1;
        self.state.last_started = now();
        self.state.digest = digest.clone();
        save(&self.root.join("session.json"), &self.state)?;
        let directory = self
            .root
            .join(format!("attempt-{:04}", self.state.attempted_calls));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let mut selected = Vec::new();
        let mut omitted = Vec::new();
        let mut size = 0;
        for p in evidence.as_array().unwrap() {
            let n = serde_json::to_vec(p).unwrap().len();
            if size + n <= 65536 {
                selected.push(p.clone());
                size += n;
            } else {
                omitted.push(p["source"].clone());
            }
        }
        let input = json!({"selected_sources":packets.iter().map(|p|p["source"].clone()).collect::<Vec<_>>(),"current_new_evidence":selected,"omitted_source_count":omitted.len(),"omitted_source_locators":omitted,"evidence_digest":digest,"previous_summary":self.state.previous,"instruction":"Previous summary is fallible prose, not evidence. Explain changes and explicitly correct prior claims using current evidence locators. Do not approve or control work."});
        save(&directory.join("stdin.json"), &input)?;
        save(
            &directory.join("command.json"),
            &json!({"executable":self.config.executable,"args":self.config.args,"cwd":std::env::current_dir().ok(),"timeout_seconds":self.config.timeout_seconds,"minimum_interval_seconds":self.config.minimum_interval_seconds,"max_calls":self.config.max_calls,"started_at":self.state.last_started}),
        )?;
        let spawn = Command::new(&self.config.executable)
            .args(&self.config.args)
            .stdin(Stdio::from(
                File::open(directory.join("stdin.json")).map_err(|e| e.to_string())?,
            ))
            .stdout(Stdio::from(
                File::create(directory.join("stdout")).map_err(|e| e.to_string())?,
            ))
            .stderr(Stdio::from(
                File::create(directory.join("stderr")).map_err(|e| e.to_string())?,
            ))
            .spawn();
        match spawn {
            Ok(child) => {
                self.active = Some(Active {
                    child,
                    start: Instant::now(),
                    directory,
                    digest: digest.clone(),
                });
                Ok(self.status("summary-running", Value::Null, &digest))
            }
            Err(e) => {
                let result =
                    json!({"spawn_error":e.to_string(),"exit_code":null,"output_conformant":false});
                save(&directory.join("exit.json"), &result)?;
                Ok(self.status("summary-failed", result, &digest))
            }
        }
    }
}
impl Drop for Summary {
    fn drop(&mut self) {
        if let Some(a) = self.active.as_mut() {
            let _ = a.child.kill();
            let _ = a.child.wait();
        }
    }
}
