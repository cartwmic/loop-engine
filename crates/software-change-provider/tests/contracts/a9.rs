use super::support;

use loop_core::OperationOutcome;
use serde_json::json;
use support::{axis_config, valid_metadata, Engine, TestDir};

#[test]
fn engine_distinguishes_inaccessible_root_provider_failure_from_review_deny_and_missing_binary() {
    let state = TestDir::new("a9-state");

    let inaccessible_artifacts = TestDir::new("a9-inaccessible-artifacts");
    let mut inaccessible_config = axis_config(&inaccessible_artifacts, "axis");
    inaccessible_config["artifact_root"] = json!(state
        .path()
        .join("missing-artifact-root")
        .to_string_lossy()
        .to_string());
    let inaccessible = Engine::new(state.path().join("inaccessible.sqlite"));
    inaccessible.start_ok("inaccessible", inaccessible_config);
    let inaccessible_outcome = inaccessible.event("inaccessible", "intent-ready");
    let inaccessible_issue = match inaccessible_outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected inaccessible-root error, got {other:?}"),
    };
    assert_eq!(inaccessible_issue.code, "provider-execution-failed");
    assert!(inaccessible_issue
        .message
        .contains("provider-exited-nonzero"));
    assert!(inaccessible_issue.message.contains("artifact_root"));
    assert_eq!(
        inaccessible.current_state("inaccessible").as_str(),
        "explore"
    );
    assert!(inaccessible
        .show("inaccessible")
        .latest_evaluations
        .is_empty());

    let review_artifacts = TestDir::new("a9-review-artifacts");
    review_artifacts.write_json("intent.json", &valid_metadata("1"));
    let review = Engine::new(state.path().join("review.sqlite"));
    review.start_ok("review-deny", axis_config(&review_artifacts, "axis"));
    assert!(matches!(
        review.event("review-deny", "intent-ready"),
        OperationOutcome::Completed(_)
    ));
    let review_outcome = review.event("review-deny", "approved");
    let review_issue = match review_outcome {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected review denial, got {other:?}"),
    };
    assert_eq!(review_issue.code, "software-change-finding-ledger-invalid");
    assert_eq!(
        review.current_state("review-deny").as_str(),
        "intent-review"
    );
    assert!(review
        .show("review-deny")
        .latest_evaluations
        .iter()
        .any(|evaluation| evaluation.is_deny()));

    let missing_binary = state.path().join("provider-does-not-exist");
    let unavailable = Engine::with_command(state.path().join("missing.sqlite"), missing_binary);
    let unavailable_outcome = unavailable.start(
        "missing-binary",
        json!({"contract_version": 3, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "none",
            "review_policies": {}
        }),
    );
    let unavailable_issue = match unavailable_outcome {
        OperationOutcome::Error(issue) => issue,
        other => panic!("expected missing-provider error, got {other:?}"),
    };
    assert_eq!(unavailable_issue.code, "provider-execution-failed");
    assert!(unavailable_issue.message.contains("provider-spawn-failed"));
    assert_ne!(
        inaccessible_issue.message, unavailable_issue.message,
        "engine must preserve distinct provider incapacity diagnostics"
    );
}

#[test]
fn engine_classifies_stale_config_evidence_as_review_denial() {
    let state = TestDir::new("a9-stale-config-state");
    let artifacts = TestDir::new("a9-stale-config-artifacts");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let engine = Engine::new(state.path().join("stale-config.sqlite"));
    engine.start_ok("stale-config", axis_config(&artifacts, "axis"));

    // New writes must use the actual required admission path. Stale evidence
    // is not manufactured as durable history through unchecked append.
    let before = engine.show("stale-config");
    let before_run = engine.authoritative("stale-config");
    let persistence =
        loop_integrations::SqlitePersistence::open(state.path().join("stale-config.sqlite"))
            .expect("open fixture persistence");
    let before_history = loop_core::Persistence::load_history(&persistence, &"stale-config".into())
        .expect("load fixture history");
    let stale = support::evidence(
        "intent-review",
        "axis",
        "pass",
        "",
        "reviewer-a9",
        "agent",
        "intent.json",
        "1",
        "stale-test-version",
    );
    let gateway =
        loop_integrations::SubprocessProviderGateway::new(std::time::Duration::from_secs(10));
    let appended = loop_core::execute_append_with_gateway(
        loop_core::AppendRequest::new(
            "stale-config",
            "stale-config-evidence",
            "review-evidence",
            stale,
            loop_core::Timestamp::from_unix_millis(0),
        ),
        &persistence,
        &gateway,
    );
    let refused = match appended {
        OperationOutcome::Rejected(issue) => issue,
        other => panic!("expected stale-config admission refusal, got {other:?}"),
    };
    assert_eq!(refused.code, "record-refused");
    let refusal = refused.details.as_ref().expect("admission refusal details");
    assert_eq!(refusal["record_id"], "stale-config-evidence");
    assert_eq!(refusal["rule"], "review-shape-policy");
    assert!(refusal["message"].as_str().is_some_and(
        |message| message.contains("review config_version does not match frozen input")
    ));
    let after = engine.show("stale-config");
    assert_eq!(after.context, before.context);
    assert_eq!(after.current_state, before.current_state);
    assert_eq!(after.state_visit, before.state_visit);
    assert_eq!(
        engine.authoritative("stale-config").last_sequence,
        before_run.last_sequence,
    );
    assert_eq!(
        loop_core::Persistence::load_history(&persistence, &"stale-config".into())
            .expect("load unchanged fixture history"),
        before_history,
    );
    assert!(!after
        .context
        .iter()
        .any(|record| record.id.as_str() == "stale-config-evidence"));
    engine.append_finding_ledger(
        "stale-config",
        "stale-config-ledger",
        "intent-review",
        "intent.json",
        "1",
        json!([]),
    );

    assert!(matches!(
        engine.event("stale-config", "intent-ready"),
        OperationOutcome::Completed(_)
    ));
    let outcome = engine.event("stale-config", "approved");
    let issue = match outcome {
        OperationOutcome::Rejected(issue) => issue,
        OperationOutcome::Error(issue) => {
            panic!("expected stale config policy denial, got evaluation error {issue:?}")
        }
        other => panic!("expected stale config policy denial, got {other:?}"),
    };
    assert_eq!(issue.code, "software-change-review-incomplete");
    let details = issue.details.as_ref().expect("evidence denial details");
    assert_eq!(details["phase"], "evidence");
    let axis = details["diagnostics"]
        .as_array()
        .expect("axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("axis diagnostic");
    assert!(axis["diagnostics"]
        .as_array()
        .expect("blocking diagnostics")
        .iter()
        .all(|diagnostic| diagnostic["category"] != "stale_config"));
    assert!(axis["diagnostics"]
        .as_array()
        .expect("blocking diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["category"] == "missing"));
    assert!(engine
        .show("stale-config")
        .context
        .iter()
        .all(|record| record.id.as_str() != "stale-config-evidence"));
    assert_eq!(
        engine.current_state("stale-config").as_str(),
        "intent-review"
    );
    let shown = engine.show("stale-config");
    assert!(shown
        .latest_evaluations
        .iter()
        .any(|evaluation| evaluation.is_deny()));
}

#[test]
fn supplied_historical_stale_config_keeps_original_read_side_diagnostics() {
    let artifacts = TestDir::new("a9-historical-stale-context");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let mut request = support::base_request(
        axis_config(&artifacts, "axis"),
        support::checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        support::context_json(
            "review-evidence",
            support::evidence(
                "intent-review",
                "axis",
                "pass",
                "",
                "historical-reviewer",
                "agent",
                "intent.json",
                "1",
                "stale-test-version"
            ),
            1
        ),
        support::context_json(
            "finding-ledger",
            support::finding_ledger("intent-review", "intent.json", "1", json!([])),
            2
        )
    ]);
    // Independently supplied historical provider input, never a newly admitted
    // malformed record or a SQLite/persistence bypass in a production run.
    let output = support::invoke(request);
    support::assert_exit(&output, 0);
    let result = support::response(&output);
    assert_eq!(result["result"], "deny");
    assert_eq!(
        result["feedback"]["code"],
        "software-change-review-incomplete"
    );
    let details = &result["feedback"]["details"];
    assert_eq!(details["phase"], "evidence");
    let axis = details["diagnostics"]
        .as_array()
        .expect("axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("axis diagnostic");
    assert!(axis["diagnostics"]
        .as_array()
        .expect("blocking diagnostics")
        .iter()
        .all(|diagnostic| diagnostic["category"] != "stale_config"));
    let informational_axis = details["informational"]
        .as_array()
        .expect("informational axis diagnostics")
        .iter()
        .find(|axis| axis["axis"] == "axis")
        .expect("informational axis diagnostic");
    let stale_config = informational_axis["diagnostics"]
        .as_array()
        .expect("informational diagnostics")
        .iter()
        .find(|diagnostic| diagnostic["category"] == "stale_config")
        .expect("stale config diagnostic");
    assert_eq!(stale_config["evidence_version"], "stale-test-version");
    assert_eq!(stale_config["run_version"], "test-1");
}

#[test]
fn supplied_stale_config_history_does_not_block_sufficient_current_coverage() {
    let artifacts = TestDir::new("a9-historical-plus-complete-current");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let mut request = support::base_request(
        axis_config(&artifacts, "axis"),
        support::checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        support::context_json(
            "review-evidence",
            support::evidence(
                "intent-review",
                "axis",
                "pass",
                "",
                "historical-reviewer",
                "agent",
                "intent.json",
                "1",
                "stale-test-version"
            ),
            1
        ),
        support::context_json(
            "review-evidence",
            support::evidence(
                "intent-review",
                "axis",
                "pass",
                "",
                "current-reviewer",
                "agent",
                "intent.json",
                "1",
                "test-1"
            ),
            2
        ),
        support::context_json(
            "finding-ledger",
            support::finding_ledger("intent-review", "intent.json", "1", json!([])),
            3
        )
    ]);
    // Stateless historical input only: no invalid new write or persistence bypass.
    let output = support::invoke(request.clone());
    support::assert_exit(&output, 0);
    assert_eq!(support::response(&output), json!({"result":"allow"}));

    let mut malformed_current = request.clone();
    let mut malformed = support::evidence(
        "intent-review",
        "axis",
        "pass",
        "",
        "malformed-current-reviewer",
        "agent",
        "intent.json",
        "1",
        "test-1",
    );
    malformed["result"] = json!("not-a-verdict");
    malformed_current["context"]
        .as_array_mut()
        .unwrap()
        .push(support::context_json("review-evidence", malformed, 4));
    let output = support::invoke(malformed_current);
    support::assert_exit(&output, 0);
    assert_eq!(support::response(&output)["result"], "deny");

    request["context"].as_array_mut().unwrap().remove(1);
    let output = support::invoke(request.clone());
    support::assert_exit(&output, 0);
    let denied = support::response(&output);
    assert_eq!(denied["result"], "deny");
    assert_eq!(denied["feedback"]["details"]["phase"], "evidence");
    assert!(denied["feedback"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|d| d["category"] == "missing"));
    assert!(denied["feedback"]["details"]["informational"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|axis| axis["diagnostics"].as_array().unwrap())
        .any(|d| d["category"] == "stale_config"
            && d["evidence_version"] == "stale-test-version"
            && d["run_version"] == "test-1"));
}

#[test]
fn otherwise_satisfied_current_context_still_runs_final_shared_classification() {
    let artifacts = TestDir::new("a9-final-shared-classifier");
    artifacts.write_json("intent.json", &valid_metadata("1"));
    let mut request = support::base_request(
        axis_config(&artifacts, "axis"),
        support::checked("intent-review", "approved", "design"),
    );
    request["context"] = json!([
        support::context_json(
            "review-evidence",
            support::evidence(
                "intent-review",
                "axis",
                "pass",
                "",
                "current-reviewer",
                "agent",
                "intent.json",
                "1",
                "test-1"
            ),
            1
        ),
        support::context_json(
            "finding-ledger",
            support::finding_ledger("intent-review", "intent.json", "1", json!([])),
            2
        )
    ]);
    let output = support::invoke(request.clone());
    support::assert_exit(&output, 0);
    assert_eq!(support::response(&output), json!({"result":"allow"}));

    // There is no intent-sufficient axis: the typed ledger/evidence checks are
    // already satisfied and do not consume this comparison. Only the final
    // shared classifier selects the current comparison and rejects its shape.
    request["context"]
        .as_array_mut()
        .unwrap()
        .push(support::context_json(
            "intent-comparison",
            json!({"subject_revision":"1"}),
            3,
        ));
    let output = support::invoke(request.clone());
    support::assert_exit(&output, 0);
    let denied = support::response(&output);
    assert_eq!(denied["result"], "deny");
    assert_eq!(
        denied["feedback"]["code"],
        "software-change-review-incomplete"
    );
    let details = &denied["feedback"]["details"];
    assert_eq!(details["phase"], "record-classification");
    assert_eq!(details["category"], "record_malformed");
    assert_eq!(details["rule"], "intent-comparison-shape-references");
    assert!(details["message"]
        .as_str()
        .expect("comparison refusal")
        .contains("invalid intent-comparison"));

    request["context"].as_array_mut().unwrap().pop();
    let output = support::invoke(request);
    support::assert_exit(&output, 0);
    assert_eq!(support::response(&output), json!({"result":"allow"}));
}
