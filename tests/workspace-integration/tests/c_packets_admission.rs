//! Public append, reviewer packet, correction and checkpoint regression coverage.
//! The Python fixtures call the real engine/providers with fresh test catalogs;
//! scripted judgments test protocol behavior, never semantic review quality.

#[path = "../../bounded_process.rs"]
mod bounded_process;

use std::process::Command;
use std::time::Duration;

// bookends:LE-188 bookends:LE-109 bookends:LE-114 bookends:LE-130
// bookends:LE-149 bookends:LE-155 bookends:LE-156 bookends:LE-158
// bookends:LE-170 bookends:LE-171 bookends:LE-107
#[test]
fn public_contract_cases() {
    let repository = workspace_integration::repository_root();
    let temporary = tempfile::tempdir().expect("external fixture root");
    let output = temporary.path().join("public-contract");
    let mut command = Command::new("python3");
    command
        .arg("-B")
        .arg(
            repository
                .join("tests/workspace-integration/tests/c_packets_admission/public_suite.py"),
        )
        .arg("--source-root")
        .arg(&repository)
        .arg("--output-root")
        .arg(&output)
        .current_dir(&repository);
    for (option, name) in [
        ("--engine", "loop-engine"),
        ("--provider", "software-change"),
        ("--checker", "bookends-check"),
        ("--policy", "policy-document"),
        ("--research", "research"),
    ] {
        command.arg(option).arg(workspace_integration::binary(name));
    }
    let result = bounded_process::run_with_deadline(
        &mut command,
        "C public contract cases",
        Duration::from_secs(900),
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let retained = temporary.keep();
            panic!("public fixture process failed: {error}; retained {retained:?}");
        }
    };
    if !result.status.success() {
        let retained = temporary.keep();
        panic!(
            "public fixtures failed; retained {retained:?}:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let stdout = String::from_utf8(result.stdout).expect("fixture UTF-8");
    let summary: serde_json::Value =
        serde_json::from_str(stdout.lines().last().expect("fixture summary"))
            .expect("fixture summary JSON");
    assert_eq!(summary["completed"].as_array().unwrap().len(), 11);
    assert_eq!(summary["semantic_review"], false);
}
