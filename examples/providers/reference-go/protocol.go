package main

import "encoding/json"

const (
	protocolMajor   = 1
	providerVersion = "reference-go/1.0.0"
)

type requestEnvelope struct {
	ProtocolMajor int             `json:"protocol_major"`
	Role          string          `json:"role"`
	InvocationID  string          `json:"invocation_id"`
	Registration  registration    `json:"registration"`
	Payload       json.RawMessage `json:"payload"`
}

type registration struct {
	RegistrationID   string   `json:"registration_id"`
	ConfigRevision   uint64   `json:"config_revision"`
	Executable       string   `json:"executable"`
	Argv             []string `json:"argv"`
	WorkingDirectory string   `json:"working_directory"`
	TimeoutSeconds   uint64   `json:"timeout_seconds"`
}

type resultEnvelope struct {
	ProtocolMajor   int    `json:"protocol_major"`
	Role            string `json:"role"`
	InvocationID    string `json:"invocation_id"`
	ProviderVersion string `json:"provider_version"`
	Result          any    `json:"result"`
}

type diagnostic struct {
	Code    string  `json:"code"`
	Message string  `json:"message"`
	Path    *string `json:"path,omitempty"`
}

type inputDeclaration struct {
	ID       string         `json:"id"`
	Kind     string         `json:"kind"`
	Required bool           `json:"required"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

type state struct {
	ID             string         `json:"id"`
	Final          bool           `json:"final"`
	StaticGuidance map[string]any `json:"static_guidance"`
	Metadata       map[string]any `json:"metadata,omitempty"`
}

type transition struct {
	SourceState string         `json:"source_state"`
	Event       string         `json:"event"`
	TargetState string         `json:"target_state"`
	GateIDs     []string       `json:"gate_ids"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type graph struct {
	InitialState          string             `json:"initial_state"`
	States                []state            `json:"states"`
	Transitions           []transition       `json:"transitions"`
	InputDeclarations     []inputDeclaration `json:"input_declarations"`
	LiveGuidanceSupported bool               `json:"live_guidance_supported"`
	Metadata              map[string]any     `json:"metadata,omitempty"`
}

type canonicalTransition struct {
	SourceStateID string   `json:"source_state_id"`
	EventID       string   `json:"event_id"`
	TargetStateID string   `json:"target_state_id"`
	GateIDs       []string `json:"gate_ids"`
}

type canonicalGraph struct {
	LiveGuidanceSupported bool                  `json:"live_guidance_supported"`
	Transitions           []canonicalTransition `json:"transitions"`
}

type evidence struct {
	ID         string         `json:"id"`
	Kind       string         `json:"kind"`
	Locator    string         `json:"locator"`
	Digest     *string        `json:"digest,omitempty"`
	MediaType  *string        `json:"media_type,omitempty"`
	Metadata   map[string]any `json:"metadata,omitempty"`
	ObservedAt *string        `json:"observed_at,omitempty"`
}

type runSnapshot struct {
	RunID                string         `json:"run_id"`
	RegistrationID       string         `json:"registration_id"`
	GraphRevision        string         `json:"graph_revision"`
	Lifecycle            string         `json:"lifecycle"`
	CurrentState         string         `json:"current_state"`
	WorkflowStateVersion uint64         `json:"workflow_state_version"`
	LifecycleVersion     uint64         `json:"lifecycle_version"`
	Inputs               map[string]any `json:"inputs"`
	StoredGraph          canonicalGraph `json:"stored_graph"`
}
