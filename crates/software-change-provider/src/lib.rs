//! Small library surface used by shipped-data acceptance tests and locator writers.
//!
//! Provider execution remains owned by the binary.  This library exposes config
//! meta-validation so tests can validate shipped initial-input files through the
//! production validator, plus the duplicated PATH `dagu` resolver and locator
//! writers used by `run-plan-graph` tests. The crate does not vendor or ship dagu.

mod config;
mod dagu;
mod overlay;
mod schema;

pub mod embedded_data;
pub mod review_candidates;

pub use dagu::{names_for_capture_root, resolve_dagu, write_locator, DaguError, DaguLocator};

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
