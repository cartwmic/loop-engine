use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WorkflowStateVersion(u64);

impl WorkflowStateVersion {
    pub fn initial() -> Self {
        Self(1)
    }

    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for WorkflowStateVersion {
    type Error = InvalidInternalVersion;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(InvalidInternalVersion)
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LifecycleVersion(u64);

impl LifecycleVersion {
    pub fn initial() -> Self {
        Self(1)
    }

    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for LifecycleVersion {
    type Error = InvalidInternalVersion;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(InvalidInternalVersion)
        } else {
            Ok(Self(value))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("internal version must be positive")]
pub struct InvalidInternalVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalSequence(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("journal sequence must be positive")]
pub struct InvalidJournalSequence;

impl JournalSequence {
    pub fn first() -> Self {
        Self(1)
    }

    pub fn next_sequence(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for JournalSequence {
    type Error = InvalidJournalSequence;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(InvalidJournalSequence)
        } else {
            Ok(Self(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JournalSequence, LifecycleVersion, WorkflowStateVersion};

    #[test]
    fn stored_versions_reconstitute_and_advance_monotonically() {
        assert_eq!(WorkflowStateVersion::initial().value(), 1);
        assert_eq!(LifecycleVersion::initial().value(), 1);
        assert_eq!(
            WorkflowStateVersion::try_from(41)
                .unwrap()
                .next()
                .unwrap()
                .value(),
            42
        );
        assert_eq!(
            LifecycleVersion::try_from(8)
                .unwrap()
                .next()
                .unwrap()
                .value(),
            9
        );
        assert!(WorkflowStateVersion::try_from(0).is_err());
        assert!(LifecycleVersion::try_from(0).is_err());
        assert_eq!(JournalSequence::try_from(9).unwrap().value(), 9);
        assert!(JournalSequence::try_from(0).is_err());
    }
}
