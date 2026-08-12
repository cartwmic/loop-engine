#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use support::{invoke, load_profile, response};

#[test]
fn standard_checked_routes_all_have_a_declared_schema_or_review_axis() {
    let config = load_profile("standard");
    let routes = [
        (
            "explore",
            "intent-ready",
            "design",
            "intent.json",
            Some("intent"),
        ),
        (
            "design",
            "design-ready",
            "design-review",
            "design.json",
            None,
        ),
        (
            "design-review",
            "approved",
            "plan",
            "design.json",
            Some("design-review"),
        ),
        ("plan", "plan-ready", "plan-review", "plan.json", None),
        (
            "plan-review",
            "approved",
            "implement",
            "plan.json",
            Some("plan-review"),
        ),
        (
            "implement",
            "implementation-ready",
            "implementation-review",
            "implementation-report.json",
            None,
        ),
        (
            "implementation-review",
            "approved",
            "validation",
            "implementation-report.json",
            Some("implementation-review"),
        ),
        (
            "validation",
            "passed",
            "end",
            "validation-report.json",
            Some("validation"),
        ),
    ];

    for (source, event, target, subject, gate) in routes {
        let has_schema = config["artifact_schemas"].get(subject).is_some();
        let has_axes = gate
            .and_then(|gate| config["review_policies"].get(gate))
            .and_then(Value::as_array)
            .is_some_and(|axes| !axes.is_empty());
        assert!(
            has_schema || has_axes,
            "{source} --{event}--> {target} has no declared obligation"
        );
    }
}

#[test]
fn describe_guidance_labels_schema_checks_conditionally() {
    let output = invoke(serde_json::json!({"operation": "describe"}));
    support::assert_exit(&output, 0);
    let workflow = response(&output);
    let instructions = workflow["states"]
        .as_array()
        .expect("workflow states")
        .iter()
        .filter_map(|state| state["instructions"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(instructions.contains(
        "when the run's configuration supplies a schema for it — read your obligations via `show`"
    ));
}
