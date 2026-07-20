use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_integrations::provider_protocol::canonical::graph_bytes;
use loop_engine_integrations::provider_protocol::dto::GraphDto;
use loop_engine_integrations::provider_protocol::graph::map_graph;
use loop_engine_integrations::provider_protocol::validation::{
    PROVIDER_RESULT_STDOUT_BYTES, parse_strict,
};
use sha2::{Digest, Sha256};

fn canonical(raw: &str) -> (String, String) {
    let (dto, _) = parse_strict::<GraphDto>(raw.as_bytes(), PROVIDER_RESULT_STDOUT_BYTES).unwrap();
    let graph = map_graph(dto).unwrap();
    let projection = SemanticGraphProjection::from_validated(&graph);
    let bytes = graph_bytes(&projection).unwrap();
    let digest = Sha256::digest(&bytes);
    let revision = format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    (String::from_utf8(bytes).unwrap(), revision)
}

#[test]
fn gv01_minimal_graph_is_byte_exact() {
    let raw = include_str!("fixtures/graphs/gv01-minimal.json");
    let expected = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}}],"transitions":[]}"#;
    let (bytes, revision) = canonical(raw);
    assert_eq!(bytes, expected);
    assert_eq!(
        revision,
        "sha256:6fd8334d3ebc9290b92e18b9667ff6072ca013f2295930bc4ffdf9a071b89d77"
    );
}

#[test]
fn gv02_and_gv03_reordering_are_equivalent() {
    let first = include_str!("fixtures/graphs/gv02-ordered.json");
    let reordered = include_str!("fixtures/graphs/gv03-reordered.json");
    let (bytes, revision) = canonical(first);
    assert_eq!(canonical(reordered), (bytes.clone(), revision.clone()));
    assert_eq!(bytes.len(), 400);
    assert_eq!(
        revision,
        "sha256:4584a384f85d331737718124c7b201a57e12472fb4921f066cfa49f17d7d28a7"
    );
}

#[test]
fn metadata_numbers_match_jcs_golden_vectors() {
    let raw = r#"{
      "initial_state":"draft","states":[{"id":"draft","static_guidance":"Go.","final":false}],
      "transitions":[],"input_declarations":[],"live_guidance_supported":false,
      "metadata":{"count":42.0,"large":1e+21,"ratio":1.50,"tiny":5e-324}}"#;
    let (bytes, revision) = canonical(raw);
    assert!(bytes.contains(r#""metadata":{"count":42,"large":1e+21,"ratio":1.5,"tiny":5e-324}"#));
    assert_eq!(
        revision,
        "sha256:d6586d19813d7238e60b389e85ac7c293885c95445ed6db324d61279ce85a54f"
    );

    let negative_zero = raw.replace(
        r#""count":42.0,"large":1e+21,"ratio":1.50,"tiny":5e-324"#,
        r#""offset":-0"#,
    );
    let (bytes, revision) = canonical(&negative_zero);
    assert!(bytes.contains(r#""metadata":{"offset":0}"#));
    assert_eq!(
        revision,
        "sha256:d10f0a20022ee783a2532adca5c8251a861a8537d493107e51c1c1ac9b3703d9"
    );
}

#[test]
fn every_digest_relevant_field_changes_identity_while_empty_metadata_does_not() {
    let baseline = r#"{
      "initial_state":"a","states":[{"id":"a","final":false,"static_guidance":{"kind":"none"}},{"id":"b","final":true,"static_guidance":{"kind":"none"}}],
      "transitions":[{"source_state":"a","event":"go","target_state":"b","gate_ids":["g"]}],
      "input_declarations":[{"id":"i","kind":"text","required":false}],
      "live_guidance_supported":false}"#;
    let base = canonical(baseline).1;
    let changes = [
        baseline.replace(r#""initial_state":"a""#, r#""initial_state":"b""#),
        baseline.replace(r#""id":"a","final":false"#, r#""id":"a","final":true"#),
        baseline.replace(r#""kind":"none""#, r#""kind":"text","text":"x""#),
        baseline.replace(r#""event":"go""#, r#""event":"stop""#),
        baseline.replace(r#""target_state":"b""#, r#""target_state":"a""#),
        baseline.replace(r#""gate_ids":["g"]"#, r#""gate_ids":["h"]"#),
        baseline.replace(
            r#""kind":"text","required":false"#,
            r#""kind":"json","required":false"#,
        ),
        baseline.replace(r#""required":false"#, r#""required":true"#),
        baseline.replace(
            r#""live_guidance_supported":false"#,
            r#""live_guidance_supported":true"#,
        ),
        baseline.replace(
            r#""live_guidance_supported":false"#,
            r#""live_guidance_supported":false,"metadata":{"x":1}"#,
        ),
    ];
    for (index, changed) in changes.into_iter().enumerate() {
        if let Ok((_, revision)) = std::panic::catch_unwind(|| canonical(&changed)) {
            assert_ne!(revision, base, "change {index}: {changed}");
        }
    }
    let empty_metadata = baseline.replace(
        r#""live_guidance_supported":false"#,
        r#""live_guidance_supported":false,"metadata":{}"#,
    );
    assert_eq!(canonical(&empty_metadata).1, base);
}

#[test]
fn duplicate_keys_are_rejected_before_mapping() {
    assert!(parse_strict::<GraphDto>(
        br#"{"initial_state":"a","initial_state":"b","states":[],"transitions":[],"input_declarations":[],"live_guidance_supported":false}"#,
        PROVIDER_RESULT_STDOUT_BYTES
    )
    .is_err());
}
