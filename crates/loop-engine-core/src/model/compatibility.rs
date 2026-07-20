use std::collections::BTreeSet;

use thiserror::Error;

use super::bounded::{BoundError, BoundedText, IDENTIFIER_UTF8_BYTES};
use super::diagnostic::{Diagnostic, Diagnostics, validate_diagnostics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityStatus {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityFinding {
    capability: BoundedText<IDENTIFIER_UTF8_BYTES>,
    status: CompatibilityStatus,
    diagnostics: Vec<Diagnostic>,
}

impl CompatibilityFinding {
    pub fn new(
        capability: impl Into<String>,
        status: CompatibilityStatus,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Self, BoundError> {
        validate_diagnostics(&diagnostics)?;
        Ok(Self {
            capability: BoundedText::opaque_non_empty("capability", capability)?,
            status,
            diagnostics,
        })
    }

    pub fn capability(&self) -> &str {
        self.capability.as_str()
    }

    pub fn status(&self) -> CompatibilityStatus {
        self.status
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityFindings(Vec<CompatibilityFinding>);

impl CompatibilityFindings {
    pub fn new(values: Vec<CompatibilityFinding>) -> Result<Self, CompatibilityError> {
        let unique = values
            .iter()
            .map(CompatibilityFinding::capability)
            .collect::<BTreeSet<_>>();
        if unique.len() != values.len() {
            return Err(CompatibilityError::DuplicateCapability);
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[CompatibilityFinding] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityReport {
    Findings(CompatibilityFindings),
    EvaluationError(Diagnostics),
}

impl CompatibilityReport {
    pub fn findings(values: Vec<CompatibilityFinding>) -> Result<Self, CompatibilityError> {
        Ok(Self::Findings(CompatibilityFindings::new(values)?))
    }

    pub fn evaluation_error(diagnostics: Vec<Diagnostic>) -> Result<Self, CompatibilityError> {
        Ok(Self::EvaluationError(Diagnostics::new(diagnostics)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompatibilityError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error("compatibility result contains duplicate capability")]
    DuplicateCapability,
}

#[cfg(test)]
mod tests {
    use super::{CompatibilityFinding, CompatibilityReport, CompatibilityStatus};

    #[test]
    fn findings_represent_all_protocol_statuses_without_latching() {
        let findings = vec![
            CompatibilityFinding::new("gates", CompatibilityStatus::Compatible, vec![]).unwrap(),
            CompatibilityFinding::new("guidance", CompatibilityStatus::Incompatible, vec![])
                .unwrap(),
            CompatibilityFinding::new("inputs", CompatibilityStatus::Unknown, vec![]).unwrap(),
        ];
        let report = CompatibilityReport::findings(findings).unwrap();
        assert!(matches!(report, CompatibilityReport::Findings(_)));
        let duplicate = vec![
            CompatibilityFinding::new("gates", CompatibilityStatus::Compatible, vec![]).unwrap(),
            CompatibilityFinding::new("gates", CompatibilityStatus::Unknown, vec![]).unwrap(),
        ];
        assert!(CompatibilityReport::findings(duplicate).is_err());
    }
}
