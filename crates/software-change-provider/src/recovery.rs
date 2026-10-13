//! Append-only recovery rules shared by admission and all read-side consumers.
//! These pure rules never store context, choose finding dispositions or waive floors.

use crate::finding_ledger::{self, FindingDisposition};
use loop_core::ContextRecord;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const RECORD_WITHDRAWAL_KIND: &str = "record-withdrawal";
pub const WITHDRAWAL_RULE: &str = "a-withdrawal-guardrails";
pub const APPLICABILITY_RULE: &str = "a-applicability-origin-not-withdrawn";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Driver {
    name: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Withdrawal {
    record_ids: Vec<String>,
    reason: String,
    attesting_driver: Driver,
}

fn check_withdrawal(
    record_id: &str,
    data: &Value,
    earlier: &[ContextRecord],
    already_withdrawn: &BTreeSet<String>,
    artifact_root: Option<&Value>,
) -> Result<Vec<String>, String> {
    let check = || -> Result<Vec<String>, String> {
        let withdrawal: Withdrawal =
            serde_json::from_value(data.clone()).map_err(|e| format!("shape: {e}"))?;
        if withdrawal.reason.trim().is_empty() {
            return Err("reason must be nonblank".into());
        }
        if withdrawal.attesting_driver.name.trim().is_empty()
            || !["human", "agent", "script"].contains(&withdrawal.attesting_driver.kind.as_str())
        {
            return Err(
                "attesting_driver needs a nonblank name and human/agent/script kind".into(),
            );
        }
        if withdrawal.record_ids.is_empty()
            || withdrawal.record_ids.iter().any(|id| id.trim().is_empty())
            || withdrawal.record_ids.iter().collect::<BTreeSet<_>>().len()
                != withdrawal.record_ids.len()
        {
            return Err("record_ids must be nonempty, unique, nonblank IDs".into());
        }
        for id in &withdrawal.record_ids {
            let matches: Vec<_> = earlier.iter().filter(|row| row.id.as_str() == id).collect();
            if matches.len() != 1 {
                return Err(format!(
                    "record_ids `{id}` must identify one earlier same-run record"
                ));
            }
            let source = matches[0];
            if id == record_id || already_withdrawn.contains(id) {
                return Err(format!(
                    "record_ids `{id}` is recursive or already withdrawn"
                ));
            }
            if ![
                "review-evidence",
                "evidence-applicability",
                "validation-command",
            ]
            .contains(&source.kind.as_str())
            {
                return Err(format!(
                    "record_ids `{id}` kind `{}` cannot be withdrawn",
                    source.kind
                ));
            }
            if source.kind == "review-evidence" && source.data["result"] == "fail" {
                let gate = source.data["gate"]
                    .as_str()
                    .ok_or("failing source has no gate identity")?;
                let subject = source.data["subject"]
                    .as_str()
                    .ok_or("failing source has no subject identity")?;
                let ledger = finding_ledger::latest_snapshot_for_recovery(
                    earlier,
                    gate,
                    subject,
                    artifact_root,
                )?;
                if !ledger.is_some_and(|ledger| {
                    ledger
                        .findings
                        .iter()
                        .any(|finding| finding.source.record_id() == id)
                }) {
                    return Err(format!("record_ids failing review `{id}` needs an exact-source entry in the latest finding ledger for {gate}/{subject}"));
                }
            }
        }
        Ok(withdrawal.record_ids)
    };
    check().map_err(|message| format!("{WITHDRAWAL_RULE}: record `{record_id}` {message}"))
}

/// C's record-withdrawal rule contribution. `earlier` is the same-run durable
/// prefix before the candidate; root supplies ledger routing sources, not citations.
pub fn record_withdrawal_guardrails(
    record_id: &str,
    data: &Value,
    earlier: &[ContextRecord],
    artifact_root: Option<&Value>,
) -> Result<(), String> {
    let withdrawn = withdrawn_record_ids(earlier, artifact_root)?;
    check_withdrawal(record_id, data, earlier, &withdrawn, artifact_root).map(|_| ())
}

/// Recheck each withdrawal against its historical prefix, never a later ledger.
/// Thus later changes cannot erase an admitted withdrawal or accepted finding.
pub fn withdrawn_record_ids(
    context: &[ContextRecord],
    artifact_root: Option<&Value>,
) -> Result<BTreeSet<String>, String> {
    let mut ordered = context.to_vec();
    ordered.sort_by_key(|row| row.sequence);
    let mut withdrawn = BTreeSet::new();
    for (index, row) in ordered
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind == RECORD_WITHDRAWAL_KIND)
    {
        withdrawn.extend(check_withdrawal(
            row.id.as_str(),
            &row.data,
            &ordered[..index],
            &withdrawn,
            artifact_root,
        )?);
    }
    Ok(withdrawn)
}

/// One exclusion set before classification. Originals stay in the caller's
/// context for immutable ledger-source lookup and accepted-unresolved duties.
pub fn excluded_record_ids(
    context: &[ContextRecord],
    gate: &str,
    subject: &str,
    artifact_root: Option<&Value>,
) -> Result<BTreeSet<String>, String> {
    let mut excluded = withdrawn_record_ids(context, artifact_root)?;
    if let Ok(Some(ledger)) =
        finding_ledger::latest_snapshot_for_recovery(context, gate, subject, artifact_root)
    {
        for finding in ledger
            .findings
            .iter()
            .filter(|finding| finding.disposition == FindingDisposition::RetiredAuthor)
        {
            let Some(source) = context
                .iter()
                .find(|row| row.id.as_str() == finding.source.record_id())
            else {
                continue;
            };
            let Some(name) = source.data.pointer("/author/name").and_then(Value::as_str) else {
                continue;
            };
            let Some(kind) = source.data.pointer("/author/kind").and_then(Value::as_str) else {
                continue;
            };
            let author = crate::evidence::AuthorIdentity::new(name, kind);
            if finding_ledger::validate_retirement(context, gate, &author, u64::MAX).is_err() {
                continue;
            }
            excluded.extend(
                context
                    .iter()
                    .filter(|row| {
                        row.kind == "review-evidence"
                            && row.data["gate"] == gate
                            && row.data.pointer("/author/name").and_then(Value::as_str)
                                == Some(name)
                            && row.data.pointer("/author/kind").and_then(Value::as_str)
                                == Some(kind)
                    })
                    .map(|row| row.id.to_string()),
            );
        }
    }
    propagate_exclusion(context, &mut excluded);
    Ok(excluded)
}

fn propagate_exclusion(context: &[ContextRecord], excluded: &mut BTreeSet<String>) {
    loop {
        let previous = excluded.len();
        for carry in context
            .iter()
            .filter(|row| row.kind == "evidence-applicability")
        {
            if carry.data.pointer("/origin/kind").and_then(Value::as_str) == Some("context-record")
                && carry
                    .data
                    .pointer("/origin/id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| excluded.contains(id))
            {
                excluded.insert(carry.id.to_string());
            }
        }
        if previous == excluded.len() {
            break;
        }
    }
}

/// Additional C evidence-applicability rule; never short-circuits C/B rules.
pub fn applicability_origin_not_withdrawn(
    record_id: &str,
    data: &Value,
    earlier: &[ContextRecord],
    artifact_root: Option<&Value>,
) -> Result<(), String> {
    let mut excluded = withdrawn_record_ids(earlier, artifact_root)?;
    // A carry preserves its original judgment gate; target supplies subject.
    let mut origin_id = data.pointer("/origin/id").and_then(Value::as_str);
    let mut visited = BTreeSet::new();
    while let Some(id) = origin_id.filter(|id| visited.insert(*id)) {
        let Some(origin) = earlier.iter().find(|row| row.id.as_str() == id) else {
            break; // C's other origin rules validate existence and shape.
        };
        if origin.kind == "evidence-applicability" {
            origin_id = origin.data.pointer("/origin/id").and_then(Value::as_str);
            continue;
        }
        if origin.kind == "review-evidence" {
            if let (Some(gate), Some(subject)) = (
                origin.data.get("gate").and_then(Value::as_str),
                data.pointer("/target/subject").and_then(Value::as_str),
            ) {
                excluded.extend(excluded_record_ids(earlier, gate, subject, artifact_root)?);
            }
        }
        break;
    }
    propagate_exclusion(earlier, &mut excluded);
    if data
        .pointer("/origin/id")
        .and_then(Value::as_str)
        .is_some_and(|id| excluded.contains(id))
    {
        return Err(format!("{APPLICABILITY_RULE}: record `{record_id}` origin is withdrawn or its author is retired (directly or through applicability)"));
    }
    Ok(())
}

/// C's reviewer-manifest shape rule, sharing the existing retirement parser.
pub fn reviewer_manifest_guardrails(record_id: &str, data: &Value) -> Result<(), String> {
    let gate = data["gate"]
        .as_str()
        .filter(|gate| !gate.trim().is_empty())
        .ok_or_else(|| {
            format!("a-retirement-guardrails: record `{record_id}` gate must be nonblank")
        })?;
    if !crate::config::GATE_IDS.contains(&gate) {
        return Err(format!(
            "a-retirement-guardrails: record `{record_id}` gate `{gate}` is unknown"
        ));
    }
    finding_ledger::parse_manifest(data)
        .map(|_| ())
        .map_err(|error| format!("a-retirement-guardrails: record `{record_id}` {error}"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use loop_core::Timestamp;
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    pub struct Root {
        pub path: PathBuf,
    }
    impl Root {
        pub fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "software-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("intent.json"),
                serde_json::to_vec(&intent()).unwrap(),
            )
            .unwrap();
            fs::write(
                path.join("design.json"),
                br#"{"revision":"r1","author":{"name":"drafter","kind":"agent"}}"#,
            )
            .unwrap();
            fs::write(path.join("source.txt"), "first\nsecond\n").unwrap();
            Self { path }
        }
        pub fn value(&self) -> Value {
            json!(self.path)
        }
        pub fn input(&self, floor: u64) -> Value {
            let mut input = json!({"contract_version":2,"config_version":"recovery-1","artifact_root":self.path,
                "criterion_policy":{"required_authors":1,"goal_required_authors":1},
                "review_policies":{"intent-review":[{"id":"intent-sufficient","description":"Cold reader","required_authors":floor}],
                    "design-review":[{"id":"axis","description":"Design","required_authors":floor}],
                    "design-adversarial-review":[{"id":"axis","description":"Challenge","required_authors":floor}]},
                "artifact_schemas":{"design.json":{"type":"object"},"intent.json":{"type":"object","properties":{"revision":{"type":"string"},"author":{"type":"object","properties":{"name":{"type":"string"},"kind":{"type":"string","enum":["human","agent","script"]}},"required":["name","kind"],"additionalProperties":false}},"required":["revision","author"]}}});
            input["artifact_schemas"]["design.json"] =
                input["artifact_schemas"]["intent.json"].clone();
            input
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }
    pub fn intent() -> Value {
        json!({"revision":"r1","author":{"name":"drafter","kind":"agent"},"baseline":{"kind":"git","repository_scope":"primary","commit":"before"},"acceptance":[]})
    }
    pub fn row(id: &str, kind: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            kind,
            data,
            sequence.into(),
            Timestamp::from_unix_millis(sequence as i64),
        )
    }
    pub fn review(id: &str, sequence: u64, name: &str, result: &str) -> ContextRecord {
        row(
            id,
            "review-evidence",
            sequence,
            json!({"gate":"intent-review","subject":"intent.json","subject_revision":"r1",
            "config_version":"recovery-1","policy_id":"intent-sufficient","review_stage":"aggregate","result":result,
            "findings":if result=="fail" {"Finding"} else {""},"author":{"name":name,"kind":"agent"},
            "review_contract_version":3,"grounds":{"reason":"Inspected exact source","evidence":[{"locator":"source.txt#L2-L2"}]},"baseline":intent()["baseline"]}),
        )
    }
    pub fn ledger(
        source: &ContextRecord,
        sequence: u64,
        disposition: &str,
        status: &str,
    ) -> ContextRecord {
        row(
            "ledger",
            "finding-ledger",
            sequence,
            json!({"schema_version":"1","gate":source.data["gate"],"subject":source.data["subject"],
            "subject_revision":source.data["subject_revision"],"author":{"name":"driver","kind":"agent"},"findings":[{
                "id":"F-one","source":{"kind":"context-record","id":source.id},"policy_id":source.data["policy_id"],
                "statement":source.data["findings"].clone(),
                "disposition":disposition,"status":status,"reason":"Recorded exact-source decision",
                "owner_phase":if disposition=="accepted" {Some("intent")} else {None},"task_ids":[],
                "review_axes":if disposition=="accepted" {vec!["intent-sufficient"]} else {vec![]}
            }]}),
        )
    }
    pub fn roster(id: &str, sequence: u64, names: &[&str]) -> ContextRecord {
        row(
            id,
            "reviewer-manifest",
            sequence,
            json!({"gate":"intent-review","authors":names.iter().map(|name|json!({"name":name,"kind":"agent"})).collect::<Vec<_>>(),"reason":"Replacement roster"}),
        )
    }
    pub fn carry(id: &str, source: &str, sequence: u64, revision: &str) -> ContextRecord {
        row(
            id,
            "evidence-applicability",
            sequence,
            json!({"origin":{"kind":"context-record","id":source},"target":{"subject":"intent.json","revision":revision,"checkpoint":null},"attesting_driver":{"name":"driver","kind":"agent"},"reason":"Judgment remains applicable"}),
        )
    }
    pub fn withdrawal(id: &str, source: &str, sequence: u64) -> ContextRecord {
        row(
            id,
            RECORD_WITHDRAWAL_KIND,
            sequence,
            json!({"record_ids":[source],"reason":"Obsolete record","attesting_driver":{"name":"driver","kind":"agent"}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::{config, evidence, finding_ledger};
    use serde_json::json;

    #[test]
    fn recovery_withdrawal_guard_preserves_unresolved_identity_after_citation_drift() {
        let root = Root::new();
        let config = config::validate_config(&root.input(1)).unwrap();
        let source = review("failure", 1, "reader", "fail");
        let withdrawal = withdrawal("withdraw", "failure", 3);
        assert!(record_withdrawal_guardrails(
            "withdraw",
            &withdrawal.data,
            std::slice::from_ref(&source),
            Some(&root.value())
        )
        .unwrap_err()
        .contains("exact-source"));
        let snapshot = ledger(&source, 2, "accepted", "unresolved");
        let mut context = vec![source.clone(), snapshot];
        std::fs::write(root.path.join("source.txt"), "first\n").unwrap();
        assert!(evidence::parse_evidence_record_for_stage(
            &source.data,
            "intent.json",
            Some(&root.value()),
            false
        )
        .is_err());
        assert!(record_withdrawal_guardrails(
            "withdraw",
            &withdrawal.data,
            &context,
            Some(&root.value())
        )
        .is_ok());
        context.push(withdrawal);
        assert!(excluded_record_ids(
            &context,
            "intent-review",
            "intent.json",
            Some(&root.value())
        )
        .unwrap()
        .contains("failure"));
        let evaluated = finding_ledger::evaluate_finding_ledger(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &config,
        );
        assert!(matches!(
            evaluated.status,
            finding_ledger::FindingLedgerStatus::Blocked { .. }
        ));
        assert_eq!(
            evaluated.snapshot.unwrap().findings[0].source.record_id(),
            "failure"
        );
        let mut resolved = ledger(&source, 4, "accepted", "resolved");
        resolved.id = "resolved-ledger".into();
        context.push(resolved);
        assert!(finding_ledger::evaluate_finding_ledger(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &config
        )
        .is_satisfied());
        assert!(withdrawn_record_ids(&context, Some(&root.value()))
            .unwrap()
            .contains("failure"));
    }

    #[test]
    fn recovery_withdrawal_guard_refuses_bad_shapes_targets_and_unledgered_fails() {
        let root = Root::new();
        let valid = withdrawal("withdraw", "pass", 2);
        let pass = review("pass", 1, "reader", "pass");
        for (field, value) in [
            ("record_ids", json!([])),
            ("record_ids", json!(["pass", "pass"])),
            ("record_ids", json!(["missing"])),
            ("reason", json!(" ")),
            ("attesting_driver", json!({"name":"","kind":"agent"})),
            ("extra", json!(true)),
        ] {
            let mut data = valid.data.clone();
            data[field] = value;
            assert!(record_withdrawal_guardrails(
                "withdraw",
                &data,
                std::slice::from_ref(&pass),
                Some(&root.value())
            )
            .unwrap_err()
            .contains("withdraw"));
        }
        for kind in [
            "advice-answer",
            "finding-ledger",
            "owner-decision",
            "record-withdrawal",
        ] {
            let source = row("pass", kind, 1, json!({}));
            assert!(
                record_withdrawal_guardrails("withdraw", &valid.data, &[source], None).is_err()
            );
        }
        let context = vec![pass, valid.clone()];
        assert!(
            record_withdrawal_guardrails("again", &valid.data, &context, Some(&root.value()))
                .unwrap_err()
                .contains("already withdrawn")
        );
        let failing = review("pass", 1, "reader", "fail");
        let mut malformed = ledger(&failing, 2, "rejected", "recorded");
        malformed.data["extra"] = json!(true);
        assert!(record_withdrawal_guardrails(
            "withdraw",
            &valid.data,
            &[failing, malformed],
            Some(&root.value())
        )
        .is_err());
    }

    #[test]
    fn recovery_retirement_excludes_malformed_author_records_and_transitive_carries_before_parsing()
    {
        let root = Root::new();
        let config = config::validate_config(&root.input(1)).unwrap();
        let pass = review("retired-source", 2, "old", "pass");
        let mut malformed = review("malformed", 5, "old", "pass");
        malformed.data["result"] = json!(7);
        let context = vec![
            roster("before", 1, &["old"]),
            pass.clone(),
            roster("after", 3, &["new"]),
            ledger(&pass, 4, "retired-author", "recorded"),
            malformed,
            carry("carry-1", "malformed", 6, "r1"),
            carry("carry-2", "carry-1", 7, "r1"),
            review("replacement", 8, "new", "pass"),
        ];
        let excluded = excluded_record_ids(
            &context,
            "intent-review",
            "intent.json",
            Some(&root.value()),
        )
        .unwrap();
        assert!(excluded.is_superset(&BTreeSet::from([
            "retired-source".into(),
            "malformed".into(),
            "carry-1".into(),
            "carry-2".into()
        ])));
        let ledger = finding_ledger::latest_snapshot_for_recovery(
            &context,
            "intent-review",
            "intent.json",
            Some(&root.value()),
        )
        .unwrap()
        .unwrap();
        let result = evidence::evaluate_staged_evidence_with_dispositions(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &evidence::AuthorIdentity::new("drafter", "agent"),
            "recovery-1",
            config.staged_axes_for_gate("intent-review").unwrap(),
            config.axis_namespace(),
            Some(&root.value()),
            Some(&ledger),
            false,
        );
        assert!(result.is_satisfied(), "{:?}", result);
        let un_staged = evidence::evaluate_evidence_with_dispositions(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &evidence::AuthorIdentity::new("drafter", "agent"),
            "recovery-1",
            config.axes_for_gate("intent-review").unwrap(),
            config.axis_namespace(),
            Some(&root.value()),
            Some(&ledger),
        );
        assert!(un_staged.is_satisfied(), "{:?}", un_staged);
        let mut readded = context;
        readded.push(roster("readded", 9, &["old", "new"]));
        assert!(!excluded_record_ids(
            &readded,
            "intent-review",
            "intent.json",
            Some(&root.value())
        )
        .unwrap()
        .contains("malformed"));
    }

    #[test]
    fn recovery_withdrawn_origins_and_carries_are_rejected_by_contribution_rule() {
        let root = Root::new();
        let context = vec![
            review("origin", 1, "reader", "pass"),
            carry("carry-1", "origin", 2, "r1"),
            carry("carry-2", "carry-1", 3, "r1"),
            withdrawal("withdraw", "origin", 4),
        ];
        let excluded = excluded_record_ids(
            &context,
            "intent-review",
            "intent.json",
            Some(&root.value()),
        )
        .unwrap();
        assert_eq!(
            excluded,
            BTreeSet::from(["origin".into(), "carry-1".into(), "carry-2".into()])
        );
        let candidate = carry("candidate", "carry-2", 5, "r1");
        assert!(applicability_origin_not_withdrawn(
            "candidate",
            &candidate.data,
            &context,
            Some(&root.value())
        )
        .unwrap_err()
        .contains(APPLICABILITY_RULE));
        let fresh = carry("candidate", "fresh", 5, "r1");
        assert!(
            applicability_origin_not_withdrawn(
                "candidate",
                &fresh.data,
                &context,
                Some(&root.value())
            )
            .is_ok(),
            "C validates existence separately"
        );
    }

    #[test]
    fn recovery_retirement_rule_reuses_closed_roster_checks() {
        let valid = roster("roster", 1, &["reader"]);
        assert!(reviewer_manifest_guardrails("roster", &valid.data).is_ok());
        for (field, value) in [
            (
                "authors",
                json!([{"name":"r","kind":"agent"},{"name":"r","kind":"agent"}]),
            ),
            ("reason", json!("")),
            ("gate", json!("missing")),
            ("extra", json!(1)),
        ] {
            let mut data = valid.data.clone();
            data[field] = value;
            assert!(reviewer_manifest_guardrails("roster", &data).is_err());
        }
    }
    #[test]
    fn recovery_ledger_identity_stays_strict_when_source_fields_cannot_be_read() {
        let root = Root::new();
        let config = config::validate_config(&root.input(1)).unwrap();
        let mut source = review("source", 1, "reader", "fail");
        let snapshot = ledger(&source, 2, "rejected", "recorded");
        source.data["author"] = json!({});
        let context = vec![source, snapshot];
        let evaluation = finding_ledger::evaluate_finding_ledger(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &config,
        );
        assert!(matches!(
            evaluation.status,
            finding_ledger::FindingLedgerStatus::Malformed { .. }
        ));
        assert!(record_withdrawal_guardrails(
            "withdraw",
            &withdrawal("withdraw", "source", 3).data,
            &context,
            Some(&root.value())
        )
        .is_err());
    }
    #[test]
    fn recovery_new_carries_refuse_retired_origins_and_chains_without_erasing_pending_findings() {
        let root = Root::new();
        let config = config::validate_config(&root.input(1)).unwrap();
        let retired = review("retired-origin", 2, "old", "pass");
        let pending = review("pending-origin", 4, "active", "fail");
        let mut snapshot = ledger(&retired, 5, "retired-author", "recorded");
        let mut finding = ledger(&pending, 5, "accepted", "unresolved").data["findings"][0].clone();
        finding["id"] = json!("F-pending");
        snapshot.data["findings"]
            .as_array_mut()
            .unwrap()
            .push(finding);
        let mut context = vec![
            roster("before", 1, &["old", "active"]),
            retired,
            roster("after", 3, &["active"]),
            pending,
            snapshot,
            carry("retired-carry", "retired-origin", 6, "r1"),
            carry("retired-chain", "retired-carry", 7, "r1"),
        ];
        let original = context.clone();
        for origin in ["retired-origin", "retired-carry", "retired-chain"] {
            let candidate = carry("candidate", origin, 8, "r1");
            let error = applicability_origin_not_withdrawn(
                "candidate",
                &candidate.data,
                &context,
                Some(&root.value()),
            )
            .unwrap_err();
            assert!(
                error.contains(APPLICABILITY_RULE)
                    && error.contains("candidate")
                    && error.contains("retired")
            );
        }
        let active = carry("candidate", "pending-origin", 8, "r1");
        assert!(
            applicability_origin_not_withdrawn(
                "candidate",
                &active.data,
                &context,
                Some(&root.value()),
            )
            .is_ok(),
            "unresolved active findings are not retired or implicitly dispositioned"
        );
        let evaluation = finding_ledger::evaluate_finding_ledger(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &config,
        );
        assert!(matches!(
            evaluation.status,
            finding_ledger::FindingLedgerStatus::Blocked { .. }
        ));
        assert!(evaluation
            .snapshot
            .unwrap()
            .accepted_unresolved()
            .any(|finding| finding.source.record_id() == "pending-origin"));
        assert_eq!(
            context, original,
            "guard never rewrites stored findings or history"
        );
        context.push(roster("readded", 8, &["old", "active"]));
        for origin in ["retired-origin", "retired-chain"] {
            let candidate = carry("candidate", origin, 9, "r1");
            assert!(
                applicability_origin_not_withdrawn(
                    "candidate",
                    &candidate.data,
                    &context,
                    Some(&root.value()),
                )
                .is_ok(),
                "a re-added author is not still retired"
            );
        }
        let readded = finding_ledger::evaluate_finding_ledger(
            &context,
            "intent-review",
            "intent.json",
            "r1",
            &config,
        );
        assert!(
            !readded.is_satisfied(),
            "re-addition supplies no ledger clearance"
        );
        assert!(
            readded
                .snapshot
                .unwrap()
                .accepted_unresolved()
                .any(|finding| finding.source.record_id() == "pending-origin"),
            "re-addition still does not resolve or remove accepted findings"
        );
    }
}
