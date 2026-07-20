//! Fresh-process provider transport with bounded concurrent stream capture.

pub(crate) mod digest;
mod error;
mod process_group;
mod spawn;
mod streams;
#[cfg(test)]
mod tests;
mod timeout;
mod traced;

pub use error::ProcessError;
#[cfg(test)]
pub(crate) use spawn::run;
pub(crate) use spawn::{ProcessObservation, run_observed};
pub use streams::CapturedStream;
pub(crate) use traced::process_failure_code;
pub use traced::{TracedProviderBoundary, base64};
