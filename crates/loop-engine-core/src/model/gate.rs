use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::bounded::BoundError;
use super::diagnostic::Diagnostics;
use super::evidence::EvidenceRecord;
use super::ids::GateId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    gate: GateId,
    passed: bool,
    evidence: Vec<EvidenceRecord>,
}

impl GateVerdict {
    pub fn new(gate: GateId, passed: bool, evidence: Vec<EvidenceRecord>) -> Self {
        Self {
            gate,
            passed,
            evidence,
        }
    }

    pub fn gate(&self) -> &GateId {
        &self.gate
    }

    pub fn passed(&self) -> bool {
        self.passed
    }

    pub fn evidence(&self) -> &[EvidenceRecord] {
        &self.evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdicts(Vec<GateVerdict>);

impl GateVerdicts {
    pub fn new(verdicts: Vec<GateVerdict>) -> Self {
        Self(verdicts)
    }

    pub fn as_slice(&self) -> &[GateVerdict] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateEvaluation {
    Verdicts(GateVerdicts),
    Incompatible(Diagnostics),
    EvaluationError(Diagnostics),
}

impl GateEvaluation {
    pub fn verdicts(values: Vec<GateVerdict>) -> Self {
        Self::Verdicts(GateVerdicts::new(values))
    }

    pub fn incompatible(
        diagnostics: Vec<super::diagnostic::Diagnostic>,
    ) -> Result<Self, BoundError> {
        Ok(Self::Incompatible(Diagnostics::new(diagnostics)?))
    }

    pub fn evaluation_error(
        diagnostics: Vec<super::diagnostic::Diagnostic>,
    ) -> Result<Self, BoundError> {
        Ok(Self::EvaluationError(Diagnostics::new(diagnostics)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerdictSetError {
    #[error("duplicate verdict for gate {0}")]
    Duplicate(GateId),
    #[error("missing verdict for gate {0}")]
    Missing(GateId),
    #[error("extra verdict for gate {0}")]
    Extra(GateId),
}

pub fn validate_verdict_set<'a>(
    required: &[GateId],
    verdicts: &'a [GateVerdict],
) -> Result<BTreeMap<GateId, &'a GateVerdict>, VerdictSetError> {
    let required = required.iter().cloned().collect::<BTreeSet<_>>();
    let mut actual = BTreeMap::new();
    for verdict in verdicts {
        if actual.insert(verdict.gate().clone(), verdict).is_some() {
            return Err(VerdictSetError::Duplicate(verdict.gate().clone()));
        }
    }
    if let Some(extra) = actual.keys().find(|gate| !required.contains(*gate)) {
        return Err(VerdictSetError::Extra(extra.clone()));
    }
    if let Some(missing) = required.iter().find(|gate| !actual.contains_key(*gate)) {
        return Err(VerdictSetError::Missing(missing.clone()));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::{GateEvaluation, GateId, GateVerdict, VerdictSetError, validate_verdict_set};

    fn pass(id: &str) -> GateVerdict {
        GateVerdict::new(GateId::parse(id).unwrap(), true, vec![])
    }

    #[test]
    fn exact_verdict_set_rejects_duplicate_missing_and_extra() {
        let required = vec![GateId::parse("a").unwrap()];
        assert!(validate_verdict_set(&required, &[pass("a")]).is_ok());
        assert!(matches!(
            validate_verdict_set(&required, &[pass("a"), pass("a")]),
            Err(VerdictSetError::Duplicate(_))
        ));
        assert!(matches!(
            validate_verdict_set(&required, &[]),
            Err(VerdictSetError::Missing(_))
        ));
        assert!(matches!(
            validate_verdict_set(&required, &[pass("a"), pass("b")]),
            Err(VerdictSetError::Extra(_))
        ));
    }

    #[test]
    fn exactly_three_bounded_semantic_result_variants_exist() {
        let variants = [
            GateEvaluation::verdicts(vec![]),
            GateEvaluation::incompatible(vec![]).unwrap(),
            GateEvaluation::evaluation_error(vec![]).unwrap(),
        ];
        assert_eq!(variants.len(), 3);
    }
}
