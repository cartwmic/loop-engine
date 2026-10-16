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
    engine_path: PathBuf,
    provider_path: PathBuf,
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
    let engine_path = workspace_integration::binary("loop-engine");
    let provider_path = workspace_integration::binary("software-change");
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
    write_json(
        &artifacts.join("reconciliation.json"),
        &super::reconciliation_fixture(),
    );
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

    let mut intent: Value =
        serde_json::from_slice(&fs::read(artifacts.join("intent.json")).unwrap()).unwrap();
    let head = Command::new("git")
        .current_dir(&repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    intent["baseline"] = json!({"kind":"git","repository_scope":"primary","commit":String::from_utf8(head.stdout).unwrap().trim()});
    write_json(&artifacts.join("intent.json"), &intent);

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
            provider_path.to_string_lossy()
        ),
    )
    .expect("provider catalog");

    RunFixture {
        artifacts,
        repository,
        database: root.join("loop.sqlite"),
        providers,
        engine_path,
        provider_path,
        profile: profile_path,
        run_id: format!("backlog-t07-{label}"),
        axis: axis.to_owned(),
        required_authors,
        config_version,
    }
}

fn engine_call(run: &RunFixture, args: &[String], label: &str) -> Value {
    let mut command = Command::new(&run.engine_path);
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
        let appended = append(
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
                "config_version": run.config_version,
                "review_contract_version": 3,
                "grounds": {
                    "reason": "Scripted workflow control against the current subject revision; not a semantic review.",
                    "evidence": [{"locator": format!("{subject}#/revision")}]
                }
            }),
            &format!("{gate} evidence {index}"),
        );
        assert_eq!(
            appended["status"], "completed",
            "{gate} evidence: {appended}"
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
    let output = Command::new(&run.provider_path)
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
            run.provider_path.to_str().expect("provider path"),
            "--engine",
            run.engine_path.to_str().expect("engine path"),
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
        let mut refused_reviews = 0;
        for record in foreign_intent_records {
            let id = format!("foreign-{}", record["id"].as_str().expect("foreign id"));
            let before = show(run, "foreign candidate snapshot");
            let result = append(
                run,
                record["kind"].as_str().expect("foreign kind"),
                &id,
                record["data"].clone(),
                "foreign policy record",
            );
            if record["kind"] == "review-evidence" {
                refused_reviews += 1;
                assert_eq!(result["status"], "rejected", "{result}");
                assert_eq!(result["code"], "record-refused", "{result}");
                assert_eq!(result["details"]["record_id"], id, "{result}");
                assert_eq!(result["details"]["rule"], "review-shape-policy", "{result}");
                assert!(
                    result["details"]["message"]
                        .as_str()
                        .expect("policy refusal")
                        .contains("review config_version does not match frozen input"),
                    "{result}"
                );
                let after = show(run, "foreign candidate remains absent");
                assert_eq!(after["result"]["context"], before["result"]["context"]);
                assert_eq!(
                    after["result"]["current_state"],
                    before["result"]["current_state"]
                );
            } else {
                // Empty same-subject ledger is not a review/config claim.
                assert_eq!(record["kind"], "finding-ledger");
                assert_eq!(result["status"], "completed", "{result}");
            }
        }
        assert!(
            refused_reviews > 0,
            "fixture must attempt real foreign review evidence"
        );
        let denied = event(run, "approved", "foreign policy evidence");
        assert_eq!(
            denied["status"], "rejected",
            "foreign evidence unexpectedly passed"
        );
        assert_eq!(
            denied["code"], "software-change-review-incomplete",
            "{denied}"
        );
        // No foreign review was stored, so the subsequent gate must
        // diagnose the still-missing required own coverage, not stale_config.
        assert!(
            denied["details"]["diagnostics"]
                .as_array()
                .expect("axis diagnostics")
                .iter()
                .any(|axis| axis["axis"] == run.axis
                    && axis["review_stage"] == "aggregate"
                    && axis["diagnostics"]
                        .as_array()
                        .expect("causes")
                        .iter()
                        .any(|cause| cause["category"] == "missing"
                            && cause["required"] == run.required_authors)),
            "{denied}"
        );
        assert!(
            !show(run, "no foreign policy contamination")["result"]["context"]
                .as_array()
                .expect("context")
                .iter()
                .any(|r| r["kind"] == "review-evidence"
                    && r["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("foreign-")))
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
        "reconciliation",
        "implementation-ready",
    );
    assert_completed(
        &event(run, "reconciliation-ready", "reconciliation-ready"),
        "implementation-review",
        "reconciliation-ready",
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

fn describe(initial_input: &Value, provider_path: &Path) -> Value {
    let mut command = Command::new(provider_path);
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
    let first_workflow = describe(first_input, &first.provider_path);
    let second_workflow = describe(second_input, &second.provider_path);
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

fn ledger(gate: &str, subject: &str, revision: &str) -> Value {
    json!({"schema_version":"1","gate":gate,"subject":subject,"subject_revision":revision,"author":{"name":"driver","kind":"agent"},"findings":[]})
}

fn prepare(run: &RunFixture, arguments: &[&str], input: &Value) -> Value {
    let mut command = Command::new(&run.provider_path);
    command.current_dir(&run.repository).args(arguments);
    let output = super::bounded_process::run_with_stdin(
        &mut command,
        "external preparation",
        &serde_json::to_vec(input).unwrap(),
    )
    .unwrap()
    .output;
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn external_intake_early_commission_and_ordinary_first_public_path() {
    let run = create_run("external-intake", "intent-sufficient", 1);
    let mut profile: Value = serde_json::from_slice(&fs::read(&run.profile).unwrap()).unwrap();
    for gate in ["intent-review", "intent-adversarial-review"] {
        profile["review_policies"][gate][0]["group"] = json!(format!("{gate}.intent-sufficient"));
        profile["review_policies"][gate][0]["self_review"] = json!(true);
    }
    write_json(&run.profile, &profile);
    let started = engine_call(
        &run,
        &[
            "--config".into(),
            run.providers.to_string_lossy().into_owned(),
            "start".into(),
            "--id".into(),
            run.run_id.clone(),
            "software-change".into(),
            format!("@{}", run.profile.display()),
        ],
        "start external intake",
    );
    assert_eq!(started["status"], "completed", "{started}");
    let shown = show(&run, "early show");
    let intent_path = run.artifacts.join("intent.json");
    let intent: Value = serde_json::from_slice(&fs::read(&intent_path).unwrap()).unwrap();
    fs::rename(&intent_path, run.artifacts.join("saved-intent.json")).unwrap();
    let early = prepare(&run, &["commission", "--slot", "intent-review"], &shown);
    assert!(
        !early["missing_inputs"].as_array().unwrap().is_empty(),
        "{early}"
    );
    fs::rename(run.artifacts.join("saved-intent.json"), &intent_path).unwrap();
    let projection = json!({"judgments":[{"axis":"intent-sufficient","result":"pass","findings":"","grounds":{"reason":"current goal permits no unwanted alternative","evidence":[{"locator":"intent.json#/revision"}]}},false,{"axis":"intent-sufficient","result":"pass"}]});
    let intake = |gate: &str, author: Value, original: Value, explicit: bool| {
        let mut input = json!({"show":shown,"assignment":{"gate":gate,"stage":"aggregate","group":format!("{gate}.intent-sufficient"),"author":author,"subject":"intent.json","subject_revision":intent["revision"]},"request":{"text":"reconstruct the final state"},"original":original});
        if explicit {
            input["projection"] = projection.clone();
        }
        prepare(&run, &["review-candidates"], &input)
    };
    let self_review = intake(
        "intent-review",
        intent["author"].clone(),
        json!({"text":"cold reconstruction"}),
        true,
    );
    assert!(self_review["records"].as_array().unwrap().is_empty());
    assert!(self_review["items"][0]["diagnostics"]
        .to_string()
        .contains("cannot be self-review"));
    let direct = intake(
        "intent-review",
        json!({"name":"unbriefed-reader","kind":"agent"}),
        json!({"text":"reconstruct the final state"}),
        true,
    );
    assert_eq!(
        direct["records"][0]["data"]["external_review"]["direct"],
        false
    );
    assert_eq!(direct["records"].as_array().unwrap().len(), 1);
    assert!(direct["items"][1]["candidate"].is_null());
    assert!(direct["items"][2]["candidate"].is_null());
    let challenge = intake(
        "intent-adversarial-review",
        json!({"name":"challenger","kind":"agent"}),
        json!({"text":format!("```json\n{projection}\n```")}),
        false,
    );
    append(
        &run,
        "review-evidence",
        "early-challenge",
        challenge["records"][0]["data"].clone(),
        "early challenge append",
    );
    append(
        &run,
        "finding-ledger",
        "challenge-ledger",
        ledger(
            "intent-adversarial-review",
            "intent.json",
            intent["revision"].as_str().unwrap(),
        ),
        "challenge ledger",
    );
    let context = show(&run, "challenge evaluation show")["result"]["context"].clone();
    let workflow = prepare(
        &run,
        &[],
        &json!({"operation":"describe","initial_input":profile}),
    );
    let evaluate = |context: Value| {
        prepare(
            &run,
            &[],
            &json!({"operation":"evaluate","workflow":workflow,"initial_input":profile,"context":context,"transition":{"source":"intent-adversarial-review","event":"approved","target":"design","kind":"checked"},"prior_evaluations":[]}),
        )
    };
    let blocked = evaluate(context);
    assert_eq!(
        blocked["feedback"]["message"], "challenge-before-ordinary-clearance",
        "{blocked}"
    );
    append(
        &run,
        "review-evidence",
        "early-direct",
        direct["records"][0]["data"].clone(),
        "early ordinary append",
    );
    append(
        &run,
        "finding-ledger",
        "ordinary-ledger",
        ledger(
            "intent-review",
            "intent.json",
            intent["revision"].as_str().unwrap(),
        ),
        "ordinary ledger",
    );
    append(
        &run,
        "intent-comparison",
        "owner-comparison",
        json!({
            "subject_revision":intent["revision"],"baseline":intent["baseline"],"reconstruction_ids":["early-direct"],
            "actor":{"name":"fixture-owner","kind":"human"},"authority":"owner",
            "dispositions":[{"reconstruction_id":"early-direct","alternative":"Fixture implementation freedom","disposition":"implementation-freedom","note":"Mechanics-only comparison."}]
        }),
        "compare cold return",
    );
    assert_eq!(
        event(&run, "intent-ready", "enter ordinary")["status"],
        "completed"
    );
    assert_eq!(
        event(&run, "approved", "count early ordinary")["status"],
        "completed"
    );
    let cleared = evaluate(show(&run, "cleared show")["result"]["context"].clone());
    assert_eq!(cleared["result"], "deny", "{cleared}");
    assert!(cleared["feedback"]["message"]
        .as_str()
        .unwrap()
        .contains("challenge-before-ordinary-clearance"));
    append(
        &run,
        "evidence-applicability",
        "challenge-current-attestation",
        json!({"origin":{"kind":"context-record","id":"early-challenge"},"target":{"subject":"intent.json","revision":intent["revision"]},"attesting_driver":{"name":"driver","kind":"human"},"reason":"The retained challenge applies to the now-cleared conclusion."}),
        "attest challenge after clearance",
    );
    let carried = evaluate(show(&run, "carried challenge show")["result"]["context"].clone());
    assert_eq!(carried["result"], "allow", "{carried}");
    append(
        &run,
        "review-evidence",
        "fresh-challenge",
        challenge["records"][0]["data"].clone(),
        "challenge cleared conclusion",
    );
    let fresh = evaluate(show(&run, "fresh challenge show")["result"]["context"].clone());
    assert_eq!(fresh["result"], "allow", "{fresh}");
    assert_eq!(
        event(&run, "approved", "challenge after ordinary")["status"],
        "completed"
    );
}

#[test]
fn external_scoped_grounding_and_secondary_checkpoint_staleness() {
    let run = create_run("source-scopes", "scope-check", 1);
    let secondary = run.repository.parent().unwrap().join("secondary");
    let cloned = Command::new("git")
        .args(["clone", "--quiet"])
        .arg(&run.repository)
        .arg(&secondary)
        .output()
        .unwrap();
    assert!(cloned.status.success(), "{cloned:?}");
    for (root, value) in [
        (&run.repository, json!({"primary":true})),
        (&secondary, json!({"secondary":true})),
    ] {
        write_json(&root.join("scoped.json"), &value);
        git(root, &["add", "scoped.json"], "scope add");
        git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "-qm",
                "scope baseline",
            ],
            "scope commit",
        );
    }
    let mut intent: Value =
        serde_json::from_slice(&fs::read(run.artifacts.join("intent.json")).unwrap()).unwrap();
    intent["source_scopes"] =
        json!({"primary":{"root":run.repository},"secondary":{"root":secondary}});
    write_json(&run.artifacts.join("intent.json"), &intent);
    let profile: Value = serde_json::from_slice(&fs::read(&run.profile).unwrap()).unwrap();
    let workflow = prepare(
        &run,
        &[],
        &json!({"operation":"describe","initial_input":profile}),
    );
    let mut evidence = json!({"gate":"intent-review","policy_id":"scope-check","review_stage":"aggregate","group":"intent-review.aggregate","result":"pass","findings":"","author":{"name":"scope-reviewer","kind":"agent"},"subject":"intent.json","subject_revision":intent["revision"],"config_version":run.config_version,"review_contract_version":3,"grounds":{"reason":"inspected secondary source","evidence":[{"locator":"secondary:scoped.json#/secondary"}]}});
    let evaluate_intent = |data: Value| {
        prepare(
            &run,
            &[],
            &json!({"operation":"evaluate","workflow":workflow,"initial_input":profile,"context":[{"id":"scoped-review","kind":"review-evidence","data":data,"sequence":1,"created_at":1},{"id":"scope-ledger","kind":"finding-ledger","data":ledger("intent-review","intent.json",intent["revision"].as_str().unwrap()),"sequence":2,"created_at":2}],"transition":{"source":"intent-review","event":"approved","target":"intent-adversarial-review","kind":"checked"},"prior_evaluations":[]}),
        )
    };
    assert_eq!(evaluate_intent(evidence.clone())["result"], "allow");
    evidence["grounds"]["evidence"][0]["locator"] = json!("primary:scoped.json#/secondary");
    assert_eq!(evaluate_intent(evidence.clone())["result"], "deny");
    evidence["grounds"]["evidence"][0]["locator"] = json!("unknown:scoped.json#/secondary");
    assert_eq!(evaluate_intent(evidence)["result"], "deny");
    checkpoint(&run, "implementation");
    let evaluate_checkpoint = || {
        prepare(
            &run,
            &[],
            &json!({"operation":"evaluate","workflow":workflow,"initial_input":profile,"context":[],"transition":{"source":"implementation-review","event":"approved","target":"implementation-adversarial-review","kind":"checked"},"prior_evaluations":[]}),
        )
    };
    let current = evaluate_checkpoint();
    assert_eq!(
        current["feedback"]["code"], "software-change-finding-ledger-invalid",
        "{current}"
    );
    write_json(&secondary.join("scoped.json"), &json!({"secondary":false}));
    let stale = evaluate_checkpoint();
    assert_eq!(stale["result"], "deny");
    assert!(
        stale
            .to_string()
            .contains("source scope repository identities changed"),
        "{stale}"
    );
}

#[test]
fn singleton_challenge_clears_independently_of_open_broad_group() {
    let run = create_run("singleton-clearance", "intent-sufficient", 1);
    let mut profile: Value = serde_json::from_slice(&fs::read(&run.profile).unwrap()).unwrap();
    let intent: Value =
        serde_json::from_slice(&fs::read(run.artifacts.join("intent.json")).unwrap()).unwrap();
    let axis = |id: &str, group: &str| json!({"id":id,"description":"test obligation","example_prompt":"judge obligation","review_stage":"aggregate","required_authors":1,"group":group});
    profile["review_policies"]["intent-review"] = json!([
        axis("intent-sufficient", "intent-review.intent-sufficient"),
        axis("broad", "intent-review.aggregate")
    ]);
    let evidence = |gate: &str, id: &str, group: &str, result: &str, findings: &str| json!({"gate":gate,"policy_id":id,"review_stage":"aggregate","group":group,"result":result,"findings":findings,"author":{"name":"reviewer","kind":"agent"},"subject":"intent.json","subject_revision":intent["revision"],"config_version":run.config_version,"baseline":intent["baseline"],"review_contract_version":3,"grounds":{"reason":"inspected current intent","evidence":[{"locator":"intent.json#/revision"}]}});
    let record = |id: &str, kind: &str, data: Value, seq: u64| json!({"id":id,"kind":kind,"data":data,"sequence":seq,"created_at":seq});
    let mut ordinary_ledger = ledger(
        "intent-review",
        "intent.json",
        intent["revision"].as_str().unwrap(),
    );
    ordinary_ledger["findings"] = json!([{"id":"F-broad-open","source":{"kind":"context-record","id":"broad-failure"},"policy_id":"broad","statement":"broad obligation remains unmet","disposition":"accepted","reason":"material defect","owner_phase":"intent","task_ids":[],"review_axes":["broad"],"status":"unresolved"}]);
    for (id, group, allowed) in [
        (
            "intent-sufficient",
            "intent-adversarial-review.intent-sufficient",
            true,
        ),
        ("broad", "intent-adversarial-review.aggregate", false),
    ] {
        profile["review_policies"]["intent-adversarial-review"] = json!([axis(id, group)]);
        let workflow = prepare(
            &run,
            &[],
            &json!({"operation":"describe","initial_input":profile}),
        );
        let context = json!([
            record(
                "singleton-pass",
                "review-evidence",
                evidence(
                    "intent-review",
                    "intent-sufficient",
                    "intent-review.intent-sufficient",
                    "pass",
                    ""
                ),
                1
            ),
            record(
                "broad-failure",
                "review-evidence",
                evidence(
                    "intent-review",
                    "broad",
                    "intent-review.aggregate",
                    "fail",
                    "broad obligation remains unmet"
                ),
                2
            ),
            record(
                "ordinary-ledger",
                "finding-ledger",
                ordinary_ledger.clone(),
                3
            ),
            // The owner comparison precedes the challenge that relies on it (AC-9, AC-19).
            record(
                "owner-comparison",
                "intent-comparison",
                json!({"subject_revision":intent["revision"],"baseline":intent["baseline"],
                "reconstruction_ids":["singleton-pass"],"actor":{"name":"fixture-owner","kind":"human"},"authority":"owner",
                "dispositions":[{"reconstruction_id":"singleton-pass","alternative":"Fixture freedom","disposition":"implementation-freedom","note":"Mechanics only."}]}),
                4
            ),
            record(
                "challenge-pass",
                "review-evidence",
                evidence("intent-adversarial-review", id, group, "pass", ""),
                5
            ),
            record(
                "challenge-ledger",
                "finding-ledger",
                ledger(
                    "intent-adversarial-review",
                    "intent.json",
                    intent["revision"].as_str().unwrap()
                ),
                6
            )
        ]);
        let result = prepare(
            &run,
            &[],
            &json!({"operation":"evaluate","workflow":workflow,"initial_input":profile,"context":context,"transition":{"source":"intent-adversarial-review","event":"approved","target":"design","kind":"checked"},"prior_evaluations":[]}),
        );
        if allowed {
            assert_eq!(result["result"], "allow", "{result}");
        } else {
            assert_eq!(
                result["feedback"]["message"], "challenge-before-ordinary-clearance",
                "{result}"
            );
        }
    }
}

#[test]
fn baseline_and_residual_schema_negatives_are_refused_publicly() {
    let run = create_run("schema-negatives", "schema-check", 1);
    let profile: Value = serde_json::from_slice(&fs::read(&run.profile).unwrap()).unwrap();
    let workflow = prepare(
        &run,
        &[],
        &json!({"operation":"describe","initial_input":profile}),
    );
    for (file, source, event, target, field, value) in [
        (
            "intent.json",
            "explore",
            "intent-ready",
            "intent-review",
            "baseline",
            json!({"kind":"git","repository_scope":"primary"}),
        ),
        (
            "implementation-report.json",
            "implementation-review",
            "approved",
            "implementation-adversarial-review",
            "residuals",
            json!([{"kind":"unknown","statement":"concrete residual","references":[]}]),
        ),
        (
            "validation-report.json",
            "validation-review",
            "approved",
            "validation-adversarial-review",
            "residuals",
            json!([{"kind":"unknown","statement":"concrete residual","references":[]}]),
        ),
    ] {
        let path = run.artifacts.join(file);
        let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut invalid = original.clone();
        invalid[field] = value;
        write_json(&path, &invalid);
        let result = prepare(
            &run,
            &[],
            &json!({"operation":"evaluate","workflow":workflow,"initial_input":profile,"context":[],"transition":{"source":source,"event":event,"target":target,"kind":"checked"},"prior_evaluations":[]}),
        );
        assert_eq!(
            result["feedback"]["code"], "software-change-schema-invalid",
            "{result}"
        );
        assert!(
            result["feedback"]["details"]["violations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["path"].as_str().is_some_and(|p| p.contains(field))),
            "{result}"
        );
        write_json(&path, &original);
    }
}
