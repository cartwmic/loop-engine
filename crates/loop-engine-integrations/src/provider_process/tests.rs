use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use loop_engine_core::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
use loop_engine_core::model::ids::{ProviderHandle, RegistrationId};

use super::{ProcessError, process_failure_code, run};

fn process_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn config(command: String, timeout_seconds: u64) -> ResolvedProviderConfig {
    ResolvedProviderConfig::new(
        RegistrationId::parse("provider-1").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new("/bin/sh", vec!["-c".into(), command], "/", timeout_seconds).unwrap(),
    )
    .unwrap()
}

#[test]
fn rejects_timeout_outside_platform_instant_range_without_panicking() {
    let _guard = process_test_guard();
    let provider = config("exit 0".into(), u64::MAX);
    assert!(matches!(
        run(&provider, b"{}"),
        Err(ProcessError::TimeoutOutOfRange(u64::MAX))
    ));
}

#[test]
fn drains_large_stdout_and_stderr_concurrently_without_deadlock() {
    let _guard = process_test_guard();
    let provider = config(
        "(head -c 1100000 /dev/zero | tr '\\0' x) & (head -c 1100000 /dev/zero | tr '\\0' y >&2) & wait".into(),
        10,
    );
    assert!(matches!(
        run(&provider, b"{}"),
        Err(ProcessError::StdoutOversized {
            max: 1_048_576,
            actual: 1_100_000
        })
    ));
}

#[test]
fn timeout_kills_descendant_process_group() {
    let _guard = process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("descendant.pid");
    let provider = config(
        format!(
            "trap '' TERM; (trap '' TERM; sleep 30) & echo $! > '{}'; sleep 30",
            pid_file.display()
        ),
        1,
    );
    let started = Instant::now();
    assert!(matches!(run(&provider, b"{}"), Err(ProcessError::Timeout)));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(5));
    assert!(elapsed < Duration::from_secs(9));
    let pid = std::fs::read_to_string(pid_file).unwrap();
    let pid = pid.trim();
    for _ in 0..20 {
        let alive = std::process::Command::new("/bin/kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("provider descendant {pid} survived timeout cleanup");
}

#[test]
fn successful_parent_exit_cleans_up_conforming_background_descendant() {
    let _guard = process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("descendant.pid");
    let provider = config(
        format!(
            "cat > /dev/null; (sleep 30 </dev/null >/dev/null 2>&1) & echo $! > '{}'; printf '{{}}'",
            pid_file.display()
        ),
        10,
    );
    assert!(run(&provider, b"{}").is_ok());
    let pid = std::fs::read_to_string(pid_file).unwrap();
    let alive = std::process::Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(
        !alive,
        "provider descendant {} survived success cleanup",
        pid.trim()
    );
}

#[test]
fn timeout_remains_authoritative_when_blocked_stdin_fails_during_termination() {
    let _guard = process_test_guard();
    let provider = config("trap '' TERM; sleep 30".into(), 1);
    let request = vec![b'x'; 4_000_000];
    assert!(matches!(
        run(&provider, &request),
        Err(ProcessError::Timeout)
    ));
}

#[test]
fn timeout_still_applies_after_parent_exits_while_descendant_holds_pipes() {
    let _guard = process_test_guard();
    let provider = config("(trap '' TERM; sleep 30) & exit 0".into(), 1);
    let started = Instant::now();
    assert!(matches!(run(&provider, b"{}"), Err(ProcessError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(9));
}

#[test]
fn timeout_does_not_block_on_descendant_that_leaves_provider_group() {
    let _guard = process_test_guard();
    let provider = config("set -m; sleep 3 & exit 0".into(), 1);
    let started = Instant::now();
    assert!(matches!(run(&provider, b"{}"), Err(ProcessError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn launch_uses_literal_argv_without_shell_interpretation() {
    let _guard = process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("shell-injection");
    let argument = format!("$HOME;touch {}", marker.display());
    let provider = ResolvedProviderConfig::new(
        RegistrationId::parse("provider-1").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new("/bin/echo", vec![argument.clone()], "/", 5).unwrap(),
    )
    .unwrap();
    let output = run(&provider, b"").unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{argument}\n")
    );
    assert!(!marker.exists());
}

#[test]
fn explicit_cwd_and_inherited_environment_do_not_depend_on_caller_cwd() {
    let _guard = process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().to_str().unwrap().to_owned();
    let observed_cwd = directory.path().canonicalize().unwrap();
    let inherited = std::env::var("HOME").unwrap();
    let provider = ResolvedProviderConfig::new(
        RegistrationId::parse("provider-1").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new(
            "/bin/sh",
            vec!["-c".into(), "printf '%s\\n%s' \"$PWD\" \"$HOME\"".into()],
            cwd,
            5,
        )
        .unwrap(),
    )
    .unwrap();
    let output = run(&provider, b"").unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n{inherited}", observed_cwd.display())
    );
}

#[test]
fn missing_nonzero_crash_signal_and_invalid_utf8_are_distinct() {
    let _guard = process_test_guard();
    let missing = ResolvedProviderConfig::new(
        RegistrationId::parse("provider-1").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new("/definitely/missing/provider", vec![], "/", 5).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        run(&missing, b""),
        Err(ProcessError::ExecutableNotFound(_))
    ));
    let nonzero = run(&config("cat >/dev/null; exit 7".into(), 5), b"");
    assert!(
        matches!(nonzero, Err(ProcessError::NonZero(Some(7)))),
        "unexpected nonzero observation: {nonzero:?}"
    );
    let crash = run(&config("cat >/dev/null; kill -SEGV $$".into(), 5), b"").unwrap_err();
    assert!(matches!(&crash, ProcessError::Crash(_)));
    assert_eq!(process_failure_code(&crash), "provider.crash");
    assert!(matches!(
        run(&config("cat >/dev/null; kill -TERM $$".into(), 5), b""),
        Err(ProcessError::Signal(_))
    ));
    assert!(matches!(
        run(&config("cat >/dev/null; printf '\\377'".into(), 5), b""),
        Err(ProcessError::InvalidUtf8)
    ));
}
