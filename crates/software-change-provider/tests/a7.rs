#[path = "support/mod.rs"]
mod support;

use loop_core::OperationOutcome;
use serde_json::json;
use support::{Engine, TestDir};

#[test]
fn malformed_config_reaches_engine_as_error_without_advancing_run() {
    let state = TestDir::new("a7-state");
    let engine = Engine::new(state.path().join("a7.sqlite"));
    let malformed = json!({
        "config_version": "test-1",
        "review_policies": {},
        "unexpected_top_level": true
    });
    engine.start_ok("a7", malformed);

    let outcome = engine.event("a7", "intent-ready");
    let issue = match outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected malformed-config engine error, got {other:?}"),
    };
    assert_eq!(issue.code, "provider-execution-failed");
    assert!(issue.message.contains("unknown top-level key"));

    let run = engine.authoritative("a7");
    assert_eq!(run.current_state.as_str(), "explore");
    assert_eq!(run.last_sequence.as_u64(), 1);
    assert!(engine.show("a7").latest_evaluations.is_empty());
}
