//! Versioned `journal.jsonl` encoder for audit export (T116).

use serde_json::Value;

use crate::persistence::mapping::{self, MappingError};
use crate::persistence::records::JournalRecord;

use super::{ExportError, canonical_json};

pub fn encode_journal_jsonl(records: &[JournalRecord]) -> Result<Vec<u8>, ExportError> {
    let mut output = Vec::new();
    for record in records {
        mapping::validate_journal_record(record).map_err(map_mapping_error)?;
        let parsed: Value =
            serde_json::from_str(&record.encoded_payload_json).map_err(|error| {
                ExportError::PersistenceFailed {
                    message: format!("encoded_payload_json: {error}"),
                }
            })?;
        let line = canonical_json(&parsed);
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn map_mapping_error(error: MappingError) -> ExportError {
    ExportError::PersistenceFailed {
        message: error.to_string(),
    }
}
