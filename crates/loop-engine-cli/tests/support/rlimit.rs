//! Unix external `RLIMIT_FSIZE` wrapper for late-sink E2E injection (T145).
//!
//! Production code receives no test branch ([operational-trace.md] § Deterministic
//! Unix SIGXFSZ / RLIMIT_FSIZE E2E contract). The wrapper ignores `SIGXFSZ` by its
//! POSIX signal number because Debian `dash` rejects the `SIGXFSZ` spelling, then
//! applies `ulimit -f` to the child shell because workspace `unsafe_code` is forbidden.
//!
//! `run_with_rlimit_fsize` uses the sandbox caller CWD and writes a harness transcript
//! beside ordinary CLI runner invocations.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::Command as AssertCommand;

use super::sandbox::E2eSandbox;

/// `sh` file-size-limit unit on supported hosts.
#[cfg(target_os = "macos")]
const BLOCK_SIZE: u64 = 1_024;
#[cfg(not(target_os = "macos"))]
const BLOCK_SIZE: u64 = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RlimitInvocation {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub transcript_path: PathBuf,
    pub byte_limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RlimitExecError {
    BinaryNotFound(String),
    Spawn(String),
    LimitNotEnforced { byte_limit: u64, write_bytes: u64 },
}

impl std::fmt::Display for RlimitExecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryNotFound(message) => {
                write!(formatter, "loop-engine binary not found: {message}")
            }
            Self::Spawn(message) => write!(formatter, "rlimit wrapper spawn failed: {message}"),
            Self::LimitNotEnforced {
                byte_limit,
                write_bytes,
            } => write!(
                formatter,
                "RLIMIT_FSIZE wrapper did not block a {write_bytes}-byte write under {byte_limit}-byte ceiling"
            ),
        }
    }
}

impl std::error::Error for RlimitExecError {}

fn ulimit_blocks(byte_limit: u64) -> u64 {
    byte_limit.div_ceil(BLOCK_SIZE).max(1)
}

fn shell_quote(value: &str) -> String {
    format!("'{value}'")
}

fn build_wrapped_command(
    home: &Path,
    caller: &Path,
    args: &[&str],
    byte_limit: u64,
) -> Result<Command, RlimitExecError> {
    let template = AssertCommand::cargo_bin("loop-engine")
        .map_err(|error| RlimitExecError::BinaryNotFound(error.to_string()))?;
    let binary = template.get_program().to_string_lossy().into_owned();
    let blocks = ulimit_blocks(byte_limit);

    let mut quoted = shell_quote(&binary);
    quoted.push_str(" --format json");
    for arg in args {
        quoted.push(' ');
        quoted.push_str(&shell_quote(arg));
    }

    let script = format!("trap '' 25\nulimit -f {blocks}\nexec {quoted}\n");

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .current_dir(caller)
        .env("LOOP_ENGINE_HOME", home);
    for key in E2eSandbox::isolated_env_removals() {
        if *key != "LOOP_ENGINE_HOME" {
            command.env_remove(key);
        }
    }
    Ok(command)
}

fn write_rlimit_transcript(invocation: &RlimitInvocation) {
    let payload = serde_json::json!({
        "kind": "rlimit_fsize",
        "byte_limit": invocation.byte_limit,
        "argv": invocation.argv,
        "env": invocation.env,
        "exit_code": invocation.exit_code,
        "stdout": String::from_utf8_lossy(&invocation.stdout),
        "stderr": String::from_utf8_lossy(&invocation.stderr),
    });
    fs::write(
        &invocation.transcript_path,
        serde_json::to_string_pretty(&payload).expect("rlimit transcript serializes"),
    )
    .expect("write rlimit transcript");
}

/// Runs the built `loop-engine` binary with `RLIMIT_FSIZE` applied to the child process.
///
/// Uses the sandbox caller CWD and records a transcript under `LOOP_ENGINE_HOME`.
pub fn run_with_rlimit_fsize(
    sandbox: &E2eSandbox,
    label: &str,
    args: &[&str],
    byte_limit: u64,
) -> Result<RlimitInvocation, RlimitExecError> {
    let mut argv = vec!["loop-engine".to_owned()];
    argv.push("--format".into());
    argv.push("json".into());
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));

    let mut env = BTreeMap::new();
    env.insert(
        "LOOP_ENGINE_HOME".into(),
        sandbox.loop_engine_home().display().to_string(),
    );

    let mut command = build_wrapped_command(
        sandbox.loop_engine_home(),
        sandbox.caller_cwd(),
        args,
        byte_limit,
    )?;
    let output = command
        .output()
        .map_err(|error| RlimitExecError::Spawn(error.to_string()))?;

    let invocation = RlimitInvocation {
        argv,
        env,
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code(),
        transcript_path: sandbox.allocate_transcript_path(label),
        byte_limit,
    };
    write_rlimit_transcript(&invocation);
    Ok(invocation)
}

/// Self-test helper: proves the same ulimit wrapper blocks writes beyond the ceiling.
pub(crate) fn verify_rlimit_blocks_writes(byte_limit: u64) -> Result<(), RlimitExecError> {
    let blocks = ulimit_blocks(byte_limit);
    let write_bytes = byte_limit.saturating_add(BLOCK_SIZE);
    let script = format!(
        "trap '' 25\nulimit -f {blocks}\ntarget=$(mktemp)\nif dd if=/dev/zero of=\"$target\" bs=1 count={write_bytes} 2>/dev/null; then\n  exit 0\nelse\n  exit 1\nfi\n"
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|error| RlimitExecError::Spawn(error.to_string()))?;
    if output.status.success() {
        return Err(RlimitExecError::LimitNotEnforced {
            byte_limit,
            write_bytes,
        });
    }
    Ok(())
}
