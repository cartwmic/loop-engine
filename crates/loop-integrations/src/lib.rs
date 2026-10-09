//! Integration adapters for Loop Engine.

pub use loop_core::{ProviderError, ProviderGateway, ProviderResolutionError, ProviderResolver};

pub mod external_work;
pub mod proof_metadata;
mod provider;
mod provider_gateway;
mod sqlite;

pub use provider::{
    ConfiguredProviderResolver, ProviderAssociationError, ProviderConfiguration,
    ProviderDefinition, ProviderInvocation,
};
pub use provider_gateway::SubprocessProviderGateway;
pub use sqlite::{SqlitePersistence, TargetedReadRequest};

pub mod author_counts;

pub mod owner_checkpoint;

pub mod review_intake;

pub mod profile_setup;
