pub mod config;
pub mod handler;
pub mod scenarios;
pub mod test_support;

mod barrier;
mod ledger;
mod ordinal;
pub mod protocol;
pub mod schema;

#[cfg(test)]
mod protocol_tests {
    use crate::protocol::{
        AnyResult, DescribeResultDto, GraphDto, ProviderRole, RegistrationDto, StateDto,
        StaticGuidanceDeclarationDto, StaticGuidanceDto, result_envelope,
    };
    use crate::scenarios::{Scenario, all_scenario_names};

    #[test]
    fn every_scenario_name_parses() {
        for name in all_scenario_names() {
            assert_eq!(Scenario::parse(name).unwrap().as_str(), *name);
        }
    }

    #[test]
    fn graph_cycle_fixture_matches_describe_output() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/graphs/cycle.json")).unwrap();
        let result = Scenario::GraphCycle.handle(&crate::protocol::AnyRequest {
            protocol_major: 1,
            role: ProviderRole::Describe,
            invocation_id: "test".into(),
            registration: sample_registration(),
            payload: serde_json::json!({}),
        });
        let AnyResult::Describe(DescribeResultDto::Description { graph }) = result else {
            panic!("expected describe result");
        };
        assert_eq!(
            graph.initial_state,
            fixture["expected"]["result"]["graph"]["initial_state"]
        );
        assert_eq!(graph.transitions.len(), 2);
    }

    #[test]
    fn graph_build_drift_changes_graph_by_ordinal() {
        use crate::scenarios::graphs;
        let first = graphs::describe(Scenario::GraphBuildDrift, Some(1));
        let second = graphs::describe(Scenario::GraphBuildDrift, Some(2));
        let DescribeResultDto::Description { graph: first_graph } = first;
        let DescribeResultDto::Description {
            graph: second_graph,
        } = second;
        assert_eq!(first_graph.initial_state, "build-a-v1");
        assert_eq!(second_graph.initial_state, "build-b-v2");
        assert_ne!(first_graph.states.len(), second_graph.states.len());
    }

    #[test]
    fn malformed_gate_payload_maps_to_evaluation_error() {
        let result = Scenario::GatePass.handle(&crate::protocol::AnyRequest {
            protocol_major: 1,
            role: ProviderRole::EvaluateGates,
            invocation_id: "bad".into(),
            registration: sample_registration(),
            payload: serde_json::json!({"not": "a gate payload"}),
        });
        let AnyResult::EvaluateGates(crate::protocol::GateResultDto::EvaluationError {
            diagnostics,
        }) = result
        else {
            panic!("expected evaluation error");
        };
        assert_eq!(diagnostics[0].code, "provider.protocol.malformed");
    }

    #[test]
    fn result_envelope_sets_protocol_major_one() {
        let envelope = result_envelope(
            ProviderRole::Describe,
            "inv",
            AnyResult::Describe(DescribeResultDto::Description {
                graph: GraphDto {
                    initial_state: "draft".into(),
                    states: vec![StateDto {
                        id: "draft".into(),
                        final_state: false,
                        static_guidance: StaticGuidanceDto::Declaration(
                            StaticGuidanceDeclarationDto::None,
                        ),
                        metadata: None,
                    }],
                    transitions: vec![],
                    input_declarations: vec![],
                    live_guidance_supported: false,
                    metadata: None,
                },
            }),
        );
        assert_eq!(envelope["protocol_major"], 1);
        assert_eq!(envelope["invocation_id"], "inv");
    }

    fn sample_registration() -> RegistrationDto {
        RegistrationDto {
            registration_id: "reg".into(),
            config_revision: 1,
            executable: "/tmp/scenario-provider".into(),
            argv: vec![],
            working_directory: "/tmp".into(),
            timeout_seconds: 60,
        }
    }
}
