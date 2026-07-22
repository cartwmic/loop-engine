//! Private provider command adapters (WP1 T122/T129).
//!
//! Delivery-layer syntax-to-core mapping and one-operation-per-intent adapters.
//! Rendering, route registration, and concrete integration construction live elsewhere.

use loop_engine_core::capabilities::Page;
use loop_engine_core::capabilities::digest::DigestComputer;
use loop_engine_core::capabilities::id_generator::IdGenerator;
use loop_engine_core::capabilities::provider_catalog::{
    ActiveRunImpact, CatalogCommandError, CatalogMutationResult, ProviderCatalog,
    ProviderCatalogRow, ProviderConfig, ProviderListFilter, ProviderResolveFailure,
    ResolvedProviderConfig,
};
use loop_engine_core::capabilities::provider_invoker::{
    CompatibilityRequest, CompatibilityResult, DescribeRequest, ProviderInvoker,
};
use loop_engine_core::capabilities::run_reader::RunReader;
use loop_engine_core::model::bounded::{BoundError, PROVIDER_TIMEOUT_SECONDS_DEFAULT};
use loop_engine_core::model::ids::{IdentifierError, ProviderHandle, RegistrationId, RequestId};
use loop_engine_core::model::provider::ProviderRegistration;
use loop_engine_core::operations::paging::PagingError;
use loop_engine_core::operations::provider_add::{self, ProviderAddExecutionError};
use loop_engine_core::operations::provider_check::{
    self, ProviderCheckExecution, ProviderCheckExecutionError, ProviderCheckMode,
};
use loop_engine_core::operations::provider_disable::{self, DisableWarningPage};
use loop_engine_core::operations::provider_list::{self, execute_impact, execute_list};
use loop_engine_core::operations::provider_rename;
use loop_engine_core::operations::provider_restore;
use loop_engine_core::operations::provider_update;
use loop_engine_integrations::persistence::DisableWarningsPage;
use thiserror::Error;

use crate::args::{
    SyntaxHandle, SyntaxIdentifier, SyntaxOpaqueWire, SyntaxPageLimit, SyntaxPath,
    SyntaxPositiveU64, SyntaxProviderArgv, SyntaxProviderTarget,
};

/// Bounded conversion failures at the CLI delivery boundary (pre-dispatch, not domain rejection).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderMapError {
    #[error(transparent)]
    Bound(#[from] BoundError),
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
    #[error(transparent)]
    Paging(#[from] PagingError),
    #[error(transparent)]
    CatalogCommand(#[from] CatalogCommandError),
}

/// Resolved provider target after syntax validation (handle vs stable registration ID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTargetRef {
    Handle(ProviderHandle),
    RegistrationId(RegistrationId),
}

pub fn map_target(target: &SyntaxProviderTarget) -> Result<ProviderTargetRef, ProviderMapError> {
    let value = target.as_str();
    if let Ok(handle) = ProviderHandle::parse(value) {
        return Ok(ProviderTargetRef::Handle(handle));
    }
    Ok(ProviderTargetRef::RegistrationId(RegistrationId::parse(
        value,
    )?))
}

/// Resolve a validated target through the authoritative catalog capability.
pub fn resolve_target<C: ProviderCatalog>(
    catalog: &C,
    target: &ProviderTargetRef,
) -> Result<ProviderCatalogRow, C::Error> {
    match target {
        ProviderTargetRef::Handle(handle) => match catalog.resolve_handle(handle) {
            Ok(row) => Ok(row),
            Err(error)
                if C::classify_resolve_failure(&error) == ProviderResolveFailure::Missing =>
            {
                let registration_id = RegistrationId::parse(handle.as_str())
                    .expect("validated provider handle is a valid opaque registration ID");
                catalog
                    .resolve_enabled(&registration_id)
                    .map(catalog_row_from_resolved)
            }
            Err(error) => Err(error),
        },
        ProviderTargetRef::RegistrationId(registration_id) => catalog
            .resolve_enabled(registration_id)
            .map(catalog_row_from_resolved),
    }
}

fn catalog_row_from_resolved(resolved: ResolvedProviderConfig) -> ProviderCatalogRow {
    let registration = ProviderRegistration::restore(
        resolved.registration_id().clone(),
        Some(resolved.handle().clone()),
        resolved.config_revision(),
        true,
    )
    .expect("resolve_enabled returns an enabled registration");
    ProviderCatalogRow {
        registration,
        config: Some(resolved.config().clone()),
    }
}

pub fn authoritative_config_revision(row: &ProviderCatalogRow) -> u64 {
    row.registration.config_revision()
}

fn map_provider_config(
    exec: &SyntaxPath,
    working_directory: &SyntaxPath,
    arg: &SyntaxProviderArgv,
    timeout: Option<&SyntaxPositiveU64>,
) -> Result<ProviderConfig, ProviderMapError> {
    let timeout_seconds = timeout
        .map(SyntaxPositiveU64::get)
        .unwrap_or(PROVIDER_TIMEOUT_SECONDS_DEFAULT);
    Ok(ProviderConfig::new(
        exec.as_str(),
        argv_elements(arg),
        working_directory.as_str(),
        timeout_seconds,
    )?)
}

fn argv_elements(arg: &SyntaxProviderArgv) -> Vec<String> {
    arg.elements
        .iter()
        .map(|element| element.as_str().to_string())
        .collect()
}

fn opaque_wire(cursor: &SyntaxOpaqueWire) -> String {
    cursor.as_str().to_string()
}

fn optional_opaque_wire(cursor: Option<&SyntaxOpaqueWire>) -> Option<String> {
    cursor.map(opaque_wire)
}

fn page_limit(limit: SyntaxPageLimit) -> u16 {
    limit.get()
}

pub fn list_filter(enabled: bool, tombstoned: bool) -> ProviderListFilter {
    match (enabled, tombstoned) {
        (true, true) => ProviderListFilter::All,
        (_, true) => ProviderListFilter::Tombstoned,
        (true, false) => ProviderListFilter::Enabled,
        (false, false) => ProviderListFilter::Enabled,
    }
}

// --- Request DTOs (core-aligned) ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAddRequest {
    pub handle: ProviderHandle,
    pub config: ProviderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderListRequest {
    Registrations {
        filter: ProviderListFilter,
        limit: u16,
        cursor: Option<String>,
    },
    ActiveRunImpact {
        registration_id: RegistrationId,
        limit: u16,
        cursor: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCheckRequest {
    pub registration_id: RegistrationId,
    pub describe_request: DescribeRequest,
    pub mode: ProviderCheckMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderUpdateRequest {
    pub registration_id: RegistrationId,
    pub expected_config_revision: u64,
    pub config: ProviderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRenameRequest {
    pub registration_id: RegistrationId,
    pub expected_config_revision: u64,
    pub handle: ProviderHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderDisableRequest {
    Warnings {
        registration_id: RegistrationId,
        limit: u16,
        warning_cursor: Option<String>,
    },
    Authorize {
        registration_id: RegistrationId,
        ack_token: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRestoreRequest {
    pub registration_id: RegistrationId,
    pub expected_config_revision: u64,
    pub handle: ProviderHandle,
    pub config: ProviderConfig,
}

pub fn map_add_request(
    handle: &SyntaxHandle,
    exec: &SyntaxPath,
    working_directory: &SyntaxPath,
    arg: &SyntaxProviderArgv,
    timeout: Option<&SyntaxPositiveU64>,
) -> Result<ProviderAddRequest, ProviderMapError> {
    Ok(ProviderAddRequest {
        handle: ProviderHandle::parse(handle.as_str())?,
        config: map_provider_config(exec, working_directory, arg, timeout)?,
    })
}

pub fn map_list_request(
    enabled: bool,
    tombstoned: bool,
    active_runs_for: Option<&SyntaxIdentifier>,
    cursor: Option<&SyntaxOpaqueWire>,
    limit: SyntaxPageLimit,
) -> Result<ProviderListRequest, ProviderMapError> {
    let limit = page_limit(limit);
    let cursor = optional_opaque_wire(cursor);
    if let Some(registration_id) = active_runs_for {
        return Ok(ProviderListRequest::ActiveRunImpact {
            registration_id: RegistrationId::parse(registration_id.as_str())?,
            limit,
            cursor,
        });
    }
    Ok(ProviderListRequest::Registrations {
        filter: list_filter(enabled, tombstoned),
        limit,
        cursor,
    })
}

pub fn map_check_request(
    registration_id: RegistrationId,
    request_id: RequestId,
    active_runs: bool,
    cursor: Option<&SyntaxOpaqueWire>,
    limit: SyntaxPageLimit,
) -> Result<ProviderCheckRequest, ProviderMapError> {
    let mode = if active_runs {
        ProviderCheckMode::ActiveRuns(provider_list::impact_request(
            Some(page_limit(limit)),
            optional_opaque_wire(cursor),
        )?)
    } else {
        ProviderCheckMode::ConformanceOnly
    };
    Ok(ProviderCheckRequest {
        registration_id,
        describe_request: DescribeRequest { request_id },
        mode,
    })
}

pub fn merge_update_config(
    current: &ProviderConfig,
    exec: &SyntaxPath,
    arg: &SyntaxProviderArgv,
    working_directory: Option<&SyntaxPath>,
    timeout: Option<&SyntaxPositiveU64>,
) -> Result<ProviderConfig, ProviderMapError> {
    let working_directory = working_directory
        .map(|path| path.as_str())
        .unwrap_or_else(|| current.working_directory());
    let timeout_seconds = timeout
        .map(SyntaxPositiveU64::get)
        .unwrap_or_else(|| current.timeout_seconds());
    Ok(ProviderConfig::new(
        exec.as_str(),
        argv_elements(arg),
        working_directory,
        timeout_seconds,
    )?)
}

pub fn map_update_request(
    registration_id: RegistrationId,
    expected_config_revision: u64,
    exec: &SyntaxPath,
    arg: &SyntaxProviderArgv,
    working_directory: Option<&SyntaxPath>,
    timeout: Option<&SyntaxPositiveU64>,
    current_config: &ProviderConfig,
) -> Result<ProviderUpdateRequest, ProviderMapError> {
    Ok(ProviderUpdateRequest {
        registration_id,
        expected_config_revision,
        config: merge_update_config(current_config, exec, arg, working_directory, timeout)?,
    })
}

pub fn map_rename_request(
    registration_id: RegistrationId,
    expected_config_revision: u64,
    new_handle: &SyntaxHandle,
) -> Result<ProviderRenameRequest, ProviderMapError> {
    Ok(ProviderRenameRequest {
        registration_id,
        expected_config_revision,
        handle: ProviderHandle::parse(new_handle.as_str())?,
    })
}

pub fn map_disable_request(
    registration_id: RegistrationId,
    warning_cursor: Option<&SyntaxOpaqueWire>,
    limit: SyntaxPageLimit,
    allow_active_runs: Option<&SyntaxOpaqueWire>,
) -> Result<ProviderDisableRequest, ProviderMapError> {
    if let Some(token) = allow_active_runs {
        return Ok(ProviderDisableRequest::Authorize {
            registration_id,
            ack_token: opaque_wire(token),
        });
    }
    Ok(ProviderDisableRequest::Warnings {
        registration_id,
        limit: page_limit(limit),
        warning_cursor: optional_opaque_wire(warning_cursor),
    })
}

pub fn map_restore_request(
    registration_id: &SyntaxIdentifier,
    handle: &SyntaxHandle,
    exec: &SyntaxPath,
    working_directory: &SyntaxPath,
    arg: &SyntaxProviderArgv,
    timeout: Option<&SyntaxPositiveU64>,
    expected_config_revision: u64,
) -> Result<ProviderRestoreRequest, ProviderMapError> {
    Ok(ProviderRestoreRequest {
        registration_id: RegistrationId::parse(registration_id.as_str())?,
        expected_config_revision,
        handle: ProviderHandle::parse(handle.as_str())?,
        config: map_provider_config(exec, working_directory, arg, timeout)?,
    })
}

// --- Renderable outcome DTOs ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegistrationView {
    pub id: String,
    pub handle: Option<String>,
    pub config_revision: u64,
    pub enabled: bool,
}

impl From<&ProviderRegistration> for ProviderRegistrationView {
    fn from(registration: &ProviderRegistration) -> Self {
        Self {
            id: registration.id().as_str().to_string(),
            handle: registration
                .handle()
                .map(|handle| handle.as_str().to_string()),
            config_revision: registration.config_revision(),
            enabled: registration.enabled(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfigView {
    pub executable: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub timeout_seconds: u64,
}

impl From<&ProviderConfig> for ProviderConfigView {
    fn from(config: &ProviderConfig) -> Self {
        Self {
            executable: config.executable().to_string(),
            argv: config
                .argv()
                .iter()
                .map(|element| element.as_str().to_string())
                .collect(),
            working_directory: config.working_directory().to_string(),
            timeout_seconds: config.timeout_seconds(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogRowView {
    pub registration: ProviderRegistrationView,
    pub config: Option<ProviderConfigView>,
}

impl From<&ProviderCatalogRow> for ProviderCatalogRowView {
    fn from(row: &ProviderCatalogRow) -> Self {
        Self {
            registration: ProviderRegistrationView::from(&row.registration),
            config: row.config.as_ref().map(ProviderConfigView::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogMutationOutcome {
    pub registration: ProviderRegistrationView,
    pub affected_active_runs: u64,
    pub impact_cursor: Option<String>,
}

impl From<CatalogMutationResult> for ProviderCatalogMutationOutcome {
    fn from(result: CatalogMutationResult) -> Self {
        Self {
            registration: ProviderRegistrationView::from(&result.registration),
            affected_active_runs: result.affected_active_runs,
            impact_cursor: result
                .impact_cursor
                .as_ref()
                .map(|cursor| cursor.as_str().to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderActiveRunImpactView {
    pub run_id: String,
    pub graph_revision: String,
}

impl From<&ActiveRunImpact> for ProviderActiveRunImpactView {
    fn from(impact: &ActiveRunImpact) -> Self {
        Self {
            run_id: impact.run_id.as_str().to_string(),
            graph_revision: impact.graph_revision.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderListOutcome {
    pub items: Vec<ProviderCatalogRowView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderActiveRunImpactListOutcome {
    pub items: Vec<ProviderActiveRunImpactView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDisableWarningsOutcome {
    pub items: Vec<ProviderActiveRunImpactView>,
    pub next_warning_cursor: Option<String>,
    pub ack_token: Option<String>,
    pub active_run_count: u64,
    pub config_revision: u64,
    pub active_set_digest: String,
}

impl From<DisableWarningsPage> for ProviderDisableWarningsOutcome {
    fn from(page: DisableWarningsPage) -> Self {
        Self {
            items: page
                .impacts
                .rows
                .iter()
                .map(ProviderActiveRunImpactView::from)
                .collect(),
            next_warning_cursor: page
                .impacts
                .next_cursor
                .as_ref()
                .map(|cursor| cursor.as_str().to_string()),
            ack_token: page
                .acknowledgement
                .as_ref()
                .map(|ack| ack.as_str().to_string()),
            active_run_count: page.snapshot.count(),
            config_revision: page.snapshot.config_revision(),
            active_set_digest: page.snapshot.digest().to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderListError<E> {
    #[error(transparent)]
    Paging(#[from] PagingError),
    #[error(transparent)]
    Catalog(E),
}

#[derive(Debug, Error)]
pub enum ProviderDisableAuthorizeError<C> {
    #[error("disable acknowledgement did not authorize mutation")]
    NotAuthorized,
    #[error(transparent)]
    Catalog(C),
}

#[derive(Debug, Error)]
pub enum ProviderCatalogMutationError<C> {
    #[error(transparent)]
    Command(#[from] CatalogCommandError),
    #[error(transparent)]
    Catalog(C),
}

// --- Adapters: exactly one core operation (or catalog read) each ---

pub fn add<I, C>(
    ids: &I,
    catalog: &C,
    request: ProviderAddRequest,
) -> Result<ProviderCatalogMutationOutcome, ProviderAddExecutionError<I::Error, C::Error>>
where
    I: IdGenerator,
    C: ProviderCatalog,
{
    provider_add::execute(ids, catalog, request.handle, request.config)
        .map(ProviderCatalogMutationOutcome::from)
}

pub fn list_registrations<C>(
    catalog: &C,
    filter: ProviderListFilter,
    limit: u16,
    cursor: Option<String>,
) -> Result<ProviderListOutcome, ProviderListError<C::Error>>
where
    C: ProviderCatalog,
{
    let page_request = provider_list::list_request(filter, Some(limit), cursor)?;
    let page = execute_list(catalog, &page_request).map_err(ProviderListError::Catalog)?;
    Ok(map_registration_page(page))
}

pub fn list_active_run_impact<C>(
    catalog: &C,
    registration_id: RegistrationId,
    limit: u16,
    cursor: Option<String>,
) -> Result<ProviderActiveRunImpactListOutcome, ProviderListError<C::Error>>
where
    C: ProviderCatalog,
{
    let page_request = provider_list::impact_request(Some(limit), cursor)?;
    let page = execute_impact(catalog, &registration_id, &page_request)
        .map_err(ProviderListError::Catalog)?;
    Ok(map_impact_page(page))
}

#[allow(
    clippy::type_complexity,
    reason = "preserves exact core capability errors"
)]
pub fn check<C, R, I, D, G, Z, Q, E>(
    catalog: &C,
    reader: &R,
    invoker: &I,
    digests: &D,
    request: ProviderCheckRequest,
    compatibility_request: G,
    row_encoded_size: Z,
) -> Result<
    ProviderCheckExecution,
    ProviderCheckExecutionError<C::Error, R::Error, I::TransportError, D::Error, Q, E>,
>
where
    C: ProviderCatalog,
    R: RunReader,
    I: ProviderInvoker,
    D: DigestComputer,
    D::Error: std::fmt::Display,
    G: FnMut(
        &ResolvedProviderConfig,
        &loop_engine_core::model::run::Run,
    ) -> Result<CompatibilityRequest, Q>,
    Z: FnMut(&ActiveRunImpact, &CompatibilityResult) -> Result<usize, E>,
{
    provider_check::execute(
        catalog,
        reader,
        invoker,
        digests,
        &request.registration_id,
        request.describe_request,
        &request.mode,
        compatibility_request,
        row_encoded_size,
    )
}

pub fn update<C>(
    catalog: &C,
    request: ProviderUpdateRequest,
) -> Result<ProviderCatalogMutationOutcome, ProviderCatalogMutationError<C::Error>>
where
    C: ProviderCatalog,
{
    let command = provider_update::command(
        request.registration_id,
        request.expected_config_revision,
        request.config,
    )?;
    provider_update::execute(catalog, command)
        .map(ProviderCatalogMutationOutcome::from)
        .map_err(ProviderCatalogMutationError::Catalog)
}

pub fn rename<C>(
    catalog: &C,
    request: ProviderRenameRequest,
) -> Result<ProviderCatalogMutationOutcome, ProviderCatalogMutationError<C::Error>>
where
    C: ProviderCatalog,
{
    let command = provider_rename::command(
        request.registration_id,
        request.expected_config_revision,
        request.handle,
    )?;
    provider_rename::execute(catalog, command)
        .map(ProviderCatalogMutationOutcome::from)
        .map_err(ProviderCatalogMutationError::Catalog)
}

/// Maps an integration-owned disable warning page; acknowledgement minting stays in persistence.
pub fn map_disable_warnings_outcome(page: DisableWarningsPage) -> ProviderDisableWarningsOutcome {
    page.into()
}

pub fn disable_authorize<C>(
    catalog: &C,
    registration_id: RegistrationId,
    page: DisableWarningPage,
) -> Result<ProviderCatalogMutationOutcome, ProviderDisableAuthorizeError<C::Error>>
where
    C: ProviderCatalog,
{
    let Some(command) = provider_disable::command(registration_id, page) else {
        return Err(ProviderDisableAuthorizeError::NotAuthorized);
    };
    provider_disable::execute(catalog, command)
        .map(ProviderCatalogMutationOutcome::from)
        .map_err(ProviderDisableAuthorizeError::Catalog)
}

pub fn restore<C>(
    catalog: &C,
    request: ProviderRestoreRequest,
) -> Result<ProviderCatalogMutationOutcome, ProviderCatalogMutationError<C::Error>>
where
    C: ProviderCatalog,
{
    let command = provider_restore::command(
        request.registration_id,
        request.expected_config_revision,
        request.handle,
        request.config,
    )?;
    provider_restore::execute(catalog, command)
        .map(ProviderCatalogMutationOutcome::from)
        .map_err(ProviderCatalogMutationError::Catalog)
}

fn map_registration_page(page: Page<ProviderCatalogRow>) -> ProviderListOutcome {
    ProviderListOutcome {
        items: page.rows.iter().map(ProviderCatalogRowView::from).collect(),
        next_cursor: page
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_string()),
    }
}

fn map_impact_page(page: Page<ActiveRunImpact>) -> ProviderActiveRunImpactListOutcome {
    ProviderActiveRunImpactListOutcome {
        items: page
            .rows
            .iter()
            .map(ProviderActiveRunImpactView::from)
            .collect(),
        next_cursor: page
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use loop_engine_core::capabilities::provider_catalog::{
        ActiveRunImpact, ActiveSetSnapshot, CatalogMutation, CatalogMutationResult,
        ProviderCatalog, ProviderCatalogRow, ProviderConfig, ProviderListFilter,
        ProviderResolveFailure, ResolvedProviderConfig,
    };
    use loop_engine_core::capabilities::{Page, PageRequest};
    use loop_engine_core::model::ids::{ProviderHandle, RegistrationId};

    use super::{ProviderTargetRef, list_filter, resolve_target};

    #[derive(Debug)]
    enum ResolveError {
        Missing,
    }

    struct RegistrationOnlyCatalog {
        registration_id: RegistrationId,
        config: ProviderConfig,
    }

    impl ProviderCatalog for RegistrationOnlyCatalog {
        type Error = ResolveError;

        fn classify_resolve_failure(_error: &Self::Error) -> ProviderResolveFailure {
            ProviderResolveFailure::Missing
        }

        fn resolve_enabled(
            &self,
            registration_id: &RegistrationId,
        ) -> Result<ResolvedProviderConfig, Self::Error> {
            assert_eq!(registration_id, &self.registration_id);
            Ok(ResolvedProviderConfig::new(
                self.registration_id.clone(),
                ProviderHandle::parse("demo").unwrap(),
                1,
                self.config.clone(),
            )
            .unwrap())
        }

        fn resolve_handle(
            &self,
            _handle: &ProviderHandle,
        ) -> Result<ProviderCatalogRow, Self::Error> {
            Err(ResolveError::Missing)
        }

        fn list(
            &self,
            _request: &PageRequest<ProviderListFilter>,
        ) -> Result<Page<ProviderCatalogRow>, Self::Error> {
            unreachable!()
        }

        fn active_run_impact(
            &self,
            _registration_id: &RegistrationId,
            _request: &PageRequest<()>,
        ) -> Result<Page<ActiveRunImpact>, Self::Error> {
            unreachable!()
        }

        fn active_set_snapshot(
            &self,
            _registration_id: &RegistrationId,
        ) -> Result<ActiveSetSnapshot, Self::Error> {
            unreachable!()
        }

        fn mutate(&self, _command: CatalogMutation) -> Result<CatalogMutationResult, Self::Error> {
            unreachable!()
        }
    }

    #[test]
    fn list_filter_matches_catalog_facet_flags() {
        assert_eq!(list_filter(false, false), ProviderListFilter::Enabled);
        assert_eq!(list_filter(true, false), ProviderListFilter::Enabled);
        assert_eq!(list_filter(false, true), ProviderListFilter::Tombstoned);
        assert_eq!(list_filter(true, true), ProviderListFilter::All);
    }

    #[test]
    fn syntactically_handle_like_registration_id_falls_back_to_stable_id() {
        let registration_id =
            RegistrationId::parse("019c7658-7146-7d51-928f-d62ad381b73f").unwrap();
        let catalog = RegistrationOnlyCatalog {
            registration_id: registration_id.clone(),
            config: ProviderConfig::new("/bin/provider", vec![], "/tmp", 1).unwrap(),
        };
        let target =
            ProviderTargetRef::Handle(ProviderHandle::parse(registration_id.as_str()).unwrap());

        let row = resolve_target(&catalog, &target).unwrap();

        assert_eq!(row.registration.id(), &registration_id);
    }
}
