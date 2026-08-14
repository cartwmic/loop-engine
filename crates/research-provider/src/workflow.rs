//! Static research workflow topology and authoring guidance.

use loop_core::{State, Transition, Workflow};

/// Return fixed research topology and input-independent guidance.
///
/// Per-run obligations are intentionally absent from this value.  Callers
/// inspect frozen initial input through `show` for those obligations.
pub(crate) fn research_workflow() -> Workflow {
    Workflow::new(
        "research",
        "scope",
        vec![
            State::new(
                "scope",
                "Scope",
                "Author the research brief in `crates/research-provider/data/templates/brief.md`: state the question, scope, observable acceptance, constraints, and non-goals. Search, fetch, and writing happen outside the provider. Do not present a chosen answer as the question. Before `scoped`, read run-frozen obligations via `show`.",
                false,
            ),
            State::new(
                "gather",
                "Gather",
                "Record gathered sources in `crates/research-provider/data/templates/sources.md`. Perform search and fetch externally; the provider never retrieves URLs. Cite extracts the later verification can check. `brief_revision` must equal current `brief.json` revision when checked. Before `gathered`, read run-frozen obligations via `show`. Use check-free `revise` for brief-owned defects.",
                false,
            ),
            State::new(
                "verify",
                "Verify",
                "For `verification.json`, run the configured deterministic check first, before commissioning external review. Author claims, cited source ids, support, and challenges using `crates/research-provider/data/templates/verification.md`. Then read policy obligations via `show` and follow `crates/research-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The provider does not judge claim truth. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`). Select the owning phase for accepted defects: verification-local defects stay in verify (edit and recheck `verification.json`, then retry checked `verified`); use nearest check-free `revise` for sources-owned defects or `revise-brief` for brief-owned defects; do not waive known defects.",
                false,
            ),
            State::new(
                "synthesize",
                "Synthesize",
                "For `report.json`, run the configured deterministic check first, before commissioning external review. Author the cited conclusion using `crates/research-provider/data/templates/report.md`. Then read policy obligations via `show` and follow `crates/research-provider/data/reviewer-protocol.md`: triage candidate reviewer output before append or mutation, require consequence proof and scope/materiality classification, and append only accepted in-scope material failures or conforming passes. The provider does not judge conclusion quality. The first review is comprehensive; use focused external reconsideration for disputed candidates and confirmation review for accepted fixes. Late findings require current evidence, violated obligation, concrete consequence, validation gap, and provenance (`newly exposed`, `fix-introduced`, or `previously overlooked`). Report-local defects stay in synthesize: edit and recheck `report.json`, then retry checked `completed`. Select the owning phase for accepted defects: nearest check-free `revise` is verification-owned only; use `revise-sources` for sources-owned defects or `revise-brief` for brief-owned defects. Do not waive known defects.",
                false,
            ),
            State::new(
                "end",
                "End",
                "The research run is complete. Preserve the final brief, sources, verification, report, and independent review-evidence described by the shipped templates.",
                true,
            ),
        ],
        vec![
            Transition::checked("scope", "scoped", "gather"),
            Transition::checked("gather", "gathered", "verify"),
            Transition::check_free("gather", "revise", "scope"),
            Transition::checked("verify", "verified", "synthesize"),
            Transition::check_free("verify", "revise", "gather"),
            Transition::check_free("verify", "revise-brief", "scope"),
            Transition::checked("synthesize", "completed", "end"),
            Transition::check_free("synthesize", "revise", "verify"),
            Transition::check_free("synthesize", "revise-sources", "gather"),
            Transition::check_free("synthesize", "revise-brief", "scope"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::TransitionKind;

    #[test]
    fn topology_is_exact_and_guidance_is_input_independent() {
        let value = research_workflow();
        assert_eq!(value.id.as_str(), "research");
        assert_eq!(value.initial_state.as_str(), "scope");
        assert_eq!(value.states.len(), 5);
        assert_eq!(value.transitions.len(), 10);
        assert_eq!(
            value
                .states
                .iter()
                .filter(|state| state.is_final)
                .map(|state| state.id.as_str())
                .collect::<Vec<_>>(),
            vec!["end"]
        );
        let routes = value
            .transitions
            .iter()
            .map(|route| {
                (
                    route.source.as_str(),
                    route.event.as_str(),
                    route.target.as_str(),
                    route.kind,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            routes,
            vec![
                ("scope", "scoped", "gather", TransitionKind::Checked),
                ("gather", "gathered", "verify", TransitionKind::Checked),
                ("gather", "revise", "scope", TransitionKind::CheckFree),
                ("verify", "verified", "synthesize", TransitionKind::Checked),
                ("verify", "revise", "gather", TransitionKind::CheckFree),
                ("verify", "revise-brief", "scope", TransitionKind::CheckFree),
                ("synthesize", "completed", "end", TransitionKind::Checked),
                ("synthesize", "revise", "verify", TransitionKind::CheckFree),
                (
                    "synthesize",
                    "revise-sources",
                    "gather",
                    TransitionKind::CheckFree
                ),
                (
                    "synthesize",
                    "revise-brief",
                    "scope",
                    TransitionKind::CheckFree
                ),
            ]
        );
        assert!(value.states[0].instructions.contains("brief.md"));
        assert!(value.states[1].instructions.contains("externally"));
        assert!(value.states[2]
            .instructions
            .contains("reviewer-protocol.md"));
        assert!(value.states[3].instructions.contains("report.md"));
    }
}
