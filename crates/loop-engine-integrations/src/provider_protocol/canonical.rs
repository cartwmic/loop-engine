use std::collections::BTreeMap;

use loop_engine_core::model::bounded::{Metadata, Value as CoreValue};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use serde_json::Value;
use thiserror::Error;

use super::dto::{
    CanonicalGraphDto, CanonicalGuidanceDto, CanonicalInputDto, CanonicalStateDto,
    CanonicalTransitionDto,
};
use super::validation::GRAPH_PROJECTION_CANONICAL_BYTES;

#[derive(Debug, Error)]
pub enum CanonicalError {
    #[error("canonical graph encoding failed: {0}")]
    Encoding(String),
    #[error("canonical graph exceeds {max} bytes (actual {actual})")]
    Oversized { max: usize, actual: usize },
}

pub fn graph_dto(projection: &SemanticGraphProjection) -> CanonicalGraphDto {
    CanonicalGraphDto {
        canonical_graph_version: projection.canonical_graph_version,
        initial_state_id: projection.initial_state.as_str().to_owned(),
        input_declarations: projection
            .inputs
            .iter()
            .map(|input| CanonicalInputDto {
                id: input.name.as_str().to_owned(),
                kind: input.kind.as_str().to_owned(),
                metadata: input.metadata.as_ref().map(metadata_value),
                required: input.required,
            })
            .collect(),
        live_guidance_supported: matches!(
            projection.live_guidance,
            LiveGuidanceCapability::Supported
        ),
        metadata: projection.metadata.as_ref().map(metadata_value),
        states: projection
            .states
            .iter()
            .map(|state| CanonicalStateDto {
                final_state: state.final_state,
                id: state.id.as_str().to_owned(),
                metadata: state.metadata.as_ref().map(metadata_value),
                static_guidance: match &state.guidance {
                    StaticGuidance::Text(text) => CanonicalGuidanceDto::Text {
                        text: text.as_str().to_owned(),
                    },
                    StaticGuidance::NoneRequired => CanonicalGuidanceDto::None,
                },
            })
            .collect(),
        transitions: projection
            .transitions
            .iter()
            .map(|transition| CanonicalTransitionDto {
                event_id: transition.event.as_str().to_owned(),
                gate_ids: transition
                    .required_gates
                    .iter()
                    .map(|gate| gate.as_str().to_owned())
                    .collect(),
                metadata: transition.metadata.as_ref().map(metadata_value),
                source_state_id: transition.source.as_str().to_owned(),
                target_state_id: transition.target.as_str().to_owned(),
            })
            .collect(),
    }
}

pub fn graph_bytes(projection: &SemanticGraphProjection) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(graph_dto(projection))
        .map_err(|error| CanonicalError::Encoding(error.to_string()))?;
    let mut output = String::new();
    write_value(&value, &mut output)?;
    let bytes = output.into_bytes();
    if bytes.len() > GRAPH_PROJECTION_CANONICAL_BYTES {
        return Err(CanonicalError::Oversized {
            max: GRAPH_PROJECTION_CANONICAL_BYTES,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

pub fn write_value(value: &Value, output: &mut String) -> Result<(), CanonicalError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => {
            let value = number
                .as_f64()
                .ok_or_else(|| CanonicalError::Encoding("number outside binary64 domain".into()))?;
            output.push_str(&format_jcs_number(value));
        }
        Value::String(value) => output.push_str(
            &serde_json::to_string(value)
                .map_err(|error| CanonicalError::Encoding(error.to_string()))?,
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_value(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key)
                        .map_err(|error| CanonicalError::Encoding(error.to_string()))?,
                );
                output.push(':');
                write_value(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

pub(crate) fn metadata_value(metadata: &Metadata) -> BTreeMap<String, Value> {
    metadata
        .values()
        .iter()
        .map(|(key, value)| (key.clone(), value_from_core(value)))
        .collect()
}

pub fn value_from_core(value: &CoreValue) -> Value {
    match value {
        CoreValue::Null => Value::Null,
        CoreValue::Bool(value) => Value::Bool(*value),
        CoreValue::Number(value) => serde_json::Number::from_f64(value.value())
            .map(Value::Number)
            .expect("core finite number must be JSON representable"),
        CoreValue::String(value) => Value::String(value.clone()),
        CoreValue::Array(values) => Value::Array(values.iter().map(value_from_core).collect()),
        CoreValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value_from_core(value)))
                .collect(),
        ),
    }
}

fn format_jcs_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let raw = serde_json::to_string(&value).expect("finite number serializes");
    if let Some((mantissa, exponent)) = raw.split_once('e') {
        let exponent: i32 = exponent.parse().expect("serde exponent is valid");
        let negative = mantissa.starts_with('-');
        let unsigned = mantissa.trim_start_matches('-');
        let mut digits = unsigned.replace('.', "");
        let integer_digits = unsigned.find('.').unwrap_or(unsigned.len()) as i32;
        let decimal_exponent = integer_digits + exponent - 1;
        if (-6..21).contains(&decimal_exponent) {
            let decimal_position = integer_digits + exponent;
            let mut expanded = if decimal_position <= 0 {
                format!("0.{}{}", "0".repeat((-decimal_position) as usize), digits)
            } else if decimal_position as usize >= digits.len() {
                digits.push_str(&"0".repeat(decimal_position as usize - digits.len()));
                digits
            } else {
                digits.insert(decimal_position as usize, '.');
                digits
            };
            if negative {
                expanded.insert(0, '-');
            }
            expanded
        } else {
            raw
        }
    } else {
        raw.strip_suffix(".0").unwrap_or(&raw).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::format_jcs_number;

    #[test]
    fn jcs_number_boundaries_and_negative_zero() {
        assert_eq!(format_jcs_number(-0.0), "0");
        assert_eq!(format_jcs_number(1.0), "1");
        assert_eq!(format_jcs_number(1e20), "100000000000000000000");
        assert_eq!(format_jcs_number(1e21), "1e+21");
        assert_eq!(format_jcs_number(1e-6), "0.000001");
        assert_eq!(format_jcs_number(1e-7), "1e-7");
        assert_eq!(format_jcs_number(5e-324), "5e-324");
    }
}
