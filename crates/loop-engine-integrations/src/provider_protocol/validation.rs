use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use serde::Deserializer;
use serde::de::{DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};
use thiserror::Error;

pub const PROVIDER_REQUEST_JSON_BYTES: usize = 4_194_304;
pub const PROVIDER_RESULT_STDOUT_BYTES: usize = 1_048_576;
pub const GRAPH_PROJECTION_CANONICAL_BYTES: usize = 524_288;

#[derive(Debug, Error)]
pub enum ProtocolValidationError {
    #[error("provider JSON exceeds {max} bytes (actual {actual})")]
    Oversized { max: usize, actual: usize },
    #[error("provider JSON is invalid UTF-8")]
    InvalidUtf8,
    #[error("provider JSON is malformed: {0}")]
    Malformed(String),
    #[error("provider JSON contains duplicate object key at {path}: {key}")]
    DuplicateKey { path: String, key: String },
    #[error("provider result contains forbidden topology field: {0}")]
    ForbiddenTopology(String),
}

#[derive(Debug, Clone)]
struct DuplicateKey {
    path: String,
    key: String,
}

struct StrictValueSeed {
    path: String,
    duplicates: Rc<RefCell<Vec<DuplicateKey>>>,
}

struct StrictVisitor {
    path: String,
    duplicates: Rc<RefCell<Vec<DuplicateKey>>>,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictVisitor {
            path: self.path,
            duplicates: self.duplicates,
        })
    }
}

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
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

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        let mut index = 0_usize;
        loop {
            let seed = StrictValueSeed {
                path: format!("{}/{}", self.path, index),
                duplicates: Rc::clone(&self.duplicates),
            };
            let Some(value) = sequence.next_element_seed(seed)? else {
                break;
            };
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
            if values.contains_key(&key) {
                self.duplicates.borrow_mut().push(DuplicateKey {
                    path: pointer_path(&self.path),
                    key: key.clone(),
                });
            }
            let seed = StrictValueSeed {
                path: format!("{}/{}", self.path, pointer_segment(&key)),
                duplicates: Rc::clone(&self.duplicates),
            };
            let value = map.next_value_seed(seed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

fn pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn pointer_path(value: &str) -> String {
    if value.is_empty() {
        "/".to_owned()
    } else {
        value.to_owned()
    }
}

pub fn parse_strict_value(bytes: &[u8], max: usize) -> Result<Value, ProtocolValidationError> {
    let (value, duplicates) = parse_value_collecting_duplicates(bytes, max)?;
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(ProtocolValidationError::DuplicateKey {
            path: duplicate.path,
            key: duplicate.key,
        });
    }
    Ok(value)
}

pub(crate) fn parse_provider_value(
    bytes: &[u8],
    max: usize,
    allow_graph_duplicates: bool,
) -> Result<(Value, Option<String>), ProtocolValidationError> {
    let (value, duplicates) = parse_value_collecting_duplicates(bytes, max)?;
    if duplicates.is_empty() {
        return Ok((value, None));
    }
    if allow_graph_duplicates
        && duplicates
            .iter()
            .all(|duplicate| duplicate.path.starts_with("/result/graph"))
    {
        let duplicate = &duplicates[0];
        return Ok((
            value,
            Some(format!(
                "duplicate object key at {}: {}",
                duplicate.path, duplicate.key
            )),
        ));
    }
    let duplicate = duplicates.into_iter().next().expect("checked non-empty");
    Err(ProtocolValidationError::DuplicateKey {
        path: duplicate.path,
        key: duplicate.key,
    })
}

fn parse_value_collecting_duplicates(
    bytes: &[u8],
    max: usize,
) -> Result<(Value, Vec<DuplicateKey>), ProtocolValidationError> {
    if bytes.len() > max {
        return Err(ProtocolValidationError::Oversized {
            max,
            actual: bytes.len(),
        });
    }
    std::str::from_utf8(bytes).map_err(|_| ProtocolValidationError::InvalidUtf8)?;
    let duplicates = Rc::new(RefCell::new(Vec::new()));
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValueSeed {
        path: String::new(),
        duplicates: Rc::clone(&duplicates),
    }
    .deserialize(&mut deserializer)
    .map_err(|error| ProtocolValidationError::Malformed(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ProtocolValidationError::Malformed(error.to_string()))?;
    let duplicates = Rc::try_unwrap(duplicates)
        .expect("strict JSON visitor releases duplicate collector")
        .into_inner();
    Ok((value, duplicates))
}

pub fn parse_strict<T: DeserializeOwned>(
    bytes: &[u8],
    max: usize,
) -> Result<(T, Value), ProtocolValidationError> {
    let value = parse_strict_value(bytes, max)?;
    let dto = serde_json::from_value(value.clone())
        .map_err(|error| ProtocolValidationError::Malformed(error.to_string()))?;
    Ok((dto, value))
}

pub fn reject_topology_fields(value: &Value) -> Result<(), ProtocolValidationError> {
    const FORBIDDEN: &[&str] = &[
        "graph",
        "states",
        "transitions",
        "input_declarations",
        "initial_state",
        "live_guidance_supported",
        "current_state",
        "target_state",
        "state",
        "event",
        "lifecycle",
    ];
    let object = value.as_object();
    if let Some(key) =
        object.and_then(|values| values.keys().find(|key| FORBIDDEN.contains(&key.as_str())))
    {
        return Err(ProtocolValidationError::ForbiddenTopology(format!(
            "/{key}"
        )));
    }
    if let Some(key) = object
        .and_then(|values| values.get("values"))
        .and_then(Value::as_object)
        .and_then(|values| values.keys().find(|key| FORBIDDEN.contains(&key.as_str())))
    {
        return Err(ProtocolValidationError::ForbiddenTopology(format!(
            "/values/{key}"
        )));
    }
    Ok(())
}

pub fn object_to_btree(value: serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    value.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{
        PROVIDER_RESULT_STDOUT_BYTES, ProtocolValidationError, parse_strict_value,
        reject_topology_fields,
    };

    #[test]
    fn strict_parser_rejects_duplicates_at_any_depth_and_trailing_values() {
        for raw in [br#"{"a":1,"a":2}"#.as_slice(), br#"{"a":{"b":1,"b":2}}"#] {
            assert!(matches!(
                parse_strict_value(raw, PROVIDER_RESULT_STDOUT_BYTES),
                Err(ProtocolValidationError::DuplicateKey { .. })
            ));
        }
        assert!(parse_strict_value(br#"{} {}"#, PROVIDER_RESULT_STDOUT_BYTES).is_err());
    }

    #[test]
    fn input_validation_output_cannot_hide_topology() {
        let value: Value = serde_json::from_str(r#"{"kind":"accepted","graph":{}}"#).unwrap();
        assert!(reject_topology_fields(&value).is_err());
        let value: Value = serde_json::from_str(r#"{"kind":"accepted","future":true}"#).unwrap();
        assert!(reject_topology_fields(&value).is_ok());
    }
}
