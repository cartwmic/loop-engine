use std::process::Command;
use std::time::Duration;

#[test]
#[ignore = "run explicitly with LOOP_ENGINE_PROCESS_TIMEOUT_PROBE=injected-wedge or diagnostic"]
fn process_timeout_probe() {
    match std::env::var("LOOP_ENGINE_PROCESS_TIMEOUT_PROBE").as_deref() {
        Ok("injected-wedge") => {
            let mut command = Command::new("python3");
            command.args([
                "-c",
                "import subprocess, time; subprocess.Popen(['sleep', '60']); time.sleep(60)",
            ]);
            super::bounded_process::run_with_deadline(
                &mut command,
                "process_timeout_probe/injected-wedge",
                Duration::from_secs(1),
            )
            .expect("injected wedge must fail through the bounded process boundary");
        }
        Ok("diagnostic") => {
            assert_eq!(
                "process_timeout_probe-left",
                "process_timeout_probe-right",
                "process_timeout_probe diagnostic assertion",
            );
        }
        Ok(mode) => panic!(
            "unknown LOOP_ENGINE_PROCESS_TIMEOUT_PROBE mode {mode:?}; use injected-wedge or diagnostic"
        ),
        Err(_) => panic!(
            "LOOP_ENGINE_PROCESS_TIMEOUT_PROBE must be set to injected-wedge or diagnostic"
        ),
    }
}
