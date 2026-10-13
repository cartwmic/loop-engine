//! Scripted unit fixtures, not execution/reviewer judgments.
use super::*;
use loop_core::{SemanticSequence, Timestamp};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
struct Fixture {
    root: PathBuf,
    input: Value,
    workflow: Workflow,
    context: Vec<ContextRecord>,
}
fn row(id: &str, kind: &str, data: Value, seq: u64) -> ContextRecord {
    ContextRecord::new(
        id,
        kind,
        data,
        SemanticSequence::new(seq),
        Timestamp::from_unix_millis(0),
    )
}
fn candidate(kind: &str, data: Value) -> CandidateRecord {
    CandidateRecord {
        id: "candidate".into(),
        kind: kind.into(),
        data,
    }
}
impl Fixture {
    fn new() -> Self {
        Self::new_at(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
    }
    fn new_at(timestamp: u128) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "provider-final-rules-{}-{}-{}",
            std::process::id(),
            timestamp,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let spec = json!({"id":"proof","command":"unit-fixture-not-executed","args":[],"owner":"driver","obligation":"scripted unit evidence"});
        let docs = [
            (
                "intent.json",
                json!({"revision":"1","author":{"name":"owner","kind":"human"},"acceptance":[{"id":"AC-1","statement":"fixture outcome"}],"outcome":"fixture","baseline":{"kind":"git","commit":"before"}}),
            ),
            (
                "design.json",
                json!({"revision":"1","author":{"name":"writer","kind":"agent"}}),
            ),
            (
                "plan.json",
                json!({"revision":"1","tasks":[{"id":"T1"}],"proof_commands":[spec.clone()]}),
            ),
            (
                "implementation-report.json",
                json!({"revision":"1","author":{"name":"implementer","kind":"agent"}}),
            ),
            (
                "validation-report.json",
                json!({"revision":"1","author":{"name":"driver","kind":"agent"},"implementation_revision":"1","command_evidence_ids":["command"],"criteria":[{"criterion_id":"AC-1","verdict_ids":["criterion"]}],"goal_verdict_ids":["goal"]}),
            ),
            ("reconciliation.json", json!({"revision":"1"})),
        ];
        for (name, value) in docs {
            std::fs::write(root.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
        let mut input: Value =
            serde_json::from_str(include_str!("../../data/configs/minimal.json")).unwrap();
        input["artifact_root"] = json!(root);
        input["config_version"] = json!("final-test-15");
        let workflow = crate::workflow::describe_workflow(Some(&input)).unwrap();
        let cwd = std::env::current_dir().unwrap();
        let identity = crate::source_identity::read(&cwd).unwrap();
        let command = json!({"proof_id":"proof","spec":spec,"external_result":{"external":true,"id":"proof","command":"unit-fixture-not-executed","args":[],"cwd":cwd,"exit_code":0,"stdout":"SCRIPTED UNIT FIXTURE, NOT AN EXECUTION","stderr":"","producer":{"name":"unit-fixture","kind":"script"},"source":identity}});
        let verdict = json!({"subject":"validation-report.json","subject_revision":"1","checkpoint":"validation-checkpoint.json","author":{"name":"independent","kind":"script"},"result":"pass","findings":[],"evidence_context_ids":["command"]});
        let owner = row(
            "owner",
            "user-steering",
            json!({"target":{"kind":"all"},"instruction":"Scripted unit owner words"}),
            1,
        );
        let command = row("command", "command-evidence", command, 2);
        let goal = row("goal", "goal-verdict", verdict, 3);
        let mut criterion = goal.clone();
        criterion.id = "criterion".into();
        criterion.kind = "criterion-verdict".into();
        criterion.sequence = SemanticSequence::new(4);
        criterion.data["criterion_id"] = json!("AC-1");
        crate::checkpoint::create(
            crate::checkpoint::CheckpointPhase::Implementation,
            &root,
            &cwd,
        )
        .unwrap();
        crate::checkpoint::create(crate::checkpoint::CheckpointPhase::Validation, &root, &cwd)
            .unwrap();
        Self {
            root,
            input,
            workflow,
            context: vec![owner, command, goal, criterion],
        }
    }
    fn ctx(&self) -> AdmissionContext<'_> {
        AdmissionContext::new(&self.input, &self.context, "validation", 7, &self.workflow).unwrap()
    }
    fn fact(&self, subject: &str, phase: Option<&str>) -> Value {
        let bytes = std::fs::read(self.root.join(subject)).unwrap();
        let mut fact = json!({"subject":subject,"revision":"1","sha256":format!("{:x}",Sha256::digest(bytes))});
        if let Some(phase) = phase {
            fact["phase"] = json!(phase);
        }
        fact
    }
    fn prepare_narrative(&mut self) -> Value {
        let fact = self.fact("design.json", None);
        self.context
            .push(row("approval", "approved-artifact", fact.clone(), 5));
        let markdown = b"Scripted unit narrative\n";
        std::fs::write(self.root.join("design-narrative.md"), markdown).unwrap();
        let binding = json!({"revision":"1/1","author":{"name":"writer","kind":"agent"},"artifact":{"subject":"design.json","revision":"1","sha256":fact["sha256"]},"approved_artifact_record_id":"approval","markdown":{"file":"design-narrative.md","sha256":format!("{:x}",Sha256::digest(markdown))}});
        std::fs::write(
            self.root.join("design-narrative.json"),
            serde_json::to_vec(&binding).unwrap(),
        )
        .unwrap();
        json!({"phase":"design","revision":"1/1","sha256":format!("{:x}",Sha256::digest(markdown))})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn remaining_existing_kind_write_controls_are_real_checks_not_inventory_facades() {
    let mut f = Fixture::new();
    let narrative = f.prepare_narrative();
    let mut reconc = json!({"revision":"1","sha256":format!("sha256:{:x}",Sha256::digest(std::fs::read(f.root.join("reconciliation.json")).unwrap())),"documents":{"intent":"1","design":"1","plan":"1"}});
    reconc["artifact_admitted"] = f.fact("implementation-report.json", Some("implementation"));
    reconc["approved_artifact"] = f.fact("implementation-report.json", None);
    let cases = vec![
        ("criterion-verdict", f.context[3].data.clone()),
        ("goal-verdict", f.context[2].data.clone()),
        ("command-evidence", f.context[1].data.clone()),
        (
            "validation-command",
            json!({"spec":{"id":"additional","command":"checker","args":[],"owner":"driver","obligation":"extra proof"}}),
        ),
        (
            "criterion-revalidation",
            json!({"subject_revision":"1","reason":"declared scope","affected_criteria":["AC-1"],"change_kind":"material"}),
        ),
        (
            "owner-decision",
            json!({"checkpoint":"design-checkpoint","subject":"design.json","subject_revision":"1","decision":"approve","text":"SCRIPTED approval","actor":{"name":"owner","kind":"human"}}),
        ),
        ("approved-artifact", f.fact("design.json", None)),
        ("artifact-admitted", f.fact("design.json", Some("design"))),
        ("narrative-admitted", narrative),
        ("reconciliation-decision", reconc),
    ];
    for (kind, data) in cases {
        let result = check(&f.ctx(), &candidate(kind, data));
        assert!(result.is_ok(), "{kind}: {result:?}");
        assert!(
            check(&f.ctx(), &candidate(kind, json!(false))).is_err(),
            "{kind} opaque invalid data escaped"
        );
    }
    assert!(unmigrated_kinds().is_empty());
}
#[test]
fn verdict_result_references_coverage_and_current_target_controls() {
    let f = Fixture::new();
    let good = f.context[3].data.clone();
    for (field, value) in [
        ("result", json!("unknown")),
        ("findings", json!(["pass cannot have findings"])),
        ("author", json!({"name":"","kind":"agent"})),
        ("criterion_id", json!("unknown")),
        ("checkpoint", json!("wrong")),
        ("evidence_context_ids", json!(["owner"])),
        ("subject_revision", json!("old")),
    ] {
        let mut data = good.clone();
        data[field] = value;
        assert!(
            check(&f.ctx(), &candidate("criterion-verdict", data)).is_err(),
            "{field}"
        );
    }
    let mut unknown = good;
    unknown["criterion_id"] = json!("AC-999");
    assert!(check(&f.ctx(), &candidate("criterion-verdict", unknown))
        .unwrap_err()
        .message
        .contains("unknown current criterion"));
    let mut partial = f.context[2].data.clone();
    partial["result"] = json!("fail");
    partial["findings"] = json!(["actual unit finding"]);
    assert!(check(&f.ctx(), &candidate("goal-verdict", partial)).is_ok()); // no gate clearance requirement
}
#[test]
fn commands_check_named_spec_scope_and_current_source_but_history_is_shape_only() {
    let f = Fixture::new();
    let good = f.context[1].data.clone();
    let mut bad = good.clone();
    bad["spec"]["args"] = json!(["changed"]);
    bad["external_result"]["args"] = json!(["changed"]);
    assert!(check(&f.ctx(), &candidate("command-evidence", bad)).is_err());
    let mut bad = good.clone();
    bad["external_result"]["source"]["scope"] = json!("unknown");
    assert!(check(&f.ctx(), &candidate("command-evidence", bad)).is_err());
    let mut stale = good;
    stale["external_result"]["source"]["state_sha256"] = json!("old-state");
    assert!(check(&f.ctx(), &candidate("command-evidence", stale.clone())).is_err());
    let records = vec![row("old-command", "command-evidence", stale, 1)];
    let ctx = AdmissionContext::new(&f.input, &records, "later-state", 999, &f.workflow).unwrap();
    assert!(classify(&ctx, &BTreeSet::new(), &[])[0].1.is_ok());
    let duplicate = json!({"spec":{"id":"proof","command":"anything","args":[],"owner":"driver","obligation":"not a replacement"}});
    assert!(check(&f.ctx(), &candidate("validation-command", duplicate)).is_err());
}
#[test]
fn standalone_and_enclosing_facts_refuse_shape_digest_and_identity_defects() {
    let f = Fixture::new();
    for kind in ["approved-artifact", "artifact-admitted"] {
        let mut good = f.fact(
            "design.json",
            (kind == "artifact-admitted").then_some("design"),
        );
        good["sha256"] = json!("a".repeat(64));
        assert!(check(&f.ctx(), &candidate(kind, good)).is_err());
    }
    let mut bad = f.fact("design.json", Some("design"));
    let mut approval = f.fact("design.json", None);
    approval["revision"] = json!("different");
    bad["approved_artifact"] = approval;
    assert!(check(&f.ctx(), &candidate("artifact-admitted", bad)).is_err());
    let mut owner = json!({"checkpoint":"design-checkpoint","subject":"design.json","subject_revision":"1","decision":"reject","target":"artifact","reason":"actual reason","text":"verbatim","actor":{"name":"owner","kind":"human"}});
    assert!(check(&f.ctx(), &candidate("owner-decision", owner.clone())).is_ok());
    owner["target"] = json!("unknown");
    assert!(check(&f.ctx(), &candidate("owner-decision", owner)).is_err());
}
#[test]
fn canonical_write_and_narrow_old_verdict_carry_classification_preserve_bytes() {
    let f = Fixture::new();
    let canonical = json!({"origin":{"kind":"context-record","id":"goal"},"target":{"subject":"validation-report.json","revision":"1","checkpoint":{"phase":"validation","report_revision":"1"}},"attesting_driver":{"name":"driver","kind":"agent"},"reason":"Explicit applicability"});
    assert!(check(
        &f.ctx(),
        &candidate("evidence-applicability", canonical.clone())
    )
    .is_ok());
    let mut old = canonical.clone();
    old["target"].as_object_mut().unwrap().remove("revision");
    old["target"]["subject_revision"] = json!("1");
    assert!(check(&f.ctx(), &candidate("evidence-applicability", old.clone())).is_err());
    let mut context = f.context.clone();
    context.push(row("old-carry", "evidence-applicability", old.clone(), 5));
    let ctx = AdmissionContext::new(&f.input, &context, "later", 999, &f.workflow).unwrap();
    let selected = BTreeSet::from(["old-carry".into()]);
    assert!(
        classify_selected(&ctx, &BTreeSet::new(), Some(&selected), &[])[0]
            .1
            .is_ok()
    );
    assert_eq!(context.last().unwrap().data, old);
    let mut conflict = canonical;
    conflict["target"]["subject_revision"] = json!("different");
    context.last_mut().unwrap().data = conflict;
    let ctx = AdmissionContext::new(&f.input, &context, "later", 999, &f.workflow).unwrap();
    assert!(
        classify_selected(&ctx, &BTreeSet::new(), Some(&selected), &[])[0]
            .1
            .is_err()
    );
}
#[test]
fn prepare_validation_runs_actual_write_checks_and_drops_refused_candidates() {
    let f = Fixture::new();
    let show = json!({"operation":"show","status":"completed","result":{"workflow_id":"software-change","current_state":"validation","state_visit":1,"initial_input":f.input,"artifact_root":f.root,"context":f.context,"workflow_graph":f.workflow,"work_slots":f.workflow.work_slots}});
    let mut packet = json!({"show":show,"working_directory":std::env::current_dir().unwrap(),"revision":"new","author":{"name":"driver","kind":"agent"},"command_results":[f.context[1].data["external_result"]]});
    let good = crate::validation::prepare(&packet).unwrap();
    assert_eq!(good["command_candidates"].as_array().unwrap().len(), 1);
    assert_eq!(good["commands_complete"], true);
    packet["command_results"][0]["source"]["scope"] = json!("unknown");
    let bad = crate::validation::prepare(&packet).unwrap();
    assert!(bad["command_candidates"].as_array().unwrap().is_empty());
    assert_eq!(bad["commands_complete"], false);
    assert!(bad["diagnostics"]
        .to_string()
        .contains("command-evidence-current-source"));
}

#[test]
fn criterion_and_goal_ledger_statements_derive_existing_literal_normalization() {
    let f = Fixture::new();
    for kind in ["criterion-verdict", "goal-verdict"] {
        let mut source = if kind == "criterion-verdict" {
            f.context[3].clone()
        } else {
            f.context[2].clone()
        };
        source.id = "failed-source".into();
        source.sequence = SemanticSequence::new(5);
        source.data["result"] = json!("fail");
        source.data["findings"] = json!(["first exact finding", "second exact finding"]);
        let mut records = f.context.clone();
        records.push(source);
        let policy = if kind == "criterion-verdict" {
            "AC-1"
        } else {
            "goal"
        };
        let data = json!({"schema_version":"1","gate":"validation-review","subject":"validation-report.json","subject_revision":"1","author":{"name":"driver","kind":"agent"},"findings":[{"id":"F-derived","source":{"kind":"context-record","id":"failed-source"},"policy_id":policy,"disposition":"rejected","reason":"Scripted disposition","owner_phase":null,"task_ids":[],"review_axes":[],"status":"recorded","summary":"Driver wording remains separate"}]});
        let config = crate::config::parse_initial_input(&f.input).unwrap();
        let parsed = crate::finding_ledger::parse_snapshot(
            &data,
            "validation-review",
            "validation-report.json",
            config.artifact_root(),
            Some(&config),
            &records,
            6,
        )
        .unwrap();
        assert_eq!(
            parsed.findings[0].statement,
            "first exact finding\\nsecond exact finding"
        );
        let mut wrong = data;
        wrong["findings"][0]["statement"] = json!("paraphrase");
        assert!(crate::finding_ledger::parse_snapshot(
            &wrong,
            "validation-review",
            "validation-report.json",
            config.artifact_root(),
            Some(&config),
            &records,
            6
        )
        .is_err());
    }
}

#[test]
fn historical_verdict_source_does_not_reopen_old_output_files_or_current_cwd() {
    let mut f = Fixture::new();
    f.context[1].data["external_result"]
        .as_object_mut()
        .unwrap()
        .remove("stdout");
    f.context[1].data["external_result"]["stdout_ref"] = json!("missing-historical-output.txt");
    assert!(
        crate::validation::historical_source_record(&f.context[2], &f.context, &f.root).is_ok()
    );
    assert!(
        crate::validation::source(&f.context[2].data, "goal-verdict", &f.context, &f.root).is_err()
    );
}

#[test]
fn fixture_cleanup_preserves_peer_with_same_timestamp() {
    let first = Fixture::new_at(0);
    let second = Fixture::new_at(0);
    drop(first);
    assert!(second.root.join("intent.json").is_file());
}
