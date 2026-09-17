//! T10 public regression coverage for provider command discovery.

use serde_json::json;
use std::fs;
use std::process::{Command, Stdio};

#[test]
fn backlog_t10_run_plan_graph_rejects_multiline_task_worker_before_dagu() {
    let working_directory = std::env::current_dir().expect("current directory");
    let marker = std::env::temp_dir().join(format!(
        "software-change-backlog-t10-task-worker-marker-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&marker);
    let worker = json!({
        "command": "/bin/sh",
        "args": ["-c", format!("touch '{}'\n", marker.display())]
    })
    .to_string();

    let output = Command::new(workspace_integration::binary("software-change"))
        .args([
            "run-plan-graph",
            "--working-directory",
            working_directory.to_str().expect("working directory UTF-8"),
            "--task-worker",
            &worker,
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run-plan-graph process");

    assert_eq!(
        output.status.code(),
        Some(2),
        "unexpected output: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line break"), "unexpected stderr: {stderr}");
    assert!(!stderr.contains("dagu start"), "Dagu was reached: {stderr}");
    assert!(!marker.exists(), "malformed task worker was started");
    let _ = fs::remove_file(&marker);
}

#[test]
fn backlog_t10_run_plan_graph_help_is_discoverable_without_a_working_directory() {
    let output = Command::new(workspace_integration::binary("software-change"))
        .args(["run-plan-graph", "--help"])
        .output()
        .expect("run-plan-graph help");

    assert!(output.status.success(), "help failed: {output:?}");
    assert!(output.stderr.is_empty(), "help wrote stderr: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: software-change run-plan-graph"));
    assert!(stdout.contains("existing absolute working directory"));
    assert!(stdout.contains("mandatory summarizer"));
}
