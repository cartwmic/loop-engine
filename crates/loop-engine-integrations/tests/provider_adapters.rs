use loop_engine_core::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
use loop_engine_core::capabilities::provider_invoker::{
    CompatibilityRequest, DescribeRequest, DescribedGraph, EvidenceContext, GateRequest,
    GuidanceRequest, InputValidationResult, ProviderInvoker, ProviderRunSnapshot,
    ValidateInputsRequest,
};
use loop_engine_core::model::compatibility::CompatibilityReport;
use loop_engine_core::model::gate::GateEvaluation;
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_validation::{GraphError, ValidatedGraph};
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{
    EventId, GateId, GraphRevision, ProviderHandle, RegistrationId, RequestId, RunId, StateId,
};
use loop_engine_core::model::live_guidance::LiveGuidanceResult;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::{InputDeclarations, RunInputs};
use loop_engine_core::model::transition::Transition;
use loop_engine_integrations::provider_protocol::SubprocessProviderInvoker;
use loop_engine_integrations::trace::TraceWriter;
use std::sync::{Arc, Mutex};

fn invoker(root: &std::path::Path, id: &str) -> SubprocessProviderInvoker {
    SubprocessProviderInvoker::new(Arc::new(Mutex::new(
        TraceWriter::create(&root.join("traces"), id).unwrap(),
    )))
}

fn config(command: String, cwd: &std::path::Path) -> ResolvedProviderConfig {
    ResolvedProviderConfig::new(
        RegistrationId::parse("registration").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new(
            "/bin/sh",
            vec!["-c".into(), command],
            cwd.to_str().unwrap(),
            5,
        )
        .unwrap(),
    )
    .unwrap()
}

fn run_snapshot() -> ProviderRunSnapshot {
    let draft = StateId::parse("draft").unwrap();
    let graph = WorkflowGraph::new_unvalidated(
        draft.clone(),
        vec![
            State::new(draft.clone(), false, StaticGuidance::NoneRequired, None),
            State::new(
                StateId::parse("done").unwrap(),
                true,
                StaticGuidance::NoneRequired,
                None,
            ),
        ],
        vec![
            Transition::new(
                draft,
                EventId::parse("go").unwrap(),
                StateId::parse("done").unwrap(),
                vec![GateId::parse("g1").unwrap()],
                None,
            )
            .unwrap(),
        ],
        InputDeclarations::default(),
        LiveGuidanceCapability::Supported,
        None,
    );
    let run = Run::create(
        RunId::parse("run-1").unwrap(),
        RegistrationId::parse("registration").unwrap(),
        ValidatedGraph::validate(graph).unwrap(),
        GraphRevision::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        RunInputs::default(),
        None,
    )
    .unwrap();
    ProviderRunSnapshot::new(run, 1).unwrap()
}

fn empty_evidence(field: &'static str) -> EvidenceContext {
    EvidenceContext::new(field, vec![], 0).unwrap()
}

#[test]
fn successful_provider_result_surfaces_non_authoritative_trace_finish_failure() {
    let directory = tempfile::tempdir().unwrap();
    let sidecar = directory.path().join("traces/.reserve/trace-failure.json");
    let output = r#"{"protocol_major":1,"role":"describe","invocation_id":"request","result":{"kind":"description","graph":{"initial_state":"draft","states":[{"id":"draft","final":false,"static_guidance":"Prepare."}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#;
    let provider = config(
        format!(
            "cat > /dev/null; /bin/dd if=/dev/zero of='{}' bs=320 count=1 conv=notrunc 2>/dev/null; printf '%s' '{}'",
            sidecar.display(),
            output
        ),
        directory.path(),
    );
    let result = invoker(directory.path(), "trace-failure")
        .describe(
            &provider,
            DescribeRequest {
                request_id: RequestId::parse("request").unwrap(),
            },
        )
        .unwrap();
    assert!(result.trace_failure.is_some());
    assert!(matches!(result.graph, DescribedGraph::Declared(_)));
}

#[test]
fn describe_request_is_input_free_and_maps_complete_raw_graph() {
    let directory = tempfile::tempdir().unwrap();
    let request_file = directory.path().join("request.json");
    let output = r#"{"protocol_major":1,"role":"describe","invocation_id":"request","provider_version":"1.2.3","result":{"kind":"description","graph":{"initial_state":"draft","states":[{"id":"draft","final":false,"static_guidance":"Prepare."}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#;
    let provider = config(
        format!(
            "cat > '{}'; printf '%s' '{}'",
            request_file.display(),
            output
        ),
        directory.path(),
    );
    let result = invoker(directory.path(), "trace-describe")
        .describe(
            &provider,
            DescribeRequest {
                request_id: RequestId::parse("request").unwrap(),
            },
        )
        .unwrap();
    let DescribedGraph::Declared(graph) = &result.graph else {
        panic!("valid declaration was classified invalid")
    };
    assert_eq!(graph.initial_state().as_str(), "draft");
    assert_eq!(result.protocol_major, 1);
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request_file).unwrap()).unwrap();
    assert_eq!(request["payload"], serde_json::json!({}));
    assert!(request.to_string().find("candidate_values").is_none());
    let trace =
        std::fs::read_to_string(directory.path().join("traces/trace-describe.jsonl")).unwrap();
    let events = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[1]["event"], "finish");
    assert_eq!(events[1]["request"]["role"], "describe");
    assert!(
        events
            .iter()
            .all(|event| event.get("environment").is_none())
    );
}

#[test]
fn describe_preserves_semantically_invalid_duplicates_for_conformance() {
    let directory = tempfile::tempdir().unwrap();
    for (index, (graph, expected)) in [
        (
            r#"{"initial_state":"a","states":[{"id":"a","final":false,"static_guidance":{"kind":"none"}},{"id":"b","final":false,"static_guidance":{"kind":"none"}}],"transitions":[{"source_state":"a","event":"go","target_state":"b","gate_ids":["g","g"]}],"input_declarations":[],"live_guidance_supported":false}"#,
            "gate",
        ),
        (
            r#"{"initial_state":"a","states":[{"id":"a","final":false,"static_guidance":{"kind":"none"}}],"transitions":[],"input_declarations":[{"id":"i","kind":"text","required":false},{"id":"i","kind":"text","required":false}],"live_guidance_supported":false}"#,
            "input",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let output = format!(
            r#"{{"protocol_major":1,"role":"describe","invocation_id":"request","result":{{"kind":"description","graph":{graph}}}}}"#
        );
        let provider = config(
            format!("cat >/dev/null; printf '%s' '{}'", output),
            directory.path(),
        );
        let described = invoker(directory.path(), &format!("invalid-graph-{index}"))
            .describe(
                &provider,
                DescribeRequest {
                    request_id: RequestId::parse("request").unwrap(),
                },
            )
            .unwrap();
        let DescribedGraph::Declared(graph) = described.graph else {
            panic!("representable invalid declaration was discarded")
        };
        let error = ValidatedGraph::validate(graph).unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (GraphError::DuplicateGate(_), "gate")
                    | (GraphError::DuplicateInput(_), "input")
            ),
            "{error}"
        );
    }
}

#[test]
fn structurally_unrepresentable_graphs_remain_completed_invalid_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let long_id = "x".repeat(129);
    let cases = [
        format!(
            r#"{{"initial_state":"{long_id}","states":[{{"id":"{long_id}","final":true,"static_guidance":{{"kind":"none"}}}}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}"#
        ),
        r#"{"initial_state":"draft","states":[{"id":"draft","final":true,"static_guidance":""}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}"#.to_owned(),
        r#"{"initial_state":"draft","states":[{"id":"draft","final":true,"static_guidance":{"kind":"none"},"metadata":{"key":1,"key":2}}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}"#.to_owned(),
    ];
    for (index, graph) in cases.into_iter().enumerate() {
        let output = format!(
            r#"{{"protocol_major":1,"role":"describe","invocation_id":"request","result":{{"kind":"description","graph":{graph}}}}}"#
        );
        let provider = config(
            format!("cat >/dev/null; printf '%s' '{}'", output),
            directory.path(),
        );
        let described = invoker(directory.path(), &format!("structural-graph-{index}"))
            .describe(
                &provider,
                DescribeRequest {
                    request_id: RequestId::parse("request").unwrap(),
                },
            )
            .unwrap();
        assert!(matches!(
            described.graph,
            DescribedGraph::Invalid(GraphError::InvalidDeclaration(_))
        ));
        assert_eq!(described.fact.outcome, OutcomeClass::Completed);
    }
}

#[test]
fn malformed_wrong_id_and_unsupported_major_are_transport_errors() {
    let directory = tempfile::tempdir().unwrap();
    for (index, (output, expected_code)) in [
        ("not-json", "provider.protocol.malformed"),
        (
            r#"{"protocol_major":1,"role":"describe","invocation_id":"wrong","result":{"kind":"description","graph":{"initial_state":"a","states":[],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#,
            "provider.protocol.malformed",
        ),
        (
            r#"{"protocol_major":2,"role":"describe","invocation_id":"request","result":{"kind":"future_v2"}}"#,
            "provider.protocol.unsupported_major",
        ),
        (
            r#"{"protocol_major":0,"role":"describe","invocation_id":"request","result":{"kind":"future_v0"}}"#,
            "provider.protocol.unsupported_major",
        ),
        (
            r#"{"protocol_major":1,"role":"describe","invocation_id":"request","provider_version":"1.0\tbeta","result":{"kind":"description","graph":{"initial_state":"a","states":[],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#,
            "provider.protocol.malformed",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let provider = config(
            format!("cat >/dev/null; printf '%s' '{}'", output),
            directory.path(),
        );
        let trace_id = format!("trace-error-{index}");
        assert!(
            invoker(directory.path(), &trace_id)
                .describe(
                    &provider,
                    DescribeRequest {
                        request_id: RequestId::parse("request").unwrap(),
                    }
                )
                .is_err()
        );
        let trace = std::fs::read_to_string(
            directory.path().join("traces").join(format!("{trace_id}.jsonl")),
        )
        .unwrap();
        let last: serde_json::Value =
            serde_json::from_str(trace.lines().last().unwrap()).unwrap();
        assert_eq!(last["event"], "failure");
        assert_eq!(last["failure_code"], expected_code);
    }
}

#[test]
fn input_validation_is_value_only_and_rejects_topology_output() {
    let directory = tempfile::tempdir().unwrap();
    let request_file = directory.path().join("request.json");
    let output = r#"{"protocol_major":1,"role":"validate_inputs","invocation_id":"request","result":{"kind":"accepted"}}"#;
    let provider = config(
        format!(
            "cat > '{}'; printf '%s' '{}'",
            request_file.display(),
            output
        ),
        directory.path(),
    );
    let result = invoker(directory.path(), "trace-validate")
        .validate_inputs(
            &provider,
            ValidateInputsRequest {
                request_id: RequestId::parse("request").unwrap(),
                input_declarations: InputDeclarations::default(),
                inputs: RunInputs::default(),
            },
        )
        .unwrap();
    assert!(matches!(result.result, InputValidationResult::Accepted));
    assert_eq!(result.fact.outcome, OutcomeClass::Completed);
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request_file).unwrap()).unwrap();
    assert!(request["payload"].get("graph").is_none());
    assert_eq!(
        request["payload"]["candidate_values"],
        serde_json::json!({})
    );

    for (index, (kind, expected)) in [
        ("rejected", OutcomeClass::Rejected),
        ("evaluation_error", OutcomeClass::Error),
    ]
    .into_iter()
    .enumerate()
    {
        let output = format!(
            r#"{{"protocol_major":1,"role":"validate_inputs","invocation_id":"request","result":{{"kind":"{kind}","diagnostics":[]}}}}"#
        );
        let provider = config(
            format!("cat >/dev/null; printf '%s' '{}'", output),
            directory.path(),
        );
        let result = invoker(directory.path(), &format!("input-outcome-{index}"))
            .validate_inputs(
                &provider,
                ValidateInputsRequest {
                    request_id: RequestId::parse("request").unwrap(),
                    input_declarations: InputDeclarations::default(),
                    inputs: RunInputs::default(),
                },
            )
            .unwrap();
        assert_eq!(result.fact.outcome, expected);
    }

    let invalid = r#"{"protocol_major":1,"role":"validate_inputs","invocation_id":"request","result":{"kind":"accepted","states":[]}}"#;
    let provider = config(
        format!("cat >/dev/null; printf '%s' '{}'", invalid),
        directory.path(),
    );
    assert!(
        invoker(directory.path(), "trace-invalid")
            .validate_inputs(
                &provider,
                ValidateInputsRequest {
                    request_id: RequestId::parse("request").unwrap(),
                    input_declarations: InputDeclarations::default(),
                    inputs: RunInputs::default(),
                }
            )
            .is_err()
    );
}

#[test]
fn trace_budget_failure_prevents_process_launch() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("traces");
    let writers = (0..7)
        .map(|index| TraceWriter::create(&directory, format!("reserved-{index}")))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let trace = Arc::new(Mutex::new(
        TraceWriter::create(&directory, "request").unwrap(),
    ));
    let marker = root.path().join("launched");
    let output = r#"{"protocol_major":1,"role":"describe","invocation_id":"request","result":{"kind":"description","graph":{"initial_state":"draft","states":[{"id":"draft","final":true,"static_guidance":{"kind":"none"}}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#;
    let provider = config(
        format!("touch '{}'; printf '%s' '{}'", marker.display(), output),
        root.path(),
    );
    let invoker = SubprocessProviderInvoker::new(trace);
    assert!(
        invoker
            .describe(
                &provider,
                DescribeRequest {
                    request_id: RequestId::parse("request").unwrap(),
                },
            )
            .is_err()
    );
    assert!(!marker.exists());
    for writer in writers {
        writer.close().unwrap();
    }
}

#[test]
fn strict_protocol_and_process_failures_are_traced_as_last_events() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        (
            "wrong-role",
            "printf '%s' '{\"protocol_major\":1,\"role\":\"validate_inputs\",\"invocation_id\":\"request\",\"result\":{\"kind\":\"accepted\"}}'".to_owned(),
        ),
        (
            "duplicate",
            "printf '%s' '{\"protocol_major\":1,\"protocol_major\":1,\"role\":\"describe\",\"invocation_id\":\"request\",\"result\":{}}'".to_owned(),
        ),
        (
            "trailing",
            "printf '%s' '{\"protocol_major\":1,\"role\":\"describe\",\"invocation_id\":\"request\",\"result\":{}} {}'".to_owned(),
        ),
        ("nonzero", "printf bad; exit 9".to_owned()),
        ("signal", "kill -TERM $$".to_owned()),
        (
            "oversize",
            "head -c 1100000 /dev/zero | tr '\\0' x".to_owned(),
        ),
        ("invalid-utf8", "printf '\\377'".to_owned()),
    ];
    for (id, command) in cases {
        let provider = config(command, directory.path());
        assert!(
            invoker(directory.path(), id)
                .describe(
                    &provider,
                    DescribeRequest {
                        request_id: RequestId::parse("request").unwrap(),
                    },
                )
                .is_err()
        );
        let trace =
            std::fs::read_to_string(directory.path().join("traces").join(format!("{id}.jsonl")))
                .unwrap();
        let last: serde_json::Value = serde_json::from_str(trace.lines().last().unwrap()).unwrap();
        assert_eq!(last["event"], "failure", "case {id}");
        assert!(last["failure_code"].as_str().is_some(), "case {id}");
    }
}

#[test]
fn stderr_is_fully_drained_but_trace_retains_bounded_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let output = r#"{"protocol_major":1,"role":"describe","invocation_id":"request","result":{"kind":"description","graph":{"initial_state":"draft","states":[{"id":"draft","final":false,"static_guidance":{"kind":"none"}}],"transitions":[],"input_declarations":[],"live_guidance_supported":false}}}"#;
    let provider = config(
        format!(
            "(head -c 1100000 /dev/zero | tr '\\0' e >&2) & printf '%s' '{}'; wait",
            output
        ),
        directory.path(),
    );
    invoker(directory.path(), "trace-stderr")
        .describe(
            &provider,
            DescribeRequest {
                request_id: RequestId::parse("request").unwrap(),
            },
        )
        .unwrap();
    let trace =
        std::fs::read_to_string(directory.path().join("traces/trace-stderr.jsonl")).unwrap();
    let finish: serde_json::Value = serde_json::from_str(trace.lines().last().unwrap()).unwrap();
    assert_eq!(finish["event"], "finish");
    assert_eq!(finish["stderr_byte_length"], 1_100_000);
    assert_eq!(finish["stderr_truncated"], true);
    assert_eq!(finish["stderr_b64"].as_str().unwrap().len(), 1_398_104);
}

#[test]
fn gate_adapter_requires_exact_verdict_set_and_rejects_misplaced_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let request = || GateRequest {
        request_id: RequestId::parse("request").unwrap(),
        run: run_snapshot(),
        event: EventId::parse("go").unwrap(),
        selected_evidence: empty_evidence("selected_evidence"),
        inline_evidence: empty_evidence("inline_evidence"),
    };
    let valid = r#"{"protocol_major":1,"role":"evaluate_gates","invocation_id":"request","result":{"kind":"verdicts","verdicts":[{"gate_id":"g1","passed":true}],"evidence":[{"id":"provider-evidence","kind":"log","locator":"opaque","metadata":{"state":"caller-defined","event":"opaque"}}]}}"#;
    let provider = config(
        format!("cat >/dev/null; printf '%s' '{}'", valid),
        directory.path(),
    );
    let result = invoker(directory.path(), "gate-valid")
        .evaluate_gates(&provider, request())
        .unwrap();
    assert_eq!(result.fact.outcome, OutcomeClass::Completed);

    for (index, (kind, expected)) in [
        ("incompatible", OutcomeClass::Rejected),
        ("evaluation_error", OutcomeClass::Error),
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = format!(
            r#"{{"protocol_major":1,"role":"evaluate_gates","invocation_id":"request","result":{{"kind":"{kind}","diagnostics":[]}}}}"#
        );
        let provider = config(
            format!("cat >/dev/null; printf '%s' '{}'", envelope),
            directory.path(),
        );
        let result = invoker(directory.path(), &format!("gate-outcome-{index}"))
            .evaluate_gates(&provider, request())
            .unwrap();
        assert_eq!(result.fact.outcome, expected);
        assert!(matches!(
            (&result.evaluation, kind),
            (GateEvaluation::Incompatible(_), "incompatible")
                | (GateEvaluation::EvaluationError(_), "evaluation_error")
        ));
    }

    for (index, result) in [
        r#"{"kind":"verdicts","verdicts":[]}"#,
        r#"{"kind":"verdicts","verdicts":[{"gate_id":"g1","passed":true},{"gate_id":"g1","passed":true}]}"#,
        r#"{"kind":"verdicts","verdicts":[{"gate_id":"g2","passed":true}]}"#,
        r#"{"kind":"incompatible","diagnostics":[],"evidence":[]}"#,
        r#"{"kind":"evaluation_error","diagnostics":[],"evidence":[]}"#,
        r#"{"kind":"verdicts","verdicts":[{"gate_id":"g1","passed":true}],"evidence":[{"id":"","kind":"log","locator":"opaque"}]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = format!(
            "{{\"protocol_major\":1,\"role\":\"evaluate_gates\",\"invocation_id\":\"request\",\"result\":{result}}}"
        );
        let provider = config(format!("printf '%s' '{}'", envelope), directory.path());
        assert!(
            invoker(directory.path(), &format!("gate-invalid-{index}"))
                .evaluate_gates(&provider, request())
                .is_err(),
            "accepted invalid gate result {result}"
        );
    }
}

#[test]
fn guidance_and_compatibility_use_bounded_non_authoritative_results_and_stored_graph() {
    let directory = tempfile::tempdir().unwrap();
    let guidance = r#"{"protocol_major":1,"role":"live_guidance","invocation_id":"request","result":{"kind":"incompatible","diagnostics":[]}}"#;
    let provider = config(
        format!("cat >/dev/null; printf '%s' '{}'", guidance),
        directory.path(),
    );
    let result = invoker(directory.path(), "guidance-incompatible")
        .live_guidance(
            &provider,
            GuidanceRequest {
                request_id: RequestId::parse("request").unwrap(),
                run_id: RunId::parse("run-1").unwrap(),
                run: run_snapshot(),
                selected_evidence: empty_evidence("selected_evidence"),
            },
        )
        .unwrap();
    assert!(matches!(result.result, LiveGuidanceResult::Incompatible(_)));
    assert_eq!(result.fact.outcome, OutcomeClass::Rejected);

    let evaluation_error = r#"{"protocol_major":1,"role":"live_guidance","invocation_id":"request","result":{"kind":"evaluation_error","diagnostics":[]}}"#;
    let provider = config(
        format!("cat >/dev/null; printf '%s' '{}'", evaluation_error),
        directory.path(),
    );
    let result = invoker(directory.path(), "guidance-error")
        .live_guidance(
            &provider,
            GuidanceRequest {
                request_id: RequestId::parse("request").unwrap(),
                run_id: RunId::parse("run-1").unwrap(),
                run: run_snapshot(),
                selected_evidence: empty_evidence("selected_evidence"),
            },
        )
        .unwrap();
    assert!(matches!(
        result.result,
        LiveGuidanceResult::EvaluationError(_)
    ));
    assert_eq!(result.fact.outcome, OutcomeClass::Error);

    let authoritative = r#"{"protocol_major":1,"role":"live_guidance","invocation_id":"request","result":{"kind":"guidance","text":"go","state":"done"}}"#;
    let provider = config(format!("printf '%s' '{}'", authoritative), directory.path());
    assert!(
        invoker(directory.path(), "guidance-authority")
            .live_guidance(
                &provider,
                GuidanceRequest {
                    request_id: RequestId::parse("request").unwrap(),
                    run_id: RunId::parse("run-1").unwrap(),
                    run: run_snapshot(),
                    selected_evidence: empty_evidence("selected_evidence"),
                },
            )
            .is_err()
    );

    let oversized_file = directory.path().join("oversized-guidance.json");
    let oversized = format!(
        "{{\"protocol_major\":1,\"role\":\"live_guidance\",\"invocation_id\":\"request\",\"result\":{{\"kind\":\"guidance\",\"text\":{}}}}}",
        serde_json::to_string(&"x".repeat(262_145)).unwrap()
    );
    std::fs::write(&oversized_file, oversized).unwrap();
    let provider = config(
        format!("cat >/dev/null; cat '{}'", oversized_file.display()),
        directory.path(),
    );
    assert!(
        invoker(directory.path(), "guidance-oversized")
            .live_guidance(
                &provider,
                GuidanceRequest {
                    request_id: RequestId::parse("request").unwrap(),
                    run_id: RunId::parse("run-1").unwrap(),
                    run: run_snapshot(),
                    selected_evidence: empty_evidence("selected_evidence"),
                },
            )
            .is_err()
    );

    let request_file = directory.path().join("compat-request.json");
    let compatibility = r#"{"protocol_major":1,"role":"check_compatibility","invocation_id":"request","result":{"kind":"findings","capabilities":[{"capability":"graph","status":"compatible","diagnostics":[]},{"capability":"guidance","status":"unknown","diagnostics":[]}]}}"#;
    let provider = config(
        format!(
            "cat > '{}'; printf '%s' '{}'",
            request_file.display(),
            compatibility
        ),
        directory.path(),
    );
    let result = invoker(directory.path(), "compatibility")
        .check_compatibility(
            &provider,
            CompatibilityRequest {
                request_id: RequestId::parse("request").unwrap(),
                run_id: RunId::parse("run-1").unwrap(),
                run: run_snapshot(),
            },
        )
        .unwrap();
    assert!(matches!(result.report, CompatibilityReport::Findings(_)));
    assert_eq!(result.fact.outcome, OutcomeClass::Completed);
    let payload: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request_file).unwrap()).unwrap();
    assert_eq!(
        payload["payload"]["stored_graph"]["initial_state_id"],
        "draft"
    );
    assert!(payload["payload"].get("latest_graph").is_none());

    let compatibility_error = r#"{"protocol_major":1,"role":"check_compatibility","invocation_id":"request","result":{"kind":"evaluation_error","diagnostics":[]}}"#;
    let provider = config(
        format!("cat >/dev/null; printf '%s' '{}'", compatibility_error),
        directory.path(),
    );
    let result = invoker(directory.path(), "compatibility-error")
        .check_compatibility(
            &provider,
            CompatibilityRequest {
                request_id: RequestId::parse("request").unwrap(),
                run_id: RunId::parse("run-1").unwrap(),
                run: run_snapshot(),
            },
        )
        .unwrap();
    assert!(matches!(
        result.report,
        CompatibilityReport::EvaluationError(_)
    ));
    assert_eq!(result.fact.outcome, OutcomeClass::Error);
}
