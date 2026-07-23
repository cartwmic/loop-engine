//! Strict run-input JSON loading for composition-owned `run.create` wiring.

mod error;
mod load;

pub use error::RunInputLoadError;
pub use load::{RUN_INPUTS_FILE_BYTES, load_optional};
