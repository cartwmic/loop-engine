package main

import (
	"encoding/json"
	"fmt"
	"sort"
)

func handleRequest(request requestEnvelope) (resultEnvelope, error) {
	if request.ProtocolMajor != protocolMajor {
		return resultEnvelope{}, fmt.Errorf("unsupported protocol_major %d", request.ProtocolMajor)
	}
	if request.InvocationID == "" {
		return resultEnvelope{}, fmt.Errorf("missing invocation_id")
	}

	var result any
	var err error
	switch request.Role {
	case "describe":
		result = map[string]any{"kind": "description", "graph": referenceGraph()}
	case "validate_inputs":
		result, err = validateInputs(request.Payload)
	case "evaluate_gates":
		result, err = evaluateGates(request.Payload)
	case "live_guidance":
		result, err = liveGuidance(request.Payload)
	case "check_compatibility":
		result, err = checkCompatibility(request.Payload)
	default:
		err = fmt.Errorf("unsupported role %q", request.Role)
	}
	if err != nil {
		return resultEnvelope{}, err
	}

	return resultEnvelope{
		ProtocolMajor:   protocolMajor,
		Role:            request.Role,
		InvocationID:    request.InvocationID,
		ProviderVersion: providerVersion,
		Result:          result,
	}, nil
}

func validateInputs(raw json.RawMessage) (any, error) {
	var payload struct {
		Declarations    []inputDeclaration `json:"declarations"`
		CandidateValues map[string]any     `json:"candidate_values"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, fmt.Errorf("decode validate_inputs payload: %w", err)
	}

	var diagnostics []diagnostic
	for _, declaration := range payload.Declarations {
		value, exists := payload.CandidateValues[declaration.ID]
		path := "/candidate_values/" + declaration.ID
		if declaration.Required && !exists {
			diagnostics = append(diagnostics, diagnostic{
				Code: "input.required", Message: "missing required input " + declaration.ID, Path: &path,
			})
			continue
		}
		if !exists {
			continue
		}
		text, ok := value.(string)
		if !ok {
			diagnostics = append(diagnostics, diagnostic{
				Code: "input.type", Message: "input " + declaration.ID + " must be a string", Path: &path,
			})
			continue
		}
		if declaration.Required && text == "" {
			diagnostics = append(diagnostics, diagnostic{
				Code: "input.empty", Message: "input " + declaration.ID + " must not be empty", Path: &path,
			})
		}
	}
	if len(diagnostics) != 0 {
		return map[string]any{"kind": "rejected", "diagnostics": diagnostics}, nil
	}
	return map[string]any{"kind": "accepted", "values": payload.CandidateValues}, nil
}

func liveGuidance(raw json.RawMessage) (any, error) {
	var payload struct {
		Snapshot runSnapshot `json:"snapshot"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, fmt.Errorf("decode live_guidance payload: %w", err)
	}
	guidance := map[string]string{
		"explore":               "Capture intent in artifact_root/intent.json, then request intent-ready.",
		"design":                "Author design.json linked to accepted intent revision, then request design-ready.",
		"design-review":         "Record reviews/design-review.json for current design revision, then request approved or changes-requested.",
		"plan":                  "Author plan.json linked to approved design revision, then request plan-ready.",
		"plan-review":           "Record reviews/plan-review.json for current plan revision, then request approved or changes-requested.",
		"implement":             "Persist implementation.json linked to approved plan revision, then request implementation-ready.",
		"implementation-review": "Record reviews/implementation-review.json for current implementation revision.",
		"validation":            "Record validation.json with verdict matching passed or failed event.",
		"end":                   "No further work remains.",
	}
	text, exists := guidance[payload.Snapshot.CurrentState]
	if !exists {
		text = "Continue using stored static guidance for state " + payload.Snapshot.CurrentState + "."
	}
	return map[string]any{"kind": "guidance", "text": text}, nil
}

func checkCompatibility(raw json.RawMessage) (any, error) {
	var payload struct {
		StoredGraph  canonicalGraph `json:"stored_graph"`
		Capabilities *[]string      `json:"capabilities"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, fmt.Errorf("decode check_compatibility payload: %w", err)
	}
	capabilities := []string{"evaluate_gates", "live_guidance"}
	if payload.Capabilities != nil {
		capabilities = *payload.Capabilities
	}

	findings := make([]map[string]any, 0, len(capabilities))
	for _, capability := range capabilities {
		switch capability {
		case "evaluate_gates":
			unsupported := unsupportedGraphGates(payload.StoredGraph)
			if len(unsupported) == 0 {
				findings = append(findings, compatibilityFinding(capability, "compatible", nil))
			} else {
				message := "stored graph requires unsupported gates: " + joinComma(unsupported)
				findings = append(findings, compatibilityFinding(capability, "incompatible", []diagnostic{{Code: "compatibility.unsupported", Message: message}}))
			}
		case "live_guidance":
			if payload.StoredGraph.LiveGuidanceSupported {
				findings = append(findings, compatibilityFinding(capability, "compatible", nil))
			} else {
				findings = append(findings, compatibilityFinding(capability, "incompatible", []diagnostic{{Code: "compatibility.unsupported", Message: "stored graph declares live guidance unsupported"}}))
			}
		default:
			findings = append(findings, compatibilityFinding(capability, "unknown", []diagnostic{{Code: "compatibility.unknown", Message: "capability " + capability + " is not evaluated by this provider"}}))
		}
	}
	return map[string]any{"kind": "findings", "capabilities": findings}, nil
}

func compatibilityFinding(capability, status string, diagnostics []diagnostic) map[string]any {
	if diagnostics == nil {
		diagnostics = []diagnostic{}
	}
	return map[string]any{"capability": capability, "status": status, "diagnostics": diagnostics}
}

func unsupportedGraphGates(stored canonicalGraph) []string {
	set := map[string]struct{}{}
	for _, transition := range stored.Transitions {
		for _, gateID := range transition.GateIDs {
			if _, ok := supportedGates[gateID]; !ok {
				set[gateID] = struct{}{}
			}
		}
	}
	result := make([]string, 0, len(set))
	for gateID := range set {
		result = append(result, gateID)
	}
	sort.Strings(result)
	return result
}

func joinComma(values []string) string {
	result := ""
	for i, value := range values {
		if i != 0 {
			result += ", "
		}
		result += value
	}
	return result
}
