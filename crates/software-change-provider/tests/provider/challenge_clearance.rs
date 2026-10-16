//! Public scripted journeys: bookends:LE-182 bookends:LE-171 bookends:LE-177 bookends:LE-185.
//! Synthetic judgments prove mechanics only; joined AC-5 proof: bookends:LE-194 bookends:LE-98.
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(label: &str) -> PathBuf {
    loop {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-challenge-clearance-{label}-{}-{suffix}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("allocate owned journey root: {error}"),
        }
    }
}

fn run_json(command: &mut Command, label: &str) -> Value {
    for key in [
        "LOOP_ENGINE_DATABASE",
        "LOOP_ENGINE_DATABASE_PATH",
        "LOOP_DATABASE",
        "LOOP_DATABASE_PATH",
        "LOOP_DB_PATH",
        "LOOP_ENGINE_DB",
        "LOOP_DB",
        "LOOP_ENGINE_HOME",
        "LOOP_HOME",
        "XDG_DATA_HOME",
    ] {
        command.env_remove(key);
    }
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
    admission_probe: bool,
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("JSON bytes")).expect("write JSON");
}

fn create_run(label: &str, axis: &str, required_authors: u64) -> RunFixture {
    let engine_path = workspace_integration::binary("loop-engine");
    let provider_path = workspace_integration::binary("software-change");
    let root = temp_root(label);
    let artifacts = root.join("artifacts");
    let repository = workspace_integration::repository_root();
    fs::create_dir_all(&artifacts).expect("artifact root");
    assert!(repository.is_dir(), "repository root");
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
            "id": "challenge-clearance-command",
            "command": "python3",
            "args": ["-c", "print('challenge_clearance proof')"],
            "owner": "challenge-clearance",
            "obligation": "A real named command completes successfully in the test repository."
        }
    ]);
    write_json(&artifacts.join("plan.json"), &plan);

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
    let config_version = format!("challenge-clearance-{label}");
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
            "description": format!("challenge_clearance {axis} obligation"),
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
        run_id: format!("challenge-clearance-{label}"),
        admission_probe: label == "admission-all-eight",
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
            "author": {"name": "challenge-clearance-driver", "kind": "agent"},
            "findings": []
        }),
        &format!("{gate} ledger"),
    );
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
            "challenge-clearance-validation",
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

fn provider(run: &RunFixture, args: &[&str], input: &Value) -> Value {
    let mut command = Command::new(&run.provider_path);
    command.current_dir(&run.repository).args(args);
    let output = super::bounded_process::run_with_stdin(
        &mut command,
        "scripted challenge helper",
        &serde_json::to_vec(input).unwrap(),
    )
    .unwrap()
    .output;
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn setup(label: &str, narratives: bool) -> RunFixture {
    let run = create_run(label, "clearance", 2);
    let mut profile: Value = serde_json::from_slice(&fs::read(&run.profile).unwrap()).unwrap();
    let mut gates: Vec<String> = profile["review_policies"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    if narratives {
        profile["extra"]["narratives"] = json!(["design", "plan", "validation"]);
        for phase in ["design", "plan", "validation"] {
            gates.push(format!("{phase}-narrative-review"));
            gates.push(format!("{phase}-narrative-adversarial-review"));
        }
    }
    for gate in gates {
        let stages = if gate.ends_with("-adversarial-review") {
            vec!["aggregate"]
        } else {
            vec!["individual", "aggregate"]
        };
        // High-rigor-style topology is essential to regression sensitivity:
        // distinct individual singleton groups, a combined aggregate group,
        // and an aggregate-only axis. The old challenge's filtered parent map
        // then sees otherwise-valid individual records as unconfigured stages.
        let mut policies = Vec::new();
        for stage in stages {
            let axes: &[&str] = if stage == "individual" {
                &["clearance", "companion"]
            } else {
                &["clearance", "companion", "aggregate-only"]
            };
            for axis in axes {
                let group = if stage == "individual" {
                    format!("{gate}.{stage}.{axis}")
                } else {
                    format!("{gate}.{stage}")
                };
                policies.push(
                    json!({"id":axis,"description":"scripted clearance obligation",
                    "example_prompt":"Inspect the current subject.","review_stage":stage,
                    "required_authors":2,"group":group}),
                );
            }
        }
        assert_eq!(
            policies
                .iter()
                .filter(|p| p["review_stage"] == "aggregate")
                .count(),
            3
        );
        if !gate.ends_with("-adversarial-review") {
            let individual: Vec<_> = policies
                .iter()
                .filter(|p| p["review_stage"] == "individual")
                .collect();
            assert_eq!(individual.len(), 2);
            assert_ne!(individual[0]["group"], individual[1]["group"]);
            assert!(individual.iter().all(|p| p["id"] != "aggregate-only"));
        }
        profile["review_policies"][&gate] = json!(policies);
    }
    if run.admission_probe {
        // Only the new AC-5 fixture needs real retained reconstruction sources
        // for the intent-comparison write. Existing sensitive profiles stay intact.
        for gate in ["intent-review", "intent-adversarial-review"] {
            let axis = json!({
                "id":"intent-sufficient","description":"Synthetic alignment obligation",
                "example_prompt":"Synthetic complete reconstruction; mechanics only.",
                "review_stage":"aggregate","required_authors":2,
                "group":format!("{gate}.intent-sufficient"),
                "cold":gate == "intent-review"
            });
            profile["review_policies"][gate]
                .as_array_mut()
                .unwrap()
                .push(axis);
        }
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
            "scripted clearance regression".into(),
        ],
        "start",
    );
    assert_eq!(started["status"], "completed", "{started}");
    run
}

// Scripted backend returns a retained request/original and assessment ID through
// the actual public candidate projector. No model or agent is launched.
fn evidence(
    run: &RunFixture,
    gate: &str,
    subject: &str,
    revision: &str,
    stage: &str,
    tag: &str,
) -> Vec<(String, Value)> {
    let shown = show(run, "script backend observation");
    // Derive every assignment from the frozen policy, including singleton
    // individual groups and the broader three-axis aggregate obligation.
    let mut groups = std::collections::BTreeMap::<String, Vec<String>>::new();
    for policy in shown["result"]["initial_input"]["review_policies"][gate]
        .as_array()
        .unwrap()
    {
        if policy["review_stage"] == stage {
            groups
                .entry(policy["group"].as_str().unwrap().to_owned())
                .or_default()
                .push(policy["id"].as_str().unwrap().to_owned());
        }
    }
    assert!(!groups.is_empty(), "no configured {gate}/{stage} groups");
    let mut records = Vec::new();
    for (group_index, (group, axes)) in groups.into_iter().enumerate() {
        for n in 0..2 {
            let judgments: Vec<_> = axes
                .iter()
                .map(|axis| {
                    json!({"axis":axis,"result":"pass","findings":"",
                "grounds":{"reason":"Scripted fixture inspected the named current subject.",
                "evidence":[{"locator":format!("{subject}#/revision")}]}})
                })
                .collect();
            let original = json!({"assessment_id":format!("{tag}-{group_index}-{n}"),"review_contract_version":3,"review_stage":stage,
                "author":{"name":format!("script-reviewer-{n}"),"kind":"script"},"judgments":judgments});
            let result = provider(
                run,
                &["review-candidates"],
                &json!({"show":shown,
                "assignment":{"gate":gate,"stage":stage,"group":group,"author":{"name":format!("script-reviewer-{n}"),"kind":"script"},"subject":subject,"subject_revision":revision},
                "request":{"text":format!("Scripted backend: inspect {gate}/{stage}/{group}/{revision}/{tag}")},"original":{"text":original.to_string()}}),
            );
            assert_eq!(
                result["records"].as_array().unwrap().len(),
                axes.len(),
                "{result}"
            );
            for (i, record) in result["records"].as_array().unwrap().iter().enumerate() {
                let id = format!("{tag}-{group_index}-{n}-{i}");
                let data = record["data"].clone();
                assert!(axes.iter().any(|axis| data["policy_id"] == *axis), "{data}");
                assert_eq!(
                    append(run, "review-evidence", &id, data.clone(), "scripted return")["status"],
                    "completed"
                );
                records.push((id, data));
            }
        }
    }
    records
}

fn ledger(run: &RunFixture, gate: &str, subject: &str, revision: &str, tag: &str) {
    review_ledger(run, gate, subject, revision, &format!("{tag}-ledger"));
}
fn ordinary(run: &RunFixture, gate: &str, subject: &str, revision: &str, aggregate_first: bool) {
    for stage in if aggregate_first {
        ["aggregate", "individual"]
    } else {
        ["individual", "aggregate"]
    } {
        evidence(
            run,
            gate,
            subject,
            revision,
            stage,
            &format!("{gate}-{stage}"),
        );
    }
    if run.admission_probe && gate == "intent-review" {
        assert_eq!(
            append(
                run,
                "intent-comparison",
                "fixture-alignment-comparison",
                comparison_data(run),
                "synthetic complete comparison"
            )["status"],
            "completed"
        );
    }
    ledger(run, gate, subject, revision, gate);
    if run.admission_probe {
        ac5_prepare_ordinary_candidates(run, gate, subject, revision);
    }
    assert_completed(
        &event(run, "approved", "ordinary checked approval"),
        &gate.replace("-review", "-adversarial-review"),
        gate,
    );
}
fn challenge(
    run: &RunFixture,
    gate: &str,
    subject: &str,
    revision: &str,
    exit: &str,
    target: &str,
) {
    let packet = provider(
        run,
        &["commission", "--slot", gate, "--stage", "aggregate"],
        &show(run, "challenge commission"),
    );
    let text = packet["commission"]["guidance"]
        .as_str()
        .expect("public challenge guidance");
    for part in [
        "passing or sufficiency conclusion",
        "individual findings to rebut",
        "first recorded",
        "evidence-applicability",
        "revise",
    ] {
        assert!(text.contains(part), "{packet}");
    }
    if run.admission_probe {
        ac5_refuse_late_ordinary_candidates(run, gate, subject, revision);
    }
    let rows = evidence(
        run,
        gate,
        subject,
        revision,
        "aggregate",
        &format!("{gate}-challenge"),
    );
    if run.admission_probe {
        ac5_challenge_accepted_controls(run, gate, subject, revision, &rows);
    }
    ledger(run, gate, subject, revision, gate);
    if gate == "intent-adversarial-review" {
        capture_intent_challenge(run, exit);
    }
    assert_completed(
        &event(run, exit, "challenge checked approval"),
        target,
        gate,
    );
}
// Exact first-attempt request and context retained for external old/new
// differential inspection. Portable CI uses current source only, never an
// installed predecessor executable. Capture before the public checked event.
fn capture_intent_challenge(run: &RunFixture, exit: &str) {
    let shown = show(run, "capture exact intent challenge context");
    assert_eq!(
        shown["result"]["current_state"],
        "intent-adversarial-review"
    );
    let workflow = &shown["result"]["workflow_graph"];
    let transition = workflow["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["source"] == "intent-adversarial-review" && t["event"] == exit)
        .unwrap();
    let request = json!({"operation":"evaluate","workflow":workflow,
        "initial_input":shown["result"]["initial_input"],"context":shown["result"]["context"],
        "transition":transition,"prior_evaluations":[]});
    let root = run.database.parent().unwrap();
    write_json(&root.join("intent-challenge-full-show.json"), &shown);
    write_json(&root.join("intent-challenge-request.json"), &request);
    let result = provider(run, &[], &request);
    write_json(&root.join("intent-challenge-current-result.json"), &result);
    assert_eq!(
        result["result"], "allow",
        "captured current-source evaluation: {result}"
    );
}

fn narrative(
    run: &RunFixture,
    phase: &str,
    artifact: &str,
    revision: &str,
    aggregate_first: bool,
    target: &str,
) {
    use sha2::{Digest, Sha256};
    let shown = show(run, "narrative approval facts");
    let approval = shown["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r["kind"] == "approved-artifact" && r["data"]["subject"] == artifact)
        .expect("approved artifact");
    let markdown =
        format!("# {phase}\nScripted narrative of the approved fixture; mechanics only.\n");
    let markdown_file = format!("{phase}-narrative.md");
    fs::write(run.artifacts.join(&markdown_file), &markdown).unwrap();
    let binding = format!("{phase}-narrative.json");
    let composite = format!("{revision}/1");
    write_json(
        &run.artifacts.join(&binding),
        &json!({"revision":composite,"author":{"name":"scripted-narrator","kind":"script"},
        "artifact":{"subject":artifact,"revision":revision,"sha256":format!("{:x}",Sha256::digest(fs::read(run.artifacts.join(artifact)).unwrap()))},
        "approved_artifact_record_id":approval["id"],"markdown":{"file":markdown_file,"sha256":format!("{:x}",Sha256::digest(markdown.as_bytes()))}}),
    );
    assert_completed(
        &event(run, "narrative-ready", "narrative admission"),
        &format!("{phase}-narrative-review"),
        "narrative",
    );
    ordinary(
        run,
        &format!("{phase}-narrative-review"),
        &binding,
        &composite,
        aggregate_first,
    );
    challenge(
        run,
        &format!("{phase}-narrative-adversarial-review"),
        &binding,
        &composite,
        if phase == "validation" {
            "passed"
        } else {
            "approved"
        },
        target,
    );
}

#[test]
fn challenge_clearance_all_eight_gates_both_ordinary_append_orders() {
    for aggregate_first in [false, true] {
        let run = setup(
            if aggregate_first {
                "aggregate-first"
            } else {
                "individual-first"
            },
            true,
        );
        for (phase, ready, subject, target) in [
            ("intent", "intent-ready", "intent.json", "design"),
            ("design", "design-ready", "design.json", "design-narrative"),
            ("plan", "plan-ready", "plan.json", "plan-narrative"),
            (
                "implementation",
                "reconciliation-ready",
                "implementation-report.json",
                "validation",
            ),
            (
                "validation",
                "validation-ready",
                "validation-report.json",
                "validation-narrative",
            ),
        ] {
            let revision = if phase == "validation" {
                prepare_validation(&run)
            } else {
                "r15".into()
            };
            if phase == "implementation" {
                checkpoint(&run, "implementation");
                assert_completed(
                    &event(&run, "implementation-ready", "implementation handoff"),
                    "reconciliation",
                    "handoff",
                );
            }
            assert_completed(
                &event(&run, ready, "checked artifact admission"),
                &format!("{phase}-review"),
                phase,
            );
            ordinary(
                &run,
                &format!("{phase}-review"),
                subject,
                &revision,
                aggregate_first,
            );
            challenge(
                &run,
                &format!("{phase}-adversarial-review"),
                subject,
                &revision,
                "approved",
                target,
            );
            if ["design", "plan", "validation"].contains(&phase) {
                narrative(
                    &run,
                    phase,
                    subject,
                    &revision,
                    aggregate_first,
                    match phase {
                        "design" => "plan",
                        "plan" => "implement",
                        _ => "end",
                    },
                );
            }
        }
        assert_eq!(
            show(&run, "terminal observation")["result"]["current_state"],
            "end"
        );
    }
}

fn carry(run: &RunFixture, id: &str, origin: &str, subject: &str, revision: &str) {
    assert_eq!(
        append(
            run,
            "evidence-applicability",
            id,
            json!({"origin":{"kind":"context-record","id":origin},"target":{"subject":subject,"revision":revision,"checkpoint":null},"attesting_driver":{"name":"scripted-driver","kind":"script"},"reason":"Explicit scripted applicability of unchanged judging scope."}),
            "applicability"
        )["status"],
        "completed"
    );
}

#[test]
fn challenge_clearance_issue_18_carry_and_fresh_after_carry_before_challenge() {
    for (label, aggregate_first, fresh) in [
        ("carry-after-aggregate", true, false),
        ("carry-after-aggregate-fresh", true, true),
        ("carry-before-aggregate", false, false),
    ] {
        let run = setup(label, false);
        // Retain the exact scripted prior subject bytes as qualified owner context.
        let current = fs::read(run.artifacts.join("intent.json")).unwrap();
        let mut old: Value = serde_json::from_slice(&current).unwrap();
        old["revision"] = json!("r14");
        write_json(&run.artifacts.join("intent.json"), &old);
        let old_bytes = fs::read_to_string(run.artifacts.join("intent.json")).unwrap();
        let origins = evidence(
            &run,
            "intent-review",
            "intent.json",
            "r14",
            "individual",
            "old-individual",
        );
        fs::write(run.artifacts.join("intent.json"), current).unwrap();
        use sha2::{Digest, Sha256};
        append(
            &run,
            "user-steering",
            "prior-subject-owner-source",
            json!({
                "target":{"kind":"all"},"instruction":"Scripted owner permits unchanged subject judging scope at r15.",
                "intent_baseline":{"revision":"r14","locator":"scripted prior intent.json", "sha256":format!("sha256:{:x}",Sha256::digest(old_bytes.as_bytes())),"json_bytes":old_bytes}
            }),
            "exact prior owner baseline",
        );
        assert_completed(
            &event(&run, "intent-ready", "intent ready"),
            "intent-review",
            "ready",
        );
        if aggregate_first {
            evidence(
                &run,
                "intent-review",
                "intent.json",
                "r15",
                "aggregate",
                "current-aggregate",
            );
        }
        for (id, _) in origins {
            carry(&run, &format!("carry-{id}"), &id, "intent.json", "r15");
        }
        if fresh {
            evidence(
                &run,
                "intent-review",
                "intent.json",
                "r15",
                "individual",
                "fresh-after-carry",
            );
        }
        if !aggregate_first {
            evidence(
                &run,
                "intent-review",
                "intent.json",
                "r15",
                "aggregate",
                "current-aggregate",
            );
        }
        ledger(&run, "intent-review", "intent.json", "r15", "ordinary");
        assert_completed(
            &event(&run, "approved", "ordinary carried approval"),
            "intent-adversarial-review",
            "ordinary",
        );
        challenge(
            &run,
            "intent-adversarial-review",
            "intent.json",
            "r15",
            "approved",
            "design",
        );
    }
}

#[test]
fn challenge_clearance_early_identity_raw_reappend_and_later_applicability() {
    let run = setup("early", false);
    let early = evidence(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "aggregate",
        "early-challenge",
    );
    assert_completed(
        &event(&run, "intent-ready", "intent ready"),
        "intent-review",
        "ready",
    );
    ordinary(&run, "intent-review", "intent.json", "r15", true);
    ledger(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "challenge",
    );
    let denied = event(&run, "approved", "early assessment denial");
    assert_eq!(denied["status"], "rejected", "{denied}");
    let text = denied.to_string();
    for (id, _) in &early {
        assert!(text.contains(id), "{denied}");
    }
    for part in [
        "assessment:",
        "ordinary_record_id",
        "intent-review-individual",
        "fresh challenge",
        "evidence-applicability",
    ] {
        assert!(text.contains(part), "{denied}");
    }
    for (id, data) in &early {
        append(
            &run,
            "review-evidence",
            &format!("copy-{id}"),
            data.clone(),
            "raw reappend",
        );
    }
    let denied = event(&run, "approved", "copied assessment denial");
    assert_eq!(denied["status"], "rejected", "{denied}");
    assert!(
        denied.to_string().contains("copy-early-challenge"),
        "{denied}"
    );
    for (id, _) in early {
        carry(
            &run,
            &format!("applicable-{id}"),
            &format!("copy-{id}"),
            "intent.json",
            "r15",
        );
    }
    assert_completed(
        &event(&run, "approved", "explicit later applicability"),
        "design",
        "carried challenge",
    );
}

#[test]
fn challenge_clearance_current_ledger_and_challenge_coverage_negatives() {
    let run = setup("safeguards", false);
    assert_completed(
        &event(&run, "intent-ready", "intent ready"),
        "intent-review",
        "ready",
    );
    ordinary(&run, "intent-review", "intent.json", "r15", true);
    // Missing challenge ledger is refused on the public path.
    let missing = event(&run, "approved", "missing challenge ledger");
    assert_eq!(
        missing["code"], "software-change-finding-ledger-invalid",
        "{missing}"
    );
    ledger(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "challenge",
    );
    assert_eq!(
        event(&run, "approved", "missing challenge axes/authors")["code"],
        "software-change-review-incomplete"
    );
    evidence(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r14",
        "aggregate",
        "stale-challenge",
    );
    let stale = event(&run, "approved", "stale challenge denial");
    assert_eq!(stale["status"], "rejected", "{stale}");
    assert!(
        stale.to_string().contains("\"category\":\"stale\""),
        "{stale}"
    );
    evidence(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "aggregate",
        "fresh-challenge",
    );
    // Joined prevention refuses malformed/late ordinary writes instead of
    // poisoning the already cleared parent snapshot. These are write controls,
    // not fabricated missing-history states.
    assert_write_refused(
        &run,
        "finding-ledger",
        "missing-current-ordinary",
        json!({"gate":"intent-review","subject":"intent.json"}),
        Some("ledger-shape-source-continuity"),
        None,
    );
    let stale_ledger = json!({
        "schema_version":"1","gate":"intent-review","subject":"intent.json",
        "subject_revision":"r14","author":{"name":"driver","kind":"agent"},"findings":[]
    });
    assert_write_refused(
        &run,
        "finding-ledger",
        "stale-ordinary-candidate",
        stale_ledger,
        None,
        None,
    );
    // True current-revision drift leaves the immutable ordinary ledger stale.
    // Refresh only challenge evidence/ledger at the new revision, so the
    // ordinary-side check (not the challenge ledger) supplies the denial.
    let original = fs::read(run.artifacts.join("intent.json")).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    changed["revision"] = json!("r16");
    write_json(&run.artifacts.join("intent.json"), &changed);
    ledger(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r16",
        "drift-challenge",
    );
    evidence(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r16",
        "aggregate",
        "drift-challenge",
    );
    let stale = event(&run, "approved", "actual ordinary ledger revision drift");
    assert_eq!(stale["status"], "rejected", "{stale}");
    assert_eq!(
        stale["message"], "challenge-before-ordinary-clearance",
        "{stale}"
    );
    assert_eq!(
        stale["details"]["ordinary_ledger"]["status"], "stale_subject",
        "{stale}"
    );
    fs::write(run.artifacts.join("intent.json"), original).unwrap();
    ledger(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "restored-challenge",
    );
    evidence(
        &run,
        "intent-adversarial-review",
        "intent.json",
        "r15",
        "aggregate",
        "restored-challenge",
    );
    assert_completed(
        &event(&run, "approved", "refused candidates did not poison parent"),
        "design",
        "current valid ledger",
    );

    // An accepted unresolved ordinary finding is a valid ordinary write, but
    // cannot enter challenge through normal approval. Parent-ledger missing/
    // unresolved challenge anomalies remain covered by the retained pure
    // option_evaluation regression; no direct DB/state rewrite is introduced.
    ordinary_unresolved_blocks_challenge_entry();
}

fn ordinary_unresolved_blocks_challenge_entry() {
    let run = setup("ordinary-unresolved", false);
    assert_completed(
        &event(&run, "intent-ready", "unresolved fixture admission"),
        "intent-review",
        "ordinary",
    );
    evidence(
        &run,
        "intent-review",
        "intent.json",
        "r15",
        "individual",
        "unresolved-individual",
    );
    evidence(
        &run,
        "intent-review",
        "intent.json",
        "r15",
        "aggregate",
        "unresolved-pass-controls",
    );
    let fault = "Scripted unresolved defect";
    let judgments: Vec<Value> = ["clearance", "companion", "aggregate-only"]
        .iter()
        .map(|axis| {
            json!({
                "axis":axis,"result":if *axis == "clearance" {"fail"} else {"pass"},
                "findings":if *axis == "clearance" {fault} else {""},
                "grounds":{"reason":"Synthetic complete-group negative control.",
                "evidence":[{"locator":"intent.json#/revision"}]}
            })
        })
        .collect();
    let original = json!({
        "review_contract_version":3,"review_stage":"aggregate",
        "author":{"name":"synthetic-negative-reviewer","kind":"script"},
        "assessment_id":"unresolved-complete-group","judgments":judgments
    });
    let returned = provider(
        &run,
        &["review-candidates"],
        &json!({
            "show":show(&run,"negative backend observation"),
            "assignment":{"gate":"intent-review","stage":"aggregate",
                "group":"intent-review.aggregate",
                "author":{"name":"synthetic-negative-reviewer","kind":"script"},
                "subject":"intent.json","subject_revision":"r15"},
            "request":{"text":"Synthetic negative fixture: inspect the complete group."},
            "original":{"text":original.to_string()}
        }),
    );
    assert_eq!(
        returned["records"].as_array().unwrap().len(),
        3,
        "{returned}"
    );
    let mut failed_id = None;
    for (i, row) in returned["records"].as_array().unwrap().iter().enumerate() {
        let id = format!("unresolved-group-{i}");
        assert_eq!(
            append(
                &run,
                "review-evidence",
                &id,
                row["data"].clone(),
                "actual scripted fail"
            )["status"],
            "completed"
        );
        if row["data"]["result"] == "fail" {
            failed_id = Some(id);
        }
    }
    let failed_id = failed_id.unwrap();
    let unresolved = json!({
        "schema_version":"1","gate":"intent-review","subject":"intent.json",
        "subject_revision":"r15","author":{"name":"driver","kind":"agent"},
        "findings":[{"id":"F-defect","source":{"kind":"context-record","id":failed_id},
            "policy_id":"clearance","statement":fault,"disposition":"accepted",
            "reason":"Synthetic accepted ordinary defect","owner_phase":"intent",
            "task_ids":[],"review_axes":["clearance"],"status":"unresolved"}]
    });
    assert_eq!(
        append(
            &run,
            "finding-ledger",
            "unresolved-ordinary",
            unresolved,
            "unresolved ledger is admitted in ordinary"
        )["status"],
        "completed"
    );
    let denied = event(
        &run,
        "approved",
        "ordinary finding prevents challenge entry",
    );
    assert_eq!(denied["status"], "rejected", "{denied}");
    assert_eq!(
        denied["code"], "software-change-finding-ledger-invalid",
        "{denied}"
    );
    assert_eq!(
        denied["details"]["status"], "accepted_unresolved",
        "{denied}"
    );
    assert_eq!(
        show(&run, "still ordinary")["result"]["current_state"],
        "intent-review"
    );
}

// Joined direct-append proof: bookends:LE-194 bookends:LE-98.
// Fixtures use synthetic actors/returns, never real owner/reviewer approvals.
fn public_history(run: &RunFixture) -> Value {
    engine_call(run, &["history".into(), run.run_id.clone()], "history")["result"].clone()
}

fn snapshot(run: &RunFixture) -> Value {
    let shown = show(run, "snapshot context");
    json!({
        "state": shown["result"]["current_state"],
        "context": shown["result"]["context"],
        "history": public_history(run)
    })
}

fn assert_write_refused(
    run: &RunFixture,
    kind: &str,
    id: &str,
    data: Value,
    rule: Option<&str>,
    ordinary_gate: Option<&str>,
) {
    let before = snapshot(run);
    let result = append(run, kind, id, data, "negative candidate append");
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["code"], "record-refused", "{result}");
    assert_eq!(result["details"]["record_id"], id, "{result}");
    if let Some(rule) = rule {
        assert_eq!(result["details"]["rule"], rule, "{result}");
    }
    if let Some(parent) = ordinary_gate {
        let message = result["details"]["message"].as_str().unwrap();
        for part in [id, parent, "revise"] {
            assert!(message.contains(part), "{result}");
        }
    }
    assert_eq!(
        snapshot(run),
        before,
        "refusal changed state/context/history"
    );
    assert!(!show(run, "candidate absence")["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == id));
}

fn applicability_data(origin: &str, subject: &str, revision: &str) -> Value {
    let checkpoint = match subject {
        "implementation-report.json" => {
            json!({"phase":"implementation","report_revision":revision})
        }
        "validation-report.json" => json!({"phase":"validation","report_revision":revision}),
        _ => Value::Null,
    };
    json!({
        "origin":{"kind":"context-record","id":origin},
        "target":{"subject":subject,"revision":revision,"checkpoint":checkpoint},
        "attesting_driver":{"name":"synthetic-admission-driver","kind":"agent"},
        "reason":"Mechanical fixture: the complete unchanged assessment remains applicable."
    })
}

fn no_op_count(gate: &str) -> Value {
    json!({
        "targets":[{"gate":gate,"stage":"aggregate","axis":"clearance"}],
        "before":2,"after":2,
        "actor":{"name":"synthetic-fixture-driver","kind":"script"},
        "authority":{"kind":"standing-delegation","reference":"isolated mechanical fixture"},
        "reason":"Fixture count is unchanged; this is not a production owner amendment."
    })
}

fn comparison_data(run: &RunFixture) -> Value {
    let shown = show(run, "current reconstruction sources");
    let intent: Value =
        serde_json::from_slice(&fs::read(run.artifacts.join("intent.json")).unwrap()).unwrap();
    let ids: Vec<String> = shown["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["kind"] == "review-evidence"
                && r["data"]["gate"] == "intent-review"
                && r["data"]["policy_id"] == "intent-sufficient"
                && r["data"]["subject_revision"] == intent["revision"]
        })
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2, "fixture must retain both cold returns");
    let dispositions: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({
                "reconstruction_id":id,"alternative":"Fixture encoding freedom",
                "disposition":"implementation-freedom",
                "note":"Synthetic mechanical comparison, not Michael's real approval."
            })
        })
        .collect();
    json!({
        "subject_revision":intent["revision"],"baseline":intent["baseline"],
        "reconstruction_ids":ids,
        "actor":{"name":"synthetic-comparison-driver","kind":"agent"},
        "authority":"isolated-fixture-authorization",
        "dispositions":dispositions
    })
}

fn ac5_prepare_ordinary_candidates(run: &RunFixture, gate: &str, subject: &str, revision: &str) {
    let shown = show(run, "ordinary allowed controls");
    let source = shown["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| {
            r["kind"] == "review-evidence"
                && r["data"]["gate"] == gate
                && r["data"]["policy_id"] == "clearance"
                && r["data"]["review_stage"] == "aggregate"
        })
        .unwrap();
    let data = source["data"].clone();
    let repeat_id = format!("ac5-{gate}-ordinary-repeat");
    assert_eq!(
        append(
            run,
            "review-evidence",
            &repeat_id,
            data.clone(),
            "ordinary evidence remains permitted"
        )["status"],
        "completed"
    );
    let amendment = no_op_count(gate);
    assert_eq!(
        append(
            run,
            "author-count-amendment",
            &format!("ac5-{gate}-ordinary-count"),
            amendment.clone(),
            "ordinary count control"
        )["status"],
        "completed"
    );
    let carry = applicability_data(&repeat_id, subject, revision);
    // Carry the complete combined assessment for this author, not isolated
    // rows from a different assessment. The replayed clearance row is latest.
    for row in shown["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["kind"] == "review-evidence"
                && r["data"]["gate"] == gate
                && r["data"]["review_stage"] == "aggregate"
                && r["data"]["group"] == format!("{gate}.aggregate")
                && r["data"]["author"] == source["data"]["author"]
        })
    {
        let origin = if row["data"]["policy_id"] == "clearance" {
            repeat_id.as_str()
        } else {
            row["id"].as_str().unwrap()
        };
        assert_eq!(
            append(
                run,
                "evidence-applicability",
                &format!(
                    "ac5-{gate}-ordinary-carry-{}",
                    row["data"]["policy_id"].as_str().unwrap()
                ),
                applicability_data(origin, subject, revision),
                "ordinary carry control"
            )["status"],
            "completed"
        );
    }
    let ledger_data = show(run, "ordinary ledger source")["result"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r["kind"] == "finding-ledger" && r["data"]["gate"] == gate)
        .unwrap()["data"]
        .clone();
    assert_eq!(
        append(
            run,
            "finding-ledger",
            &format!("ac5-{gate}-ordinary-ledger"),
            ledger_data.clone(),
            "ordinary ledger control"
        )["status"],
        "completed"
    );
    let mut candidates = vec![
        json!({"kind":"review-evidence","data":data}),
        json!({"kind":"finding-ledger","data":ledger_data}),
        json!({"kind":"author-count-amendment","data":amendment}),
        json!({"kind":"evidence-applicability","data":carry}),
    ];
    if gate == "intent-review" {
        let comparison = comparison_data(run);
        assert_eq!(
            append(
                run,
                "intent-comparison",
                "ac5-ordinary-comparison",
                comparison.clone(),
                "ordinary comparison control"
            )["status"],
            "completed"
        );
        candidates.push(json!({"kind":"intent-comparison","data":comparison}));
    }
    write_json(
        &run.database
            .parent()
            .unwrap()
            .join(format!("{gate}.ac5.json")),
        &json!(candidates),
    );
}

fn ac5_refuse_late_ordinary_candidates(
    run: &RunFixture,
    gate: &str,
    subject: &str,
    revision: &str,
) {
    let parent = format!(
        "{}-review",
        gate.strip_suffix("-adversarial-review").unwrap()
    );
    let cases: Value = serde_json::from_slice(
        &fs::read(
            run.database
                .parent()
                .unwrap()
                .join(format!("{parent}.ac5.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        cases.as_array().unwrap().len(),
        if parent == "intent-review" { 5 } else { 4 }
    );
    for row in cases.as_array().unwrap() {
        let kind = row["kind"].as_str().unwrap();
        assert_write_refused(
            run,
            kind,
            &format!("ac5-late-{gate}-{kind}"),
            row["data"].clone(),
            Some("b-ordinary-during-challenge"),
            Some(&parent),
        );
    }

    // Preview must use the same Write dispatcher; source inspection alone
    // does not prove the helper or direct append boundary.
    let shown = show(run, "late ordinary preview");
    let group = format!("{parent}.aggregate");
    let axes: Vec<Value> = shown["result"]["initial_input"]["review_policies"][&parent]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["group"] == group && p["review_stage"] == "aggregate")
        .map(|p| {
            json!({"axis":p["id"],"result":"pass","findings":"",
            "grounds":{"reason":"Synthetic late return, otherwise valid.",
            "evidence":[{"locator":format!("{subject}#/revision")}]}})
        })
        .collect();
    assert_eq!(axes.len(), 3);
    let original = json!({
        "review_contract_version":3,"review_stage":"aggregate",
        "author":{"name":"synthetic-late-reviewer","kind":"script"},
        "assessment_id":format!("ac5-preview-{gate}"),"judgments":axes
    });
    let preview = provider(
        run,
        &["review-candidates"],
        &json!({
            "show":shown,"assignment":{"gate":parent,"stage":"aggregate","group":group,
                "author":{"name":"synthetic-late-reviewer","kind":"script"},
                "subject":subject,"subject_revision":revision},
            "request":{"text":"Synthetic fixture: complete ordinary group return."},
            "original":{"text":original.to_string()}
        }),
    );
    assert!(
        preview["records"].as_array().unwrap().is_empty(),
        "{preview}"
    );
    assert_eq!(preview["items"].as_array().unwrap().len(), 3);
    for item in preview["items"].as_array().unwrap() {
        assert!(
            item["diagnostics"]
                .to_string()
                .contains("b-ordinary-during-challenge"),
            "{preview}"
        );
    }
    assert_eq!(
        snapshot(run)["context"],
        shown["result"]["context"],
        "helper changed durable context"
    );
}

fn ac5_challenge_accepted_controls(
    run: &RunFixture,
    gate: &str,
    subject: &str,
    revision: &str,
    rows: &[(String, Value)],
) {
    let source = rows
        .iter()
        .find(|(_, d)| d["policy_id"] == "clearance" && d["author"]["name"] == "script-reviewer-0")
        .unwrap();
    let repeat_id = format!("ac5-{gate}-challenge-repeat");
    assert_eq!(
        append(
            run,
            "review-evidence",
            &repeat_id,
            source.1.clone(),
            "challenge evidence remains permitted"
        )["status"],
        "completed"
    );
    for (origin_id, data) in rows.iter().filter(|(_, d)| {
        d["group"] == format!("{gate}.aggregate") && d["author"]["name"] == "script-reviewer-0"
    }) {
        let origin = if data["policy_id"] == "clearance" {
            repeat_id.as_str()
        } else {
            origin_id.as_str()
        };
        assert_eq!(
            append(
                run,
                "evidence-applicability",
                &format!(
                    "ac5-{gate}-challenge-carry-{}",
                    data["policy_id"].as_str().unwrap()
                ),
                applicability_data(origin, subject, revision),
                "challenge carry control"
            )["status"],
            "completed"
        );
    }
    assert_eq!(
        append(
            run,
            "author-count-amendment",
            &format!("ac5-{gate}-challenge-count"),
            no_op_count(gate),
            "challenge count control"
        )["status"],
        "completed"
    );
    let parent = format!(
        "{}-review",
        gate.strip_suffix("-adversarial-review").unwrap()
    );
    let unrelated = if parent == "intent-review" {
        "design-review"
    } else {
        "intent-review"
    };
    assert_eq!(
        append(
            run,
            "author-count-amendment",
            &format!("ac5-{gate}-unrelated-count"),
            no_op_count(unrelated),
            "unrelated gate remains permitted"
        )["status"],
        "completed"
    );
    if gate != "intent-adversarial-review" {
        assert_eq!(
            append(
                run,
                "intent-comparison",
                &format!("ac5-{gate}-unrelated-comparison"),
                comparison_data(run),
                "comparison belongs only to intent ordinary gate"
            )["status"],
            "completed"
        );
    }
}

#[test]
fn challenge_clearance_admission_all_eight_gates_and_all_five_kinds() {
    let run = setup("admission-all-eight", true);
    for (phase, ready, subject, target) in [
        ("intent", "intent-ready", "intent.json", "design"),
        ("design", "design-ready", "design.json", "design-narrative"),
        ("plan", "plan-ready", "plan.json", "plan-narrative"),
        (
            "implementation",
            "reconciliation-ready",
            "implementation-report.json",
            "validation",
        ),
        (
            "validation",
            "validation-ready",
            "validation-report.json",
            "validation-narrative",
        ),
    ] {
        let revision = if phase == "validation" {
            prepare_validation(&run)
        } else {
            "r15".into()
        };
        if phase == "implementation" {
            checkpoint(&run, "implementation");
            assert_completed(
                &event(&run, "implementation-ready", "implementation handoff"),
                "reconciliation",
                "handoff",
            );
        }
        assert_completed(
            &event(&run, ready, "checked admission"),
            &format!("{phase}-review"),
            phase,
        );
        ordinary(&run, &format!("{phase}-review"), subject, &revision, true);
        let observed = show(&run, "frozen admission capability");
        assert_eq!(observed["result"]["workflow_graph"]["admit"], true);
        assert_eq!(
            observed["result"]["current_state"],
            format!("{phase}-adversarial-review")
        );
        challenge(
            &run,
            &format!("{phase}-adversarial-review"),
            subject,
            &revision,
            "approved",
            target,
        );
        if ["design", "plan", "validation"].contains(&phase) {
            narrative(
                &run,
                phase,
                subject,
                &revision,
                true,
                match phase {
                    "design" => "plan",
                    "plan" => "implement",
                    _ => "end",
                },
            );
        }
    }
    assert_eq!(
        show(&run, "terminal admission proof")["result"]["current_state"],
        "end"
    );
}
