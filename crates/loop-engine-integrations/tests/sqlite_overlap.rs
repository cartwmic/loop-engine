//! SQLite overlap and locking integration tests (T119).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use loop_engine_core::capabilities::PageRequest;
use loop_engine_core::capabilities::event_attempt_writer::EventAttemptWriter;
use loop_engine_core::capabilities::persistence_commands::{CreateRunCommand, EventCommitBranch};
use loop_engine_core::capabilities::provider_catalog::{
    ActiveSetSnapshot, CatalogMutation, DisableAcknowledgement, ProviderCatalog, ProviderConfig,
};
use loop_engine_core::capabilities::run_writer::RunWriter;
use loop_engine_core::model::attempt::{
    AttemptFacts, EvidenceAssociations, JournalExtension, ProviderFact, ProviderRole,
    TransitionFact,
};
use loop_engine_core::model::bounded::{
    COLLECTION_PAGE_DATA_BUDGET_BYTES, COLLECTION_PAGE_DEFAULT_COUNT, SQLITE_BUSY_TIMEOUT_MS,
};
use loop_engine_core::model::decision::resolve_gate_free;
use loop_engine_core::model::evidence::{EvidenceAssociation, EvidenceRecord, EvidenceSource};
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_projection::SemanticGraphProjection;
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{
    EventId, EvidenceId, EvidenceKind, GraphRevision, ProviderHandle, RegistrationId, RequestId,
    RunId, StateId,
};
use loop_engine_core::model::journal::JournalDraft;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::provider::DigestObservation;
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::run::Run;
use loop_engine_core::model::run_input::InputDeclarations;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::operations::run_request::completed_command_for_test;
use loop_engine_integrations::persistence::records::{
    GV01_CANONICAL_GRAPH_JSON, GV01_GRAPH_REVISION,
};
use loop_engine_integrations::persistence::{
    CatalogPersistenceError, LogicalAuthoritySnapshot, RunCreateError, SUPPORTED_SCHEMA_VERSION,
    SqliteEventAttemptWriter, SqliteProviderCatalog, SqliteRunReads, SqliteRunWriter, SqliteStore,
    connect_with_pragmas, inspect_logical_store,
};
use loop_engine_integrations::provider_protocol::canonical::graph_bytes;
use loop_engine_integrations::sha256_digest::sha256_label;
use rusqlite::{Connection, params};
use tempfile::TempDir;

/// Frozen canonical graph for gate-free checkpoint/advance event-attempt overlap tests.
const REQUEST_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}},{"final":false,"id":"review","static_guidance":{"kind":"text","text":"Review the change."}}],"transitions":[{"event_id":"advance","gate_ids":[],"source_state_id":"draft","target_state_id":"review"},{"event_id":"checkpoint","gate_ids":[],"source_state_id":"draft","target_state_id":"draft"}]}"#;
const REQUEST_GRAPH_REVISION: &str =
    "sha256:d5b2dc73bbb81d7ce3802c6a1ad3b8ff86f51a40fc61b095a86432d5fc29dc19";

struct StoreHarness {
    _dir: TempDir,
    path: PathBuf,
    registration_id: RegistrationId,
}

impl StoreHarness {
    fn fresh() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        SqliteStore::open(&path).unwrap();
        let registration_id =
            RegistrationId::parse("019f0000-0000-7000-8000-000000000001").unwrap();
        let catalog = SqliteProviderCatalog::new(path.clone());
        catalog
            .mutate(CatalogMutation::Add {
                registration_id: registration_id.clone(),
                handle: ProviderHandle::parse("provider-a").unwrap(),
                config: sample_provider_config(),
            })
            .unwrap();
        Self {
            _dir: dir,
            path,
            registration_id,
        }
    }

    fn catalog(&self) -> SqliteProviderCatalog {
        SqliteProviderCatalog::new(self.path.clone())
    }

    fn run_writer(&self) -> SqliteRunWriter {
        SqliteRunWriter::new(self.path.clone())
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn authority_snapshot(&self) -> LogicalAuthoritySnapshot {
        LogicalAuthoritySnapshot::capture(&self.connection()).unwrap()
    }

    fn assert_authority_unchanged(&self, before: &LogicalAuthoritySnapshot) {
        let after = self.authority_snapshot();
        LogicalAuthoritySnapshot::assert_unchanged(before, &after)
            .expect("logical authority unchanged");
    }
}

fn sample_provider_config() -> ProviderConfig {
    ProviderConfig::new("/bin/provider", vec![], "/work", 60).unwrap()
}

fn gv01_graph() -> ValidatedGraph {
    let state = State::new(
        StateId::parse("draft").unwrap(),
        false,
        StaticGuidance::Text(
            loop_engine_core::model::bounded::BoundedText::non_empty(
                "static_guidance",
                "Prepare the change.",
            )
            .unwrap(),
        ),
        None,
    );
    ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
        StateId::parse("draft").unwrap(),
        vec![state],
        vec![],
        InputDeclarations::default(),
        LiveGuidanceCapability::Unsupported,
        None,
    ))
    .unwrap()
}

fn graph_revision(graph: &ValidatedGraph) -> GraphRevision {
    let projection = SemanticGraphProjection::from_validated(graph);
    GraphRevision::parse(sha256_label(
        &graph_bytes(&projection).expect("graph bytes"),
    ))
    .unwrap()
}

fn describe_fact(registration_id: &RegistrationId, config_revision: u64) -> ProviderFact {
    ProviderFact::new(
        registration_id.clone(),
        config_revision,
        ProviderRole::Describe,
        RequestId::parse("pv-describe-001").unwrap(),
        "/bin/provider",
        OutcomeClass::Completed,
        DigestObservation::Unavailable,
        None,
        Some(1),
    )
    .unwrap()
}

fn creation_command(
    run_id: &str,
    registration_id: &RegistrationId,
    expected_config_revision: u64,
) -> CreateRunCommand {
    let graph = gv01_graph();
    let revision = graph_revision(&graph);
    assert_eq!(revision.as_str(), GV01_GRAPH_REVISION);
    let run = Run::create(
        RunId::parse(run_id).unwrap(),
        registration_id.clone(),
        graph,
        revision.clone(),
        Default::default(),
        None,
    )
    .unwrap();
    let draft = JournalDraft::new(
        run.id().clone(),
        ObservedAt::parse("2026-07-17T14:00:00.123Z").unwrap(),
        "run.create",
        RequestId::parse("01J9X3K2M4N5P6Q7R8S9T0V1W").unwrap(),
        OutcomeClass::Completed,
        None,
        Some(AttemptFacts {
            provider_observations: vec![describe_fact(registration_id, expected_config_revision)],
            ..AttemptFacts::default()
        }),
        JournalExtension::RunCreated {
            graph_revision: revision,
        },
    )
    .unwrap();
    CreateRunCommand::for_test(run, expected_config_revision, draft)
}

fn persisted_counts(conn: &Connection, run_id: &str) -> (i64, i64, i64) {
    let runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let journal: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let sequences: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    (runs, journal, sequences)
}

fn assert_no_partial_run(conn: &Connection, run_id: &str) {
    let counts = persisted_counts(conn, run_id);
    assert!(
        counts == (0, 0, 0) || counts == (1, 1, 1),
        "partial run artifacts for {run_id}: {counts:?}"
    );
}

fn assert_journal_sequence_coherent(conn: &Connection, run_id: &str) {
    let next_sequence: i64 = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let max_sequence: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM journal_entries WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        next_sequence,
        max_sequence + 1,
        "journal allocator incoherent for {run_id}"
    );
}

fn assert_sqlite_busy_message(message: &str) {
    let lower = message.to_ascii_lowercase();
    assert!(
        lower.contains("locked") || lower.contains("busy"),
        "expected SQLITE_BUSY classification, got: {message}"
    );
}

fn disable_acknowledgement(
    catalog: &SqliteProviderCatalog,
    registration_id: &RegistrationId,
) -> (ActiveSetSnapshot, DisableAcknowledgement) {
    let request = PageRequest::new(
        COLLECTION_PAGE_DEFAULT_COUNT,
        COLLECTION_PAGE_DATA_BUDGET_BYTES,
        None,
        (),
    )
    .unwrap();
    let page = catalog
        .disable_warnings_page(registration_id, &request)
        .unwrap();
    (
        page.snapshot,
        page.acknowledgement
            .expect("disable acknowledgement required for overlap fixture"),
    )
}

fn tombstone_registration(
    catalog: &SqliteProviderCatalog,
    registration_id: &RegistrationId,
) -> u64 {
    let (snapshot, acknowledgement) = disable_acknowledgement(catalog, registration_id);
    let tombstone_revision = snapshot.config_revision();
    catalog
        .mutate(CatalogMutation::Disable {
            registration_id: registration_id.clone(),
            expected: snapshot,
            acknowledgement,
        })
        .unwrap();
    tombstone_revision + 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterOrder {
    CatalogFirst,
    CreateFirst,
}

fn race_create_vs_catalog_update(order: WriterOrder) {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000201";
    let ready = Arc::new(Barrier::new(2));
    let (catalog_done_tx, catalog_done_rx) = mpsc::channel::<Result<(), String>>();
    let (create_done_tx, create_done_rx) = mpsc::channel::<Result<(), String>>();

    let catalog_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let catalog_ready = Arc::clone(&ready);
    let catalog_thread = thread::spawn(move || {
        catalog_ready.wait();
        let catalog = SqliteProviderCatalog::new(catalog_path);
        let updated = ProviderConfig::new("/bin/provider2", vec![], "/work2", 120).unwrap();
        match order {
            WriterOrder::CatalogFirst => {
                let result = catalog.mutate(CatalogMutation::Update {
                    registration_id: registration_id.clone(),
                    expected_config_revision: 1,
                    config: updated,
                });
                catalog_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
            WriterOrder::CreateFirst => {
                create_done_rx
                    .recv()
                    .unwrap()
                    .expect("run create succeeded before catalog update");
                catalog.mutate(CatalogMutation::Update {
                    registration_id: registration_id.clone(),
                    expected_config_revision: 1,
                    config: updated,
                })
            }
        }
    });

    let writer_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let create_ready = Arc::clone(&ready);
    let create_thread = thread::spawn(move || {
        create_ready.wait();
        let writer = SqliteRunWriter::new(writer_path.clone());
        let command = creation_command(run_id, &registration_id, 1);
        match order {
            WriterOrder::CatalogFirst => {
                catalog_done_rx
                    .recv()
                    .unwrap()
                    .expect("catalog update succeeded before run create");
                writer.create(command)
            }
            WriterOrder::CreateFirst => {
                let result = writer.create(command);
                create_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
        }
    });

    let catalog_result = catalog_thread.join().unwrap();
    let create_result = create_thread.join().unwrap();

    let conn = harness.connection();
    match order {
        WriterOrder::CatalogFirst => {
            assert!(catalog_result.is_ok());
            assert!(matches!(
                create_result,
                Err(RunCreateError::StaleProviderConfig)
            ));
            assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
            assert_eq!(
                conn.query_row(
                    "SELECT config_revision FROM provider_registrations WHERE registration_id = ?1",
                    params![harness.registration_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                2
            );
        }
        WriterOrder::CreateFirst => {
            assert!(create_result.is_ok());
            assert!(catalog_result.is_ok());
            assert_eq!(persisted_counts(&conn, run_id), (1, 1, 1));
            assert_journal_sequence_coherent(&conn, run_id);
            let config_at_create: i64 = conn
                .query_row(
                    "SELECT config_revision_at_create FROM runs WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(config_at_create, 1);
            assert_eq!(
                conn.query_row(
                    "SELECT config_revision FROM provider_registrations WHERE registration_id = ?1",
                    params![harness.registration_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                2
            );
        }
    }
    assert_no_partial_run(&conn, run_id);
}

fn race_create_vs_disable(order: WriterOrder) {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000202";
    let catalog = harness.catalog();
    let (disable_snapshot, disable_ack) =
        disable_acknowledgement(&catalog, &harness.registration_id);
    let ready = Arc::new(Barrier::new(2));
    let (catalog_done_tx, catalog_done_rx) = mpsc::channel::<Result<(), String>>();
    let (create_done_tx, create_done_rx) = mpsc::channel::<Result<(), String>>();

    let catalog_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let catalog_ready = Arc::clone(&ready);
    let catalog_thread = thread::spawn(move || {
        catalog_ready.wait();
        let catalog = SqliteProviderCatalog::new(catalog_path);
        match order {
            WriterOrder::CatalogFirst => {
                let result = catalog.mutate(CatalogMutation::Disable {
                    registration_id: registration_id.clone(),
                    expected: disable_snapshot,
                    acknowledgement: disable_ack,
                });
                catalog_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
            WriterOrder::CreateFirst => {
                create_done_rx
                    .recv()
                    .unwrap()
                    .expect("run create succeeded before provider disable");
                let (snapshot, acknowledgement) =
                    disable_acknowledgement(&catalog, &registration_id);
                catalog.mutate(CatalogMutation::Disable {
                    registration_id: registration_id.clone(),
                    expected: snapshot,
                    acknowledgement,
                })
            }
        }
    });

    let writer_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let create_ready = Arc::clone(&ready);
    let create_thread = thread::spawn(move || {
        create_ready.wait();
        let writer = SqliteRunWriter::new(writer_path);
        let command = creation_command(run_id, &registration_id, 1);
        match order {
            WriterOrder::CatalogFirst => {
                catalog_done_rx
                    .recv()
                    .unwrap()
                    .expect("provider disable succeeded before run create");
                writer.create(command)
            }
            WriterOrder::CreateFirst => {
                let result = writer.create(command);
                create_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
        }
    });

    let disable_result = catalog_thread.join().unwrap();
    let create_result = create_thread.join().unwrap();
    let conn = harness.connection();

    match order {
        WriterOrder::CatalogFirst => {
            assert!(disable_result.is_ok());
            assert!(matches!(
                create_result,
                Err(RunCreateError::StaleProviderConfig)
            ));
            assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
            assert_eq!(
                conn.query_row(
                    "SELECT enabled FROM provider_registrations WHERE registration_id = ?1",
                    params![harness.registration_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                0
            );
        }
        WriterOrder::CreateFirst => {
            assert!(create_result.is_ok());
            assert!(disable_result.is_ok());
            assert_eq!(persisted_counts(&conn, run_id), (1, 1, 1));
            assert_journal_sequence_coherent(&conn, run_id);
            assert_eq!(
                conn.query_row(
                    "SELECT enabled FROM provider_registrations WHERE registration_id = ?1",
                    params![harness.registration_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                0
            );
        }
    }
    assert_no_partial_run(&conn, run_id);
}

fn race_create_vs_restore(order: WriterOrder) {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000203";
    let tombstone_revision = tombstone_registration(&harness.catalog(), &harness.registration_id);
    let restore_handle = ProviderHandle::parse("provider-restored").unwrap();
    let ready = Arc::new(Barrier::new(2));
    let (catalog_done_tx, catalog_done_rx) = mpsc::channel::<Result<(), String>>();
    let (create_done_tx, create_done_rx) = mpsc::channel::<Result<(), String>>();

    let catalog_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let catalog_ready = Arc::clone(&ready);
    let catalog_thread = thread::spawn(move || {
        catalog_ready.wait();
        let catalog = SqliteProviderCatalog::new(catalog_path);
        match order {
            WriterOrder::CatalogFirst => {
                let result = catalog.mutate(CatalogMutation::Restore {
                    registration_id: registration_id.clone(),
                    expected_config_revision: tombstone_revision,
                    handle: restore_handle.clone(),
                    config: sample_provider_config(),
                });
                catalog_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
            WriterOrder::CreateFirst => {
                create_done_rx
                    .recv()
                    .unwrap()
                    .expect_err("stale provider config when create races restore");
                catalog.mutate(CatalogMutation::Restore {
                    registration_id: registration_id.clone(),
                    expected_config_revision: tombstone_revision,
                    handle: restore_handle.clone(),
                    config: sample_provider_config(),
                })
            }
        }
    });

    let writer_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let create_ready = Arc::clone(&ready);
    let create_thread = thread::spawn(move || {
        create_ready.wait();
        let writer = SqliteRunWriter::new(writer_path);
        let command = creation_command(run_id, &registration_id, tombstone_revision);
        match order {
            WriterOrder::CatalogFirst => {
                catalog_done_rx
                    .recv()
                    .unwrap()
                    .expect("provider restore succeeded before run create");
                writer.create(command)
            }
            WriterOrder::CreateFirst => {
                let result = writer.create(command);
                create_done_tx
                    .send(
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|error| error.to_string()),
                    )
                    .ok();
                result
            }
        }
    });

    let restore_result = catalog_thread.join().unwrap();
    let create_result = create_thread.join().unwrap();
    let conn = harness.connection();

    match order {
        WriterOrder::CatalogFirst => {
            assert!(restore_result.is_ok());
            assert!(matches!(
                create_result,
                Err(RunCreateError::StaleProviderConfig)
            ));
            assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
        }
        WriterOrder::CreateFirst => {
            assert!(matches!(
                create_result,
                Err(RunCreateError::StaleProviderConfig)
            ));
            assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
            assert!(restore_result.is_ok());
        }
    }
    assert_no_partial_run(&conn, run_id);
}

struct EventHarness {
    _dir: TempDir,
    path: PathBuf,
    writer: SqliteEventAttemptWriter,
    reads: SqliteRunReads,
}

impl EventHarness {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        SqliteStore::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        insert_registration(&conn, "reg-1");
        insert_run(&conn, "run-1", "reg-1");
        Self {
            writer: SqliteEventAttemptWriter::new(path.clone()),
            reads: SqliteRunReads::new(path.clone()),
            path,
            _dir: dir,
        }
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn run(&self) -> Run {
        self.reads.get(&RunId::parse("run-1").unwrap()).unwrap()
    }
}

fn insert_registration(conn: &Connection, registration_id: &str) {
    conn.execute(
        "INSERT INTO provider_registrations (
            registration_id, handle, enabled, config_revision, executable, argv_json,
            working_directory, timeout_seconds, created_at, updated_at
        ) VALUES (?1, 'provider-a', 1, 1, '/bin/provider', '[]', '/work', 60,
                  '2026-07-17T12:00:00.000Z', '2026-07-17T12:00:00.000Z')",
        params![registration_id],
    )
    .unwrap();
}

fn insert_run(conn: &Connection, run_id: &str, registration_id: &str) {
    conn.execute(
        "INSERT INTO runs (
            run_id, registration_id, config_revision_at_create, current_state, lifecycle,
            workflow_state_version, lifecycle_version, label_version, graph_revision,
            canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
        ) VALUES (?1, ?2, 1, 'draft', 'active', 1, 1, 1, ?3, 1, ?4, '{}', '2026-07-17T12:00:00.000Z')",
        params![run_id, registration_id, REQUEST_GRAPH_REVISION, REQUEST_GRAPH_JSON],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 2)",
        params![run_id],
    )
    .unwrap();
}

fn sample_evidence(id: &str) -> EvidenceRecord {
    EvidenceRecord::new(
        EvidenceId::parse(id).unwrap(),
        EvidenceKind::parse("artifact").unwrap(),
        "opaque:locator",
        None,
        None,
        None,
        EvidenceSource::Caller,
        ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
    )
    .unwrap()
}

fn transition_attempt(
    run: &Run,
    event: &EventId,
    target: &StateId,
    applied: bool,
    inline: Vec<EvidenceRecord>,
) -> AttemptFacts {
    AttemptFacts {
        transition: Some(
            TransitionFact::new(
                event.clone(),
                run.current_state().clone(),
                Some(target.clone()),
                applied,
            )
            .unwrap(),
        ),
        evidence_associations: Some(EvidenceAssociations {
            inline: inline.clone(),
            ..EvidenceAssociations::default()
        }),
        evidence_recorded: Some(loop_engine_core::model::outcome::EvidenceRecordedStatus {
            inline: !inline.is_empty(),
            ..Default::default()
        }),
        ..AttemptFacts::default()
    }
}

fn journal_draft(run: &Run, outcome: OutcomeClass, attempt: AttemptFacts) -> JournalDraft {
    let reason = match outcome {
        OutcomeClass::Completed => None,
        OutcomeClass::Rejected => Some(Reason::new(ReasonCode::GateFailed, "gate failed").unwrap()),
        OutcomeClass::Error => Some(Reason::new(ReasonCode::StateStaleVersion, "stale").unwrap()),
    };
    JournalDraft::new(
        run.id().clone(),
        ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
        "run.request",
        RequestId::parse("request-1").unwrap(),
        outcome,
        reason,
        Some(attempt),
        JournalExtension::TransitionAttempt,
    )
    .unwrap()
}

fn completed_request_command(
    run: &Run,
    event: &EventId,
    request_id: &str,
    evidence_id: &str,
) -> loop_engine_core::capabilities::persistence_commands::CommitEventAttemptCommand {
    let decision = match resolve_gate_free(run, event) {
        Ok(decision) => decision,
        Err(error) => panic!("gate-free decision must resolve for test graph: {error:?}"),
    };
    let inline = sample_evidence(evidence_id);
    let associations = vec![EvidenceAssociation::new(
        inline.id().clone(),
        Some(event.clone()),
        None,
    )];
    let completed_attempt =
        transition_attempt(run, event, decision.target(), true, vec![inline.clone()]);
    let stale_attempt = transition_attempt(run, event, decision.target(), false, Vec::new());
    let mut completed = journal_draft(run, OutcomeClass::Completed, completed_attempt);
    let mut stale = journal_draft(run, OutcomeClass::Error, stale_attempt);
    completed = JournalDraft::new(
        run.id().clone(),
        ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
        "run.request",
        RequestId::parse(request_id).unwrap(),
        OutcomeClass::Completed,
        None,
        completed.attempt().cloned(),
        JournalExtension::TransitionAttempt,
    )
    .unwrap();
    stale = JournalDraft::new(
        run.id().clone(),
        ObservedAt::parse("2026-07-18T00:00:00.000Z").unwrap(),
        "run.request",
        RequestId::parse(request_id).unwrap(),
        OutcomeClass::Error,
        Some(Reason::new(ReasonCode::StateStaleVersion, "stale").unwrap()),
        stale.attempt().cloned(),
        JournalExtension::TransitionAttempt,
    )
    .unwrap();
    completed_command_for_test(
        &decision,
        None,
        vec![inline],
        associations,
        completed,
        stale,
    )
    .expect("completed command must assemble for test graph")
}

fn workflow_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT workflow_state_version FROM runs WHERE run_id = 'run-1'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn independent_run_concurrent_writes_both_persist() {
    let harness = Arc::new(StoreHarness::fresh());
    let ready = Arc::new(Barrier::new(2));
    let start = Arc::new(Barrier::new(2));
    let run_ids = [
        "019f0000-0000-7000-8000-000000000301",
        "019f0000-0000-7000-8000-000000000302",
    ];

    let handles: Vec<_> = run_ids
        .into_iter()
        .enumerate()
        .map(|(index, run_id)| {
            let harness = Arc::clone(&harness);
            let ready = Arc::clone(&ready);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                ready.wait();
                start.wait();
                let writer = SqliteRunWriter::new(harness.path.clone());
                let command = creation_command(run_id, &harness.registration_id, 1);
                (index, writer.create(command))
            })
        })
        .collect();

    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert!(outcomes.iter().all(|(_, result)| result.is_ok()));

    let conn = harness.connection();
    for run_id in [
        "019f0000-0000-7000-8000-000000000301",
        "019f0000-0000-7000-8000-000000000302",
    ] {
        assert_eq!(persisted_counts(&conn, run_id), (1, 1, 1));
        assert_journal_sequence_coherent(&conn, run_id);
        let graph_json: String = conn
            .query_row(
                "SELECT graph_canonical_projection_json FROM runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(graph_json, GV01_CANONICAL_GRAPH_JSON);
    }
}

#[test]
fn same_handle_add_race_exactly_one_wins() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    SqliteStore::open(&path).unwrap();
    let catalog = SqliteProviderCatalog::new(path.clone());
    let handle = ProviderHandle::parse("contended-handle").unwrap();
    let first_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417a").unwrap();
    let second_id = RegistrationId::parse("019f6e88-b403-73a6-89f9-ebfe668b417b").unwrap();
    let ready = Arc::new(Barrier::new(2));
    let start = Arc::new(Barrier::new(2));

    let first = {
        let path = path.clone();
        let handle = handle.clone();
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        thread::spawn(move || {
            ready.wait();
            start.wait();
            SqliteProviderCatalog::new(path).mutate(CatalogMutation::Add {
                registration_id: first_id,
                handle,
                config: sample_provider_config(),
            })
        })
    };
    let second = {
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        thread::spawn(move || {
            ready.wait();
            start.wait();
            catalog.mutate(CatalogMutation::Add {
                registration_id: second_id,
                handle,
                config: sample_provider_config(),
            })
        })
    };

    let outcomes = [first.join().unwrap(), second.join().unwrap()];
    assert!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(CatalogPersistenceError::Duplicate)))
            .count()
            == 1
    );
    assert!(outcomes.iter().any(Result::is_ok));

    let conn = Connection::open(&path).unwrap();
    let enabled_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM provider_registrations WHERE handle = 'contended-handle' AND enabled = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled_count, 1);
}

#[test]
fn run_create_races_provider_update_catalog_first() {
    race_create_vs_catalog_update(WriterOrder::CatalogFirst);
}

#[test]
fn run_create_races_provider_update_create_first() {
    race_create_vs_catalog_update(WriterOrder::CreateFirst);
}

#[test]
fn run_create_races_disable_catalog_first() {
    race_create_vs_disable(WriterOrder::CatalogFirst);
}

#[test]
fn run_create_races_disable_create_first() {
    race_create_vs_disable(WriterOrder::CreateFirst);
}

#[test]
fn run_create_races_restore_catalog_first() {
    race_create_vs_restore(WriterOrder::CatalogFirst);
}

#[test]
fn run_create_races_restore_create_first() {
    race_create_vs_restore(WriterOrder::CreateFirst);
}

#[test]
fn stale_registration_revision_never_creates_run() {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000204";
    let conn = harness.connection();
    conn.execute(
        "UPDATE provider_registrations SET config_revision = 2, updated_at = '2026-07-17T13:00:00.000Z' WHERE registration_id = ?1",
        params![harness.registration_id.as_str()],
    )
    .unwrap();
    let before = harness.authority_snapshot();

    let writer = harness.run_writer();
    let command = creation_command(run_id, &harness.registration_id, 1);
    assert!(matches!(
        writer.create(command),
        Err(RunCreateError::StaleProviderConfig)
    ));
    assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
    harness.assert_authority_unchanged(&before);
}

#[test]
fn busy_wait_releases_and_succeeds_without_partial_writes() {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000401";
    let (lock_held_tx, lock_held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let lock_path = harness.path.clone();
    let locker = thread::spawn(move || {
        let conn = connect_with_pragmas(&lock_path).unwrap();
        conn.execute("BEGIN IMMEDIATE", []).unwrap();
        lock_held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        conn.execute("ROLLBACK", []).unwrap();
    });

    lock_held_rx.recv().unwrap();

    let sync = Arc::new(Barrier::new(2));
    let writer_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let (attempt_started_tx, attempt_started_rx) = mpsc::channel();
    let (attempt_done_tx, attempt_done_rx) = mpsc::channel();
    let contender_sync = Arc::clone(&sync);
    let attempt = thread::spawn(move || {
        contender_sync.wait();
        let writer = SqliteRunWriter::new(writer_path);
        let command = creation_command(run_id, &registration_id, 1);
        attempt_started_tx.send(()).unwrap();
        let result = writer.create(command);
        attempt_done_tx.send(result).unwrap();
    });

    sync.wait();
    attempt_started_rx
        .recv()
        .expect("contender must begin create attempt while BEGIN IMMEDIATE lock is held");
    match attempt_done_rx.recv_timeout(Duration::from_millis(100)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Ok(result) => {
            panic!("create completed before lock release while lock was held: {result:?}")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("contender exited before lock release");
        }
    }

    release_tx.send(()).unwrap();
    let create_result = attempt_done_rx
        .recv()
        .expect("contender must complete after lock release");
    locker.join().unwrap();
    attempt.join().unwrap();

    assert!(create_result.is_ok());
    let conn = harness.connection();
    assert_eq!(persisted_counts(&conn, run_id), (1, 1, 1));
    assert_journal_sequence_coherent(&conn, run_id);
    assert_no_partial_run(&conn, run_id);
}

#[test]
fn busy_timeout_surfaces_locked_error_without_partial_writes() {
    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000402";
    let before = harness.authority_snapshot();
    let (lock_held_tx, lock_held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let lock_path = harness.path.clone();
    let locker = thread::spawn(move || {
        let conn = connect_with_pragmas(&lock_path).unwrap();
        conn.execute("BEGIN IMMEDIATE", []).unwrap();
        lock_held_tx.send(()).unwrap();
        // Release only after the competing writer reports timeout exhaustion.
        release_rx.recv().unwrap();
        conn.execute("ROLLBACK", []).unwrap();
    });

    lock_held_rx.recv().unwrap();
    let writer_path = harness.path.clone();
    let registration_id = harness.registration_id.clone();
    let (attempt_done_tx, attempt_done_rx) = mpsc::channel();
    let attempt = thread::spawn(move || {
        let started = Instant::now();
        let writer = SqliteRunWriter::new(writer_path);
        let command = creation_command(run_id, &registration_id, 1);
        let result = writer.create(command);
        attempt_done_tx.send((started.elapsed(), result)).unwrap();
    });

    let (elapsed, create_result) = attempt_done_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    locker.join().unwrap();
    attempt.join().unwrap();

    assert!(
        elapsed >= Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS),
        "expected busy wait to consume configured timeout, got {elapsed:?}"
    );
    match create_result {
        Err(RunCreateError::Sqlite(message)) => assert_sqlite_busy_message(&message),
        other => panic!("expected busy sqlite error, got {other:?}"),
    }
    let conn = harness.connection();
    assert_eq!(persisted_counts(&conn, run_id), (0, 0, 0));
    harness.assert_authority_unchanged(&before);
}

#[test]
fn concurrent_migration_opens_apply_schema_once() {
    let directory = Arc::new(TempDir::new().unwrap());
    let path = directory.path().join("state.db");
    let barrier = Arc::new(Barrier::new(4));

    thread::scope(|scope| {
        for _ in 0..4 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                SqliteStore::open(&path).unwrap();
            });
        }
    });

    let store = SqliteStore::open(&path).unwrap();
    let version: u32 = store
        .connection()
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(version, SUPPORTED_SCHEMA_VERSION);
    inspect_logical_store(store.connection()).unwrap();
    let integrity_length: i64 = store
        .connection()
        .query_row(
            "SELECT length(value) FROM integration_metadata WHERE key = 'integrity_key'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(integrity_length, 32);
}

#[test]
fn stale_event_cas_overlap_commits_one_success_and_one_stale() {
    let harness = Arc::new(EventHarness::new());
    let ready = Arc::new(Barrier::new(2));
    let start = Arc::new(Barrier::new(2));

    let run_a = harness.run();
    let run_b = harness.run();
    let command_a = completed_request_command(
        &run_a,
        &EventId::parse("advance").unwrap(),
        "req-a",
        "inline-a",
    );
    let command_b = completed_request_command(
        &run_b,
        &EventId::parse("advance").unwrap(),
        "req-b",
        "inline-b",
    );

    let first = {
        let harness = Arc::clone(&harness);
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        thread::spawn(move || {
            ready.wait();
            start.wait();
            (
                "a",
                harness
                    .writer
                    .commit_event_attempt(command_a)
                    .expect("first commit"),
            )
        })
    };
    let second = {
        let harness = Arc::clone(&harness);
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        thread::spawn(move || {
            ready.wait();
            start.wait();
            (
                "b",
                harness
                    .writer
                    .commit_event_attempt(command_b)
                    .expect("second commit"),
            )
        })
    };

    let (first_label, first_status) = first.join().unwrap();
    let (second_label, second_status) = second.join().unwrap();
    let branches = [
        (&first_label, first_status.branch),
        (&second_label, second_status.branch),
    ];
    assert!(
        branches
            .iter()
            .filter(|(_, branch)| *branch == EventCommitBranch::ExpectedVersions)
            .count()
            == 1
    );
    assert!(
        branches
            .iter()
            .filter(|(_, branch)| *branch == EventCommitBranch::StaleVersions)
            .count()
            == 1
    );
    assert_eq!(workflow_version(&harness.connection()), 2);
    assert_journal_sequence_coherent(&harness.connection(), "run-1");
}

/// Child-process helper: hold an IMMEDIATE writer lock until killed.
#[test]
fn sqlite_overlap_child_hold_immediate_until_killed() {
    let Some(path) = std::env::var_os("SQLITE_OVERLAP_CHILD_PATH") else {
        return;
    };
    let Some(ready) = std::env::var_os("SQLITE_OVERLAP_READY_FILE") else {
        return;
    };
    let conn = connect_with_pragmas(Path::new(&path)).unwrap();
    conn.execute("BEGIN IMMEDIATE", []).unwrap();
    std::fs::write(&ready, b"held").unwrap();
    loop {
        thread::park();
    }
}

#[cfg(unix)]
#[test]
fn killed_writer_reopen_preserves_integrity_and_authority() {
    use std::process::{Command, Stdio};

    let harness = StoreHarness::fresh();
    let run_id = "019f0000-0000-7000-8000-000000000501";
    let writer = harness.run_writer();
    writer
        .create(creation_command(run_id, &harness.registration_id, 1))
        .unwrap();
    let before = harness.authority_snapshot();
    inspect_logical_store(&harness.connection()).unwrap();

    let ready_file = harness._dir.path().join("child-ready");
    let executable = std::env::current_exe().unwrap();
    let mut child = Command::new(executable)
        .args([
            "--exact",
            "sqlite_overlap_child_hold_immediate_until_killed",
            "--nocapture",
        ])
        .env("SQLITE_OVERLAP_CHILD_PATH", &harness.path)
        .env("SQLITE_OVERLAP_READY_FILE", &ready_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    while !ready_file.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before acquiring writer lock"
        );
        thread::yield_now();
    }

    Command::new("/bin/kill")
        .args(["-9", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(!child.wait().unwrap().success());

    let store = SqliteStore::open(&harness.path).unwrap();
    inspect_logical_store(store.connection()).unwrap();
    harness.assert_authority_unchanged(&before);
    let conn = harness.connection();
    assert_eq!(persisted_counts(&conn, run_id), (1, 1, 1));
    assert_journal_sequence_coherent(&conn, run_id);
}
