//! Central coverage for the locator-repair journey: a reviewer return whose only defect is
//! unresolvable evidence locators is recovered by a provider-checked repair. The Python journey
//! drives the real engine and software-change programs on a private catalog with scripted dummy
//! reviewers; it never uses a paid model and establishes no semantic review.

#[path = "../../bounded_process.rs"]
mod bounded_process;

use std::process::Command;
use std::time::Duration;

// bookends:LE-109 bookends:LE-151 bookends:LE-155 bookends:LE-170
#[test]
fn locator_repair_journey_cases() {
    let repository = workspace_integration::repository_root();
    let temporary = tempfile::tempdir().expect("external journey root");
    let mut command = Command::new("python3");
    command
        .arg("-B")
        .arg(repository.join("scripts/locator-repair-journey.py"))
        .arg("--output-root")
        .arg(temporary.path().join("locator-repair"))
        .current_dir(&repository);
    for (option, name) in [
        ("--engine", "loop-engine"),
        ("--provider", "software-change"),
    ] {
        command.arg(option).arg(workspace_integration::binary(name));
    }
    // The inherited LOOP and XDG redirects must not reach the private catalogs.
    for (key, _) in std::env::vars() {
        if key.starts_with("LOOP_") || key.starts_with("XDG_") {
            command.env_remove(key);
        }
    }
    let result = bounded_process::run_with_deadline(
        &mut command,
        "locator-repair journey",
        Duration::from_secs(1500),
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let retained = temporary.keep();
            panic!("locator-repair journey failed: {error}; retained {retained:?}");
        }
    };
    if !result.status.success() {
        let retained = temporary.keep();
        panic!(
            "locator-repair journey failed; retained {retained:?}:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
