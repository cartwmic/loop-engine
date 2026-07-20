use loop_engine_core::model::decision::{DecisionError, apply, resolve_gate_free, resolve_gated};
use loop_engine_core::model::evidence::{EvidenceRecord, EvidenceSource};
use loop_engine_core::model::gate::{GateEvaluation, GateVerdict};
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{
    EventId, EvidenceId, EvidenceKind, GateId, GraphRevision, RegistrationId, RunId, StateId,
};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::requestable;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::{InputDeclarations, RunInputs};
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::transition::Transition;

fn run_with_id(id: &str, gates: &[&str], self_loop: bool, target_final: bool) -> Run {
    let states = vec![
        State::new(
            StateId::parse("start").unwrap(),
            false,
            StaticGuidance::NoneRequired,
            None,
        ),
        State::new(
            StateId::parse("target").unwrap(),
            target_final,
            StaticGuidance::NoneRequired,
            None,
        ),
    ];
    let graph = WorkflowGraph::new_unvalidated(
        StateId::parse("start").unwrap(),
        states,
        vec![
            Transition::new(
                StateId::parse("start").unwrap(),
                EventId::parse("go").unwrap(),
                StateId::parse(if self_loop { "start" } else { "target" }).unwrap(),
                gates
                    .iter()
                    .map(|gate| GateId::parse(*gate).unwrap())
                    .collect(),
                None,
            )
            .unwrap(),
        ],
        InputDeclarations::default(),
        LiveGuidanceCapability::Unsupported,
        None,
    );
    Run::create(
        RunId::parse(id).unwrap(),
        RegistrationId::parse("registration").unwrap(),
        ValidatedGraph::validate(graph).unwrap(),
        GraphRevision::parse(format!("sha256:{}", "0".repeat(64))).unwrap(),
        RunInputs::default(),
        None,
    )
    .unwrap()
}

fn run(gates: &[&str], self_loop: bool, target_final: bool) -> Run {
    run_with_id("run", gates, self_loop, target_final)
}

fn verdict(gate: &str, passed: bool) -> GateVerdict {
    GateVerdict::new(GateId::parse(gate).unwrap(), passed, vec![])
}

#[test]
fn gate_free_linear_self_loop_final_terminal_and_unknown_matrix() {
    let mut linear = run(&[], false, true);
    let go = EventId::parse("go").unwrap();
    let decision = resolve_gate_free(&linear, &go).unwrap();
    assert!(decision.state_changed());
    apply(&mut linear, decision).unwrap();
    assert_eq!(linear.lifecycle(), Lifecycle::Final);
    assert!(requestable::project(&linear).is_empty());
    assert!(matches!(
        resolve_gate_free(&linear, &go),
        Err(DecisionError::Terminal)
    ));

    let mut self_loop = run(&[], true, false);
    let version = self_loop.workflow_state_version();
    let decision = resolve_gate_free(&self_loop, &go).unwrap();
    assert!(!decision.state_changed());
    apply(&mut self_loop, decision).unwrap();
    assert_eq!(self_loop.workflow_state_version(), version);
    assert!(matches!(
        resolve_gate_free(&self_loop, &EventId::parse("unknown").unwrap()),
        Err(DecisionError::UnknownEvent(_))
    ));
}

#[test]
fn gated_all_pass_advances_and_every_other_result_preserves_state() {
    let run = run(&["a", "b"], false, false);
    let go = EventId::parse("go").unwrap();
    assert!(
        resolve_gated(
            &run,
            &go,
            &GateEvaluation::verdicts(vec![verdict("b", true), verdict("a", true)])
        )
        .is_ok()
    );
    for (evaluation, expected) in [
        (
            GateEvaluation::verdicts(vec![verdict("a", true), verdict("b", false)]),
            "failed",
        ),
        (
            GateEvaluation::incompatible(vec![]).unwrap(),
            "incompatible",
        ),
        (GateEvaluation::evaluation_error(vec![]).unwrap(), "error"),
    ] {
        let result = resolve_gated(&run, &go, &evaluation);
        assert!(result.is_err(), "{expected}");
        assert_eq!(run.current_state().as_str(), "start");
    }
    assert!(matches!(
        resolve_gated(
            &run,
            &go,
            &GateEvaluation::verdicts(vec![verdict("a", true)])
        ),
        Err(DecisionError::MalformedVerdicts { .. })
    ));

    let evidence = EvidenceRecord::new(
        EvidenceId::parse("provider-evidence").unwrap(),
        EvidenceKind::parse("report").unwrap(),
        "opaque:report",
        None,
        None,
        None,
        EvidenceSource::Provider,
        ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
    )
    .unwrap();
    let failed = GateEvaluation::verdicts(vec![
        GateVerdict::new(GateId::parse("a").unwrap(), true, vec![]),
        GateVerdict::new(GateId::parse("b").unwrap(), false, vec![evidence]),
    ]);
    match resolve_gated(&run, &go, &failed) {
        Err(DecisionError::GateFailed { verdicts }) => {
            assert_eq!(verdicts[1].evidence().len(), 1);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn decision_is_bound_to_run_and_resolved_state() {
    let source = run_with_id("source", &[], false, false);
    let mut other = run_with_id("other", &[], false, false);
    let decision = resolve_gate_free(&source, &EventId::parse("go").unwrap()).unwrap();
    assert!(matches!(
        apply(&mut other, decision),
        Err(DecisionError::DecisionMismatch)
    ));
}

#[test]
fn requestable_projection_does_not_predict_gate_pass() {
    let active = run(&["a"], false, false);
    let events = requestable::project(&active);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].required_gates.len(), 1);
}
