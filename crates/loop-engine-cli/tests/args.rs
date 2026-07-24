//! Integration tests for shared CLI argument primitives (T121).

#[allow(
    dead_code,
    unused_imports,
    reason = "focused target exercises shared grammar primitives"
)]
#[path = "../src/args.rs"]
mod args;

use args::{
    GlobalCli, ParseError, PlannedCommand, RunLabelMode, SyntaxError, parse_planned_application,
    planned_application_command, register_exposed_route,
};
use clap::Parser;
use loop_engine_core::model::bounded::{
    COLLECTION_PAGE_DEFAULT_COUNT, COLLECTION_PAGE_MAX_COUNT, IDENTIFIER_UTF8_BYTES,
    OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
};
use loop_engine_core::operations::catalog::OperationId;

fn parse(rest: &[&str]) -> PlannedCommand {
    parse_planned_application(rest).expect("parse should succeed")
}

fn parse_syntax(rest: &[&str]) -> SyntaxError {
    match parse_planned_application(rest).expect_err("expected syntax failure") {
        ParseError::Syntax(error) => error,
        other => panic!("expected syntax error, got {other:?}"),
    }
}

fn parse_grammar(rest: &[&str]) -> clap::Error {
    match parse_planned_application(rest).expect_err("expected grammar failure") {
        ParseError::Grammar(error) => error,
        other => panic!("expected grammar error, got {other:?}"),
    }
}

#[test]
fn root_parser_registers_zero_application_operations_before_startup() {
    let help = GlobalCli::usage_help();
    assert!(help.contains("All 21 application operations are available"));
    assert!(GlobalCli::command().get_subcommands().next().is_none());
    assert!(!help.contains("  provider "));
    assert!(!help.contains("  run "));
}

#[test]
fn planned_tree_covers_all_catalog_operations() {
    let planned = planned_application_command();
    assert!(planned.find_subcommand("provider").is_some());
    assert!(planned.find_subcommand("run").is_some());
    assert_eq!(OperationId::planned().count(), 21);
}

#[test]
fn register_exposed_route_attaches_one_operation_without_redefining_grammar() {
    let root = register_exposed_route(
        GlobalCli::command(),
        OperationId::parse("run.show").unwrap(),
    );
    let provider = root.find_subcommand("run").expect("run namespace");
    assert!(provider.find_subcommand("show").is_some());
    assert!(provider.find_subcommand("list").is_none());
}

#[test]
fn stable_ids_and_provider_config_flags_parse_with_syntax_bounds() {
    let command = parse(&[
        "provider",
        "add",
        "my.provider",
        "--exec",
        "/bin/provider",
        "--working-directory",
        "/tmp/work",
        "--arg",
        "--verbose",
        "--timeout",
        "120",
    ]);
    match command {
        PlannedCommand::ProviderAdd(parsed) => {
            assert_eq!(parsed.handle.as_str(), "my.provider");
            assert_eq!(parsed.exec.as_str(), "/bin/provider");
            assert_eq!(parsed.working_directory.as_str(), "/tmp/work");
            assert_eq!(parsed.arg.elements.len(), 1);
            assert_eq!(parsed.timeout.unwrap().get(), 120);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let command = parse(&["run", "show", "run-0123456789abcdef"]);
    match command {
        PlannedCommand::RunShow(parsed) => {
            assert_eq!(parsed.run_id.as_str(), "run-0123456789abcdef");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn inputs_and_evidence_flags_parse_with_syntax_bounds() {
    let command = parse(&[
        "run",
        "create",
        "demo",
        "--label",
        "release-candidate",
        "--inputs",
        "/tmp/inputs.json",
    ]);
    match command {
        PlannedCommand::RunCreate(parsed) => {
            assert_eq!(parsed.target.as_str(), "demo");
            assert_eq!(parsed.label.as_ref().unwrap().as_str(), "release-candidate");
            assert_eq!(parsed.inputs.as_ref().unwrap().as_str(), "/tmp/inputs.json");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let command = parse(&[
        "run",
        "evidence",
        "add",
        "run-abc",
        "--kind",
        "artifact",
        "--ref",
        "s3://bucket/object",
        "--metadata",
        "/tmp/meta.json",
    ]);
    match command {
        PlannedCommand::RunEvidenceAdd(parsed) => {
            assert_eq!(parsed.run_id.as_str(), "run-abc");
            assert_eq!(parsed.kind.as_str(), "artifact");
            assert_eq!(parsed.reference.as_str(), "s3://bucket/object");
            assert_eq!(parsed.metadata.as_ref().unwrap().as_str(), "/tmp/meta.json");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let command = parse(&[
        "run",
        "request",
        "run-abc",
        "submit",
        "--evidence-id",
        "ev-1",
        "--evidence-id",
        "ev-2",
        "--evidence",
        "/tmp/evidence.json",
        "--note",
        "attempt",
    ]);
    match command {
        PlannedCommand::RunRequest(parsed) => {
            assert_eq!(parsed.event.as_str(), "submit");
            assert_eq!(parsed.evidence_id.len(), 2);
            assert_eq!(
                parsed.evidence.as_ref().unwrap().as_str(),
                "/tmp/evidence.json"
            );
            assert_eq!(parsed.note.as_ref().unwrap().as_str(), "attempt");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn paged_cursor_and_limit_defaults_and_bounds_apply() {
    let command = parse(&["run", "list"]);
    match command {
        PlannedCommand::RunList(parsed) => {
            assert!(parsed.cursor.is_none());
            assert_eq!(parsed.limit.get(), COLLECTION_PAGE_DEFAULT_COUNT);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let command = parse(&[
        "run",
        "history",
        "run-abc",
        "--cursor",
        "Y3Vyc29yLXdpcmU",
        "--limit",
        "250",
    ]);
    match command {
        PlannedCommand::RunHistory(parsed) => {
            assert_eq!(parsed.cursor.as_ref().unwrap().as_str(), "Y3Vyc29yLXdpcmU");
            assert_eq!(parsed.limit.get(), 250);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let error = parse_syntax(&["run", "list", "--limit", "0"]);
    assert!(matches!(
        error,
        SyntaxError::OutOfRange { field: "limit", .. }
    ));

    let over = (COLLECTION_PAGE_MAX_COUNT + 1).to_string();
    let error = parse_syntax(&["run", "list", "--limit", over.as_str()]);
    assert!(matches!(
        error,
        SyntaxError::OutOfRange { field: "limit", .. }
    ));
}

#[test]
fn provider_list_active_runs_for_parses_registration_id() {
    let command = parse(&[
        "provider",
        "list",
        "--active-runs-for",
        "reg-0123456789abcdef",
        "--limit",
        "50",
    ]);
    match command {
        PlannedCommand::ProviderList(parsed) => {
            assert_eq!(
                parsed.active_runs_for.as_ref().unwrap().as_str(),
                "reg-0123456789abcdef"
            );
            assert_eq!(parsed.limit.get(), 50);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn provider_disable_uses_warning_cursor_not_cursor_flag() {
    let command = parse(&[
        "provider",
        "disable",
        "demo",
        "--warning-cursor",
        "d2FybmluZy1wYWdl",
        "--limit",
        "10",
    ]);
    match command {
        PlannedCommand::ProviderDisable(parsed) => {
            assert_eq!(
                parsed.warning_cursor.as_ref().unwrap().as_str(),
                "d2FybmluZy1wYWdl"
            );
            assert_eq!(parsed.limit.get(), 10);
            assert!(parsed.allow_active_runs.is_none());
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let _ = parse_grammar(&["provider", "disable", "demo", "--cursor", "not-allowed"]);
}

#[test]
fn provider_disable_allow_active_runs_accepts_token_syntax() {
    let token = "a".repeat(OPAQUE_INTEGRITY_WIRE_UTF8_BYTES);
    let command = parse(&[
        "provider",
        "disable",
        "demo",
        "--allow-active-runs",
        token.as_str(),
    ]);
    match command {
        PlannedCommand::ProviderDisable(parsed) => {
            assert_eq!(parsed.allow_active_runs.as_ref().unwrap().as_str(), token);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let too_long = "a".repeat(OPAQUE_INTEGRITY_WIRE_UTF8_BYTES + 1);
    let error = parse_syntax(&[
        "provider",
        "disable",
        "demo",
        "--allow-active-runs",
        too_long.as_str(),
    ]);
    assert!(matches!(
        error,
        SyntaxError::TooLong {
            field: "allow-active-runs",
            ..
        }
    ));
}

#[test]
fn forbidden_retry_revision_and_gate_bypass_flags_are_absent() {
    for forbidden in [
        "--retry",
        "--retry-key",
        "--revision",
        "--revision-token",
        "--gate-bypass",
        "--bypass-gate",
    ] {
        let _ = parse_grammar(&["run", "request", "run-abc", "submit", forbidden]);
    }
}

#[test]
fn grammar_and_syntax_failures_are_distinct() {
    let grammar = parse_grammar(&["run", "show"]);
    assert_eq!(
        grammar.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let long_id = "x".repeat(IDENTIFIER_UTF8_BYTES + 1);
    let syntax = parse_syntax(&["run", "show", long_id.as_str()]);
    assert!(matches!(
        syntax,
        SyntaxError::TooLong {
            field: "run-id",
            ..
        }
    ));
}

#[test]
fn handle_grammar_rejects_uppercase_and_invalid_edges() {
    let error = parse_syntax(&[
        "provider",
        "add",
        "Bad",
        "--exec",
        "/x",
        "--working-directory",
        "/y",
    ]);
    assert!(matches!(
        error,
        SyntaxError::InvalidSyntax { field: "handle" }
    ));

    let error = parse_syntax(&[
        "provider",
        "add",
        "-bad",
        "--exec",
        "/x",
        "--working-directory",
        "/y",
    ]);
    assert!(matches!(
        error,
        SyntaxError::InvalidSyntax { field: "handle" }
    ));
}

#[test]
fn provider_target_accepts_handle_or_registration_id() {
    let by_handle = parse(&["provider", "check", "demo.provider", "--active-runs"]);
    match by_handle {
        PlannedCommand::ProviderCheck(parsed) => {
            assert_eq!(parsed.target.as_str(), "demo.provider");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let by_id = parse(&["provider", "check", "0123456789abcdef0123456789abcdef"]);
    match by_id {
        PlannedCommand::ProviderCheck(parsed) => {
            assert_eq!(parsed.target.as_str(), "0123456789abcdef0123456789abcdef");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn run_label_requires_set_or_clear_at_grammar_layer() {
    let _ = parse_grammar(&["run", "label", "run-abc"]);
    let command = parse(&["run", "label", "run-abc", "--clear"]);
    match command {
        PlannedCommand::RunLabel(parsed) => {
            assert!(matches!(parsed.mode, RunLabelMode::Clear));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn global_cli_parses_driver_flags_and_captures_rest() {
    let cli = GlobalCli::try_parse_from([
        "loop-engine",
        "--format",
        "json",
        "--list-operations",
        "run",
        "list",
    ])
    .expect("global parse");
    assert_eq!(cli.format.as_deref(), Some("json"));
    assert!(cli.list_operations);
    assert_eq!(cli.rest, vec!["run".to_owned(), "list".to_owned()]);
}
