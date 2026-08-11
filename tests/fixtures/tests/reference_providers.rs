use loop_core::{
    ContextRecord, DurableEvaluation, EvaluationFeedback, EvaluationRequest, EvaluationResult,
    ProviderAssociation, ProviderGateway, SemanticSequence, Timestamp, Transition,
};
use loop_integrations::{ProviderError, ProviderInvocation, SubprocessProviderGateway};
use loop_reference_fixtures::{
    agents_policy_input, document_policy, fixture_binary, policy_document_initial_input,
    policy_document_workflow, readme_policy_input, software_change_initial_input,
    software_change_initial_input_with_behavior, software_change_policy_set_a,
    software_change_policy_set_b, software_change_review_context, software_change_workflow,
    FixtureProvider, DESIGN_REVIEW_GATE, PLAN_REVIEW_GATE,
};
use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

fn association(binary: &str) -> ProviderAssociation {
    ProviderInvocation::new(
        fixture_binary(binary).to_string_lossy().into_owned(),
        Vec::<String>::new(),
    )
    .to_association()
}

fn gateway() -> SubprocessProviderGateway {
    SubprocessProviderGateway::new(Duration::from_secs(2))
}

fn software_request(
    input: Value,
    transition: Transition,
    context: Vec<ContextRecord>,
    prior_evaluations: Vec<DurableEvaluation>,
) -> EvaluationRequest {
    EvaluationRequest::new(
        software_change_workflow(),
        input,
        context,
        transition,
        prior_evaluations,
    )
}

fn document_request(
    input: Value,
    transition: Transition,
    prior_evaluations: Vec<DurableEvaluation>,
) -> EvaluationRequest {
    EvaluationRequest::new(
        policy_document_workflow(),
        input,
        Vec::new(),
        transition,
        prior_evaluations,
    )
}

fn evaluate_software(
    input: Value,
    transition: Transition,
    context: Vec<ContextRecord>,
    prior_evaluations: Vec<DurableEvaluation>,
) -> Result<EvaluationResult, ProviderError> {
    gateway().evaluate(
        &association("software-change-provider"),
        software_request(input, transition, context, prior_evaluations),
    )
}

fn all_evidence_for_set_a(gate: &str, sequence_start: u64) -> Vec<ContextRecord> {
    let policies = match gate {
        DESIGN_REVIEW_GATE => vec!["architecture", "compatibility"],
        PLAN_REVIEW_GATE => vec!["coverage"],
        "implementation-review" => vec!["correctness"],
        "validation" => vec!["regression"],
        _ => Vec::new(),
    };
    policies
        .into_iter()
        .enumerate()
        .map(|(index, policy_id)| {
            software_change_review_context(
                &format!("evidence-{policy_id}"),
                gate,
                policy_id,
                true,
                json!([]),
                sequence_start + index as u64,
            )
        })
        .collect()
}

#[test]
fn software_change_describe_exposes_exact_reference_topology(
) -> Result<(), Box<dyn std::error::Error>> {
    let described = gateway().describe(&association("software-change-provider"))?;
    assert_eq!(described, software_change_workflow());
    assert_eq!(described.states.len(), 9);
    assert_eq!(described.transitions.len(), 12);
    assert_eq!(
        described
            .states
            .iter()
            .filter(|state| state.is_final)
            .map(|state| state.id.as_str())
            .collect::<Vec<_>>(),
        vec!["end"]
    );
    Ok(())
}

#[test]
fn configured_policy_set_is_read_from_initial_input_and_missing_evidence_denies() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let result = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition,
        Vec::new(),
        Vec::new(),
    )
    .expect("provider response");
    let EvaluationResult::Deny { feedback } = result else {
        panic!("missing review evidence must deny");
    };
    assert_eq!(feedback.code, "software-change-review-incomplete");
    assert!(feedback.message.contains("architecture"));
    assert!(feedback.message.contains("compatibility"));
    assert_eq!(
        feedback.details.as_ref().unwrap()["gate"],
        DESIGN_REVIEW_GATE
    );
    assert_eq!(
        feedback.details.as_ref().unwrap()["missing"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn failed_required_evidence_has_actionable_policy_specific_feedback() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let context = vec![
        software_change_review_context(
            "architecture-pass",
            DESIGN_REVIEW_GATE,
            "architecture",
            true,
            json!([]),
            2,
        ),
        software_change_review_context(
            "compatibility-fail",
            DESIGN_REVIEW_GATE,
            "compatibility",
            false,
            json!(["Document the compatibility boundary"]),
            3,
        ),
    ];
    let result = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition,
        context,
        Vec::new(),
    )
    .expect("provider response");
    let EvaluationResult::Deny { feedback } = result else {
        panic!("failed review evidence must deny");
    };
    assert_eq!(feedback.code, "software-change-review-incomplete");
    assert!(feedback.message.contains("compatibility"));
    assert!(feedback.details.as_ref().unwrap()["failed"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "Document the compatibility boundary"));
}

#[test]
fn all_required_evidence_passes_and_prior_denial_lineage_is_visible_to_fixture() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let prior = DurableEvaluation::deny(
        transition.clone(),
        EvaluationFeedback::new("software-change-review-incomplete", "Earlier finding"),
        SemanticSequence::new(1),
        Timestamp::from_unix_millis(1),
    );
    let result = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition.clone(),
        all_evidence_for_set_a(DESIGN_REVIEW_GATE, 2),
        vec![prior],
    )
    .expect("provider response");
    assert_eq!(result, EvaluationResult::Allow);

    let incomplete = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition,
        vec![all_evidence_for_set_a(DESIGN_REVIEW_GATE, 2)[0].clone()],
        vec![DurableEvaluation::deny(
            Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan"),
            EvaluationFeedback::new("earlier", "Earlier actionable finding"),
            SemanticSequence::new(1),
            Timestamp::from_unix_millis(1),
        )],
    )
    .expect("provider response");
    let EvaluationResult::Deny { feedback } = incomplete else {
        panic!("incomplete evidence must deny");
    };
    assert_eq!(
        feedback.details.as_ref().unwrap()["prior_denials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unrelated_gate_or_policy_evidence_does_not_satisfy_selected_gate() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let context = vec![
        software_change_review_context(
            "wrong-gate",
            PLAN_REVIEW_GATE,
            "architecture",
            true,
            json!([]),
            1,
        ),
        software_change_review_context(
            "wrong-policy",
            DESIGN_REVIEW_GATE,
            "not-configured",
            true,
            json!([]),
            2,
        ),
    ];
    let result = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition,
        context,
        Vec::new(),
    )
    .expect("provider response");
    let EvaluationResult::Deny { feedback } = result else {
        panic!("unrelated evidence must not satisfy the gate");
    };
    assert!(feedback.message.contains("architecture"));
    assert!(feedback.message.contains("compatibility"));
}

#[test]
fn two_material_policy_sets_use_one_provider_and_unchanged_topology() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let first = evaluate_software(
        software_change_initial_input(software_change_policy_set_a(), None),
        transition.clone(),
        all_evidence_for_set_a(DESIGN_REVIEW_GATE, 1),
        Vec::new(),
    )
    .expect("provider response");
    assert_eq!(first, EvaluationResult::Allow);

    let second_context = vec![software_change_review_context(
        "security-pass",
        DESIGN_REVIEW_GATE,
        "security-boundary",
        true,
        json!([]),
        1,
    )];
    let second = evaluate_software(
        software_change_initial_input(software_change_policy_set_b(), None),
        transition,
        second_context,
        Vec::new(),
    )
    .expect("provider response");
    assert_eq!(second, EvaluationResult::Allow);
    assert_eq!(software_change_workflow(), software_change_workflow());
}

#[test]
fn fixture_has_no_prompt_generation_and_supports_controlled_provider_failures() {
    let transition = Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan");
    let input = software_change_initial_input(software_change_policy_set_a(), None);
    let mut input_with_prompt = input.clone();
    input_with_prompt["prompt"] = Value::String("caller-owned prompt".to_owned());

    let request_without_prompt = software_request(
        input,
        transition.clone(),
        all_evidence_for_set_a(DESIGN_REVIEW_GATE, 1),
        Vec::new(),
    );
    let request_with_prompt = software_request(
        input_with_prompt,
        transition.clone(),
        all_evidence_for_set_a(DESIGN_REVIEW_GATE, 1),
        Vec::new(),
    );
    let mut request_without_prompt = serde_json::to_value(request_without_prompt).unwrap();
    request_without_prompt["operation"] = Value::String("evaluate".to_owned());
    let mut request_with_prompt = serde_json::to_value(request_with_prompt).unwrap();
    request_with_prompt["operation"] = Value::String("evaluate".to_owned());
    let first = loop_reference_fixtures::process_request(
        FixtureProvider::SoftwareChange,
        request_without_prompt,
    )
    .unwrap();
    let second = loop_reference_fixtures::process_request(
        FixtureProvider::SoftwareChange,
        request_with_prompt,
    )
    .unwrap();
    assert_eq!(first, second);

    let unsupported = evaluate_software(
        software_change_initial_input_with_behavior(
            software_change_policy_set_a(),
            None,
            "unsupported",
        ),
        transition.clone(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(unsupported, EvaluationResult::Unsupported);

    let failure = evaluate_software(
        software_change_initial_input_with_behavior(
            software_change_policy_set_a(),
            None,
            "failure",
        ),
        transition,
        Vec::new(),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(failure, ProviderError::Execution { .. }));
}

#[test]
fn policy_document_describe_has_prepare_deterministic_semantic_end_topology(
) -> Result<(), Box<dyn std::error::Error>> {
    let described = gateway().describe(&association("policy-document-provider"))?;
    assert_eq!(described, policy_document_workflow());
    assert_eq!(
        described
            .states
            .iter()
            .map(|state| state.id.as_str())
            .collect::<Vec<_>>(),
        vec!["prepare", "deterministic-review", "semantic-review", "end"]
    );
    Ok(())
}

#[test]
fn policy_document_draft_and_audit_modes_run_through_both_review_phases(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let document = directory.path().join("README.md");
    fs::write(
        &document,
        "# Loop Engine\n\ncargo test\nworkflow coordination\n",
    )?;
    for mode in ["draft", "audit"] {
        let input = readme_policy_input(document.to_string_lossy(), mode);
        let deterministic = gateway().evaluate(
            &association("policy-document-provider"),
            document_request(
                input.clone(),
                Transition::checked("deterministic-review", "passed", "semantic-review"),
                Vec::new(),
            ),
        )?;
        assert_eq!(deterministic, EvaluationResult::Allow);
        let semantic = gateway().evaluate(
            &association("policy-document-provider"),
            document_request(
                input,
                Transition::checked("semantic-review", "passed", "end"),
                Vec::new(),
            ),
        )?;
        assert_eq!(semantic, EvaluationResult::Allow);
    }
    Ok(())
}

#[test]
fn policy_document_deterministic_and_semantic_failures_are_actionable(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let document = directory.path().join("README.md");
    fs::write(&document, "# Loop Engine\n")?;
    let input = readme_policy_input(document.to_string_lossy(), "draft");

    let deterministic = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(
            input.clone(),
            Transition::checked("deterministic-review", "passed", "semantic-review"),
            Vec::new(),
        ),
    )?;
    let EvaluationResult::Deny { feedback } = deterministic else {
        panic!("missing deterministic policy must deny");
    };
    assert_eq!(feedback.code, "policy-document-deterministic-failed");
    assert!(feedback.message.contains("readme-start-command"));

    fs::write(&document, "# Loop Engine\ncargo test\n")?;
    let semantic = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(
            input,
            Transition::checked("semantic-review", "passed", "end"),
            Vec::new(),
        ),
    )?;
    let EvaluationResult::Deny { feedback } = semantic else {
        panic!("missing semantic policy must deny");
    };
    assert_eq!(feedback.code, "policy-document-semantic-failed");
    assert!(feedback.message.contains("readme-purpose"));
    Ok(())
}

#[test]
fn final_semantic_review_rechecks_current_external_document_and_lineage(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let document = directory.path().join("README.md");
    fs::write(
        &document,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    let input = readme_policy_input(document.to_string_lossy(), "draft");
    let deterministic_transition =
        Transition::checked("deterministic-review", "passed", "semantic-review");
    let semantic_transition = Transition::checked("semantic-review", "passed", "end");
    let deterministic = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(input.clone(), deterministic_transition, Vec::new()),
    )?;
    assert_eq!(deterministic, EvaluationResult::Allow);

    fs::write(&document, "# Loop Engine\n")?;
    let prior = DurableEvaluation::deny(
        semantic_transition.clone(),
        EvaluationFeedback::new("old", "Earlier semantic finding"),
        SemanticSequence::new(4),
        Timestamp::from_unix_millis(4),
    );
    let regressed = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(input.clone(), semantic_transition.clone(), vec![prior]),
    )?;
    let EvaluationResult::Deny { feedback } = regressed else {
        panic!("deterministic regression must block final semantic review");
    };
    assert_eq!(feedback.code, "policy-document-deterministic-failed");
    assert!(feedback.details.as_ref().unwrap()["prior_findings"].is_array());

    fs::write(
        &document,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    let restored = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(input, semantic_transition, Vec::new()),
    )?;
    assert_eq!(restored, EvaluationResult::Allow);
    Ok(())
}

#[test]
fn independent_document_review_ignores_prior_lineage() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let document = directory.path().join("AGENTS.md");
    fs::write(&document, "Repository scope\nValidation\n")?;
    let input = agents_policy_input(document.to_string_lossy(), "audit");
    let transition = Transition::checked("semantic-review", "passed", "end");
    let prior = DurableEvaluation::deny(
        transition.clone(),
        EvaluationFeedback::new("old", "Prior finding must not steer independent review"),
        SemanticSequence::new(1),
        Timestamp::from_unix_millis(1),
    );
    let without_lineage = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(input.clone(), transition.clone(), Vec::new()),
    )?;
    let with_lineage = gateway().evaluate(
        &association("policy-document-provider"),
        document_request(input, transition, vec![prior]),
    )?;
    assert_eq!(without_lineage, with_lineage);
    assert_eq!(without_lineage, EvaluationResult::Deny {
        feedback: EvaluationFeedback::new(
            "policy-document-semantic-failed",
            "policy-document-semantic-failed: the current document does not satisfy all required policies (agents-handoff)",
        )
        .with_details(json!({
            "mode": "audit",
            "findings": [{
                "id": "agents-handoff",
                "description": "Agent instructions explain durable handoff.",
                "message": "semantic policy `agents-handoff` is not satisfied by the current document",
                "rule": "required_text",
                "value": "durable handoff"
            }],
            "review_mode": "independent"
        }))
    });
    Ok(())
}

#[test]
fn readme_and_agents_policy_inputs_share_one_provider_mechanism(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let readme = directory.path().join("README.md");
    fs::write(
        &readme,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    let agents = directory.path().join("AGENTS.md");
    fs::write(&agents, "Repository scope\nValidation\ndurable handoff\n")?;

    for (path, input) in [
        (
            readme.clone(),
            readme_policy_input(readme.to_string_lossy(), "draft"),
        ),
        (
            agents.clone(),
            agents_policy_input(agents.to_string_lossy(), "audit"),
        ),
    ] {
        assert_eq!(input["document"]["path"], path.to_string_lossy().as_ref());
        let result = gateway().evaluate(
            &association("policy-document-provider"),
            document_request(
                input,
                Transition::checked("semantic-review", "passed", "end"),
                Vec::new(),
            ),
        )?;
        assert_eq!(result, EvaluationResult::Allow);
    }
    Ok(())
}

#[test]
fn fixture_provider_processes_are_fresh_and_do_not_rely_on_prompt_or_memory(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let document = directory.path().join("README.md");
    fs::write(
        &document,
        "# Loop Engine\ncargo test\nworkflow coordination\n",
    )?;
    let input = readme_policy_input(document.to_string_lossy(), "draft");
    let gateway = gateway();
    let association = association("policy-document-provider");
    let request = document_request(
        input,
        Transition::checked("semantic-review", "passed", "end"),
        Vec::new(),
    );
    assert_eq!(
        gateway.evaluate(&association, request.clone())?,
        EvaluationResult::Allow
    );
    assert_eq!(
        gateway.evaluate(&association, request)?,
        EvaluationResult::Allow
    );
    Ok(())
}

#[test]
fn fixture_direct_request_rejects_raw_history_fields() {
    let mut request = serde_json::to_value(software_request(
        software_change_initial_input(software_change_policy_set_a(), None),
        Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan"),
        Vec::new(),
        Vec::new(),
    ))
    .unwrap();
    request["operation"] = Value::String("evaluate".to_owned());
    request["history"] = json!([]);
    let error = loop_reference_fixtures::process_request(FixtureProvider::SoftwareChange, request)
        .unwrap_err();
    assert!(error.message().contains("unknown field") || error.message().contains("history"));
}

#[test]
fn malformed_fixture_behavior_maps_to_gateway_protocol_error() {
    let result = evaluate_software(
        software_change_initial_input_with_behavior(
            software_change_policy_set_a(),
            None,
            "malformed",
        ),
        Transition::checked(DESIGN_REVIEW_GATE, "approved", "plan"),
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(
        result,
        Err(ProviderError::MalformedResponse { .. })
    ));
}

#[test]
fn policy_document_input_constructor_keeps_identity_and_policy_shapes() {
    let input = policy_document_initial_input(
        "draft",
        "file:///tmp/README.md",
        json!([document_policy(
            "required",
            "Need heading",
            "required_text",
            "# H"
        )]),
        json!([document_policy(
            "meaning",
            "Need purpose",
            "contains_text",
            "purpose"
        )]),
        "lineage-aware",
    );
    assert_eq!(input["mode"], "draft");
    assert_eq!(input["document"]["identity"], "file:///tmp/README.md");
    assert_eq!(input["semantic_policies"][0]["id"], "meaning");
}
