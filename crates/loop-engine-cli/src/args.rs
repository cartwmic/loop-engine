//! Shared CLI argument primitives (T121).
//!
//! Production root parsing exposes global flags only. Application argv grammar for all
//! 21 planned operations lives in private modules and is reachable through
//! [`parse_planned_application`] and [`register_exposed_route`] without redefining flags.

use std::ffi::OsString;

use clap::{ArgAction, Args, Command, CommandFactory, FromArgMatches, Parser, Subcommand};
use loop_engine_core::model::bounded::{
    COLLECTION_PAGE_DEFAULT_COUNT, COLLECTION_PAGE_MAX_COUNT, EVIDENCE_LOCATOR_UTF8_BYTES,
    FILESYSTEM_PATH_UTF8_BYTES, IDENTIFIER_UTF8_BYTES, NOTE_TEXT_UTF8_BYTES,
    OPAQUE_INTEGRITY_WIRE_UTF8_BYTES, PROVIDER_ARGV_ELEMENT_COUNT,
    PROVIDER_ARGV_ELEMENT_UTF8_BYTES, PROVIDER_ARGV_ENCODED_TOTAL_BYTES,
    PROVIDER_HANDLE_UTF8_BYTES, RUN_LABEL_UTF8_BYTES,
};
use loop_engine_core::operations::catalog::OperationId;
use thiserror::Error;

pub const CLI_NAME: &str = "loop-engine";
pub const CLI_ABOUT: &str = "Loop engine control plane";

/// Global-only production root. No application subcommands are registered.
#[derive(Debug, Clone, Parser)]
#[command(
    name = CLI_NAME,
    about = CLI_ABOUT,
    disable_help_flag = true,
    disable_version_flag = true
)]
pub struct GlobalCli {
    #[arg(long)]
    pub format: Option<String>,
    #[arg(long, short = 'h', action = ArgAction::SetTrue)]
    pub help: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub version: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub list_operations: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}

impl GlobalCli {
    pub fn command() -> Command {
        <Self as CommandFactory>::command()
    }

    pub fn usage_help() -> String {
        [
            "loop-engine — workflow control plane",
            "",
            "Usage:",
            "  loop-engine [OPTIONS]",
            "",
            "Global options:",
            "  -h, --help               Print usage help",
            "      --version            Print version information",
            "      --list-operations    List the closed 21-operation argv surface",
            "      --format <human|json>  Output rendering mode (default: human)",
            "",
            "Application subcommands are not registered in this build.",
            "Use --list-operations for the closed 21-operation argv surface.",
            "",
        ]
        .join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Human,
    Json,
}

pub fn parse_render_format(value: &str) -> Result<RenderFormat, &'static str> {
    match value {
        "human" => Ok(RenderFormat::Human),
        "json" => Ok(RenderFormat::Json),
        _ => Err("format must be human or json"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyntaxError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} exceeds {max} UTF-8 bytes (actual {actual})")]
    TooLong {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field} contains a prohibited control character")]
    Control { field: &'static str },
    #[error("{field} has invalid syntax")]
    InvalidSyntax { field: &'static str },
    #[error("{field} must be between 1 and {max} inclusive (actual {actual})")]
    OutOfRange {
        field: &'static str,
        max: u16,
        actual: u64,
    },
    #[error("{field} exceeds item count {max} (actual {actual})")]
    TooMany {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("{field1} and {field2} are mutually exclusive")]
    Conflict {
        field1: &'static str,
        field2: &'static str,
    },
    #[error("{field} must be supplied")]
    Required { field: &'static str },
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error(transparent)]
    Grammar(#[from] clap::Error),
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
}

pub type SyntaxResult<T> = Result<T, SyntaxError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxText {
    value: String,
}

impl SyntaxText {
    pub fn into_inner(self) -> String {
        self.value
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxIdentifier(SyntaxText);

impl SyntaxIdentifier {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxHandle(SyntaxText);

impl SyntaxHandle {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxProviderTarget(SyntaxText);

impl SyntaxProviderTarget {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxPath(SyntaxText);

impl SyntaxPath {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxOpaqueWire(SyntaxText);

impl SyntaxOpaqueWire {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxPageLimit(u16);

impl SyntaxPageLimit {
    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxProviderArgv {
    pub elements: Vec<SyntaxText>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxPositiveU64(u64);

impl SyntaxPositiveU64 {
    pub fn get(&self) -> u64 {
        self.0
    }
}

mod syntax {
    use super::*;

    pub fn bounded_text(
        field: &'static str,
        value: String,
        max: usize,
        require_non_empty: bool,
    ) -> SyntaxResult<SyntaxText> {
        if require_non_empty && value.is_empty() {
            return Err(SyntaxError::Empty { field });
        }
        if value.len() > max {
            return Err(SyntaxError::TooLong {
                field,
                max,
                actual: value.len(),
            });
        }
        if value.chars().any(char::is_control) {
            return Err(SyntaxError::Control { field });
        }
        Ok(SyntaxText { value })
    }

    pub fn identifier(field: &'static str, value: String) -> SyntaxResult<SyntaxIdentifier> {
        Ok(SyntaxIdentifier(bounded_text(
            field,
            value,
            IDENTIFIER_UTF8_BYTES,
            true,
        )?))
    }

    pub fn optional_identifier(
        field: &'static str,
        value: String,
    ) -> SyntaxResult<SyntaxIdentifier> {
        Ok(SyntaxIdentifier(bounded_text(
            field,
            value,
            IDENTIFIER_UTF8_BYTES,
            true,
        )?))
    }

    pub fn handle(value: String) -> SyntaxResult<SyntaxHandle> {
        if value.is_empty() {
            return Err(SyntaxError::Empty { field: "handle" });
        }
        if value.len() > PROVIDER_HANDLE_UTF8_BYTES {
            return Err(SyntaxError::TooLong {
                field: "handle",
                max: PROVIDER_HANDLE_UTF8_BYTES,
                actual: value.len(),
            });
        }
        if !is_handle_syntax(&value) {
            return Err(SyntaxError::InvalidSyntax { field: "handle" });
        }
        Ok(SyntaxHandle(SyntaxText { value }))
    }

    pub fn provider_target(value: String) -> SyntaxResult<SyntaxProviderTarget> {
        if is_handle_syntax(&value) {
            handle(value.clone())?;
            return Ok(SyntaxProviderTarget(SyntaxText { value }));
        }
        identifier("target", value.clone())?;
        Ok(SyntaxProviderTarget(SyntaxText { value }))
    }

    pub fn path(field: &'static str, value: String) -> SyntaxResult<SyntaxPath> {
        Ok(SyntaxPath(bounded_text(
            field,
            value,
            FILESYSTEM_PATH_UTF8_BYTES,
            true,
        )?))
    }

    pub fn optional_path(field: &'static str, value: String) -> SyntaxResult<SyntaxPath> {
        Ok(SyntaxPath(bounded_text(
            field,
            value,
            FILESYSTEM_PATH_UTF8_BYTES,
            true,
        )?))
    }

    pub fn opaque_wire(field: &'static str, value: String) -> SyntaxResult<SyntaxOpaqueWire> {
        Ok(SyntaxOpaqueWire(bounded_text(
            field,
            value,
            OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
            true,
        )?))
    }

    pub fn optional_opaque_wire(
        field: &'static str,
        value: String,
    ) -> SyntaxResult<SyntaxOpaqueWire> {
        if value.is_empty() {
            return Ok(SyntaxOpaqueWire(SyntaxText { value }));
        }
        opaque_wire(field, value)
    }

    pub fn page_limit(raw: Option<String>) -> SyntaxResult<SyntaxPageLimit> {
        match raw {
            None => Ok(SyntaxPageLimit(COLLECTION_PAGE_DEFAULT_COUNT)),
            Some(value) => {
                let parsed = value
                    .parse::<u64>()
                    .map_err(|_| SyntaxError::InvalidSyntax { field: "limit" })?;
                if parsed == 0 || parsed > u64::from(COLLECTION_PAGE_MAX_COUNT) {
                    return Err(SyntaxError::OutOfRange {
                        field: "limit",
                        max: COLLECTION_PAGE_MAX_COUNT,
                        actual: parsed,
                    });
                }
                Ok(SyntaxPageLimit(parsed as u16))
            }
        }
    }

    pub fn optional_page_cursor(raw: Option<String>) -> SyntaxResult<Option<SyntaxOpaqueWire>> {
        optional_opaque_page_cursor("cursor", raw)
    }

    pub fn optional_warning_cursor(raw: Option<String>) -> SyntaxResult<Option<SyntaxOpaqueWire>> {
        optional_opaque_page_cursor("warning-cursor", raw)
    }

    fn optional_opaque_page_cursor(
        field: &'static str,
        raw: Option<String>,
    ) -> SyntaxResult<Option<SyntaxOpaqueWire>> {
        match raw {
            None => Ok(None),
            Some(value) if value.is_empty() => Ok(None),
            Some(value) => Ok(Some(optional_opaque_wire(field, value)?)),
        }
    }

    pub fn provider_argv(values: Vec<String>) -> SyntaxResult<SyntaxProviderArgv> {
        if values.len() > PROVIDER_ARGV_ELEMENT_COUNT {
            return Err(SyntaxError::TooMany {
                field: "arg",
                max: PROVIDER_ARGV_ELEMENT_COUNT,
                actual: values.len(),
            });
        }
        let mut total = 0usize;
        let mut elements = Vec::with_capacity(values.len());
        for value in values {
            let element = bounded_text("arg", value, PROVIDER_ARGV_ELEMENT_UTF8_BYTES, false)?;
            total += element.as_str().len();
            elements.push(element);
        }
        if total > PROVIDER_ARGV_ENCODED_TOTAL_BYTES {
            return Err(SyntaxError::TooLong {
                field: "arg",
                max: PROVIDER_ARGV_ENCODED_TOTAL_BYTES,
                actual: total,
            });
        }
        Ok(SyntaxProviderArgv { elements })
    }

    pub fn positive_u64(field: &'static str, raw: String) -> SyntaxResult<SyntaxPositiveU64> {
        let parsed = raw
            .parse::<u64>()
            .map_err(|_| SyntaxError::InvalidSyntax { field })?;
        if parsed == 0 {
            return Err(SyntaxError::OutOfRange {
                field,
                max: u16::MAX,
                actual: 0,
            });
        }
        Ok(SyntaxPositiveU64(parsed))
    }

    pub fn optional_positive_u64(
        field: &'static str,
        raw: Option<String>,
    ) -> SyntaxResult<Option<SyntaxPositiveU64>> {
        raw.map(|value| positive_u64(field, value)).transpose()
    }

    pub fn note_text(raw: Option<String>) -> SyntaxResult<Option<SyntaxText>> {
        raw.map(|value| bounded_text("note", value, NOTE_TEXT_UTF8_BYTES, true))
            .transpose()
    }

    pub fn label_text(raw: Option<String>) -> SyntaxResult<Option<SyntaxText>> {
        raw.map(|value| bounded_text("label", value, RUN_LABEL_UTF8_BYTES, true))
            .transpose()
    }

    pub fn evidence_locator(value: String) -> SyntaxResult<SyntaxText> {
        bounded_text("ref", value, EVIDENCE_LOCATOR_UTF8_BYTES, true)
    }

    fn is_handle_syntax(value: &str) -> bool {
        if value.is_empty() || value.len() > PROVIDER_HANDLE_UTF8_BYTES {
            return false;
        }
        let chars = value.chars().collect::<Vec<_>>();
        if !is_handle_alnum(chars[0]) {
            return false;
        }
        if chars.len() == 1 {
            return true;
        }
        if !is_handle_alnum(chars[chars.len() - 1]) {
            return false;
        }
        for &character in &chars[1..chars.len() - 1] {
            if !(is_handle_alnum(character)
                || character == '.'
                || character == '_'
                || character == '-')
            {
                return false;
            }
        }
        true
    }

    fn is_handle_alnum(character: char) -> bool {
        character.is_ascii_digit() || character.is_ascii_lowercase()
    }
}

mod planned {
    use super::syntax::{
        bounded_text, evidence_locator, handle, identifier, label_text, note_text,
        optional_identifier, optional_opaque_wire, optional_page_cursor, optional_path,
        optional_positive_u64, optional_warning_cursor, page_limit, path, provider_argv,
        provider_target,
    };
    use super::*;

    #[derive(Debug, Clone, Parser)]
    #[command(
        name = CLI_NAME,
        about = CLI_ABOUT,
        disable_help_flag = true,
        disable_version_flag = true,
        subcommand_required = true,
        arg_required_else_help = true
    )]
    pub struct PlannedRoot {
        #[command(subcommand)]
        pub command: ApplicationCommand,
    }

    #[derive(Debug, Clone, Subcommand)]
    pub enum ApplicationCommand {
        Provider {
            #[command(subcommand)]
            command: ProviderCommand,
        },
        Run {
            #[command(subcommand)]
            command: RunCommand,
        },
    }

    #[derive(Debug, Clone, Subcommand)]
    pub enum ProviderCommand {
        Add(ProviderAddArgs),
        List(ProviderListArgs),
        Check(ProviderCheckArgs),
        Update(ProviderUpdateArgs),
        Rename(ProviderRenameArgs),
        Disable(ProviderDisableArgs),
        Restore(ProviderRestoreArgs),
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderAddArgs {
        #[arg(allow_hyphen_values = true)]
        pub handle: String,
        #[arg(long)]
        pub exec: String,
        #[arg(long)]
        pub working_directory: String,
        #[arg(long, allow_hyphen_values = true)]
        pub arg: Vec<String>,
        #[arg(long)]
        pub timeout: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderAddParsed {
        pub handle: SyntaxHandle,
        pub exec: SyntaxPath,
        pub working_directory: SyntaxPath,
        pub arg: SyntaxProviderArgv,
        pub timeout: Option<SyntaxPositiveU64>,
    }

    impl ProviderAddArgs {
        pub fn validate(self) -> SyntaxResult<ProviderAddParsed> {
            Ok(ProviderAddParsed {
                handle: handle(self.handle)?,
                exec: path("exec", self.exec)?,
                working_directory: path("working-directory", self.working_directory)?,
                arg: provider_argv(self.arg)?,
                timeout: optional_positive_u64("timeout", self.timeout)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderListArgs {
        #[arg(long)]
        pub enabled: bool,
        #[arg(long)]
        pub tombstoned: bool,
        #[arg(long = "active-runs-for")]
        pub active_runs_for: Option<String>,
        #[arg(long)]
        pub cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderListParsed {
        pub enabled: bool,
        pub tombstoned: bool,
        pub active_runs_for: Option<SyntaxIdentifier>,
        pub cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
    }

    impl ProviderListArgs {
        pub fn validate(self) -> SyntaxResult<ProviderListParsed> {
            Ok(ProviderListParsed {
                enabled: self.enabled,
                tombstoned: self.tombstoned,
                active_runs_for: self
                    .active_runs_for
                    .map(|value| identifier("active-runs-for", value))
                    .transpose()?,
                cursor: optional_page_cursor(self.cursor)?,
                limit: page_limit(self.limit)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderCheckArgs {
        #[arg(allow_hyphen_values = true)]
        pub target: String,
        #[arg(long = "active-runs")]
        pub active_runs: bool,
        #[arg(long)]
        pub cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderCheckParsed {
        pub target: SyntaxProviderTarget,
        pub active_runs: bool,
        pub cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
    }

    impl ProviderCheckArgs {
        pub fn validate(self) -> SyntaxResult<ProviderCheckParsed> {
            Ok(ProviderCheckParsed {
                target: provider_target(self.target)?,
                active_runs: self.active_runs,
                cursor: optional_page_cursor(self.cursor)?,
                limit: page_limit(self.limit)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderUpdateArgs {
        #[arg(allow_hyphen_values = true)]
        pub target: String,
        #[arg(long)]
        pub exec: String,
        #[arg(long, allow_hyphen_values = true)]
        pub arg: Vec<String>,
        #[arg(long)]
        pub working_directory: Option<String>,
        #[arg(long)]
        pub timeout: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderUpdateParsed {
        pub target: SyntaxProviderTarget,
        pub exec: SyntaxPath,
        pub arg: SyntaxProviderArgv,
        pub working_directory: Option<SyntaxPath>,
        pub timeout: Option<SyntaxPositiveU64>,
    }

    impl ProviderUpdateArgs {
        pub fn validate(self) -> SyntaxResult<ProviderUpdateParsed> {
            Ok(ProviderUpdateParsed {
                target: provider_target(self.target)?,
                exec: path("exec", self.exec)?,
                arg: provider_argv(self.arg)?,
                working_directory: self
                    .working_directory
                    .map(|value| optional_path("working-directory", value))
                    .transpose()?,
                timeout: optional_positive_u64("timeout", self.timeout)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderRenameArgs {
        #[arg(allow_hyphen_values = true)]
        pub target: String,
        #[arg(allow_hyphen_values = true)]
        pub new_handle: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderRenameParsed {
        pub target: SyntaxProviderTarget,
        pub new_handle: SyntaxHandle,
    }

    impl ProviderRenameArgs {
        pub fn validate(self) -> SyntaxResult<ProviderRenameParsed> {
            Ok(ProviderRenameParsed {
                target: provider_target(self.target)?,
                new_handle: handle(self.new_handle)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderDisableArgs {
        #[arg(allow_hyphen_values = true)]
        pub target: String,
        #[arg(long = "warning-cursor")]
        pub warning_cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
        #[arg(long = "allow-active-runs")]
        pub allow_active_runs: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderDisableParsed {
        pub target: SyntaxProviderTarget,
        pub warning_cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
        pub allow_active_runs: Option<SyntaxOpaqueWire>,
    }

    impl ProviderDisableArgs {
        pub fn validate(self) -> SyntaxResult<ProviderDisableParsed> {
            Ok(ProviderDisableParsed {
                target: provider_target(self.target)?,
                warning_cursor: optional_warning_cursor(self.warning_cursor)?,
                limit: page_limit(self.limit)?,
                allow_active_runs: self
                    .allow_active_runs
                    .map(|value| optional_opaque_wire("allow-active-runs", value))
                    .transpose()?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct ProviderRestoreArgs {
        pub registration_id: String,
        #[arg(long)]
        pub handle: String,
        #[arg(long)]
        pub exec: String,
        #[arg(long)]
        pub working_directory: String,
        #[arg(long, allow_hyphen_values = true)]
        pub arg: Vec<String>,
        #[arg(long)]
        pub timeout: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProviderRestoreParsed {
        pub registration_id: SyntaxIdentifier,
        pub handle: SyntaxHandle,
        pub exec: SyntaxPath,
        pub working_directory: SyntaxPath,
        pub arg: SyntaxProviderArgv,
        pub timeout: Option<SyntaxPositiveU64>,
    }

    impl ProviderRestoreArgs {
        pub fn validate(self) -> SyntaxResult<ProviderRestoreParsed> {
            Ok(ProviderRestoreParsed {
                registration_id: identifier("registration-id", self.registration_id)?,
                handle: handle(self.handle)?,
                exec: path("exec", self.exec)?,
                working_directory: path("working-directory", self.working_directory)?,
                arg: provider_argv(self.arg)?,
                timeout: optional_positive_u64("timeout", self.timeout)?,
            })
        }
    }

    #[derive(Debug, Clone, Subcommand)]
    pub enum RunCommand {
        Create(RunCreateArgs),
        List(RunListArgs),
        Show(RunShowArgs),
        Graph(RunGraphArgs),
        History(RunHistoryArgs),
        Annotate(RunAnnotateArgs),
        Label(RunLabelArgs),
        Request(RunRequestArgs),
        Guidance(RunGuidanceArgs),
        Compatibility(RunCompatibilityArgs),
        Terminate(RunTerminateArgs),
        Export(RunExportArgs),
        Evidence {
            #[command(subcommand)]
            command: RunEvidenceCommand,
        },
    }

    #[derive(Debug, Clone, Subcommand)]
    pub enum RunEvidenceCommand {
        Add(RunEvidenceAddArgs),
        List(RunEvidenceListArgs),
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunCreateArgs {
        #[arg(allow_hyphen_values = true)]
        pub target: String,
        #[arg(long)]
        pub label: Option<String>,
        #[arg(long)]
        pub inputs: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunCreateParsed {
        pub target: SyntaxProviderTarget,
        pub label: Option<SyntaxText>,
        pub inputs: Option<SyntaxPath>,
    }

    impl RunCreateArgs {
        pub fn validate(self) -> SyntaxResult<RunCreateParsed> {
            Ok(RunCreateParsed {
                target: provider_target(self.target)?,
                label: label_text(self.label)?,
                inputs: self
                    .inputs
                    .map(|value| optional_path("inputs", value))
                    .transpose()?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunListArgs {
        #[arg(long)]
        pub terminal: bool,
        #[arg(long)]
        pub all: bool,
        #[arg(long)]
        pub cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunListParsed {
        pub terminal: bool,
        pub all: bool,
        pub cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
    }

    impl RunListArgs {
        pub fn validate(self) -> SyntaxResult<RunListParsed> {
            Ok(RunListParsed {
                terminal: self.terminal,
                all: self.all,
                cursor: optional_page_cursor(self.cursor)?,
                limit: page_limit(self.limit)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    #[command(arg_required_else_help = false)]
    pub struct RunShowArgs {
        pub run_id: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunShowParsed {
        pub run_id: SyntaxIdentifier,
    }

    impl RunShowArgs {
        pub fn validate(self) -> SyntaxResult<RunShowParsed> {
            Ok(RunShowParsed {
                run_id: identifier("run-id", self.run_id)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunGraphArgs {
        pub run_id: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunGraphParsed {
        pub run_id: SyntaxIdentifier,
    }

    impl RunGraphArgs {
        pub fn validate(self) -> SyntaxResult<RunGraphParsed> {
            Ok(RunGraphParsed {
                run_id: identifier("run-id", self.run_id)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunHistoryArgs {
        pub run_id: String,
        #[arg(long)]
        pub cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunHistoryParsed {
        pub run_id: SyntaxIdentifier,
        pub cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
    }

    impl RunHistoryArgs {
        pub fn validate(self) -> SyntaxResult<RunHistoryParsed> {
            Ok(RunHistoryParsed {
                run_id: identifier("run-id", self.run_id)?,
                cursor: optional_page_cursor(self.cursor)?,
                limit: page_limit(self.limit)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunEvidenceAddArgs {
        pub run_id: String,
        #[arg(long)]
        pub kind: String,
        #[arg(long = "ref")]
        pub reference: String,
        #[arg(long)]
        pub digest: Option<String>,
        #[arg(long = "media-type")]
        pub media_type: Option<String>,
        #[arg(long)]
        pub metadata: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunEvidenceAddParsed {
        pub run_id: SyntaxIdentifier,
        pub kind: SyntaxIdentifier,
        pub reference: SyntaxText,
        pub digest: Option<SyntaxText>,
        pub media_type: Option<SyntaxText>,
        pub metadata: Option<SyntaxPath>,
    }

    impl RunEvidenceAddArgs {
        pub fn validate(self) -> SyntaxResult<RunEvidenceAddParsed> {
            Ok(RunEvidenceAddParsed {
                run_id: identifier("run-id", self.run_id)?,
                kind: optional_identifier("kind", self.kind)?,
                reference: evidence_locator(self.reference)?,
                digest: self
                    .digest
                    .map(|value| bounded_text("digest", value, IDENTIFIER_UTF8_BYTES, true))
                    .transpose()?,
                media_type: self
                    .media_type
                    .map(|value| bounded_text("media-type", value, IDENTIFIER_UTF8_BYTES, true))
                    .transpose()?,
                metadata: self
                    .metadata
                    .map(|value| optional_path("metadata", value))
                    .transpose()?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunEvidenceListArgs {
        pub run_id: String,
        #[arg(long)]
        pub cursor: Option<String>,
        #[arg(long)]
        pub limit: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunEvidenceListParsed {
        pub run_id: SyntaxIdentifier,
        pub cursor: Option<SyntaxOpaqueWire>,
        pub limit: SyntaxPageLimit,
    }

    impl RunEvidenceListArgs {
        pub fn validate(self) -> SyntaxResult<RunEvidenceListParsed> {
            Ok(RunEvidenceListParsed {
                run_id: identifier("run-id", self.run_id)?,
                cursor: optional_page_cursor(self.cursor)?,
                limit: page_limit(self.limit)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunAnnotateArgs {
        pub run_id: String,
        #[arg(long)]
        pub note: Option<String>,
        #[arg(long)]
        pub actor: Option<String>,
        #[arg(long)]
        pub corrects: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunAnnotateParsed {
        pub run_id: SyntaxIdentifier,
        pub note: Option<SyntaxText>,
        pub actor: Option<SyntaxPath>,
        pub corrects: Option<SyntaxPositiveU64>,
    }

    impl RunAnnotateArgs {
        pub fn validate(self) -> SyntaxResult<RunAnnotateParsed> {
            Ok(RunAnnotateParsed {
                run_id: identifier("run-id", self.run_id)?,
                note: note_text(self.note)?,
                actor: self
                    .actor
                    .map(|value| optional_path("actor", value))
                    .transpose()?,
                corrects: optional_positive_u64("corrects", self.corrects)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunLabelArgs {
        pub run_id: String,
        #[arg(long, conflicts_with = "clear", required_unless_present = "clear")]
        pub set: Option<String>,
        #[arg(
            long,
            action = ArgAction::SetTrue,
            conflicts_with = "set",
            required_unless_present = "set"
        )]
        pub clear: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum RunLabelMode {
        Set(SyntaxText),
        Clear,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunLabelParsed {
        pub run_id: SyntaxIdentifier,
        pub mode: RunLabelMode,
    }

    impl RunLabelArgs {
        pub fn validate(self) -> SyntaxResult<RunLabelParsed> {
            let mode = match (self.set, self.clear) {
                (Some(label), false) => RunLabelMode::Set(
                    label_text(Some(label))?.expect("label text validated as non-empty"),
                ),
                (None, true) => RunLabelMode::Clear,
                (Some(_), true) => {
                    return Err(SyntaxError::Conflict {
                        field1: "set",
                        field2: "clear",
                    });
                }
                (None, false) => return Err(SyntaxError::Required { field: "set|clear" }),
            };
            Ok(RunLabelParsed {
                run_id: identifier("run-id", self.run_id)?,
                mode,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunRequestArgs {
        pub run_id: String,
        pub event: String,
        #[arg(long = "evidence-id")]
        pub evidence_id: Vec<String>,
        #[arg(long)]
        pub evidence: Option<String>,
        #[arg(long)]
        pub note: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunRequestParsed {
        pub run_id: SyntaxIdentifier,
        pub event: SyntaxIdentifier,
        pub evidence_id: Vec<SyntaxIdentifier>,
        pub evidence: Option<SyntaxPath>,
        pub note: Option<SyntaxText>,
    }

    impl RunRequestArgs {
        pub fn validate(self) -> SyntaxResult<RunRequestParsed> {
            let evidence_id = self
                .evidence_id
                .into_iter()
                .map(|value| identifier("evidence-id", value))
                .collect::<SyntaxResult<Vec<_>>>()?;
            Ok(RunRequestParsed {
                run_id: identifier("run-id", self.run_id)?,
                event: identifier("event", self.event)?,
                evidence_id,
                evidence: self
                    .evidence
                    .map(|value| optional_path("evidence", value))
                    .transpose()?,
                note: note_text(self.note)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunGuidanceArgs {
        pub run_id: String,
        #[arg(long = "evidence-id")]
        pub evidence_id: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunGuidanceParsed {
        pub run_id: SyntaxIdentifier,
        pub evidence_id: Vec<SyntaxIdentifier>,
    }

    impl RunGuidanceArgs {
        pub fn validate(self) -> SyntaxResult<RunGuidanceParsed> {
            let evidence_id = self
                .evidence_id
                .into_iter()
                .map(|value| identifier("evidence-id", value))
                .collect::<SyntaxResult<Vec<_>>>()?;
            Ok(RunGuidanceParsed {
                run_id: identifier("run-id", self.run_id)?,
                evidence_id,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunCompatibilityArgs {
        pub run_id: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunCompatibilityParsed {
        pub run_id: SyntaxIdentifier,
    }

    impl RunCompatibilityArgs {
        pub fn validate(self) -> SyntaxResult<RunCompatibilityParsed> {
            Ok(RunCompatibilityParsed {
                run_id: identifier("run-id", self.run_id)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunTerminateArgs {
        pub run_id: String,
        #[arg(long)]
        pub note: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunTerminateParsed {
        pub run_id: SyntaxIdentifier,
        pub note: Option<SyntaxText>,
    }

    impl RunTerminateArgs {
        pub fn validate(self) -> SyntaxResult<RunTerminateParsed> {
            Ok(RunTerminateParsed {
                run_id: identifier("run-id", self.run_id)?,
                note: note_text(self.note)?,
            })
        }
    }

    #[derive(Debug, Clone, Args)]
    pub struct RunExportArgs {
        pub run_id: String,
        #[arg(long)]
        pub output: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunExportParsed {
        pub run_id: SyntaxIdentifier,
        pub output: SyntaxPath,
    }

    impl RunExportArgs {
        pub fn validate(self) -> SyntaxResult<RunExportParsed> {
            Ok(RunExportParsed {
                run_id: identifier("run-id", self.run_id)?,
                output: path("output", self.output)?,
            })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PlannedCommand {
        ProviderAdd(ProviderAddParsed),
        ProviderList(ProviderListParsed),
        ProviderCheck(ProviderCheckParsed),
        ProviderUpdate(ProviderUpdateParsed),
        ProviderRename(ProviderRenameParsed),
        ProviderDisable(ProviderDisableParsed),
        ProviderRestore(ProviderRestoreParsed),
        RunCreate(RunCreateParsed),
        RunList(RunListParsed),
        RunShow(RunShowParsed),
        RunGraph(RunGraphParsed),
        RunHistory(RunHistoryParsed),
        RunEvidenceAdd(RunEvidenceAddParsed),
        RunEvidenceList(RunEvidenceListParsed),
        RunAnnotate(RunAnnotateParsed),
        RunLabel(RunLabelParsed),
        RunRequest(RunRequestParsed),
        RunGuidance(RunGuidanceParsed),
        RunCompatibility(RunCompatibilityParsed),
        RunTerminate(RunTerminateParsed),
        RunExport(RunExportParsed),
    }

    impl PlannedCommand {
        pub fn operation_id(&self) -> OperationId {
            OperationId::parse(match self {
                Self::ProviderAdd(_) => "provider.add",
                Self::ProviderList(_) => "provider.list",
                Self::ProviderCheck(_) => "provider.check",
                Self::ProviderUpdate(_) => "provider.update",
                Self::ProviderRename(_) => "provider.rename",
                Self::ProviderDisable(_) => "provider.disable",
                Self::ProviderRestore(_) => "provider.restore",
                Self::RunCreate(_) => "run.create",
                Self::RunList(_) => "run.list",
                Self::RunShow(_) => "run.show",
                Self::RunGraph(_) => "run.graph",
                Self::RunHistory(_) => "run.history",
                Self::RunEvidenceAdd(_) => "run.evidence.add",
                Self::RunEvidenceList(_) => "run.evidence.list",
                Self::RunAnnotate(_) => "run.annotate",
                Self::RunLabel(_) => "run.label",
                Self::RunRequest(_) => "run.request",
                Self::RunGuidance(_) => "run.guidance",
                Self::RunCompatibility(_) => "run.compatibility",
                Self::RunTerminate(_) => "run.terminate",
                Self::RunExport(_) => "run.export",
            })
            .expect("planned command maps to frozen catalog ID")
        }
    }

    impl ApplicationCommand {
        pub fn validate(self) -> SyntaxResult<PlannedCommand> {
            match self {
                Self::Provider { command } => match command {
                    ProviderCommand::Add(args) => Ok(PlannedCommand::ProviderAdd(args.validate()?)),
                    ProviderCommand::List(args) => {
                        Ok(PlannedCommand::ProviderList(args.validate()?))
                    }
                    ProviderCommand::Check(args) => {
                        Ok(PlannedCommand::ProviderCheck(args.validate()?))
                    }
                    ProviderCommand::Update(args) => {
                        Ok(PlannedCommand::ProviderUpdate(args.validate()?))
                    }
                    ProviderCommand::Rename(args) => {
                        Ok(PlannedCommand::ProviderRename(args.validate()?))
                    }
                    ProviderCommand::Disable(args) => {
                        Ok(PlannedCommand::ProviderDisable(args.validate()?))
                    }
                    ProviderCommand::Restore(args) => {
                        Ok(PlannedCommand::ProviderRestore(args.validate()?))
                    }
                },
                Self::Run { command } => match command {
                    RunCommand::Create(args) => Ok(PlannedCommand::RunCreate(args.validate()?)),
                    RunCommand::List(args) => Ok(PlannedCommand::RunList(args.validate()?)),
                    RunCommand::Show(args) => Ok(PlannedCommand::RunShow(args.validate()?)),
                    RunCommand::Graph(args) => Ok(PlannedCommand::RunGraph(args.validate()?)),
                    RunCommand::History(args) => Ok(PlannedCommand::RunHistory(args.validate()?)),
                    RunCommand::Annotate(args) => Ok(PlannedCommand::RunAnnotate(args.validate()?)),
                    RunCommand::Label(args) => Ok(PlannedCommand::RunLabel(args.validate()?)),
                    RunCommand::Request(args) => Ok(PlannedCommand::RunRequest(args.validate()?)),
                    RunCommand::Guidance(args) => Ok(PlannedCommand::RunGuidance(args.validate()?)),
                    RunCommand::Compatibility(args) => {
                        Ok(PlannedCommand::RunCompatibility(args.validate()?))
                    }
                    RunCommand::Terminate(args) => {
                        Ok(PlannedCommand::RunTerminate(args.validate()?))
                    }
                    RunCommand::Export(args) => Ok(PlannedCommand::RunExport(args.validate()?)),
                    RunCommand::Evidence { command } => match command {
                        RunEvidenceCommand::Add(args) => {
                            Ok(PlannedCommand::RunEvidenceAdd(args.validate()?))
                        }
                        RunEvidenceCommand::List(args) => {
                            Ok(PlannedCommand::RunEvidenceList(args.validate()?))
                        }
                    },
                },
            }
        }
    }
}

pub use planned::{
    ApplicationCommand, PlannedCommand, RunAnnotateParsed, RunCompatibilityParsed, RunCreateParsed,
    RunEvidenceAddParsed, RunEvidenceListParsed, RunExportParsed, RunGraphParsed,
    RunGuidanceParsed, RunHistoryParsed, RunLabelMode, RunLabelParsed, RunListParsed,
    RunRequestParsed, RunShowParsed, RunTerminateParsed,
};

/// Full planned application command tree. Not attached to [`GlobalCli`].
pub fn planned_application_command() -> Command {
    planned::PlannedRoot::command()
}

/// Clap grammar parse for trailing application argv (no syntax-bound validation).
pub fn parse_planned_grammar(rest: &[impl AsRef<str>]) -> Result<ApplicationCommand, clap::Error> {
    let argv = std::iter::once(OsString::from(CLI_NAME))
        .chain(rest.iter().map(|value| OsString::from(value.as_ref())))
        .collect::<Vec<_>>();
    let matches = planned_application_command().try_get_matches_from(&argv)?;
    Ok(planned::PlannedRoot::from_arg_matches(&matches)?.command)
}

/// Apply syntax-bound validation to a grammar-parsed application command.
pub fn validate_planned(command: ApplicationCommand) -> Result<PlannedCommand, SyntaxError> {
    command.validate()
}

/// Parse trailing application argv against the closed planned grammar, then apply syntax bounds.
pub fn parse_planned_application(rest: &[impl AsRef<str>]) -> Result<PlannedCommand, ParseError> {
    validate_planned(parse_planned_grammar(rest)?).map_err(ParseError::from)
}

/// Attach one reviewed operation route to a production root shell without redefining grammar.
pub fn register_exposed_route(root: Command, operation: OperationId) -> Command {
    root.subcommand(route_command(operation))
}

fn route_command(operation: OperationId) -> Command {
    match operation.as_str() {
        "provider.add" => Command::new("provider").subcommand(provider_leaf_command("add")),
        "provider.list" => Command::new("provider").subcommand(provider_leaf_command("list")),
        "provider.check" => Command::new("provider").subcommand(provider_leaf_command("check")),
        "provider.update" => Command::new("provider").subcommand(provider_leaf_command("update")),
        "provider.rename" => Command::new("provider").subcommand(provider_leaf_command("rename")),
        "provider.disable" => Command::new("provider").subcommand(provider_leaf_command("disable")),
        "provider.restore" => Command::new("provider").subcommand(provider_leaf_command("restore")),
        "run.create" => Command::new("run").subcommand(run_leaf_command("create")),
        "run.list" => Command::new("run").subcommand(run_leaf_command("list")),
        "run.show" => Command::new("run").subcommand(run_leaf_command("show")),
        "run.graph" => Command::new("run").subcommand(run_leaf_command("graph")),
        "run.history" => Command::new("run").subcommand(run_leaf_command("history")),
        "run.evidence.add" => Command::new("run")
            .subcommand(Command::new("evidence").subcommand(run_evidence_leaf_command("add"))),
        "run.evidence.list" => Command::new("run")
            .subcommand(Command::new("evidence").subcommand(run_evidence_leaf_command("list"))),
        "run.annotate" => Command::new("run").subcommand(run_leaf_command("annotate")),
        "run.label" => Command::new("run").subcommand(run_leaf_command("label")),
        "run.request" => Command::new("run").subcommand(run_leaf_command("request")),
        "run.guidance" => Command::new("run").subcommand(run_leaf_command("guidance")),
        "run.compatibility" => Command::new("run").subcommand(run_leaf_command("compatibility")),
        "run.terminate" => Command::new("run").subcommand(run_leaf_command("terminate")),
        "run.export" => Command::new("run").subcommand(run_leaf_command("export")),
        other => panic!("unsupported operation route registration: {other}"),
    }
}

fn provider_leaf_command(name: &str) -> Command {
    planned_application_command()
        .find_subcommand("provider")
        .and_then(|provider| provider.find_subcommand(name))
        .cloned()
        .unwrap_or_else(|| panic!("missing provider subcommand: {name}"))
}

fn run_leaf_command(name: &str) -> Command {
    planned_application_command()
        .find_subcommand("run")
        .and_then(|run| run.find_subcommand(name))
        .cloned()
        .unwrap_or_else(|| panic!("missing run subcommand: {name}"))
}

fn run_evidence_leaf_command(name: &str) -> Command {
    planned_application_command()
        .find_subcommand("run")
        .and_then(|run| run.find_subcommand("evidence"))
        .and_then(|evidence| evidence.find_subcommand(name))
        .cloned()
        .unwrap_or_else(|| panic!("missing run evidence subcommand: {name}"))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn production_root_has_no_application_subcommands() {
        assert!(GlobalCli::command().get_subcommands().next().is_none());
    }
}
