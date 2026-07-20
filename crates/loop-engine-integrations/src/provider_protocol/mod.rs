//! Provider protocol v1 DTOs, strict parsing, semantic mapping, and canonical graph encoding.

mod adapter;
pub mod canonical;
mod compatibility;
mod context;
mod describe;
pub mod dto;
mod evaluate_gates;
pub mod graph;
mod invoke;
mod live_guidance;
pub mod mapping;
pub mod validate_inputs;
pub mod validation;
pub mod version;

pub use adapter::SubprocessProviderInvoker;
pub use dto::PROTOCOL_MAJOR_V1;
pub use invoke::AdapterError;
