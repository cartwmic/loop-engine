use jiff::Timestamp;
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::time::ObservedAt;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTimeSource;

impl TimeSource for SystemTimeSource {
    type Error = std::convert::Infallible;

    fn now(&self) -> Result<ObservedAt, Self::Error> {
        Ok(ObservedAt::parse(&Timestamp::now().to_string())
            .expect("system timestamp must satisfy core timestamp syntax"))
    }
}
