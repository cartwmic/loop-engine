//! Software-change provider policy and non-executing preparation helpers.

// The binary and library reuse the same private parser implementations.
extern crate self as software_change_provider;
mod artifacts;
#[allow(dead_code)]
mod checkpoint;
mod config;
mod criterion;
mod evidence;
#[allow(dead_code)]
mod finding_ledger;
mod narratives;
mod overlay;
#[allow(dead_code)]
mod protocol;
mod record_facts;
pub mod recovery;
mod recovery_contract;
#[allow(dead_code)]
mod review_batch;
mod schema;
#[allow(dead_code)]
mod source_identity;
mod source_scopes;
#[allow(dead_code)]
mod validation;
#[allow(dead_code)]
mod workflow;

pub mod admission;
pub mod advice;
pub mod author_counts;
pub mod challenge_admission;
pub mod commission;
pub mod embedded_data;
pub mod intent_comparison;
pub mod intent_correction;
#[cfg(test)]
mod intent_correction_tests;
pub mod setup;

/// Validate one shipped initial-input template with production config rules.
#[doc(hidden)]
pub fn validate_config_for_tests(initial_input: &serde_json::Value) -> Result<(), String> {
    config::validate_config(initial_input)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Evaluate-time overlay clone used by overlay injection tests.
#[doc(hidden)]
pub fn apply_bookends_overlay_for_tests(initial_input: &serde_json::Value) -> serde_json::Value {
    if overlay::enabled(initial_input) {
        overlay::apply(initial_input)
    } else {
        initial_input.clone()
    }
}
