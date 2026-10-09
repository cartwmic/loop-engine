//! Exit retention for the optional provider `capture` operation.
//!
//! The run leaves an artifact-owning phase forward when a checked transition
//! moves to a state of a later phase (or `end`). Review states belong to the
//! phase they review, so the rule follows whatever live graph the frozen
//! policy stitched, including profiles without review states.

use loop_core::Transition;
use loop_integrations::retention::{retained_artifacts_effect, CaptureRequest, RetainedArtifact};
use serde_json::Value;
use std::path::Path;

/// Phase order and the artifacts each phase owns.
const PHASES: [&[&str]; 5] = [
    &["intent.json"],
    &["design.json"],
    &["plan.json"],
    &["implementation-report.json", "reconciliation.json"],
    &["validation-report.json"],
];

/// Phase index of a software-change state; `end` sorts after every phase.
fn phase(state: &str) -> Option<usize> {
    match state {
        "explore" => Some(0),
        "end" => Some(PHASES.len()),
        "implement" | "reconciliation" => Some(3),
        _ => ["intent", "design", "plan", "implementation", "validation"]
            .iter()
            .position(|prefix| state == *prefix || state.starts_with(&format!("{prefix}-"))),
    }
}

/// Artifacts the run leaves with on `transition`, or none when the transition
/// stays inside a phase, goes backward, or is check-free.
pub(crate) fn exiting_artifacts(transition: &Transition) -> &'static [&'static str] {
    if !transition.kind.is_checked() {
        return &[];
    }
    match (
        phase(transition.source.as_str()),
        phase(transition.target.as_str()),
    ) {
        (Some(source), Some(target)) if target > source && source < PHASES.len() => PHASES[source],
        _ => &[],
    }
}

/// Answer one capture request with the retained-artifacts effect, if any.
pub(crate) fn capture(request: &CaptureRequest) -> Result<Value, String> {
    let names = exiting_artifacts(&request.transition);
    if names.is_empty() {
        return Ok(loop_integrations::retention::capture_response(None));
    }
    let root = request
        .initial_input
        .get("artifact_root")
        .and_then(Value::as_str)
        .filter(|root| Path::new(root).is_absolute())
        .ok_or("capture requires an absolute frozen artifact_root")?;
    let artifacts: Vec<_> = names
        .iter()
        .map(|name| RetainedArtifact::new(*name, Path::new(root).join(name)))
        .collect();
    let effect = retained_artifacts_effect(
        &request.context,
        &request.transition,
        request.exit,
        &artifacts,
    )
    .map_err(|error| format!("could not read artifact for capture: {error}"))?;
    Ok(loop_integrations::retention::capture_response(Some(effect)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str, event: &str, target: &str) -> Vec<&'static str> {
        exiting_artifacts(&Transition::checked(source, event, target)).to_vec()
    }

    #[test]
    fn forward_phase_exits_retain_the_owning_phase_artifacts() {
        assert_eq!(
            names("intent-adversarial-review", "approved", "design"),
            ["intent.json"]
        );
        assert_eq!(
            names("design-adversarial-review", "approved", "plan"),
            ["design.json"]
        );
        assert_eq!(
            names("plan-adversarial-review", "approved", "implement"),
            ["plan.json"]
        );
        assert_eq!(
            names(
                "implementation-adversarial-review",
                "approved",
                "validation"
            ),
            ["implementation-report.json", "reconciliation.json"]
        );
        assert_eq!(
            names("validation-adversarial-review", "passed", "end"),
            ["validation-report.json"]
        );
    }

    #[test]
    fn graphs_without_review_states_retain_at_their_real_exits() {
        assert_eq!(names("explore", "intent-ready", "design"), ["intent.json"]);
        assert_eq!(names("plan", "plan-ready", "implement"), ["plan.json"]);
        assert_eq!(
            names("reconciliation", "reconciliation-ready", "validation"),
            ["implementation-report.json", "reconciliation.json"]
        );
        assert_eq!(
            names("validation", "validation-ready", "end"),
            ["validation-report.json"]
        );
    }

    #[test]
    fn same_phase_backward_and_check_free_transitions_retain_nothing() {
        assert!(names("explore", "intent-ready", "intent-review").is_empty());
        assert!(names("intent-review", "approved", "intent-adversarial-review").is_empty());
        assert!(names("implement", "implementation-ready", "reconciliation").is_empty());
        assert!(names("plan-review", "revise-design", "design").is_empty());
        assert!(
            exiting_artifacts(&Transition::check_free("plan-review", "revise", "plan")).is_empty()
        );
    }
}
