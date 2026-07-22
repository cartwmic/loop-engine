//! Event-attempt CAS concurrency tests (T113).

use std::sync::{Arc, Barrier};
use std::thread;

use loop_engine_core::capabilities::event_attempt_writer::EventAttemptWriter;
use loop_engine_core::capabilities::persistence_commands::EventCommitBranch;
use loop_engine_core::model::attempt::{
    AttemptFacts, EvidenceAssociations, JournalExtension, TransitionFact,
};
use loop_engine_core::model::decision::resolve_gate_free;
use loop_engine_core::model::evidence::{EvidenceAssociation, EvidenceRecord, EvidenceSource};
use loop_engine_core::model::graph::{State, WorkflowGraph};
use loop_engine_core::model::graph_validation::ValidatedGraph;
use loop_engine_core::model::guidance::{LiveGuidanceCapability, StaticGuidance};
use loop_engine_core::model::ids::{EventId, EvidenceId, EvidenceKind, RequestId, RunId, StateId};
use loop_engine_core::model::journal::JournalDraft;
use loop_engine_core::model::outcome::OutcomeClass;
use loop_engine_core::model::reason::{Reason, ReasonCode};
use loop_engine_core::model::time::ObservedAt;
use loop_engine_core::model::transition::Transition;
use loop_engine_core::operations::run_request::completed_command_for_test;
use loop_engine_integrations::persistence::{
    SqliteEventAttemptWriter, SqliteRunReads, SqliteStore,
};
use rusqlite::{Connection, params};
use tempfile::TempDir;

/// Frozen canonical graph for gate-free checkpoint/advance event-attempt tests.
const REQUEST_GRAPH_JSON: &str = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}},{"final":false,"id":"review","static_guidance":{"kind":"text","text":"Review the change."}}],"transitions":[{"event_id":"advance","gate_ids":[],"source_state_id":"draft","target_state_id":"review"},{"event_id":"checkpoint","gate_ids":[],"source_state_id":"draft","target_state_id":"draft"}]}"#;
const REQUEST_GRAPH_REVISION: &str =
    "sha256:d5b2dc73bbb81d7ce3802c6a1ad3b8ff86f51a40fc61b095a86432d5fc29dc19";

struct Harness {
    _dir: TempDir,
    path: std::path::PathBuf,
    writer: SqliteEventAttemptWriter,
    reads: SqliteRunReads,
}

impl Harness {
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

    fn run(&self) -> loop_engine_core::model::run::Run {
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
    run: &loop_engine_core::model::run::Run,
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

fn journal_draft(
    run: &loop_engine_core::model::run::Run,
    outcome: OutcomeClass,
    attempt: AttemptFacts,
) -> JournalDraft {
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
    run: &loop_engine_core::model::run::Run,
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

fn lifecycle_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT lifecycle_version FROM runs WHERE run_id = 'run-1'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn journal_outcome(conn: &Connection, request_id: &str) -> String {
    conn.query_row(
        "SELECT outcome FROM journal_entries
         WHERE run_id = 'run-1' AND json_extract(encoded_payload_json, '$.request_id') = ?1",
        params![request_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn concurrent_completed_requests_cas_to_one_success_and_one_stale() {
    let harness = Arc::new(Harness::new());
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
}

#[test]
fn transition_during_provider_interval_produces_stale_attempt_without_state_mutation() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let stale_command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-stale",
        "inline-stale",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(stale_command)
                .expect("stale commit")
        })
    };
    let invalidator = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let run = harness.run();
            let advance = completed_request_command(
                &run,
                &EventId::parse("advance").unwrap(),
                "req-advance",
                "inline-advance",
            );
            harness
                .writer
                .commit_event_attempt(advance)
                .expect("invalidating transition");
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    invalidator.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::StaleVersions);
    assert_eq!(workflow_version(&harness.connection()), 2);
    assert_eq!(journal_outcome(&harness.connection(), "req-stale"), "error");
    let payload: String = harness.connection().query_row(
        "SELECT encoded_payload_json FROM journal_entries
         WHERE run_id = 'run-1' AND json_extract(encoded_payload_json, '$.request_id') = 'req-stale'",
        [],
        |row| row.get(0),
    ).unwrap();
    let wire: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(wire["state_before"]["state"], "review");
    assert_eq!(wire["state_after"]["state"], "review");
    assert_eq!(wire["state_before"]["workflow_state_version"], 2);
    assert_eq!(wire["state_after"]["workflow_state_version"], 2);
}

#[test]
fn termination_during_provider_interval_produces_stale_attempt() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let stale_command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-term-stale",
        "inline-term-stale",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(stale_command)
                .expect("stale commit")
        })
    };
    let invalidator = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let conn = harness.connection();
            conn.execute(
                "UPDATE runs SET lifecycle = 'terminated', lifecycle_version = 2 WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    invalidator.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::StaleVersions);
    assert_eq!(lifecycle_version(&harness.connection()), 2);
    assert_eq!(
        journal_outcome(&harness.connection(), "req-term-stale"),
        "error"
    );
}

#[test]
fn label_mutation_during_provider_interval_does_not_invalidate_request() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-label",
        "inline-label",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(command)
                .expect("label interleave commit")
        })
    };
    let label = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let conn = harness.connection();
            conn.execute(
                "UPDATE runs SET label = 'new-label', label_version = 2 WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    label.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
    assert_eq!(workflow_version(&harness.connection()), 1);
    assert_eq!(
        journal_outcome(&harness.connection(), "req-label"),
        "completed"
    );
}

#[test]
fn evidence_append_during_provider_interval_does_not_invalidate_request() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-evidence",
        "inline-evidence",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(command)
                .expect("evidence interleave commit")
        })
    };
    let evidence = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let conn = harness.connection();
            conn.execute(
                "INSERT INTO evidence (
                    run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
                ) VALUES ('run-1', 'evidence-extra', 'artifact', 'opaque:extra', NULL, NULL, NULL, 'caller', '2026-07-18T00:00:01.000Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
                 VALUES ('run-1', 1, 'completed', '{\"journal_schema_version\":1,\"sequence\":1,\"run_id\":\"run-1\",\"request_id\":\"annotation\",\"outcome\":\"completed\"}')",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE run_journal_sequences SET next_sequence = 2 WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    evidence.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
    assert_eq!(workflow_version(&harness.connection()), 1);
    assert_eq!(
        journal_outcome(&harness.connection(), "req-evidence"),
        "completed"
    );
}

#[test]
fn colliding_evidence_append_during_provider_interval_is_journaled_as_rejected() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-evidence-conflict",
        "inline-conflict",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(command)
                .expect("colliding evidence attempt must remain journalable")
        })
    };
    let evidence = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let conn = harness.connection();
            conn.execute(
                "INSERT INTO evidence (
                    run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
                ) VALUES ('run-1', 'inline-conflict', 'artifact', 'opaque:other', NULL, NULL, NULL, 'caller', '2026-07-18T00:00:01.000Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
                 VALUES ('run-1', 1, 'completed', '{\"journal_schema_version\":1,\"sequence\":1,\"run_id\":\"run-1\",\"request_id\":\"evidence-add\",\"outcome\":\"completed\"}')",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE run_journal_sequences SET next_sequence = 2 WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    evidence.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::InlineEvidenceConflict);
    assert_eq!(workflow_version(&harness.connection()), 1);
    assert_eq!(
        journal_outcome(&harness.connection(), "req-evidence-conflict"),
        "rejected"
    );
}

#[test]
fn annotation_note_during_provider_interval_does_not_invalidate_request() {
    let harness = Arc::new(Harness::new());
    let snapshot_taken = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let run = harness.run();
    let command = completed_request_command(
        &run,
        &EventId::parse("checkpoint").unwrap(),
        "req-note",
        "inline-note",
    );

    let request = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            mutation_done.wait();
            harness
                .writer
                .commit_event_attempt(command)
                .expect("note interleave commit")
        })
    };
    let note = {
        let harness = Arc::clone(&harness);
        let snapshot_taken = Arc::clone(&snapshot_taken);
        let mutation_done = Arc::clone(&mutation_done);
        thread::spawn(move || {
            snapshot_taken.wait();
            let conn = harness.connection();
            conn.execute(
                "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
                 VALUES ('run-1', 1, 'completed', '{\"journal_schema_version\":1,\"sequence\":1,\"run_id\":\"run-1\",\"request_id\":\"note-only\",\"entry_kind\":\"annotation\",\"outcome\":\"completed\",\"note\":\"clarification\"}')",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE run_journal_sequences SET next_sequence = 2 WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
            mutation_done.wait();
        })
    };

    let status = request.join().unwrap();
    note.join().unwrap();
    assert_eq!(status.branch, EventCommitBranch::ExpectedVersions);
    assert_eq!(workflow_version(&harness.connection()), 1);
    assert_eq!(
        journal_outcome(&harness.connection(), "req-note"),
        "completed"
    );
}

#[test]
fn request_graph_supports_gate_free_checkpoint_and_advance() {
    let state = State::new(
        StateId::parse("draft").unwrap(),
        false,
        StaticGuidance::NoneRequired,
        None,
    );
    let review = State::new(
        StateId::parse("review").unwrap(),
        false,
        StaticGuidance::NoneRequired,
        None,
    );
    let graph = ValidatedGraph::validate(WorkflowGraph::new_unvalidated(
        StateId::parse("draft").unwrap(),
        vec![state, review],
        vec![
            Transition::new(
                StateId::parse("draft").unwrap(),
                EventId::parse("checkpoint").unwrap(),
                StateId::parse("draft").unwrap(),
                vec![],
                None,
            )
            .unwrap(),
            Transition::new(
                StateId::parse("draft").unwrap(),
                EventId::parse("advance").unwrap(),
                StateId::parse("review").unwrap(),
                vec![],
                None,
            )
            .unwrap(),
        ],
        loop_engine_core::model::run_input::InputDeclarations::default(),
        LiveGuidanceCapability::Unsupported,
        None,
    ))
    .unwrap();
    assert!(!graph.into_graph().transitions().is_empty());
}
