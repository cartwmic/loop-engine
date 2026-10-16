//! Shared deterministic admission mechanics. No storage or external execution.
//!
//! Existing validated kinds and ledger-cited intent corrections use this table.
mod final_rules;
#[cfg(test)]
mod final_tests;
mod review_rules;
#[cfg(test)]
mod review_tests;
use crate::author_counts;
use loop_core::{CandidateRecord, ContextRecord, Refusal, Workflow};
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Write,
    Classify,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Applicability {
    Always,
    WriteOnly,
}

/// Frozen input plus the Bookends overlay and ordered author-count amendments.
/// All consumers use the existing resolver; there is no second policy algorithm.
pub fn effective_policy(
    initial_input: &Value,
    context: &[ContextRecord],
) -> author_counts::Resolution {
    author_counts::resolve(
        initial_input,
        &serde_json::to_value(context).expect("context JSON"),
    )
}

pub struct AdmissionContext<'a> {
    pub effective_policy: Value,
    pub initial_input: &'a Value,
    pub context: &'a [ContextRecord],
    pub current_state: &'a str,
    pub state_visit: u64,
    pub workflow: &'a Workflow,
    pub artifact_root: Option<PathBuf>,
    pub repository_root: PathBuf,
}
impl<'a> AdmissionContext<'a> {
    pub fn new(
        initial_input: &'a Value,
        context: &'a [ContextRecord],
        current_state: &'a str,
        state_visit: u64,
        workflow: &'a Workflow,
    ) -> Result<Self, String> {
        Ok(Self {
            effective_policy: effective_policy(initial_input, context).effective,
            initial_input,
            context,
            current_state,
            state_visit,
            workflow,
            artifact_root: initial_input["artifact_root"].as_str().map(PathBuf::from),
            repository_root: std::env::current_dir().map_err(|e| e.to_string())?,
        })
    }
}

/// Resolve shared file and same-run record citations without execution.
pub fn validate_citations(
    grounds: &Value,
    artifact_root: &std::path::Path,
    version: Option<u32>,
    context: &[ContextRecord],
) -> Result<(), String> {
    crate::criterion::validate_grounding_with_context(grounds, artifact_root, version, context)
}

/// A/B supply this exact signature, with their own rule ID and applicability.
/// A's exclusion calculation remains A-owned; its result is passed to classify.
pub type RuleCheck = fn(&AdmissionContext<'_>, &CandidateRecord) -> Result<(), String>;
pub struct Rule {
    pub kind: &'static str,
    pub id: &'static str,
    pub applicability: Applicability,
    pub check: RuleCheck,
}

fn amendment(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let mut records = serde_json::to_value(ctx.context).expect("context JSON");
    records
        .as_array_mut()
        .expect("context array")
        .push(serde_json::json!({
            "id":record.id,"kind":record.kind,"data":record.data
        }));
    let resolution = author_counts::resolve(ctx.initial_input, &records);
    let result = resolution
        .amendments
        .last()
        .ok_or("missing amendment resolution")?;
    if result["valid"] == true {
        Ok(())
    } else {
        Err(result["diagnostic"]
            .as_str()
            .unwrap_or("invalid amendment")
            .to_owned())
    }
}

// Pure A rule contributions, dispatched by this single production table.
fn withdrawal(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let root = ctx
        .artifact_root
        .as_ref()
        .map(|path| Value::String(path.to_string_lossy().into_owned()));
    crate::recovery::record_withdrawal_guardrails(
        record.id.as_str(),
        &record.data,
        ctx.context,
        root.as_ref(),
    )
}

fn active_origin(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), String> {
    let root = ctx
        .artifact_root
        .as_ref()
        .map(|path| Value::String(path.to_string_lossy().into_owned()));
    crate::recovery::applicability_origin_not_withdrawn(
        record.id.as_str(),
        &record.data,
        ctx.context,
        root.as_ref(),
    )
}

/// Provider-owned validated-kind inventory, including ledger-cited corrections.
pub const VALIDATED_KINDS: &[&str] = &[
    "user-steering",
    "steering-incorporation",
    "review-evidence",
    "evidence-applicability",
    "finding-ledger",
    "advisory-finding-proposal",
    "reviewer-manifest",
    "author-count-amendment",
    "intent-comparison",
    "intent-correction",
    "record-withdrawal",
    "criterion-verdict",
    "goal-verdict",
    "command-evidence",
    "validation-command",
    "criterion-revalidation",
    "owner-decision",
    "approved-artifact",
    "artifact-admitted",
    "narrative-admitted",
    "reconciliation-decision",
];

/// Inventory not yet covered by the production table.
pub fn unmigrated_kinds() -> Vec<&'static str> {
    VALIDATED_KINDS
        .iter()
        .copied()
        .filter(|kind| !RULES.iter().any(|rule| rule.kind == *kind))
        .collect()
}

// B's state rule is a peer contribution to this dispatcher, not a second
// validation boundary. Its table entries below are WriteOnly.
fn ordinary_during_challenge(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
) -> Result<(), String> {
    match crate::challenge_admission::check_new_write(
        ctx.current_state,
        record.id.as_str(),
        &record.kind,
        &record.data,
        ctx.context,
    ) {
        Some((_id, message)) => Err(message),
        None => Ok(()),
    }
}

/// One production table. Peer rules remain controller-owned extensions.
pub static RULES: &[Rule] = &[
    Rule {
        kind: "record-withdrawal",
        id: crate::recovery::WITHDRAWAL_RULE,
        applicability: Applicability::Always,
        check: withdrawal,
    },
    Rule {
        kind: "evidence-applicability",
        id: crate::recovery::APPLICABILITY_RULE,
        applicability: Applicability::Always,
        check: active_origin,
    },
    Rule {
        kind: "author-count-amendment",
        id: "author-count-amendment-valid",
        applicability: Applicability::Always,
        check: amendment,
    },
    Rule {
        kind: "user-steering",
        id: "steering-shape-references",
        applicability: Applicability::Always,
        check: review_rules::steering,
    },
    Rule {
        kind: "steering-incorporation",
        id: "steering-incorporation-references",
        applicability: Applicability::Always,
        check: review_rules::incorporation,
    },
    Rule {
        kind: "review-evidence",
        id: "review-shape-policy",
        applicability: Applicability::Always,
        check: review_rules::review,
    },
    Rule {
        kind: "review-evidence",
        id: "review-assessment-identity",
        applicability: Applicability::Always,
        check: review_rules::assessment,
    },
    Rule {
        kind: "review-evidence",
        id: "review-write-citations",
        applicability: Applicability::WriteOnly,
        check: review_rules::write_citations,
    },
    Rule {
        kind: "evidence-applicability",
        id: "review-applicability-origin-target",
        applicability: Applicability::Always,
        check: review_rules::applicability,
    },
    Rule {
        kind: "evidence-applicability",
        id: "review-applicability-current-target",
        applicability: Applicability::WriteOnly,
        check: review_rules::write_target,
    },
    Rule {
        kind: "finding-ledger",
        id: "ledger-shape-source-continuity",
        applicability: Applicability::Always,
        check: review_rules::ledger,
    },
    Rule {
        kind: "reviewer-manifest",
        id: "reviewer-manifest-shape",
        applicability: Applicability::Always,
        check: review_rules::manifest,
    },
    Rule {
        kind: "intent-comparison",
        id: "intent-comparison-shape-references",
        applicability: Applicability::Always,
        check: review_rules::comparison,
    },
    Rule {
        kind: "advisory-finding-proposal",
        id: "advisory-proposal-inert",
        applicability: Applicability::Always,
        check: review_rules::advisory,
    },
    Rule {
        kind: "criterion-verdict",
        id: "criterion-verdict-shape-source",
        applicability: Applicability::Always,
        check: final_rules::verdict,
    },
    Rule {
        kind: "criterion-verdict",
        id: "criterion-verdict-current-target",
        applicability: Applicability::WriteOnly,
        check: final_rules::verdict_current,
    },
    Rule {
        kind: "goal-verdict",
        id: "goal-verdict-shape-source",
        applicability: Applicability::Always,
        check: final_rules::verdict,
    },
    Rule {
        kind: "goal-verdict",
        id: "goal-verdict-current-target",
        applicability: Applicability::WriteOnly,
        check: final_rules::verdict_current,
    },
    Rule {
        kind: "command-evidence",
        id: "command-evidence-shape",
        applicability: Applicability::Always,
        check: final_rules::command,
    },
    Rule {
        kind: "command-evidence",
        id: "command-evidence-current-source",
        applicability: Applicability::WriteOnly,
        check: final_rules::command_current,
    },
    Rule {
        kind: "validation-command",
        id: "validation-command-shape",
        applicability: Applicability::Always,
        check: final_rules::validation_command,
    },
    Rule {
        kind: "validation-command",
        id: "validation-command-additive",
        applicability: Applicability::WriteOnly,
        check: final_rules::additional_current,
    },
    Rule {
        kind: "criterion-revalidation",
        id: "criterion-revalidation-shape",
        applicability: Applicability::Always,
        check: final_rules::revalidation,
    },
    Rule {
        kind: "criterion-revalidation",
        id: "criterion-revalidation-current-target",
        applicability: Applicability::WriteOnly,
        check: final_rules::revalidation_current,
    },
    Rule {
        kind: "owner-decision",
        id: "owner-decision-shape",
        applicability: Applicability::Always,
        check: final_rules::owner,
    },
    Rule {
        kind: "approved-artifact",
        id: "approved-artifact-shape",
        applicability: Applicability::Always,
        check: final_rules::artifact,
    },
    Rule {
        kind: "approved-artifact",
        id: "approved-artifact-current-bytes",
        applicability: Applicability::WriteOnly,
        check: final_rules::artifact_current,
    },
    Rule {
        kind: "artifact-admitted",
        id: "artifact-admitted-shape",
        applicability: Applicability::Always,
        check: final_rules::artifact,
    },
    Rule {
        kind: "artifact-admitted",
        id: "artifact-admitted-current-bytes",
        applicability: Applicability::WriteOnly,
        check: final_rules::artifact_current,
    },
    Rule {
        kind: "narrative-admitted",
        id: "narrative-admitted-shape",
        applicability: Applicability::Always,
        check: final_rules::narrative,
    },
    Rule {
        kind: "narrative-admitted",
        id: "narrative-admitted-current-bytes",
        applicability: Applicability::WriteOnly,
        check: final_rules::narrative_current,
    },
    Rule {
        kind: "reconciliation-decision",
        id: "reconciliation-decision-shape",
        applicability: Applicability::Always,
        check: final_rules::reconciliation,
    },
    Rule {
        kind: "reconciliation-decision",
        id: "reconciliation-current-bytes",
        applicability: Applicability::WriteOnly,
        check: final_rules::reconciliation_current,
    },
    Rule {
        kind: "intent-correction",
        id: "intent-correction-source-baseline",
        applicability: Applicability::Always,
        check: review_rules::correction,
    },
    Rule {
        kind: "intent-correction",
        id: "intent-correction-current-target",
        applicability: Applicability::WriteOnly,
        check: review_rules::correction_current,
    },
    Rule {
        kind: "review-evidence",
        id: crate::challenge_admission::RULE_ID,
        applicability: Applicability::WriteOnly,
        check: ordinary_during_challenge,
    },
    Rule {
        kind: "finding-ledger",
        id: crate::challenge_admission::RULE_ID,
        applicability: Applicability::WriteOnly,
        check: ordinary_during_challenge,
    },
    Rule {
        kind: "author-count-amendment",
        id: crate::challenge_admission::RULE_ID,
        applicability: Applicability::WriteOnly,
        check: ordinary_during_challenge,
    },
    Rule {
        kind: "intent-comparison",
        id: crate::challenge_admission::RULE_ID,
        applicability: Applicability::WriteOnly,
        check: ordinary_during_challenge,
    },
    Rule {
        kind: "evidence-applicability",
        id: crate::challenge_admission::RULE_ID,
        applicability: Applicability::WriteOnly,
        check: ordinary_during_challenge,
    },
];

pub fn check(ctx: &AdmissionContext<'_>, record: &CandidateRecord) -> Result<(), Refusal> {
    check_with_extensions(ctx, record, Mode::Write, &[])
}

/// Additional rules are part of the same dispatch/mode boundary, not another
/// validator. A successful rule never bypasses the remaining matching rules.
pub fn check_with_extensions(
    ctx: &AdmissionContext<'_>,
    record: &CandidateRecord,
    mode: Mode,
    extensions: &[Rule],
) -> Result<(), Refusal> {
    for rule in RULES.iter().chain(extensions) {
        if rule.kind != record.kind
            || (mode == Mode::Classify && rule.applicability == Applicability::WriteOnly)
        {
            continue;
        }
        if let Err(message) = (rule.check)(ctx, record) {
            return Err(Refusal {
                rule: rule.id.to_owned(),
                message: format!("record `{}`: {message}", record.id),
            });
        }
    }
    Ok(())
}

/// Remove exclusions before selection or validation. Rules resolve immutable
/// earlier sources and ordered policy changes against each durable prefix.
/// Returning refusals preserves malformed-record diagnostics for gate callers.
pub fn classify<'a>(
    ctx: &AdmissionContext<'a>,
    excluded_ids: &BTreeSet<String>,
    extensions: &[Rule],
) -> Vec<(&'a ContextRecord, Result<(), Refusal>)> {
    classify_selected(ctx, excluded_ids, None, extensions)
}

/// Classification after exclusions, bounded to a caller's relevant selection.
/// Historical superseded ledger/comparison records need not become new blockers.
pub fn classify_selected<'a>(
    ctx: &AdmissionContext<'a>,
    excluded_ids: &BTreeSet<String>,
    selected_ids: Option<&BTreeSet<String>>,
    extensions: &[Rule],
) -> Vec<(&'a ContextRecord, Result<(), Refusal>)> {
    ctx.context
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            !excluded_ids.contains(record.id.as_str())
                && selected_ids.is_none_or(|ids| ids.contains(record.id.as_str()))
        })
        .map(|(index, record)| {
            let prefix = &ctx.context[..index];
            let prefix_ctx = AdmissionContext {
                effective_policy: effective_policy(ctx.initial_input, prefix).effective,
                initial_input: ctx.initial_input,
                context: prefix,
                current_state: ctx.current_state,
                state_visit: ctx.state_visit,
                workflow: ctx.workflow,
                artifact_root: ctx.artifact_root.clone(),
                repository_root: ctx.repository_root.clone(),
            };
            let mut candidate = CandidateRecord {
                id: record.id.clone(),
                kind: record.kind.clone(),
                data: record.data.clone(),
            };
            if record.kind == "evidence-applicability" {
                let verdict_origin = record.data["origin"]["id"]
                    .as_str()
                    .and_then(|id| prefix.iter().find(|source| source.id.as_str() == id))
                    .is_some_and(|source| {
                        matches!(source.kind.as_str(), "criterion-verdict" | "goal-verdict")
                    });
                match crate::evidence::canonical_target(&record.data["target"], verdict_origin) {
                    Ok(target) => candidate.data["target"] = target,
                    Err(message) => {
                        return (
                            record,
                            Err(Refusal {
                                rule: "applicability-target-alias".into(),
                                message: format!("record `{}`: {message}", record.id),
                            }),
                        )
                    }
                }
            }
            (
                record,
                check_with_extensions(&prefix_ctx, &candidate, Mode::Classify, extensions),
            )
        })
        .collect()
}

/// Full-show Write preview. This does not append, execute workers, or waive
/// remaining aggregate obligations. Candidate IDs are caller-supplied diagnostics.
pub fn preview(packet: &Value, record: &CandidateRecord) -> Result<(), Refusal> {
    let context_error = |message| Refusal {
        rule: "record-preview-context".into(),
        message: format!("record `{}`: {message}", record.id),
    };
    let context: Vec<ContextRecord> = serde_json::from_value(
        packet
            .get("context")
            .cloned()
            .unwrap_or(serde_json::json!([])),
    )
    .map_err(|e| context_error(e.to_string()))?;
    let workflow: Workflow = if let Some(graph) = packet.get("workflow_graph") {
        serde_json::from_value(graph.clone()).map_err(|e| context_error(e.to_string()))?
    } else {
        crate::workflow::describe_workflow(Some(&packet["initial_input"])).map_err(context_error)?
    };
    let ctx = AdmissionContext::new(
        &packet["initial_input"],
        &context,
        packet["current_state"].as_str().unwrap_or(""),
        packet["state_visit"].as_u64().unwrap_or(0),
        &workflow,
    )
    .map_err(context_error)?;
    check(&ctx, record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};
    use serde_json::json;
    fn challenge(ctx: &AdmissionContext<'_>, _: &CandidateRecord) -> Result<(), String> {
        if ctx.current_state == "challenge" {
            Err("write refused in challenge".into())
        } else {
            Ok(())
        }
    }
    #[test]
    fn existing_inventory_is_covered_and_correction_is_explicitly_reserved() {
        assert_eq!(VALIDATED_KINDS.len(), 21);
        assert!(unmigrated_kinds().is_empty());
        let mut ids = BTreeSet::new();
        for rule in RULES {
            assert!(VALIDATED_KINDS.contains(&rule.kind));
            // One logical peer rule may apply to several dispatch kinds.
            assert!(ids.insert((rule.kind, rule.id)));
        }
        assert!(RULES.iter().any(|rule| rule.kind == "intent-correction"
            && rule.id == "intent-correction-source-baseline"));
    }

    #[test]
    fn b_challenge_rule_is_registered_once_per_kind_and_write_only() {
        let expected: BTreeSet<_> = [
            "review-evidence",
            "finding-ledger",
            "author-count-amendment",
            "intent-comparison",
            "evidence-applicability",
        ]
        .into_iter()
        .collect();
        let rules: Vec<_> = RULES
            .iter()
            .filter(|rule| rule.id == crate::challenge_admission::RULE_ID)
            .collect();
        assert_eq!(rules.len(), expected.len());
        assert!(rules
            .iter()
            .all(|rule| rule.applicability == Applicability::WriteOnly));
        assert_eq!(
            rules.iter().map(|rule| rule.kind).collect::<BTreeSet<_>>(),
            expected
        );
    }

    #[test]
    fn effective_overlay_groups_and_amendments_share_one_policy_without_mutation() {
        let input = json!({"contract_version":3,"config_version":"custom","extra":{"bookends":{"enabled":true}},"review_policies":{"intent-review":[{"id":"base","review_stage":"aggregate","required_authors":2,"group":"base-group"}]}});
        let before = input.clone();
        let base = effective_policy(&input, &[]).effective;
        let overlay = base["review_policies"]["intent-review"]
            .as_array()
            .unwrap()
            .iter()
            .find(|axis| axis["id"] == "ids-grounded")
            .unwrap();
        assert_eq!(overlay["group"], "intent-review.aggregate.ids-grounded");
        assert_eq!(overlay["review_stage"], "aggregate");
        let amendment = ContextRecord::new(
            "amend-overlay",
            "author-count-amendment",
            json!({"targets":[{"gate":"intent-review","stage":"aggregate","axis":"ids-grounded"}],"before":2,"after":1,"actor":{"name":"owner","kind":"human"},"authority":{"kind":"owner","reference":"retained answer"},"reason":"Authorized count"}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        );
        let effective = effective_policy(&input, &[amendment]);
        assert!(effective.blocked.is_empty());
        let overlay = effective.effective["review_policies"]["intent-review"]
            .as_array()
            .unwrap()
            .iter()
            .find(|axis| axis["id"] == "ids-grounded")
            .unwrap();
        assert_eq!(overlay["required_authors"], 1);
        assert_eq!(overlay["group"], "intent-review.aggregate.ids-grounded");
        assert_eq!(input, before);
    }

    #[test]
    fn record_citations_use_exact_stored_record_allowed_kinds_and_pointer() {
        let absent_root = std::path::Path::new("/absent-root-not-needed-for-records");
        for kind in [
            "review-evidence",
            "intent-comparison",
            "user-steering",
            "finding-ledger",
            "evidence-applicability",
            "criterion-verdict",
            "goal-verdict",
        ] {
            let record = ContextRecord::new(
                "source",
                kind,
                json!({"a/b":{"~value":"retained"}}),
                SemanticSequence::new(1),
                Timestamp::from_unix_millis(0),
            );
            let grounds = json!({"reason":"Inspected immutable source", "evidence":[{"locator":"record:source#/data/a~1b/~0value"}]});
            assert!(
                validate_citations(
                    &grounds,
                    absent_root,
                    Some(3),
                    std::slice::from_ref(&record)
                )
                .is_ok(),
                "{kind}"
            );
            for locator in [
                "record:missing#/data",
                "record:source#/missing",
                "record:source#L1",
                "record:bad id#/data",
                "record:source#/data/~2",
                "record:#/data",
            ] {
                let grounds =
                    json!({"reason":"Inspected source", "evidence":[{"locator":locator}]});
                assert!(
                    validate_citations(
                        &grounds,
                        absent_root,
                        Some(3),
                        std::slice::from_ref(&record)
                    )
                    .is_err(),
                    "{locator}"
                );
            }
            let duplicate = [record.clone(), record];
            assert!(validate_citations(&grounds, absent_root, Some(3), &duplicate).is_err());
        }
        let note = ContextRecord::new(
            "note",
            "opaque-note",
            json!({}),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        );
        assert!(validate_citations(
            &json!({"reason":"Inspected note", "evidence":[{"locator":"record:note#/data"}]}),
            absent_root,
            Some(3),
            &[note]
        )
        .unwrap_err()
        .contains("disallowed kind"));
    }

    #[test]
    fn amendments_are_classified_against_their_durable_prefix_not_final_policy() {
        let input = json!({"criterion_policy":{"required_authors":2}});
        let workflow = Workflow::new("test", "review", vec![], vec![]);
        let data = json!({"targets":[{"criterion":"all"}],"before":2,"after":1,"actor":{"name":"owner","kind":"human"},"authority":{"kind":"owner","reference":"retained answer"},"reason":"Owner authorized scoped count"});
        let records = vec![ContextRecord::new(
            "amendment",
            "author-count-amendment",
            data,
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        )];
        let ctx = AdmissionContext::new(&input, &records, "review", 1, &workflow).unwrap();
        assert_eq!(
            ctx.effective_policy["criterion_policy"]["required_authors"],
            1
        );
        assert!(classify(&ctx, &BTreeSet::new(), &[])[0].1.is_ok());
        let stale = CandidateRecord {
            id: "stale".into(),
            kind: "author-count-amendment".into(),
            data: records[0].data.clone(),
        };
        assert_eq!(
            check(&ctx, &stale).unwrap_err().rule,
            "author-count-amendment-valid"
        );
        let opaque = CandidateRecord {
            id: "note".into(),
            kind: "opaque-note".into(),
            data: json!(false),
        };
        assert!(check(&ctx, &opaque).is_ok());
    }

    #[test]
    fn write_only_does_not_retroactively_invalidate_and_exclusion_precedes_check() {
        let input = json!({});
        let workflow = Workflow::new("test", "challenge", vec![], vec![]);
        let candidate = CandidateRecord {
            id: "ordinary".into(),
            kind: "peer-test-kind".into(),
            data: json!({}),
        };
        let records = vec![ContextRecord::new(
            candidate.id.clone(),
            candidate.kind.clone(),
            candidate.data.clone(),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(0),
        )];
        let ctx = AdmissionContext::new(&input, &records, "challenge", 1, &workflow).unwrap();
        let rules = [Rule {
            kind: "peer-test-kind",
            id: "peer-test-write-only",
            applicability: Applicability::WriteOnly,
            check: challenge,
        }];
        assert_eq!(
            check_with_extensions(&ctx, &candidate, Mode::Write, &rules)
                .unwrap_err()
                .rule,
            "peer-test-write-only"
        );
        assert!(classify(&ctx, &BTreeSet::new(), &rules)[0].1.is_ok());
        let always = [Rule {
            kind: "peer-test-kind",
            id: "peer-test-always",
            applicability: Applicability::Always,
            check: challenge,
        }];
        assert!(classify(&ctx, &BTreeSet::new(), &always)[0].1.is_err());
        assert!(classify(&ctx, &BTreeSet::from(["ordinary".into()]), &always).is_empty());
    }
}
