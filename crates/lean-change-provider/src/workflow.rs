use loop_core::{State, Transition, Workflow};

use crate::model::WORKFLOW_ID;

/// B has a review state; C is the same topology without it. An old-vs-new profile (one set up with
/// `old_vs_new`) also says what each criterion expects on the old tree.
pub fn workflow(review: bool, old_vs_new: bool) -> Workflow {
    let mut explore = "Read the repository and the request. Append one `intent` record (author, goal, constraints, hard, criteria each with id, statement and an executable check {argv, timeout_ms}) with --record-id intent-N, then request intent-ready. Set hard=true only for work that needs a written plan.".to_owned();
    if review {
        explore.push_str(" Before intent-ready, pipe `loop-engine --json show --view full RUN` into `lean-change intent-packet` and give the packet to a fresh-context reviewer who did not write the intent. Append their `intent-check` record and one `finding-disposition` per finding (fixed or rejected, with a reason). A fix is a new intent record.");
    }
    if old_vs_new {
        explore.push_str(" Each criterion also needs `old`: {\"expect\": \"fails\" | \"same\" | \"absent\" | \"none\", \"reason\": \"...\"}. Verify runs its check on the old tree (the base commit with the current proof files over it) and records the result. \"fails\": a bug fix or a feature, the check must really fail there, twice. \"same\": a refactor, exit 0 with the same output. \"absent\": brand-new behavior, any result but a pass. \"none\" with a reason is the escape hatch for work the check cannot apply to. Add --proof-path to setup for each directory or file that holds the tests or journeys (a git pathspec relative to the repository root, repeatable; use :(glob) for tests that sit beside code). With no --proof-path the old tree is the bare base: a test file the change adds is missing there, so a check that runs it gives a cheap non-zero exit, which is not proof.");
    }
    let mut states = vec![
        State::new("explore", "Explore", explore, false),
        State::new("intent-approval", "Intent approval", "Show the owner the intent record verbatim. Append their exact words as an `owner-decision` record (checkpoint \"intent\", subject \"intent\", subject_revision = the intent record id, decision approve or reject, text = their words, actor). Request approved (hard=false) or approved-hard (hard=true). Request revise to change the intent.", false),
        State::new("plan", "Plan", "Hard work only. Append a `plan` record {intent: intent record id, text}, then request planned.", false),
        State::new("implement", "Implement", "Make the change in the repository. Do not edit outside the approved intent. When done, request ready.", false),
        State::new("verify", "Verify", "Pipe `loop-engine --json show --view full RUN` into `lean-change verify` and append its output as a `verification` record. Every criterion check must pass on the current tree; any later edit makes the record stale. Then request verified. Request revise to return to implement.", false),
    ];
    let mut transitions = vec![
        Transition::checked("explore", "intent-ready", "intent-approval"),
        Transition::checked("intent-approval", "approved", "implement"),
        Transition::checked("intent-approval", "approved-hard", "plan"),
        Transition::check_free("intent-approval", "revise", "explore"),
        Transition::checked("plan", "planned", "implement"),
        Transition::check_free("plan", "revise", "explore"),
        Transition::check_free("implement", "ready", "verify"),
        Transition::check_free("implement", "revise", "explore"),
        Transition::check_free("verify", "revise", "implement"),
    ];
    if review {
        states.push(State::new("review", "Review", "One review of the diff by someone other than the author, in a fresh context. Pipe the show output into `lean-change review-packet` and give the packet to the reviewer. Append their `review` record. Append one `finding-disposition` record per finding (fixed or rejected, with a reason). After fixes, run verify again and append the new verification. Then request reviewed.", false));
        transitions.push(Transition::checked("verify", "verified", "review"));
        transitions.push(Transition::checked("review", "reviewed", "end"));
        transitions.push(Transition::check_free("review", "revise", "implement"));
    } else {
        transitions.push(Transition::checked("verify", "verified", "end"));
    }
    states.push(State::new(
        "end",
        "End",
        "Every criterion check passes on the final tree.",
        true,
    ));
    Workflow::new(WORKFLOW_ID, "explore", states, transitions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(review: bool) -> Vec<(String, String, String, bool)> {
        workflow(review, false)
            .transitions
            .iter()
            .map(|t| {
                (
                    t.source.as_str().into(),
                    t.event.as_str().into(),
                    t.target.as_str().into(),
                    t.kind.is_checked(),
                )
            })
            .collect()
    }

    #[test]
    fn b_has_seven_states_and_twelve_transitions() {
        let b = workflow(true, false);
        assert_eq!(b.states.len(), 7);
        assert_eq!(b.transitions.len(), 12);
        assert!(routes(true).contains(&(
            "verify".into(),
            "verified".into(),
            "review".into(),
            true
        )));
        assert!(routes(true).contains(&("review".into(), "reviewed".into(), "end".into(), true)));
    }

    #[test]
    fn c_is_b_without_the_review_state() {
        let c = workflow(false, false);
        assert_eq!(c.states.len(), 6);
        assert_eq!(c.transitions.len(), 10);
        assert!(c.states.iter().all(|s| s.id.as_str() != "review"));
        assert!(routes(false).contains(&("verify".into(), "verified".into(), "end".into(), true)));
        let shared: Vec<_> = routes(true)
            .into_iter()
            .filter(|r| r.0 != "review" && !(r.0 == "verify" && r.1 == "verified"))
            .collect();
        assert!(shared.iter().all(|r| routes(false).contains(r)));
    }
}
