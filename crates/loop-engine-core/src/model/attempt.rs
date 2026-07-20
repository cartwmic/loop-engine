use std::collections::BTreeSet;

use thiserror::Error;

use super::annotation::{ActorMetadata, Note};
use super::bounded::{
    BoundError, BoundedText, EVIDENCE_LOCATOR_UTF8_BYTES, GUIDANCE_TEXT_BYTES, RUN_LABEL_UTF8_BYTES,
};
use super::compatibility::CompatibilityFindings;
use super::diagnostic::{Diagnostic, Diagnostics, validate_diagnostics};
use super::evidence::EvidenceRecord;
use super::ids::{
    EventId, EvidenceId, EvidenceKind, GateId, GraphRevision, RegistrationId, RequestId, StateId,
};
use super::outcome::{EvidenceRecordedStatus, OutcomeClass};
use super::provider::DigestObservation;
use super::version::JournalSequence;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRole {
    Describe,
    ValidateInputs,
    EvaluateGates,
    LiveGuidance,
    CheckCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFact {
    pub registration_id: RegistrationId,
    pub config_revision: u64,
    pub role: ProviderRole,
    pub invocation_id: RequestId,
    pub executable: BoundedText<4_096>,
    pub outcome: OutcomeClass,
    pub digest: DigestObservation,
    pub provider_version: Option<BoundedText<256>>,
    pub protocol_major: Option<u64>,
}

impl ProviderFact {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registration_id: RegistrationId,
        config_revision: u64,
        role: ProviderRole,
        invocation_id: RequestId,
        executable: impl Into<String>,
        outcome: OutcomeClass,
        digest: DigestObservation,
        provider_version: Option<String>,
        protocol_major: Option<u64>,
    ) -> Result<Self, BoundError> {
        if config_revision == 0 || protocol_major == Some(0) {
            return Err(BoundError::InvalidType {
                field: "provider_observation_revision",
            });
        }
        Ok(Self {
            registration_id,
            config_revision,
            role,
            invocation_id,
            executable: BoundedText::opaque_non_empty("provider_executable", executable)?,
            outcome,
            digest,
            provider_version: provider_version
                .map(|value| BoundedText::opaque_non_empty("provider_version", value))
                .transpose()?,
            protocol_major,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdictFact {
    pub gate_id: GateId,
    pub passed: bool,
    pub message: Option<BoundedText<8_192>>,
}

impl GateVerdictFact {
    pub fn new(gate_id: GateId, passed: bool, message: Option<String>) -> Result<Self, BoundError> {
        Ok(Self {
            gate_id,
            passed,
            message: message
                .map(|value| BoundedText::non_empty("gate_verdict_message", value))
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdictResult {
    Verdicts(Vec<GateVerdictFact>),
    Incompatibility(Diagnostic),
    EvaluationError(Diagnostics),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdictFacts {
    pub event: EventId,
    pub gate_ids: Vec<GateId>,
    pub result: GateVerdictResult,
}

impl GateVerdictFacts {
    pub fn new(
        event: EventId,
        gate_ids: Vec<GateId>,
        result: GateVerdictResult,
    ) -> Result<Self, AttemptError> {
        let required = gate_ids.iter().cloned().collect::<BTreeSet<_>>();
        if required.len() != gate_ids.len() {
            return Err(AttemptError::DuplicateGate);
        }
        match &result {
            GateVerdictResult::Verdicts(verdicts) => {
                let actual = verdicts
                    .iter()
                    .map(|verdict| verdict.gate_id.clone())
                    .collect::<BTreeSet<_>>();
                if actual.len() != verdicts.len() || actual != required {
                    return Err(AttemptError::VerdictSetMismatch);
                }
            }
            GateVerdictResult::EvaluationError(_) => {}
            GateVerdictResult::Incompatibility(_) => {}
        }
        Ok(Self {
            event,
            gate_ids,
            result,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvidenceAssociations {
    pub inline: Vec<EvidenceRecord>,
    pub selected_ids: Vec<EvidenceId>,
    pub provider_recorded_ids: Vec<EvidenceId>,
}

impl EvidenceAssociations {
    pub fn recorded_status(&self) -> EvidenceRecordedStatus {
        EvidenceRecordedStatus {
            inline: !self.inline.is_empty(),
            selected_associations: !self.selected_ids.is_empty(),
            provider: !self.provider_recorded_ids.is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionFact {
    pub event: EventId,
    pub source: StateId,
    pub target: Option<StateId>,
    pub applied: bool,
}

impl TransitionFact {
    pub fn new(
        event: EventId,
        source: StateId,
        target: Option<StateId>,
        applied: bool,
    ) -> Result<Self, AttemptError> {
        if applied != target.is_some() {
            return Err(AttemptError::TransitionTargetShape);
        }
        Ok(Self {
            event,
            source,
            target,
            applied,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttemptFacts {
    pub transition: Option<TransitionFact>,
    pub provider_observations: Vec<ProviderFact>,
    pub gate_verdict_facts: Option<GateVerdictFacts>,
    pub evidence_associations: Option<EvidenceAssociations>,
    pub evidence_recorded: Option<EvidenceRecordedStatus>,
    pub note: Option<Note>,
    pub actor: Option<ActorMetadata>,
    pub corrects_sequence: Option<JournalSequence>,
    pub diagnostics: Vec<Diagnostic>,
}

impl AttemptFacts {
    pub fn validate(self) -> Result<Self, AttemptError> {
        validate_diagnostics(&self.diagnostics)?;
        if self.evidence_associations.is_some() != self.evidence_recorded.is_some() {
            return Err(AttemptError::EvidenceRecordedMismatch);
        }
        if let (Some(associations), Some(recorded)) =
            (&self.evidence_associations, self.evidence_recorded)
            && associations.recorded_status() != recorded
        {
            return Err(AttemptError::EvidenceRecordedMismatch);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAddedFact {
    pub evidence_id: EvidenceId,
    pub kind: EvidenceKind,
    pub locator: BoundedText<EVIDENCE_LOCATOR_UTF8_BYTES>,
    pub digest: Option<BoundedText<256>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChangeFact {
    pub label_before: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
    pub label_after: Option<BoundedText<RUN_LABEL_UTF8_BYTES>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalExtension {
    RunCreated {
        graph_revision: GraphRevision,
    },
    EvidenceAdded {
        added: Option<EvidenceAddedFact>,
    },
    Annotation,
    LabelChanged {
        change: Option<LabelChangeFact>,
    },
    TransitionAttempt,
    GuidanceAttempt {
        guidance_text: Option<BoundedText<GUIDANCE_TEXT_BYTES>>,
    },
    CompatibilityAttempt {
        findings: Option<CompatibilityFindings>,
    },
    RunTerminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttemptError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error("gate list contains a duplicate")]
    DuplicateGate,
    #[error("gate verdict set does not exactly match required gates")]
    VerdictSetMismatch,
    #[error("transition target must be present exactly when applied")]
    TransitionTargetShape,
    #[error("evidence_recorded does not match committed associations")]
    EvidenceRecordedMismatch,
}
