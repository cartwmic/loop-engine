//! Production-path integration tests for CLI-to-core request DTO mappings (WP1 T122).
//!
//! Exercises `parse_planned_application` → syntax-valid [`PlannedCommand`] → command-module
//! `map_*` helpers. Proves invalid grammar/syntax and pre-dispatch map failures stay separate
//! from domain rejection, and that mappings pass identifiers through without transition or
//! verdict policy fields.

#[allow(dead_code, reason = "mapping target imports pre-exposure grammar only")]
#[path = "../src/args.rs"]
mod args;

#[allow(
    dead_code,
    reason = "mapping target exercises DTO maps, not operation execution"
)]
#[path = "../src/commands/mod.rs"]
mod commands;

use std::any::{Any, TypeId};

use args::{ParseError, PlannedCommand, SyntaxError, parse_planned_application};
use commands::evidence::{
    map_add_request as map_evidence_add_request, map_list_request as map_evidence_list_request,
};
use commands::export::map_request as map_export_request;
use commands::provider::{
    ProviderDisableRequest, ProviderListRequest, ProviderMapError, ProviderTargetRef,
    list_filter as provider_list_filter, map_add_request, map_check_request, map_disable_request,
    map_list_request as map_provider_list_request, map_rename_request, map_restore_request,
    map_target, map_update_request,
};
use commands::run::{
    RunMapError, list_filter as run_list_filter, map_annotate_delivery, map_compatibility_run_id,
    map_create_delivery, map_graph_run_id, map_guidance_delivery, map_history_request,
    map_label_delivery, map_list_request as map_run_list_request, map_request_delivery,
    map_show_run_id, map_terminate_delivery,
};
use loop_engine_core::capabilities::provider_catalog::ProviderConfig;
use loop_engine_core::capabilities::provider_catalog::ProviderListFilter;
use loop_engine_core::capabilities::provider_invoker::DescribeRequest;
use loop_engine_core::capabilities::run_reader::RunListFilter;
use loop_engine_core::model::bounded::COLLECTION_PAGE_DEFAULT_COUNT;
use loop_engine_core::model::ids::{EventId, EvidenceId, RegistrationId, RequestId};
use loop_engine_core::operations::provider_check::ProviderCheckMode;

fn parse(rest: &[&str]) -> PlannedCommand {
    parse_planned_application(rest).expect("argv should parse")
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

fn registration_id(value: &str) -> RegistrationId {
    RegistrationId::parse(value).expect("registration id")
}

fn request_id(value: &str) -> RequestId {
    RequestId::parse(value).expect("request id")
}

fn baseline_provider_config() -> ProviderConfig {
    ProviderConfig::new("/bin/provider", vec!["--verbose".into()], "/tmp/work", 60)
        .expect("baseline provider config")
}

// --- Boundary: grammar → syntax → map → domain ---

#[test]
fn grammar_errors_stop_before_syntax_validation() {
    let error = parse_grammar(&["provider", "add"]);
    assert!(error.to_string().contains("required"));
    assert!(parse_planned_application(&["provider", "nope"]).is_err());
    let conflict = parse_grammar(&["run", "label", "run-abc", "--set", "x", "--clear"]);
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn syntax_errors_stop_before_core_mapping() {
    let error = parse_syntax(&[
        "provider",
        "add",
        "",
        "--exec",
        "/bin/provider",
        "--working-directory",
        "/tmp/work",
    ]);
    assert!(matches!(error, SyntaxError::Empty { field: "handle" }));

    let error = parse_syntax(&["run", "list", "--limit", "0"]);
    assert!(matches!(
        error,
        SyntaxError::OutOfRange { field: "limit", .. }
    ));

    let error = parse_syntax(&["run", "label", "run-abc", "--set", ""]);
    assert!(matches!(error, SyntaxError::Empty { field: "label" }));
}

#[test]
fn pre_dispatch_map_errors_are_not_domain_rejections() {
    let parsed = parse(&[
        "provider",
        "add",
        "demo.provider",
        "--exec",
        "relative/exec",
        "--working-directory",
        "/tmp/work",
    ]);
    let PlannedCommand::ProviderAdd(add) = parsed else {
        panic!("expected provider add");
    };
    let err = map_add_request(
        &add.handle,
        add.exec.as_str(),
        add.working_directory.as_str(),
        &add.arg,
        add.timeout.as_ref(),
    )
    .expect_err("relative executable path must fail at map boundary");
    assert!(
        matches!(err, ProviderMapError::Bound(_)),
        "map layer must surface bound errors, not catalog/domain outcomes: {err:?}"
    );
    assert_ne!(
        TypeId::of::<ProviderMapError>(),
        TypeId::of::<loop_engine_core::operations::CommandError>(),
        "pre-dispatch map errors must not be domain command rejections"
    );

    let parsed = parse(&[
        "run",
        "request",
        "run-abc",
        "submit",
        "--evidence-id",
        "ev-dup",
        "--evidence-id",
        "ev-dup",
    ]);
    let PlannedCommand::RunRequest(request) = parsed else {
        panic!("expected run request");
    };
    let err = map_request_delivery(&request).expect_err("duplicate evidence ids");
    assert!(matches!(err, RunMapError::DuplicateEvidenceIds));
}

#[test]
fn valid_mapping_succeeds_before_any_catalog_or_reader_domain_work() {
    let parsed = parse(&["run", "show", "run-0123456789abcdef"]);
    let PlannedCommand::RunShow(show) = parsed else {
        panic!("expected run show");
    };
    let mapped = map_show_run_id(&show).expect("show maps without domain I/O");
    assert_eq!(mapped.as_str(), "run-0123456789abcdef");
}

// --- Representative mapping for every operation family ---

#[test]
fn provider_add_maps_to_plain_core_request() {
    let parsed = parse(&[
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
    let PlannedCommand::ProviderAdd(add) = parsed else {
        panic!("expected provider add");
    };
    let request = map_add_request(
        &add.handle,
        add.exec.as_str(),
        add.working_directory.as_str(),
        &add.arg,
        add.timeout.as_ref(),
    )
    .expect("provider add maps");
    assert_eq!(request.handle.as_str(), "my.provider");
    assert_eq!(request.config.executable(), "/bin/provider");
    assert_eq!(request.config.working_directory(), "/tmp/work");
    assert_eq!(request.config.argv().len(), 1);
    assert_eq!(request.config.timeout_seconds(), 120);
}

#[test]
fn provider_list_maps_registration_and_active_run_impact_branches() {
    let parsed = parse(&[
        "provider",
        "list",
        "--enabled",
        "--cursor",
        "c1",
        "--limit",
        "50",
    ]);
    let PlannedCommand::ProviderList(list) = parsed else {
        panic!("expected provider list");
    };
    let request = map_provider_list_request(
        list.enabled,
        list.tombstoned,
        list.active_runs_for.as_ref(),
        list.cursor.as_ref(),
        list.limit,
    )
    .expect("provider list maps");
    match request {
        ProviderListRequest::Registrations {
            filter,
            limit,
            cursor,
        } => {
            assert_eq!(filter, provider_list_filter(true, false));
            assert_eq!(limit, 50);
            assert_eq!(cursor.as_deref(), Some("c1"));
        }
        other => panic!("expected registrations branch, got {other:?}"),
    }

    let parsed = parse(&[
        "provider",
        "list",
        "--active-runs-for",
        "reg-0123456789abcdef",
    ]);
    let PlannedCommand::ProviderList(list) = parsed else {
        panic!("expected provider list");
    };
    let request = map_provider_list_request(
        list.enabled,
        list.tombstoned,
        list.active_runs_for.as_ref(),
        list.cursor.as_ref(),
        list.limit,
    )
    .expect("active-run impact list maps");
    match request {
        ProviderListRequest::ActiveRunImpact {
            registration_id,
            limit,
            ..
        } => {
            assert_eq!(registration_id.as_str(), "reg-0123456789abcdef");
            assert_eq!(limit, COLLECTION_PAGE_DEFAULT_COUNT);
        }
        other => panic!("expected active-run impact branch, got {other:?}"),
    }
}

#[test]
fn provider_check_maps_conformance_and_active_runs_modes() {
    let parsed = parse(&["provider", "check", "demo.provider"]);
    let PlannedCommand::ProviderCheck(check) = parsed else {
        panic!("expected provider check");
    };
    let registration_id = registration_id("reg-0123456789abcdef");
    let describe_request = DescribeRequest {
        request_id: request_id("req-0123456789abcdef"),
    };
    let request = map_check_request(
        registration_id.clone(),
        describe_request.request_id.clone(),
        check.active_runs,
        check.cursor.as_ref(),
        check.limit,
    )
    .expect("conformance check maps");
    assert_eq!(request.registration_id, registration_id);
    assert_eq!(
        request.describe_request.request_id.as_str(),
        "req-0123456789abcdef"
    );
    assert_eq!(request.mode, ProviderCheckMode::ConformanceOnly);

    let parsed = parse(&[
        "provider",
        "check",
        "demo.provider",
        "--active-runs",
        "--cursor",
        "c2",
        "--limit",
        "25",
    ]);
    let PlannedCommand::ProviderCheck(check) = parsed else {
        panic!("expected provider check with active runs");
    };
    let request = map_check_request(
        registration_id,
        request_id("req-0123456789abcdef"),
        check.active_runs,
        check.cursor.as_ref(),
        check.limit,
    )
    .expect("active-runs check maps");
    match request.mode {
        ProviderCheckMode::ActiveRuns(page) => {
            assert_eq!(page.limit(), 25);
            assert_eq!(page.cursor().map(|cursor| cursor.as_str()), Some("c2"));
        }
        other => panic!("expected active-runs mode, got {other:?}"),
    }
}

#[test]
fn provider_update_rename_restore_map_catalog_mutation_requests() {
    let current = baseline_provider_config();
    let parsed = parse(&[
        "provider",
        "update",
        "demo.provider",
        "--exec",
        "/bin/provider-v2",
        "--arg",
        "--quiet",
        "--working-directory",
        "/tmp/other",
        "--timeout",
        "90",
    ]);
    let PlannedCommand::ProviderUpdate(update) = parsed else {
        panic!("expected provider update");
    };
    let request = map_update_request(
        registration_id("reg-0123456789abcdef"),
        7,
        &update.exec,
        &update.arg,
        update.working_directory.as_ref(),
        update.timeout.as_ref(),
        &current,
    )
    .expect("provider update maps");
    assert_eq!(request.expected_config_revision, 7);
    assert_eq!(request.config.executable(), "/bin/provider-v2");
    assert_eq!(request.config.working_directory(), "/tmp/other");
    assert_eq!(request.config.timeout_seconds(), 90);

    let parsed = parse(&["provider", "rename", "demo.provider", "renamed.provider"]);
    let PlannedCommand::ProviderRename(rename) = parsed else {
        panic!("expected provider rename");
    };
    let request = map_rename_request(
        registration_id("reg-0123456789abcdef"),
        3,
        &rename.new_handle,
    )
    .expect("provider rename maps");
    assert_eq!(request.expected_config_revision, 3);
    assert_eq!(request.handle.as_str(), "renamed.provider");

    let parsed = parse(&[
        "provider",
        "restore",
        "reg-0123456789abcdef",
        "--handle",
        "restored.provider",
        "--exec",
        "/bin/provider",
        "--working-directory",
        "/tmp/work",
        "--arg",
        "--restore",
    ]);
    let PlannedCommand::ProviderRestore(restore) = parsed else {
        panic!("expected provider restore");
    };
    let request = map_restore_request(
        &restore.registration_id,
        &restore.handle,
        &restore.exec,
        &restore.working_directory,
        &restore.arg,
        restore.timeout.as_ref(),
        11,
    )
    .expect("provider restore maps");
    assert_eq!(request.expected_config_revision, 11);
    assert_eq!(request.handle.as_str(), "restored.provider");
    assert_eq!(request.config.executable(), "/bin/provider");
}

#[test]
fn provider_disable_maps_warnings_and_authorize_branches() {
    let parsed = parse(&[
        "provider",
        "disable",
        "demo.provider",
        "--warning-cursor",
        "warn-cursor",
        "--limit",
        "10",
    ]);
    let PlannedCommand::ProviderDisable(disable) = parsed else {
        panic!("expected provider disable warnings");
    };
    let request = map_disable_request(
        registration_id("reg-0123456789abcdef"),
        disable.warning_cursor.as_ref(),
        disable.limit,
        disable.allow_active_runs.as_ref(),
    )
    .expect("disable warnings maps");
    match request {
        ProviderDisableRequest::Warnings {
            limit,
            warning_cursor,
            ..
        } => {
            assert_eq!(limit, 10);
            assert_eq!(warning_cursor.as_deref(), Some("warn-cursor"));
        }
        other => panic!("expected warnings branch, got {other:?}"),
    }

    let parsed = parse(&[
        "provider",
        "disable",
        "demo.provider",
        "--allow-active-runs",
        "ack-token-wire",
    ]);
    let PlannedCommand::ProviderDisable(disable) = parsed else {
        panic!("expected provider disable authorize");
    };
    let request = map_disable_request(
        registration_id("reg-0123456789abcdef"),
        disable.warning_cursor.as_ref(),
        disable.limit,
        disable.allow_active_runs.as_ref(),
    )
    .expect("disable authorize maps");
    match request {
        ProviderDisableRequest::Authorize { ack_token, .. } => {
            assert_eq!(ack_token, "ack-token-wire");
        }
        other => panic!("expected authorize branch, got {other:?}"),
    }
}

#[test]
fn run_create_and_list_map_delivery_and_filter_flags() {
    let parsed = parse(&[
        "run",
        "create",
        "demo.provider",
        "--label",
        "release",
        "--inputs",
        "/tmp/inputs.json",
    ]);
    let PlannedCommand::RunCreate(create) = parsed else {
        panic!("expected run create");
    };
    let delivery = map_create_delivery(&create).expect("run create maps");
    assert!(matches!(delivery.target, ProviderTargetRef::Handle(_)));
    assert_eq!(delivery.label.as_deref(), Some("release"));
    assert_eq!(delivery.inputs_path.as_deref(), Some("/tmp/inputs.json"));

    let parsed = parse(&[
        "run",
        "list",
        "--terminal",
        "--cursor",
        "lc",
        "--limit",
        "40",
    ]);
    let PlannedCommand::RunList(list) = parsed else {
        panic!("expected run list");
    };
    let request = map_run_list_request(&list).expect("run list maps");
    assert_eq!(request.filter, RunListFilter::Terminal);
    assert_eq!(request.limit, 40);
    assert_eq!(request.cursor.as_deref(), Some("lc"));
    assert_eq!(run_list_filter(false, true), RunListFilter::All);
    assert_eq!(provider_list_filter(true, true), ProviderListFilter::All);
}

#[test]
fn run_read_operations_map_identifiers_and_paging() {
    let parsed = parse(&["run", "show", "run-0123456789abcdef"]);
    let PlannedCommand::RunShow(show) = parsed else {
        panic!("expected run show");
    };
    assert_eq!(
        map_show_run_id(&show).expect("show maps").as_str(),
        "run-0123456789abcdef"
    );

    let parsed = parse(&["run", "graph", "run-graph-target"]);
    let PlannedCommand::RunGraph(graph) = parsed else {
        panic!("expected run graph");
    };
    assert_eq!(
        map_graph_run_id(&graph).expect("graph maps").as_str(),
        "run-graph-target"
    );

    let parsed = parse(&[
        "run",
        "history",
        "run-history-target",
        "--cursor",
        "hc",
        "--limit",
        "15",
    ]);
    let PlannedCommand::RunHistory(history) = parsed else {
        panic!("expected run history");
    };
    let request = map_history_request(&history).expect("history maps");
    assert_eq!(request.run_id.as_str(), "run-history-target");
    assert_eq!(request.limit, 15);
    assert_eq!(request.cursor.as_deref(), Some("hc"));
}

#[test]
fn run_annotate_label_terminate_map_metadata_only() {
    let parsed = parse(&[
        "run",
        "annotate",
        "run-abc",
        "--note",
        "audit note",
        "--actor",
        "/tmp/actor.json",
        "--corrects",
        "4",
    ]);
    let PlannedCommand::RunAnnotate(annotate) = parsed else {
        panic!("expected run annotate");
    };
    let delivery = map_annotate_delivery(&annotate).expect("annotate maps");
    assert_eq!(delivery.run_id.as_str(), "run-abc");
    assert_eq!(
        delivery.note.as_ref().map(|note| note.as_str()),
        Some("audit note")
    );
    assert_eq!(delivery.actor_path.as_deref(), Some("/tmp/actor.json"));
    assert_eq!(delivery.corrects_sequence.map(|seq| seq.value()), Some(4));

    let parsed = parse(&["run", "label", "run-abc", "--set", "prod"]);
    let PlannedCommand::RunLabel(label) = parsed else {
        panic!("expected run label set");
    };
    let delivery = map_label_delivery(&label).expect("label set maps");
    assert_eq!(delivery.label.as_deref(), Some("prod"));

    let parsed = parse(&["run", "label", "run-abc", "--clear"]);
    let PlannedCommand::RunLabel(label) = parsed else {
        panic!("expected run label clear");
    };
    let delivery = map_label_delivery(&label).expect("label clear maps");
    assert!(delivery.label.is_none());

    let parsed = parse(&["run", "terminate", "run-abc", "--note", "done"]);
    let PlannedCommand::RunTerminate(terminate) = parsed else {
        panic!("expected run terminate");
    };
    let delivery = map_terminate_delivery(&terminate).expect("terminate maps");
    assert_eq!(delivery.run_id.as_str(), "run-abc");
    assert_eq!(
        delivery.note.as_ref().map(|note| note.as_str()),
        Some("done")
    );
}

#[test]
fn run_request_guidance_compatibility_map_without_transition_policy() {
    let parsed = parse(&[
        "run",
        "request",
        "run-abc",
        "submit-event",
        "--evidence-id",
        "ev-1",
        "--evidence-id",
        "ev-2",
        "--evidence",
        "/tmp/evidence.json",
        "--note",
        "attempt",
    ]);
    let PlannedCommand::RunRequest(request) = parsed else {
        panic!("expected run request");
    };
    let delivery = map_request_delivery(&request).expect("run request maps");
    assert_eq!(delivery.run_id.as_str(), "run-abc");
    assert_eq!(delivery.event.as_str(), "submit-event");
    assert_eq!(
        delivery
            .selected_evidence_ids
            .iter()
            .map(EvidenceId::as_str)
            .collect::<Vec<_>>(),
        vec!["ev-1", "ev-2"]
    );
    assert_eq!(
        delivery.inline_evidence_path.as_deref(),
        Some("/tmp/evidence.json")
    );
    assert!(delivery.note.is_some());
    assert_no_policy_fields(&delivery);

    let parsed = parse(&["run", "guidance", "run-abc", "--evidence-id", "ev-guidance"]);
    let PlannedCommand::RunGuidance(guidance) = parsed else {
        panic!("expected run guidance");
    };
    let delivery = map_guidance_delivery(&guidance).expect("guidance maps");
    assert_eq!(delivery.run_id.as_str(), "run-abc");
    assert_eq!(delivery.selected_evidence_ids.len(), 1);
    assert_no_policy_fields(&delivery);

    let parsed = parse(&["run", "compatibility", "run-compat"]);
    let PlannedCommand::RunCompatibility(compatibility) = parsed else {
        panic!("expected run compatibility");
    };
    assert_eq!(
        map_compatibility_run_id(&compatibility)
            .expect("compatibility maps")
            .as_str(),
        "run-compat"
    );
}

#[test]
fn evidence_operations_map_inventory_requests() {
    let parsed = parse(&[
        "run",
        "evidence",
        "add",
        "run-abc",
        "--kind",
        "artifact",
        "--ref",
        "s3://bucket/object",
        "--digest",
        "sha256:abc",
        "--media-type",
        "application/json",
        "--metadata",
        "/tmp/meta.json",
    ]);
    let PlannedCommand::RunEvidenceAdd(add) = parsed else {
        panic!("expected evidence add");
    };
    let request = map_evidence_add_request(&add).expect("evidence add maps");
    assert_eq!(request.run_id.as_str(), "run-abc");
    assert_eq!(request.kind.as_str(), "artifact");
    assert_eq!(request.locator, "s3://bucket/object");
    assert_eq!(request.digest.as_deref(), Some("sha256:abc"));
    assert_eq!(request.media_type.as_deref(), Some("application/json"));
    assert_eq!(request.metadata_file.as_deref(), Some("/tmp/meta.json"));

    let parsed = parse(&[
        "run", "evidence", "list", "run-abc", "--cursor", "ec", "--limit", "30",
    ]);
    let PlannedCommand::RunEvidenceList(list) = parsed else {
        panic!("expected evidence list");
    };
    let request = map_evidence_list_request(&list).expect("evidence list maps");
    assert_eq!(request.run_id.as_str(), "run-abc");
    assert_eq!(request.page_request.limit(), 30);
}

#[test]
fn run_export_maps_to_core_export_request() {
    let parsed = parse(&[
        "run",
        "export",
        "run-export-target",
        "--output",
        "/tmp/export/out.jsonl",
    ]);
    let PlannedCommand::RunExport(export) = parsed else {
        panic!("expected run export");
    };
    let request = map_export_request(&export).expect("export maps");
    assert_eq!(request.run_id.as_str(), "run-export-target");
    assert_eq!(request.target.as_str(), "/tmp/export/out.jsonl");
}

#[test]
fn planned_command_mapping_samples_exactly_cover_operation_catalog() {
    let samples: &[&[&str]] = &[
        &[
            "provider",
            "add",
            "h",
            "--exec",
            "/bin/p",
            "--working-directory",
            "/tmp",
        ],
        &["provider", "list"],
        &["provider", "check", "h"],
        &["provider", "update", "h", "--exec", "/bin/p"],
        &["provider", "rename", "h", "h2"],
        &["provider", "disable", "h"],
        &[
            "provider",
            "restore",
            "reg-id",
            "--handle",
            "h",
            "--exec",
            "/bin/p",
            "--working-directory",
            "/tmp",
        ],
        &["run", "create", "h"],
        &["run", "list"],
        &["run", "show", "run-id"],
        &["run", "graph", "run-id"],
        &["run", "history", "run-id"],
        &[
            "run", "evidence", "add", "run-id", "--kind", "artifact", "--ref", "loc",
        ],
        &["run", "evidence", "list", "run-id"],
        &["run", "annotate", "run-id"],
        &["run", "label", "run-id", "--set", "lbl"],
        &["run", "request", "run-id", "evt"],
        &["run", "guidance", "run-id"],
        &["run", "compatibility", "run-id"],
        &["run", "terminate", "run-id"],
        &["run", "export", "run-id", "--output", "/tmp/out"],
    ];
    let observed_routes = samples
        .iter()
        .map(|argv| parse(argv).operation_id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        observed_routes,
        loop_engine_core::operations::catalog::PLANNED_OPERATION_IDS,
        "actual parsed PlannedCommand mappings must exactly cover the core catalog"
    );
}

#[test]
fn provider_target_resolution_classifies_handle_vs_registration_id() {
    let parsed = parse(&["run", "create", "demo.provider"]);
    let PlannedCommand::RunCreate(create) = parsed else {
        panic!("expected run create");
    };
    let target = map_target(&create.target).expect("handle target maps");
    assert!(matches!(target, ProviderTargetRef::Handle(_)));
    if let ProviderTargetRef::Handle(handle) = target {
        assert_eq!(handle.as_str(), "demo.provider");
    }

    let parsed = parse(&["run", "create", "REG-0123456789ABCDEF"]);
    let PlannedCommand::RunCreate(create) = parsed else {
        panic!("expected run create by registration id");
    };
    let target = map_target(&create.target).expect("registration target maps");
    assert!(matches!(target, ProviderTargetRef::RegistrationId(_)));
}

#[test]
fn mappings_do_not_select_transitions_or_reinterpret_verdicts() {
    let parsed = parse(&[
        "run",
        "request",
        "run-abc",
        "gate-event",
        "--evidence-id",
        "ev-only",
    ]);
    let PlannedCommand::RunRequest(parsed) = parsed else {
        panic!("expected run request");
    };
    let request_delivery = map_request_delivery(&parsed).expect("request delivery");
    assert_eq!(
        EventId::parse("gate-event").unwrap().as_str(),
        request_delivery.event.as_str()
    );
    assert_eq!(request_delivery.selected_evidence_ids.len(), 1);
    assert_no_policy_fields(&request_delivery);

    let parsed_check = parse(&["provider", "check", "demo.provider"]);
    let PlannedCommand::ProviderCheck(check_parsed) = parsed_check else {
        panic!("expected provider check");
    };
    let check = map_check_request(
        registration_id("reg-abc"),
        request_id("req-abc"),
        check_parsed.active_runs,
        check_parsed.cursor.as_ref(),
        check_parsed.limit,
    )
    .expect("check request");
    assert_eq!(check.mode, ProviderCheckMode::ConformanceOnly);
    assert_no_policy_fields(&check);

    let parsed_disable = parse(&["provider", "disable", "demo.provider"]);
    let PlannedCommand::ProviderDisable(disable_parsed) = parsed_disable else {
        panic!("expected provider disable");
    };
    let disable = map_disable_request(
        registration_id("reg-abc"),
        disable_parsed.warning_cursor.as_ref(),
        disable_parsed.limit,
        disable_parsed.allow_active_runs.as_ref(),
    )
    .expect("disable warnings");
    assert!(matches!(disable, ProviderDisableRequest::Warnings { .. }));
    assert_no_policy_fields(&disable);

    assert_eq!(run_list_filter(false, false), RunListFilter::Active);
    assert_eq!(run_list_filter(true, false), RunListFilter::Terminal);
    assert_eq!(
        provider_list_filter(false, false),
        ProviderListFilter::Enabled
    );
}

/// Delivery DTOs must not carry transition targets, verdict overrides, or retry tokens.
fn assert_no_policy_fields(value: &dyn Any) {
    let debug = format!("{value:?}").to_ascii_lowercase();
    for forbidden in [
        "transition",
        "verdict",
        "outcome_class",
        "next_state",
        "gate_override",
        "retry_token",
        "revision_token",
    ] {
        assert!(
            !debug.contains(forbidden),
            "mapping DTO debug representation must not expose policy field {forbidden}: {debug}"
        );
    }
}
