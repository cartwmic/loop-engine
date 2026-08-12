//! Static software-change workflow topology and authoring guidance.

use loop_core::{State, Transition, Workflow};

/// Return fixed software-change topology and input-independent guidance.
///
/// Per-run obligations are intentionally absent from this value.  Callers
/// inspect frozen initial input through `show` for those obligations.
pub(crate) fn software_change_workflow() -> Workflow {
    Workflow::new(
        "software-change",
        "explore",
        vec![
            State::new(
                "explore",
                "Explore",
                "Author intent in `crates/software-change-provider/data/templates/intent.md`: state the problem, desired outcome, acceptance boundary, constraints, and non-goals. Do not prescribe an implementation unless an external constraint requires it. Avoid target laundering: do not present a chosen solution as the problem. Before `intent-ready`, read run-frozen obligations via `show` and run configured deterministic checks before commissioning external review.",
                false,
            ),
            State::new(
                "design",
                "Design",
                "Describe structural shape, boundaries, invariants, and decisions in `crates/software-change-provider/data/templates/design.md`; design is not a work schedule. The `design-ready` event structurally checks `design.json` when the run's configuration supplies a schema for it — read your obligations via `show`.",
                false,
            ),
            State::new(
                "design-review",
                "Design review",
                "For `design.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and append conforming `review-evidence` records using `crates/software-change-provider/data/reviewer-protocol.md`; use `crates/software-change-provider/data/templates/design.md` as the artifact shape.",
                false,
            ),
            State::new(
                "plan",
                "Plan",
                "Build the dependency graph in `crates/software-change-provider/data/templates/task-packet.md`: include per-task objective, dependencies, source-of-truth references, deliverables, out-of-scope work, validation, and handoff contract. Put contract gates before parallel fan-out; plan shape, not implementation prose, is the target. Before `plan-ready`, read run-frozen obligations via `show`.",
                false,
            ),
            State::new(
                "plan-review",
                "Plan review",
                "For `plan.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and append conforming `review-evidence` records using `crates/software-change-provider/data/reviewer-protocol.md`; use `crates/software-change-provider/data/templates/task-packet.md` as the artifact shape.",
                false,
            ),
            State::new(
                "implement",
                "Implement",
                "Perform external work against the accepted plan. Document the implementation and validation report shapes using `crates/software-change-provider/data/templates/implementation-report.md` and `crates/software-change-provider/data/templates/validation-report.md`. Doc integration is part of this change: update authoritative repository documents rather than leaving a parallel change truth. Before `implementation-ready`, read run-frozen obligations via `show`.",
                false,
            ),
            State::new(
                "implementation-review",
                "Implementation review",
                "For `implementation-report.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show` and append conforming `review-evidence` records using `crates/software-change-provider/data/reviewer-protocol.md`; report coverage must identify repository state and covered document revisions.",
                false,
            ),
            State::new(
                "validation",
                "Validation",
                "For `validation-report.json`, run the configured deterministic check first, before commissioning external review. Then read policy obligations via `show`, verify intent delivery and documentation integration, and append conforming `review-evidence` records using `crates/software-change-provider/data/reviewer-protocol.md`; use `crates/software-change-provider/data/templates/validation-report.md` as the artifact shape.",
                false,
            ),
            State::new(
                "end",
                "End",
                "The software change is complete. Preserve the final artifacts, evidence, coverage manifest, and authoritative document integration described by the shipped templates.",
                true,
            ),
        ],
        vec![
            Transition::checked("explore", "intent-ready", "design"),
            Transition::checked("design", "design-ready", "design-review"),
            Transition::checked("design-review", "approved", "plan"),
            Transition::check_free("design-review", "revise", "design"),
            Transition::checked("plan", "plan-ready", "plan-review"),
            Transition::checked("plan-review", "approved", "implement"),
            Transition::check_free("plan-review", "revise", "plan"),
            Transition::checked(
                "implement",
                "implementation-ready",
                "implementation-review",
            ),
            Transition::checked(
                "implementation-review",
                "approved",
                "validation",
            ),
            Transition::check_free("implementation-review", "revise", "implement"),
            Transition::checked("validation", "passed", "end"),
            Transition::check_free("validation", "revise", "implement"),
        ],
    )
}
