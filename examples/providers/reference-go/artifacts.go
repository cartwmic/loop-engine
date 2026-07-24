package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
)

type gatePayload struct {
	Snapshot         runSnapshot `json:"snapshot"`
	Event            string      `json:"event"`
	RequiredGateIDs  []string    `json:"required_gate_ids"`
	SelectedEvidence []evidence  `json:"selected_evidence"`
	InlineEvidence   []evidence  `json:"inline_evidence"`
}

type gateVerdict struct {
	GateID string `json:"gate_id"`
	Passed bool   `json:"passed"`
}

type artifactDoc struct {
	Revision        string `json:"revision"`
	IntentRevision  string `json:"intent_revision"`
	PlanRevision    string `json:"plan_revision"`
	SubjectRevision string `json:"subject_revision"`
	Verdict         string `json:"verdict"`
}

type artifactRule struct {
	RelativePath string
	Kind         string
}

var artifactRules = map[string]artifactRule{
	"intent-ready":                            {"intent.json", "intent-document"},
	"design-ready":                            {"design.json", "design-document"},
	"design-review-approved":                  {"reviews/design-review.json", "design-review"},
	"design-review-changes-requested":         {"reviews/design-review.json", "design-review"},
	"plan-ready":                              {"plan.json", "plan-document"},
	"plan-review-approved":                    {"reviews/plan-review.json", "plan-review"},
	"plan-review-changes-requested":           {"reviews/plan-review.json", "plan-review"},
	"implementation-ready":                    {"implementation.json", "implementation-report"},
	"implementation-review-approved":          {"reviews/implementation-review.json", "implementation-review"},
	"implementation-review-changes-requested": {"reviews/implementation-review.json", "implementation-review"},
	"validation-passed":                       {"validation.json", "validation-report"},
	"validation-failed":                       {"validation.json", "validation-report"},
}

func evaluateGates(raw json.RawMessage) (any, error) {
	var payload gatePayload
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, fmt.Errorf("decode evaluate_gates payload: %w", err)
	}
	unsupported := make([]string, 0)
	for _, gateID := range payload.RequiredGateIDs {
		if _, ok := supportedGates[gateID]; !ok {
			unsupported = append(unsupported, gateID)
		}
	}
	if len(unsupported) != 0 {
		return map[string]any{
			"kind": "incompatible",
			"diagnostics": []diagnostic{{
				Code: "compatibility.unsupported", Message: "stored graph requires unsupported gates: " + joinComma(unsupported),
			}},
		}, nil
	}

	root, ok := payload.Snapshot.Inputs["artifact_root"].(string)
	if !ok || root == "" {
		return failedVerdicts(payload.RequiredGateIDs), nil
	}

	existingIDs := map[string]struct{}{}
	for _, record := range append(payload.SelectedEvidence, payload.InlineEvidence...) {
		existingIDs[record.ID] = struct{}{}
	}
	verdicts := make([]gateVerdict, 0, len(payload.RequiredGateIDs))
	generated := make([]evidence, 0, len(payload.RequiredGateIDs))
	allPassed := true
	for _, gateID := range payload.RequiredGateIDs {
		record, err := evaluateGate(gateID, payload.Event, root, existingIDs)
		passed := err == nil
		verdicts = append(verdicts, gateVerdict{GateID: gateID, Passed: passed})
		if !passed {
			allPassed = false
			continue
		}
		existingIDs[record.ID] = struct{}{}
		generated = append(generated, record)
	}

	result := map[string]any{"kind": "verdicts", "verdicts": verdicts}
	if allPassed && len(generated) != 0 {
		result["evidence"] = generated
	}
	return result, nil
}

func failedVerdicts(gateIDs []string) map[string]any {
	verdicts := make([]gateVerdict, 0, len(gateIDs))
	for _, gateID := range gateIDs {
		verdicts = append(verdicts, gateVerdict{GateID: gateID, Passed: false})
	}
	return map[string]any{"kind": "verdicts", "verdicts": verdicts}
}

func evaluateGate(gateID, event, root string, existingIDs map[string]struct{}) (evidence, error) {
	rule := artifactRules[gateID]
	doc, raw, err := readArtifact(root, rule.RelativePath)
	if err != nil {
		return evidence{}, err
	}
	if doc.Revision == "" {
		return evidence{}, fmt.Errorf("%s: revision must not be empty", rule.RelativePath)
	}

	switch gateID {
	case "design-ready":
		intent, _, err := readArtifact(root, "intent.json")
		if err != nil || doc.IntentRevision != intent.Revision {
			return evidence{}, fmt.Errorf("design intent revision mismatch")
		}
	case "plan-ready":
		design, _, err := readArtifact(root, "design.json")
		if err != nil || doc.SubjectRevision != design.Revision {
			return evidence{}, fmt.Errorf("plan design revision mismatch")
		}
	case "implementation-ready":
		plan, _, err := readArtifact(root, "plan.json")
		if err != nil || doc.PlanRevision != plan.Revision {
			return evidence{}, fmt.Errorf("implementation plan revision mismatch")
		}
	case "design-review-approved":
		if err := requireReview(root, doc, "design.json", event, "approved"); err != nil {
			return evidence{}, err
		}
	case "design-review-changes-requested":
		if err := requireReview(root, doc, "design.json", event, "changes_requested"); err != nil {
			return evidence{}, err
		}
	case "plan-review-approved":
		if err := requireReview(root, doc, "plan.json", event, "approved"); err != nil {
			return evidence{}, err
		}
	case "plan-review-changes-requested":
		if err := requireReview(root, doc, "plan.json", event, "changes_requested"); err != nil {
			return evidence{}, err
		}
	case "implementation-review-approved":
		if err := requireReview(root, doc, "implementation.json", event, "approved"); err != nil {
			return evidence{}, err
		}
	case "implementation-review-changes-requested":
		if err := requireReview(root, doc, "implementation.json", event, "changes_requested"); err != nil {
			return evidence{}, err
		}
	case "validation-passed":
		if event != "passed" || doc.Verdict != "passed" {
			return evidence{}, fmt.Errorf("validation verdict mismatch")
		}
	case "validation-failed":
		if event != "failed" || doc.Verdict != "failed" {
			return evidence{}, fmt.Errorf("validation verdict mismatch")
		}
	}
	return artifactEvidence(root, rule, doc.Revision, raw, existingIDs)
}

func requireReview(root string, review artifactDoc, subjectPath, event, verdict string) error {
	subject, _, err := readArtifact(root, subjectPath)
	if err != nil {
		return err
	}
	expectedEvent := "approved"
	if verdict == "changes_requested" {
		expectedEvent = "changes-requested"
	}
	if event != expectedEvent || review.Verdict != verdict || review.SubjectRevision != subject.Revision {
		return fmt.Errorf("review verdict or subject revision mismatch")
	}
	return nil
}

func readArtifact(root, relativePath string) (artifactDoc, []byte, error) {
	path := filepath.Join(root, filepath.FromSlash(relativePath))
	raw, err := os.ReadFile(path)
	if err != nil {
		return artifactDoc{}, nil, err
	}
	var doc artifactDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return artifactDoc{}, nil, err
	}
	return doc, raw, nil
}

func artifactEvidence(root string, rule artifactRule, revision string, raw []byte, existingIDs map[string]struct{}) (evidence, error) {
	absolute, err := filepath.Abs(filepath.Join(root, filepath.FromSlash(rule.RelativePath)))
	if err != nil {
		return evidence{}, err
	}
	locator := (&url.URL{Scheme: "file", Path: absolute}).String()
	sum := sha256.Sum256(raw)
	digest := "sha256:" + hex.EncodeToString(sum[:])
	mediaType := "application/json"
	id := rule.Kind + "-" + revision
	if _, exists := existingIDs[id]; exists {
		suffix := sha256.Sum256([]byte(locator + ":" + revision))
		id += "-" + hex.EncodeToString(suffix[:4])
	}
	return evidence{
		ID: id, Kind: rule.Kind, Locator: locator, Digest: &digest, MediaType: &mediaType,
		Metadata: map[string]any{"revision": revision},
	}, nil
}
