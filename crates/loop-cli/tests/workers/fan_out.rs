use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn worker_json(command: &str, args: &[&str]) -> String {
    json!({ "command": command, "args": args }).to_string()
}

fn contracted_worker_json(
    command: &str,
    args: &[&str],
    preamble: Option<&str>,
    required: &[&str],
) -> String {
    let mut worker = json!({
        "command": command,
        "args": args,
        "output_schema": {"required": required},
    });
    if let Some(preamble) = preamble {
        worker["preamble"] = Value::String(preamble.to_owned());
    }
    worker.to_string()
}

fn full_review_schema(axis: &str, author: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"type": "string", "const": axis},
            "author": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string", "const": author},
                    "kind": {"type": "string", "enum": ["human", "agent", "script"]}
                }
            },
            "result": {"type": "string", "enum": ["pass", "fail"]},
            "findings": {"type": "string"}
        },
        "oneOf": [
            {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
            {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}}
        ]
    })
}

fn full_worker_json(command: &str, args: &[&str], preamble: Option<&str>) -> String {
    let mut worker = json!({
        "command": command,
        "args": args,
        "full_output_schema": full_review_schema("axis-a", "reviewer-a")
    });
    if let Some(preamble) = preamble {
        worker["preamble"] = Value::String(preamble.to_owned());
    }
    worker.to_string()
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn output_worker(
    receipt: &Path,
    stdout: &str,
    exit_code: i32,
    preamble: Option<&str>,
    required: &[&str],
) -> String {
    let stdout_file = receipt.with_extension("stdout-fixture");
    std::fs::write(&stdout_file, stdout).expect("stdout fixture");
    contracted_worker_json(
        "sh",
        &[
            "-c",
            &format!("cat > \"$1\"; cat \"$2\"; exit {exit_code}"),
            "_",
            receipt.to_str().expect("utf-8 receipt"),
            stdout_file.to_str().expect("utf-8 stdout fixture"),
        ],
        preamble,
        required,
    )
}

fn cat_worker(receipt: &Path) -> String {
    worker_json(
        "sh",
        &[
            "-c",
            "cat > \"$1\"",
            "_",
            receipt.to_str().expect("utf-8 receipt"),
        ],
    )
}

fn sleep_worker(receipt: &Path, pid: &Path, done: &Path) -> String {
    worker_json(
        "sh",
        &[
            "-c",
            "cat > \"$1\"; echo $$ > \"$2\"; sleep 0.2; echo done > \"$3\"",
            "_",
            receipt.to_str().expect("utf-8 receipt"),
            pid.to_str().expect("utf-8 pid"),
            done.to_str().expect("utf-8 done"),
        ],
    )
}

fn delay_then_read_worker(receipt: &Path, pid: &Path) -> String {
    worker_json(
        "sh",
        &[
            "-c",
            "echo $$ > \"$1\"; sleep 0.4; cat > \"$2\"",
            "_",
            pid.to_str().expect("utf-8 pid"),
            receipt.to_str().expect("utf-8 receipt"),
        ],
    )
}

fn exit_worker(receipt: &Path, code: i32) -> String {
    worker_json(
        "sh",
        &[
            "-c",
            &format!("cat > \"$1\"; exit {code}"),
            "_",
            receipt.to_str().expect("utf-8 receipt"),
        ],
    )
}

fn invoke_packet(artifact_root: &Path, capture_dir: &Path, instruction_body: &str) -> String {
    json!({
        "run_id": "run-1",
        "slot_id": "slot-1",
        "artifact_root": artifact_root.to_string_lossy(),
        "instruction_body": instruction_body,
        "capture_dir": capture_dir.to_string_lossy(),
    })
    .to_string()
}

fn capture_summary(output_dir: &str) -> Value {
    let path = Path::new(output_dir).join("summary.json");
    assert!(
        path.is_file(),
        "expected summary.json at {}",
        path.display()
    );
    serde_json::from_slice(&std::fs::read(&path).expect("read summary.json")).expect("summary json")
}

fn run_fan_out(cwd: &Path, args: &[&str], stdin: &[u8]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_loop-engine"));
    command.current_dir(cwd).args(args);
    let completed =
        super::bounded_process::run_with_stdin(&mut command, "loop-engine fan-out", stdin)
            .expect("wait for fan-out");
    completed.output
}

fn pid_is_alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn parse_summary(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).unwrap_or_else(|error| {
        panic!(
            "summary must be JSON: {error}: {}",
            String::from_utf8_lossy(stdout)
        )
    })
}

#[test]
fn zero_workers_exit_2_in_bound_and_ad_hoc() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"shared").expect("write instructions");
    let packet = invoke_packet(
        &directory.path().join("artifacts"),
        &directory.path().join("captures").join("inv-1"),
        "Do the work",
    );

    let bound = run_fan_out(directory.path(), &["fan-out"], packet.as_bytes());
    assert_eq!(bound.status.code(), Some(2), "{bound:?}");

    let ad_hoc = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
        ],
        b"",
    );
    assert_eq!(ad_hoc.status.code(), Some(2), "{ad_hoc:?}");
}

#[test]
fn bound_packet_plus_instructions_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"shared").expect("write instructions");
    let receipt = directory.path().join("unused.stdin");
    let worker = cat_worker(&receipt);
    let packet = invoke_packet(
        &directory.path().join("artifacts"),
        &directory.path().join("captures").join("inv-1"),
        "Do the work",
    );
    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &worker,
        ],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot combine") || stderr.contains("instructions"),
        "{stderr}"
    );
}

#[test]
fn ad_hoc_without_instructions_is_rejected() {
    let directory = tempdir().expect("tempdir");
    let receipt = directory.path().join("unused.stdin");
    let worker = cat_worker(&receipt);
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        b"not a packet",
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn public_cli_rejects_malformed_nested_contract_and_non_five_key_packet() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"body").expect("instructions");
    let malformed_worker =
        r#"{"command":"true","args":[],"output_schema":{"required":["result","result"]}}"#;
    let malformed = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8 instructions"),
            "--worker",
            malformed_worker,
        ],
        b"",
    );
    assert_eq!(malformed.status.code(), Some(2), "{malformed:?}");

    let receipt = directory.path().join("unused.stdin");
    let worker = cat_worker(&receipt);
    let mut packet: Value = serde_json::from_str(&invoke_packet(
        &directory.path().join("artifacts"),
        &directory.path().join("captures"),
        "body",
    ))
    .expect("packet");
    packet["extra"] = json!("sixth key");
    let extra_packet = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.to_string().as_bytes(),
    );
    assert_eq!(extra_packet.status.code(), Some(2), "{extra_packet:?}");
    assert!(!receipt.exists());
}

#[test]
fn two_dummies_record_the_same_shared_stdin_and_are_reaped() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.bin");
    let shared = b"shared-bytes-without-trailer";
    std::fs::write(&instructions, shared).expect("write instructions");
    let receipt_a = directory.path().join("a.stdin");
    let receipt_b = directory.path().join("b.stdin");
    let pid_a = directory.path().join("a.pid");
    let pid_b = directory.path().join("b.pid");
    let done_a = directory.path().join("a.done");
    let done_b = directory.path().join("b.done");
    let worker_a = sleep_worker(&receipt_a, &pid_a, &done_a);
    let worker_b = sleep_worker(&receipt_b, &pid_b, &done_b);

    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &worker_a,
            "--worker",
            &worker_b,
        ],
        b"",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(std::fs::read(&receipt_a).expect("read a"), shared);
    assert_eq!(std::fs::read(&receipt_b).expect("read b"), shared);
    assert!(
        done_a.is_file(),
        "worker A must finish before fan-out exits"
    );
    assert!(
        done_b.is_file(),
        "worker B must finish before fan-out exits"
    );
    let pid_a = std::fs::read_to_string(&pid_a).expect("pid a");
    let pid_b = std::fs::read_to_string(&pid_b).expect("pid b");
    assert!(!pid_is_alive(pid_a.trim()), "worker A must be reaped");
    assert!(!pid_is_alive(pid_b.trim()), "worker B must be reaped");

    let summary = parse_summary(&output.stdout);
    assert!(summary.get("operation").is_none(), "{summary}");
    assert!(summary.get("status").is_none(), "{summary}");
    assert!(summary.get("result").is_none(), "{summary}");
    let output_dir = summary["output_dir"].as_str().expect("output_dir");
    assert!(
        output_dir.contains("fan-out-adhoc"),
        "ad hoc output_dir: {output_dir}"
    );
    assert!(Path::new(output_dir).is_absolute(), "{output_dir}");
    let workers = summary["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
    assert!(workers[0].get("status").is_none(), "{workers:?}");
    assert!(workers[0].get("conformance_error").is_none(), "{workers:?}");
    assert!(Path::new(workers[0]["stdout_path"].as_str().unwrap()).is_file());
    assert!(Path::new(workers[1]["stderr_path"].as_str().unwrap()).is_file());
    let captured = capture_summary(output_dir);
    assert_eq!(captured["workers"].as_array().expect("workers").len(), 2);
    assert_eq!(captured["workers"][0]["exit_code"], 0);
    assert_eq!(captured["workers"][1]["exit_code"], 0);
    assert!(captured["workers"][0].get("status").is_none());
    assert!(captured["workers"][0].get("conformance_error").is_none());
}

#[test]
fn dummy_nonzero_exit_still_yields_fan_out_exit_0_and_appears_in_summary() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"judge this").expect("write instructions");
    let receipt = directory.path().join("nonzero.stdin");
    let worker = exit_worker(&receipt, 7);
    let output = run_fan_out(
        directory.path(),
        &[
            "--database",
            directory
                .path()
                .join("missing")
                .join("loop.db")
                .to_str()
                .expect("utf-8"),
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &worker,
        ],
        b"",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !directory.path().join("missing").join("loop.db").exists(),
        "fan-out must not open the run database"
    );
    let summary = parse_summary(&output.stdout);
    assert_eq!(summary["workers"][0]["exit_code"], 7);
    assert_eq!(summary["workers"][0]["command"], "sh");
    assert_eq!(std::fs::read(&receipt).expect("receipt"), b"judge this");
    let captured = capture_summary(summary["output_dir"].as_str().expect("output_dir"));
    assert_eq!(captured["workers"][0]["exit_code"], 7);
}

#[test]
fn bound_mode_writes_under_capture_dir_and_records_locked_stdin() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-1");
    let receipt = directory.path().join("bound.stdin");
    let worker = cat_worker(&receipt);
    let packet = invoke_packet(&artifact_root, &capture_dir, "Review the design");
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let expected = serde_json::to_string(&serde_json::json!({
        "artifact_root": artifact_root.to_string_lossy(),
    }))
    .expect("compact json")
        + "\n";
    assert_eq!(
        std::fs::read_to_string(&receipt).expect("bound stdin"),
        expected
    );
    let summary = parse_summary(&output.stdout);
    assert_eq!(
        summary["output_dir"].as_str().unwrap(),
        capture_dir.to_string_lossy()
    );
    assert_eq!(
        summary["workers"][0]["stdout_path"].as_str().unwrap(),
        capture_dir.join("0").join("stdout").to_string_lossy()
    );
    assert!(capture_dir.join("0").join("stdout").is_file());
    let captured = capture_summary(summary["output_dir"].as_str().expect("output_dir"));
    assert_eq!(captured["workers"][0]["exit_code"], 0);
    assert_eq!(captured["workers"][0]["command"], "sh");
}

#[test]
fn bound_preamble_has_exact_context_separator_and_no_instruction_body() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifact-\"quoted\\tail");
    let capture_dir = directory.path().join("captures").join("inv-framed");
    let receipt = directory.path().join("framed.stdin");
    let preamble = "read-only role";
    let worker = output_worker(
        &receipt,
        r#"{"result":null}"#,
        0,
        Some(preamble),
        &["result"],
    );
    let packet = invoke_packet(&artifact_root, &capture_dir, "Review the design");
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let context = json!({"artifact_root": artifact_root.to_string_lossy()}).to_string();
    let expected = format!("{preamble}\n{context}\n---\n\n");
    let recorded = std::fs::read(&receipt).expect("framed stdin");
    assert_eq!(recorded, expected.as_bytes());
    let recorded_text = String::from_utf8_lossy(&recorded);
    assert!(!recorded_text.contains("instruction_body"));
    assert!(!recorded_text.contains("Review the design"));
    assert!(!recorded_text.contains("run_id:"));
    let context_value: Value = serde_json::from_str(&context).expect("context JSON");
    assert_eq!(context_value.as_object().expect("context object").len(), 1);
    assert!(context_value.get("artifact_root").is_some());
    assert!(context_value.get("capture_dir").is_none());
    assert!(context_value.get("run_id").is_none());
    assert!(context_value.get("slot_id").is_none());

    let summary = parse_summary(&output.stdout);
    assert_eq!(summary["workers"][0]["status"], "succeeded");
    assert_eq!(summary["workers"][0]["exit_code"], 0);
    assert!(summary["workers"][0].get("conformance_error").is_none());
}

#[test]
fn full_schema_retry_preserves_attempt_bytes_and_selects_same_worker_success() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-1");
    let packet = invoke_packet(&artifact_root, &capture_dir, "review assignment\n");
    let counter = directory.path().join("counter");
    fs::write(&counter, b"0").expect("counter");
    let input_root = directory.path().join("inputs");
    fs::create_dir_all(&input_root).expect("input root");
    let script = r#"
number=$(cat "$1")
number=$((number + 1))
printf '%s' "$number" > "$1"
cat > "$2/input-$number"
printf 'stderr-%s' "$number" >&2
if [ "$number" = 1 ]; then
  printf '%s' '{"axis":"wrong","author":{"name":"wrong","kind":"agent"},"result":"pass","findings":"not empty"}'
else
  printf '%s' '{"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent"},"result":"pass","findings":""}'
fi
"#;
    let worker = full_worker_json(
        "sh",
        &[
            "-c",
            script,
            "_",
            counter.to_str().expect("counter path"),
            input_root.to_str().expect("input root"),
            "--model",
            "model-a",
        ],
        Some("frozen review preamble"),
    );
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let summary = parse_summary(&output.stdout);
    let worker_summary = &summary["workers"][0];
    assert_eq!(worker_summary["status"], "succeeded");
    assert_eq!(worker_summary["selected_attempt"], 2);
    assert_eq!(worker_summary["attempts_path"], "0/attempts.json");
    assert_eq!(
        worker_summary["args"][worker_summary["args"].as_array().unwrap().len() - 1],
        "model-a"
    );

    let capture_dir = Path::new(summary["output_dir"].as_str().expect("output_dir"));
    let worker_dir = capture_dir.join("0");
    let manifest: Value = serde_json::from_slice(
        &fs::read(worker_dir.join("attempts.json")).expect("attempts manifest"),
    )
    .expect("attempts JSON");
    assert_eq!(manifest["schema_version"], "1");
    assert_eq!(manifest["selected_attempt"], 2);
    assert_eq!(manifest["exhausted"], false);
    assert_eq!(manifest["attempts"].as_array().unwrap().len(), 2);
    for number in [1, 2] {
        let attempt = &manifest["attempts"][number - 1];
        assert_eq!(attempt["number"], number);
        for key in ["stdout_sha256", "stderr_sha256"] {
            let value = attempt[key].as_str().expect("digest");
            assert_eq!(value.len(), 71, "{key}: {value}");
            assert!(value.starts_with("sha256:"), "{key}: {value}");
            assert!(value[7..].bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
    let first_stdout = br#"{"axis":"wrong","author":{"name":"wrong","kind":"agent"},"result":"pass","findings":"not empty"}"#;
    let second_stdout = br#"{"axis":"axis-a","author":{"name":"reviewer-a","kind":"agent"},"result":"pass","findings":""}"#;
    let first_stderr = b"stderr-1";
    let second_stderr = b"stderr-2";
    assert_eq!(
        fs::read(worker_dir.join("attempts/1/stdout")).unwrap(),
        first_stdout
    );
    assert_eq!(
        fs::read(worker_dir.join("attempts/1/stderr")).unwrap(),
        first_stderr
    );
    assert_eq!(
        fs::read(worker_dir.join("attempts/2/stdout")).unwrap(),
        second_stdout
    );
    assert_eq!(
        fs::read(worker_dir.join("attempts/2/stderr")).unwrap(),
        second_stderr
    );
    assert_eq!(fs::read(worker_dir.join("stdout")).unwrap(), second_stdout);
    assert_eq!(fs::read(worker_dir.join("stderr")).unwrap(), second_stderr);
    assert_eq!(
        manifest["attempts"][0]["stdout_sha256"],
        digest(first_stdout)
    );
    assert_eq!(
        manifest["attempts"][0]["stderr_sha256"],
        digest(first_stderr)
    );
    assert_eq!(
        manifest["attempts"][1]["stdout_sha256"],
        digest(second_stdout)
    );
    assert_eq!(
        manifest["attempts"][1]["stderr_sha256"],
        digest(second_stderr)
    );

    let first_input = fs::read(input_root.join("input-1")).expect("first assignment");
    let retry_input = fs::read(input_root.join("input-2")).expect("retry assignment");
    let expected_assignment = format!(
        "frozen review preamble\n{}\n---\n\n",
        serde_json::to_string(&json!({"artifact_root": artifact_root.to_string_lossy()}))
            .expect("location JSON")
    );
    assert_eq!(first_input, expected_assignment.as_bytes());
    assert!(retry_input.starts_with(expected_assignment.as_bytes()));
    assert!(retry_input
        .windows(first_stdout.len())
        .any(|window| window == first_stdout));
    for error in manifest["attempts"][0]["validation_errors"]
        .as_array()
        .expect("validation errors")
        .iter()
        .map(|value| value.as_str().expect("error"))
    {
        assert!(
            retry_input
                .windows(error.len())
                .any(|window| window == error.as_bytes()),
            "missing retry error: {error}"
        );
    }
    let retry_text = String::from_utf8(retry_input).expect("retry prompt UTF-8");
    assert!(retry_text.contains("SCHEMA-CONFORMANCE RETRY"));
    assert!(retry_text.contains("Return only the schema-conforming reconsideration"));
}

#[test]
fn full_schema_retry_exhaustion_preserves_both_attempts_and_fails_closed() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    fs::write(&instructions, b"review assignment").expect("instructions");
    let stderr_fixture = directory.path().join("stderr.txt");
    let worker = full_worker_json(
        "sh",
        &[
            "-c",
            "cat >/dev/null; printf '%s' '{\"axis\":\"wrong\"}'; cat \"$1\" >&2",
            "_",
            stderr_fixture.to_str().expect("stderr path"),
        ],
        None,
    );
    fs::write(&stderr_fixture, b"raw stderr").expect("stderr fixture");
    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("instructions path"),
            "--worker",
            &worker,
        ],
        b"",
    );
    assert_eq!(output.status.code(), Some(20), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("summary.json"));
    let capture_dir = find_latest_adhoc_capture(directory.path());
    let summary = capture_summary(capture_dir.to_str().expect("capture path"));
    let worker_summary = &summary["workers"][0];
    assert_eq!(worker_summary["status"], "failed");
    assert_eq!(worker_summary["selected_attempt"], Value::Null);
    assert_eq!(worker_summary["attempts_path"], "0/attempts.json");
    assert!(worker_summary["conformance_error"]
        .as_str()
        .expect("conformance error")
        .contains("exhausted after 2 attempts"));
    assert!(worker_summary.get("result").is_none());

    let worker_dir = capture_dir.join("0");
    let manifest: Value = serde_json::from_slice(
        &fs::read(worker_dir.join("attempts.json")).expect("attempts manifest"),
    )
    .expect("attempts JSON");
    assert_eq!(manifest["selected_attempt"], Value::Null);
    assert_eq!(manifest["exhausted"], true);
    assert_eq!(manifest["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read(worker_dir.join("attempts/1/stdout")).unwrap(),
        fs::read(worker_dir.join("attempts/2/stdout")).unwrap()
    );
    assert_eq!(
        fs::read(worker_dir.join("attempts/1/stderr")).unwrap(),
        fs::read(worker_dir.join("attempts/2/stderr")).unwrap()
    );
    assert_eq!(
        manifest["attempts"][0]["stdout_sha256"],
        digest(&fs::read(worker_dir.join("attempts/1/stdout")).unwrap())
    );
}

fn find_latest_adhoc_capture(directory: &Path) -> std::path::PathBuf {
    let mut captures = fs::read_dir(directory.join("fan-out-adhoc"))
        .expect("fan-out captures")
        .map(|entry| entry.expect("capture entry").path())
        .collect::<Vec<_>>();
    captures.sort();
    captures.pop().expect("one ad-hoc capture")
}

#[test]
fn output_schema_only_preserves_bound_stdin_and_reports_conformance() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-schema-only");
    let receipt = directory.path().join("schema-only.stdin");
    let worker = output_worker(
        &receipt,
        "prose\n```json\n{\"result\":false}\n```\n",
        9,
        None,
        &["result"],
    );
    let packet = invoke_packet(&artifact_root, &capture_dir, "judge this");
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let expected = serde_json::to_string(&json!({
        "artifact_root": artifact_root.to_string_lossy(),
    }))
    .expect("compact json")
        + "\n";
    assert_eq!(std::fs::read(&receipt).expect("stdin"), expected.as_bytes());
    let summary = parse_summary(&output.stdout);
    assert_eq!(summary["workers"][0]["status"], "succeeded");
    assert_eq!(summary["workers"][0]["exit_code"], 9);
}

#[test]
fn exit_zero_nonconforming_worker_fails_after_writing_summary() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-refusal");
    let receipt = directory.path().join("refusal.stdin");
    let worker = output_worker(
        &receipt,
        "I cannot perform that review.",
        0,
        None,
        &["result"],
    );
    let packet = invoke_packet(&artifact_root, &capture_dir, "judge this");
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(20), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("summary.json"));

    let captured = capture_summary(capture_dir.to_str().expect("utf-8 capture"));
    assert_eq!(captured["workers"][0]["status"], "failed");
    assert_eq!(captured["workers"][0]["exit_code"], 0);
    assert!(captured["workers"][0]["conformance_error"]
        .as_str()
        .expect("conformance error")
        .contains("no JSON fenced block"));
    assert!(capture_dir.join("0").join("stdout").is_file());
    assert!(capture_dir.join("0").join("stderr").is_file());
}

#[test]
fn public_cli_rejects_ambiguous_malformed_and_missing_key_stdout() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let cases = [
        (
            "ambiguous",
            "```json\n{\"result\":1}\n```\n```json\n{\"result\":2}\n```\n",
            "multiple JSON fenced blocks",
        ),
        ("malformed", "```json\n{bad}\n```\n", "malformed"),
        (
            "missing-key",
            "{\"axis\":true}\n",
            "missing required top-level keys",
        ),
    ];
    for (name, stdout, needle) in cases {
        let capture_dir = directory.path().join("captures").join(name);
        let receipt = directory.path().join(format!("{name}.stdin"));
        let worker = output_worker(&receipt, stdout, 0, None, &["result"]);
        let packet = invoke_packet(&artifact_root, &capture_dir, "judge this");
        let output = run_fan_out(
            directory.path(),
            &["fan-out", "--worker", &worker],
            packet.as_bytes(),
        );
        assert_eq!(output.status.code(), Some(20), "{name}: {output:?}");
        let captured = capture_summary(capture_dir.to_str().expect("utf-8 capture"));
        assert_eq!(captured["workers"][0]["status"], "failed", "{name}");
        assert_eq!(captured["workers"][0]["exit_code"], 0, "{name}");
        assert!(
            captured["workers"][0]["conformance_error"]
                .as_str()
                .expect("conformance error")
                .contains(needle),
            "{name}: {}",
            captured["workers"][0]["conformance_error"]
        );
    }
}

#[test]
fn ad_hoc_preamble_is_framed_without_artifact_context_and_legacy_is_unchanged() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.bin");
    std::fs::write(&instructions, b"instruction-bytes-without-lf").expect("instructions");
    let framed_receipt = directory.path().join("framed.stdin");
    let legacy_receipt = directory.path().join("legacy.stdin");
    let framed = output_worker(
        &framed_receipt,
        r#"{"result":true}"#,
        0,
        Some("role\n"),
        &["result"],
    );
    let legacy = cat_worker(&legacy_receipt);
    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8 instructions"),
            "--worker",
            &framed,
            "--worker",
            &legacy,
        ],
        b"",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        std::fs::read(&framed_receipt).expect("framed stdin"),
        b"role\n---\n\ninstruction-bytes-without-lf"
    );
    assert_eq!(
        std::fs::read(&legacy_receipt).expect("legacy stdin"),
        b"instruction-bytes-without-lf"
    );
    assert!(!std::fs::read_to_string(&framed_receipt)
        .expect("utf-8 framed")
        .contains("artifact_root"));
}

#[test]
fn bound_dummy_nonzero_exit_still_yields_fan_out_exit_0() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-7");
    let receipt = directory.path().join("nonzero.stdin");
    let worker = exit_worker(&receipt, 7);
    let packet = invoke_packet(&artifact_root, &capture_dir, "judge this");
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let summary = parse_summary(&output.stdout);
    assert_eq!(summary["workers"][0]["exit_code"], 7);
    assert!(capture_dir.join("0").join("stdout").is_file());
    let captured = capture_summary(summary["output_dir"].as_str().expect("output_dir"));
    assert_eq!(captured["workers"][0]["exit_code"], 7);
}

#[test]
fn help_lists_fan_out_and_hides_wait_invocation() {
    let output = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .arg("--help")
        .bounded_output("loop-engine fan-out")
        .expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fan-out"), "{stdout}");
    assert!(stdout.contains("--worker"), "{stdout}");
    assert!(stdout.contains("--instructions"), "{stdout}");
    assert!(stdout.contains("--max-active"), "{stdout}");
    assert!(!stdout.contains("wait-invocation"), "{stdout}");
    assert!(!stdout.contains("stdin-exec"), "{stdout}");
    assert!(!stdout.contains("fan-out-join"), "{stdout}");
    for operation in [
        "start",
        "list",
        "show",
        "append",
        "event",
        "history",
        "terminate",
        "invoke",
    ] {
        assert!(
            stdout.contains(operation),
            "help missing `{operation}`: {stdout}"
        );
    }

    let fan_out_help = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args(["fan-out", "--help"])
        .bounded_output("loop-engine fan-out")
        .expect("run fan-out --help");
    assert!(fan_out_help.status.success());
    let fan_out_stdout = String::from_utf8_lossy(&fan_out_help.stdout);
    assert!(
        fan_out_stdout.contains("--max-active N"),
        "{fan_out_stdout}"
    );
    assert!(
        fan_out_stdout.contains("uncapped concurrent worker start"),
        "{fan_out_stdout}"
    );

    let unknown = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .args(["fan-out", "--max-concurrency", "2"])
        .bounded_output("loop-engine fan-out")
        .expect("run unknown --max-concurrency");
    assert_eq!(unknown.status.code(), Some(2), "{unknown:?}");
    let stderr = String::from_utf8_lossy(&unknown.stderr);
    assert!(stderr.contains("unknown option"), "{stderr}");
    assert!(stderr.contains("--max-concurrency"), "{stderr}");
}

#[test]
fn deferred_readers_with_large_payload_start_in_parallel() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.bin");
    let payload = vec![b'x'; 2 * 1024 * 1024];
    std::fs::write(&instructions, &payload).expect("write instructions");
    let receipt_a = directory.path().join("a.stdin");
    let receipt_b = directory.path().join("b.stdin");
    let pid_a = directory.path().join("a.pid");
    let pid_b = directory.path().join("b.pid");
    let worker_a = delay_then_read_worker(&receipt_a, &pid_a);
    let worker_b = delay_then_read_worker(&receipt_b, &pid_b);
    let cwd = directory.path().to_path_buf();
    let instructions_arg = instructions.to_str().expect("utf-8").to_owned();

    let handle = thread::spawn(move || {
        run_fan_out(
            &cwd,
            &[
                "fan-out",
                "--instructions",
                &instructions_arg,
                "--worker",
                &worker_a,
                "--worker",
                &worker_b,
            ],
            b"",
        )
    });

    let first_pid_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < first_pid_deadline && !(pid_a.is_file() || pid_b.is_file()) {
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(150));
    let both_started = pid_a.is_file() && pid_b.is_file();
    let overlapping_live = both_started
        && pid_is_alive(std::fs::read_to_string(&pid_a).expect("pid a").trim())
        && pid_is_alive(std::fs::read_to_string(&pid_b).expect("pid b").trim());
    let output = handle.join().expect("fan-out thread");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        overlapping_live,
        "both workers must start concurrently with overlapping live pids before either finishes reading a 2MiB payload"
    );
    assert_eq!(std::fs::read(&receipt_a).expect("read a"), payload);
    assert_eq!(std::fs::read(&receipt_b).expect("read b"), payload);
}

#[test]
fn ad_hoc_with_terminal_stdin_does_not_wait_for_eof() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"shared").expect("write instructions");
    let script = directory.path().join("pty_fan_out.py");
    std::fs::write(
        &script,
        r#"
import os, select, signal, sys, time
signal.alarm(12)
bin, instructions, worker, cwd = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
os.chdir(cwd)
pid, fd = os.forkpty()
if pid == 0:
    os.execv(bin, [bin, "fan-out", "--instructions", instructions, "--worker", worker])
deadline = time.time() + 8
while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        try:
            os.read(fd, 4096)
        except OSError:
            pass
    wpid, status = os.waitpid(pid, os.WNOHANG)
    if wpid == pid:
        try:
            os.close(fd)
        except OSError:
            pass
        raise SystemExit(0 if os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0 else 1)
try:
    os.close(fd)
except OSError:
    pass
try:
    os.killpg(pid, 9)
except OSError:
    try:
        os.kill(pid, 9)
    except OSError:
        pass
try:
    os.waitpid(pid, 0)
except OSError:
    pass
raise SystemExit(2)
"#,
    )
    .expect("write pty helper");
    let worker = worker_json("true", &[]);
    let output = Command::new("python3")
        .args([
            script.to_str().expect("utf-8 script"),
            env!("CARGO_BIN_EXE_loop-engine"),
            instructions.to_str().expect("utf-8 instructions"),
            &worker,
            directory.path().to_str().expect("utf-8 cwd"),
        ])
        .bounded_output("loop-engine fan-out")
        .expect("run pty helper");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ad-hoc fan-out hung on terminal stdin: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn spawn_fan_out(cwd: &Path, args: &[&str], stdin: &[u8]) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_loop-engine"));
    command
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::bounded_process::prepare_process_group(&mut command);
    let mut child = command.spawn().expect("spawn loop-engine fan-out");
    {
        let mut handle = child.stdin.take().expect("fan-out stdin");
        handle.write_all(stdin).expect("write fan-out stdin");
    }
    child
}

fn read_locator(capture_dir: &Path) -> Value {
    let path = capture_dir.join("dagu-locator.json");
    serde_json::from_slice(
        &std::fs::read(&path)
            .unwrap_or_else(|error| panic!("read locator {}: {error}", path.display())),
    )
    .expect("locator json")
}

fn emitted_yaml(capture_dir: &Path) -> String {
    let locator = read_locator(capture_dir);
    let home = Path::new(locator["dagu_home"].as_str().expect("dagu_home"));
    let name = locator["dag_name"].as_str().expect("dag_name");
    std::fs::read_to_string(home.join("dags").join(format!("{name}.yaml"))).expect("emitted yaml")
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            if std::fs::metadata(path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
            {
                return true;
            }
            if path.extension().is_none() && path.file_name().is_some() {
                // locator may be a small JSON object; existence is enough
                if path.file_name() == Some(std::ffi::OsStr::new("dagu-locator.json")) {
                    return true;
                }
            }
        }
        if path.is_file() && path.ends_with("dagu-locator.json") {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    path.is_file()
}

fn kill_direct_children(parent: u32) {
    let output = Command::new("pgrep")
        .args(["-P", &parent.to_string(), "dagu"])
        .bounded_output("loop-engine fan-out")
        .expect("pgrep children");
    for pid in String::from_utf8_lossy(&output.stdout).lines() {
        let pid = pid.trim();
        if pid.is_empty() {
            continue;
        }
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
}

#[test]
fn locator_exists_during_live_worker_and_yaml_omits_retry_and_continue() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-live");
    let worker = worker_json("sh", &["-c", "echo started; sleep 2; exit 0"]);
    let packet = invoke_packet(&artifact_root, &capture_dir, "live");
    let child = spawn_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    let locator_path = capture_dir.join("dagu-locator.json");
    let stdout_path = capture_dir.join("0").join("stdout");
    assert!(
        wait_for_file(&locator_path, Duration::from_secs(15)),
        "locator missing: {}",
        locator_path.display()
    );
    let locator = read_locator(&capture_dir);
    assert!(locator["dagu_home"]
        .as_str()
        .unwrap()
        .ends_with("dagu-home"));
    assert!(locator["dag_name"].as_str().unwrap().starts_with("fanout-"));
    assert_eq!(locator["dag_name"], locator["run_name"]);
    assert_eq!(locator.as_object().expect("object").len(), 3);
    assert!(
        wait_for_file(&stdout_path, Duration::from_secs(15)),
        "stdout missing while overlay-equivalent worker is live"
    );
    let output = super::bounded_process::wait_existing(child, "loop-engine live fan-out")
        .expect("wait fan-out");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let yaml = emitted_yaml(&capture_dir);
    assert!(yaml.contains("type: graph"), "{yaml}");
    assert!(!yaml.contains("continue_on"), "{yaml}");
    assert!(!yaml.contains("retry_policy"), "{yaml}");
    assert!(!yaml.contains("max_active_steps"), "{yaml}");
}

#[test]
fn second_adhoc_dir_does_not_reuse_dag_name() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"shared").expect("instructions");
    let worker = worker_json("true", &[]);
    let first = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &worker,
        ],
        b"",
    );
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let first_summary = parse_summary(&first.stdout);
    let first_dir = first_summary["output_dir"].as_str().expect("output_dir");
    let first_locator = read_locator(Path::new(first_dir));
    let second = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &worker,
        ],
        b"",
    );
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    let second_summary = parse_summary(&second.stdout);
    let second_dir = second_summary["output_dir"].as_str().expect("output_dir");
    assert_ne!(first_dir, second_dir);
    let second_locator = read_locator(Path::new(second_dir));
    assert_ne!(first_locator["dag_name"], second_locator["dag_name"]);
    assert_ne!(first_locator["dagu_home"], second_locator["dagu_home"]);
    let previous_home = Path::new(first_locator["dagu_home"].as_str().unwrap());
    let reused = previous_home.join("dags").join(format!(
        "{}.yaml",
        second_locator["dag_name"].as_str().unwrap()
    ));
    assert!(
        !reused.is_file(),
        "second dag_name reused under previous dagu-home: {}",
        reused.display()
    );
}

#[test]
fn killed_graph_before_join_still_writes_summary() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-killed");
    let worker = worker_json("sh", &["-c", "echo started; sleep 12; exit 3"]);
    let packet = invoke_packet(&artifact_root, &capture_dir, "kill-me");
    let child = spawn_fan_out(
        directory.path(),
        &["fan-out", "--worker", &worker],
        packet.as_bytes(),
    );
    let parent = child.id();
    let stdout_path = capture_dir.join("0").join("stdout");
    let stderr_path = capture_dir.join("0").join("stderr");
    assert!(
        wait_for_file(&stdout_path, Duration::from_secs(20)),
        "worker stdout never appeared"
    );
    assert!(capture_dir.join("dagu-locator.json").is_file());
    kill_direct_children(parent);
    let output = super::bounded_process::wait_existing(child, "loop-engine killed fan-out")
        .expect("wait killed graph");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let captured = capture_summary(capture_dir.to_str().expect("utf-8"));
    let workers = captured["workers"].as_array().expect("workers");
    assert!(!workers.is_empty(), "{captured}");
    assert_eq!(workers[0]["command"], "sh");
    assert!(workers[0]["args"].is_array());
    assert!(workers[0]["exit_code"].is_number(), "{captured}");
    assert_eq!(
        workers[0]["stdout_path"].as_str().unwrap(),
        stdout_path.to_string_lossy()
    );
    assert_eq!(
        workers[0]["stderr_path"].as_str().unwrap(),
        stderr_path.to_string_lossy()
    );
    assert!(stdout_path.is_file());
    assert!(stderr_path.is_file());
}

#[test]
fn missing_join_helper_still_writes_summary_for_started_workers() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("inv-no-join");
    let worker = worker_json("sh", &["-c", "echo started; exit 0"]);
    let packet = invoke_packet(&artifact_root, &capture_dir, "no-join");
    let mut command = Command::new(env!("CARGO_BIN_EXE_loop-engine"));
    command
        .current_dir(directory.path())
        .args(["fan-out", "--worker", &worker])
        .env(
            "LOOP_ENGINE_FAN_OUT_JOIN_COMMAND",
            "/nonexistent/loop-engine-fan-out-join",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::bounded_process::prepare_process_group(&mut command);
    let mut child = command.spawn().expect("spawn fan-out");
    {
        let mut handle = child.stdin.take().expect("stdin");
        handle.write_all(packet.as_bytes()).expect("write packet");
    }
    let output = super::bounded_process::wait_existing(child, "loop-engine live fan-out")
        .expect("wait fan-out");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let stdout_path = capture_dir.join("0").join("stdout");
    let stderr_path = capture_dir.join("0").join("stderr");
    assert!(stdout_path.is_file(), "started stdout must exist");
    assert!(stderr_path.is_file(), "started stderr must exist");
    let captured = capture_summary(capture_dir.to_str().expect("utf-8"));
    assert_eq!(captured["workers"][0]["command"], "sh");
    assert_eq!(captured["workers"][0]["exit_code"], 0);
    assert_eq!(
        captured["workers"][0]["stdout_path"].as_str().unwrap(),
        stdout_path.to_string_lossy()
    );
    assert_eq!(
        captured["workers"][0]["stderr_path"].as_str().unwrap(),
        stderr_path.to_string_lossy()
    );
}

#[test]
fn spawn_failure_writes_summary_for_started_workers_and_exits_nonzero() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    std::fs::write(&instructions, b"payload").expect("instructions");
    let receipt = directory.path().join("started.stdin");
    let started = cat_worker(&receipt);
    let missing = worker_json(
        directory
            .path()
            .join("no-such-worker-binary")
            .to_str()
            .expect("utf-8"),
        &[],
    );
    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--instructions",
            instructions.to_str().expect("utf-8"),
            "--worker",
            &started,
            "--worker",
            &missing,
        ],
        b"",
    );
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("fan-out error") || stderr.contains("dagu"),
        "{stderr}"
    );
    let adhoc = directory.path().join("fan-out-adhoc");
    let mut captures = Vec::new();
    if adhoc.is_dir() {
        for entry in std::fs::read_dir(&adhoc).expect("adhoc dir") {
            let path = entry.expect("entry").path();
            if path.join("summary.json").is_file() {
                captures.push(path);
            }
        }
    }
    assert_eq!(
        captures.len(),
        1,
        "expected one ad-hoc capture with summary"
    );
    let captured = capture_summary(captures[0].to_str().expect("utf-8"));
    let workers = captured["workers"].as_array().expect("workers");
    assert!(!workers.is_empty(), "{captured}");
    assert_eq!(workers[0]["command"], "sh");
    assert!(captures[0].join("0").join("stdout").is_file());
}
