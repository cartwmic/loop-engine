use std::env;
use std::path::PathBuf;

use thiserror::Error;

use crate::scenarios::Scenario;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub scenario: Scenario,
    pub ledger_path: Option<PathBuf>,
    pub ordinal_path: Option<PathBuf>,
    pub barrier_dir: Option<PathBuf>,
    pub barrier_id: String,
    pub barrier_action: BarrierAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierAction {
    None,
    Reached,
    Release,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required --scenario")]
    MissingScenario,
    #[error("unknown argument: {0}")]
    UnknownArg(String),
    #[error("duplicate flag: {0}")]
    DuplicateFlag(&'static str),
    #[error("flag {0} requires a value")]
    MissingValue(&'static str),
    #[error("invalid barrier action: {0}")]
    InvalidBarrierAction(String),
}

pub fn parse_config() -> Result<Config, ConfigError> {
    let mut args = env::args().skip(1);
    let mut scenario = None;
    let mut ledger_path = None;
    let mut ordinal_path = None;
    let mut barrier_dir = None;
    let mut barrier_id = "default".to_string();
    let mut barrier_action = BarrierAction::None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => {
                if scenario.is_some() {
                    return Err(ConfigError::DuplicateFlag("--scenario"));
                }
                let value = args.next().ok_or(ConfigError::MissingValue("--scenario"))?;
                scenario = Some(Scenario::parse(&value).ok_or_else(|| {
                    ConfigError::UnknownArg(format!("unknown scenario: {value}"))
                })?);
            }
            "--ledger-path" => {
                if ledger_path.is_some() {
                    return Err(ConfigError::DuplicateFlag("--ledger-path"));
                }
                ledger_path = Some(PathBuf::from(
                    args.next()
                        .ok_or(ConfigError::MissingValue("--ledger-path"))?,
                ));
            }
            "--ordinal-path" => {
                if ordinal_path.is_some() {
                    return Err(ConfigError::DuplicateFlag("--ordinal-path"));
                }
                ordinal_path = Some(PathBuf::from(
                    args.next()
                        .ok_or(ConfigError::MissingValue("--ordinal-path"))?,
                ));
            }
            "--barrier-dir" => {
                if barrier_dir.is_some() {
                    return Err(ConfigError::DuplicateFlag("--barrier-dir"));
                }
                barrier_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or(ConfigError::MissingValue("--barrier-dir"))?,
                ));
            }
            "--barrier-id" => {
                barrier_id = args
                    .next()
                    .ok_or(ConfigError::MissingValue("--barrier-id"))?;
            }
            "--barrier-action" => {
                let value = args
                    .next()
                    .ok_or(ConfigError::MissingValue("--barrier-action"))?;
                barrier_action = match value.as_str() {
                    "reached" => BarrierAction::Reached,
                    "release" => BarrierAction::Release,
                    other => return Err(ConfigError::InvalidBarrierAction(other.to_string())),
                };
            }
            other => return Err(ConfigError::UnknownArg(other.to_string())),
        }
    }

    Ok(Config {
        scenario: scenario.ok_or(ConfigError::MissingScenario)?,
        ledger_path,
        ordinal_path,
        barrier_dir,
        barrier_id,
        barrier_action,
    })
}

pub fn executable_path() -> String {
    env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "scenario-provider".to_string())
}

pub fn working_directory() -> String {
    env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

pub fn argv_snapshot() -> Vec<String> {
    env::args().collect()
}
