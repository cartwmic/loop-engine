use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn schema_dir() -> PathBuf {
    repo_root().join("quality/semantic-judge/v1")
}

fn load_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn validate(instance: &Value, schema: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    validate_node(instance, schema, schema, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_node(
    instance: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let schema = resolve_schema(schema, root);

    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            validate_node(instance, subschema, root, path, errors);
        }
    }

    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let mut branch_errors = Vec::new();
        let mut matches = 0usize;
        for subschema in one_of {
            let mut candidate_errors = Vec::new();
            validate_node(instance, subschema, root, path, &mut candidate_errors);
            if candidate_errors.is_empty() {
                matches += 1;
            } else {
                branch_errors.extend(candidate_errors);
            }
        }
        if matches != 1 {
            errors.push(format!(
                "{path}: expected exactly one oneOf branch to match (matched {matches})"
            ));
            if matches == 0 {
                errors.extend(branch_errors);
            }
        }
        return;
    }

    if let Some(if_schema) = schema.get("if") {
        let mut if_errors = Vec::new();
        validate_node(instance, if_schema, root, path, &mut if_errors);
        let branch = if if_errors.is_empty() {
            schema.get("then")
        } else {
            schema.get("else")
        };
        if let Some(branch) = branch {
            validate_node(instance, branch, root, path, errors);
        }
        return;
    }

    if let Some(const_value) = schema.get("const") {
        if instance != const_value {
            errors.push(format!("{path}: expected const {const_value}"));
        }
        return;
    }

    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|value| value == instance) {
            errors.push(format!("{path}: value not in enum"));
        }
        return;
    }

    if let Some(expected_type) = schema.get("type").and_then(Value::as_str)
        && !value_matches_type(instance, expected_type)
    {
        errors.push(format!(
            "{path}: expected type {expected_type}, got {}",
            json_type_name(instance)
        ));
        return;
    }

    if let Some(min_length) = schema.get("minLength").and_then(Value::as_u64)
        && let Some(text) = instance.as_str()
        && (text.len() as u64) < min_length
    {
        errors.push(format!(
            "{path}: string shorter than minLength {min_length}"
        ));
    }

    if let Some(min_items) = schema.get("minItems").and_then(Value::as_u64)
        && let Some(items) = instance.as_array()
        && (items.len() as u64) < min_items
    {
        errors.push(format!("{path}: array shorter than minItems {min_items}"));
    }

    if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64)
        && let Some(items) = instance.as_array()
        && (items.len() as u64) > max_items
    {
        errors.push(format!("{path}: array longer than maxItems {max_items}"));
    }

    if let Some(minimum) = schema.get("minimum").and_then(Value::as_i64)
        && let Some(number) = instance.as_i64()
        && number < minimum
    {
        errors.push(format!("{path}: integer less than minimum {minimum}"));
    }

    if schema.get("type").and_then(Value::as_str) == Some("object")
        || schema.get("properties").is_some()
    {
        validate_object(instance, schema, root, path, errors);
    }

    if schema.get("type").and_then(Value::as_str) == Some("array")
        && let Some(item_schema) = schema.get("items")
        && let Some(items) = instance.as_array()
    {
        for (index, item) in items.iter().enumerate() {
            validate_node(item, item_schema, root, &format!("{path}[{index}]"), errors);
        }
    }
}

fn validate_object(
    instance: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let Some(object) = instance.as_object() else {
        return;
    };

    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let Some(properties) = schema.get("properties").and_then(Value::as_object)
    {
        let allowed: HashSet<&str> = properties.keys().map(String::as_str).collect();
        for key in object.keys() {
            if !allowed.contains(key.as_str()) {
                errors.push(format!(
                    "{path}: additional property `{key}` is not allowed"
                ));
            }
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                errors.push(format!("{path}: missing required property `{key}`"));
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, property_schema) in properties {
            if let Some(value) = object.get(key) {
                validate_node(
                    value,
                    property_schema,
                    root,
                    &format!("{path}.{key}"),
                    errors,
                );
            }
        }
    }
}

fn resolve_schema<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let Some(target) = reference.strip_prefix("#/") else {
            panic!("unsupported $ref {reference}");
        };
        let mut current = root;
        for segment in target.split('/') {
            current = current
                .get(segment)
                .unwrap_or_else(|| panic!("missing $ref segment {segment} in {reference}"));
        }
        return current;
    }
    schema
}

fn value_matches_type(instance: &Value, expected_type: &str) -> bool {
    match expected_type {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "integer" => instance.as_i64().is_some(),
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "number" => instance.is_number(),
        other => panic!("unsupported schema type {other}"),
    }
}

fn json_type_name(instance: &Value) -> &'static str {
    match instance {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) if instance.as_i64().is_some() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn request_schema() -> Value {
    load_json(&schema_dir().join("request.schema.json"))
}

fn response_schema() -> Value {
    load_json(&schema_dir().join("response.schema.json"))
}

fn assert_valid(instance: &Value, schema: &Value, label: &str) {
    if let Err(errors) = validate(instance, schema) {
        panic!("{label} should validate:\n{}", errors.join("\n"));
    }
}

fn assert_invalid(instance: &Value, schema: &Value, label: &str) {
    if let Ok(()) = validate(instance, schema) {
        panic!("{label} should fail validation");
    }
}

#[test]
fn schema_files_are_valid_json_without_provider_specific_fields() {
    for file in ["request.schema.json", "response.schema.json"] {
        let path = schema_dir().join(file);
        let text = fs::read_to_string(&path).expect("schema file should exist");
        let schema: Value = serde_json::from_str(&text).expect("schema should be valid JSON");
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema")
        );

        let lowered = text.to_ascii_lowercase();
        for forbidden in [
            "\"pi\"",
            "\"openai\"",
            "\"provider\"",
            "\"model\"",
            "\"codex\"",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "{} must not contain provider-specific field {}",
                file,
                forbidden
            );
        }
    }
}

#[test]
fn request_example_and_minimal_request_validate() {
    let schema = request_schema();
    let example = load_json(&schema_dir().join("examples/request.v1.json"));
    assert_valid(&example, &schema, "request example");

    let minimal = serde_json::json!({
        "schema_version": 1,
        "mode": "local",
        "parent_revision": "abc123",
        "candidate_revision": "def456",
        "diff": "",
        "rubrics": [{ "id": "foundation-seed", "content": "rubric text" }],
        "deterministic_evidence": [{
            "command": "git diff --check",
            "exit_code": 0,
            "stdout": "",
            "stderr": "",
            "candidate_revision": "def456"
        }]
    });
    assert_valid(&minimal, &schema, "minimal request");
}

#[test]
fn response_example_and_fixtures_validate() {
    let schema = response_schema();
    let example = load_json(&schema_dir().join("examples/response-pass.v1.json"));
    assert_valid(&example, &schema, "response example");

    for entry in fs::read_dir(schema_dir().join("fixtures"))
        .expect("fixtures directory should exist")
        .map(Result::unwrap)
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let fixture = load_json(&path);
        assert_valid(
            &fixture,
            &schema,
            &format!("fixture {}", path.file_name().unwrap().to_string_lossy()),
        );
    }
}

#[test]
fn malformed_requests_are_rejected() {
    let schema = request_schema();

    let missing_mode = serde_json::json!({
        "schema_version": 1,
        "parent_revision": "abc",
        "candidate_revision": "def",
        "diff": "",
        "rubrics": [{ "id": "foundation-seed", "content": "text" }],
        "deterministic_evidence": []
    });
    assert_invalid(&missing_mode, &schema, "missing mode");

    let empty_rubrics = serde_json::json!({
        "schema_version": 1,
        "mode": "local",
        "parent_revision": "abc",
        "candidate_revision": "def",
        "diff": "",
        "rubrics": [],
        "deterministic_evidence": []
    });
    assert_invalid(&empty_rubrics, &schema, "empty rubrics");

    let extra_field = serde_json::json!({
        "schema_version": 1,
        "mode": "local",
        "parent_revision": "abc",
        "candidate_revision": "def",
        "diff": "",
        "rubrics": [{ "id": "foundation-seed", "content": "text" }],
        "deterministic_evidence": [],
        "model": "secret"
    });
    assert_invalid(&extra_field, &schema, "provider-specific extra field");
}

#[test]
fn malformed_responses_are_rejected() {
    let schema = response_schema();

    let uncited_pass = serde_json::json!({
        "schema_version": 1,
        "parent_revision": "abc",
        "candidate_revision": "def",
        "verdict": "pass",
        "citations": [],
        "message": "uncited pass"
    });
    assert_invalid(&uncited_pass, &schema, "uncited pass");

    let unavailable_with_citations = serde_json::json!({
        "schema_version": 1,
        "parent_revision": "abc",
        "candidate_revision": "def",
        "verdict": "unavailable",
        "citations": [{
            "rubric_id": "foundation-seed",
            "rule": "I47",
            "lines": ["docs/testing.md:1"]
        }],
        "message": "should be empty citations"
    });
    assert_invalid(
        &unavailable_with_citations,
        &schema,
        "unavailable with citations",
    );

    let partial_binding = serde_json::json!({
        "schema_version": 1,
        "parent_revision": "abc",
        "candidate_revision": null,
        "verdict": "unavailable",
        "citations": [],
        "message": "partial revision binding"
    });
    assert_invalid(&partial_binding, &schema, "partial revision binding");

    let unbound_determinate = serde_json::json!({
        "schema_version": 1,
        "parent_revision": null,
        "candidate_revision": null,
        "verdict": "pass",
        "citations": [{
            "rubric_id": "foundation-seed",
            "rule": "I47",
            "lines": ["docs/testing.md:1"]
        }],
        "message": "determinate verdict requires bound revisions"
    });
    assert_invalid(
        &unbound_determinate,
        &schema,
        "determinate verdict with null revisions",
    );
}

#[test]
fn unavailable_null_binding_fixture_shape_is_valid() {
    let schema = response_schema();
    let unbound_unavailable = serde_json::json!({
        "schema_version": 1,
        "parent_revision": null,
        "candidate_revision": null,
        "verdict": "unavailable",
        "citations": [],
        "message": "malformed request"
    });
    assert_valid(
        &unbound_unavailable,
        &schema,
        "unavailable with null revision binding",
    );
}
