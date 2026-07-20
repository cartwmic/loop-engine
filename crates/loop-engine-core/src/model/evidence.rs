use super::bounded::{
    BoundError, BoundedText, EVIDENCE_LOCATOR_UTF8_BYTES, EVIDENCE_RECORD_ENCODED_BYTES, Metadata,
};
use super::ids::{EventId, EvidenceId, EvidenceKind, GateId};
use super::time::ObservedAt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceSource {
    Caller,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    id: EvidenceId,
    kind: EvidenceKind,
    locator: BoundedText<EVIDENCE_LOCATOR_UTF8_BYTES>,
    digest: Option<BoundedText<256>>,
    media_type: Option<BoundedText<256>>,
    metadata: Option<Metadata>,
    source: EvidenceSource,
    observed_at: ObservedAt,
}

impl EvidenceRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: EvidenceId,
        kind: EvidenceKind,
        locator: impl Into<String>,
        digest: Option<String>,
        media_type: Option<String>,
        metadata: Option<Metadata>,
        source: EvidenceSource,
        observed_at: ObservedAt,
    ) -> Result<Self, BoundError> {
        let locator = BoundedText::opaque_non_empty("evidence_locator", locator)?;
        let digest = digest
            .map(|value| BoundedText::opaque_non_empty("evidence_digest", value))
            .transpose()?;
        let media_type = media_type
            .map(|value| BoundedText::opaque_non_empty("evidence_media_type", value))
            .transpose()?;
        let record = Self {
            id,
            kind,
            locator,
            digest,
            media_type,
            metadata,
            source,
            observed_at,
        };
        if record.encoded_size_estimate() > EVIDENCE_RECORD_ENCODED_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "evidence_record",
                max: EVIDENCE_RECORD_ENCODED_BYTES,
                actual: record.encoded_size_estimate(),
            });
        }
        Ok(record)
    }

    pub fn id(&self) -> &EvidenceId {
        &self.id
    }

    pub fn kind(&self) -> &EvidenceKind {
        &self.kind
    }

    pub fn locator(&self) -> &str {
        self.locator.as_str()
    }

    pub fn digest(&self) -> Option<&str> {
        self.digest.as_ref().map(|value| value.as_str())
    }

    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_ref().map(|value| value.as_str())
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    pub fn source(&self) -> EvidenceSource {
        self.source
    }

    pub fn observed_at(&self) -> ObservedAt {
        self.observed_at
    }

    pub fn encoded_size_estimate(&self) -> usize {
        // Conservative escaped JSON upper bound. Integrations enforce exact bytes.
        self.id.as_str().len().saturating_mul(6)
            + self.kind.as_str().len().saturating_mul(6)
            + self.locator.as_str().len().saturating_mul(6)
            + self
                .digest
                .as_ref()
                .map_or(0, |value| value.as_str().len().saturating_mul(6))
            + self
                .media_type
                .as_ref()
                .map_or(0, |value| value.as_str().len().saturating_mul(6))
            + self
                .metadata
                .as_ref()
                .map_or(0, Metadata::json_encoded_size)
            + 256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAssociation {
    evidence_id: EvidenceId,
    event_id: Option<EventId>,
    gate_id: Option<GateId>,
}

impl EvidenceAssociation {
    pub fn new(
        evidence_id: EvidenceId,
        event_id: Option<EventId>,
        gate_id: Option<GateId>,
    ) -> Self {
        Self {
            evidence_id,
            event_id,
            gate_id,
        }
    }

    pub fn evidence_id(&self) -> &EvidenceId {
        &self.evidence_id
    }

    pub fn event_id(&self) -> Option<&EventId> {
        self.event_id.as_ref()
    }

    pub fn gate_id(&self) -> Option<&GateId> {
        self.gate_id.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::{EvidenceId, EvidenceKind, EvidenceRecord, EvidenceSource, ObservedAt};

    fn at() -> ObservedAt {
        ObservedAt::parse("2026-07-18T00:00:00Z").unwrap()
    }

    #[test]
    fn locator_is_opaque_and_rejects_controls() {
        let record = EvidenceRecord::new(
            EvidenceId::parse("e1").unwrap(),
            EvidenceKind::parse("artifact").unwrap(),
            "../provider/relative:item",
            None,
            None,
            None,
            EvidenceSource::Caller,
            at(),
        )
        .unwrap();
        assert_eq!(record.locator(), "../provider/relative:item");
        for invalid in ["", "nul\0byte", "line\nbreak"] {
            assert!(
                EvidenceRecord::new(
                    EvidenceId::parse("e1").unwrap(),
                    EvidenceKind::parse("artifact").unwrap(),
                    invalid,
                    None,
                    None,
                    None,
                    EvidenceSource::Caller,
                    at(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn same_locator_can_have_distinct_revisions() {
        let make = |id| {
            EvidenceRecord::new(
                EvidenceId::parse(id).unwrap(),
                EvidenceKind::parse("artifact").unwrap(),
                "opaque",
                None,
                None,
                None,
                EvidenceSource::Caller,
                at(),
            )
            .unwrap()
        };
        assert_ne!(make("e1"), make("e2"));
    }
}
