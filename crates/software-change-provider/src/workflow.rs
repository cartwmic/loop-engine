//! Union phase table and live-graph stitcher for software-change.
//!
//! Describe and evaluate both read [`PHASES`] so live topology and evaluation
//! duties cannot drift. When `review_policies` is omitted, describe emits the
//! sixteen-state union. When the key is present, only live review states are
//! emitted and ready/approved/passed rewire onto the next live successor.

use loop_core::{State, Transition, WorkSlot, Workflow};
use serde_json::Value;
use std::collections::BTreeSet;

/// Context kind forwarded on every live review slot's stdin.
pub(crate) const ACCEPTED_FINDINGS_KIND: &str = "accepted-findings";

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
        draft_instructions: "Author intent in `crates/software-change-provider/data/templates/intent.md`: state the problem, desired outcome, acceptance boundary, constraints, and non-goals. Do not prescribe an implementation unless an external constraint requires it. Avoid target laundering: do not present a chosen solution as the problem. Before `intent-ready`, read run-frozen obligations via `show` and run configured deterministic checks before commissioning external review.",
        draft_slot: "intent-draft",
        ready_event: "intent-ready",
        subject: "intent.json",
        parent_review: "intent-review",
        parent_review_title: "Intent review",
        parent_review_instructions: "For `intent.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for intent corrections; do not waive known defects.",
        adversarial_review: "intent-adversarial-review",
        nearest_revise_target: "explore",
        extra_owning_revises: &[],
        next_draft: Some("design"),
    },
    Phase {
        name: "design",
        draft_state: "design",
        draft_title: "Design",
        draft_instructions: "Describe structural shape, boundaries, invariants, and decisions in `crates/software-change-provider/data/templates/design.md`; design is not a work schedule. The `design-ready` event structurally checks `design.json` when the run's configuration supplies a schema for it — read your obligations via `show`.",
        draft_slot: "design-draft",
        ready_event: "design-ready",
        subject: "design.json",
        parent_review: "design-review",
        parent_review_title: "Design review",
        parent_review_instructions: "For `design.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for design corrections or `revise-intent` for intent-owned defects; do not waive known defects.",
        adversarial_review: "design-adversarial-review",
        nearest_revise_target: "design",
        extra_owning_revises: &[REVISE_INTENT],
        next_draft: Some("plan"),
    },
    Phase {
        name: "plan",
        draft_state: "plan",
        draft_title: "Plan",
        draft_instructions: "Build the dependency graph in `crates/software-change-provider/data/templates/task-packet.md`: include per-task objective, dependencies, source-of-truth references, deliverables, out-of-scope work, validation, and handoff contract. Put contract gates before parallel fan-out; plan shape, not implementation prose, is the target. Before `plan-ready`, read run-frozen obligations via `show`.",
        draft_slot: "plan-draft",
        ready_event: "plan-ready",
        subject: "plan.json",
        parent_review: "plan-review",
        parent_review_title: "Plan review",
        parent_review_instructions: "For `plan.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for plan corrections, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects; do not waive known defects.",
        adversarial_review: "plan-adversarial-review",
        nearest_revise_target: "plan",
        extra_owning_revises: &[REVISE_DESIGN, REVISE_INTENT],
        next_draft: Some("implement"),
    },
    Phase {
        name: "implementation",
        draft_state: "implement",
        draft_title: "Implement",
        draft_instructions: "Perform external work against the accepted plan. Document the implementation and validation report shapes using `crates/software-change-provider/data/templates/implementation-report.md` and `crates/software-change-provider/data/templates/validation-report.md`. Doc integration is part of this change: update authoritative repository documents rather than leaving a parallel change truth. Before `implementation-ready`, read run-frozen obligations via `show`.",
        draft_slot: "implement",
        ready_event: "implementation-ready",
        subject: "implementation-report.json",
        parent_review: "implementation-review",
        parent_review_title: "Implementation review",
        parent_review_instructions: "For `implementation-report.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Select the owning phase for accepted defects: use nearest check-free `revise` for implementation corrections, `revise-plan` for plan-owned defects, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects; do not waive known defects. Report coverage must identify repository state and covered document revisions.",
        adversarial_review: "implementation-adversarial-review",
        nearest_revise_target: "implement",
        extra_owning_revises: &[REVISE_PLAN, REVISE_DESIGN, REVISE_INTENT],
        next_draft: Some("validation"),
    },
    Phase {
        name: "validation",
        draft_state: "validation",
        draft_title: "Validation",
        draft_instructions: "Author the validation report in `crates/software-change-provider/data/templates/validation-report.md`. Verify intent delivery and documentation integration. Before the checked hop out of this room (`validation-ready` or `passed`), read run-frozen obligations via `show` and run configured deterministic checks. Validation-report-local defects stay in this draft: edit and recheck `validation-report.json`, then retry the next checked hop.",
        draft_slot: "validation-draft",
        ready_event: "validation-ready",
        subject: "validation-report.json",
        parent_review: "validation-review",
        parent_review_title: "Validation review",
        parent_review_instructions: "For `validation-report.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and follow `crates/software-change-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes and downstream regressions. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`); previous visibility or reviewer overlook does not waive known material defects. Comprehensive-first and scope/materiality burdens still bar drip-feeding or unrelated reopening. Validation-report-local defects use nearest check-free `revise` back to the validation draft, then retry the next checked hop. Select the owning phase for accepted defects: use `revise-implementation` for implementation-owned defects, `revise-plan` for plan-owned defects, `revise-design` for design-owned defects, or `revise-intent` for intent-owned defects. Do not waive known defects. Use the validation report template as the artifact shape.",
        adversarial_review: "validation-adversarial-review",
        nearest_revise_target: "validation",
        extra_owning_revises: &[REVISE_IMPLEMENTATION, REVISE_PLAN, REVISE_DESIGN, REVISE_INTENT],
        next_draft: None,
    },
];

const END_INSTRUCTIONS: &str = "The software change is complete. Preserve the final artifacts, evidence, coverage manifest, and authoritative document integration described by the shipped templates.";

/// Duties evaluate applies for a source state plus event, taken from [`PHASES`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionDuties {
    /// Draft ready/passed: schema and revision links only.
    Checked {
        subject: &'static str,
        gate: Option<&'static str>,
    },
    /// Check-free revise events are zero-obligation.
    CheckFree,
}

/// Look up evaluation duties for a source state and event from the phase table.
pub(crate) fn duties_for(source: &str, event: &str) -> Option<TransitionDuties> {
    for phase in PHASES {
        if source == phase.draft_state {
            if event == phase.ready_event || (phase.next_draft.is_none() && event == "passed") {
                return Some(TransitionDuties::Checked {
                    subject: phase.subject,
                    gate: None,
                });
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
/// `review_policies` object keeps only live review states.
pub(crate) fn describe_workflow(initial_input: Option<&Value>) -> Result<Workflow, String> {
    let review_policies = initial_input
        .and_then(Value::as_object)
        .and_then(|object| object.get("review_policies"));
    stitch(review_policies)
}

fn stitch(review_policies: Option<&Value>) -> Result<Workflow, String> {
    if let Some(value) = review_policies {
        if !value.is_object() {
            return Err("`review_policies` must be an object".to_owned());
        }
    }

    let mut hops = Vec::new();
    for phase in PHASES {
        let (parent_live, adversarial_live) = live_reviews(phase, review_policies)?;
        hops.push(Hop::Draft(phase));
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
        states.push(hop.state());
        let event = hop_event(hop, index == last_index);
        let target = if index == last_index {
            "end"
        } else {
            hops[index + 1].state_id()
        };
        transitions.push(Transition::checked(hop.state_id(), event, target));
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
    Parent(&'static Phase),
    Adversarial(&'static Phase),
}

impl Hop {
    fn state_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_state,
            Self::Parent(phase) => phase.parent_review,
            Self::Adversarial(phase) => phase.adversarial_review,
        }
    }

    fn state(self) -> State {
        match self {
            Self::Draft(phase) => State::new(
                phase.draft_state,
                phase.draft_title,
                phase.draft_instructions,
                false,
            ),
            Self::Parent(phase) => State::new(
                phase.parent_review,
                phase.parent_review_title,
                phase.parent_review_instructions,
                false,
            ),
            Self::Adversarial(phase) => {
                let title = adversarial_title(phase.parent_review_title);
                let instructions = adversarial_instructions(phase);
                State::new(phase.adversarial_review, title, instructions, false)
            }
        }
    }

    fn slot(self, event: &str) -> WorkSlot {
        let slot = WorkSlot::new(self.slot_id(), self.state_id(), event);
        match self {
            Self::Draft(_) => slot,
            Self::Parent(_) | Self::Adversarial(_) => {
                slot.with_stdin_context_kinds(vec![ACCEPTED_FINDINGS_KIND.to_owned()])
            }
        }
    }

    fn slot_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_slot,
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
        Hop::Parent(_) | Hop::Adversarial(_) => "approved",
    }
}

fn adversarial_title(parent_title: &str) -> String {
    match parent_title.strip_suffix(" review") {
        Some(stem) => format!("{stem} adversarial review"),
        None => format!("{parent_title} adversarial review"),
    }
}

fn adversarial_instructions(phase: &Phase) -> String {
    format!(
        "This counterpart adversarial review follows parent `{}` and falsifies that parent's pass claim against named obligations. {}",
        phase.parent_review, phase.parent_review_instructions
    )
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

    fn is_review_slot_id(id: &str) -> bool {
        PHASES
            .iter()
            .any(|phase| phase.parent_review == id || phase.adversarial_review == id)
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

    fn assert_review_slots_declare_accepted_findings(workflow: &Workflow) {
        let encoded = serde_json::to_value(workflow).expect("workflow JSON");
        for slot in encoded["work_slots"].as_array().expect("work_slots") {
            let id = slot["id"].as_str().expect("slot id");
            if is_review_slot_id(id) {
                assert_eq!(
                    slot.get("stdin_context_kinds"),
                    Some(&json!([ACCEPTED_FINDINGS_KIND])),
                    "review slot {id} must declare stdin_context_kinds [accepted-findings]"
                );
            } else {
                assert!(
                    slot.get("stdin_context_kinds").is_none(),
                    "draft slot {id} must omit stdin_context_kinds"
                );
            }
        }
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
    fn live_review_slots_declare_accepted_findings_and_draft_slots_omit_the_field() {
        let workflow = union();
        assert_review_slots_declare_accepted_findings(&workflow);
        let review_less = stitched(json!({}));
        assert_review_slots_declare_accepted_findings(&review_less);
        let mixed = stitched(json!({
            "intent-review": [axis("problem")],
            "validation-review": [axis("delivery")]
        }));
        assert_review_slots_declare_accepted_findings(&mixed);
        assert_eq!(
            slot(&mixed, "intent-review").stdin_context_kinds,
            vec![ACCEPTED_FINDINGS_KIND]
        );
        assert!(slot(&mixed, "intent-draft").stdin_context_kinds.is_empty());
        assert!(slot(&mixed, "validation-draft")
            .stdin_context_kinds
            .is_empty());
        assert_eq!(
            slot(&mixed, "validation-review").stdin_context_kinds,
            vec![ACCEPTED_FINDINGS_KIND]
        );
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
