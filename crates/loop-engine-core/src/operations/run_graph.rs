use crate::capabilities::run_reader::RunReader;
use crate::model::graph_projection::SemanticGraphProjection;
use crate::model::graph_validation::ValidatedGraph;
use crate::model::ids::{GraphRevision, RunId};
use crate::model::run::Run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGraph {
    pub revision: GraphRevision,
    pub graph: SemanticGraphProjection,
}

pub fn execute<R: RunReader>(reader: &R, run_id: &RunId) -> Result<StoredGraph, R::Error> {
    reader.get(run_id).map(|run| project(&run))
}

pub fn project(run: &Run) -> StoredGraph {
    StoredGraph {
        revision: run.graph_revision().clone(),
        graph: SemanticGraphProjection::from_validated(
            &ValidatedGraph::validate(run.graph().clone()).expect("stored graph remains valid"),
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn projection_uses_stored_graph_without_provider_invocation() {
        let run = crate::operations::test_support::run();
        let stored = super::project(&run);
        assert_eq!(stored.revision, *run.graph_revision());
    }
}
