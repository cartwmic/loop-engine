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
        &["read", "ctx-run", "--kind", "context", "--record-kind", "review-evidence"],
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
            "read", "ctx-run", "--kind", "record", "--id", "evidence-1", "--pointer",
            "/data/nested/findings/1",
        ],
    );
    assert_eq!(slice["result"]["value"], "f2");

    let (code, bad, _) = cli(
        &database,
        &[
            "read", "ctx-run", "--kind", "record", "--id", "evidence-1", "--pointer",
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

// bookends:LE-148 — retrieval of run context is a bounded read through the public binary, with no whole-run payload.
#[test]
fn list_filters_narrow_and_unfiltered_list_is_unchanged() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());
    let persistence = SqlitePersistence::open(&database).expect("sqlite");
    create_run(&persistence, "other-run", Some("beta"), "provider-b");
    drop(persistence);
    let (code, terminated, _) = cli(&database, &["terminate", "other-run"]);
    assert_eq!(code, 0, "{terminated}");

    let ids = |args: &[&str]| -> Vec<String> {
        let (code, value, _) = cli(&database, args);
        assert_eq!(code, 0, "{value}");
        let mut ids: Vec<String> = value["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&["list"]), ["ctx-run", "other-run"]);
    assert_eq!(ids(&["list", "--lifecycle", "active"]), ["ctx-run"]);
    assert_eq!(ids(&["list", "--lifecycle=terminated"]), ["other-run"]);
    assert_eq!(ids(&["list", "--label", "review"]), ["ctx-run"]);
    assert_eq!(ids(&["list", "--provider", "provider-b"]), ["other-run"]);
    assert!(ids(&["list", "--provider", "provider"]).is_empty());
    assert_eq!(
        ids(&["list", "--lifecycle", "active", "--label", "alpha", "--provider", "provider-a"]),
        ["ctx-run"]
    );

    let (code, bad, _) = cli(&database, &["list", "--lifecycle", "running"]);
    assert_ne!(code, 0);
    assert_eq!(bad["code"], "invalid-invocation", "{bad}");
    let (code, elsewhere, _) = cli(&database, &["show", "ctx-run", "--lifecycle", "active"]);
    assert_ne!(code, 0);
    assert_eq!(elsewhere["code"], "invalid-invocation", "{elsewhere}");
}

#[cfg(unix)]
// bookends:LE-148 — retrieval of run context is a bounded read through the public binary, with no whole-run payload.
#[test]
fn closed_pipe_ends_output_quietly() {
    let directory = tempdir().expect("tempdir");
    let database = seeded(directory.path());
    let mut child = Command::new(workspace_integration::binary("loop-engine"))
        .args([
            "--database",
            database.to_str().expect("database"),
            "--json",
            "show",
            "--view",
            "full",
            "ctx-run",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    // The reader leaves before reading anything, as `| head -c 10` does.
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!stderr.contains("Broken pipe"), "{stderr}");
}
