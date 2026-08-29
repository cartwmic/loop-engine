#![allow(dead_code)]

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TestDir {
    pub path: PathBuf,
}

impl TestDir {
    pub fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "research-provider-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    pub fn root_value(&self) -> Value {
        json!(self.path.to_string_lossy().to_string())
    }

    pub fn write_json(&self, name: &str, value: &Value) {
        fs::write(
            self.path.join(name),
            serde_json::to_vec(value).expect("serialize JSON"),
        )
        .expect("write JSON fixture");
    }

    pub fn write_text(&self, name: &str, value: &str) {
        fs::write(self.path.join(name), value).expect("write text fixture");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn provider_binary() -> PathBuf {
    workspace_integration::binary("research")
}

pub fn invoke(request: Value) -> Output {
    let mut child = Command::new(provider_binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn research provider");
    let bytes = serde_json::to_vec(&request).expect("serialize provider request");
    child
        .stdin
        .take()
        .expect("provider stdin")
        .write_all(&bytes)
        .expect("write provider request");
    child.wait_with_output().expect("wait for provider")
}

pub fn response(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "provider stdout is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stderr={:?}",
        output.stderr
    );
}

pub fn transition(source: &str, event: &str, target: &str, kind: &str) -> Value {
    json!({
        "source": source,
        "event": event,
        "target": target,
        "kind": kind
    })
}

pub fn checked(source: &str, event: &str, target: &str) -> Value {
    transition(source, event, target, "checked")
}

pub fn base_request(initial_input: Value, transition: Value) -> Value {
    json!({
        "operation": "evaluate",
        "workflow": {
            "id": "research",
            "initial_state": "scope",
            "states": [],
            "transitions": []
        },
        "initial_input": initial_input,
        "context": [],
        "transition": transition,
        "prior_evaluations": []
    })
}

pub fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "author": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {
                        "type": "string",
                        "enum": ["human", "agent", "script"]
                    }
                },
                "required": ["name", "kind"],
                "additionalProperties": false
            }
        },
        "required": ["revision", "author"],
        "additionalProperties": false
    })
}

pub fn valid_metadata(revision: &str) -> Value {
    json!({
        "revision": revision,
        "author": {"name": "owner", "kind": "human"}
    })
}

pub fn context_record(data: Value) -> Value {
    json!({
        "id": "context-1",
        "kind": "review-evidence",
        "data": data,
        "sequence": 1,
        "created_at": 1
    })
}

#[allow(clippy::too_many_arguments)]
pub fn evidence(
    gate: &str,
    policy_id: &str,
    result: &str,
    findings: &str,
    author_name: &str,
    author_kind: &str,
    subject: &str,
    subject_revision: &str,
    config_version: &str,
) -> Value {
    json!({
        "gate": gate,
        "policy_id": policy_id,
        "result": result,
        "findings": findings,
        "author": {"name": author_name, "kind": author_kind},
        "subject": subject,
        "subject_revision": subject_revision,
        "config_version": config_version
    })
}

pub fn load_profile(profile: &str) -> Value {
    let path = workspace_integration::package_root("research-provider")
        .join("data")
        .join("configs")
        .join(format!("{profile}.json"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read shipped config {path:?}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("invalid shipped config: {error}"))
}
