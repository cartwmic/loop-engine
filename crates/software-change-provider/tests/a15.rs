#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use support::{config_artifact_root, load_fixture, load_profile, Engine, TestDir};

#[test]
fn every_shipped_profile_starts_and_exposes_policies_and_schemas() {
    for profile in ["minimal", "standard", "high-rigor"] {
        let artifacts = TestDir::new(&format!("a15-{profile}-artifacts"));
        artifacts.write_json("intent.json", &load_fixture("intent-good.json"));
        let state = TestDir::new(&format!("a15-{profile}-state"));
        let engine = Engine::new(state.path().join("run.sqlite"));
        let input = config_artifact_root(load_profile(profile), &artifacts);

        engine.start_ok(profile, input.clone());
        let show = engine.show(profile);
        assert_eq!(show.initial_input, input, "{profile} initial input changed");
        assert!(show.initial_input["review_policies"].is_object());
        assert_eq!(
            show.initial_input["artifact_schemas"]
                .as_object()
                .expect("schemas")
                .len(),
            5,
            "{profile} schema surface"
        );

        let first = engine.event(profile, "intent-ready");
        match profile {
            "minimal" => assert!(matches!(first, OperationOutcome::Completed(_))),
            "standard" | "high-rigor" => {
                let issue = match first {
                    OperationOutcome::Rejected(issue) => issue,
                    other => panic!("expected defined review denial for {profile}, got {other:?}"),
                };
                assert_eq!(issue.code, "software-change-review-incomplete");
            }
            _ => unreachable!(),
        }
    }
}
