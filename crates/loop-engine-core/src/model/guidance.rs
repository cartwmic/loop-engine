use super::bounded::{BoundError, BoundedText, GUIDANCE_TEXT_BYTES};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StaticGuidance {
    Text(BoundedText<GUIDANCE_TEXT_BYTES>),
    NoneRequired,
}

impl StaticGuidance {
    pub fn text(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self::Text(BoundedText::non_empty(
            "static_guidance",
            value,
        )?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LiveGuidanceCapability {
    Supported,
    Unsupported,
}
