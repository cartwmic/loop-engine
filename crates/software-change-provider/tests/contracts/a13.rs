use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::support;

#[test]
fn standard_profile_and_task_packet_template_carry_doc_integration_contract() {
    let standard = support::load_profile("standard");
    let validation_axes: BTreeSet<&str> = standard["review_policies"]["validation-review"]
        .as_array()
        .expect("validation-review axes")
        .iter()
        .map(|entry| entry["id"].as_str().expect("axis id"))
        .collect();
    assert_eq!(
        validation_axes,
        BTreeSet::from(["docs-integrated", "intent-delivered"])
    );

    for subject in ["implementation-report.json", "validation-report.json"] {
        let schema = &standard["artifact_schemas"][subject];
        let required: BTreeSet<&str> = schema["required"]
            .as_array()
            .expect("schema required")
            .iter()
            .map(|entry| entry.as_str().expect("required field"))
            .collect();
        assert!(required.contains("revision"));
        assert!(required.contains("author"));
        assert!(required.contains("coverage"));
    }

    let template_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/templates/task-packet.md");
    let template = fs::read_to_string(&template_path).expect("read task-packet template");
    let lower = template.to_ascii_lowercase();
    assert!(lower.contains("doc integration"));
    assert!(lower.contains("authoritative"));
    assert!(lower.contains("deliverable"));

    // Keep criterion tied to shipped JSON data rather than a test-local copy.
    assert!(standard["review_policies"].is_object());
    assert!(standard["artifact_schemas"].is_object());
}
