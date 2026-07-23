use std::fmt;

use thiserror::Error;

pub const PLANNED_OPERATION_IDS: &[&str] = &[
    "provider.add",
    "provider.list",
    "provider.check",
    "provider.update",
    "provider.rename",
    "provider.disable",
    "provider.restore",
    "run.create",
    "run.list",
    "run.terminate",
    "run.show",
    "run.graph",
    "run.history",
    "run.evidence.add",
    "run.evidence.list",
    "run.annotate",
    "run.label",
    "run.request",
    "run.guidance",
    "run.compatibility",
    "run.export",
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(&'static str);

impl OperationId {
    pub fn parse(value: &str) -> Result<Self, OperationIdError> {
        PLANNED_OPERATION_IDS
            .iter()
            .find(|candidate| **candidate == value)
            .copied()
            .map(Self)
            .ok_or_else(|| OperationIdError(value.to_owned()))
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }

    pub fn facet_manifest_path(self) -> String {
        format!("quality/facets/v1/{}.json", self.0)
    }

    pub fn planned() -> impl ExactSizeIterator<Item = Self> {
        PLANNED_OPERATION_IDS.iter().copied().map(Self)
    }
}

impl fmt::Debug for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("OperationId").field(&self.0).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown operation ID: {0}")]
pub struct OperationIdError(String);

/// Runtime-exposed core operation IDs. Exposure tasks edit this reviewed array.
pub const EXPOSED_OPERATION_IDS: &[&str] = &[
    "provider.add",
    "provider.list",
    "provider.check",
    "run.create",
    "run.list",
    "run.terminate",
    "run.show",
    "run.request",
    "run.history",
];

/// Runtime-exposed core operations. Private Phase 3 operations stay absent.
pub fn exposed_operations() -> Vec<OperationId> {
    EXPOSED_OPERATION_IDS
        .iter()
        .map(|value| OperationId::parse(value).expect("exposed ID must belong to frozen catalog"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{OperationId, exposed_operations};

    #[test]
    fn planned_ids_are_unique_and_runtime_set_matches_checkpoint_d() {
        let planned = OperationId::planned().collect::<Vec<_>>();
        assert_eq!(planned.len(), 21);
        assert_eq!(planned.iter().copied().collect::<BTreeSet<_>>().len(), 21);
        assert_eq!(
            exposed_operations()
                .iter()
                .map(|operation| operation.as_str())
                .collect::<Vec<_>>(),
            [
                "provider.add",
                "provider.list",
                "provider.check",
                "run.create",
                "run.list",
                "run.terminate",
                "run.show",
                "run.request",
                "run.history",
            ]
        );
        assert_eq!(
            OperationId::parse("run.show")
                .unwrap()
                .facet_manifest_path(),
            "quality/facets/v1/run.show.json"
        );
        assert!(OperationId::parse("run.delete").is_err());
    }
}
