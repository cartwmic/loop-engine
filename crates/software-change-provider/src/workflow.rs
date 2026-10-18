//! Union phase table and live-graph stitcher for software-change.
//!
//! Describe and evaluate both read [`PHASES`] so live topology and evaluation
//! duties cannot drift. Bare describe emits the historical sixteen-state
//! union. A new contract-v3 input adds the provider-owned reconciliation hop;
//! when `review_policies` is present, only its live review states are emitted
//! and ready/approved/passed rewire onto the next live successor. Frozen
//! extra.narratives/checkpoints add only the selected phase-exit hops.

use crate::{narratives, overlay};
use loop_core::{State, Transition, WorkSlot, Workflow};
use serde_json::{json, Value};
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
pub(crate) const RECONCILIATION_DECISION_KIND: &str = "reconciliation-decision";
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

/// Owning-phase check-free revise to a phase draft.
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
        draft_instructions: "Build the dependency graph in `crates/software-change-provider/data/templates/task-packet.md`: inspect the frozen intent `operating_context` and include per-task objective, affected user or operator paths, observable completion outcome, dependencies, source-of-truth references, deliverables, out-of-scope work, validation, and handoff contract. Require realistic black-box proof when practical or a concrete impracticality reason and substitute; leave replaceable mechanisms free inside frozen decisions. Preserve meaningful dependencies; the driver may group and execute work freely. Split work into tasks that do not share writable files or state unless one depends on the other, and declare a dependency only where a real contract, context or hard-input need exists. Plan shape, not implementation prose, is the target. Before `plan-ready`, read run-frozen obligations via `show`.",
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
        draft_instructions: "Perform external work against the accepted plan and frozen intent `operating_context`; do not add excluded hostile or multi-tenant requirements or waive stated outcomes and outside obligations. Tasks with no dependency path between them (no chain of declared dependencies in either direction) are dispatched concurrently by default; state a reason when running them serially. Document the implementation and validation report shapes using `crates/software-change-provider/data/templates/implementation-report.md` and `crates/software-change-provider/data/templates/validation-report.md`. Doc integration is part of this change: update authoritative repository documents rather than leaving a parallel change truth. Before `implementation-ready`, read run-frozen obligations via `show`. If returning from reconciliation through `revise-implementation`, preserve already-authorized document edits, do not reapply them, and finalize the implementation report/checkpoint against the current post-reconciliation tree before `implementation-ready`. For earlier-phase defects, choose `revise-plan` to plan, `revise-design` to design, or `revise-intent` to explore directly; no implementation report is required for these check-free routes. The driver/harness owns any external work and its cleanup; ensure your own external work is quiescent before departing. Routes preserve artifacts, context and denial history; neither engine nor provider classifies the defect or rolls back the repository. Only routes in this run's stored graph are available; upgrading does not add them to historical runs.",
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

/// Conditional states share the owning row of PHASES; in particular intent
/// repairs go to explore, implementation repairs to implement, and validation
/// approval still concerns validation-report.json.
pub(crate) const APPROVED_ARTIFACT_KIND: &str = "approved-artifact";
pub(crate) const ARTIFACT_ADMITTED_KIND: &str = "artifact-admitted";
pub(crate) const NARRATIVE_ADMITTED_KIND: &str = "narrative-admitted";
pub(crate) const OWNER_DECISION_KIND: &str = "owner-decision";
pub(crate) const NARRATIVE_READY_EVENT: &str = "narrative-ready";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NarrativeStates {
    pub draft: &'static str,
    pub draft_slot: &'static str,
    pub review: &'static str,
    pub challenge: &'static str,
    pub subject: &'static str,
    pub markdown: &'static str,
    pub artifact_revise: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PhaseOptions {
    pub phase: &'static Phase,
    pub checkpoint: &'static str,
    pub narrative: Option<NarrativeStates>,
}

pub(crate) const PHASE_OPTIONS: &[PhaseOptions] = &[
    PhaseOptions {
        phase: &PHASES[0],
        checkpoint: "intent-checkpoint",
        narrative: None,
    },
    PhaseOptions {
        phase: &PHASES[1],
        checkpoint: "design-checkpoint",
        narrative: Some(NarrativeStates {
            draft: "design-narrative",
            draft_slot: "design-narrative-draft",
            review: "design-narrative-review",
            challenge: "design-narrative-adversarial-review",
            subject: "design-narrative.json",
            markdown: "design-narrative.md",
            artifact_revise: "revise-design",
        }),
    },
    PhaseOptions {
        phase: &PHASES[2],
        checkpoint: "plan-checkpoint",
        narrative: Some(NarrativeStates {
            draft: "plan-narrative",
            draft_slot: "plan-narrative-draft",
            review: "plan-narrative-review",
            challenge: "plan-narrative-adversarial-review",
            subject: "plan-narrative.json",
            markdown: "plan-narrative.md",
            artifact_revise: "revise-plan",
        }),
    },
    PhaseOptions {
        phase: &PHASES[3],
        checkpoint: "implementation-checkpoint",
        narrative: None,
    },
    PhaseOptions {
        phase: &PHASES[4],
        checkpoint: "validation-checkpoint",
        narrative: Some(NarrativeStates {
            draft: "validation-narrative",
            draft_slot: "validation-narrative-draft",
            review: "validation-narrative-review",
            challenge: "validation-narrative-adversarial-review",
            subject: "validation-narrative.json",
            markdown: "validation-narrative.md",
            artifact_revise: "revise-validation",
        }),
    },
];

pub(crate) fn option_enabled(input: Option<&Value>, key: &str, phase: &str) -> bool {
    input
        .and_then(|v| v.get("extra"))
        .and_then(|v| v.get(key))
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(phase)))
}

/// Source ownership, including inserted states and the implementation-owned
/// reconciliation hop. This says nothing about which states a frozen run has.
pub(crate) fn phase_options_for_state(source: &str) -> Option<&'static PhaseOptions> {
    PHASE_OPTIONS.iter().find(|options| {
        let phase = options.phase;
        source == phase.draft_state
            || source == phase.parent_review
            || source == phase.adversarial_review
            || source == options.checkpoint
            || (source == RECONCILIATION_STATE && phase.name == "implementation")
            || options
                .narrative
                .is_some_and(|n| [n.draft, n.review, n.challenge].contains(&source))
    })
}

/// One order/ownership rule for all active states, including conditional hops.
fn earlier_phase_revises(source: &str) -> impl Iterator<Item = OwningRevise> {
    let owner = phase_options_for_state(source).map(|o| o.phase.name);
    let index = PHASES
        .iter()
        .position(|p| Some(p.name) == owner)
        .unwrap_or(0);
    [
        REVISE_INTENT,
        REVISE_DESIGN,
        REVISE_PLAN,
        REVISE_IMPLEMENTATION,
    ]
    .into_iter()
    .take(index)
    .rev()
}

const EARLIER_DRAFT_GUIDANCE: &str = "In newly started runs, every active state offers check-free `revise-intent`, `revise-design`, `revise-plan` and `revise-implementation` for whichever of those phases is earlier than its owning phase, directly to `explore`, `design`, `plan` and `implement` respectively. No artifact, report, owner rejection or override is required. These routes preserve artifacts, context, findings and history, roll back nothing, and grant no approval; every forward check still applies, including a fresh owner decision on the current revision at an enabled checkpoint. Earlier-started runs keep their stored graphs.";

pub(crate) fn checkpoint_subject(options: &PhaseOptions, narrative_enabled: bool) -> &'static str {
    options
        .narrative
        .filter(|_| narrative_enabled)
        .map_or(options.phase.subject, |n| n.subject)
}

/// Routing-only contract for T5. These duties do not evaluate freshness,
/// decisions, proof or completion. Use the exact stored edge, not a remembered
/// terminal review ID; completion always reads validation-report.json.
#[allow(dead_code)] // Consumed by the separate evaluator implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PhaseExitDuties {
    pub completion_subject: Option<&'static str>,
    pub record_accepted_implementation_proof: bool,
    /// The last artifact hop entering a configured narrative/checkpoint.
    /// With omitted review rooms this can be a draft or reconciliation hop.
    pub approved_artifact: Option<&'static Phase>,
}

#[allow(dead_code)] // Routing API handed to T5; deliberately no gates here.
pub(crate) fn phase_exit_duties(source: &str, event: &str, target: &str) -> PhaseExitDuties {
    let approving = event == "approved"
        || event == "passed"
        || event == RECONCILIATION_READY_EVENT
        || PHASES.iter().any(|p| event == p.ready_event);
    let owner = phase_options_for_state(source);
    let artifact_source = owner.is_some_and(|o| {
        [
            o.phase.draft_state,
            o.phase.parent_review,
            o.phase.adversarial_review,
        ]
        .contains(&source)
            || source == RECONCILIATION_STATE
    });
    PhaseExitDuties {
        completion_subject: (event == "passed" && target == "end").then_some(PHASES[4].subject),
        record_accepted_implementation_proof: approving
            && target == PHASES[4].draft_state
            && owner.is_some_and(|o| o.phase.name == "implementation"),
        approved_artifact: owner
            .filter(|o| {
                approving
                    && artifact_source
                    && (target == o.checkpoint || o.narrative.is_some_and(|n| target == n.draft))
            })
            .map(|o| o.phase),
    }
}

fn reconciliation_instructions(bookends_enabled: bool) -> String {
    let mode = if bookends_enabled {
        "With Bookends enabled, reread the actual accepted PRD wording and every authoritative document it names; preserve live traceability and never treat a related ID as semantic coverage."
    } else {
        "With Bookends disabled, inspect only the relevant authoritative repository documents against the approved intent and delivered behavior; do not add PRD IDs, Bookends citations, candidate machinery, or overlay obligations."
    };
    format!(
        "Reconcile the frozen intent `operating_context`, approved intent, delivered behavior, and current authoritative repository documents. Author exactly `{}` using `{}`. Set `{}`. {mode} Distinguish sufficient existing wording (including an implementation defect corrected under sufficient wording), change-specific proof, and missing or changed enduring meaning. A justified no-document-change action is valid. A requirements amendment needs exact owner acceptance and separately authorized application and commit; a wrong implementation is corrected as code. A blocked decision must retain its concrete blockers. This state does not approve, progress, commit, or write `implementation-report.json`, `validation-report.json`, or checkpoint files. The external driver writes the implementation report. Report finalization, repository checkpoint, implementation review, validation, and final proof consume the post-reconciliation tree downstream. If an implementation report or checkpoint is stale after an authorized reconciliation edit, use check-free `revise-implementation` to return to `implement`, use the external implementation/report owner, and return here through `implementation-ready`; preserve the authorized document edits and do not reapply them.",
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
    /// A checked handoff with no source-artifact prerequisite. Reconciliation
    /// entry uses this; checkpoint exits require T5's separate owner-decision
    /// evaluation (and freshness on approval, never on rejection).
    CheckedNoArtifact,
    /// Check-free revise events are zero-obligation.
    CheckFree,
}

/// Look up evaluation duties for a source state and event from the phase table.
pub(crate) fn duties_for(source: &str, event: &str) -> Option<TransitionDuties> {
    if earlier_phase_revises(source).any(|revise| revise.event == event) {
        return Some(TransitionDuties::CheckFree);
    }
    if source == RECONCILIATION_STATE {
        if event == RECONCILIATION_READY_EVENT {
            return Some(TransitionDuties::Checked {
                subject: RECONCILIATION_SUBJECT,
                gate: None,
            });
        }
        if event == REVISE_IMPLEMENTATION.event {
            return Some(TransitionDuties::CheckFree);
        }
    }

    for options in PHASE_OPTIONS {
        if source == options.checkpoint {
            // T5 checks owner records, with freshness only on approving exits.
            // No artifact prerequisite may trap a checked rejection route.
            return (matches!(event, "approved" | "revise")
                || (event == "revise-narrative" && options.narrative.is_some()))
            .then_some(TransitionDuties::CheckedNoArtifact)
            .or_else(|| {
                (options.phase.next_draft.is_none() && event == "passed")
                    .then_some(TransitionDuties::CheckedNoArtifact)
            });
        }
        if let Some(n) = options.narrative {
            if [n.draft, n.review, n.challenge].contains(&source) {
                if event == n.artifact_revise || (source != n.draft && event == "revise") {
                    return Some(TransitionDuties::CheckFree);
                }
                if (source == n.draft && event == NARRATIVE_READY_EVENT)
                    || (source != n.draft && event == "approved")
                    || (options.phase.next_draft.is_none() && event == "passed")
                {
                    return Some(TransitionDuties::Checked {
                        subject: n.subject,
                        gate: if source == n.review {
                            Some(n.review)
                        } else if source == n.challenge {
                            Some(n.challenge)
                        } else {
                            None
                        },
                    });
                }
                return None;
            }
        }
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

/// The model role that must have a matching session declaration on this
/// checked edge, with the artifact subject the declaration names.
///
/// Author drafts are declared at the draft state's ready event (and at
/// `passed` when the phase has no next draft, which is the validation draft).
/// The implementation report is declared at `reconciliation-ready`, its first
/// artifact-bearing admission. The report-free `implementation-ready`
/// handoff into reconciliation requires no declaration.
pub(crate) fn declaration_boundary(
    source: &str,
    event: &str,
    duties: TransitionDuties,
) -> Option<(loop_core::ModelRole, &'static str)> {
    let TransitionDuties::Checked {
        subject,
        gate: None,
    } = duties
    else {
        return None;
    };
    let report = PHASES.iter().find(|phase| phase.name == "implementation")?;
    if source == RECONCILIATION_STATE && event == RECONCILIATION_READY_EVENT {
        return Some((loop_core::ModelRole::Implementer, report.subject));
    }
    let phase = PHASES.iter().find(|phase| {
        phase.draft_state == source
            && phase.subject == subject
            && (event == phase.ready_event || (phase.next_draft.is_none() && event == "passed"))
    })?;
    let role = if phase.name == "implementation" {
        loop_core::ModelRole::Implementer
    } else {
        loop_core::ModelRole::Author
    };
    Some((role, phase.subject))
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

/// Artifact gate identifiers in phase order, then the conditional narrative
/// gate catalog. Catalog membership does not enable a state.
#[cfg(test)]
pub(crate) fn phase_table_gate_ids() -> Vec<&'static str> {
    PHASES
        .iter()
        .flat_map(|phase| [phase.parent_review, phase.adversarial_review])
        .chain(
            PHASE_OPTIONS
                .iter()
                .filter_map(|o| o.narrative)
                .flat_map(|n| [n.review, n.challenge]),
        )
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
        .filter(|input| {
            input
                .get("contract_version")
                .and_then(Value::as_u64)
                .is_some_and(|version| version >= 3)
        })
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
        initial_input,
    )?;
    apply_driver_act_opt_in(&mut workflow, initial_input)?;
    if initial_input.is_some_and(|v| matches!(v["contract_version"].as_u64(), Some(2..=4))) {
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
                        "base_required_authors": row
                            .get("required_authors")
                            .cloned()
                            .unwrap_or(Value::from(1))
                    });
                    if initial_input
                        .and_then(|input| input.get("contract_version"))
                        .and_then(Value::as_u64)
                        .is_some_and(|version| version >= 3)
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
            "author_count_note": "Normalized review and criterion counts are base values. Use software-change author-counts with full show JSON for effective counts.",
            "base_criterion_policy": initial_input.and_then(|v| v.get("criterion_policy")),
            "advice": advice_action_guidance(state.id.as_str(), initial_input),
            "repair": "Keep validation-report-only corrections in validation. For an implementation defect owned by a frozen task, correct that work and affected dependencies externally. The driver may perform a narrow correction for an accepted unresolved implementation finding honestly owned by no task, preserving the finding and actual proof. Revise plan/design/intent only when that phase obligation is materially wrong; reconfirm affected downstream proof and explicitly carry unaffected original evidence.",
            "review": "First review is comprehensive. Confirm accepted fixes and fix-introduced holes, with explicit unaffected applicability; no rerun-until-pass. Falsify circular proof and evidence compatible with opposite outcomes against frozen obligations, without adding axes.",
            "execution": "External drivers execute all workers, advisors and proof commands. Inspect their outputs before append or progression; the provider never launches them.",
            "git": "After implementation triage and before independent review, ask the owner for the Git decision. Inspect staged names and diff; perform or confirm only the owner-authorized human/driver commit and verify its resulting identity with `git rev-parse HEAD`, or record pending/declined without Git changes or claiming a created commit. Workers do not independently commit. A commit changes proof identity: refresh invalidated receipts/report/checkpoints before review; then keep source/Git identity stable. This reminder grants no authorization."
        }));
    }
    Ok(workflow)
}

pub(crate) fn advice_occasion_descriptions() -> Value {
    serde_json::json!({
        "review-candidates": "After actual review candidates exist: assess separate support, materiality, and scope from selected evidence.",
        "accepted-defect": "After the driver accepts a defect: consider its correction owner.",
        "implementation-correction": "When an implementation correction is actually needed: consider frozen-task, narrow repair, or upstream routes without granting permission.",
        "execution-or-authority-issue": "When a captured execution, cleanup, authority, or owner-escalation issue exists.",
        "evidence-applicability": "When the driver declares selected prior evidence for reuse against a current target.",
        "requirements-reconciliation": "During actual reconciliation: compare delivered behavior with selected accepted wording and documents.",
        "review-round-departure": "Before leaving a review round: consider current judgments, driver ledger, fixes, and any real prior-round evidence.",
        "final-completion": "After current proof: assess each current AC, the whole goal, and whether checks could miss delivery."
    })
}

fn advice_action_guidance(_state_id: &str, initial_input: Option<&Value>) -> Value {
    let enabled = initial_input
        .and_then(|v| v.pointer("/extra/advice/enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "enabled": enabled,
        "occasion_map": initial_input.and_then(|v| v.pointer("/extra/advice/occasion_map")),
        "occasion_descriptions": advice_occasion_descriptions(),
        "request_path": "Select source context IDs and artifact excerpts from full show; software-change advice-request prepares typed questions. An external driver calls the advisor and appends advice-answer context.",
        "authority": "Advice is not a finding, route, permission, review verdict, proof result, or gate approval."
    })
}

fn apply_driver_act_opt_in(
    workflow: &mut Workflow,
    initial_input: Option<&Value>,
) -> Result<(), String> {
    let Some(value) = initial_input.and_then(|input| input.get("driver_act_slots")) else {
        return Ok(());
    };
    let slots = value.as_array().ok_or(
        "driver_act_slots must be an array of explicitly opted-in draft/implement slot IDs",
    )?;
    let allowed = ["intent-draft", "design-draft", "plan-draft", "implement"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeSet::new();
    for item in slots {
        let id = item
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("driver_act_slots entries must be nonempty strings")?;
        if !allowed.contains(id) {
            return Err(format!(
                "driver act is not permitted for slot `{id}`; only intent/design/plan draft and implement slots may opt in"
            ));
        }
        if !selected.insert(id) {
            return Err(format!("driver_act_slots contains duplicate slot `{id}`"));
        }
        let slot = workflow
            .work_slots
            .iter_mut()
            .find(|slot| slot.id.as_str() == id)
            .ok_or_else(|| format!("driver_act_slots names unavailable slot `{id}`"))?;
        slot.driver_act_allowed = true;
    }
    Ok(())
}

fn stitch(
    review_policies: Option<&Value>,
    bookends_enabled: bool,
    reconciliation_enabled: bool,
    semantic_coverage: bool,
    initial_input: Option<&Value>,
) -> Result<Workflow, String> {
    if let Some(value) = review_policies {
        if !value.is_object() {
            return Err("`review_policies` must be an object".to_owned());
        }
    }

    for (key, allowed) in [
        ("narratives", narratives::NARRATIVE_PHASES),
        ("checkpoints", narratives::CHECKPOINT_PHASES),
    ] {
        if let Some(value) = initial_input
            .and_then(|v| v.get("extra"))
            .and_then(|v| v.get(key))
        {
            let values = value
                .as_array()
                .ok_or_else(|| format!("extra.{key} must be an array"))?;
            let mut seen = BTreeSet::new();
            for value in values {
                let phase = value
                    .as_str()
                    .ok_or_else(|| format!("extra.{key} entries must be phase strings"))?;
                if !allowed.contains(&phase) || !seen.insert(phase) {
                    return Err(format!(
                        "extra.{key} contains unknown or duplicate phase `{phase}`"
                    ));
                }
            }
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
        let options = PHASE_OPTIONS
            .iter()
            .find(|o| o.phase.name == phase.name)
            .expect("every phase has an options row");
        let narrative_enabled = option_enabled(initial_input, "narratives", phase.name);
        if narrative_enabled {
            let narrative = options.narrative.expect("validated narrative phase");
            let (ordinary, challenge) = live_review_pair(
                narrative.review,
                narrative.challenge,
                phase.name,
                review_policies,
            )?;
            hops.push(Hop::Narrative(options));
            if ordinary {
                hops.push(Hop::NarrativeReview(options));
            }
            if challenge {
                hops.push(Hop::NarrativeChallenge(options));
            }
        }
        if option_enabled(initial_input, "checkpoints", phase.name) {
            hops.push(Hop::Checkpoint(options, narrative_enabled));
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
        let mut state = hop.state(bookends_enabled, semantic_coverage);
        if !matches!(hop, Hop::Parent(_) | Hop::Adversarial(_))
            && earlier_phase_revises(hop.state_id()).next().is_some()
        {
            state.instructions = format!("{} {EARLIER_DRAFT_GUIDANCE}", state.instructions);
        }
        states.push(state);
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
        if let Hop::Reconciliation = hop {
            // A post-edit report/checkpoint correction must reuse the existing
            // implementation owner and return through this same reconciliation
            // hop; it must not bypass the state or reapply document edits.
            transitions.push(Transition::check_free(
                hop.state_id(),
                REVISE_IMPLEMENTATION.event,
                REVISE_IMPLEMENTATION.target,
            ));
        }
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
        if let Hop::Narrative(options)
        | Hop::NarrativeReview(options)
        | Hop::NarrativeChallenge(options) = hop
        {
            let narrative = options.narrative.expect("narrative hop");
            if !matches!(hop, Hop::Narrative(_)) {
                transitions.push(Transition::check_free(
                    hop.state_id(),
                    "revise",
                    narrative.draft,
                ));
            }
            transitions.push(Transition::check_free(
                hop.state_id(),
                narrative.artifact_revise,
                options.phase.draft_state,
            ));
        }
        if let Hop::Checkpoint(options, narrative_enabled) = hop {
            transitions.push(Transition::checked(
                hop.state_id(),
                "revise",
                options.phase.draft_state,
            ));
            if narrative_enabled {
                transitions.push(Transition::checked(
                    hop.state_id(),
                    "revise-narrative",
                    options.narrative.expect("narrative enabled").draft,
                ));
            }
        }
        // Keep every pre-existing edge (including same-phase narrative repair
        // and checked checkpoint rejection), adding only missing earlier drafts.
        for revise in earlier_phase_revises(hop.state_id()) {
            if !transitions.iter().any(|edge| {
                edge.source.as_str() == hop.state_id() && edge.event.as_str() == revise.event
            }) {
                transitions.push(Transition::check_free(
                    hop.state_id(),
                    revise.event,
                    revise.target,
                ));
            }
        }
        let mut slot = hop.slot(event);
        if let Some(options) = phase_options_for_state(hop.state_id()).filter(|o| {
            option_enabled(initial_input, "narratives", o.phase.name)
                || option_enabled(initial_input, "checkpoints", o.phase.name)
        }) {
            slot.stdin_context_kinds
                .extend([APPROVED_ARTIFACT_KIND, ARTIFACT_ADMITTED_KIND].map(str::to_owned));
            if option_enabled(initial_input, "narratives", options.phase.name) {
                slot.stdin_context_kinds
                    .push(NARRATIVE_ADMITTED_KIND.to_owned());
            }
            if option_enabled(initial_input, "checkpoints", options.phase.name) {
                slot.stdin_context_kinds
                    .push(OWNER_DECISION_KIND.to_owned());
            }
        }
        work_slots.push(slot);
    }
    states.push(State::new("end", "End", END_INSTRUCTIONS, true));

    Ok(
        Workflow::new("software-change", "explore", states, transitions)
            .with_work_slots(work_slots),
    )
}

fn live_reviews(phase: &Phase, review_policies: Option<&Value>) -> Result<(bool, bool), String> {
    live_review_pair(
        phase.parent_review,
        phase.adversarial_review,
        phase.name,
        review_policies,
    )
}

fn live_review_pair(
    parent: &str,
    challenge: &str,
    phase_name: &str,
    review_policies: Option<&Value>,
) -> Result<(bool, bool), String> {
    let Some(policies) = review_policies else {
        return Ok((true, true));
    };
    let parent_live = nonempty_policy_list(policies.get(parent));
    let adversarial_live = nonempty_policy_list(policies.get(challenge));
    if adversarial_live && !parent_live {
        return Err(format!(
            "adversarial review `{}` for the {} phase is nonempty while parent review `{}` is empty or absent",
            challenge, phase_name, parent
        ));
    }
    if adversarial_live {
        let parent_ids = axis_ids(
            policies.get(parent).expect("a live parent list is present"),
            parent,
        )?;
        let adversarial_ids = axis_ids(
            policies
                .get(challenge)
                .expect("a live adversarial list is present"),
            challenge,
        )?;
        for id in &adversarial_ids {
            if !parent_ids.contains(id) {
                return Err(format!(
                    "adversarial axis `{id}` on `{}` is not on the {} phase parent list `{}`",
                    challenge, phase_name, parent
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
    Narrative(&'static PhaseOptions),
    NarrativeReview(&'static PhaseOptions),
    NarrativeChallenge(&'static PhaseOptions),
    Checkpoint(&'static PhaseOptions, bool),
}

impl Hop {
    fn state_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_state,
            Self::Reconciliation => RECONCILIATION_STATE,
            Self::Parent(phase) => phase.parent_review,
            Self::Adversarial(phase) => phase.adversarial_review,
            Self::Narrative(o) => o.narrative.expect("narrative hop").draft,
            Self::NarrativeReview(o) => o.narrative.expect("narrative hop").review,
            Self::NarrativeChallenge(o) => o.narrative.expect("narrative hop").challenge,
            Self::Checkpoint(o, _) => o.checkpoint,
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
            Self::Narrative(o) | Self::NarrativeReview(o) | Self::NarrativeChallenge(o) => {
                let n = o.narrative.expect("narrative hop");
                let title = match self {
                    Self::Narrative(_) => format!("{} narrative", o.phase.draft_title),
                    Self::NarrativeReview(_) => format!("{} narrative review", o.phase.draft_title),
                    _ => format!("{} narrative challenge review", o.phase.draft_title),
                };
                State::new(
                    self.state_id(),
                    title,
                    narrative_instructions(o, n, self),
                    false,
                )
            }
            Self::Checkpoint(o, narrative_enabled) => State::new(
                o.checkpoint,
                format!(
                    "{} owner checkpoint",
                    o.phase.parent_review_title.trim_end_matches(" review")
                ),
                checkpoint_instructions(o, narrative_enabled),
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
        // Review projections need prior judgments and, for validation review,
        // the exact retained command/verdict sources named by the report.
        let mut kinds = vec![
            FINDING_LEDGER_KIND.to_owned(),
            REVIEW_EVIDENCE_KIND.to_owned(),
            loop_core::EVIDENCE_APPLICABILITY_KIND.to_owned(),
            "user-steering".to_owned(),
            "steering-incorporation".to_owned(),
        ];
        if matches!(self, Self::Draft(phase) if phase.name == "implementation") {
            kinds.push(RECONCILIATION_DECISION_KIND.to_owned());
        }
        if matches!(
            self,
            Self::Parent(_)
                | Self::Adversarial(_)
                | Self::NarrativeReview(_)
                | Self::NarrativeChallenge(_)
        ) {
            kinds.extend([
                "command-evidence".to_owned(),
                "criterion-verdict".to_owned(),
                "goal-verdict".to_owned(),
            ]);
        }
        if matches!(self, Self::Draft(phase)|Self::Parent(phase)|Self::Adversarial(phase) if phase.name=="intent")
            || matches!(self,Self::Checkpoint(options,_) if options.phase.name=="intent")
        {
            kinds.push("intent-correction".to_owned());
        }
        slot.with_stdin_context_kinds(kinds)
    }

    fn slot_id(self) -> &'static str {
        match self {
            Self::Draft(phase) => phase.draft_slot,
            Self::Reconciliation => RECONCILIATION_DRAFT_SLOT,
            Self::Parent(phase) => phase.parent_review,
            Self::Adversarial(phase) => phase.adversarial_review,
            Self::Narrative(o) => o.narrative.expect("narrative hop").draft_slot,
            Self::NarrativeReview(_) | Self::NarrativeChallenge(_) | Self::Checkpoint(_, _) => {
                self.state_id()
            }
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
        Hop::Narrative(_) => NARRATIVE_READY_EVENT,
        Hop::Parent(_)
        | Hop::Adversarial(_)
        | Hop::NarrativeReview(_)
        | Hop::NarrativeChallenge(_)
        | Hop::Checkpoint(_, _) => "approved",
    }
}

fn narrative_instructions(options: &PhaseOptions, n: NarrativeStates, hop: Hop) -> String {
    let phase = options.phase;
    let common = format!(
        "The authoritative artifact is `{}`; never change its bytes during narrative work. Use the latest `approved-artifact` record for its revision and SHA-256 hash (the exact bytes), and the latest `artifact-admitted` record. The binding must cite that approval record. A changed artifact needs a new single-use artifact revision, fresh artifact reviews and a new narrative. Use check-free `{}` to return to `{}` for an artifact defect.",
        phase.subject, n.artifact_revise, phase.draft_state,
    );
    match hop {
        Hop::Narrative(_) => format!(
            "Only now, after the configured artifact reviews have cleared, write `{}` and its binding `{}` under the run's artifact_root using `crates/software-change-provider/data/templates/narrative.md` and `crates/software-change-provider/data/narrative-schema.json`. Write plain English for an engineer without Loop Engine context. Bind the exact approved artifact subject, revision and hash and the markdown file name and hash. Choose a never-admitted `<artifact revision>/<positive counter>` revision; every rewrite needs a new counter and fresh narrative review. Request the shown checked `{}` event (or `passed` if this is the final hop). Admission records the markdown hash as `narrative-admitted`; later approvals require that latest admitted revision and unchanged markdown bytes. {common}",
            n.markdown, n.subject, NARRATIVE_READY_EVENT,
        ),
        _ => {
            let stage = if matches!(hop, Hop::NarrativeChallenge(_)) {
                crate::commission::CHALLENGE_GUIDANCE
            } else {
                "Commission comprehensive independent ordinary narrative review; triage accepted findings before append."
            };
            format!(
                "{stage} Use `software-change commission --slot {}` with full show. Review `{}`, `{}`, `{}` and frozen `intent.json` together under the materialized policy stages and author counts. Retain genuine `review-evidence` and a driver `finding-ledger` for the binding's composite revision; the provider executes no reviewers and judges no findings. Approvals require the latest admitted narrative revision and matching markdown hash, and the artifact must still match its latest approved revision and hash. Use check-free `revise` to rewrite only the narrative in `{}` with a new counter and fresh narrative reviews, keeping unchanged artifact reviews. {common}",
                hop.slot_id(), n.markdown, n.subject, phase.subject, n.draft,
            )
        }
    }
}

fn checkpoint_instructions(options: &PhaseOptions, narrative_enabled: bool) -> String {
    let subject = checkpoint_subject(options, narrative_enabled);
    let narrative = if narrative_enabled {
        format!(" The narrative binding must carry the latest `narrative-admitted` revision and its markdown must match the admitted hash. Checked `revise-narrative` requires target `narrative` and returns to `{}`: rewrite under a new counter, re-review, then obtain a fresh owner decision; unchanged artifact reviews stay valid.", options.narrative.expect("narrative enabled").draft)
    } else {
        String::new()
    };
    format!(
        "Wait for the owner's decision on `{subject}` at its current approved revision. Use `software-change commission --slot {}` with full show for the owner view. Append `owner-decision` with closed data {{checkpoint, subject, subject_revision, decision: approve|reject, target, reason, text, actor:{{name,kind}}}}. Set checkpoint to `{}` and subject to `{subject}`; actor has a nonempty name and kind human, agent or script. Retain the owner's words verbatim in nonempty text. A rejection needs a nonempty reason and target `artifact`{}; do not invent approval. The latest well-formed decision for this checkpoint across all revisions must approve this exact subject and revision before checked `approved` (or final `passed`). Approving exits require the artifact's latest admitted revision/hash and unchanged bytes against the latest `approved-artifact` record. Checked `revise` requires an owner rejection targeting `artifact` and returns to `{}` for a new artifact revision and fresh reviews{} and a fresh decision. Rejection routes check the recorded decision, not file freshness, so already edited files do not prevent recovery.{narrative} Decisions and rejections remain in history; an override is not owner approval.",
        options.checkpoint, options.checkpoint,
        if narrative_enabled { " or `narrative`" } else { "" },
        options.phase.draft_state, if narrative_enabled { ", a new narrative" } else { "" },
    )
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
            "{} This challenge review follows parent `{}` and must meaningfully falsify that parent's pass claim only with current supplied evidence, a violated frozen obligation, a concrete consequence for change success, and why existing validation does not resolve the issue. Reject hypothetical threats, invented requirements, silence or style complaints, and mechanism-for-its-own-sake findings; do not waive material failures. {}",
            crate::commission::CHALLENGE_GUIDANCE,
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

    fn shipped_profiles() -> Vec<Value> {
        [
            include_str!("../data/configs/minimal.json"),
            include_str!("../data/configs/standard.json"),
            include_str!("../data/configs/high-rigor.json"),
        ]
        .into_iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
    }

    fn enable_options(input: &mut Value, narrative_mask: usize, checkpoint_mask: usize) {
        input["extra"]["narratives"] = json!(narratives::NARRATIVE_PHASES
            .iter()
            .enumerate()
            .filter(|(index, _)| narrative_mask & (1 << index) != 0)
            .map(|(_, name)| *name)
            .collect::<Vec<_>>());
        input["extra"]["checkpoints"] = json!(narratives::CHECKPOINT_PHASES
            .iter()
            .enumerate()
            .filter(|(index, _)| checkpoint_mask & (1 << index) != 0)
            .map(|(_, name)| *name)
            .collect::<Vec<_>>());
        for options in PHASE_OPTIONS {
            if !option_enabled(Some(input), "narratives", options.phase.name) {
                continue;
            }
            let n = options.narrative.unwrap();
            for (gate, template) in [(n.review, "ordinary"), (n.challenge, "challenge")] {
                let mut rows = input["extra"]["narrative_review"][template].clone();
                for row in rows.as_array_mut().unwrap() {
                    row["group"] = json!(row["group"]
                        .as_str()
                        .unwrap()
                        .replace("{phase}", options.phase.name));
                }
                input["review_policies"][gate] = rows;
            }
        }
    }

    fn edge<'a>(workflow: &'a Workflow, source: &str, event: &str) -> &'a Transition {
        workflow
            .transitions
            .iter()
            .find(|edge| edge.source.as_str() == source && edge.event.as_str() == event)
            .unwrap_or_else(|| panic!("missing {source}/{event}"))
    }

    #[test]
    fn off_successor_graphs_match_predecessor_except_exact_accepted_deltas() {
        use sha2::{Digest, Sha256};
        // Captured before T4 through public describe; also exact-byte equal to
        // installed 0.24.1 describe with the corresponding HEAD -14 profile.
        // Preserve complete historical workflow hashes; only the exact accepted
        // instruction/context delivery deltas below are inverted for comparison.
        // AC-8 (intent revision 5): before hashing, apply the exact inverse of the
        // accepted LE-184 delta (two literal instruction sentences and the listed
        // parallelism-honest axes); any other difference from 0.24.1 fails.
        const LE184_SENTENCES: [&str; 2] = [
            " Split work into tasks that do not share writable files or state unless one depends on the other, and declare a dependency only where a real contract, context or hard-input need exists.",
            " Tasks with no dependency path between them (no chain of declared dependencies in either direction) are dispatched concurrently by default; state a reason when running them serially.",
        ];
        let snapshots = [
            "9ee5121cbf31c6488edac046d752072e17cd3a35f3d139a0d74bec9dd4848e15",
            "f70cc5d07332379ba335219cee3272ac072cce98aaf7c83f4e9078e9e3aa5fc4",
            "68ea101e7b21ccc9257456e092e34014385b8827531c7a6adee8284bccaf08ee",
        ];
        for (mut input, expected) in shipped_profiles().into_iter().zip(snapshots) {
            let workflow = describe_workflow(Some(&input)).unwrap();
            let mut value = serde_json::to_value(&workflow).unwrap();
            // Keep historical hashes unchanged. The only new delivery delta is
            // one correction kind on these newly described intent slots; take
            // its exact inverse on this comparison copy, never stored workflows.
            let mut corrected_slots = 0;
            for slot in value["work_slots"].as_array_mut().unwrap() {
                let is_intent = ["intent-draft", "intent-review", "intent-adversarial-review"]
                    .contains(&slot["id"].as_str().unwrap());
                let kinds = slot["stdin_context_kinds"].as_array_mut().unwrap();
                assert_eq!(
                    kinds
                        .iter()
                        .filter(|kind| **kind == "intent-correction")
                        .count(),
                    usize::from(is_intent)
                );
                if is_intent {
                    kinds.retain(|kind| *kind != "intent-correction");
                    corrected_slots += 1;
                }
            }
            assert_eq!(corrected_slots, 3);
            // New-run draft routing deliberately supersedes predecessor parity.
            // Invert only these exact accepted additions; retain the old hashes.
            let added = [
                ("design", "revise-intent", "explore"),
                ("plan", "revise-design", "design"),
                ("plan", "revise-intent", "explore"),
                (RECONCILIATION_STATE, "revise-plan", "plan"),
                (RECONCILIATION_STATE, "revise-design", "design"),
                (RECONCILIATION_STATE, "revise-intent", "explore"),
                ("validation", "revise-plan", "plan"),
                ("validation", "revise-design", "design"),
                ("validation", "revise-intent", "explore"),
            ];
            for (source, event, target) in added {
                let expected =
                    serde_json::to_value(Transition::check_free(source, event, target)).unwrap();
                let edges = value["transitions"].as_array_mut().unwrap();
                assert_eq!(edges.iter().filter(|e| **e == expected).count(), 1);
                edges.retain(|e| *e != expected);
            }
            for source in [
                "design",
                "plan",
                "implement",
                RECONCILIATION_STATE,
                "validation",
            ] {
                let state = value["states"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|s| s["id"] == source)
                    .unwrap();
                let text = state["instructions"].as_str().unwrap();
                assert_eq!(text.matches(EARLIER_DRAFT_GUIDANCE).count(), 1);
                let old = text
                    .strip_suffix(&format!(" {EARLIER_DRAFT_GUIDANCE}"))
                    .unwrap()
                    .to_owned();
                state["instructions"] = json!(if source == "implement" {
                    assert_eq!(
                        old.matches("For earlier-phase defects, choose `revise-plan`")
                            .count(),
                        1
                    );
                    old.replacen(
                        "For earlier-phase defects, choose `revise-plan`",
                        "If the owner rejects earlier work, choose `revise-plan`",
                        1,
                    )
                } else {
                    old
                });
            }
            for sentence in LE184_SENTENCES {
                let mut holders: Vec<&mut Value> = value["states"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .filter(|state| {
                        state["instructions"]
                            .as_str()
                            .is_some_and(|text| text.contains(sentence))
                    })
                    .collect();
                assert_eq!(holders.len(), 1, "{sentence}");
                let text = holders[0]["instructions"].as_str().unwrap().to_owned();
                assert_eq!(text.matches(sentence).count(), 1, "{sentence}");
                holders[0]["instructions"] = json!(text.replacen(sentence, "", 1));
            }
            // B's accepted challenge-guidance delta changes instructions only.
            // Invert that exact prefix, retaining the predecessor graph receipt.
            for state in value["states"].as_array_mut().unwrap() {
                if state["id"]
                    .as_str()
                    .unwrap()
                    .ends_with("-adversarial-review")
                {
                    let prefix = format!("{} ", crate::commission::CHALLENGE_GUIDANCE);
                    let text = state["instructions"].as_str().unwrap();
                    state["instructions"] = json!(text
                        .strip_prefix(&prefix)
                        .expect("exact challenge guidance prefix"));
                }
                if let Some(axes) = state
                    .get_mut("action_guidance")
                    .and_then(|guidance| guidance.get_mut("review_axes"))
                    .and_then(Value::as_array_mut)
                {
                    axes.retain(|axis| axis["id"] != "parallelism-honest");
                }
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected,
                "{}",
                input["config_version"]
            );
            for key in ["narratives", "checkpoints", "narrative_review"] {
                input["extra"].as_object_mut().unwrap().remove(key);
            }
            assert_eq!(
                describe_workflow(Some(&input)).unwrap(),
                workflow,
                "absent options and inert template"
            );
        }
    }

    #[test]
    fn every_shipped_option_combination_stitches_order_routes_slots_and_duties() {
        for base in shipped_profiles() {
            for narrative_mask in 0..8 {
                for checkpoint_mask in 0..32 {
                    let mut input = base.clone();
                    enable_options(&mut input, narrative_mask, checkpoint_mask);
                    let workflow = describe_workflow(Some(&input)).unwrap();
                    let off = describe_workflow(Some(&base)).unwrap();
                    let mut expected = Vec::new();
                    for options in PHASE_OPTIONS {
                        let phase = options.phase;
                        expected.push(phase.draft_state);
                        if phase.name == "implementation" {
                            expected.push(RECONCILIATION_STATE);
                        }
                        expected.extend([phase.parent_review, phase.adversarial_review]);
                        let narrative_enabled =
                            option_enabled(Some(&input), "narratives", phase.name);
                        let checkpoint_enabled =
                            option_enabled(Some(&input), "checkpoints", phase.name);
                        if narrative_enabled {
                            let n = options.narrative.unwrap();
                            expected.extend([n.draft, n.review, n.challenge]);
                            for id in [n.draft, n.review, n.challenge] {
                                assert_eq!(
                                    check_free_target(&workflow, id, n.artifact_revise),
                                    phase.draft_state
                                );
                                assert_eq!(phase_options_for_state(id), Some(options));
                            }
                            for id in [n.review, n.challenge] {
                                assert_eq!(check_free_target(&workflow, id, "revise"), n.draft);
                            }
                            assert_eq!(slot(&workflow, n.draft_slot).state.as_str(), n.draft);
                            assert_eq!(
                                slot(&workflow, n.draft_slot).event.as_str(),
                                NARRATIVE_READY_EVENT
                            );
                            assert_eq!(
                                checked_hop(&workflow, phase.adversarial_review),
                                ("approved", n.draft)
                            );
                        }
                        if checkpoint_enabled {
                            expected.push(options.checkpoint);
                            let revise = edge(&workflow, options.checkpoint, "revise");
                            assert_eq!(revise.kind, TransitionKind::Checked);
                            assert_eq!(revise.target.as_str(), phase.draft_state);
                            assert_eq!(
                                checkpoint_subject(options, narrative_enabled),
                                if narrative_enabled {
                                    options.narrative.unwrap().subject
                                } else {
                                    phase.subject
                                }
                            );
                            if narrative_enabled {
                                let revise =
                                    edge(&workflow, options.checkpoint, "revise-narrative");
                                assert_eq!(revise.kind, TransitionKind::Checked);
                                assert_eq!(
                                    revise.target.as_str(),
                                    options.narrative.unwrap().draft
                                );
                            } else {
                                assert!(!workflow
                                    .transitions
                                    .iter()
                                    .any(|e| e.source.as_str() == options.checkpoint
                                        && e.event.as_str() == "revise-narrative"));
                            }
                        }
                        // Existing artifact repairs remain byte-identical, even
                        // when the approving edge now enters a conditional hop.
                        for id in [phase.parent_review, phase.adversarial_review] {
                            let before = off
                                .transitions
                                .iter()
                                .filter(|e| {
                                    e.source.as_str() == id && e.kind == TransitionKind::CheckFree
                                })
                                .collect::<Vec<_>>();
                            let after = workflow
                                .transitions
                                .iter()
                                .filter(|e| {
                                    e.source.as_str() == id && e.kind == TransitionKind::CheckFree
                                })
                                .collect::<Vec<_>>();
                            assert_eq!(before, after);
                        }
                        for work_slot in &workflow.work_slots {
                            if phase_options_for_state(work_slot.state.as_str()) != Some(options) {
                                continue;
                            }
                            let kinds = &work_slot.stdin_context_kinds;
                            let enabled = narrative_enabled || checkpoint_enabled;
                            assert_eq!(kinds.iter().any(|k| k == APPROVED_ARTIFACT_KIND), enabled);
                            assert_eq!(kinds.iter().any(|k| k == ARTIFACT_ADMITTED_KIND), enabled);
                            assert_eq!(
                                kinds.iter().any(|k| k == NARRATIVE_ADMITTED_KIND),
                                narrative_enabled
                            );
                            assert_eq!(
                                kinds.iter().any(|k| k == OWNER_DECISION_KIND),
                                checkpoint_enabled
                            );
                            assert!(!work_slot.driver_act_allowed);
                            if !enabled {
                                assert_eq!(
                                    work_slot,
                                    slot(&off, work_slot.id.as_str()),
                                    "unrelated slot changed"
                                );
                            }
                        }
                    }
                    expected.push("end");
                    assert_eq!(state_ids(&workflow), expected);
                    let forward = workflow
                        .transitions
                        .iter()
                        .filter(|e| {
                            e.event.as_str() != "revise" && !e.event.as_str().starts_with("revise-")
                        })
                        .collect::<Vec<_>>();
                    for (index, e) in forward.iter().enumerate() {
                        assert_eq!(e.source.as_str(), expected[index]);
                        assert_eq!(e.target.as_str(), expected[index + 1]);
                        assert_eq!(e.kind, TransitionKind::Checked);
                        assert_eq!(e.event.as_str() == "passed", index == forward.len() - 1);
                    }
                    for e in &workflow.transitions {
                        let duties = duties_for_transition(
                            e.source.as_str(),
                            e.event.as_str(),
                            e.target.as_str(),
                        )
                        .expect("all new and old edges have duties");
                        assert_eq!(
                            duties == TransitionDuties::CheckFree,
                            e.kind == TransitionKind::CheckFree
                        );
                        let exit = phase_exit_duties(
                            e.source.as_str(),
                            e.event.as_str(),
                            e.target.as_str(),
                        );
                        assert_eq!(
                            exit.completion_subject,
                            if e.event.as_str() == "passed" {
                                Some("validation-report.json")
                            } else {
                                None
                            }
                        );
                        assert_eq!(
                            exit.record_accepted_implementation_proof,
                            e.target.as_str() == "validation"
                                && !e.event.as_str().starts_with("revise")
                        );
                    }
                    for state in &workflow.states {
                        let source = state.id.as_str();
                        let Some(owner) = phase_options_for_state(source) else {
                            assert_eq!(source, "end");
                            assert!(!workflow.transitions.iter().any(|e| e.source == state.id));
                            continue;
                        };
                        let index = PHASES
                            .iter()
                            .position(|p| p.name == owner.phase.name)
                            .unwrap();
                        for (earlier_index, (event, target)) in [
                            ("revise-intent", "explore"),
                            ("revise-design", "design"),
                            ("revise-plan", "plan"),
                            ("revise-implementation", "implement"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let edges = workflow
                                .transitions
                                .iter()
                                .filter(|e| {
                                    e.source.as_str() == source && e.event.as_str() == event
                                })
                                .collect::<Vec<_>>();
                            if earlier_index < index {
                                assert_eq!(edges.len(), 1, "{source}/{event}");
                                assert_eq!(edges[0].target.as_str(), target);
                                assert_eq!(edges[0].kind, TransitionKind::CheckFree);
                                assert_eq!(
                                    duties_for(source, event),
                                    Some(TransitionDuties::CheckFree)
                                );
                                let exit = phase_exit_duties(source, event, target);
                                assert_eq!(exit.completion_subject, None);
                                assert_eq!(exit.approved_artifact, None);
                                assert!(!exit.record_accepted_implementation_proof);
                            } else if earlier_index > index {
                                assert!(edges.is_empty(), "no later-phase route: {source}/{event}");
                            } else if !owner
                                .narrative
                                .is_some_and(|n| [n.draft, n.review, n.challenge].contains(&source))
                                && source != RECONCILIATION_STATE
                            {
                                assert!(
                                    edges.is_empty(),
                                    "no new same-phase route: {source}/{event}"
                                );
                            }
                        }
                    }
                    assert_eq!(workflow.work_slots.len(), workflow.states.len() - 1);
                }
            }
        }
    }

    #[test]
    fn narrative_policy_omission_and_orphans_follow_existing_review_semantics() {
        for (parent, challenge, last) in [
            (false, false, "validation-narrative"),
            (true, false, "validation-narrative-review"),
            (true, true, "validation-narrative-adversarial-review"),
        ] {
            let mut input = json!({"extra":{"narratives":["validation"]},"review_policies":{}});
            if parent {
                input["review_policies"]["validation-narrative-review"] = json!([axis("a")]);
            }
            if challenge {
                input["review_policies"]["validation-narrative-adversarial-review"] =
                    json!([axis("a")]);
            }
            let workflow = describe_workflow(Some(&input)).unwrap();
            assert_eq!(has_state(&workflow, "validation-narrative-review"), parent);
            assert_eq!(
                has_state(&workflow, "validation-narrative-adversarial-review"),
                challenge
            );
            assert_eq!(
                checked_hop(&workflow, "validation"),
                ("validation-ready", "validation-narrative")
            );
            assert_eq!(checked_hop(&workflow, last), ("passed", "end"));
            input["extra"]["checkpoints"] = json!(["validation"]);
            let workflow = describe_workflow(Some(&input)).unwrap();
            let expected_event = if parent {
                "approved"
            } else {
                NARRATIVE_READY_EVENT
            };
            assert_eq!(
                checked_hop(&workflow, last),
                (expected_event, "validation-checkpoint")
            );
            assert_eq!(
                slot(&workflow, "validation-checkpoint").event.as_str(),
                "passed"
            );
        }
        let mut input = json!({"extra":{"narratives":["design"]}, "review_policies": {
            "design-narrative-adversarial-review":[axis("a")]
        }});
        assert!(describe_workflow(Some(&input))
            .unwrap_err()
            .contains("empty or absent"));
        input["review_policies"]["design-narrative-review"] = json!([axis("b")]);
        assert!(describe_workflow(Some(&input))
            .unwrap_err()
            .contains("not on"));
        // Live policies alone never turn on a narrative phase.
        input["extra"]["narratives"] = json!([]);
        assert!(!has_state(
            &describe_workflow(Some(&input)).unwrap(),
            "design-narrative-review"
        ));
    }

    #[test]
    fn new_state_titles_and_instructions_explain_sources_freshness_and_repairs() {
        let mut input = shipped_profiles().remove(2);
        enable_options(&mut input, 7, 31);
        let workflow = describe_workflow(Some(&input)).unwrap();
        for options in PHASE_OPTIONS {
            let checkpoint = workflow
                .states
                .iter()
                .find(|s| s.id.as_str() == options.checkpoint)
                .unwrap();
            assert!(checkpoint.title.ends_with("owner checkpoint"));
            for text in [
                "owner-decision",
                "verbatim",
                "nonempty reason",
                "latest well-formed",
                "not file freshness",
                "history",
                "override is not owner approval",
                options.phase.draft_state,
            ] {
                assert!(
                    checkpoint.instructions.contains(text),
                    "{} missing {text}",
                    options.checkpoint
                );
            }
            if let Some(n) = options.narrative {
                for id in [n.draft, n.review, n.challenge] {
                    let state = workflow
                        .states
                        .iter()
                        .find(|s| s.id.as_str() == id)
                        .unwrap();
                    assert!(state.title.contains("narrative"));
                    assert!(!state.title.contains("adversarial"));
                    for text in [
                        n.subject,
                        n.markdown,
                        options.phase.subject,
                        "latest",
                        "hash",
                        n.artifact_revise,
                    ] {
                        assert!(state.instructions.contains(text), "{id} missing {text}");
                    }
                    if id != n.draft {
                        assert!(state.instructions.contains("frozen `intent.json`"));
                        assert!(state.instructions.contains("finding-ledger"));
                        assert!(state.action_guidance.as_ref().unwrap()["review_axes"].is_array());
                    }
                }
                assert_eq!(
                    duties_for(n.draft, NARRATIVE_READY_EVENT),
                    Some(TransitionDuties::Checked {
                        subject: n.subject,
                        gate: None
                    })
                );
                assert_eq!(
                    duties_for(n.review, "approved"),
                    Some(TransitionDuties::Checked {
                        subject: n.subject,
                        gate: Some(n.review)
                    })
                );
                assert_eq!(
                    duties_for(n.challenge, "approved"),
                    Some(TransitionDuties::Checked {
                        subject: n.subject,
                        gate: Some(n.challenge)
                    })
                );
            }
        }
    }

    #[test]
    fn phase_exit_routing_moves_completion_and_implementation_proof_without_evaluating_them() {
        for source in [
            "validation",
            "validation-review",
            "validation-adversarial-review",
            "validation-narrative",
            "validation-narrative-review",
            "validation-narrative-adversarial-review",
            "validation-checkpoint",
        ] {
            assert_eq!(
                phase_exit_duties(source, "passed", "end").completion_subject,
                Some("validation-report.json")
            );
        }
        for source in [
            "implement",
            RECONCILIATION_STATE,
            "implementation-review",
            "implementation-adversarial-review",
            "implementation-checkpoint",
        ] {
            let event = if source == "implement" {
                "implementation-ready"
            } else if source == RECONCILIATION_STATE {
                RECONCILIATION_READY_EVENT
            } else {
                "approved"
            };
            assert!(
                phase_exit_duties(source, event, "validation").record_accepted_implementation_proof
            );
            assert!(
                !phase_exit_duties(source, "revise", "implement")
                    .record_accepted_implementation_proof
            );
        }
        for options in PHASE_OPTIONS {
            for source in [
                options.phase.draft_state,
                options.phase.parent_review,
                options.phase.adversarial_review,
            ] {
                let event = if source == options.phase.draft_state {
                    options.phase.ready_event
                } else {
                    "approved"
                };
                assert_eq!(
                    phase_exit_duties(source, event, options.checkpoint).approved_artifact,
                    Some(options.phase)
                );
                assert_eq!(
                    phase_exit_duties(options.checkpoint, "revise", options.phase.draft_state)
                        .approved_artifact,
                    None
                );
            }
        }
        assert_eq!(
            phase_exit_duties(
                RECONCILIATION_STATE,
                RECONCILIATION_READY_EVENT,
                "implementation-checkpoint"
            )
            .approved_artifact,
            Some(&PHASES[3])
        );
        assert_eq!(duties_for("intent-checkpoint", "revise-narrative"), None);
        assert_eq!(
            duties_for("implementation-checkpoint", "revise-narrative"),
            None
        );
    }

    #[test]
    fn bare_describe_refuses_malformed_phase_options_instead_of_panicking() {
        for extra in [
            json!({"narratives":["intent"]}),
            json!({"checkpoints":["reconciliation"]}),
            json!({"narratives":true}),
            json!({"checkpoints":[1]}),
            json!({"narratives":["design","design"]}),
        ] {
            assert!(describe_workflow(Some(&json!({"extra":extra}))).is_err());
        }
    }

    #[test]
    fn reviewless_enabled_graphs_keep_ready_events_and_reconciliation_proof_boundary() {
        let mut input = shipped_profiles().remove(0);
        enable_options(&mut input, 7, 31);
        input["review_policies"] = json!({});
        let workflow = describe_workflow(Some(&input)).unwrap();
        for options in PHASE_OPTIONS {
            let phase = options.phase;
            assert!(!has_state(&workflow, phase.parent_review));
            assert!(!has_state(&workflow, phase.adversarial_review));
            if let Some(n) = options.narrative {
                assert!(!has_state(&workflow, n.review));
                assert!(!has_state(&workflow, n.challenge));
                assert_eq!(
                    checked_hop(&workflow, phase.draft_state),
                    (phase.ready_event, n.draft)
                );
                assert_eq!(
                    checked_hop(&workflow, n.draft),
                    (NARRATIVE_READY_EVENT, options.checkpoint)
                );
            } else if phase.name == "implementation" {
                assert_eq!(
                    checked_hop(&workflow, "implement"),
                    (phase.ready_event, RECONCILIATION_STATE)
                );
                assert_eq!(
                    checked_hop(&workflow, RECONCILIATION_STATE),
                    (RECONCILIATION_READY_EVENT, options.checkpoint)
                );
            } else {
                assert_eq!(
                    checked_hop(&workflow, phase.draft_state),
                    (phase.ready_event, options.checkpoint)
                );
            }
        }
        assert_eq!(
            edge(&workflow, "implementation-checkpoint", "approved")
                .target
                .as_str(),
            "validation"
        );
        input["extra"]["checkpoints"] = json!(["validation"]);
        let workflow = describe_workflow(Some(&input)).unwrap();
        let entry = edge(&workflow, RECONCILIATION_STATE, RECONCILIATION_READY_EVENT);
        assert_eq!(entry.target.as_str(), "validation");
        assert!(
            phase_exit_duties(
                entry.source.as_str(),
                entry.event.as_str(),
                entry.target.as_str()
            )
            .record_accepted_implementation_proof
        );
    }

    #[test]
    fn absent_policies_keep_union_reviews_for_enabled_narratives() {
        let workflow =
            describe_workflow(Some(&json!({"extra":{"narratives":["design"]}}))).unwrap();
        assert_eq!(
            checked_hop(&workflow, "design-adversarial-review"),
            ("approved", "design-narrative")
        );
        assert_eq!(
            checked_hop(&workflow, "design-narrative"),
            (NARRATIVE_READY_EVENT, "design-narrative-review")
        );
        assert_eq!(
            checked_hop(&workflow, "design-narrative-review"),
            ("approved", "design-narrative-adversarial-review")
        );
        assert_eq!(
            checked_hop(&workflow, "design-narrative-adversarial-review"),
            ("approved", "plan")
        );
    }

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

    #[test]
    fn driver_act_requires_explicit_opt_in_and_rejects_review_slots() {
        let ordinary = union();
        assert!(!slot(&ordinary, "implement").driver_act_allowed);
        let opted = describe_workflow(Some(&json!({
            "driver_act_slots":["design-draft","implement"]
        })))
        .expect("future draft/implement opt-in");
        assert!(slot(&opted, "design-draft").driver_act_allowed);
        assert!(slot(&opted, "implement").driver_act_allowed);
        assert!(!slot(&opted, "implementation-review").driver_act_allowed);
        assert!(describe_workflow(Some(&json!({
            "driver_act_slots":["implementation-review"]
        })))
        .is_err());
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
            let mut expected = vec![
                FINDING_LEDGER_KIND,
                REVIEW_EVIDENCE_KIND,
                loop_core::EVIDENCE_APPLICABILITY_KIND,
                "user-steering",
                "steering-incorporation",
            ];
            if id == "implement" {
                expected.push(RECONCILIATION_DECISION_KIND);
            }
            if id.ends_with("-review") {
                expected.extend(["command-evidence", "criterion-verdict", "goal-verdict"]);
            }
            if [
                "intent-draft",
                "intent-review",
                "intent-adversarial-review",
                "intent-checkpoint",
            ]
            .contains(&id)
            {
                expected.push("intent-correction");
            }
            assert_eq!(
                slot.get("stdin_context_kinds"),
                Some(&json!(expected)),
                "slot {id} must support steering and stable source projection"
            );
        }
    }

    #[test]
    fn recovery_backtracking_preserves_implementation_routes_and_extends_earlier_drafts() {
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
            assert!(!workflow.transitions.iter().any(|edge| {
                edge.source.as_str() == "explore" && edge.kind == TransitionKind::CheckFree
            }));
            assert_eq!(
                check_free_target(&workflow, "design", "revise-intent"),
                "explore"
            );
            assert_eq!(
                check_free_target(&workflow, "plan", "revise-design"),
                "design"
            );
            assert_eq!(
                check_free_target(&workflow, "plan", "revise-intent"),
                "explore"
            );
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
                "driver/harness owns any external work",
                "quiescent before departing",
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
            let mut expected = vec![
                FINDING_LEDGER_KIND,
                REVIEW_EVIDENCE_KIND,
                loop_core::EVIDENCE_APPLICABILITY_KIND,
                "user-steering",
                "steering-incorporation",
            ];
            if id == "implement" {
                expected.push(RECONCILIATION_DECISION_KIND);
            }
            if id.ends_with("-review") {
                expected.extend(["command-evidence", "criterion-verdict", "goal-verdict"]);
            }
            if ["intent-review", "intent-draft"].contains(&id) {
                expected.push("intent-correction");
            }
            assert_eq!(slot(&mixed, id).stdin_context_kinds, expected);
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
        let reconciliation_revise = v3
            .transitions
            .iter()
            .find(|edge| {
                edge.source.as_str() == RECONCILIATION_STATE
                    && edge.event.as_str() == REVISE_IMPLEMENTATION.event
            })
            .expect("reconciliation report-correction route");
        assert_eq!(reconciliation_revise.target.as_str(), "implement");
        assert_eq!(reconciliation_revise.kind, TransitionKind::CheckFree);
        assert_eq!(
            duties_for_transition(
                reconciliation_revise.source.as_str(),
                reconciliation_revise.event.as_str(),
                reconciliation_revise.target.as_str()
            ),
            Some(TransitionDuties::CheckFree)
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
            .contains("external driver writes the implementation report"));
        assert!(state.instructions.contains("Bookends disabled"));
        assert!(state
            .instructions
            .contains("check-free `revise-implementation`"));
        assert!(state.instructions.contains("do not reapply them"));

        let reviewless_v3 = describe_workflow(Some(&json!({
            "contract_version": 3,
            "criterion_policy": {"required_authors": 1, "goal_required_authors": 1},
            "config_version": "test-v3",
            "review_policies": {}
        })))
        .expect("reviewless v3 workflow");
        assert_eq!(
            check_free_target(
                &reviewless_v3,
                RECONCILIATION_STATE,
                REVISE_IMPLEMENTATION.event
            ),
            "implement"
        );

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
        assert_eq!(
            duties_for(RECONCILIATION_STATE, REVISE_IMPLEMENTATION.event),
            Some(TransitionDuties::CheckFree)
        );
        assert_eq!(duties_for("design-review", "passed"), None);
        assert_eq!(duties_for("explore", "approved"), None);
    }
}
