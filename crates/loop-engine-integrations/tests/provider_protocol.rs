use std::fs;
use std::path::PathBuf;

use loop_engine_integrations::provider_protocol::dto::{
    CompatibilityResultDto, DescribeResultDto, GateResultDto, GuidanceResultDto, ResultEnvelope,
    ValidateInputsResultDto,
};
use loop_engine_integrations::provider_protocol::validation::{
    PROVIDER_REQUEST_JSON_BYTES, PROVIDER_RESULT_STDOUT_BYTES, parse_strict, reject_topology_fields,
};
use loop_engine_integrations::provider_protocol::version::require_supported_major;
use serde::de::DeserializeOwned;

fn parse<T: DeserializeOwned>(raw: &str) -> T {
    parse_strict(raw.as_bytes(), PROVIDER_RESULT_STDOUT_BYTES)
        .unwrap()
        .0
}

#[test]
fn published_schema_inventory_is_parseable() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/provider/v1");
    let expected = [
        "graph.json",
        "describe-request.json",
        "describe-result.json",
        "validate-inputs-request.json",
        "validate-inputs-result.json",
        "evaluate-gates-request.json",
        "evaluate-gates-result.json",
        "live-guidance-request.json",
        "live-guidance-result.json",
        "check-compatibility-request.json",
        "check-compatibility-result.json",
    ];
    for name in expected {
        let bytes = fs::read(directory.join(name)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }
}

#[test]
fn every_valid_result_variant_deserializes_and_wrong_tags_fail() {
    let envelope = |role: &str, result: &str| {
        format!(r#"{{"protocol_major":1,"role":"{role}","invocation_id":"i","result":{result}}}"#)
    };
    let describe = envelope(
        "describe",
        r#"{"kind":"description","graph":{"initial_state":"a","states":[{"id":"a","final":false,"static_guidance":{"kind":"none"}}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}"#,
    );
    let _: ResultEnvelope<DescribeResultDto> = parse(&describe);
    for result in [
        r#"{"kind":"accepted"}"#,
        r#"{"kind":"rejected","diagnostics":[]}"#,
        r#"{"kind":"evaluation_error","diagnostics":[]}"#,
    ] {
        let _: ResultEnvelope<ValidateInputsResultDto> =
            parse(&envelope("validate_inputs", result));
    }
    for result in [
        r#"{"kind":"verdicts","verdicts":[],"evidence":[]}"#,
        r#"{"kind":"incompatible","diagnostics":[]}"#,
        r#"{"kind":"evaluation_error","diagnostics":[]}"#,
    ] {
        let _: ResultEnvelope<GateResultDto> = parse(&envelope("evaluate_gates", result));
    }
    for result in [
        r#"{"kind":"guidance","text":"next"}"#,
        r#"{"kind":"incompatible","diagnostics":[]}"#,
        r#"{"kind":"evaluation_error","diagnostics":[]}"#,
    ] {
        let _: ResultEnvelope<GuidanceResultDto> = parse(&envelope("live_guidance", result));
    }
    for result in [
        r#"{"kind":"findings","capabilities":[]}"#,
        r#"{"kind":"evaluation_error","diagnostics":[]}"#,
    ] {
        let _: ResultEnvelope<CompatibilityResultDto> =
            parse(&envelope("check_compatibility", result));
    }
    assert!(
        parse_strict::<ResultEnvelope<DescribeResultDto>>(
            envelope("describe", r#"{"kind":"rejected","diagnostics":[]}"#).as_bytes(),
            PROVIDER_RESULT_STDOUT_BYTES,
        )
        .is_err()
    );
}

#[test]
fn same_major_unknown_fields_are_accepted_but_topology_output_is_rejected() {
    let raw = r#"{"protocol_major":1,"role":"validate_inputs","invocation_id":"i","future":true,"result":{"kind":"accepted","future_result":1}}"#;
    let _: ResultEnvelope<ValidateInputsResultDto> = parse(raw);
    let (_, value) = parse_strict::<ResultEnvelope<ValidateInputsResultDto>>(
        r#"{"protocol_major":1,"role":"validate_inputs","invocation_id":"i","result":{"kind":"accepted","graph":{}}}"#.as_bytes(),
        PROVIDER_RESULT_STDOUT_BYTES,
    )
    .unwrap();
    assert!(reject_topology_fields(&value["result"]).is_err());
    assert!(
        reject_topology_fields(&serde_json::json!({
            "kind": "accepted",
            "values": {"state": "provider-controlled"}
        }))
        .is_err()
    );
    assert!(
        reject_topology_fields(&serde_json::json!({
            "kind": "accepted",
            "metadata": {"state": "opaque-caller-metadata"}
        }))
        .is_ok()
    );
}

#[test]
fn unsupported_major_and_malformed_protocol_are_distinct() {
    assert!(require_supported_major(1).is_ok());
    assert!(require_supported_major(2).is_err());
    assert!(
        parse_strict::<ResultEnvelope<DescribeResultDto>>(
            b"not-json",
            PROVIDER_RESULT_STDOUT_BYTES
        )
        .is_err()
    );
}

#[test]
fn golden_fixtures_cover_every_result_variant_and_invalid_case() {
    let valid: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/provider/valid-result-variants.json")).unwrap();
    assert_eq!(valid.as_array().unwrap().len(), 12);
    for value in valid.as_array().unwrap() {
        let raw = serde_json::to_string(value).unwrap();
        match value["role"].as_str().unwrap() {
            "describe" => {
                let _: ResultEnvelope<DescribeResultDto> = parse(&raw);
            }
            "validate_inputs" => {
                let _: ResultEnvelope<ValidateInputsResultDto> = parse(&raw);
            }
            "evaluate_gates" => {
                let _: ResultEnvelope<GateResultDto> = parse(&raw);
            }
            "live_guidance" => {
                let _: ResultEnvelope<GuidanceResultDto> = parse(&raw);
            }
            "check_compatibility" => {
                let _: ResultEnvelope<CompatibilityResultDto> = parse(&raw);
            }
            role => panic!("unknown fixture role {role}"),
        }
    }

    let invalid: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/provider/invalid-results.json")).unwrap();
    for fixture in invalid.as_array().unwrap() {
        let raw = serde_json::to_vec(&fixture["value"]).unwrap();
        let parsed =
            parse_strict::<ResultEnvelope<DescribeResultDto>>(&raw, PROVIDER_RESULT_STDOUT_BYTES);
        if fixture["case"] == "unsupported-major" {
            let (envelope, _) = parsed.unwrap();
            assert!(require_supported_major(envelope.protocol_major).is_err());
        } else {
            assert!(parsed.is_err(), "{}", fixture["case"]);
        }
    }
}

#[test]
fn published_schemas_pin_protocol_major_and_operation_role() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/provider/v1");
    for (name, role) in [
        ("describe-request.json", "describe"),
        ("describe-result.json", "describe"),
        ("validate-inputs-request.json", "validate_inputs"),
        ("validate-inputs-result.json", "validate_inputs"),
        ("evaluate-gates-request.json", "evaluate_gates"),
        ("evaluate-gates-result.json", "evaluate_gates"),
        ("live-guidance-request.json", "live_guidance"),
        ("live-guidance-result.json", "live_guidance"),
        ("check-compatibility-request.json", "check_compatibility"),
        ("check-compatibility-result.json", "check_compatibility"),
    ] {
        let schema: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join(name)).unwrap()).unwrap();
        assert_eq!(schema["properties"]["protocol_major"]["const"], 1, "{name}");
        assert_eq!(schema["properties"]["role"]["const"], role, "{name}");
        if name.ends_with("-request.json") {
            assert_eq!(
                schema["$defs"]["RegistrationDto"]["properties"]["config_revision"]["minimum"], 1,
                "{name}"
            );
            assert_eq!(
                schema["$defs"]["RegistrationDto"]["properties"]["timeout_seconds"]["minimum"], 1,
                "{name}"
            );
        }
    }
}

#[test]
fn published_schemas_carry_runtime_bound_markers() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/provider/v1");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let schema: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            schema["x-loop-engine-bounds"]["provider_request_json_bytes"],
            PROVIDER_REQUEST_JSON_BYTES,
            "{}",
            path.display()
        );
        assert_eq!(
            schema["x-loop-engine-bounds"]["provider_result_stdout_bytes"],
            PROVIDER_RESULT_STDOUT_BYTES,
            "{}",
            path.display()
        );
    }
}
