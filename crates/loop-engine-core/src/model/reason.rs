use super::bounded::{BoundError, BoundedText, DIAGNOSTIC_ENCODED_BYTES};
use super::outcome::OutcomeClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReasonCode {
    CatalogHandleDuplicate,
    CatalogHandleInvalid,
    CatalogHandleOccupied,
    CatalogRegistrationNotFound,
    CatalogConfigInvalid,
    CatalogAckTokenInvalid,
    CatalogAckTokenStale,
    CatalogActiveRunsChanged,
    ProviderTombstoned,
    ProviderRegistrationMissing,
    ProviderRegistrationStale,
    ProviderSpawnFailed,
    ProviderExecutableNotFound,
    ProviderProtocolUnsupportedMajor,
    ProviderProtocolMalformed,
    ProviderProtocolOversized,
    ProviderProtocolInvalidUtf8,
    ProviderTimeout,
    ProviderCrash,
    ProviderNonzeroExit,
    ProviderSignal,
    ProviderEvaluationError,
    ProviderGraphInvalid,
    ProviderDriftDetected,
    ProviderEvidenceMalformed,
    RunNotFound,
    RunLifecycleDenied,
    RunLifecycleTerminal,
    EventUnknown,
    GateFailed,
    CompatibilityUnsupported,
    GuidanceUnsupported,
    InputRejected,
    InputInvalid,
    EvidenceInvalid,
    EvidenceSelectionInvalid,
    LabelInvalid,
    NoteInvalid,
    ActorInvalid,
    StateStaleVersion,
    PersistenceFailed,
    CursorInvalid,
    ResourceExhausted,
    ExportTargetInvalid,
    ExportTargetNotEmpty,
}

impl ReasonCode {
    pub const ALL: [Self; 45] = [
        Self::CatalogHandleDuplicate,
        Self::CatalogHandleInvalid,
        Self::CatalogHandleOccupied,
        Self::CatalogRegistrationNotFound,
        Self::CatalogConfigInvalid,
        Self::CatalogAckTokenInvalid,
        Self::CatalogAckTokenStale,
        Self::CatalogActiveRunsChanged,
        Self::ProviderTombstoned,
        Self::ProviderRegistrationMissing,
        Self::ProviderRegistrationStale,
        Self::ProviderSpawnFailed,
        Self::ProviderExecutableNotFound,
        Self::ProviderProtocolUnsupportedMajor,
        Self::ProviderProtocolMalformed,
        Self::ProviderProtocolOversized,
        Self::ProviderProtocolInvalidUtf8,
        Self::ProviderTimeout,
        Self::ProviderCrash,
        Self::ProviderNonzeroExit,
        Self::ProviderSignal,
        Self::ProviderEvaluationError,
        Self::ProviderGraphInvalid,
        Self::ProviderDriftDetected,
        Self::ProviderEvidenceMalformed,
        Self::RunNotFound,
        Self::RunLifecycleDenied,
        Self::RunLifecycleTerminal,
        Self::EventUnknown,
        Self::GateFailed,
        Self::CompatibilityUnsupported,
        Self::GuidanceUnsupported,
        Self::InputRejected,
        Self::InputInvalid,
        Self::EvidenceInvalid,
        Self::EvidenceSelectionInvalid,
        Self::LabelInvalid,
        Self::NoteInvalid,
        Self::ActorInvalid,
        Self::StateStaleVersion,
        Self::PersistenceFailed,
        Self::CursorInvalid,
        Self::ResourceExhausted,
        Self::ExportTargetInvalid,
        Self::ExportTargetNotEmpty,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::CatalogHandleDuplicate => "catalog.handle.duplicate",
            Self::CatalogHandleInvalid => "catalog.handle.invalid",
            Self::CatalogHandleOccupied => "catalog.handle.occupied",
            Self::CatalogRegistrationNotFound => "catalog.registration.not_found",
            Self::CatalogConfigInvalid => "catalog.config.invalid",
            Self::CatalogAckTokenInvalid => "catalog.ack_token.invalid",
            Self::CatalogAckTokenStale => "catalog.ack_token.stale",
            Self::CatalogActiveRunsChanged => "catalog.active_runs.changed",
            Self::ProviderTombstoned => "provider.tombstoned",
            Self::ProviderRegistrationMissing => "provider.registration.missing",
            Self::ProviderRegistrationStale => "provider.registration.stale",
            Self::ProviderSpawnFailed => "provider.spawn.failed",
            Self::ProviderExecutableNotFound => "provider.executable.not_found",
            Self::ProviderProtocolUnsupportedMajor => "provider.protocol.unsupported_major",
            Self::ProviderProtocolMalformed => "provider.protocol.malformed",
            Self::ProviderProtocolOversized => "provider.protocol.oversized",
            Self::ProviderProtocolInvalidUtf8 => "provider.protocol.invalid_utf8",
            Self::ProviderTimeout => "provider.timeout",
            Self::ProviderCrash => "provider.crash",
            Self::ProviderNonzeroExit => "provider.nonzero_exit",
            Self::ProviderSignal => "provider.signal",
            Self::ProviderEvaluationError => "provider.evaluation_error",
            Self::ProviderGraphInvalid => "provider.graph.invalid",
            Self::ProviderDriftDetected => "provider.drift.detected",
            Self::ProviderEvidenceMalformed => "provider.evidence.malformed",
            Self::RunNotFound => "run.not_found",
            Self::RunLifecycleDenied => "run.lifecycle.denied",
            Self::RunLifecycleTerminal => "run.lifecycle.terminal",
            Self::EventUnknown => "event.unknown",
            Self::GateFailed => "gate.failed",
            Self::CompatibilityUnsupported => "compatibility.unsupported",
            Self::GuidanceUnsupported => "guidance.unsupported",
            Self::InputRejected => "input.rejected",
            Self::InputInvalid => "input.invalid",
            Self::EvidenceInvalid => "evidence.invalid",
            Self::EvidenceSelectionInvalid => "evidence.selection.invalid",
            Self::LabelInvalid => "label.invalid",
            Self::NoteInvalid => "note.invalid",
            Self::ActorInvalid => "actor.invalid",
            Self::StateStaleVersion => "state.stale_version",
            Self::PersistenceFailed => "persistence.failed",
            Self::CursorInvalid => "cursor.invalid",
            Self::ResourceExhausted => "resource.exhausted",
            Self::ExportTargetInvalid => "export.target.invalid",
            Self::ExportTargetNotEmpty => "export.target.not_empty",
        }
    }

    pub fn outcome_class(self) -> OutcomeClass {
        match self {
            Self::ProviderTombstoned
            | Self::ProviderRegistrationMissing
            | Self::ProviderRegistrationStale
            | Self::ProviderSpawnFailed
            | Self::ProviderExecutableNotFound
            | Self::ProviderProtocolUnsupportedMajor
            | Self::ProviderProtocolMalformed
            | Self::ProviderProtocolOversized
            | Self::ProviderProtocolInvalidUtf8
            | Self::ProviderTimeout
            | Self::ProviderCrash
            | Self::ProviderNonzeroExit
            | Self::ProviderSignal
            | Self::ProviderEvaluationError
            | Self::ProviderGraphInvalid
            | Self::ProviderDriftDetected
            | Self::ProviderEvidenceMalformed
            | Self::StateStaleVersion
            | Self::PersistenceFailed
            | Self::ResourceExhausted => OutcomeClass::Error,
            _ => OutcomeClass::Rejected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    code: ReasonCode,
    message: BoundedText<DIAGNOSTIC_ENCODED_BYTES>,
}

impl Reason {
    pub fn new(code: ReasonCode, message: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self {
            code,
            message: BoundedText::non_empty("reason_message", message)?,
        })
    }

    pub fn code(&self) -> ReasonCode {
        self.code
    }

    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::{OutcomeClass, ReasonCode};
    use std::collections::BTreeSet;

    #[test]
    fn every_frozen_reason_maps_exactly_once() {
        let expected = [
            "catalog.handle.duplicate",
            "catalog.handle.invalid",
            "catalog.handle.occupied",
            "catalog.registration.not_found",
            "catalog.config.invalid",
            "catalog.ack_token.invalid",
            "catalog.ack_token.stale",
            "catalog.active_runs.changed",
            "provider.tombstoned",
            "provider.registration.missing",
            "provider.registration.stale",
            "provider.spawn.failed",
            "provider.executable.not_found",
            "provider.protocol.unsupported_major",
            "provider.protocol.malformed",
            "provider.protocol.oversized",
            "provider.protocol.invalid_utf8",
            "provider.timeout",
            "provider.crash",
            "provider.nonzero_exit",
            "provider.signal",
            "provider.evaluation_error",
            "provider.graph.invalid",
            "provider.drift.detected",
            "provider.evidence.malformed",
            "run.not_found",
            "run.lifecycle.denied",
            "run.lifecycle.terminal",
            "event.unknown",
            "gate.failed",
            "compatibility.unsupported",
            "guidance.unsupported",
            "input.rejected",
            "input.invalid",
            "evidence.invalid",
            "evidence.selection.invalid",
            "label.invalid",
            "note.invalid",
            "actor.invalid",
            "state.stale_version",
            "persistence.failed",
            "cursor.invalid",
            "resource.exhausted",
            "export.target.invalid",
            "export.target.not_empty",
        ];
        let actual = ReasonCode::ALL.map(ReasonCode::code);
        assert_eq!(actual, expected);
        let unique = actual.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), expected.len());
        assert!(
            ReasonCode::ALL
                .iter()
                .all(|code| code.outcome_class() != OutcomeClass::Completed)
        );
    }
}
