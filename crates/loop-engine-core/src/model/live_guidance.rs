use super::bounded::{BoundError, BoundedText, GUIDANCE_TEXT_BYTES};
use super::diagnostic::{Diagnostic, Diagnostics};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryGuidance {
    text: BoundedText<GUIDANCE_TEXT_BYTES>,
}

impl AdvisoryGuidance {
    pub fn new(text: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self {
            text: BoundedText::non_empty("guidance_text", text)?,
        })
    }

    pub fn text(&self) -> &str {
        self.text.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveGuidanceResult {
    Guidance(AdvisoryGuidance),
    Incompatible(Diagnostics),
    EvaluationError(Diagnostics),
}

impl LiveGuidanceResult {
    pub fn incompatible(diagnostics: Vec<Diagnostic>) -> Result<Self, BoundError> {
        Ok(Self::Incompatible(Diagnostics::new(diagnostics)?))
    }

    pub fn evaluation_error(diagnostics: Vec<Diagnostic>) -> Result<Self, BoundError> {
        Ok(Self::EvaluationError(Diagnostics::new(diagnostics)?))
    }
}

#[cfg(test)]
mod tests {
    use super::{AdvisoryGuidance, LiveGuidanceResult};

    #[test]
    fn guidance_is_text_only_and_other_results_retain_diagnostics() {
        let guidance = LiveGuidanceResult::Guidance(AdvisoryGuidance::new("next").unwrap());
        assert!(matches!(guidance, LiveGuidanceResult::Guidance(_)));
        assert!(LiveGuidanceResult::incompatible(vec![]).is_ok());
        assert!(LiveGuidanceResult::evaluation_error(vec![]).is_ok());
    }
}
