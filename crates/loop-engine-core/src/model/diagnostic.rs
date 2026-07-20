use super::bounded::{
    BoundError, BoundedText, DIAGNOSTIC_ENCODED_BYTES, DIAGNOSTICS_PER_RESULT_COUNT,
    json_string_encoded_size,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    code: BoundedText<256>,
    message: BoundedText<DIAGNOSTIC_ENCODED_BYTES>,
    path: Option<BoundedText<DIAGNOSTIC_ENCODED_BYTES>>,
}

impl Diagnostic {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        path: Option<String>,
    ) -> Result<Self, BoundError> {
        let diagnostic = Self {
            code: BoundedText::opaque_non_empty("diagnostic_code", code)?,
            message: BoundedText::non_empty("diagnostic_message", message)?,
            path: path
                .map(|value| BoundedText::non_empty("diagnostic_path", value))
                .transpose()?,
        };
        let encoded = diagnostic.encoded_size_upper_bound();
        if encoded > DIAGNOSTIC_ENCODED_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "diagnostic",
                max: DIAGNOSTIC_ENCODED_BYTES,
                actual: encoded,
            });
        }
        Ok(diagnostic)
    }

    pub fn code(&self) -> &str {
        self.code.as_str()
    }

    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_ref().map(BoundedText::as_str)
    }

    pub fn encoded_size_upper_bound(&self) -> usize {
        35usize
            .saturating_add(json_string_encoded_size(self.code.as_str()))
            .saturating_add(json_string_encoded_size(self.message.as_str()))
            .saturating_add(
                self.path
                    .as_ref()
                    .map_or(0, |path| 10 + json_string_encoded_size(path.as_str())),
            )
    }
}

pub fn validate_diagnostics(diagnostics: &[Diagnostic]) -> Result<(), BoundError> {
    if diagnostics.len() > DIAGNOSTICS_PER_RESULT_COUNT {
        return Err(BoundError::TooMany {
            field: "diagnostics",
            max: DIAGNOSTICS_PER_RESULT_COUNT,
            actual: diagnostics.len(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    pub fn new(values: Vec<Diagnostic>) -> Result<Self, BoundError> {
        validate_diagnostics(&values)?;
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, validate_diagnostics};

    #[test]
    fn preserves_optional_path_and_bounds_count() {
        let diagnostic = Diagnostic::new("bad", "message", Some("/inputs/name".into())).unwrap();
        assert_eq!(diagnostic.path(), Some("/inputs/name"));
        let diagnostics = vec![Diagnostic::new("bad", "message", None).unwrap(); 101];
        assert!(validate_diagnostics(&diagnostics).is_err());
    }
}
