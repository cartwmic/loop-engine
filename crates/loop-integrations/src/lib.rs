//! Integration adapters for Loop Engine.

pub use loop_core::{ProviderError, ProviderGateway, ProviderResolutionError, ProviderResolver};

pub mod capture;
pub mod ownership;
mod provider;
mod provider_gateway;
mod sqlite;

pub use provider::{
    ConfiguredProviderResolver, ProviderAssociationError, ProviderConfiguration,
    ProviderDefinition, ProviderInvocation,
};
pub use provider_gateway::SubprocessProviderGateway;
pub use sqlite::SqlitePersistence;
