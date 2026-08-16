#![allow(dead_code)]

use loop_core::{
    self as core, AppendContextResult, ContextRecord, EventResult, Lifecycle, OperationOutcome,
    Persistence, ProviderSelector, Run, ShowProjection, StateId, Timestamp,
};
use loop_integrations::{
    ConfiguredProviderResolver, ProviderConfiguration, ProviderDefinition, SqlitePersistence,
    SubprocessProviderGateway,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-provider-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn value(&self) -> Value {
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
    PathBuf::from(env!("CARGO_BIN_EXE_software-change"))
}

pub fn invoke(request: Value) -> Output {
    let mut child = Command::new(provider_binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn software-change provider");
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
            "id": "software-change",
            "initial_state": "explore",
            "states": [],
            "transitions": []
        },
        "initial_input": initial_input,
        "context": [],
        "transition": transition,
        "prior_evaluations": []
    })
}

pub fn context_json(kind: &str, data: Value, sequence: u64) -> Value {
    json!({
        "id": format!("context-{sequence}"),
        "kind": kind,
        "data": data,
        "sequence": sequence,
        "created_at": sequence as i64
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

#[allow(clippy::too_many_arguments)]
pub fn evidence_context(
    id: &str,
    gate: &str,
    policy_id: &str,
    result: &str,
    findings: &str,
    author_name: &str,
    author_kind: &str,
    subject: &str,
    subject_revision: &str,
    config_version: &str,
) -> ContextRecord {
    ContextRecord::new(
        id,
        "review-evidence",
        evidence(
            gate,
            policy_id,
            result,
            findings,
            author_name,
            author_kind,
            subject,
            subject_revision,
            config_version,
        ),
        0_u64.into(),
        Timestamp::from_unix_millis(0),
    )
}

pub fn load_profile(profile: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("configs")
        .join(format!("{profile}.json"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read shipped config {path:?}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("invalid shipped config: {error}"))
}

pub fn load_fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("calibration")
        .join("fixtures")
        .join(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read shipped fixture {path:?}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("invalid shipped fixture: {error}"))
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
        "author": {"name": "change-owner", "kind": "human"}
    })
}

pub fn axis_config(root: &TestDir, axis: &str) -> Value {
    json!({
        "config_version": "test-1",
        "artifact_root": root.value(),
        "review_policies": {
            "intent": [{"id": axis, "description": "test axis"}]
        },
        "artifact_schemas": {"intent.json": metadata_schema()}
    })
}

pub fn resolver_for(command: impl Into<String>) -> ConfiguredProviderResolver {
    let providers = BTreeMap::from([("software".to_owned(), ProviderDefinition::command(command))]);
    ConfiguredProviderResolver::new(ProviderConfiguration { providers })
}

pub struct Engine {
    database: PathBuf,
    resolver: ConfiguredProviderResolver,
    gateway: SubprocessProviderGateway,
}

impl Engine {
    pub fn new(database: impl Into<PathBuf>) -> Self {
        Self::with_command(database, provider_binary())
    }

    pub fn with_command(database: impl Into<PathBuf>, command: impl Into<PathBuf>) -> Self {
        let command = command.into();
        Self {
            database: database.into(),
            resolver: resolver_for(command.to_string_lossy().into_owned()),
            gateway: SubprocessProviderGateway::new(Duration::from_secs(2)),
        }
    }

    fn persistence(&self) -> SqlitePersistence {
        SqlitePersistence::open(&self.database).expect("open engine database")
    }

    pub fn start(
        &self,
        run_id: &str,
        initial_input: Value,
    ) -> OperationOutcome<core::CreateRunResult> {
        let persistence = self.persistence();
        let catalog_root = self
            .database
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        core::execute_start(
            core::StartRequest::new(
                run_id,
                ProviderSelector::from("software"),
                initial_input,
                None,
                Timestamp::from_unix_millis(1),
                catalog_root,
            ),
            &self.resolver,
            &self.gateway,
            &persistence,
        )
    }

    pub fn start_ok(&self, run_id: &str, initial_input: Value) -> Run {
        match self.start(run_id, initial_input) {
            OperationOutcome::Completed(result) => result.run,
            other => panic!("expected engine start to complete, got {other:?}"),
        }
    }

    pub fn show(&self, run_id: &str) -> ShowProjection {
        struct ShowProcess;

        impl core::WorkSlotProcess for ShowProcess {
            type Handle = ();

            fn waiter_alive(&self, _pid: u32) -> bool {
                false
            }

            fn spawn_wait_invocation(
                &self,
                _args: core::WaiterSpawnArgs,
            ) -> std::result::Result<core::StartedWaiter<()>, core::ProcessError> {
                Err(core::ProcessError::new(
                    "unsupported",
                    "show helper does not spawn waiters",
                ))
            }

            fn send_envelope_and_detach(
                &self,
                _waiter: core::StartedWaiter<()>,
                _envelope_json: &[u8],
            ) -> std::result::Result<(), core::ProcessError> {
                Err(core::ProcessError::new(
                    "unsupported",
                    "show helper does not spawn waiters",
                ))
            }
        }

        let persistence = self.persistence();
        match core::execute_show(
            core::ShowRequest::new(run_id),
            &persistence,
            &ShowProcess,
            Timestamp::from_unix_millis(1),
        ) {
            OperationOutcome::Completed(show) => show,
            other => panic!("expected engine show to complete, got {other:?}"),
        }
    }

    pub fn authoritative(&self, run_id: &str) -> Run {
        self.persistence()
            .load_authoritative_run(&run_id.into())
            .expect("load authoritative run")
    }

    pub fn event(&self, run_id: &str, event: &str) -> OperationOutcome<EventResult> {
        let persistence = self.persistence();
        core::execute_event(
            core::EventRequest::new(run_id, event),
            &self.gateway,
            &persistence,
        )
    }

    pub fn append(&self, run_id: &str, record: ContextRecord) -> ContextRecord {
        let persistence = self.persistence();
        let result: OperationOutcome<AppendContextResult> = core::execute_append(
            core::AppendRequest::new(
                run_id,
                record.id.clone(),
                record.kind.clone(),
                record.data.clone(),
                record.created_at,
            ),
            &persistence,
        );
        match result {
            OperationOutcome::Completed(result) => result.context,
            other => panic!("expected context append to complete, got {other:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_evidence(
        &self,
        run_id: &str,
        id: &str,
        gate: &str,
        policy_id: &str,
        result: &str,
        findings: &str,
        author_name: &str,
        author_kind: &str,
        subject: &str,
        subject_revision: &str,
        config_version: &str,
    ) {
        self.append(
            run_id,
            evidence_context(
                id,
                gate,
                policy_id,
                result,
                findings,
                author_name,
                author_kind,
                subject,
                subject_revision,
                config_version,
            ),
        );
    }

    pub fn current_state(&self, run_id: &str) -> StateId {
        self.authoritative(run_id).current_state
    }

    pub fn lifecycle(&self, run_id: &str) -> Lifecycle {
        self.authoritative(run_id).lifecycle
    }
}

pub fn config_artifact_root(mut config: Value, root: &TestDir) -> Value {
    config["artifact_root"] = root.value();
    config
}
