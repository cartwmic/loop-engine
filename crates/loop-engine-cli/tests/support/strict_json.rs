//! Strict JSON parsing with duplicate-key rejection for CLI envelopes and trace JSONL.

use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrictJsonError {
    Malformed(String),
    TrailingContent,
    DuplicateKey { path: String, key: String },
}

impl std::fmt::Display for StrictJsonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => {
                write!(formatter, "JSON is malformed: {message}")
            }
            Self::TrailingContent => {
                formatter.write_str("JSON contains trailing content after first value")
            }
            Self::DuplicateKey { path, key } => write!(
                formatter,
                "JSON contains duplicate object key at {path}: {key}"
            ),
        }
    }
}

impl std::error::Error for StrictJsonError {}

fn is_trailing_content_error(message: &str) -> bool {
    message.contains("trailing characters")
}

pub fn parse_strict_json_value(text: &str) -> Result<Value, StrictJsonError> {
    let mut de = serde_json::Deserializer::from_str(text);
    let duplicates = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seed = StrictValueSeed {
        path: String::new(),
        duplicates: std::rc::Rc::clone(&duplicates),
    };
    let value = match seed.deserialize(&mut de) {
        Ok(value) => value,
        Err(error) => {
            let message = error.to_string();
            if is_trailing_content_error(&message) {
                return Err(StrictJsonError::TrailingContent);
            }
            return Err(StrictJsonError::Malformed(message));
        }
    };
    if de.end().is_err() {
        return Err(StrictJsonError::TrailingContent);
    }
    if let Some(duplicate) = duplicates.borrow().first() {
        return Err(StrictJsonError::DuplicateKey {
            path: duplicate.path.clone(),
            key: duplicate.key.clone(),
        });
    }
    Ok(value)
}

#[derive(Debug, Clone)]
struct DuplicateKey {
    path: String,
    key: String,
}

struct StrictValueSeed {
    path: String,
    duplicates: std::rc::Rc<std::cell::RefCell<Vec<DuplicateKey>>>,
}

struct StrictVisitor {
    path: String,
    duplicates: std::rc::Rc<std::cell::RefCell<Vec<DuplicateKey>>>,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let visitor = StrictVisitor {
            path: self.path,
            duplicates: self.duplicates,
        };
        deserializer.deserialize_any(visitor)
    }
}

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValueSeed {
            path: self.path,
            duplicates: self.duplicates,
        }
        .deserialize(deserializer)
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        let mut index = 0_usize;
        while let Some(value) = seq.next_element_seed(StrictValueSeed {
            path: format!("{}/{}", self.path, index),
            duplicates: std::rc::Rc::clone(&self.duplicates),
        })? {
            values.push(value);
            index += 1;
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let path = if self.path.is_empty() {
                key.clone()
            } else {
                format!("{}/{}", self.path, key)
            };
            let duplicate = values.contains_key(&key);
            let value = map.next_value_seed(StrictValueSeed {
                path,
                duplicates: std::rc::Rc::clone(&self.duplicates),
            })?;
            if duplicate {
                self.duplicates.borrow_mut().push(DuplicateKey {
                    path: self.path.clone(),
                    key,
                });
            } else {
                values.insert(key, value);
            }
        }
        Ok(Value::Object(values))
    }
}
