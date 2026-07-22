//! Provider-owned protocol schema validation and DTO roundtrip checks.

use scenario_provider::protocol::ProviderRole;
use scenario_provider::protocol::{
    CompatibilityPayloadDto, CompatibilityResultDto, DescribeResultDto, EmptyPayload,
    GatePayloadDto, GateResultDto, GuidancePayloadDto, GuidanceResultDto, PROTOCOL_MAJOR_V1,
    RequestEnvelope, ResultEnvelope, ValidateInputsPayloadDto, ValidateInputsResultDto,
};
use scenario_provider::schema::{all_local_schemas, role_snake};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn parse_golden_schema(name: &str) -> Value {
    let path = format!("fixtures/schemas/{name}");
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("read {name}: {error}"));
    serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("parse {name}: {error}"))
}

fn assert_schema_equal(name: &str, generated: &Value, golden: &Value) {
    assert_eq!(
        generated, golden,
        "local schema drift for {name}; regenerate with `cargo run --example generate_local_schemas`"
    );
}

#[test]
fn local_schema_inventory_matches_golden_snapshots() {
    for entry in all_local_schemas() {
        let golden = parse_golden_schema(entry.name);
        assert_schema_equal(entry.name, &entry.schema, &golden);
    }
}

#[test]
fn local_schemas_pin_protocol_major_and_role() {
    for entry in all_local_schemas() {
        let schema = &entry.schema;
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{}",
            entry.name
        );
        if entry.name.ends_with("-request.json") || entry.name.ends_with("-result.json") {
            assert_eq!(
                schema["properties"]["protocol_major"]["const"], PROTOCOL_MAJOR_V1,
                "{}",
                entry.name
            );
            let role = entry
                .name
                .strip_suffix("-request.json")
                .or_else(|| entry.name.strip_suffix("-result.json"))
                .expect("role suffix");
            let expected_role = match role {
                "describe" => "describe",
                "validate-inputs" => "validate_inputs",
                "evaluate-gates" => "evaluate_gates",
                "live-guidance" => "live_guidance",
                "check-compatibility" => "check_compatibility",
                other => panic!("unknown schema role prefix {other}"),
            };
            assert_eq!(
                schema["properties"]["role"]["const"], expected_role,
                "{}",
                entry.name
            );
        }
    }
}

#[test]
fn local_result_schemas_expose_frozen_kind_discriminants() {
    let expected: [(&str, &[&str]); 5] = [
        ("describe-result.json", &["description"]),
        (
            "validate-inputs-result.json",
            &["accepted", "rejected", "evaluation_error"],
        ),
        (
            "evaluate-gates-result.json",
            &["verdicts", "incompatible", "evaluation_error"],
        ),
        (
            "live-guidance-result.json",
            &["guidance", "incompatible", "evaluation_error"],
        ),
        (
            "check-compatibility-result.json",
            &["findings", "evaluation_error"],
        ),
    ];
    for (name, kinds) in expected {
        let schema = parse_golden_schema(name);
        let result_def = schema["properties"]["result"]["$ref"]
            .as_str()
            .and_then(|reference| reference.strip_prefix("#/$defs/"))
            .unwrap_or_else(|| panic!("missing result $ref in {name}"));
        let variants = &schema["$defs"][result_def]["oneOf"];
        let observed: Vec<&str> = variants
            .as_array()
            .unwrap_or_else(|| panic!("{name} result variants missing"))
            .iter()
            .map(|variant| {
                variant["properties"]["kind"]["const"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{name} variant missing kind const"))
            })
            .collect();
        assert_eq!(observed, kinds, "{name}");
    }
}

fn roundtrip<T: DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug>(
    label: &str,
    value: &Value,
) {
    let typed: T = serde_json::from_value(value.clone()).unwrap_or_else(|error| {
        panic!("{label} deserialize: {error}");
    });
    let reserialized =
        serde_json::to_value(&typed).unwrap_or_else(|error| panic!("{label} serialize: {error}"));
    assert_eq!(reserialized, *value, "{label} roundtrip mismatch");
}

#[test]
fn representative_requests_roundtrip_through_typed_dtos() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../fixtures/protocol/representative-requests.json"
    ))
    .unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let role = fixture["role"].as_str().unwrap();
        let request = &fixture["request"];
        assert_eq!(request["protocol_major"], PROTOCOL_MAJOR_V1);
        assert_eq!(request["role"], role);
        match role {
            "describe" => roundtrip::<RequestEnvelope<EmptyPayload>>("describe request", request),
            "validate_inputs" => roundtrip::<RequestEnvelope<ValidateInputsPayloadDto>>(
                "validate_inputs request",
                request,
            ),
            "evaluate_gates" => {
                roundtrip::<RequestEnvelope<GatePayloadDto>>("evaluate_gates request", request)
            }
            "live_guidance" => {
                roundtrip::<RequestEnvelope<GuidancePayloadDto>>("live_guidance request", request)
            }
            "check_compatibility" => roundtrip::<RequestEnvelope<CompatibilityPayloadDto>>(
                "check_compatibility request",
                request,
            ),
            other => panic!("unknown representative request role {other}"),
        }
    }
}

#[test]
fn all_role_result_variants_roundtrip_through_typed_dtos() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../fixtures/protocol/valid-result-variants.json"
    ))
    .unwrap();
    assert_eq!(fixtures.as_array().unwrap().len(), 12);
    for fixture in fixtures.as_array().unwrap() {
        assert_eq!(fixture["protocol_major"], PROTOCOL_MAJOR_V1);
        let role = fixture["role"].as_str().unwrap();
        let invocation_id = fixture["invocation_id"].as_str().unwrap();
        match role {
            "describe" => {
                let envelope: ResultEnvelope<DescribeResultDto> =
                    serde_json::from_value(fixture.clone()).unwrap();
                assert_eq!(envelope.protocol_major, PROTOCOL_MAJOR_V1);
                assert_eq!(envelope.role, ProviderRole::Describe);
                assert_eq!(envelope.invocation_id, invocation_id);
                assert_eq!(role_snake(envelope.role), role);
                roundtrip::<ResultEnvelope<DescribeResultDto>>("describe result", fixture);
            }
            "validate_inputs" => {
                let envelope: ResultEnvelope<ValidateInputsResultDto> =
                    serde_json::from_value(fixture.clone()).unwrap();
                assert_eq!(envelope.protocol_major, PROTOCOL_MAJOR_V1);
                assert_eq!(envelope.role, ProviderRole::ValidateInputs);
                assert_eq!(envelope.invocation_id, invocation_id);
                roundtrip::<ResultEnvelope<ValidateInputsResultDto>>(
                    "validate_inputs result",
                    fixture,
                );
            }
            "evaluate_gates" => {
                let envelope: ResultEnvelope<GateResultDto> =
                    serde_json::from_value(fixture.clone()).unwrap();
                assert_eq!(envelope.protocol_major, PROTOCOL_MAJOR_V1);
                assert_eq!(envelope.role, ProviderRole::EvaluateGates);
                assert_eq!(envelope.invocation_id, invocation_id);
                roundtrip::<ResultEnvelope<GateResultDto>>("evaluate_gates result", fixture);
            }
            "live_guidance" => {
                let envelope: ResultEnvelope<GuidanceResultDto> =
                    serde_json::from_value(fixture.clone()).unwrap();
                assert_eq!(envelope.protocol_major, PROTOCOL_MAJOR_V1);
                assert_eq!(envelope.role, ProviderRole::LiveGuidance);
                assert_eq!(envelope.invocation_id, invocation_id);
                roundtrip::<ResultEnvelope<GuidanceResultDto>>("live_guidance result", fixture);
            }
            "check_compatibility" => {
                let envelope: ResultEnvelope<CompatibilityResultDto> =
                    serde_json::from_value(fixture.clone()).unwrap();
                assert_eq!(envelope.protocol_major, PROTOCOL_MAJOR_V1);
                assert_eq!(envelope.role, ProviderRole::CheckCompatibility);
                assert_eq!(envelope.invocation_id, invocation_id);
                roundtrip::<ResultEnvelope<CompatibilityResultDto>>(
                    "check_compatibility result",
                    fixture,
                );
            }
            other => panic!("unknown result fixture role {other}"),
        }
    }
}
