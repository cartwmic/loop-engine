use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support::{
    E2eSandbox, add_scenario_provider, create_run, parse_pre_dispatch_stderr,
    parse_structured_stdout, run_with_rlimit_fsize, verify_rlimit_blocks_writes,
};

struct ChildProcesses(Vec<Child>);

impl Drop for ChildProcesses {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for_count(path: &std::path::Path, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = fs::read_dir(path)
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0);
        if count >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "only {count}/{expected} workers reached barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn active_reservations_exhaust_budget_without_eviction_or_dispatch() {
    let sandbox = E2eSandbox::new();
    let fifo_dir = sandbox.caller_cwd().join("trace-fifos");
    fs::create_dir(&fifo_dir).unwrap();
    let fifos = (0..8)
        .map(|index| {
            let path = fifo_dir.join(format!("input-{index}"));
            assert!(
                Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            path
        })
        .collect::<Vec<_>>();
    let absent_provider = "019f8f00-0000-7000-8000-000000000099";

    let workers = ChildProcesses(
        fifos
            .iter()
            .map(|fifo| {
                Command::new(env!("CARGO_BIN_EXE_loop-engine"))
                    .args([
                        "--format",
                        "json",
                        "run",
                        "create",
                        absent_provider,
                        "--inputs",
                    ])
                    .arg(fifo)
                    .env("LOOP_ENGINE_HOME", sandbox.loop_engine_home())
                    .current_dir(sandbox.caller_cwd())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>(),
    );

    wait_for_count(&sandbox.traces_dir().join(".reserve"), 8);
    let exhausted = sandbox
        .runner()
        .run_json("trace-reservation-exhausted", &["provider", "list"]);
    assert_ne!(exhausted.exit_code, Some(0));
    assert!(exhausted.stdout.is_empty());
    let failure = parse_pre_dispatch_stderr(&exhausted.stderr).unwrap();
    assert_eq!(failure.value["phase"], "trace_init");
    assert!(
        failure.value["message"]
            .as_str()
            .unwrap()
            .contains("budget")
    );
    assert!(failure.value["trace"].is_null());
    assert!(!sandbox.state_db_path().exists());
    drop(workers);
}

#[test]
fn late_trace_sink_failure_does_not_reclassify_completed_operation() {
    let sandbox = E2eSandbox::new();
    let provider = add_scenario_provider(&sandbox, "late-sink", "process-oversized-stderr", &[]);
    let byte_limit = 262_144;
    verify_rlimit_blocks_writes(byte_limit).expect("RLIMIT_FSIZE enforced");
    let invocation = run_with_rlimit_fsize(
        &sandbox,
        "late-sink-provider-check",
        &["provider", "check", &provider],
        byte_limit,
    )
    .expect("run under RLIMIT_FSIZE");
    assert_eq!(invocation.exit_code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&invocation.stderr),
        "trace sink failure after dispatch: trace sink is unavailable after a prior write failure\n"
    );
    let document = parse_structured_stdout(&invocation.stdout).unwrap();
    assert_eq!(document.value["outcome"], "completed");
    assert_eq!(document.value["operation"], "provider.check");
    assert!(
        document.value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "trace.sink_failure" })
    );
}

#[test]
fn late_trace_sink_failure_preserves_committed_mutation_and_durable_readback() {
    let sandbox = E2eSandbox::new();
    let provider = add_scenario_provider(&sandbox, "late-sink-mutation", "graph-linear", &[]);
    let run_id = create_run(&sandbox, &provider, "late-sink-active-run");
    let note = "x".repeat(65_536);
    let byte_limit = 262_144;
    verify_rlimit_blocks_writes(byte_limit).expect("RLIMIT_FSIZE enforced");

    // External harness fills only the active trace after persistence intent. Production child
    // remains unmodified and encounters RLIMIT_FSIZE on its post-commit trace write.
    let traces_dir = sandbox.traces_dir();
    let existing = fs::read_dir(&traces_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<HashSet<_>>();
    let padder = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for entry in fs::read_dir(&traces_dir).unwrap().filter_map(Result::ok) {
                let path = entry.path();
                if existing.contains(&path)
                    || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
                {
                    continue;
                }
                let Ok(contents) = fs::read_to_string(&path) else {
                    continue;
                };
                if !contents.contains("\"category\":\"persistence\",\"event\":\"intent\"") {
                    continue;
                }
                let current = fs::metadata(&path).unwrap().len();
                let mut file = OpenOptions::new().append(true).open(path).unwrap();
                file.write_all(&vec![b' '; (byte_limit - current) as usize])
                    .unwrap();
                file.flush().unwrap();
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    });

    let invocation = run_with_rlimit_fsize(
        &sandbox,
        "late-sink-run-terminate",
        &["run", "terminate", &run_id, "--note", &note],
        byte_limit,
    )
    .expect("run mutation under RLIMIT_FSIZE");
    assert!(padder.join().expect("trace padder thread"));
    assert_eq!(invocation.exit_code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&invocation.stderr),
        "trace sink failure after dispatch: trace sink is unavailable after a prior write failure\n"
    );
    let document = parse_structured_stdout(&invocation.stdout).unwrap();
    assert_eq!(document.value["outcome"], "completed");
    assert_eq!(document.value["operation"], "run.terminate");
    assert!(
        document.value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "trace.sink_failure"
                    && diagnostic["context"]["after_commit"] == true
            })
    );

    let reopened = sandbox
        .runner()
        .run_json("late-sink-run-show", &["run", "show", &run_id]);
    assert_eq!(reopened.exit_code, Some(0));
    let reopened = parse_structured_stdout(&reopened.stdout).unwrap();
    assert_eq!(reopened.value["data"]["run"]["id"], run_id);
    assert_eq!(reopened.value["data"]["run"]["lifecycle"], "terminated");
}
