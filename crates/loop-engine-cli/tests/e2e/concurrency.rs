use std::fs;
use std::time::{Duration, Instant};

use crate::support::{
    E2eSandbox, ProviderAddArgs, create_run, invoke_json, scenario_provider_executable,
};

fn wait_for_reached(path: &std::path::Path, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let observed = fs::read_dir(path)
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0);
        if observed >= count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "barrier did not reach {count} workers"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn add_barrier_provider(sandbox: &E2eSandbox, barrier: &std::path::Path) -> String {
    let args = ProviderAddArgs {
        handle: "concurrency".into(),
        exec: scenario_provider_executable().clone(),
        working_directory: sandbox.provider_cwd().to_path_buf(),
        args: vec![
            "--scenario".into(),
            "gate-pass".into(),
            "--barrier-dir".into(),
            barrier.display().to_string(),
            "--barrier-id".into(),
            "alpha".into(),
            "--barrier-action".into(),
            "reached".into(),
        ],
        timeout_seconds: 10,
    }
    .to_cli_args();
    let added = invoke_json(sandbox, "concurrency-add", &args, 0);
    added.document.value["data"]["registration"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn overlapping_creates_pass_an_explicit_provider_barrier_and_both_commit() {
    let sandbox = E2eSandbox::new();
    let barrier = sandbox.caller_cwd().join("concurrency-barrier");
    let provider = add_barrier_provider(&sandbox, &barrier);

    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| create_run(&sandbox, &provider, "concurrency-first"));
        let second = scope.spawn(|| create_run(&sandbox, &provider, "concurrency-second"));
        wait_for_reached(&barrier.join("alpha/reached"), 2);
        fs::write(barrier.join("alpha/release"), b"release").unwrap();
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(first, second);

    let listed = invoke_json(
        &sandbox,
        "concurrency-list",
        &["run".into(), "list".into(), "--all".into()],
        0,
    );
    assert_eq!(
        listed.document.value["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn request_termination_overlap_reports_stale_cas_with_authoritative_snapshot() {
    let sandbox = E2eSandbox::new();
    let barrier = sandbox.caller_cwd().join("stale-barrier");
    let provider = add_barrier_provider(&sandbox, &barrier);

    let run = std::thread::scope(|scope| {
        let worker = scope.spawn(|| create_run(&sandbox, &provider, "stale-run"));
        wait_for_reached(&barrier.join("alpha/reached"), 1);
        fs::write(barrier.join("alpha/release"), b"release").unwrap();
        worker.join().unwrap()
    });
    fs::remove_file(barrier.join("alpha/release")).unwrap();
    fs::remove_dir_all(barrier.join("alpha/reached")).unwrap();

    let stale = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            invoke_json(
                &sandbox,
                "concurrency-request",
                &[
                    "run".into(),
                    "request".into(),
                    run.clone(),
                    "approve".into(),
                ],
                1,
            )
        });
        wait_for_reached(&barrier.join("alpha/reached"), 1);
        let terminated = invoke_json(
            &sandbox,
            "concurrency-terminate",
            &["run".into(), "terminate".into(), run.clone()],
            0,
        );
        assert_eq!(
            terminated.document.value["data"]["run"]["lifecycle"],
            "terminated"
        );
        fs::write(barrier.join("alpha/release"), b"release").unwrap();
        worker.join().unwrap()
    });

    assert_eq!(stale.document.value["outcome"], "error");
    assert_eq!(
        stale.document.value["reason"]["code"],
        "state.stale_version"
    );
    assert_eq!(stale.document.value["data"]["run"]["state"], "draft");
    assert_eq!(
        stale.document.value["data"]["run"]["lifecycle"],
        "terminated"
    );
    assert_eq!(stale.document.value["data"]["run"]["state_changed"], false);
    assert!(
        stale.document.value["data"]["requestable_events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
