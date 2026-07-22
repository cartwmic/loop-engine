use crate::protocol::{
    DescribeResultDto, GraphDto, InputDeclarationDto, StateDto, StaticGuidanceDto, TransitionDto,
};
use crate::scenarios::Scenario;

fn guidance_none() -> StaticGuidanceDto {
    StaticGuidanceDto::Declaration(crate::protocol::StaticGuidanceDeclarationDto::None)
}

fn state(id: &str, final_state: bool) -> StateDto {
    StateDto {
        id: id.to_string(),
        final_state,
        static_guidance: guidance_none(),
        metadata: None,
    }
}

fn transition(source: &str, event: &str, target: &str, gate_ids: &[&str]) -> TransitionDto {
    TransitionDto {
        source_state: source.to_string(),
        event: event.to_string(),
        target_state: target.to_string(),
        gate_ids: gate_ids.iter().map(|gate| gate.to_string()).collect(),
        metadata: None,
    }
}

pub fn describe(scenario: Scenario, invocation_ordinal: Option<u64>) -> DescribeResultDto {
    let graph = match scenario {
        Scenario::GraphLinear => GraphDto {
            initial_state: "start".into(),
            states: vec![
                state("start", false),
                state("middle", false),
                state("done", true),
            ],
            transitions: vec![
                transition("start", "advance", "middle", &[]),
                transition("middle", "finish", "done", &[]),
            ],
            input_declarations: sample_inputs(),
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphCycle => GraphDto {
            initial_state: "a".into(),
            states: vec![state("a", false), state("b", false)],
            transitions: vec![
                transition("a", "forward", "b", &[]),
                transition("b", "back", "a", &[]),
            ],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphSelfLoop => GraphDto {
            initial_state: "draft".into(),
            states: vec![state("draft", false)],
            transitions: vec![transition("draft", "checkpoint", "draft", &[])],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphZeroFinal => GraphDto {
            initial_state: "ongoing".into(),
            states: vec![state("ongoing", false), state("review", false)],
            transitions: vec![transition("ongoing", "advance", "review", &[])],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphMultiFinal => GraphDto {
            initial_state: "start".into(),
            states: vec![
                state("start", false),
                state("done-a", true),
                state("done-b", true),
            ],
            transitions: vec![
                transition("start", "finish-a", "done-a", &[]),
                transition("start", "finish-b", "done-b", &[]),
            ],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphInitialFinal => GraphDto {
            initial_state: "done".into(),
            states: vec![state("done", true)],
            transitions: vec![],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphNonFinalSink => GraphDto {
            initial_state: "start".into(),
            states: vec![state("start", false), state("sink", false)],
            transitions: vec![transition("start", "fall", "sink", &[])],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphAmbiguousDuplicateState => GraphDto {
            initial_state: "dup".into(),
            states: vec![state("dup", false), state("dup", true)],
            transitions: vec![],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphAmbiguousDuplicateEvent => GraphDto {
            initial_state: "start".into(),
            states: vec![state("start", false), state("other", false)],
            transitions: vec![
                transition("start", "same", "other", &[]),
                transition("start", "same", "start", &[]),
            ],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphStructurallyInvalid => GraphDto {
            initial_state: "missing".into(),
            states: vec![state("present", false)],
            transitions: vec![transition("present", "go", "missing", &[])],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        Scenario::GraphGuidanceSupported => GraphDto {
            initial_state: "draft".into(),
            states: vec![state("draft", false)],
            transitions: vec![],
            input_declarations: vec![],
            live_guidance_supported: true,
            metadata: None,
        },
        Scenario::GraphBuildDrift => {
            let build = invocation_ordinal.unwrap_or(1);
            if build <= 1 {
                GraphDto {
                    initial_state: "build-a-v1".into(),
                    states: vec![state("build-a-v1", false), state("review-a", false)],
                    transitions: vec![transition("build-a-v1", "advance", "review-a", &[])],
                    input_declarations: sample_inputs(),
                    live_guidance_supported: false,
                    metadata: None,
                }
            } else {
                GraphDto {
                    initial_state: "build-b-v2".into(),
                    states: vec![
                        state("build-b-v2", false),
                        state("review-b", false),
                        state("done-b", true),
                    ],
                    transitions: vec![
                        transition("build-b-v2", "advance", "review-b", &[]),
                        transition("review-b", "finish", "done-b", &[]),
                    ],
                    input_declarations: sample_inputs(),
                    live_guidance_supported: true,
                    metadata: None,
                }
            }
        }
        Scenario::GraphGuidanceUnsupported => GraphDto {
            initial_state: "draft".into(),
            states: vec![state("draft", false)],
            transitions: vec![],
            input_declarations: vec![],
            live_guidance_supported: false,
            metadata: None,
        },
        _ => GraphDto {
            initial_state: "draft".into(),
            states: vec![state("draft", false)],
            transitions: vec![],
            input_declarations: sample_inputs(),
            live_guidance_supported: false,
            metadata: None,
        },
    };
    DescribeResultDto::Description { graph }
}

fn sample_inputs() -> Vec<InputDeclarationDto> {
    vec![
        InputDeclarationDto {
            id: "ticket".into(),
            kind: "string".into(),
            required: true,
            metadata: None,
        },
        InputDeclarationDto {
            id: "note".into(),
            kind: "string".into(),
            required: false,
            metadata: None,
        },
    ]
}
