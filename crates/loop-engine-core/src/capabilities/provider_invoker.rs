use crate::capabilities::provider_catalog::ResolvedProviderConfig;
use crate::model::attempt::ProviderFact;
use crate::model::bounded::{
    BoundError, INLINE_EVIDENCE_CONTEXT_TOTAL_BYTES, PROVIDER_SNAPSHOT_ENVELOPE_BYTES,
    SELECTED_EVIDENCE_CONTEXT_TOTAL_BYTES,
};
use crate::model::compatibility::CompatibilityReport;
use crate::model::diagnostic::Diagnostics;
use crate::model::evidence::EvidenceRecord;
use crate::model::gate::GateEvaluation;
use crate::model::graph::WorkflowGraph;
use crate::model::graph_validation::GraphError;
use crate::model::ids::{EventId, RequestId, RunId};
use crate::model::live_guidance::LiveGuidanceResult;
use crate::model::provider::ProviderObservation;
use crate::model::run::Run;
use crate::model::run_input::{InputDeclarations, RunInputs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationError<E> {
    /// Trace budget could not reserve a provider boundary; process was not launched.
    TraceBudgetUnavailable,
    /// Provider process was attempted; observation remains durable audit input.
    Transport {
        source: E,
        fact: Box<ProviderFact>,
        /// Diagnostic sink failure after provider dispatch; provider outcome remains authoritative.
        trace_failure: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeRequest {
    pub request_id: RequestId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribedGraph {
    /// Raw mapped declaration. Each operation applies its own conformance validation.
    Declared(WorkflowGraph),
    /// Structurally invalid declaration that cannot be represented by core graph types.
    Invalid(GraphError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeResult {
    pub graph: DescribedGraph,
    /// Creation-time locator/digest snapshot retained with the run.
    pub observation: ProviderObservation,
    /// Exact attempted-invocation fact retained in the journal.
    pub fact: ProviderFact,
    pub protocol_major: u32,
    /// Post-initialization diagnostic sink failure; never changes provider semantics.
    pub trace_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidateInputsRequest {
    pub request_id: RequestId,
    /// Exact declarations returned by the preceding successful `describe` call.
    pub input_declarations: InputDeclarations,
    pub inputs: RunInputs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputValidationResult {
    Accepted,
    Rejected(Diagnostics),
    EvaluationError(Diagnostics),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputValidationInvocationResult {
    pub result: InputValidationResult,
    pub fact: ProviderFact,
    pub trace_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceContext {
    records: Vec<EvidenceRecord>,
    encoded_bytes: usize,
}

impl EvidenceContext {
    pub fn new(
        field: &'static str,
        records: Vec<EvidenceRecord>,
        encoded_bytes: usize,
    ) -> Result<Self, BoundError> {
        if (records.is_empty() && encoded_bytes != 0) || (!records.is_empty() && encoded_bytes == 0)
        {
            return Err(BoundError::InvalidType { field });
        }
        let max = match field {
            "selected_evidence" => SELECTED_EVIDENCE_CONTEXT_TOTAL_BYTES,
            _ => INLINE_EVIDENCE_CONTEXT_TOTAL_BYTES,
        };
        if encoded_bytes > max {
            return Err(BoundError::EncodedTooLarge {
                field,
                max,
                actual: encoded_bytes,
            });
        }
        Ok(Self {
            records,
            encoded_bytes,
        })
    }

    pub fn records(&self) -> &[EvidenceRecord] {
        &self.records
    }

    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRunSnapshot {
    run: Run,
    encoded_bytes: usize,
}

impl ProviderRunSnapshot {
    pub fn new(run: Run, encoded_bytes: usize) -> Result<Self, BoundError> {
        if encoded_bytes == 0 {
            return Err(BoundError::InvalidType {
                field: "provider_snapshot",
            });
        }
        if encoded_bytes > PROVIDER_SNAPSHOT_ENVELOPE_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "provider_snapshot",
                max: PROVIDER_SNAPSHOT_ENVELOPE_BYTES,
                actual: encoded_bytes,
            });
        }
        Ok(Self { run, encoded_bytes })
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRequest {
    pub request_id: RequestId,
    pub run: ProviderRunSnapshot,
    pub event: EventId,
    pub selected_evidence: EvidenceContext,
    pub inline_evidence: EvidenceContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run: ProviderRunSnapshot,
    pub selected_evidence: EvidenceContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run: ProviderRunSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateInvocationResult {
    pub evaluation: GateEvaluation,
    pub fact: ProviderFact,
    pub trace_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceInvocationResult {
    pub result: LiveGuidanceResult,
    pub fact: ProviderFact,
    pub trace_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityResult {
    pub report: CompatibilityReport,
    pub observation: ProviderObservation,
    pub fact: ProviderFact,
    pub protocol_major: u32,
    pub trace_failure: Option<String>,
}

pub trait ProviderInvoker {
    type TransportError;

    fn describe(
        &self,
        config: &ResolvedProviderConfig,
        request: DescribeRequest,
    ) -> Result<DescribeResult, InvocationError<Self::TransportError>>;

    fn validate_inputs(
        &self,
        config: &ResolvedProviderConfig,
        request: ValidateInputsRequest,
    ) -> Result<InputValidationInvocationResult, InvocationError<Self::TransportError>>;

    fn evaluate_gates(
        &self,
        config: &ResolvedProviderConfig,
        request: GateRequest,
    ) -> Result<GateInvocationResult, InvocationError<Self::TransportError>>;

    fn live_guidance(
        &self,
        config: &ResolvedProviderConfig,
        request: GuidanceRequest,
    ) -> Result<GuidanceInvocationResult, InvocationError<Self::TransportError>>;

    fn check_compatibility(
        &self,
        config: &ResolvedProviderConfig,
        request: CompatibilityRequest,
    ) -> Result<CompatibilityResult, InvocationError<Self::TransportError>>;
}

#[cfg(test)]
mod tests {
    use super::EvidenceContext;

    #[test]
    fn evidence_context_requires_exact_shape_and_bound() {
        assert!(EvidenceContext::new("selected_evidence", vec![], 0).is_ok());
        assert!(EvidenceContext::new("selected_evidence", vec![], 1).is_err());
    }
}
