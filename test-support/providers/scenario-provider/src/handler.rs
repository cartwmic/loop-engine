use std::io;

use thiserror::Error;

use crate::barrier::Barrier;
use crate::config::{BarrierAction, Config};
use crate::ledger;
use crate::ordinal;
use crate::protocol::AnyRequest;
use crate::scenarios::payload::handle_request;
use crate::scenarios::process::{
    emit_stdout, finalize_process, parse_request, read_stdin_request, validate_request,
};

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ledger error: {0}")]
    Ledger(#[from] ledger::LedgerError),
    #[error("ordinal error: {0}")]
    Ordinal(#[from] ordinal::OrdinalError),
    #[error("barrier error: {0}")]
    Barrier(#[from] crate::barrier::BarrierError),
    #[error("protocol error: {0}")]
    Protocol(&'static str),
}

pub fn run(config: &Config) -> Result<(), HandlerError> {
    let raw = read_stdin_request()?;
    let request: AnyRequest = parse_request(&raw)?;
    validate_request(&request).map_err(HandlerError::Protocol)?;

    let invocation_ordinal = config
        .ordinal_path
        .as_deref()
        .map(ordinal::next)
        .transpose()?;

    ledger::record(
        config.ledger_path.as_deref(),
        &request,
        raw.len(),
        config.scenario,
        invocation_ordinal,
    )?;

    if let (Some(dir), action) = (&config.barrier_dir, config.barrier_action) {
        let barrier = Barrier::new(dir.clone(), config.barrier_id.clone());
        match action {
            BarrierAction::Reached => barrier.reached(&request.invocation_id)?,
            BarrierAction::Release => barrier.release()?,
            BarrierAction::None => {}
        }
    }

    let result = handle_request(config.scenario, &request, invocation_ordinal);
    let outcome = emit_stdout(config.scenario, &request, result)?;
    finalize_process(outcome)
}
