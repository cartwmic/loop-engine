//! Invokes the built production CLI and parses one structured envelope per invocation (T144).

use std::collections::BTreeMap;
use std::fs;

use super::sandbox::E2eSandbox;
use super::strict_json::{StrictJsonError, parse_strict_json_value};
use assert_cmd::Command;
use serde_json::Value;

/// Maximum encoded UTF-8 bytes for one structured CLI document ([cli-contract.md]).
const STRUCTURED_CLI_ENVELOPE_BYTES: usize = 4_194_304;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub transcript_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredDocument {
    pub value: Value,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreDispatchFailure {
    pub value: Value,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredParseError {
    Empty,
    InvalidUtf8,
    Oversized { max: usize, actual: usize },
    NewlineBoundary,
    TrailingContent,
    RootNotObject,
    Malformed(String),
    DuplicateKey { path: String, key: String },
}

impl std::fmt::Display for StructuredParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("structured payload is empty"),
            Self::InvalidUtf8 => formatter.write_str("structured payload is not valid UTF-8"),
            Self::Oversized { max, actual } => {
                write!(
                    formatter,
                    "structured payload exceeds {max} bytes (actual {actual})"
                )
            }
            Self::NewlineBoundary => {
                formatter.write_str("structured payload must end with exactly one newline")
            }
            Self::TrailingContent => formatter
                .write_str("structured payload contains trailing content after first JSON value"),
            Self::RootNotObject => {
                formatter.write_str("structured payload root must be a JSON object")
            }
            Self::Malformed(message) => {
                write!(formatter, "structured payload JSON is malformed: {message}")
            }
            Self::DuplicateKey { path, key } => write!(
                formatter,
                "structured payload contains duplicate object key at {path}: {key}"
            ),
        }
    }
}

impl std::error::Error for StructuredParseError {}

/// Invokes the built `loop-engine` binary in an isolated sandbox; never calls in-process handlers.
pub struct CliRunner<'a> {
    sandbox: &'a E2eSandbox,
}

impl<'a> CliRunner<'a> {
    pub fn new(sandbox: &'a E2eSandbox) -> Self {
        Self { sandbox }
    }

    pub fn run_human(&self, label: &str, args: &[&str]) -> CliInvocation {
        self.run(label, OutputFormat::Human, args)
    }

    pub fn run_json(&self, label: &str, args: &[&str]) -> CliInvocation {
        self.run(label, OutputFormat::Json, args)
    }

    pub fn run(&self, label: &str, format: OutputFormat, args: &[&str]) -> CliInvocation {
        let mut argv = vec!["loop-engine".to_owned()];
        if format == OutputFormat::Json {
            argv.push("--format".into());
            argv.push("json".into());
        }
        argv.extend(args.iter().map(|arg| (*arg).to_owned()));

        let mut env = BTreeMap::new();
        env.insert(
            "LOOP_ENGINE_HOME".into(),
            self.sandbox.loop_engine_home().display().to_string(),
        );

        let mut command = Command::cargo_bin("loop-engine").expect("loop-engine binary");
        command.current_dir(self.sandbox.caller_cwd());
        for key in E2eSandbox::isolated_env_removals() {
            command.env_remove(key);
        }
        for (key, value) in &env {
            command.env(key, value);
        }
        if format == OutputFormat::Json {
            command.args(["--format", "json"]);
        }
        command.args(args);

        let output = command.output().expect("spawn loop-engine");
        let exit_code = output.status.code();
        let invocation = CliInvocation {
            argv,
            env,
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code,
            transcript_path: self.sandbox.allocate_transcript_path(label),
        };
        write_transcript(&invocation);
        invocation
    }
}

pub fn parse_structured_stdout(bytes: &[u8]) -> Result<StructuredDocument, StructuredParseError> {
    parse_single_json_document(bytes)
}

pub fn parse_pre_dispatch_stderr(bytes: &[u8]) -> Result<PreDispatchFailure, StructuredParseError> {
    let document = parse_single_json_document(bytes)?;
    Ok(PreDispatchFailure {
        value: document.value,
        raw: document.raw,
    })
}

fn write_transcript(invocation: &CliInvocation) {
    let payload = serde_json::json!({
        "argv": invocation.argv,
        "env": invocation.env,
        "exit_code": invocation.exit_code,
        "stdout": String::from_utf8_lossy(&invocation.stdout),
        "stderr": String::from_utf8_lossy(&invocation.stderr),
    });
    fs::write(
        &invocation.transcript_path,
        serde_json::to_string_pretty(&payload).expect("transcript serializes"),
    )
    .expect("write invocation transcript");
}

fn parse_single_json_document(bytes: &[u8]) -> Result<StructuredDocument, StructuredParseError> {
    if bytes.is_empty() {
        return Err(StructuredParseError::Empty);
    }
    if bytes.len() > STRUCTURED_CLI_ENVELOPE_BYTES {
        return Err(StructuredParseError::Oversized {
            max: STRUCTURED_CLI_ENVELOPE_BYTES,
            actual: bytes.len(),
        });
    }
    if !bytes.ends_with(b"\n") || bytes.ends_with(b"\n\n") {
        return Err(StructuredParseError::NewlineBoundary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| StructuredParseError::InvalidUtf8)?;
    let object = text.trim_end_matches('\n');
    if object.is_empty() {
        return Err(StructuredParseError::Empty);
    }
    let value = parse_strict_json_value(object).map_err(map_strict_json_error)?;
    if !value.is_object() {
        return Err(StructuredParseError::RootNotObject);
    }
    Ok(StructuredDocument {
        value,
        raw: bytes.to_vec(),
    })
}

fn map_strict_json_error(error: StrictJsonError) -> StructuredParseError {
    match error {
        StrictJsonError::Malformed(message) => StructuredParseError::Malformed(message),
        StrictJsonError::TrailingContent => StructuredParseError::TrailingContent,
        StrictJsonError::DuplicateKey { path, key } => {
            StructuredParseError::DuplicateKey { path, key }
        }
    }
}
