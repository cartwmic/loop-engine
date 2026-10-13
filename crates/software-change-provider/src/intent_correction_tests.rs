//! Scripted deterministic fixtures; no semantic/owner approval claim.
use crate::{admission, commission, intent_correction};
use loop_core::{CandidateRecord, ContextRecord, SemanticSequence, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf};
struct Fixture {
    root: PathBuf,
    initial: Value,
    previous: String,
    current: Value,
    records: Vec<ContextRecord>,
}
fn row(id: &str, kind: &str, data: Value, sequence: u64) -> ContextRecord {
    ContextRecord::new(
        id,
        kind,
        data,
        SemanticSequence::new(sequence),
        Timestamp::from_unix_millis(0),
    )
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "metadata-correction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut old: Value = serde_json::from_str(include_str!(
            "../data/calibration/fixtures/intent-good.json"
        ))
        .unwrap();
        old["revision"] = json!("1");
        let previous = serde_json::to_string_pretty(&old).unwrap();
        let mut current = old;
        current["revision"] = json!("2");
        current["outcome"] = json!("SCRIPTED correction clarification");
        std::fs::write(
            root.join("intent.json"),
            serde_json::to_vec_pretty(&current).unwrap(),
        )
        .unwrap();
        let mut initial: Value =
            serde_json::from_str(include_str!("../data/configs/minimal.json")).unwrap();
        initial["artifact_root"] = json!(root);
        initial["config_version"] = json!("metadata-test-15");
        initial["extra"]["checkpoints"] = json!(["intent"]);
        let ordinary = initial["review_policies"]["intent-review"][0].clone();
        let challenge = initial["review_policies"]["intent-adversarial-review"][0].clone();
        initial["review_policies"] =
            json!({"intent-review":[ordinary],"intent-adversarial-review":[challenge]});
        let source = json!({"gate":"intent-review","policy_id":"solution-agnostic","review_stage":"aggregate","group":"intent-review.aggregate","assessment_id":"old-return","result":"fail","findings":"Accepted intent finding","subject":"intent.json","subject_revision":"1","config_version":"metadata-test-15","author":{"name":"old-reviewer","kind":"agent"},"review_contract_version":3,"grounds":{"reason":"Scripted old inspection","evidence":[{"locator":"removed-old-source.json#/fact"}]}});
        let ledger = json!({"schema_version":"1","gate":"intent-review","subject":"intent.json","subject_revision":"1","author":{"name":"driver","kind":"agent"},"findings":[{"id":"F-fix","source":{"kind":"context-record","id":"old-review"},"policy_id":"solution-agnostic","disposition":"accepted","reason":"Accepted for scripted fixture","owner_phase":"intent","task_ids":[],"review_axes":["solution-agnostic"],"status":"unresolved"}]});
        Self {
            root,
            initial,
            previous,
            current,
            records: vec![
                row("old-review", "review-evidence", source, 1),
                row("ledger", "finding-ledger", ledger, 2),
            ],
        }
    }
    fn data(&self) -> Value {
        json!({"subject":"intent.json","subject_revision":"2","prior_intent":{"revision":"1","locator":"intent.json (retained prior bytes)","sha256":format!("sha256:{:x}",Sha256::digest(self.previous.as_bytes())),"json_bytes":self.previous},"findings":[{"ledger_id":"ledger","finding_id":"F-fix"}],"author":{"name":"driver","kind":"agent"},"reason":"SCRIPTED accepted intent finding correction; no owner approval"})
    }
    fn show(&self) -> Value {
        let workflow = crate::workflow::describe_workflow(Some(&self.initial)).unwrap();
        json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change","initial_input":self.initial,"artifact_root":self.root,"context":self.records,"work_slots":workflow.work_slots,"workflow_graph":workflow,"current_state":"intent-review","state_visit":1}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn correction_checks_real_source_ownership_latest_ledger_and_exact_prior_bytes() {
    let f = Fixture::new();
    let good = f.data();
    assert!(intent_correction::validate(&good, &f.initial, &f.records).is_ok());
    for change in ["hash", "revision", "finding", "ledger", "extra"] {
        let mut bad = good.clone();
        match change {
            "hash" => bad["prior_intent"]["sha256"] = json!("sha256:bad"),
            "revision" => bad["prior_intent"]["revision"] = json!("wrong"),
            "finding" => bad["findings"][0]["finding_id"] = json!("missing"),
            "ledger" => bad["findings"][0]["ledger_id"] = json!("missing"),
            _ => bad["owner_approval"] = json!(true),
        }
        assert!(
            intent_correction::validate(&bad, &f.initial, &f.records).is_err(),
            "{change}"
        );
    }
    for (field, value) in [
        ("owner_phase", json!("design")),
        ("disposition", json!("rejected")),
    ] {
        let mut records = f.records.clone();
        records[1].data["findings"][0][field] = value;
        assert!(intent_correction::validate(&good, &f.initial, &records).is_err());
    }
    let mut records = f.records.clone();
    records.push(row(
        "new-ledger",
        "finding-ledger",
        records[1].data.clone(),
        3,
    ));
    assert!(intent_correction::validate(&good, &f.initial, &records)
        .unwrap_err()
        .contains("latest"));
    let workflow = crate::workflow::describe_workflow(Some(&f.initial)).unwrap();
    let ctx =
        admission::AdmissionContext::new(&f.initial, &f.records, "explore", 1, &workflow).unwrap();
    assert!(admission::check(
        &ctx,
        &CandidateRecord {
            id: "correction".into(),
            kind: "intent-correction".into(),
            data: good
        }
    )
    .is_ok());
}
#[test]
fn correction_baseline_and_pointer_reach_ordinary_challenge_and_owner_views_without_owner_words() {
    let mut f = Fixture::new();
    assert!(commission::intent_revision_baseline(&f.records, &f.initial, &f.current).is_err());
    f.records
        .push(row("correction", "intent-correction", f.data(), 3));
    let baseline = commission::intent_revision_baseline(&f.records, &f.initial, &f.current)
        .unwrap()
        .unwrap();
    assert_eq!(baseline.source_kind, "intent-correction");
    assert_eq!(baseline.baseline.json_bytes, f.previous);
    let mut current = f.records[0].data.clone();
    current["subject_revision"] = json!("2");
    current["result"] = json!("pass");
    current["findings"] = json!("");
    current["author"]["name"] = json!("new-reviewer");
    current["grounds"]["evidence"][0]["locator"] =
        json!("record:correction#/data/prior_intent/json_bytes");
    f.records
        .push(row("current-review", "review-evidence", current, 4));
    for gate in ["intent-review", "intent-adversarial-review"] {
        let output =
            commission::inspect_with_stage(&f.show(), Some(gate), None, Some("aggregate")).unwrap();
        assert!(output["context"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == "correction"));
        let view = &output["owner_approval_view"];
        assert_eq!(
            view["exact_substantive_wording_delta"]["source_kind"],
            "intent-correction"
        );
        assert_eq!(
            view["exact_substantive_wording_delta"]["from_revision"],
            "1"
        );
        assert!(view["owner_sources"].as_array().unwrap().is_empty());
        assert_eq!(view["actors"]["owner_approval"]["status"], "not-performed");
    }
    let bytes = std::fs::read(f.root.join("intent.json")).unwrap();
    let fact = json!({"subject":"intent.json","revision":"2","sha256":format!("{:x}",Sha256::digest(bytes))});
    let mut admitted = fact.clone();
    admitted["phase"] = json!("intent");
    f.records
        .push(row("admitted", "artifact-admitted", admitted, 5));
    f.records
        .push(row("approved", "approved-artifact", fact, 6));
    let output = commission::inspect(&f.show(), Some("intent-checkpoint"), None).unwrap();
    assert!(output["context"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record["id"] == "correction"));
    assert_eq!(
        output["owner_approval_view"]["intent_revision_view"]["exact_substantive_wording_delta"]
            ["source_kind"],
        "intent-correction"
    );
    let grounds = json!({"reason":"Inspect exact retained bytes","evidence":[{"locator":"record:correction#/data/prior_intent/json_bytes"}]});
    assert!(admission::validate_citations(&grounds, &f.root, Some(3), &f.records).is_ok());
    for locator in [
        "record:missing#/data/prior_intent/json_bytes",
        "record:correction#/data/missing",
    ] {
        assert!(admission::validate_citations(
            &json!({"reason":"Inspect source","evidence":[{"locator":locator}]}),
            &f.root,
            Some(3),
            &f.records
        )
        .is_err());
    }
    let unknown = row("wrong", "opaque-note", json!({"prior_intent":{}}), 7);
    assert!(admission::validate_citations(&json!({"reason":"Inspect source","evidence":[{"locator":"record:wrong#/data/prior_intent"}]}),&f.root,Some(3),&[unknown]).is_err());
    let workflow = crate::workflow::describe_workflow(Some(&f.initial)).unwrap();
    let ctx =
        admission::AdmissionContext::new(&f.initial, &f.records, "later-state", 99, &workflow)
            .unwrap();
    let selected = BTreeSet::from(["correction".into()]);
    assert!(
        admission::classify_selected(&ctx, &BTreeSet::new(), Some(&selected), &[])[0]
            .1
            .is_ok()
    );
}
#[test]
fn scope_change_uses_actual_qualified_owner_baseline_not_driver_correction_without_findings() {
    let mut f = Fixture::new();
    assert!(commission::intent_revision_baseline(&f.records, &f.initial, &f.current).is_err());
    let mut fabricated = f.data();
    fabricated["findings"] = json!([]);
    assert!(intent_correction::validate(&fabricated, &f.initial, &f.records).is_err());
    let steering = json!({"target":{"kind":"all"},"instruction":"SCRIPTED actual owner scope instruction","intent_baseline":f.data()["prior_intent"]});
    f.records.push(row("owner", "user-steering", steering, 3));
    assert_eq!(
        commission::intent_revision_baseline(&f.records, &f.initial, &f.current)
            .unwrap()
            .unwrap()
            .source_kind,
        "user-steering"
    );
}
