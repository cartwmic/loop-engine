//! Versioned `state.json` encoder for audit export (T116).

use serde_json::{Value, json};

use crate::persistence::records::{EvidenceRecordRow, RunRecord};

use super::{ExportError, canonical_json_bytes, canonical_value};

const EXPORT_SCHEMA_VERSION: u32 = 1;

pub fn encode_state_json(
    run_record: &RunRecord,
    evidence_rows: &[EvidenceRecordRow],
    exported_at: &str,
) -> Result<Vec<u8>, ExportError> {
    let graph = parse_json_object(
        &run_record.graph_canonical_projection_json,
        "graph_canonical_projection_json",
    )?;
    let inputs = parse_json_object(&run_record.inputs_json, "inputs_json")?;
    let evidence = evidence_rows
        .iter()
        .map(encode_evidence_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let state = json!({
        "created_at": run_record.created_at,
        "id": run_record.run_id,
        "label": run_record.label,
        "lifecycle": run_record.lifecycle,
        "lifecycle_version": run_record.lifecycle_version,
        "label_version": run_record.label_version,
        "state": run_record.current_state,
        "workflow_state_version": run_record.workflow_state_version,
    });
    let registration_binding = json!({
        "config_revision_at_create": run_record.config_revision_at_create,
        "registration_id": run_record.registration_id,
    });
    let graph_object = {
        let mut object = match graph {
            Value::Object(map) => map,
            _ => {
                return Err(ExportError::PersistenceFailed {
                    message: "graph_canonical_projection_json must be an object".into(),
                });
            }
        };
        object.insert(
            "graph_revision".into(),
            Value::String(run_record.graph_revision.clone()),
        );
        object.insert(
            "canonical_graph_version".into(),
            json!(run_record.canonical_graph_version),
        );
        Value::Object(object)
    };
    let payload = json!({
        "evidence": evidence,
        "export_schema_version": EXPORT_SCHEMA_VERSION,
        "exported_at": exported_at,
        "graph": graph_object,
        "inputs": inputs,
        "registration_binding": registration_binding,
        "run": state,
        "run_id": run_record.run_id,
    });
    canonical_json_bytes(&payload)
}

fn encode_evidence_entry(row: &EvidenceRecordRow) -> Result<Value, ExportError> {
    let metadata = match row.metadata_json.as_deref() {
        None => Value::Null,
        Some(raw) => parse_json_object_or_null(raw, "metadata_json")?,
    };
    Ok(json!({
        "created_at": row.created_at,
        "digest": row.digest,
        "evidence_id": row.evidence_id,
        "kind": row.kind,
        "locator": row.locator,
        "media_type": row.media_type,
        "metadata": metadata,
    }))
}

fn parse_json_object(raw: &str, field: &'static str) -> Result<Value, ExportError> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|error| ExportError::PersistenceFailed {
            message: format!("{field}: {error}"),
        })?;
    if !parsed.is_object() {
        return Err(ExportError::PersistenceFailed {
            message: format!("{field} must be a JSON object"),
        });
    }
    Ok(canonical_value(&parsed))
}

fn parse_json_object_or_null(raw: &str, field: &'static str) -> Result<Value, ExportError> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|error| ExportError::PersistenceFailed {
            message: format!("{field}: {error}"),
        })?;
    match &parsed {
        Value::Null => Ok(Value::Null),
        Value::Object(_) => Ok(canonical_value(&parsed)),
        _ => Err(ExportError::PersistenceFailed {
            message: format!("{field} must be a JSON object or null"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::records::{GV01_CANONICAL_GRAPH_JSON, GV01_GRAPH_REVISION};

    fn sample_run_record() -> RunRecord {
        RunRecord {
            run_id: "run-1".into(),
            registration_id: "reg-1".into(),
            config_revision_at_create: 1,
            current_state: "draft".into(),
            lifecycle: "active".into(),
            workflow_state_version: 1,
            lifecycle_version: 1,
            label_version: 1,
            label: None,
            graph_revision: GV01_GRAPH_REVISION.into(),
            canonical_graph_version: 1,
            graph_canonical_projection_json: GV01_CANONICAL_GRAPH_JSON.into(),
            inputs_json: "{}".into(),
            created_at: "2026-07-17T12:00:00.000Z".into(),
        }
    }

    fn sample_evidence_row(metadata_json: Option<String>) -> EvidenceRecordRow {
        EvidenceRecordRow {
            run_id: "run-1".into(),
            evidence_id: "ev-1".into(),
            kind: "artifact".into(),
            locator: "file:///tmp/x".into(),
            digest: None,
            media_type: None,
            metadata_json,
            source: "caller".into(),
            created_at: "2026-07-18T00:00:00.000Z".into(),
        }
    }

    fn exported_evidence_metadata(metadata_json: Option<String>) -> Value {
        let bytes = encode_state_json(
            &sample_run_record(),
            &[sample_evidence_row(metadata_json)],
            "2026-07-18T00:00:00.000Z",
        )
        .expect("encode state");
        let parsed: Value = serde_json::from_slice(&bytes).expect("parse state");
        parsed["evidence"][0]["metadata"].clone()
    }

    #[test]
    fn evidence_metadata_absent_exports_null() {
        assert_eq!(exported_evidence_metadata(None), Value::Null);
    }

    #[test]
    fn evidence_metadata_empty_object_exports_object() {
        assert_eq!(exported_evidence_metadata(Some("{}".into())), json!({}));
    }

    #[test]
    fn evidence_metadata_nonempty_object_preserved_with_canonical_ordering() {
        assert_eq!(
            exported_evidence_metadata(Some(r#"{"z":1,"a":2}"#.into())),
            json!({"a": 2, "z": 1})
        );
    }

    #[test]
    fn evidence_metadata_null_literal_exports_null() {
        assert_eq!(exported_evidence_metadata(Some("null".into())), Value::Null);
    }

    #[test]
    fn evidence_metadata_array_is_rejected() {
        let error = encode_state_json(
            &sample_run_record(),
            &[sample_evidence_row(Some("[]".into()))],
            "2026-07-18T00:00:00.000Z",
        )
        .expect_err("array metadata");
        assert!(matches!(error, ExportError::PersistenceFailed { .. }));
    }

    #[test]
    fn evidence_metadata_scalar_is_rejected() {
        let error = encode_state_json(
            &sample_run_record(),
            &[sample_evidence_row(Some("\"note\"".into()))],
            "2026-07-18T00:00:00.000Z",
        )
        .expect_err("scalar metadata");
        assert!(matches!(error, ExportError::PersistenceFailed { .. }));
    }

    #[test]
    fn inputs_json_must_be_object() {
        let mut run = sample_run_record();
        run.inputs_json = "[]".into();
        let error =
            encode_state_json(&run, &[], "2026-07-18T00:00:00.000Z").expect_err("array inputs");
        assert!(matches!(error, ExportError::PersistenceFailed { .. }));

        run.inputs_json = "\"value\"".into();
        let error =
            encode_state_json(&run, &[], "2026-07-18T00:00:00.000Z").expect_err("scalar inputs");
        assert!(matches!(error, ExportError::PersistenceFailed { .. }));
    }
}
