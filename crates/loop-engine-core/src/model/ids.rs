use std::fmt;

use thiserror::Error;

use super::bounded::{BoundError, BoundedText, IDENTIFIER_UTF8_BYTES, PROVIDER_HANDLE_UTF8_BYTES};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentifierError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error("provider handle must match lowercase ASCII handle grammar")]
    InvalidProviderHandle,
    #[error("graph revision must be sha256: followed by 64 lowercase hexadecimal characters")]
    InvalidGraphRevision,
}

macro_rules! identifier {
    ($name:ident, $field:literal) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(BoundedText<IDENTIFIER_UTF8_BYTES>);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
                Ok(Self(BoundedText::opaque_non_empty($field, value)?))
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

identifier!(RegistrationId, "registration_id");
identifier!(RunId, "run_id");
identifier!(RequestId, "request_id");
identifier!(StateId, "state_id");
identifier!(EventId, "event_id");
identifier!(GateId, "gate_id");
identifier!(EvidenceId, "evidence_id");
identifier!(JournalId, "journal_id");
identifier!(InputName, "input_name");
identifier!(InputKind, "input_kind");
identifier!(EvidenceKind, "evidence_kind");

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphRevision(String);

impl GraphRevision {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let digest = value.strip_prefix("sha256:");
        if !digest.is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(IdentifierError::InvalidGraphRevision);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GraphRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl fmt::Display for GraphRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderHandle(BoundedText<PROVIDER_HANDLE_UTF8_BYTES>);

impl ProviderHandle {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid_alnum = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let valid_middle = |byte: u8| valid_alnum(byte) || matches!(byte, b'.' | b'_' | b'-');
        if bytes.is_empty()
            || bytes.len() > PROVIDER_HANDLE_UTF8_BYTES
            || !valid_alnum(bytes[0])
            || !valid_alnum(bytes[bytes.len() - 1])
            || (bytes.len() > 2 && !bytes[1..bytes.len() - 1].iter().copied().all(valid_middle))
        {
            return Err(IdentifierError::InvalidProviderHandle);
        }
        Ok(Self(BoundedText::non_empty("provider_handle", value)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for ProviderHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl fmt::Display for ProviderHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::{EventId, GraphRevision, JournalId, ProviderHandle, RunId};

    #[test]
    fn identifiers_are_distinct_and_bounded() {
        let run = RunId::parse("same").unwrap();
        let event = EventId::parse("same").unwrap();
        assert_eq!(run.as_str(), event.as_str());
        assert!(RunId::parse("").is_err());
        assert!(RunId::parse("x".repeat(129)).is_err());
        assert_eq!(JournalId::parse("journal").unwrap().as_str(), "journal");
    }

    #[test]
    fn graph_revision_uses_exact_digest_grammar() {
        assert!(GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).is_ok());
        for invalid in [
            "revision".to_owned(),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "a".repeat(63)),
            format!("sha512:{}", "a".repeat(64)),
        ] {
            assert!(GraphRevision::parse(invalid).is_err());
        }
    }

    #[test]
    fn provider_handle_exact_grammar() {
        for length in [1, 2, 3, 128] {
            assert!(ProviderHandle::parse("a".repeat(length)).is_ok());
        }
        for invalid in ["", "A", "é", "-a", "a-", "a/b", "a b"] {
            assert!(ProviderHandle::parse(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            ProviderHandle::parse("a.b_c-d").unwrap().as_str(),
            "a.b_c-d"
        );
    }
}
