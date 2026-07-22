//! Audit export integration tests (T116).

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use loop_engine_core::capabilities::audit_export::{AuditExporter, ExportTarget};
use loop_engine_core::capabilities::time::TimeSource;
use loop_engine_core::model::ids::RunId;
use loop_engine_core::model::time::ObservedAt;
use loop_engine_integrations::export::{
    ExportError, SqliteAuditExporter, canonical_json, sha256_label, verify_export_directory,
};
use loop_engine_integrations::persistence::SqliteStore;
use loop_engine_integrations::persistence::records::GV01_GRAPH_REVISION;
use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    observed_at: ObservedAt,
}

impl TimeSource for FixedClock {
    type Error = Infallible;

    fn now(&self) -> Result<ObservedAt, Self::Error> {
        Ok(self.observed_at)
    }
}

struct Harness {
    _root: TempDir,
    db_path: PathBuf,
    exporter: SqliteAuditExporter<FixedClock>,
}

impl Harness {
    fn new(exported_at: &str) -> Self {
        let root = TempDir::new().unwrap();
        let db_path = root.path().join("state.db");
        SqliteStore::open(&db_path).unwrap();
        let exporter = SqliteAuditExporter::with_clock(
            db_path.clone(),
            FixedClock {
                observed_at: ObservedAt::parse(exported_at).unwrap(),
            },
        );
        Self {
            _root: root,
            db_path,
            exporter,
        }
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.db_path).unwrap()
    }

    fn export_dir(&self, name: &str) -> PathBuf {
        self._root.path().join(name)
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

fn insert_run(conn: &Connection, run_id: &str, registration_id: &str, label: Option<&str>) {
    let graph_json = r#"{"canonical_graph_version":1,"initial_state_id":"draft","input_declarations":[],"live_guidance_supported":false,"states":[{"final":false,"id":"draft","static_guidance":{"kind":"text","text":"Prepare the change."}}],"transitions":[]}"#;
    conn.execute(
        "INSERT INTO runs (
            run_id, registration_id, config_revision_at_create, current_state, lifecycle,
            workflow_state_version, lifecycle_version, label_version, label, graph_revision,
            canonical_graph_version, graph_canonical_projection_json, inputs_json, created_at
        ) VALUES (?1, ?2, 1, 'draft', 'active', 1, 1, 1, ?3, ?4, 1, ?5, '{}', '2026-07-17T12:00:00.000Z')",
        params![run_id, registration_id, label, GV01_GRAPH_REVISION, graph_json],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO run_journal_sequences (run_id, next_sequence) VALUES (?1, 1)",
        params![run_id],
    )
    .unwrap();
}

fn state_json(state: &str) -> Value {
    serde_json::json!({
        "state": state,
        "lifecycle": "active",
        "workflow_state_version": 1,
        "lifecycle_version": 1
    })
}

fn creation_payload(run_id: &str) -> String {
    serde_json::json!({
        "journal_schema_version": 1,
        "sequence": 1,
        "run_id": run_id,
        "ts": "2026-07-17T14:00:00.123Z",
        "operation": "run.create",
        "request_id": "req-create-001",
        "entry_kind": "run.created",
        "outcome": "completed",
        "reason": null,
        "state_before": state_json("draft"),
        "state_after": state_json("draft"),
        "provider_observations": [{
            "registration_id": "reg-1",
            "config_revision": 1,
            "role": "describe",
            "invocation_id": "pv-describe-001",
            "executable": "/bin/provider",
            "outcome": "completed"
        }],
        "graph_revision": GV01_GRAPH_REVISION
    })
    .to_string()
}

fn insert_journal(conn: &Connection, run_id: &str, sequence: u64, payload: &str) {
    conn.execute(
        "INSERT INTO journal_entries (run_id, sequence, outcome, encoded_payload_json)
         VALUES (?1, ?2, 'completed', ?3)",
        params![run_id, i64::try_from(sequence).unwrap(), payload],
    )
    .unwrap();
}

fn insert_evidence(
    conn: &Connection,
    run_id: &str,
    evidence_id: &str,
    created_at: &str,
    locator: &str,
) {
    conn.execute(
        "INSERT INTO evidence (
            run_id, evidence_id, kind, locator, digest, media_type, metadata_json, source, created_at
        ) VALUES (?1, ?2, 'artifact', ?3, NULL, NULL, NULL, 'caller', ?4)",
        params![run_id, evidence_id, locator, created_at],
    )
    .unwrap();
}

fn seed_run_with_history(conn: &Connection, run_id: &str) {
    insert_registration(conn, "reg-1");
    insert_run(conn, run_id, "reg-1", Some("export-fixture"));
    insert_journal(conn, run_id, 1, &creation_payload(run_id));
    conn.execute(
        "UPDATE run_journal_sequences SET next_sequence = 2 WHERE run_id = ?1",
        params![run_id],
    )
    .unwrap();
}

fn read_file(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

fn db_fingerprint(conn: &Connection, run_id: &str) -> (i64, i64, i64, String) {
    let run_row: String = conn
        .query_row(
            "SELECT current_state || '|' || lifecycle || '|' || COALESCE(label, '')
             FROM runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let journal_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let evidence_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM evidence WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    let sequence_next: i64 = conn
        .query_row(
            "SELECT next_sequence FROM run_journal_sequences WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap();
    (journal_count, evidence_count, sequence_next, run_row)
}

fn parse_manifest(path: &Path) -> Value {
    serde_json::from_slice(&read_file(path)).unwrap()
}

fn verify_manifest_hashes(export_dir: &Path) {
    let manifest = parse_manifest(&export_dir.join("manifest.json"));
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .expect("files array");
    assert_eq!(files.len(), 2);
    for entry in files {
        let name = entry.get("path").and_then(Value::as_str).unwrap();
        let expected = entry.get("sha256").and_then(Value::as_str).unwrap();
        let bytes = entry.get("bytes").and_then(Value::as_u64).unwrap();
        let payload = read_file(&export_dir.join(name));
        assert_eq!(payload.len() as u64, bytes);
        assert_eq!(sha256_label(&payload), expected);
    }
}

fn assert_json_keys_sorted(value: &Value) {
    if let Value::Object(map) = value {
        let keys: Vec<_> = map.keys().map(String::as_str).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "object keys must be lexicographically sorted");
        for nested in map.values() {
            assert_json_keys_sorted(nested);
        }
    } else if let Value::Array(items) = value {
        for item in items {
            assert_json_keys_sorted(item);
        }
    }
}

#[test]
fn export_artifacts_remain_immutable_after_late_journal_commit() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-late-commit";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    drop(conn);

    let output = harness.export_dir("export-before-late-commit");
    harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap();
    verify_manifest_hashes(&output);

    let journal_before = read_file(&output.join("journal.jsonl"));
    let state_before = read_file(&output.join("state.json"));
    let manifest_before = read_file(&output.join("manifest.json"));

    let conn = harness.connection();
    let payload = serde_json::json!({
        "journal_schema_version": 1,
        "sequence": 2,
        "run_id": run_id,
        "ts": "2026-07-17T16:00:00.000Z",
        "operation": "run.annotate",
        "request_id": "req-annotate-002",
        "entry_kind": "annotation",
        "outcome": "completed",
        "reason": null,
        "state_before": state_json("draft"),
        "state_after": state_json("draft"),
        "note": "late writer"
    })
    .to_string();
    insert_journal(&conn, run_id, 2, &payload);
    conn.execute(
        "UPDATE run_journal_sequences SET next_sequence = 3 WHERE run_id = ?1",
        params![run_id],
    )
    .unwrap();
    drop(conn);

    assert_eq!(read_file(&output.join("journal.jsonl")), journal_before);
    assert_eq!(read_file(&output.join("state.json")), state_before);
    assert_eq!(read_file(&output.join("manifest.json")), manifest_before);

    let output_after = harness.export_dir("export-after-late-commit");
    harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output_after.to_str().unwrap()).unwrap(),
        )
        .unwrap();
    let journal_lines = String::from_utf8(read_file(&output_after.join("journal.jsonl"))).unwrap();
    assert_eq!(journal_lines.lines().count(), 2);
}
#[test]
fn export_orders_journal_evidence_and_manifest_files_deterministically() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-order-1";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    insert_evidence(
        &conn,
        run_id,
        "ev-b",
        "2026-07-17T12:00:00.000Z",
        "opaque:b",
    );
    insert_evidence(
        &conn,
        run_id,
        "ev-a",
        "2026-07-17T12:00:00.000Z",
        "opaque:a",
    );
    drop(conn);

    let output = harness.export_dir("export-a");
    let parsed_run_id = RunId::parse(run_id).unwrap();
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&parsed_run_id, &target)
        .unwrap();

    let state: Value = serde_json::from_slice(&read_file(&output.join("state.json"))).unwrap();
    let evidence = state
        .get("evidence")
        .and_then(Value::as_array)
        .expect("evidence array");
    assert_eq!(
        evidence
            .iter()
            .map(|row| row.get("evidence_id").and_then(Value::as_str).unwrap())
            .collect::<Vec<_>>(),
        vec!["ev-a", "ev-b"]
    );

    let journal_lines = String::from_utf8(read_file(&output.join("journal.jsonl"))).unwrap();
    let sequences = journal_lines
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap()
                .get("sequence")
                .and_then(Value::as_u64)
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1]);
    for line in journal_lines.lines() {
        let parsed: Value = serde_json::from_str(line).unwrap();
        assert_json_keys_sorted(&parsed);
        assert_eq!(line, canonical_json(&parsed));
    }

    let manifest = parse_manifest(&output.join("manifest.json"));
    let file_paths = manifest
        .get("files")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|entry| entry.get("path").and_then(Value::as_str).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(file_paths, vec!["journal.jsonl", "state.json"]);
    verify_manifest_hashes(&output);

    let output_b = harness.export_dir("export-b");
    let target_b = ExportTarget::parse(output_b.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&parsed_run_id, &target_b)
        .unwrap();

    assert_eq!(
        read_file(&output.join("state.json")),
        read_file(&output_b.join("state.json"))
    );
    assert_eq!(
        read_file(&output.join("journal.jsonl")),
        read_file(&output_b.join("journal.jsonl"))
    );
    let manifest_b = parse_manifest(&output_b.join("manifest.json"));
    assert_eq!(canonical_json(&manifest), canonical_json(&manifest_b));
}

#[test]
fn export_rejects_non_empty_output_directory() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let conn = harness.connection();
    seed_run_with_history(&conn, "run-overwrite");
    drop(conn);

    let output = harness.export_dir("export-target");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("marker.txt"), b"occupied").unwrap();

    let error = harness
        .exporter
        .export_consistent(
            &RunId::parse("run-overwrite").unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::TargetNotEmpty));
}

#[test]
fn export_does_not_dereference_external_locator() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-locator";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    let external = harness.export_dir("external-target.txt");
    let locator = format!("file://{}", external.display());
    insert_evidence(
        &conn,
        run_id,
        "ev-external",
        "2026-07-17T12:00:01.000Z",
        &locator,
    );
    drop(conn);

    assert!(!external.exists());
    let output = harness.export_dir("export-locator");
    harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap();
    assert!(!external.exists());

    let state: Value = serde_json::from_slice(&read_file(&output.join("state.json"))).unwrap();
    let stored = state
        .get("evidence")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("locator"))
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(stored, locator);
}

#[test]
fn export_leaves_database_unchanged() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-unchanged";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    drop(conn);

    let before = db_fingerprint(&harness.connection(), run_id);

    let output = harness.export_dir("export-unchanged");
    harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap();

    let after = db_fingerprint(&harness.connection(), run_id);
    assert_eq!(before, after);
}

#[test]
fn successful_export_rejects_repeat_publication_to_same_directory() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let conn = harness.connection();
    seed_run_with_history(&conn, "run-repeat");
    drop(conn);

    let output = harness.export_dir("export-repeat");
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&RunId::parse("run-repeat").unwrap(), &target)
        .unwrap();
    verify_manifest_hashes(&output);

    let error = harness
        .exporter
        .export_consistent(&RunId::parse("run-repeat").unwrap(), &target)
        .unwrap_err();
    assert!(matches!(error, ExportError::TargetNotEmpty));
}

#[test]
fn export_cross_process_worker() {
    let Some(db_path) = std::env::var_os("LOOP_EXPORT_CHILD_DB") else {
        return;
    };
    let output = PathBuf::from(std::env::var_os("LOOP_EXPORT_CHILD_TARGET").unwrap());
    let ready = PathBuf::from(std::env::var_os("LOOP_EXPORT_CHILD_READY").unwrap());
    let gate = PathBuf::from(std::env::var_os("LOOP_EXPORT_CHILD_GATE").unwrap());
    let result_path = PathBuf::from(std::env::var_os("LOOP_EXPORT_CHILD_RESULT").unwrap());
    fs::write(&ready, b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.exists() {
        assert!(
            Instant::now() < deadline,
            "parent did not release export gate"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let exporter = SqliteAuditExporter::with_clock(
        PathBuf::from(db_path),
        FixedClock {
            observed_at: ObservedAt::parse("2026-07-17T15:00:00.000Z").unwrap(),
        },
    );
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    let result = exporter.export_consistent(&RunId::parse("run-cross-process").unwrap(), &target);
    let outcome = match result {
        Ok(_) => "published",
        Err(ExportError::TargetNotEmpty) => "target-not-empty",
        Err(error) => panic!("unexpected child export error: {error}"),
    };
    fs::write(result_path, outcome).unwrap();
}

#[test]
fn concurrent_cross_process_publication_has_one_winner_and_target_not_empty_loser() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let conn = harness.connection();
    seed_run_with_history(&conn, "run-cross-process");
    drop(conn);

    let output = harness.export_dir("export-cross-process");
    let gate = harness._root.path().join("release-export");
    let executable = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    let mut ready_paths = Vec::new();
    let mut result_paths = Vec::new();
    for child_id in 0..2 {
        let ready = harness._root.path().join(format!("ready-{child_id}"));
        let result = harness._root.path().join(format!("result-{child_id}"));
        let child = Command::new(&executable)
            .arg("--exact")
            .arg("export_cross_process_worker")
            .arg("--nocapture")
            .env("LOOP_EXPORT_CHILD_DB", &harness.db_path)
            .env("LOOP_EXPORT_CHILD_TARGET", &output)
            .env("LOOP_EXPORT_CHILD_READY", &ready)
            .env("LOOP_EXPORT_CHILD_GATE", &gate)
            .env("LOOP_EXPORT_CHILD_RESULT", &result)
            .spawn()
            .unwrap();
        children.push(child);
        ready_paths.push(ready);
        result_paths.push(result);
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    while ready_paths.iter().any(|path| !path.exists()) {
        assert!(
            Instant::now() < deadline,
            "children did not reach export gate"
        );
        thread::sleep(Duration::from_millis(5));
    }
    fs::write(&gate, b"release").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    let mut outcomes = result_paths
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    outcomes.sort();
    assert_eq!(outcomes, ["published", "target-not-empty"]);
    verify_manifest_hashes(&output);
}

#[test]
fn export_rejects_concurrent_loser_after_winner_publishes() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let conn = harness.connection();
    seed_run_with_history(&conn, "run-concurrent-loser");
    drop(conn);

    let output = harness.export_dir("export-concurrent");
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&RunId::parse("run-concurrent-loser").unwrap(), &target)
        .unwrap();
    verify_manifest_hashes(&output);

    fs::write(
        output.join("staging-loser.txt"),
        b"would-be foreign if rename won",
    )
    .unwrap();

    let error = harness
        .exporter
        .export_consistent(&RunId::parse("run-concurrent-loser").unwrap(), &target)
        .unwrap_err();
    assert!(matches!(error, ExportError::TargetNotEmpty));
    verify_manifest_hashes(&output);
}

#[test]
fn export_rejects_invalid_inputs_json_shape() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-invalid-inputs";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    conn.execute(
        "UPDATE runs SET inputs_json = '[]' WHERE run_id = ?1",
        params![run_id],
    )
    .unwrap();
    drop(conn);

    let output = harness.export_dir("export-invalid-inputs");
    let error = harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::PersistenceFailed { .. }));
    assert!(!output.exists());
}

#[test]
fn export_rejects_invalid_evidence_metadata_json_shape() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-invalid-metadata";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    insert_evidence(
        &conn,
        run_id,
        "ev-invalid",
        "2026-07-17T12:00:00.000Z",
        "opaque:meta",
    );
    conn.execute(
        "UPDATE evidence SET metadata_json = ?1 WHERE evidence_id = ?2",
        params!["[]", "ev-invalid"],
    )
    .unwrap();
    drop(conn);

    let output = harness.export_dir("export-invalid-metadata");
    let error = harness
        .exporter
        .export_consistent(
            &RunId::parse(run_id).unwrap(),
            &ExportTarget::parse(output.to_str().unwrap()).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::PersistenceFailed { .. }));
    assert!(!output.exists());
}

#[test]
fn verify_export_directory_rejects_reordered_manifest_inventory() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-verify-reordered";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    drop(conn);

    let output = harness.export_dir("export-verify-reordered");
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&RunId::parse(run_id).unwrap(), &target)
        .unwrap();

    let manifest = parse_manifest(&output.join("manifest.json"));
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .expect("files");
    assert_eq!(files.len(), 2);
    let reordered = serde_json::json!({
        "export_manifest_schema_version": manifest["export_manifest_schema_version"],
        "export_schema_version": manifest["export_schema_version"],
        "exported_at": manifest["exported_at"],
        "files": [files[1].clone(), files[0].clone()],
        "run_id": manifest["run_id"],
    });
    fs::write(
        output.join("manifest.json"),
        canonical_json(&reordered).into_bytes(),
    )
    .unwrap();

    let error = verify_export_directory(&target, &RunId::parse(run_id).unwrap()).unwrap_err();
    assert!(matches!(error, ExportError::ResourceExhausted { .. }));
}

#[test]
fn verify_export_directory_rejects_noncanonical_manifest_bytes() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-verify-noncanonical";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    drop(conn);

    let output = harness.export_dir("export-verify-noncanonical");
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&RunId::parse(run_id).unwrap(), &target)
        .unwrap();

    let manifest: Value =
        serde_json::from_slice(&read_file(&output.join("manifest.json"))).unwrap();
    fs::write(
        output.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let error = verify_export_directory(&target, &RunId::parse(run_id).unwrap()).unwrap_err();
    assert!(matches!(error, ExportError::ResourceExhausted { .. }));
}

#[test]
fn verify_export_directory_rejects_malformed_manifest_json() {
    let harness = Harness::new("2026-07-17T15:00:00.000Z");
    let run_id = "run-verify-malformed";
    let conn = harness.connection();
    seed_run_with_history(&conn, run_id);
    drop(conn);

    let output = harness.export_dir("export-verify-malformed");
    let target = ExportTarget::parse(output.to_str().unwrap()).unwrap();
    harness
        .exporter
        .export_consistent(&RunId::parse(run_id).unwrap(), &target)
        .unwrap();
    fs::write(output.join("manifest.json"), b"{not json").unwrap();

    let error = verify_export_directory(&target, &RunId::parse(run_id).unwrap()).unwrap_err();
    assert!(matches!(error, ExportError::ResourceExhausted { .. }));
}
