//! Public-process regression coverage for the backlog's core CLI proof gaps.
//!
//! These cases deliberately use fresh `loop-engine` processes and a scripted
//! provider.  The provider records the bytes it receives, while the test-owned
//! SQLite trigger is the only fault injection used here.

use loop_core::{State, Transition, WorkSlot, Workflow};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

struct ProviderFixture {
    _root: TempDir,
    app_home: PathBuf,
    cwd_a: PathBuf,
    cwd_b: PathBuf,
    database: PathBuf,
    config: PathBuf,
    requests: PathBuf,
    entered: PathBuf,
    release: PathBuf,
    marker: PathBuf,
    defaults: bool,
}

impl ProviderFixture {
    fn new(workflow: Workflow) -> Self {
        Self::with_changed(workflow.clone(), workflow, false)
    }

    fn with_changed(original: Workflow, changed: Workflow, defaults: bool) -> Self {
        let root = tempdir().expect("fixture tempdir");
        let root_path = root.path();
        let app_home = root_path.join("app-home");
        let cwd_a = root_path.join("cwd-a");
        let cwd_b = root_path.join("cwd-b");
        let requests = root_path.join("provider-requests");
        fs::create_dir_all(&app_home).expect("app home");
        fs::create_dir_all(&cwd_a).expect("first cwd");
        fs::create_dir_all(&cwd_b).expect("second cwd");
        fs::create_dir_all(&requests).expect("provider request directory");

        let workflow_path = root_path.join("workflow.json");
        let changed_path = root_path.join("changed-workflow.json");
        fs::write(
            &workflow_path,
            serde_json::to_vec(&original).expect("workflow JSON"),
        )
        .expect("write workflow");
        fs::write(
            &changed_path,
            serde_json::to_vec(&changed).expect("changed workflow JSON"),
        )
        .expect("write changed workflow");

        let entered = root_path.join("provider-entered");
        let release = root_path.join("provider-release");
        let marker = root_path.join("use-changed-describe");
        let provider = root_path.join("provider.sh");
        let script = format!(
            r#"#!/bin/sh
set -eu
request_dir={requests}
workflow={workflow}
changed_workflow={changed}
marker={marker}
entered={entered}
release={release}
request=$(mktemp "$request_dir/request.XXXXXX")
cat > "$request"
python3 - "$request" "$workflow" "$changed_workflow" "$marker" "$entered" "$release" <<'PY'
import json
import pathlib
import sys
import time

request_path, workflow_path, changed_path, marker_path, entered_path, release_path = sys.argv[1:]
request = json.loads(pathlib.Path(request_path).read_text())
if request.get("operation") == "describe":
    selected = changed_path if pathlib.Path(marker_path).exists() else workflow_path
    print(pathlib.Path(selected).read_text(), end="")
    raise SystemExit(0)

if request.get("operation") != "evaluate":
    print(json.dumps({{"result": "unsupported"}}))
    raise SystemExit(0)

initial_input = request.get("initial_input")
if not isinstance(initial_input, dict):
    initial_input = {{}}
behavior = initial_input.get("fixture_behavior", "allow")
transition = request.get("transition", {{}})
source = transition.get("source")
event = transition.get("event")
prior = request.get("prior_evaluations", [])

if behavior in ("block-allow", "block-deny") and event == "approve":
    pathlib.Path(entered_path).write_text("entered")
    while not pathlib.Path(release_path).exists():
        time.sleep(0.01)


def allow(effect=False):
    response = {{"result": "allow"}}
    if effect:
        response["context_append"] = {{
            "kind": "provider-effect",
            "data": {{"source": "backlog-t01", "must_rollback": True}},
        }}
    print(json.dumps(response))


def deny(code):
    print(json.dumps({{
        "result": "deny",
        "feedback": {{"code": code, "message": code}},
    }}))


if behavior == "atomic-failure":
    allow(effect=True)
elif behavior == "lineage":
    if source == "start" and event == "approve":
        count = len(prior)
        if count == 0:
            deny("lineage-deny-1")
        elif count == 1:
            allow()
        elif count == 2:
            deny("lineage-deny-3")
        else:
            print(json.dumps({{"result": "unsupported"}}))
    elif event == "unsupported":
        print(json.dumps({{"result": "unsupported"}}))
    elif event == "fail":
        raise SystemExit(17)
    else:
        allow()
elif behavior == "block-deny" and event == "approve":
    deny("blocked-by-script")
elif behavior == "deny" and event == "approve":
    deny("blocked-by-script")
elif behavior == "unsupported" or event == "unsupported":
    print(json.dumps({{"result": "unsupported"}}))
elif behavior == "failure" or event == "fail":
    raise SystemExit(17)
elif behavior == "deny":
    deny("blocked-by-script")
else:
    allow()
PY
"#,
            requests = shell_quote(&requests.to_string_lossy()),
            workflow = shell_quote(&workflow_path.to_string_lossy()),
            changed = shell_quote(&changed_path.to_string_lossy()),
            marker = shell_quote(&marker.to_string_lossy()),
            entered = shell_quote(&entered.to_string_lossy()),
            release = shell_quote(&release.to_string_lossy()),
        );
        fs::write(&provider, script).expect("write scripted provider");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&provider, fs::Permissions::from_mode(0o755))
                .expect("make scripted provider executable");
        }

        let config = app_home.join("providers.toml");
        let command = toml_quote(&provider.to_string_lossy());
        fs::write(
            &config,
            format!("[providers.fixture]\ncommand = \"{command}\"\n"),
        )
        .expect("write provider catalog");

        Self {
            database: root_path.join("catalog.sqlite"),
            _root: root,
            app_home,
            cwd_a,
            cwd_b,
            config,
            requests,
            entered,
            release,
            marker,
            defaults,
        }
    }

    fn actual_default_database(&self) -> PathBuf {
        self.app_home.join("loop.db")
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn toml_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn workflow(states: Vec<State>, transitions: Vec<Transition>) -> Workflow {
    Workflow::new("backlog-t01", "start", states, transitions)
}

fn checked_workflow() -> Workflow {
    workflow(
        vec![
            State::new("start", "Start", "Original start instructions", false),
            State::new("middle", "Middle", "Middle instructions", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![Transition::checked("start", "approve", "middle")],
    )
}

fn race_workflow() -> Workflow {
    workflow(
        vec![
            State::new("start", "Start", "Evaluate the work", false),
            State::new("middle", "Middle", "Continue the work", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::checked("start", "approve", "middle"),
            Transition::checked("start", "retry", "middle"),
            Transition::checked("middle", "finish", "done"),
            Transition::check_free("start", "advance", "middle"),
        ],
    )
}

fn lineage_workflow() -> Workflow {
    workflow(
        vec![
            State::new("start", "Start", "Start instructions", false),
            State::new("review", "Review", "Review instructions", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![
            Transition::checked("start", "approve", "start"),
            Transition::checked("start", "other", "start"),
            Transition::check_free("start", "to-review", "review"),
            Transition::checked("start", "unsupported", "start"),
            Transition::checked("start", "fail", "start"),
            Transition::checked("review", "approve", "review"),
            Transition::checked("review", "finish", "done"),
        ],
    )
}

fn slotted_workflow(id: &str, title: &str, event: &str) -> Workflow {
    workflow(
        vec![
            State::new("start", title, "Original start instructions", false),
            State::new("done", "Done", "Finished", true),
        ],
        vec![Transition::checked("start", event, "done")],
    )
    .with_work_slots(vec![WorkSlot::new(id, "start", event)])
}

fn output_for(fixture: &ProviderFixture, cwd: &Path, parts: Vec<String>) -> Output {
    let mut command = command_for(fixture, cwd);
    command.args(parts);
    command.output().expect("run loop-engine subprocess")
}

fn command_for(fixture: &ProviderFixture, cwd: &Path) -> Command {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.current_dir(cwd);
    command
        .env("HOME", fixture.app_home.join("home"))
        .env("XDG_DATA_HOME", fixture.app_home.join("xdg-data"))
        .env("XDG_CONFIG_HOME", fixture.app_home.join("xdg-config"))
        .env("LOOP_ENGINE_HOME", &fixture.app_home)
        .env(
            "LOOP_ENGINE_OWNERSHIP_DIRECTORY",
            fixture.app_home.join("ownership"),
        );
    for name in [
        "LOOP_ENGINE_DATABASE",
        "LOOP_ENGINE_DATABASE_PATH",
        "LOOP_DATABASE",
        "LOOP_DATABASE_PATH",
        "LOOP_DB_PATH",
        "LOOP_ENGINE_DB",
        "LOOP_DB",
        "LOOP_ENGINE_PROVIDER_CONFIG",
        "LOOP_ENGINE_PROVIDER_CONFIG_PATH",
        "LOOP_PROVIDER_CONFIG",
        "LOOP_PROVIDER_CONFIG_PATH",
        "LOOP_ENGINE_CONFIG",
        "LOOP_ENGINE_CONFIG_PATH",
        "LOOP_CONFIG",
        "LOOP_CONFIG_PATH",
    ] {
        command.env_remove(name);
    }
    command
}

fn base_args(fixture: &ProviderFixture) -> Vec<String> {
    let mut args = vec!["--json".to_owned()];
    if !fixture.defaults {
        args.extend([
            "--database".to_owned(),
            fixture.database.to_string_lossy().into_owned(),
            "--config".to_owned(),
            fixture.config.to_string_lossy().into_owned(),
        ]);
    }
    args
}

fn cli(fixture: &ProviderFixture, parts: &[&str]) -> Value {
    cli_at(fixture, &fixture.cwd_a, parts)
}

fn cli_at(fixture: &ProviderFixture, cwd: &Path, parts: &[&str]) -> Value {
    let mut args = base_args(fixture);
    args.extend(parts.iter().map(|part| (*part).to_owned()));
    let output = output_for(fixture, cwd, args);
    parse_output(output)
}

fn cli_owned(fixture: &ProviderFixture, cwd: &Path, parts: Vec<String>) -> (Output, Value) {
    let mut args = base_args(fixture);
    args.extend(parts);
    let output = output_for(fixture, cwd, args);
    let value = parse_output(output.clone());
    (output, value)
}

fn spawn_cli(fixture: &ProviderFixture, cwd: &Path, parts: Vec<String>) -> Child {
    let mut args = base_args(fixture);
    args.extend(parts);
    let mut command = command_for(fixture, cwd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().expect("spawn loop-engine subprocess")
}

fn parse_output(output: Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.trim().is_empty(), "CLI emitted no JSON: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "CLI output was not JSON: {error}; stdout={stdout}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn start(fixture: &ProviderFixture, run_id: &str, initial_input: Value) -> Value {
    let input = serde_json::to_string(&initial_input).expect("initial input JSON");
    let mut parts = vec![
        "start".to_owned(),
        "fixture".to_owned(),
        input,
        "--id".to_owned(),
        run_id.to_owned(),
    ];
    let mut args = base_args(fixture);
    args.append(&mut parts);
    let output = output_for(fixture, &fixture.cwd_a, args);
    let value = parse_output(output);
    assert_eq!(value["status"], "completed", "start failed: {value}");
    value
}

fn start_at(fixture: &ProviderFixture, cwd: &Path, run_id: &str, initial_input: Value) -> Value {
    let input = serde_json::to_string(&initial_input).expect("initial input JSON");
    let parts = vec![
        "start".to_owned(),
        "fixture".to_owned(),
        input,
        "--id".to_owned(),
        run_id.to_owned(),
    ];
    let (output, value) = cli_owned(fixture, cwd, parts);
    assert_eq!(output.status.code(), Some(0), "start failed: {value}");
    assert_eq!(value["status"], "completed", "start failed: {value}");
    value
}

fn show_full(fixture: &ProviderFixture, run_id: &str) -> Value {
    cli(fixture, &["show", run_id, "--view", "full"])
}

fn history(fixture: &ProviderFixture, run_id: &str) -> Value {
    cli(fixture, &["history", run_id])
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for scripted provider marker {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn release_provider(fixture: &ProviderFixture) {
    fs::write(&fixture.release, b"release").expect("release scripted provider");
}

fn provider_requests(fixture: &ProviderFixture) -> Vec<Value> {
    let mut paths = fs::read_dir(&fixture.requests)
        .expect("read provider request directory")
        .map(|entry| entry.expect("provider request entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            serde_json::from_slice(&fs::read(path).expect("read provider request"))
                .expect("provider request JSON")
        })
        .collect()
}

fn evaluation_requests(fixture: &ProviderFixture) -> Vec<Value> {
    provider_requests(fixture)
        .into_iter()
        .filter(|request| request["operation"] == "evaluate")
        .collect()
}

fn assert_error_code(value: &Value, code: &str) {
    assert_eq!(value["status"], "error", "expected error: {value}");
    assert_eq!(value["code"], code, "unexpected error: {value}");
}

fn assert_rejected_code(value: &Value, code: &str) {
    assert_eq!(value["status"], "rejected", "expected rejection: {value}");
    assert_eq!(value["code"], code, "unexpected rejection: {value}");
}

#[test]
fn backlog_t01_graph_validation_public_cli_cases() {
    let invalid = [
        (
            "duplicate-state",
            workflow(
                vec![
                    State::new("start", "Start", "start", false),
                    State::new("start", "Again", "again", false),
                ],
                vec![],
            ),
            "duplicate-state-id",
        ),
        (
            "undefined-endpoint",
            workflow(
                vec![State::new("start", "Start", "start", false)],
                vec![Transition::check_free(
                    "missing-source",
                    "go",
                    "missing-target",
                )],
            ),
            "undefined-transition-source",
        ),
        (
            "undefined-target",
            workflow(
                vec![State::new("start", "Start", "start", false)],
                vec![Transition::check_free("start", "go", "missing-target")],
            ),
            "undefined-transition-target",
        ),
        (
            "duplicate-source-event",
            workflow(
                vec![
                    State::new("start", "Start", "start", false),
                    State::new("one", "One", "one", false),
                    State::new("two", "Two", "two", false),
                ],
                vec![
                    Transition::check_free("start", "go", "one"),
                    Transition::checked("start", "go", "two"),
                ],
            ),
            "duplicate-source-event",
        ),
    ];

    for (run_id, definition, code) in invalid {
        let fixture = ProviderFixture::new(definition);
        let input = serde_json::to_string(&json!({})).expect("input JSON");
        let (output, value) = cli_owned(
            &fixture,
            &fixture.cwd_a,
            vec![
                "start".to_owned(),
                "fixture".to_owned(),
                input,
                "--id".to_owned(),
                run_id.to_owned(),
            ],
        );
        assert_eq!(output.status.code(), Some(20), "invalid graph: {value}");
        assert_error_code(&value, code);
        let listed = cli(&fixture, &["list"]);
        assert_eq!(
            listed["status"], "completed",
            "list after rejection: {listed}"
        );
        assert!(listed["result"].as_array().unwrap().is_empty());
    }

    let permitted = [
        (
            "disconnected",
            workflow(
                vec![
                    State::new("start", "Start", "start", false),
                    State::new("next", "Next", "next", false),
                    State::new("orphan", "Orphan", "orphan", true),
                ],
                vec![Transition::check_free("start", "next", "next")],
            ),
        ),
        (
            "sink",
            workflow(vec![State::new("start", "Start", "sink", false)], vec![]),
        ),
        (
            "cyclic",
            workflow(
                vec![
                    State::new("start", "Start", "start", false),
                    State::new("loop", "Loop", "loop", false),
                ],
                vec![
                    Transition::check_free("start", "next", "loop"),
                    Transition::check_free("loop", "back", "start"),
                ],
            ),
        ),
        (
            "no-final",
            workflow(
                vec![
                    State::new("start", "Start", "start", false),
                    State::new("next", "Next", "next", false),
                ],
                vec![Transition::check_free("start", "next", "next")],
            ),
        ),
    ];

    for (run_id, definition) in permitted {
        let fixture = ProviderFixture::new(definition);
        let started = start(&fixture, run_id, json!({"case": run_id}));
        assert_eq!(started["result"]["run"]["id"], run_id);
        let shown = show_full(&fixture, run_id);
        assert_eq!(shown["status"], "completed", "permitted graph: {shown}");
        assert_eq!(shown["result"]["run_id"], run_id);
        assert_eq!(shown["result"]["lifecycle"], "active");
        assert!(shown["result"]["requestable_events"].is_array());
    }
}

#[test]
fn backlog_t01_history_failure_public_cli_is_atomic() {
    let fixture = ProviderFixture::new(checked_workflow());
    let run_id = "history-failure";
    start(
        &fixture,
        run_id,
        json!({"fixture_behavior":"atomic-failure","policy":"same-policy"}),
    );
    let before = show_full(&fixture, run_id);
    assert_eq!(before["result"]["state_visit"], 0);

    let connection = Connection::open(&fixture.database).expect("open test-owned SQLite");
    connection
        .execute_batch(
            "CREATE TRIGGER backlog_t01_fail_history
             BEFORE INSERT ON history_entries
             BEGIN
                 SELECT RAISE(ABORT, 'backlog_t01 forced history failure');
             END;",
        )
        .expect("install test-owned history failure");
    drop(connection);

    let (event_output, event) = cli_owned(
        &fixture,
        &fixture.cwd_b,
        vec!["event".to_owned(), run_id.to_owned(), "approve".to_owned()],
    );
    assert_eq!(event_output.status.code(), Some(20), "event: {event}");
    assert_error_code(&event, "persistence-failure");

    // This inspection is a new CLI process, not the process that attempted the
    // failed event.  The provider effect, state mutation, evaluation and both
    // history rows from the attempted transaction must all be absent.
    let after = cli_at(
        &fixture,
        &fixture.cwd_b,
        &["show", run_id, "--view", "full"],
    );
    assert_eq!(after["result"]["current_state"], "start");
    assert_eq!(after["result"]["lifecycle"], "active");
    assert_eq!(after["result"]["state_visit"], 0);
    assert!(after["result"]["context"].as_array().unwrap().is_empty());
    assert!(after["result"]["evaluation_history"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(after["result"]["latest_evaluations"]
        .as_array()
        .unwrap()
        .is_empty());
    let history = cli_at(&fixture, &fixture.cwd_b, &["history", run_id]);
    assert_eq!(history["result"].as_array().unwrap().len(), 1);
    assert_eq!(history["result"][0]["action"]["kind"], "run_created");
    let listed = cli_at(&fixture, &fixture.cwd_b, &["list"]);
    assert_eq!(listed["result"][0]["current_state"], "start");
}

#[test]
fn backlog_t01_exact_transition_lineage_public_cli() {
    let fixture = ProviderFixture::new(lineage_workflow());
    let run_id = "exact-lineage";
    start(
        &fixture,
        run_id,
        json!({
            "fixture_behavior":"lineage",
            "policy_ids":{"start":"shared-policy","review":"shared-policy"}
        }),
    );

    let event = |event: &str| -> Value {
        let _ = show_full(&fixture, run_id);
        let (output, value) = cli_owned(
            &fixture,
            &fixture.cwd_b,
            vec!["event".to_owned(), run_id.to_owned(), event.to_owned()],
        );
        assert!(!output.stdout.is_empty());
        value
    };

    let first = event("approve");
    assert_rejected_code(&first, "lineage-deny-1");
    assert_eq!(event("other")["status"], "completed");
    assert_eq!(event("approve")["status"], "completed");
    let third = event("approve");
    assert_rejected_code(&third, "lineage-deny-3");
    assert_error_code(&event("unsupported"), "provider-unsupported");
    assert_error_code(&event("fail"), "provider-execution-failed");
    assert_error_code(&event("approve"), "provider-unsupported");
    assert_eq!(event("to-review")["status"], "completed");
    assert_eq!(event("approve")["status"], "completed");

    let requests = evaluation_requests(&fixture);
    let mut start_approve = requests
        .iter()
        .filter(|request| {
            request["transition"]["source"] == "start"
                && request["transition"]["event"] == "approve"
        })
        .collect::<Vec<_>>();
    start_approve.sort_by_key(|request| request["prior_evaluations"].as_array().unwrap().len());
    assert_eq!(start_approve.len(), 4);
    let prior_lengths = start_approve
        .iter()
        .map(|request| request["prior_evaluations"].as_array().unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(prior_lengths, vec![0, 1, 2, 3]);
    assert_eq!(
        start_approve[3]["prior_evaluations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|evaluation| evaluation["result"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!({"result":"deny","feedback":{"code":"lineage-deny-1","message":"lineage-deny-1"}}),
            json!({"result":"allow"}),
            json!({"result":"deny","feedback":{"code":"lineage-deny-3","message":"lineage-deny-3"}}),
        ]
    );

    let other = requests
        .iter()
        .find(|request| request["transition"]["event"] == "other")
        .expect("other request");
    assert!(other["prior_evaluations"].as_array().unwrap().is_empty());
    let review_approve = requests
        .iter()
        .find(|request| {
            request["transition"]["source"] == "review"
                && request["transition"]["event"] == "approve"
        })
        .expect("review approve request");
    assert!(review_approve["prior_evaluations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        review_approve["initial_input"]["policy_ids"]["start"],
        "shared-policy"
    );
    assert_eq!(
        review_approve["initial_input"]["policy_ids"]["review"],
        "shared-policy"
    );

    let shown = show_full(&fixture, run_id);
    let evaluations = shown["result"]["evaluation_history"].as_array().unwrap();
    assert_eq!(evaluations.len(), 5);
    assert_eq!(
        evaluations
            .iter()
            .filter(|evaluation| {
                evaluation["transition"]["source"] == "start"
                    && evaluation["transition"]["event"] == "approve"
            })
            .count(),
        3
    );
    assert_eq!(
        evaluations
            .iter()
            .filter(|evaluation| evaluation["transition"]["event"] == "unsupported")
            .count(),
        0
    );
    assert_eq!(
        evaluations
            .iter()
            .filter(|evaluation| evaluation["transition"]["event"] == "fail")
            .count(),
        0
    );
    let sequences = evaluations
        .iter()
        .map(|evaluation| evaluation["sequence"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn backlog_t01_context_append_races_preserve_allow_and_deny_snapshots() {
    for (behavior, follow_up) in [("block-allow", "finish"), ("block-deny", "retry")] {
        let fixture = ProviderFixture::new(race_workflow());
        let run_id = format!("context-race-{behavior}");
        start(&fixture, &run_id, json!({"fixture_behavior":behavior}));
        let _ = show_full(&fixture, &run_id);

        let event_process = spawn_cli(
            &fixture,
            &fixture.cwd_a,
            vec!["event".to_owned(), run_id.clone(), "approve".to_owned()],
        );
        wait_for_file(&fixture.entered);

        for ordinal in [1, 2] {
            let record_id = format!("during-{behavior}-{ordinal}");
            let data = serde_json::to_string(&json!({"ordinal": ordinal})).unwrap();
            let (output, value) = cli_owned(
                &fixture,
                &fixture.cwd_b,
                vec![
                    "append".to_owned(),
                    run_id.clone(),
                    "context-race".to_owned(),
                    data,
                    "--record-id".to_owned(),
                    record_id.clone(),
                ],
            );
            assert_eq!(output.status.code(), Some(0), "append: {value}");
            assert_eq!(value["result"]["context"]["id"], record_id);
        }
        release_provider(&fixture);
        let event_output = event_process
            .wait_with_output()
            .expect("wait for context race event");
        let event = parse_output(event_output);
        if behavior == "block-allow" {
            assert_eq!(event["status"], "completed", "allow race: {event}");
        } else {
            assert_rejected_code(&event, "blocked-by-script");
        }

        let first_show = show_full(&fixture, &run_id);
        let context_ids = first_show["result"]["context"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            context_ids,
            vec![
                format!("during-{behavior}-1"),
                format!("during-{behavior}-2"),
            ]
        );
        // bookends:LE-128 — context-only appends stay outside the in-flight
        // evaluation snapshot while the later evaluation sees them in order.
        if behavior == "block-allow" {
            assert_eq!(first_show["result"]["current_state"], "middle");
        } else {
            assert_eq!(first_show["result"]["current_state"], "start");
        }

        let _ = show_full(&fixture, &run_id);
        let (follow_output, follow) = cli_owned(
            &fixture,
            &fixture.cwd_b,
            vec!["event".to_owned(), run_id.clone(), follow_up.to_owned()],
        );
        assert_eq!(follow_output.status.code(), Some(0), "follow-up: {follow}");
        assert_eq!(follow["status"], "completed");

        let requests = evaluation_requests(&fixture);
        assert_eq!(requests.len(), 2);
        let first_request = requests
            .iter()
            .find(|request| request["transition"]["event"] == "approve")
            .expect("blocked request");
        let later_request = requests
            .iter()
            .find(|request| request["transition"]["event"] == follow_up)
            .expect("follow-up request");
        assert!(first_request["context"].as_array().unwrap().is_empty());
        let later_ids = later_request["context"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(later_ids, context_ids);
        let final_show = show_full(&fixture, &run_id);
        assert_eq!(final_show["result"]["context"].as_array().unwrap().len(), 2);
        assert_eq!(
            final_show["result"]["evaluation_history"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}

#[test]
fn backlog_t01_state_and_lifecycle_races_invalidate_late_evaluations() {
    for (behavior, mutation) in [
        ("block-allow", "terminate"),
        ("block-deny", "terminate"),
        ("block-allow", "advance"),
        ("block-deny", "advance"),
    ] {
        let fixture = ProviderFixture::new(race_workflow());
        let run_id = format!("invalidation-{behavior}-{mutation}");
        start(&fixture, &run_id, json!({"fixture_behavior":behavior}));
        let _ = show_full(&fixture, &run_id);
        let event_process = spawn_cli(
            &fixture,
            &fixture.cwd_a,
            vec!["event".to_owned(), run_id.clone(), "approve".to_owned()],
        );
        wait_for_file(&fixture.entered);

        let (mutation_output, mutation_value) = if mutation == "terminate" {
            cli_owned(
                &fixture,
                &fixture.cwd_b,
                vec!["terminate".to_owned(), run_id.clone()],
            )
        } else {
            cli_owned(
                &fixture,
                &fixture.cwd_b,
                vec!["event".to_owned(), run_id.clone(), "advance".to_owned()],
            )
        };
        assert_eq!(
            mutation_output.status.code(),
            Some(0),
            "mutation: {mutation_value}"
        );
        assert_eq!(mutation_value["status"], "completed");
        release_provider(&fixture);
        let late_output = event_process
            .wait_with_output()
            .expect("wait for stale evaluation");
        let late = parse_output(late_output);
        assert_error_code(
            &late,
            if mutation == "terminate" {
                "lifecycle-conflict"
            } else {
                "control-revision-conflict"
            },
        );

        let shown = show_full(&fixture, &run_id);
        assert!(shown["result"]["context"].as_array().unwrap().is_empty());
        assert!(shown["result"]["evaluation_history"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            shown["result"]["lifecycle"],
            if mutation == "terminate" {
                "terminated"
            } else {
                "active"
            }
        );
        assert_eq!(
            shown["result"]["current_state"],
            if mutation == "terminate" {
                "start"
            } else {
                "middle"
            }
        );
        let history_before = history(&fixture, &run_id);
        assert_eq!(history_before["result"].as_array().unwrap().len(), 2);

        if mutation == "terminate" {
            assert!(shown["result"]["requestable_events"]
                .as_array()
                .unwrap()
                .is_empty());
            let data = serde_json::to_string(&json!({"after":"termination"})).unwrap();
            assert_rejected_code(
                &cli_owned(
                    &fixture,
                    &fixture.cwd_b,
                    vec![
                        "append".to_owned(),
                        run_id.clone(),
                        "late".to_owned(),
                        data,
                        "--record-id".to_owned(),
                        "after-termination".to_owned(),
                    ],
                )
                .1,
                "run-not-active",
            );
            assert_rejected_code(
                &cli_owned(
                    &fixture,
                    &fixture.cwd_b,
                    vec!["event".to_owned(), run_id.clone(), "approve".to_owned()],
                )
                .1,
                "run-not-active",
            );
            assert_rejected_code(
                &cli_owned(
                    &fixture,
                    &fixture.cwd_b,
                    vec!["terminate".to_owned(), run_id.clone()],
                )
                .1,
                "run-not-active",
            );
            let history_after = history(&fixture, &run_id);
            assert_eq!(
                history_after["result"].as_array().unwrap().len(),
                history_before["result"].as_array().unwrap().len()
            );
        }
    }
}

#[test]
fn backlog_t01_frozen_topology_and_work_slot_catalog_survive_provider_change() {
    let original = slotted_workflow("slot-original", "Original", "approve");
    let changed = slotted_workflow("slot-changed", "Changed", "review");
    let fixture = ProviderFixture::with_changed(original, changed, false);
    let old_input = json!({});
    start(&fixture, "frozen-old", old_input);
    let old_before = show_full(&fixture, "frozen-old");
    assert_eq!(old_before["result"]["workflow_id"], "backlog-t01");
    assert_eq!(old_before["result"]["current_state_title"], "Original");
    assert_eq!(old_before["result"]["work_slots"][0]["id"], "slot-original");
    assert_eq!(old_before["result"]["work_slots"][0]["event"], "approve");

    fs::write(&fixture.marker, b"changed").expect("switch provider description");
    let new_input = json!({});
    start(&fixture, "frozen-new", new_input);
    let new_show = show_full(&fixture, "frozen-new");
    assert_eq!(new_show["result"]["current_state_title"], "Changed");
    assert_eq!(new_show["result"]["work_slots"][0]["id"], "slot-changed");
    assert_eq!(new_show["result"]["work_slots"][0]["event"], "review");

    let old_after = show_full(&fixture, "frozen-old");
    assert_eq!(old_after["result"]["current_state_title"], "Original");
    assert!(old_after["result"]["current_state_instructions"]
        .as_str()
        .unwrap()
        .contains("Original start instructions"));
    assert_eq!(old_after["result"]["work_slots"][0]["id"], "slot-original");
    assert_eq!(old_after["result"]["work_slots"][0]["event"], "approve");
    assert!(old_after["result"]["requestable_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event"] == "approve"));
}

// bookends:LE-135 — fresh public CLI processes started from different working
// directories share the default catalog, allocate distinct artifact roots, and
// expose those runs through later list/show inspection.
#[test]
fn backlog_t01_default_catalog_and_caller_ids_survive_fresh_processes() {
    let fixture = ProviderFixture::with_changed(
        workflow(
            vec![
                State::new("start", "Start", "Start", false),
                State::new("done", "Done", "Done", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        ),
        workflow(
            vec![
                State::new("start", "Start", "Start", false),
                State::new("done", "Done", "Done", true),
            ],
            vec![Transition::check_free("start", "finish", "done")],
        ),
        true,
    );

    let first = start_at(
        &fixture,
        &fixture.cwd_a,
        "caller-run-one",
        json!({"objective":"one"}),
    );
    let first_artifact = first["result"]["run"]["initial_input"]["artifact_root"]
        .as_str()
        .expect("default artifact root")
        .to_owned();
    assert_eq!(first["result"]["run"]["id"], "caller-run-one");
    assert!(Path::new(&first_artifact).is_dir());
    assert_eq!(
        Path::new(&first_artifact).canonicalize().unwrap(),
        fixture
            .app_home
            .join("runs/caller-run-one")
            .canonicalize()
            .unwrap()
    );

    let second = start_at(
        &fixture,
        &fixture.cwd_b,
        "caller-run-two",
        json!({"objective":"two"}),
    );
    let second_artifact = second["result"]["run"]["initial_input"]["artifact_root"]
        .as_str()
        .expect("second default artifact root")
        .to_owned();
    assert_eq!(second["result"]["run"]["id"], "caller-run-two");
    assert!(Path::new(&second_artifact).is_dir());
    assert_ne!(first_artifact, second_artifact);

    let listed = cli_at(&fixture, &fixture.cwd_b, &["list"]);
    let listed_ids = listed["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(listed_ids, vec!["caller-run-one", "caller-run-two"]);
    assert!(listed["result"].as_array().unwrap().iter().all(|run| {
        run["artifact_root"]
            .as_str()
            .is_some_and(|path| Path::new(path).is_dir())
    }));
    assert!(fixture.actual_default_database().is_file());

    let _ = cli_at(
        &fixture,
        &fixture.cwd_b,
        &["show", "caller-run-one", "--view", "full"],
    );
    let data = serde_json::to_string(&json!({"from":"fresh-process"})).unwrap();
    let (append_output, append) = cli_owned(
        &fixture,
        &fixture.cwd_b,
        vec![
            "append".to_owned(),
            "caller-run-one".to_owned(),
            "note".to_owned(),
            data,
            "--record-id".to_owned(),
            "caller-record-one".to_owned(),
        ],
    );
    assert_eq!(append_output.status.code(), Some(0), "append: {append}");
    assert_eq!(append["result"]["run"]["id"], "caller-run-one");
    assert_eq!(append["result"]["context"]["id"], "caller-record-one");

    let inspected = cli_at(
        &fixture,
        &fixture.cwd_a,
        &["show", "caller-run-one", "--view", "full"],
    );
    assert_eq!(inspected["result"]["run_id"], "caller-run-one");
    assert_eq!(inspected["result"]["context"][0]["id"], "caller-record-one");
    let history = cli_at(&fixture, &fixture.cwd_a, &["history", "caller-run-one"]);
    assert!(history["result"].as_array().unwrap().iter().any(|entry| {
        entry["action"]["kind"] == "context_appended"
            && entry["action"]["context_record_id"] == "caller-record-one"
    }));
}
