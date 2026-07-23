//! Real-adapter composition tests (T123): temp machine paths, config layers, and
//! startup-adopted trace writer. No application routes or exposure wiring.

#[path = "../src/composition.rs"]
mod composition;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use composition::{TraceCorrelation, build_application, load_configuration};
use loop_engine_integrations::configuration::{
    CliDefaults, EnvironmentPaths, MachinePaths, OutputFormat,
};
use loop_engine_integrations::persistence::{
    LogicalAuthoritySnapshot, SUPPORTED_SCHEMA_VERSION, connect_with_pragmas,
};
use loop_engine_integrations::trace::TraceWriter;

const REQUEST_ID: &str = "01J9X3K2M4N5P6Q7R8S9T0V1WX";
static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

struct CurrentDirGuard {
    original: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CurrentDirGuard {
    fn enter(path: &Path) -> Self {
        let lock = CURRENT_DIR_LOCK
            .lock()
            .expect("current-directory test lock");
        let original = std::env::current_dir().expect("current directory");
        std::env::set_current_dir(path).expect("set current directory");
        Self {
            original,
            _lock: lock,
        }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).expect("restore current directory");
    }
}

fn lock_current_dir() -> MutexGuard<'static, ()> {
    CURRENT_DIR_LOCK
        .lock()
        .expect("current-directory test lock")
}

fn isolated_paths() -> (tempfile::TempDir, MachinePaths) {
    let home = tempfile::tempdir().expect("temp home");
    let paths = MachinePaths::resolve(&EnvironmentPaths {
        home: None,
        loop_engine_home: Some(home.path().as_os_str().to_owned()),
        xdg_config_home: None,
        xdg_state_home: None,
    })
    .expect("machine paths");
    (home, paths)
}

fn adopt_trace(traces_dir: &Path) -> TraceCorrelation {
    let writer = TraceWriter::create(traces_dir, REQUEST_ID).expect("trace writer");
    TraceCorrelation::adopt(writer)
}

fn count_trace_files(traces_dir: &Path) -> usize {
    fs::read_dir(traces_dir)
        .expect("trace directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .count()
}

#[test]
fn adopted_trace_can_return_sole_writer() {
    let (_home, paths) = isolated_paths();
    let trace = adopt_trace(&paths.traces);
    let writer = match trace.try_into_writer() {
        Ok(writer) => writer,
        Err(_) => panic!("sole trace owner must unwrap"),
    };
    assert_eq!(writer.request_id(), REQUEST_ID);
}

#[test]
fn build_application_opens_migrated_sqlite_store() {
    let _current_dir = lock_current_dir();
    let (_home, paths) = isolated_paths();
    let trace = adopt_trace(&paths.traces);
    let app = build_application(paths, trace, CliDefaults::default()).expect("application");

    assert!(app.database_path().is_file(), "database file must exist");
    let conn = connect_with_pragmas(app.database_path()).expect("read connection");
    let snapshot = LogicalAuthoritySnapshot::capture(&conn).expect("logical snapshot");
    assert_eq!(snapshot.user_version, SUPPORTED_SCHEMA_VERSION);
    assert_eq!(snapshot.tables.len(), 7);
    assert!(snapshot.integrity_key_hash.starts_with("sha256:"));
}

#[test]
fn load_configuration_applies_cli_project_global_precedence() {
    let (_home, paths) = isolated_paths();
    fs::create_dir_all(&paths.config_root).expect("config root");
    fs::write(
        &paths.global_config,
        r#"
schema_version = 1

[defaults]
format = "json"
timeout_seconds = 120
"#,
    )
    .expect("global config");

    let project = tempfile::tempdir().expect("project dir");
    fs::write(
        project.path().join(".loop-engine.toml"),
        r#"
schema_version = 1

[defaults]
format = "human"
provider = "project-provider"
"#,
    )
    .expect("project config");

    let _current_dir = CurrentDirGuard::enter(project.path());
    let loaded = load_configuration(
        &paths,
        &CliDefaults {
            format: None,
            provider: Some("cli-provider".into()),
            timeout_seconds: None,
        },
    )
    .expect("loaded configuration");

    assert_eq!(
        loaded.caller_cwd,
        std::env::current_dir().expect("caller cwd remains available")
    );
    assert_eq!(loaded.defaults.format, OutputFormat::Human);
    assert_eq!(loaded.defaults.provider.as_deref(), Some("cli-provider"));
    assert_eq!(loaded.defaults.timeout_seconds, 120);
    assert!(loaded.project.is_some());
    assert!(loaded.global.is_some());
}

#[test]
fn build_application_adopts_startup_trace_without_second_trace_file() {
    let _current_dir = lock_current_dir();
    let (_home, paths) = isolated_paths();
    let trace = adopt_trace(&paths.traces);
    assert_eq!(count_trace_files(&paths.traces), 1);

    let app = build_application(paths, trace, CliDefaults::default()).expect("application");

    assert_eq!(app.trace.request_id(), REQUEST_ID);
    assert!(app.trace.persistence_trace().is_enabled());
    assert_eq!(count_trace_files(&app.configuration.paths.traces), 1);
}

#[test]
fn build_application_wires_real_integration_adapters() {
    let _current_dir = lock_current_dir();
    let (_home, paths) = isolated_paths();
    let trace = adopt_trace(&paths.traces);
    let app = build_application(paths, trace, CliDefaults::default()).expect("application");

    let _ = &app.ids;
    let _ = &app.clock;
    let _ = &app.digests;
    let _ = &app.catalog;
    let _ = &app.invoker;
    let _ = &app.run_create;
    let _ = &app.run_mutations;
    let _ = &app.run_reads;
    let _ = &app.event_attempts;
    let _ = &app.guidance;
    let _ = &app.compatibility;
    let _ = &app.evidence_reads;
    let _ = &app.history;
    let _ = &app.exporter;
}
