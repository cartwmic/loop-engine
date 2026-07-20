use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use loop_engine_core::capabilities::provider_catalog::{ProviderConfig, ResolvedProviderConfig};
use loop_engine_core::capabilities::provider_invoker::{DescribeRequest, ProviderInvoker};
use loop_engine_core::model::ids::{ProviderHandle, RegistrationId, RequestId};
use loop_engine_integrations::provider_protocol::SubprocessProviderInvoker;
use loop_engine_integrations::trace::TraceWriter;

#[test]
fn timeout_terminates_provider_process_group_and_descendant_without_retry() {
    let root = tempfile::tempdir().unwrap();
    let pid_file = root.path().join("descendant.pid");
    let command = format!(
        "(trap '' TERM; sleep 30) & echo $! > '{}'; wait",
        pid_file.display()
    );
    let provider = ResolvedProviderConfig::new(
        RegistrationId::parse("registration").unwrap(),
        ProviderHandle::parse("provider").unwrap(),
        1,
        ProviderConfig::new(
            "/bin/sh",
            vec!["-c".into(), command],
            root.path().to_str().unwrap(),
            1,
        )
        .unwrap(),
    )
    .unwrap();
    let writer = TraceWriter::create(&root.path().join("traces"), "trace-timeout").unwrap();
    let invoker = SubprocessProviderInvoker::new(Arc::new(Mutex::new(writer)));
    let started = Instant::now();
    let result = invoker.describe(
        &provider,
        DescribeRequest {
            request_id: RequestId::parse("request").unwrap(),
        },
    );
    assert!(result.is_err());
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(5));
    assert!(elapsed < Duration::from_secs(9));
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = || {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "descendant {pid} survived");
    let trace = std::fs::read_to_string(root.path().join("traces/trace-timeout.jsonl")).unwrap();
    let events = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["category"], "provider");
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[1]["event"], "failure");
    assert_eq!(events[1]["failure_code"], "provider.timeout");
}
