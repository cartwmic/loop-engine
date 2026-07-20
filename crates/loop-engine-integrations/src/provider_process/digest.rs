use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

use sha2::{Digest, Sha256};

use loop_engine_core::capabilities::provider_catalog::ResolvedProviderConfig;
use loop_engine_core::model::provider::DigestObservation;

use crate::sha256_digest::DigestError;

#[cfg(target_os = "linux")]
const OPEN_NONBLOCK: i32 = 0o4000;
#[cfg(target_os = "macos")]
const OPEN_NONBLOCK: i32 = 0x0004;

pub fn observe_executable_digest(
    config: &ResolvedProviderConfig,
) -> Result<DigestObservation, DigestError> {
    match std::fs::metadata(config.config().executable()) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) | Err(_) => return Ok(DigestObservation::Unavailable),
    }
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_NONBLOCK)
        .open(config.config().executable())
    {
        Ok(file) => file,
        Err(_) => return Ok(DigestObservation::Unavailable),
    };
    match file.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) | Err(_) => return Ok(DigestObservation::Unavailable),
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return Ok(DigestObservation::Unavailable),
        };
        digest.update(&buffer[..read]);
    }
    let encoded = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    DigestObservation::observed(format!("sha256:{encoded}"))
        .map_err(|error| DigestError::Model(error.to_string()))
}
