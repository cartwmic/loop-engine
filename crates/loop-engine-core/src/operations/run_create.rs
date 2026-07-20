use thiserror::Error;

use crate::capabilities::digest::DigestComputer;
use crate::capabilities::id_generator::IdGenerator;
use crate::capabilities::persistence_commands::{CommitStatus, CreateRunCommand};
use crate::capabilities::provider_catalog::{ProviderCatalog, ResolvedProviderConfig};
use crate::capabilities::provider_invoker::{
    DescribeRequest, DescribeResult, DescribedGraph, InputValidationInvocationResult,
    InputValidationResult, InvocationError, ProviderInvoker, ValidateInputsRequest,
};
use crate::capabilities::run_writer::RunWriter;
use crate::model::attempt::{JournalExtension, ProviderRole};
use crate::model::graph_projection::SemanticGraphProjection;
use crate::model::graph_validation::{GraphError, ValidatedGraph};
use crate::model::ids::{GraphRevision, RegistrationId, RunId};
use crate::model::journal::{JournalDraft, JournalEntryKind};
use crate::model::outcome::OutcomeClass;
use crate::model::provider::DigestObservation;
use crate::model::run::Run;
use crate::model::run_input::{InputDeclarations, RunInputs};
use crate::operations::{CommandError, validate_journal};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunCreateError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Bound(#[from] crate::model::bounded::BoundError),
    #[error("provider rejected run inputs")]
    InputsRejected,
    #[error("provider input evaluation failed")]
    InputEvaluationError,
    #[error("provider executable changed during creation")]
    DigestDrift,
    #[error("provider configuration revision must be positive")]
    InvalidProviderConfigRevision,
    #[error(transparent)]
    Command(#[from] CommandError),
}

pub fn needs_input_validation(declarations: &InputDeclarations, inputs: &RunInputs) -> bool {
    declarations.values().next().is_some() || !inputs.values().is_empty()
}

pub fn reject_observed_digest_drift(
    before: &DigestObservation,
    after: &DigestObservation,
) -> Result<(), RunCreateError> {
    match (before, after) {
        (DigestObservation::Observed(before), DigestObservation::Observed(after))
            if before != after =>
        {
            Err(RunCreateError::DigestDrift)
        }
        _ => Ok(()),
    }
}

pub fn classify_input_result(result: &InputValidationResult) -> Result<(), RunCreateError> {
    match result {
        InputValidationResult::Accepted => Ok(()),
        InputValidationResult::Rejected(_) => Err(RunCreateError::InputsRejected),
        InputValidationResult::EvaluationError(_) => Err(RunCreateError::InputEvaluationError),
    }
}

fn validate_described_graph(graph: DescribedGraph) -> Result<ValidatedGraph, GraphError> {
    match graph {
        DescribedGraph::Declared(graph) => ValidatedGraph::validate(graph),
        DescribedGraph::Invalid(error) => Err(error),
    }
}

pub fn build_run(
    run_id: RunId,
    registration_id: RegistrationId,
    described: DescribeResult,
    graph_revision: GraphRevision,
    inputs: RunInputs,
    label: Option<String>,
) -> Result<Run, RunCreateError> {
    Ok(Run::create(
        run_id,
        registration_id,
        validate_described_graph(described.graph)?,
        graph_revision,
        inputs,
        label,
    )?)
}

#[derive(Debug)]
pub enum RunCreateExecutionError<C, I, D, G, W, J> {
    Catalog(C),
    Invocation(InvocationError<I>),
    Digest(D),
    Id(G),
    Journal(J),
    Writer(W),
    Operation(RunCreateError),
}

type RunCreateExecutionResult<C, I, D, G, W, J> =
    Result<CommitStatus, RunCreateExecutionError<C, I, D, G, W, J>>;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn execute<C, I, D, G, W, F, J>(
    catalog: &C,
    invoker: &I,
    digests: &D,
    ids: &G,
    writer: &W,
    registration_id: &RegistrationId,
    inputs: RunInputs,
    label: Option<String>,
    journal: F,
) -> RunCreateExecutionResult<C::Error, I::TransportError, D::Error, G::Error, W::Error, J>
where
    C: ProviderCatalog,
    I: ProviderInvoker,
    D: DigestComputer,
    D::Error: std::fmt::Display,
    G: IdGenerator,
    W: RunWriter,
    F: FnOnce(
        &crate::capabilities::provider_catalog::ResolvedProviderConfig,
        &DescribeResult,
        Option<&InputValidationInvocationResult>,
        &Run,
    ) -> Result<JournalDraft, J>,
{
    let config = catalog
        .resolve_enabled(registration_id)
        .map_err(RunCreateExecutionError::Catalog)?;
    let digest_before = digests
        .executable_digest(&config)
        .map_err(RunCreateExecutionError::Digest)?;
    let described = invoker
        .describe(
            &config,
            DescribeRequest {
                request_id: ids.request_id().map_err(RunCreateExecutionError::Id)?,
            },
        )
        .map_err(RunCreateExecutionError::Invocation)?;
    let validated = validate_described_graph(described.graph.clone())
        .map_err(|error| RunCreateExecutionError::Operation(error.into()))?;
    let input_result = if needs_input_validation(validated.graph().inputs(), &inputs) {
        let result = invoker
            .validate_inputs(
                &config,
                ValidateInputsRequest {
                    request_id: ids.request_id().map_err(RunCreateExecutionError::Id)?,
                    input_declarations: validated.graph().inputs().clone(),
                    inputs: inputs.clone(),
                },
            )
            .map_err(RunCreateExecutionError::Invocation)?;
        classify_input_result(&result.result).map_err(RunCreateExecutionError::Operation)?;
        reject_observed_digest_drift(&described.fact.digest, &result.fact.digest)
            .map_err(RunCreateExecutionError::Operation)?;
        Some(result)
    } else {
        None
    };
    let digest_after = digests
        .executable_digest(&config)
        .map_err(RunCreateExecutionError::Digest)?;
    reject_observed_digest_drift(&digest_before, &digest_after)
        .map_err(RunCreateExecutionError::Operation)?;
    let projection = SemanticGraphProjection::from_validated(&validated);
    let revision = digests.graph_revision(&projection).map_err(|error| {
        RunCreateExecutionError::Operation(GraphError::CanonicalEncoding(error.to_string()).into())
    })?;
    let run = Run::create(
        ids.run_id().map_err(RunCreateExecutionError::Id)?,
        registration_id.clone(),
        validated,
        revision,
        inputs,
        label,
    )
    .map_err(|error| RunCreateExecutionError::Operation(error.into()))?;
    let creation_entry = journal(&config, &described, input_result.as_ref(), &run)
        .map_err(RunCreateExecutionError::Journal)?;
    let command = command(
        run,
        &config,
        &described,
        input_result.as_ref(),
        creation_entry,
    )
    .map_err(RunCreateExecutionError::Operation)?;
    writer
        .create(command)
        .map_err(RunCreateExecutionError::Writer)
}

pub fn command(
    run: Run,
    config: &ResolvedProviderConfig,
    described: &DescribeResult,
    input_result: Option<&InputValidationInvocationResult>,
    creation_entry: JournalDraft,
) -> Result<CreateRunCommand, RunCreateError> {
    if config.config_revision() == 0 {
        return Err(RunCreateError::InvalidProviderConfigRevision);
    }
    validate_journal(
        &creation_entry,
        run.id(),
        "run.create",
        JournalEntryKind::RunCreated,
    )?;
    if described.fact.role != ProviderRole::Describe
        || described.fact.outcome != OutcomeClass::Completed
        || described.fact.registration_id != *config.registration_id()
        || described.fact.config_revision != config.config_revision()
        || described.fact.executable.as_str() != config.config().executable()
        || described.fact.executable.as_str() != described.observation.locator()
        || described.fact.digest != *described.observation.digest()
        || described
            .fact
            .provider_version
            .as_ref()
            .map(|version| version.as_str())
            != described.observation.version()
        || described.fact.protocol_major != Some(u64::from(described.protocol_major))
    {
        return Err(CommandError::JournalMismatch.into());
    }
    let mut expected_facts = vec![described.fact.clone()];
    if let Some(result) = input_result {
        let expected_outcome = match result.result {
            InputValidationResult::Accepted => OutcomeClass::Completed,
            InputValidationResult::Rejected(_) => OutcomeClass::Rejected,
            InputValidationResult::EvaluationError(_) => OutcomeClass::Error,
        };
        if result.fact.role != ProviderRole::ValidateInputs
            || result.fact.outcome != expected_outcome
        {
            return Err(CommandError::JournalMismatch.into());
        }
        expected_facts.push(result.fact.clone());
    }
    if !matches!(
        creation_entry.extension(),
        JournalExtension::RunCreated { graph_revision } if graph_revision == run.graph_revision()
    ) || !creation_entry.attempt().is_some_and(|attempt| {
        attempt.provider_observations == expected_facts
            && attempt.provider_observations.iter().all(|fact| {
                fact.config_revision == config.config_revision()
                    && fact.registration_id == *run.registration_id()
                    && fact.executable.as_str() == config.config().executable()
            })
    }) {
        return Err(CommandError::JournalMismatch.into());
    }
    Ok(CreateRunCommand {
        run,
        expected_config_revision: config.config_revision(),
        creation_entry,
    })
}

#[cfg(test)]
mod tests {
    use super::{classify_input_result, needs_input_validation, reject_observed_digest_drift};
    use crate::capabilities::provider_invoker::InputValidationResult;
    use crate::model::ids::{InputKind, InputName};
    use crate::model::provider::DigestObservation;
    use crate::model::run_input::{InputDeclaration, InputDeclarations, RunInputs};

    #[test]
    fn skips_empty_inputs_and_classifies_input_and_digest_results() {
        assert!(!needs_input_validation(
            &InputDeclarations::default(),
            &RunInputs::default()
        ));
        let declarations = InputDeclarations::new(vec![InputDeclaration::new(
            InputName::parse("value").unwrap(),
            InputKind::parse("text").unwrap(),
            false,
            None,
        )])
        .unwrap();
        assert!(needs_input_validation(&declarations, &RunInputs::default()));
        assert!(classify_input_result(&InputValidationResult::Accepted).is_ok());
        assert!(
            classify_input_result(&InputValidationResult::Rejected(Default::default())).is_err()
        );
        let first = DigestObservation::observed(format!("sha256:{}", "a".repeat(64))).unwrap();
        let second = DigestObservation::observed(format!("sha256:{}", "b".repeat(64))).unwrap();
        assert!(reject_observed_digest_drift(&first, &second).is_err());
        assert!(reject_observed_digest_drift(&first, &first).is_ok());
    }
}
