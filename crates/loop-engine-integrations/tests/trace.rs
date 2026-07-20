use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Barrier};

use loop_engine_core::model::bounded::IDENTIFIER_UTF8_BYTES;
use loop_engine_integrations::provider_process::base64;
use loop_engine_integrations::trace::{
    TRACE_DIRECTORY_BUDGET_BYTES, TRACE_INIT_RESERVATION_BYTES,
    TRACE_PROVIDER_CALL_RESERVATION_BYTES, TraceCategory, TraceEvent, TraceWriter,
};

fn event(request_id: &str, category: &str, name: &str) -> TraceEvent {
    let category = match category {
        "invocation" => TraceCategory::Invocation,
        "provider" => TraceCategory::Provider,
        value => panic!("unsupported test category {value}"),
    };
    TraceEvent::new(request_id, category, name, BTreeMap::new())
}

#[test]
fn creates_private_unique_jsonl_and_accounts_actual_bytes_once() {
    let root = tempfile::tempdir().unwrap();
    let machine_home = root.path().join("new/machine-home");
    let directory = machine_home.join("traces");
    let mut writer = TraceWriter::create(&directory, "request-1").unwrap();
    let encoded = writer
        .write(&event("request-1", "invocation", "start"))
        .unwrap();
    assert_eq!(
        writer.unused_reservation(),
        TRACE_INIT_RESERVATION_BYTES - encoded as u64
    );
    let path = writer.path().to_owned();
    for private_directory in [root.path().join("new"), machine_home, directory.clone()] {
        assert_eq!(
            std::fs::metadata(private_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    writer.close().unwrap();
    assert_eq!(std::fs::read(&path).unwrap().len(), encoded);
    assert!(TraceWriter::create(&directory, "request-1").is_err());
}

#[test]
fn initialization_failure_is_distinct_before_any_trace_exists() {
    let root = tempfile::tempdir().unwrap();
    let blocked = root.path().join("not-a-directory");
    std::fs::write(&blocked, b"occupied").unwrap();
    assert!(TraceWriter::create(&blocked, "request").is_err());
    assert_eq!(std::fs::read(&blocked).unwrap(), b"occupied");
}

#[test]
fn request_id_cannot_escape_trace_directory_or_spoof_event_envelope() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("traces");
    assert!(TraceWriter::create(&directory, "../escaped").is_err());
    assert!(!root.path().join("escaped.jsonl").exists());

    let mut writer = TraceWriter::create(&directory, "request-safe").unwrap();
    let mut payload = BTreeMap::new();
    payload.insert("request_id".into(), serde_json::json!("spoofed"));
    let spoofed = TraceEvent::new("request-safe", TraceCategory::Invocation, "start", payload);
    assert!(writer.write(&spoofed).is_err());
    assert_eq!(std::fs::metadata(writer.path()).unwrap().len(), 0);
    writer.close().unwrap();
}

#[test]
fn rotation_lock_contention_has_bounded_retry() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let writer = TraceWriter::create(&directory, "seed").unwrap();
    writer.close().unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(".rotation.lock"))
        .unwrap();
    lock.lock().unwrap();
    let started = std::time::Instant::now();
    assert!(TraceWriter::create(&directory, "contended").is_err());
    let elapsed = started.elapsed();
    assert!(elapsed >= std::time::Duration::from_secs(1));
    assert!(elapsed < std::time::Duration::from_secs(3));
    lock.unlock().unwrap();
}

#[test]
fn provider_reservation_consumes_lines_and_releases_only_unused_remainder() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let mut writer = TraceWriter::create(&directory, "request-2").unwrap();
    writer.reserve_provider_call().unwrap();
    let before = writer.unused_reservation();
    let bytes = writer
        .write(&event("request-2", "provider", "start"))
        .unwrap() as u64;
    let released = writer.release_provider_call().unwrap();
    assert_eq!(released, TRACE_PROVIDER_CALL_RESERVATION_BYTES - bytes);
    assert_eq!(writer.unused_reservation(), TRACE_INIT_RESERVATION_BYTES);
    assert_eq!(
        before,
        TRACE_INIT_RESERVATION_BYTES + TRACE_PROVIDER_CALL_RESERVATION_BYTES
    );
    writer.close().unwrap();
}

#[test]
fn concurrent_writers_keep_distinct_files_and_directory_under_budget() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("traces");
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|index| {
            let directory = directory.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let request = format!("request-{index}");
                barrier.wait();
                let mut writer = TraceWriter::create(&directory, &request).unwrap();
                writer
                    .write(&event(&request, "invocation", "start"))
                    .unwrap();
                writer.close().unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let actual = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum::<u64>();
    assert!(actual < TRACE_DIRECTORY_BUDGET_BYTES);
}

#[test]
fn control_heavy_event_uses_exact_on_disk_json_encoding() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let mut payload = BTreeMap::new();
    payload.insert(
        "message".to_owned(),
        serde_json::Value::String("\0\n\r\t\u{1f}\"\\".repeat(1_000)),
    );
    let value = TraceEvent::new("request-control", TraceCategory::Parse, "failure", payload);
    let expected = serde_json::to_vec(&value).unwrap().len() + 1;
    let mut writer = TraceWriter::create(&directory, "request-control").unwrap();
    assert_eq!(writer.write(&value).unwrap(), expected);
    writer.close().unwrap();
    assert_eq!(
        std::fs::read(directory.join("request-control.jsonl"))
            .unwrap()
            .len(),
        expected
    );
}

#[test]
fn published_trace_fixtures_are_versioned_and_never_duplicate_parsed_stdout() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/trace/v1");
    let schema: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("event.schema.json")).unwrap()).unwrap();
    assert_eq!(schema["properties"]["trace_schema_version"]["const"], 1);
    assert_eq!(
        schema["properties"]["request_id"]["maxLength"],
        IDENTIFIER_UTF8_BYTES
    );
    assert!(schema.get("x-loop-engine-bound-markers").is_some());
    let mut count = 0;
    for entry in std::fs::read_dir(root.join("fixtures")).unwrap() {
        let path = entry.unwrap().path();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["trace_schema_version"], 1, "{}", path.display());
        assert!(value.get("parsed_result").is_none(), "{}", path.display());
        assert!(value.get("result").is_none(), "{}", path.display());
        count += 1;
    }
    assert!(count >= 7);
}

#[test]
fn binary_stream_encoding_is_standard_base64_once() {
    assert_eq!(base64(&[0, 1, 2, 253, 254, 255]), "AAEC/f7/");
}

#[test]
fn trace_child_process_writer() {
    let Some(directory) = std::env::var_os("LOOP_ENGINE_TRACE_CHILD_DIR") else {
        return;
    };
    let id = std::env::var("LOOP_ENGINE_TRACE_CHILD_ID").unwrap();
    let directory = std::path::PathBuf::from(directory);
    let mut writer = TraceWriter::create(&directory, &id).unwrap();
    writer.write(&event(&id, "invocation", "start")).unwrap();
    writer.close().unwrap();
}

#[test]
fn independent_processes_coordinate_reservations_and_files() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let executable = std::env::current_exe().unwrap();
    let children = (0..6)
        .map(|index| {
            std::process::Command::new(&executable)
                .args(["--exact", "trace_child_process_writer", "--nocapture"])
                .env("LOOP_ENGINE_TRACE_CHILD_DIR", &directory)
                .env("LOOP_ENGINE_TRACE_CHILD_ID", format!("child-{index}"))
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(
        std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(
                |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
            )
            .count(),
        6
    );
}

#[test]
fn ten_provider_calls_convert_added_reservations_without_double_counting() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let mut writer = TraceWriter::create(&directory, "ten-calls").unwrap();
    for index in 0..10 {
        writer.reserve_provider_call().unwrap();
        let mut payload = BTreeMap::new();
        payload.insert(
            "chunk".into(),
            serde_json::Value::String("x".repeat(1024 * 1024)),
        );
        writer
            .write(&TraceEvent::new(
                "ten-calls",
                TraceCategory::Provider,
                format!("chunk-{index}"),
                payload,
            ))
            .unwrap();
        writer.release_provider_call().unwrap();
        assert_eq!(writer.unused_reservation(), TRACE_INIT_RESERVATION_BYTES);
    }
    let actual = std::fs::metadata(writer.path()).unwrap().len();
    let sidecar_bytes = std::fs::read(directory.join(".reserve/ten-calls.json")).unwrap();
    let sidecar = sidecar_bytes
        .chunks(160)
        .filter_map(|slot| serde_json::from_slice::<serde_json::Value>(slot).ok())
        .max_by_key(|value| value["generation"].as_u64().unwrap())
        .unwrap();
    assert_eq!(
        sidecar["unused_reservation_bytes"],
        TRACE_INIT_RESERVATION_BYTES
    );
    assert!(actual + TRACE_INIT_RESERVATION_BYTES < TRACE_DIRECTORY_BUDGET_BYTES);
    writer.close().unwrap();
}

#[test]
fn provider_reservation_reconciles_crash_stale_sidecars() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let mut writer = TraceWriter::create(&directory, "active").unwrap();
    let stale_value = 120_u64 * 1024 * 1024;
    let stale = directory.join(".reserve/stale.json");
    std::fs::write(
        &stale,
        serde_json::to_vec(&serde_json::json!({
            "unused_reservation_bytes": stale_value,
            "unused_reservation_complement": !stale_value,
        }))
        .unwrap(),
    )
    .unwrap();
    writer.reserve_provider_call().unwrap();
    assert!(!stale.exists());
    writer.release_provider_call().unwrap();
    writer.close().unwrap();
}

#[test]
fn concurrent_initialization_reserves_retained_file_slots() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    std::fs::create_dir_all(&directory).unwrap();
    for index in 0..99 {
        std::fs::File::create(directory.join(format!("closed-{index:03}.jsonl"))).unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let directory = directory.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                TraceWriter::create(&directory, format!("active-{index}"))
                    .unwrap()
                    .close()
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let retained = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .count();
    assert_eq!(retained, 100);
}

#[test]
fn provider_call_reservation_rejects_one_event_beyond_its_encoded_budget() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let mut writer = TraceWriter::create(&directory, "too-large-call").unwrap();
    writer.reserve_provider_call().unwrap();
    let mut payload = BTreeMap::new();
    payload.insert(
        "chunk".into(),
        serde_json::Value::String("x".repeat(TRACE_PROVIDER_CALL_RESERVATION_BYTES as usize)),
    );
    let oversized = TraceEvent::new("too-large-call", TraceCategory::Provider, "finish", payload);
    assert!(writer.write(&oversized).is_err());
    assert_eq!(std::fs::metadata(writer.path()).unwrap().len(), 0);
    writer.release_provider_call().unwrap();
    writer.close().unwrap();
}

#[test]
fn stale_reservation_is_reconciled_and_active_trace_is_not_evicted() {
    let directory = tempfile::tempdir().unwrap().path().join("traces");
    let active = TraceWriter::create(&directory, "active").unwrap();
    let closed = directory.join("closed.jsonl");
    let file = std::fs::File::create(&closed).unwrap();
    file.set_len(120 * 1024 * 1024).unwrap();
    drop(file);
    let mut second = TraceWriter::create(&directory, "second").unwrap();
    assert!(active.path().exists());
    assert!(!closed.exists());
    second
        .write(&event("second", "invocation", "start"))
        .unwrap();
    let rotation = std::fs::read_to_string(second.path()).unwrap();
    let rotation = rotation
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rotation[0]["category"], "invocation");
    assert_eq!(rotation[0]["event"], "start");
    assert_eq!(rotation[1]["category"], "trace");
    assert_eq!(rotation[1]["event"], "rotation_evict");
    assert_eq!(
        rotation[1]["evicted_path"],
        closed.to_string_lossy().as_ref()
    );
    assert_eq!(rotation[1]["encoded_bytes_reclaimed"], 120 * 1024 * 1024);
    second.close().unwrap();
    drop(active);

    assert!(directory.join(".reserve/active.json").exists());
    let reconciler = TraceWriter::create(&directory, "reconciler").unwrap();
    assert!(!directory.join(".reserve/active.json").exists());
    assert!(directory.join("active.jsonl").exists());
    reconciler.close().unwrap();
}
