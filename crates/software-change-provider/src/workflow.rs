//! Union phase table and live-graph stitcher for software-change.
//!
//! Describe and evaluate both read [`PHASES`] so live topology and evaluation
//! duties cannot drift. Bare describe emits the historical sixteen-state
//! union. A new contract-v3 input adds the provider-owned reconciliation hop;
//! when `review_policies` is present, only its live review states are emitted
//! and ready/approved/passed rewire onto the next live successor.

use crate::overlay;
use loop_core::{State, Transition, WorkSlot, Workflow};
use serde_json::Value;
use std::collections::BTreeSet;

/// Context kinds forwarded to review and implementation slots. The
/// implementation slot also receives immutable review-evidence sources so
/// provider-side stable references can be resolved without copying them into
/// finding-ledger records.
pub(crate) const FINDING_LEDGER_KIND: &str = "finding-ledger";
pub(crate) const REVIEW_EVIDENCE_KIND: &str = "review-evidence";

/// Public provider contract for the post-implementation reconciliation phase.
/// Keep these names stable: fresh contract-v3 graphs expose them and dependants
/// consume this source contract rather than reconstructing it from task output.
pub(crate) const RECONCILIATION_STATE: &str = "reconciliation";
pub(crate) const RECONCILIATION_DRAFT_SLOT: &str = "reconciliation-draft";
pub(crate) const RECONCILIATION_READY_EVENT: &str = "reconciliation-ready";
pub(crate) const RECONCILIATION_SUBJECT: &str = "reconciliation.json";
pub(crate) const RECONCILIATION_SCHEMA_PATH: &str =
    "crates/software-change-provider/data/reconciliation-schema.json";
pub(crate) const RECONCILIATION_RESULT_FIELDS: &[&str] = &[
    "revision",
    "author",
    "mode",
    "branch",
    "document_observations",
    "behavior_observations",
    "action",
    "action_reason",
    "authorization",
    "application",
    "commit",
    "traceability",
    "proof_references",
    "blockers",
    "decision",
];

const BOOKENDS_STATE_GUIDANCE: &str = "Bookends overlay: every current intent criterion has one `prd_traceability` disposition (`linked-live`, `candidate`, or `not-applicable`). Linked-live IDs must be live PRD IDs; candidates must be parser-valid proposed records. At every durable e2e/journey or declared contract test boundary, cite the applicable live PRD ID in the captured result for driver triage. Never mint an ID.";

fn bookends_citation_hint() -> String {
    ["bookends", ":LE-", "<n>"].concat()
}

/// Owning-phase check-free revise from a review state to an earlier draft.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OwningRevise {
    pub event: &'static str,
    pub target: &'static str,
}

/// One row of the provider-owned union phase table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Phase {
    pub name: &'static str,
    pub draft_state: &'static str,
    pub draft_title: &'static str,
    pub draft_instructions: &'static str,
    pub draft_slot: &'static str,
    pub ready_event: &'static str,
    pub draft_revises: &'static [OwningRevise],
    pub subject: &'static str,
    /// Parent review state id, gate id, and slot id (the three names match).
    pub parent_review: &'static str,
    pub parent_review_title: &'static str,
    pub parent_review_instructions: &'static str,
    /// Adversarial review state id, gate id, and slot id (the three names match).
    pub adversarial_review: &'static str,
    pub nearest_revise_target: &'static str,
    pub extra_owning_revises: &'static [OwningRevise],
    /// Next phase's draft state; `None` means `end` after validation.
    pub next_draft: Option<&'static str>,
}

const REVISE_INTENT: OwningRevise = OwningRevise {
    event: "revise-intent",
    target: "explore",
};
const REVISE_DESIGN: OwningRevise = OwningRevise {
    event: "revise-design",
    target: "design",
};
const REVISE_PLAN: OwningRevise = OwningRevise {
    event: "revise-plan",
    target: "plan",
};
const REVISE_IMPLEMENTATION: OwningRevise = OwningRevise {
    event: "revise-implementation",
    target: "implement",
};

/// The five-phase union table. Describe stitches from it; evaluate maps
/// source-state duties from it.
pub(crate) const PHASES: &[Phase] = &[
    Phase {
        name: "intent",
        draft_state: "explore",
        draft_title: "Explore",
        draft_instructions: "Author intent in `crates/software-change-provider/data/templates/intent.md`: state the problem, desired outcome, acceptance boundary, constraints, and non-goals, plus the required `operating_context` (`operators`, `environment`, `threat_boundary`, `accepted_risks`, and `outside_obligations`). Do not prescribe an implementation unless an external constraint requires it. Avoid target laundering: do not present a chosen solution as the problem. Before `intent-ready`, read run-frozen obligations via `show` and run configured deterministic checks before commissioning external review.",
        draft_slot: "intent-draft",
        ready_event: "intent-ready",
        draft_revises: &[],
        subject: "intent.json",
        parent_review: "intent-review",
        parent_review_title: "Intent review",
        parent_review_instructions: "For `intent.json`, run the configured deterministic check first, before commissioning external review. Read the frozen intent `operating_context` before judging; do not demand excluded hostile-user or multi-tenant hardening, and never waive a stated outcome or outside obligation. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for intent corrections; do not waive known defects.",
        adversarial_review: "intent-adversarial-review",
        nearest_revise_target: "explore",
        extra_owning_revises: &[],
        next_draft: Some("design"),
    },
    Phase {
        name: "design",
        draft_state: "design",
        draft_title: "Design",
        draft_instructions: "Describe structural shape, boundaries, invariants, and decisions in `crates/software-change-provider/data/templates/design.md`; design is not a work schedule. Preserve the frozen intent `operating_context`, outcomes, and outside obligations while leaving replaceable mechanisms free. The `design-ready` event structurally checks `design.json` when the run's configuration supplies a schema for it — read your obligations via `show`.",
        draft_slot: "design-draft",
        ready_event: "design-ready",
        draft_revises: &[],
        subject: "design.json",
        parent_review: "design-review",
        parent_review_title: "Design review",
        parent_review_instructions: "For `design.json`, run the configured deterministic check first, before commissioning external review. Read the frozen intent `operating_context` before judging; do not demand excluded hostile-user or multi-tenant hardening, and never waive a stated outcome or outside obligation. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for design corrections or `revise-intent` for intent-owned defects; do not waive known defects.",
        adversarial_review: "design-adversarial-review",
        nearest_revise_target: "design",
        extra_owning_revises: &[REVISE_INTENT],
        next_draft: Some("plan"),
    },
    Phase {
        name: "plan",
        draft_state: "plan",
        draft_title: "Plan",
        draft_instructions: "Build the dependency graph in `crates/software-change-provider/data/templates/task-packet.md`: inspect the frozen intent `operating_context` and include per-task objective, affected user or operator paths, observable completion outcome, dependencies, source-of-truth references, deliverables, out-of-scope work, validation, and handoff contract. Require realistic black-box proof when practical or a concrete impracticality reason and substitute; leave replaceable mechanisms free inside frozen decisions. Put contract gates before parallel fan-out; plan shape, not implementation prose, is the target. Before `plan-ready`, read run-frozen obligations via `show`.",
        draft_slot: "plan-draft",
        ready_event: "plan-ready",
        draft_revises: &[],
        subject: "plan.json",
        parent_review: "plan-review",
        parent_review_title: "Plan review",
        parent_review_instructions: "For `plan.json`, run the configured deterministic check first, before commissioning external review. Read the frozen intent `operating_context` before judging; do not demand excluded hostile-user or multi-tenant hardening, and never waive a stated outcome or outside obligation. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for plan corrections, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects; do not waive known defects.",
        adversarial_review: "plan-adversarial-review",
        nearest_revise_target: "plan",
        extra_owning_revises: &[REVISE_DESIGN, REVISE_INTENT],
        next_draft: Some("implement"),
    },
    Phase {
        name: "implementation",
        draft_state: "implement",
        draft_title: "Implement",
        draft_instructions: "Perform external work against the accepted plan and frozen intent `operating_context`; do not add excluded hostile or multi-tenant requirements or waive stated outcomes and outside obligations. Document the implementation and validation report shapes using `crates/software-change-provider/data/templates/implementation-report.md` and `crates/software-change-provider/data/templates/validation-report.md`. Doc integration is part of this change: update authoritative repository documents rather than leaving a parallel change truth. Before `implementation-ready`, read run-frozen obligations via `show`. If the owner rejects earlier work, choose `revise-plan` to plan, `revise-design` to design, or `revise-intent` to explore directly; no implementation report is required for these check-free routes. First wait for owned work to finish or use `cancel-invocation` and verify cleanup, including elapsed-but-live work and pending cancellation. Routes preserve artifacts, captures, invocation and denial history; neither engine nor provider classifies the defect or rolls back the repository. Only routes in this run's stored graph are available; upgrading does not add them to historical runs.",
        draft_slot: "implement",
        ready_event: "implementation-ready",
        draft_revises: &[REVISE_PLAN, REVISE_DESIGN, REVISE_INTENT],
        subject: "implementation-report.json",
        parent_review: "implementation-review",
        parent_review_title: "Implementation review",
        parent_review_instructions: "For `implementation-report.json`, run the configured deterministic check first, before commissioning external review. Read the frozen intent `operating_context` before judging; do not demand excluded hostile-user or multi-tenant hardening, and never waive a stated outcome or outside obligation. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for implementation corrections, `revise-plan` for plan-owned defects, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects; do not waive known defects. Report coverage must identify repository state and covered document revisions.",
        adversarial_review: "implementation-adversarial-review",
        nearest_revise_target: "implement",
        extra_owning_revises: &[REVISE_PLAN, REVISE_DESIGN, REVISE_INTENT],
        next_draft: Some("validation"),
    },
    Phase {
        name: "validation",
        draft_state: "validation",
        draft_title: "Validation",
        draft_instructions: "Author the validation report in `crates/software-change-provider/data/templates/validation-report.md`. Inspect the frozen intent `operating_context`, prove observable outcomes and outside obligations rather than activity, and semantically inspect every new or changed Bookends citation. Verify intent delivery and documentation integration. Before the checked hop out of this room (`validation-ready` or `passed`), read run-frozen obligations via `show` and run configured deterministic checks. Validation-report-local defects stay in this draft: edit and recheck `validation-report.json`, then retry the next checked hop.",
        draft_slot: "validation-draft",
        ready_event: "validation-ready",
        draft_revises: &[REVISE_IMPLEMENTATION],
        subject: "validation-report.json",
        parent_review: "validation-review",
        parent_review_title: "Validation review",
        parent_review_instructions: "For `validation-report.json`, run the configured deterministic check first, before commissioning external review. Read the frozen intent `operating_context` before judging; reject activity-only evidence and token-only Bookends citations, do not demand excluded hostile-user or multi-tenant hardening, and never waive a stated outcome or outside obligation. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Validation-report-local defects use nearest check-free `revise` back to the validation draft, then retry the next checked hop. Select the owning phase for accepted defects: use `revise-implementation` for implementation-owned defects, `revise-plan` for plan-owned defects, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects. Do not waive known defects. Use the validation report template as the artifact shape.",
        adversarial_review: "validation-adversarial-review",
        nearest_revise_target: "validation",
        extra_owning_revises: &[REVISE_IMPLEMENTATION, REVISE_PLAN, REVISE_DESIGN, REVISE_INTENT],
        next_draft: None,
    },
];

fn reconciliation_instructions(bookends_enabled: bool) -> String {
    let mode = if bookends_enabled {
        "With Bookends enabled, reread the actual accepted PRD wording and every authoritative document it names; preserve live traceability and never treat a related ID as semantic coverage."
    } else {
        "With Bookends disabled, inspect only the relevant authoritative repository documents against the approved intent and delivered behavior; do not add PRD IDs, Bookends citations, candidate machinery, or overlay obligations."
    };
    format!(
        "Reconcile the frozen intent `operating_context`, approved intent, delivered behavior, and current authoritative repository documents. Author exactly `{}` using `{}`. Set `{}`. {mode} Distinguish sufficient existing wording (including an implementation defect corrected under sufficient wording), change-specific proof, and missing or changed enduring meaning. A justified no-document-change action is valid. A requirements amendment needs exact owner acceptance and separately authorized application and commit; a wrong implementation is corrected as code. A blocked decision must retain its concrete blockers. This state does not approve, progress, commit, or write `implementation-report.json`, `validation-report.json`, or checkpoint files. The graph summarizer remains the sole implementation-report writer. Report finalization, repository checkpoint, implementation review, validation, and final proof consume the post-reconciliation tree downstream.",
        RECONCILIATION_SUBJECT,
        RECONCILIATION_SCHEMA_PATH,
        RECONCILIATION_RESULT_FIELDS.join("`, `"),
    )
}

const END_INSTRUCTIONS: &str = "The software change is complete. Preserve the final artifacts, evidence, coverage manifest, and authoritative document integration described by the shipped templates.";

/// Duties evaluate applies for a source state plus event, taken from [`PHASES`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionDuties {
    /// Draft ready/passed: schema, revision links, and phase evidence.
    Checked {
        subject: &'static str,
        gate: Option<&'static str>,
    },
    /// A checked handoff whose destination owns the next artifact. It has no
    /// report, checkpoint, or other artifact prerequisite of its own.
    CheckedNoArtifact,
    /// Check-free revise events are zero-obligation.
    CheckFree,
}

/// Look up evaluation duties for a source state and event from the phase table.
pub(crate) fn duties_for(source: &str, event: &str) -> Option<TransitionDuties> {
    if source == RECONCILIATION_STATE && event == RECONCILIATION_READY_EVENT {
        return Some(TransitionDuties::Checked {
            subject: RECONCILIATION_SUBJECT,
            gate: None,
        });
    }

    for phase in PHASES {
        if source == phase.draft_state {
            if event == phase.ready_event || (phase.next_draft.is_none() && event == "passed") {
                return Some(TransitionDuties::Checked {
                    subject: phase.subject,
                    gate: None,
                });
            }
            if phase
                .draft_revises
                .iter()
                .any(|revise| revise.event == event)
            {
                return Some(TransitionDuties::CheckFree);
            }
            return None;
        }
        if source == phase.parent_review {
            return review_duties(phase, phase.parent_review, event);
        }
        if source == phase.adversarial_review {
            return review_duties(phase, phase.adversarial_review, event);
        }
    }
    None
}

/// Look up duties for an exact snapshotted edge. The target matters for the
/// contract-v3 implementation handoff: entering reconciliation is checked by
/// the engine but has no report or checkpoint prerequisite. Historical edges
/// still use the original phase-table duties.
pub(crate) fn duties_for_transition(
    source: &str,
    event: &str,
    target: &str,
) -> Option<TransitionDuties> {
    if source == "implement" && event == "implementation-ready" && target == RECONCILIATION_STATE {
        return Some(TransitionDuties::CheckedNoArtifact);
    }
    duties_for(source, event)
}

fn review_duties(phase: &Phase, gate: &'static str, event: &str) -> Option<TransitionDuties> {
    if event == "approved" || (phase.next_draft.is_none() && event == "passed") {
        return Some(TransitionDuties::Checked {
            subject: phase.subject,
            gate: Some(gate),
        });
    }
    if event == "revise"
        || phase
            .extra_owning_revises
            .iter()
            .any(|revise| revise.event == event)
    {
        return Some(TransitionDuties::CheckFree);
    }
    None
}

/// Gate identifiers derived from the phase table, in table order.
#[cfg(test)]
pub(crate) fn phase_table_gate_ids() -> Vec<&'static str> {
    PHASES
        .iter()
        .flat_map(|phase| [phase.parent_review, phase.adversarial_review])
        .collect()
}

/// Union catalog used when `review_policies` is omitted.
#[cfg(test)]
pub(crate) fn software_change_workflow() -> Workflow {
    describe_workflow(None).expect("omitted review_policies yields the union graph")
}

/// Stitch the live workflow implied by optional describe `initial_input`.
///
/// Omitted `initial_input`, a non-object, or an object without
/// `review_policies` yields the sixteen-state union. A present
/// `review_policies` object keeps only live review states. A new contract-v3
/// input also gets the provider-owned reconciliation hop; the workflow is
/// snapshotted by engine start, so stored older graphs remain unchanged.
pub(crate) fn describe_workflow(initial_input: Option<&Value>) -> Result<Workflow, String> {
    let review_policies = initial_input
        .and_then(Value::as_object)
        .and_then(|object| object.get("review_policies"));
    let reconciliation_enabled = initial_input
        .filter(|input| input.get("contract_version").and_then(Value::as_u64) == Some(3))
        .is_some_and(|input| {
            // Version-10 contract-v3 inputs are historical snapshots. New
            // profile revisions gain the provider-owned phase; custom/test
            // contract-v3 inputs remain enabled unless they explicitly carry
            // the frozen -10 suffix.
            !input
                .get("config_version")
                .and_then(Value::as_str)
                .is_some_and(|version| version.ends_with("-10"))
        });
    let bookends_enabled = initial_input.is_some_and(overlay::enabled);
    let semantic_coverage = initial_input.is_some_and(overlay::semantic_coverage_enabled);
    let mut workflow = stitch(
        review_policies,
        bookends_enabled,
        reconciliation_enabled,
        semantic_coverage,
    )?;
    if initial_input.is_some_and(|v| matches!(v["contract_version"].as_u64(), Some(2 | 3))) {
        for slot in &mut workflow.work_slots {
            if slot.id.as_str().starts_with("validation") || slot.id.as_str() == "implement" {
                slot.stdin_context_kinds.extend(
                    [
                        "command-evidence",
                        "validation-command",
                        "criterion-verdict",
                        "goal-verdict",
                        "criterion-revalidation",
                    ]
                    .map(str::to_owned),
                );
            }
        }
    }
    for state in &mut workflow.states {
        if state.is_final {
            continue;
        }
        let policies = review_policies.and_then(|p| p.get(state.id.as_str()));
        let axes = policies.and_then(Value::as_array).map(|rows| {
            rows.iter()
                .map(|row| {
                    let mut axis = serde_json::json!({
                        "id": row["id"],
                        "required_authors": row
                            .get("required_authors")
                            .cloned()
                            .unwrap_or(Value::from(1))
                    });
                    if initial_input
                        .and_then(|input| input.get("contract_version"))
                        .and_then(Value::as_u64)
                        == Some(3)
                    {
                        axis["review_stage"] = row
                            .get("review_stage")
                            .or_else(|| row.get("stage"))
                            .cloned()
                            .unwrap_or_else(|| Value::String("aggregate".to_owned()));
                    }
                    axis
                })
                .collect::<Vec<_>>()
        });
        state.action_guidance = Some(serde_json::json!({
            "review_axes": axes,
            "policy_status": if review_policies.is_some() { "frozen" } else { "unknown: inspect full initial_input" },
            "criterion_policy": initial_input.and_then(|v| v.get("criterion_policy")),
            "repair": "Keep validation-report-only corrections in validation. For an implementation defect owned by a frozen task, select that task and its dependants. Use repair_finding_ids only for an accepted unresolved implementation finding honestly owned by no task. Revise plan/design/intent only when that phase obligation is materially wrong; reconfirm affected downstream proof and explicitly carry unaffected original evidence.",
            "review": "First review is comprehensive. Confirm accepted fixes and fix-introduced holes, with explicit unaffected applicability; no rerun-until-pass. Falsify circular proof and evidence compatible with opposite outcomes against frozen obligations, without adding axes.",
            "execution": "Bound: invoke the shown frozen slot; do not perform its worker body. Unbound: follow the provider skill externally. Triage captures before append or progression. Budget serial tasks, summarizer, proof and review together; overrun is attention, not retry permission.",
            "git": "After implementation triage and before independent review, ask the owner for the Git decision. Inspect staged names and diff; perform or confirm only the owner-authorized human/driver commit and verify its resulting identity with `git rev-parse HEAD`, or record pending/declined without Git changes or claiming a created commit. Workers do not independently commit. A commit changes proof identity: refresh invalidated receipts/report/checkpoints before review; then keep source/Git identity stable. This reminder grants no authorization."
        }));
    }
    Ok(workflow)
}

fn stitch(
    review_policies: Option<&Value>,
    bookends_enabled: bool,
    reconciliation_enabled: bool,
    semantic_coverage: bool,
) -> Result<Workflow, String> {
    if let Some(value) = review_policies {
        if !value.is_object() {
            return Err("`review_policies` must be an object".to_owned());
        }
    }

    let mut hops = Vec::new();
    for phase in PHASES {
        let (parent_live, adversarial_live) = live_reviews(phase, review_policies)?;
        hops.push(Hop::Draft(phase));
        if reconciliation_enabled && phase.name == "implementation" {
            hops.push(Hop::Reconciliation);
        }
        if parent_live {
            hops.push(Hop::Parent(phase));
        }
        if adversarial_live {
            hops.push(Hop::Adversarial(phase));
        }
    }

    let last_index = hops
        .len()
        .checked_sub(1)
        .expect("the phase table always emits at least the five draft hops");

    let mut states = Vec::new();
    let mut transitions = Vec::new();
    let mut work_slots = Vec::new();
    for (index, hop) in hops.iter().copied().enumerate() {
        states.push(hop.state(bookends_enabled, semantic_coverage));
        let event = hop_event(hop, index == last_index);
        let target = if index == last_index {
            "end"
        } else {
            hops[index + 1].state_id()
        };
        // The implementation handoff is an engine-checked boundary with no
        // report/checkpoint obligation. Reconciliation owns its own result;
        // the following checked `reconciliation-ready` edge validates it.
        transitions.push(Transition::checked(hop.state_id(), event, target));
        if let Hop::Draft(phase) = hop {
            for revise in phase.draft_revises {
                transitions.push(Transition::check_free(
                    hop.state_id(),
                    revise.event,
                    revise.target,
                ));
            }
        }
        if let Hop::Parent(phase) | Hop::Adversarial(phase) = hop {
            transitions.push(Transition::check_free(
                hop.state_id(),
                "revise",
                phase.nearest_revise_target,
            ));
            for revise in phase.extra_owning_revises {
                transitions.push(Transition::check_free(
                    hop.state_id(),
                    revise.event,
                    revise.target,
                ));
            }
        }
        work_slots.push(hop.slot(event));
    }
    states.push(State::new("end", "End", END_INSTRUCTIONS, true));

    Ok(
        Workflow::new("software-change", "explore", states, transitions)
            .with_work_slots(work_slots),
    )
}

fn live_reviews(phase: &Phase, review_policies: Option<&Value>) -> Result<(bool, bool), String> {
    let Some(policies) = review_policies else {
        return Ok((true, true));
    };
    let parent_live = nonempty_policy_list(policies.get(phase.parent_review));
    let adversarial_live = nonempty_policy_list(policies.get(phase.adversarial_review));
    if adversarial_live && !parent_live {
        return Err(format!(
            "adversarial review `{}` for the {} phase is nonempty while parent review `{}` is empty or absent",
            phase.adversarial_review, phase.name, phase.parent_review
        ));
    }
    if adversarial_live {
        let parent_ids = axis_ids(
            policies
                .get(phase.parent_review)
                .expect("a live parent list is present"),
            phase.parent_review,
        )?;
        let adversarial_ids = axis_ids(
            policies
                .get(phase.adversarial_review)
                .expect("a live adversarial list is present"),
            phase.adversarial_review,
        )?;
        for id in &adversarial_ids {
            if !parent_ids.contains(id) {
                return Err(format!(
                    "adversarial axis `{id}` on `{}` is not on the {} phase parent list `{}`",
                    phase.adversarial_review, phase.name, phase.parent_review
                ));
            }
        }
    }
    Ok((parent_live, adversarial_live))
}

fn nonempty_policy_list(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|entries| !entries.is_empty())
}

fn axis_ids(list: &Value, gate: &str) -> Result<BTreeSet<String>, String> {
    let Some(entries) = list.as_array() else {
        return Err(format!("`{gate}` policy list must be an array"));
    };
    let mut ids = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        match entry.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => {
                ids.insert(id.to_owned());
            }
            _ => {
                return Err(format!(
                    "`{gate}` policy entry {index} must have a nonempty string `id`"
                ));
            }
        }
    }
    Ok(ids)
}

#[derive(Clone, Copy)]
enum Hop {
    Draft(&'static Phase),
    Reconciliation,
    Parent(&'static Phase),
    Adversarial(&'static Phase),
}

impl Hop {
    fn state_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_state,
            Self::Reconciliation => RECONCILIATION_STATE,
            Self::Parent(phase) => phase.parent_review,
            Self::Adversarial(phase) => phase.adversarial_review,
        }
    }

    fn state(self, bookends_enabled: bool, semantic_coverage: bool) -> State {
        match self {
            Self::Draft(phase) => State::new(
                phase.draft_state,
                phase.draft_title,
                with_bookends_guidance(
                    phase.draft_instructions,
                    bookends_enabled,
                    semantic_coverage,
                ),
                false,
            ),
            Self::Reconciliation => State::new(
                RECONCILIATION_STATE,
                "Reconciliation",
                with_bookends_guidance(
                    &reconciliation_instructions(bookends_enabled),
                    bookends_enabled,
                    semantic_coverage,
                ),
                false,
            ),
            Self::Parent(phase) => State::new(
                phase.parent_review,
                phase.parent_review_title,
                with_bookends_guidance(
                    phase.parent_review_instructions,
                    bookends_enabled,
                    semantic_coverage,
                ),
                false,
            ),
            Self::Adversarial(phase) => {
                let title = adversarial_title(phase.parent_review_title);
                let instructions =
                    adversarial_instructions(phase, bookends_enabled, semantic_coverage);
                State::new(phase.adversarial_review, title, instructions, false)
            }
        }
    }

    fn slot(self, event: &str) -> WorkSlot {
        let slot = WorkSlot::new(self.slot_id(), self.state_id(), event);
        // Preserve source records needed by provider projections; steering is
        // eligible for every later draft/review, not only implementation.
        slot.with_stdin_context_kinds(vec![
            FINDING_LEDGER_KIND.to_owned(),
            REVIEW_EVIDENCE_KIND.to_owned(),
            loop_core::EVIDENCE_APPLICABILITY_KIND.to_owned(),
            "user-steering".to_owned(),
            "steering-incorporation".to_owned(),
        ])
    }

    fn slot_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_slot,
            Self::Reconciliation => RECONCILIATION_DRAFT_SLOT,
            Self::Parent(phase) => phase.parent_review,
            Self::Adversarial(phase) => phase.adversarial_review,
        }
    }
}

fn hop_event(hop: Hop, last: bool) -> &'static str {
    if last {
        return "passed";
    }
    match hop {
        Hop::Draft(phase) => phase.ready_event,
        Hop::Reconciliation => RECONCILIATION_READY_EVENT,
        Hop::Parent(_) | Hop::Adversarial(_) => "approved",
    }
}

fn adversarial_title(parent_title: &str) -> String {
    match parent_title.strip_suffix(" review") {
        Some(stem) => format!("{stem} challenge review"),
        None => format!("{parent_title} challenge review"),
    }
}

fn adversarial_instructions(
    phase: &Phase,
    bookends_enabled: bool,
    semantic_coverage: bool,
) -> String {
    with_bookends_guidance(
        &format!(
            "This challenge review follows parent `{}` and must meaningfully falsify that parent's pass claim only with current supplied evidence, a violated frozen obligation, a concrete consequence for change success, and why existing validation does not resolve the issue. Reject hypothetical threats, invented requirements, silence or style complaints, and mechanism-for-its-own-sake findings; do not waive material failures. {}",
            phase.parent_review, phase.parent_review_instructions
        ),
        bookends_enabled,
        semantic_coverage,
    )
}

const BOOKENDS_SEMANTIC_GUIDANCE: &str = "For a new semantic-coverage profile, `ids-grounded` is not an ID/topic/token check: read each cited requirement's actual normative wording and every authoritative document it explicitly names. Classify sufficient existing wording, change-specific proof, missing or changed enduring meaning, or an implementation defect under sufficient wording. Keep proposals provisional and preserve owner acceptance, application, and commit as explicit pending statuses; a parser-valid candidate is not live. Bookends-disabled runs do not acquire PRD IDs or overlay obligations.";

fn with_bookends_guidance(instructions: &str, enabled: bool, semantic_coverage: bool) -> String {
    if enabled {
        let semantic = if semantic_coverage {
            format!(" {BOOKENDS_SEMANTIC_GUIDANCE}")
        } else {
            String::new()
        };
        format!(
            "{instructions} {BOOKENDS_STATE_GUIDANCE}{semantic} Citation spelling: `{}`.",
            bookends_citation_hint()
        )
    } else {
        instructions.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::TransitionKind;
    use serde_json::json;

    fn union() -> Workflow {
        describe_workflow(None).expect("union")
    }

    fn stitched(policies: Value) -> Workflow {
        describe_workflow(Some(&json!({ "review_policies": policies }))).expect("stitch")
    }

    fn stitch_err(policies: Value) -> String {
        describe_workflow(Some(&json!({ "review_policies": policies })))
            .expect_err("expected fail closed")
    }

    fn axis(id: &str) -> Value {
        json!({ "id": id, "description": "test axis" })
    }

    fn state_ids(workflow: &Workflow) -> Vec<&str> {
        workflow
            .states
            .iter()
            .map(|state| state.id.as_str())
            .collect()
    }

    fn slot_ids(workflow: &Workflow) -> Vec<&str> {
        workflow
            .work_slots
            .iter()
            .map(|slot| slot.id.as_str())
            .collect()
    }

    fn checked_hop<'a>(workflow: &'a Workflow, source: &str) -> (&'a str, &'a str) {
        let hops: Vec<_> = workflow
            .transitions
            .iter()
            .filter(|transition| {
                transition.source.as_str() == source && transition.kind == TransitionKind::Checked
            })
            .collect();
        assert_eq!(hops.len(), 1, "expected one checked hop from {source}");
        (hops[0].event.as_str(), hops[0].target.as_str())
    }

    fn check_free_target<'a>(workflow: &'a Workflow, source: &str, event: &str) -> &'a str {
        workflow
            .transitions
            .iter()
            .find(|transition| {
                transition.source.as_str() == source
                    && transition.event.as_str() == event
                    && transition.kind == TransitionKind::CheckFree
            })
            .unwrap_or_else(|| panic!("missing check-free `{event}` from {source}"))
            .target
            .as_str()
    }

    fn has_state(workflow: &Workflow, id: &str) -> bool {
        workflow.states.iter().any(|state| state.id.as_str() == id)
    }

    fn slot<'a>(workflow: &'a Workflow, id: &str) -> &'a WorkSlot {
        workflow
            .work_slots
            .iter()
            .find(|slot| slot.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing slot {id}"))
    }

    fn assert_nearest_revises(workflow: &Workflow) {
        for phase in PHASES {
            for review in [phase.parent_review, phase.adversarial_review] {
                if !has_state(workflow, review) {
                    continue;
                }
                assert_eq!(
                    check_free_target(workflow, review, "revise"),
                    phase.nearest_revise_target,
                    "{review} nearest revise must return to {} draft {}",
                    phase.name,
                    phase.draft_state
                );
            }
        }
    }

    fn assert_owning_phase_revises(workflow: &Workflow) {
        if has_state(workflow, "design-review") {
            assert_eq!(
                check_free_target(workflow, "design-review", "revise-intent"),
                "explore"
            );
        }
        if has_state(workflow, "plan-review") {
            assert_eq!(
                check_free_target(workflow, "plan-review", "revise-design"),
                "design"
            );
            assert_eq!(
                check_free_target(workflow, "plan-review", "revise-intent"),
                "explore"
            );
        }
        if has_state(workflow, "implementation-review") {
            assert_eq!(
                check_free_target(workflow, "implementation-review", "revise-plan"),
                "plan"
            );
            assert_eq!(
                check_free_target(workflow, "implementation-review", "revise-design"),
                "design"
            );
            assert_eq!(
                check_free_target(workflow, "implementation-review", "revise-intent"),
                "explore"
            );
        }
    }

    fn assert_review_slots_declare_finding_ledger(workflow: &Workflow) {
        let encoded = serde_json::to_value(workflow).expect("workflow JSON");
        for slot in encoded["work_slots"].as_array().expect("work_slots") {
            let id = slot["id"].as_str().expect("slot id");
            let expected = json!([
                FINDING_LEDGER_KIND,
                REVIEW_EVIDENCE_KIND,
                loop_core::EVIDENCE_APPLICABILITY_KIND,
                "user-steering",
                "steering-incorporation"
            ]);
            assert_eq!(
                slot.get("stdin_context_kinds"),
                Some(&expected),
                "slot {id} must support steering and stable source projection"
            );
        }
    }

    #[test]
    fn recovery_backtracking_only_implement_gains_three_check_free_owning_routes() {
        for workflow in [
            union(),
            describe_workflow(Some(&json!({"review_policies": {}}))).unwrap(),
        ] {
            let routes: Vec<_> = workflow
                .transitions
                .iter()
                .filter(|edge| {
                    edge.source.as_str() == "implement" && edge.kind == TransitionKind::CheckFree
                })
                .map(|edge| (edge.event.as_str(), edge.target.as_str()))
                .collect();
            assert_eq!(
                routes,
                [
                    ("revise-plan", "plan"),
                    ("revise-design", "design"),
                    ("revise-intent", "explore")
                ]
            );
            for (event, _) in routes {
                assert_eq!(
                    duties_for("implement", event),
                    Some(TransitionDuties::CheckFree)
                );
            }
            for state in ["explore", "design", "plan"] {
                assert!(!workflow
                    .transitions
                    .iter()
                    .any(|edge| edge.source.as_str() == state
                        && edge.kind == TransitionKind::CheckFree));
            }
            assert_eq!(
                check_free_target(&workflow, "validation", "revise-implementation"),
                "implement"
            );
            let instructions = &workflow
                .states
                .iter()
                .find(|state| state.id.as_str() == "implement")
                .unwrap()
                .instructions;
            for phrase in [
                "no implementation report",
                "cancel-invocation",
                "elapsed-but-live",
                "stored graph",
                "rolls back",
            ] {
                assert!(instructions.contains(phrase), "missing {phrase}");
            }
        }
    }

    #[test]
    fn overlay_guidance_reaches_driver_and_bound_state_instructions() {
        let workflow = describe_workflow(Some(&json!({
            "extra": {"bookends": {"enabled": true}},
            "review_policies": {}
        })))
        .expect("overlay workflow");
        for state_id in ["explore", "design", "validation"] {
            let state = workflow
                .states
                .iter()
                .find(|state| state.id.as_str() == state_id)
                .expect(state_id);
            assert!(state.instructions.contains("durable e2e/journey"));
            assert!(state
                .instructions
                .contains(&["bookends", ":LE-", "<n>"].concat()));
        }
    }

    #[test]
    fn new_bookends_profiles_receive_semantic_handoff_guidance_without_new_axis() {
        let workflow = describe_workflow(Some(&json!({
            "contract_version": 3,
            "config_version": "standard-11",
            "extra": {"bookends": {"enabled": true}},
            "review_policies": {
                "intent-review": [{"id": "ordinary"}],
                "intent-adversarial-review": [{"id": "ordinary"}]
            }
        })))
        .expect("new semantic profile");
        for state_id in ["explore", "intent-review", "intent-adversarial-review"] {
            let state = workflow
                .states
                .iter()
                .find(|state| state.id.as_str() == state_id)
                .expect(state_id);
            assert!(state.instructions.contains("actual normative wording"));
            assert!(state.instructions.contains("explicitly names"));
            assert!(state.instructions.contains("implementation defect"));
        }
        let old = describe_workflow(Some(&json!({
            "contract_version": 3,
            "config_version": "standard-10",
            "extra": {"bookends": {"enabled": true}},
            "review_policies": {
                "intent-review": [{"id": "ordinary"}],
                "intent-adversarial-review": [{"id": "ordinary"}]
            }
        })))
        .expect("frozen semantic profile");
        let old_state = old
            .states
            .iter()
            .find(|state| state.id.as_str() == "intent-review")
            .expect("old intent review");
        assert!(!old_state.instructions.contains("actual normative wording"));
    }

    #[test]
    fn omitted_review_policies_yields_sixteen_state_union_catalog() {
        let workflow = union();
        assert_eq!(
            state_ids(&workflow),
            vec![
                "explore",
                "intent-review",
                "intent-adversarial-review",
                "design",
                "design-review",
                "design-adversarial-review",
                "plan",
                "plan-review",
                "plan-adversarial-review",
                "implement",
                "implementation-review",
                "implementation-adversarial-review",
                "validation",
                "validation-review",
                "validation-adversarial-review",
                "end",
            ]
        );
        assert_eq!(
            slot_ids(&workflow),
            vec![
                "intent-draft",
                "intent-review",
                "intent-adversarial-review",
                "design-draft",
                "design-review",
                "design-adversarial-review",
                "plan-draft",
                "plan-review",
                "plan-adversarial-review",
                "implement",
                "implementation-review",
                "implementation-adversarial-review",
                "validation-draft",
                "validation-review",
                "validation-adversarial-review",
            ]
        );
        assert_eq!(slot(&workflow, "intent-draft").state.as_str(), "explore");
        assert_eq!(
            slot(&workflow, "intent-draft").event.as_str(),
            "intent-ready"
        );
        assert_eq!(
            slot(&workflow, "validation-draft").state.as_str(),
            "validation"
        );
        assert_eq!(
            slot(&workflow, "validation-draft").event.as_str(),
            "validation-ready"
        );
        assert_eq!(
            describe_workflow(Some(&json!({"objective": "test"}))).expect("no policies key"),
            workflow
        );
        assert_eq!(software_change_workflow(), workflow);
    }

    #[test]
    fn nonempty_lists_keep_states_empty_or_absent_lists_omit_and_rewire() {
        let only_design = stitched(json!({
            "design-review": [axis("shape")]
        }));
        assert_eq!(
            state_ids(&only_design),
            vec![
                "explore",
                "design",
                "design-review",
                "plan",
                "implement",
                "validation",
                "end",
            ]
        );
        assert_eq!(
            checked_hop(&only_design, "explore"),
            ("intent-ready", "design")
        );
        assert_eq!(
            checked_hop(&only_design, "design"),
            ("design-ready", "design-review")
        );
        assert_eq!(
            checked_hop(&only_design, "design-review"),
            ("approved", "plan")
        );
        assert_eq!(
            checked_hop(&only_design, "plan"),
            ("plan-ready", "implement")
        );
        assert_eq!(
            checked_hop(&only_design, "implement"),
            ("implementation-ready", "validation")
        );
        assert_eq!(checked_hop(&only_design, "validation"), ("passed", "end"));
        assert!(!has_state(&only_design, "intent-review"));
        assert!(!has_state(&only_design, "design-adversarial-review"));
        assert!(!has_state(&only_design, "plan-review"));

        let empty_parent = stitched(json!({
            "design-review": [],
            "plan-review": [axis("deps")]
        }));
        assert!(!has_state(&empty_parent, "design-review"));
        assert_eq!(
            checked_hop(&empty_parent, "design"),
            ("design-ready", "plan")
        );
        assert_eq!(
            checked_hop(&empty_parent, "plan"),
            ("plan-ready", "plan-review")
        );
        assert_eq!(
            checked_hop(&empty_parent, "plan-review"),
            ("approved", "implement")
        );

        let review_less = stitched(json!({}));
        assert_eq!(
            state_ids(&review_less),
            vec![
                "explore",
                "design",
                "plan",
                "implement",
                "validation",
                "end",
            ]
        );
        assert_eq!(
            checked_hop(&review_less, "explore"),
            ("intent-ready", "design")
        );
        assert_eq!(
            checked_hop(&review_less, "design"),
            ("design-ready", "plan")
        );
        assert_eq!(
            checked_hop(&review_less, "plan"),
            ("plan-ready", "implement")
        );
        assert_eq!(
            checked_hop(&review_less, "implement"),
            ("implementation-ready", "validation")
        );
        assert_eq!(checked_hop(&review_less, "validation"), ("passed", "end"));
    }

    #[test]
    fn last_hop_passed_assignment() {
        let validation_review_only = stitched(json!({
            "validation-review": [axis("delivery")]
        }));
        assert_eq!(
            checked_hop(&validation_review_only, "validation"),
            ("validation-ready", "validation-review")
        );
        assert_eq!(
            checked_hop(&validation_review_only, "validation-review"),
            ("passed", "end")
        );
        assert_eq!(
            slot(&validation_review_only, "validation-draft")
                .event
                .as_str(),
            "validation-ready"
        );
        assert_eq!(
            slot(&validation_review_only, "validation-review")
                .event
                .as_str(),
            "passed"
        );

        let adversarial_last = stitched(json!({
            "validation-review": [axis("delivery")],
            "validation-adversarial-review": [axis("delivery")]
        }));
        assert_eq!(
            checked_hop(&adversarial_last, "validation-review"),
            ("approved", "validation-adversarial-review")
        );
        assert_eq!(
            checked_hop(&adversarial_last, "validation-adversarial-review"),
            ("passed", "end")
        );
        assert_eq!(
            slot(&adversarial_last, "validation-review").event.as_str(),
            "approved"
        );
        assert_eq!(
            slot(&adversarial_last, "validation-adversarial-review")
                .event
                .as_str(),
            "passed"
        );

        let review_less = stitched(json!({}));
        assert_eq!(checked_hop(&review_less, "validation"), ("passed", "end"));
        assert_eq!(
            slot(&review_less, "validation-draft").event.as_str(),
            "passed"
        );
        assert!(!has_state(&review_less, "validation-review"));
        assert!(!has_state(&review_less, "validation-adversarial-review"));
    }

    #[test]
    fn orphan_adversarial_list_and_unknown_counterpart_id_fail_closed() {
        let orphan_absent = stitch_err(json!({
            "intent-adversarial-review": [axis("axis")]
        }));
        assert!(
            orphan_absent.contains("intent-adversarial-review"),
            "{orphan_absent}"
        );
        assert!(orphan_absent.contains("empty or absent"), "{orphan_absent}");

        let orphan_empty = stitch_err(json!({
            "intent-review": [],
            "intent-adversarial-review": [axis("axis")]
        }));
        assert!(orphan_empty.contains("empty or absent"), "{orphan_empty}");

        let unknown = stitch_err(json!({
            "intent-review": [axis("parent-axis")],
            "intent-adversarial-review": [axis("other-axis")]
        }));
        assert!(unknown.contains("other-axis"), "{unknown}");
        assert!(unknown.contains("not on"), "{unknown}");

        let subset = stitched(json!({
            "intent-review": [axis("a"), axis("b")],
            "intent-adversarial-review": [axis("a")]
        }));
        assert!(has_state(&subset, "intent-review"));
        assert!(has_state(&subset, "intent-adversarial-review"));
    }

    #[test]
    fn nearest_revise_from_every_live_review_returns_to_that_phase_draft() {
        let workflow = union();
        assert_nearest_revises(&workflow);
        assert_eq!(
            check_free_target(&workflow, "intent-review", "revise"),
            "explore"
        );
        assert_eq!(
            check_free_target(&workflow, "intent-adversarial-review", "revise"),
            "explore"
        );
        assert_eq!(
            check_free_target(&workflow, "design-review", "revise"),
            "design"
        );
        assert_eq!(
            check_free_target(&workflow, "design-adversarial-review", "revise"),
            "design"
        );
        assert_eq!(
            check_free_target(&workflow, "plan-review", "revise"),
            "plan"
        );
        assert_eq!(
            check_free_target(&workflow, "plan-adversarial-review", "revise"),
            "plan"
        );
        assert_eq!(
            check_free_target(&workflow, "implementation-review", "revise"),
            "implement"
        );
        assert_eq!(
            check_free_target(&workflow, "implementation-adversarial-review", "revise"),
            "implement"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise"),
            "validation"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-adversarial-review", "revise"),
            "validation"
        );
        assert_owning_phase_revises(&workflow);
        assert_eq!(
            check_free_target(&workflow, "design-adversarial-review", "revise-intent"),
            "explore"
        );
        assert_eq!(
            check_free_target(&workflow, "plan-adversarial-review", "revise-design"),
            "design"
        );
        assert_eq!(
            check_free_target(&workflow, "plan-adversarial-review", "revise-intent"),
            "explore"
        );
        assert_eq!(
            check_free_target(
                &workflow,
                "implementation-adversarial-review",
                "revise-plan"
            ),
            "plan"
        );
        assert_eq!(
            check_free_target(
                &workflow,
                "implementation-adversarial-review",
                "revise-design"
            ),
            "design"
        );
        assert_eq!(
            check_free_target(
                &workflow,
                "implementation-adversarial-review",
                "revise-intent"
            ),
            "explore"
        );

        let live_subset = stitched(json!({
            "design-review": [axis("shape")],
            "plan-review": [axis("deps")],
            "implementation-review": [axis("code")]
        }));
        assert_nearest_revises(&live_subset);
        assert_owning_phase_revises(&live_subset);
    }

    #[test]
    fn validation_reviews_expose_revise_implementation_and_nearest_revise_is_not_implement() {
        let workflow = union();
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise"),
            "validation"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-adversarial-review", "revise"),
            "validation"
        );
        assert_ne!(
            check_free_target(&workflow, "validation-review", "revise"),
            "implement"
        );
        assert_ne!(
            check_free_target(&workflow, "validation-adversarial-review", "revise"),
            "implement"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise-implementation"),
            "implement"
        );
        assert_eq!(
            check_free_target(
                &workflow,
                "validation-adversarial-review",
                "revise-implementation"
            ),
            "implement"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise-plan"),
            "plan"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise-design"),
            "design"
        );
        assert_eq!(
            check_free_target(&workflow, "validation-review", "revise-intent"),
            "explore"
        );
        assert!(
            !workflow.transitions.iter().any(|transition| {
                transition.source.as_str() == "validation" && transition.event.as_str() == "revise"
            }),
            "validation draft must not keep nearest-revise; that edge lives on the review states"
        );
    }

    #[test]
    fn review_and_implementation_slots_declare_finding_ledger_other_drafts_omit_it() {
        let workflow = union();
        assert_review_slots_declare_finding_ledger(&workflow);
        let review_less = stitched(json!({}));
        assert_review_slots_declare_finding_ledger(&review_less);
        let mixed = stitched(json!({
            "intent-review": [axis("problem")],
            "validation-review": [axis("delivery")]
        }));
        assert_review_slots_declare_finding_ledger(&mixed);
        for id in [
            "intent-review",
            "intent-draft",
            "validation-draft",
            "implement",
            "validation-review",
        ] {
            assert_eq!(
                slot(&mixed, id).stdin_context_kinds,
                vec![
                    FINDING_LEDGER_KIND,
                    REVIEW_EVIDENCE_KIND,
                    loop_core::EVIDENCE_APPLICABILITY_KIND,
                    "user-steering",
                    "steering-incorporation"
                ]
            );
        }
    }

    #[test]
    fn gate_ids_match_review_state_names_from_the_phase_table() {
        assert_eq!(crate::config::GATE_IDS, phase_table_gate_ids().as_slice());
        for phase in PHASES {
            assert!(phase.parent_review_title.contains("review"));
            assert!(crate::config::GATE_IDS.contains(&phase.parent_review));
            assert!(crate::config::GATE_IDS.contains(&phase.adversarial_review));
            assert_eq!(phase.nearest_revise_target, phase.draft_state);
        }
        assert!(!crate::config::GATE_IDS.contains(&"intent"));
        assert!(!crate::config::GATE_IDS.contains(&"validation"));
    }

    #[test]
    fn challenge_review_wording_preserves_machine_ids_and_failure_burden() {
        let workflow = union();
        for phase in PHASES {
            let state = workflow
                .states
                .iter()
                .find(|state| state.id == phase.adversarial_review.into())
                .unwrap_or_else(|| panic!("missing {}", phase.adversarial_review));
            assert_eq!(
                state.title,
                format!(
                    "{} challenge review",
                    phase.parent_review_title.trim_end_matches(" review")
                )
            );
            let instructions = state.instructions.to_ascii_lowercase();
            for clause in [
                "challenge review",
                "meaningfully falsify",
                "current supplied evidence",
                "violated frozen obligation",
                "concrete consequence",
                "why existing validation does not resolve",
                "hypothetical threats",
                "invented requirements",
                "mechanism-for-its-own-sake",
            ] {
                assert!(
                    instructions.contains(clause),
                    "{} challenge guidance missing {clause:?}",
                    phase.adversarial_review
                );
            }
            assert!(
                !state
                    .title
                    .to_ascii_lowercase()
                    .contains("adversarial review")
                    && !instructions.contains("adversarial review"),
                "{} leaked machine terminology into human-facing wording",
                phase.adversarial_review
            );
            assert!(
                phase.adversarial_review.contains("adversarial"),
                "machine review ID changed: {}",
                phase.adversarial_review
            );
        }
    }

    #[test]
    fn contract_v3_inserts_reconciliation_without_migrating_older_graphs() {
        let v3 = describe_workflow(Some(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "test-v3",
            "review_policies": {
                "implementation-review": [axis("code")]
            }
        })))
        .expect("v3 workflow");

        let state_ids = v3
            .states
            .iter()
            .map(|state| state.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            state_ids,
            vec![
                "explore",
                "design",
                "plan",
                "implement",
                "reconciliation",
                "implementation-review",
                "validation",
                "end",
            ]
        );
        let implement_edge = v3
            .transitions
            .iter()
            .find(|edge| {
                edge.source.as_str() == "implement"
                    && edge.event.as_str() == "implementation-ready"
                    && edge.target.as_str() == RECONCILIATION_STATE
            })
            .expect("implementation enters reconciliation");
        assert_eq!(implement_edge.kind, TransitionKind::Checked);
        assert_eq!(
            duties_for_transition(
                implement_edge.source.as_str(),
                implement_edge.event.as_str(),
                implement_edge.target.as_str()
            ),
            Some(TransitionDuties::CheckedNoArtifact)
        );
        let reconciliation_edge = v3
            .transitions
            .iter()
            .find(|edge| {
                edge.source.as_str() == RECONCILIATION_STATE
                    && edge.event.as_str() == RECONCILIATION_READY_EVENT
            })
            .expect("reconciliation enters final implementation boundary");
        assert_eq!(reconciliation_edge.target.as_str(), "implementation-review");
        assert_eq!(
            duties_for_transition(
                reconciliation_edge.source.as_str(),
                reconciliation_edge.event.as_str(),
                reconciliation_edge.target.as_str()
            ),
            Some(TransitionDuties::Checked {
                subject: RECONCILIATION_SUBJECT,
                gate: None,
            })
        );
        let slot = v3
            .work_slots
            .iter()
            .find(|slot| slot.id.as_str() == RECONCILIATION_DRAFT_SLOT)
            .expect("reconciliation draft slot");
        assert_eq!(slot.state.as_str(), RECONCILIATION_STATE);
        assert_eq!(slot.event.as_str(), RECONCILIATION_READY_EVENT);
        let state = v3
            .states
            .iter()
            .find(|state| state.id.as_str() == RECONCILIATION_STATE)
            .expect("reconciliation state");
        assert!(state.instructions.contains(RECONCILIATION_SCHEMA_PATH));
        for field in RECONCILIATION_RESULT_FIELDS {
            assert!(
                state.instructions.contains(field),
                "missing result field {field}"
            );
        }
        assert!(state
            .instructions
            .contains("sole implementation-report writer"));
        assert!(state.instructions.contains("Bookends disabled"));

        let bookends = describe_workflow(Some(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "test-v3",
            "extra": {"bookends": {"enabled": true}},
            "review_policies": {}
        })))
        .expect("Bookends workflow");
        let bookends_state = bookends
            .states
            .iter()
            .find(|state| state.id.as_str() == RECONCILIATION_STATE)
            .expect("Bookends reconciliation state");
        assert!(bookends_state
            .instructions
            .contains("actual accepted PRD wording"));
        assert!(!bookends_state.instructions.contains("do not add PRD IDs"));

        let old_v2 = describe_workflow(Some(&json!({
            "contract_version": 2,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "old-v2",
            "review_policies": {}
        })))
        .expect("old workflow");
        assert!(!old_v2
            .states
            .iter()
            .any(|state| state.id.as_str() == RECONCILIATION_STATE));
        assert_eq!(
            old_v2
                .transitions
                .iter()
                .find(|edge| {
                    edge.source.as_str() == "implement"
                        && edge.event.as_str() == "implementation-ready"
                })
                .expect("old implementation edge")
                .target
                .as_str(),
            "validation"
        );

        // A stored pre-feature v3 snapshot still names the old target. The
        // target-aware duty lookup must not reinterpret it as reconciliation.
        assert_eq!(
            duties_for_transition("implement", "implementation-ready", "implementation-review"),
            Some(TransitionDuties::Checked {
                subject: "implementation-report.json",
                gate: None,
            })
        );
    }

    #[test]
    fn reconciliation_result_fields_match_the_closed_schema_source() {
        let schema: Value =
            serde_json::from_str(include_str!("../data/reconciliation-schema.json"))
                .expect("reconciliation schema JSON");
        let required = schema["required"].as_array().expect("required fields");
        let required = required
            .iter()
            .map(|value| value.as_str().expect("required field"))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required,
            RECONCILIATION_RESULT_FIELDS.iter().copied().collect()
        );
        assert_eq!(schema["additionalProperties"], Value::Bool(false));
    }

    #[test]
    fn duties_for_maps_draft_ready_and_review_approved_from_the_same_table() {
        assert_eq!(
            duties_for("explore", "intent-ready"),
            Some(TransitionDuties::Checked {
                subject: "intent.json",
                gate: None,
            })
        );
        assert_eq!(
            duties_for("intent-review", "approved"),
            Some(TransitionDuties::Checked {
                subject: "intent.json",
                gate: Some("intent-review"),
            })
        );
        assert_eq!(
            duties_for("intent-adversarial-review", "approved"),
            Some(TransitionDuties::Checked {
                subject: "intent.json",
                gate: Some("intent-adversarial-review"),
            })
        );
        assert_eq!(
            duties_for("validation", "passed"),
            Some(TransitionDuties::Checked {
                subject: "validation-report.json",
                gate: None,
            })
        );
        assert_eq!(
            duties_for("validation-review", "passed"),
            Some(TransitionDuties::Checked {
                subject: "validation-report.json",
                gate: Some("validation-review"),
            })
        );
        assert_eq!(
            duties_for("validation-review", "revise"),
            Some(TransitionDuties::CheckFree)
        );
        assert_eq!(
            duties_for("validation-adversarial-review", "revise-implementation"),
            Some(TransitionDuties::CheckFree)
        );
        assert_eq!(duties_for("design-review", "passed"), None);
        assert_eq!(duties_for("explore", "approved"), None);
    }
}
