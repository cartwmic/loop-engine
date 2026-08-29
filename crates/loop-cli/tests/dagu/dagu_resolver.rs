use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

use loop_cli::{names_for_capture_root, write_locator, DaguLocator};

fn engine() -> Command {
    Command::new(workspace_integration::binary("loop-engine"))
}

fn prepend_path(directory: &Path) -> OsString {
    let mut dirs = vec![directory.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&existing));
    }
    env::join_paths(dirs).expect("join PATH")
}

fn path_without_dagu() -> OsString {
    let mut dirs = Vec::new();
    if let Some(existing) = env::var_os("PATH") {
        for dir in env::split_paths(&existing) {
            if dir.join("dagu").is_file() {
                continue;
            }
            dirs.push(dir);
        }
    }
    dirs.push(PathBuf::from("/bin"));
    dirs.push(PathBuf::from("/usr/bin"));
    env::join_paths(dirs).expect("join PATH without dagu")
}

fn write_version_stub(directory: &Path, version_line: &str) -> PathBuf {
    let path = directory.join("dagu");
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' '{version_line}'\n"),
    )
    .expect("write dagu stub");
    let mut permissions = fs::metadata(&path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod dagu stub");
    path
}

/// Prints `version` for the resolver gate; other argv exits nonzero so a
/// version-ok stub is not mistaken for a real Dagu graph runner.
fn write_version_ok_stub(directory: &Path, version_line: &str) -> PathBuf {
    let path = directory.join("dagu");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  version|--version) printf '%s\\n' '{version_line}'; exit 0 ;;\n  *) printf 'stub-not-real-dagu\\n' >&2; exit 42 ;;\nesac\n"
        ),
    )
    .expect("write version-ok stub");
    let mut permissions = fs::metadata(&path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod dagu stub");
    path
}

fn write_non_executable_stub(directory: &Path) -> PathBuf {
    let path = directory.join("dagu");
    fs::write(&path, "#!/bin/sh\necho 2.14.0\n").expect("write non-executable dagu");
    let mut permissions = fs::metadata(&path).expect("stub metadata").permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&path, permissions).expect("chmod non-executable dagu");
    path
}

fn fan_out_operand() -> String {
    json!({
        "design-review": {
            "command": "loop-engine",
            "args": [
                "fan-out",
                "--worker",
                json!({"command": "/bin/echo", "args": ["ok"]}).to_string()
            ]
        }
    })
    .to_string()
}

fn run_fan_out_with_path(path: OsString, receipt: &Path) -> std::process::Output {
    let directory = receipt.parent().expect("receipt parent");
    let instructions = directory.join("instructions.txt");
    fs::write(&instructions, "duty").expect("write instructions");
    let worker = json!({
        "command": "sh",
        "args": [
            "-c",
            "echo spawned > \"$1\"",
            "_",
            receipt.to_str().expect("utf-8 receipt")
        ]
    })
    .to_string();
    engine()
        .args([
            "fan-out",
            "--worker",
            &worker,
            "--instructions",
            instructions.to_str().expect("utf-8 instructions"),
        ])
        .env("PATH", path)
        .bounded_output("loop-engine dagu-resolver")
        .expect("run fan-out")
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn stub_reporting_2_14_0_is_accepted() {
    let directory = tempdir().expect("tempdir");
    let stub = write_version_ok_stub(directory.path(), "2.14.0");
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(prepend_path(directory.path()), &receipt);
    assert_ne!(
        output.status.code(),
        Some(0),
        "a version-ok stub is not a real dagu start; stdout={}",
        stdout_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(
        !stderr.contains("reports version 2.14.0"),
        "2.14.0 stub must pass the version gate, stderr={stderr}"
    );
    assert!(
        stderr.contains("stub-not-real-dagu") || stderr.contains("dagu validate"),
        "version-ok stub should fail at validate/start, stderr={stderr}"
    );
    assert!(!receipt.exists(), "version-ok stub must not spawn a worker");
    assert!(stub.exists());
}

#[test]
fn stub_reporting_2_15_is_accepted() {
    let directory = tempdir().expect("tempdir");
    write_version_ok_stub(directory.path(), "2.15.1");
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(prepend_path(directory.path()), &receipt);
    assert_ne!(
        output.status.code(),
        Some(0),
        "a version-ok stub is not a real dagu start; stdout={}",
        stdout_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(
        !stderr.contains("reports version 2.15.1"),
        "2.15.1 stub must pass the version gate, stderr={stderr}"
    );
    assert!(
        stderr.contains("stub-not-real-dagu") || stderr.contains("dagu validate"),
        "version-ok stub should fail at validate/start, stderr={stderr}"
    );
    assert!(!receipt.exists(), "version-ok stub must not spawn a worker");
}

#[test]
fn stub_reporting_2_13_0_is_rejected_before_spawn() {
    let directory = tempdir().expect("tempdir");
    let stub = write_version_stub(directory.path(), "2.13.0");
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(prepend_path(directory.path()), &receipt);
    assert_ne!(output.status.code(), Some(0), "{}", stdout_text(&output));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(stderr.contains("2.13.0"), "{stderr}");
    assert!(
        stderr.contains(stub.to_str().expect("utf-8 stub")),
        "{stderr}"
    );
    assert!(!receipt.exists(), "too-old dagu must not spawn a worker");
}

#[test]
fn missing_path_dagu_error_names_required_version_and_does_not_spawn() {
    let directory = tempdir().expect("tempdir");
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(path_without_dagu(), &receipt);
    assert_ne!(output.status.code(), Some(0), "{}", stdout_text(&output));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(stderr.contains("PATH lookup found nothing"), "{stderr}");
    assert!(!receipt.exists(), "missing dagu must not spawn a worker");
}

#[test]
fn non_executable_dagu_is_rejected_and_names_path() {
    let directory = tempdir().expect("tempdir");
    let stub = write_non_executable_stub(directory.path());
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(prepend_path(directory.path()), &receipt);
    assert_ne!(output.status.code(), Some(0), "{}", stdout_text(&output));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(
        stderr.contains(stub.to_str().expect("utf-8 stub")),
        "{stderr}"
    );
    assert!(
        !receipt.exists(),
        "non-executable dagu must not spawn a worker"
    );
}

#[test]
fn non_semver_stub_is_rejected() {
    let directory = tempdir().expect("tempdir");
    write_version_stub(directory.path(), "development");
    let receipt = directory.path().join("spawned");
    let output = run_fan_out_with_path(prepend_path(directory.path()), &receipt);
    assert_ne!(output.status.code(), Some(0));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("2.14.0"), "{stderr}");
    assert!(!receipt.exists());
}

#[test]
fn preview_bindings_with_dagu_missing_exits_0_and_names_2_14_0() {
    let operand = fan_out_operand();
    let output = engine()
        .args(["preview-bindings", &operand])
        .env("PATH", path_without_dagu())
        .bounded_output("loop-engine dagu-resolver")
        .expect("run preview-bindings");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr_text(&output),
        stdout_text(&output)
    );
    let stdout = stdout_text(&output);
    assert!(stdout.contains("2.14.0"), "{stdout}");
    assert!(stdout.contains("PATH lookup found nothing"), "{stdout}");
    let report: Value = serde_json::from_str(stdout.trim()).expect("preview JSON");
    assert_eq!(report["dagu"]["ok"], false);
    assert_eq!(report["dagu"]["required"], "2.14.0");
}

#[test]
fn preview_bindings_with_ok_dagu_reports_path_and_version() {
    let directory = tempdir().expect("tempdir");
    let stub = write_version_stub(directory.path(), "2.14.0");
    let operand = fan_out_operand();
    let output = engine()
        .args(["preview-bindings", &operand])
        .env("PATH", prepend_path(directory.path()))
        .bounded_output("loop-engine dagu-resolver")
        .expect("run preview-bindings");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={} stdout={}",
        stderr_text(&output),
        stdout_text(&output)
    );
    let report: Value = serde_json::from_str(stdout_text(&output).trim()).expect("preview JSON");
    assert_eq!(report["dagu"]["ok"], true);
    assert_eq!(report["dagu"]["version"], "2.14.0");
    assert_eq!(report["dagu"]["required"], "2.14.0");
    let path = report["dagu"]["path"].as_str().expect("dagu path");
    assert_eq!(path, stub.to_str().expect("utf-8 stub"));
}

#[test]
fn write_locator_uses_isolated_home_and_unique_fanout_names() {
    let first_dir = tempdir().expect("tempdir a");
    let second_dir = tempdir().expect("tempdir b");
    let first = first_dir.path().join("capture-one");
    let second = second_dir.path().join("capture-two");
    fs::create_dir_all(&first).expect("mkdir first");
    fs::create_dir_all(&second).expect("mkdir second");

    let first_locator = write_locator(&first, "fanout-capture-one", "fanout-capture-one")
        .expect("write first locator");
    let second_locator = write_locator(&second, "fanout-capture-two", "fanout-capture-two")
        .expect("write second locator");

    let first_path = first.join("dagu-locator.json");
    let parsed: DaguLocator =
        serde_json::from_slice(&fs::read(&first_path).expect("read locator")).expect("json");
    let keys: serde_json::Map<String, Value> =
        serde_json::from_slice(&fs::read(&first_path).expect("read locator")).expect("object");
    assert_eq!(keys.len(), 3);
    assert!(keys.contains_key("dagu_home"));
    assert!(keys.contains_key("dag_name"));
    assert!(keys.contains_key("run_name"));
    assert_eq!(parsed, first_locator);
    assert!(Path::new(&parsed.dagu_home).is_absolute());
    assert_eq!(
        Path::new(&parsed.dagu_home),
        fs::canonicalize(first.join("dagu-home")).expect("canonicalize home")
    );
    assert_eq!(parsed.dag_name, "fanout-capture-one");
    assert_eq!(parsed.run_name, "fanout-capture-one");
    assert!(!parsed.dag_name.is_empty());
    assert_ne!(first_locator.dag_name, second_locator.dag_name);
    assert_ne!(first_locator.run_name, second_locator.run_name);
    assert_ne!(first_locator.dagu_home, second_locator.dagu_home);
    assert!(first.join("dagu-home").is_dir());
}

#[test]
fn names_for_long_invocation_dir_stay_under_dagu_limit() {
    let root = tempdir().expect("tempdir");
    let first = root.path().join("invocation-1787044324400584000-1-89864");
    let second = root.path().join("invocation-1787044324400584000-1-89865");
    fs::create_dir_all(&first).expect("mkdir first");
    fs::create_dir_all(&second).expect("mkdir second");
    let (first_dag, first_run) = names_for_capture_root(&first).expect("first names");
    let (second_dag, _) = names_for_capture_root(&second).expect("second names");
    assert_eq!(first_dag, first_run);
    assert!(first_dag.starts_with("fanout-"));
    assert!(first_dag.len() < 40, "{first_dag}");
    assert!(second_dag.len() < 40, "{second_dag}");
    assert_ne!(first_dag, second_dag);
    let short = root.path().join("capture-one");
    fs::create_dir_all(&short).expect("mkdir short");
    let (short_dag, _) = names_for_capture_root(&short).expect("short names");
    assert_eq!(short_dag, "fanout-capture-one");
}
