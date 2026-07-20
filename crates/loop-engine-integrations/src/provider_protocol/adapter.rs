use std::sync::{Arc, Mutex};

use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::capabilities::provider_invoker::{
    CompatibilityRequest, CompatibilityResult, DescribeRequest, DescribeResult,
    GateInvocationResult, GateRequest, GuidanceInvocationResult, GuidanceRequest,
    InputValidationInvocationResult, InvocationError, ProviderInvoker, ValidateInputsRequest,
};

use super::compatibility;
use super::describe;
use super::evaluate_gates;
use super::invoke::AdapterError;
use super::live_guidance;
use super::validate_inputs;
use crate::provider_process::TracedProviderBoundary;
use crate::trace::TraceWriter;

#[derive(Clone)]
pub struct SubprocessProviderInvoker {
    boundary: TracedProviderBoundary,
}

impl SubprocessProviderInvoker {
    pub fn new(trace: Arc<Mutex<TraceWriter>>) -> Self {
        Self {
            boundary: TracedProviderBoundary::new(trace),
        }
    }
}

impl ProviderInvoker for SubprocessProviderInvoker {
    type TransportError = AdapterError;

    fn describe(
        &self,
        config: &ResolvedProviderConfig,
        request: DescribeRequest,
    ) -> Result<DescribeResult, InvocationError<Self::TransportError>> {
        describe::describe(&self.boundary, config, request)
    }

    fn validate_inputs(
        &self,
        config: &ResolvedProviderConfig,
        request: ValidateInputsRequest,
    ) -> Result<InputValidationInvocationResult, InvocationError<Self::TransportError>> {
        validate_inputs::validate_inputs(&self.boundary, config, request)
    }

    fn evaluate_gates(
        &self,
        config: &ResolvedProviderConfig,
        request: GateRequest,
    ) -> Result<GateInvocationResult, InvocationError<Self::TransportError>> {
        evaluate_gates::evaluate_gates(&self.boundary, config, request)
    }

    fn live_guidance(
        &self,
        config: &ResolvedProviderConfig,
        request: GuidanceRequest,
    ) -> Result<GuidanceInvocationResult, InvocationError<Self::TransportError>> {
        live_guidance::live_guidance(&self.boundary, config, request)
    }

    fn check_compatibility(
        &self,
        config: &ResolvedProviderConfig,
        request: CompatibilityRequest,
    ) -> Result<CompatibilityResult, InvocationError<Self::TransportError>> {
        compatibility::check_compatibility(&self.boundary, config, request)
    }
}
