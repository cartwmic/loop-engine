#[path = "support/mod.rs"]
mod support;

use loop_core::{OperationOutcome, TransitionKind};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use support::{Engine, TestDir};

const EXPECTED_STATES: &[(&str, bool)] = &[
    ("explore", false),
    ("design", false),
    ("design-review", false),
    ("plan", false),
    ("plan-review", false),
    ("implement", false),
    ("implementation-review", false),
    ("validation", false),
    ("end", true),
];

const OWNING_PHASE_ROUTES: &[(&str, &str, &str)] = &[
    ("design-review", "revise-intent", "explore"),
    ("plan-review", "revise-design", "design"),
    ("plan-review", "revise-intent", "explore"),
    ("implementation-review", "revise-plan", "plan"),
    ("implementation-review", "revise-design", "design"),
    ("implementation-review", "revise-intent", "explore"),
    ("validation", "revise-plan", "plan"),
    ("validation", "revise-design", "design"),
    ("validation", "revise-intent", "explore"),
];

const CHECKED_PROGRESS: &[(&str, &str, &str)] = &[
    ("explore", "intent-ready", "design"),
    ("design", "design-ready", "design-review"),
    ("design-review", "approved", "plan"),
    ("plan", "plan-ready", "plan-review"),
    ("plan-review", "approved", "implement"),
    ("implement", "implementation-ready", "implementation-review"),
    ("implementation-review", "approved", "validation"),
];

const EXPECTED_TRANSITIONS: &[(&str, &str, &str, &str)] = &[
    ("explore", "intent-ready", "design", "checked"),
    ("design", "design-ready", "design-review", "checked"),
    ("design-review", "approved", "plan", "checked"),
    ("design-review", "revise", "design", "check-free"),
    ("design-review", "revise-intent", "explore", "check-free"),
    ("plan", "plan-ready", "plan-review", "checked"),
    ("plan-review", "approved", "implement", "checked"),
    ("plan-review", "revise", "plan", "check-free"),
    ("plan-review", "revise-design", "design", "check-free"),
    ("plan-review", "revise-intent", "explore", "check-free"),
    (
        "implement",
        "implementation-ready",
        "implementation-review",
        "checked",
    ),
    ("implementation-review", "approved", "validation", "checked"),
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
    ("validation", "passed", "end", "checked"),
    ("validation", "revise", "implement", "check-free"),
    ("validation", "revise-plan", "plan", "check-free"),
    ("validation", "revise-design", "design", "check-free"),
    ("validation", "revise-intent", "explore", "check-free"),
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

#[test]
fn owning_phase_routes_are_requestable_committed_and_persisted_on_fresh_runs() {
    assert_eq!(OWNING_PHASE_ROUTES.len(), 9);

    for (index, &(source, event, target)) in OWNING_PHASE_ROUTES.iter().enumerate() {
        let state = TestDir::new(&format!("a14-route-state-{index}"));
        let engine = Engine::new(state.path().join("route.sqlite"));
        let run_id = format!("a14-route-{index}");
        engine.start_ok(
            &run_id,
            json!({
                "config_version": "a14-route-test",
                "review_policies": {}
            }),
        );

        let mut current = "explore";
        for &(progress_source, progress_event, progress_target) in CHECKED_PROGRESS {
            assert_eq!(
                progress_source, current,
                "fresh run path drift before {source}"
            );
            let outcome = engine.event(&run_id, progress_event);
            let committed = match outcome {
                OperationOutcome::Completed(result) => result,
                other => {
                    panic!("expected checked progress {progress_event} to commit, got {other:?}")
                }
            };
            assert_eq!(committed.run.current_state.as_str(), progress_target);
            current = progress_target;
            if current == source {
                break;
            }
        }
        assert_eq!(current, source, "fresh run did not reach route source");

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
        "explore\n  └─ intent-ready [checked] → design",
        "design\n  └─ design-ready [checked] → design-review",
        "design-review\n  ├─ approved [checked] → plan",
        "  ├─ revise [check-free] → design",
        "  └─ revise-intent [check-free] → explore",
        "plan\n  └─ plan-ready [checked] → plan-review",
        "plan-review\n  ├─ approved [checked] → implement",
        "  ├─ revise [check-free] → plan",
        "  ├─ revise-design [check-free] → design",
        "  └─ revise-intent [check-free] → explore",
        "implement\n  └─ implementation-ready [checked] → implementation-review",
        "implementation-review\n  ├─ approved [checked] → validation",
        "  ├─ revise [check-free] → implement",
        "  ├─ revise-plan [check-free] → plan",
        "  ├─ revise-design [check-free] → design",
        "  └─ revise-intent [check-free] → explore",
        "validation\n  ├─ passed [checked] → end",
        "  ├─ revise [check-free] → implement",
        "  ├─ revise-plan [check-free] → plan",
        "  ├─ revise-design [check-free] → design",
        "  └─ revise-intent [check-free] → explore",
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
        "design-review",
        "plan-review",
        "implementation-review",
        "validation",
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

    let validation = states
        .iter()
        .find(|state| state["id"] == "validation")
        .expect("validation state")["instructions"]
        .as_str()
        .expect("validation guidance");
    let validation_lower = validation.to_ascii_lowercase();
    for clause in [
        "validation-report-local defects stay in validation",
        "edit and recheck `validation-report.json`",
        "retry checked `passed`",
        "do not use `revise` for report-local corrections",
        "from validation, select the owning phase: `revise` is only for implementation-owned defects",
        "`revise-plan` for plan-owned defects",
        "`revise-design` for design-owned defects",
        "`revise-intent` for intent-owned defects",
    ] {
        assert!(
            validation_lower.contains(clause),
            "validation guidance missing routing semantic: {clause}"
        );
    }
    assert!(
        !validation_lower.contains("revise for validation-report-local"),
        "validation guidance routes report-local corrections through revise"
    );
}
