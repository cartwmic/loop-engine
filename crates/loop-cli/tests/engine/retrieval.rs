//! Black-box proof for run-context retrieval (bookends:LE-148): context headers,
//! exact records, history kinds, list filters and quiet closed pipes, all
//! through the binary.
use super::bounded_process::CommandExt;
use loop_core::{
    AppendContextRequest, CreateRunRequest, Lifecycle, Persistence, ProviderAssociation, State,
    Timestamp, Transition, Workflow,
};
use loop_integrations::SqlitePersistence;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn create_run(persistence: &SqlitePersistence, run_id: &str, label: Option<&str>, provider: &str) {
    persistence
        .create_run(CreateRunRequest::new(
            run_id,
            label.map(str::to_owned),
            Workflow::new(
                "retrieval-workflow",
                "start",
                vec![State::new("start", "Start", "work", false)],
                vec![Transition::check_free("start", "finish", "start")],
            ),
            ProviderAssociation::new(json!({"identity": provider})),
            json!({}),
            "start",
            Lifecycle::Active,
            Timestamp::from_unix_millis(1),
            provider,
            None,
        ))
        .expect("create run");
    persistence
        .load_show_data(&run_id.into())
        .expect("observe run");
}

fn append(persistence: &SqlitePersistence, run_id: &str, id: &str, kind: &str, data: Value) {
    persistence
        .append_context(AppendContextRequest::new(
            run_id,
            id,
            kind,
            data,
            Timestamp::from_unix_millis(2),
        ))
        .expect("append");
}

fn cli(database: &Path, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(workspace_integration::binary("loop-engine"))
        .args(["--database", database.to_str().expect("database"), "--json"])
        .args(args)
        .bounded_output("loop-engine retrieval")
        .expect("loop-engine");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    (output.status.code().unwrap_or(-1), value, stdout)
}

fn seeded(directory: &Path) -> std::path::PathBuf {
    let database = directory.join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "ctx-run", Some("alpha-review"), "provider-a");
    let transcript = "t".repeat(20_000);
    append(
        &persistence,
        "ctx-run",
        "evidence-1",
        "review-evidence",
        json!({"axis": "a", "result": "pass", "round": 3, "ok": true, "none": null,
            "long": "x".repeat(200), "nested": {"findings": ["f1", "f2"]},
            "transcript": transcript}),
    );
    append(
        &persistence,
        "ctx-run",
        "note-1",
        "driver-note",
        json!({"text": "short"}),
    );
    append(
        &persistence,
        "ctx-run",
        "evidence-2",
        "review-evidence",
        json!({"axis": "b", "result": "fail", "nested": {"findings": ["f3"]}}),
    );
    database
}

// bookends:LE-148 — retrieval of run context is a bounded read through the public binary, with no whole-run payload.
#[test]
fn context_headers_page_filter_and_stay_small() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());

    let (code, headers, raw_headers) = cli(&database, &["read", "ctx-run", "--kind", "context"]);
    assert_eq!(code, 0, "{headers}");
    let result = &headers["result"];
    assert_eq!(result["total"], 3);
    let ids: Vec<_> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["evidence-1", "note-1", "evidence-2"]);
    let first = &result["items"][0];
    assert_eq!(first["kind"], "review-evidence");
    assert_eq!(first["scalars"]["result"], "pass");
    assert_eq!(first["scalars"]["round"], 3);
    assert_eq!(first["scalars"]["ok"], true);
    assert!(first["scalars"].as_object().unwrap().contains_key("none"));
    assert!(first["scalars"]["none"].is_null());
    assert!(first["scalars"].get("nested").is_none(), "{first}");
    let long = first["scalars"]["long"].as_str().unwrap();
    assert_eq!(long.chars().count(), 81, "80 characters plus an ellipsis");
    assert!(first["data_bytes"].as_u64().unwrap() > 20_000);

    let (_, full, raw_full) = cli(&database, &["show", "--view", "full", "ctx-run"]);
    assert_eq!(full["result"]["context"].as_array().unwrap().len(), 3);
    assert!(
        raw_headers.len() * 10 < raw_full.len(),
        "headers {} bytes vs full show {} bytes",
        raw_headers.len(),
        raw_full.len()
    );

    let (_, filtered, _) = cli(
        &database,
        &[
            "read",
            "ctx-run",
            "--kind",
            "context",
            "--record-kind",
            "review-evidence",
        ],
    );
    assert_eq!(filtered["result"]["total"], 2);

    let (_, page, _) = cli(
        &database,
        &["read", "ctx-run", "--kind", "context", "--limit", "2"],
    );
    assert_eq!(page["result"]["truncated"], true);
    let cursor = page["result"]["next_cursor"].as_u64().unwrap().to_string();
    let (_, rest, _) = cli(
        &database,
        &["read", "ctx-run", "--kind", "context", "--cursor", &cursor],
    );
    assert_eq!(rest["result"]["items"][0]["id"], "evidence-2");
    assert_eq!(rest["result"]["truncated"], false);
}

// bookends:LE-148 — retrieval of run context is a bounded read through the public binary, with no whole-run payload.
#[test]
fn record_read_matches_full_show_and_pointer_slices_it() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());
    let (_, full, _) = cli(&database, &["show", "--view", "full", "ctx-run"]);
    let expected = full["result"]["context"][0].clone();

    let (code, record, _) = cli(
        &database,
        &["read", "ctx-run", "--kind", "record", "--id", "evidence-1"],
    );
    assert_eq!(code, 0, "{record}");
    assert_eq!(record["result"]["value"], expected);

    let (_, slice, _) = cli(
        &database,
        &[
            "read",
            "ctx-run",
            "--kind",
            "record",
            "--id",
            "evidence-1",
            "--pointer",
            "/data/nested/findings/1",
        ],
    );
    assert_eq!(slice["result"]["value"], "f2");

    let (code, bad, _) = cli(
        &database,
        &[
            "read",
            "ctx-run",
            "--kind",
            "record",
            "--id",
            "evidence-1",
            "--pointer",
            "/data/nested/missing",
        ],
    );
    assert_ne!(code, 0);
    assert_eq!(bad["code"], "record-pointer-not-found", "{bad}");
    assert!(bad["message"].as_str().unwrap().contains("findings"));

    let (code, unknown, _) = cli(
        &database,
        &["read", "ctx-run", "--kind", "record", "--id", "evidence"],
    );
    assert_ne!(code, 0);
    assert_eq!(unknown["code"], "record-not-found", "{unknown}");
    assert!(unknown["message"].as_str().unwrap().contains("evidence-2"));

    let (code, missing, _) = cli(&database, &["read", "ctx-run", "--kind", "record"]);
    assert_ne!(code, 0);
    assert_eq!(missing["code"], "invalid-invocation", "{missing}");
}

// bookends:LE-148 — retrieval of run context is a bounded read through the public binary, with no whole-run payload.
#[test]
fn history_entries_name_the_context_kind() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());

    let (_, history, _) = cli(&database, &["history", "ctx-run"]);
    let entries = history["result"].as_array().unwrap();
    let kinds: Vec<_> = entries
        .iter()
        .filter(|entry| entry["action"]["kind"] == "context_appended")
        .map(|entry| entry["context_kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["review-evidence", "driver-note", "review-evidence"]);
    assert!(entries
        .iter()
        .filter(|entry| entry["action"]["kind"] != "context_appended")
        .all(|entry| entry.get("context_kind").is_none()));

    let (_, read, _) = cli(&database, &["read", "ctx-run", "--kind", "history"]);
    let kinds: Vec<_> = read["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["action"]["kind"] == "context_appended")
        .map(|item| item["context_kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["review-evidence", "driver-note", "review-evidence"]);
}

// bookends:LE-148 — reads are bounded: page limits, wrong-kind options and malformed pointers refuse.
#[test]
fn malformed_requests_and_pointers_refuse() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());
    let refused = |args: &[&str], code: &str| {
        let (status, value, _) = cli(&database, args);
        assert_ne!(status, 0, "{value}");
        assert_eq!(value["code"], code, "{value}");
    };
    refused(
        &["read", "ctx-run", "--kind", "context", "--limit", "0"],
        "invalid-read-limit",
    );
    refused(
        &["read", "ctx-run", "--kind", "context", "--limit", "501"],
        "invalid-read-limit",
    );
    refused(
        &["read", "ctx-run", "--kind", "history", "--record-kind", "x"],
        "invalid-invocation",
    );
    refused(
        &["read", "ctx-run", "--kind", "context", "--id", "x"],
        "invalid-invocation",
    );
    refused(
        &["read", "ctx-run", "--kind", "context", "--pointer", "/data"],
        "invalid-invocation",
    );
    let record = [
        "read",
        "ctx-run",
        "--kind",
        "record",
        "--id",
        "evidence-1",
        "--pointer",
    ];
    // Array indexes are canonical; escapes are only ~0 and ~1.
    for pointer in [
        "/data/nested/findings/01",
        "/data/nested/findings/+1",
        "/data/nested/findings/-",
        "/data/nested/findings/2",
        "/data/nested~2",
        "data/nested",
    ] {
        let mut args = record.to_vec();
        args.push(pointer);
        let (status, value, _) = cli(&database, &args);
        assert_ne!(status, 0, "{pointer}: {value}");
        assert_eq!(
            value["code"], "record-pointer-not-found",
            "{pointer}: {value}"
        );
    }
    let mut args = record.to_vec();
    args.push("/data/nested/findings/0");
    assert_eq!(cli(&database, &args).1["result"]["value"], "f1");
}

// bookends:LE-148 — a ten-megabyte run still answers header reads through the public binary.
#[test]
fn context_headers_on_a_ten_megabyte_run_complete() {
    let directory = tempdir().expect("tempdir");
    let database = directory.path().join("loop.sqlite");
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "big-run", None, "provider-a");
    for index in 0..100 {
        append(
            &persistence,
            "big-run",
            &format!("record-{index}"),
            "review-evidence",
            json!({"axis": index, "transcript": "x".repeat(100_000)}),
        );
    }
    drop(persistence);
    let started = std::time::Instant::now();
    let (code, headers, raw) = cli(&database, &["read", "big-run", "--kind", "context"]);
    let elapsed = started.elapsed();
    assert_eq!(code, 0, "{headers}");
    assert_eq!(headers["result"]["items"].as_array().unwrap().len(), 100);
    assert!(raw.len() < 40_000, "{} bytes", raw.len());
    assert!(elapsed.as_secs() < 5, "{elapsed:?}");
}

/// Run the binary with its output pipe already closed at the reader end, and
/// return its stderr and whether it exited successfully before the deadline.
#[cfg(unix)]
fn run_with_closed_stdout(args: &[&str]) -> (bool, String) {
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let mut child = Command::new(workspace_integration::binary("loop-engine"))
        .args(args)
        .stdout(writer)
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            panic!("loop-engine did not finish with a closed stdout pipe");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    (status.success(), stderr)
}

// bookends:LE-148 — a reader that has closed the pipe ends output quietly and successfully.
#[cfg(unix)]
#[test]
fn closed_pipe_ends_output_quietly() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());
    let db = database.to_str().expect("database");
    for args in [
        vec![
            "--database",
            db,
            "--json",
            "show",
            "--view",
            "full",
            "ctx-run",
        ],
        vec![
            "monitor",
            "--run",
            "ctx-run",
            "--database",
            db,
            "--json",
            "--poll-seconds",
            "0.05",
        ],
    ] {
        let (succeeded, stderr) = run_with_closed_stdout(&args);
        assert!(succeeded, "{args:?}: {stderr}");
        assert!(stderr.is_empty(), "{args:?}: {stderr}");
    }
}
