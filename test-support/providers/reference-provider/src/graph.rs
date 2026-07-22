//! Reference software-change workflow graph declaration.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::config::DescribeGraphVariant;
use crate::protocol::{
    GraphDto, InputDeclarationDto, StateDto, StaticGuidanceDeclarationDto, StaticGuidanceDto,
    TransitionDto,
};

#[cfg(test)]
pub const REGISTRATION_ID: &str = "019f6e88-b403-73a6-89f9-ebfe668b417e";

pub const STATE_EXPLORE: &str = "explore";
pub const STATE_DESIGN: &str = "design";
pub const STATE_DESIGN_REVIEW: &str = "design-review";
pub const STATE_PLAN: &str = "plan";
pub const STATE_PLAN_REVIEW: &str = "plan-review";
pub const STATE_IMPLEMENT: &str = "implement";
pub const STATE_IMPLEMENTATION_REVIEW: &str = "implementation-review";
pub const STATE_VALIDATION: &str = "validation";
pub const STATE_END: &str = "end";

pub const GATE_INTENT_READY: &str = "intent-ready";
pub const GATE_DESIGN_READY: &str = "design-ready";
pub const GATE_DESIGN_REVIEW_APPROVED: &str = "design-review-approved";
pub const GATE_DESIGN_REVIEW_CHANGES: &str = "design-review-changes-requested";
pub const GATE_PLAN_READY: &str = "plan-ready";
pub const GATE_PLAN_REVIEW_APPROVED: &str = "plan-review-approved";
pub const GATE_PLAN_REVIEW_CHANGES: &str = "plan-review-changes-requested";
pub const GATE_IMPLEMENTATION_READY: &str = "implementation-ready";
pub const GATE_IMPLEMENTATION_REVIEW_APPROVED: &str = "implementation-review-approved";
pub const GATE_IMPLEMENTATION_REVIEW_CHANGES: &str = "implementation-review-changes-requested";
pub const GATE_VALIDATION_PASSED: &str = "validation-passed";
pub const GATE_VALIDATION_FAILED: &str = "validation-failed";

/// Gate retained in stored graphs from older provider builds; current code does not implement it.
pub const GATE_LEGACY_STORED_ONLY: &str = "legacy-intent-ready";

/// All gate identifiers implemented by the current provider build.
pub const SUPPORTED_GATES: &[&str] = &[
    GATE_INTENT_READY,
    GATE_DESIGN_READY,
    GATE_DESIGN_REVIEW_APPROVED,
    GATE_DESIGN_REVIEW_CHANGES,
    GATE_PLAN_READY,
    GATE_PLAN_REVIEW_APPROVED,
    GATE_PLAN_REVIEW_CHANGES,
    GATE_IMPLEMENTATION_READY,
    GATE_IMPLEMENTATION_REVIEW_APPROVED,
    GATE_IMPLEMENTATION_REVIEW_CHANGES,
    GATE_VALIDATION_PASSED,
    GATE_VALIDATION_FAILED,
];

pub fn build_graph(variant: DescribeGraphVariant) -> GraphDto {
    let mut graph = GraphDto {
        initial_state: STATE_EXPLORE.to_string(),
        states: vec![
            state(
                STATE_EXPLORE,
                false,
                "Explore the change context and capture intent.",
            ),
            state(
                STATE_DESIGN,
                false,
                "Produce a technical design linked to accepted intent.",
            ),
            state(
                STATE_DESIGN_REVIEW,
                false,
                "Review the design and record an approving or revision verdict.",
            ),
            state(
                STATE_PLAN,
                false,
                "Produce an actionable implementation plan.",
            ),
            state(
                STATE_PLAN_REVIEW,
                false,
                "Review the plan for feasibility and alignment.",
            ),
            state(
                STATE_IMPLEMENT,
                false,
                "Carry out the plan and persist completion evidence.",
            ),
            state(
                STATE_IMPLEMENTATION_REVIEW,
                false,
                "Review implementation against intent, design, and plan.",
            ),
            state(
                STATE_VALIDATION,
                false,
                "Validate the completed result through provider-defined checks.",
            ),
            state(STATE_END, true, "Workflow completed successfully."),
        ],
        transitions: vec![
            transition(
                STATE_EXPLORE,
                "intent-ready",
                STATE_DESIGN,
                &[GATE_INTENT_READY],
            ),
            transition(
                STATE_DESIGN,
                "design-ready",
                STATE_DESIGN_REVIEW,
                &[GATE_DESIGN_READY],
            ),
            transition(
                STATE_DESIGN_REVIEW,
                "approved",
                STATE_PLAN,
                &[GATE_DESIGN_REVIEW_APPROVED],
            ),
            transition(
                STATE_DESIGN_REVIEW,
                "changes-requested",
                STATE_DESIGN,
                &[GATE_DESIGN_REVIEW_CHANGES],
            ),
            transition(
                STATE_PLAN,
                "plan-ready",
                STATE_PLAN_REVIEW,
                &[GATE_PLAN_READY],
            ),
            transition(
                STATE_PLAN_REVIEW,
                "approved",
                STATE_IMPLEMENT,
                &[GATE_PLAN_REVIEW_APPROVED],
            ),
            transition(
                STATE_PLAN_REVIEW,
                "changes-requested",
                STATE_PLAN,
                &[GATE_PLAN_REVIEW_CHANGES],
            ),
            transition(
                STATE_IMPLEMENT,
                "implementation-ready",
                STATE_IMPLEMENTATION_REVIEW,
                &[GATE_IMPLEMENTATION_READY],
            ),
            transition(
                STATE_IMPLEMENTATION_REVIEW,
                "approved",
                STATE_VALIDATION,
                &[GATE_IMPLEMENTATION_REVIEW_APPROVED],
            ),
            transition(
                STATE_IMPLEMENTATION_REVIEW,
                "changes-requested",
                STATE_IMPLEMENT,
                &[GATE_IMPLEMENTATION_REVIEW_CHANGES],
            ),
            transition(
                STATE_VALIDATION,
                "passed",
                STATE_END,
                &[GATE_VALIDATION_PASSED],
            ),
            transition(
                STATE_VALIDATION,
                "failed",
                STATE_IMPLEMENT,
                &[GATE_VALIDATION_FAILED],
            ),
        ],
        input_declarations: input_declarations(),
        live_guidance_supported: true,
        metadata: Some(metadata("reference-workflow", "1")),
    };

    if variant == DescribeGraphVariant::V2 {
        graph.metadata = Some(metadata("reference-workflow", "2"));
        graph.input_declarations.push(InputDeclarationDto {
            id: "policy_root".to_string(),
            kind: "path".to_string(),
            required: false,
            metadata: None,
        });
    }

    graph
}

fn state(id: &str, final_state: bool, guidance: &str) -> StateDto {
    StateDto {
        id: id.to_string(),
        final_state,
        static_guidance: StaticGuidanceDto::Declaration(StaticGuidanceDeclarationDto::Text {
            text: guidance.to_string(),
        }),
        metadata: None,
    }
}

fn transition(source: &str, event: &str, target: &str, gate_ids: &[&str]) -> TransitionDto {
    TransitionDto {
        source_state: source.to_string(),
        event: event.to_string(),
        target_state: target.to_string(),
        gate_ids: gate_ids.iter().map(|id| (*id).to_string()).collect(),
        metadata: None,
    }
}

fn input_declarations() -> Vec<InputDeclarationDto> {
    vec![
        InputDeclarationDto {
            id: "change_id".to_string(),
            kind: "string".to_string(),
            required: false,
            metadata: None,
        },
        InputDeclarationDto {
            id: "artifact_root".to_string(),
            kind: "path".to_string(),
            required: true,
            metadata: None,
        },
        InputDeclarationDto {
            id: "work_root".to_string(),
            kind: "path".to_string(),
            required: false,
            metadata: None,
        },
    ]
}

fn metadata(workflow: &str, version: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("workflow".to_string(), json!(workflow)),
        ("workflow_version".to_string(), json!(version)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn graph_matches_reference_workflow_topology() {
        let graph = build_graph(DescribeGraphVariant::V1);

        let state_ids: HashSet<_> = graph.states.iter().map(|s| s.id.as_str()).collect();
        let expected_states = [
            STATE_EXPLORE,
            STATE_DESIGN,
            STATE_DESIGN_REVIEW,
            STATE_PLAN,
            STATE_PLAN_REVIEW,
            STATE_IMPLEMENT,
            STATE_IMPLEMENTATION_REVIEW,
            STATE_VALIDATION,
            STATE_END,
        ];
        for state in expected_states {
            assert!(state_ids.contains(state), "missing state {state}");
        }

        assert_eq!(graph.initial_state, STATE_EXPLORE);
        assert!(graph.live_guidance_supported);
        assert!(
            graph
                .states
                .iter()
                .find(|s| s.id == STATE_END)
                .is_some_and(|s| s.final_state)
        );

        let edges: HashSet<_> = graph
            .transitions
            .iter()
            .map(|t| {
                (
                    t.source_state.as_str(),
                    t.event.as_str(),
                    t.target_state.as_str(),
                )
            })
            .collect();

        assert!(edges.contains(&(STATE_EXPLORE, "intent-ready", STATE_DESIGN)));
        assert!(edges.contains(&(STATE_DESIGN, "design-ready", STATE_DESIGN_REVIEW)));
        assert!(edges.contains(&(STATE_DESIGN_REVIEW, "approved", STATE_PLAN)));
        assert!(edges.contains(&(STATE_DESIGN_REVIEW, "changes-requested", STATE_DESIGN)));
        assert!(edges.contains(&(STATE_PLAN, "plan-ready", STATE_PLAN_REVIEW)));
        assert!(edges.contains(&(STATE_PLAN_REVIEW, "approved", STATE_IMPLEMENT)));
        assert!(edges.contains(&(STATE_PLAN_REVIEW, "changes-requested", STATE_PLAN)));
        assert!(edges.contains(&(
            STATE_IMPLEMENT,
            "implementation-ready",
            STATE_IMPLEMENTATION_REVIEW
        )));
        assert!(edges.contains(&(STATE_IMPLEMENTATION_REVIEW, "approved", STATE_VALIDATION)));
        assert!(edges.contains(&(
            STATE_IMPLEMENTATION_REVIEW,
            "changes-requested",
            STATE_IMPLEMENT
        )));
        assert!(edges.contains(&(STATE_VALIDATION, "passed", STATE_END)));
        assert!(edges.contains(&(STATE_VALIDATION, "failed", STATE_IMPLEMENT)));
    }

    #[test]
    fn v2_graph_adds_optional_input_without_mutating_v1() {
        let v1 = build_graph(DescribeGraphVariant::V1);
        let v2 = build_graph(DescribeGraphVariant::V2);

        assert_eq!(v1.states.len(), v2.states.len());
        assert_eq!(v1.transitions.len(), v2.transitions.len());
        assert_eq!(v2.input_declarations.len(), v1.input_declarations.len() + 1);
        assert!(v2.input_declarations.iter().any(|d| d.id == "policy_root"));
    }
}
