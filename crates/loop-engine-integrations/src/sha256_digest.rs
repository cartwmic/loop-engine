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

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn raw_argv_digest_uses_plain_sha256_hex() {
        assert_eq!(
            sha256_hex(b"loop-engine\0provider\0list"),
            "10718841a5e20e85aad8f29cc38168cf2c6baccf84193453280044305c2ffb5f"
        );
    }
}
