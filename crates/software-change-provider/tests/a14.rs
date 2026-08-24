#[path = "support/mod.rs"]
mod support;

use loop_core::{OperationOutcome, TransitionKind};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
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

#[test]
fn owning_phase_routes_are_requestable_committed_and_persisted_on_fresh_runs() {
    assert_eq!(OWNING_PHASE_ROUTES.len(), 10);

    for (index, &(source, event, target)) in OWNING_PHASE_ROUTES.iter().enumerate() {
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
            json!([{"id": "axis", "description": "test axis"}]),
        );
        let subject = subject_for_review(source);
        let mut schemas = serde_json::Map::new();
        schemas.insert(subject.to_owned(), metadata_schema());
        engine.start_ok(
            &run_id,
            json!({
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
            fs::write(
                Path::new(root).join("validation-report.json"),
                serde_json::to_vec(&valid_metadata("1")).expect("serialize validation report"),
            )
            .expect("write validation report");
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
                .output()
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
}

#[test]
fn describe_matches_snapshot_and_engine_prd_reference_topology() {
    let output = support::invoke(serde_json::json!({"operation": "describe"}));
    support::assert_exit(&output, 0);
    assert_eq!(output.stdout, include_bytes!("snapshots/describe.json"));
    let workflow: Value = support::response(&output);
    let snapshot: Value = serde_json::from_slice(include_bytes!("snapshots/describe.json"))
        .expect("snapshot workflow JSON");

    // Snapshot equality catches guidance drift; these semantic assertions catch
    // a changed edge or final flag even if someone regenerates that snapshot.
    assert_expected_topology(&workflow);
    assert_expected_topology(&snapshot);

    let prd_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PRD.md");
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
        "implement\n  └─ implementation-ready [checked] → implementation-review",
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
    let workflow: Value = serde_json::from_slice(include_bytes!("snapshots/describe.json"))
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
