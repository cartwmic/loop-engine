use serde_json::{json, Value};
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

fn output_worker(
    receipt: &Path,
    stdout: &str,
    exit_code: i32,
    preamble: Option<&str>,
    required: &[&str],
) -> String {
    contracted_worker_json(
        "sh",
        &[
            "-c",
            &format!("cat > \"$1\"; printf %s \"$2\"; exit {exit_code}"),
            "_",
            receipt.to_str().expect("utf-8 receipt"),
            stdout,
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_loop-engine"))
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loop-engine fan-out");
    {
        let mut handle = child.stdin.take().expect("fan-out stdin");
        handle.write_all(stdin).expect("write fan-out stdin");
    }
    child.wait_with_output().expect("wait for fan-out")
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
    let expected = format!(
        "Review the design\n\nrun_id: run-1\nslot_id: slot-1\nartifact_root: {}\n",
        artifact_root.to_string_lossy()
    );
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
fn bound_preamble_has_exact_context_separator_legacy_body_and_trailer_order() {
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
    let legacy = format!(
        "Review the design\n\nrun_id: run-1\nslot_id: slot-1\nartifact_root: {}\n",
        artifact_root.to_string_lossy()
    );
    let expected = format!("{preamble}\n{context}\n---\n\n{legacy}");
    let recorded = std::fs::read(&receipt).expect("framed stdin");
    assert_eq!(recorded, expected.as_bytes());
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
    let expected = format!(
        "judge this\n\nrun_id: run-1\nslot_id: slot-1\nartifact_root: {}\n",
        artifact_root.to_string_lossy()
    );
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
        .output()
        .expect("run --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fan-out"), "{stdout}");
    assert!(stdout.contains("--worker"), "{stdout}");
    assert!(stdout.contains("--instructions"), "{stdout}");
    assert!(!stdout.contains("wait-invocation"), "{stdout}");
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
    let output = handle.join().expect("fan-out thread");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        both_started,
        "both workers must start before either finishes reading a 2MiB payload"
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
import os, sys, time
bin, instructions = sys.argv[1], sys.argv[2]
worker = sys.argv[3]
pid, fd = os.forkpty()
if pid == 0:
    os.execv(bin, [bin, "fan-out", "--instructions", instructions, "--worker", worker])
deadline = time.time() + 3
while time.time() < deadline:
    wpid, status = os.waitpid(pid, os.WNOHANG)
    if wpid == pid:
        raise SystemExit(0 if os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0 else 1)
    time.sleep(0.05)
try:
    os.kill(pid, 9)
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
        ])
        .output()
        .expect("run pty helper");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ad-hoc fan-out hung on terminal stdin: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
