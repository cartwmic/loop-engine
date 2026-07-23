//! Sole CLI composition root (T123).
//!
//! Constructs concrete configuration loading, SQLite persistence, provider process/protocol
//! adapters, system clock, UUID identifiers, SHA-256 digests, and traced persistence/provider
//! boundaries for private operations. Accepts the operational trace initialized in startup;
//! it must not create a second trace file for the same invocation.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use loop_engine_integrations::configuration::{
    CliDefaults, ConfigurationDto, ConfigurationError, MachinePaths, ResolvedDefaults,
    discover_project_config, load_optional, resolve_defaults,
};
use loop_engine_integrations::export::SqliteAuditExporter;
use loop_engine_integrations::persistence::{
    CompatibilityAttemptWriter, GuidanceAttemptWriter, OptionalTraceSink, PersistenceError,
    SqliteEventAttemptWriter, SqliteEvidenceReads, SqliteHistoryReads, SqliteProviderCatalog,
    SqliteRunMutations, SqliteRunReads, SqliteRunRequestReader, SqliteRunWriter, SqliteStore,
};
use loop_engine_integrations::provider_protocol::SubprocessProviderInvoker;
use loop_engine_integrations::sha256_digest::Sha256DigestComputer;
use loop_engine_integrations::system_clock::SystemTimeSource;
use loop_engine_integrations::trace::TraceWriter;
use loop_engine_integrations::uuid_ids::UuidV7Generator;
use thiserror::Error;

/// Correlation for one invocation trace file already opened by startup.
#[derive(Clone)]
pub struct TraceCorrelation {
    request_id: String,
    writer: Arc<Mutex<TraceWriter>>,
}

impl TraceCorrelation {
    /// Adopts the writer created before dispatch; does not call [`TraceWriter::create`].
    pub fn adopt(writer: TraceWriter) -> Self {
        let request_id = writer.request_id().to_owned();
        Self {
            request_id,
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn writer(&self) -> Arc<Mutex<TraceWriter>> {
        Arc::clone(&self.writer)
    }

    pub fn try_into_writer(self) -> Result<TraceWriter, Self> {
        let Self { request_id, writer } = self;
        match Arc::try_unwrap(writer) {
            Ok(mutex) => Ok(mutex
                .into_inner()
                .unwrap_or_else(|poison| poison.into_inner())),
            Err(writer) => Err(Self { request_id, writer }),
        }
    }

    pub fn persistence_trace(&self) -> OptionalTraceSink {
        OptionalTraceSink::from_arc(Arc::clone(&self.writer))
    }
}

/// Machine-local paths plus loaded optional config layers and resolved defaults.
#[derive(Debug, Clone)]
pub struct LoadedConfiguration {
    pub paths: MachinePaths,
    pub caller_cwd: PathBuf,
    pub global: Option<ConfigurationDto>,
    pub project: Option<ConfigurationDto>,
    pub defaults: ResolvedDefaults,
}

pub fn load_configuration(
    paths: &MachinePaths,
    cli_defaults: &CliDefaults,
) -> Result<LoadedConfiguration, ConfigurationError> {
    let global = load_optional(&paths.global_config)?;
    let caller_cwd = std::env::current_dir().map_err(ConfigurationError::CurrentDirectory)?;
    let project = match discover_project_config(&caller_cwd)? {
        Some(project_path) => load_optional(&project_path)?,
        None => None,
    };
    let defaults = resolve_defaults(cli_defaults, project.as_ref(), global.as_ref());
    Ok(LoadedConfiguration {
        paths: paths.clone(),
        caller_cwd,
        global,
        project,
        defaults,
    })
}

/// Explicit dependency graph for private operation dispatch.
pub struct Application {
    pub trace: TraceCorrelation,
    pub configuration: LoadedConfiguration,
    pub ids: UuidV7Generator,
    pub clock: SystemTimeSource,
    pub digests: Sha256DigestComputer,
    pub catalog: SqliteProviderCatalog,
    pub invoker: SubprocessProviderInvoker,
    pub run_create: SqliteRunWriter,
    pub run_mutations: SqliteRunMutations,
    pub run_reads: SqliteRunReads,
    pub event_attempts: SqliteEventAttemptWriter,
    pub guidance: GuidanceAttemptWriter,
    pub compatibility: CompatibilityAttemptWriter,
    pub evidence_reads: SqliteEvidenceReads,
    pub run_request_reads: SqliteRunRequestReader,
    pub history: SqliteHistoryReads,
    pub exporter: SqliteAuditExporter,
    store: SqliteStore,
}

#[derive(Debug, Error)]
pub enum ApplicationBuildError {
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
}

pub fn build_application(
    paths: MachinePaths,
    trace: TraceCorrelation,
    cli_defaults: CliDefaults,
) -> Result<Application, ApplicationBuildError> {
    let configuration = load_configuration(&paths, &cli_defaults)?;
    build_application_from_configuration(configuration, trace)
}

pub fn build_application_from_configuration(
    configuration: LoadedConfiguration,
    trace: TraceCorrelation,
) -> Result<Application, ApplicationBuildError> {
    let persistence_trace = trace.persistence_trace();
    let database_path = configuration.paths.database.clone();
    let store = SqliteStore::open_traced(&database_path, persistence_trace.clone())?;
    let adapters = persistence_adapters(&database_path, persistence_trace.clone());
    let invoker = SubprocessProviderInvoker::new(trace.writer());
    let run_request_reads =
        SqliteRunRequestReader::new(adapters.run_reads.clone(), adapters.evidence_reads.clone());
    Ok(Application {
        trace,
        configuration,
        ids: UuidV7Generator,
        clock: SystemTimeSource,
        digests: Sha256DigestComputer,
        catalog: adapters.catalog,
        invoker,
        run_create: adapters.run_create,
        run_mutations: adapters.run_mutations,
        run_reads: adapters.run_reads,
        event_attempts: adapters.event_attempts,
        guidance: adapters.guidance,
        compatibility: adapters.compatibility,
        evidence_reads: adapters.evidence_reads,
        run_request_reads,
        history: adapters.history,
        exporter: SqliteAuditExporter::with_trace(database_path, persistence_trace),
        store,
    })
}

struct PersistenceAdapters {
    catalog: SqliteProviderCatalog,
    run_create: SqliteRunWriter,
    run_mutations: SqliteRunMutations,
    run_reads: SqliteRunReads,
    event_attempts: SqliteEventAttemptWriter,
    guidance: GuidanceAttemptWriter,
    compatibility: CompatibilityAttemptWriter,
    evidence_reads: SqliteEvidenceReads,
    history: SqliteHistoryReads,
}

fn persistence_adapters(database_path: &Path, trace: OptionalTraceSink) -> PersistenceAdapters {
    PersistenceAdapters {
        catalog: SqliteProviderCatalog::with_trace(database_path, trace.clone()),
        run_create: SqliteRunWriter::with_trace(database_path, trace.clone()),
        run_mutations: SqliteRunMutations::with_trace(database_path, trace.clone()),
        run_reads: SqliteRunReads::with_trace(database_path, trace.clone()),
        event_attempts: SqliteEventAttemptWriter::with_trace(database_path, trace.clone()),
        guidance: GuidanceAttemptWriter::with_trace(database_path, trace.clone()),
        compatibility: CompatibilityAttemptWriter::with_trace(database_path, trace.clone()),
        evidence_reads: SqliteEvidenceReads::with_trace(database_path, trace.clone()),
        history: SqliteHistoryReads::with_trace(database_path, trace),
    }
}

impl Application {
    pub fn database_path(&self) -> &Path {
        self.store.path()
    }
}
