package main

import (
	"bytes"
	"encoding/json"
	"path/filepath"
	"testing"
)

func TestReferenceGraphTopology(t *testing.T) {
	graph := referenceGraph()
	if graph.InitialState != "explore" {
		t.Fatalf("initial state = %q", graph.InitialState)
	}
	if len(graph.States) != 9 || len(graph.Transitions) != 12 {
		t.Fatalf("got %d states and %d transitions", len(graph.States), len(graph.Transitions))
	}
	if !graph.States[len(graph.States)-1].Final || graph.States[len(graph.States)-1].ID != "end" {
		t.Fatal("end must be final")
	}
}

func TestValidateInputsRejectsMissingArtifactRoot(t *testing.T) {
	payload, err := json.Marshal(map[string]any{
		"declarations":     referenceGraph().InputDeclarations,
		"candidate_values": map[string]any{},
	})
	if err != nil {
		t.Fatal(err)
	}
	result, err := validateInputs(payload)
	if err != nil {
		t.Fatal(err)
	}
	object := result.(map[string]any)
	if object["kind"] != "rejected" {
		t.Fatalf("result = %#v", result)
	}
}

func TestHappyPathIntentGate(t *testing.T) {
	root, err := filepath.Abs("fixtures/artifacts/happy-path")
	if err != nil {
		t.Fatal(err)
	}
	payload, err := json.Marshal(gatePayload{
		Snapshot: runSnapshot{Inputs: map[string]any{"artifact_root": root}},
		Event:    "intent-ready", RequiredGateIDs: []string{"intent-ready"},
		SelectedEvidence: []evidence{}, InlineEvidence: []evidence{},
	})
	if err != nil {
		t.Fatal(err)
	}
	result, err := evaluateGates(payload)
	if err != nil {
		t.Fatal(err)
	}
	object := result.(map[string]any)
	verdicts := object["verdicts"].([]gateVerdict)
	if len(verdicts) != 1 || !verdicts[0].Passed {
		t.Fatalf("verdicts = %#v", verdicts)
	}
}

func TestTransportCorrelatesEnvelope(t *testing.T) {
	request := `{"protocol_major":1,"role":"describe","invocation_id":"inv-1","registration":{"registration_id":"reg-1","config_revision":1,"executable":"provider","argv":[],"working_directory":".","timeout_seconds":5},"payload":{}}`
	var output bytes.Buffer
	if err := run(bytes.NewBufferString(request), &output); err != nil {
		t.Fatal(err)
	}
	var response resultEnvelope
	if err := json.Unmarshal(output.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if response.InvocationID != "inv-1" || response.Role != "describe" || response.ProtocolMajor != 1 {
		t.Fatalf("response = %#v", response)
	}
}

func TestTransportRejectsTrailingValue(t *testing.T) {
	request := `{"protocol_major":1,"role":"describe","invocation_id":"inv-1","registration":{},"payload":{}} {}`
	if err := run(bytes.NewBufferString(request), &bytes.Buffer{}); err == nil {
		t.Fatal("expected trailing-value rejection")
	}
}

func TestTransportRejectsNestedDuplicateKey(t *testing.T) {
	request := `{"protocol_major":1,"role":"describe","invocation_id":"inv-1","registration":{},"payload":{"x":1,"x":2}}`
	if err := run(bytes.NewBufferString(request), &bytes.Buffer{}); err == nil {
		t.Fatal("expected duplicate-key rejection")
	}
}
