use crate::capabilities::digest::DigestComputer;
use crate::capabilities::provider_catalog::{
    ActiveRunImpact, ProviderCatalog, ResolvedProviderConfig,
};
use crate::capabilities::provider_invoker::{
    CompatibilityRequest, DescribeRequest, DescribeResult, DescribedGraph, InvocationError,
    ProviderInvoker,
};
use crate::capabilities::run_reader::RunReader;
use crate::capabilities::{Page, PageCursor, PageRequest};
use crate::model::attempt::ProviderRole;
use crate::model::bounded::{
    COLLECTION_PAGE_DATA_BUDGET_BYTES, PROVIDER_CALLS_PER_PAGED_INVOCATION_MAX,
};
use crate::model::compatibility::CompatibilityReport;
use crate::model::diagnostic::Diagnostics;
use crate::model::graph::WorkflowGraph;
use crate::model::graph_projection::SemanticGraphProjection;
use crate::model::graph_validation::{GraphError, ValidatedGraph};
use crate::model::ids::{GraphRevision, RunId};
use crate::model::outcome::OutcomeClass;
use crate::model::provider::ProviderObservation;

pub const PROVIDER_CALLS_PER_PAGE_MAX: usize = PROVIDER_CALLS_PER_PAGED_INVOCATION_MAX;
pub const COMPATIBILITY_CALLS_PER_PAGE_MAX: usize = PROVIDER_CALLS_PER_PAGE_MAX - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCheckMode {
    ConformanceOnly,
    ActiveRuns(PageRequest<()>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedRun {
    pub impact: ActiveRunImpact,
    pub report: CompatibilityReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphConformance {
    Valid {
        revision: GraphRevision,
    },
    /// Invalid declarations are a completed provider-check finding, not a transport error.
    Invalid {
        error: GraphError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCheckSummary {
    pub config_revision: u64,
    pub protocol_major: u32,
    pub graph_conformance: GraphConformance,
    pub observation: ProviderObservation,
}

/// Integration mints an authenticated cursor from `AfterRun`; core never invents wire tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCheckContinuation {
    SourceCursor(Option<PageCursor>),
    AfterRun(RunId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCheckPage {
    pub summary: ProviderCheckSummary,
    pub rows: Vec<CheckedRun>,
    pub continuation: ProviderCheckContinuation,
    pub provider_calls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceBudgetDisposition {
    BeforeFirstRow {
        unchanged_cursor: Option<PageCursor>,
    },
    AfterProgress(Box<ProviderCheckPage>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCheckExecution {
    Completed(Box<ProviderCheckPage>),
    TraceBudget(TraceBudgetDisposition),
}

#[derive(Debug)]
pub enum ProviderCheckExecutionError<C, R, I, D, Q, Z> {
    Catalog(C),
    Reader(R),
    Describe(InvocationError<I>),
    Compatibility(InvocationError<I>),
    CompatibilityEvaluation(Diagnostics),
    Digest(D),
    Request(Q),
    RowSize(Z),
    RowTooLarge,
    InvalidPlan,
}

type ProviderCheckResult<C, R, I, D, Q, Z> =
    Result<ProviderCheckExecution, ProviderCheckExecutionError<C, R, I, D, Q, Z>>;

#[allow(clippy::too_many_arguments)]
pub fn execute<C, R, I, D, G, Z, Q, E>(
    catalog: &C,
    reader: &R,
    invoker: &I,
    digests: &D,
    registration_id: &crate::model::ids::RegistrationId,
    describe_request: DescribeRequest,
    mode: &ProviderCheckMode,
    mut compatibility_request: G,
    mut row_encoded_size: Z,
) -> ProviderCheckResult<C::Error, R::Error, I::TransportError, D::Error, Q, E>
where
    C: ProviderCatalog,
    R: RunReader,
    I: ProviderInvoker,
    D: DigestComputer,
    D::Error: std::fmt::Display,
    G: FnMut(&ResolvedProviderConfig, &crate::model::run::Run) -> Result<CompatibilityRequest, Q>,
    Z: FnMut(
        &ActiveRunImpact,
        &crate::capabilities::provider_invoker::CompatibilityResult,
    ) -> Result<usize, E>,
{
    let impact_request = match mode {
        ProviderCheckMode::ConformanceOnly => None,
        ProviderCheckMode::ActiveRuns(request) => Some(request),
    };
    let original_cursor = impact_request.and_then(|request| request.cursor().cloned());
    let config = catalog
        .resolve_enabled(registration_id)
        .map_err(ProviderCheckExecutionError::Catalog)?;
    let described = match invoker.describe(&config, describe_request) {
        Ok(described) => described,
        Err(InvocationError::TraceBudgetUnavailable) => {
            return Ok(ProviderCheckExecution::TraceBudget(
                TraceBudgetDisposition::BeforeFirstRow {
                    unchanged_cursor: original_cursor,
                },
            ));
        }
        Err(error) => return Err(ProviderCheckExecutionError::Describe(error)),
    };
    if !describe_binding_matches(&config, &described) {
        return Err(ProviderCheckExecutionError::InvalidPlan);
    }
    let graph_conformance = match described.graph.clone() {
        DescribedGraph::Declared(graph) => classify_graph_with_digest(graph, digests),
        DescribedGraph::Invalid(error) => GraphConformance::Invalid { error },
    };
    let summary = ProviderCheckSummary {
        config_revision: config.config_revision(),
        protocol_major: described.protocol_major,
        graph_conformance,
        observation: described.observation.clone(),
    };
    let Some(impact_request) = impact_request else {
        return Ok(ProviderCheckExecution::Completed(Box::new(
            ProviderCheckPage {
                summary,
                rows: vec![],
                continuation: ProviderCheckContinuation::SourceCursor(None),
                provider_calls: 1,
            },
        )));
    };
    let bounded_impact_request = compatibility_page_request(impact_request)
        .map_err(|_| ProviderCheckExecutionError::InvalidPlan)?;
    let impacts = catalog
        .active_run_impact("provider.check", registration_id, &bounded_impact_request)
        .map_err(ProviderCheckExecutionError::Catalog)?;
    if impacts.rows.len() > COMPATIBILITY_CALLS_PER_PAGE_MAX {
        return Err(ProviderCheckExecutionError::InvalidPlan);
    }
    if impacts.rows.is_empty() {
        return aggregate_page(summary, impacts, vec![])
            .map(|page| ProviderCheckExecution::Completed(Box::new(page)))
            .ok_or(ProviderCheckExecutionError::InvalidPlan);
    }
    let mut reports = Vec::new();
    let mut page_bytes = 0usize;
    let mut compatibility_calls = 0usize;
    for impact in &impacts.rows {
        let run = reader
            .get(&impact.run_id)
            .map_err(ProviderCheckExecutionError::Reader)?;
        let request =
            compatibility_request(&config, &run).map_err(ProviderCheckExecutionError::Request)?;
        match invoker.check_compatibility(&config, request) {
            Ok(result) if compatibility_result_matches(&config, &result) => {
                compatibility_calls += 1;
                if let CompatibilityReport::EvaluationError(diagnostics) = &result.report {
                    return Err(ProviderCheckExecutionError::CompatibilityEvaluation(
                        diagnostics.clone(),
                    ));
                }
                let encoded_bytes = row_encoded_size(impact, &result)
                    .map_err(ProviderCheckExecutionError::RowSize)?;
                if encoded_bytes == 0 {
                    return Err(ProviderCheckExecutionError::InvalidPlan);
                }
                if encoded_bytes > impact_request.byte_limit() {
                    if reports.is_empty() {
                        return Err(ProviderCheckExecutionError::RowTooLarge);
                    }
                    break;
                }
                if page_bytes.saturating_add(encoded_bytes) > impact_request.byte_limit() {
                    break;
                }
                page_bytes += encoded_bytes;
                reports.push((result.report, encoded_bytes));
            }
            Ok(_) => return Err(ProviderCheckExecutionError::InvalidPlan),
            Err(InvocationError::TraceBudgetUnavailable) => {
                let completed = if reports.is_empty() {
                    None
                } else {
                    aggregate_page(summary, impacts, reports)
                };
                return Ok(ProviderCheckExecution::TraceBudget(trace_budget_failure(
                    original_cursor,
                    completed,
                )));
            }
            Err(error) => return Err(ProviderCheckExecutionError::Compatibility(error)),
        }
    }
    aggregate_page(summary, impacts, reports)
        .map(|mut page| {
            page.provider_calls = 1 + compatibility_calls;
            ProviderCheckExecution::Completed(Box::new(page))
        })
        .ok_or(ProviderCheckExecutionError::InvalidPlan)
}

fn compatibility_result_matches(
    config: &ResolvedProviderConfig,
    result: &crate::capabilities::provider_invoker::CompatibilityResult,
) -> bool {
    let expected_outcome = match &result.report {
        CompatibilityReport::Findings(_) => OutcomeClass::Completed,
        CompatibilityReport::EvaluationError(_) => OutcomeClass::Error,
    };
    result.protocol_major > 0
        && result.fact.role == ProviderRole::CheckCompatibility
        && result.fact.outcome == expected_outcome
        && result.fact.registration_id == *config.registration_id()
        && result.fact.config_revision == config.config_revision()
        && result.fact.executable.as_str() == config.config().executable()
        && result.fact.executable.as_str() == result.observation.locator()
        && result.fact.digest == *result.observation.digest()
        && result
            .fact
            .provider_version
            .as_ref()
            .map(|version| version.as_str())
            == result.observation.version()
        && result.fact.protocol_major == Some(u64::from(result.protocol_major))
}

fn compatibility_page_request(request: &PageRequest<()>) -> Result<PageRequest<()>, ()> {
    PageRequest::new(
        request
            .limit()
            .min(u16::try_from(COMPATIBILITY_CALLS_PER_PAGE_MAX).map_err(|_| ())?),
        request.byte_limit(),
        request.cursor().cloned(),
        (),
    )
    .map_err(|_| ())
}

fn describe_binding_matches(config: &ResolvedProviderConfig, described: &DescribeResult) -> bool {
    described.protocol_major > 0
        && described.fact.role == ProviderRole::Describe
        && described.fact.outcome == OutcomeClass::Completed
        && described.fact.registration_id == *config.registration_id()
        && described.fact.config_revision == config.config_revision()
        && described.fact.executable.as_str() == config.config().executable()
        && described.fact.executable.as_str() == described.observation.locator()
        && described.fact.digest == *described.observation.digest()
        && described
            .fact
            .provider_version
            .as_ref()
            .map(|version| version.as_str())
            == described.observation.version()
        && described.fact.protocol_major == Some(u64::from(described.protocol_major))
}

fn classify_graph_with_digest<D>(graph: WorkflowGraph, digests: &D) -> GraphConformance
where
    D: DigestComputer,
    D::Error: std::fmt::Display,
{
    match ValidatedGraph::validate(graph) {
        Ok(validated) => {
            match digests.graph_revision(&SemanticGraphProjection::from_validated(&validated)) {
                Ok(revision) => GraphConformance::Valid { revision },
                Err(error) => GraphConformance::Invalid {
                    error: GraphError::CanonicalEncoding(error.to_string()),
                },
            }
        }
        Err(error) => GraphConformance::Invalid { error },
    }
}

pub fn classify_graph(graph: WorkflowGraph, valid_revision: GraphRevision) -> GraphConformance {
    match ValidatedGraph::validate(graph) {
        Ok(_) => GraphConformance::Valid {
            revision: valid_revision,
        },
        Err(error) => GraphConformance::Invalid { error },
    }
}

/// Builds either a full source page or a safe partial page ending after the last completed call.
pub fn aggregate_page(
    summary: ProviderCheckSummary,
    impacts: Page<ActiveRunImpact>,
    reports: Vec<(CompatibilityReport, usize)>,
) -> Option<ProviderCheckPage> {
    if impacts.rows.len() > COMPATIBILITY_CALLS_PER_PAGE_MAX
        || reports.len() > impacts.rows.len()
        || reports.len() > COMPATIBILITY_CALLS_PER_PAGE_MAX
        || reports.iter().any(|(report, encoded_bytes)| {
            *encoded_bytes == 0 || matches!(report, CompatibilityReport::EvaluationError(_))
        })
        || reports
            .iter()
            .try_fold(0usize, |total, (_, size)| total.checked_add(*size))
            .is_none_or(|total| total > COLLECTION_PAGE_DATA_BUDGET_BYTES)
        || (impacts.rows.is_empty() && impacts.next_cursor.is_some())
        || (!impacts.rows.is_empty() && reports.is_empty())
    {
        return None;
    }
    let all_fetched_rows_completed = impacts.rows.len() == reports.len();
    let rows = impacts
        .rows
        .into_iter()
        .zip(reports)
        .map(|(impact, (report, _))| CheckedRun { impact, report })
        .collect::<Vec<_>>();
    let continuation = if all_fetched_rows_completed {
        ProviderCheckContinuation::SourceCursor(impacts.next_cursor)
    } else {
        ProviderCheckContinuation::AfterRun(rows.last()?.impact.run_id.clone())
    };
    let provider_calls = 1 + rows.len();
    Some(ProviderCheckPage {
        summary,
        rows,
        continuation,
        provider_calls,
    })
}

pub fn trace_budget_failure(
    original_cursor: Option<PageCursor>,
    completed: Option<ProviderCheckPage>,
) -> TraceBudgetDisposition {
    match completed {
        Some(page) if !page.rows.is_empty() => {
            TraceBudgetDisposition::AfterProgress(Box::new(page))
        }
        _ => TraceBudgetDisposition::BeforeFirstRow {
            unchanged_cursor: original_cursor,
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::capabilities::Page;
    use crate::capabilities::digest::DigestComputer;
    use crate::capabilities::provider_catalog::{ActiveRunImpact, ResolvedProviderConfig};
    use crate::model::compatibility::{
        CompatibilityFinding, CompatibilityReport, CompatibilityStatus,
    };
    use crate::model::graph::{State, WorkflowGraph};
    use crate::model::graph_projection::SemanticGraphProjection;
    use crate::model::guidance::{LiveGuidanceCapability, StaticGuidance};
    use crate::model::ids::{GraphRevision, RegistrationId, RunId, StateId};
    use crate::model::provider::{DigestObservation, ProviderObservation};
    use crate::model::run_input::InputDeclarations;
    use crate::model::time::ObservedAt;

    use super::{
        GraphConformance, ProviderCheckContinuation, ProviderCheckSummary, TraceBudgetDisposition,
        aggregate_page, classify_graph, classify_graph_with_digest, trace_budget_failure,
    };

    fn summary() -> ProviderCheckSummary {
        ProviderCheckSummary {
            config_revision: 1,
            protocol_major: 1,
            graph_conformance: super::GraphConformance::Valid {
                revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
            },
            observation: ProviderObservation::new(
                RegistrationId::parse("registration").unwrap(),
                "/provider",
                DigestObservation::Unavailable,
                None,
                ObservedAt::parse("2026-07-18T00:00:00Z").unwrap(),
            )
            .unwrap(),
        }
    }

    fn impact(id: &str) -> ActiveRunImpact {
        ActiveRunImpact {
            run_id: RunId::parse(id).unwrap(),
            graph_revision: GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        }
    }

    fn report(status: CompatibilityStatus) -> CompatibilityReport {
        CompatibilityReport::findings(vec![
            CompatibilityFinding::new("gates", status, vec![]).unwrap(),
        ])
        .unwrap()
    }

    struct FailingGraphDigest;

    impl DigestComputer for FailingGraphDigest {
        type Error = &'static str;

        fn graph_revision(
            &self,
            _graph: &SemanticGraphProjection,
        ) -> Result<GraphRevision, Self::Error> {
            Err("canonical graph exceeds bound")
        }

        fn executable_digest(
            &self,
            _config: &ResolvedProviderConfig,
        ) -> Result<DigestObservation, Self::Error> {
            unreachable!()
        }
    }

    #[test]
    fn canonical_encoding_failure_is_completed_conformance_finding() {
        let state = StateId::parse("a").unwrap();
        let graph = WorkflowGraph::new_unvalidated(
            state.clone(),
            vec![State::new(state, false, StaticGuidance::NoneRequired, None)],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        assert!(matches!(
            classify_graph_with_digest(graph, &FailingGraphDigest),
            GraphConformance::Invalid {
                error: crate::model::graph_validation::GraphError::CanonicalEncoding(_)
            }
        ));
    }

    #[test]
    fn invalid_graph_is_completed_conformance_finding() {
        let graph = WorkflowGraph::new_unvalidated(
            StateId::parse("missing").unwrap(),
            vec![],
            vec![],
            InputDeclarations::default(),
            LiveGuidanceCapability::Unsupported,
            None,
        );
        assert!(matches!(
            classify_graph(
                graph,
                GraphRevision::parse(format!("sha256:{}", "a".repeat(64))).unwrap()
            ),
            GraphConformance::Invalid { .. }
        ));
    }

    #[test]
    fn legal_large_page_is_capped_to_remaining_provider_call_budget() {
        let request = crate::capabilities::PageRequest::new(1_000, 8_192, None, ()).unwrap();
        let bounded = super::compatibility_page_request(&request).unwrap();
        assert_eq!(bounded.limit(), 9);
        assert_eq!(bounded.byte_limit(), 8_192);
    }

    #[test]
    fn zero_page_uses_only_describe_call() {
        let page = aggregate_page(
            summary(),
            Page {
                rows: vec![],
                next_cursor: None,
            },
            vec![],
        )
        .unwrap();
        assert_eq!(page.provider_calls, 1);
    }

    #[test]
    fn oversized_row_cannot_be_aggregated_or_truncated() {
        assert!(
            aggregate_page(
                summary(),
                Page {
                    rows: vec![impact("one")],
                    next_cursor: None,
                },
                vec![(
                    report(CompatibilityStatus::Compatible),
                    crate::model::bounded::COLLECTION_PAGE_DATA_BUDGET_BYTES + 1,
                )],
            )
            .is_none()
        );
    }

    #[test]
    fn evaluation_error_cannot_be_aggregated_as_completed_row() {
        let evaluation_error = CompatibilityReport::evaluation_error(vec![]).unwrap();
        assert!(
            aggregate_page(
                summary(),
                Page {
                    rows: vec![impact("one")],
                    next_cursor: None,
                },
                vec![(evaluation_error, 100)],
            )
            .is_none()
        );
    }

    #[test]
    fn mixed_page_is_non_latching_and_trace_budget_preserves_progress() {
        let page = aggregate_page(
            summary(),
            Page {
                rows: vec![impact("one"), impact("two")],
                next_cursor: None,
            },
            vec![
                (report(CompatibilityStatus::Compatible), 100),
                (report(CompatibilityStatus::Incompatible), 100),
            ],
        )
        .unwrap();
        assert_eq!(page.provider_calls, 3);
        assert!(matches!(
            trace_budget_failure(None, Some(page)),
            TraceBudgetDisposition::AfterProgress(_)
        ));
    }

    #[test]
    fn partial_progress_continues_after_last_completed_run() {
        let page = aggregate_page(
            summary(),
            Page {
                rows: vec![impact("one"), impact("two")],
                next_cursor: None,
            },
            vec![(report(CompatibilityStatus::Compatible), 100)],
        )
        .unwrap();
        assert!(matches!(
            page.continuation,
            ProviderCheckContinuation::AfterRun(ref id) if id.as_str() == "one"
        ));
    }
}
