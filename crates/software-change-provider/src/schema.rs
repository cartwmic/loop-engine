//! Generic validator for bounded JSON schemas and JSON instances.

#![allow(dead_code)]

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Schema types supported by this bounded language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SchemaType {
    Object,
    Array,
    String,
}

impl SchemaType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value.as_str() {
            Some("object") => Some(Self::Object),
            Some("array") => Some(Self::Array),
            Some("string") => Some(Self::String),
            _ => None,
        }
    }
}

/// Keywords permitted on an object schema, in sorted order.
const OBJECT_KEYWORDS: &[&str] = &["additionalProperties", "properties", "required", "type"];
/// Keywords permitted on an array schema, in sorted order.
const ARRAY_KEYWORDS: &[&str] = &["items", "minItems", "type"];
/// Keywords permitted on a string schema, in sorted order.
const STRING_KEYWORDS: &[&str] = &["enum", "minLength", "type"];

// The provider's bounded schema language normally has no regex keyword. The
// bookends overlay needs exactly this one closed-field token, so it is
// accepted as a private schema extension without widening the general
// allowlist.
const REQUIREMENT_ID_PATTERN: &str = r"^LE-[1-9][0-9]*$";
const REQUIREMENT_ID_SCHEMA_PATH: &str = "/properties/requirement_ids/items";

/// Return exact keyword allowlist for one schema type.
///
/// This is intentionally kept as one small, inspectable table.  Production
/// acceptance below derives from these tables, so the pin test protects both
/// the declared subset and the validator from accidental subset drift.
fn allowed_keywords(schema_type: SchemaType) -> &'static [&'static str] {
    match schema_type {
        SchemaType::Object => OBJECT_KEYWORDS,
        SchemaType::Array => ARRAY_KEYWORDS,
        SchemaType::String => STRING_KEYWORDS,
    }
}

const ALL_SCHEMA_TYPES: &[SchemaType] =
    &[SchemaType::Object, SchemaType::Array, SchemaType::String];

fn keyword_owner(keyword: &str) -> Option<SchemaType> {
    let mut owner = None;
    let mut owner_count = 0;

    for schema_type in ALL_SCHEMA_TYPES {
        if allowed_keywords(*schema_type).contains(&keyword) {
            owner = Some(*schema_type);
            owner_count += 1;
        }
    }

    // Keywords shared by all schema types (currently only `type`) have no
    // placement owner.  A keyword present in exactly one table is restricted
    // to that schema type; unknown keywords have no owner either.
    if owner_count == 1 {
        owner
    } else {
        None
    }
}

fn known_keyword(keyword: &str) -> bool {
    ALL_SCHEMA_TYPES
        .iter()
        .any(|schema_type| allowed_keywords(*schema_type).contains(&keyword))
}

/// A schema-level violation.  These are malformed-schema diagnostics and are
/// kept separate from instance violations at the type level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaViolation {
    pub(crate) path: String,
    pub(crate) rule: String,
    pub(crate) message: String,
}

impl MetaViolation {
    fn new(path: impl Into<String>, rule: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            rule: rule.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for MetaViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.rule,
            display_path(&self.path),
            self.message
        )
    }
}

/// Report returned when schema meta-validation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationReport {
    pub(crate) violations: Vec<MetaViolation>,
}

impl MetaValidationReport {
    fn new(mut violations: Vec<MetaViolation>) -> Self {
        sort_meta_violations(&mut violations);
        Self { violations }
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    pub(crate) fn violations(&self) -> &[MetaViolation] {
        &self.violations
    }

    pub(crate) fn into_violations(self) -> Vec<MetaViolation> {
        self.violations
    }
}

/// An instance-level schema violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceViolation {
    pub(crate) path: String,
    pub(crate) rule: String,
    pub(crate) message: String,
}

impl InstanceViolation {
    fn new(path: impl Into<String>, rule: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            rule: rule.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for InstanceViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.rule,
            display_path(&self.path),
            self.message
        )
    }
}

/// Report returned after evaluating an instance against a valid schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceReport {
    pub(crate) violations: Vec<InstanceViolation>,
}

impl InstanceReport {
    fn new(mut violations: Vec<InstanceViolation>) -> Self {
        sort_instance_violations(&mut violations);
        Self { violations }
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    pub(crate) fn violations(&self) -> &[InstanceViolation] {
        &self.violations
    }

    pub(crate) fn into_violations(self) -> Vec<InstanceViolation> {
        self.violations
    }
}

/// Result of checking one schema and one instance.
///
/// `SchemaInvalid` and `InstanceInvalid` cannot be confused by callers.  A
/// valid schema with a valid instance is represented by `Valid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckResult {
    Valid,
    SchemaInvalid(MetaValidationReport),
    InstanceInvalid(InstanceReport),
}

/// A compiled, meta-valid schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedSchema {
    root: SchemaNode,
}

impl ValidatedSchema {
    /// Evaluate instance and collect every applicable violation.
    pub(crate) fn evaluate(&self, instance: &Value) -> InstanceReport {
        let mut violations = Vec::new();
        evaluate_node(&self.root, instance, "", &mut violations);
        InstanceReport::new(violations)
    }
}

/// Validate schema meta-rules and compile it for repeated instance checks.
pub(crate) fn validate_schema(schema: &Value) -> Result<ValidatedSchema, MetaValidationReport> {
    let mut violations = Vec::new();
    let root = compile_node(schema, "", &mut violations);
    if violations.is_empty() {
        // `compile_node` returns a node for an object.  For a non-object root,
        // it returns a harmless placeholder while recording an error.
        Ok(ValidatedSchema { root })
    } else {
        Err(MetaValidationReport::new(violations))
    }
}

/// Check schema meta-rules, then evaluate instance when schema is valid.
pub(crate) fn check(schema: &Value, instance: &Value) -> CheckResult {
    match validate_schema(schema) {
        Ok(schema) => {
            let report = schema.evaluate(instance);
            if report.is_valid() {
                CheckResult::Valid
            } else {
                CheckResult::InstanceInvalid(report)
            }
        }
        Err(report) => CheckResult::SchemaInvalid(report),
    }
}

/// Evaluate an instance against a previously validated schema.
pub(crate) fn evaluate_instance(schema: &ValidatedSchema, instance: &Value) -> InstanceReport {
    schema.evaluate(instance)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemaNode {
    schema_type: SchemaType,
    properties: BTreeMap<String, SchemaNode>,
    required: Vec<String>,
    additional_properties: bool,
    items: Option<Box<SchemaNode>>,
    min_items: Option<u64>,
    min_length: Option<u64>,
    enum_values: Vec<String>,
    pattern: Option<String>,
}

impl SchemaNode {
    fn empty() -> Self {
        Self {
            schema_type: SchemaType::String,
            properties: BTreeMap::new(),
            required: Vec::new(),
            additional_properties: true,
            items: None,
            min_items: None,
            min_length: None,
            enum_values: Vec::new(),
            pattern: None,
        }
    }
}

fn compile_node(value: &Value, path: &str, violations: &mut Vec<MetaViolation>) -> SchemaNode {
    let Some(object) = value.as_object() else {
        violations.push(MetaViolation::new(
            path,
            "schema",
            "schema must be a JSON object",
        ));
        return SchemaNode::empty();
    };

    let schema_type = match object.get("type") {
        None => {
            violations.push(MetaViolation::new(
                child_path(path, "type"),
                "type",
                "schema must declare `type`",
            ));
            None
        }
        Some(value) => match SchemaType::from_value(value) {
            Some(schema_type) => Some(schema_type),
            None => {
                violations.push(MetaViolation::new(
                    child_path(path, "type"),
                    "type",
                    "type must be one of `object`, `array`, or `string`",
                ));
                None
            }
        },
    };

    for keyword in object.keys() {
        if keyword == "pattern"
            && path == REQUIREMENT_ID_SCHEMA_PATH
            && schema_type == Some(SchemaType::String)
            && object.get("pattern").and_then(Value::as_str) == Some(REQUIREMENT_ID_PATTERN)
        {
            continue;
        }
        if !known_keyword(keyword) {
            violations.push(MetaViolation::new(
                child_path(path, keyword),
                "unknown-keyword",
                format!("unknown schema keyword `{keyword}`"),
            ));
            continue;
        }

        if let (Some(schema_type), Some(owner)) = (schema_type, keyword_owner(keyword)) {
            if schema_type != owner {
                violations.push(MetaViolation::new(
                    child_path(path, keyword),
                    "keyword-placement",
                    format!(
                        "keyword `{keyword}` is only legal with type `{}`",
                        owner.as_str()
                    ),
                ));
            }
        }
    }

    let properties = compile_properties(object, path, violations);
    let required = compile_required(object, path, &properties, violations);
    let additional_properties = compile_additional_properties(object, path, violations);
    let items = compile_items(object, path, violations);
    let min_items = compile_nonnegative_integer(object, path, "minItems", violations);
    let min_length = compile_nonnegative_integer(object, path, "minLength", violations);
    let enum_values = compile_enum(object, path, violations);
    let pattern = compile_requirement_id_pattern(path, object, schema_type);

    SchemaNode {
        schema_type: schema_type.unwrap_or(SchemaType::String),
        properties,
        required,
        additional_properties,
        items,
        min_items,
        min_length,
        enum_values,
        pattern,
    }
}

fn compile_properties(
    object: &Map<String, Value>,
    path: &str,
    violations: &mut Vec<MetaViolation>,
) -> BTreeMap<String, SchemaNode> {
    let Some(value) = object.get("properties") else {
        return BTreeMap::new();
    };
    let Some(properties) = value.as_object() else {
        violations.push(MetaViolation::new(
            child_path(path, "properties"),
            "properties",
            "`properties` must be an object of subschemas",
        ));
        return BTreeMap::new();
    };

    properties
        .iter()
        .map(|(name, schema)| {
            (
                name.clone(),
                compile_node(
                    schema,
                    &child_path(&child_path(path, "properties"), name),
                    violations,
                ),
            )
        })
        .collect()
}

fn compile_required(
    object: &Map<String, Value>,
    path: &str,
    properties: &BTreeMap<String, SchemaNode>,
    violations: &mut Vec<MetaViolation>,
) -> Vec<String> {
    let Some(value) = object.get("required") else {
        return Vec::new();
    };
    let Some(required) = value.as_array() else {
        violations.push(MetaViolation::new(
            child_path(path, "required"),
            "required",
            "`required` must be an array of unique property names",
        ));
        return Vec::new();
    };

    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, entry) in required.iter().enumerate() {
        let entry_path = child_path(&child_path(path, "required"), &index.to_string());
        let Some(name) = entry.as_str() else {
            violations.push(MetaViolation::new(
                entry_path,
                "required",
                "each `required` entry must be a string",
            ));
            continue;
        };

        if !seen.insert(name.to_owned()) {
            violations.push(MetaViolation::new(
                entry_path.clone(),
                "required",
                format!("duplicate required property `{name}`"),
            ));
        }
        if !properties.contains_key(name) {
            violations.push(MetaViolation::new(
                entry_path,
                "required",
                format!("required property `{name}` is absent from `properties`"),
            ));
        }
        names.push(name.to_owned());
    }
    names
}

fn compile_additional_properties(
    object: &Map<String, Value>,
    path: &str,
    violations: &mut Vec<MetaViolation>,
) -> bool {
    let Some(value) = object.get("additionalProperties") else {
        return true;
    };
    if value != &Value::Bool(false) {
        violations.push(MetaViolation::new(
            child_path(path, "additionalProperties"),
            "additionalProperties",
            "`additionalProperties` must be literal `false`",
        ));
        return true;
    }
    false
}

fn compile_items(
    object: &Map<String, Value>,
    path: &str,
    violations: &mut Vec<MetaViolation>,
) -> Option<Box<SchemaNode>> {
    let value = object.get("items")?;
    if !value.is_object() {
        violations.push(MetaViolation::new(
            child_path(path, "items"),
            "items",
            "`items` must be one subschema object",
        ));
        return None;
    }
    Some(Box::new(compile_node(
        value,
        &child_path(path, "items"),
        violations,
    )))
}

fn compile_nonnegative_integer(
    object: &Map<String, Value>,
    path: &str,
    keyword: &str,
    violations: &mut Vec<MetaViolation>,
) -> Option<u64> {
    let value = object.get(keyword)?;
    match value.as_u64() {
        Some(number) => Some(number),
        None => {
            violations.push(MetaViolation::new(
                child_path(path, keyword),
                keyword,
                format!("`{keyword}` must be an integer greater than or equal to zero"),
            ));
            None
        }
    }
}

fn compile_requirement_id_pattern(
    path: &str,
    object: &Map<String, Value>,
    schema_type: Option<SchemaType>,
) -> Option<String> {
    if path == REQUIREMENT_ID_SCHEMA_PATH
        && schema_type == Some(SchemaType::String)
        && object.get("pattern").and_then(Value::as_str) == Some(REQUIREMENT_ID_PATTERN)
    {
        Some(REQUIREMENT_ID_PATTERN.to_owned())
    } else {
        None
    }
}

fn compile_enum(
    object: &Map<String, Value>,
    path: &str,
    violations: &mut Vec<MetaViolation>,
) -> Vec<String> {
    let Some(value) = object.get("enum") else {
        return Vec::new();
    };
    let Some(values) = value.as_array() else {
        violations.push(MetaViolation::new(
            child_path(path, "enum"),
            "enum",
            "`enum` must be a non-empty array of unique strings",
        ));
        return Vec::new();
    };
    if values.is_empty() {
        violations.push(MetaViolation::new(
            child_path(path, "enum"),
            "enum",
            "`enum` must not be empty",
        ));
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let value_path = child_path(&child_path(path, "enum"), &index.to_string());
        let Some(value) = value.as_str() else {
            violations.push(MetaViolation::new(
                value_path,
                "enum",
                "each `enum` entry must be a string",
            ));
            continue;
        };
        if !seen.insert(value.to_owned()) {
            violations.push(MetaViolation::new(
                value_path,
                "enum",
                format!("duplicate enum value `{value}`"),
            ));
        }
        result.push(value.to_owned());
    }
    result
}

fn evaluate_node(
    schema: &SchemaNode,
    instance: &Value,
    path: &str,
    violations: &mut Vec<InstanceViolation>,
) {
    if !matches_type(schema.schema_type, instance) {
        violations.push(InstanceViolation::new(
            path,
            "type",
            format!(
                "expected `{}`, found `{}`",
                schema.schema_type.as_str(),
                instance_type_name(instance)
            ),
        ));
        return;
    }

    match schema.schema_type {
        SchemaType::Object => {
            let object = instance
                .as_object()
                .expect("validated schema type and instance type must agree");

            for required in &schema.required {
                if !object.contains_key(required) {
                    violations.push(InstanceViolation::new(
                        path,
                        "required",
                        format!("required property `{required}` is missing"),
                    ));
                }
            }

            if !schema.additional_properties {
                for name in object.keys() {
                    if !schema.properties.contains_key(name) {
                        violations.push(InstanceViolation::new(
                            child_path(path, name),
                            "additionalProperties",
                            format!("property `{name}` is not allowed"),
                        ));
                    }
                }
            }

            for (name, property_schema) in &schema.properties {
                if let Some(value) = object.get(name) {
                    evaluate_node(property_schema, value, &child_path(path, name), violations);
                }
            }
        }
        SchemaType::Array => {
            let array = instance
                .as_array()
                .expect("validated schema type and instance type must agree");
            if let Some(min_items) = schema.min_items {
                let actual = array.len() as u64;
                if actual < min_items {
                    violations.push(InstanceViolation::new(
                        path,
                        "minItems",
                        format!("expected at least {min_items} items, found {actual}"),
                    ));
                }
            }
            if let Some(items) = &schema.items {
                for (index, value) in array.iter().enumerate() {
                    evaluate_node(
                        items,
                        value,
                        &child_path(path, &index.to_string()),
                        violations,
                    );
                }
            }
        }
        SchemaType::String => {
            let string = instance
                .as_str()
                .expect("validated schema type and instance type must agree");
            if let Some(min_length) = schema.min_length {
                let actual = string.chars().count() as u64;
                if actual < min_length {
                    violations.push(InstanceViolation::new(
                        path,
                        "minLength",
                        format!("expected length at least {min_length}, found {actual}"),
                    ));
                }
            }
            if !schema.enum_values.is_empty()
                && !schema
                    .enum_values
                    .iter()
                    .any(|candidate| candidate == string)
            {
                violations.push(InstanceViolation::new(
                    path,
                    "enum",
                    "string is not one of enum values".to_owned(),
                ));
            }
            if schema.pattern.as_deref() == Some(REQUIREMENT_ID_PATTERN)
                && !matches_requirement_id_pattern(string)
            {
                violations.push(InstanceViolation::new(
                    path,
                    "pattern",
                    format!("string does not match `{REQUIREMENT_ID_PATTERN}`"),
                ));
            }
        }
    }
}

fn matches_requirement_id_pattern(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("LE-") else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_digit() && first != '0')
        && chars.all(|character| character.is_ascii_digit())
}

fn matches_type(schema_type: SchemaType, instance: &Value) -> bool {
    match schema_type {
        SchemaType::Object => instance.is_object(),
        SchemaType::Array => instance.is_array(),
        SchemaType::String => instance.is_string(),
    }
}

fn instance_type_name(instance: &Value) -> &'static str {
    match instance {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn sort_meta_violations(violations: &mut [MetaViolation]) {
    violations.sort_by(|left, right| {
        (&left.path, &left.rule, &left.message).cmp(&(&right.path, &right.rule, &right.message))
    });
}

fn sort_instance_violations(violations: &mut [InstanceViolation]) {
    violations.sort_by(|left, right| {
        (&left.path, &left.rule, &left.message).cmp(&(&right.path, &right.rule, &right.message))
    });
}

fn child_path(base: &str, segment: &str) -> String {
    format!("{base}/{}", escape_pointer_segment(segment))
}

fn escape_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn display_path(path: &str) -> &str {
    if path.is_empty() {
        "<root>"
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_schema(value: Value) -> ValidatedSchema {
        validate_schema(&value).expect("schema must be valid")
    }

    fn meta_rules(value: Value) -> Vec<String> {
        validate_schema(&value)
            .expect_err("schema must be invalid")
            .violations
            .into_iter()
            .map(|violation| violation.rule)
            .collect()
    }

    fn instance_violations(schema: Value, instance: Value) -> Vec<InstanceViolation> {
        valid_schema(schema).evaluate(&instance).violations
    }

    #[test]
    fn meta_rejects_missing_type_at_root_and_nested_levels() {
        let root_rules = meta_rules(json!({}));
        assert!(root_rules.iter().any(|rule| rule == "type"));

        let nested_rules = meta_rules(json!({
            "type": "object",
            "properties": {"child": {"type": "array", "items": {}}}
        }));
        assert!(nested_rules.iter().filter(|rule| *rule == "type").count() >= 1);
    }

    #[test]
    fn meta_rejects_unknown_keys_recursively() {
        let report = validate_schema(&json!({
            "type": "object",
            "properties": {
                "child": {"type": "object", "unknown": true}
            },
            "unknown": true
        }))
        .expect_err("unknown keywords must be rejected");

        assert_eq!(
            report
                .violations
                .iter()
                .filter(|violation| violation.rule == "unknown-keyword")
                .map(|violation| violation.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/properties/child/unknown", "/unknown"]
        );
    }

    #[test]
    fn meta_rejects_invalid_type_values() {
        let report = validate_schema(&json!({"type": "number"}))
            .expect_err("unsupported type must be rejected");
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].rule, "type");
    }

    #[test]
    fn meta_enforces_keyword_placement_for_each_type() {
        let cases = [
            ("object", "items", json!({"type": "array"})),
            ("object", "minItems", json!(0)),
            ("object", "minLength", json!(0)),
            ("object", "enum", json!(["x"])),
            ("array", "properties", json!({})),
            ("array", "required", json!([])),
            ("array", "additionalProperties", json!(false)),
            ("array", "minLength", json!(0)),
            ("array", "enum", json!(["x"])),
            ("string", "properties", json!({})),
            ("string", "required", json!([])),
            ("string", "additionalProperties", json!(false)),
            ("string", "items", json!({"type": "string"})),
            ("string", "minItems", json!(0)),
        ];

        for (schema_type, keyword, value) in cases {
            let mut schema = json!({"type": schema_type});
            schema[keyword] = value;
            let report = validate_schema(&schema).expect_err("misplaced keyword must be rejected");
            assert!(
                report
                    .violations
                    .iter()
                    .any(|violation| violation.rule == "keyword-placement"),
                "missing placement violation for {schema_type}/{keyword}: {:?}",
                report.violations
            );
        }
    }

    #[test]
    fn meta_rejects_properties_that_are_not_an_object_of_subschemas() {
        assert!(meta_rules(json!({
            "type": "object",
            "properties": []
        }))
        .contains(&"properties".to_owned()));

        let report = validate_schema(&json!({
            "type": "object",
            "properties": {"child": "not a schema"}
        }))
        .expect_err("property value must be a schema object");
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.path == "/properties/child" && violation.rule == "schema"));
    }

    #[test]
    fn meta_rejects_required_shape_duplicates_and_absent_properties() {
        let report = validate_schema(&json!({
            "type": "object",
            "properties": {"present": {"type": "string"}},
            "required": ["present", "present", "missing", 1]
        }))
        .expect_err("invalid required entries must be rejected");

        assert!(report.violations.iter().any(
            |violation| violation.rule == "required" && violation.message.contains("duplicate")
        ));
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.rule == "required" && violation.message.contains("absent")));
        assert!(report
            .violations
            .iter()
            .any(|violation| violation.rule == "required" && violation.message.contains("string")));

        assert!(meta_rules(json!({
            "type": "object",
            "required": "present"
        }))
        .contains(&"required".to_owned()));
    }

    #[test]
    fn meta_requires_literal_false_for_additional_properties() {
        for value in [json!(true), json!({"type": "string"}), json!(null)] {
            assert!(meta_rules(json!({
                "type": "object",
                "additionalProperties": value
            }))
            .contains(&"additionalProperties".to_owned()));
        }
        assert!(validate_schema(&json!({
            "type": "object",
            "additionalProperties": false
        }))
        .is_ok());
    }

    #[test]
    fn meta_validates_items_and_nested_items() {
        assert!(meta_rules(json!({
            "type": "array",
            "items": []
        }))
        .contains(&"items".to_owned()));
        assert!(meta_rules(json!({
            "type": "array",
            "items": {"type": "object", "properties": {"nested": {}}}
        }))
        .contains(&"type".to_owned()));
    }

    #[test]
    fn meta_validates_nonnegative_integer_keywords() {
        for (keyword, value) in [
            ("minItems", json!(-1)),
            ("minItems", json!(1.5)),
            ("minItems", json!("1")),
            ("minLength", json!(-1)),
            ("minLength", json!(1.5)),
            ("minLength", json!("1")),
        ] {
            let mut schema = json!({
                "type": if keyword == "minItems" { "array" } else { "string" }
            });
            schema[keyword] = value;
            assert!(
                meta_rules(schema).contains(&keyword.to_owned()),
                "invalid {keyword} must be rejected"
            );
        }
    }

    #[test]
    fn meta_requires_nonempty_unique_string_enum() {
        assert!(meta_rules(json!({"type": "string", "enum": []})).contains(&"enum".to_owned()));
        let report = validate_schema(&json!({
            "type": "string",
            "enum": ["one", "one", 2]
        }))
        .expect_err("enum entries must be unique strings");
        assert!(
            report
                .violations
                .iter()
                .filter(|violation| violation.rule == "enum")
                .count()
                >= 2
        );
    }

    #[test]
    fn allowlist_pin_matches_normative_subset() {
        let expected = [
            (
                SchemaType::Object,
                ["additionalProperties", "properties", "required", "type"].as_slice(),
            ),
            (SchemaType::Array, ["items", "minItems", "type"].as_slice()),
            (SchemaType::String, ["enum", "minLength", "type"].as_slice()),
        ];
        for (schema_type, keywords) in expected {
            assert_eq!(allowed_keywords(schema_type), keywords);
        }

        let valid_values = [
            ("type", json!("object")),
            ("properties", json!({})),
            ("required", json!([])),
            ("additionalProperties", json!(false)),
            ("items", json!({"type": "string"})),
            ("minItems", json!(0)),
            ("minLength", json!(0)),
            ("enum", json!(["value"])),
        ];
        for (schema_type, keywords) in expected {
            let mut schema = json!({"type": schema_type.as_str()});
            for keyword in keywords {
                if *keyword == "type" {
                    continue;
                }
                let value = valid_values
                    .iter()
                    .find(|(candidate, _)| candidate == keyword)
                    .expect("every allowed keyword has a valid test value")
                    .1
                    .clone();
                schema[*keyword] = value;
            }
            assert!(
                validate_schema(&schema).is_ok(),
                "all allowed keywords must be accepted for {schema_type:?}"
            );
        }

        for (schema_type, forbidden) in [
            (
                "object",
                ["pattern", "minLength", "items", "oneOf", "$ref"].as_slice(),
            ),
            (
                "array",
                ["pattern", "minLength", "properties", "oneOf", "$ref"].as_slice(),
            ),
            (
                "string",
                [
                    "pattern",
                    "items",
                    "minItems",
                    "properties",
                    "oneOf",
                    "$ref",
                ]
                .as_slice(),
            ),
        ] {
            for keyword in forbidden {
                let mut schema = json!({"type": schema_type});
                schema[*keyword] = json!(null);
                assert!(
                    validate_schema(&schema).is_err(),
                    "unsupported keyword `{keyword}` must be rejected"
                );
            }
        }
    }

    #[test]
    fn requirement_id_pattern_is_only_allowed_on_the_overlay_items_schema() {
        let valid = json!({
            "type": "object",
            "properties": {
                "requirement_ids": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "pattern": REQUIREMENT_ID_PATTERN
                    }
                }
            }
        });
        assert!(validate_schema(&valid).is_ok());

        let invalid = json!({
            "type": "string",
            "pattern": REQUIREMENT_ID_PATTERN
        });
        assert!(validate_schema(&invalid).is_err());
    }

    #[test]
    fn instance_type_mismatch_does_not_cascade() {
        let violations = instance_violations(
            json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 3, "enum": ["abc"]}
                },
                "additionalProperties": false
            }),
            json!("wrong type"),
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, "type");
        assert_eq!(violations[0].path, "");
    }

    #[test]
    fn instance_collects_object_rules_and_nested_properties() {
        let violations = instance_violations(
            json!({
                "type": "object",
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string", "minLength": 3},
                    "kind": {"type": "string", "enum": ["good"]}
                },
                "additionalProperties": false
            }),
            json!({"name": "x", "extra": true}),
        );
        assert_eq!(
            violations
                .iter()
                .map(|violation| (&violation.path, &violation.rule))
                .collect::<Vec<_>>(),
            vec![
                (&"".to_owned(), &"required".to_owned()),
                (&"/extra".to_owned(), &"additionalProperties".to_owned()),
                (&"/name".to_owned(), &"minLength".to_owned()),
            ]
        );
    }

    #[test]
    fn instance_collects_array_rules_and_type_mismatches_without_cascade() {
        let violations = instance_violations(
            json!({
                "type": "array",
                "minItems": 3,
                "items": {"type": "string", "minLength": 2}
            }),
            json!(["", 1]),
        );
        assert_eq!(violations.len(), 3);
        assert_eq!(violations[0].rule, "minItems");
        assert_eq!(violations[1].path, "/0");
        assert_eq!(violations[1].rule, "minLength");
        assert_eq!(violations[2].path, "/1");
        assert_eq!(violations[2].rule, "type");
    }

    #[test]
    fn instance_checks_string_length_and_exact_enum_equality() {
        let short = instance_violations(json!({"type": "string", "minLength": 2}), json!("é"));
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].rule, "minLength");

        let wrong = instance_violations(json!({"type": "string", "enum": ["A"]}), json!("a"));
        assert_eq!(wrong.len(), 1);
        assert_eq!(wrong[0].rule, "enum");

        let exact = instance_violations(json!({"type": "string", "enum": ["A"]}), json!("A"));
        assert!(exact.is_empty());
    }

    #[test]
    fn instance_paths_escape_json_pointer_segments() {
        let violations = instance_violations(
            json!({
                "type": "object",
                "properties": {"a/b~c": {"type": "string", "minLength": 2}}
            }),
            json!({"a/b~c": ""}),
        );
        assert_eq!(violations[0].path, "/a~1b~0c");
    }

    #[test]
    fn instance_violation_order_is_deterministic_and_path_rule_keyed() {
        let schema = json!({
            "type": "object",
            "required": ["z", "a"],
            "properties": {
                "z": {"type": "string", "minLength": 4},
                "a": {"type": "array", "minItems": 2, "items": {"type": "string"}}
            },
            "additionalProperties": false
        });
        let instance = json!({"z": "", "a": [false], "x": 1});
        let first = instance_violations(schema.clone(), instance.clone());
        for _ in 0..32 {
            assert_eq!(first, instance_violations(schema.clone(), instance.clone()));
        }

        assert!(first.windows(2).all(|pair| {
            (&pair[0].path, &pair[0].rule, &pair[0].message)
                <= (&pair[1].path, &pair[1].rule, &pair[1].message)
        }));
    }

    #[test]
    fn check_distinguishes_schema_and_instance_failures() {
        assert!(matches!(
            check(&json!({"type": "number"}), &json!(1)),
            CheckResult::SchemaInvalid(_)
        ));
        assert!(matches!(
            check(&json!({"type": "string"}), &json!(1)),
            CheckResult::InstanceInvalid(_)
        ));
        assert!(matches!(
            check(&json!({"type": "string"}), &json!("ok")),
            CheckResult::Valid
        ));
    }
}
