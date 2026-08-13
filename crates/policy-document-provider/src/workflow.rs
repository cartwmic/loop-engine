use loop_core::{State, Transition, Workflow};
pub fn workflow() -> Workflow {
    Workflow::new("policy-document", "prepare", vec![
        State::new("prepare", "Prepare", "Draft or revise target document. Frozen mode and policy obligations are available in initial input.", false),
        State::new("deterministic-review", "Deterministic review", "Run configured deterministic checks against current target bytes. Fix every reported policy violation before semantic review.", false),
        State::new("semantic-review", "Semantic review", "Obtain external review-evidence for every configured semantic policy. Provider validates evidence shape, identity, and freshness; it does not judge content.", false),
        State::new("end", "End", "Target document complete for current deterministic and external semantic evidence.", true),
    ], vec![
        Transition::check_free("prepare", "ready", "deterministic-review"),
        Transition::checked("deterministic-review", "passed", "semantic-review"),
        Transition::check_free("deterministic-review", "revise", "prepare"),
        Transition::checked("semantic-review", "passed", "end"),
        Transition::check_free("semantic-review", "revise", "prepare"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::TransitionKind;

    #[test]
    fn topology_is_exact_and_guidance_is_input_independent() {
        let value = workflow();
        assert_eq!(value.id.as_str(), "policy-document");
        assert_eq!(value.initial_state.as_str(), "prepare");
        assert_eq!(value.states.len(), 4);
        assert_eq!(value.transitions.len(), 5);
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
                (
                    "prepare",
                    "ready",
                    "deterministic-review",
                    TransitionKind::CheckFree
                ),
                (
                    "deterministic-review",
                    "passed",
                    "semantic-review",
                    TransitionKind::Checked
                ),
                (
                    "deterministic-review",
                    "revise",
                    "prepare",
                    TransitionKind::CheckFree
                ),
                ("semantic-review", "passed", "end", TransitionKind::Checked),
                (
                    "semantic-review",
                    "revise",
                    "prepare",
                    TransitionKind::CheckFree
                ),
            ]
        );
        assert!(value.states[0].instructions.contains("mode"));
        assert!(value.states[2].instructions.contains("external"));
    }
}
