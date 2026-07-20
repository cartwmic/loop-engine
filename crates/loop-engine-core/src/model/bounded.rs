use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;

pub const IDENTIFIER_UTF8_BYTES: usize = 128;
pub const PROVIDER_HANDLE_UTF8_BYTES: usize = 128;
pub const RUN_LABEL_UTF8_BYTES: usize = 256;
pub const NOTE_TEXT_UTF8_BYTES: usize = 65_536;
pub const ACTOR_METADATA_ENCODED_BYTES: usize = 16_384;
pub const EVIDENCE_LOCATOR_UTF8_BYTES: usize = 8_192;
pub const FILESYSTEM_PATH_UTF8_BYTES: usize = 4_096;
pub const PROVIDER_ARGV_ELEMENT_COUNT: usize = 128;
pub const PROVIDER_ARGV_ELEMENT_UTF8_BYTES: usize = 16_384;
pub const PROVIDER_ARGV_ENCODED_TOTAL_BYTES: usize = 262_144;
pub const PROVIDER_TIMEOUT_SECONDS_DEFAULT: u64 = 60;
pub const GUIDANCE_TEXT_BYTES: usize = 262_144;
pub const DIAGNOSTIC_ENCODED_BYTES: usize = 8_192;
pub const DIAGNOSTICS_PER_RESULT_COUNT: usize = 100;
pub const METADATA_NESTING_DEPTH: usize = 16;
pub const RUN_INPUTS_ENCODED_TOTAL_BYTES: usize = 1_048_576;
pub const INLINE_EVIDENCE_CONTEXT_TOTAL_BYTES: usize = 1_048_576;
pub const SELECTED_EVIDENCE_CONTEXT_TOTAL_BYTES: usize = 1_048_576;
pub const PROVIDER_SNAPSHOT_ENVELOPE_BYTES: usize = 524_288;
pub const EVIDENCE_RECORD_ENCODED_BYTES: usize = 65_536;
pub const JOURNAL_EVIDENCE_ASSOCIATIONS_ENCODED_BYTES: usize = 262_144;
pub const JOURNAL_PROVIDER_FACTS_ENCODED_BYTES: usize = 262_144;
pub const JOURNAL_GATE_VERDICT_FACTS_ENCODED_BYTES: usize = 524_288;
pub const JOURNAL_ENTRY_ENCODED_BYTES: usize = 2_621_440;
pub const COLLECTION_PAGE_DEFAULT_COUNT: u16 = 100;
pub const COLLECTION_PAGE_MAX_COUNT: u16 = 1_000;
pub const OPAQUE_INTEGRITY_WIRE_UTF8_BYTES: usize = 768;
pub const COLLECTION_PAGE_DATA_BUDGET_BYTES: usize = 3_145_728;
pub const PROVIDER_CALLS_PER_PAGED_INVOCATION_MAX: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BoundError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} exceeds {max} UTF-8 bytes (actual {actual})")]
    TooLong {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field} contains a prohibited control character")]
    Control { field: &'static str },
    #[error("{field} exceeds metadata nesting depth {max}")]
    TooDeep { field: &'static str, max: usize },
    #[error("{field} exceeds encoded size {max} bytes (actual {actual})")]
    EncodedTooLarge {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field} exceeds item count {max} (actual {actual})")]
    TooMany {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field} must be a finite IEEE 754 binary64 number")]
    NonFiniteNumber { field: &'static str },
    #[error("{field} has invalid value type")]
    InvalidType { field: &'static str },
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundedText<const MAX: usize>(String);

impl<const MAX: usize> BoundedText<MAX> {
    pub fn new(field: &'static str, value: impl Into<String>) -> Result<Self, BoundError> {
        let value = value.into();
        if value.len() > MAX {
            return Err(BoundError::TooLong {
                field,
                max: MAX,
                actual: value.len(),
            });
        }
        Ok(Self(value))
    }

    pub fn non_empty(field: &'static str, value: impl Into<String>) -> Result<Self, BoundError> {
        let value = value.into();
        if value.is_empty() {
            return Err(BoundError::Empty { field });
        }
        Self::new(field, value)
    }

    pub fn opaque(field: &'static str, value: impl Into<String>) -> Result<Self, BoundError> {
        let text = Self::new(field, value)?;
        if text.0.chars().any(char::is_control) {
            return Err(BoundError::Control { field });
        }
        Ok(text)
    }

    pub fn opaque_non_empty(
        field: &'static str,
        value: impl Into<String>,
    ) -> Result<Self, BoundError> {
        let text = Self::non_empty(field, value)?;
        if text.0.chars().any(char::is_control) {
            return Err(BoundError::Control { field });
        }
        Ok(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const MAX: usize> fmt::Debug for BoundedText<MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<const MAX: usize> fmt::Display for BoundedText<MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiniteNumber(u64);

impl FiniteNumber {
    pub fn new(field: &'static str, value: f64) -> Result<Self, BoundError> {
        if !value.is_finite() {
            return Err(BoundError::NonFiniteNumber { field });
        }
        let normalized = if value == 0.0 { 0.0 } else { value };
        Ok(Self(normalized.to_bits()))
    }

    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// Core-owned JSON-like value after strict outer parsing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Null,
    Bool(bool),
    Number(FiniteNumber),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn validate(
        &self,
        field: &'static str,
        max_depth: usize,
        max_encoded_bytes: usize,
    ) -> Result<(), BoundError> {
        let depth = self.depth();
        if depth > max_depth {
            return Err(BoundError::TooDeep {
                field,
                max: max_depth,
            });
        }
        let size = self.json_encoded_size();
        if size > max_encoded_bytes {
            return Err(BoundError::EncodedTooLarge {
                field,
                max: max_encoded_bytes,
                actual: size,
            });
        }
        Ok(())
    }

    pub fn depth(&self) -> usize {
        match self {
            Self::Array(values) => 1 + values.iter().map(Self::depth).max().unwrap_or(0),
            Self::Object(values) => 1 + values.values().map(Self::depth).max().unwrap_or(0),
            _ => 1,
        }
    }

    pub fn json_encoded_size(&self) -> usize {
        match self {
            Self::Null => 4,
            Self::Bool(true) => 4,
            Self::Bool(false) => 5,
            // Any finite binary64 value has a JCS representation shorter than this
            // conservative bound. Integrations still own exact canonical encoding.
            Self::Number(_) => 24,
            Self::String(value) => json_string_encoded_size(value),
            Self::Array(values) => values
                .iter()
                .map(Self::json_encoded_size)
                .sum::<usize>()
                .saturating_add(values.len().saturating_sub(1))
                .saturating_add(2),
            Self::Object(values) => values
                .iter()
                .map(|(key, value)| {
                    json_string_encoded_size(key)
                        .saturating_add(1)
                        .saturating_add(value.json_encoded_size())
                })
                .sum::<usize>()
                .saturating_add(values.len().saturating_sub(1))
                .saturating_add(2),
        }
    }
}

pub(crate) fn json_string_encoded_size(value: &str) -> usize {
    2 + value
        .chars()
        .map(|character| match character {
            '"' | '\\' | '\u{0008}' | '\u{0009}' | '\u{000a}' | '\u{000c}' | '\u{000d}' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            other => other.len_utf8(),
        })
        .sum::<usize>()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Metadata(BTreeMap<String, Value>);

impl Metadata {
    pub fn new(
        field: &'static str,
        values: BTreeMap<String, Value>,
        max_encoded_bytes: usize,
    ) -> Result<Option<Self>, BoundError> {
        if values.is_empty() {
            return Ok(None);
        }
        let value = Value::Object(values.clone());
        value.validate(field, METADATA_NESTING_DEPTH, max_encoded_bytes)?;
        Ok(Some(Self(values)))
    }

    pub fn values(&self) -> &BTreeMap<String, Value> {
        &self.0
    }

    pub fn json_encoded_size(&self) -> usize {
        Value::Object(self.0.clone()).json_encoded_size()
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundError, BoundedText, FiniteNumber, Metadata, Value};
    use std::collections::BTreeMap;

    #[test]
    fn text_bounds_count_utf8_bytes_without_truncation() {
        assert!(BoundedText::<4>::new("text", "éé").is_ok());
        assert!(matches!(
            BoundedText::<3>::new("text", "éé"),
            Err(BoundError::TooLong { actual: 4, .. })
        ));
    }

    #[test]
    fn metadata_checks_depth_exact_escaping_and_encoded_size() {
        let deep = Value::Array(vec![Value::Array(vec![Value::Null])]);
        assert!(matches!(
            deep.validate("metadata", 2, 100),
            Err(BoundError::TooDeep { .. })
        ));
        let escaped = Value::String("\u{0001}".repeat(2));
        assert_eq!(escaped.json_encoded_size(), 14);
        assert!(matches!(
            escaped.validate("metadata", 16, 13),
            Err(BoundError::EncodedTooLarge { .. })
        ));
    }

    #[test]
    fn metadata_normalizes_empty_objects_and_number_lexical_equivalence() {
        assert_eq!(
            Metadata::new("metadata", BTreeMap::new(), 100).unwrap(),
            None
        );
        assert_eq!(
            FiniteNumber::new("number", -0.0).unwrap(),
            FiniteNumber::new("number", 0.0).unwrap()
        );
        assert_eq!(
            FiniteNumber::new("number", 1.0).unwrap(),
            FiniteNumber::new("number", 1e0).unwrap()
        );
        assert!(FiniteNumber::new("number", f64::NAN).is_err());
    }
}
