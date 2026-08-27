use super::bounded_process::CommandExt;
use super::support;

use loop_core::{Lifecycle, OperationOutcome};
use serde_json::json;
use std::fs;
use std::process::Command;
use support::{provider_binary, Engine, TestDir};

#[test]
fn missing_policies_errors_on_first_check_and_leaves_engine_state_unchanged() {
    let state = TestDir::new("a2-missing-state");
    let engine = Engine::new(state.path().join("missing.sqlite"));
    let run = engine.start_ok("missing-policies", json!({"config_version": "standard-1"}));
    assert_eq!(run.workflow.states.len(), 16);
    let slot_ids: Vec<_> = run
        .workflow
        .work_slots
        .iter()
        .map(|slot| slot.id.as_str())
        .collect();
    assert!(slot_ids.contains(&"intent-draft"));
    assert!(slot_ids.contains(&"validation-draft"));
    assert!(slot_ids.contains(&"intent-review"));
    let draft = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.id.as_str() == "intent-draft")
        .expect("intent-draft");
    assert!(draft.stdin_context_kinds.is_empty());
    let review = run
        .workflow
        .work_slots
        .iter()
        .find(|slot| slot.id.as_str() == "intent-review")
        .expect("intent-review");
    assert_eq!(review.stdin_context_kinds, ["finding-ledger"]);

    let outcome = engine.event("missing-policies", "intent-ready");
    let issue = match outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected evaluation error, got {other:?}"),
    };
    assert!(issue.message.contains("minimal"));
    assert!(issue.message.contains("standard"));
    assert!(issue.message.contains("high-rigor"));

    let run = engine.authoritative("missing-policies");
    assert_eq!(run.current_state.as_str(), "explore");
    assert_eq!(run.last_sequence.as_u64(), 1);
    assert!(engine
        .show("missing-policies")
        .latest_evaluations
        .is_empty());
}

#[test]
fn explicitly_empty_policies_walk_to_end_with_allocated_artifact_root() {
    let state = TestDir::new("a2-empty-state");
    let repository = state.path().join("repository");
    fs::create_dir_all(&repository).expect("create empty-policy repository");
    fs::write(repository.join("marker.txt"), b"baseline\n").expect("write empty-policy marker");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "software-change a2"],
        vec!["config", "user.email", "a2@example.invalid"],
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
    let engine = Engine::with_command(state.path().join("empty.sqlite"), &wrapper);
    let input = json!({"config_version": "none", "review_policies": {}});
    engine.start_ok("empty-policies", input);

    let artifact_root = state.path().join("runs").join("empty-policies");
    for name in ["intent.json", "design.json", "plan.json"] {
        fs::write(artifact_root.join(name), br#"{"revision":"1"}"#)
            .expect("write checkpoint document");
    }
    for name in ["implementation-report.json", "validation-report.json"] {
        fs::write(artifact_root.join(name), br#"{"revision":"1"}"#)
            .expect("write checkpoint report");
    }
    for phase in ["implementation", "validation"] {
        let output = Command::new(provider_binary())
            .args([
                "checkpoint",
                "--phase",
                phase,
                "--artifact-root",
                artifact_root.to_str().expect("artifact root"),
                "--working-directory",
                repository.to_str().expect("repository"),
            ])
            .current_dir(&repository)
            .bounded_output("software-change checkpoint")
            .expect("run checkpoint");
        assert!(
            output.status.success(),
            "checkpoint {phase} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let shown = engine.show("empty-policies");
    let allocated = state
        .path()
        .join("runs")
        .join("empty-policies")
        .canonicalize()
        .expect("allocated catalog path");
    assert_eq!(
        shown.initial_input["artifact_root"],
        json!(allocated.to_string_lossy().to_string())
    );
    assert!(shown.initial_input.get("artifact_schemas").is_none());
    assert!(shown
        .initial_input
        .get("review_policies")
        .and_then(|value| value.as_object())
        .is_some_and(|policies| policies.is_empty()));

    for event in [
        "intent-ready",
        "design-ready",
        "plan-ready",
        "implementation-ready",
        "passed",
    ] {
        let result = engine.event("empty-policies", event);
        assert!(
            matches!(result, OperationOutcome::Completed(_)),
            "{event}: {result:?}"
        );
    }

    assert_eq!(engine.current_state("empty-policies").as_str(), "end");
    assert_eq!(engine.lifecycle("empty-policies"), Lifecycle::Final);
}
