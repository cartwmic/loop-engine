use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(label: &str) -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "software-change-backlog-t07-{label}-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary root");
    path
}

fn engine() -> PathBuf {
    workspace_integration::binary("loop-engine")
}

fn provider() -> PathBuf {
    workspace_integration::binary("software-change")
}

fn run_json(command: &mut Command, label: &str) -> Value {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label} did not return JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success() || value["status"] == "rejected",
        "{label} failed: {value}; stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    value
}

struct RunFixture {
    artifacts: PathBuf,
    repository: PathBuf,
    database: PathBuf,
    providers: PathBuf,
    profile: PathBuf,
    run_id: String,
    axis: String,
    required_authors: u64,
    config_version: String,
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON bytes")).expect("write JSON");
}

fn git(repository: &Path, args: &[&str], label: &str) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("git {label}: {error}"));
    assert!(
        output.status.success(),
        "git {label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_run(label: &str, axis: &str, required_authors: u64) -> RunFixture {
    let root = temp_root(label);
    let artifacts = root.join("artifacts");
    let repository = root.join("repository");
    fs::create_dir_all(&artifacts).expect("artifact root");
    fs::create_dir_all(&repository).expect("repository root");
    for (name, source) in [
        ("intent.json", "intent-good.json"),
        ("design.json", "design-good.json"),
        ("plan.json", "plan-good.json"),
        (
            "implementation-report.json",
            "implementation-report-good.json",
        ),
        ("validation-report.json", "validation-report-good.json"),
    ] {
        fs::copy(
            workspace_integration::package_root("software-change-provider")
                .join("data/calibration/fixtures")
                .join(source),
            artifacts.join(name),
        )
        .unwrap_or_else(|error| panic!("copy {name}: {error}"));
    }
    let mut plan: Value =
        serde_json::from_slice(&fs::read(artifacts.join("plan.json")).expect("plan fixture"))
            .expect("plan JSON");
    plan["proof_commands"] = json!([
        {
            "id": "backlog-t07-command",
            "command": "python3",
            "args": ["-c", "print('backlog_t07 proof')"],
            "owner": "backlog-t07",
            "obligation": "A real named command completes successfully in the test repository."
        }
    ]);
    write_json(&artifacts.join("plan.json"), &plan);

    fs::write(repository.join("tracked.txt"), "backlog_t07 baseline\n").expect("baseline");
    git(&repository, &["init", "-q"], "init");
    git(
        &repository,
        &["config", "user.name", "backlog-t07"],
        "user.name",
    );
    git(
        &repository,
        &["config", "user.email", "backlog-t07@example.invalid"],
        "user.email",
    );
    git(
        &repository,
        &["config", "commit.gpgsign", "false"],
        "commit.gpgsign",
    );
    git(&repository, &["add", "-A"], "add");
    git(
        &repository,
        &["commit", "-qm", "backlog_t07 baseline"],
        "commit",
    );

    let mut profile: Value = serde_json::from_str(
        &fs::read_to_string(
            workspace_integration::package_root("software-change-provider")
                .join("data/configs/minimal.json"),
        )
        .expect("minimal profile"),
    )
    .expect("minimal profile JSON");
    let config_version = format!("backlog-t07-{label}");
    profile["config_version"] = json!(config_version);
    profile["criterion_policy"] = json!({
        "required_authors": required_authors,
        "goal_required_authors": required_authors
    });
    profile["artifact_root"] = json!(artifacts.to_string_lossy().to_string());
    let gates = profile["review_policies"]
        .as_object()
        .expect("shipped review policies")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for gate in gates {
        profile["review_policies"][&gate] = json!([{
            "id": axis,
            "description": format!("backlog_t07 {axis} obligation"),
            "example_prompt": format!("Judge the frozen {axis} obligation."),
            "review_stage": "aggregate",
            "required_authors": required_authors
        }]);
    }
    profile
        .as_object_mut()
        .expect("profile object")
        .remove("work_slot_bindings");
    let profile_path = root.join("profile.json");
    write_json(&profile_path, &profile);
    let providers = root.join("providers.toml");
    fs::write(
        &providers,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            provider().to_string_lossy()
        ),
    )
    .expect("provider catalog");

    RunFixture {
        artifacts,
        repository,
        database: root.join("loop.sqlite"),
        providers,
        profile: profile_path,
        run_id: format!("backlog-t07-{label}"),
        axis: axis.to_owned(),
        required_authors,
        config_version,
    }
}

fn engine_call(run: &RunFixture, args: &[String], label: &str) -> Value {
    let mut command = Command::new(engine());
    command
        .current_dir(&run.repository)
        .arg("--database")
        .arg(&run.database)
        .arg("--json")
        .args(args);
    run_json(&mut command, label)
}

fn show(run: &RunFixture, label: &str) -> Value {
    engine_call(
        run,
        &[
            "show".into(),
            "--view".into(),
            "full".into(),
            run.run_id.clone(),
        ],
        label,
    )
}

fn append(run: &RunFixture, kind: &str, id: &str, data: Value, label: &str) -> Value {
    let _ = show(run, &format!("{label} observation"));
    engine_call(
        run,
        &[
            "append".into(),
            format!("--record-id={id}"),
            run.run_id.clone(),
            kind.to_owned(),
            serde_json::to_string(&data).expect("record JSON"),
        ],
        label,
    )
}

fn event(run: &RunFixture, name: &str, label: &str) -> Value {
    let _ = show(run, &format!("{label} observation"));
    engine_call(
        run,
        &["event".into(), run.run_id.clone(), name.to_owned()],
        label,
    )
}

fn assert_completed(value: &Value, target: &str, label: &str) {
    assert_eq!(value["status"], "completed", "{label}: {value}");
    assert_eq!(
        value["result"]["run"]["current_state"], target,
        "{label}: {value}"
    );
}

fn review_evidence(run: &RunFixture, gate: &str, subject: &str, revision: &str, prefix: &str) {
    for index in 0..run.required_authors {
        let author = format!("{prefix}-{index}");
        let id = format!("{prefix}-{gate}-{index}");
        append(
            run,
            "review-evidence",
            &id,
            json!({
                "gate": gate,
                "policy_id": run.axis,
                "review_stage": "aggregate",
                "result": "pass",
                "findings": "",
                "author": {"name": author, "kind": "script"},
                "subject": subject,
                "subject_revision": revision,
                "config_version": run.config_version
            }),
            &format!("{gate} evidence {index}"),
        );
    }
}

fn review_ledger(run: &RunFixture, gate: &str, subject: &str, revision: &str, id: &str) {
    append(
        run,
        "finding-ledger",
        id,
        json!({
            "schema_version": "1",
            "gate": gate,
            "subject": subject,
            "subject_revision": revision,
            "author": {"name": "backlog-t07-driver", "kind": "agent"},
            "findings": []
        }),
        &format!("{gate} ledger"),
    );
}

fn pass_review(
    run: &RunFixture,
    gate: &str,
    event_name: &str,
    target: &str,
    subject: &str,
    revision: &str,
    prefix: &str,
) {
    let denied = event(run, event_name, &format!("{gate} missing ledger"));
    assert_eq!(
        denied["status"], "rejected",
        "{gate} should require its ledger"
    );
    assert_eq!(
        denied["code"], "software-change-finding-ledger-invalid",
        "{denied}"
    );
    review_evidence(run, gate, subject, revision, prefix);
    review_ledger(
        run,
        gate,
        subject,
        revision,
        &format!("{prefix}-{gate}-ledger"),
    );
    let allowed = event(run, event_name, &format!("{gate} complete"));
    assert_completed(&allowed, target, gate);
}

fn checkpoint(run: &RunFixture, phase: &str) {
    let output = Command::new(provider())
        .current_dir(&run.repository)
        .args([
            "checkpoint",
            "--phase",
            phase,
            "--artifact-root",
            run.artifacts.to_str().expect("artifact path"),
            "--working-directory",
            run.repository.to_str().expect("repository path"),
        ])
        .output()
        .expect("checkpoint process");
    assert!(
        output.status.success(),
        "{phase} checkpoint failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).expect("checkpoint JSON");
}

fn prepare_validation(run: &RunFixture) -> String {
    let shown = show(run, "validation preparation show");
    let helper =
        workspace_integration::repository_root().join("tests/fixtures/prepare-validation.py");
    let output = Command::new("python3")
        .current_dir(&run.repository)
        .args([
            helper.to_str().expect("prepare helper"),
            "--provider",
            provider().to_str().expect("provider path"),
            "--engine",
            engine().to_str().expect("engine path"),
            "--working-directory",
            run.repository.to_str().expect("repository path"),
            "--revision",
            "backlog-t07-validation",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("prepare stdin")
                .write_all(&serde_json::to_vec(&shown).expect("show JSON"))?;
            child.wait_with_output()
        })
        .expect("prepare-validation process");
    assert!(
        output.status.success(),
        "prepare-validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared: Value = serde_json::from_slice(&output.stdout).expect("prepared JSON");
    for record in prepared["records"].as_array().expect("prepared records") {
        append(
            run,
            record["kind"].as_str().expect("record kind"),
            record["record_id"].as_str().expect("record ID"),
            record["data"].clone(),
            "validation record",
        );
    }
    serde_json::from_slice::<Value>(
        &fs::read(run.artifacts.join("validation-report.json")).expect("validation report"),
    )
    .expect("validation report JSON")["revision"]
        .as_str()
        .expect("validation revision")
        .to_owned()
}

fn drive_run(run: &RunFixture, foreign_intent_records: &[Value]) -> Value {
    assert!(run.artifacts.join("intent.json").is_file());
    assert!(run.artifacts.join("design.json").is_file());
    assert!(run.artifacts.join("plan.json").is_file());

    let started = engine_call(
        run,
        &[
            "--config".into(),
            run.providers.to_string_lossy().into_owned(),
            "start".into(),
            "--id".into(),
            run.run_id.clone(),
            "software-change".into(),
            format!("@{}", run.profile.display()),
            "backlog t07 public run".into(),
        ],
        "software-change start",
    );
    assert_eq!(started["status"], "completed", "{started}");
    let initial = show(run, "early context show");
    assert_eq!(initial["result"]["current_state"], "explore");
    assert_eq!(
        initial["result"]["initial_input"]["config_version"],
        run.config_version
    );

    assert_completed(
        &event(run, "intent-ready", "intent-ready"),
        "intent-review",
        "intent-ready",
    );
    if foreign_intent_records.is_empty() {
        pass_review(
            run,
            "intent-review",
            "approved",
            "intent-adversarial-review",
            "intent.json",
            "r15",
            "own-intent",
        );
        pass_review(
            run,
            "intent-adversarial-review",
            "approved",
            "design",
            "intent.json",
            "r15",
            "own-intent-challenge",
        );
    } else {
        for record in foreign_intent_records {
            append(
                run,
                record["kind"].as_str().expect("foreign kind"),
                &format!("foreign-{}", record["id"].as_str().expect("foreign id")),
                record["data"].clone(),
                "foreign policy record",
            );
        }
        let denied = event(run, "approved", "foreign policy evidence");
        assert_eq!(
            denied["status"], "rejected",
            "foreign evidence unexpectedly passed"
        );
        assert_eq!(
            denied["code"], "software-change-review-incomplete",
            "{denied}"
        );
        assert!(
            denied.to_string().contains("stale_config"),
            "wrong-run policy diagnostic omitted stale config: {denied}"
        );
        review_evidence(run, "intent-review", "intent.json", "r15", "own-intent");
        review_ledger(
            run,
            "intent-review",
            "intent.json",
            "r15",
            "own-intent-ledger",
        );
        assert_completed(
            &event(run, "approved", "own intent after foreign denial"),
            "intent-adversarial-review",
            "intent-review",
        );
        pass_review(
            run,
            "intent-adversarial-review",
            "approved",
            "design",
            "intent.json",
            "r15",
            "own-intent-challenge",
        );
    }

    assert_completed(
        &event(run, "design-ready", "design-ready"),
        "design-review",
        "design-ready",
    );
    pass_review(
        run,
        "design-review",
        "approved",
        "design-adversarial-review",
        "design.json",
        "r15",
        "own-design",
    );
    pass_review(
        run,
        "design-adversarial-review",
        "approved",
        "plan",
        "design.json",
        "r15",
        "own-design-challenge",
    );

    assert_completed(
        &event(run, "plan-ready", "plan-ready"),
        "plan-review",
        "plan-ready",
    );
    pass_review(
        run,
        "plan-review",
        "approved",
        "plan-adversarial-review",
        "plan.json",
        "r15",
        "own-plan",
    );
    pass_review(
        run,
        "plan-adversarial-review",
        "approved",
        "implement",
        "plan.json",
        "r15",
        "own-plan-challenge",
    );

    checkpoint(run, "implementation");
    assert_completed(
        &event(run, "implementation-ready", "implementation-ready"),
        "implementation-review",
        "implementation-ready",
    );
    pass_review(
        run,
        "implementation-review",
        "approved",
        "implementation-adversarial-review",
        "implementation-report.json",
        "r15",
        "own-implementation",
    );
    pass_review(
        run,
        "implementation-adversarial-review",
        "approved",
        "validation",
        "implementation-report.json",
        "r15",
        "own-implementation-challenge",
    );

    let validation_revision = prepare_validation(run);
    assert_completed(
        &event(run, "validation-ready", "validation-ready"),
        "validation-review",
        "validation-ready",
    );
    pass_review(
        run,
        "validation-review",
        "approved",
        "validation-adversarial-review",
        "validation-report.json",
        &validation_revision,
        "own-validation",
    );
    pass_review(
        run,
        "validation-adversarial-review",
        "passed",
        "end",
        "validation-report.json",
        &validation_revision,
        "own-validation-challenge",
    );
    let final_show = show(run, "completed software-change show");
    assert_eq!(final_show["result"]["current_state"], "end", "{final_show}");
    assert_eq!(final_show["result"]["lifecycle"], "final", "{final_show}");
    final_show
}

fn state_topology(workflow: &Value) -> Value {
    Value::Array(
        workflow["states"]
            .as_array()
            .expect("workflow states")
            .iter()
            .map(|state| {
                json!({
                    "id": state["id"],
                    "title": state["title"],
                    "final": state["final"]
                })
            })
            .collect(),
    )
}

fn describe(initial_input: &Value) -> Value {
    let mut command = Command::new(provider());
    command
        .current_dir(workspace_integration::repository_root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = command.spawn().expect("provider describe");
    serde_json::to_writer(
        child.stdin.as_mut().expect("provider stdin"),
        &json!({"operation":"describe","initial_input":initial_input}),
    )
    .expect("describe request");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("provider describe wait");
    assert!(output.status.success(), "describe failed: {:?}", output);
    serde_json::from_slice(&output.stdout).expect("describe response")
}

#[test]
fn backlog_t07_software_change_starts_with_substantial_early_context_and_completes() {
    let run = create_run("early", "early-axis", 1);
    let final_show = drive_run(&run, &[]);
    let context = final_show["result"]["context"].as_array().expect("context");
    assert!(context
        .iter()
        .any(|record| record["kind"] == "review-evidence"));
    assert!(run.artifacts.join("intent.json").is_file());
    assert!(run.artifacts.join("design.json").is_file());
    assert!(run.artifacts.join("plan.json").is_file());
}

#[test]
fn backlog_t07_same_topology_different_frozen_policies_reject_foreign_evidence() {
    let first = create_run("policy-a", "shared-axis", 1);
    let first_final = drive_run(&first, &[]);
    let first_input = &first_final["result"]["initial_input"];
    let first_context: Vec<Value> = first_final["result"]["context"]
        .as_array()
        .expect("first context")
        .iter()
        .filter(|record| {
            matches!(
                record["kind"].as_str(),
                Some("review-evidence") | Some("finding-ledger")
            ) && record["data"]["gate"] == "intent-review"
        })
        .cloned()
        .collect();
    assert!(
        !first_context.is_empty(),
        "first run lacked policy evidence"
    );

    let second = create_run("policy-b", "shared-axis", 2);
    let second_final = drive_run(&second, &first_context);
    let second_input = &second_final["result"]["initial_input"];
    let first_workflow = describe(first_input);
    let second_workflow = describe(second_input);
    assert_eq!(first_workflow["id"], "software-change");
    assert_eq!(
        state_topology(&first_workflow),
        state_topology(&second_workflow)
    );
    assert_eq!(
        first_workflow["transitions"],
        second_workflow["transitions"]
    );
    assert_eq!(first_workflow["work_slots"], second_workflow["work_slots"]);
    assert_ne!(
        first_input["review_policies"],
        second_input["review_policies"]
    );
    assert_ne!(
        first_input["criterion_policy"],
        second_input["criterion_policy"]
    );
    assert_eq!(first_final["result"]["lifecycle"], "final");
    assert_eq!(second_final["result"]["lifecycle"], "final");
}

#[test]
fn backlog_t07_policy_document_semantic_failure_is_current_and_corrected_in_both_modes() {
    let repository = workspace_integration::repository_root();
    let script = repository.join("scripts/policy-document-journey.py");
    let engine_path = workspace_integration::binary("loop-engine");
    let provider_path = workspace_integration::binary("policy-document");
    let profile = repository.join("crates/policy-document-provider/data/readme.json");
    for mode in ["draft", "audit"] {
        let output = Command::new("python3")
            .current_dir(&repository)
            .args([
                script.to_str().expect("journey path"),
                "--engine",
                engine_path.to_str().expect("engine path"),
                "--provider",
                provider_path.to_str().expect("provider path"),
                "--profile",
                profile.to_str().expect("profile path"),
                "--mode",
                mode,
            ])
            .output()
            .expect("policy journey");
        assert!(
            output.status.success(),
            "policy {mode} journey failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("current semantic finding denial with exact findings"),
            "policy {mode} journey omitted current-failure proof: {stdout}"
        );
        assert!(
            stdout.contains(&format!("\"mode\": \"{mode}\"")),
            "{stdout}"
        );
    }
}
