use crate::capabilities::provider_catalog::ResolvedProviderConfig;
use crate::model::graph_projection::SemanticGraphProjection;
use crate::model::ids::GraphRevision;
use crate::model::provider::DigestObservation;

/// Hashing boundary. Canonical encoding and executable reads stay outside core.
pub trait DigestComputer {
    type Error;

    /// Integration canonicalizes this validated semantic projection before hashing.
    fn graph_revision(&self, graph: &SemanticGraphProjection)
    -> Result<GraphRevision, Self::Error>;
    fn executable_digest(
        &self,
        config: &ResolvedProviderConfig,
    ) -> Result<DigestObservation, Self::Error>;
}
