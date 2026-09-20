//! T10 public regression coverage for provider command discovery.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn t10_temp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary root")
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON bytes")).expect("write JSON");
}

fn t10_engine() -> PathBuf {
    workspace_integration::binary("loop-engine")
}

fn t10_provider() -> PathBuf {
    workspace_integration::binary("software-change")
}

fn engine_json(database: &Path, cwd: &Path, args: &[String]) -> Value {
    let output = Command::new(t10_engine())
        .current_dir(cwd)
        .args(["--database", database.to_str().expect("database"), "--json"])
        .args(args)
        .output()
        .expect("engine process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "engine output is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "engine command failed: args={args:?}, value={value}, stderr={:?}",
        output.stderr
    );
    value
}

fn engine_output(database: &Path, cwd: &Path, args: &[String]) -> (Value, Output) {
    let output = Command::new(t10_engine())
        .current_dir(cwd)
        .args(["--database", database.to_str().expect("database"), "--json"])
        .args(args)
        .output()
        .expect("engine process");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "engine output is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (value, output)
}

fn provider_output(cwd: &Path, args: &[String], input: &Value) -> Output {
    let mut command = Command::new(t10_provider());
    command.current_dir(cwd).args(args);
    command.stdin(Stdio::piped());
    super::bounded_process::run_with_stdin(
        &mut command,
        "software-change backlog_t10 validation",
        &serde_json::to_vec(input).expect("provider input JSON"),
    )
    .expect("provider process")
    .output
}

fn provider_json(cwd: &Path, args: &[String], input: &Value) -> Value {
    let output = provider_output(cwd, args, input);
    assert_eq!(
        output.status.code(),
        Some(0),
        "provider stderr: {:?}",
        output.stderr
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "provider output is not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn t10_show(database: &Path, cwd: &Path, run_id: &str) -> Value {
    engine_json(
        database,
        cwd,
        &["show".into(), "--view".into(), "full".into(), run_id.into()],
    )
}

fn t10_event(database: &Path, cwd: &Path, run_id: &str, event: &str) -> Value {
    let _ = t10_show(database, cwd, run_id);
    engine_json(
        database,
        cwd,
        &["event".into(), run_id.into(), event.into()],
    )
}

fn t10_checkpoint(artifact_root: &Path, repository: &Path, phase: &str) {
    let output = Command::new(t10_provider())
        .current_dir(repository)
        .args([
            "checkpoint",
            "--phase",
            phase,
            "--artifact-root",
            artifact_root.to_str().expect("artifact root"),
            "--working-directory",
            repository.to_str().expect("repository"),
        ])
        .output()
        .expect("checkpoint process");
    assert!(
        output.status.success(),
        "{phase} checkpoint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn t10_append(
    database: &Path,
    cwd: &Path,
    run_id: &str,
    kind: &str,
    record_id: &str,
    data: &Value,
) -> Value {
    let _ = t10_show(database, cwd, run_id);
    let (value, output) = engine_output(
        database,
        cwd,
        &[
            "append".into(),
            run_id.into(),
            "--kind".into(),
            kind.into(),
            "--record-id".into(),
            record_id.into(),
            serde_json::to_string(data).expect("record JSON"),
        ],
    );
    assert!(
        output.status.success() && value["status"] == "completed",
        "append failed: {value}; stderr={:?}",
        output.stderr
    );
    value
}

fn t10_profile(artifact_root: &Path) -> Value {
    let schema_path = workspace_integration::package_root("software-change-provider")
        .join("data/validation-report-schema.json");
    let schema: Value =
        serde_json::from_slice(&fs::read(schema_path).expect("validation report schema"))
            .expect("validation report schema JSON");
    json!({
        "contract_version": 3,
        "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
        "config_version": "backlog-t10-validation",
        "artifact_root": artifact_root.to_string_lossy(),
        "review_policies": {
            "validation-review": [{"id": "delivery", "description": "delivery", "review_stage": "aggregate", "required_authors": 1}],
            "validation-adversarial-review": [{"id": "delivery", "description": "challenge", "review_stage": "aggregate", "required_authors": 1}]
        },
        "artifact_schemas": {"validation-report.json": schema}
    })
}

fn t10_repository(root: &Path) -> PathBuf {
    let repository = root.join("repository");
    fs::create_dir_all(&repository).expect("repository");
    fs::write(repository.join("marker.txt"), "baseline\\n").expect("marker");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "backlog_t10"],
        vec!["config", "user.email", "backlog_t10@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(Command::new("git")
            .current_dir(&repository)
            .args(args)
            .status()
            .expect("git process")
            .success());
    }
    fs::canonicalize(repository).expect("canonical repository")
}

fn t10_context_record(show: &mut Value, id: &str, data: Value) {
    let context = show["result"]["context"]
        .as_array_mut()
        .expect("show context");
    let sequence = context
        .iter()
        .filter_map(|record| record["sequence"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    context.push(json!({
        "id": id,
        "kind": "validation-command",
        "data": data,
        "sequence": sequence,
        "created_at": sequence
    }));
}

#[test]
fn backlog_t10_run_plan_graph_rejects_multiline_task_worker_before_dagu() {
    let working_directory = std::env::current_dir().expect("current directory");
    let marker = std::env::temp_dir().join(format!(
        "software-change-backlog-t10-task-worker-marker-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&marker);
    let worker = json!({
        "command": "/bin/sh",
        "args": ["-c", format!("touch '{}'\n", marker.display())]
    })
    .to_string();

    let output = Command::new(workspace_integration::binary("software-change"))
        .args([
            "run-plan-graph",
            "--working-directory",
            working_directory.to_str().expect("working directory UTF-8"),
            "--task-worker",
            &worker,
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run-plan-graph process");

    assert_eq!(
        output.status.code(),
        Some(2),
        "unexpected output: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line break"), "unexpected stderr: {stderr}");
    assert!(!stderr.contains("dagu start"), "Dagu was reached: {stderr}");
    assert!(!marker.exists(), "malformed task worker was started");
    let _ = fs::remove_file(&marker);
}

#[test]
fn backlog_t10_prepare_validation_addition_is_publicly_appendable_and_collected() {
    let root = t10_temp_root();
    let repository = t10_repository(root.path());
    let artifact_root = root.path().join("artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact root");

    let profile = root.path().join("profile.json");
    write_json(&profile, &t10_profile(&artifact_root));
    let providers = root.path().join("providers.toml");
    fs::write(
        &providers,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            t10_provider().to_string_lossy()
        ),
    )
    .expect("provider catalog");
    let database = root.path().join("run.sqlite");
    let run_id = "backlog-t10-validation-envelope";
    let started = engine_json(
        &database,
        &repository,
        &[
            "--config".into(),
            providers.to_string_lossy().into_owned(),
            "start".into(),
            "--id".into(),
            run_id.into(),
            "software-change".into(),
            format!("@{}", profile.display()),
            "backlog t10 validation envelope".into(),
        ],
    );
    let actual_artifact_root = PathBuf::from(
        started["result"]["run"]["initial_input"]["artifact_root"]
            .as_str()
            .expect("artifact root"),
    );
    assert_eq!(actual_artifact_root, artifact_root);
    fs::create_dir_all(&actual_artifact_root).expect("allocated artifact root");

    let marker = root.path().join("extra-command-runs.txt");
    let required = json!({
        "id": "required-proof",
        "command": "python3",
        "args": ["-c", "print('backlog_t10 required proof')"],
        "owner": "driver",
        "obligation": "the required proof command succeeds"
    });
    let extra_script = "from pathlib import Path; import sys; Path(sys.argv[1]).open('a', encoding='utf-8').write('extra\\n'); print('backlog_t10 extra proof')";
    let extra = json!({
        "id": "extra-proof",
        "command": "python3",
        "args": ["-c", extra_script, marker.to_string_lossy()],
        "owner": "driver",
        "obligation": "the supplemental scripted proof succeeds"
    });
    write_json(
        &actual_artifact_root.join("intent.json"),
        &json!({
            "revision": "intent-1",
            "author": {"name": "owner", "kind": "human"},
            "acceptance": [{"id": "AC-1", "statement": "the named proof succeeds"}]
        }),
    );
    write_json(
        &actual_artifact_root.join("design.json"),
        &json!({"revision": "design-1", "author": {"name": "owner", "kind": "human"}}),
    );
    write_json(
        &actual_artifact_root.join("plan.json"),
        &json!({
            "revision": "plan-1",
            "author": {"name": "owner", "kind": "human"},
            "tasks": [],
            "proof_commands": [required]
        }),
    );
    write_json(
        &actual_artifact_root.join("implementation-report.json"),
        &json!({"revision": "implementation-1", "author": {"name": "owner", "kind": "human"}}),
    );
    write_json(
        &actual_artifact_root.join("reconciliation.json"),
        &super::reconciliation_fixture(),
    );

    for event in ["intent-ready", "design-ready", "plan-ready"] {
        assert_eq!(
            t10_event(&database, &repository, run_id, event)["status"],
            "completed",
            "{event}"
        );
    }
    t10_checkpoint(&actual_artifact_root, &repository, "implementation");
    assert_eq!(
        t10_event(&database, &repository, run_id, "implementation-ready")["status"],
        "completed"
    );
    assert_eq!(
        t10_event(&database, &repository, run_id, "reconciliation-ready")["status"],
        "completed"
    );
    let validation_show = t10_show(&database, &repository, run_id);
    assert_eq!(validation_show["result"]["current_state"], "validation");

    // This is the public before/guard case: a flat provider command cannot
    // impersonate the engine-owned append origin field.
    let (forged, forged_output) = engine_output(
        &database,
        &repository,
        &[
            "append".into(),
            run_id.into(),
            "--kind".into(),
            "validation-command".into(),
            "--record-id".into(),
            "validation-t10-forged-flat".into(),
            serde_json::to_string(&required).expect("forged command JSON"),
        ],
    );
    assert!(forged_output.status.success() || forged["status"] == "rejected");
    assert_eq!(forged["status"], "rejected");
    assert_eq!(forged["code"], "selected-output-linkage-refused");
    assert!(forged["message"].as_str().unwrap().contains("`command`"));

    let matrix_path = actual_artifact_root.join("t10-capture-matrix.json");
    let capture_root = actual_artifact_root.join("t10-capture");
    write_json(
        &matrix_path,
        &json!({
            "rows": [
                {
                    "id": required["id"],
                    "argv": [required["command"], required["args"][0], required["args"][1]],
                    "environment": {},
                    "inherit_environment": [],
                    "timeout_ms": 120000,
                    "obligations": [required["obligation"]]
                },
                {
                    "id": extra["id"],
                    "argv": [extra["command"], extra["args"][0], extra["args"][1], extra["args"][2]],
                    "environment": {},
                    "inherit_environment": [],
                    "timeout_ms": 120000,
                    "obligations": [extra["obligation"]]
                }
            ]
        }),
    );
    let capture = Command::new(t10_engine())
        .current_dir(&repository)
        .args([
            "capture-matrix",
            "--matrix",
            matrix_path.to_str().expect("matrix"),
            "--working-directory",
            repository.to_str().expect("repository"),
            "--output-dir",
            capture_root.to_str().expect("capture root"),
        ])
        .output()
        .expect("capture matrix");
    assert!(
        capture.status.success(),
        "capture matrix failed: stdout={:?}, stderr={:?}",
        capture.stdout,
        capture.stderr
    );
    let capture_index = capture_root.join("index.json");
    assert!(capture_index.is_file(), "capture index missing");
    assert_eq!(
        fs::read_to_string(&marker).expect("prepared extra output"),
        "extra\n"
    );

    let preparation_packet = json!({
        "show": validation_show,
        "working_directory": repository,
        "revision": "validation-t10",
        "author": {"name": "backlog_t10", "kind": "script"},
        "capture_indexes": [capture_index],
        "execution_settings": {"environment": {}, "inherit_environment": [], "timeout_ms": 120000},
        "additions": [extra]
    });
    let prepared_output = provider_output(
        &repository,
        &["prepare-validation".into()],
        &preparation_packet,
    );
    assert_eq!(
        prepared_output.status.code(),
        Some(0),
        "prepare-validation stderr: {:?}",
        prepared_output.stderr
    );
    let prepared: Value = serde_json::from_slice(&prepared_output.stdout).expect("prepared JSON");
    assert_eq!(prepared["status"], "draft-only");
    assert_eq!(prepared["commands_complete"], true, "prepared: {prepared}");
    assert_eq!(prepared["addition_candidates"].as_array().unwrap().len(), 1);
    assert_eq!(prepared["addition_candidates"][0]["data"]["spec"], extra);
    assert!(prepared["addition_candidates"][0]["data"]
        .get("command")
        .is_none());

    let duplicate_packet = json!({
        "show": t10_show(&database, &repository, run_id),
        "working_directory": repository,
        "revision": "validation-t10-duplicate",
        "author": {"name": "backlog_t10", "kind": "script"},
        "capture_indexes": [capture_index],
        "execution_settings": {"environment": {}, "inherit_environment": [], "timeout_ms": 120000},
        "additions": [required]
    });
    let duplicate_output = provider_output(
        &repository,
        &["prepare-validation".into()],
        &duplicate_packet,
    );
    assert_eq!(duplicate_output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate_output.stderr).contains("duplicates/replaces"));

    let addition = &prepared["addition_candidates"][0];
    t10_append(
        &database,
        &repository,
        run_id,
        addition["kind"].as_str().expect("addition kind"),
        addition["record_id"].as_str().expect("addition ID"),
        &addition["data"],
    );

    let commissioned = provider_json(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &t10_show(&database, &repository, run_id),
    );
    let proof_commands = commissioned["commission"]["proof_commands"]
        .as_array()
        .expect("commission proof commands");
    assert_eq!(proof_commands.len(), 2);
    assert_eq!(proof_commands[0], required);
    assert_eq!(
        proof_commands
            .iter()
            .find(|command| command["id"] == "extra-proof")
            .expect("commissioned addition")["command"],
        "python3"
    );

    // Execute the newly commissioned command through the public validation
    // runner, then append the real captured command-evidence record.
    let run_output = provider_output(
        &repository,
        &[
            "run-validation".into(),
            "--engine".into(),
            t10_engine().to_string_lossy().into_owned(),
            "--working-directory".into(),
            repository.to_string_lossy().into_owned(),
            "--revision".into(),
            "validation-t10-execution".into(),
            "--commands".into(),
            "extra-proof".into(),
        ],
        &t10_show(&database, &repository, run_id),
    );
    assert_eq!(
        run_output.status.code(),
        Some(0),
        "run-validation failed: {:?}",
        run_output.stderr
    );
    let executed: Value = serde_json::from_slice(&run_output.stdout).expect("run-validation JSON");
    assert_eq!(executed["commands_passed"], true);
    assert_eq!(executed["command_candidates"].as_array().unwrap().len(), 1);
    assert_eq!(
        executed["command_candidates"][0]["data"]["proof_id"],
        "extra-proof"
    );
    assert_eq!(
        fs::read_to_string(&marker).expect("executed extra output"),
        "extra\nextra\n"
    );

    let candidate = &executed["command_candidates"][0];
    t10_append(
        &database,
        &repository,
        run_id,
        candidate["kind"].as_str().expect("candidate kind"),
        candidate["record_id"].as_str().expect("candidate ID"),
        &candidate["data"],
    );
    let collected = provider_json(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &t10_show(&database, &repository, run_id),
    );
    let collection = collected["validation_collection"]
        .as_array()
        .expect("validation collection");
    let extra_evidence_id = candidate["record_id"]
        .as_str()
        .expect("candidate record ID");
    assert!(
        collected["commission"]["record_ids"]
            .as_array()
            .expect("commission record IDs")
            .iter()
            .any(|id| id.as_str() == Some(extra_evidence_id)),
        "collection did not retain extra command evidence: {collected}"
    );
    assert!(collection.iter().all(|row| row["mode"] == "pending"));

    // Existing flat records remain readable, while duplicate/replacement and
    // unknown envelope shapes still fail closed in the public commission CLI.
    let mut legacy_show = t10_show(&database, &repository, run_id);
    let mut legacy = required.clone();
    legacy["id"] = json!("legacy-proof");
    t10_context_record(
        &mut legacy_show,
        "validation-t10-legacy-flat",
        legacy.clone(),
    );
    let legacy_commission = provider_json(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &legacy_show,
    );
    assert!(legacy_commission["commission"]["proof_commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["id"] == "legacy-proof"));

    let mut duplicate_show = t10_show(&database, &repository, run_id);
    t10_context_record(
        &mut duplicate_show,
        "validation-t10-duplicate-record",
        json!({"spec": required}),
    );
    let duplicate_commission = provider_output(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &duplicate_show,
    );
    assert_eq!(duplicate_commission.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate_commission.stderr).contains("duplicates/replaces"));

    let mut unknown_show = t10_show(&database, &repository, run_id);
    t10_context_record(
        &mut unknown_show,
        "validation-t10-unknown-envelope",
        json!({"spec": extra, "unexpected": true}),
    );
    let unknown_commission = provider_output(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &unknown_show,
    );
    assert_eq!(unknown_commission.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown_commission.stderr).contains("unknown field"));

    let mut blank_show = t10_show(&database, &repository, run_id);
    let mut blank = extra.clone();
    blank["owner"] = json!("");
    t10_context_record(
        &mut blank_show,
        "validation-t10-blank-field",
        json!({"spec": blank}),
    );
    let blank_commission = provider_output(
        &repository,
        &[
            "commission".into(),
            "--slot".into(),
            "validation-draft".into(),
        ],
        &blank_show,
    );
    assert_eq!(blank_commission.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&blank_commission.stderr).contains("blank fields"));
}

#[test]
fn backlog_t10_run_plan_graph_help_is_discoverable_without_a_working_directory() {
    let output = Command::new(workspace_integration::binary("software-change"))
        .args(["run-plan-graph", "--help"])
        .output()
        .expect("run-plan-graph help");

    assert!(output.status.success(), "help failed: {output:?}");
    assert!(output.stderr.is_empty(), "help wrote stderr: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: software-change run-plan-graph"));
    assert!(stdout.contains("existing absolute working directory"));
    assert!(stdout.contains("mandatory summarizer"));
}
