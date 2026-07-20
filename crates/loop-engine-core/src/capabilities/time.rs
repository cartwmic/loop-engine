use crate::model::time::ObservedAt;

/// Current-time source. Model code never reads system time directly.
pub trait TimeSource {
    type Error;

    fn now(&self) -> Result<ObservedAt, Self::Error>;
}
