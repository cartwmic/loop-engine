use thiserror::Error;

use super::attempt::{AttemptError, AttemptFacts, JournalExtension, ProviderRole};
use super::bounded::{
    ACTOR_METADATA_ENCODED_BYTES, BoundError, BoundedText, JOURNAL_ENTRY_ENCODED_BYTES,
    JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES, JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES,
    JOURNAL_PROVIDER_FACTS_ENCODED_BYTES, NOTE_TEXT_UTF8_BYTES,
};
use super::ids::{RequestId, RunId, StateId};
use super::lifecycle::Lifecycle;
use super::outcome::OutcomeClass;
use super::reason::Reason;
use super::time::ObservedAt;
use super::version::{JournalSequence, LifecycleVersion, WorkflowStateVersion};

const JOURNAL_DIAGNOSTIC_AGGREGATE_BYTES: usize = 819_200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalEntryKind {
    RunCreated,
    EvidenceAdded,
    Annotation,
    LabelChanged,
    TransitionAttempt,
    GuidanceAttempt,
    CompatibilityAttempt,
    RunTerminated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFact {
    pub state: StateId,
    pub lifecycle: Lifecycle,
    pub workflow_state_version: WorkflowStateVersion,
    pub lifecycle_version: LifecycleVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JournalEncodedSizes {
    pub entry: usize,
    pub evidence_associations: usize,
    pub provider_observations: usize,
    pub gate_verdict_facts: usize,
    pub diagnostics: usize,
    pub note: usize,
    pub actor: usize,
}

/// Sequence/state-free journal facts prepared before an atomic persistence transaction.
/// Persistence assigns sequence, authoritative state facts, and exact encoded sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalDraft {
    run_id: RunId,
    observed_at: ObservedAt,
    operation: BoundedText<128>,
    request_id: RequestId,
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    reason: Option<Reason>,
    attempt: Option<AttemptFacts>,
    extension: JournalExtension,
}

impl JournalDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: RunId,
        observed_at: ObservedAt,
        operation: impl Into<String>,
        request_id: RequestId,
        outcome: OutcomeClass,
        reason: Option<Reason>,
        attempt: Option<AttemptFacts>,
        extension: JournalExtension,
    ) -> Result<Self, JournalError> {
        match (outcome, &reason) {
            (OutcomeClass::Completed, Some(_)) => return Err(JournalError::CompletedWithReason),
            (OutcomeClass::Completed, None) => {}
            (_, Some(reason)) if reason.code().outcome_class() == outcome => {}
            _ => return Err(JournalError::MissingOrMismatchedReason),
        }
        let operation = BoundedText::opaque_non_empty("journal_operation", operation)?;
        let kind = kind_for_extension(&extension);
        if operation_for_kind(kind) != operation.as_str() {
            return Err(JournalError::OperationKindMismatch);
        }
        validate_extension_outcome(&extension, outcome)?;
        let attempt = attempt.map(AttemptFacts::validate).transpose()?;
        validate_attempt_shape(kind, outcome, &attempt)?;
        Ok(Self {
            run_id,
            observed_at,
            operation,
            request_id,
            kind,
            outcome,
            reason,
            attempt,
            extension,
        })
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn observed_at(&self) -> ObservedAt {
        self.observed_at
    }

    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    pub fn operation(&self) -> &str {
        self.operation.as_str()
    }

    pub fn kind(&self) -> JournalEntryKind {
        self.kind
    }

    pub fn outcome(&self) -> OutcomeClass {
        self.outcome
    }

    pub fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    pub fn attempt(&self) -> Option<&AttemptFacts> {
        self.attempt.as_ref()
    }

    pub fn extension(&self) -> &JournalExtension {
        &self.extension
    }

    pub(crate) fn replacing_extension(
        self,
        extension: JournalExtension,
    ) -> Result<Self, JournalError> {
        Self::new(
            self.run_id,
            self.observed_at,
            self.operation.as_str(),
            self.request_id,
            self.outcome,
            self.reason,
            self.attempt,
            extension,
        )
    }

    pub fn finalize(
        self,
        sequence: JournalSequence,
        state_before: StateFact,
        state_after: StateFact,
        encoded_sizes: JournalEncodedSizes,
    ) -> Result<JournalEntry, JournalError> {
        JournalEntry::new(
            sequence,
            self.run_id,
            self.observed_at,
            self.operation.as_str(),
            self.request_id,
            self.outcome,
            self.reason,
            state_before,
            state_after,
            self.attempt,
            self.extension,
            encoded_sizes,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    sequence: JournalSequence,
    run_id: RunId,
    observed_at: ObservedAt,
    operation: BoundedText<128>,
    request_id: RequestId,
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    reason: Option<Reason>,
    state_before: StateFact,
    state_after: StateFact,
    attempt: Option<AttemptFacts>,
    extension: JournalExtension,
    encoded_sizes: JournalEncodedSizes,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JournalError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Attempt(#[from] AttemptError),
    #[error("completed journal entry cannot carry a reason")]
    CompletedWithReason,
    #[error("non-completed journal entry requires matching reason")]
    MissingOrMismatchedReason,
    #[error("journal operation does not produce this entry kind")]
    OperationKindMismatch,
    #[error("journal entry attempt shape is invalid for its kind or outcome")]
    InvalidAttemptShape,
    #[error("journal state/version facts are inconsistent with the durable effect")]
    InvalidStateFacts,
    #[error("run.created must be sequence 1 and other entry kinds must follow it")]
    InvalidSequence,
    #[error("exact encoded component size is absent or present inconsistently")]
    EncodedComponentShape,
    #[error("correction link must reference an earlier sequence")]
    InvalidCorrectionLink,
}

impl JournalEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: JournalSequence,
        run_id: RunId,
        observed_at: ObservedAt,
        operation: impl Into<String>,
        request_id: RequestId,
        outcome: OutcomeClass,
        reason: Option<Reason>,
        state_before: StateFact,
        state_after: StateFact,
        attempt: Option<AttemptFacts>,
        extension: JournalExtension,
        encoded_sizes: JournalEncodedSizes,
    ) -> Result<Self, JournalError> {
        match (outcome, &reason) {
            (OutcomeClass::Completed, Some(_)) => return Err(JournalError::CompletedWithReason),
            (OutcomeClass::Completed, None) => {}
            (_, Some(reason)) if reason.code().outcome_class() == outcome => {}
            _ => return Err(JournalError::MissingOrMismatchedReason),
        }
        let operation = BoundedText::opaque_non_empty("journal_operation", operation)?;
        let kind = kind_for_extension(&extension);
        if operation_for_kind(kind) != operation.as_str() {
            return Err(JournalError::OperationKindMismatch);
        }
        if (kind == JournalEntryKind::RunCreated) != (sequence == JournalSequence::first()) {
            return Err(JournalError::InvalidSequence);
        }
        validate_extension_outcome(&extension, outcome)?;
        let attempt = attempt.map(AttemptFacts::validate).transpose()?;
        if attempt
            .as_ref()
            .and_then(|facts| facts.corrects_sequence)
            .is_some_and(|corrected| corrected >= sequence)
        {
            return Err(JournalError::InvalidCorrectionLink);
        }
        validate_attempt_shape(kind, outcome, &attempt)?;
        validate_state_facts(kind, outcome, &state_before, &state_after, attempt.as_ref())?;
        validate_encoded_sizes(&encoded_sizes, attempt.as_ref())?;
        Ok(Self {
            sequence,
            run_id,
            observed_at,
            operation,
            request_id,
            kind,
            outcome,
            reason,
            state_before,
            state_after,
            attempt,
            extension,
            encoded_sizes,
        })
    }

    pub fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn observed_at(&self) -> ObservedAt {
        self.observed_at
    }

    pub fn operation(&self) -> &str {
        self.operation.as_str()
    }

    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    pub fn kind(&self) -> JournalEntryKind {
        self.kind
    }

    pub fn outcome(&self) -> OutcomeClass {
        self.outcome
    }

    pub fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    pub fn state_before(&self) -> &StateFact {
        &self.state_before
    }

    pub fn state_after(&self) -> &StateFact {
        &self.state_after
    }

    pub fn attempt(&self) -> Option<&AttemptFacts> {
        self.attempt.as_ref()
    }

    pub fn extension(&self) -> &JournalExtension {
        &self.extension
    }

    pub fn state_changed(&self) -> bool {
        self.state_before.state != self.state_after.state
    }

    pub fn encoded_size(&self) -> usize {
        self.encoded_sizes.entry
    }
}

fn validate_extension_outcome(
    extension: &JournalExtension,
    outcome: OutcomeClass,
) -> Result<(), JournalError> {
    let valid = match extension {
        JournalExtension::RunCreated { .. } => outcome == OutcomeClass::Completed,
        JournalExtension::EvidenceAdded { added } => match outcome {
            OutcomeClass::Completed => added.is_some(),
            OutcomeClass::Rejected => added.is_none(),
            OutcomeClass::Error => false,
        },
        JournalExtension::Annotation => outcome != OutcomeClass::Error,
        JournalExtension::RunTerminated => true,
        JournalExtension::LabelChanged { change } => match outcome {
            OutcomeClass::Completed => change.is_some(),
            OutcomeClass::Rejected => change.is_none(),
            OutcomeClass::Error => false,
        },
        JournalExtension::TransitionAttempt => true,
        JournalExtension::GuidanceAttempt { guidance_text } => {
            (outcome == OutcomeClass::Completed) == guidance_text.is_some()
        }
        JournalExtension::CompatibilityAttempt { findings } => {
            (outcome == OutcomeClass::Completed) == findings.is_some()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(JournalError::InvalidAttemptShape)
    }
}

fn kind_for_extension(extension: &JournalExtension) -> JournalEntryKind {
    match extension {
        JournalExtension::RunCreated { .. } => JournalEntryKind::RunCreated,
        JournalExtension::EvidenceAdded { .. } => JournalEntryKind::EvidenceAdded,
        JournalExtension::Annotation => JournalEntryKind::Annotation,
        JournalExtension::LabelChanged { .. } => JournalEntryKind::LabelChanged,
        JournalExtension::TransitionAttempt => JournalEntryKind::TransitionAttempt,
        JournalExtension::GuidanceAttempt { .. } => JournalEntryKind::GuidanceAttempt,
        JournalExtension::CompatibilityAttempt { .. } => JournalEntryKind::CompatibilityAttempt,
        JournalExtension::RunTerminated => JournalEntryKind::RunTerminated,
    }
}

fn operation_for_kind(kind: JournalEntryKind) -> &'static str {
    match kind {
        JournalEntryKind::RunCreated => "run.create",
        JournalEntryKind::EvidenceAdded => "run.evidence.add",
        JournalEntryKind::Annotation => "run.annotate",
        JournalEntryKind::LabelChanged => "run.label",
        JournalEntryKind::TransitionAttempt => "run.request",
        JournalEntryKind::GuidanceAttempt => "run.guidance",
        JournalEntryKind::CompatibilityAttempt => "run.compatibility",
        JournalEntryKind::RunTerminated => "run.terminate",
    }
}

fn validate_attempt_shape(
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    attempt: &Option<AttemptFacts>,
) -> Result<(), JournalError> {
    let required = matches!(
        kind,
        JournalEntryKind::RunCreated
            | JournalEntryKind::EvidenceAdded
            | JournalEntryKind::Annotation
            | JournalEntryKind::TransitionAttempt
            | JournalEntryKind::GuidanceAttempt
            | JournalEntryKind::CompatibilityAttempt
    );
    if required && attempt.is_none() {
        return Err(JournalError::InvalidAttemptShape);
    }
    if let Some(attempt) = attempt {
        if attempt.corrects_sequence.is_some() && kind != JournalEntryKind::Annotation {
            return Err(JournalError::InvalidAttemptShape);
        }
        if matches!(
            kind,
            JournalEntryKind::EvidenceAdded | JournalEntryKind::TransitionAttempt
        ) && (attempt.evidence_associations.is_none() || attempt.evidence_recorded.is_none())
        {
            return Err(JournalError::InvalidAttemptShape);
        }
        if (kind == JournalEntryKind::TransitionAttempt) != attempt.transition.is_some() {
            return Err(JournalError::InvalidAttemptShape);
        }
        if let Some(transition) = &attempt.transition
            && (outcome == OutcomeClass::Completed) != transition.applied
        {
            return Err(JournalError::InvalidAttemptShape);
        }
        if kind == JournalEntryKind::RunCreated {
            let roles = attempt
                .provider_observations
                .iter()
                .map(|fact| fact.role)
                .collect::<Vec<_>>();
            if (roles != [ProviderRole::Describe]
                && roles != [ProviderRole::Describe, ProviderRole::ValidateInputs])
                || attempt
                    .provider_observations
                    .iter()
                    .any(|fact| fact.outcome != OutcomeClass::Completed)
            {
                return Err(JournalError::InvalidAttemptShape);
            }
        }
    }
    Ok(())
}

fn validate_state_facts(
    kind: JournalEntryKind,
    outcome: OutcomeClass,
    before: &StateFact,
    after: &StateFact,
    attempt: Option<&AttemptFacts>,
) -> Result<(), JournalError> {
    if outcome != OutcomeClass::Completed {
        return if before == after {
            Ok(())
        } else {
            Err(JournalError::InvalidStateFacts)
        };
    }
    match kind {
        JournalEntryKind::RunCreated
            if before != after
                || before.workflow_state_version != WorkflowStateVersion::initial()
                || before.lifecycle_version != LifecycleVersion::initial() =>
        {
            return Err(JournalError::InvalidStateFacts);
        }
        JournalEntryKind::TransitionAttempt => {
            let transition = attempt
                .and_then(|value| value.transition.as_ref())
                .ok_or(JournalError::InvalidAttemptShape)?;
            if transition.source != before.state
                || transition.target.as_ref() != Some(&after.state)
                || before.lifecycle.is_terminal()
            {
                return Err(JournalError::InvalidStateFacts);
            }
            let state_changed = before.state != after.state;
            let lifecycle_changed = before.lifecycle != after.lifecycle;
            if !version_alignment(
                before.workflow_state_version.value(),
                after.workflow_state_version.value(),
                state_changed,
            ) || !version_alignment(
                before.lifecycle_version.value(),
                after.lifecycle_version.value(),
                lifecycle_changed,
            ) {
                return Err(JournalError::InvalidStateFacts);
            }
        }
        JournalEntryKind::RunTerminated
            if before.state != after.state
                || before.lifecycle != Lifecycle::Active
                || after.lifecycle != Lifecycle::Terminated
                || before.workflow_state_version != after.workflow_state_version
                || !version_alignment(
                    before.lifecycle_version.value(),
                    after.lifecycle_version.value(),
                    true,
                ) =>
        {
            return Err(JournalError::InvalidStateFacts);
        }
        JournalEntryKind::RunTerminated => {}
        _ if before != after => return Err(JournalError::InvalidStateFacts),
        _ => {}
    }
    Ok(())
}

fn version_alignment(before: u64, after: u64, changed: bool) -> bool {
    if changed {
        before.checked_add(1) == Some(after)
    } else {
        before == after
    }
}

fn validate_encoded_sizes(
    sizes: &JournalEncodedSizes,
    attempt: Option<&AttemptFacts>,
) -> Result<(), JournalError> {
    let check = |field, actual, max| {
        if actual > max {
            Err(JournalError::Bound(BoundError::EncodedTooLarge {
                field,
                max,
                actual,
            }))
        } else {
            Ok(())
        }
    };
    if sizes.entry == 0 {
        return Err(JournalError::EncodedComponentShape);
    }
    check("journal_entry", sizes.entry, JOURNAL_ENTRY_ENCODED_BYTES)?;
    check(
        "journal_evidence_associations",
        sizes.evidence_associations,
        JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES,
    )?;
    check(
        "journal_provider_facts",
        sizes.provider_observations,
        JOURNAL_PROVIDER_FACTS_ENCODED_BYTES,
    )?;
    check(
        "journal_gate_verdict_facts",
        sizes.gate_verdict_facts,
        JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES,
    )?;
    check(
        "journal_diagnostic_aggregate",
        sizes.diagnostics,
        JOURNAL_DIAGNOSTIC_AGGREGATE_BYTES,
    )?;
    check("note", sizes.note, NOTE_TEXT_UTF8_BYTES)?;
    check("actor", sizes.actor, ACTOR_METADATA_ENCODED_BYTES)?;
    let component_present = |present: bool, bytes: usize| present == (bytes > 0);
    let valid_shape = match attempt {
        None => [
            sizes.evidence_associations,
            sizes.provider_observations,
            sizes.gate_verdict_facts,
            sizes.diagnostics,
            sizes.note,
            sizes.actor,
        ]
        .into_iter()
        .all(|size| size == 0),
        Some(attempt) => {
            component_present(
                attempt.evidence_associations.is_some(),
                sizes.evidence_associations,
            ) && component_present(
                !attempt.provider_observations.is_empty(),
                sizes.provider_observations,
            ) && component_present(
                attempt.gate_verdict_facts.is_some(),
                sizes.gate_verdict_facts,
            ) && component_present(!attempt.diagnostics.is_empty(), sizes.diagnostics)
                && component_present(attempt.note.is_some(), sizes.note)
                && component_present(attempt.actor.is_some(), sizes.actor)
        }
    };
    if !valid_shape {
        return Err(JournalError::EncodedComponentShape);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BoundError, JOURNAL_ENTRY_ENCODED_BYTES, JournalEncodedSizes, JournalEntry, JournalError,
        JournalSequence, Lifecycle, LifecycleVersion, ObservedAt, OutcomeClass, RequestId, RunId,
        StateFact, StateId, WorkflowStateVersion,
    };
    use crate::model::annotation::{ActorMetadata, Note};
    use crate::model::attempt::{
        AttemptFacts, EvidenceAssociations, GateVerdictFact, GateVerdictFacts, GateVerdictResult,
        JournalExtension, ProviderFact, ProviderRole, TransitionFact,
    };
    use crate::model::bounded::Value;
    use crate::model::ids::{EventId, EvidenceId, GateId, GraphRevision, RegistrationId};
    use crate::model::provider::DigestObservation;
    use crate::model::reason::{Reason, ReasonCode};
    use std::collections::BTreeMap;

    fn state(name: &str, workflow: u64) -> StateFact {
        StateFact {
            state: StateId::parse(name).unwrap(),
            lifecycle: Lifecycle::Active,
            workflow_state_version: WorkflowStateVersion::try_from(workflow).unwrap(),
            lifecycle_version: LifecycleVersion::initial(),
        }
    }

    fn sizes(entry: usize) -> JournalEncodedSizes {
        JournalEncodedSizes {
            entry,
            ..JournalEncodedSizes::default()
        }
    }

    fn creation_attempt() -> AttemptFacts {
        AttemptFacts {
            provider_observations: vec![
                ProviderFact::new(
                    RegistrationId::parse("registration").unwrap(),
                    1,
                    ProviderRole::Describe,
                    RequestId::parse("describe-invocation").unwrap(),
                    "/provider",
                    OutcomeClass::Completed,
                    DigestObservation::Unavailable,
                    None,
                    Some(1),
                )
                .unwrap(),
            ],
            ..AttemptFacts::default()
        }
    }

    fn creation_sizes(entry: usize) -> JournalEncodedSizes {
        JournalEncodedSizes {
            entry,
            provider_observations: 100,
            ..JournalEncodedSizes::default()
        }
    }

    #[test]
    fn creation_kind_is_bound_to_operation_and_sequence() {
        let entry = JournalEntry::new(
            JournalSequence::first(),
            RunId::parse("run").unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.create",
            RequestId::parse("request").unwrap(),
            OutcomeClass::Completed,
            None,
            state("a", 1),
            state("a", 1),
            Some(creation_attempt()),
            JournalExtension::RunCreated {
                graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            },
            creation_sizes(100),
        )
        .unwrap();
        assert_eq!(entry.operation(), "run.create");
        assert!(matches!(
            JournalEntry::new(
                JournalSequence::first(),
                RunId::parse("run").unwrap(),
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                "run.create",
                RequestId::parse("request").unwrap(),
                OutcomeClass::Completed,
                None,
                state("a", 1),
                state("a", 1),
                None,
                JournalExtension::RunCreated {
                    graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64)))
                        .unwrap(),
                },
                sizes(100),
            ),
            Err(JournalError::InvalidAttemptShape)
        ));
        assert!(matches!(
            JournalEntry::new(
                JournalSequence::first(),
                RunId::parse("run").unwrap(),
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                "run.show",
                RequestId::parse("request").unwrap(),
                OutcomeClass::Completed,
                None,
                state("a", 1),
                state("a", 1),
                Some(creation_attempt()),
                JournalExtension::RunCreated {
                    graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64)))
                        .unwrap(),
                },
                creation_sizes(100),
            ),
            Err(JournalError::OperationKindMismatch)
        ));
        assert!(matches!(
            JournalEntry::new(
                JournalSequence::first(),
                RunId::parse("run").unwrap(),
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                "run.create",
                RequestId::parse("request").unwrap(),
                OutcomeClass::Rejected,
                Some(Reason::new(ReasonCode::InputRejected, "rejected").unwrap()),
                state("a", 1),
                state("a", 1),
                Some(creation_attempt()),
                JournalExtension::RunCreated {
                    graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64)))
                        .unwrap(),
                },
                creation_sizes(100),
            ),
            Err(JournalError::InvalidAttemptShape)
        ));
    }

    #[test]
    fn aggregate_bound_accepts_maximum_and_rejects_one_byte_over() {
        let build = |entry| {
            JournalEntry::new(
                JournalSequence::first(),
                RunId::parse("run").unwrap(),
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                "run.create",
                RequestId::parse("request").unwrap(),
                OutcomeClass::Completed,
                None,
                state("a", 1),
                state("a", 1),
                Some(creation_attempt()),
                JournalExtension::RunCreated {
                    graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64)))
                        .unwrap(),
                },
                creation_sizes(entry),
            )
        };
        assert!(build(JOURNAL_ENTRY_ENCODED_BYTES).is_ok());
        assert!(matches!(
            build(JOURNAL_ENTRY_ENCODED_BYTES + 1),
            Err(JournalError::Bound(BoundError::EncodedTooLarge { .. }))
        ));
    }

    #[test]
    fn completed_self_loop_is_applied_without_workflow_version_bump() {
        let attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    EventId::parse("again").unwrap(),
                    StateId::parse("a").unwrap(),
                    Some(StateId::parse("a").unwrap()),
                    true,
                )
                .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(Default::default()),
            ..AttemptFacts::default()
        };
        let entry = JournalEntry::new(
            JournalSequence::try_from(2).unwrap(),
            RunId::parse("run").unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.request",
            RequestId::parse("request").unwrap(),
            OutcomeClass::Completed,
            None,
            state("a", 4),
            state("a", 4),
            Some(attempt),
            JournalExtension::TransitionAttempt,
            JournalEncodedSizes {
                entry: 100,
                evidence_associations: 2,
                ..JournalEncodedSizes::default()
            },
        )
        .unwrap();
        assert!(!entry.state_changed());
    }

    #[test]
    fn full_transition_attempt_retains_bounded_nested_facts() {
        let gate = GateId::parse("review").unwrap();
        let associations = EvidenceAssociations {
            selected_ids: vec![EvidenceId::parse("existing").unwrap()],
            ..EvidenceAssociations::default()
        };
        let attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    EventId::parse("go").unwrap(),
                    StateId::parse("a").unwrap(),
                    Some(StateId::parse("b").unwrap()),
                    true,
                )
                .unwrap(),
            ),
            provider_observations: vec![
                ProviderFact::new(
                    RegistrationId::parse("registration").unwrap(),
                    3,
                    ProviderRole::EvaluateGates,
                    RequestId::parse("invocation").unwrap(),
                    "/provider",
                    OutcomeClass::Completed,
                    DigestObservation::Unavailable,
                    Some("1.0.0".into()),
                    Some(1),
                )
                .unwrap(),
            ],
            gate_verdict_facts: Some(
                GateVerdictFacts::new(
                    EventId::parse("go").unwrap(),
                    vec![gate.clone()],
                    GateVerdictResult::Verdicts(vec![
                        GateVerdictFact::new(gate, true, None).unwrap(),
                    ]),
                )
                .unwrap(),
            ),
            evidence_recorded: Some(associations.recorded_status()),
            evidence_associations: Some(associations),
            note: Some(Note::new("attempted").unwrap()),
            actor: Some(
                ActorMetadata::new(Value::Object(BTreeMap::from([(
                    "kind".into(),
                    Value::String("agent".into()),
                )])))
                .unwrap(),
            ),
            corrects_sequence: None,
            diagnostics: vec![],
        };
        let entry = JournalEntry::new(
            JournalSequence::try_from(2).unwrap(),
            RunId::parse("run").unwrap(),
            ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            "run.request",
            RequestId::parse("request").unwrap(),
            OutcomeClass::Completed,
            None,
            state("a", 1),
            state("b", 2),
            Some(attempt),
            JournalExtension::TransitionAttempt,
            JournalEncodedSizes {
                entry: 1_000,
                evidence_associations: 10,
                provider_observations: 10,
                gate_verdict_facts: 10,
                diagnostics: 0,
                note: 9,
                actor: 16,
            },
        )
        .unwrap();
        let facts = entry.attempt().unwrap();
        assert_eq!(facts.provider_observations[0].config_revision, 3);
        assert!(facts.evidence_recorded.unwrap().selected_associations);
        assert!(facts.corrects_sequence.is_none());
    }

    #[test]
    fn rejection_requires_matching_reason_and_unchanged_state() {
        let reason = Reason::new(ReasonCode::EventUnknown, "unknown").unwrap();
        let attempt = AttemptFacts {
            transition: Some(
                TransitionFact::new(
                    EventId::parse("missing").unwrap(),
                    StateId::parse("a").unwrap(),
                    Some(StateId::parse("b").unwrap()),
                    false,
                )
                .unwrap(),
            ),
            evidence_associations: Some(EvidenceAssociations::default()),
            evidence_recorded: Some(Default::default()),
            ..AttemptFacts::default()
        };
        assert!(
            JournalEntry::new(
                JournalSequence::try_from(2).unwrap(),
                RunId::parse("run").unwrap(),
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
                "run.request",
                RequestId::parse("request").unwrap(),
                OutcomeClass::Rejected,
                Some(reason),
                state("a", 1),
                state("a", 1),
                Some(attempt),
                JournalExtension::TransitionAttempt,
                JournalEncodedSizes {
                    entry: 100,
                    evidence_associations: 2,
                    ..JournalEncodedSizes::default()
                },
            )
            .is_ok()
        );
    }
}
