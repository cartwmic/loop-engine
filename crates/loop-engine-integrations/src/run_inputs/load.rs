use std::io::Read;
use std::path::Path;

use loop_engine_core::model::bounded::{RUN_INPUTS_ENCODED_TOTAL_BYTES, Value as CoreValue};
use loop_engine_core::model::ids::InputName;
use loop_engine_core::model::run_input::{InputError, RunInputs};
use serde_json::Value;

use crate::provider_protocol::mapping;
use crate::provider_protocol::validation::{ProtocolValidationError, parse_strict_value};

use super::RunInputLoadError;

pub const RUN_INPUTS_FILE_BYTES: usize = RUN_INPUTS_ENCODED_TOTAL_BYTES;

/// Load optional run-input JSON from `path`.
///
/// `None` yields empty [`RunInputs`]. When a path is provided, the document must be
/// one strict JSON object mapping input names to values, bounded by
/// [`RUN_INPUTS_FILE_BYTES`], with duplicate keys and trailing values rejected.
pub fn load_optional(path: Option<&Path>) -> Result<RunInputs, RunInputLoadError> {
    let Some(path) = path else {
        return Ok(RunInputs::default());
    };
    let bytes = read_bounded_file(path)?;
    let value = parse_strict_value(&bytes, RUN_INPUTS_FILE_BYTES)
        .map_err(|error| map_protocol_error(path, error))?;
    let object = value
        .as_object()
        .ok_or_else(|| RunInputLoadError::Malformed {
            path: path.to_owned(),
            message: "root must be a JSON object".into(),
        })?;
    let candidate = object
        .iter()
        .map(|(key, value)| map_entry(path, key, value))
        .collect::<Result<Vec<_>, RunInputLoadError>>()?;
    RunInputs::from_wire_candidate(candidate).map_err(InputError::into)
}

fn map_entry(
    path: &Path,
    key: &str,
    value: &Value,
) -> Result<(InputName, CoreValue), RunInputLoadError> {
    let name = InputName::parse(key).map_err(InputError::from)?;
    let core = mapping::core_value(value.clone(), &format!("/{key}")).map_err(|error| {
        RunInputLoadError::Malformed {
            path: path.to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok((name, core))
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, RunInputLoadError> {
    let file = std::fs::File::open(path).map_err(|source| RunInputLoadError::Read {
        path: path.to_owned(),
        source,
    })?;
    let metadata_length = file
        .metadata()
        .map_err(|source| RunInputLoadError::Read {
            path: path.to_owned(),
            source,
        })?
        .len();
    if metadata_length > RUN_INPUTS_FILE_BYTES as u64 {
        return Err(RunInputLoadError::TooLarge {
            path: path.to_owned(),
            max: RUN_INPUTS_FILE_BYTES,
            actual: usize::try_from(metadata_length).unwrap_or(usize::MAX),
        });
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata_length)
            .unwrap_or(RUN_INPUTS_FILE_BYTES)
            .min(RUN_INPUTS_FILE_BYTES),
    );
    file.take(RUN_INPUTS_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| RunInputLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > RUN_INPUTS_FILE_BYTES {
        return Err(RunInputLoadError::TooLarge {
            path: path.to_owned(),
            max: RUN_INPUTS_FILE_BYTES,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

fn map_protocol_error(path: &Path, error: ProtocolValidationError) -> RunInputLoadError {
    RunInputLoadError::Malformed {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::model::bounded::Value as CoreValue;
    use loop_engine_core::model::ids::InputName;

    use super::*;

    #[test]
    fn none_path_yields_empty_inputs() {
        assert!(load_optional(None).expect("none path").values().is_empty());
    }

    #[test]
    fn strict_object_maps_into_core_values() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("inputs.json");
        std::fs::write(&path, r#"{"artifact-root":"/tmp/work","enabled":true}"#).unwrap();
        let inputs = load_optional(Some(&path)).expect("load inputs");
        assert_eq!(
            inputs
                .values()
                .get(&InputName::parse("artifact-root").unwrap()),
            Some(&CoreValue::String("/tmp/work".into()))
        );
        assert_eq!(
            inputs.values().get(&InputName::parse("enabled").unwrap()),
            Some(&CoreValue::Bool(true))
        );
    }

    #[test]
    fn rejects_duplicate_keys_and_trailing_values() {
        let directory = tempfile::tempdir().expect("tempdir");
        let duplicate = directory.path().join("duplicate.json");
        std::fs::write(&duplicate, r#"{"a":1,"a":2}"#).unwrap();
        assert!(matches!(
            load_optional(Some(&duplicate)),
            Err(RunInputLoadError::Malformed { .. })
        ));

        let trailing = directory.path().join("trailing.json");
        std::fs::write(&trailing, "{} {}").unwrap();
        assert!(matches!(
            load_optional(Some(&trailing)),
            Err(RunInputLoadError::Malformed { .. })
        ));
    }

    #[test]
    fn rejects_non_object_root_and_invalid_names() {
        let directory = tempfile::tempdir().expect("tempdir");
        let array = directory.path().join("array.json");
        std::fs::write(&array, "[]").unwrap();
        assert!(matches!(
            load_optional(Some(&array)),
            Err(RunInputLoadError::Malformed { message, .. })
            if message == "root must be a JSON object"
        ));

        let invalid_name = directory.path().join("invalid-name.json");
        std::fs::write(&invalid_name, r#"{"":null}"#).unwrap();
        assert!(matches!(
            load_optional(Some(&invalid_name)),
            Err(RunInputLoadError::Input(_))
        ));
    }
}
