use std::fmt;

use jiff::Timestamp;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid timestamp: {0}")]
pub struct TimestampError(String);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservedAt(Timestamp);

impl ObservedAt {
    pub fn parse(value: &str) -> Result<Self, TimestampError> {
        value
            .parse::<Timestamp>()
            .map(Self)
            .map_err(|error| TimestampError(error.to_string()))
    }

    pub fn as_timestamp(self) -> Timestamp {
        self.0
    }
}

impl fmt::Debug for ObservedAt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
