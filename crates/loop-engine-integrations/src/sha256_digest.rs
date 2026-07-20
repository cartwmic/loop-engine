use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::ids::GraphRevision;
use loop_engine_core::model::provider::DigestObservation;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provider_protocol::canonical::{CanonicalError, graph_bytes};

#[derive(Debug, Default, Clone, Copy)]
pub struct Sha256DigestComputer;

#[derive(Debug, Error)]
pub enum DigestError {
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    #[error("provider executable digest unavailable: {0}")]
    ExecutableRead(#[source] std::io::Error),
    #[error("digest model rejected computed value: {0}")]
    Model(String),
}

impl DigestComputer for Sha256DigestComputer {
    type Error = DigestError;

    fn graph_revision(
        &self,
        graph: &SemanticGraphProjection,
    ) -> Result<GraphRevision, Self::Error> {
        let bytes = graph_bytes(graph)?;
        GraphRevision::parse(sha256_label(&bytes))
            .map_err(|error| DigestError::Model(error.to_string()))
    }

    fn executable_digest(
        &self,
        config: &ResolvedProviderConfig,
    ) -> Result<DigestObservation, Self::Error> {
        crate::provider_process::digest::observe_executable_digest(config)
    }
}

pub fn sha256_label(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let encoded = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{encoded}")
}
