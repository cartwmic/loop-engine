use super::bounded_process::CommandExt;
use super::support;

use loop_core::{OperationOutcome, TransitionKind};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::{metadata_schema, provider_binary, valid_metadata, Engine, TestDir};

const EXPECTED_STATES: &[(&str, bool)] = &[
    ("explore", false),
    ("intent-review", false),
    ("intent-adversarial-review", false),
    ("design", false),
    ("design-review", false),
    ("design-adversarial-review", false),
    ("plan", false),
    ("plan-review", false),
    ("plan-adversarial-review", false),
    ("implement", false),
    ("implementation-review", false),
    ("implementation-adversarial-review", false),
    ("validation", false),
    ("validation-review", false),
    ("validation-adversarial-review", false),
    ("end", true),
];

const OWNING_PHASE_ROUTES: &[(&str, &str, &str)] = &[
    ("design-review", "revise-intent", "explore"),
    ("plan-review", "revise-design", "design"),
    ("plan-review", "revise-intent", "explore"),
    ("implementation-review", "revise-plan", "plan"),
    ("implementation-review", "revise-design", "design"),
    ("implementation-review", "revise-intent", "explore"),
    ("validation-review", "revise-implementation", "implement"),
    ("validation-review", "revise-plan", "plan"),
    ("validation-review", "revise-design", "design"),
    ("validation-review", "revise-intent", "explore"),
];

const EXPECTED_TRANSITIONS: &[(&str, &str, &str, &str)] = &[
    ("explore", "intent-ready", "intent-review", "checked"),
    (
        "intent-review",
        "approved",
        "intent-adversarial-review",
        "checked",
    ),
    ("intent-review", "revise", "explore", "check-free"),
    ("intent-adversarial-review", "approved", "design", "checked"),
    (
        "intent-adversarial-review",
        "revise",
        "explore",
        "check-free",
    ),
    ("design", "design-ready", "design-review", "checked"),
    (
        "design-review",
        "approved",
        "design-adversarial-review",
        "checked",
    ),
    ("design-review", "revise", "design", "check-free"),
    ("design-review", "revise-intent", "explore", "check-free"),
    ("design-adversarial-review", "approved", "plan", "checked"),
    (
        "design-adversarial-review",
        "revise",
        "design",
        "check-free",
    ),
    (
        "design-adversarial-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
    ("plan", "plan-ready", "plan-review", "checked"),
    (
        "plan-review",
        "approved",
        "plan-adversarial-review",
        "checked",
    ),
    ("plan-review", "revise", "plan", "check-free"),
    ("plan-review", "revise-design", "design", "check-free"),
    ("plan-review", "revise-intent", "explore", "check-free"),
    (
        "plan-adversarial-review",
        "approved",
        "implement",
        "checked",
    ),
    ("plan-adversarial-review", "revise", "plan", "check-free"),
    (
        "plan-adversarial-review",
        "revise-design",
        "design",
        "check-free",
    ),
    (
        "plan-adversarial-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
    (
        "implement",
        "implementation-ready",
        "implementation-review",
        "checked",
    ),
    ("implement", "revise-plan", "plan", "check-free"),
    ("implement", "revise-design", "design", "check-free"),
    ("implement", "revise-intent", "explore", "check-free"),
    (
        "implementation-review",
        "approved",
        "implementation-adversarial-review",
        "checked",
    ),
    ("implementation-review", "revise", "implement", "check-free"),
    ("implementation-review", "revise-plan", "plan", "check-free"),
    (
        "implementation-review",
        "revise-design",
        "design",
        "check-free",
    ),
    (
        "implementation-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
    (
        "implementation-adversarial-review",
        "approved",
        "validation",
        "checked",
    ),
    (
        "implementation-adversarial-review",
        "revise",
        "implement",
        "check-free",
    ),
    (
        "implementation-adversarial-review",
        "revise-plan",
        "plan",
        "check-free",
    ),
    (
        "implementation-adversarial-review",
        "revise-design",
        "design",
        "check-free",
    ),
    (
        "implementation-adversarial-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
    (
        "validation",
        "validation-ready",
        "validation-review",
        "checked",
    ),
    (
        "validation",
        "revise-implementation",
        "implement",
        "check-free",
    ),
    (
        "validation-review",
        "approved",
        "validation-adversarial-review",
        "checked",
    ),
    ("validation-review", "revise", "validation", "check-free"),
    (
        "validation-review",
        "revise-implementation",
        "implement",
        "check-free",
    ),
    ("validation-review", "revise-plan", "plan", "check-free"),
    ("validation-review", "revise-design", "design", "check-free"),
    (
        "validation-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
    ("validation-adversarial-review", "passed", "end", "checked"),
    (
        "validation-adversarial-review",
        "revise",
        "validation",
        "check-free",
    ),
    (
        "validation-adversarial-review",
        "revise-implementation",
        "implement",
        "check-free",
    ),
    (
        "validation-adversarial-review",
        "revise-plan",
        "plan",
        "check-free",
    ),
    (
        "validation-adversarial-review",
        "revise-design",
        "design",
        "check-free",
    ),
    (
        "validation-adversarial-review",
        "revise-intent",
        "explore",
        "check-free",
    ),
];

fn assert_expected_topology(workflow: &Value) {
    assert_eq!(workflow["initial_state"], "explore");

    let actual_states: Vec<(String, bool)> = workflow["states"]
        .as_array()
        .expect("workflow states")
        .iter()
        .map(|state| {
            (
                state["id"].as_str().expect("state id").to_owned(),
                state["final"].as_bool().expect("state final flag"),
            )
        })
        .collect();
    let expected_states: Vec<(String, bool)> = EXPECTED_STATES
        .iter()
        .map(|(id, is_final)| ((*id).to_owned(), *is_final))
        .collect();
    assert_eq!(actual_states, expected_states);

    let actual_transitions: Vec<(String, String, String, String)> = workflow["transitions"]
        .as_array()
        .expect("workflow transitions")
        .iter()
        .map(|transition| {
            (
                transition["source"]
                    .as_str()
                    .expect("transition source")
                    .to_owned(),
                transition["event"]
                    .as_str()
                    .expect("transition event")
                    .to_owned(),
                transition["target"]
                    .as_str()
                    .expect("transition target")
                    .to_owned(),
                transition["kind"]
                    .as_str()
                    .expect("transition kind")
                    .to_owned(),
            )
        })
        .collect();
    let expected_transitions: Vec<(String, String, String, String)> = EXPECTED_TRANSITIONS
        .iter()
        .map(|(source, event, target, kind)| {
            (
                (*source).to_owned(),
                (*event).to_owned(),
                (*target).to_owned(),
                (*kind).to_owned(),
            )
        })
        .collect();
    assert_eq!(actual_transitions, expected_transitions);
}

fn draft_events_to_review(source: &str) -> &'static [&'static str] {
    match source {
        "design-review" => &["intent-ready", "design-ready"],
        "plan-review" => &["intent-ready", "design-ready", "plan-ready"],
        "implementation-review" => &[
            "intent-ready",
            "design-ready",
            "plan-ready",
            "implementation-ready",
        ],
        "validation-review" => &[
            "intent-ready",
            "design-ready",
            "plan-ready",
            "implementation-ready",
            "validation-ready",
        ],
        other => panic!("no draft path to {other}"),
    }
}

fn subject_for_review(source: &str) -> &'static str {
    match source {
        "design-review" => "design.json",
        "plan-review" => "plan.json",
        "implementation-review" => "implementation-report.json",
        "validation-review" => "validation-report.json",
        other => panic!("no subject for {other}"),
    }
}

fn assert_owning_phase_route(index: usize) {
    assert_eq!(OWNING_PHASE_ROUTES.len(), 10);
    let (source, event, target) = OWNING_PHASE_ROUTES[index];
    let state = TestDir::new(&format!("a14-route-state-{index}"));
    let repository = state.path().join("repository");
    fs::create_dir_all(&repository).expect("create route repository");
    fs::write(repository.join("marker.txt"), b"baseline\n").expect("write route marker");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "software-change a14"],
        vec!["config", "user.email", "a14@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repository)
            .status()
            .expect("run git")
            .success());
    }
    let wrapper = state.path().join("provider-wrapper.py");
    fs::write(
            &wrapper,
            format!(
                "#!/usr/bin/env python3\nimport os\nos.chdir({repository:?})\nos.execv({provider:?}, [{provider:?}] + os.sys.argv[1:])\n",
                repository = repository.to_string_lossy(),
                provider = provider_binary().to_string_lossy(),
            ),
        )
        .expect("write provider wrapper");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("chmod provider wrapper");
    let engine = Engine::with_command(state.path().join("route.sqlite"), &wrapper);
    let run_id = format!("a14-route-{index}");
    let mut policies = serde_json::Map::new();
    policies.insert(
        source.to_owned(),
        json!([{"id": "axis", "description": "test axis", "review_stage": "aggregate"}]),
    );
    let subject = subject_for_review(source);
    let mut schemas = serde_json::Map::new();
    schemas.insert(
        subject.to_owned(),
        if source == "validation-review" {
            support::load_profile("minimal")["artifact_schemas"][subject].clone()
        } else {
            metadata_schema()
        },
    );
    engine.start_ok(
            &run_id,
            json!({"contract_version": 3, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
                "config_version": "a14-route-test",
                "review_policies": policies,
                "artifact_schemas": schemas
            }),
        );
    let shown = engine.show(&run_id);
    let root = shown.initial_input["artifact_root"]
        .as_str()
        .expect("allocated artifact_root");
    fs::write(
        Path::new(root).join(subject),
        serde_json::to_vec(&valid_metadata("1")).expect("serialize subject"),
    )
    .expect("write subject artifact");
    if matches!(source, "implementation-review" | "validation-review") {
        fs::write(Path::new(root).join("intent.json"), br#"{"revision":"1"}"#)
            .expect("write checkpoint intent");
        fs::write(Path::new(root).join("design.json"), br#"{"revision":"1"}"#)
            .expect("write checkpoint design");
        fs::write(Path::new(root).join("plan.json"), br#"{"revision":"1"}"#)
            .expect("write checkpoint plan");
        fs::write(
            Path::new(root).join("implementation-report.json"),
            serde_json::to_vec(&valid_metadata("1")).expect("serialize implementation report"),
        )
        .expect("write implementation report");
    }
    if source == "validation-review" {
        for name in [
            "intent",
            "design",
            "plan",
            "implementation-report",
            "validation-report",
        ] {
            let fixture = if name == "plan" {
                support::executable_fixture_plan()
            } else {
                support::load_fixture(&format!("{name}-good.json"))
            };
            fs::write(
                Path::new(root).join(format!("{name}.json")),
                serde_json::to_vec(&fixture).unwrap(),
            )
            .unwrap();
        }
    }
    let create_checkpoint = |phase: &str| {
        let output = Command::new(provider_binary())
            .args([
                "checkpoint",
                "--phase",
                phase,
                "--artifact-root",
                root,
                "--working-directory",
                repository.to_str().expect("repository path"),
            ])
            .current_dir(&repository)
            .bounded_output("software-change contract checkpoint")
            .expect("run checkpoint");
        assert!(
            output.status.success(),
            "checkpoint {phase} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    if matches!(source, "implementation-review" | "validation-review") {
        create_checkpoint("implementation");
    }
    if source == "validation-review" {
        create_checkpoint("validation");
    }

    for progress_event in draft_events_to_review(source) {
        if *progress_event == "validation-ready" {
            engine.append_candidates(
                &run_id,
                support::validation_fixture(&shown.initial_input, &repository),
            );
        }
        let outcome = engine.event(&run_id, progress_event);
        match outcome {
            OperationOutcome::Completed(_) => {}
            other => {
                panic!("expected checked progress {progress_event} to commit, got {other:?}")
            }
        }
    }
    assert_eq!(
        engine.current_state(&run_id).as_str(),
        source,
        "fresh run did not reach route source"
    );

    let shown = engine.show(&run_id);
    let routes: Vec<_> = shown
        .requestable_events
        .iter()
        .filter(|candidate| candidate.event.as_str() == event)
        .collect();
    assert_eq!(
        routes.len(),
        1,
        "route {source}/{event} not uniquely exposed"
    );
    let route = routes[0];
    assert_eq!(route.target.as_str(), target);
    assert_eq!(route.kind, TransitionKind::CheckFree);

    let outcome = engine.event(&run_id, event);
    let committed = match outcome {
        OperationOutcome::Completed(result) => result,
        other => panic!("expected {source}/{event} to commit, got {other:?}"),
    };
    assert_eq!(committed.run.current_state.as_str(), target);

    // `authoritative` and `show` each reopen persistence, proving target
    // survived the event call rather than only appearing in its response.
    assert_eq!(engine.authoritative(&run_id).current_state.as_str(), target);
    assert_eq!(engine.show(&run_id).current_state.as_str(), target);
}

#[test]
fn owning_phase_route_design_review_revise_intent() {
    assert_owning_phase_route(0);
}

#[test]
fn owning_phase_route_plan_review_revise_design() {
    assert_owning_phase_route(1);
}

#[test]
fn owning_phase_route_plan_review_revise_intent() {
    assert_owning_phase_route(2);
}

#[test]
fn owning_phase_route_implementation_review_revise_plan() {
    assert_owning_phase_route(3);
}

#[test]
fn owning_phase_route_implementation_review_revise_design() {
    assert_owning_phase_route(4);
}

#[test]
fn owning_phase_route_implementation_review_revise_intent() {
    assert_owning_phase_route(5);
}

#[test]
fn owning_phase_route_validation_review_revise_implementation() {
    assert_owning_phase_route(6);
}

#[test]
fn owning_phase_route_validation_review_revise_plan() {
    assert_owning_phase_route(7);
}

#[test]
fn owning_phase_route_validation_review_revise_design() {
    assert_owning_phase_route(8);
}

#[test]
fn owning_phase_route_validation_review_revise_intent() {
    assert_owning_phase_route(9);
}

#[test]
fn describe_matches_snapshot_and_engine_prd_reference_topology() {
    let output = support::invoke(serde_json::json!({"operation": "describe"}));
    support::assert_exit(&output, 0);
    assert_eq!(output.stdout, include_bytes!("../snapshots/describe.json"));
    let workflow: Value = support::response(&output);
    let snapshot: Value = serde_json::from_slice(include_bytes!("../snapshots/describe.json"))
        .expect("snapshot workflow JSON");

    // Snapshot equality catches guidance drift; these semantic assertions catch
    // a changed edge or final flag even if someone regenerates that snapshot.
    assert_expected_topology(&workflow);
    assert_expected_topology(&snapshot);

    let prd_path = workspace_integration::repository_root().join("docs/PRD.md");
    let prd = fs::read_to_string(&prd_path).expect("read engine PRD");
    for line in [
        "explore\n  └─ intent-ready [checked] → intent-review",
        "intent-review\n  ├─ approved [checked] → intent-adversarial-review",
        "  └─ revise [check-free] → explore",
        "intent-adversarial-review\n  ├─ approved [checked] → design",
        "design\n  └─ design-ready [checked] → design-review",
        "design-review\n  ├─ approved [checked] → design-adversarial-review",
        "  ├─ revise [check-free] → design",
        "  └─ revise-intent [check-free] → explore",
        "design-adversarial-review\n  ├─ approved [checked] → plan",
        "plan\n  └─ plan-ready [checked] → plan-review",
        "plan-review\n  ├─ approved [checked] → plan-adversarial-review",
        "  ├─ revise [check-free] → plan",
        "  ├─ revise-design [check-free] → design",
        "plan-adversarial-review\n  ├─ approved [checked] → implement",
        concat!(
            "implement\n",
            "  ├─ implementation-ready [checked] → implementation-review\n",
            "  ├─ revise-plan [check-free] → plan\n",
            "  ├─ revise-design [check-free] → design\n",
            "  └─ revise-intent [check-free] → explore",
        ),
        "implementation-review\n  ├─ approved [checked] → implementation-adversarial-review",
        "  ├─ revise [check-free] → implement",
        "  ├─ revise-plan [check-free] → plan",
        "implementation-adversarial-review\n  ├─ approved [checked] → validation",
        "validation\n  └─ validation-ready [checked] → validation-review",
        "validation-review\n  ├─ approved [checked] → validation-adversarial-review",
        "  ├─ revise [check-free] → validation",
        "  ├─ revise-implementation [check-free] → implement",
        "validation-adversarial-review\n  ├─ passed [checked] → end",
        "end [final]",
    ] {
        assert!(prd.contains(line), "PRD topology line missing: {line}");
    }
}

#[test]
fn review_states_expose_convergence_guidance() {
    let workflow: Value = serde_json::from_slice(include_bytes!("../snapshots/describe.json"))
        .expect("snapshot workflow JSON");
    let states = workflow["states"].as_array().expect("workflow states");
    for state_id in [
        "intent-review",
        "design-review",
        "plan-review",
        "implementation-review",
        "validation-review",
    ] {
        let state = states
            .iter()
            .find(|state| state["id"] == state_id)
            .unwrap_or_else(|| panic!("missing review state {state_id}"));
        let guidance = state["instructions"].as_str().expect("state guidance");
        for clause in [
            "triage candidate reviewer output before append or mutation",
            "focused external reconsideration",
            "owning phase",
        ] {
            assert!(
                guidance.to_ascii_lowercase().contains(clause),
                "{state_id} guidance missing convergence clause: {clause}"
            );
        }
    }

    let validation_review = states
        .iter()
        .find(|state| state["id"] == "validation-review")
        .expect("validation-review state")["instructions"]
        .as_str()
        .expect("validation-review guidance");
    let validation_review_lower = validation_review.to_ascii_lowercase();
    for clause in [
        "validation-report-local defects use nearest check-free `revise` back to the validation draft",
        "`revise-implementation` for implementation-owned defects",
        "`revise-plan` for plan-owned defects",
        "`revise-design` for design-owned defects",
        "`revise-intent` for intent-owned defects",
    ] {
        assert!(
            validation_review_lower.contains(clause),
            "validation-review guidance missing routing semantic: {clause}"
        );
    }

    let validation = states
        .iter()
        .find(|state| state["id"] == "validation")
        .expect("validation state")["instructions"]
        .as_str()
        .expect("validation guidance");
    let validation_lower = validation.to_ascii_lowercase();
    for clause in [
        "validation-report-local defects stay in this draft",
        "edit and recheck `validation-report.json`",
        "retry the next checked hop",
    ] {
        assert!(
            validation_lower.contains(clause),
            "validation draft guidance missing routing semantic: {clause}"
        );
    }
    assert!(
        !validation_lower.contains("revise for validation-report-local"),
        "validation draft guidance routes report-local corrections through revise"
    );
}

fn init_validation_repository(path: &Path) {
    fs::create_dir_all(path).expect("create validation repository");
    fs::write(path.join("marker.txt"), b"baseline\n").expect("write validation marker");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "software-change validation"],
        vec!["config", "user.email", "validation@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "baseline"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("run git")
                .success(),
            "git command failed"
        );
    }
}

fn mutate_first_validation_verdict(request: &Value, mutation: impl FnOnce(&mut Value)) -> Value {
    let mut request = request.clone();
    let verdict = request["context"]
        .as_array_mut()
        .expect("validation context")
        .iter_mut()
        .find(|record| record["kind"] == "criterion-verdict")
        .expect("criterion verdict");
    mutation(&mut verdict["data"]);
    request
}

fn validation_response(request: &Value, repository: &Path) -> Value {
    let output = support::invoke_in_dir(request.clone(), repository);
    support::assert_exit(&output, 0);
    support::response(&output)
}

fn assert_validation_denied(request: &Value, repository: &Path, code: &str) {
    let response = validation_response(request, repository);
    assert_eq!(response["result"], "deny");
    assert_eq!(response["feedback"]["code"], code, "{response}");
}

fn append_review_records(
    context: &mut Vec<Value>,
    gate: &str,
    subject: &str,
    revision: &str,
    config_version: &str,
    axes: &[&str],
) {
    let start = context.len() + 1;
    for (index, axis) in axes.iter().enumerate() {
        let sequence = start + index;
        context.push(json!({
            "id": format!("validation-cache-{gate}-{axis}"),
            "kind": "review-evidence",
            "data": support::evidence(
                gate,
                axis,
                "pass",
                "",
                &format!("validation-cache-reviewer-{gate}-{axis}"),
                "agent",
                subject,
                revision,
                config_version,
            ),
            "sequence": sequence,
            "created_at": sequence
        }));
    }
    let sequence = start + axes.len();
    context.push(json!({
        "id": format!("validation-cache-{gate}-ledger"),
        "kind": "finding-ledger",
        "data": support::finding_ledger(gate, subject, revision, json!([])),
        "sequence": sequence,
        "created_at": sequence
    }));
}

fn selected_validation_paths(context: &Value) -> (PathBuf, PathBuf) {
    let evidence = context["context"]
        .as_array()
        .expect("validation context")
        .iter()
        .find(|record| record["kind"] == "command-evidence")
        .expect("command evidence");
    let index_path = PathBuf::from(
        evidence["data"]["capture"]["index"]
            .as_str()
            .expect("capture index"),
    );
    let index: Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read validation capture index"))
            .expect("validation capture index JSON");
    let receipt = index["receipts"]
        .as_array()
        .and_then(|receipts| receipts.first())
        .and_then(|row| row["receipt"].as_str())
        .expect("selected receipt");
    let receipt_path = PathBuf::from(receipt);
    let receipt_path = if receipt_path.is_absolute() {
        receipt_path
    } else {
        index_path
            .parent()
            .expect("capture index parent")
            .join(receipt_path)
    };
    let receipt_value: Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read validation receipt"))
            .expect("validation receipt JSON");
    let stdout = receipt_value["stdout"].as_str().expect("receipt stdout");
    let stdout_path = receipt_path.parent().expect("receipt parent").join(stdout);
    (receipt_path, stdout_path)
}

#[test]
fn validation_rechecks_changed_capture_and_reference_inputs_on_each_public_call() {
    let artifacts = TestDir::new("validation-cache-artifacts");
    let repository = TestDir::new("validation-cache-repository");
    init_validation_repository(repository.path());
    let repository_path = repository
        .path()
        .canonicalize()
        .expect("canonical repository");

    for (name, fixture) in [
        ("intent.json", "intent-good.json"),
        ("design.json", "design-good.json"),
        (
            "implementation-report.json",
            "implementation-report-good.json",
        ),
    ] {
        artifacts.write_json(name, &support::load_fixture(fixture));
    }
    let mut plan = support::executable_fixture_plan();
    let original_commands = plan["proof_commands"]
        .as_array()
        .expect("fixture proof commands")
        .clone();
    // The current run-local prepare helper requires four named commands. Keep
    // this test's workload real while giving it four distinct obligations.
    plan["proof_commands"] = json!((0..4)
        .map(|index| {
            let mut command = original_commands[index % original_commands.len()].clone();
            command["id"] = json!(format!("validation-cache-proof-{index}"));
            command
        })
        .collect::<Vec<_>>());
    artifacts.write_json("plan.json", &plan);

    let implementation_checkpoint = Command::new(provider_binary())
        .args([
            "checkpoint",
            "--phase",
            "implementation",
            "--artifact-root",
            artifacts.path().to_str().expect("artifact root"),
            "--working-directory",
            repository_path.to_str().expect("repository"),
        ])
        .current_dir(&repository_path)
        .output()
        .expect("implementation checkpoint");
    assert!(
        implementation_checkpoint.status.success(),
        "implementation checkpoint failed: {}",
        String::from_utf8_lossy(&implementation_checkpoint.stderr)
    );

    let input = support::config_artifact_root(support::load_profile("minimal"), &artifacts);
    let config_version = input["config_version"].as_str().expect("config version");
    let mut implementation_context = Vec::new();
    append_review_records(
        &mut implementation_context,
        "implementation-adversarial-review",
        "implementation-report.json",
        "r15",
        config_version,
        &[
            "tasks-actually-done",
            "no-scope-creep",
            "design-faithful-final",
        ],
    );
    let implementation_request = support::base_request(
        input.clone(),
        support::checked(
            "implementation-adversarial-review",
            "approved",
            "validation",
        ),
    );
    let mut implementation_request = implementation_request;
    implementation_request["context"] = Value::Array(implementation_context);
    assert_eq!(
        validation_response(&implementation_request, &repository_path),
        json!({"result": "allow"})
    );

    let records = support::validation_fixture(&input, &repository_path);
    let mut context: Vec<Value> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            json!({
                "id": record["record_id"],
                "kind": record["kind"],
                "data": record["data"],
                "sequence": index + 1,
                "created_at": index + 1
            })
        })
        .collect();
    append_review_records(
        &mut context,
        "validation-review",
        "validation-report.json",
        "r15",
        config_version,
        &[
            "intent-delivered",
            "docs-integrated",
            "requirement-proof-mapping",
        ],
    );
    let context = Value::Array(context);
    let mut request = support::base_request(
        input,
        support::checked(
            "validation-review",
            "approved",
            "validation-adversarial-review",
        ),
    );
    request["context"] = context;

    assert_eq!(
        validation_response(&request, repository.path()),
        json!({"result": "allow"})
    );

    let (receipt_path, stdout_path) = selected_validation_paths(&request);
    let stdout = fs::read(&stdout_path).expect("selected stdout");
    let mut altered_stdout = stdout.clone();
    altered_stdout.extend_from_slice(b"tampered");
    fs::write(&stdout_path, altered_stdout).expect("alter selected stdout");
    assert_validation_denied(
        &request,
        repository.path(),
        "software-change-criterion-incomplete",
    );
    fs::write(&stdout_path, stdout).expect("restore selected stdout");

    let receipt = fs::read(&receipt_path).expect("selected receipt");
    let mut altered_receipt: Value = serde_json::from_slice(&receipt).expect("receipt JSON");
    altered_receipt["id"] = json!("tampered-receipt");
    fs::write(
        &receipt_path,
        serde_json::to_vec(&altered_receipt).expect("altered receipt JSON"),
    )
    .expect("alter selected receipt");
    assert_validation_denied(
        &request,
        repository.path(),
        "software-change-criterion-incomplete",
    );
    fs::write(&receipt_path, receipt).expect("restore selected receipt");

    let marker = repository.path().join("marker.txt");
    let marker_bytes = fs::read(&marker).expect("read marker");
    fs::write(&marker, b"changed between public evaluations\n").expect("alter repository");
    assert_validation_denied(
        &request,
        repository.path(),
        "software-change-checkpoint-invalid",
    );
    fs::write(&marker, marker_bytes).expect("restore repository");
    assert_eq!(
        validation_response(&request, repository.path()),
        json!({"result": "allow"})
    );

    let command_id = request["context"]
        .as_array()
        .expect("validation context")
        .iter()
        .find(|record| record["kind"] == "command-evidence")
        .and_then(|record| record["id"].as_str())
        .expect("command ID")
        .to_owned();
    let malformed = mutate_first_validation_verdict(&request, |data| {
        data["evidence_context_ids"] = json!("not-an-array");
    });
    assert_validation_denied(
        &malformed,
        repository.path(),
        "software-change-criterion-incomplete",
    );
    let duplicate = mutate_first_validation_verdict(&request, |data| {
        data["evidence_context_ids"] = json!([command_id.clone(), command_id.clone()]);
    });
    assert_validation_denied(
        &duplicate,
        repository.path(),
        "software-change-criterion-incomplete",
    );
    let unknown = mutate_first_validation_verdict(&request, |data| {
        data["evidence_context_ids"] = json!(["missing-command-evidence"]);
    });
    assert_validation_denied(
        &unknown,
        repository.path(),
        "software-change-criterion-incomplete",
    );

    let report: Value = serde_json::from_slice(
        &fs::read(artifacts.path().join("validation-report.json")).expect("read validation report"),
    )
    .expect("validation report JSON");
    let implementation = support::load_fixture("implementation-report-good.json");
    for author in [report["author"].clone(), implementation["author"].clone()] {
        let wrong_author = mutate_first_validation_verdict(&request, |data| {
            data["author"] = author.clone();
        });
        assert_validation_denied(
            &wrong_author,
            repository.path(),
            "software-change-criterion-incomplete",
        );
    }
}
