use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use xtask::process::{
    Cancellation, CancellationRequest, CleanupOutcome, EnvironmentChanges, ProcessSpec,
    ProcessTermination, SpawnFailureKind, StreamEncoding, StreamKind, spawn,
    spawn_with_cancellation,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process/process-fixture")
}

fn candidate() -> TempDir {
    tempfile::tempdir().expect("candidate root")
}

fn spec(root: &Path, args: &[&str]) -> ProcessSpec {
    ProcessSpec::new(
        fixture().to_string_lossy().into_owned(),
        args.iter().map(|value| (*value).to_owned()).collect(),
        root,
        root,
        Duration::from_secs(5),
        64 * 1024,
    )
}

fn process_is_live(pid: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if Instant::now() >= deadline {
            return true;
        }
        let child = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", pid])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn targeted ps");
        let output = wait_for_probe(child, deadline, pid);
        let state = String::from_utf8_lossy(&output);
        if state.trim().is_empty() || state.trim_start().starts_with('Z') {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_probe(mut child: std::process::Child, deadline: Instant, pid: &str) -> Vec<u8> {
    loop {
        match child.try_wait().expect("poll targeted ps") {
            Some(_) => {
                child.wait().expect("reap targeted ps");
                let mut output = Vec::new();
                child
                    .stdout
                    .take()
                    .expect("ps stdout")
                    .take(64)
                    .read_to_end(&mut output)
                    .expect("read bounded ps output");
                return output;
            }
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("timed out probing process {pid}");
            }
        }
    }
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "fixture did not write {}", path.display());
}

#[test]
fn preserves_exact_argv_including_empty_and_space_values() {
    let root = candidate();
    let outcome = spawn(spec(
        root.path(),
        &["argv", "", " ", "two words", "$HOME;false"],
    ))
    .await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert_eq!(
        outcome.stdout.data(),
        "<>\n< >\n<two words>\n<$HOME;false>\n"
    );
    assert!(outcome.stdout.complete());
}

#[test]
fn inherits_environment_then_applies_set_and_unset_with_unset_winning() {
    let root = candidate();
    let inherited = std::env::var("HOME").expect("HOME inherited by test process");
    let mut set = BTreeMap::new();
    set.insert("PROCESS_SET".to_owned(), "set value".to_owned());
    set.insert("PROCESS_REMOVED".to_owned(), "must lose".to_owned());
    let unset = BTreeSet::from(["PROCESS_REMOVED".to_owned()]);
    let configured =
        spec(root.path(), &["environment"]).with_environment(EnvironmentChanges::new(set, unset));

    let outcome = spawn(configured).await_completion();
    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert_eq!(
        outcome.stdout.data(),
        format!("{inherited}\nset value\nunset")
    );
}

#[test]
fn keeps_stdout_and_stderr_separate() {
    let root = candidate();
    let outcome = spawn(spec(root.path(), &["streams"])).await_completion();

    assert_eq!(outcome.stdout.data(), "stdout-bytes");
    assert_eq!(outcome.stderr.data(), "stderr-bytes");
    assert_eq!(outcome.stdout.encoding(), StreamEncoding::Utf8);
    assert_eq!(outcome.stderr.encoding(), StreamEncoding::Utf8);
}

#[test]
fn invalid_utf8_round_trips_as_base64() {
    let root = candidate();
    let outcome = spawn(spec(root.path(), &["invalid-utf8"])).await_completion();

    assert_eq!(outcome.stdout.encoding(), StreamEncoding::Base64);
    assert_eq!(outcome.stdout.data(), "/wBB");
    assert_eq!(outcome.stdout.exact_bytes(), &[0xff, 0x00, b'A']);
    assert!(outcome.stdout.complete());
    let json = serde_json::to_value(&outcome).expect("serialize evidence");
    assert_eq!(json["stdout"]["encoding"], "base64");
    assert_eq!(json["stdout"]["data"], "/wBB");
    assert_eq!(json["stderr"]["encoding"], "utf-8");
}

#[test]
fn base64_padding_is_exact_for_one_and_two_byte_tails() {
    let root = candidate();
    let one = spawn(spec(root.path(), &["invalid-utf8-one"])).await_completion();
    let two = spawn(spec(root.path(), &["invalid-utf8-two"])).await_completion();

    assert_eq!(one.stdout.exact_bytes(), &[0xff]);
    assert_eq!(one.stdout.data(), "/w==");
    assert_eq!(two.stdout.exact_bytes(), &[0xff, 0xfe]);
    assert_eq!(two.stdout.data(), "//4=");
}

#[test]
fn missing_executable_is_typed_spawn_failure() {
    let root = candidate();
    let missing = ProcessSpec::new(
        "/definitely/missing/reusable-validator",
        Vec::new(),
        root.path(),
        root.path(),
        Duration::from_secs(1),
        1024,
    );
    let outcome = spawn(missing).await_completion();

    assert!(matches!(
        outcome.termination,
        ProcessTermination::SpawnFailure {
            failure_kind: SpawnFailureKind::ExecutableNotFound,
            ..
        }
    ));
    assert!(matches!(outcome.cleanup, CleanupOutcome::NotRequired));
}

#[test]
fn distinguishes_nonzero_exit_and_signal() {
    let root = candidate();
    let nonzero = spawn(spec(root.path(), &["exit", "7"])).await_completion();
    assert_eq!(nonzero.termination, ProcessTermination::Exit { code: 7 });

    let signaled = spawn(spec(root.path(), &["signal", "TERM"])).await_completion();
    assert_eq!(
        signaled.termination,
        ProcessTermination::Signal { signal: 15 }
    );
}

#[test]
fn timeout_terminates_process_group_and_records_cleanup() {
    let root = candidate();
    let pid_file = root.path().join("child.pid");
    let command = spec(
        root.path(),
        &["child-tree", pid_file.to_str().expect("utf8 pid path")],
    )
    .with_timeout(Duration::from_millis(150));
    let started = Instant::now();
    let outcome = spawn(command).await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Timeout);
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(outcome.duration_millis >= 100);
    assert!(matches!(
        outcome.cleanup,
        CleanupOutcome::Completed {
            term_sent: true,
            kill_sent: true
        }
    ));
    let pid = fs::read_to_string(pid_file).expect("child pid");
    assert!(
        !process_is_live(pid.trim()),
        "child process survived timeout"
    );
}

#[test]
fn externally_triggered_cancellation_is_idempotent_and_cleans_group() {
    let root = candidate();
    let pid_file = root.path().join("child.pid");
    let running = spawn(spec(
        root.path(),
        &["child-tree", pid_file.to_str().expect("utf8 pid path")],
    ));
    let cancellation = running.cancellation_handle();
    wait_for_file(&pid_file);

    assert_eq!(cancellation.cancel(), CancellationRequest::Requested);
    assert_eq!(cancellation.cancel(), CancellationRequest::AlreadyRequested);
    let outcome = running.await_completion();
    assert_eq!(outcome.termination, ProcessTermination::Cancelled);
    assert!(matches!(outcome.cleanup, CleanupOutcome::Completed { .. }));
    assert_eq!(cancellation.cancel(), CancellationRequest::AlreadyFinished);
    let pid = fs::read_to_string(pid_file).expect("child pid");
    assert!(!process_is_live(pid.trim()), "child survived cancellation");
}

#[test]
fn shared_cancellation_terminates_and_awaits_multiple_active_groups() {
    let root = candidate();
    let cancellation = Cancellation::new();
    let first = spawn_with_cancellation(spec(root.path(), &["sleep", "30"]), &cancellation)
        .expect("first registered process");
    let second = spawn_with_cancellation(spec(root.path(), &["sleep", "30"]), &cancellation)
        .expect("second registered process");

    assert_eq!(cancellation.cancel(), CancellationRequest::Requested);
    assert!(spawn_with_cancellation(spec(root.path(), &["sleep", "30"]), &cancellation).is_none());
    let first = first.await_completion();
    let second = second.await_completion();
    assert_eq!(first.termination, ProcessTermination::Cancelled);
    assert_eq!(second.termination, ProcessTermination::Cancelled);
    assert!(matches!(first.cleanup, CleanupOutcome::Completed { .. }));
    assert!(matches!(second.cleanup, CleanupOutcome::Completed { .. }));
}

#[test]
fn cancellation_finish_atomically_excludes_late_signals_and_children() {
    let root = candidate();
    let finished = Cancellation::new();

    assert!(finished.finish());
    assert_eq!(finished.cancel(), CancellationRequest::AlreadyFinished);
    assert!(spawn_with_cancellation(spec(root.path(), &["sleep", "30"]), &finished).is_none());
    assert!(finished.finish());

    let interrupted = Cancellation::new();
    assert_eq!(interrupted.cancel(), CancellationRequest::Requested);
    assert!(!interrupted.finish());
}

#[test]
fn failed_process_can_trigger_external_sibling_cancellation() {
    let root = candidate();
    let sibling = spawn(spec(root.path(), &["sleep", "30"]));
    let sibling_cancellation = sibling.cancellation_handle();
    let failed = spawn(spec(root.path(), &["exit", "9"])).await_completion();

    assert_eq!(failed.termination, ProcessTermination::Exit { code: 9 });
    assert_eq!(
        sibling_cancellation.cancel(),
        CancellationRequest::Requested
    );
    let cancelled = sibling.await_completion();
    assert_eq!(cancelled.termination, ProcessTermination::Cancelled);
    assert!(matches!(
        cancelled.cleanup,
        CleanupOutcome::Completed { .. }
    ));
}

#[test]
fn successful_parent_exit_cleans_background_descendant() {
    let root = candidate();
    let pid_file = root.path().join("child.pid");
    let outcome = spawn(spec(
        root.path(),
        &[
            "background-child",
            pid_file.to_str().expect("utf8 pid path"),
        ],
    ))
    .await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert!(matches!(outcome.cleanup, CleanupOutcome::Completed { .. }));
    let pid = fs::read_to_string(pid_file).expect("child pid");
    assert!(!process_is_live(pid.trim()), "background child survived");
}

#[test]
fn stdout_limit_cancels_immediately_and_marks_only_capture_incomplete() {
    let root = candidate();
    let command = spec(root.path(), &["flood-stdout"]).with_max_output_bytes(37);
    let started = Instant::now();
    let outcome = spawn(command).await_completion();

    assert_eq!(
        outcome.termination,
        ProcessTermination::OutputLimit {
            streams: vec![StreamKind::Stdout]
        }
    );
    assert_eq!(outcome.stdout.exact_bytes().len(), 37);
    assert!(!outcome.stdout.complete());
    assert!(outcome.stderr.complete());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn output_exactly_at_limit_is_complete() {
    let root = candidate();
    let outcome = spawn(spec(root.path(), &["exact-output", "37"]).with_max_output_bytes(37))
        .await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert_eq!(outcome.stdout.exact_bytes().len(), 37);
    assert!(outcome.stdout.complete());
    assert!(outcome.success(), "{outcome:#?}");
}

#[test]
fn quick_exit_output_limit_wins_after_leader_completion() {
    let root = candidate();
    // Descendant emits only when cleanup SIGTERM proves leader completion was
    // observed, making the late reader report deterministic.
    let ready_file = root.path().join("output-writer.ready");
    let outcome = spawn(
        spec(
            root.path(),
            &[
                "quick-exit-over-limit",
                ready_file.to_str().expect("utf8 ready path"),
            ],
        )
        .with_max_output_bytes(7),
    )
    .await_completion();

    assert_eq!(
        outcome.termination,
        ProcessTermination::OutputLimit {
            streams: vec![StreamKind::Stdout]
        }
    );
    assert_eq!(outcome.stdout.exact_bytes().len(), 7);
    assert!(!outcome.stdout.complete());
    assert!(!outcome.success());
    assert!(matches!(
        outcome.cleanup,
        CleanupOutcome::Completed {
            term_sent: true,
            kill_sent: true
        }
    ));
}

#[test]
fn stderr_has_independent_output_limit() {
    let root = candidate();
    let command = spec(root.path(), &["flood-stderr"]).with_max_output_bytes(19);
    let outcome = spawn(command).await_completion();

    assert_eq!(
        outcome.termination,
        ProcessTermination::OutputLimit {
            streams: vec![StreamKind::Stderr]
        }
    );
    assert_eq!(outcome.stderr.exact_bytes().len(), 19);
    assert!(!outcome.stderr.complete());
    assert!(outcome.stdout.complete());
}

#[test]
fn sends_stdin_without_shell_or_text_conversion() {
    let root = candidate();
    let input = vec![0, b'a', 0xff, b'\n'];
    let outcome = spawn(spec(root.path(), &["stdin"]).with_stdin(input.clone())).await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert_eq!(outcome.stdout.exact_bytes(), input);
}

#[test]
fn timeout_and_cancellation_interrupt_blocked_large_stdin() {
    let root = candidate();
    let input = vec![b'x'; 8 * 1024 * 1024];
    let timed_out = spawn(
        spec(root.path(), &["ignore-stdin"])
            .with_stdin(input.clone())
            .with_timeout(Duration::from_millis(100)),
    )
    .await_completion();
    assert_eq!(timed_out.termination, ProcessTermination::Timeout);

    let running = spawn(spec(root.path(), &["ignore-stdin"]).with_stdin(input));
    let cancellation = running.cancellation_handle();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(cancellation.cancel(), CancellationRequest::Requested);
    let cancelled = running.await_completion();
    assert_eq!(cancelled.termination, ProcessTermination::Cancelled);
}

#[test]
fn cancellation_cannot_overwrite_observed_completion() {
    let root = candidate();
    let pid_file = root.path().join("stubborn.pid");
    let running = spawn(spec(
        root.path(),
        &[
            "completed-with-stubborn-descendant",
            pid_file.to_str().expect("utf8 pid path"),
        ],
    ));
    let cancellation = running.cancellation_handle();
    wait_for_file(&pid_file);
    let waiter = thread::spawn(|| running.await_completion());
    thread::sleep(Duration::from_millis(100));

    assert_eq!(cancellation.cancel(), CancellationRequest::AlreadyFinished);
    let outcome = waiter.join().expect("await thread");
    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert!(
        matches!(outcome.cleanup, CleanupOutcome::Completed { .. }),
        "{outcome:#?}"
    );
}

#[test]
fn cancellation_racing_drop_never_targets_released_group() {
    for iteration in 0..25 {
        let root = candidate();
        let pid_file = root.path().join(format!("leader-{iteration}.pid"));
        let running = spawn(spec(
            root.path(),
            &["write-pid-sleep", pid_file.to_str().expect("utf8 pid path")],
        ));
        let cancellation = running.cancellation_handle();
        wait_for_file(&pid_file);
        let barrier = Arc::new(Barrier::new(2));
        let cancel_barrier = Arc::clone(&barrier);
        let cancel_thread = thread::spawn(move || {
            cancel_barrier.wait();
            cancellation.cancel()
        });
        barrier.wait();
        drop(running);
        let request = cancel_thread.join().expect("cancel thread");
        assert!(matches!(
            request,
            CancellationRequest::Requested
                | CancellationRequest::AlreadyRequested
                | CancellationRequest::AlreadyFinished
        ));
        let pid = fs::read_to_string(&pid_file).expect("leader pid");
        assert!(!process_is_live(pid.trim()), "dropped leader survived");
    }
}

#[test]
fn cleanup_failure_prevents_success() {
    let root = candidate();
    let mut outcome = spawn(spec(root.path(), &["streams"])).await_completion();
    assert!(outcome.success());
    outcome.cleanup = CleanupOutcome::Failed {
        term_sent: true,
        kill_sent: true,
        message: "fixture cleanup failure".to_owned(),
    };
    assert!(!outcome.success());
}

#[test]
fn rejects_cwd_outside_candidate_root_including_symlink_escape() {
    let root = candidate();
    let outside = candidate();
    let direct = ProcessSpec::new(
        fixture().to_string_lossy().into_owned(),
        vec!["streams".to_owned()],
        root.path(),
        outside.path(),
        Duration::from_secs(1),
        1024,
    );
    let direct_outcome = spawn(direct).await_completion();
    assert!(matches!(
        direct_outcome.termination,
        ProcessTermination::SpawnFailure {
            failure_kind: SpawnFailureKind::CwdOutsideCandidate,
            ..
        }
    ));

    let link = root.path().join("escape");
    symlink(outside.path(), &link).expect("escape symlink");
    let escaped = ProcessSpec::new(
        fixture().to_string_lossy().into_owned(),
        vec!["streams".to_owned()],
        root.path(),
        &link,
        Duration::from_secs(1),
        1024,
    );
    let escaped_outcome = spawn(escaped).await_completion();
    assert!(matches!(
        escaped_outcome.termination,
        ProcessTermination::SpawnFailure {
            failure_kind: SpawnFailureKind::CwdOutsideCandidate,
            ..
        }
    ));
}

#[test]
fn resolves_relative_cwd_beneath_candidate_root() {
    let root = candidate();
    fs::create_dir(root.path().join("nested")).expect("nested cwd");
    let command = ProcessSpec::new(
        "/bin/pwd",
        Vec::new(),
        root.path(),
        "nested",
        Duration::from_secs(1),
        1024,
    );
    let outcome = spawn(command).await_completion();

    assert_eq!(outcome.termination, ProcessTermination::Exit { code: 0 });
    assert_eq!(
        outcome.stdout.data().trim_end(),
        root.path()
            .join("nested")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
}
