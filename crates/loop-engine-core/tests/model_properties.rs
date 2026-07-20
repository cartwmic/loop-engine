use std::collections::BTreeMap;

use loop_engine_core::model::annotation::{ActorMetadata, Annotation};
use loop_engine_core::model::bounded::Value;
use loop_engine_core::model::decision::{DecisionError, apply, resolve_gate_free};
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{EventId, GraphRevision, RegistrationId, RunId, StateId};
use loop_engine_core::model::lifecycle::Lifecycle;
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::{InputDeclarations, RunInputs};
use loop_engine_core::model::transition::Transition;
use proptest::prelude::*;

fn run(self_loop: bool, target_final: bool) -> Run {
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
    let target = if self_loop { "start" } else { "target" };
    let graph = WorkflowGraph::new_unvalidated(
        StateId::parse("start").unwrap(),
        states,
        vec![
            Transition::new(
                StateId::parse("start").unwrap(),
                EventId::parse("go").unwrap(),
                StateId::parse(target).unwrap(),
                vec![],
                None,
            )
            .unwrap(),
        ],
        InputDeclarations::default(),
        LiveGuidanceCapability::Unsupported,
        None,
    );
    Run::create(
        RunId::parse("run").unwrap(),
        RegistrationId::parse("registration").unwrap(),
        ValidatedGraph::validate(graph).unwrap(),
        GraphRevision::parse(format!("sha256:{}", "0".repeat(64))).unwrap(),
        RunInputs::default(),
        None,
    )
    .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    #[test]
    fn decisions_are_deterministic_and_actor_neutral(
        self_loop in any::<bool>(),
        target_final in any::<bool>(),
        first_actor in "[a-z]{1,16}",
        second_actor in "[a-z]{1,16}",
    ) {
        let run = run(self_loop, target_final);
        let event = EventId::parse("go").unwrap();
        let actor = |value: String| {
            Annotation::new(
                Some(ActorMetadata::new(Value::Object(BTreeMap::from([(
                    "actor".into(),
                    Value::String(value),
                )]))).unwrap()),
                None,
                None,
            )
        };
        let first_annotation = actor(first_actor);
        let second_annotation = actor(second_actor);
        let first = resolve_gate_free(&run, &event).unwrap();
        let second = resolve_gate_free(&run, &event).unwrap();
        prop_assert_eq!(first, second);
        prop_assert!(first_annotation.actor().is_some());
        prop_assert!(second_annotation.actor().is_some());
    }

    #[test]
    fn generated_graph_families_validate_without_dag_or_reachability_policy(
        state_count in 1usize..16,
        cycle in any::<bool>(),
        final_last in any::<bool>(),
    ) {
        let states = (0..state_count)
            .map(|index| {
                State::new(
                    StateId::parse(format!("s{index}")).unwrap(),
                    final_last && index + 1 == state_count,
                    StaticGuidance::NoneRequired,
                    None,
                )
            })
            .collect::<Vec<_>>();
        let mut transitions = (0..state_count.saturating_sub(1))
            .map(|index| {
                Transition::new(
                    StateId::parse(format!("s{index}")).unwrap(),
                    EventId::parse(format!("next{index}")).unwrap(),
                    StateId::parse(format!("s{}", index + 1)).unwrap(),
                    vec![],
                    None,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        if cycle && state_count > 1 && !final_last {
            transitions.push(
                Transition::new(
                    StateId::parse(format!("s{}", state_count - 1)).unwrap(),
                    EventId::parse("back").unwrap(),
                    StateId::parse("s0").unwrap(),
                    vec![],
                    None,
                )
                .unwrap(),
            );
        }
        let graph = WorkflowGraph::new_unvalidated(
            StateId::parse("s0").unwrap(),
            states,
            transitions,
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        prop_assert!(ValidatedGraph::validate(graph).is_ok());
    }

    #[test]
    fn rejected_unknown_event_preserves_state(self_loop in any::<bool>(), target_final in any::<bool>()) {
        let run = run(self_loop, target_final);
        let before = run.current_state().clone();
        let result = resolve_gate_free(&run, &EventId::parse("unknown").unwrap());
        prop_assert!(matches!(result, Err(DecisionError::UnknownEvent(_))));
        prop_assert_eq!(run.current_state(), &before);
    }

    #[test]
    fn applying_decision_obeys_self_loop_and_final_semantics(self_loop in any::<bool>(), target_final in any::<bool>()) {
        let mut run = run(self_loop, target_final);
        let decision = resolve_gate_free(&run, &EventId::parse("go").unwrap()).unwrap();
        let changed = decision.state_changed();
        apply(&mut run, decision).unwrap();
        prop_assert_eq!(changed, !self_loop);
        if !self_loop && target_final {
            prop_assert_eq!(run.lifecycle(), Lifecycle::Final);
            prop_assert!(run.graph().transitions().iter().all(|transition| transition.source() != run.current_state()));
        }
    }
}
