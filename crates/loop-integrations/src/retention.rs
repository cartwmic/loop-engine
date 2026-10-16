//! Shared builder for provider-authored `retained-artifacts` capture effects.
//!
//! A provider decides which checked transitions leave an artifact-owning
//! phase and which files that phase owns; this module only reads those files'
//! exact bytes and shapes one opaque context effect. Core never interprets it.

use loop_core::{CaptureExit, ContextAppendEffect, ContextRecord, Transition, Workflow};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Context kind of a retained-artifacts record.
pub const RETAINED_ARTIFACTS_KIND: &str = "retained-artifacts";

/// The `capture` request the engine sends to a provider whose workflow
/// declares `capture`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRequest {
    pub operation: String,
    pub workflow: Workflow,
    pub initial_input: Value,
    pub context: Vec<ContextRecord>,
    pub transition: Transition,
    pub exit: CaptureExit,
}

/// The provider's `capture` answer: `{}` or `{"context_append": effect}`.
pub fn capture_response(effect: Option<ContextAppendEffect>) -> Value {
    match effect {
        Some(effect) => json!({"context_append": {"kind": effect.kind, "data": effect.data}}),
        None => json!({}),
    }
}

/// One artifact a phase is leaving with: its stable name and absolute path.
#[derive(Clone, Debug)]
pub struct RetainedArtifact {
    pub name: String,
    pub path: PathBuf,
}

impl RetainedArtifact {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
        }
    }
}

/// Build the `retained-artifacts` effect for one capturing transition.
///
/// Each artifact entry carries its exact bytes (`encoding` `utf-8` when the
/// bytes are valid UTF-8, otherwise `base64`), size and sha256. When the
/// bytes equal the latest earlier retained copy of the same artifact name in
/// this run, the entry instead names that record in `same_as` and stores no
/// content. A missing file is recorded with `absent: true`. Any other read
/// error is returned so the caller can refuse the capture.
pub fn retained_artifacts_effect(
    context: &[ContextRecord],
    transition: &Transition,
    exit: CaptureExit,
    artifacts: &[RetainedArtifact],
) -> std::io::Result<ContextAppendEffect> {
    let mut entries = Vec::new();
    for artifact in artifacts {
        let path = artifact.path.display().to_string();
        let bytes = match std::fs::read(&artifact.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                entries.push(json!({"name": artifact.name, "path": path, "absent": true}));
                continue;
            }
            Err(error) => return Err(error),
        };
        let sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
        let mut entry = Map::new();
        entry.insert("name".into(), json!(artifact.name));
        entry.insert("path".into(), json!(path));
        entry.insert("sha256".into(), json!(sha256));
        entry.insert("size".into(), json!(bytes.len()));
        match latest_content_holder(context, &artifact.name) {
            Some((holder, previous)) if previous == sha256 => {
                entry.insert("same_as".into(), json!(holder));
            }
            _ => match String::from_utf8(bytes) {
                Ok(text) => {
                    entry.insert("encoding".into(), json!("utf-8"));
                    entry.insert("content".into(), json!(text));
                }
                Err(error) => {
                    entry.insert("encoding".into(), json!("base64"));
                    entry.insert("content".into(), json!(base64(error.as_bytes())));
                }
            },
        }
        entries.push(Value::Object(entry));
    }
    Ok(ContextAppendEffect::new(
        RETAINED_ARTIFACTS_KIND,
        json!({
            "schema_version": 1,
            "transition": {
                "source": transition.source,
                "event": transition.event,
                "target": transition.target,
            },
            "exit": exit,
            "artifacts": entries,
        }),
    ))
}

/// The latest earlier retained copy of `name` that is present (not absent):
/// the record id holding its content and its sha256. A `same_as` entry is
/// followed to the record it names so references always point at content.
fn latest_content_holder(context: &[ContextRecord], name: &str) -> Option<(String, String)> {
    let mut ordered: Vec<&ContextRecord> = context
        .iter()
        .filter(|record| record.kind == RETAINED_ARTIFACTS_KIND)
        .collect();
    ordered.sort_by_key(|record| record.sequence);
    ordered.iter().rev().find_map(|record| {
        let entry = record.data["artifacts"]
            .as_array()?
            .iter()
            .find(|entry| entry["name"] == name)?;
        if entry.get("absent").is_some() {
            return None;
        }
        let sha256 = entry["sha256"].as_str()?.to_owned();
        let holder = entry["same_as"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| record.id.as_str().to_owned());
        Some((holder, sha256))
    })
}

/// Standard base64 (RFC 4648, with padding).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{SemanticSequence, Timestamp};

    fn record(id: &str, sequence: u64, data: Value) -> ContextRecord {
        ContextRecord::new(
            id,
            RETAINED_ARTIFACTS_KIND,
            data,
            SemanticSequence::new(sequence),
            Timestamp::from_unix_millis(1),
        )
    }

    fn transition() -> Transition {
        Transition::checked("plan-adversarial-review", "approved", "implement")
    }

    #[test]
    fn retains_utf8_and_binary_bytes_exactly() {
        let directory = tempfile::tempdir().expect("tempdir");
        let text = directory.path().join("plan.json");
        let binary = directory.path().join("blob.bin");
        std::fs::write(&text, "{\"revision\": \"1\"}\n").expect("write");
        std::fs::write(&binary, [0xff_u8, 0x00, 0x61]).expect("write");
        let effect = retained_artifacts_effect(
            &[],
            &transition(),
            CaptureExit::Checked,
            &[
                RetainedArtifact::new("plan.json", &text),
                RetainedArtifact::new("blob.bin", &binary),
            ],
        )
        .expect("effect");
        assert_eq!(effect.kind, RETAINED_ARTIFACTS_KIND);
        let artifacts = effect.data["artifacts"].as_array().expect("artifacts");
        assert_eq!(artifacts[0]["encoding"], "utf-8");
        assert_eq!(artifacts[0]["content"], "{\"revision\": \"1\"}\n");
        assert_eq!(artifacts[1]["encoding"], "base64");
        assert_eq!(artifacts[1]["content"], "/wBh");
        assert_eq!(artifacts[1]["size"], 3);
        assert_eq!(effect.data["exit"], "checked");
        assert_eq!(effect.data["transition"]["target"], "implement");
    }

    #[test]
    fn unchanged_bytes_reference_the_record_holding_content() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("plan.json");
        std::fs::write(&path, "same").expect("write");
        let first = retained_artifacts_effect(
            &[],
            &transition(),
            CaptureExit::Checked,
            &[RetainedArtifact::new("plan.json", &path)],
        )
        .expect("first");
        let holder = record("holder", 5, first.data.clone());
        let second = retained_artifacts_effect(
            std::slice::from_ref(&holder),
            &transition(),
            CaptureExit::Checked,
            &[RetainedArtifact::new("plan.json", &path)],
        )
        .expect("second");
        assert_eq!(second.data["artifacts"][0]["same_as"], "holder");
        assert!(second.data["artifacts"][0].get("content").is_none());
        // A reference to a reference still names the content holder.
        let referencing = record("referencing", 9, second.data.clone());
        let third = retained_artifacts_effect(
            &[holder, referencing],
            &transition(),
            CaptureExit::Override,
            &[RetainedArtifact::new("plan.json", &path)],
        )
        .expect("third");
        assert_eq!(third.data["artifacts"][0]["same_as"], "holder");
        assert_eq!(third.data["exit"], "override");
        // Changed bytes store content again.
        std::fs::write(&path, "changed").expect("write");
        let changed = retained_artifacts_effect(
            &[record("holder", 5, first.data)],
            &transition(),
            CaptureExit::Checked,
            &[RetainedArtifact::new("plan.json", &path)],
        )
        .expect("changed");
        assert_eq!(changed.data["artifacts"][0]["content"], "changed");
    }

    #[test]
    fn missing_file_is_recorded_absent() {
        let directory = tempfile::tempdir().expect("tempdir");
        let effect = retained_artifacts_effect(
            &[],
            &transition(),
            CaptureExit::Override,
            &[RetainedArtifact::new(
                "plan.json",
                directory.path().join("plan.json"),
            )],
        )
        .expect("effect");
        assert_eq!(effect.data["artifacts"][0]["absent"], true);
        assert!(effect.data["artifacts"][0].get("content").is_none());
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
}
