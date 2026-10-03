use super::bounded_process::CommandExt;
use loop_core::{
    AppendContextRequest, CommitTransitionRequest, CreateRunRequest, EvaluationFeedback,
    InnerWorker, Lifecycle, Persistence, ProviderAssociation, RecordDenialRequest, State,
    Timestamp, Transition, WorkSlot, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn workflow() -> Workflow {
    Workflow::new(
        "stable-reference-workflow",
        "start",
        vec![State::new("start", "Start", "work", false)],
        vec![Transition::check_free("start", "finish", "start")],
    )
    .with_work_slots(vec![WorkSlot::new("review", "start", "finish")])
}

fn selected_worker(assignment: &str, path: &Path) -> InnerWorker {
    let mut worker = InnerWorker::new("/bin/worker", vec!["--review".to_owned()], 0);
    worker.assignment_id = assignment.to_owned();
    worker.selected_attempt = Some(2);
    worker.selected_output_sha256 = Some("sha256:selected-output".to_owned());
    worker.selected_output_path = Some(path.to_string_lossy().into_owned());
    worker.declared_output_contract = Some(json!({"type": "object"}));
    worker
}

fn create_run(persistence: &SqlitePersistence, run_id: &str) {
    persistence
        .create_run(CreateRunRequest::new(
            run_id,
            None,
            workflow(),
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            json!({
                "policy": "v1",
                "work_slot_bindings": {
                    "review": {"command": "/bin/worker", "args": ["--review"]}
                }
            }),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            None,
        ))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
}

fn create_completed_invocation(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    capture_dir: &Path,
    workers: Vec<InnerWorker>,
) {
    rusqlite::Connection::open(database).unwrap().execute(
        r#"INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json, instruction_digest, subject, waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at, capture_dir, inner_workers_json) VALUES (?1, ?2, 'review', '{"command":"/bin/worker","args":["--review"]}', 'instruction-v1', 'subject-v1', 0, 3, 1000, 'succeeded', 0, 4, ?3, ?4)"#,
        rusqlite::params![run_id, invocation_id, capture_dir.to_string_lossy(), serde_json::to_string(&workers).unwrap()],
    ).unwrap();
}

fn cli(database: &Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database path"),
            "--json",
        ])
        .args(args)
        .bounded_output("loop-engine stable references")
        .expect("loop-engine");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    (output.status.code().unwrap_or(-1), value, stdout)
}

fn show(database: &Path) -> Value {
    let (_, value, _) = cli(database, &["show", "--view", "full", "stable-run"]);
    value
}

#[test]
fn append_rejects_caller_spoof_and_legacy_new_operations() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    let capture = directory.path().join("capture");
    create_completed_invocation(
        &database,
        "stable-run",
        "invocation-1",
        &capture,
        vec![selected_worker(
            "axis-a",
            &capture.join("axis-a/attempts/2/stdout"),
        )],
    );

    let (_, spoofed, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"origin":{"kind":"selected-assignment-output","id":"invocation-1","assignment_id":"axis-a"},"loop_engine_origin":{"invocation_id":"fake","assignment_id":"axis-a","selected_attempt":99,"selected_output_sha256":"sha256:fake","selected_output_path":"capture/fake","capture_dir":"/tmp/fake","command":"fake","args":[],"binding":{"command":"fake","args":[]}}}"#,
            "--record-id",
            "spoofed",
        ],
    );
    assert_eq!(spoofed["status"], "rejected");
    assert_eq!(spoofed["code"], "selected-assignment-output-retired");

    for kind in ["unchanged-carry", "override-carry"] {
        let (_, retired, _) = cli(
            &database,
            &[
                "append",
                "stable-run",
                kind,
                r#"{"source_record_id":"old","invocation_id":"invocation-1","assignment_id":"axis-a","attesting_driver":{"name":"driver"}}"#,
            ],
        );
        assert_eq!(retired["status"], "rejected", "{retired}");
        assert_eq!(retired["code"], "carry-refused", "{retired}");
    }

    let (_, verbose, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "review-evidence",
            r#"{"originating_output":{"invocation_id":"invocation-1","assignment_id":"axis-a","selected_attempt":2,"sha256":"sha256:selected-output","path":"capture/axis-a/attempts/2/stdout"}}"#,
        ],
    );
    assert_eq!(verbose["status"], "rejected");
    assert_eq!(verbose["code"], "selected-output-linkage-refused");
}

#[test]
fn evidence_applicability_is_concise_same_run_context_and_stays_visible() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "stable-run");
    persistence
        .append_context(AppendContextRequest::new(
            "stable-run",
            "original-evidence",
            "review-evidence",
            json!({"axis": "axis-a", "result": "pass", "findings": "", "author": {"name": "reviewer"}}),
            Timestamp::from_unix_millis(2),
        ))
        .expect("append source evidence");

    let (_, applicability, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "evidence-applicability",
            r#"{"origin":{"kind":"context-record","id":"original-evidence"},"target":{"subject":"review.json","revision":"2","checkpoint":"checkpoint-2"},"attesting_driver":{"name":"driver-1","kind":"human"},"reason":"The reviewed policy and subject remain applicable."}"#,
            "--record-id",
            "applicability-1",
        ],
    );
    assert_eq!(applicability["status"], "completed", "{applicability}");
    assert_eq!(
        applicability["result"]["context"]["kind"],
        "evidence-applicability"
    );
    assert_eq!(
        applicability["result"]["context"]["data"],
        json!({
            "origin": {"kind": "context-record", "id": "original-evidence"},
            "target": {"subject": "review.json", "revision": "2", "checkpoint": "checkpoint-2"},
            "attesting_driver": {"name": "driver-1", "kind": "human"},
            "reason": "The reviewed policy and subject remain applicable."
        })
    );

    let (_, missing, _) = cli(
        &database,
        &[
            "append",
            "stable-run",
            "evidence-applicability",
            r#"{"origin":{"kind":"context-record","id":"missing"},"target":{"revision":"2"},"attesting_driver":{"name":"driver"},"reason":"reason"}"#,
        ],
    );
    assert_eq!(missing["status"], "rejected");
    assert_eq!(missing["code"], "evidence-applicability-refused");

    let shown = show(&database);
    let contexts = shown["result"]["context"].as_array().expect("contexts");
    assert!(contexts.iter().any(|record| {
        record["id"] == "original-evidence" && record["data"]["author"]["name"] == "reviewer"
    }));
    let current = contexts
        .iter()
        .find(|record| record["id"] == "applicability-1")
        .expect("applicability context");
    assert_eq!(current["data"]["origin"]["id"], "original-evidence");
    assert!(current["data"].get("overridden_inputs").is_none());
    assert!(current["data"].get("attested_dimensions").is_none());
}

#[test]
fn core_legacy_carry_requests_decode_but_are_not_executed() {
    let request = serde_json::from_value::<AppendContextRequest>(json!({
        "run_id": "run-1",
        "record_id": "record-1",
        "kind": "unchanged-carry",
        "data": {},
        "created_at": 1,
        "carry": {
            "source_record_id": "source",
            "invocation_id": "invocation",
            "assignment_id": "axis-a",
            "act": "unchanged",
            "attesting_driver": {"name": "driver"}
        }
    }))
    .expect("legacy request remains decodable");
    assert!(request.carry.is_some());
}

// --- Public CLI acceptance-lane regressions (AC-2) ---

/// A checked self-loop lets the tests seed durable workflow evaluations
/// without a provider, through the test-owned SQLite trigger only.
fn acceptance_workflow() -> Workflow {
    Workflow::new(
        "acceptance-workflow",
        "start",
        vec![State::new("start", "Start", "Do the work", false)],
        vec![
            Transition::checked("start", "check", "start"),
            Transition::check_free("start", "finish", "start"),
        ],
    )
}

fn create_acceptance_run(persistence: &SqlitePersistence, run_id: &str) {
    persistence
        .create_run(CreateRunRequest::new(
            run_id,
            None,
            acceptance_workflow(),
            ProviderAssociation::new(json!({"identity": "provider-v1"})),
            json!({}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            "provider-v1",
            None,
        ))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
}

fn create_completed_invocation_without_worker_output(
    database: &Path,
    run_id: &str,
    invocation_id: &str,
    capture_dir: &Path,
) {
    let workers: Vec<InnerWorker> = Vec::new();
    rusqlite::Connection::open(database).unwrap().execute(
        r#"INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json, instruction_digest, subject, waiter_pid, started_at, allowed_time_ms, status, exit_code, completed_at, capture_dir, inner_workers_json) VALUES (?1, ?2, 'review', '{"command":"/bin/worker","args":["--review"]}', 'instruction-v1', 'subject-v1', 0, 3, 1000, 'succeeded', 0, 4, ?3, ?4)"#,
        rusqlite::params![run_id, invocation_id, capture_dir.to_string_lossy(), serde_json::to_string(&workers).unwrap()],
    ).unwrap();
}

fn accepted_decision(locator: &str, driver: &str, target: Value) -> Value {
    json!({"acceptance": {
        "state": "accepted",
        "attesting_driver": driver,
        "source_locators": [locator],
        "target": target
    }})
}

fn append_decision(database: &Path, run_id: &str, data: &Value) -> String {
    let (code, envelope, _) = cli(
        database,
        &["append", run_id, "driver-decision", &data.to_string()],
    );
    assert_eq!(code, 0, "public append must succeed: {envelope}");
    assert_eq!(envelope["status"], "completed", "{envelope}");
    envelope["result"]["context"]["id"]
        .as_str()
        .expect("appended record id")
        .to_owned()
}

fn show_result(database: &Path, run_id: &str) -> Value {
    cli(database, &["show", "--view", "full", run_id]).1["result"].clone()
}

#[test]
fn public_show_projects_explicit_accepted_result_separate_from_execution_and_worker_output() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_acceptance_run(&persistence, "acceptance-run");
    let capture = directory.path().join("capture");
    create_completed_invocation_without_worker_output(
        &database,
        "acceptance-run",
        "invocation-1",
        &capture,
    );

    // Exit 0 execution completion is a separate lane and never becomes
    // acceptance on its own.
    let before = show_result(&database, "acceptance-run");
    assert_eq!(before["execution"]["state"], "succeeded");
    assert_eq!(before["acceptance"]["state"], "unknown");
    assert!(before["acceptance"]["reason"]
        .as_str()
        .expect("unknown reason")
        .contains("no explicit provider or driver acceptance"));
    assert_eq!(before["conformance"]["state"], "unknown");
    assert_eq!(before["acceptance"]["evidence"]["state"], "partial");

    let locator = capture
        .join("axis-a/attempts/2/stdout")
        .to_string_lossy()
        .into_owned();
    let record_id = append_decision(
        &database,
        "acceptance-run",
        &accepted_decision(
            &locator,
            "driver-1",
            json!({"run_id": "acceptance-run", "invocation_id": "invocation-1"}),
        ),
    );

    let after = show_result(&database, "acceptance-run");
    let acceptance = &after["acceptance"];
    assert_eq!(acceptance["state"], "accepted");
    assert_eq!(acceptance["attributed_to"], "driver-1");
    let locators = acceptance["evidence"]["source_locators"]
        .as_array()
        .expect("evidence locators");
    assert!(locators
        .iter()
        .any(|value| value == &json!(format!("full.context[{record_id}]"))));
    assert!(locators.iter().any(|value| value == &json!(locator)));
    assert!(acceptance["meaning"]
        .as_str()
        .expect("acceptance meaning")
        .contains("not inferred"));
    assert_eq!(acceptance["uncertainty"]["state"], "present");

    // Bounded status does not interpret the full decision history; its locator
    // directs the caller to the full, attributed decision above.
    let (_, status, _) = cli(&database, &["show", "--view", "status", "acceptance-run"]);
    assert_eq!(status["result"]["acceptance"]["state"], "unknown");
    assert_eq!(
        status["result"]["locators"]["full"],
        json!([
            "loop-engine",
            "--database",
            database,
            "--json",
            "show",
            "acceptance-run",
            "--view",
            "full"
        ])
    );
}

#[test]
fn public_show_acceptance_stays_unknown_for_arbitrary_conflicting_stale_and_mismatched_sources() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_acceptance_run(&persistence, "plain-run");
    create_acceptance_run(&persistence, "conflict-run");
    create_acceptance_run(&persistence, "stale-run");
    create_acceptance_run(&persistence, "mismatch-run");
    create_completed_invocation_without_worker_output(
        &database,
        "mismatch-run",
        "invocation-1",
        &directory.path().join("capture"),
    );

    // Arbitrary provider text is not an acceptance decision.
    let (code, envelope, _) = cli(
        &database,
        &[
            "append",
            "plain-run",
            "review-evidence",
            r#"{"result":"pass","findings":""}"#,
        ],
    );
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(
        show_result(&database, "plain-run")["acceptance"]["state"],
        "unknown"
    );

    // Accepted and rejected decisions without an unambiguous current target
    // stay unknown instead of picking a side.
    append_decision(
        &database,
        "conflict-run",
        &accepted_decision("/tmp/one", "driver-1", json!({"run_id": "conflict-run"})),
    );
    append_decision(
        &database,
        "conflict-run",
        &json!({"acceptance": {
            "state": "rejected",
            "attesting_driver": "driver-1",
            "source_locators": ["/tmp/two"],
            "target": {"run_id": "conflict-run"}
        }}),
    );
    let conflict = show_result(&database, "conflict-run");
    assert_eq!(conflict["acceptance"]["state"], "unknown");
    assert!(conflict["acceptance"]["reason"]
        .as_str()
        .expect("conflict reason")
        .contains("conflicting"));

    // A stale decision does not populate acceptance.
    append_decision(
        &database,
        "stale-run",
        &json!({"acceptance": {
            "state": "accepted",
            "attesting_driver": "driver-1",
            "source_locators": ["/tmp/stale"],
            "freshness": {"state": "stale"}
        }}),
    );
    assert_eq!(
        show_result(&database, "stale-run")["acceptance"]["state"],
        "unknown"
    );

    // A decision naming another invocation stays unknown for this run.
    append_decision(
        &database,
        "mismatch-run",
        &accepted_decision(
            "/tmp/elsewhere",
            "driver-1",
            json!({"run_id": "mismatch-run", "invocation_id": "invocation-9"}),
        ),
    );
    let mismatch = show_result(&database, "mismatch-run");
    assert_eq!(mismatch["acceptance"]["state"], "unknown");
    assert!(mismatch["acceptance"]["reason"]
        .as_str()
        .expect("mismatch reason")
        .contains("not the selected invocation"));
}

#[test]
fn public_show_keeps_workflow_outcomes_and_acceptance_independent_lanes() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_acceptance_run(&persistence, "allow-run");
    create_acceptance_run(&persistence, "deny-run");
    let transition = Transition::checked("start", "check", "start");

    let allow_revision = persistence
        .load_authoritative_run(&"allow-run".into())
        .expect("allow run")
        .control_revision;
    persistence
        .commit_transition(CommitTransitionRequest::new(
            "allow-run",
            allow_revision,
            "start",
            transition.clone(),
            Lifecycle::Active,
        ))
        .expect("allow checked self-loop");
    let allowed = show_result(&database, "allow-run");
    assert_eq!(
        allowed["acceptance"]["workflow_transition"]["state"],
        "accepted"
    );
    assert!(allowed["acceptance"]["workflow_transition_meaning"]
        .as_str()
        .expect("transition meaning")
        .contains("not a domain-result acceptance"));
    assert_eq!(allowed["acceptance"]["state"], "unknown");

    let deny_revision = persistence
        .load_authoritative_run(&"deny-run".into())
        .expect("deny run")
        .control_revision;
    persistence
        .record_denial(RecordDenialRequest::new(
            "deny-run",
            deny_revision,
            "start",
            transition,
            EvaluationFeedback::new("missing-proof", "fixture denial"),
        ))
        .expect("deny checked self-loop");
    let denied = show_result(&database, "deny-run");
    assert_eq!(
        denied["acceptance"]["workflow_transition"]["state"],
        "rejected"
    );
    assert_eq!(denied["acceptance"]["state"], "unknown");

    // An explicit attributed decision populates only the acceptance lane.
    append_decision(
        &database,
        "deny-run",
        &accepted_decision("/tmp/evidence", "driver-1", json!({"run_id": "deny-run"})),
    );
    let decided = show_result(&database, "deny-run");
    assert_eq!(decided["acceptance"]["state"], "accepted");
    assert_eq!(
        decided["acceptance"]["workflow_transition"]["state"],
        "rejected"
    );
}

fn monitor_snapshot(database: &Path, run_id: &str, observation: Option<&Path>) -> Value {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.args([
        "monitor",
        "--database",
        database.to_str().expect("utf-8 database"),
        "--json",
        "--poll-seconds",
        "0.05",
        "--run",
        run_id,
    ]);
    if let Some(observation) = observation {
        command.args([
            "--observation",
            observation.to_str().expect("utf-8 observation"),
        ]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::bounded_process::prepare_process_group(&mut command);
    let mut child = command.spawn().expect("spawn monitor");
    let pgid = child.id() as i32;
    let stdout = child.stdout.take().expect("monitor stdout piped");
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(row) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if send.send(row).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut snapshot = None;
    while Instant::now() < deadline {
        match receive.recv_timeout(Duration::from_millis(250)) {
            Ok(row) => {
                if row["event"] == "snapshot"
                    && row["acceptance"].is_object()
                    && row["acceptance"]["state"].is_string()
                {
                    snapshot = Some(row);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // Stopping the observer never cancels observed work; only the observer
    // process group is terminated after the first source snapshot.
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.wait();
    drop(receive);
    let _ = reader.join();
    snapshot.expect("monitor emitted a source snapshot with acceptance lanes")
}

#[test]
fn public_monitor_projects_accepted_observation_and_stays_unknown_without_it() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_acceptance_run(&persistence, "monitor-run");
    let capture = directory.path().join("capture");
    create_completed_invocation_without_worker_output(
        &database,
        "monitor-run",
        "invocation-1",
        &capture,
    );
    let locator = capture
        .join("axis-a/attempts/2/stdout")
        .to_string_lossy()
        .into_owned();
    let observation = directory.path().join("dated-observation.json");
    fs::write(
        &observation,
        serde_json::to_vec(&json!({
            "run_id": "monitor-run",
            "sampled_at_ms": 123,
            "attesting_driver": "driver-1",
            "judgment": {"result": "accepted", "source_locators": [locator]}
        }))
        .expect("observation json"),
    )
    .expect("write observation");

    let accepted = monitor_snapshot(&database, "monitor-run", Some(&observation));
    assert_eq!(accepted["event"], "snapshot");
    assert_eq!(accepted["acceptance"]["state"], "accepted");
    assert_eq!(accepted["acceptance"]["attributed_to"], "driver-1");
    let locators = accepted["acceptance"]["evidence"]["source_locators"]
        .as_array()
        .expect("evidence locators");
    assert!(locators.iter().any(|value| value
        .as_str()
        .is_some_and(|text| text.starts_with("observation:"))));
    assert!(locators.iter().any(|value| value == &json!(locator)));
    assert_eq!(accepted["acceptance"]["uncertainty"]["state"], "present");
    // External execution is not sampled; attributed acceptance stays separate.
    assert_eq!(accepted["execution"]["state"], "unknown");
    assert_ne!(accepted["acceptance"], accepted["execution"]);

    let without = monitor_snapshot(&database, "monitor-run", None);
    assert_eq!(without["acceptance"]["state"], "unknown");
    assert!(without["acceptance"]["reason"]
        .as_str()
        .expect("unknown reason")
        .contains("no explicit"));
}

#[test]
fn public_show_projects_explicit_rejected_decision_and_keeps_lanes_separate() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_acceptance_run(&persistence, "rejected-run");
    let capture = directory.path().join("capture");
    create_completed_invocation_without_worker_output(
        &database,
        "rejected-run",
        "invocation-1",
        &capture,
    );

    // A clean explicit rejection is projected with its attribution and
    // evidence, and stays separate from the execution lane.
    let locator = capture
        .join("axis-a/attempts/2/stdout")
        .to_string_lossy()
        .into_owned();
    append_decision(
        &database,
        "rejected-run",
        &json!({"acceptance": {
            "state": "rejected",
            "attesting_driver": "driver-1",
            "source_locators": [locator],
            "target": {"run_id": "rejected-run", "invocation_id": "invocation-1"}
        }}),
    );
    let after = show_result(&database, "rejected-run");
    let acceptance = &after["acceptance"];
    assert_eq!(acceptance["state"], "rejected");
    assert_eq!(acceptance["attributed_to"], "driver-1");
    let locators = acceptance["evidence"]["source_locators"]
        .as_array()
        .expect("evidence locators");
    assert!(locators.iter().any(|value| value == &json!(locator)));
    assert!(acceptance["meaning"]
        .as_str()
        .expect("acceptance meaning")
        .contains("not inferred"));
    // Machine execution completion is a separate lane and never inherits the
    // domain rejection.
    assert_eq!(after["execution"]["state"], "succeeded");
    assert_ne!(acceptance, &after["execution"]);
    let (_, status, _) = cli(&database, &["show", "--view", "status", "rejected-run"]);
    assert_eq!(status["result"]["acceptance"]["state"], "unknown");
    assert_eq!(
        status["result"]["locators"]["full"],
        json!([
            "loop-engine",
            "--database",
            database,
            "--json",
            "show",
            "rejected-run",
            "--view",
            "full"
        ])
    );
}
