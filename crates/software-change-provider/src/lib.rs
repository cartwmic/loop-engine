//! Small library surface used by shipped-data acceptance tests.
//!
//! Provider execution remains owned by the binary.  This library exposes only
//! config meta-validation so tests can validate shipped initial-input files
//! through the production validator instead of duplicating its rules.

mod config;
mod schema;

/// Validate one shipped initial-input template with production config rules.
#[doc(hidden)]
pub fn validate_config_for_tests(initial_input: &serde_json::Value) -> Result<(), String> {
    config::validate_config(initial_input)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
