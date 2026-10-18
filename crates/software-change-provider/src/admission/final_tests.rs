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

// An explicit current command carry preserves its original execution identity.
// Synthetic unit evidence only; the outer CLI regression executes a real helper.
fn carried_command_fixture() -> Fixture {
    let mut f = Fixture::new();
    f.context[1].data["external_result"]["source"]["state_sha256"] =
        json!("sha256:prior-unit-fixture-state");
    let carry = json!({
        "origin":{"kind":"context-record","id":"command"},
        "target":{"subject":"validation-report.json","revision":"1",
            "checkpoint":{"phase":"validation","report_revision":"1"}},
        "attesting_driver":{"name":"driver","kind":"script"},
        "reason":"Only unrelated source changed; the complete proof obligation is unchanged."
    });
    f.context
        .push(row("command-carry", "evidence-applicability", carry, 5));
    let path = f.root.join("validation-report.json");
    let mut report: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    report["command_evidence_ids"] = json!(["command-carry"]);
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    crate::checkpoint::create(
        crate::checkpoint::CheckpointPhase::Validation,
        &f.root,
        &std::env::current_dir().unwrap(),
    )
    .unwrap();
    f
}

#[test]
fn fresh_criterion_and_goal_share_gates_verified_current_command_carry() {
    let f = carried_command_fixture();
    let before = f.context.clone();
    for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
        let result = check(&f.ctx(), &candidate(kind, f.context[index].data.clone()));
        assert!(result.is_ok(), "{kind}: {result:?}");
    }
    let report: Value =
        serde_json::from_slice(&std::fs::read(f.root.join("validation-report.json")).unwrap())
            .unwrap();
    let result = crate::validation::evaluate(
        &f.input,
        &f.context,
        &f.root,
        &report,
        true,
        &f.workflow,
        &BTreeSet::new(),
    );
    assert!(
        result.is_ok(),
        "gate must accept the same resolved original command IDs: {result:?}"
    );
    assert_eq!(
        f.context, before,
        "eligibility never rewrites original records"
    );
}

#[test]
fn fresh_verdict_command_carry_retains_invalid_and_stale_guards() {
    for kind in ["criterion-verdict", "goal-verdict"] {
        for defect in [
            "no-carry",
            "wrong-target",
            "wrong-checkpoint",
            "stale-report-checkpoint",
            "wrong-spec",
            "failed-command",
            "non-ancestor",
            "duplicate-proof",
            "unindexed-command",
            "wrapper-reference",
            "withdrawn-carry",
            "wrong-origin",
            "self-authored",
        ] {
            let mut f = carried_command_fixture();
            let mut data = f.context[if kind == "criterion-verdict" { 3 } else { 2 }]
                .data
                .clone();
            let expected = match defect {
                "no-carry" => {
                    let path = f.root.join("validation-report.json");
                    let mut report: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    report["command_evidence_ids"] = json!(["command"]);
                    std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
                    "stale-source"
                }
                "wrong-target" => {
                    f.context[4].data["target"]["revision"] = json!("wrong-target");
                    "target revision is stale"
                }
                "wrong-checkpoint" => {
                    f.context[4].data["target"]["checkpoint"]["report_revision"] = json!("wrong");
                    "target checkpoint is not current"
                }
                "stale-report-checkpoint" => {
                    let path = f.root.join("validation-report.json");
                    let mut report: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    report["author"]["name"] = json!("changed-report-driver");
                    std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
                    "checkpoint mismatch: report bytes changed"
                }
                "wrong-spec" => {
                    f.context[1].data["spec"]["obligation"] = json!("a changed proof obligation");
                    "differs from effective proof obligation"
                }
                "failed-command" => {
                    f.context[1].data["external_result"]["exit_code"] = json!(1);
                    "failed command"
                }
                "non-ancestor" => {
                    f.context[1].data["external_result"]["source"]["head"] = json!("0".repeat(40));
                    "ancestor-or-equal source"
                }
                "duplicate-proof" => {
                    let path = f.root.join("validation-report.json");
                    let mut report: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    report["command_evidence_ids"] = json!(["command-carry", "duplicate-carry"]);
                    std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
                    let mut duplicate = f.context[4].clone();
                    duplicate.id = "duplicate-carry".into();
                    duplicate.sequence = SemanticSequence::new(6);
                    f.context.push(duplicate);
                    "duplicate command proof ID"
                }
                "unindexed-command" => {
                    let mut other = f.context[1].clone();
                    other.id = "unindexed".into();
                    other.data["external_result"]["source"] =
                        crate::source_identity::read(&std::env::current_dir().unwrap()).unwrap();
                    other.sequence = SemanticSequence::new(6);
                    f.context.push(other);
                    data["evidence_context_ids"] = json!(["unindexed"]);
                    "current indexed command evidence"
                }
                "wrapper-reference" => {
                    data["evidence_context_ids"] = json!(["command-carry"]);
                    "unsupported evidence reference"
                }
                "wrong-origin" => {
                    f.context[4].data["origin"]["kind"] = json!("selected-assignment-output");
                    "invalid command applicability declaration"
                }
                "self-authored" => {
                    let path = f.root.join("validation-report.json");
                    let report: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    data["author"] = report["author"].clone();
                    "self-authorship cannot satisfy criterion policy"
                }
                "withdrawn-carry" => {
                    f.context.push(row(
                        "withdraw",
                        "record-withdrawal",
                        json!({
                            "attesting_driver":{"name":"driver","kind":"agent"},
                            "reason":"Withdraw fixture carry","record_ids":["command-carry"]
                        }),
                        6,
                    ));
                    "excluded record"
                }
                _ => unreachable!(),
            };
            if matches!(defect, "no-carry" | "duplicate-proof") {
                // These cases intentionally change the current command index,
                // not checkpoint validity. Bind those exact report bytes so the
                // named source/duplicate guard, rather than a stale checkpoint,
                // is actually reached. The separate stale-report case above
                // keeps checkpoint invalidity as an explicit refusal control.
                crate::checkpoint::create(
                    crate::checkpoint::CheckpointPhase::Validation,
                    &f.root,
                    &std::env::current_dir().unwrap(),
                )
                .unwrap();
            }
            let before = f.context.clone();
            let result = check(&f.ctx(), &candidate(kind, data));
            let refusal = result.expect_err(defect);
            assert!(
                refusal.message.contains(expected),
                "{kind}/{defect}: {refusal:?}"
            );
            assert_eq!(
                f.context, before,
                "refusal must not rewrite history: {defect}"
            );
        }
    }
}

#[test]
fn failed_current_commands_admit_negative_verdicts_but_never_clear_validation() {
    let mut f = Fixture::new();
    f.context[1].data["external_result"]["exit_code"] = json!(1);
    for index in [2, 3] {
        f.context[index].data["result"] = json!("fail");
        f.context[index].data["findings"] = json!(["The scripted current execution failed."]);
    }
    let before = f.context.clone();
    for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
        assert!(
            check(&f.ctx(), &candidate(kind, f.context[index].data.clone())).is_ok(),
            "{kind}"
        );
        let mut pass = f.context[index].data.clone();
        pass["result"] = json!("pass");
        pass["findings"] = json!([]);
        let refusal = check(&f.ctx(), &candidate(kind, pass)).unwrap_err();
        assert!(
            refusal
                .message
                .contains("passing verdict references a failed command"),
            "{refusal:?}"
        );
    }
    let report: Value =
        serde_json::from_slice(&std::fs::read(f.root.join("validation-report.json")).unwrap())
            .unwrap();
    let refusal = crate::validation::evaluate(
        &f.input,
        &f.context,
        &f.root,
        &report,
        true,
        &f.workflow,
        &BTreeSet::new(),
    )
    .unwrap_err();
    assert!(
        refusal.contains("failed external command result"),
        "{refusal}"
    );
    assert_eq!(
        f.context, before,
        "negative admission and gate refusal never rewrite history"
    );
}

#[test]
fn failed_command_carries_are_refused_even_for_negative_verdict_admission() {
    let mut f = carried_command_fixture();
    f.context[1].data["external_result"]["exit_code"] = json!(1);
    for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
        let mut negative = f.context[index].data.clone();
        negative["result"] = json!("fail");
        negative["findings"] = json!(["The original scripted execution failed."]);
        let refusal = check(&f.ctx(), &candidate(kind, negative)).unwrap_err();
        assert!(
            refusal.message.contains(
                "applicable external result must succeed and name an ancestor-or-equal source"
            ),
            "{kind}: {refusal:?}"
        );
    }
}

#[test]
fn an_unreferenced_failed_current_command_does_not_block_positive_verdict_admission() {
    let mut f = Fixture::new();
    let mut failed = f.context[1].clone();
    failed.id = "failed-current".into();
    failed.sequence = SemanticSequence::new(5);
    failed.data["proof_id"] = json!("failed-proof");
    failed.data["spec"]["id"] = json!("failed-proof");
    failed.data["external_result"]["id"] = json!("failed-proof");
    failed.data["external_result"]["exit_code"] = json!(1);
    let path = f.root.join("plan.json");
    let mut plan: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    plan["proof_commands"]
        .as_array_mut()
        .unwrap()
        .push(failed.data["spec"].clone());
    std::fs::write(path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let path = f.root.join("validation-report.json");
    let mut report: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    report["command_evidence_ids"] = json!(["command", "failed-current"]);
    std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
    f.context.push(failed);
    crate::checkpoint::create(
        crate::checkpoint::CheckpointPhase::Validation,
        &f.root,
        &std::env::current_dir().unwrap(),
    )
    .unwrap();
    for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
        assert!(
            check(&f.ctx(), &candidate(kind, f.context[index].data.clone())).is_ok(),
            "{kind}"
        );
    }
}

// A legacy worker can succeed while its captured proof fails. These are
// explicitly synthetic unit fixtures, not claims of executed native proofs.
fn legacy_command_fixture(field: &str, value: Value) -> Fixture {
    let mut f = Fixture::new();
    let cwd = std::env::current_dir().unwrap();
    let repository = crate::checkpoint::repository_identity(&cwd).unwrap();
    let spec = f.context[1].data["spec"].clone();
    let mut raw = json!({
        "cwd":cwd, "spec":spec, "stdout":"SCRIPTED UNIT CAPTURE", "stderr":"",
        "elapsed_ms":1, "timed_out":false, "spawn_error":null, "exit_code":0,
        "repository_before":repository, "repository_after":repository,
    });
    raw[field] = value;
    let bytes = serde_json::to_vec(&raw).unwrap();
    std::fs::write(f.root.join("legacy-output.json"), &bytes).unwrap();
    std::fs::write(f.root.join("legacy-stderr.txt"), b"").unwrap();
    let summary = json!({"workers":[{
        "assignment_id":"legacy-proof", "exit_code":0,
        "stdout_path":f.root.join("legacy-output.json"), "stderr_path":f.root.join("legacy-stderr.txt"),
        "selected_output_path":f.root.join("legacy-output.json"), "selected_attempt":1,
        "selected_output_sha256":format!("sha256:{:x}", Sha256::digest(&bytes)),
        "args":["validation-command",cwd,serde_json::to_string(&spec).unwrap(),"fixture"],
    }]});
    std::fs::write(
        f.root.join("legacy-summary.json"),
        serde_json::to_vec(&summary).unwrap(),
    )
    .unwrap();
    f.context[1].data = json!({
        "proof_id":"proof", "capture":{"summary":f.root.join("legacy-summary.json"),"assignment_id":"legacy-proof"}
    });
    f
}

#[test]
fn cached_legacy_proofs_recheck_pass_success_without_blocking_negative_admission() {
    for (field, value, expected) in [
        ("exit_code", json!(1), "command failed"),
        ("timed_out", json!(true), "command failed"),
        (
            "spawn_error",
            json!("scripted missing executable"),
            "command failed",
        ),
        (
            "repository_before",
            json!(format!("sha256:{}", "0".repeat(64))),
            "repository identity",
        ),
    ] {
        let f = legacy_command_fixture(field, value);
        let before = f.context.clone();
        for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
            let refusal =
                check(&f.ctx(), &candidate(kind, f.context[index].data.clone())).unwrap_err();
            assert!(
                refusal.message.contains(expected),
                "{kind}/{field}: {refusal:?}"
            );
            let mut negative = f.context[index].data.clone();
            negative["result"] = json!("fail");
            negative["findings"] = json!(["The actual scripted proof did not succeed unchanged."]);
            assert!(
                check(&f.ctx(), &candidate(kind, negative)).is_ok(),
                "{kind}/{field}"
            );
        }
        assert_eq!(
            f.context, before,
            "legacy verification never rewrites original capture evidence"
        );
    }
    let f = legacy_command_fixture("exit_code", json!(0));
    for (kind, index) in [("criterion-verdict", 3), ("goal-verdict", 2)] {
        assert!(
            check(&f.ctx(), &candidate(kind, f.context[index].data.clone())).is_ok(),
            "{kind}"
        );
    }
}

#[test]
fn fixture_cleanup_preserves_peer_with_same_timestamp() {
    let first = Fixture::new_at(0);
    let second = Fixture::new_at(0);
    drop(first);
    assert!(second.root.join("intent.json").is_file());
}
