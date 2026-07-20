use super::bounded::{
    ACTOR_METADATA_ENCODED_BYTES, BoundError, BoundedText, METADATA_NESTING_DEPTH,
    NOTE_TEXT_UTF8_BYTES, Value,
};
use super::version::JournalSequence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorMetadata(Value);

impl ActorMetadata {
    pub fn new(value: Value) -> Result<Self, BoundError> {
        if !matches!(value, Value::Object(_)) {
            return Err(BoundError::InvalidType {
                field: "actor_metadata_object",
            });
        }
        value.validate(
            "actor_metadata",
            METADATA_NESTING_DEPTH,
            ACTOR_METADATA_ENCODED_BYTES,
        )?;
        Ok(Self(value))
    }

    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note(BoundedText<NOTE_TEXT_UTF8_BYTES>);

impl Note {
    pub fn new(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self(BoundedText::non_empty("note", value)?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Annotation {
    actor: Option<ActorMetadata>,
    note: Option<Note>,
    corrects: Option<JournalSequence>,
}

impl Annotation {
    pub fn new(
        actor: Option<ActorMetadata>,
        note: Option<Note>,
        corrects: Option<JournalSequence>,
    ) -> Self {
        Self {
            actor,
            note,
            corrects,
        }
    }

    pub fn actor(&self) -> Option<&ActorMetadata> {
        self.actor.as_ref()
    }

    pub fn note(&self) -> Option<&Note> {
        self.note.as_ref()
    }

    pub fn corrects(&self) -> Option<JournalSequence> {
        self.corrects
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorMetadata, Annotation, Value};
    use std::collections::BTreeMap;

    #[test]
    fn actor_is_opaque_metadata_without_authority() {
        let value = Value::Object(BTreeMap::from([(
            "kind".into(),
            Value::String("agent".into()),
        )]));
        let actor = ActorMetadata::new(value.clone()).unwrap();
        let annotation = Annotation::new(Some(actor), None, None);
        assert_eq!(annotation.actor().unwrap().value(), &value);
        assert!(ActorMetadata::new(Value::String("authority".into())).is_err());
    }
}
