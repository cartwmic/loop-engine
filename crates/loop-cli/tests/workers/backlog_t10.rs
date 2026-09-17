//! T10 public regression coverage for worker argv and fan-out launch boundaries.

use super::bounded_process::run_with_stdin;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn backlog_t10_fan_out_rejects_multiline_worker_argv_before_dagu() {
    let directory = tempdir().expect("tempdir");
    let instructions = directory.path().join("instructions.txt");
    let marker = directory.path().join("worker-started");
    fs::write(&instructions, b"frozen instructions\n").expect("instructions");

    let worker = json!({
        "command": "/bin/sh",
        "args": [
            "-c",
            format!("printf started > '{}'\n", marker.display())
        ]
    })
    .to_string();
    let mut command = Command::new(workspace_integration::binary("loop-engine"));
    command.current_dir(directory.path()).args([
        "fan-out",
        "--instructions",
        instructions.to_str().expect("instructions path"),
        "--worker",
        &worker,
    ]);
    let completed = run_with_stdin(&mut command, "backlog_t10 multiline worker argv", &[])
        .expect("fan-out process");

    assert_eq!(completed.output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&completed.output.stderr);
    assert!(stderr.contains("line break"), "unexpected stderr: {stderr}");
    assert!(!marker.exists(), "Dagu admitted the malformed worker");
    assert!(
        !directory.path().join("fan-out-adhoc").exists(),
        "malformed argv created a capture graph"
    );
}
