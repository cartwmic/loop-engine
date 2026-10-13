use super::*;
use loop_core::{SemanticSequence, Timestamp};
use serde_json::json;

struct Fixture {
    root: PathBuf,
    input: Value,
    workflow: Workflow,
    revision: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "shared-review-rules-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let intent: Value = serde_json::from_str(include_str!(
            "../../data/calibration/fixtures/intent-good.json"
        ))
        .unwrap();
        std::fs::write(
            root.join("intent.json"),
            serde_json::to_vec_pretty(&intent).unwrap(),
        )
        .unwrap();
        let mut input: Value =
            serde_json::from_str(include_str!("../../data/configs/minimal.json")).unwrap();
        input["artifact_root"] = json!(root);
        input["config_version"] = json!("rule-test-15");
        let mut axes = input["review_policies"]["intent-review"]
            .as_array()
            .unwrap()[..2]
            .to_vec();
        for axis in &mut axes {
            axis["required_authors"] = json!(2);
        }
        input["review_policies"] = json!({"intent-review":axes});
        let workflow = crate::workflow::describe_workflow(Some(&input)).unwrap();
        Self {
            root,
            input,
            workflow,
            revision: intent["revision"].as_str().unwrap().into(),
        }
    }
    fn ctx<'a>(&'a self, records: &'a [ContextRecord]) -> AdmissionContext<'a> {
        // Positive candidate checks belong at an ordinary write point.
        self.ctx_at(records, "intent-review")
    }
    fn ctx_at<'a>(&'a self, records: &'a [ContextRecord], state: &'a str) -> AdmissionContext<'a> {
        AdmissionContext::new(&self.input, records, state, 7, &self.workflow).unwrap()
    }
    fn review(&self) -> Value {
        json!({"gate":"intent-review","policy_id":"solution-agnostic","review_stage":"aggregate","group":"intent-review.aggregate","assessment_id":"actual-return","review_contract_version":3,"result":"pass","findings":"","author":{"name":"reviewer","kind":"agent"},"subject":"intent.json","subject_revision":self.revision,"config_version":"rule-test-15","grounds":{"reason":"Scripted test inspection","evidence":[{"locator":"intent.json#/outcome"}]}})
    }
    fn ledger(&self) -> Value {
        json!({"schema_version":"1","gate":"intent-review","subject":"intent.json","subject_revision":self.revision,"author":{"name":"driver","kind":"agent"},"findings":[{"id":"F-one","source":{"kind":"context-record","id":"failed"},"policy_id":"solution-agnostic","statement":"original finding","disposition":"accepted","reason":"Driver retained actual finding","owner_phase":"intent","task_ids":[],"review_axes":["solution-agnostic"],"status":"unresolved"}]})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn proposed(kind: &str, data: Value) -> CandidateRecord {
    CandidateRecord {
        id: "candidate".into(),
        kind: kind.into(),
        data,
    }
}
fn stored(id: &str, kind: &str, data: Value, seq: u64) -> ContextRecord {
    ContextRecord::new(
        id,
        kind,
        data,
        SemanticSequence::new(seq),
        Timestamp::from_unix_millis(0),
    )
}

#[test]
fn review_rule_accepts_a_single_performed_judgment_without_aggregate_readiness() {
    let fixture = Fixture::new();
    let ctx = fixture.ctx(&[]);
    let good = proposed("review-evidence", fixture.review());
    assert!(check(&ctx, &good).is_ok()); // no sibling, second author or ledger yet
    for (field, value) in [
        ("policy_id", json!("unknown")),
        ("review_stage", json!("individual")),
        ("group", json!("wrong")),
        ("config_version", json!("stale")),
        ("subject", json!("design.json")),
        ("result", json!("unknown")),
        ("assessment_id", json!("")),
    ] {
        let mut bad = good.clone();
        bad.data[field] = value;
        assert!(check(&ctx, &bad).is_err(), "{field}");
    }
    let mut missing = good.clone();
    missing
        .data
        .as_object_mut()
        .unwrap()
        .remove("assessment_id");
    assert!(check(&ctx, &missing)
        .unwrap_err()
        .message
        .contains("assessment_id"));
    let mut historical = good;
    historical.data["subject_revision"] = json!("older");
    historical.data["grounds"]["evidence"][0]["locator"] = json!("missing.json#/fact");
    assert_eq!(
        check(&ctx, &historical).unwrap_err().rule,
        "review-write-citations"
    );
    let records = vec![stored("historical", "review-evidence", historical.data, 1)];
    assert!(classify(
        &fixture.ctx_at(&records, "intent-adversarial-review"),
        &BTreeSet::new(),
        &[],
    )[0]
    .1
    .is_ok());
}

#[test]
fn new_write_grounding_cannot_be_omitted_without_changing_legacy_read_classification() {
    let fixture = Fixture::new();
    let ctx = fixture.ctx(&[]);
    assert!(check(&ctx, &proposed("review-evidence", fixture.review())).is_ok());
    let mut ungrounded = fixture.review();
    ungrounded
        .as_object_mut()
        .unwrap()
        .remove("review_contract_version");
    ungrounded.as_object_mut().unwrap().remove("grounds");
    let refusal = check(&ctx, &proposed("review-evidence", ungrounded.clone())).unwrap_err();
    assert_eq!(refusal.rule, "review-write-citations");
    assert!(refusal.message.contains("new grounded review evidence"));
    let legacy = vec![stored("legacy", "review-evidence", ungrounded, 1)];
    assert!(classify(
        &fixture.ctx_at(&legacy, "intent-adversarial-review"),
        &BTreeSet::new(),
        &[],
    )[0]
    .1
    .is_ok());
}

#[test]
fn steering_and_incorporation_reuse_closed_shape_digest_and_earlier_reference_checks() {
    let fixture = Fixture::new();
    let ctx = fixture.ctx(&[]);
    let good = proposed(
        "user-steering",
        json!({"target":{"kind":"all"},"instruction":"Exact retained owner words"}),
    );
    assert!(check(&ctx, &good).is_ok());
    for data in [
        json!({"text":"wrong shape"}),
        json!({"target":{"kind":"slots","ids":["unknown"]},"instruction":"words"}),
        json!({"target":{"kind":"all"},"instruction":"words","supersedes":["missing"]}),
        json!({"target":{"kind":"all"},"instruction":"words","intent_baseline":{"revision":"old","locator":"retained","sha256":"sha256:bad","json_bytes":"{}"}}),
    ] {
        assert!(check(&ctx, &proposed("user-steering", data)).is_err());
    }
    let records = vec![stored("owner", "user-steering", good.data, 1)];
    let ctx = fixture.ctx(&records);
    assert!(check(
        &ctx,
        &proposed(
            "steering-incorporation",
            json!({"steering_ids":["owner"],"applied":"Actual application attestation"})
        )
    )
    .is_ok());
    assert!(check(
        &ctx,
        &proposed(
            "steering-incorporation",
            json!({"steering_ids":["missing"],"applied":"claim"})
        )
    )
    .is_err());
    assert!(check(
        &ctx,
        &proposed(
            "user-steering",
            json!({"target":{"kind":"all"},"instruction":"replacement","supersedes":["owner"]})
        )
    )
    .is_ok());
}

#[test]
fn applicability_new_write_uses_current_target_self_review_eligibility() {
    let fixture = Fixture::new();
    let original = fixture.review();
    let records = vec![stored("original", "review-evidence", original.clone(), 1)];
    let intent_path = fixture.root.join("intent.json");
    let mut current: Value = serde_json::from_slice(&std::fs::read(&intent_path).unwrap()).unwrap();
    current["revision"] = json!("next");
    current["author"] = original["author"].clone();
    std::fs::write(&intent_path, serde_json::to_vec_pretty(&current).unwrap()).unwrap();
    let carry = proposed(
        "evidence-applicability",
        json!({
            "origin":{"kind":"context-record","id":"original"},
            "target":{"subject":"intent.json","revision":"next","checkpoint":null},
            "attesting_driver":{"name":"driver","kind":"agent"},
            "reason":"SCRIPTED mechanical current-target eligibility control"
        }),
    );
    let ctx = fixture.ctx(&records);
    let mut direct = original.clone();
    direct["subject_revision"] = json!("next");
    let refused_direct = check(&ctx, &proposed("review-evidence", direct)).unwrap_err();
    assert_eq!(refused_direct.rule, "review-shape-policy");
    assert!(refused_direct
        .message
        .contains("self review is not allowed"));
    let refused_carry = check(&ctx, &carry).unwrap_err();
    assert_eq!(refused_carry.rule, "review-applicability-current-target");
    assert!(refused_carry.message.contains("self review is not allowed"));
    current["author"] = json!({"name":"different-subject-author","kind":"agent"});
    std::fs::write(&intent_path, serde_json::to_vec_pretty(&current).unwrap()).unwrap();
    assert!(check(&fixture.ctx(&records), &carry).is_ok());
    assert_eq!(records[0].data, original);
}

#[test]
fn applicability_current_target_keeps_explicit_self_review_permission() {
    let mut fixture = Fixture::new();
    for axis in fixture.input["review_policies"]["intent-review"]
        .as_array_mut()
        .unwrap()
    {
        axis["self_review"] = json!(true);
    }
    let original = fixture.review();
    let records = vec![stored("original", "review-evidence", original.clone(), 1)];
    let path = fixture.root.join("intent.json");
    let mut current: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    current["revision"] = json!("next");
    current["author"] = original["author"].clone();
    std::fs::write(&path, serde_json::to_vec_pretty(&current).unwrap()).unwrap();
    let carry = proposed(
        "evidence-applicability",
        json!({
            "origin":{"kind":"context-record","id":"original"},
            "target":{"subject":"intent.json","revision":"next","checkpoint":null},
            "attesting_driver":{"name":"driver","kind":"agent"},
            "reason":"SCRIPTED explicit frozen self-review permission control"
        }),
    );
    assert!(check(&fixture.ctx(&records), &carry).is_ok());
}

#[test]
fn applicability_resolves_original_identity_target_and_shared_policy() {
    let fixture = Fixture::new();
    let records = vec![stored("original", "review-evidence", fixture.review(), 1)];
    let ctx = fixture.ctx(&records);
    let good = proposed(
        "evidence-applicability",
        json!({"origin":{"kind":"context-record","id":"original"},"target":{"subject":"intent.json","revision":fixture.revision,"checkpoint":null},"attesting_driver":{"name":"driver","kind":"agent"},"reason":"Explicit appraisal"}),
    );
    assert!(check(&ctx, &good).is_ok());
    let mut stale = good.clone();
    stale.data["target"]["revision"] = json!("older");
    assert_eq!(
        check(&ctx, &stale).unwrap_err().rule,
        "review-applicability-current-target"
    );
    let stored_history = vec![
        records[0].clone(),
        stored("old-carry", "evidence-applicability", stale.data, 2),
    ];
    assert!(classify(
        &fixture.ctx_at(&stored_history, "intent-adversarial-review"),
        &BTreeSet::new(),
        &[],
    )
    .iter()
    .all(|(_, result)| result.is_ok()));
    let mut bad = good.clone();
    bad.data["origin"]["id"] = json!("missing");
    assert!(check(&ctx, &bad).is_err());
    let mut bad = good.clone();
    bad.data["target"]["subject"] = json!("design.json");
    assert!(check(&ctx, &bad).is_err());
    let mut bad = good;
    bad.data["attesting_driver"]["name"] = json!("");
    assert!(check(&ctx, &bad).is_err());
    let bad_records = vec![stored(
        "original",
        "review-evidence",
        json!({"gate":"intent-review","subject":"intent.json"}),
        1,
    )];
    assert!(check(&fixture.ctx(&bad_records), &proposed("evidence-applicability", json!({"origin":{"kind":"context-record","id":"original"},"target":{"subject":"intent.json","revision":fixture.revision,"checkpoint":null},"attesting_driver":{"name":"driver","kind":"agent"},"reason":"appraisal"}))).is_err());
}

#[test]
fn ledger_keeps_historical_source_identity_and_unresolved_findings_are_valid_writes() {
    let fixture = Fixture::new();
    let mut fail = fixture.review();
    fail["result"] = json!("fail");
    fail["findings"] = json!("original finding");
    fail["grounds"]["evidence"][0]["locator"] = json!("removed.json#/fact");
    let records = vec![stored("failed", "review-evidence", fail.clone(), 1)];
    let good = proposed("finding-ledger", fixture.ledger());
    assert!(check(&fixture.ctx(&records), &good).is_ok()); // unresolved, not readiness
    let mut changed = good.clone();
    changed.data["findings"][0]["statement"] = json!("paraphrase");
    assert!(check(&fixture.ctx(&records), &changed).is_err());
    let previous = stored("ledger", "finding-ledger", good.data.clone(), 2);
    let history = vec![records[0].clone(), previous];
    let mut omitted = good.clone();
    omitted.data["findings"] = json!([]);
    assert!(check(&fixture.ctx(&history), &omitted).is_err());
    let proposal = stored(
        "failed",
        "advisory-finding-proposal",
        json!({"suggestion":"not a judgment"}),
        1,
    );
    assert!(check(&fixture.ctx(&[proposal]), &good).is_err());
    let mut missing_stage = fail.clone();
    missing_stage
        .as_object_mut()
        .unwrap()
        .remove("review_stage");
    assert!(check(
        &fixture.ctx(&[stored("failed", "review-evidence", missing_stage, 1)]),
        &good
    )
    .is_err());
    let mut corrupt = fail;
    corrupt["config_version"] = json!("wrong");
    assert!(check(
        &fixture.ctx(&[stored("failed", "review-evidence", corrupt, 1)]),
        &good
    )
    .is_err());
}

#[test]
fn manifests_are_shape_not_retirement_or_count_clearance_and_proposals_stay_inert() {
    let fixture = Fixture::new();
    let ctx = fixture.ctx(&[]);
    assert!(check(
        &ctx,
        &proposed(
            "reviewer-manifest",
            json!({"gate":"intent-review","authors":[],"reason":"Explicit empty roster"})
        )
    )
    .is_ok());
    assert!(check(&ctx, &proposed("reviewer-manifest", json!({"gate":"intent-review","authors":[{"name":"a","kind":"agent"},{"name":"a","kind":"agent"}],"reason":"duplicate"}))).is_err());
    assert!(check(&ctx, &proposed("advisory-finding-proposal", json!(false))).is_ok());
    assert_eq!(
        check(&ctx, &proposed("intent-correction", json!({})))
            .unwrap_err()
            .rule,
        "intent-correction-source-baseline"
    );
}

#[test]
fn comparison_shape_can_record_partial_coverage_or_unwanted_alternative_without_clearance() {
    let fixture = Fixture::new();
    let baseline = json!({"kind":"git","commit":"before"});
    let mut cold_data = fixture.review();
    cold_data["policy_id"] = json!("intent-sufficient");
    cold_data["baseline"] = baseline.clone();
    let cold = stored("cold", "review-evidence", cold_data, 1);
    let good = proposed(
        "intent-comparison",
        json!({"subject_revision":fixture.revision,"baseline":baseline,"reconstruction_ids":["cold"],"actor":{"name":"owner","kind":"human"},"authority":"owner","dispositions":[{"reconstruction_id":"cold","alternative":"material permitted choice","disposition":"unwanted-permitted","note":"Needs correction"}]}),
    );
    assert!(check(&fixture.ctx(std::slice::from_ref(&cold)), &good).is_ok());
    let mut bad = good.clone();
    bad.data["reconstruction_ids"] = json!(["missing"]);
    assert!(check(&fixture.ctx(std::slice::from_ref(&cold)), &bad).is_err());
    let mut bad = good;
    bad.data["baseline"] = json!({"kind":"git","commit":"different"});
    assert!(check(&fixture.ctx(&[cold]), &bad).is_err());
}

#[test]
fn commission_uses_the_real_table_and_applies_exclusions_before_selection() {
    let fixture = Fixture::new();
    let mut invalid = fixture.review();
    invalid.as_object_mut().unwrap().remove("assessment_id");
    let records = vec![
        stored("invalid", "review-evidence", invalid, 1),
        stored("proposal", "advisory-finding-proposal", json!(false), 2),
    ];
    let packet = json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change","initial_input":fixture.input,"artifact_root":fixture.root,"context":records,"work_slots":fixture.workflow.work_slots,"workflow_graph":fixture.workflow,"current_state":"intent-review","state_visit":1}});
    let before = packet.clone();
    let diagnosed = crate::commission::inspect_with_stage(
        &packet,
        Some("intent-review"),
        None,
        Some("aggregate"),
    )
    .unwrap();
    let diagnostic = diagnosed["record_diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["record_id"] == "invalid")
        .expect("the malformed record remains diagnosed, not silently accepted");
    assert_eq!(diagnostic["rule"], "review-assessment-identity");
    assert!(diagnostic["reason"]
        .as_str()
        .unwrap()
        .contains("assessment_id"));
    assert!(!diagnosed["context"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record["id"] == "invalid" || record["id"] == "proposal"));
    let output = crate::commission::inspect_with_stage_excluding(
        &packet,
        Some("intent-review"),
        None,
        Some("aggregate"),
        &BTreeSet::from(["invalid".into()]),
        &[],
    )
    .unwrap();
    assert!(!output["context"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record["id"] == "invalid" || record["id"] == "proposal"));
    assert!(!output["record_diagnostics"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["record_id"] == "invalid")));
    assert_eq!(packet, before);
}

#[test]
fn exclusions_and_state_change_do_not_reclassify_stored_review_by_write_only_extension() {
    fn no_new_writes(_: &AdmissionContext<'_>, _: &CandidateRecord) -> Result<(), String> {
        Err("new write not allowed".into())
    }
    let fixture = Fixture::new();
    let records = vec![
        stored("valid", "review-evidence", fixture.review(), 1),
        stored("invalid", "review-evidence", json!(false), 2),
    ];
    let rules = [Rule {
        kind: "review-evidence",
        id: "peer-test-only",
        applicability: Applicability::WriteOnly,
        check: no_new_writes,
    }];
    let ctx = fixture.ctx(&records);
    assert_eq!(
        check_with_extensions(
            &ctx,
            &proposed("review-evidence", fixture.review()),
            Mode::Write,
            &rules
        )
        .unwrap_err()
        .rule,
        "peer-test-only"
    );
    // The ordinary control still reaches the contributed extension above.
    // In challenge, the production B rule runs first and rejects a NEW write.
    let challenge_ctx = fixture.ctx_at(&records, "intent-adversarial-review");
    assert_eq!(
        check_with_extensions(
            &challenge_ctx,
            &proposed("review-evidence", fixture.review()),
            Mode::Write,
            &rules,
        )
        .unwrap_err()
        .rule,
        crate::challenge_admission::RULE_ID
    );
    // Both WriteOnly rules are skipped for previously stored ordinary history.
    let result = classify(&challenge_ctx, &BTreeSet::from(["invalid".into()]), &rules);
    assert_eq!(result.len(), 1);
    assert!(result[0].1.is_ok());
}
