use super::bounded_process::CommandExt;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

const INSPECTOR: &str = r#"
import json, os, sys
from pathlib import Path

out = Path(sys.argv[1])
code = int(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
stdin = sys.stdin.buffer.read()
(out / "argv.json").write_text(json.dumps(sys.argv[1:]), encoding="utf-8")
(out / "stdin.bin").write_bytes(stdin)
(out / "env.json").write_text(json.dumps(dict(os.environ)), encoding="utf-8")
raise SystemExit(code)
"#;

fn engine() -> Command {
    Command::new(env!("CARGO_BIN_EXE_loop-engine"))
}

fn write_inspector(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("inspector.py");
    fs::write(&path, INSPECTOR).expect("write inspector");
    path
}

fn sidecar_exit_code(path: &Path) -> i32 {
    let parsed: Value =
        serde_json::from_slice(&fs::read(path).expect("read sidecar")).expect("sidecar json");
    parsed["exit_code"]
        .as_i64()
        .expect("exit_code i64")
        .try_into()
        .expect("exit_code i32")
}

#[test]
fn json_bearing_worker_args_are_literal() {
    let directory = tempdir().expect("tempdir");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let sidecar = directory
        .path()
        .join("capture")
        .join("0")
        .join("inner_exit.json");
    let out = directory.path().join("inspect");
    let json_arg = r#"{"required":["key"],"nested":{"a":"b c"}}"#;
    fs::write(&stdin_file, b"duty-text").expect("write stdin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
            json_arg,
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(sidecar_exit_code(&sidecar), 0);

    let argv: Vec<String> =
        serde_json::from_str(&fs::read_to_string(out.join("argv.json")).expect("argv"))
            .expect("argv json");
    assert_eq!(argv[2], json_arg);
}

#[test]
fn colocates_pi_session_dir_when_unset_and_leaves_argv_unchanged() {
    let directory = tempdir().expect("tempdir");
    let inspector = write_inspector(directory.path());
    let worker_dir = directory.path().join("worker");
    fs::create_dir_all(&worker_dir).expect("worker dir");
    let stdin_file = worker_dir.join("stdin");
    let sidecar = worker_dir.join("inner_exit.json");
    let out = directory.path().join("inspect");
    let frozen = r#"{"frozen":true}"#;
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = engine()
        .env_remove("PI_CODING_AGENT_SESSION_DIR")
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
            frozen,
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(sidecar_exit_code(&sidecar), 0);

    let sessions = worker_dir.join("sessions");
    assert!(
        sessions.is_dir(),
        "sessions directory must be created at {}",
        sessions.display()
    );

    let env: Value = serde_json::from_str(&fs::read_to_string(out.join("env.json")).expect("env"))
        .expect("env json");
    assert_eq!(
        env["PI_CODING_AGENT_SESSION_DIR"].as_str(),
        Some(sessions.to_str().expect("utf-8"))
    );

    let argv: Vec<String> =
        serde_json::from_str(&fs::read_to_string(out.join("argv.json")).expect("argv"))
            .expect("argv json");
    assert_eq!(
        argv,
        vec![
            out.to_str().expect("utf-8").to_owned(),
            "0".to_owned(),
            frozen.to_owned(),
        ]
    );
    assert!(
        argv.iter().all(|arg| arg != "--session-dir"),
        "child argv must not gain --session-dir: {argv:?}"
    );
}

#[test]
fn preserves_inherited_pi_session_dir() {
    let directory = tempdir().expect("tempdir");
    let inspector = write_inspector(directory.path());
    let worker_dir = directory.path().join("worker");
    fs::create_dir_all(&worker_dir).expect("worker dir");
    let stdin_file = worker_dir.join("stdin");
    let sidecar = worker_dir.join("inner_exit.json");
    let out = directory.path().join("inspect");
    let preset = directory.path().join("preset-sessions");
    fs::create_dir_all(&preset).expect("preset sessions");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = engine()
        .env("PI_CODING_AGENT_SESSION_DIR", &preset)
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(sidecar_exit_code(&sidecar), 0);

    assert!(
        !worker_dir.join("sessions").exists(),
        "inherited PI_CODING_AGENT_SESSION_DIR must not create <stdin-file parent>/sessions"
    );

    let env: Value = serde_json::from_str(&fs::read_to_string(out.join("env.json")).expect("env"))
        .expect("env json");
    assert_eq!(
        env["PI_CODING_AGENT_SESSION_DIR"].as_str(),
        Some(preset.to_str().expect("utf-8"))
    );

    let argv: Vec<String> =
        serde_json::from_str(&fs::read_to_string(out.join("argv.json")).expect("argv"))
            .expect("argv json");
    assert!(
        argv.iter().all(|arg| arg != "--session-dir"),
        "child argv must not gain --session-dir: {argv:?}"
    );
}

#[test]
fn binary_stdin_bytes_are_delivered_intact() {
    let directory = tempdir().expect("tempdir");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let sidecar = directory.path().join("inner_exit.json");
    let out = directory.path().join("inspect");
    let duty = b"\x00\xff\xfe{not utf8}\n\x80";
    fs::write(&stdin_file, duty).expect("write stdin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let captured = fs::read(out.join("stdin.bin")).expect("read captured stdin");
    assert_eq!(captured, duty);
}

#[test]
fn argv_and_env_omit_duty_bytes() {
    let directory = tempdir().expect("tempdir");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let sidecar = directory.path().join("inner_exit.json");
    let out = directory.path().join("inspect");
    let duty = "UNIQUE-STDIN-EXEC-DUTY-BYTES-7f3a";
    fs::write(&stdin_file, duty.as_bytes()).expect("write stdin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let argv: Vec<String> =
        serde_json::from_str(&fs::read_to_string(out.join("argv.json")).expect("argv"))
            .expect("argv json");
    assert!(
        argv.iter().all(|arg| arg != duty),
        "duty bytes must not appear on child argv: {argv:?}"
    );

    let env: Value = serde_json::from_str(&fs::read_to_string(out.join("env.json")).expect("env"))
        .expect("env json");
    let env_obj = env.as_object().expect("env object");
    for (key, value) in env_obj {
        let Some(text) = value.as_str() else {
            continue;
        };
        assert_ne!(
            text, duty,
            "child environment key `{key}` must not equal stdin-file contents"
        );
    }
}

#[test]
fn sidecar_records_waitpid_then_helper_exits_0() {
    let directory = tempdir().expect("tempdir");
    let stdin_file = directory.path().join("duty.bin");
    let sidecar = directory
        .path()
        .join("nested")
        .join("0")
        .join("inner_exit.json");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            "python3",
            "-c",
            "raise SystemExit(3)",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let parsed: Value =
        serde_json::from_slice(&fs::read(&sidecar).expect("read sidecar")).expect("sidecar json");
    assert_eq!(parsed, serde_json::json!({"exit_code": 3}));
}

#[test]
fn propagate_mode_uses_inner_waitpid_and_rejects_sidecar_file() {
    let directory = tempdir().expect("tempdir");
    let stdin_file = directory.path().join("duty.bin");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let rejected = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--sidecar-file",
            directory
                .path()
                .join("inner_exit.json")
                .to_str()
                .expect("utf-8"),
            "--",
            "python3",
            "-c",
            "raise SystemExit(0)",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec propagate with sidecar");
    assert_ne!(rejected.status.code(), Some(0), "{rejected:?}");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            "-c",
            "raise SystemExit(3)",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec propagate");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
}

#[test]
fn spawn_failure_exits_nonzero_without_sidecar() {
    let directory = tempdir().expect("tempdir");
    let stdin_file = directory.path().join("duty.bin");
    let sidecar = directory.path().join("inner_exit.json");
    fs::write(&stdin_file, b"duty").expect("write stdin");
    let missing = directory.path().join("missing-stdin-exec-bin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            missing.to_str().expect("utf-8"),
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec missing binary");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !sidecar.exists(),
        "spawn failure must not write a successful sidecar"
    );

    let not_executable = directory.path().join("not-executable");
    fs::write(&not_executable, b"#!/bin/sh\nexit 0\n").expect("write not-executable");
    let mut permissions = fs::metadata(&not_executable)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&not_executable, permissions).expect("chmod");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            sidecar.to_str().expect("utf-8"),
            "--",
            not_executable.to_str().expect("utf-8"),
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec not executable");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !sidecar.exists(),
        "not-executable spawn failure must not write a successful sidecar"
    );
}

#[test]
fn missing_command_exits_nonzero() {
    let directory = tempdir().expect("tempdir");
    let stdin_file = directory.path().join("duty.bin");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec without COMMAND");
    assert_ne!(output.status.code(), Some(0), "{output:?}");

    let output = engine()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            directory
                .path()
                .join("inner_exit.json")
                .to_str()
                .expect("utf-8"),
            "--",
        ])
        .bounded_output("loop-engine stdin-exec")
        .expect("run stdin-exec with empty command after --");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn help_omits_stdin_exec() {
    let help = engine()
        .arg("--help")
        .bounded_output("loop-engine stdin-exec")
        .expect("run --help");
    assert!(help.status.success());
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(
        !stdout.contains("stdin-exec"),
        "help must not mention hidden stdin-exec: {stdout}"
    );

    let fan_out_help = engine()
        .args(["fan-out", "--help"])
        .bounded_output("loop-engine stdin-exec")
        .expect("run fan-out --help");
    assert!(fan_out_help.status.success());
    let fan_out_stdout = String::from_utf8_lossy(&fan_out_help.stdout);
    assert!(
        !fan_out_stdout.contains("stdin-exec"),
        "fan-out help must not mention hidden stdin-exec: {fan_out_stdout}"
    );
}
