package main

var supportedGates = map[string]struct{}{
	"intent-ready":                            {},
	"design-ready":                            {},
	"design-review-approved":                  {},
	"design-review-changes-requested":         {},
	"plan-ready":                              {},
	"plan-review-approved":                    {},
	"plan-review-changes-requested":           {},
	"implementation-ready":                    {},
	"implementation-review-approved":          {},
	"implementation-review-changes-requested": {},
	"validation-passed":                       {},
	"validation-failed":                       {},
}

func referenceGraph() graph {
	return graph{
		InitialState: "explore",
		States: []state{
			workflowState("explore", false, "Explore change context and capture intent."),
			workflowState("design", false, "Produce technical design linked to accepted intent."),
			workflowState("design-review", false, "Review design and record approving or revision verdict."),
			workflowState("plan", false, "Produce actionable implementation plan."),
			workflowState("plan-review", false, "Review plan for feasibility and alignment."),
			workflowState("implement", false, "Carry out plan and persist completion evidence."),
			workflowState("implementation-review", false, "Review implementation against intent, design, and plan."),
			workflowState("validation", false, "Validate completed result through provider-defined checks."),
			workflowState("end", true, "Workflow completed successfully."),
		},
		Transitions: []transition{
			workflowTransition("explore", "intent-ready", "design", "intent-ready"),
			workflowTransition("design", "design-ready", "design-review", "design-ready"),
			workflowTransition("design-review", "approved", "plan", "design-review-approved"),
			workflowTransition("design-review", "changes-requested", "design", "design-review-changes-requested"),
			workflowTransition("plan", "plan-ready", "plan-review", "plan-ready"),
			workflowTransition("plan-review", "approved", "implement", "plan-review-approved"),
			workflowTransition("plan-review", "changes-requested", "plan", "plan-review-changes-requested"),
			workflowTransition("implement", "implementation-ready", "implementation-review", "implementation-ready"),
			workflowTransition("implementation-review", "approved", "validation", "implementation-review-approved"),
			workflowTransition("implementation-review", "changes-requested", "implement", "implementation-review-changes-requested"),
			workflowTransition("validation", "passed", "end", "validation-passed"),
			workflowTransition("validation", "failed", "implement", "validation-failed"),
		},
		InputDeclarations: []inputDeclaration{
			{ID: "change_id", Kind: "string", Required: false},
			{ID: "artifact_root", Kind: "path", Required: true},
			{ID: "work_root", Kind: "path", Required: false},
		},
		LiveGuidanceSupported: true,
		Metadata: map[string]any{
			"workflow":         "reference-workflow",
			"workflow_version": "1",
		},
	}
}

func workflowState(id string, final bool, guidance string) state {
	return state{
		ID:             id,
		Final:          final,
		StaticGuidance: map[string]any{"kind": "text", "text": guidance},
	}
}

func workflowTransition(source, event, target, gateID string) transition {
	return transition{
		SourceState: source,
		Event:       event,
		TargetState: target,
		GateIDs:     []string{gateID},
	}
}
