use super::bounded_process;
use loop_integrations::ownership::{Admission, OWNERSHIP_ENV};
use serde_json::json;
use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn wait_for(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(Instant::now() < deadline, "missing {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn recovery_ownership_both_helpers_publish_before_primary_and_inhibit_queued_work() {
    for binary in ["loop-engine", "software-change"] {
        let root = tempfile::tempdir().unwrap();
        let ownership = root.path().join("ownership");
        let input = root.path().join("stdin");
        let marker = root.path().join("marker");
        let release = root.path().join("release");
        fs::write(&input, "{}").unwrap();
        let script = root.path().join("worker.py");
        fs::write(
            &script,
            r#"import json, os, pathlib, sys, time
ownership = pathlib.Path(os.environ['LOOP_ENGINE_OWNERSHIP_DIRECTORY'])
parent = os.getppid()
record = json.loads((ownership / ('helper-%s.json' % parent)).read_text())
assert record['root_pid'] == parent
assert record['process_group_id'] == os.getpgrp()
pathlib.Path(sys.argv[1]).write_text('published-before-primary')
while not pathlib.Path(sys.argv[2]).exists(): time.sleep(.01)
print('partial capture retained', flush=True)
sys.exit(7)
"#,
        )
        .unwrap();
        let mut command = Command::new(workspace_integration::binary(binary));
        command
            .args(["stdin-exec", "--stdin-file"])
            .arg(&input)
            .args(["--exit-mode", "propagate", "--", "python3"])
            .arg(&script)
            .arg(&marker)
            .arg(&release)
            .env(OWNERSHIP_ENV, &ownership)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        bounded_process::prepare_process_group(&mut command);
        let admission = Admission::acquire(&ownership).unwrap();
        let child = command.spawn().unwrap();
        admission
            .publish(&loop_core::OwnedExecution {
                root_pid: child.id(),
                process_group_id: child.id(),
                admission_directory: ownership.clone(),
                graph_locator: None,
            })
            .unwrap();
        assert_eq!(
            loop_integrations::ownership::read_ownership(root.path().to_str().unwrap())
                .unwrap()
                .unwrap()
                .root_pid,
            child.id()
        );
        // Primary execution is held behind the shared admission barrier, even
        // though this real helper's ownership is already discoverable.
        std::thread::sleep(Duration::from_millis(150));
        assert!(!marker.exists());
        drop(admission);
        wait_for(&marker);
        let admission = Admission::acquire(&ownership).unwrap();
        admission
            .admit_stop(&json!({"reason":"T05 seam proof"}))
            .unwrap();
        drop(admission);
        fs::write(&release, "release").unwrap();
        let output = bounded_process::wait_existing(child, "ownership helper actual exit").unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert!(String::from_utf8_lossy(&output.stdout).contains("partial capture retained"));
        fs::remove_file(&marker).unwrap();
        // This later helper is queued at admission. It must recheck the
        // durable stop marker after acquiring the lock, never before it.
        let admission = Admission::acquire(&ownership).unwrap();
        let child = command.spawn().unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(!marker.exists());
        drop(admission);
        let output =
            bounded_process::wait_existing(child, "ownership queued helper refusal").unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("launch inhibited"));
        assert!(!marker.exists(), "{binary} admitted queued primary work");
        assert!(loop_integrations::ownership::cleanup_pending(
            root.path().to_str().unwrap()
        ));
        assert!(
            !loop_integrations::ownership::live_owned_work(root.path().to_str().unwrap()).unwrap()
        );
    }
}
