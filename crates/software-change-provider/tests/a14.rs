#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use std::path::Path;

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

const EXPECTED_TRANSITIONS: &[(&str, &str, &str, &str)] = &[
    ("explore", "intent-ready", "design", "checked"),
    ("design", "design-ready", "design-review", "checked"),
    ("design-review", "approved", "plan", "checked"),
    ("design-review", "revise", "design", "check-free"),
    ("plan", "plan-ready", "plan-review", "checked"),
    ("plan-review", "approved", "implement", "checked"),
    ("plan-review", "revise", "plan", "check-free"),
    (
        "implement",
        "implementation-ready",
        "implementation-review",
        "checked",
    ),
    ("implementation-review", "approved", "validation", "checked"),
    ("implementation-review", "revise", "implement", "check-free"),
    ("validation", "passed", "end", "checked"),
    ("validation", "revise", "implement", "check-free"),
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
        "  └─ revise [check-free] → design",
        "plan\n  └─ plan-ready [checked] → plan-review",
        "plan-review\n  ├─ approved [checked] → implement",
        "  └─ revise [check-free] → plan",
        "implement\n  └─ implementation-ready [checked] → implementation-review",
        "implementation-review\n  ├─ approved [checked] → validation",
        "  └─ revise [check-free] → implement",
        "validation\n  ├─ passed [checked] → end",
        "  └─ revise [check-free] → implement",
        "end [final]",
    ] {
        assert!(prd.contains(line), "PRD topology line missing: {line}");
    }
}
