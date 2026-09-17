use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn worker_json(command: &str, args: &[&str]) -> String {
    json!({"command": command, "args": args}).to_string()
}

fn contracted_worker_json(command: &str, args: &[&str], required: &[&str]) -> String {
    json!({
        "command": command,
        "args": args,
        "output_schema": {"required": required},
    })
    .to_string()
}

fn full_worker_json(command: &str, args: &[&str]) -> String {
    json!({
        "command": command,
        "args": args,
        "full_output_schema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["result"],
            "properties": {"result": {"type": "string"}},
        },
    })
    .to_string()
}

fn invoke_packet(
    artifact_root: &Path,
    capture_dir: &Path,
    instruction_body: &str,
    assignment_selection: Option<&[&str]>,
) -> String {
    let mut packet = json!({
        "run_id": "run-1",
        "slot_id": "slot-1",
        "artifact_root": artifact_root.to_string_lossy(),
        "instruction_body": instruction_body,
        "capture_dir": capture_dir.to_string_lossy(),
    });
    if let Some(selection) = assignment_selection {
        packet["assignment_selection"] = json!(selection);
    }
    packet.to_string()
}

fn run_fan_out(cwd: &Path, args: &[&str], stdin: &[u8]) -> std::process::Output {
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.current_dir(cwd).args(args);
    super::bounded_process::run_with_stdin(&mut command, "loop-engine fan-out", stdin)
        .expect("wait for fan-out")
        .output
}

fn parse_summary(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).unwrap_or_else(|error| {
        panic!(
            "fan-out summary must be JSON: {error}: {}",
            String::from_utf8_lossy(stdout)
        )
    })
}

fn capture_summary(capture_dir: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(capture_dir.join("summary.json")).expect("read fan-out summary"),
    )
    .expect("summary JSON")
}

#[test]
fn backlog_t04_barrier_orders_groups_and_does_not_forward_first_output() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("ordered");
    let first_done = directory.path().join("first.done");
    let second_started = directory.path().join("second.started");
    let second_early = directory.path().join("second.early");
    let second_input = directory.path().join("second.stdin");

    let first_script = "cat >/dev/null; sleep 0.3; printf done > \"$1\"; printf '%s' '{\"result\":\"fail\",\"findings\":\"FIRST-STAGE-SECRET\"}'";
    let first = contracted_worker_json(
        "/bin/sh",
        &[
            "-c",
            first_script,
            "_",
            first_done.to_str().expect("first marker path"),
        ],
        &["result"],
    );
    let second_script = "if [ ! -f \"$1\" ]; then printf early > \"$2\"; fi; printf started > \"$3\"; cat > \"$4\"; if grep -q FIRST-STAGE-SECRET \"$4\"; then exit 41; fi";
    let second = worker_json(
        "/bin/sh",
        &[
            "-c",
            second_script,
            "_",
            first_done.to_str().expect("first marker path"),
            second_early.to_str().expect("early marker path"),
            second_started.to_str().expect("second marker path"),
            second_input.to_str().expect("second input path"),
        ],
    );
    let packet = invoke_packet(&artifact_root, &capture_dir, "frozen instructions", None);

    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &first, "--then", "--worker", &second],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(first_done.is_file(), "first group must finish");
    assert!(second_started.is_file(), "second group must run");
    assert!(
        !second_early.exists(),
        "second group started before the first group completed"
    );
    assert_eq!(
        fs::read(&second_input).expect("second stdin"),
        (serde_json::to_string(&json!({
            "artifact_root": artifact_root.to_string_lossy(),
        }))
        .expect("compact location JSON")
            + "\n")
            .as_bytes()
    );
    assert!(
        !String::from_utf8_lossy(&fs::read(&second_input).expect("second stdin"))
            .contains("FIRST-STAGE-SECRET"),
        "first-stage output leaked into second-stage input"
    );

    let summary = parse_summary(&output.stdout);
    // bookends:LE-138 — the public fan-out path preserves both assignments,
    // the barrier, and the absence of first-stage output leakage.
    assert_eq!(summary["workers"][0]["assignment_id"], "worker-0");
    assert_eq!(summary["workers"][1]["assignment_id"], "worker-1");
    assert_eq!(summary["workers"][0]["status"], "succeeded");
    assert_eq!(summary["workers"][1]["exit_code"], 0);
    assert_eq!(
        capture_summary(&capture_dir)["workers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let locator: Value =
        serde_json::from_slice(&fs::read(capture_dir.join("dagu-locator.json")).expect("locator"))
            .expect("locator JSON");
    let dagu_home = Path::new(locator["dagu_home"].as_str().expect("dagu home"));
    let yaml = fs::read_to_string(
        dagu_home
            .join("dags")
            .join(format!("{}.yaml", locator["dag_name"].as_str().unwrap())),
    )
    .expect("emitted graph YAML");
    assert!(yaml.contains("name: \"barrier\""), "{yaml}");
    assert!(
        yaml.contains("name: \"w1\"\n    action: exec\n    depends:\n      - \"barrier\""),
        "{yaml}"
    );
    assert!(!yaml.contains("max_active_steps"), "{yaml}");
}

#[test]
fn backlog_t04_divider_shapes_fail_before_workers_and_preview_lists_both_groups() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    fs::write(&instructions, b"unchanged instruction bytes").expect("instructions");
    let receipt = directory.path().join("must-not-start");
    let worker = worker_json(
        "/bin/sh",
        &[
            "-c",
            "cat >/dev/null; printf started > \"$1\"",
            "_",
            receipt.to_str().expect("receipt path"),
        ],
    );

    for args in [
        vec![
            "fan-out",
            "--instructions",
            instructions.to_str().expect("instructions path"),
            "--then",
            "--worker",
            worker.as_str(),
        ],
        vec![
            "fan-out",
            "--instructions",
            instructions.to_str().expect("instructions path"),
            "--worker",
            worker.as_str(),
            "--then",
        ],
        vec![
            "fan-out",
            "--instructions",
            instructions.to_str().expect("instructions path"),
            "--worker",
            worker.as_str(),
            "--then",
            "--then",
            "--worker",
            worker.as_str(),
        ],
    ] {
        let output = run_fan_out(directory.path(), &args, b"");
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(!receipt.exists(), "malformed divider launched a worker");
    }

    let valid = json!({
        "review": {
            "command": workspace_integration::binary_string("loop-engine"),
            "args": [
                "fan-out",
                "--worker", worker,
                "--then",
                "--worker", json!({"command":"/bin/true","args":[]}).to_string(),
            ],
        }
    })
    .to_string();
    let preview = Command::new(workspace_integration::binary("loop-engine"))
        .args(["preview-bindings", &valid])
        .bounded_output("loop-engine preview-bindings")
        .expect("preview bindings");
    assert_eq!(preview.status.code(), Some(0), "{preview:?}");
    let report: Value = serde_json::from_slice(&preview.stdout).expect("preview report");
    assert_eq!(
        report["bindings"][0]["workers"].as_array().unwrap().len(),
        2
    );
    assert!(report["bindings"][0]["args"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "--then"));
}

#[test]
fn backlog_t04_max_active_caps_a_group_and_preserves_ad_hoc_instruction_bytes() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions");
    let instruction_bytes = b"instruction-bytes-without-newline";
    fs::write(&instructions, instruction_bytes).expect("instructions");
    let lock = directory.path().join("active.lock");
    let overlap = directory.path().join("overlap");
    let first_input = directory.path().join("first.input");
    let second_input = directory.path().join("second.input");
    let third_input = directory.path().join("third.input");

    let capped_script = "cat > \"$3\"; if mkdir \"$1\" 2>/dev/null; then sleep 0.25; rmdir \"$1\"; else printf overlap > \"$2\"; sleep 0.25; fi";
    let first = worker_json(
        "/bin/sh",
        &[
            "-c",
            capped_script,
            "_",
            lock.to_str().expect("lock path"),
            overlap.to_str().expect("overlap path"),
            first_input.to_str().expect("first input path"),
        ],
    );
    let second = worker_json(
        "/bin/sh",
        &[
            "-c",
            capped_script,
            "_",
            lock.to_str().expect("lock path"),
            overlap.to_str().expect("overlap path"),
            second_input.to_str().expect("second input path"),
        ],
    );
    let third = worker_json(
        "/bin/sh",
        &[
            "-c",
            "cat > \"$1\"",
            "_",
            third_input.to_str().expect("third input path"),
        ],
    );
    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out",
            "--max-active",
            "1",
            "--instructions",
            instructions.to_str().expect("instructions path"),
            "--worker",
            &first,
            "--worker",
            &second,
            "--then",
            "--worker",
            &third,
        ],
        b"",
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    for input in [&first_input, &second_input, &third_input] {
        assert_eq!(
            fs::read(input).expect("worker instruction capture"),
            instruction_bytes
        );
    }
    assert!(
        !overlap.exists(),
        "--max-active 1 allowed overlapping workers"
    );
    let summary = parse_summary(&output.stdout);
    let workers = summary["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 3);
    for worker in workers {
        assert_eq!(worker["exit_code"], 0);
        assert!(
            worker.get("status").is_none(),
            "uncontracted output: {worker}"
        );
        let stdout = Path::new(worker["stdout_path"].as_str().expect("stdout path"));
        assert_eq!(fs::read(stdout).expect("empty stdout"), b"");
    }
    let capture_dir = Path::new(summary["output_dir"].as_str().expect("output dir"));
    let yaml = {
        let locator: Value = serde_json::from_slice(
            &fs::read(capture_dir.join("dagu-locator.json")).expect("locator"),
        )
        .expect("locator JSON");
        let home = Path::new(locator["dagu_home"].as_str().expect("dagu home"));
        fs::read_to_string(
            home.join("dags")
                .join(format!("{}.yaml", locator["dag_name"].as_str().unwrap())),
        )
        .expect("graph YAML")
    };
    assert!(yaml.contains("max_active_steps: 1"), "{yaml}");
}

#[test]
fn backlog_t04_selected_groups_keep_assignment_ids_and_skip_unselected_workers() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("selected");
    let first_done = directory.path().join("selected-first.done");
    let unselected = directory.path().join("unselected.started");
    let second_started = directory.path().join("selected-second.started");

    let first = worker_json(
        "/bin/sh",
        &[
            "-c",
            "cat >/dev/null; sleep 0.2; printf done > \"$1\"",
            "_",
            first_done.to_str().expect("first marker path"),
        ],
    );
    let skipped = worker_json(
        "/bin/sh",
        &[
            "-c",
            "printf started > \"$1\"",
            "_",
            unselected.to_str().expect("unselected path"),
        ],
    );
    let second = worker_json(
        "/bin/sh",
        &[
            "-c",
            "if [ ! -f \"$1\" ]; then exit 42; fi; cat >/dev/null; printf started > \"$2\"",
            "_",
            first_done.to_str().expect("first marker path"),
            second_started.to_str().expect("second marker path"),
        ],
    );
    let packet = invoke_packet(
        &artifact_root,
        &capture_dir,
        "selection does not rewrite the binding",
        Some(&["worker-2", "worker-0"]),
    );

    let output = run_fan_out(
        directory.path(),
        &[
            "fan-out", "--worker", &first, "--worker", &skipped, "--then", "--worker", &second,
        ],
        packet.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!unselected.exists(), "unselected worker started");
    assert!(first_done.is_file());
    assert!(second_started.is_file());
    let summary = parse_summary(&output.stdout);
    let workers = summary["workers"].as_array().expect("workers");
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0]["assignment_id"], "worker-0");
    assert_eq!(workers[1]["assignment_id"], "worker-2");
    assert_eq!(workers[0]["exit_code"], 0);
    assert_eq!(workers[1]["exit_code"], 0);
}

#[test]
fn backlog_t04_first_group_exhausted_conformance_blocks_second_and_keeps_attempts() {
    let directory = tempdir().expect("tempdir");
    let artifact_root = directory.path().join("artifacts");
    let capture_dir = directory.path().join("captures").join("exhausted");
    let second_started = directory.path().join("second.started");
    let invalid = r#"{"wrong":true}"#;
    let first = full_worker_json(
        "/bin/sh",
        &["-c", &format!("cat >/dev/null; printf '%s' '{invalid}'")],
    );
    let second = worker_json(
        "/bin/sh",
        &[
            "-c",
            "printf started > \"$1\"",
            "_",
            second_started.to_str().expect("second path"),
        ],
    );
    let packet = invoke_packet(&artifact_root, &capture_dir, "frozen", None);
    let output = run_fan_out(
        directory.path(),
        &["fan-out", "--worker", &first, "--then", "--worker", &second],
        packet.as_bytes(),
    );
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !second_started.exists(),
        "second group ran after conformance failure"
    );

    let summary = capture_summary(&capture_dir);
    assert_eq!(summary["workers"][0]["assignment_id"], "worker-0");
    assert_eq!(summary["workers"][0]["status"], "failed");
    assert_eq!(summary["workers"][0]["selected_attempt"], Value::Null);
    assert_eq!(summary["workers"][1]["assignment_id"], "worker-1");
    assert!(summary["workers"][1].get("status").is_none());

    let worker_dir = capture_dir.join("0");
    let manifest: Value = serde_json::from_slice(
        &fs::read(worker_dir.join("attempts.json")).expect("attempt manifest"),
    )
    .expect("attempt manifest JSON");
    assert_eq!(manifest["exhausted"], true);
    assert_eq!(manifest["attempts"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read(worker_dir.join("attempts/1/stdout")).expect("first raw attempt"),
        invalid.as_bytes()
    );
    assert_eq!(
        fs::read(worker_dir.join("attempts/2/stdout")).expect("second raw attempt"),
        invalid.as_bytes()
    );

    // Give a mistakenly admitted second worker a chance to leave its marker
    // before the test finishes, making the negative assertion deterministic.
    thread::sleep(Duration::from_millis(50));
    assert!(!second_started.exists(), "second group was admitted late");
}
