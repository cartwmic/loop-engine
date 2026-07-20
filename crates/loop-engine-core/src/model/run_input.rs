use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::bounded::{
    BoundError, METADATA_NESTING_DEPTH, Metadata, RUN_INPUTS_ENCODED_TOTAL_BYTES, Value,
};
use super::ids::{IdentifierError, InputKind, InputName};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputDeclaration {
    name: InputName,
    kind: InputKind,
    required: bool,
    metadata: Option<Metadata>,
}

impl InputDeclaration {
    pub fn new(
        name: InputName,
        kind: InputKind,
        required: bool,
        metadata: Option<Metadata>,
    ) -> Self {
        Self {
            name,
            kind,
            required,
            metadata,
        }
    }

    pub fn name(&self) -> &InputName {
        &self.name
    }

    pub fn kind(&self) -> &InputKind {
        &self.kind
    }

    pub fn required(&self) -> bool {
        self.required
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InputError {
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error("duplicate input declaration: {0}")]
    DuplicateDeclaration(String),
    #[error("duplicate input value: {0}")]
    DuplicateValue(String),
    #[error("undeclared input value: {0}")]
    Undeclared(String),
    #[error("missing required input: {0}")]
    Missing(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputDeclarations(Vec<InputDeclaration>);

impl InputDeclarations {
    pub fn new(declarations: Vec<InputDeclaration>) -> Result<Self, InputError> {
        let mut names = BTreeSet::new();
        for declaration in &declarations {
            if !names.insert(declaration.name.clone()) {
                return Err(InputError::DuplicateDeclaration(
                    declaration.name.to_string(),
                ));
            }
        }
        Ok(Self::new_unvalidated(declarations))
    }

    pub fn new_unvalidated(declarations: Vec<InputDeclaration>) -> Self {
        Self(declarations)
    }

    pub fn values(&self) -> impl Iterator<Item = &InputDeclaration> {
        self.0.iter()
    }

    pub fn validate(&self, candidate: Vec<(InputName, Value)>) -> Result<RunInputs, InputError> {
        let mut seen = BTreeSet::new();
        let mut values = BTreeMap::new();
        for (name, value) in candidate {
            if !seen.insert(name.clone()) {
                return Err(InputError::DuplicateValue(name.to_string()));
            }
            if !self.0.iter().any(|declaration| declaration.name == name) {
                return Err(InputError::Undeclared(name.to_string()));
            }
            value.validate(
                "run_inputs",
                METADATA_NESTING_DEPTH,
                RUN_INPUTS_ENCODED_TOTAL_BYTES,
            )?;
            values.insert(name, value);
        }
        for declaration in self.0.iter().filter(|value| value.required) {
            if !values.contains_key(&declaration.name) {
                return Err(InputError::Missing(declaration.name.to_string()));
            }
        }
        let total = values
            .iter()
            .map(|(name, value)| name.as_str().len() + 3 + value.json_encoded_size())
            .sum::<usize>()
            .saturating_add(values.len().saturating_sub(1))
            .saturating_add(2);
        if total > RUN_INPUTS_ENCODED_TOTAL_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "run_inputs",
                max: RUN_INPUTS_ENCODED_TOTAL_BYTES,
                actual: total,
            }
            .into());
        }
        Ok(RunInputs(values))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunInputs(BTreeMap<InputName, Value>);

impl RunInputs {
    pub fn values(&self) -> &BTreeMap<InputName, Value> {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{InputDeclaration, InputDeclarations, InputError, InputKind, InputName, Value};

    fn declaration(name: &str, required: bool) -> InputDeclaration {
        InputDeclaration::new(
            InputName::parse(name).unwrap(),
            InputKind::parse("text").unwrap(),
            required,
            None,
        )
    }

    #[test]
    fn validation_rejects_missing_undeclared_and_duplicates() {
        let declarations = InputDeclarations::new(vec![declaration("required", true)]).unwrap();
        assert!(matches!(
            declarations.validate(vec![]),
            Err(InputError::Missing(_))
        ));
        assert!(matches!(
            declarations.validate(vec![(InputName::parse("other").unwrap(), Value::Null)]),
            Err(InputError::Undeclared(_))
        ));
        let name = InputName::parse("required").unwrap();
        assert!(matches!(
            declarations.validate(vec![(name.clone(), Value::Null), (name, Value::Null)]),
            Err(InputError::DuplicateValue(_))
        ));
    }
}
