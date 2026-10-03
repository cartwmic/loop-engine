//! Software-change provider policy and non-executing preparation helpers.

mod config;
mod criterion;
mod overlay;
mod recovery_contract;
mod schema;
#[allow(dead_code)]
mod workflow;

pub mod advice;
pub mod author_counts;
pub mod commission;
pub mod embedded_data;
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
