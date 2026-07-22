pub(crate) mod graphs;
mod inputs;
pub(crate) mod payload;
pub(crate) mod process;
mod roles;

use serde_json::{Value, json};

use crate::protocol::{AnyRequest, AnyResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    GraphLinear,
    GraphCycle,
    GraphSelfLoop,
    GraphZeroFinal,
    GraphMultiFinal,
    GraphInitialFinal,
    GraphNonFinalSink,
    GraphAmbiguousDuplicateState,
    GraphAmbiguousDuplicateEvent,
    GraphStructurallyInvalid,
    GraphGuidanceSupported,
    GraphGuidanceUnsupported,
    GraphBuildDrift,
    InputRequiredAccepted,
    InputOptionalAccepted,
    InputRequiredRejected,
    InputInvalidRejected,
    InputEvaluationError,
    GatePass,
    GateFail,
    GateMixed,
    GateExactSetViolation,
    GateCallerEvidence,
    GateProviderEvidence,
    GateProviderEvidenceDuplicate,
    GateProviderEvidenceCollision,
    GateIncompatible,
    GateEvaluationError,
    GuidanceText,
    GuidanceIncompatible,
    GuidanceEvaluationError,
    CompatibilityAllCompatible,
    CompatibilityIncompatible,
    CompatibilityMixed,
    CompatibilityEvaluationError,
    ProcessMalformedJson,
    ProcessExtraStdout,
    ProcessMissingStdout,
    ProcessWrongMajor,
    ProcessNonzeroExit,
    ProcessSignal,
    ProcessTimeout,
    ProcessOversizedStdout,
    ProcessOversizedStderr,
    ProcessInvalidUtf8,
}

impl Scenario {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "graph-linear" => Self::GraphLinear,
            "graph-cycle" => Self::GraphCycle,
            "graph-self-loop" => Self::GraphSelfLoop,
            "graph-zero-final" => Self::GraphZeroFinal,
            "graph-multi-final" => Self::GraphMultiFinal,
            "graph-initial-final" => Self::GraphInitialFinal,
            "graph-non-final-sink" => Self::GraphNonFinalSink,
            "graph-ambiguous-duplicate-state" => Self::GraphAmbiguousDuplicateState,
            "graph-ambiguous-duplicate-event" => Self::GraphAmbiguousDuplicateEvent,
            "graph-structurally-invalid" => Self::GraphStructurallyInvalid,
            "graph-guidance-supported" => Self::GraphGuidanceSupported,
            "graph-guidance-unsupported" => Self::GraphGuidanceUnsupported,
            "graph-build-drift" => Self::GraphBuildDrift,
            "input-required-accepted" => Self::InputRequiredAccepted,
            "input-optional-accepted" => Self::InputOptionalAccepted,
            "input-required-rejected" => Self::InputRequiredRejected,
            "input-invalid-rejected" => Self::InputInvalidRejected,
            "input-evaluation-error" => Self::InputEvaluationError,
            "gate-pass" => Self::GatePass,
            "gate-fail" => Self::GateFail,
            "gate-mixed" => Self::GateMixed,
            "gate-exact-set-violation" => Self::GateExactSetViolation,
            "gate-caller-evidence" => Self::GateCallerEvidence,
            "gate-provider-evidence" => Self::GateProviderEvidence,
            "gate-provider-evidence-duplicate" => Self::GateProviderEvidenceDuplicate,
            "gate-provider-evidence-collision" => Self::GateProviderEvidenceCollision,
            "gate-incompatible" => Self::GateIncompatible,
            "gate-evaluation-error" => Self::GateEvaluationError,
            "guidance-text" => Self::GuidanceText,
            "guidance-incompatible" => Self::GuidanceIncompatible,
            "guidance-evaluation-error" => Self::GuidanceEvaluationError,
            "compatibility-all-compatible" => Self::CompatibilityAllCompatible,
            "compatibility-incompatible" => Self::CompatibilityIncompatible,
            "compatibility-mixed" => Self::CompatibilityMixed,
            "compatibility-evaluation-error" => Self::CompatibilityEvaluationError,
            "process-malformed-json" => Self::ProcessMalformedJson,
            "process-extra-stdout" => Self::ProcessExtraStdout,
            "process-missing-stdout" => Self::ProcessMissingStdout,
            "process-wrong-major" => Self::ProcessWrongMajor,
            "process-nonzero-exit" => Self::ProcessNonzeroExit,
            "process-signal" => Self::ProcessSignal,
            "process-timeout" => Self::ProcessTimeout,
            "process-oversized-stdout" => Self::ProcessOversizedStdout,
            "process-oversized-stderr" => Self::ProcessOversizedStderr,
            "process-invalid-utf8" => Self::ProcessInvalidUtf8,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraphLinear => "graph-linear",
            Self::GraphCycle => "graph-cycle",
            Self::GraphSelfLoop => "graph-self-loop",
            Self::GraphZeroFinal => "graph-zero-final",
            Self::GraphMultiFinal => "graph-multi-final",
            Self::GraphInitialFinal => "graph-initial-final",
            Self::GraphNonFinalSink => "graph-non-final-sink",
            Self::GraphAmbiguousDuplicateState => "graph-ambiguous-duplicate-state",
            Self::GraphAmbiguousDuplicateEvent => "graph-ambiguous-duplicate-event",
            Self::GraphStructurallyInvalid => "graph-structurally-invalid",
            Self::GraphGuidanceSupported => "graph-guidance-supported",
            Self::GraphGuidanceUnsupported => "graph-guidance-unsupported",
            Self::GraphBuildDrift => "graph-build-drift",
            Self::InputRequiredAccepted => "input-required-accepted",
            Self::InputOptionalAccepted => "input-optional-accepted",
            Self::InputRequiredRejected => "input-required-rejected",
            Self::InputInvalidRejected => "input-invalid-rejected",
            Self::InputEvaluationError => "input-evaluation-error",
            Self::GatePass => "gate-pass",
            Self::GateFail => "gate-fail",
            Self::GateMixed => "gate-mixed",
            Self::GateExactSetViolation => "gate-exact-set-violation",
            Self::GateCallerEvidence => "gate-caller-evidence",
            Self::GateProviderEvidence => "gate-provider-evidence",
            Self::GateProviderEvidenceDuplicate => "gate-provider-evidence-duplicate",
            Self::GateProviderEvidenceCollision => "gate-provider-evidence-collision",
            Self::GateIncompatible => "gate-incompatible",
            Self::GateEvaluationError => "gate-evaluation-error",
            Self::GuidanceText => "guidance-text",
            Self::GuidanceIncompatible => "guidance-incompatible",
            Self::GuidanceEvaluationError => "guidance-evaluation-error",
            Self::CompatibilityAllCompatible => "compatibility-all-compatible",
            Self::CompatibilityIncompatible => "compatibility-incompatible",
            Self::CompatibilityMixed => "compatibility-mixed",
            Self::CompatibilityEvaluationError => "compatibility-evaluation-error",
            Self::ProcessMalformedJson => "process-malformed-json",
            Self::ProcessExtraStdout => "process-extra-stdout",
            Self::ProcessMissingStdout => "process-missing-stdout",
            Self::ProcessWrongMajor => "process-wrong-major",
            Self::ProcessNonzeroExit => "process-nonzero-exit",
            Self::ProcessSignal => "process-signal",
            Self::ProcessTimeout => "process-timeout",
            Self::ProcessOversizedStdout => "process-oversized-stdout",
            Self::ProcessOversizedStderr => "process-oversized-stderr",
            Self::ProcessInvalidUtf8 => "process-invalid-utf8",
        }
    }

    pub fn is_process_failure(self) -> bool {
        matches!(
            self,
            Self::ProcessMalformedJson
                | Self::ProcessExtraStdout
                | Self::ProcessMissingStdout
                | Self::ProcessWrongMajor
                | Self::ProcessNonzeroExit
                | Self::ProcessSignal
                | Self::ProcessTimeout
                | Self::ProcessOversizedStdout
                | Self::ProcessOversizedStderr
                | Self::ProcessInvalidUtf8
        )
    }

    pub fn ledger_facts(self, invocation_ordinal: Option<u64>) -> (Option<String>, Option<Value>) {
        let ordinal = invocation_ordinal.unwrap_or(1);
        match self {
            Self::GraphLinear => (
                Some("graph".into()),
                Some(json!({"shape": "linear", "ordinal": ordinal})),
            ),
            Self::GraphBuildDrift => (
                Some("paired_call".into()),
                Some(json!({
                    "paired_call": "graph_build_drift",
                    "ordinal": ordinal,
                    "graph_build": if ordinal <= 1 { "a" } else { "b" },
                })),
            ),
            Self::ProcessWrongMajor => (None, Some(json!({"transport": "wrong_major"}))),
            Self::GateProviderEvidenceDuplicate => (
                Some("provider_evidence".into()),
                Some(json!({"fault": "duplicate_id"})),
            ),
            Self::GateProviderEvidenceCollision => (
                Some("provider_evidence".into()),
                Some(json!({"fault": "caller_collision"})),
            ),
            _ => (None, None),
        }
    }

    pub fn handle(self, request: &AnyRequest) -> AnyResult {
        payload::handle_request(self, request, None)
    }
}

pub fn all_scenario_names() -> &'static [&'static str] {
    &[
        "graph-linear",
        "graph-cycle",
        "graph-self-loop",
        "graph-zero-final",
        "graph-multi-final",
        "graph-initial-final",
        "graph-non-final-sink",
        "graph-ambiguous-duplicate-state",
        "graph-ambiguous-duplicate-event",
        "graph-structurally-invalid",
        "graph-guidance-supported",
        "graph-guidance-unsupported",
        "graph-build-drift",
        "input-required-accepted",
        "input-optional-accepted",
        "input-required-rejected",
        "input-invalid-rejected",
        "input-evaluation-error",
        "gate-pass",
        "gate-fail",
        "gate-mixed",
        "gate-exact-set-violation",
        "gate-caller-evidence",
        "gate-provider-evidence",
        "gate-provider-evidence-duplicate",
        "gate-provider-evidence-collision",
        "gate-incompatible",
        "gate-evaluation-error",
        "guidance-text",
        "guidance-incompatible",
        "guidance-evaluation-error",
        "compatibility-all-compatible",
        "compatibility-incompatible",
        "compatibility-mixed",
        "compatibility-evaluation-error",
        "process-malformed-json",
        "process-extra-stdout",
        "process-missing-stdout",
        "process-wrong-major",
        "process-nonzero-exit",
        "process-signal",
        "process-timeout",
        "process-oversized-stdout",
        "process-oversized-stderr",
        "process-invalid-utf8",
    ]
}

pub fn scenario_fixture_category(name: &str) -> Option<&'static str> {
    if name.starts_with("graph-") {
        Some("graphs")
    } else if name.starts_with("input-") {
        Some("inputs")
    } else if name.starts_with("process-") {
        Some("process")
    } else if name.starts_with("gate-")
        || name.starts_with("guidance-")
        || name.starts_with("compatibility-")
    {
        Some("roles")
    } else {
        None
    }
}
