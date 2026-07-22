use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config;
use crate::protocol::{AnyRequest, ProviderRole};
use crate::scenarios::Scenario;

pub const MAX_REQUEST_FACT_BYTES: u64 = 65_536;

#[derive(Debug, Serialize, Deserialize)]
pub struct RawRequestFacts {
    pub protocol_major: u32,
    pub role: ProviderRole,
    pub invocation_id: String,
    pub payload_byte_length: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub invocation_id: String,
    pub role: ProviderRole,
    pub executable: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub scenario: String,
    pub request: RawRequestFacts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_ordinal: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_facts: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn append_entry(path: &Path, entry: &LedgerEntry) -> Result<(), LedgerError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(entry)?;
    line.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

pub fn request_facts(request: &AnyRequest, payload_byte_length: usize) -> RawRequestFacts {
    RawRequestFacts {
        protocol_major: request.protocol_major,
        role: request.role,
        invocation_id: request.invocation_id.clone(),
        payload_byte_length: (payload_byte_length as u64).min(MAX_REQUEST_FACT_BYTES),
    }
}

pub fn record(
    path: Option<&Path>,
    request: &AnyRequest,
    payload_byte_length: usize,
    scenario: Scenario,
    invocation_ordinal: Option<u64>,
) -> Result<(), LedgerError> {
    let Some(path) = path else {
        return Ok(());
    };
    let (digest_mode, scenario_facts) = scenario.ledger_facts(invocation_ordinal);
    append_entry(
        path,
        &LedgerEntry {
            invocation_id: request.invocation_id.clone(),
            role: request.role,
            executable: config::executable_path(),
            argv: config::argv_snapshot(),
            working_directory: config::working_directory(),
            scenario: scenario.as_str().to_string(),
            request: request_facts(request, payload_byte_length),
            invocation_ordinal,
            digest_mode,
            scenario_facts,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::protocol::RegistrationDto;
    use crate::scenarios::Scenario;
    use crate::test_support::TempDir;

    fn sample_request(invocation_id: &str) -> AnyRequest {
        AnyRequest {
            protocol_major: 1,
            role: ProviderRole::Describe,
            invocation_id: invocation_id.to_string(),
            registration: RegistrationDto {
                registration_id: "reg".into(),
                config_revision: 1,
                executable: "/tmp/scenario-provider".into(),
                argv: vec![],
                working_directory: "/tmp".into(),
                timeout_seconds: 60,
            },
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn append_only_jsonl_is_concurrent_safe() {
        let temp = TempDir::new("ledger-unit");
        let ledger = temp.path().join("invocations.jsonl");
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let ledger = ledger.clone();
                thread::spawn(move || {
                    record(
                        Some(&ledger),
                        &sample_request(&format!("inv-{index}")),
                        2,
                        Scenario::GraphLinear,
                        Some(index as u64 + 1),
                    )
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let lines = std::fs::read_to_string(&ledger).unwrap();
        assert_eq!(lines.lines().count(), 8);
        let first: LedgerEntry = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(first.request.protocol_major, 1);
        assert!(first.request.payload_byte_length <= MAX_REQUEST_FACT_BYTES);
        assert!(first.request.invocation_id.starts_with("inv-"));
    }

    #[test]
    fn request_facts_exclude_registration_authority() {
        let request = sample_request("inv-facts");
        let facts = request_facts(&request, 128);
        let encoded = serde_json::to_value(facts).unwrap();
        assert!(encoded.get("registration").is_none());
        assert!(encoded.get("executable").is_none());
        assert_eq!(encoded["payload_byte_length"], 128);
    }
}
