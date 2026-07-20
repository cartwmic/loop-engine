use thiserror::Error;

use super::dto::PROTOCOL_MAJOR_V1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("unsupported provider protocol major {actual}; supported major is {supported}")]
pub struct UnsupportedMajor {
    pub actual: u32,
    pub supported: u32,
}

pub fn require_supported_major(actual: u32) -> Result<(), UnsupportedMajor> {
    if actual == PROTOCOL_MAJOR_V1 {
        Ok(())
    } else {
        Err(UnsupportedMajor {
            actual,
            supported: PROTOCOL_MAJOR_V1,
        })
    }
}
