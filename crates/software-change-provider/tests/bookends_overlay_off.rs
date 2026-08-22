#[path = "support/mod.rs"]
mod support;

use loop_core::{Lifecycle, OperationOutcome};
use serde_json::{json, Value};
use support::{config_artifact_root, load_fixture, load_profile, Engine, TestDir};

const PROFILES: &[&str] = &["minimal", "standard", "high-rigor"];

fn write_good_artifacts(root: &TestDir) {
    root.write_json("intent.json", &load_fixture("intent-good.json"));
    root.write_json("design.json", &load_fixture("design-good.json"));
    root.write_json("plan.json", &load_fixture("plan-good.json"));
    root.write_json(
        "implementation-report.json",
        &load_fixture("implementation-report-good.json"),
    );
    root.write_json(
        "validation-report.json",
        &load_fixture("validation-report-good.json"),
    );
}

fn nonempty_axes(profile: &Value, gate: &str) -> Vec<(String, u64)> {
    profile["review_policies"][gate]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|axis| {
            let id = axis.get("id")?.as_str()?.to_owned();
            let required = axis
                .get("required_authors")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            Some((id, required))
        })
        .collect()
}

fn pass_gate(
    engine: &Engine,
    run_id: &str,
    gate: &str,
    subject: &str,
    config_version: &str,
    axes: &[(String, u64)],
    last: bool,
) {
    for (axis, required) in axes {
        for author_index in 0..*required {
            engine.append_evidence(
                run_id,
                &format!("{run_id}-{gate}-{axis}-{author_index}"),
                gate,
                axis,
                "pass",
                "",
                &format!("reviewer-{author_index}"),
                "agent",
                subject,
                "r15",
                config_version,
            );
        }
    }
    engine.append_accepted_findings(
        run_id,
        &format!("{run_id}-{gate}-accepted"),
        gate,
        subject,
        "r15",
        json!([]),
    );
    let event = if last { "passed" } else { "approved" };
    let outcome = engine.event(run_id, event);
    assert!(
        matches!(outcome, OperationOutcome::Completed(_)),
        "{run_id} {gate} {event}: {outcome:?}"
    );
}

fn walk_shipped_profile_to_end(profile_name: &str) {
    let artifacts = TestDir::new(&format!("overlay-off-{profile_name}-artifacts"));
    write_good_artifacts(&artifacts);
    for subject in [
        "intent.json",
        "design.json",
        "plan.json",
        "implementation-report.json",
        "validation-report.json",
    ] {
        let value = serde_json::from_slice::<Value>(
            &std::fs::read(artifacts.path().join(subject)).expect("read artifact"),
        )
        .expect("artifact json");
        assert!(
            value.get("requirement_ids").is_none(),
            "{profile_name} {subject} must have no requirement_ids"
        );
    }

    let state = TestDir::new(&format!("overlay-off-{profile_name}-state"));
    let engine = Engine::new(state.path().join("run.sqlite"));
    let input = config_artifact_root(load_profile(profile_name), &artifacts);
    assert!(
        input
            .get("extra")
            .and_then(|extra| extra.get("bookends"))
            .is_none(),
        "{profile_name} shipped extra must not enable bookends"
    );
    let config_version = input["config_version"]
        .as_str()
        .expect("config_version")
        .to_owned();

    engine.start_ok(profile_name, input.clone());
    assert_eq!(engine.current_state(profile_name).as_str(), "explore");

    let phases = [
        (
            "intent-ready",
            "intent-review",
            "intent-adversarial-review",
            "intent.json",
        ),
        (
            "design-ready",
            "design-review",
            "design-adversarial-review",
            "design.json",
        ),
        (
            "plan-ready",
            "plan-review",
            "plan-adversarial-review",
            "plan.json",
        ),
        (
            "implementation-ready",
            "implementation-review",
            "implementation-adversarial-review",
            "implementation-report.json",
        ),
        (
            "validation-ready",
            "validation-review",
            "validation-adversarial-review",
            "validation-report.json",
        ),
    ];

    let mut remaining_reviews = phases
        .iter()
        .map(|(_, parent, adversarial, _)| {
            usize::from(!nonempty_axes(&input, parent).is_empty())
                + usize::from(!nonempty_axes(&input, adversarial).is_empty())
        })
        .sum::<usize>();

    for (ready, parent, adversarial, subject) in phases {
        let outcome = engine.event(profile_name, ready);
        assert!(
            matches!(outcome, OperationOutcome::Completed(_)),
            "{profile_name} {ready}: {outcome:?}"
        );
        for gate in [parent, adversarial] {
            let axes = nonempty_axes(&input, gate);
            if axes.is_empty() {
                continue;
            }
            remaining_reviews -= 1;
            pass_gate(
                &engine,
                profile_name,
                gate,
                subject,
                &config_version,
                &axes,
                remaining_reviews == 0,
            );
        }
    }

    assert_eq!(engine.current_state(profile_name).as_str(), "end");
    assert_eq!(engine.lifecycle(profile_name), Lifecycle::Final);
}

#[test]
fn overlay_off_each_shipped_profile_starts_progresses_and_passes_without_git_prd() {
    for profile in PROFILES {
        walk_shipped_profile_to_end(profile);
    }
}

#[test]
fn overlay_off_high_rigor_evaluate_allows_missing_requirement_ids() {
    let artifacts = TestDir::new("overlay-off-high-rigor-eval");
    artifacts.write_json("intent.json", &load_fixture("intent-good.json"));
    let output = support::invoke(support::base_request(
        config_artifact_root(load_profile("high-rigor"), &artifacts),
        support::checked("explore", "intent-ready", "intent-review"),
    ));
    support::assert_exit(&output, 0);
    assert_eq!(
        support::response(&output),
        json!({"result": "allow"}),
        "overlay-off high-rigor must allow intent.json without requirement_ids"
    );
}
