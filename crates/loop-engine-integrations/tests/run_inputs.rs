use std::path::Path;

use loop_engine_core::model::bounded::{BoundError, RUN_INPUTS_ENCODED_TOTAL_BYTES, Value};
use loop_engine_core::model::ids::InputName;
use loop_engine_integrations::run_inputs::{RunInputLoadError, load_optional};

#[test]
fn absent_path_yields_empty_run_inputs() {
    assert!(
        load_optional(None)
            .expect("absent path")
            .values()
            .is_empty()
    );
}

#[test]
fn valid_object_loads_bounded_values_without_declarations() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("inputs.json");
    std::fs::write(
        &path,
        r#"{"artifact-root":"/tmp/work","count":3,"tags":["a","b"]}"#,
    )
    .unwrap();

    let inputs = load_optional(Some(&path)).expect("valid inputs load");
    assert_eq!(
        inputs
            .values()
            .get(&InputName::parse("artifact-root").unwrap()),
        Some(&Value::String("/tmp/work".into()))
    );
    assert_eq!(
        inputs.values().get(&InputName::parse("count").unwrap()),
        Some(&Value::Number(
            loop_engine_core::model::bounded::FiniteNumber::new("provider_number", 3.0).unwrap()
        ))
    );
    assert_eq!(
        inputs.values().get(&InputName::parse("tags").unwrap()),
        Some(&Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ]))
    );
}

#[test]
fn strict_parse_rejects_duplicate_keys_and_trailing_json() {
    let directory = tempfile::tempdir().expect("tempdir");

    let duplicate = directory.path().join("duplicate.json");
    std::fs::write(&duplicate, r#"{"name":"first","name":"second"}"#).unwrap();
    assert!(matches!(
        load_optional(Some(&duplicate)),
        Err(RunInputLoadError::Malformed { .. })
    ));

    let trailing = directory.path().join("trailing.json");
    std::fs::write(&trailing, r#"{"ok":true} null"#).unwrap();
    assert!(matches!(
        load_optional(Some(&trailing)),
        Err(RunInputLoadError::Malformed { .. })
    ));
}

#[test]
fn oversized_file_is_rejected_before_parse() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("oversized.json");
    let oversized = "x".repeat(RUN_INPUTS_ENCODED_TOTAL_BYTES + 1);
    std::fs::write(&path, oversized).unwrap();

    assert!(matches!(
        load_optional(Some(&path)),
        Err(RunInputLoadError::TooLarge { max, .. }) if max == RUN_INPUTS_ENCODED_TOTAL_BYTES
    ));
}

#[test]
fn aggregate_encoded_total_bound_is_enforced() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("too-large-object.json");
    let numbers = std::iter::repeat_n("0", 25_000)
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        &path,
        format!(r#"{{"first":[{numbers}],"second":[{numbers}]}}"#),
    )
    .unwrap();

    assert!(matches!(
        load_optional(Some(&path)),
        Err(RunInputLoadError::Input(
            loop_engine_core::model::run_input::InputError::Bound(BoundError::EncodedTooLarge {
                field: "run_inputs",
                ..
            })
        ))
    ));
}

#[test]
fn missing_path_surfaces_read_error() {
    let path = Path::new("/definitely/missing/run-inputs.json");
    assert!(matches!(
        load_optional(Some(path)),
        Err(RunInputLoadError::Read { .. })
    ));
}
