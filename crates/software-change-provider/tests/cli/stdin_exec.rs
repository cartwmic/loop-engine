use super::bounded_process::CommandExt;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-stdin-exec-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_software-change"))
}

fn write_inspector(directory: &Path) -> PathBuf {
    let path = directory.join("inspector.py");
    fs::write(&path, INSPECTOR).expect("write inspector");
    path
}

#[test]
fn json_bearing_worker_args_are_literal() {
    let directory = TestDir::new("json-args");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let out = directory.path().join("inspect");
    let json_arg = r#"{"required":["key"],"nested":{"a":"b c"}}"#;
    fs::write(&stdin_file, b"duty-text").expect("write stdin");

    let output = bin()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
            json_arg,
        ])
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let argv: Vec<String> =
        serde_json::from_str(&fs::read_to_string(out.join("argv.json")).expect("argv"))
            .expect("argv json");
    assert_eq!(argv[2], json_arg);
}

#[test]
fn colocates_pi_session_dir_when_unset_and_leaves_argv_unchanged() {
    let directory = TestDir::new("session-colocate");
    let inspector = write_inspector(directory.path());
    let worker_dir = directory.path().join("worker");
    fs::create_dir_all(&worker_dir).expect("worker dir");
    let stdin_file = worker_dir.join("stdin");
    let out = directory.path().join("inspect");
    let frozen = r#"{"frozen":true}"#;
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = bin()
        .env_remove("PI_CODING_AGENT_SESSION_DIR")
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
            frozen,
        ])
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");

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
    let directory = TestDir::new("session-preserve");
    let inspector = write_inspector(directory.path());
    let worker_dir = directory.path().join("worker");
    fs::create_dir_all(&worker_dir).expect("worker dir");
    let stdin_file = worker_dir.join("stdin");
    let out = directory.path().join("inspect");
    let preset = directory.path().join("preset-sessions");
    fs::create_dir_all(&preset).expect("preset sessions");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let output = bin()
        .env("PI_CODING_AGENT_SESSION_DIR", &preset)
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");

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
    let directory = TestDir::new("binary-stdin");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let out = directory.path().join("inspect");
    let duty = b"\x00\xff\xfe{not utf8}\n\x80";
    fs::write(&stdin_file, duty).expect("write stdin");

    let output = bin()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let captured = fs::read(out.join("stdin.bin")).expect("read captured stdin");
    assert_eq!(captured, duty);
}

#[test]
fn argv_and_env_omit_duty_bytes() {
    let directory = TestDir::new("omit-duty");
    let inspector = write_inspector(directory.path());
    let stdin_file = directory.path().join("duty.bin");
    let out = directory.path().join("inspect");
    let duty = "UNIQUE-STDIN-EXEC-DUTY-BYTES-7f3a";
    fs::write(&stdin_file, duty.as_bytes()).expect("write stdin");

    let output = bin()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            "python3",
            inspector.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
            "0",
        ])
        .bounded_output("software-change stdin-exec")
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
fn propagate_mode_uses_inner_waitpid_and_rejects_sidecar_file() {
    let directory = TestDir::new("propagate");
    let stdin_file = directory.path().join("duty.bin");
    fs::write(&stdin_file, b"duty").expect("write stdin");

    let rejected = bin()
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
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec propagate with sidecar");
    assert_ne!(rejected.status.code(), Some(0), "{rejected:?}");

    let output = bin()
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
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec propagate");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
}

#[test]
fn spawn_failure_exits_nonzero() {
    let directory = TestDir::new("spawn-failure");
    let stdin_file = directory.path().join("duty.bin");
    fs::write(&stdin_file, b"duty").expect("write stdin");
    let missing = directory.path().join("missing-stdin-exec-bin");

    let output = bin()
        .args([
            "stdin-exec",
            "--stdin-file",
            stdin_file.to_str().expect("utf-8"),
            "--exit-mode",
            "propagate",
            "--",
            missing.to_str().expect("utf-8"),
        ])
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec missing binary");
    assert_ne!(output.status.code(), Some(0), "{output:?}");

    let sidecar = directory.path().join("inner_exit.json");
    let not_executable = directory.path().join("not-executable");
    fs::write(&not_executable, b"#!/bin/sh\nexit 0\n").expect("write not-executable");
    let mut permissions = fs::metadata(&not_executable)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&not_executable, permissions).expect("chmod");

    let output = bin()
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
        .bounded_output("software-change stdin-exec")
        .expect("run stdin-exec not executable");
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !sidecar.exists(),
        "spawn failure must not write a successful sidecar"
    );
}

#[test]
fn help_and_version_omit_stdin_exec() {
    for flag in ["--help", "-h"] {
        let help = bin()
            .arg(flag)
            .bounded_output("software-change stdin-exec")
            .expect("run help");
        assert!(help.status.success(), "flag={flag}: {help:?}");
        let stdout = String::from_utf8_lossy(&help.stdout);
        assert!(
            !stdout.contains("stdin-exec"),
            "help must not mention hidden stdin-exec: {stdout}"
        );
    }

    for flag in ["--version", "-V"] {
        let version = bin()
            .arg(flag)
            .bounded_output("software-change stdin-exec")
            .expect("run version");
        assert!(version.status.success(), "flag={flag}: {version:?}");
        let stdout = String::from_utf8_lossy(&version.stdout);
        assert!(
            !stdout.contains("stdin-exec"),
            "version must not mention hidden stdin-exec: {stdout}"
        );
    }
}
