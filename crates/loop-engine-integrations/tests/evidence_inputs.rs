use std::fs;

use loop_engine_core::model::evidence::EvidenceSource;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_integrations::evidence_inputs::{InlineEvidenceLoadError, load_optional};

fn observed_at() -> ObservedAt {
    ObservedAt::parse("2026-07-22T12:00:00Z").unwrap()
}

fn load_document(
    document: &str,
) -> Result<Vec<loop_engine_core::model::evidence::EvidenceRecord>, InlineEvidenceLoadError> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("evidence.json");
    fs::write(&path, document).unwrap();
    load_optional(Some(&path), observed_at())
}

#[test]
fn strict_inline_evidence_maps_caller_records_without_dereferencing_locators() {
    let records = load_document(
        r#"[{"id":"artifact-1","kind":"report","locator":"opaque:missing","digest":"sha256:abc","media_type":"application/json","metadata":{"score":1.5,"nested":{"ok":true}}}]"#,
    )
    .unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id().as_str(), "artifact-1");
    assert_eq!(records[0].locator(), "opaque:missing");
    assert_eq!(records[0].source(), EvidenceSource::Caller);
    assert_eq!(records[0].observed_at(), observed_at());
}

#[test]
fn strict_inline_evidence_rejects_duplicate_keys_trailing_values_and_non_array_roots() {
    let duplicate = load_document(
        r#"[{"id":"artifact-1","id":"artifact-2","kind":"report","locator":"opaque:x"}]"#,
    )
    .unwrap_err();
    assert!(matches!(duplicate, InlineEvidenceLoadError::Json(_)));

    let trailing = load_document("[] []").unwrap_err();
    assert!(matches!(trailing, InlineEvidenceLoadError::Json(_)));

    let root = load_document(r#"{"items":[]}"#).unwrap_err();
    assert!(matches!(root, InlineEvidenceLoadError::RootNotArray));
}

#[test]
fn strict_inline_evidence_rejects_unknown_fields_duplicate_ids_and_invalid_timestamps() {
    let unknown =
        load_document(r#"[{"id":"artifact-1","kind":"report","locator":"opaque:x","extra":true}]"#)
            .unwrap_err();
    assert!(matches!(unknown, InlineEvidenceLoadError::Shape(_)));

    let duplicate_ids = load_document(
        r#"[{"id":"artifact-1","kind":"report","locator":"opaque:x"},{"id":"artifact-1","kind":"report","locator":"opaque:y"}]"#,
    )
    .unwrap_err();
    assert!(matches!(duplicate_ids, InlineEvidenceLoadError::Shape(_)));

    let timestamp = load_document(
        r#"[{"id":"artifact-1","kind":"report","locator":"opaque:x","observed_at":"tomorrow"}]"#,
    )
    .unwrap_err();
    assert!(
        matches!(timestamp, InlineEvidenceLoadError::Field { path, .. } if path == "/0/observed_at")
    );
}
