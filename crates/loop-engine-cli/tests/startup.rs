use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

fn isolated_home() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp home")
}

fn command_with_home(home: &Path) -> Command {
    let mut command = Command::cargo_bin("loop-engine").expect("loop-engine binary");
    command.env("LOOP_ENGINE_HOME", home).env_remove("HOME");
    command
}

fn read_trace_events(trace_dir: &Path) -> Vec<Value> {
    let traces = fs::read_dir(trace_dir)
        .expect("trace directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .map(|entry| fs::read_to_string(entry.path()).expect("trace file"))
        .collect::<Vec<_>>();
    assert_eq!(traces.len(), 1, "expected exactly one trace file");
    traces[0]
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonl line"))
        .collect()
}

fn event_key(event: &Value) -> (&str, &str) {
    (
        event["category"].as_str().expect("category"),
        event["event"].as_str().expect("event"),
    )
}

fn assert_trace_lifecycle(events: &[Value], expected_finish: i64) {
    let keys = events.iter().map(event_key).collect::<Vec<_>>();
    assert_eq!(keys.first(), Some(&("invocation", "start")));
    assert_eq!(keys.last(), Some(&("invocation", "finish")));
    assert_eq!(
        events.last().and_then(|event| event["exit_code"].as_i64()),
        Some(expected_finish)
    );
}

#[test]
fn help_creates_expected_trace_before_dispatch() {
    let home = isolated_home();
    let output = command_with_home(home.path())
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("stdout utf8");
    assert!(stdout.contains("Global options"));

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 0);
    assert!(
        events
            .iter()
            .any(|event| { event_key(event) == ("driver", "metadata") && event["kind"] == "help" })
    );
    assert!(events[0]["argv"].is_array());
    assert!(events[0].get("argv_digest").is_none());
    assert_eq!(
        fs::read_dir(home.path().join("traces"))
            .expect("trace dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
            .map(|entry| fs::metadata(entry.path()).expect("trace metadata"))
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .next(),
        Some(0o600)
    );
}

#[test]
fn version_creates_expected_trace_before_dispatch() {
    let home = isolated_home();
    command_with_home(home.path())
        .arg("--version")
        .assert()
        .success()
        .stdout("loop-engine 0.1.0\n");

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 0);
    assert!(
        events.iter().any(|event| {
            event_key(event) == ("driver", "metadata") && event["kind"] == "version"
        })
    );
}

#[test]
fn parse_error_creates_expected_trace_and_no_database() {
    let home = isolated_home();
    command_with_home(home.path())
        .args(["--format", "json", "--unknown-flag"])
        .assert()
        .code(64)
        .stdout("");

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 64);
    assert!(
        events
            .iter()
            .any(|event| event_key(event) == ("parse", "failure"))
    );
    assert!(!home.path().join("state.db").exists());
}

#[test]
fn config_error_creates_expected_trace_and_no_database() {
    let home = isolated_home();
    fs::write(
        home.path().join("config.toml"),
        "schema_version = 2\n[defaults]\n",
    )
    .expect("write config");

    command_with_home(home.path())
        .args(["--format", "json", "run", "list"])
        .assert()
        .code(64)
        .stdout("");

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 64);
    assert!(events.iter().any(|event| {
        event_key(event) == ("invocation", "error") && event["phase"] == "config"
    }));
    assert!(!home.path().join("state.db").exists());
}

#[test]
fn list_operations_json_reports_exposed_routes_and_trace_lifecycle() {
    let home = isolated_home();
    let output = command_with_home(home.path())
        .args(["--format", "json", "--list-operations"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("stdout utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("driver json");
    assert_eq!(payload["kind"], "operation_list");
    assert_eq!(
        payload["operations"],
        json!([
            {
                "id": "provider.add",
                "argv": "provider add <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]"
            },
            {
                "id": "provider.list",
                "argv": "provider list [--enabled] [--tombstoned] [--active-runs-for <REGISTRATION-ID>] [--cursor <CURSOR>] [--limit <COUNT>]"
            }
        ])
    );

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 0);
    assert!(events.iter().any(|event| {
        event_key(event) == ("driver", "metadata") && event["kind"] == "list_operations"
    }));
    assert!(!home.path().join("state.db").exists());
}

#[test]
fn list_operations_human_reports_exposed_routes_and_trace_lifecycle() {
    let home = isolated_home();
    let output = command_with_home(home.path())
        .arg("--list-operations")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("stdout utf8");
    assert_eq!(
        stdout,
        concat!(
            "provider.add\tprovider add <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]\n",
            "provider.list\tprovider list [--enabled] [--tombstoned] [--active-runs-for <REGISTRATION-ID>] [--cursor <CURSOR>] [--limit <COUNT>]\n",
        )
    );

    let events = read_trace_events(&home.path().join("traces"));
    assert_trace_lifecycle(&events, 0);
    assert!(events.iter().any(|event| {
        event_key(event) == ("driver", "metadata") && event["kind"] == "list_operations"
    }));
    assert!(!home.path().join("state.db").exists());
}

#[test]
fn application_argv_is_rejected_before_database_open() {
    let home = isolated_home();
    command_with_home(home.path())
        .args(["--format", "json", "run", "list"])
        .assert()
        .code(64)
        .stdout("");

    assert!(!home.path().join("state.db").exists());
}

#[test]
fn trace_init_failure_emits_rich_stderr_and_does_no_database_work() {
    let home = isolated_home();
    fs::create_dir_all(home.path()).expect("home dir");
    fs::write(home.path().join("traces"), b"blocked").expect("block traces path");

    let output = command_with_home(home.path())
        .args(["--format", "json", "--help"])
        .assert()
        .code(64);
    assert!(output.get_output().stdout.is_empty());
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("stderr utf8");
    assert!(stderr.contains("\"phase\":\"trace_init\""));
    assert!(stderr.contains("failed to initialize operational trace"));

    assert!(!home.path().join("state.db").exists());
    let reserve_dir = home.path().join("traces/.reserve");
    assert!(!reserve_dir.exists() || fs::read_dir(reserve_dir).unwrap().count() == 0);
}

#[test]
fn trace_request_id_matches_filename_and_driver_output() {
    let home = isolated_home();
    let assert = command_with_home(home.path())
        .args(["--format", "json", "--version"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("driver json");
    let request_id = payload["request_id"].as_str().expect("request id");
    let trace_path = PathBuf::from(payload["trace"].as_str().expect("trace path"));
    assert_eq!(
        trace_path,
        home.path()
            .join("traces")
            .join(format!("{request_id}.jsonl"))
    );

    let events = read_trace_events(&home.path().join("traces"));
    assert!(
        events
            .iter()
            .all(|event| event["request_id"].as_str() == Some(request_id))
    );
}
